use std::path::{Path, PathBuf};

use pfx_bake::{Anchor, BakeScene, GridSpec, bake, write_artifact};
use pfx_gpu::Gpu;
use pfx_live::frame::{Camera, Frame, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::probes::ProbeLighting;
use pfx_live::sky::SkySource;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::{Camera as TraceCamera, Scene as TraceScene, Sun as TraceSun, Trace};

const SIZE: u32 = 96;
const FOV_Y: f32 = 40.0;
const NEAR: f32 = 0.05;
const FAR: f32 = 20.0;
const TRACE_SAMPLES: u32 = 512;
const PROBE_SAMPLES: u32 = 64;
const SEED: u32 = 17;
const HOUR: f32 = 12.0;

struct Mesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Mesh {
    fn plane(half: f32, height: f32) -> Self {
        Self {
            positions: vec![
                [-half, height, -half],
                [half, height, -half],
                [-half, height, half],
                [half, height, half],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            indices: vec![0, 2, 1, 1, 2, 3],
        }
    }

    fn sphere(centre: [f32; 3], radius: f32, rings: u32, segments: u32) -> Self {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..=segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                let normal = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                positions.push(std::array::from_fn(|k| centre[k] + normal[k] * radius));
                normals.push(normal);
            }
        }
        let mut indices = Vec::new();
        let row = segments + 1;
        for ring in 0..rings {
            for segment in 0..segments {
                let a = ring * row + segment;
                let b = a + row;
                indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        Self {
            positions,
            normals,
            indices,
        }
    }

    fn triangles(&self, material: u32) -> Vec<Triangle> {
        self.indices
            .chunks_exact(3)
            .map(|corner| Triangle {
                vertices: std::array::from_fn(|k| self.positions[corner[k] as usize]),
                material,
            })
            .collect()
    }
}

struct Light {
    sky: Sky,
    sun: Sun,
}

impl Light {
    fn uniform_sky(radiance: f32) -> Self {
        Self {
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[radiance, radiance, radiance, 1.0]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                colour: [1.0; 3],
                intensity: 0.0,
            },
        }
    }

    fn sun_only(direction: [f32; 3], intensity: f32) -> Self {
        Self {
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0, 0.0, 0.0, 1.0]],
            },
            sun: Sun {
                direction: unit(direction),
                colour: [1.0; 3],
                intensity,
            },
        }
    }

    fn trace_sun(&self) -> TraceSun {
        TraceSun {
            direction: self.sun.direction,
            color: self.sun.colour,
            intensity: self.sun.intensity,
        }
    }
}

struct View {
    eye: [f32; 3],
    target: [f32; 3],
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl View {
    fn axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let forward = unit(std::array::from_fn(|k| self.target[k] - self.eye[k]));
        let right = unit(cross(forward, [0.0, 0.0, -1.0]));
        let up = cross(right, forward);
        (forward, right, up)
    }

