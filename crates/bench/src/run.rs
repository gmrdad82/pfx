use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pfx_gpu::pace::Pace;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_input::Recording;
use pfx_live::renderer::Renderer;
use pfx_live::stats::FrameStats;
use pfx_load::scene::Scene;
use pfx_play::{Options, PlaySession};
use serde_json::{Map, Value, json};

use crate::args::{DEFAULT_SIZE, Plan, parse};
use crate::game::BenchGame;
use crate::summary::{Summary, summarize};

pub const FPS: f64 = 60.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub summary: Summary,
    pub stats: PathBuf,
    pub ticks: u64,
    pub inputs: u64,
    pub size: (u32, u32),
    pub camera: ([f32; 3], [f32; 3]),
    pub line: String,
}

fn rounded(values: [f32; 3]) -> Value {
    json!(values.map(|value| (f64::from(value) * 10_000.0).round() / 10_000.0))
}

fn target(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pfx bench output"),
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

fn scene_name(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || "scene".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let name = name.strip_suffix(".toml").unwrap_or(&name);
    name.strip_suffix(".scene").unwrap_or(name).to_string()
}

fn stats_path(
    plan: &Plan,
    caller_tmp: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    if let Some(path) = &plan.stats {
        return Ok(path.clone());
    }
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    Ok(caller_tmp()?
        .join("pfx")
        .join("bench")
        .join(format!("{}-{seconds}.jsonl", scene_name(&plan.scene))))
}

fn open_recording(path: &Path) -> Result<Recording, String> {
    if !path.is_file() {
        return Err(format!("{}: no such recording", path.display()));
    }
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{}: not a recording: {error}", path.display()))
}

pub fn run_plan(
    plan: &Plan,
    caller_tmp: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<Report, String> {
    if !plan.scene.is_file() {
        return Err(format!("{}: no such scene file", plan.scene.display()));
    }
    let scene =
        Scene::open(&plan.scene).map_err(|error| format!("{}: {error:?}", plan.scene.display()))?;
    if let Some(name) = &plan.camera
        && scene.object(name).is_none()
    {
        return Err(format!(
            "{}: no object named {name:?} to stand the camera at",
            plan.scene.display()
        ));
    }
    let recording = plan.replay.as_deref().map(open_recording).transpose()?;
    let (width, height) = plan.size.unwrap_or(DEFAULT_SIZE);
    let stats_file = stats_path(plan, caller_tmp)?;
    if let Some(parent) = stats_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    File::create(&stats_file).map_err(|error| format!("{}: {error}", stats_file.display()))?;
    let stats = FrameStats::to_file(&stats_file)
        .map_err(|error| format!("{}: {error}", stats_file.display()))?;

    let gpu = pollster::block_on(Gpu::headless())?;
    let output = target(&gpu, width, height);
    let mut renderer = Renderer::new(gpu, width, height).map_err(|error| error.to_string())?;
    renderer.set_frame_stats(stats.clone());
    let staged = renderer.stage(&scene)?;
    let steps = recording.clone();
    let mut session = PlaySession::play_with(
        &scene,
        Some(&staged),
        Box::new(BenchGame::new(plan.camera.clone())),
        Options {
            seed: plan.seed,
            replay: recording,
            ..Options::default()
        },
    )
    .map_err(|error| error.to_string())?;
    session.set_frame_stats(stats.clone());

    let frames = (plan.seconds * FPS).ceil() as u64;
    let aspect = width as f32 / height as f32;
    let seed = plan.seed as u32;
    let finish = staged.finish();
    let mut pace = Pace::new();
    for _ in 0..frames {
        session.advance((1.0 / FPS) as f32);
        let drawn = session
            .present(&staged, &mut renderer, aspect, seed)
            .map_err(|error| error.to_string())?;
        let view = &output.view;
        let renderer = &mut renderer;
        let mut failed = None;
        pace.frame(|| {
            renderer
                .render(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap_or_else(|error| {
                    failed = Some(error);
                    Vec::new()
                })
        });
        if let Some(error) = failed {
            return Err(error);
        }
    }
    let ticks = session.world().ticks();
    let camera = (session.world().camera.at, session.world().camera.look_at);
    let inputs = steps.map_or(0, |recording| {
        recording
            .steps
            .iter()
            .take(ticks as usize)
            .map(|events| events.len() as u64)
            .sum()
    });
    session.stop();
    stats.finish();

    let text = std::fs::read_to_string(&stats_file)
        .map_err(|error| format!("{}: {error}", stats_file.display()))?;
    let summary = summarize(&text)?;
    let mut fields = Map::new();
    fields.insert("scene".into(), json!(plan.scene.to_string_lossy()));
    fields.insert("seed".into(), json!(plan.seed));
    fields.insert("size".into(), json!([width, height]));
    fields.insert("ticks".into(), json!(ticks));
    fields.insert("inputs".into(), json!(inputs));
    fields.insert(
        "camera".into(),
        json!({ "at": rounded(camera.0), "look_at": rounded(camera.1) }),
    );
    fields.insert("stats".into(), json!(stats_file.to_string_lossy()));
    let line = summary.json(fields).to_string();
    Ok(Report {
        summary,
        stats: stats_file,
        ticks,
        inputs,
        size: (width, height),
        camera,
        line,
    })
}

pub fn run(
    args: &[String],
    caller_tmp: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<(), String> {
    let plan = parse(args)?;
    let report = run_plan(&plan, caller_tmp)?;
    println!("{}", report.line);
    Ok(())
}
