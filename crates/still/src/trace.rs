use crate::picture::{Film, Reach};
use crate::staging::{IDENTITY, Staging, Steps, View, eyepiece};
use crate::text::Overlay;
use pfx_build::{Built, Registry};
use pfx_core::memory::{Plan as MemoryPlan, Run};
use pfx_denoise::Denoiser;
use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Stats};
use pfx_live::frame::Matrix;
use pfx_post::Chain;
use pfx_recipe::{Camera, Recipe};
use pfx_recipe_scene::pace::{Report, TurnState, turn_reporting};
use pfx_trace::checkpoint::Keeper;
use pfx_trace::gpu::Builds;
use pfx_trace::sky::Environment;
use pfx_trace::sobol::Sampler;
use pfx_trace::sprites::Spray;
use pfx_trace::stage::Staged;
use pfx_trace::{Adaptive, Matte, Round, Trace, TraceError};
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::path::Path;

pub const SAMPLES: u32 = 64;
pub const SIZE: [u32; 2] = [1586, 992];
pub const EQUIRECT_SIZE: [u32; 2] = [2048, 1024];
pub const NOISE: Noise = Noise {
    threshold: 0.01,
    min_samples: 16,
};
pub const DRAFT: (u32, Noise) = (
    12,
    Noise {
        threshold: 0.05,
        min_samples: 6,
    },
);
const GROWTH: f32 = 1.5;
const DENOISE_ALIGN: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noise {
    pub threshold: f32,
    pub min_samples: u32,
}

impl Noise {
    pub fn floored(self) -> Noise {
        Noise {
            min_samples: self
                .min_samples
                .max(Adaptive::automatic_min_samples(self.threshold)),
            ..self
        }
    }

