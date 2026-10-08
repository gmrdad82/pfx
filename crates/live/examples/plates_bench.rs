mod manners;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_bake::plate::{PlateRun, bake_plates, parse_plates};
use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Staged, camera as live_camera};
use pfx_load::scene::{Camera as SceneCamera, Scene};

const WARM: u32 = 30;

fn copy(from: &Path, to: &Path) {
    std::fs::copy(from, to).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
}

fn prepare(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "block.gltf",
        "panel.gltf",
        "pillar.gltf",
        "ball.gltf",
        "screen.png",
    ] {
        copy(
            &manifest.join("../load/tests/scenes").join(file),
            &root.join(file),
        );
    }
    copy(
        &manifest.join("../bake/tests/plates/desk.materials.toml"),
        &root.join("desk.materials.toml"),
    );
    copy(
        &manifest.join("../text/fonts/EBGaramond[wght].ttf"),
        &root.join("serif.ttf"),
    );
    let base =
        std::fs::read_to_string(manifest.join("../bake/tests/plates/desk.scene.toml")).unwrap();
    std::fs::write(
        root.join("desk.scene.toml"),
        format!("{base}\n[plates]\ndir = \"plates/main\"\n"),
    )
    .unwrap();
    std::fs::write(root.join("bare.scene.toml"), base).unwrap();
}

fn bake(root: &Path, size: [u32; 2]) -> f64 {
    let recipe = format!(
        "[plates]\nscene = \"desk.scene.toml\"\nsize = [{}, {}]\noverscan = 0.1\nthreshold = 0.01\nmin_samples = 16\nmax_samples = 256\n",
        size[0], size[1]
    );
    let recipe = parse_plates(recipe.as_bytes()).unwrap().unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let mut between = |ms: f64| turns.add(ms);
    let mut log = |line: &str| println!("{line}");
    let started = Instant::now();
    bake_plates(PlateRun {
        gpu: &gpu,
        recipe: &recipe,
        scene: &root.join("desk.scene.toml"),
        anchors: &[12.0, 17.0],
        seed: 7,
        max_samples: None,
        out: &root.join("plates"),
        force: false,
        between: &mut between,
        log: &mut log,
    })
    .unwrap();
    turns.turn();
    started.elapsed().as_secs_f64()
}

fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("plates bench output"),
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

#[derive(Clone, Copy, PartialEq)]
enum Motion {
    Still,
    Drift,
    Moving,
}

fn drifted(base: &SceneCamera, time: f32) -> SceneCamera {
    let yaw = (time * std::f32::consts::TAU / 4.0).sin() * 0.4f32.to_radians();
    let pitch = (time * std::f32::consts::TAU / 5.0).sin() * 0.2f32.to_radians();
    let offset = [
        base.at[0] - base.look_at[0],
        base.at[1] - base.look_at[1],
        base.at[2] - base.look_at[2],
    ];
    let (s, c) = yaw.sin_cos();
    let turned = [
        offset[0] * c + offset[2] * s,
        offset[1],
        -offset[0] * s + offset[2] * c,
    ];
    let mut camera = *base;
    camera.at = [
        base.look_at[0] + turned[0],
        base.look_at[1] + turned[1] + pitch * 2.0,
        base.look_at[2] + turned[2],
    ];
    camera
}

struct Result {
    gpu_p50: f64,
    gpu_p95: f64,
    passes: Vec<(String, f64)>,
}

