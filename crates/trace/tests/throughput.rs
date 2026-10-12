use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::scene::Scene as LoadedScene;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const SAMPLES: u32 = 16;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_desk_runs_every_lane_and_reports_its_throughput() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("throughput");
    std::fs::create_dir_all(&root).unwrap();
    for file in [
        "block.gltf",
        "panel.gltf",
        "pillar.gltf",
        "ball.gltf",
        "screen.png",
    ] {
        std::fs::copy(
            manifest.join("../load/tests/scenes").join(file),
            root.join(file),
        )
        .unwrap();
    }
    for file in ["desk.materials.toml", "desk.scene.toml"] {
        std::fs::copy(
            manifest.join("../bake/tests/plates").join(file),
            root.join(file),
        )
        .unwrap();
    }
    std::fs::copy(
        manifest.join("../text/fonts/EBGaramond[wght].ttf"),
        root.join("serif.ttf"),
    )
    .unwrap();
    let scene = LoadedScene::open(root.join("desk.scene.toml")).unwrap();
    let staged = pfx_trace::stage::scene_at(&scene, WIDTH as f32 / HEIGHT as f32, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    trace
        .sample_paced(&gpu, 2, 1, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let stats = trace
        .sample_paced(&gpu, SAMPLES, 3, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let gpu_ms: f64 = stats.milliseconds.iter().sum();
    let paths = f64::from(WIDTH * HEIGHT * SAMPLES);
    eprintln!(
        "desk, {SAMPLES} spp at {WIDTH}x{HEIGHT}: wall {:.0} ms, gpu {:.0} ms ({:.0}% busy), {} submissions (longest {:.1} ms), lanes {}, {:.2} M paths/s on the gpu, {:.2} M paths/s wall",
        stats.wall_ms,
        gpu_ms,
        100.0 * gpu_ms / stats.wall_ms,
        stats.milliseconds.len(),
        stats.milliseconds.iter().cloned().fold(0.0, f64::max),
        trace.lanes(),
        paths / gpu_ms / 1e3,
        paths / stats.wall_ms / 1e3,
    );
    assert_eq!(trace.lanes(), 64);
}
