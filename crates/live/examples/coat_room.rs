#[allow(dead_code)]
#[path = "../tests/support/coat_room.rs"]
mod coat_room;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coat_room::{
    Agreement, Live, Region, Room, agreement, bake_cube, bake_probes, cube_artifact, cube_spec,
    grid_spec, local_spec, luminance, regions, scratch, timings_ms, trace,
};
use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, GpuProfiler, wgpu};

struct Variant {
    name: &'static str,
    cube: Option<f32>,
    occlusion: f32,
}

const VARIANTS: [Variant; 6] = [
    Variant {
        name: "sky only",
        cube: None,
        occlusion: 0.0,
    },
    Variant {
        name: "sky only, occlusion 0.5",
        cube: None,
        occlusion: 0.5,
    },
    Variant {
        name: "room cube, fade 0.25",
        cube: Some(0.25),
        occlusion: 0.0,
    },
    Variant {
        name: "room cube, fade 0.25, occlusion 0.5",
        cube: Some(0.25),
        occlusion: 0.5,
    },
    Variant {
        name: "room cube, fade 0",
        cube: Some(0.0),
        occlusion: 0.0,
    },
    Variant {
        name: "room cube, fade 0, occlusion 0.5",
        cube: Some(0.0),
        occlusion: 0.5,
    },
];

fn arg(index: usize, default: u32) -> u32 {
    std::env::args()
        .nth(index)
        .map(|value| value.parse().expect("arguments are numbers"))
        .unwrap_or(default)
}

