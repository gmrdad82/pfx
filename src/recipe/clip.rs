use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use pfx_build::{Built, Registry};
use pfx_glb::DMat4;
use pfx_recipe::{Camera, CameraPath, Rate, Recipe};
use pfx_still::{NOISE, Noise, Step};

use super::checkpoint::{self, Checkpoint, Every};
use super::ledger::{Ledger, Verified};
use super::process::on_path;
use super::{pair, refuse, size};

const SAMPLES: u32 = 64;
const MAX_FPS: u32 = 240;
const MANIFEST: &str = "clip.txt";

type Matrix = [[f32; 4]; 4];

fn turns(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(turns) if turns.is_finite() => Ok(turns),
        _ => Err("is not a finite number of turns".into()),
    }
}

pub(super) fn rate(text: &str) -> Result<Rate, String> {
    Rate::parse_within(text, 1, MAX_FPS)
}

fn video(text: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(text);
    match path.extension().and_then(|e| e.to_str()) {
        Some("mp4" | "webm") => Ok(path),
        _ => Err("ends in neither .mp4 nor .webm".into()),
    }
}

#[derive(clap::Args, Clone, Debug, PartialEq)]
pub(super) struct Args {
    #[arg(value_name = "recipe", help = "The recipe to film")]
    pub(super) recipe: PathBuf,
    #[arg(
        long,
        value_name = "name",
        help = "The recipe's camera, with a path to move along, or still with --spin"
    )]
    camera: String,
    #[arg(
        long,
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..),
        required_unless_present = "timeline",
        conflicts_with = "timeline",
        help = "How many frames"
    )]
    frames: Option<u32>,
    #[arg(
        long,
        value_name = "F",
        value_parser = rate,
        required_unless_present = "timeline",
        conflicts_with = "timeline",
        help = "Frames per second, from 1 to 240: a whole number, num/den such as 24000/1001, or 23.976 (23.98), 29.97, 47.952 (47.95), 59.94, 119.88"
    )]
    fps: Option<Rate>,
    #[arg(
        long,
        help = "Film the recipe's [timeline], frames 0 to its last at its fps, the camera from its rig when it has one"
    )]
    timeline: bool,
    #[arg(
        long,
        value_name = "object",
        conflicts_with = "timeline",
        help = "Turn this object and everything parented to it about the world Y axis through its origin, the camera and lights held still"
    )]
    spin: Option<String>,
    #[arg(
        long = "turns",
        value_name = "N",
        requires = "spin",
        allow_negative_numbers = true,
        value_parser = turns,
        help = "How many turns --spin makes over the clip, negative the other way [default: 1]"
    )]
    turns: Option<f64>,
    #[arg(
        long,
        value_name = "A..B",
        value_parser = pfx_run::jobs::state::range,
        conflicts_with = "film",
        help = "Make only frames A to B of the clip, both kept, numbered, named and seeded as in a whole run, so ranges made apart join in one folder"
    )]
    range: Option<[u32; 2]>,
    #[arg(long, value_name = "WxH", value_parser = size, help = "Each frame's size in pixels")]
    size: [u32; 2],
    #[arg(
        long,
        value_name = "dir",
        help = "The folder for the frames; a second run resumes it"
    )]
    out: PathBuf,
    #[arg(long, help = "Path-trace each frame instead of drawing it live")]
    trace: bool,
    #[arg(
        long,
        value_name = "N",
        requires = "trace",
        value_parser = clap::value_parser!(u32).range(1..),
        help = "The tracer's samples per pixel [default: 64]"
    )]
    spp: Option<u32>,
    #[arg(
        long,
        value_name = "T",
        requires = "trace",
        value_parser = super::render::threshold,
        help = "A pixel stops once its noise is under this, after at least Cycles' automatic floor of samples; 0 runs every pixel to --spp [default: the recipe's trace.threshold, else 0]"
    )]
    threshold: Option<f32>,
    #[arg(
        long,
        value_name = "every",
        requires = "trace",
        value_parser = checkpoint::every,
        help = "Save a traced frame's samples this often, in minutes or as samples like 64spp, so a stopped run resumes inside the frame"
    )]
    checkpoint: Option<Every>,
    #[arg(
        long = "dump-scene",
        value_name = "file.jsonl",
        requires = "trace",
        help = "List every instance each traced frame holds in this file, one JSON line a frame (docs/recipe-clips.md): its piece, object, material, mesh and how many instances share it, its world bounds, its transform's determinant and squash, its sides, cut-out and clip, whether it moves, and its hardware slot"
    )]
    dump_scene: Option<PathBuf>,
    #[arg(
        long,
        value_name = "off|on",
        action = clap::ArgAction::Set,
        hide_possible_values = true,
        requires = "trace",
        value_parser = PossibleValuesParser::new(["off", "on"]).map(|v| v == "on"),
        help = "Denoise the trace [default: on]"
    )]
    denoise: Option<bool>,
    #[arg(
        long,
        value_name = "dir",
        requires = "trace",
        help = "Also write each traced frame's raw beauty, albedo and normal into this folder as linear .exr, the denoiser's inputs"
    )]
    passes: Option<PathBuf>,
    #[arg(
        long,
        help = "Progress on stderr as JSON lines: the frame, of how many, its samples and an ETA"
    )]
    json: bool,
    #[arg(
        long = "rerender-from",
        value_name = "frame",
        help = "Remove this frame and every later one, then make them again"
    )]
    rerender_from: Option<u32>,
    #[arg(
        long,
        value_name = "name",
        help = "Film this variant instead of the recipe"
    )]
    variant: Option<String>,
    #[arg(
        long,
        value_name = "key=value",
        value_parser = pair,
        help = "Replace a value the recipe has; repeatable"
    )]
    set: Vec<String>,
    #[arg(
        long = "video",
        value_name = "file",
        value_parser = video,
        help = "Encode the frames into this .mp4 or .webm with pcod"
    )]
    film: Option<PathBuf>,
    #[command(flatten)]
    picture: super::Picture,
    #[command(flatten)]
    memory: super::Memory,
}

impl Args {
    fn bits(&self) -> u8 {
        if self.picture.wide() { 16 } else { 8 }
    }

    fn denoised(&self) -> bool {
        self.denoise != Some(false)
    }

    fn samples(&self) -> Option<u32> {
        self.trace.then(|| self.spp.unwrap_or(SAMPLES))
    }

    fn count(&self) -> u32 {
        self.frames.unwrap_or(1)
    }

    fn rate(&self) -> Rate {
        self.fps.unwrap_or(Rate::whole(1))
    }

    fn spun(&self) -> f64 {
        self.turns.unwrap_or(1.0)
    }

    fn span(&self) -> Result<[u32; 2], String> {
        let count = self.count();
        match self.range {
            None => Ok([0, count - 1]),
            Some([first, last]) if last < count => Ok([first, last]),
            Some([first, last]) => Err(format!(
                "--range {first}..{last}: the clip has frames 0 to {}",
                count - 1
            )),
        }
    }
}

fn chosen(args: &Args) -> Result<Recipe, String> {
    let file = args.recipe.display().to_string();
    let failed = |e: pfx_recipe::Error| super::read_failure(&file, &e);
    let loaded = pfx_recipe::load(&args.recipe).map_err(failed)?;
    let chosen = match &args.variant {
        Some(name) => pfx_recipe::apply_variant(&loaded.recipe, name).map_err(failed)?,
        None => loaded.recipe,
    };
    pfx_recipe::apply_set(&chosen, &args.set).map_err(failed)
}

fn degrees(turns: f64, frame: u32, frames: u32) -> f64 {
    turns * 360.0 * f64::from(frame) / f64::from(frames.max(1))
}

fn spun(recipe: &Recipe, name: &str) -> Result<usize, String> {
    recipe
        .objects
        .iter()
        .position(|object| object.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = recipe.objects.iter().map(|o| o.name.as_str()).collect();
            if known.is_empty() {
                format!("--spin {name}: the recipe has no objects")
            } else {
                format!(
                    "--spin {name}: no such object, expected one of: {}",
                    known.join(", ")
                )
            }
        })
}

fn motion(recipe: &Recipe, object: usize, degrees: f64) -> Result<DMat4, String> {
    let pivot = pfx_build::world(recipe, object)
        .map_err(|e| e.to_string())?
        .w_axis
        .truncate();
    Ok(DMat4::from_translation(pivot)
        * pfx_build::rotation([0.0, degrees, 0.0])
        * DMat4::from_translation(-pivot))
}

fn narrow(matrix: DMat4) -> Matrix {
    matrix
        .to_cols_array_2d()
        .map(|column| column.map(|v| v as f32))
}

fn held(args: &Args, camera: &Camera) -> Result<(), String> {
    if args.timeline {
        return Ok(());
    }
    match (&args.spin, &camera.path) {
        (Some(object), Some(_)) => Err(format!(
            "camera.{}: --spin {object} holds the camera still, and this camera has a [camera.path]; film the spin through a camera without one",
            camera.name
        )),
        (Some(_), None) => Ok(()),
        (None, _) => path_of(camera).map(|_| ()),
    }
}

fn resolved(args: &Args) -> Result<Args, String> {
    if !args.timeline {
        return Ok(args.clone());
    }
    let file = args.recipe.display().to_string();
    let timeline = chosen(args)?.timeline.ok_or_else(|| {
        format!("{file}: --timeline films the recipe's [timeline], and the recipe has none (docs/recipe-format.md, Timeline)")
    })?;
    Ok(Args {
        frames: Some(timeline.frames + 1),
        fps: Some(timeline.fps),
        ..args.clone()
    })
}

fn name(frame: u32, frames: u32) -> String {
    let digits = (frames.saturating_sub(1)).to_string().len().max(4);
    format!("frame-{frame:0digits$}.png")
}

fn checkpoint_of(dir: &Path, frame: u32, frames: u32) -> PathBuf {
    checkpoint::path_beside(&dir.join(name(frame, frames)))
}

