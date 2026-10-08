use pfx_gpu::{Gpu, wgpu};
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, OpaqueFeatures, Scene, SceneWind, Sun, multiply,
};
use pfx_live::maps::MapImages;
use pfx_live::noise_tiles;
use pfx_load::{ColorSpace, Image, Pixels};
use pfx_materials::{
    Crinkle, Fibre, Grime, Material, NoiseKind, NoiseLayer, PlankWood, Scratch, WallMottle,
};

const SIZE: u32 = 512;
const ABOVE: f32 = 0.02;
const PITCH: f32 = 31.0;
const FOV: f32 = 60.0;
const HEIGHTS: [f32; 4] = [0.0123, 0.0371, 0.0629, 0.0917];

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

fn places() -> Vec<[f32; 3]> {
    HEIGHTS
        .iter()
        .enumerate()
        .flat_map(|(i, &height)| {
            (0..24).map(move |k| {
                [
                    0.731 * (k % 6) as f32 + 0.113 * i as f32 + 0.157 * (k / 6) as f32,
                    height + 0.0031 * (k / 6) as f32,
                    -0.419 * (k % 6) as f32 - 0.271 * i as f32 + 0.389 * (k / 6) as f32,
                ]
            })
        })
        .collect()
}

fn camera(at: [f32; 3]) -> Camera {
    let b = basis();
    let eye = [at[0], at[1] + ABOVE, at[2]];
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
            -dot(b.right, eye),
            -dot(b.up, eye),
            dot(b.forward, eye),
            1.0,
        ],
    ];
    Camera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: eye,
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
    let t = ABOVE / -d[1];
    Some(std::array::from_fn(|k| d[k] * t))
}

fn pixel(x: u32, y: u32) -> Option<f32> {
    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
    let centre = hit(cx, cy)?;
    let right = hit(cx + 1.0, cy)?;
    let below = hit(cx, cy + 1.0)?;
    let length = |a: [f32; 3]| {
        let d: [f32; 3] = std::array::from_fn(|k| a[k] - centre[k]);
        dot(d, d).sqrt()
    };
    Some(length(right).max(length(below)))
}

fn regions(near_rate: f32, far_rate: f32) -> (Vec<usize>, Vec<usize>) {
    let mut near = Vec::new();
    let mut far = Vec::new();
    for y in 0..SIZE - 1 {
        for x in 0..SIZE - 1 {
            let Some(size) = pixel(x, y) else {
                continue;
            };
            let index = (y * SIZE + x) as usize;
            if size * near_rate < 0.4 {
                near.push(index);
            }
            if size * far_rate > 1.25 {
                far.push(index);
            }
        }
    }
    (near, far)
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
    material.layers[0] = NoiseLayer { seed: 7, ..layer };
    if layer.kind == NoiseKind::Scratch {
        material.clearcoat = 1.0;
        material.clearcoat_roughness = 0.08;
    }
    material
}

fn uncoated_scratch() -> Material {
    let mut material = plain();
    material.roughness = 0.3;
    material.layers[0] = Scratch::SAMPLE.layer(0.8, 7);
    material
}

fn silent(mut material: Material) -> Material {
    material.layers[0].amplitude = 0.0;
    material
}

const SKY_SUN: [f32; 3] = [0.2, 0.8, 0.5];
const GLINT_SUN: [f32; 3] = [0.05, 0.52, -0.85];

