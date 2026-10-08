mod budget;
mod cold;
mod detail;
#[path = "../manners/mod.rs"]
mod manners;
mod occlusion;
mod paint;
mod physics;
mod recipe;
mod reference;
mod room;
mod shadow_parts;
mod terms;
mod volume;
mod words;

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_bake::impostor::{PropSpec, bake_prop, bounds};
use pfx_bake::reflection::{
    ReflectionSpec, ReflectionWrite, bake_reflection, read_reflection_artifact,
    write_reflection_artifact,
};
use pfx_bake::{
    Anchor, Artifact, BakeScene, GridSpec, SceneSource, bake, read_artifact, scene_hash,
    write_artifact,
};
use pfx_core::camera::{
    DEG, Depth, Director, Drift, Harmonic, Hush, Level, Look, Nudge, Orbit, Pose, Preset, Redraw,
    Wander, Zoom,
};
use pfx_core::motion::Rest;
use pfx_geom::strip::{Roller, Strip};
use pfx_geom::tree::{Tree, TreeSpec};
use pfx_gpu::stats::FrameStats;
use pfx_gpu::{Gpu, OffscreenTarget, PassTiming, wgpu};
use pfx_live::canopy::CanopyMode;
use pfx_live::deform::{Deformer, DeformerId, RollerFrame, WindFrame};
use pfx_live::effects::{
    CreatureInstance, CreatureMesh, CreatureVertex, ParticleInstance, Plume, RoomHaze, VolumeKind,
};
use pfx_live::frame::{
    self, Camera, Instance, InstanceSurface, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun,
};
use pfx_live::glass::{self, Surface};
use pfx_live::impostor::{ImpostorInstance, ImpostorPass};
use pfx_live::maps::Caustic;
use pfx_live::probes::ProbeLighting;
use pfx_live::renderer::{Effects, Exposure, Finish, Liquid, Renderer, Volume};
use pfx_live::shadow::{Quality, ReceiverBox};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::{Content, ContentLayer, Material};
use pfx_post::Style;
use pfx_trace::bvh::Triangle;

use room::{Cast, Room};

pub const WIDTH: u32 = 3840;
pub const HEIGHT: u32 = 2160;
pub const DECK: (u32, u32) = (1280, 800);
pub const PLATE: (u32, u32) = DECK;
const TIMED_FRAMES: u32 = 600;
pub const SETTLE_FRAMES: u32 = 48;
const WARM_FRAMES: u32 = 120;
const FINISH_FRAMES: u32 = 240;
pub const HOUR: f32 = 16.0;
pub const ANCHORS: [f32; 3] = [12.0, 16.0, 19.0];
pub const SPACING: f32 = 0.3;
pub const SAMPLES: u32 = 64;
pub const SEED: u32 = 29;
pub const NEAR: f32 = 0.05;
pub const FAR: f32 = 40.0;
const EXPOSURE: f32 = 7.5;
const SUN_RADIUS_DEG: f32 = 0.27;
const POSES: usize = 4096;
const VIDEO_RATE: f32 = 30.0;
pub const RECEIVER: ([f32; 3], [f32; 3]) = ([-2.3, 0.0, -2.2], [2.7, 2.4, 0.3]);
const STRIP: Strip = Strip {
    length: 0.5,
    width: 0.24,
};
const CORE: f32 = 0.012;
const THICKNESS: f32 = 0.0015;
const ROLL_AT: [f32; 3] = [-0.6, room::TABLE_TOP + 0.001, -0.5];
const TANK: [f32; 3] = room::VESSEL;
const TANK_HALF: [f32; 3] = [0.1, 0.08, 0.075];
const WATER: f32 = 0.11;

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = dot(v, v).sqrt().max(1e-20);
    v.map(|x| x / n)
}

pub fn rotate(v: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    let d = dot(axis, v);
    let x = cross(axis, v);
    std::array::from_fn(|k| v[k] * c + x[k] * s + axis[k] * d * (1.0 - c))
}

pub fn point(m: &Matrix, p: [f32; 3]) -> [f32; 3] {
    let w = frame::transform(*m, [p[0], p[1], p[2], 1.0]);
    [w[0], w[1], w[2]]
}

fn harmonic(weight: f32, rate: f32, phase: f32) -> Harmonic {
    Harmonic {
        weight,
        rate,
        phase,
    }
}

pub fn preset() -> Preset {
    Preset {
        gain: 2.0,
        orbit: Orbit {
            stiffness: 5.0,
            yaw: 0.5 * DEG,
            pitch: 0.3 * DEG,
            hold: 2.0,
        },
        look: Look {
            stiffness: 3.0,
            lean: 0.15,
            yaw: 0.8 * DEG,
            pitch: 0.5 * DEG,
            sign: 1.0,
        },
        zoom: Zoom {
            stiffness: 3.0,
            hold: 6.0,
            max: 1.2,
            rate: 1.05,
        },
        wander: Wander {
            idle: 20.0,
            every: 30.0,
            hold: 4.0,
        },
        hush: Some(Hush {
            floor: 0.3,
            stiffness: 3.0,
            blocks_cursor: true,
        }),
        depth: Some(Depth {
            focus: 4.0,
            blur: 0.0,
            floor: 0.5,
        }),
        nudge: Nudge {
            stiffness: 50.0,
            pitch: 0.05 * DEG,
            hold: 0.12,
            floor: 0.3,
        },
        drift: Drift {
            yaw: [
                harmonic(0.6, 0.11, 0.0),
                harmonic(0.3, 0.27, 1.3),
                harmonic(0.1, 0.61, 2.9),
            ],
            yaw_gain: 0.9 * DEG,
            pitch: [
                harmonic(0.6, 0.09, 0.7),
                harmonic(0.3, 0.23, 2.1),
                harmonic(0.1, 0.53, 4.0),
            ],
            pitch_gain: 0.45 * DEG,
            roll: harmonic(1.0, 0.07, 0.4),
            roll_gain: 0.05 * DEG,
            offset: harmonic(1.0, 0.05, 1.9),
            offset_gain: 0.01,
            calmed: false,
            quiet_roll: true,
        },
        redraw: Redraw::Always,
        rest: Rest::COARSE,
    }
}

fn poses() -> Vec<(f32, f32, f32)> {
    let mut director = Director::new(Level::Full, preset(), 4.0, 0.0);
    (0..POSES)
        .map(|frame| {
            let pose: Pose = director.advance_to(frame as f32 / 60.0);
            (pose.yaw, pose.pitch, pose.zoom)
        })
        .collect()
}

pub struct Lens {
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub fov_y: f32,
    pub shift: [f32; 2],
    poses: Vec<(f32, f32, f32)>,
}

impl Default for Lens {
    fn default() -> Self {
        Self {
            position: [0.9, 1.5, 2.4],
            target: [-0.9, 0.95, -1.0],
            fov_y: 52.0,
            shift: [0.0, 0.0],
            poses: poses(),
        }
    }
}

