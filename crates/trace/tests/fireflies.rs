use pfx_geom::mesh::Mesh as GeomMesh;
use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::Lens;
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Output, Projection, Sun};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;
const FOV_Y: f32 = 40.0;
const EYE: [f32; 3] = [0.0, 1.3, 4.6];
const TARGET: [f32; 3] = [0.0, 0.6, 0.0];
const SUN: [f32; 3] = [0.35, 0.8, 0.55];
const SUN_RADIUS_DEG: f32 = 3.0;
const SKY: f32 = 0.03;
const SEED: u32 = 81;
const OUTLIER_MULTIPLE: f32 = 4.0;
const OUTLIER_FLOOR: f32 = 0.01;
const CLAMP_INDIRECT: f32 = 4.0;
const FILTER_GLOSSY: f32 = 0.25;
const OFF_BYTES: [u64; 3] = [
    0x674f_a934_1fdd_cf91,
    0xa381_2026_00e7_9695,
    0x8dfd_8091_2445_7c6f,
];

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|value| value * value).sum::<f32>().sqrt();
    v.map(|value| value / length)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn translated(at: [f32; 3]) -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [at[0], at[1], at[2], 1.0],
    ]
}

fn box_mesh(half: [f32; 3]) -> GeomMesh {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for (normal, u, v) in faces {
        let first = positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push(std::array::from_fn(|k| {
                (normal[k] + u[k] * a + v[k] * b) * half[k]
            }));
            normals.push(normal);
            uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
        }
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    GeomMesh::new(positions, normals, Vec::new(), uvs, indices)
}

fn sphere_mesh(radius: f32, rings: u32, segments: u32) -> GeomMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for ring in 0..=rings {
        let theta = std::f32::consts::PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
            let normal = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            positions.push(normal.map(|value| value * radius));
            normals.push(normal);
            uvs.push([segment as f32 / segments as f32, ring as f32 / rings as f32]);
        }
    }
    let row = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * row + segment;
            let b = a + row;
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    GeomMesh::new(positions, normals, Vec::new(), uvs, indices)
}

fn staged(mesh: &GeomMesh) -> Mesh<'_> {
    Mesh {
        positions: &mesh.positions,
        normals: &mesh.normals,
        tangents: &mesh.tangents,
        uvs: &mesh.uvs,
        alpha: None,
        indices: &mesh.indices,
    }
}

fn materials() -> Vec<Material> {
    vec![
        Material {
            base: [0.6; 3],
            roughness: 0.9,
            ..Material::default()
        },
        Material {
            base: [0.7, 0.68, 0.65],
            roughness: 0.9,
            ..Material::default()
        },
        Material {
            base: [0.92, 0.92, 0.94],
            roughness: 0.02,
            metalness: 1.0,
            ..Material::default()
        },
        Material {
            base: [0.55, 0.08, 0.06],
            roughness: 0.6,
            clearcoat: 1.0,
            ..Material::default()
        },
        Material {
            base: [1.0; 3],
            roughness: 0.01,
            transmission: 1.0,
            ior: 1.5,
            ..Material::default()
        },
    ]
}

fn camera() -> Camera {
    let tan = (FOV_Y.to_radians() * 0.5).tan();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let forward = normalize(std::array::from_fn(|k| TARGET[k] - EYE[k]));
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    Camera {
        origin: EYE,
        forward,
        right: right.map(|value| value * tan * aspect),
        up: up.map(|value| value * tan),
    }
}

#[derive(Clone, Copy, Debug)]
struct Setting {
    name: &'static str,
    clamp_indirect: f32,
    filter_glossy: f32,
}

const OFF: Setting = Setting {
    name: "off",
    clamp_indirect: 0.0,
    filter_glossy: 0.0,
};
const CLAMP: Setting = Setting {
    name: "clamp_indirect",
    clamp_indirect: CLAMP_INDIRECT,
    filter_glossy: 0.0,
};
const FILTER: Setting = Setting {
    name: "filter_glossy",
    clamp_indirect: 0.0,
    filter_glossy: FILTER_GLOSSY,
};
const BOTH: Setting = Setting {
    name: "both",
    clamp_indirect: CLAMP_INDIRECT,
    filter_glossy: FILTER_GLOSSY,
};

