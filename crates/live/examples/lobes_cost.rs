use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::{ball, ball_png};
use pfx_load::scene::Scene;
use serde_json::json;

const WIDTH: u32 = 2160;
const HEIGHT: u32 = 3840;
const HEAD_PX: f32 = 600.0;
const WARM: u32 = 30;
const FRAMES: u32 = 120;
const SAMPLES: u32 = 16;

const MATERIALS: &str = "format = 1

[materials.plain]
base = [0.8, 0.6, 0.5]
roughness = 0.5

[materials.fabric]
base = [0.8, 0.6, 0.5]
roughness = 0.5
fabric = { color = [0.9, 0.9, 0.9], roughness = 0.5 }

[materials.brushed]
base = [0.8, 0.6, 0.5]
roughness = 0.5
anisotropy = { strength = 0.8, rotation = 0.0 }

[materials.scattered]
base = [0.8, 0.6, 0.5]
roughness = 0.5
scatter = { color = [1.0, 0.3, 0.2], radius = 0.01, amount = 1.0 }

[materials.all]
base = [0.8, 0.6, 0.5]
roughness = 0.5
fabric = { color = [0.9, 0.9, 0.9], roughness = 0.5 }
anisotropy = { strength = 0.8, rotation = 0.0 }
scatter = { color = [1.0, 0.3, 0.2], radius = 0.01, amount = 1.0 }
";

fn map(texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::new();
    for y in 0..512 {
        for x in 0..512 {
            rgba.extend(texel(x, y));
        }
    }
    ball_png(512, 512, &rgba)
}

fn folder(name: &str, material: Option<&str>) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/lobes-cost")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let glb = match material {
        Some(_) => ball(json!({ "name": "ball" }), &[]),
        None => ball(
            json!({
                "name": "ball",
                "pbrMetallicRoughness": {
                    "baseColorTexture": { "index": 0 },
                    "metallicRoughnessTexture": { "index": 2 },
                },
                "normalTexture": { "index": 1 },
                "occlusionTexture": { "index": 2 },
                "extras": { "detailNormalTexture": { "index": 1, "texCoord": 1, "extensions": { "KHR_texture_transform": { "scale": [4.0, 4.0] } } } },
            }),
            &[
                map(|x, y| [(x / 2) as u8, (y / 2) as u8, 160, 255]),
                map(|x, y| [128 + ((x / 8 + y / 8) % 2 * 40) as u8, 128, 230, 255]),
                map(|x, _| [200, (x / 2) as u8, 0, 255]),
            ],
        ),
    };
    std::fs::write(root.join("ball.glb"), glb).unwrap();
    std::fs::write(root.join("ball.materials.toml"), MATERIALS).unwrap();
    let object = match material {
        Some(name) => {
            format!("[[object]]\nname = \"ball\"\nmesh = \"ball\"\nmaterial = \"{name}\"\n")
        }
        None => "[[object]]\nname = \"ball\"\nmesh = \"ball\"\n".to_string(),
    };
    let height = HEIGHT as f32 / HEAD_PX;
    std::fs::write(
        root.join("ball.scene.toml"),
        format!(
            "format = 1\nmaterials = [\"ball.materials.toml\"]\n\n[mesh.ball]\nfile = \"ball.glb\"\n\n[camera]\nat = [0.0, 0.0, 3.0]\nlook_at = [0.0, 0.0, 0.0]\nprojection = \"orthographic\"\nheight = {height}\n\n[sun]\nmodel = \"authored\"\ntoward = [0.6, 0.4, 0.7]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\n\n{object}"
        ),
    )
    .unwrap();
    root.join("ball.scene.toml")
}

fn target(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lobes cost"),
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
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut opaque = Vec::new();
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
            opaque.push(
                passes
                    .iter()
                    .filter(|pass| pass.label.contains("opaque"))
                    .map(|pass| pass.milliseconds)
                    .sum(),
            );
            total.push(pfx_gpu::pace::gpu_ms(&passes).unwrap_or(f64::NAN));
        }
    }
    staged.release(&mut renderer).unwrap();
    (median(opaque), median(total))
}

fn traced(scene: &Scene) -> f64 {
    let staged = pfx_trace::stage::scene_at(scene, WIDTH as f32 / HEIGHT as f32, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    trace.sample(&gpu, 1, 1).unwrap();
    let start = std::time::Instant::now();
    for pass in 0..SAMPLES {
        trace.sample(&gpu, 1, 2 + pass).unwrap();
    }
    start.elapsed().as_secs_f64() * 1000.0 / f64::from(SAMPLES)
}

fn main() {
    for (name, material) in [
        ("plain", Some("plain")),
        ("fabric", Some("fabric")),
        ("brushed", Some("brushed")),
        ("scattered", Some("scattered")),
        ("all", Some("all")),
        ("maps", None),
    ] {
        let scene = Scene::open(folder(name, material)).unwrap();
        let (opaque, total) = live(&scene);
        let sample = traced(&scene);
        println!(
            "{{\"case\":\"{name}\",\"live_opaque_ms\":{opaque:.4},\"live_frame_ms\":{total:.4},\"trace_sample_ms\":{sample:.3}}}"
        );
    }
}
