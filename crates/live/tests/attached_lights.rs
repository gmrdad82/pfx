use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::frame::{multiply, transform};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_post::Chain;

const WIDTH: u32 = 192;
const HEIGHT: u32 = 192;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;

const SCENE: &str = "materials = [\"lamp.materials.toml\"]
fallback = \"matte\"

[mesh.block]
file = \"block.gltf\"

[[object]]
name = \"floor\"
mesh = \"block\"
at = [0.0, -0.05, 0.0]
scale = [8.0, 0.1, 8.0]

[[object]]
name = \"car\"
mesh = \"block\"
at = [-1.0, 0.8, 0.0]
scale = 0.05

[[mover]]
name = \"drive\"
objects = [\"car\"]
kind = \"slide\"
axis = [1.0, 0.0, 0.0]
travel = [0.0, 2.0]
period = 2.0

[[light]]
name = \"head\"
kind = \"spot\"
parent = \"car\"
at = [0.0, -2.0, 0.0]
direction = [0.0, -1.0, 0.0]
cone = [15.0, 25.0]
intensity = 3.0
range = 4.0
shadow = false

[camera]
at = [0.0, 6.0, 0.01]
look_at = [0.0, 0.0, 0.0]
fov = 60.0
";

fn folder() -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("attached-lights");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes/block.gltf"),
        root.join("block.gltf"),
    )
    .unwrap();
    std::fs::write(
        root.join("lamp.materials.toml"),
        "[materials.matte]\nbase = [0.7, 0.7, 0.7]\nroughness = 1.0\nspecular = 0.0\n",
    )
    .unwrap();
    std::fs::write(root.join("lamp.scene.toml"), SCENE).unwrap();
    root
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("attached lights output"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
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
        width: WIDTH,
        height: HEIGHT,
    }
}

fn centroid(pixels: &[f32]) -> [f32; 2] {
    let peak = pixels.iter().fold(0.0_f32, |most, &value| most.max(value));
    let mut sum = [0.0f32; 3];
    for (index, &value) in pixels.iter().enumerate() {
        if value > peak * 0.5 {
            sum[0] += (index % WIDTH as usize) as f32 + 0.5;
            sum[1] += (index / WIDTH as usize) as f32 + 0.5;
            sum[2] += 1.0;
        }
    }
    assert!(sum[2] > 20.0, "{} lit pixels", sum[2]);
    [sum[0] / sum[2], sum[1] / sum[2]]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_light_on_a_moving_object_moves_with_it() {
    let scene = Scene::open(folder().join("lamp.scene.toml")).unwrap();
    let camera = scene.camera_or_default();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(&scene).unwrap();
    renderer.set_finish(Chain::new());
    let view_projection = multiply(camera.projection(ASPECT), camera.view());
    let mut shot = |time: f32| {
        for _ in 0..4 {
            let finish = staged.finish();
            let drawn = staged.frame(ASPECT, time, 7);
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
        }
        let pixels: Vec<f32> = renderer
            .gpu()
            .readback_rgba16(&output)
            .unwrap()
            .chunks_exact(4)
            .map(|texel| half::f16::from_bits(texel[1]).to_f32())
            .collect();
        let light = scene.lights["head"].placed(&scene.posed(&camera, time));
        let floor = [light.position[0], 0.0, light.position[2], 1.0];
        let clip = transform(view_projection, floor);
        let expected = [
            (clip[0] / clip[3] * 0.5 + 0.5) * WIDTH as f32,
            (0.5 - clip[1] / clip[3] * 0.5) * HEIGHT as f32,
        ];
        (centroid(&pixels), expected, light.position)
    };
    let (rest, rest_expected, rest_light) = shot(0.0);
    let (moved, moved_expected, moved_light) = shot(1.0);
    println!(
        "light {rest_light:?} -> {moved_light:?}; spot {rest:?} (expected {rest_expected:?}) -> {moved:?} (expected {moved_expected:?})"
    );
    assert!(
        (moved_light[0] - rest_light[0]).abs() > 0.5,
        "the mover moves the light"
    );
    for (seen, expected) in [(rest, rest_expected), (moved, moved_expected)] {
        assert!(
            (seen[0] - expected[0]).abs() < 2.0 && (seen[1] - expected[1]).abs() < 2.0,
            "{seen:?} against {expected:?}"
        );
    }
}
