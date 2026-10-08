use std::path::PathBuf;

use crate::coat_room::{
    Arrays, HOUR, ROOM_MAX, ROOM_MIN, SEED, coated_material, cross, dot, floor, floor_material,
    identity, live_sun, luminance, read_ids, scratch, sky, toward_sun, trace_sun, unit,
    wall_material, walls,
};
use pfx_bake::reflection::{
    Cube, REFLECTION_FORMAT, ReflectionArtifact, ReflectionManifest, ReflectionSpec,
    bake_reflection,
};
use pfx_bake::{Anchor, BakeScene, GridSpec, bake, write_artifact};
use pfx_gpu::{Gpu, GpuProfiler};
use pfx_live::frame::{
    Camera, Frame, Instance, InstanceSurface, Matrix, Scene, SceneWind, multiply,
};
use pfx_live::maps::ContentFormat;
use pfx_live::probes::{ProbeLighting, verified_artifact};
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_live::sky::SkySource;
use pfx_materials::{Blend, ContentLayer, Material};
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Lens;
use pfx_trace::stage::{Image, Mesh, Placement, PlacementContent, Stage};
use pfx_trace::{Camera as TraceCamera, Projection};

pub const WALL_ID: u32 = 1;
pub const FLOOR_ID: u32 = 2;
pub const DESK_ID: u32 = 3;
pub const PLATE_ID: u32 = 4;
pub const FACE_ID: u32 = 5;
pub const CARD_ID: u32 = 6;
pub const CURL_ID: u32 = 7;

pub const DESK_MIN: [f32; 3] = [-0.9, 0.0, -1.95];
pub const DESK_MAX: [f32; 3] = [0.5, 0.72, -1.0];

pub const PLATE_AT: [f32; 3] = [-0.2, 0.72, -1.45];
pub const PLATE_TILT: f32 = 15.0;
pub const PLATE_SIZE: [f32; 3] = [0.012, 0.08, 0.24];
pub const PLATE_PRINT: [u32; 2] = [384, 128];
pub const CAPTURE: [f32; 3] = [-0.08, 0.95, -1.45];

pub const NEAR: f32 = 0.02;
pub const FAR: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
pub struct Eye {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov_y: f32,
}

impl Eye {
    pub fn axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let forward = unit(std::array::from_fn(|k| self.target[k] - self.eye[k]));
        let right = unit(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        (forward, right, up)
    }

    pub fn live(&self, width: u32, height: u32) -> Camera {
        let (forward, right, up) = self.axes();
        let eye = self.eye;
        let view: Matrix = [
            [right[0], up[0], -forward[0], 0.0],
            [right[1], up[1], -forward[1], 0.0],
            [right[2], up[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
        ];
        let tan = (self.fov_y.to_radians() * 0.5).tan();
        let aspect = width as f32 / height as f32;
        let projection: Matrix = [
            [1.0 / (tan * aspect), 0.0, 0.0, 0.0],
            [0.0, 1.0 / tan, 0.0, 0.0],
            [0.0, 0.0, FAR / (NEAR - FAR), -1.0],
            [0.0, 0.0, FAR * NEAR / (NEAR - FAR), 0.0],
        ];
        Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: eye,
        }
    }

    pub fn traced(&self, width: u32, height: u32) -> TraceCamera {
        let (forward, right, up) = self.axes();
        let tan = (self.fov_y.to_radians() * 0.5).tan();
        let aspect = width as f32 / height as f32;
        TraceCamera {
            origin: self.eye,
            forward,
            right: right.map(|v| v * tan * aspect),
            up: up.map(|v| v * tan),
        }
    }

    pub fn shadow(&self, width: u32, height: u32) -> View {
        let (forward, _, up) = self.axes();
        View {
            eye: self.eye,
            forward,
            up,
            fov_y: self.fov_y.to_radians(),
            aspect: width as f32 / height as f32,
            near: NEAR,
            far: FAR,
        }
    }

    pub fn ray(&self, x: f32, y: f32, width: u32, height: u32) -> [f32; 3] {
        let (forward, right, up) = self.axes();
        let tan = (self.fov_y.to_radians() * 0.5).tan();
        let aspect = width as f32 / height as f32;
        let nx = x / width as f32 * 2.0 - 1.0;
        let ny = 1.0 - y / height as f32 * 2.0;
        unit(std::array::from_fn(|k| {
            forward[k] + right[k] * nx * tan * aspect + up[k] * ny * tan
        }))
    }
}

pub struct Part {
    pub mesh: Arrays,
    pub material: u32,
    pub id: u32,
    pub printed: bool,
    pub release: f32,
}

pub struct Set {
    pub name: &'static str,
    pub parts: Vec<Part>,
    pub materials: Vec<Material>,
    pub print: [u32; 2],
    pub texels: Vec<[u8; 4]>,
    pub eye: Eye,
    pub cube: ReflectionSpec,
}

impl Set {
    pub fn triangles(&self) -> Vec<Triangle> {
        self.parts
            .iter()
            .flat_map(|part| part.mesh.triangles(part.material))
            .collect()
    }

