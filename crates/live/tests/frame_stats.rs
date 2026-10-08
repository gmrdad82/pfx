#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::Path;

use pfx_gpu::Gpu;
use pfx_gpu::pace::Pace;
use pfx_gpu::screens::{SystemSource, detect};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::stats::FrameStats;
use pfx_load::scene::Scene;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;
const FRAMES: u64 = 6;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn frame_stats_carry_a_positive_gpu_time_within_two_frames() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("frame-stats");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("stats.jsonl");
    let _ = std::fs::remove_file(&path);
    let scene = Scene::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes/room.scene.toml"),
    )
    .unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_frame_stats(FrameStats::to_file(&path).unwrap());
    let stats = renderer.frame_stats();
    let mut staged = renderer.stage(&scene).unwrap();
    let mut pace = Pace::new();
    for frame in 0..FRAMES {
        let finish = staged.finish();
        stats.sim_begin();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, frame as f32 / 60.0, SEED);
        stats.sim_end();
        let renderer = &mut renderer;
        let view = &output.view;
        pace.frame(|| {
            renderer
                .submit(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap();
        });
    }
    renderer.wait_frames(0).unwrap();
    stats.finish();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len() as u64, FRAMES + 1);
    let end = lines.last().unwrap();
    assert_eq!(end["end"], true);
    assert_eq!(end["frames"], FRAMES);
    assert_eq!(end["dropped"], 0);
    let mut timed = 0;
    for (index, line) in lines[..FRAMES as usize].iter().enumerate() {
        assert_eq!(line["tick"], index as u64);
        assert_eq!(line["kind"], "full");
        assert_eq!(line["device"], detect(&SystemSource).device.label());
        assert_eq!(line["size"], serde_json::json!([WIDTH, HEIGHT]));
        assert!(line["submit_ms"].as_f64().unwrap() > 0.0);
        assert!(line["sim_ms"].as_f64().is_some());
        if let Some(gpu) = line["gpu_ms"].as_f64() {
            assert!(gpu > 0.0, "{line}");
            let tick = line["gpu_tick"].as_u64().unwrap();
            assert!(tick <= index as u64 && index as u64 - tick <= 2, "{line}");
            timed += 1;
        } else {
            assert!(line["gpu_tick"].is_null());
        }
    }
    assert!(timed > 0, "no line carries a GPU time");
    let _ = std::fs::remove_file(&path);
}
