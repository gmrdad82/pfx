use std::path::PathBuf;
use std::time::Instant;

use pfx_gpu::Gpu;
use pfx_gpu::pace::Frozen;

use super::*;
use crate::compare::{Picture, Score, score};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn out_dir() -> PathBuf {
    let dir = crate_dir().join("../../tmp/asset-look");
    std::fs::create_dir_all(dir.join("tracer")).unwrap();
    dir
}

fn args(kind: &str, extra: &[&str]) -> Vec<String> {
    let recipes = crate_dir().join("tests/recipes").display().to_string();
    let marks = crate_dir().join("tests/marks").display().to_string();
    let mut out: Vec<String> = ["pfx"].iter().map(|s| s.to_string()).collect();
    out.extend(kind.split_whitespace().map(str::to_string));
    out.extend(
        ["--recipes", &recipes, "--assets", &marks, "--no-archive"]
            .iter()
            .map(|s| s.to_string()),
    );
    out.extend(extra.iter().map(|s| s.to_string()));
    out
}

pub struct Traced {
    pub picture: Picture,
    pub seconds: f64,
    pub summary: render::Summary,
}

pub fn traced(gpu: &Gpu, command: &str, extra: &[&str], name: &str) -> Traced {
    let cli = Cli::try_parse_from(args(command, extra)).unwrap();
    let (opts, scene) = match &cli.command {
        Kind::Logo { product, opts } => {
            let r = recipes_of(&opts.recipes).unwrap();
            (
                opts.clone(),
                single_scene(opts, &r, product, "logo", None).unwrap(),
            )
        }
        Kind::Wordmark { product, opts } => {
            let r = recipes_of(&opts.recipes).unwrap();
            (
                opts.clone(),
                single_scene(opts, &r, product, "wordmark", None).unwrap(),
            )
        }
        Kind::Lockup { product, opts } => {
            let r = recipes_of(&opts.recipes).unwrap();
            (
                opts.clone(),
                single_scene(opts, &r, product, "lockup", None).unwrap(),
            )
        }
        Kind::Tile { product, opts } => {
            let r = recipes_of(&opts.recipes).unwrap();
            (
                opts.clone(),
                single_scene(opts, &r, product, "tile", None).unwrap(),
            )
        }
        Kind::Card { product, opts } => {
            let r = recipes_of(&opts.recipes).unwrap();
            let card = card_opts(opts, &r, product).unwrap();
            let scene = single_scene(&card, &r, product, "card", None).unwrap();
            (card, scene)
        }
        Kind::Icons {
            product,
            set,
            only,
            opts,
        } => {
            let r = recipes_of(&opts.recipes).unwrap();
            let icon = only.as_deref().unwrap();
            (
                opts.clone(),
                single_scene(opts, &r, product, "icons", Some((set, icon))).unwrap(),
            )
        }
        Kind::Still {
            product,
            shot,
            opts,
        } => {
            let r = recipes_of(&opts.recipes).unwrap();
            (
                opts.clone(),
                shot_scene(opts, &r, product, shot, "still").unwrap(),
            )
        }
        _ => panic!("not a still: {command}"),
    };
    let still = still_job(&opts, &scene).unwrap();
    let dir = out_dir().join("tracer");
    let raw = dir.join(format!("{name}-raw.png"));
    let frozen = Frozen::from_env();
    let held = frozen.ms();
    let started = Instant::now();
    let summary = render::frames(gpu, &still.job, |_, _, frame| {
        render::write_master(&raw, &frame)
    })
    .unwrap();
    let seconds =
        (started.elapsed().as_secs_f64() - (frozen.ms() - held).max(0.0) / 1000.0).max(0.0);
    let delivery = dir.join(format!("{name}.png"));
    finish(&opts, &raw, &delivery, still.size).unwrap();
    std::fs::remove_file(&raw).ok();
    Traced {
        picture: Picture::read(&delivery).unwrap(),
        seconds,
        summary,
    }
}

