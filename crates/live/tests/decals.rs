use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::maps::{self, MapImages, MapTextures};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::{DECAL_SIDE, ball_png, ground, ground_lifted};
use pfx_load::scene::Scene;
use pfx_materials::{Crack, Material, NoiseLayer, Track};
use pfx_post::Chain;
use serde_json::{Value, json};

const SIDE: u32 = 128;
const HEIGHT: f32 = 2.0;
const SAMPLES: u32 = 32;
const FRAMES: u32 = 8;

fn folder(name: &str, glb: &[u8]) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("decals")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ground.glb"), glb).unwrap();
    std::fs::write(
        root.join("ground.scene.toml"),
        format!(
            "format = 1\n\n[mesh.ground]\nfile = \"ground.glb\"\n\n[[object]]\nname = \"ground\"\nmesh = \"ground\"\n\n[camera]\nat = [0.0, 4.0, 0.0]\nlook_at = [0.0, 0.0, 0.0]\nup = [0.0, 0.0, -1.0]\nprojection = \"orthographic\"\nheight = {HEIGHT}\n\n[sun]\nmodel = \"authored\"\ntoward = [0.25, 1.0, 0.35]\ncolor = [1.0, 1.0, 1.0]\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\n"
        ),
    )
    .unwrap();
    root.join("ground.scene.toml")
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("decals output"),
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
    for _ in 0..FRAMES {
        let finish = staged.finish();
        let drawn = staged.frame(1.0, 0.0, 0);
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
    let staged = pfx_trace::stage::scene_at(scene, 1.0, 0.0).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, SIDE, SIDE).unwrap();
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

fn both(name: &str, glb: &[u8]) -> [Vec<[f32; 3]>; 2] {
    let scene = Scene::open(&folder(name, glb)).unwrap();
    [live(&scene), traced(&scene)]
}

fn at(index: usize) -> [f32; 2] {
    let scale = HEIGHT / SIDE as f32;
    let x = ((index as u32 % SIDE) as f32 + 0.5 - SIDE as f32 * 0.5) * scale;
    let z = ((index as u32 / SIDE) as f32 + 0.5 - SIDE as f32 * 0.5) * scale;
    [x, z]
}

fn inside(index: usize) -> bool {
    let [x, z] = at(index);
    x.abs().max(z.abs()) < DECAL_SIDE * 0.4
}

fn outside(index: usize) -> bool {
    let [x, z] = at(index);
    x.abs().max(z.abs()) > DECAL_SIDE * 0.6
}

