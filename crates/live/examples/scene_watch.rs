mod manners;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::{Reload, SceneError, SceneWatch};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
const FRAMES: u32 = 8;

struct Step {
    name: &'static str,
    file: &'static str,
    from: &'static str,
    to: &'static str,
}

const STEPS: [Step; 5] = [
    Step {
        name: "material-colour",
        file: "materials.toml",
        from: "base = [0.7, 0.32, 0.22]",
        to: "base = [0.2, 0.6, 0.3]",
    },
    Step {
        name: "light-power",
        file: "lights.scene.toml",
        from: "intensity = 6.0",
        to: "intensity = 18.0",
    },
    Step {
        name: "mesh-swap",
        file: "room.scene.toml",
        from: "file = \"block.gltf\"",
        to: "file = \"ball.gltf\"",
    },
    Step {
        name: "broken",
        file: "lights.scene.toml",
        from: "range = 8.0",
        to: "range = = 8.0",
    },
    Step {
        name: "fixed",
        file: "lights.scene.toml",
        from: "range = = 8.0",
        to: "range = 8.0",
    },
];

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene watch output"),
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

fn save(renderer: &Renderer, output: &OffscreenTarget, path: &Path) {
    let bytes: Vec<u8> = renderer
        .gpu()
        .readback_rgba16(output)
        .unwrap()
        .into_iter()
        .map(|value| (half::f16::from_bits(value).to_f32().clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    let mut encoder = png::Encoder::new(File::create(path).unwrap(), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
    println!("wrote {}", path.display());
}

fn copy_scene(into: &Path) {
    let _ = std::fs::remove_dir_all(into);
    std::fs::create_dir_all(into).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), into.join(entry.file_name())).unwrap();
    }
}

fn edit(folder: &Path, step: &Step) {
    let path = folder.join(step.file);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replacen(step.from, step.to, 1)).unwrap();
}

fn heard(watch: &mut SceneWatch) -> Option<Result<Reload, SceneError>> {
    for _ in 0..300 {
        if let Some(result) = watch.poll() {
            return Some(result);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    watch.check()
}

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/scene_watch");
    let folder: PathBuf = out.join("scene");
    copy_scene(&folder);
    let mut watch = SceneWatch::open(folder.join("room.scene.toml")).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged: Staged = renderer.stage(watch.scene()).unwrap();
    let mut pace = manners::Pace::timed("scene_watch", "frames", u64::from(FRAMES) * 6);
    let mut time = 0.0;
    let mut still = |renderer: &mut Renderer, staged: &mut Staged, name: &str, index: usize| {
        for _ in 0..FRAMES {
            let finish = staged.finish();
            let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, time, 1);
            pace.frame(|| {
                renderer
                    .render(
                        &drawn.scene,
                        &drawn.text,
                        &drawn.effects,
                        finish,
                        &output.view,
                    )
                    .unwrap()
            });
            time += 1.0 / 60.0;
        }
        time += 1.0;
        save(
            renderer,
            &output,
            &out.join(format!("{index:02}-{name}.png")),
        );
    };
    still(&mut renderer, &mut staged, "loaded", 0);
    for (index, step) in STEPS.iter().enumerate() {
        edit(&folder, step);
        match heard(&mut watch) {
            Some(Ok(reload)) => {
                let applied = staged
                    .apply(&mut renderer, &reload.scene, &reload.diff)
                    .unwrap();
                println!(
                    "{}: {:?}{}",
                    step.name,
                    applied,
                    if reload.recovered { ", recovered" } else { "" }
                );
            }
            Some(Err(error)) => println!("{}: kept the last good scene: {error}", step.name),
            None => println!("{}: nothing changed", step.name),
        }
        still(&mut renderer, &mut staged, step.name, index + 1);
    }
}
