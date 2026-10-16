use std::time::Instant;

use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun, Trace};

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
}

fn lumpy(triangles: usize) -> Arrays {
    let rings = ((triangles as f64 / 4.0).sqrt() as u32).max(4);
    let segments = rings * 2;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for ring in 0..=rings {
        let theta = std::f32::consts::PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
            let n = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            let r = 1.0 + 0.08 * (7.0 * phi).sin() * (5.0 * theta).sin();
            positions.push(n.map(|v| v * r));
            normals.push(n);
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
    Arrays {
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; positions.len()],
        positions,
        normals,
        uvs,
        indices,
    }
}

fn turned(angle: f32) -> Transform {
    let (s, c) = angle.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 1.1, 0.0, 1.0],
    ]
}

fn stage(mesh: &Arrays, floor: &Arrays, angle: f32, size: [u32; 2], gpu: &Gpu) -> Trace {
    let meshes = [mesh.mesh(), floor.mesh()];
    let placements = [
        Placement::new(0, turned(angle), 0),
        Placement::new(
            1,
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            1,
        ),
    ];
    let materials = [
        Material {
            base: [0.7, 0.3, 0.2],
            roughness: 0.6,
            ..Material::default()
        },
        Material {
            base: [0.5, 0.5, 0.5],
            roughness: 1.0,
            ..Material::default()
        },
    ];
    let aspect = size[0] as f32 / size[1] as f32;
    Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.3, 0.35, 0.4, 1.0]],
        },
        sun: Sun {
            direction: [0.36, 0.86, 0.36],
            color: [1.0, 0.95, 0.9],
            intensity: 3.0,
        },
        camera: Camera {
            origin: [0.0, 1.4, 4.0],
            forward: [0.0, -0.075, -1.0],
            right: [0.6 * aspect, 0.0, 0.0],
            up: [0.0, 0.6, 0.0],
        },
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap()
    .trace(gpu, size[0], size[1])
    .unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let triangles: usize = args.next().map_or(4_000_000, |a| a.parse().unwrap());
    let frames: u32 = args.next().map_or(6, |a| a.parse().unwrap());
    let size = [2160, 3840];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mesh = lumpy(triangles);
    let floor = Arrays {
        positions: vec![
            [-6.0, 0.0, -6.0],
            [6.0, 0.0, -6.0],
            [6.0, 0.0, 6.0],
            [-6.0, 0.0, 6.0],
        ],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0; 2]; 4],
        indices: vec![0, 2, 1, 0, 3, 2],
    };
    let count = mesh.indices.len() / 3;
    let angle = |frame: u32| frame as f32 * 0.05;
    let mut kept: Option<Trace> = None;
    let mut restaged = Vec::new();
    let mut posed = Vec::new();
    let mut sampled = [Vec::new(), Vec::new()];
    for frame in 0..frames {
        let start = Instant::now();
        let mut trace = stage(&mesh, &floor, angle(frame), size, &gpu);
        restaged.push(start.elapsed().as_secs_f64() * 1e3);
        let start = Instant::now();
        trace.sample(&gpu, 1, frame).unwrap();
        sampled[0].push(start.elapsed().as_secs_f64() * 1e3);
        drop(trace);
        let trace = kept.get_or_insert_with(|| stage(&mesh, &floor, angle(0), size, &gpu));
        let start = Instant::now();
        trace
            .set_instance_transforms(&gpu, &[turned(angle(frame))])
            .unwrap();
        gpu.device
            .poll(pfx_gpu::wgpu::PollType::wait_indefinitely())
            .unwrap();
        posed.push(start.elapsed().as_secs_f64() * 1e3);
        let start = Instant::now();
        trace.sample(&gpu, 1, frame).unwrap();
        sampled[1].push(start.elapsed().as_secs_f64() * 1e3);
    }
    let median = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() / 2]
    };
    let trace = kept.unwrap();
    println!(
        "{count} triangles, {frames} frames at {}x{}, hardware rays {}",
        size[0],
        size[1],
        trace.hardware_rays()
    );
    println!(
        "restage per frame: {:.1} ms median (frames {:?}), then one sample {:.1} ms",
        median(&restaged),
        restaged.iter().map(|v| v.round()).collect::<Vec<_>>(),
        median(&sampled[0])
    );
    println!(
        "moved instance: {:.1} ms median (frames {:?}), then one sample {:.1} ms, builds {:?}",
        median(&posed[1..]),
        posed.iter().map(|v| v.round()).collect::<Vec<_>>(),
        median(&sampled[1]),
        trace.builds()
    );
}