fn manifest(args: &Args) -> String {
    let recipe = std::fs::canonicalize(&args.recipe).unwrap_or_else(|_| args.recipe.clone());
    let mut text = format!(
        "pfx clip\nrecipe {}\ncamera {}\nframes {}\nsize {}x{}\n",
        recipe.display(),
        args.camera,
        args.count(),
        args.size[0],
        args.size[1]
    );
    if args.timeline {
        text.push_str("timeline\n");
    }
    if let Some(object) = &args.spin {
        text.push_str(&format!("spin {object}, {} turns\n", args.spun()));
    }
    match args.samples() {
        Some(samples) => {
            text.push_str(&format!("renderer traced, {samples} spp"));
            if let Some(threshold) = args.threshold {
                text.push_str(&format!(", threshold {threshold}"));
            }
            text.push('\n');
            if !args.denoised() {
                text.push_str("denoise off\n");
            }
            if args.passes.is_some() {
                text.push_str("passes\n");
            }
        }
        None => text.push_str("renderer live\n"),
    }
    if let Some(variant) = &args.variant {
        text.push_str(&format!("variant {variant}\n"));
    }
    for set in &args.set {
        text.push_str(&format!("set {set}\n"));
    }
    if args.picture.wide() {
        text.push_str("png16\n");
    } else if args.picture.dither(0).is_none() {
        text.push_str("dither off\n");
    }
    text
}