    pub fn bake_scene(&self) -> BakeScene {
        BakeScene {
            triangles: self.triangles(),
            shapes: Vec::new(),
            materials: self.materials.clone(),
            anchors: vec![Anchor {
                hour: HOUR,
                sky: sky(),
                sun: trace_sun(),
            }],
        }
    }

    pub fn released(&self, release: bool) -> Vec<f32> {
        self.parts
            .iter()
            .map(|part| if release { part.release } else { 1.0 })
            .collect()
    }
}

fn no_content(mut arrays: Arrays) -> Arrays {
    arrays.uvs = arrays.uvs.iter().map(|_| [-1.0, -1.0]).collect();
    arrays
}

pub fn block(min: [f32; 3], max: [f32; 3]) -> Arrays {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let (dx, dy, dz) = (x1 - x0, y1 - y0, z1 - z0);
    let mut arrays = Arrays::new();
    arrays.quad([x1, y0, z0], [0.0, dy, 0.0], [0.0, 0.0, dz]);
    arrays.quad([x0, y0, z0], [0.0, 0.0, dz], [0.0, dy, 0.0]);
    arrays.quad([x0, y1, z0], [0.0, 0.0, dz], [dx, 0.0, 0.0]);
    arrays.quad([x0, y0, z0], [dx, 0.0, 0.0], [0.0, 0.0, dz]);
    arrays.quad([x0, y0, z1], [dx, 0.0, 0.0], [0.0, dy, 0.0]);
    arrays.quad([x0, y0, z0], [0.0, dy, 0.0], [dx, 0.0, 0.0]);
    no_content(arrays)
}

fn rotate_z(v: [f32; 3], degrees: f32) -> [f32; 3] {
    let (s, c) = degrees.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

fn placed(mut arrays: Arrays, degrees: f32, offset: [f32; 3]) -> Arrays {
    for p in &mut arrays.positions {
        let r = rotate_z(*p, degrees);
        *p = std::array::from_fn(|k| r[k] + offset[k]);
    }
    for n in &mut arrays.normals {
        *n = rotate_z(*n, degrees);
    }
    for t in &mut arrays.tangents {
        let r = rotate_z([t[0], t[1], t[2]], degrees);
        *t = [r[0], r[1], r[2], t[3]];
    }
    arrays
}

pub fn plate() -> (Arrays, Arrays) {
    let [t, h, w] = PLATE_SIZE;
    let body = block([-t, 0.0, -w * 0.5], [-0.0005, h, w * 0.5]);
    let mut face = Arrays::new();
    face.quad([0.0, 0.0, w * 0.5], [0.0, 0.0, -w], [0.0, h, 0.0]);
    (
        placed(body, PLATE_TILT, PLATE_AT),
        placed(face, PLATE_TILT, PLATE_AT),
    )
}

pub fn plate_normal() -> [f32; 3] {
    rotate_z([1.0, 0.0, 0.0], PLATE_TILT)
}

pub fn plate_uv(point: [f32; 3]) -> Option<[f32; 2]> {
    let [_, h, w] = PLATE_SIZE;
    let local: [f32; 3] = std::array::from_fn(|k| point[k] - PLATE_AT[k]);
    let local = rotate_z(local, -PLATE_TILT);
    let uv = [(w * 0.5 - local[2]) / w, local[1] / h];
    ((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1])).then_some(uv)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Core,
    Letters,
    Ground,
}

pub fn plate_ink(uv: [f32; 2]) -> Option<Ink> {
    let [u, v] = uv;
    if (0.08..=0.3).contains(&u) && (0.22..=0.78).contains(&v) {
        return Some(Ink::Core);
    }
    if (0.38..=0.92).contains(&u) && (0.22..=0.78).contains(&v) {
        let bar = ((u - 0.38) / 0.06).fract();
        return Some(if bar < 0.5 { Ink::Letters } else { Ink::Ground });
    }
    Some(Ink::Ground)
}

fn texels(size: [u32; 2], ink: impl Fn([f32; 2]) -> bool) -> Vec<[u8; 4]> {
    let [width, height] = size;
    (0..width * height)
        .map(|index| {
            let uv = [
                (index % width) as f32 / width as f32 + 0.5 / width as f32,
                (index / width) as f32 / height as f32 + 0.5 / height as f32,
            ];
            if ink(uv) {
                [3, 3, 4, 255]
            } else {
                [0, 0, 0, 0]
            }
        })
        .collect()
}

pub fn desk_material() -> Material {
    Material {
        base: [0.32, 0.24, 0.17],
        roughness: 0.65,
        specular: 0.04,
        ..Material::default()
    }
}

pub fn plate_material() -> Material {
    coated_material()
}

pub fn nameplate() -> Set {
    let (body, face) = plate();
    let part = |mesh, material, id| Part {
        mesh,
        material,
        id,
        printed: id == FACE_ID,
        release: 1.0,
    };
    Set {
        name: "nameplate",
        parts: vec![
            part(walls(), 0, WALL_ID),
            part(floor(), 1, FLOOR_ID),
            part(block(DESK_MIN, DESK_MAX), 2, DESK_ID),
            part(body, 3, PLATE_ID),
            part(face, 3, FACE_ID),
        ],
        materials: vec![
            wall_material(),
            floor_material(),
            desk_material(),
            plate_material(),
        ],
        print: PLATE_PRINT,
        texels: texels(PLATE_PRINT, |uv| {
            matches!(plate_ink(uv), Some(Ink::Core | Ink::Letters))
        }),
        eye: Eye {
            eye: [0.55, 1.08, -1.32],
            target: [-0.2, 0.76, -1.45],
            fov_y: 20.0,
        },
        cube: desk_cube(128),
    }
}

pub fn desk_cube(resolution: u32) -> ReflectionSpec {
    ReflectionSpec {
        name: Some("desk".into()),
        position: CAPTURE,
        min: [CAPTURE[0] - 0.5, CAPTURE[1] - 0.25, CAPTURE[2] - 0.45],
        max: [CAPTURE[0] + 0.05, CAPTURE[1] + 0.25, CAPTURE[2] + 0.45],
        resolution,
        anchors: None,
        fade: 0.25,
        priority: 0.0,
    }
}

pub fn room_cube(resolution: u32) -> ReflectionSpec {
    ReflectionSpec {
        name: Some("room".into()),
        position: CAPTURE,
        min: ROOM_MIN,
        max: ROOM_MAX,
        resolution,
        anchors: None,
        fade: 0.25,
        priority: 0.0,
    }
}

pub fn room_grid() -> GridSpec {
    GridSpec {
        min: [-1.8, 0.005, -1.8],
        max: [1.8, 2.405, 1.8],
        spacing: 0.3,
    }
}

pub fn desk_grid() -> GridSpec {
    GridSpec {
        min: [-0.6, 0.725, -1.85],
        max: [0.3, 1.125, -1.05],
        spacing: 0.05,
    }
}

pub fn bake_probes(gpu: &Gpu, set: &Set, name: &str, spec: GridSpec, samples: u32) -> PathBuf {
    let scene = set.bake_scene();
    let grids = bake(gpu, &scene, spec, samples, SEED).unwrap();
    let relative = PathBuf::from("local-cube").join(set.name).join(name);
    let root = scratch("").join(&relative);
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    write_artifact(&scratch(""), &relative, &scene, &grids, samples, SEED).unwrap();
    root
}

pub fn bake_cube(gpu: &Gpu, set: &Set, spec: &ReflectionSpec, samples: u32) -> Cube {
    bake_reflection(gpu, &set.bake_scene(), spec, samples, SEED, 0).unwrap()
}

pub fn cube_artifact(spec: ReflectionSpec, cube: Cube, samples: u32) -> ReflectionArtifact {
    let mut artifact = box_artifact(spec, cube, samples);
    artifact.manifest.format = REFLECTION_FORMAT.into();
    artifact
}

pub fn box_artifact(spec: ReflectionSpec, cube: Cube, samples: u32) -> ReflectionArtifact {
    ReflectionArtifact {
        manifest: ReflectionManifest {
            schema_version: 1,
            scene_hash: String::new(),
            spec,
            anchors: vec![HOUR],
            sample_count: samples,
            seed: SEED,
            format: String::new(),
            color_space: String::new(),
            coordinates: String::new(),
            files: Vec::new(),
        },
        cubes: vec![cube],
    }
}

pub fn trace(gpu: &Gpu, set: &Set, width: u32, height: u32, samples: u32) -> Vec<[f32; 3]> {
    let meshes: Vec<Mesh<'_>> = set.parts.iter().map(|part| part.mesh.staged()).collect();
    let placements: Vec<Placement> = set
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| Placement {
            content: part.printed.then(|| {
                PlacementContent::new(Image {
                    width: set.print[0],
                    height: set.print[1],
                    texels: &set.texels,
                    srgb: true,
                })
            }),
            ..Placement::new(index as u32, identity(), part.material)
        })
        .collect();
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &set.materials,
        sky: sky(),
        sun: trace_sun(),
        camera: set.eye.traced(width, height),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut trace = staged.trace(gpu, width, height).unwrap();
    let mut done = 0;
    let mut pass = 0;
    while done < samples {
        let count = (samples - done).min(4);
        trace.sample(gpu, count, SEED + 101 * pass).unwrap();
        done += count;
        pass += 1;
    }
    trace
        .readback(gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

pub struct Live {
    pub frame: Frame,
    pub shadows: Shadows,
    handles: Vec<pfx_live::frame::MeshHandle>,
}

impl Live {
    pub fn new(gpu: Gpu, set: &Set, width: u32, height: u32, probes: &[PathBuf]) -> Self {
        let mut frame = Frame::new(gpu, width, height).unwrap();
        frame.set_sky(SkySource::Hdr(sky()), true);
        frame
            .set_content(0, set.print[0], set.print[1], ContentFormat::Srgb8)
            .unwrap();
        let bytes: Vec<u8> = set.texels.iter().flatten().copied().collect();
        frame
            .write_content(0, [0, 0, set.print[0], set.print[1]], &bytes)
            .unwrap();
        let artifacts: Vec<_> = probes
            .iter()
            .map(|path| verified_artifact(path).unwrap())
            .collect();
        let lighting = ProbeLighting::from_artifacts(
            &frame.gpu.device,
            &frame.gpu.queue,
            &artifacts[0],
            &artifacts[1..],
            HOUR,
            [-1.0; 3],
        )
        .unwrap();
        frame.set_probes(lighting);
        let handles = set
            .parts
            .iter()
            .map(|part| frame.upload_mesh(part.mesh.data()).unwrap())
            .collect();
        let shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                receiver: Some(ReceiverBox {
                    min: ROOM_MIN,
                    max: ROOM_MAX,
                }),
                caster_margin: 3.0,
                ..Quality::default()
            },
        );
        Self {
            frame,
            shadows,
            handles,
        }
    }

