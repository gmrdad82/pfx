mod manners;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::renderer::{Effects, Exposure, Finish, Renderer, Text};
use pfx_live::screen_shadows::{look_quality, look_screen};
use pfx_load::scene::{ShadowQuality, Shadows};
use pfx_materials::Material;

const SIZES: [(u32, u32); 2] = [(1920, 1080), (3840, 2160)];
const ROUNDS: usize = 5;
const WARM: u32 = 20;
const FRAMES: u32 = 60;
const EYE: [f32; 3] = [0.0, 4.0, 14.0];
const TARGET: [f32; 3] = [0.0, 0.5, 0.0];
const FOV_Y: f32 = 50.0;
const GRID: i32 = 6;
const SPACING: f32 = 2.5;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    [a[0] / length, a[1] / length, a[2] / length]
}

fn view() -> Matrix {
    let forward = unit(sub(TARGET, EYE));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, EYE), -dot(up, EYE), dot(forward, EYE), 1.0],
    ]
}

fn projection(aspect: f32) -> Matrix {
    let near = 0.1;
    let far = 120.0;
    let f = 1.0 / (FOV_Y.to_radians() * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn translated(x: f32, z: f32, height: f32) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, height, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [x, 0.0, z, 1.0],
    ]
}

fn mesh(
    renderer: &mut Renderer,
    faces: &[([[f32; 3]; 4], [f32; 3])],
) -> pfx_live::frame::MeshHandle {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for (corners, normal) in faces {
        let base = positions.len() as u32;
        positions.extend_from_slice(corners);
        normals.extend_from_slice(&[*normal; 4]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let count = positions.len();
    renderer
        .upload_mesh(MeshData {
            positions: &positions,
            normals: &normals,
            tangents: &vec![[1.0, 0.0, 0.0, 1.0]; count],
            uvs: &vec![[0.0, 0.0]; count],
            uvs1: None,
            alpha: None,
            indices: &indices,
        })
        .unwrap()
}

fn instances(renderer: &mut Renderer) -> Vec<Instance> {
    let h = 40.0;
    let ground = mesh(
        renderer,
        &[(
            [[-h, 0.0, -h], [-h, 0.0, h], [h, 0.0, h], [h, 0.0, -h]],
            [0.0, 1.0, 0.0],
        )],
    );
    let (lo, hi) = (-0.5, 0.5);
    let cube = mesh(
        renderer,
        &[
            (
                [[lo, 1.0, lo], [lo, 1.0, hi], [hi, 1.0, hi], [hi, 1.0, lo]],
                [0.0, 1.0, 0.0],
            ),
            (
                [[lo, 0.0, hi], [hi, 0.0, hi], [hi, 1.0, hi], [lo, 1.0, hi]],
                [0.0, 0.0, 1.0],
            ),
            (
                [[hi, 0.0, lo], [lo, 0.0, lo], [lo, 1.0, lo], [hi, 1.0, lo]],
                [0.0, 0.0, -1.0],
            ),
            (
                [[hi, 0.0, hi], [hi, 0.0, lo], [hi, 1.0, lo], [hi, 1.0, hi]],
                [1.0, 0.0, 0.0],
            ),
            (
                [[lo, 0.0, lo], [lo, 0.0, hi], [lo, 1.0, hi], [lo, 1.0, lo]],
                [-1.0, 0.0, 0.0],
            ),
        ],
    );
    let mut placed = vec![Instance::new(ground, translated(0.0, 0.0, 1.0), 0, 1)];
    for row in -GRID..GRID {
        for column in -GRID..GRID {
            let id = placed.len() as u32 + 1;
            let height = 1.0 + ((row * 7 + column * 3).rem_euclid(5)) as f32 * 0.4;
            placed.push(Instance::new(
                cube,
                translated(column as f32 * SPACING, row as f32 * SPACING, height),
                0,
                id,
            ));
        }
    }
    for instance in &mut placed {
        instance.two_sided = true;
    }
    placed
}

fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screen shadow cost output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    OffscreenTarget {
        view: texture.create_view(&Default::default()),
        texture,
        format: wgpu::TextureFormat::Rgba16Float,
        width,
        height,
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn settings() -> Vec<(&'static str, Shadows)> {
    let preset = |quality| Shadows {
        quality: Some(quality),
        ..Shadows::default()
    };
    vec![
        ("medium, today's default", Shadows::default()),
        (
            "medium with occlusion",
            Shadows {
                occlusion: Some(true),
                ..Shadows::default()
            },
        ),
        (
            "high, occlusion only",
            Shadows {
                contact: Some(false),
                ..preset(ShadowQuality::High)
            },
        ),
        (
            "high, contact only",
            Shadows {
                occlusion: Some(false),
                ..preset(ShadowQuality::High)
            },
        ),
        ("high", preset(ShadowQuality::High)),
        ("ultra", preset(ShadowQuality::Ultra)),
        ("low", preset(ShadowQuality::Low)),
    ]
}

fn run(gpu: &Gpu, width: u32, height: u32) -> Vec<(String, f64, f64, f64)> {
    let target = output(gpu, width, height);
    let mut renderer = Renderer::new(gpu.clone(), width, height).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let placed = instances(&mut renderer);
    let materials = [Material {
        base: [0.6, 0.6, 0.6],
        roughness: 0.8,
        ..Material::default()
    }];
    let elevation = 15.0_f32.to_radians();
    let heading = 35.0_f32.to_radians();
    let sun = Sun {
        direction: [
            elevation.cos() * heading.cos(),
            elevation.sin(),
            elevation.cos() * heading.sin(),
        ],
        colour: [1.0; 3],
        intensity: 3.0,
    };
    let camera = Camera {
        view: view(),
        projection: projection(width as f32 / height as f32),
        previous_view_projection: multiply(projection(width as f32 / height as f32), view()),
        position: EYE,
    };
    let base = renderer.shadows().quality();
    let rows = settings();
    let mut frame_ms = vec![Vec::new(); rows.len()];
    let mut screen_ms = vec![Vec::new(); rows.len()];
    let mut opaque_ms = vec![Vec::new(); rows.len()];
    let planned = (ROUNDS * rows.len()) as u64 * u64::from(WARM + FRAMES);
    let mut pace = manners::Pace::timed("screen_cost", &format!("{width}x{height}"), planned);
    let mut number = 0u32;
    for round in 0..ROUNDS {
        for (row, (_, look)) in rows.iter().enumerate() {
            let quality = look_quality(look, base);
            if quality != renderer.shadows().quality() {
                renderer.set_shadow_quality(quality);
            }
            renderer.set_screen_shadows(look_screen(look)).unwrap();
            let mut frames = Vec::new();
            let mut screens = Vec::new();
            let mut opaques = Vec::new();
            for frame in 0..WARM + FRAMES {
                number += 1;
                let scene = Scene {
                    camera,
                    time: 0.0,
                    seed: number,
                    sun,
                    instances: &placed,
                    materials: &materials,
                    deformers: &[],
                    wind: SceneWind::default(),
                };
                let passes = pace.frame(|| {
                    renderer
                        .render(
                            &scene,
                            &Text::default(),
                            &Effects::default(),
                            Finish::Standard,
                            &target.view,
                        )
                        .unwrap()
                });
                if frame < WARM {
                    continue;
                }
                let sum = |prefix: &str| -> f64 {
                    passes
                        .iter()
                        .filter(|pass| pass.label.starts_with(prefix))
                        .map(|pass| pass.milliseconds)
                        .sum()
                };
                frames.push(pfx_gpu::pace::gpu_ms(&passes).unwrap_or(f64::NAN));
                screens.push(sum("screen shadow"));
                opaques.push(sum("opaque"));
            }
            if round == 0 && width == SIZES[0].0 {
                save(gpu, &target, row);
            }
            frame_ms[row].push(median(frames));
            screen_ms[row].push(median(screens));
            opaque_ms[row].push(median(opaques));
        }
    }
    rows.iter()
        .enumerate()
        .map(|(row, (name, _))| {
            (
                name.to_string(),
                median(frame_ms[row].clone()),
                median(screen_ms[row].clone()),
                median(opaque_ms[row].clone()),
            )
        })
        .collect()
}

fn save(gpu: &Gpu, target: &OffscreenTarget, row: usize) {
    let folder = std::path::Path::new("tmp/screen-cost");
    std::fs::create_dir_all(folder).unwrap();
    let rgba: Vec<u8> = gpu
        .readback_rgba16(target)
        .unwrap()
        .chunks_exact(4)
        .flat_map(|texel| {
            let channel = |bits: u16| {
                let value = half::f16::from_bits(bits).to_f32().clamp(0.0, 1.0);
                (value.powf(1.0 / 2.2) * 255.0 + 0.5) as u8
            };
            [channel(texel[0]), channel(texel[1]), channel(texel[2]), 255]
        })
        .collect();
    let file = std::fs::File::create(folder.join(format!("{row}.png"))).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), target.width, target.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&rgba)
        .unwrap();
}

fn main() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    println!("{}", gpu.adapter.get_info().name);
    for (width, height) in SIZES {
        for (name, frame, screen, opaque) in run(&gpu, width, height) {
            println!(
                "{{\"size\":\"{width}x{height}\",\"setting\":\"{name}\",\"frame_ms\":{frame:.3},\"screen_ms\":{screen:.3},\"opaque_ms\":{opaque:.3}}}"
            );
        }
    }
}
