use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_materials::{AlphaMode, Material, fresnel_dielectric, thin_dielectric};
use pfx_trace::detail::{Lens, Transform};
use pfx_trace::stage::{Mesh, Placement, Stage};
use pfx_trace::{Camera, Projection, Sun};

const SIZE: u32 = 48;

struct Arrays {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tangents: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Arrays {
    fn cuboid(half: [f32; 3]) -> Self {
        let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
        ];
        let mut arrays = Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
        };
        for (normal, u, v) in faces {
            let first = arrays.positions.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                arrays.positions.push(std::array::from_fn(|k| {
                    (normal[k] + u[k] * a + v[k] * b) * half[k]
                }));
                arrays.normals.push(normal);
                arrays.tangents.push([u[0], u[1], u[2], 1.0]);
                arrays.uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
            }
            arrays
                .indices
                .extend([first, first + 1, first + 2, first, first + 2, first + 3]);
        }
        arrays
    }

    fn quad(half: f32, z: f32) -> Self {
        Self {
            positions: vec![
                [-half, -half, z],
                [half, -half, z],
                [half, half, z],
                [-half, half, z],
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
            alpha: None,
            indices: &self.indices,
            uvs1: None,
        }
    }
}

fn identity() -> Transform {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn emitter(level: f32) -> Material {
    Material {
        base: [0.0; 3],
        roughness: 1.0,
        specular: 0.0,
        emission: [level; 3],
        ..Material::default()
    }
}

fn clear_glass(thin_walled: bool) -> Material {
    Material {
        base: [1.0; 3],
        roughness: 0.0,
        specular: 0.0,
        transmission: 1.0,
        ior: 1.5,
        thin_walled,
        ..Material::default()
    }
}

fn trace(
    gpu: &Gpu,
    meshes: &[Mesh<'_>],
    placements: &[Placement<'_>],
    materials: &[Material],
    samples: u32,
    turns: &mut Turns,
) -> Vec<[f32; 3]> {
    let tan = (8.0_f32.to_radians() * 0.5).tan();
    let staged = Stage {
        meshes,
        instances: placements,
        materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
        camera: Camera {
            origin: [0.0, 0.0, 4.0],
            forward: [0.0, 0.0, -1.0],
            right: [tan, 0.0, 0.0],
            up: [0.0, tan, 0.0],
        },
        projection: Projection::Perspective,
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut traced = staged.trace(gpu, SIZE, SIZE).unwrap();
    let mut pacer = Pacer::default();
    let stats = traced
        .sample_paced(gpu, samples, 11, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    assert!(stats.longest_ms() < 300.0);
    traced
        .readback(gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|pixel| {
            std::array::from_fn(|channel| {
                f32::from_le_bytes(pixel[channel * 4..channel * 4 + 4].try_into().unwrap())
            })
        })
        .collect()
}

fn centre(pixels: &[[f32; 3]]) -> f32 {
    let (lo, hi) = (SIZE / 4, SIZE - SIZE / 4);
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in lo..hi {
        for x in lo..hi {
            sum += pixels[(y * SIZE + x) as usize].iter().sum::<f32>() / 3.0;
            count += 1.0;
        }
    }
    sum / count
}

fn passed_through_two_faces(face: f32) -> f32 {
    let transmitted = 1.0 - face;
    transmitted * transmitted / (1.0 - face * face)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_thin_walled_box_passes_two_slab_faces_where_a_solid_passes_two_interfaces() {
    let shell = Arrays::cuboid([0.4, 0.4, 0.2]);
    let wall = Arrays::quad(3.0, -1.0);
    let meshes = [shell.mesh(), wall.mesh()];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let face = fresnel_dielectric(1.0, 1.5);
    let slab = thin_dielectric(face)[0];
    for (thin_walled, expected) in [
        (false, passed_through_two_faces(face)),
        (true, passed_through_two_faces(slab)),
    ] {
        let materials = [clear_glass(thin_walled), emitter(1.0)];
        let placements = [
            Placement::new(0, identity(), 0),
            Placement::new(1, identity(), 1),
        ];
        let seen = centre(&trace(
            &gpu,
            &meshes,
            &placements,
            &materials,
            64,
            &mut turns,
        ));
        println!("thin walled {thin_walled}: {seen:.4} against {expected:.4}");
        assert!(
            (seen - expected).abs() < 0.01,
            "thin walled {thin_walled}: {seen} against {expected}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_masked_surface_is_cut_below_its_cutoff_and_a_blended_one_lets_one_minus_alpha_through() {
    let card = Arrays::quad(0.6, 0.5);
    let wall = Arrays::quad(3.0, -1.0);
    let meshes = [card.mesh(), wall.mesh()];
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut turns = Turns::default();
    let black = |alpha_mode: AlphaMode, alpha: f32| Material {
        base: [0.0; 3],
        roughness: 1.0,
        specular: 0.0,
        alpha,
        alpha_mode,
        ..Material::default()
    };
    for (card, expected) in [
        (black(AlphaMode::Opaque, 0.3), 0.0),
        (black(AlphaMode::Mask, 0.3), 1.0),
        (black(AlphaMode::Mask, 0.6), 0.0),
        (
            Material {
                alpha_cutoff: 0.2,
                ..black(AlphaMode::Mask, 0.3)
            },
            0.0,
        ),
        (black(AlphaMode::Blend, 0.25), 0.75),
        (black(AlphaMode::Blend, 0.7), 0.3),
    ] {
        let materials = [card, emitter(1.0)];
        let placements = [
            Placement::new(0, identity(), 0),
            Placement::new(1, identity(), 1),
        ];
        let seen = centre(&trace(
            &gpu,
            &meshes,
            &placements,
            &materials,
            64,
            &mut turns,
        ));
        println!(
            "{:?} alpha {} cutoff {}: {seen:.4} against {expected}",
            card.alpha_mode, card.alpha, card.alpha_cutoff
        );
        assert!(
            (seen - expected).abs() < 0.03,
            "{:?} alpha {}: {seen} against {expected}",
            card.alpha_mode,
            card.alpha
        );
    }
}