pub const REFERENCES: &[(&str, &str, &str)] = &[
    ("logo", "logo sample", "--size 512"),
    ("wordmark", "wordmark sample", "--size 512"),
    ("lockup", "lockup sample", "--size 512"),
    ("tile", "tile sample", "--size 512"),
    ("card", "card sample", ""),
    (
        "icon-shapes-dot",
        "icons sample shapes --only dot",
        "--size 512",
    ),
    (
        "icon-shapes-ring",
        "icons sample shapes --only ring",
        "--size 512",
    ),
    ("scene-shapes", "still sample shapes", "--size 512"),
];

fn reference(name: &str) -> Picture {
    let path = crate_dir()
        .join("tests/reference")
        .join(format!("{name}.png"));
    Picture::read(&path).unwrap_or_else(|e| panic!("the frozen reference: {e}"))
}

fn record(name: &str, s: &Score, t: &Traced) {
    let line = json!({
        "name": name,
        "mean": s.mean,
        "p95": s.p95,
        "ssim": s.ssim,
        "seconds": t.seconds,
        "gpu_ms": t.summary.gpu_ms,
        "mean_samples": t.summary.mean_samples,
        "triangles": t.summary.triangles,
    });
    std::fs::write(
        out_dir().join(format!("{name}.json")),
        serde_json::to_vec_pretty(&line).unwrap(),
    )
    .unwrap();
}

pub fn check(name: &str, mean: f64, p95: f64, ssim: f64) {
    let (_, command, extra) = REFERENCES.iter().find(|(n, _, _)| *n == name).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let extra: Vec<&str> = extra.split_whitespace().collect();
    let t = traced(&gpu, command, &extra, name);
    let s = score(&reference(name), &t.picture).unwrap();
    record(name, &s, &t);
    eprintln!(
        "{name}: CIEDE2000 mean {:.2}, p95 {:.2}, SSIM {:.4}; {:.1} s, GPU {:.0} ms, {:.0} samples",
        s.mean, s.p95, s.ssim, t.seconds, t.summary.gpu_ms, t.summary.mean_samples
    );
    assert!(
        s.mean <= mean && s.p95 <= p95 && s.ssim >= ssim,
        "{name}: mean {:.2} (≤ {mean}), p95 {:.2} (≤ {p95}), SSIM {:.4} (≥ {ssim})",
        s.mean,
        s.p95,
        s.ssim
    );
}

pub fn speed(name: &str, draft: bool) -> (f64, f64, f64) {
    let (_, command, extra) = REFERENCES.iter().find(|(n, _, _)| *n == name).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut extra: Vec<&str> = extra.split_whitespace().collect();
    if draft {
        extra.push("--draft");
    }
    let label = format!("{name}-{}", if draft { "draft" } else { "final" });
    let t = traced(&gpu, command, &extra, &label);
    (t.seconds, t.summary.gpu_ms, t.summary.mean_samples)
}

fn mark_luminance(picture: &Picture, mask: &Picture) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for (p, m) in picture.rgba.iter().zip(&mask.rgba) {
        if m[3] == 255 && p[3] == 255 {
            let lin: Vec<f64> = (0..3)
                .map(|c| f64::from(pfx_post::view::srgb_decode(f32::from(p[c]) / 255.0)))
                .collect();
            sum += 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
            count += 1;
        }
    }
    sum / count.max(1) as f64
}

pub fn split(name: &str) -> (f64, f64, f64, f64) {
    let (_, command, extra) = REFERENCES.iter().find(|(n, _, _)| *n == name).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let frozen = reference(name);
    let base: Vec<&str> = extra.split_whitespace().chain(["--draft"]).collect();
    let full = traced(&gpu, command, &base, &format!("{name}-split-full")).picture;
    let mut rig = base.clone();
    rig.extend(["--set", "rig.strength=0.000001"]);
    let rig = traced(&gpu, command, &rig, &format!("{name}-split-rig")).picture;
    let mut room = base.clone();
    room.extend([
        "--set",
        "rig.key=0",
        "--set",
        "rig.fill=0",
        "--set",
        "rig.rim=0",
    ]);
    let room = traced(&gpu, command, &room, &format!("{name}-split-room")).picture;
    (
        mark_luminance(&frozen, &frozen),
        mark_luminance(&full, &frozen),
        mark_luminance(&rig, &frozen),
        mark_luminance(&room, &frozen),
    )
}
