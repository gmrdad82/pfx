mod manners;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::cull::{CullStats, PassCounts};
use pfx_live::frame::{Matrix, multiply};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    Frustum,
    Occlusion,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Frustum => "frustum",
            Mode::Occlusion => "frustum and occlusion",
        }
    }
}

struct Plan {
    scene: PathBuf,
    size: (u32, u32),
    frames: u32,
    rounds: u32,
    spin: f32,
    modes: Vec<Mode>,
}

fn plan() -> Plan {
    let mut plan = Plan {
        scene: PathBuf::from("samples/chase.pitofx"),
        size: (2560, 1440),
        frames: 180,
        rounds: 2,
        spin: 0.0,
        modes: vec![Mode::Off, Mode::Frustum, Mode::Occlusion],
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let mut value = || words.next().expect("a flag needs a value").clone();
        match word.as_str() {
            "--size" => {
                let text = value();
                let (w, h) = text.split_once('x').expect("--size WxH");
                plan.size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--frames" => plan.frames = value().parse().unwrap(),
            "--rounds" => plan.rounds = value().parse().unwrap(),
            "--spin" => plan.spin = value().parse().unwrap(),
            "--modes" => {
                plan.modes = value()
                    .split(',')
                    .map(|mode| match mode {
                        "off" => Mode::Off,
                        "frustum" => Mode::Frustum,
                        "on" => Mode::Occlusion,
                        other => panic!("unknown mode {other}"),
                    })
                    .collect()
            }
            path => plan.scene = PathBuf::from(path),
        }
    }
    plan
}

fn output(gpu: &Gpu, (width, height): (u32, u32)) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cull cost output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format: wgpu::TextureFormat::Rgba16Float,
        width,
        height,
    }
}

fn yaw(view: Matrix, angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    let turn = [
        [c, 0.0, s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [-s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    multiply(turn, view)
}

fn percentile(values: &mut [f64], at: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() as f64 - 1.0) * at).round() as usize;
    values[index]
}

#[derive(Default)]
struct Tally {
    passes: BTreeMap<String, Vec<f64>>,
    totals: Vec<f64>,
    stats: Option<CullStats>,
    hash: u64,
}

fn run(plan: &Plan, scene: &Scene, mode: Mode, tally: &mut Tally) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, plan.size);
    let mut renderer = Renderer::new(gpu, plan.size.0, plan.size.1).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_culling(mode != Mode::Off);
    renderer.set_occlusion(mode == Mode::Occlusion);
    let mut staged = renderer.stage(scene).unwrap();
    let aspect = plan.size.0 as f32 / plan.size.1 as f32;
    let mut pace = manners::Pace::timed("cull_cost", mode.name(), u64::from(plan.frames));
    let mut previous: Option<Matrix> = None;
    let warm = 30;
    for index in 0..plan.frames + warm {
        let time = index as f32 / 60.0;
        let finish = staged.finish();
        let mut drawn = staged.frame(aspect, time, 1);
        if plan.spin != 0.0 {
            let angle = plan.spin * time;
            drawn.scene.camera.view = yaw(drawn.scene.camera.view, angle);
            let now = multiply(drawn.scene.camera.projection, drawn.scene.camera.view);
            drawn.scene.camera.previous_view_projection = previous.unwrap_or(now);
            previous = Some(now);
        }
        let timings = pace.frame(|| {
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &target.view,
                )
                .unwrap()
        });
        if index < warm {
            continue;
        }
        let mut total = 0.0;
        for timing in &timings {
            tally
                .passes
                .entry(timing.label.to_string())
                .or_default()
                .push(timing.milliseconds);
            total += timing.milliseconds;
        }
        if !timings.is_empty() {
            tally.totals.push(total);
        }
    }
    tally.stats = Some(renderer.cull_stats().clone());
    let pixels = renderer.gpu().readback_rgba16(&target).unwrap();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    pixels.hash(&mut hasher);
    tally.hash = hasher.finish();
}

fn line(name: &str, counts: PassCounts) -> String {
    format!(
        "{name}: {} draws, {} instances, {} triangles",
        counts.draws, counts.instances, counts.triangles
    )
}

fn main() {
    let plan = plan();
    let scene = Scene::open(&plan.scene).unwrap();
    let mut tallies: Vec<Tally> = plan.modes.iter().map(|_| Tally::default()).collect();
    for _ in 0..plan.rounds {
        for (mode, tally) in plan.modes.iter().zip(&mut tallies) {
            run(&plan, &scene, *mode, tally);
        }
    }
    println!(
        "{} at {}x{}, {} frames x {} rounds, spin {}",
        plan.scene.display(),
        plan.size.0,
        plan.size.1,
        plan.frames,
        plan.rounds,
        plan.spin
    );
    for (mode, tally) in plan.modes.iter().zip(&mut tallies) {
        println!("culling {}", mode.name());
        println!(
            "  gpu total p50 {:.3} ms p99 {:.3} ms",
            percentile(&mut tally.totals, 0.5),
            percentile(&mut tally.totals, 0.99)
        );
        for (name, values) in &mut tally.passes {
            println!(
                "  {name:<28} p50 {:>7.3} p99 {:>7.3}",
                percentile(values, 0.5),
                percentile(values, 0.99)
            );
        }
        if let Some(stats) = &tally.stats {
            println!("  instances {}", stats.instances);
            println!("  occlusion tested {}", stats.occlusion_tested);
            println!("  {}", line("camera", stats.camera));
            for (index, counts) in stats.cascades.iter().enumerate() {
                println!("  {}", line(&format!("cascade {}", index + 1), *counts));
            }
            println!("  {}", line("light faces", stats.light_faces));
        }
        println!("  last frame hash {:016x}", tally.hash);
    }
}
