#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::Path;

use pfx_gpu::pace::Pace;
use pfx_gpu::screens::{SystemSource, detect};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::flat::{CurvePoint, Draw, FlatScene, Light, ShadowCurve, Srgba};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::stats::FrameStats;
use pfx_load::scene::Scene;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;
const FRAMES: u64 = 6;
const FLAT_FRAMES: u32 = 12;

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

fn board(frame: u32) -> Vec<Draw> {
    (0..24u32)
        .map(|index| {
            let x = 12.0 + ((index * 37 + frame * 11) % 140) as f32;
            let y = 10.0 + ((index * 23 + frame * 7) % 76) as f32;
            Draw::rect([x, y], [14.0, 10.0], 3.0)
                .fill(Srgba::hex(0x3080c0 + index * 0x0a0400))
                .elevation(1.0 + (index % 3) as f32)
        })
        .collect()
}

fn flat_frames(in_flight: u64, path: &Path) -> (Vec<u8>, u64) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = gpu
        .offscreen(WIDTH, HEIGHT, wgpu::TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    let mut renderer = Renderer::new_with_output_format(
        gpu.clone(),
        WIDTH,
        HEIGHT,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    )
    .unwrap();
    renderer.set_frames_in_flight(in_flight);
    renderer.set_frame_stats(FrameStats::to_file(path).unwrap());
    let curve = ShadowCurve::new(
        Srgba::hex(0x000000),
        vec![CurvePoint::new(1.0, 2.0, 3.0, 0.3)],
    )
    .unwrap();
    let mut most = 0;
    let mut pace = Pace::new();
    for frame in 0..FLAT_FRAMES {
        let draws = board(frame);
        let scene = FlatScene {
            layout: [WIDTH as f32, HEIGHT as f32],
            clear: Some(Srgba::hex(0x101820)),
            curve: &curve,
            light: Light::default(),
            draws: &draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame,
            seed: SEED,
        };
        let renderer = &mut renderer;
        pace.frame(|| {
            renderer.render_flat(&scene, &target.view).unwrap();
        });
        most = most.max(renderer.frames_in_flight());
    }
    let pixels = gpu.readback_rgba8(&target).unwrap();
    renderer.frame_stats().finish();
    (pixels, most)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn flat_frames_in_flight_draw_the_same_bytes_wait_for_nothing_and_stay_bounded() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("frame-stats");
    std::fs::create_dir_all(&root).unwrap();
    let blocking = root.join("blocking.jsonl");
    let pipelined = root.join("pipelined.jsonl");
    for path in [&blocking, &pipelined] {
        let _ = std::fs::remove_file(path);
    }
    let (waited, waited_most) = flat_frames(0, &blocking);
    let (piped, piped_most) = flat_frames(2, &pipelined);
    assert_eq!(waited, piped);
    assert_eq!(waited_most, 0);
    assert!(piped_most <= 2, "{piped_most} frames in flight");
    let lines = |path: &Path| -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|line: &serde_json::Value| line.get("tick").is_some())
            .collect()
    };
    let blocking_lines = lines(&blocking);
    let pipelined_lines = lines(&pipelined);
    assert_eq!(blocking_lines.len(), FLAT_FRAMES as usize);
    assert_eq!(pipelined_lines.len(), FLAT_FRAMES as usize);
    for line in &blocking_lines {
        assert_eq!(line["schema_version"], 3);
        assert!(line["wait_ms"].as_f64().is_some(), "{line}");
        assert!(line["prepare_ms"].as_f64().is_some(), "{line}");
        let parts: f64 = ["prepare_ms", "encode_ms", "poll_ms", "queue_ms"]
            .iter()
            .map(|key| line[key].as_f64().unwrap())
            .sum();
        let submit = line["submit_ms"].as_f64().unwrap();
        assert!((parts - submit).abs() < 0.01, "{line}");
    }
    for path in [&blocking, &pipelined] {
        let _ = std::fs::remove_file(path);
    }
}

fn culled_lines(occlusion: bool) -> (Vec<serde_json::Value>, bool) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("frame-stats-culled");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("stats-{occlusion}.jsonl"));
    let _ = std::fs::remove_file(&path);
    let scene = Scene::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes/room.scene.toml"),
    )
    .unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_occlusion(occlusion);
    renderer.set_frame_stats(FrameStats::to_file(&path).unwrap());
    let stats = renderer.frame_stats();
    let supported = renderer.frame().occlusion_supported();
    let mut staged = renderer.stage(&scene).unwrap();
    for frame in 0..FRAMES {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, frame as f32 / 60.0, SEED);
        renderer
            .submit(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &output.view,
            )
            .unwrap();
        renderer.wait_frames(0).unwrap();
    }
    stats.finish();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let lines = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|line: &serde_json::Value| line.get("tick").is_some())
        .collect();
    (lines, supported)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn frame_stats_count_what_the_camera_culled_and_what_occlusion_dropped() {
    let (lines, supported) = culled_lines(true);
    assert_eq!(lines.len() as u64, FRAMES);
    let mut occluded_seen = false;
    for line in &lines {
        assert_eq!(line["schema_version"], 3);
        let instances = line["instances"]
            .as_u64()
            .expect("a 3D frame counts its instances");
        assert!(instances > 0, "{line}");
        let culled = line["culled"].as_u64().unwrap();
        assert!(culled <= instances, "{line}");
        if let Some(occluded) = line["occluded"].as_u64() {
            assert!(occluded <= instances - culled, "{line}");
            occluded_seen = true;
        } else {
            assert!(line["occluded"].is_null());
        }
    }
    assert!(
        lines[0]["occluded"].is_null(),
        "the first tally cannot have landed"
    );
    assert_eq!(occluded_seen, supported);
    let (plain, _) = culled_lines(false);
    for (line, other) in plain.iter().zip(&lines) {
        assert!(line["occluded"].is_null(), "{line}");
        assert_eq!(line["instances"], other["instances"]);
        assert_eq!(line["culled"], other["culled"]);
    }
}
