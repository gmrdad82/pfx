use std::collections::BTreeMap;

use pfx_gpu::Gpu;
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, OpaqueFeatures, Scene, SceneWind, Sun, multiply,
};
use pfx_live::maps::RING_FADE;
use pfx_materials::{
    Fibre, Grime, Material, NoiseKind, NoiseLayer, PlankWood, SCRATCH_STRETCH, Scratch, WallMottle,
};

const SIZE: u32 = 512;
const EYE: [f32; 3] = [0.0, 0.02, 0.0];
const PITCH: f32 = 31.0;
const FOV: f32 = 60.0;
const PLANK: f32 = PlankWood::SAMPLE.width;
const RUN: u32 = 8;

type Group = (i64, u32, u32);

struct Basis {
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
}

fn basis() -> Basis {
    let pitch = PITCH.to_radians();
    Basis {
        right: [1.0, 0.0, 0.0],
        up: [0.0, pitch.cos(), -pitch.sin()],
        forward: [0.0, -pitch.sin(), -pitch.cos()],
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn camera() -> Camera {
    let b = basis();
    let near = 0.005;
    let far = 20.0;
    let f = 1.0 / (FOV.to_radians() * 0.5).tan();
    let projection = [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    let view = [
        [b.right[0], b.up[0], -b.forward[0], 0.0],
        [b.right[1], b.up[1], -b.forward[1], 0.0],
        [b.right[2], b.up[2], -b.forward[2], 0.0],
        [
            -dot(b.right, EYE),
            -dot(b.up, EYE),
            dot(b.forward, EYE),
            1.0,
        ],
    ];
    Camera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: EYE,
    }
}

fn hit(x: f32, y: f32) -> Option<[f32; 3]> {
    let b = basis();
    let tan = (FOV.to_radians() * 0.5).tan();
    let nx = (2.0 * x / SIZE as f32 - 1.0) * tan;
    let ny = (1.0 - 2.0 * y / SIZE as f32) * tan;
    let d: [f32; 3] = std::array::from_fn(|k| b.forward[k] + b.right[k] * nx + b.up[k] * ny);
    if d[1] >= -1e-4 {
        return None;
    }
    let t = -EYE[1] / d[1];
    Some(std::array::from_fn(|k| EYE[k] + d[k] * t))
}

fn footprint(x: u32, y: u32) -> Option<([f32; 3], [f32; 3])> {
    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
    let centre = hit(cx, cy)?;
    let right = hit(cx + 1.0, cy)?;
    let below = hit(cx, cy + 1.0)?;
    Some((
        std::array::from_fn(|k| right[k] - centre[k]),
        std::array::from_fn(|k| below[k] - centre[k]),
    ))
}

fn cells(dx: [f32; 3], dy: [f32; 3], rate: [f32; 3]) -> f32 {
    let length = |d: [f32; 3]| d.iter().map(|v| v * v).sum::<f32>().sqrt();
    length(dx).max(length(dy)) * rate.into_iter().fold(0.0, f32::max)
}

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn plain() -> Material {
    Material {
        base: [0.6, 0.55, 0.5],
        roughness: 0.6,
        ..Material::default()
    }
}

fn layered(layer: NoiseLayer) -> Material {
    let mut material = plain();
    material.layers[0] = layer;
    material
}

fn render(frame: &mut Frame, material: Material, on: OpaqueFeatures) -> Vec<f32> {
    let handle = frame
        .upload_mesh(MeshData {
            positions: &[
                [-20.0, 0.0, -20.0],
                [20.0, 0.0, -20.0],
                [-20.0, 0.0, 0.5],
                [20.0, 0.0, 0.5],
            ],
            normals: &[[0.0, 1.0, 0.0]; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 2, 1, 1, 2, 3],
        })
        .unwrap();
    let instances = [Instance::new(handle, identity(), 0, 1)];
    let materials = [material];
    let scene = Scene {
        camera: camera(),
        time: 0.0,
        seed: 5,
        sun: Sun {
            direction: [0.2, 0.8, 0.5],
            colour: [1.0, 0.95, 0.9],
            intensity: 1.5,
        },
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.set_opaque_on(on);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame.encode(&scene, &mut encoder, None, None).unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .into_iter()
        .map(|bits| half::f16::from_bits(bits).to_f32())
        .collect()
}

fn encode(linear: f32) -> f32 {
    let v = linear.clamp(0.0, 1.0);
    let s = pfx_materials::encode_channel(v);
    s * 255.0
}

fn luma(pixels: &[f32], index: usize) -> f64 {
    let p = &pixels[index * 4..index * 4 + 3];
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

fn worst_encoded(a: &[f32], b: &[f32], pixels: &[usize]) -> f32 {
    pixels
        .iter()
        .flat_map(|&i| (0..3).map(move |c| i * 4 + c))
        .map(|k| (encode(a[k]) - encode(b[k])).abs())
        .fold(0.0, f32::max)
}

fn mean_encoded(a: &[f32], b: &[f32], pixels: &[usize]) -> f64 {
    let total: f64 = pixels
        .iter()
        .flat_map(|&i| (0..3).map(move |c| i * 4 + c))
        .map(|k| f64::from((encode(a[k]) - encode(b[k])).abs()))
        .sum();
    total / (pixels.len() * 3) as f64
}

fn spread(pixels: &[f32], region: &[(usize, Group)]) -> (f64, f64) {
    let mut groups: BTreeMap<Group, Vec<f64>> = BTreeMap::new();
    for &(index, group) in region {
        groups.entry(group).or_default().push(luma(pixels, index));
    }
    let mut total = 0.0;
    let mut squares = 0.0;
    for values in groups.values() {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        total += values.iter().sum::<f64>();
        squares += values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
    }
    let count = region.len() as f64;
    (total / count, (squares / count).sqrt())
}

struct Case {
    name: &'static str,
    material: Material,
    near: Vec<[f32; 3]>,
    far: Vec<[f32; 3]>,
    flat_mean: bool,
    planks: bool,
}

fn cases() -> Vec<Case> {
    let all = |r: f32| [r, r, r];
    let fibre = Fibre::SAMPLE;
    let wood = PlankWood::SAMPLE;
    let wall = WallMottle::SAMPLE;
    let grime = Grime::SAMPLE;
    let scratch = Scratch::SAMPLE;
    let fibres = vec![
        [fibre.across, fibre.along, fibre.across],
        all(fibre.fine),
        all(fibre.mottle),
        all(fibre.ripple),
        all(fibre.swell),
    ];
    let rings = [9.0, 0.0, wood.rings * RING_FADE];
    let figure = [wood.figure[0], 0.0, wood.figure[1]];
    let walls = vec![
        all(wall.fine[1]),
        all(wall.fine[0]),
        all(wall.swell[0]),
        all(wall.swell[1]),
    ];
    let stretch = scratch.line * SCRATCH_STRETCH;
    let scratches = vec![
        [stretch, scratch.line, stretch],
        all(scratch.smudge[0]),
        all(scratch.smudge[1]),
        all(scratch.speck),
    ];
    vec![
        Case {
            name: "value",
            material: layered(NoiseLayer::new(NoiseKind::Value, 300.0, 1.0, 7)),
            near: vec![all(300.0)],
            far: vec![all(300.0)],
            flat_mean: true,
            planks: false,
        },
        Case {
            name: "fibre",
            material: layered(fibre.layer(1.0, 7)),
            near: fibres.clone(),
            far: fibres,
            flat_mean: true,
            planks: false,
        },
        Case {
            name: "plank wood",
            material: layered(wood.layer(1.0, 7)),
            near: vec![rings, [1.4, 0.0, 6.0], [6.0, 0.0, 20.0], figure],
            far: vec![rings, figure],
            flat_mean: false,
            planks: true,
        },
        Case {
            name: "wall mottle",
            material: layered(wall.layer(1.0, 7)),
            near: walls.clone(),
            far: walls,
            flat_mean: false,
            planks: false,
        },
        Case {
            name: "grime",
            material: layered(grime.layer(1.0, 7)),
            near: vec![all(grime.frequencies[0]), all(grime.frequencies[1])],
            far: vec![all(grime.frequencies[0]), all(grime.frequencies[1])],
            flat_mean: false,
            planks: false,
        },
        Case {
            name: "scratch",
            material: layered(scratch.layer(0.8, 7)),
            near: scratches.clone(),
            far: scratches,
            flat_mean: false,
            planks: false,
        },
    ]
}

fn plank(case: &Case, x: u32, y: u32, dx: [f32; 3], dy: [f32; 3]) -> Option<i64> {
    if !case.planks {
        return Some(0);
    }
    let p = hit(x as f32 + 0.5, y as f32 + 0.5)?;
    let along = p[2] / PLANK;
    let inside = (along - along.floor()) * PLANK;
    let reach = dx[2].abs().max(dy[2].abs()) + 0.002;
    (inside > reach && PLANK - inside > reach).then_some(along.floor() as i64)
}

fn regions(case: &Case) -> (Vec<usize>, Vec<(usize, Group)>) {
    let mut near = Vec::new();
    let mut far = Vec::new();
    for y in 0..SIZE - 1 {
        for x in 0..SIZE - 1 {
            let Some((dx, dy)) = footprint(x, y) else {
                continue;
            };
            let index = (y * SIZE + x) as usize;
            if case.near.iter().all(|&rate| cells(dx, dy, rate) < 0.4) {
                near.push(index);
            }
            if case.far.iter().all(|&rate| cells(dx, dy, rate) > 1.25)
                && let Some(group) = plank(case, x, y, dx, dy)
            {
                far.push((index, (group, y, x / RUN)));
            }
        }
    }
    (near, far)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn octaves_under_a_pixel_fade_to_their_mean_and_close_detail_stays() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let hashed = OpaqueFeatures::named("procedural_detail").unwrap();
    let fine = OpaqueFeatures::named("fine_noise").unwrap().union(hashed);
    let flat = render(&mut frame, plain(), hashed);
    for case in cases() {
        let (near, far) = regions(&case);
        assert!(
            near.len() > 1000,
            "{}: {} near pixels",
            case.name,
            near.len()
        );
        assert!(far.len() > 1000, "{}: {} far pixels", case.name, far.len());
        let faded = render(&mut frame, case.material, hashed);
        assert!(
            frame.opaque_drawn().iter().all(|used| !used.fine_noise),
            "{}: fine noise is compiled out by default",
            case.name
        );
        let again = render(&mut frame, case.material, hashed);
        assert!(
            faded
                .iter()
                .zip(&again)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{}: the frame is deterministic",
            case.name
        );
        let full = render(&mut frame, case.material, fine);
        let near_worst = worst_encoded(&faded, &full, &near);
        let far_pixels: Vec<usize> = far.iter().map(|&(index, _)| index).collect();
        let far_change = mean_encoded(&faded, &full, &far_pixels);
        let (full_mean, full_spread) = spread(&full, &far);
        let (faded_mean, faded_spread) = spread(&faded, &far);
        let flat_full = mean_encoded(&full, &flat, &far_pixels);
        let flat_faded = mean_encoded(&faded, &flat, &far_pixels);
        println!(
            "{}: {} near px worst {near_worst:.3}/255; {} far px change {far_change:.2}/255, luma mean {full_mean:.5} -> {faded_mean:.5}, spread {full_spread:.5} -> {faded_spread:.5}, from plain {flat_full:.2} -> {flat_faded:.2}/255",
            case.name,
            near.len(),
            far.len(),
        );
        assert!(
            near_worst <= 1.0,
            "{}: near pixels moved {near_worst}/255",
            case.name
        );
        assert!(
            faded_spread < full_spread * 0.5,
            "{}: far spread {faded_spread} against {full_spread}",
            case.name
        );
        assert!(
            (faded_mean - full_mean).abs() < full_mean * 0.03,
            "{}: far mean {faded_mean} against {full_mean}",
            case.name
        );
        if case.flat_mean {
            assert!(
                flat_faded < 0.5 && flat_faded < flat_full * 0.25,
                "{}: far pixels {flat_faded}/255 from the layer's mean, {flat_full}/255 unfaded",
                case.name
            );
        }
    }
}