    pub fn set_cube(&mut self, artifact: Option<ReflectionArtifact>) {
        self.frame
            .set_local_reflections(artifact.map(|a| vec![a]), HOUR)
            .unwrap();
    }

    pub fn render(
        &mut self,
        set: &Set,
        occlusion: f32,
        releases: &[f32],
        profiler: Option<&mut GpuProfiler>,
    ) -> Option<usize> {
        self.frame.set_reflection_occlusion(occlusion).unwrap();
        let surfaces: Vec<InstanceSurface> = releases
            .iter()
            .map(|&reflection_occlusion| InstanceSurface {
                reflection_occlusion,
                ..InstanceSurface::default()
            })
            .collect();
        self.frame.set_surfaces(&surfaces).unwrap();
        let width = self.frame.targets.width;
        let height = self.frame.targets.height;
        let instances: Vec<Instance> = set
            .parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let mut instance =
                    Instance::new(self.handles[index], identity(), part.material, part.id);
                instance.two_sided = true;
                instance
            })
            .collect();
        let scene = Scene {
            camera: set.eye.live(width, height),
            time: 0.0,
            seed: SEED,
            sun: live_sun(),
            instances: &instances,
            materials: &set.materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let fit = self
            .shadows
            .fit(&set.eye.shadow(width, height), toward_sun());
        let mut encoder = self
            .frame
            .gpu
            .device
            .create_command_encoder(&Default::default());
        let slot = match profiler {
            Some(profiler) => {
                self.frame
                    .encode(
                        &scene,
                        &mut encoder,
                        Some((&mut self.shadows, &fit)),
                        Some(&mut *profiler),
                    )
                    .unwrap();
                profiler.finish(&mut encoder)
            }
            None => {
                self.frame
                    .encode(&scene, &mut encoder, Some((&mut self.shadows, &fit)), None)
                    .unwrap();
                None
            }
        };
        self.frame.gpu.queue.submit(Some(encoder.finish()));
        slot
    }

    pub fn image(&self) -> Vec<[f32; 3]> {
        self.frame
            .gpu
            .readback_rgba16(&self.frame.targets.hdr)
            .unwrap()
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
            .collect()
    }

    pub fn ids(&self) -> Vec<u32> {
        read_ids(&self.frame)
    }
}