fn traced_cached(gpu: &Gpu, room: &Room, width: u32, height: u32, samples: u32) -> Vec<[f32; 3]> {
    let tag = if room.materials[2].clearcoat > 0.0 {
        ""
    } else {
        "matte-"
    };
    let path = scratch("coat-room").join(format!("traced-{tag}{width}x{height}-{samples}.f32"));
    if let Ok(bytes) = fs::read(&path)
        && bytes.len() == (width * height * 12) as usize
    {
        return bytes
            .chunks_exact(12)
            .map(|p| {
                std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
            })
            .collect();
    }
    let began = Instant::now();
    let image = trace(gpu, room, width, height, samples);
    println!(
        "traced {width}x{height} at {samples} samples in {:.1} s",
        began.elapsed().as_secs_f64()
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes: Vec<u8> = image
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    fs::write(&path, bytes).unwrap();
    image
}

fn tone(value: f32) -> u8 {
    let mapped = value * 1.4 / (1.0 + value * 1.4);
    let srgb = pfx_materials::encode_channel(mapped);
    (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn heat(ratio: f64) -> [u8; 3] {
    let t = (ratio.max(1e-3).log2() / 2.0).clamp(-1.0, 1.0) as f32;
    if t >= 0.0 {
        [255, (255.0 * (1.0 - t)) as u8, (255.0 * (1.0 - t)) as u8]
    } else {
        [(255.0 * (1.0 + t)) as u8, (255.0 * (1.0 + t)) as u8, 255]
    }
}

struct Sheet {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Sheet {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![24; (width * height * 3) as usize],
        }
    }

    fn put(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        if x < self.width && y < self.height {
            let at = ((y * self.width + x) * 3) as usize;
            self.pixels[at..at + 3].copy_from_slice(&rgb);
        }
    }

    fn save(&self, path: &Path) {
        let file = fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&self.pixels)
            .unwrap();
    }
}

fn crop(ids: &[u32], width: u32, height: u32) -> [u32; 4] {
    let mut x0 = width;
    let mut y0 = height;
    let mut x1 = 0;
    let mut y1 = 0;
    for (index, id) in ids.iter().enumerate() {
        if *id == coat_room::TOP_ID || *id == coat_room::BODY_ID {
            let x = index as u32 % width;
            let y = index as u32 / width;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    let margin = (x1 - x0) / 10;
    [
        x0.saturating_sub(margin),
        y0.saturating_sub(margin),
        (x1 + margin).min(width - 1),
        (y1 + margin).min(height - 1),
    ]
}

fn print_row(name: &str, region: Region, a: &Agreement) {
    println!(
        "{name:<38} {:<16} {:>8} px  live {:>8.4}  traced {:>8.4}  mean ratio {:>6.3}  p50 {:>6.3}  p95 {:>6.3}  within 0.8-1.25 {:>5.1}%",
        region.name(),
        a.pixels,
        a.live,
        a.traced,
        a.ratio,
        a.p50,
        a.p95,
        a.within * 100.0
    );
}

fn opaque_cost(live: &mut Live, room: &Room, frames: u32) -> (f64, f64) {
    let device = live.frame.gpu.device.clone();
    let queue = live.frame.gpu.queue.clone();
    let mut profiler = GpuProfiler::new(&device, &queue);
    let mut turns = Turns::default();
    let mut opaque = Vec::new();
    let uncapped = std::env::var_os("PFX_UNCAPPED").is_some();
    for _ in 0..frames {
        let began = Instant::now();
        let slot = live.render(
            room,
            live.frame.probes().reflection_occlusion(),
            Some(&mut profiler),
        );
        if let Some(slot) = slot {
            profiler.submitted(slot);
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mut gpu_ms = 0.0;
        for frame in profiler.collect(&device) {
            gpu_ms += frame.iter().map(|t| t.milliseconds).sum::<f64>();
            if let Some(ms) = timings_ms(&frame, "opaque PBR") {
                opaque.push(ms);
            }
        }
        turns.add(gpu_ms);
        let spent = began.elapsed();
        if !uncapped && spent < Duration::from_micros(16_667) {
            std::thread::sleep(Duration::from_micros(16_667) - spent);
        }
    }
    opaque.sort_by(f64::total_cmp);
    let skip = opaque.len() / 10;
    let kept = &opaque[skip..];
    (
        kept[kept.len() / 2],
        kept.iter().sum::<f64>() / kept.len() as f64,
    )
}

fn main() {
    let width = arg(1, 1920);
    let height = arg(2, 1080);
    let samples = arg(3, 256);
    let frames = arg(4, 240);
    let out: PathBuf = scratch("coat-room");
    fs::create_dir_all(&out).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut room = Room::new();
    let matte = std::env::var_os("COAT_ROOM_MATTE").is_some();
    if matte {
        room.materials = coat_room::matte(room.materials);
    }
    let tag = if matte { "matte-" } else { "" };
    let began = Instant::now();
    let mut probe_paths = Vec::new();
    for (name, spec) in [("probes", grid_spec()), ("local", local_spec())] {
        let path = scratch("coat-room").join(format!("{tag}{name}"));
        if !path.join("manifest.json").exists() {
            bake_probes(&gpu, &room, &format!("{tag}{name}"), spec, 256);
        }
        probe_paths.push(path);
    }
    println!("probes ready in {:.1} s", began.elapsed().as_secs_f64());
    let began = Instant::now();
    let cubes: Vec<(f32, pfx_bake::reflection::Cube)> = [0.25_f32, 0.0]
        .iter()
        .map(|&fade| (fade, bake_cube(&gpu, &room, &cube_spec(fade, 128), 128)))
        .collect();
    println!("cubes baked in {:.1} s", began.elapsed().as_secs_f64());
    let traced = traced_cached(&gpu, &room, width, height, samples);
    let mut live = Live::new(gpu, &room, width, height, &probe_paths);
    live.render(&room, 0.0, None);
    let ids = live.ids();
    let labels = regions(&ids, width, height);
    let [cx0, cy0, cx1, cy1] = crop(&ids, width, height);
    let tile_w = (cx1 - cx0 + 1).min(720);
    let scale = (cx1 - cx0 + 1) as f32 / tile_w as f32;
    let tile_h = ((cy1 - cy0 + 1) as f32 / scale) as u32;
    let mut sheet = Sheet::new(tile_w * 3 + 8, (tile_h + 4) * (VARIANTS.len() as u32 + 1));
    let draw = |sheet: &mut Sheet, column: u32, row: u32, pixel: &dyn Fn(usize) -> [u8; 3]| {
        for y in 0..tile_h {
            for x in 0..tile_w {
                let sx = cx0 + (x as f32 * scale) as u32;
                let sy = cy0 + (y as f32 * scale) as u32;
                let index = (sy * width + sx) as usize;
                sheet.put(
                    column * (tile_w + 4) + x,
                    row * (tile_h + 4) + y,
                    pixel(index),
                );
            }
        }
    };
    draw(&mut sheet, 0, 0, &|index| traced[index].map(tone));
    draw(&mut sheet, 1, 0, &|index| match labels[index] {
        Some(Region::Coat) => [200, 120, 60],
        Some(Region::Print) => [40, 40, 40],
        Some(Region::Body) => [120, 80, 160],
        Some(Region::Floor) => [80, 120, 80],
        None => [0, 0, 0],
    });
    println!("{width}x{height}, trace {samples} samples per pixel");
    for (row, variant) in VARIANTS.iter().enumerate() {
        let artifact = variant.cube.map(|fade| {
            let cube = cubes
                .iter()
                .find(|(f, _)| *f == fade)
                .map(|(_, cube)| cube.clone())
                .unwrap();
            cube_artifact(cube_spec(fade, 128), cube, 128)
        });
        live.set_cube(artifact);
        live.render(&room, variant.occlusion, None);
        let image = live.image();
        for region in Region::ALL {
            print_row(
                variant.name,
                region,
                &agreement(&image, &traced, &labels, region),
            );
        }
        let row = row as u32 + 1;
        draw(&mut sheet, 0, row, &|index| image[index].map(tone));
        draw(&mut sheet, 1, row, &|index| {
            heat((luminance(image[index]) + 1e-3) / (luminance(traced[index]) + 1e-3))
        });
        draw(&mut sheet, 2, row, &|index| {
            let difference: [f32; 3] =
                std::array::from_fn(|c| (image[index][c] - traced[index][c]).abs() * 4.0);
            difference.map(tone)
        });
    }
    let sheet_path = out.join(format!("contact-{tag}{width}x{height}.png"));
    sheet.save(&sheet_path);
    println!("wrote {}", sheet_path.display());
    if frames > 0 {
        for fade in [None, Some(0.25_f32)] {
            let artifact = fade.map(|fade| {
                let cube = cubes.iter().find(|(f, _)| *f == fade).unwrap().1.clone();
                cube_artifact(cube_spec(fade, 128), cube, 128)
            });
            let name = if fade.is_some() {
                "room cube"
            } else {
                "sky only"
            };
            live.set_cube(artifact);
            let (p50, mean) = opaque_cost(&mut live, &room, frames);
            println!(
                "opaque at {width}x{height}, {name}: p50 {p50:.3} ms, mean {mean:.3} ms over {frames} frames"
            );
        }
    }
}
