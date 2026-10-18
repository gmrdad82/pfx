use std::path::{Path, PathBuf};

use pfx_bench::shot::{Quality, Shot, play};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::Renderer;
use pfx_load::scene::Scene;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const FPS: u32 = 2;
const FRAMES: u64 = 3;
const SEED: u64 = 7;

const SCENE: &str = r#"format = 1

[mesh.block]
file = "block.gltf"

[[object]]
name = "block"
mesh = "block"

[[object]]
name = "floor"
mesh = "block"
at = [0.0, -0.75, 0.0]
scale = [6.0, 0.5, 6.0]

[[light]]
name = "lamp"
position = [1.2, 1.4, 1.6]
intensity = 2.0
range = 6.0
keys = { intensity = [[0.0, 1.0], [1.0, 6.0]], color = [[0.0, [1.0, 0.2, 0.1]], [1.0, [0.2, 0.4, 1.0]]] }

[sun]
model = "daylight"
hour = 9.0
keys = { hour = [[0.0, 8.0], [1.0, 15.0]] }

[sky]
kind = "analytic"

[camera]
at = [0.0, 1.2, 3.5]
look_at = [0.0, 0.0, 0.0]
fov = 45.0
"#;

fn scene() -> Scene {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("keyed-shot");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(root.join("keyed.scene.toml"), SCENE).unwrap();
    Scene::open(root.join("keyed.scene.toml")).unwrap()
}

fn target(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("keyed shot output"),
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

fn shot(scene: &Scene) -> Vec<Vec<u16>> {
    let mut frames = Vec::new();
    play(
        &Shot {
            scene,
            size: (WIDTH, HEIGHT),
            fps: FPS,
            from: 0,
            frames: FRAMES,
            camera: None,
            quality: Quality::Final,
            seed: SEED,
            shutter: None,
            fixed_exposure: false,
        },
        |frame| {
            frames.push(frame.pixels);
            Ok(())
        },
    )
    .unwrap();
    frames
}

fn staged(scene: &Scene) -> Vec<Vec<u16>> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = target(&gpu);
    let mut renderer = Renderer::new(gpu.clone(), WIDTH, HEIGHT).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.wait_sky().unwrap();
    renderer.resize(WIDTH, HEIGHT).unwrap();
    let finish = staged.finish();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    (0..FRAMES)
        .map(|number| {
            let time = (number as f64 / f64::from(FPS)) as f32;
            let drawn = staged.frame(aspect, time, SEED as u32);
            renderer.tune(drawn.effects.tuned.unwrap()).unwrap();
            if renderer.sky_settling() {
                renderer.wait_sky().unwrap();
            }
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            gpu.readback_rgba16(&output).unwrap()
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn keyed_light_and_sun_hour_draw_the_same_frames_in_a_shot_and_staged() {
    let scene = scene();
    assert!(!scene.keys.lights.is_empty() && scene.keys.sun.is_some());
    let shot = shot(&scene);
    let staged = staged(&scene);
    assert_eq!(shot.len(), FRAMES as usize);
    assert_ne!(shot[0], shot[2], "the keys change the frame over time");
    for (number, (shot, staged)) in shot.iter().zip(&staged).enumerate() {
        let differ = shot.iter().zip(staged).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "frame {number}: {differ} values differ");
    }
}
