use crate::pace::{TurnState, turn_soon};
use pfx_build::{Built, Part, Registry};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_recipe::{Camera, Recipe};
use std::fmt;
use std::path::Path;

mod cards;
mod eye;
mod light;
mod maps;
mod memory;
pub mod pace;
mod parts;
mod picture;
mod stage;
mod text;
mod vector;

pub use cards::{
    Card, Cards, Face, Picture, clips, decode, facing, layered, layered_as, linear, placeholder,
    premultiplied,
};
pub use light::{Lighting, Lit, Shine};
pub use maps::Set as MapSet;
pub use memory::Memory;
pub use parts::{Palette, Piece, glb_parts, material, palette, palette_with, pieces};
use pfx_live::frame::Matrix;
pub use pfx_live::frame::MeshStats;
pub use pfx_post::quantise::Dither;
pub use picture::{
    difference, psnr, read_png, read_png_shape, read_png_size, write_frame, write_png, write_wide,
};
pub use stage::{Change, Outline, Pick, Prepared, Stage, family_of};
pub use text::{Laid, fonts};

pub const SETTLE_FRAMES: u32 = 48;
pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
pub const FRAME_RATE: f32 = 60.0;
pub const UNMEASURED_MS: f64 = 50.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eye {
    pub position: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub fov_y_deg: f64,
    pub shift: [f64; 2],
    pub ortho_height: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub fn still(recipe: &Path, camera: &str, size: [u32; 2]) -> Result<Vec<u8>, Error> {
    still_with(recipe, camera, size, None, &Registry::new())
}

pub fn still_with(
    recipe: &Path,
    camera: &str,
    size: [u32; 2],
    variant: Option<&str>,
    registry: &Registry,
) -> Result<Vec<u8>, Error> {
    let (recipe, built) = load(recipe, variant, &[], registry)?;
    let gpu = headless()?;
    shoot(&gpu, &recipe, &built.parts, camera, size)
}

pub fn load(
    path: &Path,
    variant: Option<&str>,
    set: &[String],
    registry: &Registry,
) -> Result<(Recipe, Built), Error> {
    load_at(
        path,
        variant,
        set,
        &pfx_recipe::Pick::default(),
        registry,
        None,
    )
}

pub fn poses(built: &Built, fps: u32, time: f64) -> std::collections::BTreeMap<String, Matrix> {
    let mut posed = std::collections::BTreeMap::new();
    for track in built.tracks.iter().filter(|track| track.camera.is_none()) {
        if track.keys.is_empty() {
            continue;
        }
        let world = pfx_build::matrix(&pfx_build::sample(&track.keys, fps, time));
        for part in built
            .parts
            .iter()
            .filter(|part| part.node.name == track.node)
        {
            let model = if part.node.keep_transform {
                world * part.node.transform.inverse()
            } else {
                world
            };
            let model = model
                .to_cols_array_2d()
                .map(|column| column.map(|v| v as f32));
            posed.insert(part.node.name.clone(), model);
        }
    }
    posed
}

pub fn posed(recipe: &Recipe, built: &Built, time: f64) -> Result<(Recipe, Built), Error> {
    pfx_build::pose(recipe, built, time).map_err(|e| Error(e.to_string()))
}

pub fn load_at(
    path: &Path,
    variant: Option<&str>,
    set: &[String],
    pick: &pfx_recipe::Pick,
    registry: &Registry,
    time: Option<f64>,
) -> Result<(Recipe, Built), Error> {
    let failed = |e: pfx_recipe::Error| Error(e.to_string());
    let loaded = pfx_recipe::load(path).map_err(failed)?;
    let recipe = match variant {
        Some(name) => pfx_recipe::apply_variant(&loaded.recipe, name).map_err(failed)?,
        None => loaded.recipe,
    };
    let recipe = pfx_recipe::apply_set(&recipe, set).map_err(failed)?;
    let recipe = pick.apply(&recipe).map_err(failed)?;
    let base = path.parent().unwrap_or(Path::new(""));
    let built = pfx_build::build_with(&recipe, base, registry).map_err(|e| Error(e.to_string()))?;
    let mut recipe = built.recipe(&recipe);
    for (name, wet) in built.wet() {
        if recipe.effects.wet.contains_key(name) {
            continue;
        }
        let settings = [
            ("amount", wet.amount),
            ("puddles", wet.puddles),
            ("puddle_scale", wet.puddle_scale),
        ]
        .into_iter()
        .filter_map(|(key, v)| v.map(|v| (key, pfx_recipe::Setting::Number(f64::from(v)))))
        .chain(
            wet.seed
                .map(|seed| ("seed", pfx_recipe::Setting::Whole(seed))),
        )
        .collect();
        recipe.effects.wet.insert(name.to_string(), settings);
    }
    match time {
        Some(time) => posed(&recipe, &built, time),
        None => Ok((recipe, built)),
    }
}

pub fn seed(recipe: &Recipe) -> u32 {
    (recipe.seed & u64::from(u32::MAX)) as u32
}

pub fn headless() -> Result<Gpu, Error> {
    block_on(Gpu::headless()).map_err(|e| Error(format!("no GPU: {e}")))
}

pub fn camera<'a>(recipe: &'a Recipe, name: &str) -> Result<&'a Camera, Error> {
    recipe
        .cameras
        .iter()
        .find(|camera| camera.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = recipe.cameras.iter().map(|c| c.name.as_str()).collect();
            Error(if known.is_empty() {
                format!("camera.{name}: the recipe has no cameras")
            } else {
                format!(
                    "camera.{name}: no such camera, expected one of: {}",
                    known.join(", ")
                )
            })
        })
}

