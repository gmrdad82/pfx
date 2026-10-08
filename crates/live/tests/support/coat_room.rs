use std::path::{Path, PathBuf};

use pfx_bake::reflection::{
    ReflectionArtifact, ReflectionManifest, ReflectionSpec, bake_reflection,
};
use pfx_bake::{Anchor, BakeScene, GridSpec, bake, write_artifact};
use pfx_gpu::{Gpu, GpuProfiler, PassTiming, wgpu};
use pfx_live::frame::{Camera, Frame, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::maps::ContentFormat;
use pfx_live::probes::{ProbeLighting, verified_artifact};
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::{Blend, ContentLayer, Material};
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Lens;
use pfx_trace::stage::{Image, Mesh, Placement, PlacementContent, Stage};
use pfx_trace::{Camera as TraceCamera, Projection, Sun as TraceSun};

pub const ROOM_MIN: [f32; 3] = [-2.0, 0.0, -2.0];
pub const ROOM_MAX: [f32; 3] = [2.0, 2.8, 2.0];
pub const WINDOW: [f32; 4] = [-0.8, 0.8, 1.0, 2.2];
pub const BOX_CENTRE: [f32; 3] = [0.0, 0.07, 0.4];
pub const BOX_HALF: [f32; 3] = [0.26, 0.07, 0.19];
pub const BOX_RADIUS: f32 = 0.03;
pub const EYE: [f32; 3] = [0.0, 1.04, 1.03];
pub const TARGET: [f32; 3] = [0.0, 0.12, 0.4];
pub const FOV_Y: f32 = 32.0;
pub const NEAR: f32 = 0.05;
pub const FAR: f32 = 12.0;
pub const HOUR: f32 = 12.0;
pub const SEED: u32 = 23;
pub const PRINT: [u32; 2] = [288, 200];
pub const BODY_ID: u32 = 3;
pub const TOP_ID: u32 = 4;
pub const FLOOR_ID: u32 = 2;
pub const WALL_ID: u32 = 1;
pub const CUBE_CENTRE: [f32; 3] = [0.0, 1.4, 0.0];

pub struct Arrays {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Arrays {
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        }
    }

    pub fn data(&self) -> MeshData<'_> {
        MeshData {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            uvs1: None,
            alpha: None,
            indices: &self.indices,
        }
    }

    pub fn staged(&self) -> Mesh<'_> {
        Mesh {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            alpha: None,
            indices: &self.indices,
        }
    }

    pub fn triangles(&self, material: u32) -> Vec<Triangle> {
        self.indices
            .chunks_exact(3)
            .map(|corner| Triangle {
                vertices: std::array::from_fn(|k| self.positions[corner[k] as usize]),
                material,
            })
            .collect()
    }

    pub fn quad(&mut self, origin: [f32; 3], du: [f32; 3], dv: [f32; 3]) {
        let normal = unit(cross(du, dv));
        let tangent = unit(du);
        let start = self.positions.len() as u32;
        for (a, b) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            self.positions
                .push(std::array::from_fn(|k| origin[k] + du[k] * a + dv[k] * b));
            self.normals.push(normal);
            self.tangents
                .push([tangent[0], tangent[1], tangent[2], 1.0]);
            self.uvs.push([a, b]);
        }
        self.indices
            .extend([0, 1, 2, 0, 2, 3].map(|index| start + index));
    }
}

impl Default for Arrays {
    fn default() -> Self {
        Self::new()
    }
}

