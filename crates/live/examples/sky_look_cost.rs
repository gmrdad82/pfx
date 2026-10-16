use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;

const WIDTH: u32 = 2160;
const HEIGHT: u32 = 3840;
const WARM: u32 = 30;
const FRAMES: u32 = 120;

fn folder(name: &str, sky: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/sky-look-cost")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("sky.scene.toml"),
        format!(
            "format = 1\n\n[camera]\nat = [0.0, 1.0, 0.0]\nlook_at = [0.0, 1.2, -1.0]\nfov = 60.0\n\n[sun]\nmodel = \"daylight\"\nhour = 18.0\n\n[sky]\nkind = \"analytic\"\n{sky}"
        ),
    )
    .unwrap();
    root.join("sky.scene.toml")
}

fn target(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky look cost"),
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
    OffscreenTarget {
        view: texture.create_view(&Default::default()),
        texture,
        format: wgpu::TextureFormat::Rgba16Float,
        width: WIDTH,
        height: HEIGHT,
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn live(scene: &Scene) -> (f64, f64) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = target(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.wait_sky().unwrap();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut sky = Vec::new();
    let mut total = Vec::new();
    for frame in 0..WARM + FRAMES {
        let finish = staged.finish();
        let drawn = staged.frame(aspect, 0.0, 7);
        let passes = renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &output.view,
            )
            .unwrap();
        if frame >= WARM {
            sky.push(
                passes
                    .iter()
                    .filter(|pass| pass.label == "sky")
                    .map(|pass| pass.milliseconds)
                    .sum(),
            );
            total.push(pfx_gpu::pace::gpu_ms(&passes).unwrap_or(f64::NAN));
        }
    }
    staged.release(&mut renderer).unwrap();
    (median(sky), median(total))
}

fn main() {
    for (name, sky) in [
        ("plain", ""),
        ("backdrop", "backdrop = 2.0\n"),
        (
            "horizon",
            "backdrop = 2.0\nhorizon_color = [1.0, 0.6, 0.8]\nhorizon_width = 12.0\nhorizon_amount = 0.6\n",
        ),
    ] {
        let scene = Scene::open(folder(name, sky)).unwrap();
        let (sky, total) = live(&scene);
        println!("{{\"case\":\"{name}\",\"sky_pass_ms\":{sky:.4},\"live_frame_ms\":{total:.4}}}");
    }
}
