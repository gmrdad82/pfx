use std::time::Instant;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, TimingSource, Turns};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun};

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

    fn ground(half: f32) -> Self {
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

    fn lumpy(triangles: usize, seed: u32) -> Self {
        let rings = ((triangles as f64 / 4.0).sqrt() as u32).max(4);
        let segments = rings * 2;
        let (a, b) = (3.0 + (seed % 5) as f32, 2.0 + (seed % 3) as f32);
        let mut out = Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        };
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..=segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                let n = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let r = 0.5 * (1.0 + 0.15 * (a * phi).sin() * (b * theta).sin());
                out.positions.push([n[0] * r, n[1] * r + 0.5, n[2] * r]);
                out.normals.push(n);
                out.tangents.push([1.0, 0.0, 0.0, 1.0]);
                out.uvs
                    .push([segment as f32 / segments as f32, ring as f32 / rings as f32]);
            }
        }
        let row = segments + 1;
        for ring in 0..rings {
            for segment in 0..segments {
                let a = ring * row + segment;
                let b = a + row;
                out.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        out
    }
}

fn hash(value: u32) -> f32 {
    let mut x = value.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
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
    (0..8)
        .map(|k| Material {
            base: [
                0.2 + 0.7 * hash(k * 3),
                0.2 + 0.7 * hash(k * 3 + 1),
                0.2 + 0.7 * hash(k * 3 + 2),
            ],
            roughness: 0.3 + 0.6 * hash(k + 40),
            specular: 0.4,
            metalness: if k % 4 == 3 { 1.0 } else { 0.0 },
            ..Material::default()
        })
        .collect()
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let number = |name: &str, default: usize| {
        words
            .iter()
            .position(|word| word == name)
            .and_then(|at| words.get(at + 1))
            .map_or(default, |value| value.parse().expect("a whole number"))
    };
    let size = [
        number("--width", 2160) as u32,
        number("--height", 3840) as u32,
    ];
    let samples = number("--samples", 4) as u32;
    let instances = number("--instances", 20_000).max(1);
    let kinds = number("--meshes", 32).clamp(1, instances);
    let triangles = number("--triangles", 4_000_000);
    let budget = std::time::Duration::from_secs(number("--seconds", 600) as u64);
    let gpu = pollster::block_on(Gpu::headless()).expect("a GPU");
    let features = gpu.device.features();
    let limits = gpu.device.limits();
    println!(
        "adapter {} ({:?}, {} {}), ray queries offered {}, timestamps {} inside encoders {} inside passes {}",
        gpu.info.name,
        gpu.info.backend,
        gpu.info.driver,
        gpu.info.driver_info,
        features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY),
        features.contains(wgpu::Features::TIMESTAMP_QUERY),
        features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
        features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
    );
    println!(
        "limits: buffer {} storage binding {} storage buffers per stage {} workgroup invocations {} workgroup storage {}",
        limits.max_buffer_size,
        limits.max_storage_buffer_binding_size,
        limits.max_storage_buffers_per_shader_stage,
        limits.max_compute_invocations_per_workgroup,
        limits.max_compute_workgroup_storage_size,
    );
    let started = Instant::now();
    let mut arrays = vec![Arrays::ground(400.0)];
    arrays.extend((0..kinds).map(|k| Arrays::lumpy(triangles / kinds, k as u32)));
    let meshes: Vec<Mesh<'_>> = arrays.iter().map(Arrays::mesh).collect();
    let side = (instances as f64).sqrt().ceil() as usize;
    let spacing = 1.6;
    let mut placements = vec![Placement::new(0, placed([0.0; 3], 0.0, 1.0), 0)];
    placements.extend((0..instances).map(|k| {
        let (row, column) = (k / side, k % side);
        let at = [
            (column as f32 - side as f32 * 0.5) * spacing + 0.4 * hash(k as u32),
            0.0,
            -(row as f32) * spacing - 2.0 + 0.4 * hash(k as u32 + 7),
        ];
        let mesh = 1 + (k % kinds) as u32;
        Placement::new(
            mesh,
            placed(
                at,
                6.0 * hash(k as u32 + 11),
                0.6 + 0.8 * hash(k as u32 + 13),
            ),
            1 + (k % 7) as u32,
        )
    }));
    let tan = 0.45_f32;
    let aspect = size[0] as f32 / size[1] as f32;
    let forward = {
        let f = [0.0_f32, -0.35, -1.0];
        let l = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
        f.map(|v| v / l)
    };
    let up = [0.0, -forward[2] * tan, forward[1] * tan];
    let staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials(),
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.35, 0.4, 0.5, 1.0]],
        },
        sun: Sun {
            direction: [0.3, 0.8, 0.5],
            color: [1.0, 0.95, 0.9],
            intensity: 3.0,
        },
        camera: Camera {
            origin: [0.0, 6.0, 4.0],
            forward,
            right: [tan * aspect, 0.0, 0.0],
            up,
        },
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .expect("a staged scene");
    let staged_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut trace = staged.trace(&gpu, size[0], size[1]).expect("a trace");
    let built_ms = started.elapsed().as_secs_f64() * 1e3 - staged_ms;
    println!(
        "scene: {} instances of {kinds} meshes, {} triangles, hardware rays {}, {} MB on the GPU, staged {staged_ms:.0} ms, built {built_ms:.0} ms",
        trace.instance_count(),
        triangles,
        trace.hardware_rays(),
        trace.gpu_bytes() / 1_000_000,
    );
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let mut walls = Vec::new();
    for sample in 0..samples {
        if sample > 0 && started.elapsed() > budget {
            println!(
                "stopped after {sample} samples: past the {} s budget",
                budget.as_secs()
            );
            break;
        }
        let began = Instant::now();
        let stats = trace
            .sample_paced(&gpu, 1, 7, &mut pacer, |ms| turns.add(ms))
            .expect("a sample");
        let wall = began.elapsed().as_secs_f64() * 1e3;
        let work: f64 = stats.milliseconds.iter().sum();
        walls.push(wall);
        println!(
            "sample {sample}: {wall:.0} ms wall, {} submissions, {work:.0} ms timed ({}), median {:.2} ms, longest {:.2} ms, {:.3} ms wall per submission",
            stats.count(),
            match stats.source() {
                TimingSource::Gpu => "GPU",
                TimingSource::Cpu => "fence",
                TimingSource::Mixed => "mixed",
                TimingSource::Unmeasured => "untimed",
            },
            stats.median_ms(),
            stats.longest_ms(),
            wall / stats.count().max(1) as f64,
        );
    }
    let began = Instant::now();
    let _ = trace.readback_color(&gpu).expect("a readback");
    println!(
        "{}x{}: {:.0} ms per sample (median of {}), readback {:.0} ms",
        size[0],
        size[1],
        median(&mut walls),
        samples,
        began.elapsed().as_secs_f64() * 1e3,
    );
}