fn render(frame: &mut Frame, material: Material, at: [f32; 3], on: OpaqueFeatures) -> Vec<f32> {
    let sun = if material.layers[0].kind == NoiseKind::Scratch {
        GLINT_SUN
    } else {
        SKY_SUN
    };
    let handle = frame
        .upload_mesh(MeshData {
            positions: &[
                [at[0] - 20.0, at[1], at[2] - 20.0],
                [at[0] + 20.0, at[1], at[2] - 20.0],
                [at[0] - 20.0, at[1], at[2] + 0.5],
                [at[0] + 20.0, at[1], at[2] + 0.5],
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
        camera: camera(at),
        time: 0.0,
        seed: 5,
        sun: Sun {
            direction: sun,
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
    let pixels = frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .into_iter()
        .map(|bits| half::f16::from_bits(bits).to_f32())
        .collect();
    frame.release_mesh(handle).unwrap();
    pixels
}

fn encode(linear: f32) -> f64 {
    let v = linear.clamp(0.0, 1.0);
    let s = pfx_materials::encode_channel(v);
    f64::from(s) * 255.0
}

#[derive(Default)]
struct Effect {
    count: f64,
    sum: f64,
    within: f64,
}

impl Effect {
    fn add(&mut self, shaded: &[f32], flat: &[f32], pixels: &[usize]) {
        let values: Vec<f64> = pixels
            .iter()
            .flat_map(|&index| (0..3).map(move |c| index * 4 + c))
            .map(|k| encode(shaded[k]) - encode(flat[k]))
            .collect();
        let total: f64 = values.iter().sum();
        let mean = total / values.len() as f64;
        self.count += values.len() as f64;
        self.sum += total;
        self.within += values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
    }

    fn mean(&self) -> f64 {
        self.sum / self.count
    }

    fn spread(&self) -> f64 {
        (self.within / self.count).sqrt()
    }
}

struct Case {
    name: &'static str,
    feature: &'static str,
    material: Material,
    near_rate: f32,
    far_rate: f32,
    spread_band: f64,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "scratch",
            feature: "scratch",
            material: layered(Scratch::SAMPLE.layer(0.8, 0)),
            near_rate: Scratch::SAMPLE.line,
            far_rate: Scratch::SAMPLE.line,
            spread_band: 0.15,
        },
        Case {
            name: "scratch, uncoated",
            feature: "scratch",
            material: uncoated_scratch(),
            near_rate: Scratch::SAMPLE.line,
            far_rate: Scratch::SAMPLE.line,
            spread_band: 0.15,
        },
        Case {
            name: "wall mottle",
            feature: "wall",
            material: layered(WallMottle::SAMPLE.layer(1.0, 0)),
            near_rate: WallMottle::SAMPLE.fine[1],
            far_rate: WallMottle::SAMPLE.fine[1],
            spread_band: 0.15,
        },
        Case {
            name: "fibre",
            feature: "fibre",
            material: layered(Fibre::SAMPLE.layer(1.0, 0)),
            near_rate: Fibre::SAMPLE.across,
            far_rate: Fibre::SAMPLE.across,
            spread_band: 0.15,
        },
        Case {
            name: "crinkle",
            feature: "crinkle",
            material: layered(Crinkle::SAMPLE.layer(1.0, 0)),
            near_rate: Crinkle::SAMPLE.frequencies[1],
            far_rate: Crinkle::SAMPLE.frequencies[1],
            spread_band: 0.15,
        },
    ]
}

fn kept() -> [(&'static str, Material); 2] {
    [
        ("grime", layered(Grime::SAMPLE.layer(1.0, 0))),
        ("plank wood", layered(PlankWood::SAMPLE.layer(1.0, 0))),
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn grime_and_wood_keep_their_hash_noise_beside_the_tiles() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let at = places()[3];
    for (name, material) in kept() {
        let shipped = render(&mut frame, material, at, OpaqueFeatures::NONE);
        let hashed = render(&mut frame, material, at, hashed());
        assert!(
            shipped
                .iter()
                .zip(&hashed)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{name}: the tiles changed the frame"
        );
    }
}

fn hashed() -> OpaqueFeatures {
    OpaqueFeatures::named("procedural_detail").unwrap()
}

fn general() -> OpaqueFeatures {
    OpaqueFeatures::ALL
        .without(OpaqueFeatures::named("fine_noise").unwrap())
        .without(OpaqueFeatures::named("procedural_lacquer").unwrap())
        .without(hashed())
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn each_kind_from_the_tiles_keeps_the_hash_noises_strength_and_mean() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    assert_eq!(frame.maps().noise, Some(0));
    let places = places();
    let mut failures = Vec::new();
    for case in cases() {
        let (near, far) = regions(case.near_rate, case.far_rate);
        assert!(near.len() > 1000, "{}: {} near px", case.name, near.len());
        assert!(far.len() > 1000, "{}: {} far px", case.name, far.len());
        let mut hash = [Effect::default(), Effect::default()];
        let mut tile = [Effect::default(), Effect::default()];
        for (index, &at) in places.iter().enumerate() {
            let flat = render(&mut frame, silent(case.material), at, OpaqueFeatures::NONE);
            let tiled = render(&mut frame, case.material, at, OpaqueFeatures::NONE);
            if index == 0 {
                let drawn = frame.opaque_drawn().to_vec();
                assert!(
                    drawn.iter().all(|used| used.names().contains(&case.feature)
                        && !used.procedural_detail
                        && !used.lacquer),
                    "{}: the kind is compiled in alone, from the tiles: {:?}",
                    case.name,
                    drawn.iter().map(|used| used.names()).collect::<Vec<_>>()
                );
                let again = render(&mut frame, case.material, at, OpaqueFeatures::NONE);
                assert!(
                    tiled
                        .iter()
                        .zip(&again)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{}: the frame is deterministic",
                    case.name
                );
                let wide = render(&mut frame, case.material, at, general());
                assert!(
                    tiled
                        .iter()
                        .zip(&wide)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{}: the general pipeline reads the tiles alike",
                    case.name
                );
            }
            let hashed = render(&mut frame, case.material, at, hashed());
            for (k, region) in [&near, &far].into_iter().enumerate() {
                hash[k].add(&hashed, &flat, region);
                tile[k].add(&tiled, &flat, region);
            }
        }
        for (k, region) in ["near", "far"].into_iter().enumerate() {
            println!(
                "{}, {region}: {} px over {} placements; effect mean {:+.3}/255 hashed, {:+.3}/255 tiled (change {:+.3}); spread {:.3} hashed, {:.3} tiled (ratio {:.3})",
                case.name,
                (if k == 0 { near.len() } else { far.len() }),
                places.len(),
                hash[k].mean(),
                tile[k].mean(),
                tile[k].mean() - hash[k].mean(),
                hash[k].spread(),
                tile[k].spread(),
                tile[k].spread() / hash[k].spread()
            );
        }
        let ratio = tile[0].spread() / hash[0].spread();
        if (ratio - 1.0).abs() > case.spread_band {
            failures.push(format!("{}: near spread ratio {ratio:.3}", case.name));
        }
        let change = tile[0].mean() - hash[0].mean();
        if change.abs() > 0.5 {
            failures.push(format!("{}: near mean change {change:+.3}/255", case.name));
        }
        let change = tile[1].mean() - hash[1].mean();
        if change.abs() > 0.5 {
            failures.push(format!("{}: far mean change {change:+.3}/255", case.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

fn read_layer(gpu: &Gpu, texture: &wgpu::Texture, layer: u32) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("noise tile"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    mapped
        .chunks_exact(row as usize)
        .flat_map(|line| line[..(width * 4) as usize].to_vec())
        .collect()
}

fn metal(size: u32, value: u8) -> Image {
    Image {
        width: size,
        height: size,
        space: ColorSpace::Linear,
        pixels: Pixels::Eight(vec![value; (size * size * 4) as usize]),
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_tiles_follow_the_lacquer_tile_and_without_them_the_kinds_hash() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let fitting = [metal(noise_tiles::SIZE, 90)];
    frame
        .set_maps(&MapImages {
            metal: &fitting,
            ..MapImages::default()
        })
        .unwrap();
    let lacquer = frame.maps().noise.expect("a 512² metal map leaves room");
    assert_eq!(frame.maps().metal.layers, lacquer + 1 + noise_tiles::LAYERS);
    for layer in 0..noise_tiles::LAYERS {
        let read = read_layer(&frame.gpu, &frame.maps().metal.texture, lacquer + 1 + layer);
        assert!(
            read == noise_tiles::bytes(layer),
            "tile layer {layer} holds other bytes"
        );
    }
    let at = places()[5];
    let other = [metal(64, 90)];
    for case in cases() {
        frame.set_maps(&MapImages::default()).unwrap();
        let hashed = render(&mut frame, case.material, at, hashed());
        frame
            .set_maps(&MapImages {
                metal: &other,
                ..MapImages::default()
            })
            .unwrap();
        assert_eq!(frame.maps().noise, None);
        let untiled = render(&mut frame, case.material, at, OpaqueFeatures::NONE);
        let same = hashed
            .iter()
            .zip(&untiled)
            .filter(|(a, b)| a.to_bits() == b.to_bits())
            .count();
        let worst = hashed
            .iter()
            .zip(&untiled)
            .map(|(a, b)| (encode(*a) - encode(*b)).abs())
            .fold(0.0, f64::max);
        println!(
            "{}: without the tiles, {same} of {} values equal the forced hash noise, worst {worst:.4}/255",
            case.name,
            hashed.len()
        );
        assert!(
            worst < 0.1 && same * 1000 > hashed.len() * 999,
            "{}: without the tiles the kind moved {worst}/255 from its hash noise",
            case.name
        );
    }
}