pub fn eroded<T: Copy + PartialEq>(
    width: u32,
    height: u32,
    reach: i64,
    label: impl Fn(u32, u32) -> Option<T>,
) -> Vec<Option<T>> {
    let labels: Vec<Option<T>> = (0..width * height)
        .map(|index| label(index % width, index / width))
        .collect();
    let (w, h) = (width as i64, height as i64);
    (0..w * h)
        .map(|index| {
            let (x, y) = (index % w, index / w);
            let own = labels[index as usize]?;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        return None;
                    }
                    if labels[(ny * w + nx) as usize] != Some(own) {
                        return None;
                    }
                }
            }
            Some(own)
        })
        .collect()
}

pub fn plate_regions(set: &Set, ids: &[u32], width: u32, height: u32) -> Vec<Option<Ink>> {
    let normal = plate_normal();
    let reach = (width as i64 / 480).max(1);
    eroded(width, height, reach, |x, y| {
        if ids[(y * width + x) as usize] != FACE_ID {
            return None;
        }
        let ray = set.eye.ray(x as f32 + 0.5, y as f32 + 0.5, width, height);
        let along = dot(ray, normal);
        if along >= 0.0 {
            return None;
        }
        let to: [f32; 3] = std::array::from_fn(|k| PLATE_AT[k] - set.eye.eye[k]);
        let t = dot(to, normal) / along;
        let hit = std::array::from_fn(|k| set.eye.eye[k] + ray[k] * t);
        plate_uv(hit).and_then(plate_ink)
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Agreement {
    pub pixels: usize,
    pub live: [f64; 3],
    pub traced: [f64; 3],
    pub ratio: f64,
    pub within: f64,
}

pub fn agreement<T: Copy + PartialEq>(
    live: &[[f32; 3]],
    traced: &[[f32; 3]],
    labels: &[Option<T>],
    region: T,
) -> Agreement {
    let selected: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, label)| **label == Some(region))
        .map(|(index, _)| index)
        .collect();
    let mut sum_live = [0.0f64; 3];
    let mut sum_traced = [0.0f64; 3];
    for &index in &selected {
        for c in 0..3 {
            sum_live[c] += f64::from(live[index][c]);
            sum_traced[c] += f64::from(traced[index][c]);
        }
    }
    let count = selected.len().max(1) as f64;
    let live_mean = sum_live.map(|v| v / count);
    let traced_mean = sum_traced.map(|v| v / count);
    let to_rgb = |v: [f64; 3]| [v[0] as f32, v[1] as f32, v[2] as f32];
    let floor = luminance(to_rgb(traced_mean)) * 0.05;
    let within = selected
        .iter()
        .filter(|&&index| {
            let ratio = (luminance(live[index]) + floor) / (luminance(traced[index]) + floor);
            (0.8..=1.25).contains(&ratio)
        })
        .count() as f64
        / count;
    Agreement {
        pixels: selected.len(),
        live: live_mean,
        traced: traced_mean,
        ratio: luminance(to_rgb(live_mean)) / luminance(to_rgb(traced_mean)).max(1e-12),
        within,
    }
}

pub fn tone(value: f32) -> u8 {
    let mapped = value * 1.4 / (1.0 + value * 1.4);
    let srgb = pfx_materials::encode_channel(mapped);
    (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn mean_abs_tone(live: &[[f32; 3]], traced: &[[f32; 3]], mask: &[bool]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for (index, &on) in mask.iter().enumerate() {
        if on {
            for c in 0..3 {
                sum += (f64::from(tone(live[index][c])) - f64::from(tone(traced[index][c]))).abs();
            }
            count += 3.0;
        }
    }
    sum / f64::max(count, 1.0)
}

pub const STRIP_AT: [f32; 3] = [-0.25, 0.72, -1.35];
pub const CARD_WIDTH: f32 = 0.06;
pub const CARD_LENGTH: f32 = 0.09;
pub const CARD_GAP: f32 = 0.003;
pub const ROLL_RADIUS: f32 = 0.009;
pub const ROLL_TURN: f32 = 1.5 * std::f32::consts::PI;
pub const CARD_LIFT: f32 = 0.0004;
pub const CARD_PRINT: [u32; 2] = [128, 192];

pub fn card_material() -> Material {
    Material {
        base: [0.82, 0.8, 0.74],
        roughness: 0.6,
        specular: 0.04,
        content_layer: ContentLayer {
            slot: 0,
            blend: Blend::Over,
            ink_roughness: 0.25,
            emboss: 0.0,
            strength: 1.0,
        },
        ..Material::default()
    }
}

pub fn card_ink(uv: [f32; 2]) -> bool {
    let [u, v] = uv;
    if !(0.08..=0.92).contains(&u) || !(0.12..=0.88).contains(&v) {
        return false;
    }
    if v < 0.42 {
        return (u - 0.5).abs() < (0.42 - v) * 0.9;
    }
    ((v - 0.5) / 0.09).fract() < 0.55
}

pub fn roll_profile(s: f32) -> ([f32; 2], [f32; 2]) {
    let flat = CARD_WIDTH - ROLL_RADIUS * ROLL_TURN;
    if s <= flat {
        return ([s, 0.0], [0.0, 1.0]);
    }
    let angle = (s - flat) / ROLL_RADIUS;
    let (sin, cos) = angle.sin_cos();
    (
        [flat + ROLL_RADIUS * sin, ROLL_RADIUS * (1.0 - cos)],
        [-sin, cos],
    )
}

pub fn card(left: f32, curled: bool) -> Arrays {
    let steps = if curled { 48 } else { 1 };
    let mut arrays = Arrays::new();
    let [x0, y0, z0] = STRIP_AT;
    for i in 0..=steps {
        let a = i as f32 / steps as f32;
        let s = CARD_WIDTH * a;
        let ([across, rise], [nx, ny]) = if curled {
            roll_profile(s)
        } else {
            ([s, 0.0], [0.0, 1.0])
        };
        for (b, along) in [(0.0, CARD_LENGTH * 0.5), (1.0, -CARD_LENGTH * 0.5)] {
            arrays
                .positions
                .push([x0 + left + across, y0 + CARD_LIFT + rise, z0 + along]);
            arrays.normals.push([nx, ny, 0.0]);
            arrays.tangents.push([ny, -nx, 0.0, 1.0]);
            arrays.uvs.push([a, b]);
        }
    }
    for i in 0..steps {
        let s = (i * 2) as u32;
        arrays.indices.extend([s, s + 2, s + 3, s, s + 3, s + 1]);
    }
    arrays
}

pub fn card_lefts() -> [f32; 3] {
    let pitch = CARD_WIDTH + CARD_GAP;
    [-pitch * 1.5, -pitch * 0.5, pitch * 0.5]
}

pub fn curl() -> Set {
    let [left, middle, right] = card_lefts();
    let part = |mesh, material, id: u32| Part {
        mesh,
        material,
        id,
        printed: id == CARD_ID || id == CURL_ID,
        release: if id == CURL_ID { 0.0 } else { 1.0 },
    };
    Set {
        name: "curl",
        parts: vec![
            part(walls(), 0, WALL_ID),
            part(floor(), 1, FLOOR_ID),
            part(block(DESK_MIN, DESK_MAX), 2, DESK_ID),
            part(card(left, false), 3, CARD_ID),
            part(card(middle, true), 3, CURL_ID),
            part(card(right, false), 3, CARD_ID),
        ],
        materials: vec![
            wall_material(),
            floor_material(),
            desk_material(),
            card_material(),
        ],
        print: CARD_PRINT,
        texels: texels(CARD_PRINT, card_ink),
        eye: Eye {
            eye: [STRIP_AT[0] + 0.03, STRIP_AT[1] + 0.2, STRIP_AT[2] + 0.2],
            target: [STRIP_AT[0], STRIP_AT[1] + 0.006, STRIP_AT[2]],
            fov_y: 40.0,
        },
        cube: ReflectionSpec {
            name: Some("strip".into()),
            position: [STRIP_AT[0], STRIP_AT[1] + 0.3, STRIP_AT[2]],
            min: ROOM_MIN,
            max: ROOM_MAX,
            resolution: 128,
            anchors: None,
            fade: 0.25,
            priority: 0.0,
        },
    }
}

pub fn strip_grid() -> GridSpec {
    GridSpec {
        min: [-0.375, 0.7265625, -1.4375],
        max: [-0.125, 0.7734375, -1.265625],
        spacing: 0.0078125,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strip {
    FlatInk,
    FlatCard,
    CurlInk,
    CurlCard,
}

fn barycentric_uv(arrays: &Arrays, triangle: usize, point: [f32; 3]) -> [f32; 2] {
    let corner = |k: usize| arrays.indices[triangle * 3 + k] as usize;
    let [a, b, c] = [0, 1, 2].map(|k| arrays.positions[corner(k)]);
    let ab: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
    let ac: [f32; 3] = std::array::from_fn(|k| c[k] - a[k]);
    let ap: [f32; 3] = std::array::from_fn(|k| point[k] - a[k]);
    let (d00, d01, d11) = (dot(ab, ab), dot(ab, ac), dot(ac, ac));
    let (d20, d21) = (dot(ap, ab), dot(ap, ac));
    let denominator = d00 * d11 - d01 * d01;
    let v = (d11 * d20 - d01 * d21) / denominator;
    let w = (d00 * d21 - d01 * d20) / denominator;
    let u = 1.0 - v - w;
    let [ua, ub, uc] = [0, 1, 2].map(|k| arrays.uvs[corner(k)]);
    std::array::from_fn(|k| ua[k] * u + ub[k] * v + uc[k] * w)
}

pub fn strip_regions(set: &Set, ids: &[u32], width: u32, height: u32) -> Vec<Option<Strip>> {
    let reach = (width as i64 / 480).max(1);
    let mut triangles = Vec::new();
    let mut owners = Vec::new();
    for (index, part) in set.parts.iter().enumerate() {
        let start = triangles.len();
        triangles.extend(part.mesh.triangles(index as u32));
        owners.extend((0..triangles.len() - start).map(|k| (index, k)));
    }
    let bvh = pfx_trace::bvh::Bvh::build(&triangles);
    eroded(width, height, reach, |x, y| {
        let id = ids[(y * width + x) as usize];
        if id != CARD_ID && id != CURL_ID {
            return None;
        }
        let ray = set.eye.ray(x as f32 + 0.5, y as f32 + 0.5, width, height);
        let hit = bvh.intersect(
            &triangles,
            pfx_trace::bvh::Ray {
                origin: set.eye.eye,
                direction: ray,
            },
            f32::INFINITY,
        )?;
        let (part, triangle) = owners[hit.index as usize];
        if set.parts[part].id != id {
            return None;
        }
        let point = std::array::from_fn(|k| set.eye.eye[k] + ray[k] * hit.distance);
        let inked = card_ink(barycentric_uv(&set.parts[part].mesh, triangle, point));
        Some(match (id == CURL_ID, inked) {
            (false, true) => Strip::FlatInk,
            (false, false) => Strip::FlatCard,
            (true, true) => Strip::CurlInk,
            (true, false) => Strip::CurlCard,
        })
    })
}

pub const TUNNEL_AT: [f32; 3] = [0.25, 0.72, -1.25];
pub const TUNNEL_HALF: [f32; 2] = [0.03, 0.04];
pub const BRIDGE_ID: u32 = 8;

pub fn glossy_card() -> Material {
    Material {
        base: [0.03, 0.03, 0.035],
        roughness: 0.2,
        specular: 0.04,
        ..Material::default()
    }
}

pub fn tunnel() -> Set {
    let [x, y, z] = TUNNEL_AT;
    let [hx, hz] = TUNNEL_HALF;
    let mut lying = Arrays::new();
    lying.quad(
        [x - hx, y + CARD_LIFT, z + hz],
        [2.0 * hx, 0.0, 0.0],
        [0.0, 0.0, -2.0 * hz],
    );
    let part = |mesh, material, id| Part {
        mesh,
        material,
        id,
        printed: false,
        release: 1.0,
    };
    Set {
        name: "tunnel",
        parts: vec![
            part(walls(), 0, WALL_ID),
            part(floor(), 1, FLOOR_ID),
            part(block(DESK_MIN, DESK_MAX), 2, DESK_ID),
            part(no_content(lying), 3, CARD_ID),
            part(
                block(
                    [x - hx - 0.02, y + 0.015, z - hz],
                    [x + hx + 0.02, y + 0.02, z + hz * 0.25],
                ),
                2,
                BRIDGE_ID,
            ),
        ],
        materials: vec![
            wall_material(),
            floor_material(),
            desk_material(),
            glossy_card(),
        ],
        print: CARD_PRINT,
        texels: texels(CARD_PRINT, |_| false),
        eye: Eye {
            eye: [x, y + 0.03, z + 0.3],
            target: [x, y, z],
            fov_y: 16.0,
        },
        cube: ReflectionSpec {
            name: Some("tunnel".into()),
            position: [x, y + 0.3, z],
            min: ROOM_MIN,
            max: ROOM_MAX,
            resolution: 128,
            anchors: None,
            fade: 0.25,
            priority: 0.0,
        },
    }
}

pub fn tunnel_grid() -> GridSpec {
    GridSpec {
        min: [0.1875, 0.7265625, -1.3125],
        max: [0.3125, 0.7734375, -1.1875],
        spacing: 0.0078125,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    Covered,
    Open,
}

pub fn tunnel_regions(set: &Set, ids: &[u32], width: u32, height: u32) -> Vec<Option<Cover>> {
    let reach = (width as i64 / 480).max(1);
    let [_, y, z] = TUNNEL_AT;
    let [_, hz] = TUNNEL_HALF;
    eroded(width, height, reach, |px, py| {
        if ids[(py * width + px) as usize] != CARD_ID {
            return None;
        }
        let ray = set.eye.ray(px as f32 + 0.5, py as f32 + 0.5, width, height);
        let t = (y + CARD_LIFT - set.eye.eye[1]) / ray[1];
        let hit_z = set.eye.eye[2] + ray[2] * t;
        if hit_z < z + hz * 0.25 - 0.005 {
            Some(Cover::Covered)
        } else if hit_z > z + hz * 0.25 + 0.005 {
            Some(Cover::Open)
        } else {
            None
        }
    })
}
