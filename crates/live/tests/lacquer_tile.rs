use pfx_gpu::{Gpu, wgpu};
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, OpaqueFeatures, Scene, SceneWind, Sun, multiply,
};
use pfx_live::lacquer;
use pfx_live::maps::MapImages;
use pfx_load::{ColorSpace, Image, Pixels};
use pfx_materials::{CoatWobble, Crinkle, Family, Grime, Material, NoiseLayer, PlankWood, Scratch};

const SIZE: u32 = 512;
const ABOVE: f32 = 0.02;
const PITCH: f32 = 31.0;
const FOV: f32 = 60.0;
const HEIGHTS: [f32; 4] = [0.0123, 0.0371, 0.0629, 0.0917];
const AMPLITUDE: f32 = 0.1;

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
            (0..6).map(move |k| {
                [
                    0.731 * k as f32 + 0.113 * i as f32,
                    height,
                    -0.419 * k as f32 - 0.271 * i as f32,
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

fn regions() -> (Vec<usize>, Vec<usize>) {
    let mut near = Vec::new();
    let mut far = Vec::new();
    for y in 0..SIZE - 1 {
        for x in 0..SIZE - 1 {
            let Some(size) = pixel(x, y) else {
                continue;
            };
            let index = (y * SIZE + x) as usize;
            let [fine, broad, _] = CoatWobble::SAMPLE.frequencies;
            if size * fine < 0.4 {
                near.push(index);
            }
            if size * fine > 1.25 && size * broad < 0.4 {
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
        base: [0.6, 0.2, 0.05],
        roughness: 0.45,
        ..Material::default()
    }
}

fn lacquered(amplitude: f32) -> Material {
    let mut material = plain();
    material.family = Family::Lacquer;
    material.clearcoat = 1.0;
    material.clearcoat_roughness = 0.07;
    material.layers[0] = CoatWobble::SAMPLE.layer(amplitude, 0);
    material
}

fn layered(layer: NoiseLayer) -> Material {
    let mut material = plain();
    material.layers[0] = NoiseLayer { seed: 7, ..layer };
    material
}

fn render(frame: &mut Frame, material: Material, at: [f32; 3], on: OpaqueFeatures) -> Vec<f32> {
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

fn luma(pixels: &[f32], index: usize) -> f64 {
    let p = &pixels[index * 4..index * 4 + 3];
    0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
}

#[derive(Default)]
struct Bump {
    count: f64,
    sum: f64,
    squares: f64,
    base: f64,
}

impl Bump {
    fn add(&mut self, bumped: &[f32], flat: &[f32], pixels: &[usize]) {
        for &index in pixels {
            let d = luma(bumped, index) - luma(flat, index);
            self.count += 1.0;
            self.sum += d;
            self.squares += d * d;
            self.base += luma(flat, index);
        }
    }

    fn rms(&self) -> f64 {
        (self.squares / self.count).sqrt()
    }

    fn mean(&self) -> f64 {
        self.sum / self.count
    }

    fn base(&self) -> f64 {
        self.base / self.count
    }
}

fn procedural() -> OpaqueFeatures {
    OpaqueFeatures::named("procedural_lacquer").unwrap()
}

fn hashed_detail() -> OpaqueFeatures {
    OpaqueFeatures::named("procedural_detail").unwrap()
}

fn general() -> OpaqueFeatures {
    OpaqueFeatures::ALL
        .without(OpaqueFeatures::named("fine_noise").unwrap())
        .without(procedural())
        .without(hashed_detail())
}

fn compare_planes(
    frame: &mut Frame,
    name: &str,
    material: Material,
    near_band: f64,
    far_band: f64,
) {
    let (near, far) = regions();
    assert!(near.len() > 1000, "{} near pixels", near.len());
    assert!(far.len() > 1000, "{} far pixels", far.len());
    let mut hashed = [Bump::default(), Bump::default()];
    let mut tiled = [Bump::default(), Bump::default()];
    let places = places();
    for (index, &at) in places.iter().enumerate() {
        let flat = render(frame, plain(), at, OpaqueFeatures::NONE);
        let tile = render(frame, material, at, OpaqueFeatures::NONE);
        let hash = render(frame, material, at, procedural());
        for (index, region) in [&near, &far].into_iter().enumerate() {
            hashed[index].add(&hash, &flat, region);
            tiled[index].add(&tile, &flat, region);
        }
        if index > 0 {
            continue;
        }
        let again = render(frame, material, at, OpaqueFeatures::NONE);
        assert!(
            frame
                .opaque_drawn()
                .iter()
                .all(|used| used.lacquer && !used.procedural_lacquer),
            "{name}: the lacquer bump samples the tile"
        );
        assert!(
            tile.iter()
                .zip(&again)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{name}: the frame is deterministic"
        );
        let general_tile = render(frame, material, at, general());
        assert!(
            tile.iter()
                .zip(&general_tile)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{name}: the general pipeline samples the tile alike"
        );
    }
    for (index, region) in ["near", "far"].into_iter().enumerate() {
        let (hash, tile) = (&hashed[index], &tiled[index]);
        let ratio = tile.rms() / hash.rms();
        let shift = (tile.mean() - hash.mean()).abs() / hash.base();
        println!(
            "{name}, {region}: {} px over {} placements, bump rms {:.6} hashed, {:.6} tiled (ratio {ratio:.3}); mean shift {:.6} hashed, {:.6} tiled ({:.4}% of {:.5})",
            hash.count,
            places.len(),
            hash.rms(),
            tile.rms(),
            hash.mean(),
            tile.mean(),
            shift * 100.0,
            hash.base()
        );
    }
    for (index, (region, band)) in [("near", near_band), ("far", far_band)]
        .into_iter()
        .enumerate()
    {
        let (hash, tile) = (&hashed[index], &tiled[index]);
        let ratio = tile.rms() / hash.rms();
        assert!(
            hash.rms() > hash.base() * 0.002,
            "{name}, {region}: the bump is too faint to measure"
        );
        assert!(
            (ratio - 1.0).abs() < band,
            "{name}, {region}: the tiled bump's rms is {ratio} of the hashed one's"
        );
        assert!(
            (tile.mean() - hash.mean()).abs() < hash.mean().abs() * 0.1,
            "{name}, {region}: the bump's mean effect moved from {} to {}",
            hash.mean(),
            tile.mean()
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_lacquer_plane_near_and_far_matches_the_hashed_bump() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    assert_eq!(frame.maps().noise, Some(0));
    compare_planes(
        &mut frame,
        "lacquer layer",
        lacquered(AMPLITUDE),
        0.12,
        0.25,
    );
    assert!(
        tile_levels(&frame)[0] == lacquer::bytes(&CoatWobble::SAMPLE),
        "the frame fits its tile to the scene's coat"
    );
}

fn read_layer(gpu: &Gpu, texture: &wgpu::Texture, layer: u32, level: u32) -> Vec<u8> {
    let width = (texture.width() >> level).max(1);
    let height = (texture.height() >> level).max(1);
    let row = (width * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lacquer tile level"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
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

fn tile_levels(frame: &Frame) -> Vec<Vec<u8>> {
    let maps = frame.maps();
    let layer = maps.noise.expect("the frame carries the lacquer tile");
    (0..maps.metal.mips)
        .map(|level| read_layer(&frame.gpu, &maps.metal.texture, layer, level))
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
fn the_uploaded_tile_is_the_same_bytes_every_time() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut first = Frame::new(gpu, 64, 64).unwrap();
    let levels = tile_levels(&first);
    assert_eq!(levels.len(), 10);
    assert_eq!(levels[0], lacquer::bytes(&CoatWobble::default()));
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let second = Frame::new(gpu, 64, 64).unwrap();
    assert!(
        tile_levels(&second) == levels,
        "a second frame uploads other bytes"
    );

    let fitting = [metal(lacquer::SIZE, 90)];
    first
        .set_maps(&MapImages {
            metal: &fitting,
            ..MapImages::default()
        })
        .unwrap();
    assert_eq!(first.maps().noise, Some(1));
    assert!(
        tile_levels(&first) == levels,
        "a spare layer holds other bytes"
    );

    let other = [metal(64, 90)];
    first
        .set_maps(&MapImages {
            metal: &other,
            ..MapImages::default()
        })
        .unwrap();
    assert_eq!(first.maps().noise, None);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn frames_of_parts_without_lacquer_do_not_change() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let at = places()[7];
    let other = [metal(64, 90)];
    for (name, material) in [
        ("plain", plain()),
        ("grime", layered(Grime::SAMPLE.layer(1.0, 0))),
        ("wood", layered(PlankWood::SAMPLE.layer(1.0, 0))),
        ("scratch", layered(Scratch::SAMPLE.layer(0.8, 0))),
        ("crinkle", layered(Crinkle::SAMPLE.layer(1.0, 0))),
    ] {
        frame.set_maps(&MapImages::default()).unwrap();
        let shipped = render(&mut frame, material, at, hashed_detail());
        assert!(
            frame.opaque_drawn().iter().all(|used| !used.lacquer),
            "{name}: lacquer is compiled out"
        );
        let hashed = render(
            &mut frame,
            material,
            at,
            procedural().union(hashed_detail()),
        );
        let general = render(&mut frame, material, at, general().union(hashed_detail()));
        frame
            .set_maps(&MapImages {
                metal: &other,
                ..MapImages::default()
            })
            .unwrap();
        let untiled = render(&mut frame, material, at, hashed_detail());
        for (what, pixels) in [
            ("hash noise forced", &hashed),
            ("the general pipeline", &general),
            ("no tile", &untiled),
        ] {
            assert!(
                shipped
                    .iter()
                    .zip(pixels)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{name}: {what} changed the frame"
            );
        }
    }
    let lacquer = render(&mut frame, lacquered(AMPLITUDE), at, OpaqueFeatures::NONE);
    let hashed = render(&mut frame, lacquered(AMPLITUDE), at, procedural());
    assert!(
        lacquer
            .iter()
            .zip(&hashed)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "without a tile the lacquer keeps its hash noise"
    );
}
