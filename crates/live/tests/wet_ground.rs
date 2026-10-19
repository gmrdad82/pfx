use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_post::Chain;
use serde_json::json;

const WIDTH: u32 = 384;
const HEIGHT: u32 = 256;
const SAMPLES: u32 = 32;
const GROUND: f32 = 16.0;

fn plane_glb(half: f32, texture: Option<&[u8]>) -> Vec<u8> {
    let positions: [[f32; 3]; 4] = [
        [-half, 0.0, half],
        [half, 0.0, half],
        [half, 0.0, -half],
        [-half, 0.0, -half],
    ];
    let normals = [[0.0f32, 1.0, 0.0]; 4];
    let uvs: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let mut bin = Vec::new();
    for p in positions {
        for v in p {
            bin.extend(v.to_le_bytes());
        }
    }
    for n in normals {
        for v in n {
            bin.extend(v.to_le_bytes());
        }
    }
    for t in uvs {
        for v in t {
            bin.extend(v.to_le_bytes());
        }
    }
    for i in indices {
        bin.extend(i.to_le_bytes());
    }
    let image_at = bin.len();
    if let Some(png) = texture {
        bin.extend_from_slice(png);
    }
    let mut document = json!({
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
    if let Some(png) = texture {
        document["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "buffer": 0, "byteOffset": image_at, "byteLength": png.len() }));
        document["images"] = json!([{ "bufferView": 4, "mimeType": "image/png" }]);
        document["textures"] = json!([{ "source": 0 }]);
        document["materials"] = json!([{
            "name": "road",
            "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 }, "metallicFactor": 0.0, "roughnessFactor": 0.6 },
        }]);
        document["meshes"][0]["primitives"][0]["material"] = json!(0);
    }
    pfx_load::fixture::glb(&document.to_string(), &bin)
}

fn folder(name: &str, ground: &str, extra: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("wet-ground")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ground.glb"), plane_glb(GROUND * 0.5, None)).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(
        root.join("ground.materials.toml"),
        format!(
            "format = 1\n\n[materials.paint]\nbase = [0.6, 0.12, 0.05]\nroughness = 0.5\nspecular = 0.04\n\n[materials.road]\n{ground}\n"
        ),
    )
    .unwrap();
    let path = root.join("ground.scene.toml");
    std::fs::write(
        &path,
        format!(
            "format = 1\nmaterials = [\"ground.materials.toml\"]\n\n[mesh.ground]\nfile = \"ground.glb\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"ground\"\nmesh = \"ground\"\nmaterial = \"road\"\n\n[[object]]\nname = \"box\"\nmesh = \"block\"\nmaterial = \"paint\"\nat = [0.0, 0.5, 0.0]\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.45, -1.0]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\n\n[camera]\nat = [0.0, 0.9, 3.0]\nlook_at = [0.0, 0.4, 0.0]\nfov = 50.0\n{extra}"
        ),
    )
    .unwrap();
    path
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wet ground output"),
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

fn live(scene: &Scene) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    for _ in 0..8 {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, 0.0, 7);
        renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
    }
    let pixels = renderer
        .gpu()
        .readback_rgba16(&target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| {
            std::array::from_fn(|c| {
                pfx_materials::linear_channel(half::f16::from_bits(p[c]).to_f32())
            })
        })
        .collect();
    staged.release(&mut renderer).unwrap();
    pixels
}

fn traced(scene: &Scene) -> Vec<[f32; 3]> {
    let staged = pfx_trace::stage::scene_at(scene, WIDTH as f32 / HEIGHT as f32, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    for pass in 0..SAMPLES {
        trace.sample(&gpu, 1, 11 + pass).unwrap();
    }
    trace
        .readback(&gpu)
        .unwrap()
        .color
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn save(name: &str, pixels: &[[f32; 3]]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/wet-ground");
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            p.map(|v| {
                (pfx_materials::encode_channel((v * 1.5).clamp(0.0, 1.0)) * 255.0).round() as u8
            })
        })
        .collect();
    writer.write_image_data(&bytes).unwrap();
}

fn luminance(p: &[f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn region(pixels: &[[f32; 3]], x: [u32; 2], y: [u32; 2]) -> f32 {
    let values: Vec<f32> = (y[0]..y[1])
        .flat_map(|row| (x[0]..x[1]).map(move |column| (row * WIDTH + column) as usize))
        .map(|index| luminance(&pixels[index]))
        .collect();
    values.iter().sum::<f32>() / values.len() as f32
}

const ROAD: &str = "base = [0.06, 0.06, 0.065]\nroughness = 0.6\nspecular = 0.04\n";
const CHALK: &str = "base = [0.5, 0.5, 0.5]\nroughness = 0.9\nspecular = 0.02\n";
const NEAR: ([u32; 2], [u32; 2]) = ([32, 352], [208, 252]);

fn layer(base: &str, kind: &str, frequency: f32, amplitude: f32, params: &str) -> String {
    format!(
        "{base}\n[[materials.road.layers]]\nkind = \"{kind}\"\nfrequency = {frequency:?}\namplitude = {amplitude:?}\nseed = 0\n{params}"
    )
}

fn grime(roughness: [f32; 2]) -> String {
    layer(
        ROAD,
        "grime",
        1.0,
        1.0,
        &format!(
            "params = [160.0, 800.0, 0.9, 0.84, 0.76, 1.03, 1.02, 1.0, {:?}, {:?}, 0.0, 0.0]\n",
            roughness[0], roughness[1]
        ),
    )
}

fn both(name: &str, ground: &str, extra: &str) -> [Vec<[f32; 3]>; 2] {
    let scene = Scene::open(folder(name, ground, extra)).unwrap();
    let pair = [live(&scene), traced(&scene)];
    save(&format!("{name}-live"), &pair[0]);
    save(&format!("{name}-traced"), &pair[1]);
    pair
}

fn cells(pixels: &[[f32; 3]], plain: &[[f32; 3]]) -> Vec<f32> {
    let mut out = Vec::new();
    for cy in (NEAR.1[0]..NEAR.1[1] - 16).step_by(16) {
        for cx in (NEAR.0[0]..NEAR.0[1]).step_by(16) {
            let x = [cx, cx + 16];
            let y = [cy, cy + 16];
            out.push(region(pixels, x, y) / region(plain, x, y).max(1e-6));
        }
    }
    out
}

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let mut ab = 0.0;
    let mut aa = 0.0;
    let mut bb = 0.0;
    for (x, y) in a.iter().zip(b) {
        ab += (x - ma) * (y - mb);
        aa += (x - ma).powi(2);
        bb += (y - mb).powi(2);
    }
    ab / (aa * bb).sqrt().max(1e-12)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn material_layers_agree_with_the_tracer_on_a_generated_plane() {
    let mut means = Vec::new();
    for (name, ground) in [
        ("plain", ROAD.to_string()),
        ("fbm", layer(ROAD, "fbm", 30.0, 0.35, "")),
        ("grime-rough", grime([1.25, 0.6])),
        ("grime-smooth", grime([1.0, 0.92])),
        ("grime-bare", layer(ROAD, "grime", 1.0, 1.0, "")),
        ("wall-bare", layer(ROAD, "wall_mottle", 1.0, 1.0, "")),
    ] {
        let [live, traced] = both(name, &ground, "");
        let pair = [
            region(&live, NEAR.0, NEAR.1),
            region(&traced, NEAR.0, NEAR.1),
        ];
        println!("{name}: live {:.4} traced {:.4}", pair[0], pair[1]);
        assert!(
            (pair[0] - pair[1]).abs() <= pair[1] * 0.04,
            "{name}: live {} against traced {}",
            pair[0],
            pair[1]
        );
        means.push(pair);
    }
    let [plain, _, rough, smooth, bare, _] = means[..] else {
        unreachable!()
    };
    let live_step = rough[0] - smooth[0];
    let traced_step = rough[1] - smooth[1];
    assert!(
        traced_step > 0.0 && live_step > 0.5 * traced_step,
        "rougher grime factors moved live by {live_step}, traced by {traced_step}"
    );
    assert!(
        bare[0] > plain[0] * 0.8,
        "a grime layer without params turned the road into a mirror: {} against {}",
        bare[0],
        plain[0]
    );
    let [plain_live, plain_traced] = both("chalk", CHALK, "");
    let [fbm_live, fbm_traced] = both("chalk-fbm", &layer(CHALK, "fbm", 2.0, 1.0, ""), "");
    let live_cells = cells(&fbm_live, &plain_live);
    let traced_cells = cells(&fbm_traced, &plain_traced);
    let spread = |cells: &[f32]| {
        cells.iter().cloned().fold(f32::MIN, f32::max)
            - cells.iter().cloned().fold(f32::MAX, f32::min)
    };
    let r = correlation(&live_cells, &traced_cells);
    println!(
        "fbm over chalk: correlation {r:.3}, spread live {:.3} traced {:.3}",
        spread(&live_cells),
        spread(&traced_cells)
    );
    assert!(spread(&traced_cells) > 0.2, "the tracer drew no fbm");
    assert!(r > 0.8, "live and traced fbm differ: correlation {r}");
}

const HAZE: &str = "\n[haze]\namount = 0.12\nfalloff = 0.05\n";
const POLE: &str = "\n[[object]]\nname = \"pole\"\nmesh = \"block\"\nmaterial = \"paint\"\nat = [-1.2, 1.5, -1.0]\nscale = [0.012, 3.0, 0.012]\n";

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_thin_pole_keeps_its_contrast_against_a_hazy_sky() {
    let clear = live(&Scene::open(folder("pole-clear", ROAD, POLE)).unwrap());
    let hazy = live(&Scene::open(folder("pole-hazy", ROAD, &format!("{POLE}{HAZE}"))).unwrap());
    save("pole-clear-live", &clear);
    save("pole-hazy-live", &hazy);
    let mut pole = Vec::new();
    let mut sky = Vec::new();
    for y in 5..60u32 {
        for x in 95..120u32 {
            let i = (y * WIDTH + x) as usize;
            if luminance(&clear[i]) < 0.05 {
                pole.push(i);
            } else if (0..4)
                .all(|d| luminance(&clear[i + d]) > 0.05 && luminance(&clear[i - d]) > 0.05)
            {
                sky.push(i);
            }
        }
    }
    let mean = |image: &[[f32; 3]], set: &[usize]| {
        set.iter().map(|&i| luminance(&image[i])).sum::<f32>() / set.len() as f32
    };
    let (pole_clear, pole_hazy) = (mean(&clear, &pole), mean(&hazy, &pole));
    let (sky_clear, sky_hazy) = (mean(&clear, &sky), mean(&hazy, &sky));
    println!(
        "pole {} px: clear {pole_clear:.4} hazy {pole_hazy:.4}; sky {} px: clear {sky_clear:.4} hazy {sky_hazy:.4}",
        pole.len(),
        sky.len()
    );
    assert!(pole.len() > 30 && sky.len() > 500);
    assert!(sky_hazy > sky_clear * 1.5, "the haze reached the sky");
    assert!(
        pole_hazy < sky_hazy * 0.3,
        "the pole took the sky's haze: {pole_hazy} against the sky's {sky_hazy}"
    );
}

const SLAB: &str = "\n[[object]]\nname = \"slab\"\nmesh = \"block\"\nmaterial = \"paint\"\nat = [-1.5, 0.4, 0.4]\nscale = [1.0, 0.12, 1.0]\n";

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_wet_floor_reflects_dark_under_a_raised_slab() {
    let wet = format!("{ROAD}wet = {{ amount = 1.0, puddles = 1.0 }}\n");
    let [live, traced] = both("wet", &wet, SLAB);
    let mut means = Vec::new();
    for (label, x, y) in [
        ("under the slab", [5u32, 90u32], [175u32, 215u32]),
        ("under the box", [150, 230], [146, 176]),
        ("open ground", [20, 100], [120, 140]),
    ] {
        let pair = [region(&live, x, y), region(&traced, x, y)];
        println!("{label}: live {:.4} traced {:.4}", pair[0], pair[1]);
        assert!(
            (pair[0] - pair[1]).abs() <= pair[1] * 0.2 + 0.002,
            "{label}: live {} against traced {}",
            pair[0],
            pair[1]
        );
        means.push(pair);
    }
    let shade = |k: usize| means[0][k] / means[2][k];
    println!(
        "under the slab over open ground: live {:.3} traced {:.3}",
        shade(0),
        shade(1)
    );
    assert!(
        (shade(0) - shade(1)).abs() <= shade(1) * 0.25,
        "the floor under the slab reflects {} of the open ground live, {} traced",
        shade(0),
        shade(1)
    );
}

const CELLS: u32 = 32;

fn checker() -> Vec<u8> {
    let side = CELLS * 8;
    let mut rgba = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let light = (x / 8 + y / 8) % 2 == 0;
            let v = if light { 200 } else { 40 };
            rgba.extend([v, v, v, 255]);
        }
    }
    pfx_load::fixture::ball_png(side, side, &rgba)
}

fn textured(name: &str, library: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("wet-ground")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("ground.glb"),
        plane_glb(GROUND * 0.5, Some(&checker())),
    )
    .unwrap();
    std::fs::write(
        root.join("ground.materials.toml"),
        format!(
            "format = 1\n\n[materials.paint]\nbase = [0.6, 0.12, 0.05]\nroughness = 0.5\n{library}"
        ),
    )
    .unwrap();
    let path = root.join("ground.scene.toml");
    std::fs::write(
        &path,
        "format = 1\nmaterials = [\"ground.materials.toml\"]\n\n[mesh.ground]\nfile = \"ground.glb\"\n\n[[object]]\nname = \"ground\"\nmesh = \"ground\"\n\n[sun]\nmodel = \"authored\"\ntoward = [-0.3, 0.8, 0.5]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\n\n[camera]\nat = [0.0, 0.9, 3.0]\nlook_at = [0.0, 0.4, 0.0]\nfov = 50.0\n",
    )
    .unwrap();
    path
}

