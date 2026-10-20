use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::{BALL_RADIUS, ball};
use pfx_load::scene::Scene;
use pfx_post::Chain;
use serde_json::json;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 192;
const SAMPLES: u32 = 48;
const EYE: [f32; 3] = [0.0, 0.5, 2.2];
const CENTRE: [f32; 3] = [0.0, 0.5, 0.0];
const FOV: f32 = 35.0;

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("scene-reflections")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ball.glb"), ball(json!({ "name": "ball" }), &[])).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(
        root.join("street.materials.toml"),
        "format = 1\n\n[materials.gloss]\nbase = [0.02, 0.02, 0.02]\nroughness = 0.05\nspecular = 0.04\nclearcoat = 1.0\nclearcoat_roughness = 0.03\n\n[materials.wall]\nbase = [0.04, 0.04, 0.045]\nroughness = 0.9\nspecular = 0.02\n",
    )
    .unwrap();
    let path = root.join("street.scene.toml");
    std::fs::write(
        &path,
        format!(
            "format = 1\nmaterials = [\"street.materials.toml\"]\n\n[mesh.ball]\nfile = \"ball.glb\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"ball\"\nmesh = \"ball\"\nmaterial = \"gloss\"\nat = {CENTRE:?}\n\n[[object]]\nname = \"left\"\nmesh = \"block\"\nmaterial = \"wall\"\nat = [-1.4, 3.0, 0.0]\nscale = [0.2, 6.0, 30.0]\n\n[[object]]\nname = \"right\"\nmesh = \"block\"\nmaterial = \"wall\"\nat = [1.4, 3.0, 0.0]\nscale = [0.2, 6.0, 30.0]\n\n[[object]]\nname = \"road\"\nmesh = \"block\"\nmaterial = \"wall\"\nat = [0.0, -0.05, 0.0]\nscale = [3.0, 0.1, 30.0]\n\n[sun]\nmodel = \"authored\"\ntoward = [0.1, 0.08, -1.0]\ncolor = [1.0, 0.7, 0.45]\nirradiance = 1.0\n\n[sky]\nkind = \"analytic\"\n\n[camera]\nat = {EYE:?}\nlook_at = {CENTRE:?}\nfov = {FOV:?}\n"
        ),
    )
    .unwrap();
    path
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene reflections output"),
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
    drawn(scene, false)
}

fn drawn(scene: &Scene, background: bool) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer.set_background_reflections(background);
    let mut staged = renderer.stage(scene).unwrap();
    assert_eq!(renderer.wait_reflections().unwrap(), background);
    assert!(!renderer.reflections_pending());
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
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/scene-reflections");
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder.write_header().unwrap();
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            p.map(|v| (pfx_materials::encode_channel(v.clamp(0.0, 1.0)) * 255.0).round() as u8)
        })
        .collect();
    writer.write_image_data(&bytes).unwrap();
}

fn luminance(p: &[f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn normal_at(x: u32, y: u32) -> Option<[f32; 3]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |a: [f32; 3]| {
        let l = dot(a, a).sqrt();
        a.map(|v| v / l)
    };
    let forward = unit(sub(CENTRE, EYE));
    let right = unit([-forward[2], 0.0, forward[0]]);
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    let tan = (FOV * 0.5).to_radians().tan();
    let sx = ((x as f32 + 0.5) / WIDTH as f32 * 2.0 - 1.0) * tan * WIDTH as f32 / HEIGHT as f32;
    let sy = (1.0 - (y as f32 + 0.5) / HEIGHT as f32 * 2.0) * tan;
    let ray = unit(std::array::from_fn(|k| {
        forward[k] + right[k] * sx + up[k] * sy
    }));
    let oc = sub(EYE, CENTRE);
    let b = dot(oc, ray);
    let c = dot(oc, oc) - BALL_RADIUS * BALL_RADIUS;
    let h = b * b - c;
    if h <= 0.0 {
        return None;
    }
    let t = -b - h.sqrt();
    let hit: [f32; 3] = std::array::from_fn(|k| EYE[k] + ray[k] * t);
    Some(unit(sub(hit, CENTRE)))
}

fn flanks(pixels: &[[f32; 3]]) -> f32 {
    let mut sum = 0.0;
    let mut count = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if let Some(n) = normal_at(x, y)
                && n[0].abs() > 0.75
                && n[0].abs() < 0.93
                && n[1].abs() < 0.3
            {
                sum += luminance(&pixels[(y * WIDTH + x) as usize]);
                count += 1;
            }
        }
    }
    assert!(count > 100, "{count} flank pixels");
    sum / count as f32
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_glossy_sphere_between_dark_walls_reflects_the_walls_not_the_sky() {
    let scene = Scene::open(folder("street")).unwrap();
    let live = live(&scene);
    let traced = traced(&scene);
    save("street-live", &live);
    save("street-traced", &traced);
    let (a, b) = (flanks(&live), flanks(&traced));
    println!("flanks: live {a:.4} traced {b:.4}");
    assert!(
        (a - b).abs() <= b * 0.35 + 0.002,
        "the sphere's flanks read {a} live, {b} traced"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_scene_reflection_captured_in_the_background_draws_the_same_frames() {
    let scene = Scene::open(folder("background")).unwrap();
    assert!(drawn(&scene, true) == live(&scene));
}