pub fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn walls() -> Arrays {
    let [x0, y0, z0] = ROOM_MIN;
    let [x1, y1, z1] = ROOM_MAX;
    let [wx0, wx1, wy0, wy1] = WINDOW;
    let mut arrays = Arrays::new();
    arrays.quad([x0, y1, z0], [x1 - x0, 0.0, 0.0], [0.0, 0.0, z1 - z0]);
    arrays.quad([x1, y0, z1], [x0 - x1, 0.0, 0.0], [0.0, y1 - y0, 0.0]);
    arrays.quad([x0, y0, z0], [0.0, y1 - y0, 0.0], [0.0, 0.0, z1 - z0]);
    arrays.quad([x1, y0, z0], [0.0, 0.0, z1 - z0], [0.0, y1 - y0, 0.0]);
    arrays.quad([x0, y0, z0], [x1 - x0, 0.0, 0.0], [0.0, wy0 - y0, 0.0]);
    arrays.quad([x0, wy1, z0], [x1 - x0, 0.0, 0.0], [0.0, y1 - wy1, 0.0]);
    arrays.quad([x0, wy0, z0], [wx0 - x0, 0.0, 0.0], [0.0, wy1 - wy0, 0.0]);
    arrays.quad([wx1, wy0, z0], [x1 - wx1, 0.0, 0.0], [0.0, wy1 - wy0, 0.0]);
    arrays
}

pub fn floor() -> Arrays {
    let [x0, y0, z0] = ROOM_MIN;
    let [x1, _, z1] = ROOM_MAX;
    let mut arrays = Arrays::new();
    arrays.quad([x0, y0, z0], [0.0, 0.0, z1 - z0], [x1 - x0, 0.0, 0.0]);
    let uvs: Vec<[f32; 2]> = arrays.uvs.iter().map(|_| [-1.0, -1.0]).collect();
    arrays.uvs = uvs;
    arrays
}

fn axis_samples(inner: f32, steps: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for i in (0..=steps).rev() {
        let t = (std::f32::consts::FRAC_PI_4 * i as f32 / steps as f32).tan();
        out.push(-inner - BOX_RADIUS * t);
    }
    for i in 0..=steps {
        let t = (std::f32::consts::FRAC_PI_4 * i as f32 / steps as f32).tan();
        out.push(inner + BOX_RADIUS * t);
    }
    out
}

pub fn inner() -> [f32; 3] {
    BOX_HALF.map(|h| h - BOX_RADIUS)
}

pub fn rounded_box() -> (Arrays, Arrays) {
    let steps = 6;
    let inner = inner();
    let mut body = Arrays::new();
    let mut top = Arrays::new();
    for face in 0..6 {
        let axis = face / 2;
        let sign = if face % 2 == 0 { 1.0 } else { -1.0 };
        let u_axis = (axis + 1) % 3;
        let v_axis = (axis + 2) % 3;
        let us = axis_samples(inner[u_axis], steps);
        let vs = axis_samples(inner[v_axis], steps);
        let point = |i: usize, j: usize| {
            let mut p = [0.0_f32; 3];
            p[axis] = sign * BOX_HALF[axis];
            p[u_axis] = us[i];
            p[v_axis] = vs[j];
            let c: [f32; 3] = std::array::from_fn(|k| p[k].clamp(-inner[k], inner[k]));
            let n = unit(std::array::from_fn(|k| p[k] - c[k]));
            let position: [f32; 3] =
                std::array::from_fn(|k| BOX_CENTRE[k] + c[k] + BOX_RADIUS * n[k]);
            let mut tangent = [0.0_f32; 3];
            tangent[u_axis] = 1.0;
            let tangent = unit(std::array::from_fn(|k| tangent[k] - n[k] * n[u_axis]));
            (position, n, tangent)
        };
        for i in 0..us.len() - 1 {
            for j in 0..vs.len() - 1 {
                let flat_top = axis == 1 && sign > 0.0 && i == steps && j == steps;
                let target = if flat_top { &mut top } else { &mut body };
                let start = target.positions.len() as u32;
                for (a, b) in [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)] {
                    let (position, normal, tangent) = point(a, b);
                    target.positions.push(position);
                    target.normals.push(normal);
                    target
                        .tangents
                        .push([tangent[0], tangent[1], tangent[2], 1.0]);
                    target.uvs.push(if flat_top {
                        top_uv(position)
                    } else {
                        [-1.0, -1.0]
                    });
                }
                let quad = if sign > 0.0 {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                target.indices.extend(quad.map(|k| start + k));
            }
        }
    }
    (body, top)
}

