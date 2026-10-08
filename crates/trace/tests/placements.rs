use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::detail::{ContentFace, Lens, Transform};
use pfx_trace::stage::{ALPHA_CUTOFF, Image, Mesh, Placement, PlacementContent, Stage, Staged};
use pfx_trace::{Camera, Projection, Sun};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 80;
const PER_METRE: f32 = 40.0;
const IDENTITY: Transform = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
const RING_CENTRE: [f32; 2] = [0.6, 0.0];
const RING_SHADOW: [f32; 2] = [-0.4, 0.0];
const RING: (f32, f32) = (0.3, 0.45);

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
            alpha: None,
            indices: &self.indices,
        }
    }

    fn quad(lo: [f32; 2], hi: [f32; 2], height: f32) -> Self {
        Self {
            positions: vec![
                [lo[0], height, lo[1]],
                [hi[0], height, lo[1]],
                [hi[0], height, hi[1]],
                [lo[0], height, hi[1]],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 2, 1, 0, 3, 2],
        }
    }

    fn sphere(centre: [f32; 3], radius: f32) -> Self {
        let (rings, segments) = (24, 48);
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
            positions: normals
                .iter()
                .map(|n| std::array::from_fn(|k| centre[k] + n[k] * radius))
                .collect(),
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; normals.len()],
            normals,
            uvs,
            indices,
        }
    }
}

fn lambert(base: [f32; 3]) -> Material {
    Material {
        base,
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    }
}