fn ground_cell(x: u32, y: u32) -> Option<bool> {
    let eye = [0.0f32, 0.9, 3.0];
    let target = [0.0f32, 0.4, 0.0];
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let unit = |a: [f32; 3]| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        a.map(|v| v / l)
    };
    let forward = unit(sub(target, eye));
    let right = unit([-forward[2], 0.0, forward[0]]);
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    let tan = (25.0f32).to_radians().tan();
    let sx = ((x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0) * tan * WIDTH as f32 / HEIGHT as f32;
    let sy = (1.0 - (y as f32 + 0.5) / HEIGHT as f32 * 2.0) * tan;
    let ray: [f32; 3] = std::array::from_fn(|k| forward[k] + right[k] * sx + up[k] * sy);
    if ray[1] >= 0.0 {
        return None;
    }
    let t = -eye[1] / ray[1];
    let half = GROUND * 0.5;
    let u = (eye[0] + ray[0] * t + half) / GROUND * CELLS as f32;
    let v = (half - (eye[2] + ray[2] * t)) / GROUND * CELLS as f32;
    let inner = |w: f32| (0.2..0.8).contains(&(w - w.floor()));
    (u > 0.0 && v > 0.0 && inner(u) && inner(v))
        .then(|| (u.floor() as i64 + v.floor() as i64) % 2 == 0)
}