fn claim(dir: &Path, wanted: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(MANIFEST);
    match std::fs::read_to_string(&path) {
        Ok(found) if found == wanted => Ok(()),
        Ok(found) => Err(format!(
            "{}: the frames here were made by another line, so they cannot be resumed with this one; use another --out or clear it\nthere:\n{found}here:\n{wanted}",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            pfx_core::atomic::write(&path, wanted.as_bytes())
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn encode_line(frames: &Path, video: &Path, fps: Rate) -> Result<Vec<String>, String> {
    let absolute = |path: &Path| {
        std::path::absolute(path)
            .map(|path| path.display().to_string())
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    Ok(vec![
        absolute(frames)?,
        absolute(video)?,
        "--fps".into(),
        fps.to_string(),
        "--profile".into(),
        "best".into(),
        "--force".into(),
    ])
}

fn precheck(args: &Args) -> Result<(), String> {
    let same = |a: &Path, b: &Path| match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    if args
        .passes
        .as_deref()
        .is_some_and(|passes| same(passes, &args.out))
    {
        return Err(format!(
            "--passes {}: the passes go to a folder of their own, not the frames' --out",
            args.out.display()
        ));
    }
    let (camera, _) = super::precheck(
        &args.recipe,
        args.variant.as_deref(),
        &args.set,
        &args.camera,
        None,
    )?;
    if let Some(name) = &args.spin {
        spun(&chosen(args)?, name)?;
    }
    held(args, &camera)
}

fn requeue(args: &[String]) -> Option<ExitCode> {
    super::process::requeue("clip", args, "traces directly")
}

fn path_of(camera: &Camera) -> Result<&CameraPath, String> {
    camera.path.as_ref().ok_or_else(|| {
        format!(
            "camera.{}: the camera has no path, so every frame would be the same; give it a [camera.path] (docs/recipe-format.md)",
            camera.name
        )
    })
}

enum Renderer {
    Live(
        Box<pfx_recipe_scene::Reel>,
        Box<(Recipe, Built, pfx_recipe_scene::Keyed)>,
    ),
    Traced(Box<pfx_still::Reel>, u32, Option<Noise>, super::Picture),
}

impl Renderer {
    fn recipe(&self) -> &Recipe {
        match self {
            Renderer::Live(_, base) => &base.0,
            Renderer::Traced(reel, ..) => &reel.recipe,
        }
    }

    fn spin(&mut self, object: usize, motion: Matrix) -> Result<(), String> {
        match self {
            Renderer::Live(reel, _) => reel.preview(object, motion).map_err(|e| e.0),
            Renderer::Traced(reel, ..) => reel.spin(object, motion).map_err(|e| e.to_string()),
        }
    }

    fn pose(&mut self, time: f64, before: Option<f64>, name: &str) -> Result<Camera, String> {
        match self {
            Renderer::Live(reel, base) => {
                let (recipe, built, keyed) = &mut **base;
                reel.set_keyed(keyed.sample(built, time));
                if built.skinned() {
                    let now = built.skinned_parts(time);
                    match before {
                        Some(before) => reel.reshape_from(&now, &built.skinned_parts(before)),
                        None => reel.reshape(&now),
                    }
                    .map_err(|e| e.0)?;
                }
                let lit = recipe.keyed.keys().any(|entry| entry.starts_with("light."));
                let recipe =
                    pfx_build::pose_cameras(recipe, built, time).map_err(|e| e.to_string())?;
                if lit {
                    reel.set_lights(&recipe.lights).map_err(|e| e.0)?;
                }
                pfx_recipe_scene::camera(&recipe, name)
                    .cloned()
                    .map_err(|e| e.0)
            }
            Renderer::Traced(reel, ..) => reel.pose_moving(time, name).map_err(|e| e.to_string()),
        }
    }

    fn blurs(&self) -> bool {
        matches!(self, Renderer::Live(reel, _) if reel.blurs())
    }

    fn traced(&self) -> bool {
        matches!(self, Renderer::Traced(..))
    }

    fn traced_instances(&self) -> Option<Vec<pfx_still::TracedPiece>> {
        match self {
            Renderer::Traced(reel, ..) => reel.traced_instances(),
            Renderer::Live(..) => None,
        }
    }

    fn build(&self) -> Option<String> {
        let Renderer::Traced(reel, ..) = self else {
            return None;
        };
        let exe = std::env::current_exe().ok()?;
        let sha = super::ledger::sha256(&exe).ok()?;
        Some(format!("{sha} on {}", reel.adapter()))
    }

    fn settle(&mut self, camera: &Camera) -> Result<f64, String> {
        match self {
            Renderer::Live(reel, _) => {
                let eye = reel.through(camera);
                reel.settle(&eye).map_err(|e| e.0)
            }
            Renderer::Traced(..) => Ok(0.0),
        }
    }

    fn frame(
        &mut self,
        camera: &Camera,
        (size, to): ([u32; 2], &Path),
        settled: Option<f64>,
        step: &mut dyn FnMut(Step),
        keeper: Option<&mut dyn pfx_still::Keeper>,
        passes: Option<&Path>,
    ) -> Result<(String, Option<pfx_still::Paced>), String> {
        match self {
            Renderer::Live(reel, _) => {
                let eye = reel.through(camera);
                let (rgba, gpu_ms) = match settled {
                    Some(before) => reel
                        .develop(&eye)
                        .map(|(rgba, gpu_ms)| (rgba, gpu_ms + before)),
                    None => reel.shoot(&eye),
                }
                .map_err(|e| e.0)?;
                match reel.output().wide {
                    true => pfx_recipe_scene::write_wide(to, size, &rgba),
                    false => pfx_recipe_scene::write_frame(to, size, &rgba),
                }
                .map_err(|e| e.0)?;
                let peak = pfx_core::memory::peak();
                Ok((
                    format!("{gpu_ms:.0} ms on the GPU; {}", peak.line(reel.plan())),
                    None,
                ))
            }
            Renderer::Traced(reel, samples, noise, picture) => {
                let take = pfx_still::Take {
                    size,
                    samples: *samples,
                    noise: *noise,
                    seed: pfx_still::Seed::Locked,
                };
                reel.guides = passes.is_some();
                let rendered = reel
                    .frame_kept(camera, take, step, keeper)
                    .map_err(|e| e.to_string())?;
                if let Some(dir) = passes {
                    let name = to
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("frame");
                    pfx_still::write_passes(dir, name, &rendered.film)?;
                }
                let pixels = pfx_still::finished(
                    &rendered.film,
                    (&rendered.finish, &rendered.overlay),
                    reel.denoise,
                )?;
                let size = rendered.film.size;
                match picture.wide() {
                    true => pfx_recipe_scene::write_wide(
                        to,
                        size,
                        &pfx_post::quantise::sixteen(pixels.len(), 4, |at| {
                            let [r, g, b, _] = pixels[at];
                            [r, g, b, 1.0]
                        }),
                    ),
                    false => pfx_recipe_scene::write_frame(
                        to,
                        size,
                        &pfx_still::dithered_at(
                            size,
                            [0, 0],
                            picture.dither(u64::from(rendered.seed)),
                            &pixels,
                        ),
                    ),
                }
                .map_err(|e| e.0)?;
                let paced = rendered.paced;
                let mut summary = format!(
                    "{} submissions, the longest {:.2} ms, {:.0} ms on the GPU, {} bottom and {} top BVH builds this frame; {}",
                    paced.submissions,
                    paced.longest_ms,
                    paced.gpu_ms,
                    paced.builds.bottom,
                    paced.builds.top,
                    super::render::memory_line(&paced)
                );
                if let Some(restaged) = &rendered.restaged {
                    summary.push_str("; ");
                    summary.push_str(restaged);
                }
                Ok((summary, Some(paced)))
            }
        }
    }
}

fn open(args: &Args, registry: &Registry) -> Result<(Renderer, Camera), String> {
    let (renderer, recipe) = match args.samples() {
        None => {
            let (recipe, built) =
                pfx_recipe_scene::load(&args.recipe, args.variant.as_deref(), &args.set, registry)
                    .map_err(|e| e.0)?;
            let camera = pfx_recipe_scene::camera(&recipe, &args.camera).map_err(|e| e.0)?;
            held(args, camera)?;
            if let Some(name) = &args.spin {
                spun(&recipe, name)?;
            }
            let gpu = pfx_recipe_scene::headless().map_err(|e| e.0)?;
            let mut reel = pfx_recipe_scene::Reel::with_output(
                &gpu,
                &recipe,
                &built.parts,
                args.size,
                args.picture.output(0, crate::recipe::Background::Sky),
            )
            .map_err(|e| e.0)?;
            let fps = recipe.timeline.map_or(Rate::whole(1), |t| t.fps);
            let keyed = pfx_recipe_scene::Keyed::of(&built, fps);
            reel.bind_keys(keyed.names());
            let base = Box::new((recipe.clone(), built, keyed));
            (Renderer::Live(Box::new(reel), base), recipe)
        }
        Some(samples) => {
            let mut reel = pfx_still::Reel::open_for(
                &args.recipe,
                args.variant.as_deref(),
                &args.set,
                None,
                registry,
                args.size,
            )
            .map_err(|e| e.to_string())?;
            reel.denoise = args.denoised();
            let recipe = reel.recipe.clone();
            let noise = noise(args, &recipe.trace, samples);
            (
                Renderer::Traced(Box::new(reel), samples, noise, args.picture),
                recipe,
            )
        }
    };
    let camera = pfx_recipe_scene::camera(&recipe, &args.camera).map_err(|e| e.0)?;
    held(args, camera)?;
    Ok((renderer, camera.clone()))
}

fn noise(args: &Args, trace: &pfx_recipe::Trace, samples: u32) -> Option<Noise> {
    let threshold = args
        .threshold
        .or(trace.threshold.map(|t| t as f32))
        .unwrap_or(0.0);
    (threshold > 0.0 && samples >= 2).then(|| {
        Noise {
            threshold,
            min_samples: trace.min_samples.unwrap_or(NOISE.min_samples),
        }
        .floored()
    })
}

struct Told {
    json: bool,
    began: Instant,
    of: u32,
    todo: u32,
    made: u32,
    last: Option<Instant>,
    path: Option<pfx_still::TracePath>,
}

impl Told {
    fn new(json: bool, of: u32, todo: u32) -> Told {
        Told {
            json,
            began: Instant::now(),
            of,
            todo,
            made: 0,
            last: None,
            path: None,
        }
    }

    fn eta(&self, fraction: f64) -> serde_json::Value {
        let progress = f64::from(self.made) + fraction;
        let elapsed = self.began.elapsed().as_secs_f64();
        if progress <= 0.0 {
            return serde_json::Value::Null;
        }
        let left = (f64::from(self.todo) - progress).max(0.0);
        super::render::rounded(elapsed / progress * left, 1)
    }

    fn line<const N: usize>(&self, stage: &str, fields: [(&str, serde_json::Value); N]) {
        eprintln!(
            "{}",
            super::render::json_line(stage, self.began.elapsed().as_secs_f64(), fields)
        );
    }

    fn step(&mut self, frame: u32, step: Step, pixels: u64) {
        if let Step::Path(path) = step {
            self.traced_on(frame, *path);
            return;
        }
        if !self.json {
            return;
        }
        let (samples, spp, fraction) = match step {
            Step::Trace { done, total } => (
                f64::from(done),
                total,
                f64::from(done) / f64::from(total.max(1)),
            ),
            Step::Converge {
                cap, done, total, ..
            } => (
                done as f64 / pixels.max(1) as f64,
                cap,
                if total == 0 {
                    0.0
                } else {
                    (done as f64 / total as f64).min(1.0)
                },
            ),
            _ => return,
        };
        let now = Instant::now();
        if self
            .last
            .is_some_and(|last| now < last + std::time::Duration::from_secs(1))
        {
            return;
        }
        self.last = Some(now);
        self.line(
            "trace",
            [
                ("frame", frame.into()),
                ("of", self.of.into()),
                ("samples", (samples.round() as u64).into()),
                ("spp", spp.into()),
                ("fraction", super::render::rounded(fraction, 4)),
                ("eta_s", self.eta(fraction)),
            ],
        );
    }

    fn traced_on(&mut self, frame: u32, path: pfx_still::TracePath) {
        if self.path.as_ref() == Some(&path) {
            return;
        }
        if self.json {
            let mut fields = super::render::path_json(&path);
            fields["line"] = path.line().into();
            self.line("path", [("frame", frame.into()), ("path", fields)]);
        } else {
            eprintln!("pfx clip: tracing on {}", path.line());
        }
        self.path = Some(path);
    }

    fn frame(&mut self, frame: u32, done: u32, to: &Path, cost: Option<&pfx_still::Paced>) {
        self.made += 1;
        self.last = None;
        if !self.json {
            return;
        }
        let fields = [
            ("frame", frame.into()),
            ("of", self.of.into()),
            ("done", done.into()),
            ("out", to.display().to_string().into()),
            ("eta_s", self.eta(0.0)),
        ];
        match cost {
            None => self.line("frame", fields),
            Some(paced) => {
                let [a, b, c, d, e] = fields;
                self.line(
                    "frame",
                    [
                        a,
                        b,
                        c,
                        d,
                        e,
                        ("submissions", paced.submissions.into()),
                        ("longest_ms", super::render::rounded(paced.longest_ms, 2)),
                        ("gpu_ms", super::render::rounded(paced.gpu_ms, 0)),
                        ("bottom_builds", paced.builds.bottom.into()),
                        ("top_builds", paced.builds.top.into()),
                    ],
                )
            }
        }
    }
}

fn resuming(done: u32, count: u32, first: u32, samples: Option<u32>) -> String {
    let inside = samples.map_or(String::new(), |samples| {
        format!(", from its checkpoint at {samples} samples")
    });
    format!("resuming: {done} of {count} frames done, starting at frame {first}{inside}")
}

fn plural(count: usize, one: &str) -> String {
    format!("{count} {one}{}", if count == 1 { "" } else { "s" })
}

fn verified_line(found: &Verified) -> String {
    let mut line = format!(
        "{} verified by their sha256",
        plural(found.verified.len() - found.adopted.len(), "frame")
    );
    if !found.adopted.is_empty() {
        line.push_str(&format!(
            ", {} without one decoded whole and recorded",
            plural(found.adopted.len(), "frame")
        ));
    }
    if found.redone.is_empty() {
        line.push_str(", none redone");
    } else {
        let redone: Vec<String> = found
            .redone
            .iter()
            .map(|(frame, why)| format!("frame {frame} {why}"))
            .collect();
        line.push_str(&format!(
            "; redoing {}: {}",
            plural(found.redone.len(), "frame"),
            redone.join(", ")
        ));
    }
    line
}

fn differs(frame: u32, out: &Path) -> String {
    format!(
        "frame {frame} rendered again differs from the one in {}: this pfx build, the GPU driver or state carried between frames now makes other bytes, so nothing more is rendered and old and new frames never mix; --rerender-from {frame} makes it and every later frame again",
        out.display()
    )
}

fn checked_path(dir: &Path, frame: u32, frames: u32) -> PathBuf {
    let name = name(frame, frames);
    dir.join(format!(
        ".{}.recheck.partial.png",
        name.trim_end_matches(".png")
    ))
}

fn rerender(args: &Args, ledger: &mut Ledger, last: u32) -> Result<(), String> {
    let count = args.count();
    let Some(from) = args.rerender_from else {
        return Ok(());
    };
    if from >= count {
        return Err(format!(
            "--rerender-from {from}: the clip's frames run from 0 to {}",
            count - 1
        ));
    }
    for frame in from..=last {
        for path in [
            args.out.join(name(frame, count)),
            checkpoint_of(&args.out, frame, count),
        ] {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    ledger.forget([from, last]);
    Ok(())
}

fn frame_in(name: &str) -> Option<u32> {
    let rest = &name[name.find("frame-")? + "frame-".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn frames(args: &Args, registry: &Registry, progress: &mut dyn FnMut(u32)) -> Result<(), String> {
    let [first, last] = args.span()?;
    let line = manifest(args);
    claim(&args.out, &line)?;
    let count = args.count();
    let ours = |name: &str| frame_in(name).is_none_or(|frame| (first..=last).contains(&frame));
    let (swept, found, before) = Ledger::update(&args.out, |ledger| {
        let mut swept = pfx_core::atomic::sweep_matching(&args.out, ours)?;
        if let Some(passes) = &args.passes {
            swept.extend(pfx_core::atomic::sweep_matching(passes, ours)?);
        }
        rerender(args, ledger, last)?;
        let found = ledger.verify(
            &args.out,
            |frame| name(frame, count),
            ([first, last], count, args.size, args.bits()),
        )?;
        let before = found
            .todo
            .first()
            .and_then(|frame| frame.checked_sub(1))
            .filter(|before| !found.todo.contains(before))
            .filter(|&before| {
                before >= first
                    || ledger.holds(&args.out.join(name(before, count)), before, args.size)
            });
        Ok((swept, found, before))
    })?;
    let todo = found.todo.clone();
    for frame in (first..=last).filter(|frame| !todo.contains(frame)) {
        let stale = checkpoint_of(&args.out, frame, count);
        if stale.exists() {
            std::fs::remove_file(&stale).map_err(|e| format!("{}: {e}", stale.display()))?;
        }
    }
    let made = last - first + 1;
    let done = made - todo.len() as u32;
    let part = match args.range {
        Some(_) => format!(", frames {first} to {last} of {count}"),
        None => String::new(),
    };
    eprintln!(
        "pfx clip: {} of {made} frames to make, {done} already finished in {}{part}",
        todo.len(),
        args.out.display()
    );
    if !swept.is_empty() {
        eprintln!(
            "pfx clip: removed {} a stopped run left",
            plural(swept.len(), "unfinished file")
        );
    }
    let check = before.filter(|_| args.rerender_from.is_none());
    let jobs: Vec<(u32, bool)> = check
        .map(|frame| (frame, true))
        .into_iter()
        .chain(todo.iter().map(|&frame| (frame, false)))
        .collect();
    let mut told = Told::new(args.json, count, jobs.len() as u32);
    let key = |frame: u32| checkpoint::key(&format!("{line}frame {frame}\n"), &args.recipe);
    if let Some(&next) = todo.first() {
        let mut quiet = |_: &Path, _: u32| {};
        let found_samples = args.samples().and_then(|cap| {
            Checkpoint::open(
                checkpoint_of(&args.out, next, count),
                key(next),
                (None, cap),
                &mut quiet,
            )
            .found()
        });
        if done > 0 || !found.redone.is_empty() || found_samples.is_some() {
            eprintln!("pfx clip: {}", resuming(done, made, next, found_samples));
            eprintln!("pfx clip: {}", verified_line(&found));
            match (check, args.rerender_from) {
                (Some(before), _) => eprintln!(
                    "pfx clip: frame {before} renders again first and must match its stored bytes"
                ),
                (None, Some(from)) => eprintln!(
                    "pfx clip: frames from {from} are made again, without checking frame {}",
                    from.saturating_sub(1)
                ),
                (None, None) => {}
            }
            if told.json {
                let redone: Vec<serde_json::Value> = found
                    .redone
                    .iter()
                    .map(|(frame, why)| serde_json::json!({"frame": frame, "why": why}))
                    .collect();
                told.line(
                    "resume",
                    [
                        ("done", done.into()),
                        ("of", made.into()),
                        ("from", next.into()),
                        ("checkpoint_samples", found_samples.into()),
                        ("verified", found.verified.clone().into()),
                        ("adopted", found.adopted.clone().into()),
                        ("redone", redone.into()),
                        ("check", check.into()),
                        ("rerender_from", args.rerender_from.into()),
                    ],
                );
            }
        }
    }
    progress(done);
    let mut dump = match &args.dump_scene {
        Some(path) => {
            if done > 0 {
                eprintln!(
                    "pfx clip: {} lists only the frames this run renders; the {} already finished in {} are left out, so use a fresh --out to list them all",
                    path.display(),
                    plural(done as usize, "frame"),
                    args.out.display()
                );
            }
            Some(Dump::create(path)?)
        }
        None => None,
    };
    if todo.is_empty() {
        return Ok(());
    }
    let (mut renderer, camera) = open(args, registry)?;
    let build = renderer.build();
    let proven = match (check, build.as_deref()) {
        (Some(before), Some(build)) => Ledger::read(&args.out)?
            .proven(build)
            .map(|by| (before, by)),
        _ => None,
    };
    if let Some((before, by)) = proven {
        eprintln!(
            "pfx clip: frame {before} is not rendered again: this pfx build on this GPU and driver already rendered frame {by} again to its stored bytes"
        );
        if told.json {
            told.line(
                "check",
                [
                    ("frame", before.into()),
                    ("result", "proven".into()),
                    ("by", by.into()),
                ],
            );
        }
        told.todo -= 1;
    }
    let jobs: Vec<(u32, bool)> = jobs
        .into_iter()
        .filter(|&(_, checking)| !checking || proven.is_none())
        .collect();
    let spin = match &args.spin {
        Some(name) => Some(spun(renderer.recipe(), name)?),
        None => None,
    };
    let path = if args.timeline || spin.is_some() {
        None
    } else {
        Some(path_of(&camera)?)
    };
    let mut left_out = pfx_recipe_scene::unapplied(renderer.recipe(), renderer.traced());
    if renderer.traced() && path.is_none() && spin.is_none() {
        left_out.retain(|key| *key != "motion_blur");
    }
    if let Some(line) = pfx_recipe_scene::left_out_line("clip", &left_out, renderer.traced()) {
        eprintln!("{line}");
    }
    let place = |renderer: &mut Renderer, frame: u32| -> Result<Camera, String> {
        Ok(match (spin, path) {
            (Some(object), _) => {
                let turned = motion(
                    renderer.recipe(),
                    object,
                    degrees(args.spun(), frame, count),
                )?;
                renderer.spin(object, narrow(turned))?;
                camera.clone()
            }
            (None, Some(path)) => camera.at(path.time(frame, count)),
            (None, None) => {
                let at = |frame: u32| args.rate().time(u64::from(frame));
                let before = frame.checked_sub(1).filter(|_| renderer.blurs()).map(at);
                renderer.pose(at(frame), before, &args.camera)?
            }
        })
    };
    let pixels = u64::from(args.size[0]) * u64::from(args.size[1]);
    let json = args.json;
    let mut written = 0;
    for &(frame, checking) in &jobs {
        let settled = match frame.checked_sub(1).filter(|_| renderer.blurs()) {
            Some(prior) => {
                let before = place(&mut renderer, prior)?;
                Some(renderer.settle(&before)?)
            }
            None => None,
        };
        let pose = place(&mut renderer, frame)?;
        let stored = args.out.join(name(frame, count));
        let to = if checking {
            checked_path(&args.out, frame, count)
        } else {
            stored.clone()
        };
        let mut tell = |path: &Path, samples: u32| {
            if json {
                eprintln!(
                    "{}",
                    super::render::json_line(
                        "checkpoint",
                        0.0,
                        [
                            ("checkpoint", path.display().to_string().into()),
                            ("frame", frame.into()),
                            ("at_samples", samples.into()),
                        ]
                    )
                );
            } else {
                eprintln!(
                    "pfx clip: checkpoint {} at {samples} samples",
                    path.display()
                );
            }
        };
        let mut kept = args
            .samples()
            .filter(|_| !checking)
            .map(|cap| {
                Checkpoint::open(
                    checkpoint_of(&args.out, frame, count),
                    key(frame),
                    (args.checkpoint, cap),
                    &mut tell,
                )
            })
            .filter(|kept| args.checkpoint.is_some() || kept.found().is_some());
        if let Some(samples) = kept
            .as_ref()
            .and_then(Checkpoint::found)
            .filter(|_| todo.first() != Some(&frame))
        {
            eprintln!("pfx clip: frame {frame} resumes from its checkpoint at {samples} samples");
        }
        let (summary, cost) = renderer.frame(
            &pose,
            (args.size, &to),
            settled,
            &mut |step| told.step(frame, step, pixels),
            kept.as_mut().map(|kept| kept as &mut dyn pfx_still::Keeper),
            args.passes.as_deref().filter(|_| !checking),
        )?;
        if let Some(dump) = &mut dump {
            let pieces = renderer.traced_instances().ok_or_else(|| {
                format!("frame {frame} was traced but its trace lists no instances")
            })?;
            dump.frame(args, listed(frame, &pieces))?;
        }
        if checking {
            let again = std::fs::read(&to).map_err(|e| format!("{}: {e}", to.display()));
            let _ = std::fs::remove_file(&to);
            let same = again?
                == std::fs::read(&stored).map_err(|e| format!("{}: {e}", stored.display()))?;
            Ledger::update(&args.out, |ledger| {
                ledger.checked(frame, same, build.clone());
                Ok(())
            })?;
            if json {
                told.line(
                    "check",
                    [
                        ("frame", frame.into()),
                        ("result", if same { "same" } else { "differs" }.into()),
                    ],
                );
            }
            if !same {
                return Err(differs(frame, &args.out));
            }
            eprintln!(
                "pfx clip: frame {frame} rendered again to its stored bytes; carrying on at frame {}",
                frame + 1
            );
            told.made += 1;
            continue;
        }
        if let Some(kept) = &kept {
            kept.clear()?;
        }
        Ledger::update(&args.out, |ledger| ledger.record(frame, args.size, &stored))?;
        written += 1;
        eprintln!("pfx clip: {}, {summary}", stored.display());
        println!("{}", stored.display());
        told.frame(frame, done + written, &stored, cost.as_ref());
        progress(done + written);
    }
    Ok(())
}

struct Dump {
    path: PathBuf,
    file: std::io::BufWriter<std::fs::File>,
}

impl Dump {
    fn create(path: &Path) -> Result<Self, String> {
        let failed = |e: std::io::Error| format!("{}: {e}", path.display());
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            file: std::io::BufWriter::new(std::fs::File::create(path).map_err(failed)?),
        })
    }

    fn frame(&mut self, args: &Args, mut line: serde_json::Value) -> Result<(), String> {
        use std::io::Write;
        line["recipe"] = serde_json::json!(args.recipe.display().to_string());
        line["camera"] = serde_json::json!(args.camera);
        let failed = |e: std::io::Error| format!("{}: {e}", self.path.display());
        serde_json::to_writer(&mut self.file, &line).map_err(|e| failed(e.into()))?;
        self.file.write_all(b"\n").map_err(failed)?;
        self.file.flush().map_err(failed)
    }
}

fn listed(frame: u32, pieces: &[pfx_still::TracedPiece]) -> serde_json::Value {
    use serde_json::json;
    let mut meshes = std::collections::BTreeMap::new();
    for traced in pieces.iter().map(|piece| &piece.traced) {
        if let Some(mesh) = traced.mesh {
            meshes.insert(mesh, (traced.triangles, traced.instanced));
        }
    }
    let list: Vec<serde_json::Value> = pieces
        .iter()
        .map(|piece| {
            let traced = &piece.traced;
            let mut fields = json!({
                "index": traced.index,
                "name": piece.name,
                "object": piece.object,
                "material": piece.material,
                "traced": traced.mesh.is_some(),
                "mesh": traced.mesh,
                "shared": piece.shared,
                "triangles": traced.triangles,
                "instanced": traced.instanced,
                "moving": traced.moving,
                "deforming": traced.deforming,
                "sprite": traced.sprite,
            });
            if let Some(two_sided) = piece.two_sided {
                fields["two_sided"] = json!(two_sided);
            }
            if let Some(look) = &piece.look {
                fields["base"] = json!(look.base);
                fields["alpha"] = json!(look.alpha);
                fields["alpha_mode"] = json!(look.alpha_mode);
                fields["alpha_cutoff"] = json!(look.alpha_cutoff);
                fields["transmission"] = json!(look.transmission);
            }
            if traced.mesh.is_some() {
                fields["one_sided"] = json!(traced.one_sided);
                fields["shadow_only"] = json!(traced.shadow_only);
                fields["casts_shadow"] = json!(traced.casts_shadow);
                fields["cutout"] = json!(traced.cutout);
                fields["clipped"] = json!(traced.clipped);
                fields["cutoff"] = json!(traced.alpha_cutoff);
            }
            if let Some(bounds) = traced.bounds {
                fields["bounds"] = json!({ "lo": bounds.lo, "hi": bounds.hi });
            }
            if let (Some(det), Some(squash)) = (traced.determinant(), traced.squash()) {
                fields["determinant"] = json!(det);
                fields["squash"] = super::render::rounded(squash, 6);
            }
            if let Some(listed) = traced.hardware {
                fields["tlas_mask"] = json!(listed.mask);
                fields["tlas_custom"] = json!(listed.custom);
                fields["tlas_in_place"] = json!(
                    listed.rows.is_some()
                        && listed.rows == traced.rows
                        && listed.custom == traced.record
                );
                fields["tlas_opaque"] = json!(listed.opaque);
                fields["hardware_mover"] = json!(listed.mover);
            }
            fields
        })
        .collect();
    json!({
        "frame": frame,
        "instances": list.len(),
        "traced": pieces.iter().filter(|piece| piece.traced.mesh.is_some()).count(),
        "meshes": meshes
            .iter()
            .map(|(mesh, (triangles, instances))| json!({
                "mesh": mesh,
                "triangles": triangles,
                "instances": instances,
            }))
            .collect::<Vec<_>>(),
        "list": list,
    })
}

fn unencoded(args: &Args, video: &Path) -> String {
    format!(
        "{} needs pcod (pito-codecs) on the PATH for --video; the frames are kept in {}",
        video.display(),
        args.out.display()
    )
}

fn encode(args: &Args, video: &Path, pcod: bool) -> Result<(), String> {
    if !pcod {
        return Err(unencoded(args, video));
    }
    pfx_core::atomic::write_with(video, |partial| {
        let line = encode_line(&args.out, partial, args.rate())?;
        eprintln!("pfx clip: pcod {}", line.join(" "));
        let status = Command::new("pcod")
            .args(&line)
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("cannot start pcod: {e}"))?;
        if !status.success() {
            return Err(format!("pcod failed ({status}) making {}", video.display()));
        }
        Ok(())
    })?;
    println!("{}", video.display());
    Ok(())
}

pub fn run(parsed: &Args, args: &[String], registry: &Registry) -> ExitCode {
    let parsed = &match resolved(parsed) {
        Ok(parsed) => parsed,
        Err(message) => return refuse("clip", &message),
    };
    if let Err(message) = parsed.memory.set() {
        return refuse("clip", &message);
    }
    if parsed.samples().is_some() {
        if let Err(message) = precheck(parsed) {
            return refuse("clip", &message);
        }
        if let Some(code) = requeue(args) {
            return code;
        }
    }
    let made = frames(parsed, registry, &mut |_| {}).and_then(|()| match &parsed.film {
        Some(video) => encode(parsed, video, on_path("pcod")),
        None => Ok(()),
    });
    match made {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => refuse("clip", &error),
    }
}

#[cfg(test)]
mod tests {
    use super::super::process::{self, Machine};
    use super::*;

    fn line(exe: &str, args: &[String], machine: Machine<'_>) -> Option<Vec<String>> {
        process::line("clip", exe, args, machine)
    }

    fn strings(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    fn parse(words: &[String]) -> Result<Args, String> {
        let words: Vec<&str> = std::iter::once("clip")
            .chain(words.iter().map(String::as_str))
            .collect();
        match super::super::parsed(&words)? {
            super::super::Verb::Clip(args) => Ok(args),
            _ => Err("not clip".into()),
        }
    }

    fn pending(dir: &Path, frames: u32, size: [u32; 2], bits: u8) -> Vec<u32> {
        Ledger::default()
            .verify(
                dir,
                |frame| name(frame, frames),
                ([0, frames.saturating_sub(1)], frames, size, bits),
            )
            .unwrap()
            .todo
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp/clip-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn without_an_encoder_the_frames_stay_and_the_way_on_is_named() {
        let dir = scratch("no-encoder");
        let mut args = parse(&strings(
            "a.toml --camera main --frames 2 --fps 24 --size 8x8 --out d",
        ))
        .unwrap();
        args.out = dir.clone();
        let video = dir.join("c.mp4");
        let error = encode(&args, &video, false).unwrap_err();
        assert_eq!(error, unencoded(&args, &video));
        assert!(error.contains("needs pcod (pito-codecs)"));
        assert!(!error.contains("ffmpeg"));
        assert!(!video.exists());
    }

    #[test]
    fn a_range_makes_part_of_the_clip_in_the_same_folder_a_whole_run_uses() {
        let line = "a.toml --camera main --frames 8 --fps 24 --size 8x8 --out d";
        let whole = parse(&strings(line)).unwrap();
        let part = parse(&strings(&format!("{line} --range 2..5"))).unwrap();
        assert_eq!(part.range, Some([2, 5]));
        assert_eq!(whole.span(), Ok([0, 7]));
        assert_eq!(part.span(), Ok([2, 5]));
        assert_eq!(manifest(&part), manifest(&whole));
        let over = parse(&strings(&format!("{line} --range 6..8"))).unwrap();
        assert_eq!(
            over.span(),
            Err("--range 6..8: the clip has frames 0 to 7".into())
        );
        for bad in ["--range 5..2", "--range 3", "--range 2..5 --video c.mp4"] {
            assert!(parse(&strings(&format!("{line} {bad}"))).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_flag_is_read() {
        let args = parse(&strings(
            "table.toml --camera main --frames 24 --fps 30 --spin body --turns -0.5 --size 320x200 --out tmp/c --trace --spp 16 --threshold 0.02 --checkpoint 30 --dump-scene tmp/c-scene.jsonl --json --rerender-from 3 --variant bare --set sky.intensity=0.4 --video tmp/c.webm --format png16 --memory-limit 6 --denoise off --passes tmp/p",
        ))
        .unwrap();
        assert_eq!(
            args,
            Args {
                recipe: "table.toml".into(),
                camera: "main".into(),
                frames: Some(24),
                fps: Some(Rate::whole(30)),
                timeline: false,
                spin: Some("body".into()),
                turns: Some(-0.5),
                range: None,
                size: [320, 200],
                out: "tmp/c".into(),
                trace: true,
                spp: Some(16),
                threshold: Some(0.02),
                checkpoint: Some(Every::Minutes(30.0)),
                denoise: Some(false),
                passes: Some("tmp/p".into()),
                dump_scene: Some("tmp/c-scene.jsonl".into()),
                json: true,
                rerender_from: Some(3),
                variant: Some("bare".into()),
                set: vec!["sky.intensity=0.4".into()],
                film: Some("tmp/c.webm".into()),
                picture: super::super::Picture {
                    format: Some(true),
                    dither: None,
                },
                memory: super::super::Memory {
                    limit: Some(6 << 30),
                },
            }
        );
        assert_eq!(args.bits(), 16);
        let live = parse(&strings(
            "a.toml --camera main --frames 3 --fps 24 --size 8x8 --out d",
        ))
        .unwrap();
        assert_eq!(live.samples(), None);
        assert_eq!(live.film, None);
        let traced = parse(&strings(
            "a.toml --camera main --frames 3 --fps 24 --size 8x8 --out d --trace",
        ))
        .unwrap();
        assert_eq!(traced.samples(), Some(64));
        assert!(traced.denoised() && traced.passes.is_none());
    }

    #[test]
    fn a_traced_clip_stops_pixels_no_earlier_than_the_automatic_floor() {
        let args = |line: &str| {
            parse(&strings(&format!(
                "a.toml --camera main --frames 3 --fps 24 --size 8x8 --out d --trace {line}"
            )))
            .unwrap()
        };
        let trace = |threshold: Option<f64>, min_samples: Option<u32>| pfx_recipe::Trace {
            threshold,
            min_samples,
            ..pfx_recipe::Trace::default()
        };
        assert_eq!(noise(&args(""), &trace(None, None), 64), None);
        let floored = noise(&args(""), &trace(Some(0.01), Some(8)), 256).unwrap();
        assert_eq!(floored.min_samples, 64);
        let flagged = noise(&args("--threshold 0.05"), &trace(None, Some(48)), 256).unwrap();
        assert_eq!((flagged.threshold, flagged.min_samples), (0.05, 48));
        assert_eq!(
            noise(&args("--threshold 0"), &trace(Some(0.01), None), 256),
            None
        );
    }

    #[test]
    fn a_figurine_orbit_is_planned_live_and_traced() {
        let _sample = super::super::sample("figurine");
        let dir = scratch("figurine");
        let planned = |text: &str| {
            let words = strings(text);
            assert!(super::super::claims(
                &[strings("clip"), words.clone()].concat()
            ));
            let args = parse(&words).unwrap();
            precheck(&args).unwrap();
            resolved(&args).unwrap()
        };
        let live = planned(
            "samples/figurine.toml --camera orbit --frames 24 --fps 24 --size 480x300 --out tmp/f",
        );
        let text = manifest(&live);
        assert!(
            text.contains("camera orbit\nframes 24\nsize 480x300\nrenderer live\n"),
            "{text}"
        );
        assert_eq!(
            pending(&dir, live.count(), live.size, 8),
            (0..24).collect::<Vec<_>>()
        );
        assert_eq!(live.rate(), Rate::whole(24));
        let traced = planned(
            "samples/figurine.toml --camera orbit --frames 8 --fps 8 --size 320x200 --trace --spp 16 --variant grin --set object.head.morph.smile=0.6 --out tmp/f",
        );
        let text = manifest(&traced);
        assert!(
            text.contains(
                "renderer traced, 16 spp\nvariant grin\nset object.head.morph.smile=0.6\n"
            ),
            "{text}"
        );
        let camera = pfx_recipe_scene::camera(&chosen(&traced).unwrap(), "orbit")
            .unwrap()
            .clone();
        let path = path_of(&camera).unwrap();
        assert_eq!(
            *path,
            pfx_recipe::CameraPath {
                kind: pfx_recipe::PathKind::Orbit(pfx_recipe::Orbit {
                    target: [0.0, 0.95, 0.0],
                    turns: 1.0,
                    rise_deg: 10.0,
                    radius_scale: 1.0,
                }),
                ease: pfx_recipe::Ease::Linear,
            }
        );
        assert_eq!(path.time(4, traced.count()), 0.5);
        let still = parse(&strings(
            "samples/figurine.toml --camera hero --frames 8 --fps 8 --size 320x200 --out tmp/f",
        ))
        .unwrap();
        assert!(
            precheck(&still)
                .unwrap_err()
                .contains("camera.hero: the camera has no path")
        );
    }

    #[test]
    fn mistakes_are_refused() {
        let base = "a.toml --camera main --frames 3 --fps 24 --size 8x8 --out d";
        assert!(parse(&strings(base)).is_ok());
        let filmed = parse(&strings(
            "a.toml --camera main --timeline --size 8x8 --out d",
        ))
        .unwrap();
        assert!(filmed.timeline && filmed.frames.is_none() && filmed.fps.is_none());
        for extra in [
            "--spp 8",
            "--checkpoint 10",
            "--trace --checkpoint 0",
            "--trace --checkpoint 8x",
            "--trace --spp 0",
            "--denoise off",
            "--passes p",
            "--trace --denoise",
            "--trace --denoise maybe",
            "--trace --passes",
            "--video a.gif",
            "--video a",
            "--nope",
            "b.toml",
            "--set",
            "--timeline",
            "--turns 2",
            "--spin body --turns",
            "--spin body --turns inf",
            "--spin body --turns x",
        ] {
            assert!(
                parse(&strings(&format!("{base} {extra}"))).is_err(),
                "{extra}"
            );
        }
        for bad in [
            "a.toml --frames 3 --fps 24 --size 8x8 --out d",
            "a.toml --camera main --fps 24 --size 8x8 --out d",
            "a.toml --camera main --frames 3 --size 8x8 --out d",
            "a.toml --camera main --frames 3 --fps 24 --out d",
            "a.toml --camera main --frames 3 --fps 24 --size 8x8",
            "--camera main --frames 3 --fps 24 --size 8x8 --out d",
            "a.toml --camera main --frames 0 --fps 24 --size 8x8 --out d",
            "a.toml --camera main --frames 3 --fps 0 --size 8x8 --out d",
            "a.toml --camera main --frames 3 --fps 241 --size 8x8 --out d",
            "a.toml --camera main --frames -1 --fps 24 --size 8x8 --out d",
            "a.toml --camera main --frames 3 --fps 24 --size 0x8 --out d",
            "a.toml --camera main --timeline --spin body --size 8x8 --out d",
        ] {
            assert!(parse(&strings(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn frames_are_numbered_from_zero_wide_enough_for_the_last() {
        assert_eq!(name(0, 24), "frame-0000.png");
        assert_eq!(name(23, 24), "frame-0023.png");
        assert_eq!(name(9999, 10000), "frame-9999.png");
        assert_eq!(name(7, 10001), "frame-00007.png");
        assert_eq!(
            pfx_core::atomic::partial(Path::new(&name(3, 24))),
            Path::new(".frame-0003.partial.png")
        );
        assert_eq!(
            checkpoint_of(Path::new("d"), 3, 24),
            Path::new("d/.frame-0003.checkpoint")
        );
    }

    #[test]
    fn an_opaque_frame_is_written_as_rgb_and_a_transparent_one_keeps_its_alpha() {
        let dir = scratch("colour");
        let colour = |path: &Path| {
            let file = std::fs::File::open(path).unwrap();
            png::Decoder::new(std::io::BufReader::new(file))
                .read_info()
                .unwrap()
                .info()
                .color_type
        };
        let opaque: Vec<u8> = (0..4 * 2 * 4)
            .map(|n| if n % 4 == 3 { 255 } else { (n * 7) as u8 })
            .collect();
        let mut clear = opaque.clone();
        clear[7] = 128;
        for (rgba, wanted) in [
            (&opaque, png::ColorType::Rgb),
            (&clear, png::ColorType::Rgba),
        ] {
            let path = dir.join(format!("{wanted:?}.png"));
            pfx_recipe_scene::write_frame(&path, [4, 2], rgba).unwrap();
            assert_eq!(colour(&path), wanted);
            assert_eq!(
                pfx_recipe_scene::read_png(&path).unwrap(),
                ([4, 2], rgba.clone())
            );
            assert_eq!(
                pfx_recipe_scene::read_png_shape(&path).unwrap(),
                ([4, 2], 8)
            );
        }
    }

    #[test]
    fn resuming_skips_finished_frames() {
        let dir = scratch("resume");
        let size = [4, 2];
        let rgba = vec![200u8; 4 * 2 * 4];
        let opaque = vec![255u8; 4 * 2 * 4];
        pfx_recipe_scene::write_png(&dir.join(name(0, 5)), size, &rgba).unwrap();
        pfx_recipe_scene::write_frame(&dir.join(name(2, 5)), size, &opaque).unwrap();
        pfx_recipe_scene::write_png(&dir.join(name(3, 5)), [2, 2], &rgba[..16]).unwrap();
        let whole = std::fs::read(dir.join(name(0, 5))).unwrap();
        std::fs::write(dir.join(name(4, 5)), &whole[..whole.len() - 20]).unwrap();
        std::fs::write(
            dir.join(pfx_core::atomic::partial(Path::new(&name(1, 5)))),
            &whole,
        )
        .unwrap();
        assert_eq!(pending(&dir, 5, size, 8), [1, 3, 4]);
        assert_eq!(pending(&dir, 5, size, 16), [0, 1, 2, 3, 4]);
        assert_eq!(pending(&dir, 5, [4, 4], 8), [0, 1, 2, 3, 4]);
        assert_eq!(pending(&scratch("empty"), 3, size, 8), [0, 1, 2]);
    }

    #[test]
    fn a_folder_resumes_only_the_line_that_made_it() {
        let dir = scratch("claim");
        let line = |extra: &str| {
            parse(&strings(&format!(
                "a.toml --camera main --frames 3 --size 8x8 --out {} {extra}",
                dir.display()
            )))
            .unwrap()
        };
        let first = manifest(&line("--fps 24"));
        assert!(first.contains("renderer live\n"), "{first}");
        claim(&dir, &first).unwrap();
        claim(&dir, &first).unwrap();
        assert_eq!(manifest(&line("--fps 30 --video a.mp4")), first);
        let traced = manifest(&line("--fps 24 --trace --spp 32 --set sky.intensity=0.5"));
        assert!(traced.contains("renderer traced, 32 spp\nset sky.intensity=0.5\n"));
        assert_eq!(
            manifest(&line(
                "--fps 24 --trace --spp 32 --denoise on --set sky.intensity=0.5"
            )),
            traced
        );
        let raw = manifest(&line("--fps 24 --trace --spp 32 --denoise off --passes p"));
        assert!(
            raw.contains("renderer traced, 32 spp\ndenoise off\npasses\n"),
            "{raw}"
        );
        let refused = claim(&dir, &traced).unwrap_err();
        assert!(refused.contains("made by another line"), "{refused}");
    }

    #[test]
    fn pcod_reads_the_frames_folder_at_the_clips_rate() {
        let line = encode_line(Path::new("-d"), Path::new("-c.webm"), Rate::whole(24)).unwrap();
        let here = std::env::current_dir().unwrap();
        assert_eq!(
            line,
            [
                here.join("-d").display().to_string(),
                here.join("-c.webm").display().to_string(),
                "--fps".into(),
                "24".into(),
                "--profile".into(),
                "best".into(),
                "--force".into(),
            ]
        );
        let film = rate("23.976").unwrap();
        let line = encode_line(Path::new("-d"), Path::new("-c.webm"), film).unwrap();
        assert_eq!(
            line,
            [
                here.join("-d").display().to_string(),
                here.join("-c.webm").display().to_string(),
                "--fps".into(),
                "24000/1001".into(),
                "--profile".into(),
                "best".into(),
                "--force".into(),
            ]
        );
    }

    #[test]
    fn fps_takes_whole_rational_and_ntsc_rates_and_refuses_the_rest_in_clap_shape() {
        let base = "a.toml --camera main --frames 3 --size 8x8 --out d --fps";
        for (fps, num, den) in [
            ("24", 24, 1),
            ("24000/1001", 24000, 1001),
            ("30000/1001", 30000, 1001),
            ("60000/1001", 60000, 1001),
            ("23.976", 24000, 1001),
            ("29.97", 30000, 1001),
            ("59.94", 60000, 1001),
            ("47.952", 48000, 1001),
            ("119.88", 120000, 1001),
        ] {
            let parsed = parse(&strings(&format!("{base} {fps}"))).unwrap();
            let found = parsed.rate();
            assert_eq!((found.num(), found.den()), (num, den), "{fps}");
        }
        let refused = parse(&strings(&format!("{base} 23.97"))).unwrap_err();
        assert!(
            refused.contains("invalid value '23.97' for '--fps <F>': ")
                && refused.contains("use 24000/1001"),
            "{refused}"
        );
        for bad in ["12.5", "0", "241", "1/2", "abc"] {
            assert!(parse(&strings(&format!("{base} {bad}"))).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_traced_line_is_build_slice_then_pgpu_clip_unless_queued() {
        let args = strings("a.toml --camera main --trace");
        let full = Machine {
            queued: false,
            pgpu: true,
            systemd: true,
            cpus: Some("2-5"),
        };
        assert_eq!(
            line("/opt/pfx", &args, full).unwrap().join(" "),
            "systemd-run --user --quiet --scope --collect --slice=build.slice -p MemoryMax=12G -p MemorySwapMax=0 -- taskset -c 2-5 nice -n 10 pgpu run --class clip --as pfx -- /opt/pfx clip a.toml --camera main --trace"
        );
        let queued = Machine {
            queued: true,
            ..full
        };
        assert_eq!(line("pfx", &args, queued), None);
    }

    #[test]
    fn a_bad_recipe_or_camera_refuses_with_1_before_the_queue() {
        let one = format!("{:?}", ExitCode::FAILURE);
        let tail = "--frames 3 --fps 24 --size 8x8 --out tmp/clip-tests/never --trace";
        for line in [
            format!("nowhere/none.toml --camera main {tail}"),
            format!("crates/recipe/tests/fixtures/full.toml --camera nope {tail}"),
            format!("crates/recipe/tests/fixtures/full.toml --camera main {tail}"),
        ] {
            let words = strings(&line);
            let code = run(&parse(&words).unwrap(), &words, &Registry::new());
            assert_eq!(format!("{code:?}"), one, "{line}");
        }
        assert!(!Path::new("tmp/clip-tests/never").exists());
    }

    const BODY: &str = "recipe = 1\nname = \"turntable\"\n[[object]]\nname = \"body\"\nkind = \"box\"\nsize = [0.01, 0.01, 0.01]\nat = [1.0, 0.5, -2.0]\nrotate = [0.0, 30.0, 0.0]\nhidden = true\n[[object]]\nname = \"figure\"\nkind = \"box\"\nsize = [0.2, 0.4, 0.2]\nparent = \"body\"\nat = [0.3, 0.0, 0.1]\nrotate = [0.0, 0.0, 10.0]\n[[object]]\nname = \"hat\"\nkind = \"box\"\nsize = [0.1, 0.1, 0.1]\nparent = \"figure\"\nat = [0.0, 0.3, 0.0]\n[[object]]\nname = \"floor\"\nkind = \"box\"\nsize = [4.0, 0.1, 4.0]\n[[camera]]\nname = \"main\"\nposition = [0.0, 1.0, 3.0]\ntarget = [1.0, 0.5, -2.0]\nfov_y_deg = 40.0\n";

    #[test]
    fn a_spin_turns_from_zero_to_its_turns_over_the_clip() {
        assert_eq!(degrees(1.0, 0, 8), 0.0);
        assert_eq!(degrees(1.0, 8, 8), 360.0);
        assert_eq!(degrees(1.0, 2, 8), 90.0);
        assert_eq!(degrees(-2.0, 8, 8), -720.0);
        assert_eq!(degrees(0.5, 4, 8), 90.0);
        let recipe = pfx_recipe::parse(BODY).unwrap();
        let body = spun(&recipe, "body").unwrap();
        assert_eq!(
            motion(&recipe, body, degrees(1.0, 0, 8)).unwrap(),
            DMat4::IDENTITY
        );
        let whole = motion(&recipe, body, degrees(3.0, 8, 8)).unwrap();
        assert!(whole.abs_diff_eq(DMat4::IDENTITY, 1e-12), "{whole:?}");
        let message = spun(&recipe, "nobody").unwrap_err();
        assert!(
            message.contains("no such object, expected one of: body, figure, hat, floor"),
            "{message}"
        );
    }

    #[test]
    fn a_spin_turns_the_whole_parent_chain_about_its_own_origin() {
        let recipe = pfx_recipe::parse(BODY).unwrap();
        let body = spun(&recipe, "body").unwrap();
        let turned = motion(&recipe, body, 75.0).unwrap();
        let mut rotated = recipe.clone();
        rotated.objects[body].rotate[1] += 75.0;
        for name in ["body", "figure", "hat"] {
            let at = spun(&recipe, name).unwrap();
            let moved = turned * pfx_build::world(&recipe, at).unwrap();
            let expected = pfx_build::world(&rotated, at).unwrap();
            assert!(moved.abs_diff_eq(expected, 1e-12), "{name}");
        }
        let pivot = pfx_build::world(&recipe, body).unwrap().w_axis;
        assert!((turned * pivot).abs_diff_eq(pivot, 1e-12));
        assert_eq!(
            pfx_recipe_scene::family_of(&recipe, body),
            [true, true, true, false]
        );
        let figure = spun(&recipe, "figure").unwrap();
        assert_eq!(
            pfx_recipe_scene::family_of(&recipe, figure),
            [false, true, true, false]
        );
        let hat = pfx_build::world(&recipe, spun(&recipe, "hat").unwrap()).unwrap();
        let around = motion(&recipe, figure, 90.0).unwrap() * hat;
        let origin = pfx_build::world(&recipe, figure).unwrap().w_axis;
        let lever = hat.w_axis - origin;
        let swung = around.w_axis - origin;
        assert!((swung.x - lever.z).abs() < 1e-12 && (swung.z + lever.x).abs() < 1e-12);
        assert!((swung.y - lever.y).abs() < 1e-12);
    }

    #[test]
    fn a_spin_holds_a_still_camera_and_refuses_a_moving_one() {
        let recipe = pfx_recipe::parse(BODY).unwrap();
        let line = |rest: &str| {
            parse(&strings(&format!(
                "a.toml --camera main --frames 8 --fps 24 --size 8x8 --out d {rest}"
            )))
            .unwrap()
        };
        assert_eq!(held(&line("--spin body"), &recipe.cameras[0]), Ok(()));
        assert!(
            held(&line(""), &recipe.cameras[0])
                .unwrap_err()
                .contains("has no path")
        );
        let mut moving = recipe.cameras[0].clone();
        moving.path = orbit();
        let refused = held(&line("--spin body --turns 2"), &moving).unwrap_err();
        assert!(refused.contains("holds the camera still"), "{refused}");
        let spun = manifest(&line("--spin body --turns -1.5"));
        assert!(spun.contains("spin body, -1.5 turns\n"), "{spun}");
        assert_eq!(
            manifest(&line("--spin body")),
            manifest(&line("--spin body --turns 1"))
        );
        assert!(!manifest(&line("")).contains("spin"));
    }

    fn orbit() -> Option<CameraPath> {
        pfx_recipe::parse(
            "recipe = 1\nname = \"o\"\n[[camera]]\nname = \"main\"\nposition = [0.0, 1.0, 2.0]\ntarget = [0.0, 0.0, 0.0]\nfov_y_deg = 40.0\n[camera.path]\nkind = \"orbit\"\ntarget = [0.0, 0.0, 0.0]\nturns = 1.0\n",
        )
        .unwrap()
        .cameras[0]
            .path
            .clone()
    }

    #[test]
    fn a_camera_without_a_path_is_refused() {
        let recipe = pfx_recipe::parse(
            "recipe = 1\nname = \"still\"\n[[camera]]\nname = \"main\"\nposition = [0.0, 1.0, 2.0]\ntarget = [0.0, 0.0, 0.0]\nfov_y_deg = 40.0\n",
        )
        .unwrap();
        let message = path_of(&recipe.cameras[0]).unwrap_err();
        assert!(message.contains("has no path"), "{message}");
    }

    fn table_orbit(dir: &Path) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let table = std::fs::read_to_string(root.join("tests/golden/table/table.toml")).unwrap();
        let recipe = dir.join("table-orbit.toml");
        std::fs::write(
            &recipe,
            format!(
                "{table}\n[camera.path]\nkind = \"orbit\"\ntarget = [0.0, 0.03, -0.02]\nturns = 1.0\n"
            ),
        )
        .unwrap();
        recipe
    }

    fn table_line(recipe: &Path, out: &Path, rest: &str) -> Args {
        parse(&strings(&format!(
            "{} --camera main --frames 3 --fps 24 --out {} {rest}",
            recipe.display(),
            out.display()
        )))
        .unwrap()
    }

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn three_table_frames_start_on_the_shot_and_resume_to_the_same_bytes() {
        let dir = scratch("live");
        let recipe = table_orbit(&dir);
        let out = dir.join("frames");
        let args = table_line(&recipe, &out, "--size 320x200");
        frames(&args, &Registry::new(), &mut |_| {}).unwrap();
        let read = |frame: u32| {
            let (size, rgba) = pfx_recipe_scene::read_png(&out.join(name(frame, 3))).unwrap();
            assert_eq!(size, args.size);
            rgba
        };
        let made: Vec<Vec<u8>> = (0..3).map(read).collect();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (table, built) = pfx_recipe_scene::load(
            &root.join("tests/golden/table/table.toml"),
            None,
            &[],
            &Registry::new(),
        )
        .unwrap();
        let shot = pfx_recipe_scene::shoot_posed_as(
            &pfx_recipe_scene::headless().unwrap(),
            (&table, &built),
            "main",
            args.size,
            None,
            args.picture.output(0, crate::recipe::Background::Sky),
        )
        .unwrap();
        assert!(
            made[0] == shot,
            "frame 0 differs from pfx shot --still of the camera"
        );
        assert!(made[1] != made[0] && made[2] != made[1]);
        for frame in [1, 2] {
            std::fs::remove_file(out.join(name(frame, 3))).unwrap();
        }
        assert_eq!(pending(&out, 3, args.size, 8), [1, 2]);
        frames(&args, &Registry::new(), &mut |_| {}).unwrap();
        for frame in 0..3 {
            assert!(
                read(frame) == made[frame as usize],
                "frame {frame} after resuming"
            );
        }
        assert!(pending(&out, 3, args.size, 8).is_empty());
    }

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn a_traced_frame_is_the_render_of_its_pose() {
        let dir = scratch("traced");
        let recipe = table_orbit(&dir);
        let out = dir.join("frames");
        let args = table_line(&recipe, &out, "--size 160x100 --trace --spp 4");
        let mut done = Vec::new();
        frames(&args, &Registry::new(), &mut |n| done.push(n)).unwrap();
        assert_eq!(done, [0, 1, 2, 3]);
        let request = pfx_still::Request {
            recipe: &recipe,
            camera: "main",
            size: args.size,
            samples: 4,
            noise: None,
            seed: None,
            view: pfx_still::View::Lens,
            variant: None,
            set: &[],
            pick: &pfx_recipe::Pick::default(),
            crop: None,
            denoise: true,
            time: None,
            transparent: false,
        };
        let rendered = pfx_still::render(&request, &Registry::new(), &mut |_| {}).unwrap();
        let pixels =
            pfx_still::finished(&rendered.film, (&rendered.finish, &rendered.overlay), true)
                .unwrap();
        let render = pfx_still::dithered_at(
            args.size,
            [0, 0],
            args.picture.dither(u64::from(rendered.seed)),
            &pixels,
        );
        let (_, first) = pfx_recipe_scene::read_png(&out.join(name(0, 3))).unwrap();
        assert!(
            first == render,
            "frame 0 differs from pfx render --recipe of the camera"
        );
        let (_, second) = pfx_recipe_scene::read_png(&out.join(name(1, 3))).unwrap();
        assert!(second != first);
    }

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn the_passes_are_the_same_denoised_or_not_and_off_is_the_raw_trace() {
        let dir = scratch("passes");
        let recipe = table_orbit(&dir);
        let stem = name(0, 3).trim_end_matches(".png").to_string();
        let made = |denoise: &str| {
            let out = dir.join(format!("frames-{denoise}"));
            let passes = dir.join(format!("passes-{denoise}"));
            let args = table_line(
                &recipe,
                &out,
                &format!(
                    "--size 160x100 --trace --spp 4 --range 0..0 --denoise {denoise} --passes {}",
                    passes.display()
                ),
            );
            frames(&args, &Registry::new(), &mut |_| {}).unwrap();
            let (_, png) = pfx_recipe_scene::read_png(&out.join(name(0, 3))).unwrap();
            let files =
                pfx_still::pass_paths(&passes, &stem).map(|path| std::fs::read(path).unwrap());
            (args, png, files, passes)
        };
        let (_, on, on_files, on_passes) = made("on");
        let (args, off, off_files, _) = made("off");
        assert!(
            on_files == off_files,
            "the passes differ with --denoise on and off"
        );
        assert!(on != off);
        let [_, albedo, normal] = pfx_still::pass_paths(&on_passes, &stem);
        for guide in [albedo, normal] {
            let (size, pixels) = pfx_still::read_exr(&guide).unwrap();
            assert_eq!(size, args.size);
            assert!(
                pixels.iter().any(|p| p[..3] != [0.0; 3]),
                "{}",
                guide.display()
            );
        }
        let request = pfx_still::Request {
            recipe: &recipe,
            camera: "main",
            size: args.size,
            samples: 4,
            noise: None,
            seed: None,
            view: pfx_still::View::Lens,
            variant: None,
            set: &[],
            pick: &pfx_recipe::Pick::default(),
            crop: None,
            denoise: false,
            time: None,
            transparent: false,
        };
        let rendered = pfx_still::render(&request, &Registry::new(), &mut |_| {}).unwrap();
        let pixels =
            pfx_still::finished(&rendered.film, (&rendered.finish, &rendered.overlay), false)
                .unwrap();
        let render = pfx_still::dithered_at(
            args.size,
            [0, 0],
            args.picture.dither(u64::from(rendered.seed)),
            &pixels,
        );
        assert!(
            off == render,
            "a --denoise off frame differs from pfx render --recipe --denoise off of the camera"
        );
    }

    const NTSC: &str = r#"recipe = 1
name = "ntsc"

[timeline]
fps = "24000/1001"
frames = 12

[motion_blur]
shutter = 0.5

[sun]
model = "authored"
toward = [0.3, 0.6, 1.0]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 4.0

[[camera]]
name = "main"
position = [0.75, 0.5, 4.0]
target = [0.75, 0.5, 0.0]
fov_y_deg = 45.0

[materials.white]
base = [0.9, 0.9, 0.9]
roughness = 0.8

[[object]]
name = "block"
kind = "box"
size = [0.5, 0.5, 0.5]
material = "white"
at = [0.0, 0.5, 0.0]
keys = { at = [[0.0, [0.0, 0.5, 0.0]], [0.5, [1.5, 0.5, 0.0]]] }
"#;

    #[test]
    #[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
    fn a_ntsc_traced_range_resumes_to_the_bytes_of_a_full_run() {
        let dir = scratch("ntsc");
        let recipe = dir.join("ntsc.toml");
        std::fs::write(&recipe, NTSC).unwrap();
        let out = dir.join("frames");
        let args = resolved(
            &parse(&strings(&format!(
                "{} --camera main --timeline --size 96x48 --out {} --trace --spp 2",
                recipe.display(),
                out.display()
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (args.count(), args.rate().to_string()),
            (13, "24000/1001".into())
        );
        frames(&args, &Registry::new(), &mut |_| {}).unwrap();
        let read = |frame: u32| {
            pfx_recipe_scene::read_png(&out.join(name(frame, 13)))
                .unwrap()
                .1
        };
        let made: Vec<Vec<u8>> = (0..13).map(read).collect();
        assert!(made[10] != made[11] && made[11] != made[12]);
        for frame in 10..13 {
            std::fs::remove_file(out.join(name(frame, 13))).unwrap();
        }
        assert_eq!(pending(&out, 13, args.size, 8), [10, 11, 12]);
        frames(&args, &Registry::new(), &mut |_| {}).unwrap();
        for frame in 0..13 {
            assert!(
                read(frame) == made[frame as usize],
                "frame {frame} after resuming"
            );
        }
    }

    #[test]
    fn the_scene_dump_lists_every_instance_with_its_mesh_bounds_and_squash() {
        let traced = |index: usize, mesh: Option<usize>, transform: Option<Matrix>| {
            let rows = transform.map(|m| {
                [
                    m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1],
                    m[0][2], m[1][2], m[2][2], m[3][2],
                ]
            });
            pfx_still::TracedInstance {
                index,
                mesh,
                triangles: if mesh.is_some() { 12 } else { 0 },
                instanced: usize::from(mesh.is_some()) * 2,
                record: mesh.map(|_| index as u32 + 1),
                transform,
                rows,
                bounds: mesh.map(|_| pfx_still::TraceBounds {
                    lo: [0.0, 0.0, 0.0],
                    hi: [1.0, 2.0, 3.0],
                }),
                casts_shadow: mesh.is_some(),
                alpha_cutoff: mesh.map(|_| 0.5),
                hardware: mesh.map(|_| pfx_still::TlasListed {
                    mask: Some(0xff),
                    rows,
                    custom: Some(index as u32 + 1),
                    opaque: true,
                    mover: false,
                }),
                ..pfx_still::TracedInstance::default()
            }
        };
        let piece = |name: &str, traced| pfx_still::TracedPiece {
            name: name.into(),
            object: Some("tree".into()),
            material: Some("leaf".into()),
            shared: Some(0),
            two_sided: Some(true),
            look: Some(Default::default()),
            traced,
        };
        let turned = [
            [0.0, 0.0, -2.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [2.0, 0.0, 0.0, 0.0],
            [5.0, 0.0, 9.0, 1.0],
        ];
        let sheared = [
            [1.0, 0.0, 0.0, 0.0],
            [0.9, 0.1, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut drifted = piece("tree.d", traced(3, Some(3), Some(turned)));
        if let Some(listed) = &mut drifted.traced.hardware {
            listed.custom = Some(1);
        }
        let listed = listed(
            7,
            &[
                piece("tree.a", traced(0, Some(3), Some(turned))),
                piece("tree.b", traced(1, Some(3), Some(sheared))),
                piece("tree.c", traced(2, None, None)),
                drifted,
            ],
        );
        assert_eq!(listed["frame"], 7);
        assert_eq!(listed["instances"], 4);
        assert_eq!(listed["traced"], 3);
        assert_eq!(
            listed["meshes"],
            serde_json::json!([{ "mesh": 3, "triangles": 12, "instances": 2 }])
        );
        let list = listed["list"].as_array().unwrap();
        let keys = |entry: &serde_json::Value| -> Vec<String> {
            let mut keys: Vec<String> = entry.as_object().unwrap().keys().cloned().collect();
            keys.sort_unstable();
            keys
        };
        let mut traced_keys = vec![
            "alpha",
            "alpha_cutoff",
            "alpha_mode",
            "base",
            "bounds",
            "casts_shadow",
            "clipped",
            "cutoff",
            "cutout",
            "deforming",
            "determinant",
            "hardware_mover",
            "index",
            "instanced",
            "material",
            "mesh",
            "moving",
            "name",
            "object",
            "one_sided",
            "shadow_only",
            "shared",
            "sprite",
            "squash",
            "tlas_custom",
            "tlas_in_place",
            "tlas_mask",
            "tlas_opaque",
            "traced",
            "transmission",
            "triangles",
            "two_sided",
        ];
        traced_keys.sort_unstable();
        assert_eq!(keys(&list[0]), traced_keys);
        let untraced: Vec<&str> = traced_keys
            .iter()
            .copied()
            .filter(|key| {
                ![
                    "bounds",
                    "casts_shadow",
                    "clipped",
                    "cutoff",
                    "cutout",
                    "determinant",
                    "hardware_mover",
                    "one_sided",
                    "shadow_only",
                    "squash",
                    "tlas_custom",
                    "tlas_in_place",
                    "tlas_mask",
                    "tlas_opaque",
                ]
                .contains(key)
            })
            .collect();
        assert_eq!(keys(&list[2]), untraced);
        assert_eq!(list[0]["name"], "tree.a");
        assert_eq!(list[0]["mesh"], 3);
        assert_eq!(list[0]["instanced"], 2);
        assert_eq!(list[0]["bounds"]["hi"], serde_json::json!([1.0, 2.0, 3.0]));
        assert_eq!(list[0]["determinant"], 8.0);
        assert_eq!(list[0]["squash"], 1.0);
        assert_eq!(list[0]["tlas_mask"], 255);
        assert_eq!(list[0]["tlas_custom"], 1);
        assert_eq!(list[0]["tlas_in_place"], true);
        assert_eq!(list[0]["one_sided"], false);
        assert_eq!(list[0]["casts_shadow"], true);
        assert_eq!(list[0]["cutoff"], 0.5);
        assert!(list[1]["squash"].as_f64().unwrap() < 0.2);
        assert_eq!(list[2]["traced"], false);
        assert_eq!(list[3]["tlas_in_place"], false);
    }
}