fn run(
    renderer: &mut Renderer,
    staged: &mut Staged,
    target: &OffscreenTarget,
    motion: Motion,
    frames: u32,
) -> Result {
    let (width, height) = renderer.size();
    let aspect = width as f32 / height as f32;
    let base = staged.camera();
    let mut previous = None;
    let mut gpu = Vec::with_capacity(frames as usize);
    let mut passes: BTreeMap<String, f64> = BTreeMap::new();
    let mut pace = manners::Pace::new();
    for frame in 0..WARM + frames {
        let time = match motion {
            Motion::Moving => frame as f32 / 60.0,
            Motion::Still | Motion::Drift => 0.0,
        };
        let pose = match motion {
            Motion::Still => base,
            Motion::Drift | Motion::Moving => drifted(&base, frame as f32 / 60.0),
        };
        let finish = staged.finish();
        let drawn = staged.frame(aspect, time, frame);
        let mut scene = drawn.scene;
        let mut camera = live_camera(&pose, aspect);
        if let Some(previous) = previous {
            camera.previous_view_projection = previous;
        }
        previous = Some(pfx_live::frame::multiply(camera.projection, camera.view));
        scene.camera = camera;
        scene.time = frame as f32 / 60.0;
        let timings = pace.frame(|| {
            renderer
                .render(&scene, &drawn.text, &drawn.effects, finish, &target.view)
                .unwrap()
        });
        if frame >= WARM {
            if let Some(ms) = manners::Measured::gpu_ms(&timings) {
                gpu.push(ms);
            }
            for timing in timings {
                *passes.entry(timing.label).or_default() += timing.milliseconds / frames as f64;
            }
        }
    }
    gpu.sort_by(f64::total_cmp);
    let at = |q: f64| {
        gpu.get(((gpu.len() as f64 * q) as usize).min(gpu.len().saturating_sub(1)))
            .copied()
            .unwrap_or(f64::NAN)
    };
    let mut passes: Vec<(String, f64)> = passes.into_iter().collect();
    passes.sort_by(|a, b| b.1.total_cmp(&a.1));
    Result {
        gpu_p50: at(0.5),
        gpu_p95: at(0.95),
        passes,
    }
}

fn viewer(scene: &Scene, width: u32, height: u32) -> (Renderer, Staged, OffscreenTarget) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, width, height);
    let mut renderer = Renderer::new(gpu, width, height).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let staged = renderer.stage(scene).unwrap();
    (renderer, staged, target)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str, default: u32| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .map_or(default, |text| text.parse().expect("a number"))
    };
    let width = value("--width", 3840);
    let height = value("--height", 2160);
    let frames = value("--frames", 240);
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/plates-bench")
        .join(format!("{width}x{height}"));
    prepare(&root);
    let seconds = bake(&root, [width, height]);
    println!("plate bake (or check): {seconds:.1} s");
    let plated = Scene::open(root.join("desk.scene.toml")).unwrap();
    let bare = Scene::open(root.join("bare.scene.toml")).unwrap();
    let mut lines = Vec::new();
    {
        let (mut renderer, mut staged, target) = viewer(&bare, width, height);
        for (label, motion) in [
            ("full redraw, still", Motion::Still),
            ("full redraw, drift and moving parts", Motion::Moving),
        ] {
            let result = run(&mut renderer, &mut staged, &target, motion, frames);
            lines.push((label, result));
        }
    }
    let (mut renderer, mut staged, target) = viewer(&plated, width, height);
    assert!(staged.plated(), "the bench scene is plated");
    let info = renderer.plates().unwrap().info();
    for (label, motion) in [
        ("plates, still", Motion::Still),
        ("plates, drift", Motion::Drift),
        ("plates, drift and moving parts", Motion::Moving),
    ] {
        let result = run(&mut renderer, &mut staged, &target, motion, frames);
        lines.push((label, result));
    }
    println!(
        "{width}x{height}: plate {}x{}, {} anchors, {:.1} MB per anchor, {:.1} MB shared",
        info.width,
        info.height,
        info.anchors,
        info.bytes_per_anchor as f64 / 1e6,
        info.shared_bytes as f64 / 1e6
    );
    let mut report = String::new();
    for (label, result) in &lines {
        let top: Vec<String> = result
            .passes
            .iter()
            .take(6)
            .map(|(pass, ms)| format!("{pass} {ms:.2}"))
            .collect();
        let line = format!(
            "{width}x{height} {label}: GPU p50 {:.2} ms, p95 {:.2} ms; {}",
            result.gpu_p50,
            result.gpu_p95,
            top.join(", ")
        );
        println!("{line}");
        report.push_str(&line);
        report.push('\n');
    }
    std::fs::write(root.join("report.txt"), report).unwrap();
}