fn luminance(p: &[f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn mean(pixels: &[[f32; 3]], keep: impl Fn(usize) -> bool) -> f32 {
    let (sum, count) = pixels
        .iter()
        .enumerate()
        .filter(|(index, _)| keep(*index))
        .fold((0.0f64, 0u32), |(sum, count), (_, p)| {
            (sum + f64::from(luminance(p)), count + 1)
        });
    (sum / f64::from(count.max(1))) as f32
}

fn pbr(base: [f32; 4], roughness: f32) -> Value {
    json!({
        "pbrMetallicRoughness": {
            "baseColorFactor": base,
            "metallicFactor": 0.0,
            "roughnessFactor": roughness,
        },
    })
}

fn blended(base: [f32; 4], roughness: f32) -> Value {
    let mut material = pbr(base, roughness);
    material["alphaMode"] = json!("BLEND");
    material
}

const RENDERERS: [&str; 2] = ["live", "traced"];

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_blended_decal_darkens_and_polishes_the_ground_alike_in_both_renderers() {
    let asphalt = pbr([0.25, 0.25, 0.25, 1.0], 0.8);
    let bare = both("bare", &ground(asphalt.clone(), &[], &[]));
    let dark = both(
        "dark",
        &ground(
            asphalt.clone(),
            &[blended([0.02, 0.02, 0.02, 0.6], 0.8)],
            &[],
        ),
    );
    let glossy = both(
        "glossy",
        &ground(asphalt, &[blended([0.25, 0.25, 0.25, 0.6], 0.15)], &[]),
    );
    let mut darkening = [0.0; 2];
    let mut polish = [0.0; 2];
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let ground = mean(&bare[k], inside);
        darkening[k] = mean(&dark[k], inside) / ground;
        polish[k] = mean(&glossy[k], inside) / ground;
        let kept = mean(&dark[k], outside) / mean(&bare[k], outside);
        println!(
            "{renderer}: ground {ground}, dark decal {}, glossy decal {}, outside kept {kept}",
            darkening[k], polish[k]
        );
        assert!((kept - 1.0).abs() < 0.03, "{renderer}: outside {kept}");
    }
    assert!(darkening[1] < 0.75, "traced darkening {}", darkening[1]);
    assert!(
        (darkening[0] - darkening[1]).abs() < 0.02,
        "the decal darkens the ground to {} live and {} traced",
        darkening[0],
        darkening[1]
    );
    assert!(polish[1] < 0.98, "traced polish {}", polish[1]);
    assert!(
        (polish[0] - polish[1]).abs() < 0.01,
        "the decal's roughness changes the ground by {} live and {} traced",
        polish[0],
        polish[1]
    );
}

fn noise_png(side: u32, seed: u32, rgba: impl Fn(f32) -> [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ seed)
                .wrapping_mul(2_654_435_761);
            bytes.extend(rgba((h >> 8) as f32 / (1u32 << 24) as f32));
        }
    }
    ball_png(side, side, &bytes)
}

fn asphalt_png() -> Vec<u8> {
    noise_png(64, 7, |v| {
        let g = (70.0 + 60.0 * v) as u8;
        [g, g, g, 255]
    })
}

fn mapped(layers: &[NoiseLayer]) -> Value {
    let mut engine = Material {
        base: [1.0, 1.0, 1.0],
        roughness: 0.7,
        ..Material::default()
    };
    for layer in layers {
        engine.push_layer(*layer);
    }
    json!({
        "pbrMetallicRoughness": {
            "baseColorTexture": { "index": 0 },
            "metallicFactor": 0.0,
            "roughnessFactor": 0.7,
        },
        "extras": { "pfxMaterial": serde_json::to_value(engine).unwrap() },
    })
}

const CRACK: Crack = Crack {
    cell: 0.6,
    width: 0.04,
    warp: 0.05,
    warp_frequency: 1.5,
    density: 1.0,
    spread: 0.3,
    darkness: 0.85,
    roughness: 0.95,
    randomness: 1.0,
};

const TRACK: Track = Track {
    period: 2.0,
    offset: 1.0,
    gauge: 0.5,
    band: 0.12,
    wander: 0.0,
    wander_frequency: 0.04,
    streak: [6.0, 0.35],
    tint: 0.6,
    polish: 0.6,
};

fn worn(index: usize) -> f32 {
    let [x, z] = at(index);
    let up = [0.0, 1.0, 0.0];
    let crack = pfx_materials::crack([x, 0.0, z], up, CRACK, 0, 0.0);
    let track = pfx_materials::track([x + 2.0, z + 2.0], TRACK, 0, 0.0);
    crack.max(track)
}

