mod manners;

use std::f32::consts::{PI, TAU};
use std::fs::File;
use std::path::Path;

use pfx_bake::{Anchor, BakeScene, GridSpec, bake, write_artifact};
use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_geom::{mesh::Mesh, shapes::Shape};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{
    self, Camera, Instance, Matrix, MeshData, MeshHandle, Scene, SceneWind, Sun,
};
use pfx_live::probes::ProbeLighting;
use pfx_live::renderer::{Effects, Finish, Renderer, Text};
use pfx_live::sky::{AnalyticSky, SkySource};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_post::Style;
use pfx_trace::bvh::Triangle;

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn translate(x: f32, y: f32, z: f32) -> Matrix {
    let mut model = identity();
    model[3] = [x, y, z, 1.0];
    model
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

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

fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = dot(v, v).sqrt();
    v.map(|x| x / n)
}

fn view(eye: [f32; 3], target: [f32; 3]) -> Matrix {
    let forward = unit(sub(target, eye));
    let right = unit(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ]
}

fn projection(width: u32, height: u32) -> Matrix {
    let near = 0.05;
    let far = 80.0;
    let f = 1.0 / (0.72_f32 / 2.0).tan();
    let aspect = width as f32 / height as f32;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn upload(renderer: &mut Renderer, mesh: &Mesh) -> MeshHandle {
    renderer
        .upload_mesh(MeshData {
            positions: &mesh.positions,
            normals: &mesh.normals,
            tangents: &mesh.tangents,
            uvs: &mesh.uvs,
            uvs1: None,
            alpha: None,
            indices: &mesh.indices,
        })
        .unwrap()
}

fn equirect(model: &AnalyticSky) -> Sky {
    let (width, height) = (32, 16);
    let mut texels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let theta = (y as f32 + 0.5) / height as f32 * PI;
            let phi = ((x as f32 + 0.5) / width as f32 - 0.5) * TAU;
            let direction = [
                theta.sin() * phi.sin(),
                theta.cos(),
                -theta.sin() * phi.cos(),
            ];
            let colour = model.diffuse(direction);
            texels.push([colour[0], colour[1], colour[2], 1.0]);
        }
    }
    Sky {
        width,
        height,
        texels,
    }
}

fn triangles(mesh: &Mesh, model: Matrix, material: u32) -> Vec<Triangle> {
    mesh.indices
        .chunks_exact(3)
        .map(|face| {
            let vertices = std::array::from_fn(|corner| {
                let index = face[corner];
                let position = mesh.positions[index as usize];
                let point = frame::transform(model, [position[0], position[1], position[2], 1.0]);
                [point[0], point[1], point[2]]
            });
            Triangle { vertices, material }
        })
        .collect()
}

fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("style look output"),
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
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format: wgpu::TextureFormat::Rgba16Float,
        width,
        height,
    }
}

fn save(renderer: &Renderer, output: &OffscreenTarget, path: &str) -> Vec<u8> {
    let pixels = renderer.gpu().readback_rgba16(output).unwrap();
    let bytes: Vec<u8> = pixels
        .into_iter()
        .map(|value| (half::f16::from_bits(value).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    let file = File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, output.width, output.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
    bytes
}

fn label(sheet: &mut [u8], width: usize, x: usize, y: usize, name: &str) {
    for (letter, ch) in name.chars().enumerate() {
        let rows: [u8; 7] = match ch {
            'A' => [14, 17, 17, 31, 17, 17, 17],
            'B' => [30, 17, 17, 30, 17, 17, 30],
            'C' => [15, 16, 16, 16, 16, 16, 15],
            'D' => [30, 17, 17, 17, 17, 17, 30],
            'E' => [31, 16, 16, 30, 16, 16, 31],
            'H' => [17, 17, 17, 31, 17, 17, 17],
            'I' => [31, 4, 4, 4, 4, 4, 31],
            'L' => [16, 16, 16, 16, 16, 16, 31],
            'M' => [17, 27, 21, 21, 17, 17, 17],
            'N' => [17, 25, 21, 19, 17, 17, 17],
            'O' => [14, 17, 17, 17, 17, 17, 14],
            'R' => [30, 17, 17, 30, 20, 18, 17],
            'S' => [15, 16, 16, 14, 1, 1, 30],
            'U' => [17, 17, 17, 17, 17, 17, 14],
            _ => [0; 7],
        };
        for (row, bits) in rows.into_iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    for yy in 0..2 {
                        for xx in 0..2 {
                            let index =
                                ((y + row * 2 + yy) * width + x + letter * 14 + column * 2 + xx)
                                    * 4;
                            sheet[index..index + 4].copy_from_slice(&[245, 239, 220, 255]);
                        }
                    }
                }
            }
        }
    }
}