    fn camera(&self) -> Camera {
        let (forward, right, up) = self.axes();
        let eye = self.eye;
        let view: Matrix = [
            [right[0], up[0], -forward[0], 0.0],
            [right[1], up[1], -forward[1], 0.0],
            [right[2], up[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
        ];
        let tan = (FOV_Y.to_radians() * 0.5).tan();
        let projection: Matrix = [
            [1.0 / tan, 0.0, 0.0, 0.0],
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

    fn trace_camera(&self) -> TraceCamera {
        let (forward, right, up) = self.axes();
        let tan = (FOV_Y.to_radians() * 0.5).tan();
        TraceCamera {
            origin: self.eye,
            forward,
            right: right.map(|v| v * tan),
            up: up.map(|v| v * tan),
        }
    }
}

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp")
        .join(name)
}

fn bake_probes(
    gpu: &Gpu,
    name: &str,
    triangles: &[Triangle],
    materials: &[Material],
    light: &Light,
    spec: GridSpec,
) -> PathBuf {
    let scene = BakeScene {
        triangles: triangles.to_vec(),
        shapes: Vec::new(),
        materials: materials.to_vec(),
        anchors: vec![Anchor {
            hour: HOUR,
            sky: light.sky.clone(),
            sun: light.trace_sun(),
        }],
    };
    let grids = bake(gpu, &scene, spec, PROBE_SAMPLES, SEED).unwrap();
    let relative = Path::new("energy").join(name);
    let root = scratch("energy").join(name);
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    write_artifact(&scratch(""), &relative, &scene, &grids, PROBE_SAMPLES, SEED).unwrap();
    root
}

fn live(
    frame: &mut Frame,
    mesh: &Mesh,
    material: Material,
    light: &Light,
    view: &View,
    probes: Option<&Path>,
) -> Vec<[f32; 3]> {
    let handle = frame
        .upload_mesh(MeshData {
            positions: &mesh.positions,
            normals: &mesh.normals,
            tangents: &vec![[1.0, 0.0, 0.0, 1.0]; mesh.positions.len()],
            uvs: &vec![[0.0, 0.0]; mesh.positions.len()],
            uvs1: None,
            alpha: None,
            indices: &mesh.indices,
        })
        .unwrap();
    frame.set_sky(SkySource::Hdr(light.sky.clone()), true);
    if let Some(path) = probes {
        let probes =
            ProbeLighting::load(&frame.gpu.device, &frame.gpu.queue, path, HOUR, [-1.0; 3])
                .unwrap();
        frame.set_probes(probes);
    }
    let instances = [Instance::new(handle, identity(), 0, 1)];
    let materials = [material];
    let scene = Scene {
        camera: view.camera(),
        time: 0.0,
        seed: SEED,
        sun: light.sun,
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.render(&scene).unwrap();
    frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32()))
        .collect()
}

fn traced(gpu: &Gpu, mesh: &Mesh, material: Material, light: &Light, view: &View) -> Vec<[f32; 3]> {
    let scene = TraceScene {
        triangles: mesh.triangles(0),
        shapes: Vec::new(),
        materials: vec![material],
        sky: light.sky.clone(),
        camera: view.trace_camera(),
        sun: light.trace_sun(),
    };
    let mut trace = Trace::new(gpu, &scene, SIZE, SIZE).unwrap();
    for _ in 0..TRACE_SAMPLES / 4 {
        trace.sample(gpu, 4, SEED).unwrap();
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

fn mean(image: &[[f32; 3]], keep: impl Fn(u32, u32) -> bool) -> [f64; 3] {
    let mut sum = [0.0f64; 3];
    let mut count = 0.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            if keep(x, y) {
                let pixel = image[(y * SIZE + x) as usize];
                for c in 0..3 {
                    sum[c] += f64::from(pixel[c]);
                }
                count += 1.0;
            }
        }
    }
    assert!(count > 0.0);
    sum.map(|s| s / count)
}

fn centre(x: u32, y: u32) -> bool {
    let quarter = SIZE / 4;
    (quarter..SIZE - quarter).contains(&x) && (quarter..SIZE - quarter).contains(&y)
}

fn assert_close(label: &str, live: [f64; 3], traced: [f64; 3], tolerance: f64) {
    println!("{label}: live {live:?}, traced {traced:?}");
    for c in 0..3 {
        let ratio = live[c] / traced[c];
        assert!(
            (ratio - 1.0).abs() <= tolerance,
            "{label}: channel {c} live {} against traced {} (ratio {ratio:.4})",
            live[c],
            traced[c]
        );
    }
}

fn lambert() -> Material {
    Material {
        base: [0.8, 0.8, 0.8],
        roughness: 1.0,
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        ..Material::default()
    }
}

fn plane_view() -> View {
    View {
        eye: [0.0, 1.5, 0.6],
        target: [0.0, 0.0, 0.0],
    }
}

fn plane_grid() -> GridSpec {
    GridSpec {
        min: [-0.6, 0.0, -0.6],
        max: [0.6, 0.2, 0.6],
        spacing: 0.1,
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_lambert_plane_under_a_uniform_sky_matches_the_tracer_through_probes() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mesh = Mesh::plane(4.0, -0.001);
    let light = Light::uniform_sky(1.0);
    let view = plane_view();
    let probes = bake_probes(
        &gpu,
        "sky-plane",
        &mesh.triangles(0),
        &[lambert()],
        &light,
        plane_grid(),
    );
    let reference = mean(&traced(&gpu, &mesh, lambert(), &light, &view), centre);
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let ours = mean(
        &live(&mut frame, &mesh, lambert(), &light, &view, Some(&probes)),
        centre,
    );
    assert_close("uniform sky through probes", ours, reference, 0.02);
    std::fs::remove_dir_all(probes).unwrap();
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_lambert_plane_under_the_sun_alone_matches_the_tracer() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mesh = Mesh::plane(4.0, -0.001);
    let light = Light::sun_only([0.3, 0.8, 0.4], 3.0);
    let view = plane_view();
    let probes = bake_probes(
        &gpu,
        "sun-plane",
        &mesh.triangles(0),
        &[lambert()],
        &light,
        plane_grid(),
    );
    let reference = mean(&traced(&gpu, &mesh, lambert(), &light, &view), centre);
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let ours = mean(
        &live(&mut frame, &mesh, lambert(), &light, &view, Some(&probes)),
        centre,
    );
    assert_close("sun alone", ours, reference, 0.02);
    std::fs::remove_dir_all(probes).unwrap();
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_mirror_sphere_reflects_a_uniform_sky_as_the_tracer_does() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mesh = Mesh::sphere([0.0, 0.0, 0.0], 0.5, 48, 96);
    let mirror = Material {
        base: [0.95, 0.95, 0.95],
        roughness: 0.0,
        metalness: 1.0,
        ..Material::default()
    };
    let light = Light::uniform_sky(1.0);
    let view = View {
        eye: [0.0, 0.4, 2.2],
        target: [0.0, 0.0, 0.0],
    };
    let traced_image = traced(&gpu, &mesh, mirror, &light, &view);
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let live_image = live(&mut frame, &mesh, mirror, &light, &view, None);
    let inner = |x: u32, y: u32| {
        let dx = x as f32 + 0.5 - SIZE as f32 * 0.5;
        let dy = y as f32 + 0.5 - SIZE as f32 * 0.5;
        (dx * dx + dy * dy).sqrt() < SIZE as f32 * 0.2
    };
    assert_close(
        "mirror sphere",
        mean(&live_image, inner),
        mean(&traced_image, inner),
        0.02,
    );
}
