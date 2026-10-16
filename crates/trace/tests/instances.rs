use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::gpu::Builds;
use pfx_trace::sobol::Sampler;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun, Trace};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;
const IDENTITY: Transform = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

struct Arrays {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Arrays {
    fn mesh(&self) -> Mesh<'_> {
        Mesh {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            uvs1: None,
            alpha: None,
            indices: &self.indices,
        }
    }

    fn quad(half: f32) -> Self {
        Self {
            positions: vec![
                [-half, 0.0, -half],
                [half, 0.0, -half],
                [half, 0.0, half],
                [-half, 0.0, half],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 2, 1, 0, 3, 2],
        }
    }

    fn sphere(radius: f32) -> Self {
        let (rings, segments) = (16, 32);
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..=segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                normals.push([
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ]);
                uvs.push([segment as f32 / segments as f32, ring as f32 / rings as f32]);
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
            positions: normals.iter().map(|n| n.map(|v| v * radius)).collect(),
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; normals.len()],
            normals,
            uvs,
            indices,
        }
    }
}

fn placed(at: [f32; 3], turn: f32, scale: f32) -> Transform {
    let (s, c) = turn.sin_cos();
    [
        [c * scale, 0.0, -s * scale, 0.0],
        [0.0, scale, 0.0, 0.0],
        [s * scale, 0.0, c * scale, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn materials() -> Vec<Material> {
    let lambert = |base| Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    vec![
        lambert([0.6, 0.6, 0.6]),
        lambert([0.8, 0.2, 0.1]),
        Material {
            emission: [6.0, 5.0, 4.0],
            ..lambert([0.0; 3])
        },
    ]
}

fn camera(x: f32) -> Camera {
    let tan = 0.5_f32;
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let origin = [x, 2.0, 4.0];
    let target = [0.0, 0.4, 0.0];
    let f = normalize(std::array::from_fn(|k| target[k] - origin[k]));
    let r = normalize([-f[2], 0.0, f[0]]);
    let u = [
        r[1] * f[2] - r[2] * f[1],
        r[2] * f[0] - r[0] * f[2],
        r[0] * f[1] - r[1] * f[0],
    ];
    Camera {
        origin,
        forward: f,
        right: r.map(|v| v * tan * aspect),
        up: u.map(|v| v * tan),
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

fn pose(moved: bool) -> Vec<Transform> {
    if moved {
        vec![
            IDENTITY,
            placed([0.5, 0.45, -0.2], 0.9, 0.9),
            placed([-0.9, 0.35, 0.6], -0.4, 0.7),
            placed([-0.3, 1.6, -0.5], 0.6, 1.0),
        ]
    } else {
        vec![
            IDENTITY,
            placed([-0.4, 0.5, 0.0], 0.0, 1.0),
            placed([0.8, 0.5, 0.4], 0.3, 1.0),
            placed([0.0, 1.8, -0.8], 0.0, 1.0),
        ]
    }
}

static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn staged(transforms: &[Transform], hardware: bool, gpu: &Gpu) -> Trace {
    let floor = Arrays::quad(3.0);
    let ball = Arrays::sphere(0.5);
    let panel = Arrays::quad(0.4);
    let meshes = [floor.mesh(), ball.mesh(), panel.mesh()];
    let placements = [
        Placement::new(0, transforms[0], 0),
        Placement::new(1, transforms[1], 1),
        Placement::new(1, transforms[2], 0),
        Placement::new(2, transforms[3], 2),
    ];
    let materials = materials();
    let mut staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.08, 0.09, 0.1, 1.0]],
        },
        sun: Sun {
            direction: normalize([0.4, 1.0, 0.3]),
            color: [1.0, 0.95, 0.9],
            intensity: 2.0,
        },
        camera: camera(0.0),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    staged.detail.sampler = Sampler::Pcg;
    let _env = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = std::env::var_os("PFX_TRACE_RT");
    unsafe { std::env::set_var("PFX_TRACE_RT", if hardware { "1" } else { "0" }) };
    let trace = staged.trace(gpu, WIDTH, HEIGHT).unwrap();
    match before {
        Some(value) => unsafe { std::env::set_var("PFX_TRACE_RT", value) },
        None => unsafe { std::env::remove_var("PFX_TRACE_RT") },
    }
    trace
}

fn traced(trace: &mut Trace, gpu: &Gpu) -> Vec<u8> {
    trace.sample(gpu, 8, 11).unwrap();
    trace.readback(gpu).unwrap().color
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn moved_instances_trace_as_a_fresh_stage_on_both_paths() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    for hardware in [false, true] {
        let mut kept = staged(&pose(false), hardware, &gpu);
        if hardware && !kept.hardware_rays() {
            println!("no ray queries on this adapter; the hardware path is skipped");
            continue;
        }
        let before = traced(&mut kept, &gpu);
        kept.set_instance_transforms(&gpu, &pose(true)).unwrap();
        let moved = traced(&mut kept, &gpu);
        let mut fresh = staged(&pose(true), hardware, &gpu);
        let expected = traced(&mut fresh, &gpu);
        assert_ne!(
            before, moved,
            "hardware {hardware}: the move changed nothing"
        );
        let (a, b) = (floats(&moved), floats(&expected));
        let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        let largest = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0_f32, f32::max);
        println!(
            "hardware {hardware}: {differing} of {} lanes differ, largest {largest}",
            a.len()
        );
        assert_eq!(moved, expected, "hardware {hardware}");
        assert_eq!(kept.builds().bottom, fresh.builds().bottom);
        assert_eq!(kept.builds().top, fresh.builds().top + 1);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_camera_move_keeps_the_trace_and_builds_nothing() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged(&pose(false), true, &gpu);
    let built = trace.builds();
    assert_eq!(built, Builds { bottom: 3, top: 1 });
    let first = traced(&mut trace, &gpu);
    trace.set_camera(camera(1.5)).unwrap();
    let turned = traced(&mut trace, &gpu);
    assert_ne!(first, turned);
    assert_eq!(trace.builds(), built);
    trace.set_instance_transforms(&gpu, &pose(false)).unwrap();
    assert_eq!(trace.builds(), built);
    trace
        .set_instance_transforms(&gpu, &pose(true)[..2])
        .unwrap();
    assert_eq!(trace.builds(), Builds { bottom: 3, top: 2 });
    assert!(trace.set_instance_transforms(&gpu, &[IDENTITY; 5]).is_err());
}
