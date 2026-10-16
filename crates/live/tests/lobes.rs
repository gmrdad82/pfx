use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::{BALL_RADIUS, ball, ball_png};
use pfx_load::scene::Scene;
use pfx_post::Chain;
use serde_json::{Value, json};

const SIDE: u32 = 128;
const HEIGHT: f32 = 1.2;
const SAMPLES: u32 = 24;

const MATERIALS: &str = "format = 1

[materials.plain]
base = [0.3, 0.05, 0.05]
roughness = 0.9
specular = 0.02

[materials.fabric]
base = [0.3, 0.05, 0.05]
roughness = 0.9
specular = 0.02
fabric = { color = [0.9, 0.9, 0.9], roughness = 0.5 }

[materials.soaked]
base = [0.3, 0.05, 0.05]
roughness = 0.9
specular = 0.02
fabric = { color = [0.9, 0.9, 0.9], roughness = 0.5 }
wet = { amount = 1.0 }

[materials.metal]
base = [0.8, 0.8, 0.8]
metalness = 1.0
roughness = 0.3

[materials.brushed]
base = [0.8, 0.8, 0.8]
metalness = 1.0
roughness = 0.3
anisotropy = { strength = 0.9, rotation = 0.0 }

[materials.skin]
base = [0.8, 0.6, 0.5]
roughness = 0.7
specular = 0.02

[materials.scattered]
base = [0.8, 0.6, 0.5]
roughness = 0.7
specular = 0.02
scatter = { color = [1.0, 0.3, 0.2], radius = 0.25, amount = 1.0 }
";

fn folder(name: &str, glb: &[u8], body: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("lobes")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ball.glb"), glb).unwrap();
    std::fs::write(root.join("ball.materials.toml"), MATERIALS).unwrap();
    std::fs::write(
        root.join("ball.scene.toml"),
        format!(
            "format = 1\nmaterials = [\"ball.materials.toml\"]\n\n[mesh.ball]\nfile = \"ball.glb\"\n\n[camera]\nat = [0.0, 0.0, 3.0]\nlook_at = [0.0, 0.0, 0.0]\nprojection = \"orthographic\"\nheight = {HEIGHT}\n\n{body}"
        ),
    )
    .unwrap();
    root.join("ball.scene.toml")
}

fn sun(toward: [f32; 3]) -> String {
    format!(
        "[sun]\nmodel = \"authored\"\ntoward = [{}, {}, {}]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n",
        toward[0], toward[1], toward[2]
    )
}

fn ball_object(material: Option<&str>) -> String {
    match material {
        Some(name) => {
            format!("[[object]]\nname = \"ball\"\nmesh = \"ball\"\nmaterial = \"{name}\"\n")
        }
        None => "[[object]]\nname = \"ball\"\nmesh = \"ball\"\n".to_string(),
    }
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lobes output"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
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
        width: SIDE,
        height: SIDE,
    }
}

