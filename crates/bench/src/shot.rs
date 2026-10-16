use pfx_gpu::pace::{Pace, WallClock, gpu_ms};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::shadow::Quality as Shadows;
use pfx_load::scene::{Camera, Matrix, Physics, Projection, Scene, multiply};
use pfx_play::{ObjectId, Options, PlaySession, SceneGame, World};

use crate::game::placed;
use crate::run::target;

pub const WARM_FRAMES: u64 = 8;
pub const DRAFT_SHADOW_MIN: u32 = 512;
pub const LEAD_SECONDS: f64 = 1e-6;
pub const MAX_STEPS: u32 = 64;
pub const CAMERA_PREFIX: &str = "camera.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Draft,
    Final,
}

impl Quality {
    pub fn name(self) -> &'static str {
        match self {
            Quality::Draft => "draft",
            Quality::Final => "final",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lens {
    pub fov: Option<f32>,
    pub near: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Follow {
    pub name: String,
    pub lens: Lens,
}

#[derive(Clone, Debug)]
pub struct Shot<'a> {
    pub scene: &'a Scene,
    pub size: (u32, u32),
    pub fps: u32,
    pub from: u64,
    pub frames: u64,
    pub camera: Option<Follow>,
    pub quality: Quality,
    pub seed: u64,
    pub shutter: Option<f32>,
    pub focus_keys: Vec<[f32; 2]>,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub index: u64,
    pub number: u64,
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
    pub gpu_ms: Option<f64>,
    pub camera: ([f32; 3], [f32; 3]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Played {
    pub tick_rate: f64,
    pub shadow_resolution: u32,
    pub warm_frames: u64,
    pub adapter: String,
    pub followed: Option<String>,
}

fn whole(value: f64) -> bool {
    (value - value.round()).abs() < 1e-4
}

pub fn tick_rate(scene_rate: f64, fps: u32, shutter: Option<f32>) -> f64 {
    let fps = f64::from(fps.max(1));
    let floor = (scene_rate / fps).ceil().max(1.0) as u32;
    let fits = |steps: u32| shutter.is_none_or(|open| whole(f64::from(open) * f64::from(steps)));
    let steps = (floor..=floor.max(MAX_STEPS))
        .find(|steps| fits(*steps))
        .unwrap_or(floor);
    fps * f64::from(steps)
}

pub fn draft_shadows(quality: Shadows) -> Shadows {
    Shadows {
        resolution: (quality.resolution / 2)
            .max(DRAFT_SHADOW_MIN)
            .min(quality.resolution),
        ..quality
    }
}

pub fn warm_frames(quality: Quality, from: u64, adapts: bool) -> u64 {
    match (adapts, quality) {
        (true, _) => from,
        (false, Quality::Draft) => 0,
        (false, Quality::Final) => WARM_FRAMES.min(from),
    }
}

enum Anchor {
    Object(ObjectId),
    Node(ObjectId, usize),
}

fn candidates(name: &str) -> Vec<String> {
    let mut names = vec![name.to_string()];
    if !name.starts_with(CAMERA_PREFIX) {
        names.push(format!("{CAMERA_PREFIX}{name}"));
    }
    names
}

fn anchor(scene: &Scene, world: &World, name: &str) -> Option<(Anchor, String)> {
    let names = candidates(name);
    for wanted in &names {
        if let Some(id) = world.object(wanted) {
            return Some((Anchor::Object(id), wanted.clone()));
        }
    }
    for wanted in &names {
        for object in &scene.objects {
            let Some(id) = world.object(&object.name) else {
                continue;
            };
            let Some(animator) = world.animator(id) else {
                continue;
            };
            if let Some(joint) = animator
                .rig()
                .skeleton()
                .joints()
                .iter()
                .position(|joint| &joint.name == wanted)
            {
                return Some((Anchor::Node(id, joint), format!("{}/{wanted}", object.name)));
            }
        }
    }
    None
}

fn anchored(world: &World, anchor: &Anchor) -> Matrix {
    match *anchor {
        Anchor::Object(id) => world.model(id),
        Anchor::Node(id, joint) => {
            let model = world.model(id);
            match world
                .palette(id)
                .and_then(|palette| palette.get(joint).copied())
            {
                Some(node) => multiply(model, node),
                None => model,
            }
        }
    }
}

fn aimed(base: Camera, matrix: Matrix, lens: &Lens) -> Camera {
    let mut camera = placed(matrix, base);
    if let Some(fov) = lens.fov {
        camera.projection = Projection::Perspective { fov };
    }
    if let Some(near) = lens.near {
        camera.near = near;
    }
    camera
}

pub fn focus_at(keys: &[[f32; 2]], time: f32) -> Option<f32> {
    let first = keys.first()?;
    if time <= first[0] {
        return Some(first[1]);
    }
    for pair in keys.windows(2) {
        let ([t0, m0], [t1, m1]) = (pair[0], pair[1]);
        if time <= t1 {
            let span = t1 - t0;
            let u = if span > 0.0 { (time - t0) / span } else { 1.0 };
            return Some(m0 + (m1 - m0) * u);
        }
    }
    keys.last().map(|last| last[1])
}

fn advance_to(session: &mut PlaySession, seconds: f64) {
    let now = session.world().clock().time();
    let real = (seconds - now) as f32;
    if real > 0.0 {
        session.advance(real);
    }
}

pub fn play(
    shot: &Shot<'_>,
    mut each: impl FnMut(Frame) -> Result<(), String>,
) -> Result<Played, String> {
    let (width, height) = shot.size;
    let fps = shot.fps.max(1);
    let mut scene = shot.scene.clone();
    let physics = scene.physics_or_default();
    let rate = tick_rate(f64::from(physics.rate), fps, shot.shutter);
    scene.physics = Some(Physics {
        rate: rate as f32,
        ..physics
    });

    let gpu = pollster::block_on(Gpu::headless())?;
    let limit = gpu.device.limits().max_texture_dimension_2d;
    if width > limit || height > limit {
        return Err(format!(
            "{width}x{height} is over this GPU's {limit} pixel texture limit"
        ));
    }
    let adapter = gpu.adapter.get_info().name;
    let output = target(&gpu, width, height);
    let mut renderer =
        Renderer::new(gpu.clone(), width, height).map_err(|error| error.to_string())?;
    let staged = renderer.stage(&scene)?;
    renderer.wait_sky()?;
    let shadows = match shot.quality {
        Quality::Draft => draft_shadows(renderer.shadows().quality()),
        Quality::Final => renderer.shadows().quality(),
    };
    if shadows != renderer.shadows().quality() {
        renderer.set_shadow_quality(shadows);
    }
    let mut session = PlaySession::play_with(
        &scene,
        Some(&staged),
        Box::new(SceneGame),
        Options {
            seed: shot.seed,
            max_ticks: u32::MAX,
            ..Options::default()
        },
    )
    .map_err(|error| error.to_string())?;

    let base = session.world().camera;
    let follow = match &shot.camera {
        Some(follow) => {
            let (found, named) =
                anchor(&scene, session.world(), &follow.name).ok_or_else(|| {
                    format!(
                        "{}: no object or animated node named {:?} to follow with the camera",
                        scene.path.display(),
                        follow.name
                    )
                })?;
            Some((found, named, follow.lens.clone()))
        }
        None => None,
    };
    let place = |session: &mut PlaySession, time: f64| {
        if let Some((found, _, lens)) = &follow {
            let matrix = anchored(session.world(), found);
            session.world_mut().camera = aimed(base, matrix, lens);
        }
        let camera = &mut session.world_mut().camera;
        if let (Some(focus), Some(field)) = (
            focus_at(&shot.focus_keys, time as f32),
            camera.depth_of_field.as_mut(),
        ) {
            field.distance = focus;
        }
    };

    let adapts = matches!(renderer.exposure_mode(), Exposure::Auto { .. });
    let warm = warm_frames(shot.quality, shot.from, adapts);
    let first = shot.from - warm;
    let last = shot.from + shot.frames;
    let aspect = width as f32 / height as f32;
    let seed = shot.seed as u32;
    let finish = staged.finish();
    let mut pace = Pace::with_clock(WallClock::new(), None);
    for number in 0..last {
        let time = number as f64 / f64::from(fps);
        if number >= first
            && let Some(open) = shot.shutter
        {
            let before = time - f64::from(open) / f64::from(fps);
            if before + LEAD_SECONDS > session.world().clock().time() {
                advance_to(&mut session, before + LEAD_SECONDS);
                place(&mut session, before);
                session
                    .present(&staged, &mut renderer, aspect, seed)
                    .map_err(|error| error.to_string())?;
            }
        }
        advance_to(&mut session, time + LEAD_SECONDS);
        place(&mut session, time);
        if let Some(exit) = session.world().exit() {
            return Err(format!("the scene stopped at frame {number}: {exit:?}"));
        }
        if number < first {
            continue;
        }
        let drawn = session
            .present(&staged, &mut renderer, aspect, seed)
            .map_err(|error| error.to_string())?;
        if renderer.sky_settling() {
            renderer.wait_sky()?;
        }
        let view = &output.view;
        let renderer = &mut renderer;
        let mut failed = None;
        let passes = pace.frame(|| {
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
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("{error:?}"))?;
        if number < shot.from {
            continue;
        }
        let camera = session.world().camera;
        let pixels = gpu.readback_rgba16(&output)?;
        each(Frame {
            index: number - shot.from,
            number,
            time,
            width,
            height,
            pixels,
            gpu_ms: gpu_ms(&passes),
            camera: (camera.at, camera.look_at),
        })?;
    }
    session.stop();
    Ok(Played {
        tick_rate: rate,
        shadow_resolution: shadows.resolution,
        warm_frames: warm,
        adapter,
        followed: follow.map(|(_, named, _)| named),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_and_every_shutter_opening_lands_on_a_tick() {
        assert_eq!(tick_rate(60.0, 60, None), 60.0);
        assert_eq!(tick_rate(60.0, 30, None), 60.0);
        assert_eq!(tick_rate(60.0, 24, None), 72.0);
        assert_eq!(tick_rate(60.0, 60, Some(0.5)), 120.0);
        assert_eq!(tick_rate(60.0, 24, Some(0.5)), 96.0);
        assert_eq!(tick_rate(120.0, 60, Some(0.25)), 240.0);
        assert_eq!(focus_at(&[[0.0, 4.0], [2.0, 8.0]], 1.0), Some(6.0));
        assert_eq!(focus_at(&[[1.0, 4.0], [2.0, 8.0]], 0.0), Some(4.0));
        assert_eq!(focus_at(&[[1.0, 4.0], [2.0, 8.0]], 3.0), Some(8.0));
        assert_eq!(focus_at(&[], 1.0), None);
        assert_eq!(warm_frames(Quality::Draft, 300, true), 300);
        assert_eq!(warm_frames(Quality::Final, 300, true), 300);
        assert_eq!(warm_frames(Quality::Draft, 300, false), 0);
        assert_eq!(warm_frames(Quality::Final, 300, false), WARM_FRAMES);
        assert_eq!(warm_frames(Quality::Final, 3, false), 3);
    }
}