fn ring_texels() -> Vec<[u8; 4]> {
    let side = 64;
    (0..side * side)
        .map(|index| {
            let u = ((index % side) as f32 + 0.5) / side as f32 - 0.5;
            let v = ((index / side) as f32 + 0.5) / side as f32 - 0.5;
            let r = (u * u + v * v).sqrt();
            if (RING.0..=RING.1).contains(&r) {
                [255, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        })
        .collect()
}

fn render(gpu: &Gpu, meshes: &[Mesh<'_>], placements: &[Placement<'_>]) -> Vec<[f32; 3]> {
    let materials = [
        lambert([0.5; 3]),
        lambert([0.8, 0.1, 0.1]),
        lambert([0.0; 3]),
    ];
    let staged = Stage {
        meshes,
        instances: placements,
        materials: &materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        },
        sun: Sun {
            direction: {
                let length = 1.25_f32.sqrt();
                [1.0 / length, 0.5 / length, 0.0]
            },
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: Camera {
            origin: [0.0, 3.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, -1.0],
        },
        projection: Projection::Orthographic {
            width: WIDTH as f32 / PER_METRE,
            height: HEIGHT as f32 / PER_METRE,
        },
        lens: Lens::default(),
    }
    .build()
    .unwrap();
    let mut trace = staged.trace(gpu, WIDTH, HEIGHT).unwrap();
    for pass in 0..32 {
        trace.sample(gpu, 4, 5 + pass).unwrap();
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

fn disc(image: &[[f32; 3]], centre: [f32; 2], inner: f32, outer: f32) -> [f64; 3] {
    let mut sum = [0.0_f64; 3];
    let mut count = 0.0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let world = [
                (x as f32 + 0.5) / PER_METRE - WIDTH as f32 / PER_METRE / 2.0,
                (y as f32 + 0.5) / PER_METRE - HEIGHT as f32 / PER_METRE / 2.0,
            ];
            let r = ((world[0] - centre[0]).powi(2) + (world[1] - centre[1]).powi(2)).sqrt();
            if (inner..outer).contains(&r) {
                let pixel = image[(y * WIDTH + x) as usize];
                for c in 0..3 {
                    sum[c] += f64::from(pixel[c]);
                }
                count += 1.0;
            }
        }
    }
    assert!(count >= 20.0, "{count} pixels around {centre:?}");
    sum.map(|s| s / count)
}

fn grey(colour: [f64; 3]) -> f64 {
    colour.iter().sum::<f64>() / 3.0
}

fn assert_near(seen: [f64; 3], want: [f64; 3], tolerance: f64, what: &str) {
    for c in 0..3 {
        let ratio = seen[c] / want[c];
        assert!(
            (ratio - 1.0).abs() <= tolerance,
            "{what}: channel {c} {} against {} (ratio {ratio:.4})",
            seen[c],
            want[c]
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_placement_that_casts_no_shadow_leaves_the_floor_lit_and_still_renders() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let floor = Arrays::quad([-2.0, -1.0], [2.0, 1.0], 0.0);
    let sphere = Arrays::sphere([0.6, 0.6, 0.0], 0.3);
    let meshes = [floor.mesh(), sphere.mesh()];
    let bare = render(&gpu, &meshes, &[Placement::new(0, IDENTITY, 0)]);
    let casting = render(
        &gpu,
        &meshes,
        &[
            Placement::new(0, IDENTITY, 0),
            Placement::new(1, IDENTITY, 1),
        ],
    );
    let quiet = render(
        &gpu,
        &meshes,
        &[
            Placement::new(0, IDENTITY, 0),
            Placement {
                casts_shadow: false,
                ..Placement::new(1, IDENTITY, 1)
            },
        ],
    );
    let shadow = [-0.6, 0.0];
    let open = disc(&bare, shadow, 0.0, 0.12);
    let shaded = disc(&casting, shadow, 0.0, 0.12);
    let unshaded = disc(&quiet, shadow, 0.0, 0.12);
    println!("floor at the shadow: bare {open:?}, casting {shaded:?}, no shadow {unshaded:?}");
    assert!(grey(shaded) < 0.1 * grey(open));
    assert_near(
        unshaded,
        open,
        0.02,
        "the floor under a sphere that casts no shadow",
    );
    let top = disc(&quiet, [0.6, 0.0], 0.0, 0.12);
    let floor_there = disc(&bare, [0.6, 0.0], 0.0, 0.12);
    println!("sphere top {top:?}, floor there {floor_there:?}");
    assert!(
        top[0] > 4.0 * top[1],
        "the sphere still renders red: {top:?}"
    );
    assert!(top[1] < 0.5 * floor_there[1]);
    assert_near(
        disc(&quiet, [0.6, 0.0], 0.0, 0.12),
        disc(&casting, [0.6, 0.0], 0.0, 0.12),
        0.02,
        "the sphere itself",
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_cutout_carrier_shows_its_ring_and_the_ring_shadow_only() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let floor = Arrays::quad([-2.0, -1.0], [2.0, 1.0], 0.0);
    let carrier = Arrays::quad(
        [RING_CENTRE[0] - 0.5, RING_CENTRE[1] - 0.5],
        [RING_CENTRE[0] + 0.5, RING_CENTRE[1] + 0.5],
        0.5,
    );
    let meshes = [floor.mesh(), carrier.mesh()];
    let texels = ring_texels();
    let image = Image {
        width: 64,
        height: 64,
        texels: &texels,
        srgb: true,
    };
    let carried = |cutout| Placement {
        content: Some(PlacementContent {
            cutout,
            face: ContentFace::Front,
            ..PlacementContent::new(image)
        }),
        ..Placement::new(1, IDENTITY, 2)
    };
    let bare = render(&gpu, &meshes, &[Placement::new(0, IDENTITY, 0)]);
    let cut = render(
        &gpu,
        &meshes,
        &[Placement::new(0, IDENTITY, 0), carried(true)],
    );
    let solid = render(
        &gpu,
        &meshes,
        &[Placement::new(0, IDENTITY, 0), carried(false)],
    );
    let middle = (RING.0 + RING.1) / 2.0;
    let ring = disc(&cut, RING_CENTRE, middle - 0.04, middle + 0.04);
    println!("ring {ring:?}");
    assert!(ring[0] > 0.3 && ring[0] > 20.0 * ring[1], "{ring:?}");

    let hole = disc(&cut, RING_CENTRE, 0.0, RING.0 - 0.08);
    let hole_bare = disc(&bare, RING_CENTRE, 0.0, RING.0 - 0.08);
    let hole_solid = disc(&solid, RING_CENTRE, 0.0, RING.0 - 0.08);
    println!("through the hole {hole:?}, no carrier {hole_bare:?}, solid {hole_solid:?}");
    assert_near(hole, hole_bare, 0.03, "the floor through the hole");
    assert!(grey(hole_solid) < 0.1 * grey(hole_bare));

    let corner = [RING_CENTRE[0] + 0.43, RING_CENTRE[1] + 0.41];
    assert_near(
        disc(&cut, corner, 0.0, 0.07),
        disc(&bare, corner, 0.0, 0.07),
        0.03,
        "the floor through the carrier's clear corner",
    );

    let ring_shadow = disc(&cut, RING_SHADOW, middle - 0.04, middle + 0.04);
    let lit_there = disc(&bare, RING_SHADOW, middle - 0.04, middle + 0.04);
    println!("ring shadow {ring_shadow:?}, lit {lit_there:?}");
    assert!(grey(ring_shadow) < 0.1 * grey(lit_there));

    let hole_shadow = disc(&cut, RING_SHADOW, 0.0, RING.0 - 0.08);
    let hole_lit = disc(&bare, RING_SHADOW, 0.0, RING.0 - 0.08);
    let solid_shadow = disc(&solid, RING_SHADOW, 0.0, RING.0 - 0.08);
    println!("the hole's light {hole_shadow:?}, no carrier {hole_lit:?}, solid {solid_shadow:?}");
    assert_near(
        hole_shadow,
        hole_lit,
        0.03,
        "the floor lit through the hole",
    );
    assert!(grey(solid_shadow) < 0.1 * grey(hole_lit));
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn paced(
    gpu: &Gpu,
    staged: &Staged,
    width: u32,
    height: u32,
    samples: u32,
    seed: u32,
    turns: &mut Turns,
) -> (Vec<u8>, Vec<u8>, f64) {
    let mut trace = staged.trace(gpu, width, height).unwrap();
    let mut pacer = Pacer::default();
    let stats = trace
        .sample_paced(gpu, samples, seed, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    let output = trace.readback(gpu).unwrap();
    (output.color, output.albedo, stats.longest_ms())
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn unset_placement_fields_trace_the_bytes_they_traced_before() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let floor = Arrays::quad([-1.0, -0.6], [1.0, 0.6], 0.0);
    let card = Arrays::quad([-0.2, -0.2], [0.2, 0.2], 0.4);
    let alpha = [1.0_f32, 0.2, 0.8];
    let wedge = Mesh {
        positions: &[[0.4, 0.2, -0.1], [0.9, 0.2, -0.1], [0.4, 0.2, 0.4]],
        normals: &[[0.0, 1.0, 0.0]; 3],
        tangents: &[[1.0, 0.0, 0.0, 1.0]; 3],
        uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        alpha: Some(&alpha),
        indices: &[0, 1, 2],
    };
    let texels = ring_texels();
    let image = Image {
        width: 64,
        height: 64,
        texels: &texels,
        srgb: true,
    };
    let meshes = [floor.mesh(), card.mesh(), wedge];
    let materials = [
        lambert([0.5; 3]),
        lambert([0.8, 0.1, 0.1]),
        lambert([0.0; 3]),
    ];
    let content = PlacementContent {
        cutout: true,
        face: ContentFace::Front,
        ..PlacementContent::new(image)
    };
    let plain = [
        Placement::new(0, IDENTITY, 0),
        Placement {
            content: Some(content),
            ..Placement::new(1, IDENTITY, 2)
        },
        Placement::new(2, IDENTITY, 1),
    ];
    let stated = [
        Placement {
            clip: [[0.0; 4]; 2],
            shadow_only: false,
            alpha_cutoff: ALPHA_CUTOFF,
            two_sided: true,
            ..Placement::new(0, IDENTITY, 0)
        },
        Placement {
            content: Some(content),
            clip: [[0.0; 4]; 2],
            shadow_only: false,
            alpha_cutoff: ALPHA_CUTOFF,
            two_sided: true,
            ..Placement::new(1, IDENTITY, 2)
        },
        Placement {
            clip: [[0.0; 4]; 2],
            shadow_only: false,
            alpha_cutoff: ALPHA_CUTOFF,
            two_sided: true,
            ..Placement::new(2, IDENTITY, 1)
        },
    ];
    let mut turns = Turns::default();
    let (plain_color, _, plain_longest) = paced(
        &gpu,
        &digest_stage(&meshes, &plain, &materials),
        48,
        32,
        4,
        9,
        &mut turns,
    );
    let (stated_color, _, stated_longest) = paced(
        &gpu,
        &digest_stage(&meshes, &stated, &materials),
        48,
        32,
        4,
        9,
        &mut turns,
    );
    let hash = digest(&plain_color);
    println!(
        "unset fields digest {hash:016x} longest {plain_longest:.2} ms, stated {stated_longest:.2} ms"
    );
    assert_eq!(hash, 0x6ad8_c2f3_f97b_194e);
    assert_eq!(plain_color, stated_color);
    assert!(plain_longest < 300.0 && stated_longest < 300.0);
}

fn digest_stage<'a>(
    meshes: &'a [Mesh<'a>],
    placements: &'a [Placement<'a>],
    materials: &'a [Material],
) -> Staged {
    Stage {
        meshes,
        instances: placements,
        materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0, 0.0, 0.0, 1.0]],
        },
        sun: Sun {
            direction: {
                let length = 1.25_f32.sqrt();
                [1.0 / length, 0.5 / length, 0.0]
            },
            color: [1.0; 3],
            intensity: 3.0,
        },
        camera: Camera {
            origin: [0.0, 3.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, -1.0],
        },
        projection: Projection::Orthographic {
            width: 48.0 / PER_METRE,
            height: 32.0 / PER_METRE,
        },
        lens: Lens::default(),
    }
    .build()
    .unwrap()
}

const SOFT_SIDE: u32 = 64;
const SOFT_VIEW: f32 = 0.9;
const SOFT_SAMPLES: u32 = 8;
const SOFT_SEED: u32 = 11;

fn soft_ring() -> Vec<[u8; 4]> {
    let side = SOFT_SIDE;
    let (inner, peak, outer) = (0.18_f32, 0.32, 0.46);
    (0..side * side)
        .map(|index| {
            let u = ((index % side) as f32 + 0.5) / side as f32 - 0.5;
            let v = ((index / side) as f32 + 0.5) / side as f32 - 0.5;
            let radius = (u * u + v * v).sqrt();
            let alpha = if radius <= inner || radius >= outer {
                0.0
            } else if radius < peak {
                (radius - inner) / (peak - inner)
            } else {
                (outer - radius) / (outer - peak)
            };
            let byte = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            [255, 0, 0, byte]
        })
        .collect()
}

fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

fn next_random(state: &mut u32) -> f32 {
    *state = pcg(*state);
    (*state >> 8) as f32 / 16_777_216.0
}

fn content_alpha(texels: &[[u8; 4]], uv: [f32; 2]) -> f32 {
    let side = SOFT_SIDE;
    let size = side as f32;
    let point = [
        (uv[0] * size - 0.5).clamp(0.0, size - 1.0),
        (uv[1] * size - 0.5).clamp(0.0, size - 1.0),
    ];
    let lo = [point[0].floor() as u32, point[1].floor() as u32];
    let hi = [(lo[0] + 1).min(side - 1), (lo[1] + 1).min(side - 1)];
    let fraction = [point[0].fract(), point[1].fract()];
    let texel = |x: u32, y: u32| f32::from(texels[(y * side + x) as usize][3]) / 255.0;
    let a = texel(lo[0], lo[1]);
    let b = texel(hi[0], lo[1]);
    let c = texel(lo[0], hi[1]);
    let d = texel(hi[0], hi[1]);
    let along = |left: f32, right: f32| left + (right - left) * fraction[0];
    along(a, b) + (along(c, d) - along(a, b)) * fraction[1]
}

fn reference_mask(texels: &[[u8; 4]], cutoff: f32) -> Vec<bool> {
    let side = SOFT_SIDE;
    let half = SOFT_VIEW * 0.5;
    let mut mask = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            let index = y * side + x;
            let mut hits = 0u32;
            for sample in 0..SOFT_SAMPLES {
                let mut state = pcg(index.wrapping_mul(1973).wrapping_add(pcg(SOFT_SEED
                    .wrapping_mul(9277)
                    .wrapping_add(sample.wrapping_mul(7919))
                    .wrapping_add(26699))));
                let jitter = [next_random(&mut state), next_random(&mut state)];
                let ndc = [
                    (x as f32 + jitter[0]) / side as f32 * 2.0 - 1.0,
                    (y as f32 + jitter[1]) / side as f32 * 2.0 - 1.0,
                ];
                let world = [ndc[0] * half, ndc[1] * half];
                let alpha = content_alpha(texels, [world[0] + 0.5, world[1] + 0.5]);
                if alpha >= cutoff {
                    hits += 1;
                }
            }
            mask.push(hits * 2 >= SOFT_SAMPLES);
        }
    }
    mask
}