pub fn shoot(
    gpu: &Gpu,
    recipe: &Recipe,
    parts: &[Part],
    camera_name: &str,
    size: [u32; 2],
) -> Result<Vec<u8>, Error> {
    let lens = camera(recipe, camera_name)?;
    let mut reel = Reel::new(gpu, recipe, parts, size)?;
    let eye = reel.through(lens);
    reel.shoot(&eye).map(|(rgba, _)| rgba)
}

pub fn unapplied(recipe: &Recipe, traced: bool) -> Vec<&'static str> {
    let graded = recipe
        .look
        .as_ref()
        .is_some_and(|look| look.view != pfx_recipe::View::Standard && look.grade.haze > 0.0);
    let blurred = recipe
        .effects
        .motion_blur
        .as_ref()
        .is_some_and(|settings| light::motion_blur(settings).active());
    [
        ("motion_blur", traced && blurred),
        ("look.grade.haze", traced && graded),
        ("haze", recipe.effects.haze.is_some()),
        ("particles", !recipe.effects.particles.is_empty()),
    ]
    .into_iter()
    .filter_map(|(key, left)| left.then_some(key))
    .collect()
}

pub fn unapplied_line(verb: &str, recipe: &Recipe, traced: bool) -> Option<String> {
    left_out_line(verb, &unapplied(recipe, traced), traced)
}

pub fn left_out_line(verb: &str, keys: &[&str], traced: bool) -> Option<String> {
    (!keys.is_empty()).then(|| {
        format!(
            "pfx {verb}: the {} renderer leaves out {}",
            if traced { "traced" } else { "live" },
            keys.join(", ")
        )
    })
}

pub fn shoot_posed(
    gpu: &Gpu,
    (recipe, built): (&Recipe, &Built),
    camera_name: &str,
    size: [u32; 2],
    time: Option<f64>,
) -> Result<Vec<u8>, Error> {
    shoot_posed_as(
        gpu,
        (recipe, built),
        camera_name,
        size,
        time,
        Output::default(),
    )
}

pub fn shoot_posed_as(
    gpu: &Gpu,
    (recipe, built): (&Recipe, &Built),
    camera_name: &str,
    size: [u32; 2],
    time: Option<f64>,
    output: Output,
) -> Result<Vec<u8>, Error> {
    let lens = camera(recipe, camera_name)?;
    let mut reel = Reel::with_output(gpu, recipe, &built.parts, size, output)?;
    if let (Some(time), Some(timeline)) = (time, recipe.timeline)
        && reel.blurs()
    {
        let before = time - 1.0 / f64::from(timeline.fps.max(1));
        reel.set_poses(poses(built, timeline.fps, before));
        if built.skinned() {
            reel.reshape(&built.skinned_parts(before))?;
        }
        let early =
            pfx_build::pose_cameras(recipe, built, before).map_err(|e| Error(e.to_string()))?;
        let eye = reel.through(camera(&early, camera_name)?);
        reel.settle(&eye)?;
        reel.set_poses(std::collections::BTreeMap::new());
        if built.skinned() {
            reel.reshape_from(&built.skinned_parts(time), &built.skinned_parts(before))?;
        }
        let eye = reel.through(lens);
        return reel.develop(&eye).map(|(rgba, _)| rgba);
    }
    let eye = reel.through(lens);
    reel.shoot(&eye).map(|(rgba, _)| rgba)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub wide: bool,
    pub dither: Option<pfx_post::quantise::Dither>,
}

impl Output {
    pub fn bytes_per_pixel(&self) -> usize {
        if self.wide { 8 } else { 4 }
    }
}

