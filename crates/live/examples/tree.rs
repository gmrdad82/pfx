use std::fs::File;
use std::time::Instant;

use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_geom::{
    mesh::Mesh,
    shapes::Shape,
    tree::{Tree, TreeSpec},
};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{self, Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun};
use pfx_live::renderer::{Effects, Finish, Renderer, Text};
use pfx_live::sky::{AnalyticSky, SkySource};
use pfx_materials::{Material, NoiseKind, NoiseLayer};

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn translate(x: f32, y: f32, z: f32) -> Matrix {
    let mut matrix = identity();
    matrix[3] = [x, y, z, 1.0];
    matrix
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
    let norm = dot(v, v).sqrt();
    v.map(|value| value / norm)
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
    let far = 5000.0;
    let f = 1.0 / (0.72_f32 / 2.0).tan();
    let aspect = width as f32 / height as f32;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tree example output"),
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

fn save(renderer: &Renderer, output: &OffscreenTarget, path: &str) {
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
}

fn save_crown_crop(renderer: &Renderer, output: &OffscreenTarget, path: &str) {
    let pixels = renderer.gpu().readback_rgba16(output).unwrap();
    let side = 1024_u32;
    let left = (output.width - side) / 2;
    let top = (output.height - side) / 2;
    let mut bytes = Vec::with_capacity((side * side * 4) as usize);
    for y in top..top + side {
        for x in left..left + side {
            let offset = ((y * output.width + x) * 4) as usize;
            for value in &pixels[offset..offset + 4] {
                bytes.push(
                    (half::f16::from_bits(*value).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8,
                );
            }
        }
    }
    let file = File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

fn add_tree(combined: &mut Tree, tree: &Tree, x: f32) {
    combined.flower_count += tree.flower_count;
    let offset = combined.wood.positions.len() as u32;
    let mut positions = combined.wood.positions.clone();
    positions.extend(
        tree.wood
            .positions
            .iter()
            .map(|point| [point[0] + x, point[1], point[2]]),
    );
    let mut normals = combined.wood.normals.clone();
    normals.extend_from_slice(&tree.wood.normals);
    let mut tangents = combined.wood.tangents.clone();
    tangents.extend_from_slice(&tree.wood.tangents);
    let mut uvs = combined.wood.uvs.clone();
    uvs.extend_from_slice(&tree.wood.uvs);
    let mut indices = combined.wood.indices.clone();
    indices.extend(tree.wood.indices.iter().map(|index| index + offset));
    combined.wood = Mesh::new(positions, normals, tangents, uvs, indices);
    combined.sway.extend(tree.sway.iter().map(|sway| {
        let mut sway = *sway;
        sway.pivot[0] += x;
        sway
    }));
    combined.cards.extend(tree.cards.iter().map(|card| {
        let mut card = *card;
        card.position[0] += x;
        card.sway.pivot[0] += x;
        card
    }));
    combined
        .petal_sources
        .extend(tree.petal_sources.iter().map(|source| {
            let mut source = *source;
            source.position[0] += x;
            source.sway.pivot[0] += x;
            source
        }));
}

fn main() {
    let (width, height) = (3840, 2160);
    let cherry = Tree::new(21, TreeSpec::cherry(), 0.009);
    let plum = Tree::new(51, TreeSpec::plum(), 0.009);
    let magnolia = Tree::new(81, TreeSpec::magnolia(), 0.009);
    for (name, tree) in [
        ("cherry", &cherry),
        ("plum", &plum),
        ("magnolia", &magnolia),
    ] {
        println!(
            "{name}: triangles={} cards={} flowers={}",
            tree.wood.indices.len() / 3,
            tree.cards.len(),
            tree.flower_count
        );
    }
    let mut combined = cherry.clone();
    for point in &mut combined.wood.positions {
        point[0] -= 10.5;
    }
    combined.wood.bounds = pfx_geom::mesh::Bounds::from_points(&combined.wood.positions);
    for sway in &mut combined.sway {
        sway.pivot[0] -= 10.5;
    }
    for card in &mut combined.cards {
        card.position[0] -= 10.5;
        card.sway.pivot[0] -= 10.5;
    }
    for source in &mut combined.petal_sources {
        source.position[0] -= 10.5;
        source.sway.pivot[0] -= 10.5;
    }
    add_tree(&mut combined, &plum, 0.0);
    add_tree(&mut combined, &magnolia, 10.0);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, width, height).unwrap();
    let output = output(renderer.gpu(), width, height);
    let wood = renderer
        .set_tree(&combined, [0.0; 3], 0.0, 10, 0.0, [1.0; 3])
        .unwrap();
    let grass = Shape::RoundBox {
        half: [6000.0, 0.05, 6000.0],
        radius: 0.04,
    }
    .mesh(0.04);
    let ground = renderer
        .upload_mesh(MeshData {
            positions: &grass.positions,
            normals: &grass.normals,
            tangents: &grass.tangents,
            uvs: &grass.uvs,
            uvs1: None,
            alpha: None,
            indices: &grass.indices,
        })
        .unwrap();
    let materials = vec![
        Material {
            base: [0.19, 0.31, 0.12],
            roughness: 0.94,
            ..Default::default()
        },
        Material {
            base: [0.27, 0.16, 0.13],
            roughness: 0.91,
            layers: [
                NoiseLayer::new(NoiseKind::Bark, 1.0, 1.0, 37),
                NoiseLayer::default(),
                NoiseLayer::default(),
                NoiseLayer::default(),
            ],
            ..Default::default()
        },
    ];
    let instances = [
        Instance::new(ground, translate(0.0, -0.05, 0.0), 0, 1),
        Instance::new(wood, identity(), 1, 2),
    ];
    let eye = [0.0, 7.0, 31.0];
    let projection = projection(width, height);
    let wide_view = view(eye, [0.0, 5.2, 0.0]);
    let camera = Camera {
        view: wide_view,
        projection,
        previous_view_projection: frame::multiply(projection, wide_view),
        position: eye,
    };
    std::fs::create_dir_all("tmp").unwrap();
    let hour = 16.0;
    let daylight = Daylight {
        hour,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    };
    renderer
        .set_sky(
            SkySource::Analytic(AnalyticSky::new(
                daylight,
                REFERENCE_HOUR,
                2.6,
                [0.32, 0.3, 0.27],
            )),
            hour as f32,
        )
        .unwrap();
    let scene = Scene {
        camera,
        time: (hour / 24.0) as f32,
        seed: 91,
        sun: Sun::from_daylight(daylight, REFERENCE_HOUR),
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let start = Instant::now();
    let timings = renderer
        .render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &output.view,
        )
        .unwrap();
    println!(
        "hour={hour}: wall_ms={:.2} gpu_ms={:.2} passes={timings:?}",
        start.elapsed().as_secs_f64() * 1000.0,
        timings
            .iter()
            .map(|timing| timing.milliseconds)
            .sum::<f64>()
    );
    save(&renderer, &output, &format!("tmp/trees-{hour:.0}.png"));
    let eye = [-8.5, 7.0, 6.0];
    let close_view_16 = view(eye, [-8.5, 7.0, 0.0]);
    let camera = Camera {
        view: close_view_16,
        projection,
        previous_view_projection: frame::multiply(projection, close_view_16),
        position: eye,
    };
    let scene = Scene {
        camera,
        time: 16.0 / 24.0,
        seed: 91,
        sun: Sun::from_daylight(
            Daylight {
                hour: 16.0,
                day: Daylight::DAY,
                latitude: Daylight::LATITUDE,
                heading: Daylight::HEADING,
            },
            REFERENCE_HOUR,
        ),
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    renderer
        .render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &output.view,
        )
        .unwrap();
    save(&renderer, &output, "tmp/tree-cherry-close-16.png");
    save_crown_crop(&renderer, &output, "tmp/tree-cherry-crown-crop-16.png");
    let hour = 10.0;
    let morning = Daylight {
        hour: hour as f64,
        day: Daylight::DAY,
        latitude: Daylight::LATITUDE,
        heading: Daylight::HEADING,
    };
    renderer
        .set_sky(
            SkySource::Analytic(AnalyticSky::new(
                morning,
                REFERENCE_HOUR,
                2.6,
                [0.32, 0.3, 0.27],
            )),
            hour,
        )
        .unwrap();
    let scene = Scene {
        camera: Camera {
            view: wide_view,
            projection,
            previous_view_projection: frame::multiply(projection, wide_view),
            position: [0.0, 7.0, 31.0],
        },
        time: hour / 24.0,
        seed: 91,
        sun: Sun::from_daylight(morning, REFERENCE_HOUR),
        instances: &instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    renderer
        .render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &output.view,
        )
        .unwrap();
    save(&renderer, &output, "tmp/trees-10.png");
    let close_eye = [-8.5, 7.0, 6.0];
    let close_view = view(close_eye, [-8.5, 7.0, 0.0]);
    let scene = Scene {
        camera: Camera {
            view: close_view,
            projection,
            previous_view_projection: frame::multiply(projection, close_view),
            position: close_eye,
        },
        ..scene
    };
    renderer
        .render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &output.view,
        )
        .unwrap();
    save(&renderer, &output, "tmp/tree-cherry-close-10.png");
    let sun = Sun::from_daylight(
        Daylight {
            hour: 16.0,
            day: Daylight::DAY,
            latitude: Daylight::LATITUDE,
            heading: Daylight::HEADING,
        },
        REFERENCE_HOUR,
    );
    renderer
        .set_sky(
            SkySource::Analytic(AnalyticSky::new(
                daylight,
                REFERENCE_HOUR,
                2.6,
                [0.32, 0.3, 0.27],
            )),
            16.0,
        )
        .unwrap();
    let back_eye = [-8.5 - sun.direction[0] * 8.0, 7.0, -sun.direction[2] * 8.0];
    let back_view = view(back_eye, [-8.5, 7.0, 0.0]);
    let scene = Scene {
        camera: Camera {
            view: back_view,
            projection,
            previous_view_projection: frame::multiply(projection, back_view),
            position: back_eye,
        },
        time: 16.0 / 24.0,
        sun,
        ..scene
    };
    renderer
        .render(
            &scene,
            &Text::default(),
            &Effects::default(),
            Finish::Standard,
            &output.view,
        )
        .unwrap();
    save(&renderer, &output, "tmp/tree-cherry-backlit-16.png");
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_geom::tree::{Card, CardLod, SwayVertex};

    fn pixel(pixels: &[u16], x: usize, y: usize) -> [f32; 3] {
        let offset = (y * 256 + x) * 4;
        std::array::from_fn(|index| half::f16::from_bits(pixels[offset + index]).to_f32())
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn live_card_atlas_colour_and_backlight() {
        let mut tree = Tree::new(23, TreeSpec::cherry(), 0.02);
        tree.cards = vec![Card {
            position: [0.0; 3],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            size: [0.25, 0.25],
            color: [1.8, 1.55, 1.68, 1.0],
            atlas: [0.0, 0.0, 1.0 / 6.0, 1.0],
            sway: SwayVertex {
                pivot: [0.0; 3],
                level: 0.0,
                stiffness: 1.0,
            },
            blossom: true,
            lod: CardLod::Always,
        }];
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, 256, 256).unwrap();
        let output = output(renderer.gpu(), 256, 256);
        renderer
            .set_tree(&tree, [0.0; 3], -1.0, 1, 0.0, [1.0; 3])
            .unwrap();
        let eye = [0.0, 0.0, 2.0];
        let view = view(eye, [0.0; 3]);
        let camera = Camera {
            view,
            projection: projection(256, 256),
            previous_view_projection: frame::multiply(projection(256, 256), view),
            position: eye,
        };
        let materials = [Material::default()];
        let draw = |renderer: &mut Renderer, direction: [f32; 3], intensity: f32| {
            let scene = Scene {
                camera,
                time: 0.0,
                seed: 1,
                sun: Sun {
                    direction,
                    colour: [1.0; 3],
                    intensity,
                },
                instances: &[],
                materials: &materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            renderer
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &output.view,
                )
                .unwrap();
            renderer.gpu().readback_rgba16(&output).unwrap()
        };
        let front = draw(&mut renderer, [0.0, 0.0, 1.0], 3.0);
        let center = pixel(&front, 128, 128);
        let tip = pixel(&front, 128, 169);
        assert!(
            center[0] - center[1] > tip[0] - tip[1],
            "center={center:?} tip={tip:?}"
        );
        let dark = draw(&mut renderer, [0.0, 0.0, -1.0], 0.0);
        let back = draw(&mut renderer, [0.0, 0.0, -1.0], 3.0);
        let luminance = |pixel: [f32; 3]| pixel[0] * 0.2126 + pixel[1] * 0.7152 + pixel[2] * 0.0722;
        let unlit = luminance(pixel(&dark, 128, 128));
        let transmitted = luminance(pixel(&back, 128, 128));
        assert!(
            transmitted > unlit + 0.01,
            "unlit={unlit} transmitted={transmitted}"
        );
    }
}