pub fn top_y() -> f32 {
    BOX_CENTRE[1] + BOX_HALF[1]
}

pub fn top_uv(position: [f32; 3]) -> [f32; 2] {
    let inner = inner();
    [
        (position[0] - BOX_CENTRE[0] + inner[0]) / (2.0 * inner[0]),
        (position[2] - BOX_CENTRE[2] + inner[2]) / (2.0 * inner[2]),
    ]
}

pub fn print_alpha(uv: [f32; 2]) -> bool {
    let [u, v] = uv;
    if !(0.12..=0.88).contains(&u) || !(0.18..=0.82).contains(&v) {
        return false;
    }
    if (0.12..=0.40).contains(&u) && (0.18..=0.48).contains(&v) {
        return true;
    }
    if (0.50..=0.88).contains(&u) {
        let line = ((v - 0.18) / 0.08).floor();
        let within = (v - 0.18) / 0.08 - line;
        return within < 0.55 && line < 8.0;
    }
    (0.56..=0.82).contains(&v) && ((u - 0.12) / 0.05).fract() < 0.6
}

pub fn print_texels() -> Vec<[u8; 4]> {
    let [width, height] = PRINT;
    (0..width * height)
        .map(|index| {
            let x = index % width;
            let y = index / width;
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            if print_alpha(uv) {
                [3, 3, 4, 255]
            } else {
                [0, 0, 0, 0]
            }
        })
        .collect()
}

pub fn wall_material() -> Material {
    Material {
        base: [0.62, 0.6, 0.56],
        roughness: 0.9,
        specular: 0.0,
        ..Material::default()
    }
}

pub fn floor_material() -> Material {
    Material {
        base: [0.34, 0.29, 0.25],
        roughness: 0.7,
        specular: 0.04,
        ..Material::default()
    }
}

pub fn coated_material() -> Material {
    Material {
        base: [0.66, 0.13, 0.03],
        roughness: 0.5,
        specular: 0.04,
        clearcoat: 1.0,
        clearcoat_roughness: 0.06,
        content_layer: ContentLayer {
            slot: 0,
            blend: Blend::Over,
            ink_roughness: 0.6,
            emboss: 0.0,
            strength: 1.0,
        },
        ..Material::default()
    }
}

pub fn materials() -> [Material; 3] {
    [wall_material(), floor_material(), coated_material()]
}

pub fn matte(materials: [Material; 3]) -> [Material; 3] {
    materials.map(|material| Material {
        specular: 0.0,
        clearcoat: 0.0,
        ..material
    })
}

pub fn toward_sun() -> [f32; 3] {
    unit([0.25, 0.9, -0.36])
}

pub const SUN_INTENSITY: f32 = 8.0;

pub fn sky() -> Sky {
    let width = 64;
    let height = 32;
    let zenith = [0.42, 0.62, 1.2];
    let horizon = [1.5, 1.5, 1.55];
    let ground = [0.18, 0.165, 0.15];
    let texels = (0..width * height)
        .map(|index| {
            let y = index / width;
            let theta = std::f32::consts::PI * (y as f32 + 0.5) / height as f32;
            let up = theta.cos();
            let rgb: [f32; 3] = if up >= 0.0 {
                let t = up.powf(0.6);
                std::array::from_fn(|c| horizon[c] + (zenith[c] - horizon[c]) * t)
            } else {
                ground
            };
            [rgb[0], rgb[1], rgb[2], 1.0]
        })
        .collect();
    Sky {
        width,
        height,
        texels,
    }
}

pub struct Room {
    pub materials: [Material; 3],
    pub walls: Arrays,
    pub floor: Arrays,
    pub body: Arrays,
    pub top: Arrays,
}

impl Room {
    pub fn new() -> Self {
        let (body, top) = rounded_box();
        Self {
            materials: materials(),
            walls: walls(),
            floor: floor(),
            body,
            top,
        }
    }

