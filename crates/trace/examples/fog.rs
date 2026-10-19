#[path = "../tests/support/street.rs"]
mod street;

use std::path::Path;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Timing, Turns};
use pfx_trace::Trace;
use street::{Haze, Street};

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

fn write_png(path: &Path, size: [u32; 2], pixels: &[[f32; 4]]) {
    let rgb: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| (0..3).map(move |k| encode(pixel[k])))
        .collect();
    let file = std::fs::File::create(path).expect("a png file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&rgb))
        .expect("a written png");
}

fn staged(gpu: &Gpu, street: Street, size: [u32; 2]) -> Trace {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
    let scene = street::scene(
        &base,
        if street.haze.is_some() {
            "fog"
        } else {
            "clear"
        },
        street,
    );
    let mut staged =
        pfx_trace::stage::scene(&scene, size[0] as f32 / size[1] as f32).expect("a staged street");
    if let (Some(fog), Some(bounces)) = (staged.detail.fog.as_mut(), bounces()) {
        fog.bounces = bounces;
    }
    staged.trace(gpu, size[0], size[1]).expect("a trace")
}

fn bounces() -> Option<u32> {
    let words: Vec<String> = std::env::args().collect();
    let at = words.iter().position(|word| word == "--bounces")?;
    words.get(at + 1)?.parse().ok()
}

struct Traced {
    pixels: Vec<[f32; 4]>,
    gpu_ms: f64,
    on_gpu: bool,
}

fn traced(gpu: &Gpu, trace: &mut Trace, samples: u32, seed: u32) -> Traced {
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let stats = trace
        .sample_paced(gpu, samples, seed, &mut pacer, |ms| turns.add(ms))
        .expect("samples");
    turns.turn();
    Traced {
        pixels: street::pixels(&trace.readback_color(gpu).expect("a readback")),
        gpu_ms: stats.milliseconds.iter().sum(),
        on_gpu: stats.timings.iter().all(|timing| *timing == Timing::Gpu),
    }
}

fn fogged(lamp: bool) -> Street {
    Street {
        haze: Some(Haze::STREET),
        lamp,
        sun: street::SIDE_SUN,
        dark: false,
    }
}

fn clear() -> Street {
    Street {
        sun: street::SIDE_SUN,
        ..Street::default()
    }
}

fn look(gpu: &Gpu, out: &Path, samples: u32) {
    std::fs::create_dir_all(out).expect("the out folder");
    let size = [1920, 1080];
    for (name, street) in [
        ("clear", clear()),
        ("fog", fogged(false)),
        ("fog-lamp", fogged(true)),
    ] {
        let mut trace = staged(gpu, street, size);
        let image = traced(gpu, &mut trace, samples, 7);
        let path = out.join(format!("{name}.png"));
        write_png(&path, size, &image.pixels);
        println!("{} ({:.0} ms of GPU)", path.display(), image.gpu_ms);
    }
}

fn band(size: [u32; 2]) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
    let rows = size[1] * 30 / 100..size[1] * 42 / 100;
    let columns = size[0] * 30 / 100..size[0] * 70 / 100;
    (rows, columns)
}

fn profile(pixels: &[[f32; 4]], size: [u32; 2]) -> Vec<f32> {
    let (rows, columns) = band(size);
    columns
        .map(|x| {
            rows.clone()
                .map(|y| street::luminance(pixels[(y * size[0] + x) as usize]))
                .sum::<f32>()
                / rows.len() as f32
        })
        .collect()
}

fn contrast(profile: &[f32]) -> f32 {
    let mut sorted = profile.to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
    let mean = sorted.iter().sum::<f32>() / sorted.len() as f32;
    (at(0.95) - at(0.05)) / mean
}

fn noise(pixels: &[[f32; 4]], reference: &[[f32; 4]], size: [u32; 2]) -> f32 {
    let (rows, columns) = band(size);
    let mut error = 0.0_f64;
    let mut level = 0.0_f64;
    let mut count = 0.0_f64;
    for y in rows {
        for x in columns.clone() {
            let at = (y * size[0] + x) as usize;
            let (a, b) = (
                street::luminance(pixels[at]),
                street::luminance(reference[at]),
            );
            error += f64::from(a - b).powi(2);
            level += f64::from(b);
            count += 1.0;
        }
    }
    ((error / count).sqrt() / (level / count)) as f32
}