fn albedo_mask(bytes: &[u8]) -> Vec<bool> {
    bytes
        .chunks_exact(16)
        .map(|pixel| f32::from_le_bytes(pixel[12..16].try_into().unwrap()) >= 0.5)
        .collect()
}

fn iou(left: &[bool], right: &[bool]) -> f64 {
    let mut intersection = 0.0;
    let mut union = 0.0;
    for (a, b) in left.iter().zip(right) {
        if *a && *b {
            intersection += 1.0;
        }
        if *a || *b {
            union += 1.0;
        }
    }
    intersection / union
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn three_cutoffs_trace_three_ring_widths() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let texels = soft_ring();
    let image = Image {
        width: SOFT_SIDE,
        height: SOFT_SIDE,
        texels: &texels,
        srgb: true,
    };
    let card = Arrays::quad([-0.5, -0.5], [0.5, 0.5], 0.0);
    let meshes = [card.mesh()];
    let materials = [lambert([0.0; 3])];
    let mut turns = Turns::default();
    let mut counts = Vec::new();
    for cutoff in [0.25_f32, 0.5, 0.9] {
        let placements = [Placement {
            alpha_cutoff: cutoff,
            content: Some(PlacementContent {
                cutout: true,
                face: ContentFace::Front,
                ..PlacementContent::new(image)
            }),
            ..Placement::new(0, IDENTITY, 0)
        }];
        let staged = Stage {
            meshes: &meshes,
            instances: &placements,
            materials: &materials,
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0, 0.0, 0.0, 1.0]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 3.0,
            },
            camera: Camera {
                origin: [0.0, 3.0, 0.0],
                forward: [0.0, -1.0, 0.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 0.0, -1.0],
            },
            projection: Projection::Orthographic {
                width: SOFT_VIEW,
                height: SOFT_VIEW,
            },
            lens: Lens::default(),
        }
        .build()
        .unwrap();
        let (_, albedo, longest) = paced(
            &gpu,
            &staged,
            SOFT_SIDE,
            SOFT_SIDE,
            SOFT_SAMPLES,
            SOFT_SEED,
            &mut turns,
        );
        let traced = albedo_mask(&albedo);
        let reference = reference_mask(&texels, cutoff);
        let overlap = iou(&traced, &reference);
        let count = traced.iter().filter(|hit| **hit).count();
        println!("cutoff {cutoff}: covered {count}, iou {overlap:.4}, longest {longest:.2} ms");
        assert!(overlap >= 0.95, "cutoff {cutoff}: iou {overlap}");
        assert!(longest < 300.0, "{longest}");
        counts.push(count);
    }
    assert!(counts[0] > counts[1] && counts[1] > counts[2], "{counts:?}");
}