fn near_wear(index: usize, reach: i32) -> bool {
    let (x, y) = ((index as u32 % SIDE) as i32, (index as u32 / SIDE) as i32);
    (-reach..=reach).any(|dy| {
        (-reach..=reach).any(|dx| {
            let (nx, ny) = (x + dx, y + dy);
            (0..SIDE as i32).contains(&nx)
                && (0..SIDE as i32).contains(&ny)
                && worn((ny as u32 * SIDE + nx as u32) as usize) > 0.02
        })
    })
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn pavement_layers_over_a_mapped_ground_change_it_only_where_they_say_in_both_renderers() {
    let images = [asphalt_png()];
    let plain = both("mapped", &ground(mapped(&[]), &[], &images));
    let layered = both(
        "mapped-layers",
        &ground(
            mapped(&[CRACK.layer(1.0, 0), TRACK.layer(1.0, 0)]),
            &[],
            &images,
        ),
    );
    let clear: Vec<usize> = (0..(SIDE * SIDE) as usize)
        .filter(|index| !near_wear(*index, 3))
        .collect();
    let wear: Vec<usize> = (0..(SIDE * SIDE) as usize)
        .filter(|index| worn(*index) > 0.7)
        .collect();
    assert!(
        clear.len() > 2000 && wear.len() > 400,
        "{} {}",
        clear.len(),
        wear.len()
    );
    for (k, renderer) in RENDERERS.iter().enumerate() {
        let ratio = |keep: &[usize]| {
            let a: f64 = keep
                .iter()
                .map(|&i| f64::from(luminance(&layered[k][i])))
                .sum();
            let b: f64 = keep
                .iter()
                .map(|&i| f64::from(luminance(&plain[k][i])))
                .sum();
            (a / b) as f32
        };
        let worst = clear
            .iter()
            .map(|&i| (luminance(&layered[k][i]) / luminance(&plain[k][i]).max(1e-4) - 1.0).abs())
            .fold(0.0f32, f32::max);
        let (kept, changed) = (ratio(&clear), ratio(&wear));
        println!("{renderer}: away from the wear {kept} (worst pixel {worst}), on it {changed}");
        assert!(
            (kept - 1.0).abs() < 0.01,
            "{renderer}: away from the wear {kept}"
        );
        assert!(changed < 0.8, "{renderer}: on the wear {changed}");
    }
}

fn chain(width: u32, height: u32) -> u64 {
    let mut total = 0;
    let (mut w, mut h) = (width, height);
    loop {
        total += u64::from(w) * u64::from(h) * 4;
        if w == 1 && h == 1 {
            return total;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn decal_maps_take_the_memory_of_their_own_pixels() {
    let asphalt = noise_png(256, 3, |v| {
        let g = (60.0 + 80.0 * v) as u8;
        [g, g, g, 255]
    });
    let bumps = noise_png(256, 5, |v| [(118.0 + 20.0 * v) as u8, 128, 250, 255]);
    let stain = noise_png(64, 9, |v| [20, 20, 20, (120.0 + 100.0 * v) as u8]);
    let skid = noise_png(32, 11, |v| [10, 10, 10, (200.0 * v) as u8]);
    let decal = |index: u32| {
        json!({
            "alphaMode": "BLEND",
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": index },
                "metallicFactor": 0.0,
                "roughnessFactor": 0.4,
            },
        })
    };
    let glb = ground(
        json!({
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicFactor": 0.0,
                "roughnessFactor": 0.8,
            },
            "normalTexture": { "index": 1 },
        }),
        &[decal(2), decal(3)],
        &[asphalt, bumps, stain, skid],
    );
    let scene = Scene::open(&folder("memory", &glb)).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, SIDE, SIDE).unwrap();
    assert_eq!(renderer.map_bytes(), 0);
    let staged = renderer.stage(&scene).unwrap();
    let held = renderer.map_bytes();
    let own = chain(256, 256) * 2 + chain(64, 64) + chain(32, 32);
    let largest = chain(256, 256) * 4;
    println!(
        "set {}: {held} bytes, own sizes {own}, at the largest {largest}",
        renderer.map_set()
    );
    if renderer.map_set() {
        assert_eq!(held, own);
    } else {
        assert_eq!(held, largest);
    }
    staged.release(&mut renderer).unwrap();
    assert_eq!(renderer.map_bytes(), 0);
}

fn in_sheet_shadow(index: usize) -> bool {
    let [x, z] = at(index);
    (-0.615..-0.52).contains(&x) && (-0.4..0.2).contains(&z)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_blended_sheet_above_the_ground_still_casts_its_coverage_of_shadow() {
    let asphalt = pbr([0.25, 0.25, 0.25, 1.0], 0.8);
    let mut sheet = blended([0.9, 0.9, 0.9, 0.6], 0.8);
    sheet["doubleSided"] = json!(true);
    let bare = both("sheet-bare", &ground(asphalt.clone(), &[], &[]));
    let lifted = both("sheet-lifted", &ground_lifted(asphalt, &[sheet], &[], 0.5));
    let mut shade = [0.0; 2];
    for (k, renderer) in RENDERERS.iter().enumerate() {
        shade[k] = mean(&lifted[k], in_sheet_shadow) / mean(&bare[k], in_sheet_shadow);
        let kept = mean(&lifted[k], outside) / mean(&bare[k], outside);
        println!(
            "{renderer}: in the sheet's shadow {}, elsewhere {kept}",
            shade[k]
        );
        assert!(
            shade[k] < 0.85,
            "{renderer}: the sheet's shadow keeps {}",
            shade[k]
        );
    }
    assert!(
        (shade[0] - shade[1]).abs() < 0.15,
        "the sheet's shadow keeps {} live and {} traced",
        shade[0],
        shade[1]
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn without_the_map_set_every_map_of_a_kind_takes_the_largest_size() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let flat = |width: u32, height: u32| {
        pfx_load::surface::from_rgba8(
            width,
            height,
            pfx_load::ColorSpace::Srgb,
            &vec![[80, 80, 80, 255]; (width * height) as usize],
        )
    };
    let base = [flat(64, 64), flat(16, 32)];
    let images = MapImages {
        base: &base,
        ..MapImages::default()
    };
    let arrays = MapTextures::new_in(&gpu.device, &gpu.queue, &images, None);
    assert!(arrays.is_err(), "an array of mixed sizes is refused");
    let same = [flat(64, 64), flat(64, 64)];
    let arrays = MapTextures::new_in(
        &gpu.device,
        &gpu.queue,
        &MapImages {
            base: &same,
            ..MapImages::default()
        },
        None,
    )
    .unwrap();
    assert!(arrays.set.is_none());
    assert_eq!(arrays.bytes(), chain(64, 64) * 2);
    let Some(layout) = maps::set_layout(&gpu.device) else {
        println!("this device has no binding arrays; the set is not tried");
        return;
    };
    let set = MapTextures::new_in(&gpu.device, &gpu.queue, &images, Some(&layout)).unwrap();
    assert!(set.set.is_some());
    assert_eq!(set.bytes(), chain(64, 64) + chain(16, 32));
    let empty = MapTextures::new_in(
        &gpu.device,
        &gpu.queue,
        &MapImages::default(),
        Some(&layout),
    )
    .unwrap();
    assert_eq!(empty.bytes(), 0);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_map_set_refuses_more_images_of_a_kind_than_its_slots() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let Some(layout) = maps::set_layout(&gpu.device) else {
        println!("this device has no binding arrays; the set is not tried");
        return;
    };
    let one =
        pfx_load::surface::from_rgba8(1, 1, pfx_load::ColorSpace::Linear, &[[128, 128, 255, 255]]);
    let many = vec![one; maps::MAP_SLOTS as usize + 1];
    let refused = MapTextures::new_in(
        &gpu.device,
        &gpu.queue,
        &MapImages {
            normal: &many,
            ..MapImages::default()
        },
        Some(&layout),
    );
    let Err(message) = refused else {
        panic!("{} normal maps were taken", many.len());
    };
    assert!(message.contains("at most 256"), "{message}");
    let full = MapTextures::new_in(
        &gpu.device,
        &gpu.queue,
        &MapImages {
            normal: &many[..maps::MAP_SLOTS as usize],
            ..MapImages::default()
        },
        Some(&layout),
    )
    .unwrap();
    assert_eq!(full.bytes(), 4 * u64::from(maps::MAP_SLOTS));
}
