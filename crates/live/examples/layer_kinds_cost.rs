use std::path::{Path, PathBuf};

use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use serde_json::json;

const WARM: u32 = 30;
const FRAMES: u32 = 120;
const SIZE: [u32; 2] = [3840, 2160];
const KINDS: [(&str, f32); 7] = [
    ("none", 0.0),
    ("leaf", 1.0),
    ("bark", 1.0),
    ("value", 6.0),
    ("fbm", 6.0),
    ("flow", 400.0),
    ("wall_mottle", 1.0),
];

fn plane(half: f32) -> Vec<u8> {
    let mut bin = Vec::new();
    for value in [
        -half, -half, 0.0, half, -half, 0.0, half, half, 0.0, -half, half, 0.0,
    ] {
        bin.extend(f32::to_le_bytes(value));
    }
    for _ in 0..4 {
        for value in [0.0f32, 0.0, 1.0] {
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
        "nodes": [{ "mesh": 0, "name": "patch" }],
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
            { "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-half, -half, 0.0], "max": [half, half, 0.0] },
            { "bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5125, "count": 6, "type": "SCALAR" },
        ],
    });
    pfx_load::fixture::glb(&document.to_string(), &bin)
}

fn folder(kind: &str, frequency: f32) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/layer-kinds-cost")
        .join(kind);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("patch.glb"), plane(4.0)).unwrap();
    let layers = if kind == "none" {
        String::new()
    } else {
        format!(
            "layers = [\n  {{ kind = \"{kind}\", frequency = {frequency:?}, amplitude = 0.8, seed = 3 }},\n]\n"
        )
    };
    std::fs::write(
        root.join("patch.materials.toml"),
        format!(
            "format = 1\n\n[materials.warm]\nbase = [0.6, 0.45, 0.3]\nroughness = 0.6\nspecular = 0.3\n{layers}"
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("patch.scene.toml"),
        "format = 1\nmaterials = [\"patch.materials.toml\"]\n\n[mesh.patch]\nfile = \"patch.glb\"\n\n[[object]]\nname = \"patch\"\nmesh = \"patch\"\nmaterial = \"warm\"\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.5, 0.8]\ncolor = [1.0, 0.95, 0.9]\nirradiance = 2.0\n\n[sky]\nkind = \"analytic\"\n\n[camera]\nat = [0.0, 0.0, 1.6]\nlook_at = [0.0, 0.0, 0.0]\nfov = 50.0\n",
    )
    .unwrap();
    root.join("patch.scene.toml")
}

fn target(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("layer kinds cost"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
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
        width: SIZE[0],
        height: SIZE[1],
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let mut turns = Turns::default();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = target(&gpu);
    let mut renderer = Renderer::new(gpu, SIZE[0], SIZE[1]).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let aspect = SIZE[0] as f32 / SIZE[1] as f32;
    for (kind, frequency) in KINDS {
        let scene = Scene::open(folder(kind, frequency)).unwrap();
        let mut staged = renderer.stage(&scene).unwrap();
        let mut opaque = Vec::new();
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
                opaque.push(
                    timings
                        .iter()
                        .filter(|timing| timing.label == "opaque PBR")
                        .map(|timing| timing.milliseconds)
                        .sum(),
                );
                total.push(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
            }
        }
        staged.release(&mut renderer).unwrap();
        println!(
            "{{\"kind\":\"{kind}\",\"size\":\"{}x{}\",\"opaque_ms\":{:.4},\"frame_ms\":{:.4}}}",
            SIZE[0],
            SIZE[1],
            median(opaque),
            median(total)
        );
    }
}
