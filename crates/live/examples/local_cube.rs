#[allow(dead_code)]
#[path = "../tests/support/coat_room.rs"]
mod coat_room;
#[allow(dead_code)]
#[path = "../tests/support/local_cube.rs"]
mod local_cube;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coat_room::{ROOM_MAX, ROOM_MIN, scratch, timings_ms};
use local_cube::{
    Live, Set, agreement, bake_cube, bake_probes, box_artifact, cube_artifact, curl, desk_grid,
    mean_abs_tone, nameplate, plate_regions, room_grid, strip_grid, strip_regions, tone, trace,
    tunnel, tunnel_grid, tunnel_regions,
};
use pfx_bake::GridSpec;
use pfx_bake::reflection::ReflectionSpec;
use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, GpuProfiler, wgpu};
use pfx_live::frame::OpaqueFeatures;

#[derive(Clone, Copy)]
enum Cube {
    None,
    Own(f32),
    OwnBox(f32),
    Room(f32),
    Distant,
}

struct Variant {
    name: &'static str,
    cube: Cube,
    occlusion: f32,
    local_off: bool,
    sky_visibility: bool,
    keep: bool,
}

const fn variant(name: &'static str, cube: Cube, occlusion: f32) -> Variant {
    Variant {
        name,
        cube,
        occlusion,
        local_off: false,
        sky_visibility: true,
        keep: false,
    }
}

const NAMEPLATE: [Variant; 10] = [
    variant("no cube", Cube::None, 0.5),
    Variant {
        local_off: true,
        ..variant("desk cube, local off", Cube::Own(0.25), 0.5)
    },
    variant("desk cube, box parallax", Cube::OwnBox(0.25), 0.5),
    variant("desk cube, box parallax, fade 0", Cube::OwnBox(0.0), 0.5),
    variant("desk cube", Cube::Own(0.25), 0.5),
    variant("desk cube, fade 0", Cube::Own(0.0), 0.5),
    variant("desk cube, fade 0, occ 0", Cube::Own(0.0), 0.0),
    variant("desk cube, fade 0, occ 1", Cube::Own(0.0), 1.0),
    variant("room box, fade 0", Cube::Room(0.0), 0.5),
    variant("no parallax, fade 0", Cube::Distant, 0.5),
];

const CURL: [Variant; 7] = [
    variant("own cube, occ 0.5", Cube::Own(0.25), 0.5),
    Variant {
        keep: true,
        ..variant("own cube, occ 0.5, curl kept", Cube::Own(0.25), 0.5)
    },
    variant("own cube, occ 1", Cube::Own(0.25), 1.0),
    Variant {
        keep: true,
        ..variant("own cube, occ 1, curl kept", Cube::Own(0.25), 1.0)
    },
    variant("own cube, occ 0", Cube::Own(0.25), 0.0),
    variant("own cube, box parallax", Cube::OwnBox(0.25), 0.5),
    variant("no cube, occ 0.5", Cube::None, 0.5),
];

const TUNNEL: [Variant; 7] = [
    variant("own cube, occ 0", Cube::Own(0.25), 0.0),
    variant("own cube, box parallax", Cube::OwnBox(0.25), 0.0),
    variant("own cube, occ 0.5", Cube::Own(0.25), 0.5),
    variant("own cube, occ 1", Cube::Own(0.25), 1.0),
    variant("no cube, occ 0", Cube::None, 0.0),
    variant("no cube, occ 0.5", Cube::None, 0.5),
    variant("no cube, occ 1", Cube::None, 1.0),
];

fn arg(index: usize, default: u32) -> u32 {
    std::env::args()
        .nth(index)
        .map(|value| value.parse().expect("arguments are numbers"))
        .unwrap_or(default)
}

fn cached(path: &Path, make: impl FnOnce() -> Vec<[f32; 3]>) -> Vec<[f32; 3]> {
    if let Ok(bytes) = fs::read(path) {
        return bytes
            .chunks_exact(12)
            .map(|p| {
                std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
            })
            .collect();
    }
    let image = make();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes: Vec<u8> = image
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    fs::write(path, bytes).unwrap();
    image
}

fn save(path: &Path, width: u32, height: u32, images: &[&[[f32; 3]]], crop: [u32; 4]) {
    let [x0, y0, cw, ch] = crop;
    let total = cw * images.len() as u32;
    let mut pixels = Vec::with_capacity((total * ch * 3) as usize);
    for y in y0..y0 + ch {
        for image in images {
            for x in x0..x0 + cw {
                let p = image[(y.min(height - 1) * width + x.min(width - 1)) as usize];
                pixels.extend(p.map(tone));
            }
        }
    }
    let file = fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), total, ch);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

fn spec(set: &Set, cube: Cube) -> Option<ReflectionSpec> {
    match cube {
        Cube::None => None,
        Cube::Own(fade) => Some(ReflectionSpec {
            fade,
            ..set.cube.clone()
        }),
        Cube::OwnBox(fade) => Some(ReflectionSpec {
            fade,
            ..set.cube.clone()
        }),
        Cube::Room(fade) => Some(ReflectionSpec {
            fade,
            min: ROOM_MIN,
            max: ROOM_MAX,
            ..set.cube.clone()
        }),
        Cube::Distant => Some(ReflectionSpec {
            min: [-1000.0; 3],
            max: [1000.0; 3],
            fade: 0.0,
            ..set.cube.clone()
        }),
    }
}

