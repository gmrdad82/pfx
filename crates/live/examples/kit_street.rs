use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_post::Chain;
use serde_json::json;

const USAGE: &str = "usage: kit_street write <folder> [side] | kit_street live|trace <scene.toml>";
const COPIES: usize = 10;
const PANELS: [&str; 4] = ["brick", "glass", "wood", "trim"];
const WIDTH: u32 = 540;
const HEIGHT: u32 = 960;

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

fn png(side: u32, seed: u32, normal: bool) -> Vec<u8> {
    let mut texels = Vec::with_capacity((side * side * 3) as usize);
    for y in 0..side {
        for x in 0..side {
            let mortar = (y % 64) < 4 || ((x + (y / 64 % 2) * 64) % 128) < 4;
            let grain = (hash(x / 4, y / 4, seed) & 31) as u8;
            if normal {
                let tilt = if mortar { 96 } else { 128 };
                texels.extend([tilt + grain / 2, 128, 255]);
            } else if mortar {
                texels.extend([200 + grain / 2, 196 + grain / 2, 188 + grain / 2]);
            } else {
                let base = (seed * 37 % 96) as u8;
                texels.extend([120 + base / 2 + grain, 90 + grain, 70 + base / 3 + grain]);
            }
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, side, side);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        if !normal {
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        }
        let mut writer = encoder.write_header().expect("the map header encodes");
        writer.write_image_data(&texels).expect("the map encodes");
    }
    bytes
}

fn kit(maps: &[(Vec<u8>, Vec<u8>)], colours: &[[f32; 3]]) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut view = |bin: &mut Vec<u8>, bytes: &[u8]| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len() }));
        bin.extend_from_slice(bytes);
        views.len() - 1
    };
    for (index, name) in PANELS.iter().enumerate() {
        let left = index as f32 * 2.0 - 4.0;
        let positions: Vec<f32> = [[0.0, 0.0], [2.0, 0.0], [2.0, 6.0], [0.0, 6.0]]
            .iter()
            .flat_map(|[x, y]| [left + x, *y, 0.0])
            .collect();
        let normals: Vec<f32> = (0..4).flat_map(|_| [0.0, 0.0, 1.0]).collect();
        let tangents: Vec<f32> = (0..4).flat_map(|_| [1.0, 0.0, 0.0, 1.0]).collect();
        let uvs: Vec<f32> = vec![0.0, 3.0, 1.0, 3.0, 1.0, 0.0, 0.0, 0.0];
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let floats = |values: &[f32]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let at = accessors.len();
        for (values, kind) in [
            (floats(&positions), "VEC3"),
            (floats(&normals), "VEC3"),
            (floats(&tangents), "VEC4"),
            (floats(&uvs), "VEC2"),
        ] {
            let v = view(&mut bin, &values);
            let mut accessor =
                json!({ "bufferView": v, "componentType": 5126, "count": 4, "type": kind });
            if kind == "VEC3" && accessors.len() == at {
                accessor["min"] = json!([left, 0.0, 0.0]);
                accessor["max"] = json!([left + 2.0, 6.0, 0.0]);
            }
            accessors.push(accessor);
        }
        let v = view(
            &mut bin,
            &indices
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<u8>>(),
        );
        accessors
            .push(json!({ "bufferView": v, "componentType": 5125, "count": 6, "type": "SCALAR" }));
        let (base, normal) = &maps[index];
        let base_view = view(&mut bin, base);
        images.push(json!({ "bufferView": base_view, "mimeType": "image/png" }));
        let normal_view = view(&mut bin, normal);
        images.push(json!({ "bufferView": normal_view, "mimeType": "image/png" }));
        let [r, g, b] = colours[index];
        materials.push(json!({
            "name": name,
            "pbrMetallicRoughness": {
                "baseColorFactor": [r, g, b, 1.0],
                "baseColorTexture": { "index": index * 2 },
                "metallicFactor": 0.0,
                "roughnessFactor": 0.8,
            },
            "normalTexture": { "index": index * 2 + 1 },
        }));
        meshes.push(json!({ "primitives": [{
            "attributes": { "POSITION": at, "NORMAL": at + 1, "TANGENT": at + 2, "TEXCOORD_0": at + 3 },
            "indices": at + 4,
            "material": index,
        }]}));
        nodes.push(json!({ "mesh": index, "name": name }));
    }
    let textures: Vec<_> = (0..images.len())
        .map(|source| json!({ "source": source }))
        .collect();
    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": (0..PANELS.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "images": images,
        "textures": textures,
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": accessors,
    });
    pfx_load::fixture::glb(&document.to_string(), &bin)
}

