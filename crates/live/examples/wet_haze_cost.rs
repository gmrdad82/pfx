use std::path::{Path, PathBuf};

use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::capture::SceneReflections;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use serde_json::json;

const WARM: u32 = 30;
const FRAMES: u32 = 120;
const SIZES: [[u32; 2]; 2] = [[1920, 1080], [3840, 2160]];
const PASSES: [&str; 3] = ["height fog march", "height fog upsample", "reflections"];

fn plane(half: f32) -> Vec<u8> {
    let mut bin = Vec::new();
    for value in [
        -half, 0.0, half, half, 0.0, half, half, 0.0, -half, -half, 0.0, -half,
    ] {
        bin.extend(f32::to_le_bytes(value));
    }
    for _ in 0..4 {
        for value in [0.0f32, 1.0, 0.0] {
            bin.extend(value.to_le_bytes());
        }
    }
    for value in [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0] {
        bin.extend(value.to_le_bytes());
    }
    for index in [0u32, 1, 2, 0, 2, 3] {
        bin.extend(index.to_le_bytes());
    }
    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": "ground" }],
        "meshes": [{ "primitives": [{
            "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
            "indices": 3,
        }]}],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 48 },
            { "buffer": 0, "byteOffset": 48, "byteLength": 48 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 32 },
            { "buffer": 0, "byteOffset": 128, "byteLength": 24 },
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-half, 0.0, -half], "max": [half, 0.0, half] },
            { "bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5125, "count": 6, "type": "SCALAR" },
        ],
    });
    pfx_load::fixture::glb(&document.to_string(), &bin)
}

fn folder() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/wet-haze-cost");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ground.glb"), plane(8.0)).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(
        root.join("ground.materials.toml"),
        "format = 1\n\n[materials.paint]\nbase = [0.6, 0.12, 0.05]\nroughness = 0.5\nspecular = 0.04\n\n[materials.road]\nbase = [0.06, 0.06, 0.065]\nroughness = 0.6\nspecular = 0.04\nwet = { amount = 1.0, puddles = 1.0 }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("ground.scene.toml"),
        "format = 1\nmaterials = [\"ground.materials.toml\"]\n\n[mesh.ground]\nfile = \"ground.glb\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"ground\"\nmesh = \"ground\"\nmaterial = \"road\"\n\n[[object]]\nname = \"box\"\nmesh = \"block\"\nmaterial = \"paint\"\nat = [0.0, 0.5, 0.0]\n\n[[object]]\nname = \"slab\"\nmesh = \"block\"\nmaterial = \"paint\"\nat = [-1.5, 0.4, 0.4]\nscale = [1.0, 0.12, 1.0]\n\n[[object]]\nname = \"pole\"\nmesh = \"block\"\nmaterial = \"paint\"\nat = [-1.2, 1.5, -1.0]\nscale = [0.012, 3.0, 0.012]\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.45, -1.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\n\n[camera]\nat = [0.0, 0.9, 3.0]\nlook_at = [0.0, 0.4, 0.0]\nfov = 50.0\n\n[haze]\namount = 0.12\nfalloff = 0.05\n",
    )
    .unwrap();
    root.join("ground.scene.toml")
}

fn target(gpu: &Gpu, size: [u32; 2]) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wet haze cost"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
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
        width: size[0],
        height: size[1],
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn stage_cost(scene: &Scene, turns: &mut Turns) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, 1920, 1080).unwrap();
    let mut first = 0.0;
    let mut times = [Vec::new(), Vec::new()];
    for round in 0..11 {
        let on = round % 2 == 0;
        renderer
            .set_scene_reflections(on.then(SceneReflections::default))
            .unwrap();
        turns.poll();
        let start = std::time::Instant::now();
        let staged = renderer.stage(scene).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        staged.release(&mut renderer).unwrap();
        turns.turn();
        if round == 0 {
            first = ms;
        } else {
            times[usize::from(!on)].push(ms);
        }
    }
    let [on, off] = times;
    println!(
        "{{\"stage_first_ms\":{first:.1},\"stage_reflections_ms\":{:.1},\"stage_plain_ms\":{:.1}}}",
        median(on),
        median(off)
    );
}

fn main() {
    let scene = Scene::open(folder()).unwrap();
    let mut turns = Turns::default();
    stage_cost(&scene, &mut turns);
    for size in SIZES {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let output = target(&gpu, size);
        let mut renderer = Renderer::new(gpu, size[0], size[1]).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let mut staged = renderer.stage(&scene).unwrap();
        let aspect = size[0] as f32 / size[1] as f32;
        let mut passes: Vec<Vec<f64>> = vec![Vec::new(); PASSES.len()];
        let mut total = Vec::new();
        for frame in 0..WARM + FRAMES {
            let finish = staged.finish();
            let drawn = staged.frame(aspect, 0.0, 7);
            turns.poll();
            let timings = renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
            turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
            turns.turn();
            if frame >= WARM {
                for (label, values) in PASSES.iter().zip(&mut passes) {
                    values.push(
                        timings
                            .iter()
                            .filter(|timing| timing.label == *label)
                            .map(|timing| timing.milliseconds)
                            .sum(),
                    );
                }
                total.push(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
            }
        }
        staged.release(&mut renderer).unwrap();
        let [march, upsample, reflections] = [0, 1, 2].map(|k| median(passes[k].clone()));
        println!(
            "{{\"size\":\"{}x{}\",\"fog_march_ms\":{march:.4},\"fog_upsample_ms\":{upsample:.4},\"reflections_ms\":{reflections:.4},\"frame_ms\":{:.4}}}",
            size[0],
            size[1],
            median(total)
        );
    }
}