impl Lens {
    pub fn view(&self, yaw: f32, pitch: f32) -> ([f32; 3], Matrix) {
        let forward = unit(sub(self.target, self.position));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let arm = sub(self.position, self.target);
        let arm = rotate(rotate(arm, up, yaw), right, pitch);
        let eye = [0, 1, 2].map(|k| self.target[k] + arm[k]);
        let forward = unit(sub(self.target, eye));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let view = [
            [right[0], up[0], -forward[0], 0.0],
            [right[1], up[1], -forward[1], 0.0],
            [right[2], up[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
        ];
        (eye, view)
    }

    pub fn projection(&self, width: u32, height: u32) -> Matrix {
        self.zoomed(width, height, 1.0)
    }

    fn zoomed(&self, width: u32, height: u32, zoom: f32) -> Matrix {
        let tan_y = (self.fov_y.to_radians() * 0.5).tan() / zoom.max(1e-3);
        let tan_x = tan_y * width as f32 / height as f32;
        [
            [1.0 / tan_x, 0.0, 0.0, 0.0],
            [0.0, 1.0 / tan_y, 0.0, 0.0],
            [self.shift[0], self.shift[1], FAR / (NEAR - FAR), -1.0],
            [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
        ]
    }

    pub fn framed(&self) -> ([f32; 3], [f32; 3]) {
        RECEIVER
    }

    fn pose(&self, number: u32, still: bool) -> (f32, f32, f32) {
        if still {
            (0.0, 0.0, 1.0)
        } else {
            self.poses[(number as usize).min(POSES - 1)]
        }
    }

    pub fn camera(&self, number: u32, still: bool, width: u32, height: u32) -> Camera {
        let (yaw, pitch, zoom) = self.pose(number, still);
        let (eye, view) = self.view(yaw, pitch);
        let (before_yaw, before_pitch, before_zoom) = self.pose(number.saturating_sub(1), still);
        let (_, before) = self.view(before_yaw, before_pitch);
        Camera {
            view,
            projection: self.zoomed(width, height, zoom),
            previous_view_projection: frame::multiply(
                self.zoomed(width, height, before_zoom),
                before,
            ),
            position: eye,
        }
    }

    pub fn trace_camera(&self, width: u32, height: u32) -> pfx_trace::Camera {
        let forward = unit(sub(self.target, self.position));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        let tan_y = (self.fov_y.to_radians() * 0.5).tan();
        let tan_x = tan_y * width as f32 / height as f32;
        pfx_trace::Camera {
            origin: self.position,
            forward,
            right: right.map(|v| v * tan_x),
            up: up.map(|v| v * tan_y),
        }
    }
}

pub struct Lighting {
    source: SceneSource,
}

impl Lighting {
    pub fn load() -> Self {
        Self {
            source: SceneSource::load(&recipe::folder()).unwrap(),
        }
    }

    pub fn anchor(&self, hour: f32) -> Anchor {
        self.source.with_hours(&[hour]).unwrap().remove(0)
    }

    pub fn sun(&self, hour: f32) -> Sun {
        let sun = self.anchor(hour).sun;
        Sun {
            direction: sun.direction,
            colour: sun.color,
            intensity: sun.intensity,
        }
    }

    pub fn sky(&self, hour: f32) -> Sky {
        self.anchor(hour).sky
    }

    pub fn source(&self, hour: f32) -> SkySource {
        SkySource::Hdr(self.sky(hour))
    }

    pub fn scene(&self) -> &BakeScene {
        &self.source.scene
    }
}

pub fn snap(lo: [f32; 3], hi: [f32; 3], margin: f32) -> GridSpec {
    let min = lo.map(|v| ((v - margin) / SPACING).floor() * SPACING);
    let max = hi.map(|v| ((v + margin) / SPACING).ceil() * SPACING);
    GridSpec {
        min,
        max: std::array::from_fn(|axis| {
            min[axis] + ((max[axis] - min[axis]) / SPACING).round() * SPACING
        }),
        spacing: SPACING,
    }
}

pub fn probe_spec() -> GridSpec {
    snap(
        [room::LEFT, room::FLOOR, room::BACK],
        [room::RIGHT, room::CEILING, room::FRONT],
        0.0,
    )
}

fn sized(min: [f32; 3], max: [f32; 3], spacing: f32) -> GridSpec {
    GridSpec {
        min,
        max: std::array::from_fn(|k| min[k] + ((max[k] - min[k]) / spacing).ceil() * spacing),
        spacing,
    }
}

pub fn volumes() -> Vec<(&'static str, GridSpec)> {
    let [cx, cz] = room::TABLE_CENTRE;
    let [hx, hz] = room::TABLE_HALF;
    vec![
        ("probes", probe_spec()),
        (
            "near-table",
            sized(
                [cx - hx - 0.1, room::TABLE_TOP - 0.05, cz - hz - 0.1],
                [cx + hx + 0.1, room::TABLE_TOP + 0.45, cz + hz + 0.1],
                0.15,
            ),
        ),
        (
            "cavity",
            sized([1.95, room::FLOOR, -1.95], [2.65, 0.55, -1.25], 0.07),
        ),
    ]
}

pub fn reflections() -> Vec<ReflectionSpec> {
    let [cx, cz] = room::TABLE_CENTRE;
    let [hx, hz] = room::TABLE_HALF;
    vec![
        ReflectionSpec {
            name: Some("table".into()),
            position: [cx, room::TABLE_TOP + 0.35, cz + 0.2],
            min: [cx - hx - 0.4, room::FLOOR, cz - hz - 0.5],
            max: [cx + hx + 0.4, room::TABLE_TOP + 0.9, cz + hz + 0.6],
            resolution: 128,
            anchors: None,
            fade: 0.1,
            priority: 1.0,
        },
        ReflectionSpec {
            name: Some("shelves".into()),
            position: [0.8, 1.6, room::BACK + 0.8],
            min: [room::SHELF_X[0] - 0.2, 0.9, room::BACK],
            max: [room::SHELF_X[1] + 0.2, 2.4, room::BACK + 1.2],
            resolution: 128,
            anchors: None,
            fade: 0.1,
            priority: 0.0,
        },
    ]
}

pub fn target(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    target_in(gpu, width, height, wgpu::TextureFormat::Rgba16Float)
}

pub fn target_in(
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> OffscreenTarget {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC;
    if format == wgpu::TextureFormat::Rgba16Float {
        usage |= wgpu::TextureUsages::STORAGE_BINDING;
    }
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format,
        width,
        height,
    }
}

pub fn triangles(room: &Room) -> (Vec<Triangle>, Vec<u32>) {
    let mut triangles = Vec::new();
    let mut owners = Vec::new();
    for (index, part) in room.parts.iter().enumerate() {
        if part.cast == Cast::Only {
            continue;
        }
        let mesh = &room.meshes[part.mesh].1.mesh;
        for corner in mesh.indices.chunks_exact(3) {
            let vertices =
                std::array::from_fn(|k| point(&part.model, mesh.positions[corner[k] as usize]));
            let c = cross(sub(vertices[1], vertices[0]), sub(vertices[2], vertices[0]));
            if dot(c, c) < 1e-28 {
                continue;
            }
            triangles.push(Triangle {
                vertices,
                material: part.material as u32,
            });
            owners.push(index as u32 + 1);
        }
    }
    (triangles, owners)
}

pub fn bake_scene(lighting: &Lighting) -> BakeScene {
    let scene = lighting.scene();
    BakeScene {
        triangles: scene.triangles.clone(),
        shapes: scene.shapes.clone(),
        materials: scene.materials.clone(),
        anchors: scene.anchors.clone(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shown {
    Bare,
    Rest,
    Moving,
}

pub struct Placed {
    pub rest: Instance,
    pub content: Content,
    pub material: String,
    pub surface: InstanceSurface,
}

#[derive(Clone, Copy)]
pub struct Steam {
    pub source: [f32; 3],
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub density: f32,
    pub anisotropy: f32,
}

pub struct Bench {
    pub renderer: Renderer,
    pub output: OffscreenTarget,
    pub instances: Vec<Instance>,
    pub placed: Vec<Placed>,
    pub materials: Vec<Material>,
    pub bare: Vec<Material>,
    pub objects: Vec<String>,
    pub lens: Lens,
    pub lighting: Lighting,
    pub hour: f32,
    pub occlusion_scales: Vec<(String, f32)>,
    pub steam: Option<Steam>,
    pub bird_mesh: Option<CreatureMesh>,
    pub submit: bool,
    pub glass: Vec<Surface>,
    pub haze: Option<RoomHaze>,
    pub motes: Vec<ParticleInstance>,
    pub words: words::Words,
    pub video: Vec<Vec<u8>>,
    pub shown_frame: Option<usize>,
    pub uploads: Vec<f64>,
    pub moving_sun: bool,
    pub finish: Finish,
    pub props: Vec<ImpostorInstance>,
    pub made: f64,
    fluid: (wgpu::Texture, wgpu::TextureView),
    ripple: (wgpu::Texture, wgpu::TextureView),
    roll: usize,
}

fn ripple_texture(gpu: &Gpu, label: &str) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn upload(renderer: &mut Renderer, geometry: &room::Geometry) -> MeshHandle {
    renderer
        .upload_mesh(MeshData {
            positions: &geometry.mesh.positions,
            normals: &geometry.mesh.normals,
            tangents: &geometry.mesh.tangents,
            uvs: &geometry.mesh.uvs,
            uvs1: geometry.uvs1.as_deref(),
            alpha: geometry.alpha.as_deref(),
            indices: &geometry.mesh.indices,
        })
        .unwrap()
}

pub fn roller(time: f32) -> RollerFrame {
    let at = STRIP.length * (0.55 + 0.3 * (time * 0.5).sin());
    let roll = Roller {
        at,
        core: CORE,
        thickness: THICKNESS,
    };
    RollerFrame {
        at,
        outer: roll.outer(&STRIP),
        thickness: THICKNESS,
    }
}

fn wind(time: f32) -> WindFrame {
    WindFrame {
        time,
        strength: 0.5 + 0.2 * (time * 0.37).sin(),
        direction: [0.9, 0.0, 0.35],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Cube,
    Ball,
    Pool,
}

pub struct Pane {
    pub name: &'static str,
    pub shape: Shape,
    pub model: Matrix,
    pub material: &'static str,
    pub casts_shadow: bool,
}

pub fn panes() -> Vec<Pane> {
    let top = room::TABLE_TOP;
    let pane = |name, shape, model, material, casts_shadow| Pane {
        name,
        shape,
        model,
        material,
        casts_shadow,
    };
    vec![
        pane(
            "clear sheet",
            Shape::Cube,
            room::place(
                [-0.95, top + 0.16, -1.2],
                room::turn_y(0.2),
                [0.42, 0.32, 0.006],
            ),
            "glass",
            false,
        ),
        pane(
            "tinted slab",
            Shape::Cube,
            room::place(
                [-2.05, top + 0.09, -0.48],
                room::turn_y(0.1),
                [0.08, 0.18, 0.2],
            ),
            "tinted",
            true,
        ),
        pane(
            "tank",
            Shape::Cube,
            room::place(
                [TANK[0], top + TANK_HALF[1], TANK[2]],
                room::identity(),
                TANK_HALF.map(|v| v * 2.0),
            ),
            "glass",
            false,
        ),
        pane(
            "water",
            Shape::Pool,
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [TANK[0], top + WATER, TANK[2], 1.0],
            ],
            "water",
            false,
        ),
        pane(
            "lens",
            Shape::Ball,
            room::place(room::LENS, room::identity(), [room::LENS_RADIUS * 2.0; 3]),
            "glass",
            true,
        ),
        pane(
            "milky slab",
            Shape::Cube,
            room::place(
                [0.75, top + 0.07, -0.85],
                room::turn_y(-0.4),
                [0.14, 0.14, 0.04],
            ),
            "milk",
            false,
        ),
        pane(
            "soap film",
            Shape::Ball,
            room::place([0.7, top + 0.1, -1.2], room::identity(), [0.2; 3]),
            "soap",
            false,
        ),
        pane(
            "film slab",
            Shape::Cube,
            room::place(
                [0.4, top + 0.09, -0.85],
                room::turn_y(0.5),
                [0.2, 0.18, 0.012],
            ),
            "film_slab",
            false,
        ),
    ]
}

pub fn shape_mesh(shape: Shape) -> pfx_geom::mesh::Mesh {
    match shape {
        Shape::Cube => room::cube(),
        Shape::Ball => room::ball(32, 64),
        Shape::Pool => {
            glass::Pass::liquid_grid(64, 64, [TANK_HALF[0] - 0.006, TANK_HALF[2] - 0.006], WATER)
        }
    }
}

pub fn glass_surfaces(renderer: &mut Renderer, first_id: u32) -> Vec<Surface> {
    let library = room::glass_library();
    let mut handles: Vec<(Shape, glass::MeshHandle)> = Vec::new();
    panes()
        .into_iter()
        .enumerate()
        .map(|(index, pane)| {
            let mesh = match handles.iter().find(|(shape, _)| *shape == pane.shape) {
                Some((_, handle)) => *handle,
                None => {
                    let handle = renderer.upload_glass(&shape_mesh(pane.shape)).unwrap();
                    handles.push((pane.shape, handle));
                    handle
                }
            };
            let material = library[room::material_index(&library, pane.material)].1;
            let liquid = pane.shape == Shape::Pool;
            Surface {
                mesh,
                model: pane.model,
                material,
                liquid,
                fluid_height: if liquid { 1.0 } else { 0.0 },
                ripple_height: if liquid { 1.0 } else { 0.0 },
                caustic_strength: 0.0,
                tinted: (pane.casts_shadow && material.absorption > 0.0).then_some(material),
                id: first_id + index as u32,
                casts_shadow: pane.casts_shadow,
                shadow_only: false,
                clip: [[0.0; 4]; 2],
            }
        })
        .collect()
}

impl Bench {
    pub fn surfaces(&self, shown: Shown) -> Vec<InstanceSurface> {
        self.placed
            .iter()
            .map(|placed| {
                let mut surface = match shown {
                    Shown::Bare => InstanceSurface {
                        cutout: placed.surface.cutout,
                        ..InstanceSurface::default()
                    },
                    _ => placed.surface,
                };
                if let Some((_, scale)) = self
                    .occlusion_scales
                    .iter()
                    .find(|(name, _)| *name == placed.material)
                {
                    surface.reflection_occlusion = *scale;
                }
                surface
            })
            .collect()
    }

    pub fn place(&mut self, shown: Shown) -> Vec<InstanceSurface> {
        for (index, placed) in self.placed.iter().enumerate() {
            let mut instance = placed.rest;
            instance.previous_model = self.instances[index].model;
            self.instances[index] = instance;
        }
        self.surfaces(shown)
    }

    fn write_ripples(&self, time: f32) {
        let mut bytes = Vec::with_capacity(64 * 64 * 2);
        for y in 0..64 {
            for x in 0..64 {
                let u = x as f32 / 63.0 - 0.5;
                let v = y as f32 / 63.0 - 0.5;
                let r = (u * u + v * v).sqrt();
                let height = 0.002 * (r * 48.0 - time * 3.0).sin() * (1.0 - r).max(0.0)
                    + 0.0008 * (u * 31.0 + time * 1.7).sin() * (v * 23.0 - time * 1.1).cos();
                bytes.extend_from_slice(&half::f16::from_f32(height).to_bits().to_le_bytes());
            }
        }
        self.renderer.gpu().queue.write_texture(
            self.ripple.0.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(128),
                rows_per_image: Some(64),
            },
            self.ripple.0.size(),
        );
    }

    pub fn sun_hour(&self, number: u32, still: bool) -> f32 {
        if still || !self.moving_sun {
            self.hour
        } else {
            self.hour + number as f32 / 60.0 * (1.0 / 120.0)
        }
    }

    pub fn render(&mut self, number: u32, still: bool, shown: Shown) -> Vec<PassTiming> {
        let time = if still { 0.0 } else { number as f32 / 60.0 };
        let previous_time = if still {
            0.0
        } else {
            number.saturating_sub(1) as f32 / 60.0
        };
        let surfaces = self.place(shown);
        self.renderer.frame().set_surfaces(&surfaces).unwrap();
        if shown == Shown::Moving && !self.video.is_empty() {
            let index = (time * VIDEO_RATE) as usize % self.video.len();
            if self.shown_frame != Some(index) {
                let started = Instant::now();
                let (width, height) = paint::VIDEO;
                self.renderer
                    .frame()
                    .write_content(paint::VIDEO_SLOT, [0, 0, width, height], &self.video[index])
                    .unwrap();
                self.uploads.push(started.elapsed().as_secs_f64() * 1000.0);
                self.shown_frame = Some(index);
            }
        }
        self.write_ripples(time);
        let (width, height) = self.renderer.size();
        let camera = self.lens.camera(number, still, width, height);
        let rolling = shown == Shown::Moving;
        let deformers = [Deformer::Roller {
            current: roller(if rolling { time } else { 0.0 }),
            previous: roller(if rolling { previous_time } else { 0.0 }),
        }];
        let scene = Scene {
            camera,
            time,
            seed: 23,
            sun: self.lighting.sun(self.sun_hour(number, still)),
            instances: &self.instances,
            materials: if shown == Shown::Bare {
                &self.bare
            } else {
                &self.materials
            },
            deformers: &deformers,
            wind: SceneWind {
                current: wind(time),
                previous: wind(previous_time),
            },
        };
        let flight = if still { 0.0 } else { time };
        let birds: Vec<CreatureInstance> = room::BIRDS
            .iter()
            .enumerate()
            .map(|(index, at)| CreatureInstance {
                position_scale: [
                    at[0] + (flight * 0.4 + index as f32).sin() * 0.15,
                    at[1] + (flight * 0.7 + index as f32 * 2.0).sin() * 0.05,
                    at[2] + (flight * 0.3 + index as f32).cos() * 0.2,
                    0.12,
                ],
                right: [1.0, 0.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0, 0.0],
                forward: [0.0, 0.0, 1.0, 0.0],
                motion: [index as f32 * 0.37 + flight, 0.6, 0.0, 0.0],
            })
            .collect();
        let text = self.words.text(
            self.renderer.gpu(),
            &camera,
            &self.instances[self.roll],
            width,
            height,
        );
        let effects = Effects {
            volume: self.steam.map(|steam| Volume {
                kind: VolumeKind::Plume(Plume::at(steam.source)),
                lo: steam.lo,
                hi: steam.hi,
                color: [0.9, 0.9, 0.92],
                density: steam.density,
                anisotropy: steam.anisotropy,
                time_scale: 0.5,
            }),
            particles: &self.motes,
            creatures: if self.bird_mesh.is_some() {
                &birds
            } else {
                &[]
            },
            creature_mesh: self.bird_mesh.as_ref(),
            glass: &self.glass,
            liquid: Some(Liquid {
                fluid: &self.fluid.1,
                ripple: &self.ripple.1,
            }),
            props: &self.props,
        };
        if self.submit {
            return self
                .renderer
                .submit(&scene, &text, &effects, self.finish, &self.output.view)
                .unwrap()
                .arrived
                .pop()
                .map(|timings| timings.passes)
                .unwrap_or_default();
        }
        self.renderer
            .render(&scene, &text, &effects, self.finish, &self.output.view)
            .unwrap()
    }

    pub fn pixels(&self) -> Vec<u8> {
        self.renderer
            .gpu()
            .readback_rgba16(&self.output)
            .unwrap()
            .into_iter()
            .map(|v| (half::f16::from_bits(v).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    }

    pub fn ids(&self) -> Vec<u32> {
        let gpu = self.renderer.gpu();
        let targets = &self.renderer.frame().targets;
        let (width, height) = (targets.width, targets.height);
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bench ids"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            targets.ids.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            targets.ids.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = buffer.slice(..).get_mapped_range();
        let mut ids = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            let start = (y * row) as usize;
            ids.extend(
                bytes[start..start + (width * 4) as usize]
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
            );
        }
        ids
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if self.renderer.size() != (width, height) {
            self.renderer.resize(width, height).unwrap();
            self.output = target(self.renderer.gpu(), width, height);
        }
    }

    pub fn hide_unbaked(&mut self) {
        let rooms = room::room().parts.len();
        for index in rooms..self.placed.len() {
            let mut rest = self.placed[index].rest;
            rest.model = room::translate([0.0, -100.0, 0.0]);
            rest.previous_model = rest.model;
            rest.casts_shadow = false;
            self.placed[index].rest = rest;
            self.instances[index] = rest;
        }
        if self.objects.iter().any(|name| name == "tree") {
            self.renderer.set_canopy_strength(0.0).unwrap();
        }
    }

    pub fn set_hour(&mut self, hour: f32) {
        self.hour = hour;
        self.renderer
            .set_sky(self.lighting.source(hour), hour)
            .unwrap();
    }
}

pub struct Options {
    pub bc: bool,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub probes: bool,
    pub tree: bool,
    pub effects: bool,
    pub content: bool,
}

impl Options {
    pub fn from_env() -> Self {
        let (width, height) = size();
        Self {
            bc: std::env::var_os("BENCH_DETAIL").is_some()
                || std::env::var_os("BENCH_BAKE_DETAIL").is_some(),
            format: wgpu::TextureFormat::Rgba16Float,
            width,
            height,
            probes: std::env::var_os("BENCH_SKIP_LIGHT_BAKES").is_none(),
            tree: std::env::var_os("BENCH_NO_TREE").is_none(),
            effects: true,
            content: true,
        }
    }

    pub fn small(width: u32, height: u32) -> Self {
        Self {
            bc: false,
            format: wgpu::TextureFormat::Rgba16Float,
            width,
            height,
            probes: false,
            tree: false,
            effects: true,
            content: true,
        }
    }
}

pub fn size() -> (u32, u32) {
    match std::env::var("BENCH_SIZE").as_deref() {
        Ok("deck") | Ok("1280x800") => DECK,
        _ => (WIDTH, HEIGHT),
    }
}

pub fn root() -> PathBuf {
    let root = PathBuf::from("tmp/bench");
    std::fs::create_dir_all(&root).unwrap();
    root
}

pub fn probe_artifacts() -> (Artifact, Vec<Artifact>) {
    let root = root();
    let mut artifacts = volumes()
        .into_iter()
        .map(|(name, _)| read_artifact(&root.join(format!("probes-{name}"))).unwrap());
    let main = artifacts.next().unwrap();
    (main, artifacts.collect())
}

fn bake_light(renderer: &mut Renderer, lighting: &Lighting) {
    let scene = bake_scene(lighting);
    let root = root();
    let hash = scene_hash(&scene, probe_spec(), SAMPLES, SEED).unwrap();
    for (name, spec) in volumes() {
        let dims = spec.dimensions().unwrap();
        let folder = root.join(format!("probes-{name}"));
        let wanted = scene_hash(&scene, spec, SAMPLES, SEED).unwrap();
        if read_artifact(&folder).is_ok_and(|artifact| artifact.manifest.scene_hash == wanted) {
            println!(
                "probe bake {name}: {}x{}x{} probes every {} m at {ANCHORS:?} h, reused, scene hash unchanged",
                dims[0], dims[1], dims[2], spec.spacing
            );
            continue;
        }
        let started = Instant::now();
        let grids = bake(renderer.gpu(), &scene, spec, SAMPLES, SEED).unwrap();
        println!(
            "probe bake {name}: {}x{}x{} probes every {} m at {ANCHORS:?} h, {SAMPLES} samples, {:.1} s",
            dims[0],
            dims[1],
            dims[2],
            spec.spacing,
            started.elapsed().as_secs_f64()
        );
        if folder.exists() {
            std::fs::remove_dir_all(&folder).unwrap();
        }
        write_artifact(
            Path::new("tmp"),
            &Path::new("bench").join(format!("probes-{name}")),
            &scene,
            &grids,
            SAMPLES,
            SEED,
        )
        .unwrap();
    }
    let (main, locals) = probe_artifacts();
    let lighting_probes = ProbeLighting::from_artifacts(
        &renderer.gpu().device,
        &renderer.gpu().queue,
        &main,
        &locals,
        HOUR,
        [-1.0; 3],
    )
    .unwrap();
    renderer.set_probes(lighting_probes).unwrap();
    let mut artifacts = Vec::new();
    for spec in reflections() {
        let name = spec.name.clone().unwrap();
        let dir = root.join(format!("reflection-{name}"));
        let fresh = read_reflection_artifact(&dir).is_ok_and(|artifact| {
            artifact.manifest.scene_hash == hash
                && artifact.manifest.spec == spec
                && artifact.manifest.sample_count == SAMPLES
                && artifact.manifest.seed == SEED
        });
        if fresh {
            println!("reflection bake {name}: one 128² cube at {ANCHORS:?} h, reused");
        } else {
            let started = Instant::now();
            let cubes = (0..scene.anchors.len())
                .map(|index| bake_reflection(renderer.gpu(), &scene, &spec, SAMPLES, SEED, index))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            println!(
                "reflection bake {name}: one 128² cube at {ANCHORS:?} h, {SAMPLES} samples, {:.1} s",
                started.elapsed().as_secs_f64()
            );
            if dir.exists() {
                std::fs::remove_dir_all(&dir).unwrap();
            }
            write_reflection_artifact(ReflectionWrite {
                tmp_root: Path::new("tmp"),
                relative: &Path::new("bench").join(format!("reflection-{name}")),
                scene_hash: hash.clone(),
                spec: &spec,
                anchors: &scene.anchors,
                cubes: &cubes,
                samples: SAMPLES,
                seed: SEED,
            })
            .unwrap();
        }
        artifacts.push(read_reflection_artifact(&dir).unwrap());
    }
    renderer
        .frame()
        .set_local_reflections(Some(artifacts), HOUR)
        .unwrap();
}

fn props(renderer: &mut Renderer, lighting: &Lighting) -> Vec<ImpostorInstance> {
    let mesh = room::ball(16, 32);
    let mut triangles = Vec::new();
    for corner in mesh.indices.chunks_exact(3) {
        triangles.push(Triangle {
            vertices: std::array::from_fn(|k| mesh.positions[corner[k] as usize].map(|v| v * 0.3)),
            material: 0,
        });
    }
    let prop_bounds = bounds(&triangles, &[]).unwrap();
    let library = room::library();
    let scene = BakeScene {
        triangles,
        shapes: Vec::new(),
        materials: vec![library[room::material_index(&library, "slate")].1],
        anchors: vec![lighting.anchor(HOUR)],
    };
    let artifact = bake_prop(
        renderer.gpu(),
        &scene,
        &PropSpec {
            name: "bench-prop".into(),
            node: Some("bench".into()),
            file: None,
            frames_per_side: 8,
            atlas_size: 512,
            hemisphere: false,
            anchors: vec![HOUR],
        },
        prop_bounds,
        8,
        SEED,
    )
    .unwrap();
    renderer.set_props(Some(
        ImpostorPass::new(
            renderer.gpu(),
            artifact,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
            2,
        )
        .unwrap(),
    ));
    [[2.1, 0.15, 1.6], [2.5, 0.15, 1.3]]
        .into_iter()
        .enumerate()
        .map(|(index, centre)| {
            ImpostorInstance::new(centre, prop_bounds.radius, 2000 + index as u32, 0.0).unwrap()
        })
        .collect()
}

fn bird_mesh(device: &wgpu::Device) -> CreatureMesh {
    let positions = [
        [0.0, 0.0, 0.0],
        [-1.0, 0.0, 0.1],
        [-0.1, 0.0, -0.25],
        [0.1, 0.0, -0.25],
        [1.0, 0.0, 0.1],
    ];
    let vertices = positions.map(|p| CreatureVertex {
        position: [p[0], p[1], p[2], 0.0],
    });
    CreatureMesh::new(device, &vertices, &[0, 1, 2, 0, 3, 4])
}

fn motes() -> Vec<ParticleInstance> {
    (0..1200_u32)
        .map(|i| {
            let x = room::hash(i.wrapping_mul(3).wrapping_add(23));
            let y = room::hash(i.wrapping_mul(3).wrapping_add(24));
            let z = room::hash(i.wrapping_mul(3).wrapping_add(25));
            ParticleInstance {
                position_size: [
                    room::LEFT + 0.1 + x * 2.2,
                    room::TABLE_TOP + y * 1.5,
                    room::WINDOW_Z[0] + z * (room::WINDOW_Z[1] - room::WINDOW_Z[0]),
                    0.0025,
                ],
                velocity_alpha: [0.004, -0.002, 0.003, 0.4],
                color_rotation: [1.0, 0.95, 0.85, i as f32 * 0.51],
                detail: [1.0, 0.01, room::hash(i), 0.0],
            }
        })
        .collect()
}

pub fn build(options: &Options) -> Bench {
    let room = room::room();
    let lighting = Lighting::load();
    let gpu = if options.bc {
        detail::gpu()
    } else {
        pollster::block_on(Gpu::headless()).unwrap()
    };
    let making = Instant::now();
    let mut renderer =
        Renderer::new_with_output_format(gpu, options.width, options.height, options.format)
            .unwrap();
    let made = making.elapsed().as_secs_f64();
    renderer.set_exposure(Exposure::Fixed(EXPOSURE)).unwrap();
    let mut quality = renderer.shadows().quality();
    quality.receiver = Some(ReceiverBox {
        min: RECEIVER.0,
        max: RECEIVER.1,
    });
    quality.caster_margin = 0.3;
    quality.sun_radius_deg = SUN_RADIUS_DEG;
    renderer.set_shadow_quality(quality);
    let handles: Vec<MeshHandle> = room
        .meshes
        .iter()
        .map(|(_, geometry)| upload(&mut renderer, geometry))
        .collect();
    let mut instances = Vec::new();
    let mut objects = Vec::new();
    for (index, part) in room.parts.iter().enumerate() {
        let mut instance = Instance::new(
            handles[part.mesh],
            part.model,
            part.material as u32,
            index as u32 + 1,
        );
        instance.age = part.age;
        instance.casts_shadow = part.cast != Cast::Never;
        instance.shadow_only = part.cast == Cast::Only;
        instance.two_sided = part.mesh == room::PAGE || part.mesh == room::SCREEN;
        instances.push(instance);
        objects.push(part.name.clone());
    }
    let strip = STRIP.mesh(CORE, 0.0004, None, 8);
    let strip_handle = renderer
        .upload_mesh(MeshData {
            positions: &strip.positions,
            normals: &strip.normals,
            tangents: &strip.tangents,
            uvs: &strip.uvs,
            uvs1: None,
            alpha: None,
            indices: &strip.indices,
        })
        .unwrap();
    let mut rolled = Instance::new(
        strip_handle,
        room::place(ROLL_AT, room::turn_y(-0.3), [1.0; 3]),
        room::material_index(&room.materials, "sheet") as u32,
        instances.len() as u32 + 1,
    );
    rolled.deformer = DeformerId(0);
    rolled.two_sided = true;
    let roll = instances.len();
    instances.push(rolled);
    objects.push("rolled sheet".into());
    let mut materials = recipe::materials(&room);
    for ((name, _), material) in room.materials.iter().zip(materials.iter_mut()) {
        if let Some((content, layer)) = room::content(name) {
            material.content = content;
            material.content_layer = layer;
        }
    }
    if options.tree {
        let tree = Tree::new(
            23,
            TreeSpec {
                height: 4.5,
                trunk_radius: 0.18,
                crown_width: 3.6,
                crown_height: 2.8,
                ..TreeSpec::cherry()
            },
            0.004,
        );
        println!(
            "tree outside: wood triangles={} cards={} flowers={}",
            tree.wood.indices.len() / 3,
            tree.cards.len(),
            tree.flower_count
        );
        let id = instances.len() as u32 + 1;
        let wood = renderer
            .set_tree(
                &tree,
                room::TREE_ORIGIN,
                room::TABLE_TOP,
                id,
                20.0,
                [0.95, 0.8, 0.85],
            )
            .unwrap();
        renderer.set_canopy_mode(CanopyMode::Procedural).unwrap();
        renderer.set_canopy_sharpness(0.3).unwrap();
        instances.push(Instance::new(
            wood,
            room::translate(room::TREE_ORIGIN),
            room::material_index(&room.materials, "bark") as u32,
            id,
        ));
        objects.push("tree".into());
    }
    if options.probes {
        bake_light(&mut renderer, &lighting);
    }
    renderer.set_sky(lighting.source(HOUR), HOUR).unwrap();
    let mut painted = options.content.then(|| paint::Painted::new(&lighting));
    let mut surfaces = vec![InstanceSurface::default(); instances.len()];
    for (index, part) in room.parts.iter().enumerate() {
        surfaces[index].cutout = part.cutout;
    }
    if let Some(painted) = painted.as_mut() {
        painted.upload(&renderer).unwrap();
        for (index, surface) in painted.surfaces(&room) {
            surfaces[index] = InstanceSurface {
                cutout: surfaces[index].cutout,
                ..surface
            };
        }
        let receivers = room
            .parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.name == "table top" || part.name == "felt mat")
            .map(|(index, _)| index)
            .collect();
        renderer
            .frame()
            .set_caustic(Some(Caustic {
                slot: paint::CAUSTIC_SLOT,
                rect: painted.caustic_rect,
                height: room::TABLE_TOP,
                strength: 1.0,
                receivers,
            }))
            .unwrap();
    }
    let bare: Vec<Material> = materials
        .iter()
        .map(|material| Material {
            content: Content::None,
            content_layer: ContentLayer::default(),
            ..*material
        })
        .collect();
    let first_glass = instances.len() as u32 + 1;
    let glass = if options.effects {
        glass_surfaces(&mut renderer, first_glass)
    } else {
        Vec::new()
    };
    objects.extend(
        panes()
            .iter()
            .take(glass.len())
            .map(|pane| pane.name.to_string()),
    );
    let placed = instances
        .iter()
        .zip(&surfaces)
        .map(|(instance, surface)| Placed {
            rest: *instance,
            content: materials[instance.material as usize].content,
            material: room
                .materials
                .get(instance.material as usize)
                .map_or("bark", |(name, _)| name)
                .to_string(),
            surface: *surface,
        })
        .collect();
    let fluid = ripple_texture(renderer.gpu(), "bench fluid");
    let ripple = ripple_texture(renderer.gpu(), "bench ripple");
    renderer.gpu().queue.write_texture(
        fluid.0.as_image_copy(),
        &vec![0u8; 64 * 64 * 2],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(128),
            rows_per_image: Some(64),
        },
        fluid.0.size(),
    );
    let words = words::Words::new(&renderer, options.width, options.height);
    let bird_mesh = options.effects.then(|| bird_mesh(&renderer.gpu().device));
    let haze = options.effects.then_some(RoomHaze {
        fog: 0.02,
        smoke: 0.0,
        mist: 0.03,
        floor: 0.1,
        phase: 0.35,
        back: 0.7,
        gold: [1.0, 0.95, 0.85],
        ambient: [0.03; 3],
        reach: 3.0,
        unmapped: 0.0,
        lo: [room::LEFT, room::FLOOR, room::BACK],
        hi: [room::RIGHT, room::CEILING, room::FRONT],
        seed: 7,
    });
    renderer.set_haze(haze);
    let props = if options.effects {
        props(&mut renderer, &lighting)
    } else {
        Vec::new()
    };
    let output = target_in(
        renderer.gpu(),
        options.width,
        options.height,
        options.format,
    );
    Bench {
        renderer,
        output,
        instances,
        placed,
        materials,
        bare,
        objects,
        lens: Lens::default(),
        lighting,
        hour: HOUR,
        occlusion_scales: Vec::new(),
        steam: options.effects.then_some(Steam {
            source: [TANK[0], room::TABLE_TOP + WATER, TANK[2]],
            lo: [
                TANK[0] - TANK_HALF[0],
                room::TABLE_TOP + WATER,
                TANK[2] - TANK_HALF[2],
            ],
            hi: [
                TANK[0] + TANK_HALF[0],
                room::TABLE_TOP + WATER + 0.3,
                TANK[2] + TANK_HALF[2],
            ],
            density: 1.0,
            anisotropy: 0.35,
        }),
        bird_mesh,
        submit: false,
        glass,
        haze,
        motes: if options.effects { motes() } else { Vec::new() },
        words,
        video: painted.map(|painted| painted.video).unwrap_or_default(),
        shown_frame: None,
        uploads: Vec::new(),
        moving_sun: true,
        finish: Finish::Standard,
        props,
        made,
        fluid,
        ripple,
        roll,
    }
}

pub fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let file = File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
    println!("wrote {}", path.display());
}

pub fn percentile(values: &[f64], fraction: f64) -> f64 {
    values[((values.len() - 1) as f64 * fraction).round() as usize]
}

fn time(bench: &mut Bench, shown: Shown) {
    let (width, height) = bench.renderer.size();
    let mut stats = FrameStats::new();
    stats.set_size(width, height);
    stats.set_renderer("live");
    let mut pace = manners::Pace::new();
    for number in SETTLE_FRAMES..SETTLE_FRAMES + WARM_FRAMES {
        pace.frame(|| bench.render(number, false, shown));
    }
    let mut frame_ms = Vec::with_capacity(TIMED_FRAMES as usize);
    let mut pass_ms: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    let first = SETTLE_FRAMES + WARM_FRAMES;
    let mut pace = manners::Pace::timed("bench", "moving", TIMED_FRAMES as u64);
    for number in first..first + TIMED_FRAMES {
        let before = Instant::now();
        let timings = bench.render(number, false, shown);
        let ms = before.elapsed().as_secs_f64() * 1000.0;
        frame_ms.push(ms);
        let gpu_ms = timings.iter().map(|t| t.milliseconds).sum::<f64>();
        stats.record_frame(ms, ms, gpu_ms, &timings);
        for timing in timings {
            let entry = pass_ms.entry(timing.label).or_default();
            entry.0 += timing.milliseconds;
            entry.1 += 1;
        }
        pace.after_gpu(ms, Some(gpu_ms));
    }
    frame_ms.sort_by(f64::total_cmp);
    let frames = manners::Frames::of(&frame_ms);
    println!(
        "{shown:?}: {width}x{height}, {TIMED_FRAMES} frames with the camera moving: {:.2} fps, frame p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
        frames.fps, frames.p50, frames.p95, frames.p99, frames.max
    );
    let mut gpu_total = 0.0;
    for (pass, (total, count)) in &pass_ms {
        let mean = total / *count as f64;
        gpu_total += mean;
        println!("  {pass}: {mean:.3} ms");
    }
    println!("  sum of pass means: {gpu_total:.3} ms");
}

fn settle(bench: &mut Bench, shown: Shown) {
    let mut pace = manners::Pace::new();
    for number in 0..SETTLE_FRAMES {
        pace.frame(|| bench.render(number, true, shown));
    }
}

fn picks(bench: &Bench) {
    let (width, height) = bench.renderer.size();
    let ids = bench.ids();
    for (label, x, y) in [
        ("centre", 0.5, 0.5),
        ("window", 0.08, 0.4),
        ("shelves", 0.6, 0.3),
        ("table", 0.4, 0.75),
        ("floor", 0.6, 0.97),
    ] {
        let px = ((x * width as f32) as u32).min(width - 1);
        let py = ((y * height as f32) as u32).min(height - 1);
        let id = ids[(py * width + px) as usize];
        let name = id
            .checked_sub(1)
            .and_then(|i| bench.objects.get(i as usize))
            .map_or("(nothing)", String::as_str);
        println!("pick {label} at ({px}, {py}): id {id} -> {name}");
    }
}

pub fn finish_named(name: &str) -> Option<Finish> {
    if name == "standard" {
        return Some(Finish::Standard);
    }
    Style::ALL
        .into_iter()
        .find(|style| format!("{style:?}").eq_ignore_ascii_case(name))
        .map(Finish::Style)
}

fn main() {
    let root = root();
    match std::env::var("BENCH_RECIPE").as_deref() {
        Ok("write") => return recipe::write(&recipe::folder()),
        Ok(_) => {
            let stale = recipe::stale();
            println!("stale recipe files: {stale:?}");
            assert!(stale.is_empty());
            return;
        }
        Err(_) => {}
    }
    match std::env::var("BENCH_REFERENCE").as_deref() {
        Ok("trace") => {
            let regions = reference::trace(&root);
            let path = root.join("reference.csv");
            std::fs::write(&path, reference::format(&regions)).unwrap();
            println!("wrote {} with {} regions", path.display(), regions.len());
            return;
        }
        Ok(_) => {
            reference::compare(&root);
            return;
        }
        Err(_) => {}
    }
    if std::env::var_os("BENCH_PHYSICS").is_some() {
        physics::measure();
        return;
    }
    let mut options = Options::from_env();
    let shadows = std::env::var_os("BENCH_SHADOWS").is_some();
    if shadows || std::env::var_os("BENCH_TERMS").is_some() {
        options.tree = false;
    }
    let lens = Lens::default();
    let detail_check = std::env::var_os("BENCH_DETAIL_CHECK").is_some();
    let detail = if options.bc {
        let room = room::room();
        let parts = detail::parts(&room, &recipe::materials(&room));
        Some(match std::env::var_os("BENCH_DETAIL") {
            Some(folder) => detail::load(&parts, Path::new(&folder)),
            None => detail::bake(&parts, &lens, (options.width, options.height), detail_check),
        })
    } else {
        None
    };
    let started = Instant::now();
    let mut bench = build(&options);
    if let Some(baked) = &detail {
        detail::apply(&mut bench, baked);
    }
    println!(
        "bench room: {} instances on {} meshes, {} materials, {} glass surfaces, built in {:.2} s",
        bench.instances.len(),
        room::room().meshes.len(),
        bench.materials.len(),
        bench.glass.len(),
        started.elapsed().as_secs_f64()
    );
    println!(
        "shadow atlas: {}², sun {:?} at {HOUR} h",
        bench.renderer.shadows().resolution(),
        bench.lighting.sun(HOUR).direction
    );
    if let Ok(name) = std::env::var("BENCH_FINISH") {
        bench.finish = finish_named(&name).unwrap_or_else(|| panic!("no finish named {name}"));
    }
    if let Some(baked) = detail.as_ref().filter(|_| detail_check) {
        detail::check(&mut bench, baked);
        return;
    }
    if shadows {
        let room = room::room();
        let (triangles, owners) = triangles(&room);
        let names: Vec<String> = room
            .materials
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        shadow_parts::run(
            &mut bench,
            &shadow_parts::Scene {
                triangles: &triangles,
                owners: &owners,
                material_names: &names,
                materials: &recipe::materials(&room),
                radius_deg: SUN_RADIUS_DEG,
            },
        );
        return;
    }
    if std::env::var_os("BENCH_COLD").is_some() {
        let made = bench.made;
        cold::run(&mut bench, made);
        return;
    }
    if std::env::var_os("BENCH_OVERLAP").is_some() {
        overlap(&mut bench);
        return;
    }
    if std::env::var_os("BENCH_OCCLUSION").is_some() {
        let light = occlusion::Light::load();
        let parts =
            std::env::var("BENCH_OCCLUSION_PARTS").unwrap_or_else(|_| "plates,timing".into());
        if parts.split(',').any(|part| part == "timing") {
            occlusion::timing(&mut bench, &light);
        }
        if parts.split(',').any(|part| part == "plates") {
            occlusion::plates(&mut bench, &light, &root.join("occlusion"));
        }
        return;
    }
    if std::env::var_os("BENCH_TERMS").is_some() {
        let room = room::room();
        let (triangles, _) = triangles(&room);
        let bare = bench.bare.clone();
        let reflection = reflections().remove(0);
        terms::Terms {
            renderer: &mut bench.renderer,
            instances: &bench.instances,
            materials: &bare,
            triangles: &triangles,
            lens: &bench.lens,
            lighting: &bench.lighting,
            spec: probe_spec(),
            reflection,
        }
        .run();
        return;
    }
    let room_triangles = triangles(&room::room()).0;
    let traced = detail.is_none();
    if std::env::var_os("BENCH_BREAKDOWN").is_some() {
        budget::breakdown(&mut bench);
        return;
    }
    if std::env::var_os("BENCH_CUTS").is_some() {
        budget::cuts(&mut bench);
        return;
    }
    if std::env::var_os("BENCH_VOLUME").is_some() {
        volume::cuts(&mut bench);
        return;
    }
    if std::env::var_os("BENCH_FOOTPRINT").is_some() {
        budget::footprint(&mut bench, &room_triangles, traced);
        return;
    }
    if std::env::var_os("BENCH_TILES").is_some() {
        budget::tiles(&mut bench, &room_triangles, traced);
        return;
    }
    if std::env::var_os("BENCH_LACQUER").is_some() {
        budget::lacquer(&mut bench, &room_triangles, traced);
        return;
    }
    if std::env::var_os("BENCH_FINISHES").is_some() {
        finishes(&mut bench);
        return;
    }
    let (width, height) = bench.renderer.size();
    let tag = format!("{width}x{height}");
    for (shown, name) in [
        (Shown::Bare, "bare"),
        (Shown::Moving, "moving"),
        (Shown::Rest, "rest"),
    ] {
        settle(&mut bench, shown);
        save_png(
            &root.join(format!("bench-{tag}-{name}.png")),
            width,
            height,
            &bench.pixels(),
        );
    }
    picks(&bench);
    if std::env::var_os("BENCH_STILLS_ONLY").is_some() {
        return;
    }
    for shown in [Shown::Bare, Shown::Rest, Shown::Moving] {
        time(&mut bench, shown);
    }
    if !bench.uploads.is_empty() {
        let mut uploads = bench.uploads.clone();
        uploads.sort_by(f64::total_cmp);
        println!(
            "video uploads: {} frames of {}x{} with mips, CPU submit p50 {:.3} ms, max {:.3} ms",
            uploads.len(),
            paint::VIDEO.0,
            paint::VIDEO.1,
            percentile(&uploads, 0.5),
            uploads[uploads.len() - 1]
        );
    }
    let stats = bench.renderer.lod_stats();
    println!(
        "LOD: {} triangles drawn of {} ideal; visible cards {}",
        stats.triangles,
        stats.ideal_triangles,
        bench.renderer.frame().visible_card_count()
    );
}

fn finishes(bench: &mut Bench) {
    let (width, height) = bench.renderer.size();
    println!(
        "finishes: {width}x{height}, the room moving, {FINISH_FRAMES} frames each after {WARM_FRAMES} to settle; post ms mean and the frame's GPU sum"
    );
    println!("| finish | post ms | GPU ms |");
    println!("|---|---:|---:|");
    let all = std::iter::once(Finish::Standard).chain(Style::ALL.into_iter().map(Finish::Style));
    let mut number = SETTLE_FRAMES;
    for finish in all {
        bench.finish = finish;
        let mut pace =
            manners::Pace::timed("bench", "finishes", u64::from(WARM_FRAMES + FINISH_FRAMES));
        for _ in 0..WARM_FRAMES {
            pace.frame(|| bench.render(number, false, Shown::Moving));
            number += 1;
        }
        let mut post = Vec::new();
        let mut gpu = Vec::new();
        for _ in 0..FINISH_FRAMES {
            let timings = pace.frame(|| bench.render(number, false, Shown::Moving));
            number += 1;
            post.push(
                timings
                    .iter()
                    .filter(|t| t.label == "post")
                    .map(|t| t.milliseconds)
                    .sum::<f64>(),
            );
            gpu.push(timings.iter().map(|t| t.milliseconds).sum::<f64>());
        }
        let name = match finish {
            Finish::Standard => "standard".to_string(),
            Finish::Style(style) => format!("{style:?}").to_lowercase(),
        };
        println!(
            "| {name} | {:.3} | {:.3} |",
            post.iter().sum::<f64>() / post.len() as f64,
            gpu.iter().sum::<f64>() / gpu.len() as f64
        );
        save_png(
            &root().join(format!("finish-{width}x{height}-{name}.png")),
            width,
            height,
            &bench.pixels(),
        );
    }
    bench.finish = Finish::Standard;
}

fn overlap(bench: &mut Bench) {
    let uncapped = std::env::var("PFX_UNCAPPED").is_ok_and(|value| value == "1");
    let period = if uncapped { 0.0 } else { 1000.0 / 60.0 };
    let (width, height) = bench.renderer.size();
    let first = SETTLE_FRAMES + WARM_FRAMES;
    for submit in [false, true] {
        bench.submit = submit;
        let mut clock = manners::Wall::new();
        let mut tally = manners::Tally::timed(
            "bench",
            if submit {
                "overlap submit"
            } else {
                "overlap blocking"
            },
            u64::from(first + TIMED_FRAMES - SETTLE_FRAMES),
            manners::Clock::now_ms(&mut clock),
        );
        let mut calls = Vec::with_capacity(TIMED_FRAMES as usize);
        let mut frames = Vec::with_capacity(TIMED_FRAMES as usize);
        let mut waited = 0.0;
        let mut drained = 0.0;
        let mut pending = 0.0;
        let mut longest: f64 = 0.0;
        let drops = bench.renderer.timing_drops();
        for number in SETTLE_FRAMES..first + TIMED_FRAMES {
            let timed = number >= first;
            let before = Instant::now();
            bench.render(number, false, Shown::Rest);
            let call = before.elapsed().as_secs_f64() * 1000.0;
            if submit {
                let wait = Instant::now();
                bench.renderer.wait_frames(2).unwrap();
                if timed {
                    waited += wait.elapsed().as_secs_f64() * 1000.0;
                }
            }
            let frame = before.elapsed().as_secs_f64() * 1000.0;
            if timed {
                calls.push(call);
                frames.push(frame);
            }
            pending += frame;
            longest = longest.max(frame);
            if pending >= 50.0 || number + 1 == first {
                if submit {
                    let drain = Instant::now();
                    bench.renderer.wait_frames(0).unwrap();
                    if timed {
                        drained += drain.elapsed().as_secs_f64() * 1000.0;
                    }
                }
                manners::Clock::turn(
                    &mut clock,
                    manners::Turn {
                        work_ms: pending,
                        longest_ms: longest,
                    },
                );
                pending = 0.0;
                longest = 0.0;
                tally.report(manners::Clock::now_ms(&mut clock));
            }
            tally.frame();
            let rest = period - before.elapsed().as_secs_f64() * 1000.0;
            if rest > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f64(rest / 1000.0));
            }
        }
        if submit {
            let drain = Instant::now();
            bench.renderer.wait_frames(0).unwrap();
            drained += drain.elapsed().as_secs_f64() * 1000.0;
        }
        let total = frames.iter().sum::<f64>() + drained;
        calls.sort_by(f64::total_cmp);
        let mut sorted = frames.clone();
        sorted.sort_by(f64::total_cmp);
        let now = bench.renderer.timing_drops();
        println!(
            "{}: {width}x{height}, {TIMED_FRAMES} frames, {}: calling thread p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms; frame p50 {:.3} ms, p99 {:.3} ms; {:.2} fps over the run{}",
            if submit {
                "non-blocking submit"
            } else {
                "blocking render"
            },
            if uncapped {
                "uncapped"
            } else {
                "capped at 60 fps"
            },
            percentile(&calls, 0.5),
            percentile(&calls, 0.95),
            calls[calls.len() - 1],
            percentile(&sorted, 0.5),
            percentile(&sorted, 0.99),
            TIMED_FRAMES as f64 * 1000.0 / total,
            if submit {
                format!(
                    "; waited {:.3} ms a frame to keep two frames in flight and {:.3} ms in all to drain before turns; timings dropped {} with the ring full and {} late",
                    waited / TIMED_FRAMES as f64,
                    drained,
                    now.ring - drops.ring,
                    now.late - drops.late
                )
            } else {
                String::new()
            }
        );
    }
    bench.submit = false;
}

#[cfg(test)]
#[path = "../manners/tests.rs"]
mod manners_tests;

#[cfg(test)]
mod tests;