fn wide_target(gpu: &Gpu, size: [u32; 2]) -> OffscreenTarget {
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("recipe reel wide output"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
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
        format,
        width: size[0],
        height: size[1],
    }
}

pub struct Reel {
    stage: Stage,
    target: OffscreenTarget,
    turns: TurnState,
    size: [u32; 2],
    output: Output,
}

impl Reel {
    pub fn new(gpu: &Gpu, recipe: &Recipe, parts: &[Part], size: [u32; 2]) -> Result<Reel, Error> {
        Reel::with_output(gpu, recipe, parts, size, Output::default())
    }

    pub fn with_output(
        gpu: &Gpu,
        recipe: &Recipe,
        parts: &[Part],
        size: [u32; 2],
        output: Output,
    ) -> Result<Reel, Error> {
        if size[0] == 0 || size[1] == 0 {
            return Err(Error(format!("the size {}x{} is empty", size[0], size[1])));
        }
        let format = if output.wide {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        };
        let mut stage = Stage::made_as(gpu.clone(), Prepared::of(recipe, parts)?, size, format)?;
        stage.blur_motion(true);
        if !output.wide {
            stage.set_output_dither(output.dither)?;
        }
        let target = if output.wide {
            wide_target(gpu, size)
        } else {
            gpu.offscreen(size[0], size[1], format).map_err(Error)?
        };
        Ok(Reel {
            stage,
            target,
            turns: TurnState::default(),
            size,
            output,
        })
    }

    pub fn output(&self) -> Output {
        self.output
    }

    pub fn pose(&mut self, recipe: &Recipe, parts: &[Part]) -> Result<(), Error> {
        self.stage.apply(Prepared::of(recipe, parts)?).map(|_| ())
    }

    pub fn preview(&mut self, object: usize, motion: Matrix) -> Result<(), Error> {
        self.stage.preview(object, motion)
    }

    pub fn set_poses(&mut self, posed: std::collections::BTreeMap<String, Matrix>) -> bool {
        self.stage.set_poses(posed)
    }

    pub fn reshape(&mut self, parts: &[Part]) -> Result<usize, Error> {
        self.stage.reshape(parts)
    }

    pub fn reshape_from(&mut self, parts: &[Part], before: &[Part]) -> Result<usize, Error> {
        self.stage.reshape_from(parts, before)
    }

    pub fn set_lights(&mut self, lights: &[pfx_recipe::Light]) -> Result<bool, Error> {
        self.stage.set_lights(lights)
    }

    pub fn eye(&self, camera: &Camera) -> Eye {
        Eye::from_camera(camera, f64::from(self.size[0]) / f64::from(self.size[1]))
    }

    pub fn through(&mut self, camera: &Camera) -> Eye {
        self.stage.look_through(Some(camera));
        self.eye(camera)
    }

    pub fn blurs(&self) -> bool {
        self.stage.blurs_motion()
    }

    pub fn settle(&mut self, eye: &Eye) -> Result<f64, Error> {
        let mut gpu_ms = 0.0;
        for frame in 0..SETTLE_FRAMES {
            gpu_ms += self.once(eye, frame)?;
        }
        Ok(gpu_ms)
    }

    pub fn shoot(&mut self, eye: &Eye) -> Result<(Vec<u8>, f64), Error> {
        let gpu_ms = self.settle(eye)?;
        self.read(gpu_ms)
    }

    pub fn develop(&mut self, eye: &Eye) -> Result<(Vec<u8>, f64), Error> {
        let gpu_ms = self.once(eye, SETTLE_FRAMES)?;
        self.read(gpu_ms)
    }

    fn once(&mut self, eye: &Eye, frame: u32) -> Result<f64, Error> {
        let measured =
            self.stage
                .render_measured(eye, frame as f32 / FRAME_RATE, &self.target.view)?;
        turn_soon(&mut self.turns, measured.unwrap_or(UNMEASURED_MS));
        Ok(measured.unwrap_or(0.0))
    }

    fn read(&mut self, gpu_ms: f64) -> Result<(Vec<u8>, f64), Error> {
        let gpu = self.stage.gpu();
        let bytes = if self.output.wide {
            let halves = gpu.readback_rgba16(&self.target).map_err(Error)?;
            halves
                .iter()
                .flat_map(|&h| pfx_post::quantise::wide(pfx_post::quantise::half(h)).to_be_bytes())
                .collect()
        } else {
            gpu.readback_rgba8(&self.target).map_err(Error)?
        };
        Ok((bytes, gpu_ms))
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::yield_now();
    }
}