fn checks(pixels: &[[f32; 3]]) -> [f32; 2] {
    let mut sums = [0.0f32; 2];
    let mut counts = [0u32; 2];
    for y in NEAR.1[0]..NEAR.1[1] {
        for x in NEAR.0[0]..NEAR.0[1] {
            if let Some(light) = ground_cell(x, y) {
                let k = usize::from(!light);
                sums[k] += luminance(&pixels[(y * WIDTH + x) as usize]);
                counts[k] += 1;
            }
        }
    }
    [sums[0] / counts[0] as f32, sums[1] / counts[1] as f32]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_wet_layer_keeps_the_ground_texture_under_it() {
    let dry = Scene::open(textured("textured-dry", "")).unwrap();
    let wet = Scene::open(textured(
        "textured-wet",
        "\n[materials.road]\nbase = [1.0, 1.0, 1.0]\nroughness = 0.6\nwet = { amount = 0.7 }\n",
    ))
    .unwrap();
    for (renderer, render) in [
        ("live", live as fn(&Scene) -> Vec<[f32; 3]>),
        ("traced", traced as fn(&Scene) -> Vec<[f32; 3]>),
    ] {
        let dry = render(&dry);
        let wet = render(&wet);
        save(&format!("textured-dry-{renderer}"), &dry);
        save(&format!("textured-wet-{renderer}"), &wet);
        let [dry_light, dry_dark] = checks(&dry);
        let [wet_light, wet_dark] = checks(&wet);
        println!(
            "{renderer}: dry light {dry_light:.4} dark {dry_dark:.4}; wet light {wet_light:.4} dark {wet_dark:.4}"
        );
        assert!(
            dry_light > dry_dark * 2.0,
            "{renderer}: the texture draws dry"
        );
        assert!(
            wet_light > wet_dark * 1.5,
            "{renderer}: the wet layer hid the texture, {wet_light} against {wet_dark}"
        );
        assert!(
            wet_light < dry_light * 0.9,
            "{renderer}: the wet layer darkens"
        );
    }
}