fn sheet(images: &[Vec<u8>], width: usize, height: usize) {
    let sheet_width = width * 2;
    let sheet_height = (height + 28) * 2;
    let mut pixels = vec![255_u8; sheet_width * sheet_height * 4];
    for (index, (image, title)) in images
        .iter()
        .zip(["CEL", "RUBBER HOSE", "COMIC", "MODERN COMIC"])
        .enumerate()
    {
        let x = index % 2 * width;
        let y = index / 2 * (height + 28);
        for row in 0..height {
            let source = row * width * 4;
            let target = ((y + row + 28) * sheet_width + x) * 4;
            pixels[target..target + width * 4].copy_from_slice(&image[source..source + width * 4]);
        }
        for row in y..y + 28 {
            let start = (row * sheet_width + x) * 4;
            for pixel in pixels[start..start + width * 4].chunks_exact_mut(4) {
                pixel.copy_from_slice(&[26, 25, 29, 255]);
            }
        }
        label(&mut pixels, sheet_width, x + 16, y + 7, title);
    }
    let file = File::create("tmp/looks/sheet.png").unwrap();
    let mut encoder = png::Encoder::new(file, sheet_width as u32, sheet_height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

fn main() {
    let (width, height) = (1920, 1080);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, width, height).unwrap();
    let output = output(renderer.gpu(), width, height);
    let ground_mesh = Shape::RoundBox {
        half: [20.0, 0.08, 20.0],
        radius: 0.06,
    }
    .mesh(0.04);
    let ground = upload(&mut renderer, &ground_mesh);
    let torso = upload(
        &mut renderer,
        &Shape::RoundBox {
            half: [0.52, 0.66, 0.31],
            radius: 0.20,
        }
        .mesh(0.018),
    );
    let head = upload(
        &mut renderer,
        &Shape::Ellipsoid {
            radius: 0.43,
            squash: 0.85,
        }
        .mesh(0.018),
    );
    let ball = upload(
        &mut renderer,
        &Shape::Ellipsoid {
            radius: 0.43,
            squash: 1.0,
        }
        .mesh(0.018),
    );
    let box_mesh = upload(
        &mut renderer,
        &Shape::RoundBox {
            half: [0.38, 0.38, 0.38],
            radius: 0.12,
        }
        .mesh(0.018),
    );
    let eyes = upload(
        &mut renderer,
        &Shape::Ellipsoid {
            radius: 0.047,
            squash: 0.7,
        }
        .mesh(0.012),
    );
    let nose = upload(
        &mut renderer,
        &Shape::Ellipsoid {
            radius: 0.083,
            squash: 0.75,
        }
        .mesh(0.012),
    );
    let hair = upload(
        &mut renderer,
        &Shape::RoundBox {
            half: [0.43, 0.13, 0.28],
            radius: 0.1,
        }
        .mesh(0.012),
    );
    let left_arm = upload(
        &mut renderer,
        &Shape::Capsule {
            a: [0.0, 0.0, 0.0],
            b: [-0.85, -0.55, 0.1],
            radius: 0.18,
        }
        .mesh(0.018),
    );
    let right_arm = upload(
        &mut renderer,
        &Shape::Capsule {
            a: [0.0, 0.0, 0.0],
            b: [0.75, -0.22, 0.3],
            radius: 0.18,
        }
        .mesh(0.018),
    );
    let left_leg = upload(
        &mut renderer,
        &Shape::Capsule {
            a: [0.0, 0.0, 0.0],
            b: [-0.12, -0.95, 0.1],
            radius: 0.21,
        }
        .mesh(0.018),
    );
    let right_leg = upload(
        &mut renderer,
        &Shape::Capsule {
            a: [0.0, 0.0, 0.0],
            b: [0.12, -0.95, 0.1],
            radius: 0.21,
        }
        .mesh(0.018),
    );
    let materials = vec![
        Material {
            base: [0.30, 0.38, 0.48],
            roughness: 0.85,
            ..Default::default()
        },
        Material {
            base: [0.12, 0.68, 0.26],
            roughness: 0.48,
            ..Default::default()
        },
        Material {
            base: [1.0, 0.58, 0.33],
            roughness: 0.62,
            ..Default::default()
        },
        Material {
            base: [0.91, 0.11, 0.10],
            roughness: 0.34,
            ..Default::default()
        },
        Material {
            base: [0.98, 0.75, 0.08],
            roughness: 0.38,
            ..Default::default()
        },
        Material {
            base: [0.06, 0.75, 0.38],
            roughness: 0.46,
            ..Default::default()
        },
        Material {
            base: [0.90, 0.92, 0.96],
            roughness: 0.08,
            metalness: 0.75,
            ..Default::default()
        },
        Material {
            base: [0.24, 0.19, 0.25],
            roughness: 0.95,
            ..Default::default()
        },
        Material {
            base: [0.08, 0.42, 0.12],
            roughness: 0.8,
            ..Default::default()
        },
        Material {
            base: [0.025, 0.02, 0.03],
            roughness: 0.8,
            ..Default::default()
        },
    ];
    let mut instances = Vec::new();
    let mut add = |mesh, model, material| {
        let id = instances.len() as u32 + 1;
        instances.push(Instance::new(mesh, model, material, id));
    };
    add(ground, translate(0.0, -0.08, 0.0), 0);
    add(torso, translate(0.0, 1.35, 0.0), 1);
    add(head, translate(0.0, 2.35, 0.0), 2);
    add(hair, translate(0.0, 2.71, -0.03), 8);
    add(eyes, translate(-0.15, 2.42, 0.35), 9);
    add(eyes, translate(0.15, 2.42, 0.35), 9);
    add(nose, translate(0.0, 2.30, 0.38), 2);
    add(left_arm, translate(-0.48, 1.74, 0.0), 3);
    add(right_arm, translate(0.48, 1.74, 0.0), 3);
    add(left_leg, translate(-0.27, 0.95, 0.0), 1);
    add(right_leg, translate(0.27, 0.95, 0.0), 1);
    add(ball, translate(-2.0, 0.43, -0.3), 4);
    add(ball, translate(2.0, 0.43, -0.3), 6);
    add(box_mesh, translate(-2.0, 0.38, -1.4), 5);
    add(box_mesh, translate(2.0, 0.38, -1.4), 3);
    let eye = [3.2, 2.7, 6.7];
    let projection = projection(width, height);
    let view = view(eye, [0.0, 1.15, 0.0]);
    let camera = Camera {
        view,
        projection,
        previous_view_projection: frame::multiply(projection, view),
        position: eye,
    };
    let daylight = Daylight {
        hour: 16.0,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    };
    let sky = AnalyticSky::new(daylight, REFERENCE_HOUR, 2.6, [0.32, 0.3, 0.27]);
    std::fs::create_dir_all("tmp/looks").unwrap();
    let bake_scene = BakeScene {
        triangles: triangles(&ground_mesh, translate(0.0, -0.08, 0.0), 0),
        shapes: Vec::new(),
        materials: materials.clone(),
        anchors: vec![Anchor {
            hour: 16.0,
            sky: equirect(&sky),
            sun: pfx_trace::Sun {
                direction: sky.sun,
                color: sky.sun_colour,
                intensity: sky.sun_intensity,
            },
        }],
    };
    let grids = bake(
        renderer.gpu(),
        &bake_scene,
        GridSpec {
            min: [-4.0, 0.0, -4.0],
            max: [4.0, 4.0, 4.0],
            spacing: 2.0,
        },
        4,
        23,
    )
    .unwrap();
    if Path::new("tmp/looks/probes").exists() {
        std::fs::remove_dir_all("tmp/looks/probes").unwrap();
    }
    write_artifact(
        Path::new("tmp"),
        Path::new("looks/probes"),
        &bake_scene,
        &grids,
        4,
        23,
    )
    .unwrap();
    let probes = ProbeLighting::load(
        &renderer.gpu().device,
        &renderer.gpu().queue,
        Path::new("tmp/looks/probes"),
        16.0,
        [-1.0; 3],
    )
    .unwrap();
    renderer.set_probes(probes).unwrap();
    renderer.set_sky(SkySource::Analytic(sky), 16.0).unwrap();
    let mut scene = Scene {
        camera,
        time: 0.0,
        seed: 23,
        sun: Sun::from_daylight(daylight, REFERENCE_HOUR),
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let mut pace = manners::Pace::timed("looks", "frames", 60);
    for frame in 0..60 {
        scene.time = frame as f32 / 60.0;
        pace.frame(|| {
            renderer
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &output.view,
                )
                .unwrap()
        });
    }
    let geometry = renderer.readback_geometry().unwrap();
    let covered = geometry
        .depth
        .iter()
        .filter(|value| **value < 0.999)
        .count();
    let oriented = geometry
        .normals
        .iter()
        .filter(|value| value.iter().map(|v| v * v).sum::<f32>() > 0.5)
        .count();
    let mean_hdr = geometry
        .colour
        .iter()
        .map(|pixel| pixel[0] * 0.2126 + pixel[1] * 0.7152 + pixel[2] * 0.0722)
        .sum::<f32>()
        / geometry.colour.len() as f32;
    println!("geometry: {covered} depth pixels, {oriented} normal pixels, HDR mean {mean_hdr:.3}");
    let mut images = Vec::new();
    for style in [
        Style::Cel,
        Style::RubberHose,
        Style::Comic,
        Style::ModernComic,
    ] {
        let timings = renderer
            .render(
                &scene,
                &Text::default(),
                &Effects::default(),
                Finish::Style(style),
                &output.view,
            )
            .unwrap();
        println!(
            "{}: {:?}",
            style.name(),
            timings
                .iter()
                .filter(|t| t.label.contains("post"))
                .collect::<Vec<_>>()
        );
        images.push(save(
            &renderer,
            &output,
            &format!("tmp/looks/{}.png", style.name()),
        ));
    }
    sheet(&images, width as usize, height as usize);
    renderer
        .render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &output.view,
        )
        .unwrap();
    save(&renderer, &output, "tmp/looks/source.png");
}
