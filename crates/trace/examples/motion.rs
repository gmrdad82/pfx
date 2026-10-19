#[path = "../tests/support/spinning.rs"]
mod spinning;

use std::time::Instant;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Timing, Turns};
use pfx_trace::motion::steps_of;
use spinning::Moves;

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn write_png(path: &std::path::Path, size: [u32; 2], bytes: &[u8]) {
    let encode = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0 + 0.5) as u8
    };
    let rgb: Vec<u8> = bytes
        .chunks_exact(16)
        .flat_map(|texel| {
            (0..3).map(move |k| {
                encode(f32::from_le_bytes([
                    texel[4 * k],
                    texel[4 * k + 1],
                    texel[4 * k + 2],
                    texel[4 * k + 3],
                ]))
            })
        })
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

fn proof(gpu: &Gpu, out: &std::path::Path) {
    std::fs::create_dir_all(out).expect("the out folder");
    let size = [1920, 1080];
    let moving = Moves {
        steps: steps_of(1),
        travel: 0.8,
        spin: 1.6,
        pan: 0.0,
        rise: 0.4,
        dense: 0,
    };
    for (name, moves) in [("sharp", Moves::default()), ("blurred", moving)] {
        let mut trace = spinning::stage(gpu, moves, true, size, 64).expect("a stage");
        trace.sample(gpu, 64, 7).expect("samples");
        let bytes = trace.readback_color(gpu).expect("a readback");
        let path = out.join(format!("{name}.png"));
        write_png(&path, size, &bytes);
        println!("{}", path.display());
    }
}

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if let Some(at) = words.iter().position(|word| word == "--out") {
        let gpu = pollster::block_on(Gpu::headless()).expect("a GPU");
        proof(&gpu, std::path::Path::new(&words[at + 1]));
        return;
    }
    let number = |name: &str, default: usize| {
        words
            .iter()
            .position(|word| word == name)
            .and_then(|at| words.get(at + 1))
            .map_or(default, |value| value.parse().expect("a whole number"))
    };
    let size = [
        number("--width", 3840) as u32,
        number("--height", 2160) as u32,
    ];
    let samples = number("--samples", 16) as u32;
    let rounds = number("--rounds", 5);
    let dense = number("--dense", 200_000);
    let settings = [0_u32, 1, 3];
    let gpu = pollster::block_on(Gpu::headless()).expect("a GPU");
    let mut stage_ms = vec![Vec::new(); settings.len()];
    let mut trace_ms = vec![Vec::new(); settings.len()];
    let mut hardware = false;
    let mut timed_on_gpu = true;
    for _ in 0..rounds {
        for (slot, &setting) in settings.iter().enumerate() {
            let steps = steps_of(setting);
            let moves = Moves {
                steps,
                travel: if steps > 0 { 0.8 } else { 0.0 },
                spin: if steps > 0 { 1.6 } else { 0.0 },
                pan: if steps > 0 { 0.2 } else { 0.0 },
                rise: if steps > 0 { 0.4 } else { 0.0 },
                dense,
            };
            let start = Instant::now();
            let mut trace = spinning::stage(&gpu, moves, true, size, samples).expect("a stage");
            stage_ms[slot].push(start.elapsed().as_secs_f64() * 1e3);
            hardware = trace.hardware_rays();
            let mut pacer = Pacer::default();
            let mut turns = Turns::default();
            let stats = trace
                .sample_paced(&gpu, samples, 7, &mut pacer, |ms| turns.add(ms))
                .expect("samples");
            let _ = trace.readback_color(&gpu).expect("a readback");
            timed_on_gpu &= stats.timings.iter().all(|timing| *timing == Timing::Gpu);
            trace_ms[slot].push(stats.milliseconds.iter().sum());
        }
    }
    println!(
        "{}x{}, {samples} spp, {} rounds, a {dense}-triangle ball riding the body, hardware rays {hardware}, {} time",
        size[0],
        size[1],
        rounds,
        if timed_on_gpu {
            "GPU timestamp"
        } else {
            "fence"
        }
    );
    for (slot, &setting) in settings.iter().enumerate() {
        println!(
            "motion steps {setting} ({} poses): stage {:.0} ms wall, trace {:.0} ms of GPU (median)",
            steps_of(setting),
            median(&mut stage_ms[slot]),
            median(&mut trace_ms[slot])
        );
    }
}