fn opaque_ms(live: &mut Live, set: &Set, occlusion: f32, frames: u32) -> f64 {
    let device = live.frame.gpu.device.clone();
    let queue = live.frame.gpu.queue.clone();
    let mut profiler = GpuProfiler::new(&device, &queue);
    let mut turns = Turns::default();
    let mut opaque = Vec::new();
    let uncapped = std::env::var_os("PFX_UNCAPPED").is_some();
    let releases = set.released(true);
    for _ in 0..frames {
        let began = Instant::now();
        if let Some(slot) = live.render(set, occlusion, &releases, Some(&mut profiler)) {
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
    let kept = &opaque[opaque.len() / 10..];
    kept[kept.len() / 2]
}

struct Measured {
    labels: Vec<Option<usize>>,
    names: &'static [&'static str],
}

fn plate_measure(set: &Set, ids: &[u32], width: u32, height: u32) -> Measured {
    Measured {
        labels: plate_regions(set, ids, width, height)
            .into_iter()
            .map(|r| r.map(|r| r as usize))
            .collect(),
        names: &["ink core", "letters", "ground"],
    }
}

fn strip_measure(set: &Set, ids: &[u32], width: u32, height: u32) -> Measured {
    Measured {
        labels: strip_regions(set, ids, width, height)
            .into_iter()
            .map(|r| r.map(|r| r as usize))
            .collect(),
        names: &["flat ink", "flat card", "curl ink", "curl card"],
    }
}

fn tunnel_measure(set: &Set, ids: &[u32], width: u32, height: u32) -> Measured {
    Measured {
        labels: tunnel_regions(set, ids, width, height)
            .into_iter()
            .map(|r| r.map(|r| r as usize))
            .collect(),
        names: &["covered", "open"],
    }
}

fn run(
    gpu: &Gpu,
    set: &Set,
    grids: &[(&str, GridSpec)],
    variants: &[Variant],
    measure: fn(&Set, &[u32], u32, u32) -> Measured,
    [width, height, samples, frames]: [u32; 4],
) {
    let out = scratch("local-cube");
    fs::create_dir_all(&out).unwrap();
    let began = Instant::now();
    let traced = cached(
        &out.join(format!(
            "traced-{}-{width}x{height}-{samples}.f32",
            set.name
        )),
        || trace(gpu, set, width, height, samples),
    );
    println!(
        "{}: traced in {:.1} s",
        set.name,
        began.elapsed().as_secs_f64()
    );
    let probes: Vec<PathBuf> = grids
        .iter()
        .map(|(name, spec)| bake_probes(gpu, set, name, *spec, 128))
        .collect();
    let cube = bake_cube(gpu, set, &set.cube, 64);
    let mut live = Live::new(gpu.clone(), set, width, height, &probes);
    let mut images = Vec::new();
    let mut measured = None;
    for variant in variants {
        live.set_cube(spec(set, variant.cube).map(|spec| {
            if matches!(
                variant.cube,
                Cube::OwnBox(_) | Cube::Room(_) | Cube::Distant
            ) {
                box_artifact(spec, cube.clone(), 64)
            } else {
                cube_artifact(spec, cube.clone(), 64)
            }
        }));
        live.frame.set_sky_visibility(variant.sky_visibility);
        live.frame.set_opaque_off(OpaqueFeatures {
            local_reflections: variant.local_off,
            ..OpaqueFeatures::default()
        });
        let releases = set.released(variant.keep);
        live.render(set, variant.occlusion, &releases, None);
        let image = live.image();
        let regions = measured.get_or_insert_with(|| measure(set, &live.ids(), width, height));
        let mask: Vec<bool> = regions.labels.iter().map(|r| r.is_some()).collect();
        println!(
            "{:<34} |live - trace| {:.2}",
            variant.name,
            mean_abs_tone(&image, &traced, &mask)
        );
        for (index, name) in regions.names.iter().enumerate() {
            let found = agreement(&image, &traced, &regions.labels, index);
            let rgb = |v: [f64; 3]| v.map(|c| tone(c as f32));
            let mask: Vec<bool> = regions.labels.iter().map(|r| *r == Some(index)).collect();
            println!(
                "    {name:<10} live {:?} traced {:?} ratio {:.3} within {:.0}% |d| {:.1} ({} px)",
                rgb(found.live),
                rgb(found.traced),
                found.ratio,
                found.within * 100.0,
                mean_abs_tone(&image, &traced, &mask),
                found.pixels
            );
        }
        if frames > 0 {
            println!(
                "    opaque p50 {:.3} ms",
                opaque_ms(&mut live, set, variant.occlusion, frames)
            );
        }
        images.push(image);
    }
    let mut sheet: Vec<&[[f32; 3]]> = vec![&traced];
    sheet.extend(images.iter().map(|i| i.as_slice()));
    save(
        &out.join(format!("{}-{width}x{height}.png", set.name)),
        width,
        height,
        &sheet,
        [0, 0, width, height],
    );
    for path in probes {
        fs::remove_dir_all(path).unwrap();
    }
}

fn main() {
    let size = [arg(1, 960), arg(2, 540), arg(3, 256), arg(4, 0)];
    let which = std::env::args().nth(5).unwrap_or_else(|| "both".into());
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    if which == "nameplate" || which == "both" {
        run(
            &gpu,
            &nameplate(),
            &[("probes", room_grid()), ("local", desk_grid())],
            &NAMEPLATE,
            plate_measure,
            size,
        );
    }
    if which == "tunnel" || which == "both" {
        run(
            &gpu,
            &tunnel(),
            &[
                ("probes", room_grid()),
                ("tunnel", tunnel_grid()),
                ("local", desk_grid()),
            ],
            &TUNNEL,
            tunnel_measure,
            size,
        );
    }
    if which == "curl" || which == "both" {
        run(
            &gpu,
            &curl(),
            &[
                ("probes", room_grid()),
                ("strip", strip_grid()),
                ("local", desk_grid()),
            ],
            &CURL,
            strip_measure,
            size,
        );
    }
}
