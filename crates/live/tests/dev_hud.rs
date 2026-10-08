#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::{Path, PathBuf};

use pfx_gpu::pace::Pace;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_input::device::Key;
use pfx_input::event::InputEvent;
use pfx_live::hud::{Hud, MARGIN};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_live::stats::FrameStats;
use pfx_load::scene::Scene;

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;
const SEED: u32 = 7;

fn shots() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/dev-hud");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn room() -> Scene {
    Scene::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes/room.scene.toml"))
        .unwrap()
}

fn render(renderer: &mut Renderer, staged: &mut Staged, output: &OffscreenTarget, frame: u32) {
    let stats = renderer.frame_stats();
    let finish = staged.finish();
    stats.sim_begin();
    let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, frame as f32 / 60.0, SEED);
    stats.sim_end();
    renderer
        .render(
            &drawn.scene,
            &drawn.text,
            &drawn.effects,
            finish,
            &output.view,
        )
        .unwrap();
}

fn f3(hud: &mut Hud) {
    for pressed in [true, false] {
        assert!(hud.input(&InputEvent::Key {
            key: Key::F3,
            pressed,
        }));
    }
}

fn inside(rect: [u32; 4], x: u32, y: u32) -> bool {
    x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3]
}

fn near(pixel: &[u8], colour: egui::Color32, tolerance: u8) -> bool {
    [colour.r(), colour.g(), colour.b()]
        .iter()
        .zip(pixel)
        .all(|(want, got)| want.abs_diff(*got) <= tolerance)
}

fn write_png(path: &Path, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_hud_draws_top_right_over_a_neutral_frame_and_f3_toggles_it() {
    let path = shots().join(format!("stats-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = gpu
        .offscreen(WIDTH, HEIGHT, wgpu::TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    let mut renderer =
        Renderer::new_with_output_format(gpu.clone(), WIDTH, HEIGHT, output.format).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_frame_stats(FrameStats::to_file(&path).unwrap());
    let stats = renderer.frame_stats();
    let mut staged = renderer.stage(&room()).unwrap();
    let mut hud = Hud::new();
    let mut pace = Pace::new();
    let mut plain = Vec::new();
    for frame in 0..8 {
        if frame == 4 {
            f3(&mut hud);
            assert!(hud.visible());
        }
        pace.frame(|| {
            render(&mut renderer, &mut staged, &output, frame);
            if frame == 7 {
                plain = gpu.readback_rgba8(&output).unwrap();
            }
            hud.draw(&mut renderer, &output.view);
        });
        assert_eq!(hud.rect().is_some(), frame >= 4);
    }
    let shown = gpu.readback_rgba8(&output).unwrap();
    write_png(&shots().join("hud-over-room.png"), &shown);
    let rect = hud.rect().unwrap();
    let margin = MARGIN as u32;
    assert_eq!(rect[0] + rect[2], WIDTH - margin, "{rect:?}");
    assert_eq!(rect[1], margin);
    assert!(
        rect[2] > 200 && rect[3] > 120 && rect[3] < HEIGHT,
        "{rect:?}"
    );
    let theme = pfx_theme::pfx();
    let (mut changed, mut fill, mut hot, mut text) = (0, 0, 0, 0);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let at = ((y * WIDTH + x) * 4) as usize;
            let (before, after) = (&plain[at..at + 4], &shown[at..at + 4]);
            if !inside(rect, x, y) {
                assert_eq!(before, after, "pixel {x},{y} outside the HUD changed");
                continue;
            }
            changed += usize::from(before != after);
            fill += usize::from(near(after, theme.bg, 1));
            hot += usize::from(near(after, theme.hot, 2));
            text += usize::from(near(after, theme.text, 8));
        }
    }
    let area = (rect[2] * rect[3]) as usize;
    assert!(changed * 2 > area, "{changed} of {area}");
    assert!(fill * 2 > area, "{fill} of {area} in the panel's colour");
    assert!(hot >= 100, "the 16.67 ms line: {hot} pixels");
    assert!(text >= 20, "text: {text} pixels");
    let corner = (((rect[1] + 4) * WIDTH + rect[0] + 4) * 4) as usize;
    assert!(near(&shown[corner..corner + 4], theme.bg, 1));

    f3(&mut hud);
    assert!(!hud.visible());
    pace.frame(|| hud.draw(&mut renderer, &output.view));
    assert!(hud.rect().is_none());
    assert_eq!(gpu.readback_rgba8(&output).unwrap(), shown);

    stats.finish();
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 9);
    for (index, line) in lines[..8].iter().enumerate() {
        let hud_ms = line["hud_ms"].as_f64();
        if index >= 4 {
            assert!(hud_ms.is_some_and(|ms| ms > 0.0), "{line}");
        } else {
            assert!(line["hud_ms"].is_null(), "{line}");
        }
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_hud_writes_linear_colour_into_a_float_frame_and_measures_without_a_file() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu.clone(), WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_frame_stats(FrameStats::off());
    let mut staged = renderer.stage(&room()).unwrap();
    let mut hud = Hud::new();
    hud.set_visible(true);
    let mut pace = Pace::new();
    for frame in 0..3 {
        pace.frame(|| {
            render(&mut renderer, &mut staged, &output, frame);
            hud.draw(&mut renderer, &output.view);
        });
    }
    let stats = renderer.frame_stats();
    assert!(stats.enabled() && !stats.writes());
    let recent = stats.recent();
    assert_eq!(recent.len(), 2);
    assert!(recent.iter().all(|sample| sample.hud_ms.is_some()));
    let rect = hud.rect().unwrap();
    let pixels = gpu.readback_rgba16(&output).unwrap();
    let at = (((rect[1] + 4) * WIDTH + rect[0] + 4) * 4) as usize;
    let linear = half::f16::from_bits(pixels[at]).to_f32();
    let want = ((14.0 / 255.0 + 0.055) / 1.055f32).powf(2.4);
    assert!((linear - want).abs() < 0.0015, "{linear} against {want}");
}
