use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::motion::Motion;
use pfx_trace::sobol::Sampler;
use pfx_trace::stage::{Mesh, Placement, PlacementMotion, Stage};
use pfx_trace::{Camera, Projection, Sun, Trace};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;
const TAN: f32 = 0.5;
const EYE: [f32; 3] = [0.0, 0.0, 4.0];
const WALL: f32 = -2.0;
const PANES: usize = 10;
const PANE: f32 = 0.3;
const SLANT: f32 = 0.75;
const SAMPLES: u32 = 16;
const SEED: u32 = 3;

struct Arrays {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Arrays {
    fn square(half: f32) -> Self {
        Self {
            positions: vec![
                [-half, -half, 0.0],
                [half, -half, 0.0],
                [half, half, 0.0],
                [-half, half, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

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
}

fn placed(at: [f32; 3]) -> Transform {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

fn camera() -> Camera {
    let aspect = WIDTH as f32 / HEIGHT as f32;
    Camera {
        origin: EYE,
        forward: [0.0, 0.0, -1.0],
        right: [TAN * aspect, 0.0, 0.0],
        up: [0.0, TAN, 0.0],
    }
}

fn pixel(p: [f32; 3]) -> (usize, usize) {
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let depth = EYE[2] - p[2];
    let x = (p[0] - EYE[0]) / (depth * TAN * aspect);
    let y = -(p[1] - EYE[1]) / (depth * TAN);
    (
        ((x + 1.0) * 0.5 * WIDTH as f32) as usize,
        ((y + 1.0) * 0.5 * HEIGHT as f32) as usize,
    )
}

fn pane_at(k: usize) -> [f32; 3] {
    let z = -0.1 * k as f32;
    [SLANT * z, 0.0, z]
}

fn lambert(base: [f32; 3]) -> Material {
    Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    }
}

fn glass() -> Material {
    Material {
        base: [0.92, 0.9, 0.88],
        roughness: 0.05,
        transmission: 0.95,
        ior: 1.5,
        ..Material::default()
    }
}

static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn staged(gpu: &Gpu, glassy: bool, hardware: bool) -> Trace {
    let wall = Arrays::square(3.0);
    let pane = Arrays::square(PANE);
    let meshes = [wall.mesh(), pane.mesh()];
    let paths: Vec<Vec<Transform>> = (0..PANES)
        .rev()
        .map(|k| {
            let at = pane_at(k);
            vec![
                placed([at[0], at[1] - 0.01, at[2]]),
                placed([at[0], at[1] + 0.01, at[2]]),
            ]
        })
        .collect();
    let mut placements = vec![Placement::new(0, placed([0.0, 0.0, WALL]), 0)];
    for (j, path) in paths.iter().enumerate() {
        let material = if j + 1 == PANES { 2 } else { 1 };
        placements.push(Placement {
            motion: Some(PlacementMotion {
                transforms: path,
                meshes: &[],
            }),
            ..Placement::new(1, path[0], material)
        });
    }
    let materials = if glassy {
        vec![lambert([0.7; 3]), glass(), glass()]
    } else {
        vec![
            lambert([0.7; 3]),
            lambert([0.8; 3]),
            lambert([0.9, 0.1, 0.1]),
        ]
    };
    let mut stage = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.05, 0.05, 0.06, 1.0]],
        },
        sun: Sun {
            direction: normalize([SLANT * 0.8, 0.0, 0.8]),
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: camera(),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    stage.detail.sampler = Sampler::Pcg;
    stage.detail.transmissive_shadows = glassy;
    stage.detail.motion = Some(Motion {
        steps: 2,
        cameras: vec![camera(); 2],
        curve: Vec::new(),
    });
    let _env = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = std::env::var_os("PFX_TRACE_RT");
    unsafe { std::env::set_var("PFX_TRACE_RT", if hardware { "1" } else { "0" }) };
    let trace = stage.trace(gpu, WIDTH, HEIGHT).unwrap();
    match before {
        Some(value) => unsafe { std::env::set_var("PFX_TRACE_RT", value) },
        None => unsafe { std::env::remove_var("PFX_TRACE_RT") },
    }
    trace
}

fn render(gpu: &Gpu, glassy: bool, hardware: bool) -> Option<Vec<[f32; 3]>> {
    let mut trace = staged(gpu, glassy, hardware);
    if hardware && !trace.hardware_rays() {
        println!("no ray queries on this adapter; the hardware path is skipped");
        return None;
    }
    trace.sample(gpu, SAMPLES, SEED).unwrap();
    let bytes = trace.readback(gpu).unwrap().color;
    Some(
        bytes
            .chunks_exact(16)
            .map(|texel| {
                std::array::from_fn(|k| {
                    f32::from_le_bytes([
                        texel[4 * k],
                        texel[4 * k + 1],
                        texel[4 * k + 2],
                        texel[4 * k + 3],
                    ])
                })
            })
            .collect(),
    )
}

fn around(image: &[[f32; 3]], world: [f32; 3]) -> [f32; 3] {
    let (cx, cy) = pixel(world);
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in cy - 1..=cy + 1 {
        for x in cx - 1..=cx + 1 {
            let texel = image[y * WIDTH as usize + x];
            for k in 0..3 {
                sum[k] += texel[k];
            }
            count += 1.0;
        }
    }
    sum.map(|v| v / count)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

struct Seen {
    front: [f32; 3],
    shadow: f32,
    lit: f32,
    mean: f32,
}

fn seen(image: &[[f32; 3]]) -> Seen {
    let umbra = [SLANT * WALL, 0.0, WALL];
    Seen {
        front: around(image, pane_at(0)),
        shadow: luma(around(image, umbra)),
        lit: luma(around(image, [-umbra[0], 0.0, WALL])),
        mean: image.iter().map(|c| luma(*c)).sum::<f32>() / image.len() as f32,
    }
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance * a.abs().max(b.abs()).max(1e-3)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_ray_through_more_movers_than_one_batch_finds_the_nearest_and_its_shadow() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let software = seen(&render(&gpu, false, false).unwrap());
    let front = software.front;
    assert!(
        front[0] > 3.0 * front[1],
        "software: the nearest pane is not the one in front: {front:?}"
    );
    assert!(
        software.shadow < 0.25 * software.lit,
        "software: the stack casts no shadow: {} against {}",
        software.shadow,
        software.lit
    );
    let Some(image) = render(&gpu, false, true) else {
        return;
    };
    let hardware = seen(&image);
    println!(
        "front {:?} / {:?}, shadow {} / {}, lit {} / {}, mean {} / {}",
        software.front,
        hardware.front,
        software.shadow,
        hardware.shadow,
        software.lit,
        hardware.lit,
        software.mean,
        hardware.mean
    );
    assert!(
        hardware.front[0] > 3.0 * hardware.front[1],
        "hardware: the nearest pane is not the one in front: {:?}",
        hardware.front
    );
    assert!(
        hardware.shadow < 0.25 * hardware.lit,
        "hardware: the stack casts no shadow"
    );
    assert!(close(software.front[0], hardware.front[0], 0.05));
    assert!(close(software.shadow, hardware.shadow, 0.1));
    assert!(close(software.lit, hardware.lit, 0.05));
    assert!(close(software.mean, hardware.mean, 0.02));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_shadow_through_more_glass_movers_than_one_batch_passes_their_tints() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let software = seen(&render(&gpu, true, false).unwrap());
    let ratio = software.shadow / software.lit;
    assert!(
        ratio > 0.05 && ratio < 0.9,
        "software: the glass stack passes {ratio} of the light"
    );
    let Some(image) = render(&gpu, true, true) else {
        return;
    };
    let hardware = seen(&image);
    let passed = hardware.shadow / hardware.lit;
    println!("glass stack passes {ratio} in software, {passed} in hardware");
    assert!(close(ratio, passed, 0.1), "{ratio} against {passed}");
    assert!(close(software.mean, hardware.mean, 0.02));
}