    fn adaptive(self, cap: u32) -> Option<Adaptive> {
        let adaptive = Adaptive {
            threshold: self.threshold,
            min_samples: self.min_samples.clamp(2, cap.max(2)),
            max_samples: cap,
            growth: GROWTH,
        };
        adaptive.valid().then_some(adaptive)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Seed {
    #[default]
    Locked,
    Animated(u32),
}

impl Seed {
    pub fn of(self, seed: u32) -> u32 {
        match self {
            Seed::Locked => seed,
            Seed::Animated(frame) => pfx_trace::sobol::animated_seed(seed, frame),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Take {
    pub size: [u32; 2],
    pub samples: u32,
    pub noise: Option<Noise>,
    pub seed: Seed,
}

impl Take {
    pub fn adaptive(&self) -> Option<Adaptive> {
        self.noise.and_then(|noise| noise.adaptive(self.samples))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Stage,
    Path(Box<pfx_trace::rt::Path>),
    Trace {
        done: u32,
        total: u32,
    },
    Converge {
        gpu_ms: u64,
        cap: u32,
        done: u64,
        total: u64,
    },
    Write,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    Refused(String),
    Failed(String),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Refused(message) | Fault::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Fault {}

impl From<pfx_recipe_scene::Error> for Fault {
    fn from(error: pfx_recipe_scene::Error) -> Fault {
        Fault::Failed(error.0)
    }
}

impl From<String> for Fault {
    fn from(message: String) -> Fault {
        Fault::Failed(message)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub recipe: &'a Path,
    pub camera: &'a str,
    pub size: [u32; 2],
    pub samples: u32,
    pub noise: Option<Noise>,
    pub seed: Option<u32>,
    pub view: View,
    pub variant: Option<&'a str>,
    pub set: &'a [String],
    pub pick: &'a pfx_recipe::Pick,
    pub crop: Option<Crop>,
    pub denoise: bool,
    pub time: Option<f64>,
    pub transparent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub rect: [u32; 4],
    pub apron: u32,
    pub finished: bool,
}

impl Crop {
    pub fn fits(rect: [u32; 4], size: [u32; 2]) -> bool {
        let [x, y, width, height] = rect;
        width > 0
            && height > 0
            && x.checked_add(width).is_some_and(|end| end <= size[0])
            && y.checked_add(height).is_some_and(|end| end <= size[1])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub full: [u32; 2],
    pub origin: [u32; 2],
    pub size: [u32; 2],
}

impl Region {
    pub fn whole(size: [u32; 2]) -> Region {
        Region {
            full: size,
            origin: [0, 0],
            size,
        }
    }

    pub fn around(full: [u32; 2], rect: [u32; 4], apron: u32) -> Region {
        let [x, y, width, height] = rect;
        let low = [x.saturating_sub(apron), y.saturating_sub(apron)];
        let high = [
            (x + width).saturating_add(apron).min(full[0]),
            (y + height).saturating_add(apron).min(full[1]),
        ];
        Region {
            full,
            origin: low,
            size: [high[0] - low[0], high[1] - low[1]],
        }
    }

    pub fn rect(&self) -> [u32; 4] {
        [self.origin[0], self.origin[1], self.size[0], self.size[1]]
    }

    pub fn grown(&self, by: u32) -> Region {
        Region::around(self.full, self.rect(), by)
    }

    pub fn aligned(&self, step: u32) -> Region {
        let step = step.max(1);
        let low = self.origin.map(|at| at / step * step);
        let high = [0, 1].map(|k| {
            (self.origin[k] + self.size[k])
                .div_ceil(step)
                .saturating_mul(step)
                .min(self.full[k])
        });
        Region {
            full: self.full,
            origin: low,
            size: [high[0] - low[0], high[1] - low[1]],
        }
    }

    fn open(
        &self,
        gpu: &Gpu,
        staged: &Staged<Environment>,
        transparent: bool,
    ) -> Result<Trace, Fault> {
        let mut trace = staged
            .trace(gpu, self.size[0], self.size[1])
            .map_err(refused)?;
        if transparent {
            trace
                .set_matte(Matte::transparent())
                .map_err(|e| Fault::Failed(format!("the tracer's matte: {e}")))?;
        }
        if *self != Region::whole(self.full) {
            trace
                .set_window(self.full, self.origin)
                .map_err(|e| Fault::Failed(format!("the tracer's window: {e}")))?;
        }
        Ok(trace)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    rounds: Vec<u32>,
    reached: usize,
    active: u64,
    work: u64,
    work_ms: f64,
}

impl Plan {
    pub fn new(adaptive: &Adaptive, pixels: u64) -> Plan {
        Plan {
            rounds: adaptive.rounds(),
            reached: 0,
            active: pixels,
            work: 0,
            work_ms: 0.0,
        }
    }

    fn before(&self) -> u32 {
        self.reached.checked_sub(1).map_or(0, |k| self.rounds[k])
    }

    pub fn reach(&mut self, round: Round, gpu_ms: f64) {
        self.work += u64::from(round.samples.saturating_sub(self.before())) * self.active;
        self.active = round.active;
        self.reached = (self.reached + 1).min(self.rounds.len());
        self.work_ms = gpu_ms;
        if round.active == 0 {
            self.reached = self.rounds.len();
        }
    }

    fn next(&self) -> u64 {
        self.rounds.get(self.reached).map_or(0, |&samples| {
            u64::from(samples - self.before()) * self.active
        })
    }

    pub fn total(&self) -> u64 {
        let mut total = self.work;
        let mut before = self.before();
        for &samples in &self.rounds[self.reached..] {
            total += u64::from(samples - before) * self.active;
            before = samples;
        }
        total
    }

    pub fn done(&self, gpu_ms: f64) -> u64 {
        if self.work == 0 || self.work_ms <= 0.0 {
            return 0;
        }
        let rate = self.work as f64 / self.work_ms;
        let partial = ((gpu_ms - self.work_ms).max(0.0) * rate) as u64;
        self.work + partial.min(self.next())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Paced {
    pub submissions: usize,
    pub longest_ms: f64,
    pub gpu_ms: f64,
    pub wall_ms: f64,
    pub trace_bytes: u64,
    pub gpu_resident: Option<u64>,
    pub planned: [u64; 2],
    pub builds: Builds,
}

impl Paced {
    fn add(&mut self, stats: &Stats) {
        self.submissions += stats.count();
        self.longest_ms = self.longest_ms.max(stats.longest_ms());
        self.gpu_ms += stats.milliseconds.iter().sum::<f64>();
        self.wall_ms += stats.wall_ms;
    }
}

struct Turns<'a> {
    state: TurnState,
    turn: &'a mut dyn FnMut(Report),
}

impl<'a> Turns<'a> {
    fn new(turn: &'a mut dyn FnMut(Report)) -> Turns<'a> {
        Turns {
            state: TurnState::default(),
            turn,
        }
    }

    fn soon(&mut self, slice_ms: f64) -> bool {
        self.state.add(slice_ms);
        if !self.state.due() {
            return false;
        }
        self.now();
        true
    }

    fn now(&mut self) {
        (self.turn)(self.state.take());
    }
}

pub struct Rendered {
    pub film: Film,
    pub finish: Chain,
    pub overlay: Overlay,
    pub paced: Paced,
    pub seed: u32,
    pub mean_samples: f64,
    pub most_samples: u32,
    pub region: Region,
    pub rect: [u32; 4],
    pub left_out: Vec<&'static str>,
    pub restaged: Option<String>,
}

impl Rendered {
    pub fn size(&self) -> [u32; 2] {
        [self.rect[2], self.rect[3]]
    }

    fn within(&self) -> [u32; 2] {
        [
            self.rect[0] - self.region.origin[0],
            self.rect[1] - self.region.origin[1],
        ]
    }

    pub fn crop(&self) -> Cow<'_, Film> {
        if self.region.rect() == self.rect {
            return Cow::Borrowed(&self.film);
        }
        Cow::Owned(self.film.cut(self.within(), self.size()))
    }

    pub fn finished(&self, denoised: bool) -> Result<Vec<[f32; 4]>, String> {
        let pixels = crate::picture::finished_at(
            &self.film,
            (&self.finish, &self.overlay),
            denoised,
            self.region.origin,
            self.region.full,
        )?;
        if self.region.rect() == self.rect {
            return Ok(pixels);
        }
        Ok(crate::picture::cut(
            &pixels,
            self.film.size[0],
            self.within(),
            self.size(),
        ))
    }
}

pub fn render(
    request: &Request<'_>,
    registry: &Registry,
    progress: &mut dyn FnMut(Step),
) -> Result<Rendered, Fault> {
    render_kept(request, registry, progress, None)
}

pub fn render_kept(
    request: &Request<'_>,
    registry: &Registry,
    progress: &mut dyn FnMut(Step),
    keeper: Option<&mut dyn Keeper>,
) -> Result<Rendered, Fault> {
    if request.samples == 0 {
        return Err(Fault::Failed("a render needs at least one sample".into()));
    }
    progress(Step::Stage);
    let mut turn = turn_reporting;
    let (recipe, built) = pfx_recipe_scene::load_at(
        request.recipe,
        request.variant,
        request.set,
        request.pick,
        registry,
        request.time,
    )?;
    let plan = pfx_core::memory::plan(
        &pfx_recipe_scene::census(&recipe, &built),
        Run::Trace {
            size: planned_size(request),
            denoise: request.denoise,
            held: false,
        },
    );
    plan.admit().map_err(Fault::Refused)?;
    turn(Report::default());
    let mut staging = Staging::of(&recipe, &built.parts)?;
    let emitted = pfx_build::emitters(&recipe, &built).map_err(|e| Fault::Failed(e.to_string()))?;
    let puffs = crate::sprays::puffs(
        &emitted,
        &built,
        recipe.timeline.map_or(pfx_build::Rate::whole(1), |t| t.fps),
        request.time.unwrap_or(0.0),
    );
    drop(built);
    staging.shed();
    let camera = pfx_recipe_scene::camera(&recipe, request.camera)?;
    let reach = match request.crop {
        Some(crop) if crop.finished => {
            if let Some(reason) = crate::picture::crop_refusal(
                (&staging.finish, &Overlay::of(&staging.text)),
                request.size,
            ) {
                return Err(Fault::Refused(format!(
                    "a crop's .png or master cannot match the full frame: {reason}; crop to .exr or .hdr"
                )));
            }
            crate::picture::reach(&staging.finish)
        }
        _ => Reach::NONE,
    };
    let mut staged = staging.stage(eyepiece(camera, request.view, request.size)?)?;
    let facing = staged.scene.camera;
    staged
        .spray(&Spray::new(&puffs, facing))
        .map_err(Fault::Failed)?;
    staging.images = Vec::new();
    staging.textures = Default::default();
    staged.detail.sampler = Sampler::Sobol {
        samples: request.samples,
    };
    staged.detail.fit_textures = Some(request.size);
    turn(Report::default());
    let adaptive = request
        .noise
        .and_then(|noise| noise.adaptive(request.samples));
    let (frame, region) = match request.crop {
        Some(crop) if !Crop::fits(crop.rect, request.size) => {
            return Err(Fault::Refused(format!(
                "the crop {:?} does not lie inside the {}x{} frame",
                crop.rect, request.size[0], request.size[1]
            )));
        }
        Some(crop) => {
            let frame = Region::around(request.size, crop.rect, reach.pixels).aligned(reach.align);
            let region = frame
                .grown(crop.apron.max(adaptive.map_or(0, |a| a.margin())))
                .aligned(if request.denoise { DENOISE_ALIGN } else { 1 });
            (frame, region)
        }
        None => (Region::whole(request.size), Region::whole(request.size)),
    };
    let gpu = pfx_recipe_scene::headless()?;
    let seed = request
        .seed
        .unwrap_or_else(|| pfx_recipe_scene::seed(&recipe));
    let mut denoiser = request.denoise.then(|| denoiser(&gpu)).transpose()?;
    let (film, mut paced, mean_samples, most_samples) = match adaptive {
        Some(adaptive) => converge(
            &gpu,
            &staged,
            Shot {
                region,
                seed,
                denoiser: denoiser.as_mut(),
                transparent: request.transparent,
            },
            adaptive,
            progress,
            &mut turn,
            keeper,
        )?,
        None => {
            let (film, paced) = trace_in(
                &gpu,
                &staged,
                Shot {
                    region,
                    seed,
                    denoiser: denoiser.as_mut(),
                    transparent: request.transparent,
                },
                request.samples,
                progress,
                &mut turn,
                keeper,
            )?;
            (film, paced, f64::from(request.samples), request.samples)
        }
    };
    paced.planned = [plan.cpu(), plan.gpu()];
    let film = if frame == region {
        film
    } else {
        film.cut(
            [
                frame.origin[0] - region.origin[0],
                frame.origin[1] - region.origin[1],
            ],
            frame.size,
        )
    };
    Ok(Rendered {
        film,
        overlay: Overlay::of(&staging.text),
        finish: staging
            .finish
            .at(request.time.unwrap_or(0.0), pfx_recipe_scene::seed(&recipe)),
        paced,
        seed,
        mean_samples,
        most_samples,
        region: frame,
        rect: request.crop.map_or(frame.rect(), |crop| crop.rect),
        left_out: pfx_recipe_scene::unapplied(&recipe, true),
        restaged: None,
    })
}

pub struct Reel {
    pub recipe: Recipe,
    pub seed: u32,
    pub motion_steps: u32,
    pub denoise: bool,
    pub guides: bool,
    base: Recipe,
    built: Built,
    staging: Staging,
    gpu: Gpu,
    denoiser: Option<Denoiser>,
    kept: Option<Kept>,
    moving_cameras: Vec<Camera>,
    time: f64,
    emitted: Vec<pfx_build::Emitted>,
    sprite_room: usize,
    shutter: Vec<f64>,
    plan: MemoryPlan,
}

pub const MOTION_STEPS: u32 = 1;

struct Kept {
    trace: Trace,
    size: [u32; 2],
    samples: u32,
}

impl Reel {
    pub fn open(
        recipe: &Path,
        variant: Option<&str>,
        set: &[String],
        seed: Option<u32>,
        registry: &Registry,
    ) -> Result<Reel, Fault> {
        Reel::open_for(recipe, variant, set, seed, registry, [0, 0])
    }

    pub fn open_for(
        recipe: &Path,
        variant: Option<&str>,
        set: &[String],
        seed: Option<u32>,
        registry: &Registry,
        size: [u32; 2],
    ) -> Result<Reel, Fault> {
        let (recipe, built) = pfx_recipe_scene::load(recipe, variant, set, registry)?;
        let plan = pfx_core::memory::plan(
            &pfx_recipe_scene::census(&recipe, &built),
            Run::Trace {
                size,
                denoise: true,
                held: true,
            },
        );
        plan.admit().map_err(Fault::Refused)?;
        turn_reporting(Report::default());
        let staging = Staging::of(&recipe, &built.parts)?;
        let emitted =
            pfx_build::emitters(&recipe, &built).map_err(|e| Fault::Failed(e.to_string()))?;
        let gpu = pfx_recipe_scene::headless()?;
        let seed = seed.unwrap_or_else(|| pfx_recipe_scene::seed(&recipe));
        Ok(Reel {
            base: recipe.clone(),
            recipe,
            seed,
            motion_steps: MOTION_STEPS,
            denoise: true,
            guides: false,
            built,
            staging,
            gpu,
            denoiser: None,
            kept: None,
            moving_cameras: Vec::new(),
            time: 0.0,
            sprite_room: emitted
                .iter()
                .map(|emitter| emitter.recipe.max as usize)
                .sum(),
            emitted,
            shutter: Vec::new(),
            plan,
        })
    }

    pub fn plan(&self) -> &MemoryPlan {
        &self.plan
    }

    pub fn adapter(&self) -> String {
        let info = &self.gpu.info;
        format!(
            "{}, {:?}, {} {}",
            info.name, info.backend, info.driver, info.driver_info
        )
    }

    pub fn builds(&self) -> Option<Builds> {
        self.kept.as_ref().map(|kept| kept.trace.builds())
    }

    pub fn timeline(&self) -> Option<pfx_recipe::Timeline> {
        self.base.timeline
    }

    pub fn pose(&mut self, time: f64) -> Result<(), Fault> {
        self.time = time;
        self.staging.steps = None;
        self.moving_cameras.clear();
        self.shutter.clear();
        self.posed_at(time)?;
        self.staging.refog(&self.recipe);
        if self.built.skinned() {
            for piece in pfx_recipe_scene::pieces(&self.built.skinned_parts(time)) {
                if let Some(found) = self
                    .staging
                    .pieces
                    .iter_mut()
                    .find(|p| p.name == piece.name && p.material == piece.material)
                {
                    found.geometry = piece.geometry;
                    self.kept = None;
                }
            }
            if self.kept.is_none() {
                self.staging.share();
            }
        }
        Ok(())
    }

    fn posed_at(&mut self, time: f64) -> Result<(), Fault> {
        self.recipe = pfx_build::pose_cameras(&self.base, &self.built, time)
            .map_err(|e| Fault::Failed(e.to_string()))?;
        let fps = self
            .base
            .timeline
            .map_or(pfx_build::Rate::whole(1), |t| t.fps);
        let posed = pfx_recipe_scene::poses(&self.built, fps, time);
        for (model, piece) in self.staging.posed.iter_mut().zip(&self.staging.pieces) {
            *model = posed.get(&piece.name).copied().unwrap_or(IDENTITY);
        }
        Ok(())
    }

    pub fn blurs(&self) -> bool {
        self.staging.motion_blur.is_some() && self.motion_steps > 0
    }

    pub fn pose_moving(&mut self, time: f64, camera: &str) -> Result<Camera, Fault> {
        let blur = self.staging.motion_blur.filter(|_| self.motion_steps > 0);
        let Some(blur) = blur else {
            self.pose(time)?;
            return Ok(pfx_recipe_scene::camera(&self.recipe, camera)?.clone());
        };
        let step = self.base.timeline.map_or(1.0, |t| t.step());
        let shutter =
            pfx_trace::motion::Shutter::around(time, step, f64::from(blur.shutter), blur.position);
        let steps = pfx_trace::motion::steps_of(self.motion_steps);
        let times = match self.base.timeline {
            Some(timeline) => shutter.times_held(steps, 0.0, timeline.seconds()),
            None => shutter.times(steps),
        };
        let mut posed = Vec::with_capacity(times.len());
        let mut cameras = Vec::with_capacity(times.len());
        let mut shapes: Vec<(usize, Vec<pfx_recipe_scene::Piece>)> = Vec::new();
        for &at in &times {
            self.posed_at(at)?;
            posed.push(self.staging.posed.clone());
            cameras.push(pfx_recipe_scene::camera(&self.recipe, camera)?.clone());
            if self.built.skinned() {
                for piece in pfx_recipe_scene::pieces(&self.built.skinned_parts(at)) {
                    let Some(index) = self
                        .staging
                        .pieces
                        .iter()
                        .position(|p| p.name == piece.name && p.material == piece.material)
                    else {
                        continue;
                    };
                    match shapes.iter_mut().find(|(found, _)| *found == index) {
                        Some((_, steps)) => steps.push(piece),
                        None => shapes.push((index, vec![piece])),
                    }
                }
            }
        }
        self.pose(time)?;
        shapes.retain(|(_, steps)| steps.len() == times.len());
        self.moving_cameras = cameras;
        self.shutter = times;
        self.staging.steps = Some(Steps {
            posed,
            shapes,
            cameras: Vec::new(),
        });
        Ok(pfx_recipe_scene::camera(&self.recipe, camera)?.clone())
    }

    pub fn spin(&mut self, object: usize, motion: Matrix) -> Result<(), Fault> {
        if object >= self.recipe.objects.len() {
            return Err(Fault::Failed(format!(
                "no object {object} to spin, the recipe has {}",
                self.recipe.objects.len()
            )));
        }
        let family = pfx_recipe_scene::family_of(&self.recipe, object);
        for (posed, piece) in self.staging.posed.iter_mut().zip(&self.staging.pieces) {
            if family.get(piece.object).copied().unwrap_or(false) {
                *posed = motion;
            }
        }
        Ok(())
    }

    pub fn frame(
        &mut self,
        camera: &Camera,
        take: Take,
        progress: &mut dyn FnMut(Step),
    ) -> Result<Rendered, Fault> {
        self.frame_kept(camera, take, progress, None)
    }

    pub fn frame_kept(
        &mut self,
        camera: &Camera,
        take: Take,
        progress: &mut dyn FnMut(Step),
        keeper: Option<&mut dyn Keeper>,
    ) -> Result<Rendered, Fault> {
        let Take { size, samples, .. } = take;
        if samples == 0 {
            return Err(Fault::Failed("a render needs at least one sample".into()));
        }
        progress(Step::Stage);
        let mut turn = turn_reporting;
        let lens = eyepiece(camera, View::Lens, size)?;
        let failed = |e: String| Fault::Failed(format!("the tracer: {e}"));
        let mut traced_cameras = Vec::with_capacity(self.moving_cameras.len());
        for moving in &self.moving_cameras {
            traced_cameras.push(eyepiece(moving, View::Lens, size)?.camera);
        }
        if let Some(steps) = self.staging.steps.as_mut() {
            steps.cameras = traced_cameras;
        }
        let puffs = self.puffs();
        let blur = self.sprite_steps(&puffs);
        let spray = Spray {
            steps: &blur,
            ..Spray::new(&puffs, lens.camera)
        };
        let moves = self.staging.steps.as_ref().is_some_and(Steps::moves) || spray.moves();
        let mut since = Builds::default();
        let mut restaged = None;
        let mut kept = match self.kept.take() {
            Some(mut kept)
                if kept.size == size
                    && kept.samples == samples
                    && kept.trace.moving() == moves
                    && kept.trace.sprite_quad().is_some() == spray.shown().next().is_some()
                    && kept.trace.sky_halvings()
                        == pfx_trace::gpu::sky_halvings(
                            &self.staging.sky,
                            &lens.camera,
                            lens.projection,
                            size,
                        ) =>
            {
                since = kept.trace.builds();
                let placements = self.staging.placements(&lens.eye);
                let models: Vec<Matrix> =
                    placements.iter().map(|placement| placement.model).collect();
                kept.trace.set_projection(lens.projection).map_err(failed)?;
                kept.trace.set_lens(lens.lens).map_err(failed)?;
                kept.trace.set_camera(lens.camera).map_err(failed)?;
                if let Some(fog) = self.staging.fog {
                    kept.trace.set_fog(fog).map_err(failed)?;
                }
                let motion: Option<(Vec<Vec<Matrix>>, &[pfx_trace::Camera])> =
                    self.staging.steps.as_ref().filter(|_| moves).map(|steps| {
                        let per_step: Vec<Vec<Matrix>> = steps
                            .posed
                            .iter()
                            .map(|posed| self.staging.models(posed, &lens.eye))
                            .collect();
                        let motion = (0..models.len())
                            .map(|piece| {
                                per_step
                                    .iter()
                                    .filter_map(|models| models.get(piece).copied())
                                    .collect()
                            })
                            .collect();
                        (motion, &steps.cameras[..])
                    });
                let shown = spray.shown().count();
                if shown > kept.trace.sprite_room() {
                    restaged = Some(format!(
                        "{shown} particles outgrew the trace's room for {}, so this frame staged a new trace",
                        kept.trace.sprite_room()
                    ));
                }
                match kept.trace.set_frame(
                    &self.gpu,
                    &models,
                    &spray,
                    motion
                        .as_ref()
                        .map(|(motion, cameras)| (&motion[..], *cameras)),
                ) {
                    Ok(()) => kept,
                    Err(_) => {
                        since = Builds::default();
                        self.restage(lens, size, samples, &spray)?
                    }
                }
            }
            _ => self.restage(lens, size, samples, &spray)?,
        };
        turn(Report::default());
        if self.denoise && self.denoiser.is_none() {
            self.denoiser = Some(denoiser(&self.gpu)?);
        }
        let seed = take.seed.of(self.seed);
        let denoiser = self.denoiser.as_mut().filter(|_| self.denoise);
        let shot = (size, seed, denoiser);
        let traced = match take.adaptive() {
            Some(adaptive) => adapted(
                &self.gpu,
                &mut kept.trace,
                shot,
                adaptive,
                progress,
                &mut turn,
                keeper,
            ),
            None => traced(
                &self.gpu,
                &mut kept.trace,
                shot,
                samples,
                progress,
                &mut turn,
                keeper,
            )
            .map(|(film, paced)| (film, paced, f64::from(samples), samples)),
        };
        let built = kept.trace.builds();
        self.kept = Some(kept);
        let (film, mut paced, mean_samples, most_samples) = traced?;
        let film = self.guided(film)?;
        paced.builds = Builds {
            bottom: built.bottom - since.bottom,
            top: built.top - since.top,
        };
        paced.planned = [self.plan.cpu(), self.plan.gpu()];
        Ok(Rendered {
            film,
            overlay: Overlay::of(&self.staging.text),
            finish: self
                .staging
                .finish
                .clone()
                .at(self.time, pfx_recipe_scene::seed(&self.base)),
            paced,
            seed,
            mean_samples,
            most_samples,
            region: Region::whole(size),
            rect: Region::whole(size).rect(),
            left_out: pfx_recipe_scene::unapplied(&self.recipe, true)
                .into_iter()
                .filter(|key| *key != "motion_blur" || self.staging.steps.is_none())
                .collect(),
            restaged,
        })
    }

    fn puffs(&self) -> Vec<pfx_trace::sprites::Puff> {
        crate::sprays::puffs(
            &self.emitted,
            &self.built,
            self.base
                .timeline
                .map_or(pfx_build::Rate::whole(1), |t| t.fps),
            self.time,
        )
    }

    fn sprite_steps(&self, puffs: &[pfx_trace::sprites::Puff]) -> Vec<f32> {
        if puffs.is_empty() || self.staging.steps.is_none() || self.shutter.len() < 2 {
            return Vec::new();
        }
        self.shutter
            .iter()
            .map(|at| (at - self.time) as f32)
            .collect()
    }

    fn guided(&self, mut film: Film) -> Result<Film, Fault> {
        let Some(kept) = self.kept.as_ref().filter(|_| self.guides) else {
            return Ok(film);
        };
        if film.albedo.is_empty() {
            let failed = |e: String| Fault::Failed(format!("the tracer's readback: {e}"));
            let read = |target| {
                self.gpu
                    .readback_bytes(target)
                    .and_then(|bytes| floats(&bytes))
                    .map_err(failed)
            };
            film.albedo = read(&kept.trace.albedo)?;
            film.normal = read(&kept.trace.normal)?;
        }
        Ok(film)
    }

    fn restage(
        &self,
        lens: crate::staging::Eyepiece,
        size: [u32; 2],
        samples: u32,
        spray: &Spray<'_>,
    ) -> Result<Kept, Fault> {
        let mut staged = self.staging.stage(lens)?;
        if !spray.steps.is_empty() && staged.detail.motion.is_none() {
            staged.detail.motion = Some(pfx_trace::motion::Motion {
                steps: self.shutter.len(),
                cameras: Vec::new(),
                curve: Vec::new(),
            });
        }
        staged.spray(spray).map_err(Fault::Failed)?;
        staged.detail.sprite_room = self.sprite_room;
        staged.detail.sampler = Sampler::Sobol { samples };
        staged.detail.fit_textures = Some(size);
        Ok(Kept {
            trace: Region::whole(size).open(&self.gpu, &staged, false)?,
            size,
            samples,
        })
    }
}

fn planned_size(request: &Request<'_>) -> [u32; 2] {
    match request.crop {
        Some(crop) => {
            let [_, _, width, height] = crop.rect;
            [
                (width + 2 * crop.apron).min(request.size[0]),
                (height + 2 * crop.apron).min(request.size[1]),
            ]
        }
        None => request.size,
    }
}

pub fn trace(
    gpu: &Gpu,
    staged: &Staged<Environment>,
    size: [u32; 2],
    samples: u32,
    seed: u32,
    progress: &mut dyn FnMut(Step),
    turn: &mut dyn FnMut(Report),
) -> Result<(Film, Paced), Fault> {
    trace_in(
        gpu,
        staged,
        Shot {
            region: Region::whole(size),
            seed,
            denoiser: None,
            transparent: false,
        },
        samples,
        progress,
        turn,
        None,
    )
}

pub struct Shot<'a> {
    pub region: Region,
    pub seed: u32,
    pub denoiser: Option<&'a mut Denoiser>,
    pub transparent: bool,
}

fn trace_in(
    gpu: &Gpu,
    staged: &Staged<Environment>,
    shot: Shot<'_>,
    samples: u32,
    progress: &mut dyn FnMut(Step),
    turn: &mut dyn FnMut(Report),
    keeper: Option<&mut dyn Keeper>,
) -> Result<(Film, Paced), Fault> {
    let Shot {
        region,
        seed,
        denoiser,
        transparent,
    } = shot;
    let mut trace = region.open(gpu, staged, transparent)?;
    traced(
        gpu,
        &mut trace,
        (region.size, seed, denoiser),
        samples,
        progress,
        turn,
        keeper,
    )
}

fn traced(
    gpu: &Gpu,
    trace: &mut Trace,
    (size, seed, denoiser): ([u32; 2], u32, Option<&mut Denoiser>),
    samples: u32,
    progress: &mut dyn FnMut(Step),
    turn: &mut dyn FnMut(Report),
    mut keeper: Option<&mut dyn Keeper>,
) -> Result<(Film, Paced), Fault> {
    let mut turns = Turns::new(turn);
    pfx_core::memory::look_gpu();
    progress(Step::Path(Box::new(trace.path(gpu))));
    let failed = |e: String| Fault::Failed(format!("the tracer's checkpoint: {e}"));
    let resumed = keeper.as_deref_mut().and_then(Keeper::resume);
    if let Some(saved) = resumed {
        if saved.adaptive.is_some() || saved.samples > samples {
            return Err(failed("it was made by another render".into()));
        }
        trace.restore(gpu, &saved).map_err(failed)?;
    }
    let from = trace.samples();
    let trace = RefCell::new(trace);
    let mut paced = sampled(
        (from, samples),
        &mut turns,
        progress,
        |pacer, between| {
            trace
                .borrow_mut()
                .sample_paced(gpu, 1, seed, pacer, between)
        },
        &mut |done| {
            let Some(keeper) = keeper.as_deref_mut() else {
                return Ok(());
            };
            if done >= samples || !keeper.due(done) {
                return Ok(());
            }
            let saved = trace.borrow().save(gpu).map_err(failed)?;
            keeper.keep(saved).map_err(failed)
        },
    )?;
    let trace = trace.into_inner();
    paced.trace_bytes = trace.gpu_bytes();
    let film = developed(gpu, trace, size, samples, denoiser)?;
    paced.gpu_resident = pfx_core::memory::peak().gpu;
    turns.now();
    Ok((film, paced))
}

pub fn converge(
    gpu: &Gpu,
    staged: &Staged<Environment>,
    shot: Shot<'_>,
    adaptive: Adaptive,
    progress: &mut dyn FnMut(Step),
    turn: &mut dyn FnMut(Report),
    keeper: Option<&mut dyn Keeper>,
) -> Result<(Film, Paced, f64, u32), Fault> {
    let Shot {
        region,
        seed,
        denoiser,
        transparent,
    } = shot;
    let mut trace = region.open(gpu, staged, transparent)?;
    adapted(
        gpu,
        &mut trace,
        (region.size, seed, denoiser),
        adaptive,
        progress,
        turn,
        keeper,
    )
}

fn adapted(
    gpu: &Gpu,
    trace: &mut Trace,
    (size, seed, denoiser): ([u32; 2], u32, Option<&mut Denoiser>),
    adaptive: Adaptive,
    progress: &mut dyn FnMut(Step),
    turn: &mut dyn FnMut(Report),
    keeper: Option<&mut dyn Keeper>,
) -> Result<(Film, Paced, f64, u32), Fault> {
    pfx_core::memory::look_gpu();
    progress(Step::Path(Box::new(trace.path(gpu))));
    let mut turns = Turns::new(turn);
    let mut pacer = Pacer::default();
    let cap = adaptive.max_samples;
    let pixels = u64::from(size[0]) * u64::from(size[1]);
    let watch = RefCell::new((Plan::new(&adaptive, pixels), 0.0f64, progress));
    let tell = |watch: &mut (Plan, f64, &mut dyn FnMut(Step))| {
        let (plan, gpu_ms, progress) = watch;
        progress(Step::Converge {
            gpu_ms: *gpu_ms as u64,
            cap,
            done: plan.done(*gpu_ms),
            total: plan.total(),
        });
    };
    turns.now();
    tell(&mut watch.borrow_mut());
    let converged = trace
        .sample_adaptive_kept(
            gpu,
            adaptive,
            seed,
            &mut pacer,
            |ms| {
                let mut watch = watch.borrow_mut();
                watch.1 += ms;
                tell(&mut watch);
                turns.soon(ms)
            },
            |round| {
                let mut watch = watch.borrow_mut();
                let gpu_ms = watch.1;
                watch.0.reach(round, gpu_ms);
                tell(&mut watch);
            },
            keeper,
        )
        .map_err(|e| Fault::Failed(format!("the tracer: {e}")))?;
    let mut paced = Paced {
        trace_bytes: trace.gpu_bytes(),
        ..Paced::default()
    };
    paced.add(&converged.stats);
    turns.now();
    let film = developed(gpu, trace, size, cap, denoiser)?;
    paced.gpu_resident = pfx_core::memory::peak().gpu;
    turns.now();
    Ok((
        film,
        paced,
        converged.mean_samples(),
        converged.max_samples(),
    ))
}

fn denoiser(gpu: &Gpu) -> Result<Denoiser, Fault> {
    Denoiser::new(gpu).map_err(|e| Fault::Failed(format!("the denoiser: {e}")))
}

fn developed(
    gpu: &Gpu,
    trace: &Trace,
    size: [u32; 2],
    samples: u32,
    denoiser: Option<&mut Denoiser>,
) -> Result<Film, Fault> {
    let denoised = denoiser
        .map(|denoiser| {
            denoiser.denoise_textures(
                gpu,
                &trace.color.texture,
                &trace.albedo.texture,
                &trace.normal.texture,
            )
        })
        .transpose()
        .map_err(|e| Fault::Failed(format!("the denoiser: {e}")))?;
    pfx_core::memory::look_gpu();
    let failed = |e: String| Fault::Failed(format!("the tracer's readback: {e}"));
    let (color, albedo, normal) = if denoised.is_some() {
        let color = trace.readback_color(gpu).map_err(failed)?;
        (floats(&color).map_err(failed)?, Vec::new(), Vec::new())
    } else {
        let output = trace.readback(gpu).map_err(failed)?;
        (
            floats(&output.color).map_err(failed)?,
            floats(&output.albedo).map_err(failed)?,
            floats(&output.normal).map_err(failed)?,
        )
    };
    Ok(Film {
        size,
        color,
        albedo,
        normal,
        denoised,
        samples,
        transparent: trace.matte().transparent,
    })
}

fn sampled(
    (from, samples): (u32, u32),
    turns: &mut Turns<'_>,
    progress: &mut dyn FnMut(Step),
    mut sample: impl FnMut(&mut Pacer, &mut dyn FnMut(f64) -> bool) -> Result<Stats, String>,
    sampled: &mut dyn FnMut(u32) -> Result<(), Fault>,
) -> Result<Paced, Fault> {
    let mut pacer = Pacer::default();
    let mut paced = Paced::default();
    turns.now();
    progress(Step::Trace {
        done: from,
        total: samples,
    });
    for done in from + 1..=samples {
        let stats = sample(&mut pacer, &mut |ms| turns.soon(ms))
            .map_err(|e| Fault::Failed(format!("the tracer: {e}")))?;
        paced.add(&stats);
        sampled(done)?;
        progress(Step::Trace {
            done,
            total: samples,
        });
    }
    turns.now();
    Ok(paced)
}

fn refused(error: TraceError) -> Fault {
    match error {
        TraceError::Refused(refusal) => Fault::Refused(format!(
            "this GPU is below the tracer's floor, so pfx render --recipe cannot trace on it: {refusal}"
        )),
        TraceError::Setup(message) => {
            Fault::Failed(format!("the tracer could not start: {message}"))
        }
    }
}

fn floats(bytes: &[u8]) -> Result<Vec<[f32; 4]>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len() / 16).map_err(|error| {
        format!(
            "a film of {} pixels needs {} bytes: {error}",
            bytes.len() / 16,
            bytes.len()
        )
    })?;
    out.extend(bytes.chunks_exact(16).map(|pixel| {
        std::array::from_fn(|k| {
            f32::from_le_bytes([
                pixel[k * 4],
                pixel[k * 4 + 1],
                pixel[k * 4 + 2],
                pixel[k * 4 + 3],
            ])
        })
    }));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_gpu::pace::Timing;
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    #[derive(Clone, Debug, PartialEq)]
    enum Event {
        Turn(f64),
        Sample,
        Done(u32),
    }

    fn slices(milliseconds: &[f64]) -> Stats {
        Stats {
            milliseconds: milliseconds.to_vec(),
            timings: vec![Timing::Gpu; milliseconds.len()],
            wall_ms: milliseconds.iter().sum(),
        }
    }

    fn run(samples: u32, slice_ms: &[f64]) -> (Vec<Event>, Paced) {
        let events = RefCell::new(Vec::new());
        let mut turn = |report: Report| events.borrow_mut().push(Event::Turn(report.work_ms));
        let mut turns = Turns {
            state: TurnState::new(50.0).with_wall_ms(1e9),
            turn: &mut turn,
        };
        let paced = sampled(
            (0, samples),
            &mut turns,
            &mut |step| {
                if let Step::Trace { done, .. } = step {
                    events.borrow_mut().push(Event::Done(done));
                }
            },
            |_, between| {
                events.borrow_mut().push(Event::Sample);
                for &ms in slice_ms {
                    between(ms);
                }
                Ok(slices(slice_ms))
            },
            &mut |_| Ok(()),
        )
        .unwrap();
        (events.into_inner(), paced)
    }

    #[test]
    fn the_loop_turns_after_staging_and_after_the_last_sample() {
        let (events, paced) = run(3, &[1.0, 2.0, 1.0]);
        assert_eq!(
            events,
            [
                Event::Turn(0.0),
                Event::Done(0),
                Event::Sample,
                Event::Done(1),
                Event::Sample,
                Event::Done(2),
                Event::Sample,
                Event::Done(3),
                Event::Turn(12.0),
            ]
        );
        assert_eq!(paced.submissions, 9);
        assert_eq!(paced.longest_ms, 2.0);
        assert_eq!(paced.gpu_ms, 12.0);
    }

    #[test]
    fn the_loop_turns_every_fifty_ms_of_slices_and_reports_the_rest_at_the_end() {
        let (events, _) = run(2, &[20.0, 20.0, 20.0]);
        let turns: Vec<f64> = events
            .iter()
            .filter_map(|event| match event {
                Event::Turn(ms) => Some(*ms),
                _ => None,
            })
            .collect();
        assert_eq!(turns, [0.0, 60.0, 60.0, 0.0]);
        assert_eq!(events.first(), Some(&Event::Turn(0.0)));
        assert_eq!(events.last(), Some(&Event::Turn(0.0)));
    }

    #[test]
    fn a_failed_sample_stops_the_loop_with_the_tracers_words() {
        let mut turned = 0;
        let mut turn = |_: Report| turned += 1;
        let mut turns = Turns {
            state: TurnState::new(50.0).with_wall_ms(1e9),
            turn: &mut turn,
        };
        let fault = sampled(
            (0, 4),
            &mut turns,
            &mut |_| {},
            |_, _| Err("lost".into()),
            &mut |_| Ok(()),
        )
        .err()
        .unwrap();
        assert_eq!(fault, Fault::Failed("the tracer: lost".into()));
        assert_eq!(turned, 1);
    }

    #[test]
    fn the_engines_wall_rule_turns_after_little_work_once_four_hundred_ms_have_passed() {
        let reports = RefCell::new(Vec::new());
        let mut turn = |report: Report| reports.borrow_mut().push(report);
        let mut state = TurnState::new(50.0);
        state.reset_at(Instant::now() - Duration::from_secs(1));
        let mut late = Turns {
            state,
            turn: &mut turn,
        };
        assert!(late.soon(1.5));
        let mut idle = Turns {
            state: TurnState::new(50.0).with_wall_ms(1e9),
            turn: &mut turn,
        };
        assert!(!idle.soon(1.5));
        assert!(!idle.soon(48.0));
        assert!(idle.soon(0.5));
        let reports = reports.into_inner();
        assert_eq!(
            reports,
            [
                Report {
                    work_ms: 1.5,
                    longest_ms: 1.5
                },
                Report {
                    work_ms: 50.0,
                    longest_ms: 48.0
                }
            ]
        );
    }

    #[test]
    fn the_plan_counts_pixel_samples_and_shrinks_as_pixels_stop() {
        let adaptive = Adaptive {
            threshold: 0.01,
            min_samples: 16,
            max_samples: 100,
            growth: 1.5,
        };
        let mut plan = Plan::new(&adaptive, 10);
        assert_eq!(plan.total(), 1000);
        assert_eq!(plan.done(5.0), 0);
        plan.reach(
            Round {
                samples: 8,
                active: 10,
            },
            8.0,
        );
        assert_eq!(plan.total(), 1000);
        assert_eq!(plan.done(8.0), 80);
        assert_eq!(plan.done(12.0), 120);
        assert_eq!(plan.done(1e6), 160);
        plan.reach(
            Round {
                samples: 16,
                active: 4,
            },
            16.0,
        );
        assert_eq!(plan.total(), 160 + 84 * 4);
        plan.reach(
            Round {
                samples: 24,
                active: 0,
            },
            19.0,
        );
        assert_eq!(plan.total(), 192);
        assert_eq!(plan.done(30.0), 192);
    }

    #[test]
    fn a_crop_grows_by_its_apron_inside_the_frame() {
        assert_eq!(
            Region::around([100, 50], [10, 40, 20, 10], 12),
            Region {
                full: [100, 50],
                origin: [0, 28],
                size: [42, 22],
            }
        );
        assert_eq!(
            Region::around([100, 50], [0, 0, 100, 50], 9),
            Region::whole([100, 50])
        );
        assert_eq!(
            Region::around([100, 50], [21, 18, 20, 10], 3).aligned(16),
            Region {
                full: [100, 50],
                origin: [16, 0],
                size: [32, 32],
            }
        );
        assert_eq!(
            Region::around([100, 50], [70, 20, 30, 10], 0)
                .aligned(16)
                .rect(),
            [64, 16, 36, 16]
        );
        assert!(Crop::fits([90, 0, 10, 50], [100, 50]));
        for rect in [[91, 0, 10, 1], [0, 0, 0, 1], [u32::MAX, 0, 2, 1]] {
            assert!(!Crop::fits(rect, [100, 50]), "{rect:?}");
        }
    }

    #[test]
    fn readback_bytes_become_pixels() {
        let bytes: Vec<u8> = [1.0f32, 0.5, -2.0, 1.0, 0.0, 3.25, 7.0, 1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            floats(&bytes).unwrap(),
            [[1.0, 0.5, -2.0, 1.0], [0.0, 3.25, 7.0, 1.0]]
        );
    }

    #[test]
    fn a_refusal_is_told_in_plain_words() {
        let refusal = pfx_gpu::floor::GpuRefusal {
            adapter: None,
            shortfall: pfx_gpu::floor::GpuShortfall::NoAdapter,
        };
        let Fault::Refused(message) = refused(TraceError::Refused(refusal)) else {
            panic!("a refusal stays a refusal");
        };
        assert!(message.contains("below the tracer's floor"), "{message}");
        assert_eq!(
            refused(TraceError::Setup("bad".into())),
            Fault::Failed("the tracer could not start: bad".into())
        );
    }

    #[test]
    fn a_render_without_samples_or_a_recipe_fails_before_the_gpu() {
        let mut steps = Vec::new();
        let request = Request {
            recipe: Path::new("missing.toml"),
            camera: "main",
            size: [8, 8],
            samples: 0,
            noise: None,
            seed: None,
            view: View::Lens,
            variant: None,
            set: &[],
            pick: &pfx_recipe::Pick::default(),
            crop: None,
            denoise: false,
            time: None,
            transparent: false,
        };
        assert!(render(&request, &Registry::new(), &mut |s| steps.push(s)).is_err());
        assert!(steps.is_empty());
        let request = Request {
            samples: 4,
            ..request
        };
        assert!(render(&request, &Registry::new(), &mut |s| steps.push(s)).is_err());
        assert_eq!(steps, [Step::Stage]);
    }
}