fn live(scene: &Scene) -> Vec<[f32; 3]> {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu);
    let mut renderer = Renderer::new(gpu, SIDE, SIDE).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(Chain::new());
    for _ in 0..8 {
        let finish = staged.finish();
        let drawn = staged.frame(1.0, 0.0, 7);
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

fn traced(scene: &Scene, samples: u32) -> Vec<[f32; 3]> {
    let staged = pfx_trace::stage::scene_at(scene, 1.0, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, SIDE, SIDE).unwrap();
    for pass in 0..samples {
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

fn both(path: &Path, samples: u32) -> [Vec<[f32; 3]>; 2] {
    let scene = Scene::open(path).unwrap();
    [live(&scene), traced(&scene, samples)]
}

fn normal_at(index: usize) -> Option<[f32; 3]> {
    let x = (index as u32 % SIDE) as f32 + 0.5;
    let y = (index as u32 / SIDE) as f32 + 0.5;
    let scale = HEIGHT / SIDE as f32 / BALL_RADIUS;
    let nx = (x - SIDE as f32 * 0.5) * scale;
    let ny = (SIDE as f32 * 0.5 - y) * scale;
    let rr = nx * nx + ny * ny;
    (rr < 0.9).then(|| [nx, ny, (1.0 - rr).sqrt()])
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|value| value / length)
}

fn luminance(p: &[f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn region(pixels: &[[f32; 3]], keep: impl Fn([f32; 3]) -> bool) -> Vec<[f32; 3]> {
    pixels
        .iter()
        .enumerate()
        .filter_map(|(index, pixel)| normal_at(index).filter(|n| keep(*n)).map(|_| *pixel))
        .collect()
}

fn mean(pixels: &[[f32; 3]]) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    for pixel in pixels {
        for c in 0..3 {
            sum[c] += f64::from(pixel[c]);
        }
    }
    sum.map(|total| (total / pixels.len().max(1) as f64) as f32)
}

fn ripple(pixels: &[[f32; 3]], keep: impl Fn([f32; 3]) -> bool) -> f32 {
    let mut sum = 0.0;
    let mut count = 0;
    for (index, pixel) in pixels.iter().enumerate() {
        let right = index + 1;
        if right % SIDE as usize == 0 {
            continue;
        }
        let (Some(a), Some(b)) = (normal_at(index), normal_at(right)) else {
            continue;
        };
        if keep(a) && keep(b) {
            sum += (luminance(pixel) - luminance(&pixels[right])).abs();
            count += 1;
        }
    }
    sum / count.max(1) as f32
}

fn peak(pixels: &[[f32; 3]]) -> f32 {
    pixels.iter().map(luminance).fold(0.0, f32::max)
}

fn elongation(pixels: &[[f32; 3]]) -> f32 {
    let mut lit: Vec<f32> = pixels
        .iter()
        .enumerate()
        .filter(|(index, _)| normal_at(*index).is_some())
        .map(|(_, pixel)| luminance(pixel))
        .collect();
    lit.sort_by(f32::total_cmp);
    let top = lit[lit.len() * 98 / 100];
    let mut points = Vec::new();
    for (index, pixel) in pixels.iter().enumerate() {
        if normal_at(index).is_some() && luminance(pixel) > 0.5 * top {
            points.push([(index as u32 % SIDE) as f32, (index as u32 / SIDE) as f32]);
        }
    }
    let count = points.len().max(1) as f32;
    let cx = points.iter().map(|p| p[0]).sum::<f32>() / count;
    let cy = points.iter().map(|p| p[1]).sum::<f32>() / count;
    let (mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0);
    for p in &points {
        xx += (p[0] - cx).powi(2) / count;
        yy += (p[1] - cy).powi(2) / count;
        xy += (p[0] - cx) * (p[1] - cy) / count;
    }
    let mid = (xx + yy) * 0.5;
    let radius = ((xx - yy) * (xx - yy) * 0.25 + xy * xy).sqrt();
    ((mid + radius) / (mid - radius).max(1e-3)).sqrt()
}

fn plain_ball() -> Vec<u8> {
    ball(json!({ "name": "ball" }), &[])
}

fn render_material(name: &str, material: &str, toward: [f32; 3]) -> [Vec<[f32; 3]>; 2] {
    let path = folder(
        name,
        &plain_ball(),
        &format!(
            "{}\n[sky]\nkind = \"analytic\"\n\n{}",
            sun(toward),
            ball_object(Some(material))
        ),
    );
    both(&path, SAMPLES)
}

const RENDERERS: [&str; 2] = ["live", "traced"];

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn sheen_brightens_the_rim_in_both_renderers() {
    let toward = [0.2, 0.3, 1.0];
    let plain = render_material("sheen-off", "plain", toward);
    let fabric = render_material("sheen-on", "fabric", toward);
    let rim = |n: [f32; 3]| n[2] < 0.45;
    let mut gains = Vec::new();
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let off = luminance(&mean(&region(&plain[k], rim)));
        let on = luminance(&mean(&region(&fabric[k], rim)));
        assert!(on > off * 1.3, "{renderer}: rim {on} against {off}");
        gains.push(on / off);
    }
    assert!(
        gains[0] / gains[1] < 1.6 && gains[1] / gains[0] < 1.6,
        "the renderers' sheen gains differ: {gains:?}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_lobed_material_still_soaks_in_both_renderers() {
    let toward = [0.2, 0.3, 1.0];
    let dry = render_material("soak-dry", "fabric", toward);
    let wet = render_material("soak-wet", "soaked", toward);
    let body = |n: [f32; 3]| n[2] > 0.6 && n[1] < 0.0;
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let before = mean(&region(&dry[k], body));
        let after = mean(&region(&wet[k], body));
        assert!(
            after[0] < before[0] * 0.9,
            "{renderer}: soaked {after:?} against {before:?}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn anisotropy_stretches_the_highlight_in_both_renderers() {
    let toward = [0.3, 0.3, 1.0];
    let round = render_material("anisotropy-off", "metal", toward);
    let brushed = render_material("anisotropy-on", "brushed", toward);
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let off = elongation(&round[k]);
        let on = elongation(&brushed[k]);
        assert!(on > off * 1.5, "{renderer}: elongation {on} against {off}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn scatter_warms_the_terminator_in_both_renderers() {
    let toward = [1.0, 0.0, 0.25];
    let skin = render_material("scatter-off", "skin", toward);
    let scattered = render_material("scatter-on", "scattered", toward);
    let light = unit(toward);
    let band = |n: [f32; 3]| {
        let ndl = n[0] * light[0] + n[1] * light[1] + n[2] * light[2];
        (0.0..0.2).contains(&ndl)
    };
    let mut gains = Vec::new();
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let off = mean(&region(&skin[k], band));
        let on = mean(&region(&scattered[k], band));
        assert!(
            on[0] > off[0] * 1.2,
            "{renderer}: red {on:?} against {off:?}"
        );
        assert!(
            on[0] / on[1].max(1e-6) > off[0] / off[1].max(1e-6) * 1.1,
            "{renderer}: the band is not warmer, {on:?} against {off:?}"
        );
        gains.push(on[0] / off[0]);
    }
    assert!(
        gains[0] / gains[1] < 1.6 && gains[1] / gains[0] < 1.6,
        "the renderers' scatter gains differ: {gains:?}"
    );
}

fn flat(width: u32, height: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.extend(texel(x, y));
        }
    }
    ball_png(width, height, &rgba)
}

fn bumps() -> Vec<u8> {
    flat(64, 64, |x, y| {
        let tilt = if (x / 4 + y / 4) % 2 == 0 { 40 } else { 216 };
        [tilt, 128, 200, 255]
    })
}

fn textured(name: &str, material: Value, images: &[Vec<u8>], body: &str) -> [Vec<[f32; 3]>; 2] {
    let path = folder(
        name,
        &ball(material, images),
        &format!("{body}\n{}", ball_object(None)),
    );
    both(&path, SAMPLES)
}

fn pbr(extra: Value) -> Value {
    let mut material = json!({
        "name": "ball",
        "pbrMetallicRoughness": { "baseColorFactor": [0.7, 0.7, 0.7, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.8 },
    });
    for (key, value) in extra.as_object().unwrap() {
        if key == "pbrMetallicRoughness" {
            for (inner, value) in value.as_object().unwrap() {
                material["pbrMetallicRoughness"][inner] = value.clone();
            }
        } else {
            material[key] = value.clone();
        }
    }
    material
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_base_colour_map_paints_the_ball_in_both_renderers() {
    let stripes = flat(16, 16, |_, y| {
        if y < 8 {
            [230, 20, 20, 255]
        } else {
            [20, 20, 230, 255]
        }
    });
    let lit = sun([0.0, 0.0, 1.0]);
    let pixels = textured(
        "base",
        pbr(json!({ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } })),
        std::slice::from_ref(&stripes),
        &lit,
    );
    let moved = textured(
        "base-moved",
        pbr(
            json!({ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0, "extensions": { "KHR_texture_transform": { "offset": [0.0, 0.5] } } } } }),
        ),
        &[stripes],
        &lit,
    );
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let top = mean(&region(&pixels[k], |n| n[1] > 0.3));
        let bottom = mean(&region(&pixels[k], |n| n[1] < -0.3));
        assert!(top[0] > top[2] * 3.0, "{renderer}: top {top:?}");
        assert!(bottom[2] > bottom[0] * 3.0, "{renderer}: bottom {bottom:?}");
        let top = mean(&region(&moved[k], |n| n[1] > 0.3));
        assert!(top[2] > top[0] * 3.0, "{renderer}: moved top {top:?}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn normal_and_detail_maps_roughen_the_shading_in_both_renderers() {
    let lit = sun([0.4, 0.4, 1.0]);
    let smooth = textured("bare", pbr(json!({})), &[], &lit);
    let normal = textured(
        "normal",
        pbr(json!({ "normalTexture": { "index": 0, "scale": 1.0 } })),
        &[bumps()],
        &lit,
    );
    let detail = textured(
        "detail",
        pbr(
            json!({ "extras": { "detailNormalTexture": { "index": 0, "texCoord": 1, "scale": 1.0, "extensions": { "KHR_texture_transform": { "scale": [2.0, 2.0] } } } } }),
        ),
        &[bumps()],
        &lit,
    );
    let lit_side = |n: [f32; 3]| n[2] > 0.5;
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let base = ripple(&smooth[k], lit_side);
        let mapped = ripple(&normal[k], lit_side);
        let detailed = ripple(&detail[k], lit_side);
        assert!(
            mapped > base * 2.0,
            "{renderer}: normal map {mapped} against {base}"
        );
        assert!(
            detailed > base * 2.0,
            "{renderer}: detail map {detailed} against {base}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn metallic_roughness_and_occlusion_maps_act_in_both_renderers() {
    let lit = sun([0.3, 0.3, 1.0]);
    let rough = flat(8, 8, |_, _| [255, 255, 0, 255]);
    let glossy = flat(8, 8, |_, _| [255, 50, 0, 255]);
    let metal = flat(8, 8, |_, _| [255, 255, 255, 255]);
    let with = |name: &str, image: Vec<u8>| {
        textured(
            name,
            pbr(
                json!({ "pbrMetallicRoughness": { "metallicFactor": 1.0, "roughnessFactor": 1.0, "metallicRoughnessTexture": { "index": 0 } } }),
            ),
            &[image],
            &lit,
        )
    };
    let rough = with("rough", rough);
    let glossy = with("glossy", glossy);
    let metal = with("metal", metal);
    let dielectric = textured(
        "dielectric",
        pbr(json!({ "pbrMetallicRoughness": { "roughnessFactor": 1.0 } })),
        &[],
        &lit,
    );
    let sky = "[sky]\nkind = \"analytic\"\n";
    let open = textured("open", pbr(json!({})), &[], sky);
    let shaded = textured(
        "shaded",
        pbr(json!({ "occlusionTexture": { "index": 0, "strength": 1.0 } })),
        &[flat(8, 8, |_, _| [64, 64, 64, 255])],
        sky,
    );
    let ball_region = |_: [f32; 3]| true;
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let sharp = peak(&region(&glossy[k], ball_region));
        let broad = peak(&region(&rough[k], ball_region));
        assert!(
            sharp > broad * 1.5,
            "{renderer}: glossy peak {sharp} against {broad}"
        );
        let away = |n: [f32; 3]| n[1] < -0.5;
        let metal_side = luminance(&mean(&region(&metal[k], away)));
        let plain_side = luminance(&mean(&region(&dielectric[k], away)));
        assert!(
            metal_side < plain_side * 0.8,
            "{renderer}: metal {metal_side} against {plain_side}"
        );
        let bright = luminance(&mean(&region(&open[k], ball_region)));
        let dark = luminance(&mean(&region(&shaded[k], ball_region)));
        assert!(
            bright > 0.0 && dark < bright * 0.6,
            "{renderer}: occluded {dark} against {bright}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_glb_reads_its_sheen_and_anisotropy_extensions() {
    let toward = [0.2, 0.3, 1.0];
    let lit = sun(toward);
    let plain = textured("glb-plain", pbr(json!({})), &[], &lit);
    let sheen = textured(
        "glb-sheen",
        pbr(
            json!({ "extensions": { "KHR_materials_sheen": { "sheenColorFactor": [0.9, 0.9, 0.9], "sheenRoughnessFactor": 0.5 } } }),
        ),
        &[],
        &lit,
    );
    let metal =
        json!({ "pbrMetallicRoughness": { "metallicFactor": 1.0, "roughnessFactor": 0.3 } });
    let round = textured("glb-round", pbr(metal.clone()), &[], &lit);
    let mut brushed = pbr(metal);
    brushed["extensions"] = json!({ "KHR_materials_anisotropy": { "anisotropyStrength": 0.9, "anisotropyRotation": 0.0 } });
    let brushed = textured("glb-brushed", brushed, &[], &lit);
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let off = luminance(&mean(&region(&plain[k], |n| n[2] < 0.45)));
        let on = luminance(&mean(&region(&sheen[k], |n| n[2] < 0.45)));
        assert!(on > off * 1.2, "{renderer}: sheen rim {on} against {off}");
        let stretched = elongation(&brushed[k]);
        let circle = elongation(&round[k]);
        assert!(
            stretched > circle * 1.5,
            "{renderer}: elongation {stretched} against {circle}"
        );
    }
}