    pub fn meshes(&self) -> [&Arrays; 4] {
        [&self.walls, &self.floor, &self.body, &self.top]
    }

    pub fn placements(&self) -> [(u32, u32, u32); 4] {
        [
            (0, 0, WALL_ID),
            (1, 1, FLOOR_ID),
            (2, 2, BODY_ID),
            (3, 2, TOP_ID),
        ]
    }

    pub fn triangles(&self) -> Vec<Triangle> {
        let mut triangles = Vec::new();
        for (mesh, material, _) in self.placements() {
            triangles.extend(self.meshes()[mesh as usize].triangles(material));
        }
        triangles
    }
}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

pub fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn axes() -> ([f32; 3], [f32; 3], [f32; 3]) {
    let forward = unit(std::array::from_fn(|k| TARGET[k] - EYE[k]));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    (forward, right, up)
}

pub fn live_camera(width: u32, height: u32) -> Camera {
    let (forward, right, up) = axes();
    let view: Matrix = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, EYE), -dot(up, EYE), dot(forward, EYE), 1.0],
    ];
    let tan = (FOV_Y.to_radians() * 0.5).tan();
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
        position: EYE,
    }
}

pub fn trace_camera(width: u32, height: u32) -> TraceCamera {
    let (forward, right, up) = axes();
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = width as f32 / height as f32;
    TraceCamera {
        origin: EYE,
        forward,
        right: right.map(|v| v * tan * aspect),
        up: up.map(|v| v * tan),
    }
}

pub fn shadow_view(width: u32, height: u32) -> View {
    let (forward, _, up) = axes();
    View {
        eye: EYE,
        forward,
        up,
        fov_y: FOV_Y.to_radians(),
        aspect: width as f32 / height as f32,
        near: NEAR,
        far: FAR,
    }
}

pub fn pixel_ray(x: f32, y: f32, width: u32, height: u32) -> [f32; 3] {
    let (forward, right, up) = axes();
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = width as f32 / height as f32;
    let nx = x / width as f32 * 2.0 - 1.0;
    let ny = 1.0 - y / height as f32 * 2.0;
    unit(std::array::from_fn(|k| {
        forward[k] + right[k] * nx * tan * aspect + up[k] * ny * tan
    }))
}

pub fn printed_at(x: f32, y: f32, width: u32, height: u32) -> Option<bool> {
    let ray = pixel_ray(x, y, width, height);
    if ray[1] >= 0.0 {
        return None;
    }
    let t = (top_y() - EYE[1]) / ray[1];
    let hit: [f32; 3] = std::array::from_fn(|k| EYE[k] + ray[k] * t);
    let uv = top_uv(hit);
    if !(0.0..=1.0).contains(&uv[0]) || !(0.0..=1.0).contains(&uv[1]) {
        return None;
    }
    Some(print_alpha(uv))
}

pub fn trace_sun() -> TraceSun {
    TraceSun {
        direction: toward_sun(),
        color: [1.0; 3],
        intensity: SUN_INTENSITY,
    }
}

pub fn live_sun() -> Sun {
    Sun {
        direction: toward_sun(),
        colour: [1.0; 3],
        intensity: SUN_INTENSITY,
    }
}

pub fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(name)
}

pub fn bake_scene(room: &Room) -> BakeScene {
    BakeScene {
        triangles: room.triangles(),
        shapes: Vec::new(),
        materials: room.materials.to_vec(),
        anchors: vec![Anchor {
            hour: HOUR,
            sky: sky(),
            sun: trace_sun(),
        }],
    }
}

pub fn grid_spec() -> GridSpec {
    GridSpec {
        min: [-1.8, 0.005, -1.8],
        max: [1.8, 2.405, 1.8],
        spacing: 0.3,
    }
}

pub fn local_spec() -> GridSpec {
    GridSpec {
        min: [-0.4, 0.005, 0.1],
        max: [0.4, 0.305, 0.7],
        spacing: 0.05,
    }
}