fn colour(copy: usize) -> [f32; 3] {
    let k = copy as f32 / COPIES as f32;
    [0.55 + 0.45 * k, 0.9 - 0.4 * k, 0.6 + 0.3 * (1.0 - k)]
}

const LOOK: &str = "\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.6, 1.0]\ncolor = [1.0, 0.96, 0.9]\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\n\n[camera]\nat = [0.0, 3.0, 110.0]\nlook_at = [0.0, 3.0, 0.0]\nfov = 70.0\n";

fn write(folder: &Path, side: u32) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let maps: Vec<(Vec<u8>, Vec<u8>)> = (0..PANELS.len() as u32)
        .map(|seed| (png(side, seed + 1, false), png(side, seed + 11, true)))
        .collect();
    let save = |name: &str, bytes: &[u8]| {
        std::fs::write(folder.join(name), bytes).map_err(|e| format!("{name}: {e}"))
    };
    let at = |copy: usize| (copy as f32 - (COPIES - 1) as f32 / 2.0) * 9.0;
    let mut copies = String::from("format = 1\n");
    for copy in 0..COPIES {
        save(&format!("kit{copy}.glb"), &kit(&maps, &[colour(copy); 4]))?;
        copies += &format!(
            "\n[mesh.kit{copy}]\nfile = \"kit{copy}.glb\"\n\n[[object]]\nname = \"house{copy}\"\nmesh = \"kit{copy}\"\nat = [{:?}, 0.0, 0.0]\n",
            at(copy)
        );
    }
    save("copies.scene.toml", (copies + LOOK).as_bytes())?;
    save("kit.glb", &kit(&maps, &[[1.0; 3]; 4]))?;
    let mut tinted = String::from("format = 1\n\n[mesh.kit]\nfile = \"kit.glb\"\n");
    for copy in 0..COPIES {
        let [r, g, b] = colour(copy);
        tinted += &format!(
            "\n[[object]]\nname = \"house{copy}\"\nmesh = \"kit\"\nat = [{:?}, 0.0, 0.0]\ntint = [{r:?}, {g:?}, {b:?}]\n",
            at(copy)
        );
    }
    save("tinted.scene.toml", (tinted + LOOK).as_bytes())?;
    Ok(())
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("kit street output"),
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

fn live(scene: &Scene) -> Result<(), String> {
    let gpu = pollster::block_on(Gpu::headless())?;
    let target = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT)?;
    renderer.set_exposure(Exposure::Fixed(1.0))?;
    let mut staged = renderer.stage(scene)?;
    renderer.set_finish(Chain::new());
    for frame in 0..8 {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, frame as f32 / 60.0, 7);
        renderer.render(
            &drawn.scene,
            &drawn.text,
            &drawn.effects,
            finish,
            &target.view,
        )?;
        pfx_core::memory::look_gpu();
    }
    renderer.gpu().readback_rgba16(&target)?;
    pfx_core::memory::look_gpu();
    staged.release(&mut renderer)?;
    Ok(())
}

fn trace(scene: &Scene) -> Result<(), String> {
    let staged = pfx_trace::stage::scene_at(scene, WIDTH as f32 / HEIGHT as f32, 0.0)?;
    let gpu = pollster::block_on(Gpu::headless())?;
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT)?;
    for pass in 0..8 {
        trace.sample(&gpu, 1, 11 + pass)?;
        pfx_core::memory::look_gpu();
    }
    trace.readback(&gpu)?;
    pfx_core::memory::look_gpu();
    Ok(())
}

fn gb(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "unknown".into(), |b| format!("{:.2} GB", b as f64 / 1e9))
}

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [verb, folder] if verb == "write" => {
            write(Path::new(folder), 2048).map(|()| format!("wrote {folder}"))
        }
        [verb, folder, side] if verb == "write" => {
            let side = side.parse().map_err(|_| USAGE.to_string())?;
            write(Path::new(folder), side).map(|()| format!("wrote {folder}"))
        }
        [verb, path] if verb == "live" || verb == "trace" => {
            pfx_core::memory::watch_gpu(std::time::Duration::from_millis(50));
            let path = PathBuf::from(path);
            let scene = Scene::open(&path).map_err(|e| e.to_string())?;
            let loaded = pfx_core::memory::peak();
            if verb == "live" {
                live(&scene)?;
            } else {
                trace(&scene)?;
            }
            let peak = pfx_core::memory::peak();
            Ok(format!(
                "{verb} {}: {}x{}, loaded at {} on the CPU; peak {} on the CPU, {} on the GPU",
                path.display(),
                WIDTH,
                HEIGHT,
                gb(loaded.cpu),
                gb(peak.cpu),
                gb(peak.gpu)
            ))
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("kit_street: {error}");
            ExitCode::FAILURE
        }
    }
}