fn shafts(gpu: &Gpu, out: Option<&Path>, reference_samples: u32) {
    let size = [1920, 1080];
    let mut trace = staged(gpu, fogged(false), size);
    let reference = traced(gpu, &mut trace, reference_samples, 101);
    let mut trace = staged(gpu, fogged(false), size);
    let quick = traced(gpu, &mut trace, 64, 7);
    let (wanted, found) = (
        contrast(&profile(&reference.pixels, size)),
        contrast(&profile(&quick.pixels, size)),
    );
    println!(
        "shafts at 1080p: contrast {found:.3} at 64 spp, {wanted:.3} at {reference_samples} spp; noise {:.4} of the band's mean at 64 spp ({:.0} and {:.0} ms of GPU)",
        noise(&quick.pixels, &reference.pixels, size),
        quick.gpu_ms,
        reference.gpu_ms
    );
    if let Some(out) = out {
        std::fs::create_dir_all(out).expect("the out folder");
        write_png(&out.join("shafts-64.png"), size, &quick.pixels);
        write_png(
            &out.join(format!("shafts-{reference_samples}.png")),
            size,
            &reference.pixels,
        );
    }
}

fn cost(gpu: &Gpu, size: [u32; 2], samples: u32, rounds: usize) {
    let mut times = [Vec::new(), Vec::new()];
    let mut on_gpu = true;
    let mut hardware = false;
    for _ in 0..rounds {
        for (slot, street) in [clear(), fogged(false)].into_iter().enumerate() {
            let mut trace = staged(gpu, street, size);
            hardware = trace.hardware_rays();
            let image = traced(gpu, &mut trace, samples, 7);
            on_gpu &= image.on_gpu;
            times[slot].push(image.gpu_ms);
        }
    }
    println!(
        "{}x{}, {samples} spp, {rounds} rounds alternating, hardware rays {hardware}, {} time: fog off {:.0} ms, fog on {:.0} ms of GPU (median)",
        size[0],
        size[1],
        if on_gpu { "GPU timestamp" } else { "fence" },
        median(&mut times[0]),
        median(&mut times[1])
    );
}

fn change(fogged: &[[f32; 3]], plain: &[[f32; 3]]) -> Vec<[f32; 3]> {
    fogged
        .iter()
        .zip(plain)
        .map(|(a, b)| std::array::from_fn(|c| a[c] - b[c]))
        .collect()
}

fn both(gpu: &Gpu, out: &Path, samples: u32) {
    std::fs::create_dir_all(out).expect("the out folder");
    let size = [960, 540];
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
    let mut shots = Vec::new();
    for (name, street) in [
        ("clear", clear()),
        ("fog", fogged(false)),
        (
            "dark-clear",
            Street {
                dark: true,
                ..clear()
            },
        ),
        (
            "dark",
            Street {
                dark: true,
                ..fogged(false)
            },
        ),
    ] {
        let scene = street::scene(&base, &format!("both {name}"), street);
        let live: Vec<[f32; 4]> = street::live(gpu, &scene, size)
            .into_iter()
            .map(|p| [p[0], p[1], p[2], 1.0])
            .collect();
        write_png(&out.join(format!("live-{name}.png")), size, &live);
        let mut trace = staged(gpu, street, size);
        let image = traced(gpu, &mut trace, samples, 7);
        write_png(&out.join(format!("traced-{name}.png")), size, &image.pixels);
        shots.push((street::levels(&live), street::levels(&image.pixels)));
    }
    let lift =
        |change: &[[f32; 3]]| street::mean(change.iter().map(|t| (t[0] + t[1] + t[2]) / 3.0));
    for (name, plain, fogged) in [("the street", 0, 1), ("a black street", 2, 3)] {
        let (live_change, traced_change) = (
            change(&shots[fogged].0, &shots[plain].0),
            change(&shots[fogged].1, &shots[plain].1),
        );
        let blocks = |image: &[[f32; 3]]| street::blocks(image, size, 8);
        println!(
            "{name}: live and traced {:.1} levels apart clear, {:.1} in fog ({:.1} in 8 px blocks); the fog's own change {:.1} levels apart, lifting the frame {:.1} live and {:.1} traced",
            street::apart(&shots[plain].0, &shots[plain].1),
            street::apart(&shots[fogged].0, &shots[fogged].1),
            street::apart(&blocks(&shots[fogged].0), &blocks(&shots[fogged].1)),
            street::apart(&live_change, &traced_change),
            lift(&live_change),
            lift(&traced_change)
        );
    }
}

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        words
            .iter()
            .position(|word| word == name)
            .and_then(|at| words.get(at + 1))
    };
    let number = |name: &str, default: u32| {
        value(name).map_or(default, |value| value.parse().expect("a whole number"))
    };
    let gpu = pollster::block_on(Gpu::headless()).expect("a GPU");
    let out = value("--out").map(Path::new);
    if words.iter().any(|word| word == "--both") {
        both(&gpu, out.expect("--out <folder>"), number("--samples", 128));
    } else if words.iter().any(|word| word == "--look") {
        look(&gpu, out.expect("--out <folder>"), number("--samples", 64));
    } else if words.iter().any(|word| word == "--shafts") {
        shafts(&gpu, out, number("--reference", 2048));
    } else {
        cost(
            &gpu,
            [number("--width", 3840), number("--height", 2160)],
            number("--samples", 16),
            number("--rounds", 5) as usize,
        );
    }
}