fn traced(gpu: &Gpu, setting: Setting, samples: u32, turns: &mut Turns) -> Output {
    let floor = box_mesh([5.0, 0.05, 5.0]);
    let wall = box_mesh([5.0, 3.0, 0.05]);
    let sphere = sphere_mesh(0.45, 32, 64);
    let panel = box_mesh([0.4, 0.55, 0.04]);
    let block = box_mesh([0.32, 0.32, 0.32]);
    let meshes = [
        staged(&floor),
        staged(&wall),
        staged(&sphere),
        staged(&panel),
        staged(&block),
    ];
    let placements = [
        Placement::new(0, translated([0.0, -0.05, 0.0]), 0),
        Placement::new(1, translated([0.0, 3.0, -1.6]), 1),
        Placement::new(2, translated([-1.15, 0.45, 0.1]), 2),
        Placement::new(3, translated([0.0, 0.55, -0.6]), 3),
        Placement::new(4, translated([1.15, 0.32, 0.1]), 4),
    ];
    let materials = materials();
    let mut staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[SKY, SKY, SKY, 1.0]],
        },
        sun: Sun {
            direction: normalize(SUN),
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: camera(),
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    staged.detail.sun_radius_deg = SUN_RADIUS_DEG;
    staged.detail.clamp_indirect = setting.clamp_indirect;
    staged.detail.filter_glossy = setting.filter_glossy;
    let mut trace = staged.trace(gpu, WIDTH, HEIGHT).unwrap();
    let mut pacer = Pacer::default();
    let stats = trace
        .sample_paced(gpu, samples, SEED, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    assert!(
        stats.longest_ms() < 100.0,
        "longest submission {} ms",
        stats.longest_ms()
    );
    trace.readback(gpu).unwrap()
}

fn luminance(output: &Output) -> Vec<f32> {
    output
        .color
        .chunks_exact(16)
        .map(|texel| {
            let c: [f32; 3] = std::array::from_fn(|k| {
                f32::from_le_bytes(texel[k * 4..k * 4 + 4].try_into().unwrap())
            });
            0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
        })
        .collect()
}

fn outliers(output: &Output) -> usize {
    let values = luminance(output);
    let (width, height) = (WIDTH as i32, HEIGHT as i32);
    let mut count = 0;
    for y in 0..height {
        for x in 0..width {
            let mut around: Vec<f32> = (-2..=2)
                .flat_map(|dy| (-2..=2).map(move |dx| (x + dx, y + dy)))
                .filter(|&(nx, ny)| {
                    (nx, ny) != (x, y) && nx >= 0 && ny >= 0 && nx < width && ny < height
                })
                .map(|(nx, ny)| values[(ny * width + nx) as usize])
                .collect();
            around.sort_by(f32::total_cmp);
            let median = around[around.len() / 2];
            if values[(y * width + x) as usize] > OUTLIER_MULTIPLE * median.max(OUTLIER_FLOOR) {
                count += 1;
            }
        }
    }
    count
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn both_settings_off_keep_the_firefly_scene_bytes() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let off = traced(&gpu, OFF, 16, &mut turns);
    let hashes = [fnv(&off.color), fnv(&off.albedo), fnv(&off.normal)];
    println!("off: {hashes:x?} outliers {}", outliers(&off));
    assert_eq!(hashes, OFF_BYTES);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_settings_cut_the_fireflies_of_sharp_lobes_after_a_diffuse_bounce() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let mut counts = Vec::new();
    let mut glints = Vec::new();
    for samples in [64, 1024] {
        for setting in [OFF, CLAMP, FILTER, BOTH] {
            let output = traced(&gpu, setting, samples, &mut turns);
            let count = outliers(&output);
            let values = luminance(&output);
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            let glint = values.iter().copied().fold(0.0, f32::max);
            println!(
                "{samples} spp, {}: {count} outliers, mean {mean:.4}, glint {glint:.2}",
                setting.name
            );
            counts.push(count);
            glints.push(glint);
        }
    }
    let [off_64, .., off, clamp, filter, both] = counts[..] else {
        unreachable!()
    };
    assert!(off > off_64, "{counts:?}");
    for count in [clamp, filter, both] {
        assert!(3 * count <= off, "{counts:?}");
    }
    assert!(both <= clamp.min(filter), "{counts:?}");
    let off_glint = glints[4];
    for glint in &glints[5..] {
        assert!((glint / off_glint - 1.0).abs() < 0.02, "{glints:?}");
    }
}
