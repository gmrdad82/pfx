mod args;
mod lens;
mod sets;
mod write;

pub use args::{DEFAULT_FPS, Plan, parse, rate};
pub use write::{CHANNELS, RATE};

use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx::harness::tools;
use pfx_bench::shot::{self as play, Follow, Lens, Played, Quality, Shot};
use pfx_core::rate::Rate;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use args::Quality as Asked;

use crate::progress::{self, Progress};

pub const SCHEMA: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
pub const AUDIO: &str = "audio.wav";

fn scene_name(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || "scene".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let name = name
        .strip_suffix(".pitofx")
        .or_else(|| name.strip_suffix(".toml"))
        .unwrap_or(&name);
    name.strip_suffix(".scene").unwrap_or(name).to_string()
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join(path))
}

fn quality(plan: &Plan) -> Quality {
    match plan.quality {
        Asked::Draft => Quality::Draft,
        Asked::Final => Quality::Final,
    }
}

fn ours(dir: &Path) -> bool {
    std::fs::read(dir.join(MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|manifest| manifest.get("shot").is_some())
}

fn prepare(dir: &Path) -> Result<(), String> {
    if dir.is_file() {
        return Err(format!(
            "{} is a file, not a folder for frames",
            dir.display()
        ));
    }
    if dir.is_dir() {
        pfx_core::atomic::sweep(dir)?;
        let entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?
            .filter_map(Result::ok)
            .collect();
        let frames_only = entries.iter().all(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            write::is_frame_name(&name) || name == AUDIO
        });
        if !frames_only && !ours(dir) {
            return Err(format!(
                "{} holds files pfx shot did not write; name an empty or new --out",
                dir.display()
            ));
        }
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if write::is_frame_name(&name) || name == MANIFEST {
                std::fs::remove_file(entry.path())
                    .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            }
        }
    }
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))
}

fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@".contains(c))
    {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

pub fn fps_value(fps: Rate) -> Value {
    match fps.whole_fps() {
        Some(whole) => json!(whole),
        None => json!(fps.fps()),
    }
}

pub fn pcod(out: &Path, wav: Option<&Path>, fps: Rate) -> String {
    let video = out.with_extension("mp4");
    let video = if video == out {
        out.join("shot.mp4")
    } else {
        video
    };
    let audio = wav.map_or_else(String::new, |wav| format!(" --audio {}", quoted(wav)));
    format!(
        "pcod frames {}{audio} --fps {fps} {}",
        quoted(out),
        quoted(&video)
    )
}

fn spread(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let frames = pfx_gpu::pace::Frames::of(&sorted);
    json!({ "p50": rounded(frames.p50), "p99": rounded(frames.p99) })
}

fn rounded(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(feature = "shot-sound")]
fn scored(script: &Path, frames: u64, fps: Rate) -> Result<Option<Vec<f32>>, String> {
    pfx_sound::shot::render(script, frames as usize, fps).map(Some)
}

#[cfg(feature = "shot-sound")]
pub fn script_wav(script: &Path, frames: u64, fps: Rate, wav: &Path) -> Result<(), String> {
    let samples = scored(script, frames, fps)?.unwrap_or_default();
    if let Some(parent) = wav.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    write::wav(wav, samples)
}

#[cfg(feature = "shot-sound")]
pub fn audio_frames(frames: u64, fps: Rate) -> usize {
    pfx_sound::shot::audio_frames(frames as usize, fps)
}

#[cfg(not(feature = "shot-sound"))]
fn scored(_script: &Path, _frames: u64, _fps: Rate) -> Result<Option<Vec<f32>>, String> {
    Ok(None)
}

pub(crate) fn audio(
    plan: &Plan,
    script: Option<&Path>,
    out: &Path,
) -> Result<(Option<PathBuf>, Value), String> {
    let wanted = plan.audio.is_some() || script.is_some();
    if !wanted {
        return Ok((None, Value::Null));
    }
    let wav = match &plan.audio {
        Some(path) => absolute(path)?,
        None => out.join(AUDIO),
    };
    let length = write::audio_frames(plan.frames, plan.fps) * usize::from(write::CHANNELS);
    let (samples, source, note) = match script {
        Some(script) => match scored(script, plan.frames, plan.fps)? {
            Some(samples) => (
                samples,
                "script",
                "the sound script's offline mix".to_string(),
            ),
            None => (
                vec![0.0; length],
                "silence",
                "silence: this pfx was built without the offline mix (the shot-sound feature)"
                    .to_string(),
            ),
        },
        None => (
            vec![0.0; length],
            "silence",
            "silence: the scene names no sound script and --sound was not given".to_string(),
        ),
    };
    if let Some(parent) = wav.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    write::wav(&wav, samples)?;
    let detail = json!({
        "source": source,
        "script": script.map(|script| script.to_string_lossy().into_owned()),
        "rate": write::RATE,
        "channels": write::CHANNELS,
        "note": note,
    });
    Ok((Some(wav), detail))
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn gigabytes(bytes: Option<u64>) -> Value {
    bytes.map_or(Value::Null, |bytes| {
        json!((bytes as f64 / pfx_core::memory::GB as f64 * 100.0).round() / 100.0)
    })
}

fn memory(played: &Played) -> Value {
    json!({
        "limit_gb": gigabytes(pfx_core::memory::limit()),
        "planned_cpu_gb": gigabytes(Some(played.plan.cpu())),
        "planned_gpu_gb": gigabytes(Some(played.plan.gpu())),
        "cpu_peak_gb": gigabytes(played.peak.cpu),
        "gpu_peak_gb": gigabytes(played.peak.gpu),
    })
}

pub fn run(plan: &Plan) -> Result<(), String> {
    pfx_core::memory::from_env(plan.memory_limit)?;
    if plan.scene.is_dir() {
        return Err(format!(
            "{} is a folder; pfx shot renders one scene file offline and opens no window (a project folder opens with pfx gui)",
            plan.scene.display()
        ));
    }
    if !plan.scene.is_file() {
        return Err(format!("{}: no such scene file", plan.scene.display()));
    }
    let scene_path = absolute(&plan.scene)?;
    let mut sets = Vec::new();
    if let Some(hour) = plan.hour {
        sets.push(format!("sun.hour={hour:?}"));
    }
    sets.extend(plan.sets.iter().cloned());
    let applied = sets::applied(&scene_path, &sets)?;
    let mut warnings = Vec::new();
    let camera = match &plan.camera {
        Some(name) => {
            let found = lens::find(&applied.scene, name);
            if let Some(aspect) = found.as_ref().and_then(|found| found.aspect) {
                let frame = plan.size.0 as f32 / plan.size.1 as f32;
                if (aspect - frame).abs() > 0.01 * frame {
                    warnings.push(format!(
                        "the camera node's aspect ratio {aspect} is not the frame's {frame:.4}; its vertical field of view is kept"
                    ));
                }
            }
            Some(Follow {
                name: name.clone(),
                lens: found.map_or_else(Lens::default, |found| found.lens),
            })
        }
        None => None,
    };
    let script = match &plan.sound {
        Some(path) => Some(absolute(path)?),
        None => applied.sound.clone(),
    };
    if let Some(script) = &script
        && !script.is_file()
    {
        return Err(format!("{}: no such sound script", script.display()));
    }
    let out = match &plan.out {
        Some(out) => absolute(out)?,
        None => tools::caller_tmp()?.join("pfx").join("shot").join(format!(
            "{}-{}",
            scene_name(&scene_path),
            quality(plan).name()
        )),
    };
    prepare(&out)?;

    let started = Instant::now();
    let writers = write::Writers::new(
        plan.format,
        plan.dither
            .then(|| pfx_post::quantise::Dither::seeded(plan.seed)),
    );
    let mut gpu = Vec::new();
    let mut progress = Progress::new("pfx shot", plan.frames, !plan.json && progress::live());
    let shot = Shot {
        scene: &applied.scene,
        size: plan.size,
        fps: plan.fps,
        from: plan.from,
        frames: plan.frames,
        camera,
        quality: quality(plan),
        seed: plan.seed,
        shutter: plan.shutter,
        fixed_exposure: plan.fixed_exposure,
    };
    let played = play::play(&shot, |frame| {
        if let Some(ms) = frame.gpu_ms {
            gpu.push(ms);
        }
        if plan.json {
            println!(
                "{}",
                json!({
                    "frame": frame.index,
                    "of": plan.frames,
                    "number": frame.number,
                    "time": frame.time,
                    "gpu_ms": frame.gpu_ms.map(rounded),
                    "camera": { "at": frame.camera.0, "look_at": frame.camera.1 },
                    "wall_ms": rounded(started.elapsed().as_secs_f64() * 1000.0),
                })
            );
        }
        let index = frame.index;
        writers.write(&out, index, frame.width, frame.height, frame.pixels)?;
        progress.frame(index + 1);
        Ok(())
    });
    let written = writers.finish();
    progress.clear();
    let played = played?;
    written?;
    let (wav, sound) = audio(plan, script.as_deref(), &out)?;
    let wall = started.elapsed().as_secs_f64() * 1000.0;

    let mut fields = Map::new();
    fields.insert("shot".into(), json!(SCHEMA));
    fields.insert("frames".into(), json!(plan.frames));
    fields.insert("from".into(), json!(plan.from));
    fields.insert("first_number".into(), json!(plan.from));
    fields.insert("fps".into(), fps_value(plan.fps));
    fields.insert("frame_rate".into(), json!(plan.fps.to_string()));
    fields.insert("size".into(), json!([plan.size.0, plan.size.1]));
    fields.insert("seconds".into(), json!(plan.fps.time(plan.frames)));
    fields.insert("format".into(), json!(plan.format.name()));
    fields.insert(
        "dither".into(),
        json!(plan.format == args::Format::Png && plan.dither),
    );
    fields.insert(
        "wav".into(),
        json!(wav.as_ref().map(|wav| wav.to_string_lossy().into_owned())),
    );
    fields.insert("audio".into(), sound);
    fields.insert("scene".into(), json!(scene_path.to_string_lossy()));
    fields.insert("scene_sha256".into(), json!(sha256(&scene_path)?));
    fields.insert(
        "rev".into(),
        json!(
            scene_path
                .parent()
                .and_then(|dir| tools::revision_from_dir(dir).ok())
        ),
    );
    fields.insert("pfx".into(), json!(env!("CARGO_PKG_VERSION")));
    fields.insert("quality".into(), json!(quality(plan).name()));
    fields.insert("shadow_resolution".into(), json!(played.shadow_resolution));
    fields.insert("warm_frames".into(), json!(played.warm_frames));
    fields.insert("tick_rate".into(), json!(played.tick_rate));
    fields.insert("shutter".into(), json!(plan.shutter));
    fields.insert(
        "exposure".into(),
        json!(if plan.fixed_exposure { "fixed" } else { "auto" }),
    );
    fields.insert("camera".into(), json!(played.followed));
    fields.insert("hour".into(), json!(plan.hour));
    fields.insert("seed".into(), json!(plan.seed));
    fields.insert("sets".into(), json!(plan.sets));
    fields.insert("gpu_ms".into(), spread(&gpu));
    fields.insert(
        "wall_ms".into(),
        json!({
            "total": rounded(wall),
            "per_frame": rounded(wall / plan.frames.max(1) as f64),
        }),
    );
    fields.insert("out".into(), json!(out.to_string_lossy()));
    fields.insert("pcod".into(), json!(pcod(&out, wav.as_deref(), plan.fps)));
    fields.insert("adapter".into(), json!(played.adapter));
    fields.insert("warnings".into(), json!(warnings));
    fields.insert("memory".into(), memory(&played));
    let manifest = Value::Object(fields);
    let text = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
    pfx_core::atomic::write(&out.join(MANIFEST), format!("{text}\n").as_bytes())?;
    if plan.json {
        println!("{manifest}");
    } else {
        println!(
            "pfx shot: {} frames at {}x{}, {} fps, in {}",
            plan.frames,
            plan.size.0,
            plan.size.1,
            plan.fps.label(),
            out.display()
        );
        println!("{}", pcod(&out, wav.as_deref(), plan.fps));
        eprintln!("pfx shot: {}", played.peak.line(&played.plan));
    }
    Ok(())
}