pub fn bake_probes(gpu: &Gpu, room: &Room, name: &str, spec: GridSpec, samples: u32) -> PathBuf {
    let scene = bake_scene(room);
    let grids = bake(gpu, &scene, spec, samples, SEED).unwrap();
    let relative = Path::new("coat-room").join(name);
    let root = scratch("coat-room").join(name);
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    write_artifact(&scratch(""), &relative, &scene, &grids, samples, SEED).unwrap();
    root
}

pub fn cube_spec(fade: f32, resolution: u32) -> ReflectionSpec {
    ReflectionSpec {
        name: Some("room".into()),
        position: CUBE_CENTRE,
        min: ROOM_MIN,
        max: ROOM_MAX,
        resolution,
        anchors: None,
        fade,
        priority: 0.0,
    }
}

pub fn bake_cube(
    gpu: &Gpu,
    room: &Room,
    spec: &ReflectionSpec,
    samples: u32,
) -> pfx_bake::reflection::Cube {
    bake_reflection(gpu, &bake_scene(room), spec, samples, SEED, 0).unwrap()
}

pub fn cube_artifact(
    spec: ReflectionSpec,
    cube: pfx_bake::reflection::Cube,
    samples: u32,
) -> ReflectionArtifact {
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

pub fn trace(gpu: &Gpu, room: &Room, width: u32, height: u32, samples: u32) -> Vec<[f32; 3]> {
    let meshes: Vec<Mesh<'_>> = room.meshes().iter().map(|a| a.staged()).collect();
    let texels = print_texels();
    let placements: Vec<Placement> = room
        .placements()
        .iter()
        .map(|&(mesh, material, id)| Placement {
            content: (id == TOP_ID).then(|| {
                PlacementContent::new(Image {
                    width: PRINT[0],
                    height: PRINT[1],
                    texels: &texels,
                    srgb: true,
                })
            }),
            ..Placement::new(mesh, identity(), material)
        })
        .collect();
    let materials = room.materials;
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: sky(),
        sun: trace_sun(),
        camera: trace_camera(width, height),
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    pub cube: bool,
    pub occlusion: f32,
}

pub struct Live {
    pub frame: Frame,
    pub shadows: Shadows,
    handles: Vec<pfx_live::frame::MeshHandle>,
}

impl Live {
    pub fn new(gpu: Gpu, room: &Room, width: u32, height: u32, probes: &[PathBuf]) -> Self {
        let mut frame = Frame::new(gpu, width, height).unwrap();
        frame.set_sky(SkySource::Hdr(sky()), true);
        frame
            .set_content(0, PRINT[0], PRINT[1], ContentFormat::Srgb8)
            .unwrap();
        let bytes: Vec<u8> = print_texels().iter().flatten().copied().collect();
        frame
            .write_content(0, [0, 0, PRINT[0], PRINT[1]], &bytes)
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
        let handles = room
            .meshes()
            .iter()
            .map(|a| frame.upload_mesh(a.data()).unwrap())
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
        room: &Room,
        occlusion: f32,
        profiler: Option<&mut GpuProfiler>,
    ) -> Option<usize> {
        self.frame.set_reflection_occlusion(occlusion).unwrap();
        let width = self.frame.targets.width;
        let height = self.frame.targets.height;
        let instances: Vec<Instance> = room
            .placements()
            .iter()
            .map(|&(mesh, material, id)| {
                let mut instance =
                    Instance::new(self.handles[mesh as usize], identity(), material, id);
                instance.two_sided = id == WALL_ID || id == FLOOR_ID;
                instance
            })
            .collect();
        let materials = room.materials;
        let scene = Scene {
            camera: live_camera(width, height),
            time: 0.0,
            seed: SEED,
            sun: live_sun(),
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let fit = self.shadows.fit(&shadow_view(width, height), toward_sun());
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

pub fn read_ids(frame: &Frame) -> Vec<u32> {
    let texture = &frame.targets.ids;
    let size = texture.size();
    let row = (size.width * 4).div_ceil(256) * 256;
    let buffer = frame.gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("coat room ids readback"),
        size: u64::from(row * size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    frame.gpu.queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    frame
        .gpu
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    let mut ids = Vec::new();
    for y in 0..size.height {
        let start = (y * row) as usize;
        ids.extend(
            mapped[start..start + (size.width * 4) as usize]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
        );
    }
    drop(mapped);
    buffer.unmap();
    ids
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Coat,
    Print,
    Body,
    Floor,
}

impl Region {
    pub const ALL: [Region; 4] = [Region::Coat, Region::Print, Region::Body, Region::Floor];

    pub fn name(self) -> &'static str {
        match self {
            Region::Coat => "top coat",
            Region::Print => "print",
            Region::Body => "bevel and sides",
            Region::Floor => "floor",
        }
    }
}

pub fn regions(ids: &[u32], width: u32, height: u32) -> Vec<Option<Region>> {
    let w = width as i64;
    let h = height as i64;
    let printed: Vec<Option<bool>> = (0..width * height)
        .map(|index| {
            let x = (index % width) as f32 + 0.5;
            let y = (index / width) as f32 + 0.5;
            printed_at(x, y, width, height)
        })
        .collect();
    let reach = (width as i64 / 480).max(1);
    (0..w * h)
        .map(|index| {
            let x = index % w;
            let y = index / w;
            let id = ids[index as usize];
            let mut same = true;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        same = false;
                        continue;
                    }
                    let other = (ny * w + nx) as usize;
                    if ids[other] != id
                        || (id == TOP_ID && printed[other] != printed[index as usize])
                    {
                        same = false;
                    }
                }
            }
            if !same {
                return None;
            }
            match id {
                TOP_ID => match printed[index as usize] {
                    Some(true) => Some(Region::Print),
                    Some(false) => Some(Region::Coat),
                    None => None,
                },
                BODY_ID => Some(Region::Body),
                FLOOR_ID => Some(Region::Floor),
                _ => None,
            }
        })
        .collect()
}

pub fn luminance(rgb: [f32; 3]) -> f64 {
    0.2126 * f64::from(rgb[0]) + 0.7152 * f64::from(rgb[1]) + 0.0722 * f64::from(rgb[2])
}

#[derive(Clone, Copy, Debug)]
pub struct Agreement {
    pub pixels: usize,
    pub live: f64,
    pub traced: f64,
    pub ratio: f64,
    pub p50: f64,
    pub p95: f64,
    pub within: f64,
}

pub fn agreement(
    live: &[[f32; 3]],
    traced: &[[f32; 3]],
    labels: &[Option<Region>],
    region: Region,
) -> Agreement {
    let mut ratios = Vec::new();
    let mut sum_live = 0.0;
    let mut sum_traced = 0.0;
    let selected: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, label)| **label == Some(region))
        .map(|(index, _)| index)
        .collect();
    for &index in &selected {
        sum_live += luminance(live[index]);
        sum_traced += luminance(traced[index]);
    }
    let count = selected.len().max(1) as f64;
    let floor = sum_traced / count * 0.05;
    for &index in &selected {
        let a = luminance(live[index]);
        let b = luminance(traced[index]);
        ratios.push((a + floor) / (b + floor));
    }
    ratios.sort_by(f64::total_cmp);
    let at = |q: f64| {
        if ratios.is_empty() {
            f64::NAN
        } else {
            ratios[((ratios.len() - 1) as f64 * q).round() as usize]
        }
    };
    let within = ratios
        .iter()
        .filter(|ratio| (0.8..=1.25).contains(*ratio))
        .count() as f64
        / count;
    Agreement {
        pixels: selected.len(),
        live: sum_live / count,
        traced: sum_traced / count,
        ratio: sum_live / sum_traced.max(1e-12),
        p50: at(0.5),
        p95: at(0.95),
        within,
    }
}

pub fn timings_ms(timings: &[PassTiming], label: &str) -> Option<f64> {
    timings
        .iter()
        .find(|timing| timing.label == label)
        .map(|timing| timing.milliseconds)
}
