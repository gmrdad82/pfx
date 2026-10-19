mod common;

use common::scratch;
use pfx_build::{Registry, write};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_materials::NoiseKind;
use pfx_recipe_scene::pace::{TurnState, turn_soon};
use pfx_recipe_scene::{SETTLE_FRAMES, headless, shoot};
use pfx_still::{Request, View, looked_with};
use std::path::{Path, PathBuf};

const SIZE: [u32; 2] = [256, 256];
const SEED: u32 = 7;
const CROP: u32 = 64;

const PATCH: &str = r#"recipe = 1
name = "patch"

[[camera]]
name = "main"
kind = "perspective"
position = [0.0, 0.0, 1.6]
target = [0.0, 0.0, 0.0]
up = [0.0, 1.0, 0.0]
fov_y_deg = 40.0
fstop = 0.0

[sun]
model = "authored"
toward = [0.3, 0.5, 0.8]
radius_deg = 0.8
color = [1.0, 0.95, 0.9]
irradiance = 2.0

[sky]
kind = "analytic"
intensity = 0.8

[look]
exposure = 1.0

[materials.warm]
base = [0.6, 0.24, 0.13]
roughness = 0.6
specular = 0.16
sheen = 0.15
subsurface = 0.3
subsurface_tint = [1.0, 0.45, 0.32]
clearcoat = 0.2
layers = [
  { kind = "wall_mottle", frequency = 1.0, amplitude = 1.0, seed = 3, params = [2.5, 7.0, 400.0, 1400.0, 250.0, 36.0, 0.12, 0.05, 0.016, 0.006, 0.0, 0.0] },
  { kind = "grime", frequency = 1.0, amplitude = 0.8, seed = 5, params = [160.0, 800.0, 0.9, 0.84, 0.76, 1.03, 1.02, 1.0, 1.2, 0.65, 0.0, 0.0] },
]

[[object]]
name = "patch"
kind = "plane"
size = [1.2, 1.2]
rotate = [90.0, 0.0, 0.0]
material = "warm"
"#;

fn emitted(name: &str, bake: bool) -> Scene {
    emitted_from(name, PATCH, bake)
}

fn emitted_from(name: &str, patch: &str, bake: bool) -> Scene {
    let folder = scratch(&format!("layers-gpu-{name}"));
    let text = if bake {
        format!("{patch}bake_layers = {{ size = 256 }}\n")
    } else {
        patch.to_string()
    };
    let recipe = folder.join("patch.toml");
    std::fs::write(&recipe, text).unwrap();
    let (mut r, built) = pfx_recipe_scene::load(&recipe, None, &[], &Registry::new()).unwrap();
    r.output.glb = Some("patch.glb".into());
    r.output.scene = Some("patch.scene.toml".into());
    r.output.scene_camera = Some("main".into());
    write(&built, &r, &folder, Some(&folder)).unwrap();
    let scene = Scene::open(folder.join("patch.scene.toml")).unwrap();
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
    scene
}

fn live_frame(gpu: &Gpu, scene: &Scene) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = gpu.offscreen(SIZE[0], SIZE[1], format).unwrap();
    let mut renderer = Renderer::new_with_output_format(gpu.clone(), SIZE[0], SIZE[1], format)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    let finish = staged.finish();
    let mut turns = TurnState::default();
    for frame in 0..SETTLE_FRAMES {
        let drawn = staged.frame(
            SIZE[0] as f32 / SIZE[1] as f32,
            frame as f32 / pfx_recipe_scene::FRAME_RATE,
            SEED,
        );
        let timings = renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
        let measured: f64 = timings.iter().map(|t| t.milliseconds).sum();
        turn_soon(
            &mut turns,
            if timings.is_empty() {
                pfx_recipe_scene::UNMEASURED_MS
            } else {
                measured
            },
        );
    }
    gpu.readback_rgba8(&target).unwrap()
}

fn mean(rgba: &[u8]) -> [f64; 3] {
    let [w, h] = SIZE;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in (h - CROP) / 2..(h + CROP) / 2 {
        for x in (w - CROP) / 2..(w + CROP) / 2 {
            let at = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                sum[c] += f64::from(rgba[at + c]);
            }
            count += 1.0;
        }
    }
    sum.map(|s| s / count)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_baked_layered_patch_draws_live_in_the_layered_materials_colour() {
    let gpu = headless().unwrap();
    let layered = mean(&live_frame(&gpu, &emitted("layered", false)));
    let baked = mean(&live_frame(&gpu, &emitted("baked", true)));
    eprintln!("layered {layered:?} baked {baked:?}");
    for c in 0..3 {
        assert!(
            (layered[c] - baked[c]).abs() <= 1.0,
            "channel {c}: layered {layered:?}, baked {baked:?}"
        );
    }
}

fn baked_view(name: &str) -> PathBuf {
    let folder = scratch(&format!("layers-gpu-view-{name}"));
    let recipe = folder.join("patch.toml");
    std::fs::write(&recipe, format!("{PATCH}bake_layers = {{ size = 256 }}\n")).unwrap();
    let (mut r, built) = pfx_recipe_scene::load(&recipe, None, &[], &Registry::new()).unwrap();
    r.output.glb = Some("patch.glb".into());
    write(&built, &r, &folder, Some(&folder)).unwrap();
    let (head, _) = PATCH.split_once("[[object]]").unwrap();
    let view = folder.join("view.toml");
    std::fs::write(
        &view,
        format!("{head}[[object]]\nname = \"patch\"\nkind = \"mesh\"\npath = \"patch.glb\"\n"),
    )
    .unwrap();
    view
}

fn layered_recipe(name: &str) -> PathBuf {
    let folder = scratch(&format!("layers-gpu-recipe-{name}"));
    let recipe = folder.join("patch.toml");
    std::fs::write(&recipe, PATCH).unwrap();
    recipe
}

fn recipe_live(gpu: &Gpu, recipe: &Path) -> [f64; 3] {
    let (r, built) = pfx_recipe_scene::load(recipe, None, &[], &Registry::new()).unwrap();
    mean(&shoot(gpu, &r, &built.parts, "main", SIZE).unwrap())
}

fn recipe_traced(recipe: &Path) -> [f64; 3] {
    let rendered = pfx_still::render(
        &Request {
            recipe,
            camera: "main",
            size: SIZE,
            samples: TRACED_SAMPLES,
            noise: None,
            seed: Some(SEED),
            view: View::Lens,
            variant: None,
            set: &[],
            pick: &pfx_recipe::Pick::default(),
            crop: None,
            denoise: false,
            time: None,
        },
        &Registry::new(),
        &mut |_| {},
    )
    .unwrap();
    mean(&looked_with(&rendered.film, &rendered.finish, false).unwrap())
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_recipe_that_imports_a_baked_glb_draws_in_the_layered_materials_colour() {
    let gpu = headless().unwrap();
    let layered = layered_recipe("layered");
    let view = baked_view("baked");
    let live = (recipe_live(&gpu, &layered), recipe_live(&gpu, &view));
    eprintln!("recipe live: layered {:?} baked {:?}", live.0, live.1);
    let traced = (recipe_traced(&layered), recipe_traced(&view));
    eprintln!("recipe traced: layered {:?} baked {:?}", traced.0, traced.1);
    for c in 0..3 {
        assert!(
            (live.0[c] - live.1[c]).abs() <= 1.0,
            "live channel {c}: layered {:?}, baked {:?}",
            live.0,
            live.1
        );
        assert!(
            (traced.0[c] - traced.1[c]).abs() <= TRACED_LEVELS,
            "traced channel {c}: layered {:?}, baked {:?}",
            traced.0,
            traced.1
        );
    }
}

fn linear_target(gpu: &Gpu) -> pfx_gpu::OffscreenTarget {
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("layer kinds output"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    pfx_gpu::OffscreenTarget {
        texture,
        view,
        format,
        width: SIZE[0],
        height: SIZE[1],
    }
}

fn live_linear(gpu: &Gpu, scene: &Scene) -> [f64; 3] {
    let target = linear_target(gpu);
    let mut renderer = Renderer::new(gpu.clone(), SIZE[0], SIZE[1])
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    renderer.set_finish(pfx_post::Chain::new());
    let mut turns = TurnState::default();
    for _ in 0..SETTLE_FRAMES {
        let finish = staged.finish();
        let drawn = staged.frame(SIZE[0] as f32 / SIZE[1] as f32, 0.0, SEED);
        let timings = renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
        let measured: f64 = timings.iter().map(|t| t.milliseconds).sum();
        turn_soon(
            &mut turns,
            if timings.is_empty() {
                pfx_recipe_scene::UNMEASURED_MS
            } else {
                measured
            },
        );
    }
    let pixels: Vec<f32> = gpu
        .readback_rgba16(&target)
        .unwrap()
        .into_iter()
        .map(|bits| pfx_materials::linear_channel(half::f16::from_bits(bits).to_f32()))
        .collect();
    staged.release(&mut renderer).unwrap();
    linear_mean(&pixels)
}

fn traced(gpu: &Gpu, scene: &Scene) -> [f64; 3] {
    let staged = pfx_trace::stage::scene_at(scene, SIZE[0] as f32 / SIZE[1] as f32, 0.0).unwrap();
    let mut trace = staged.trace(gpu, SIZE[0], SIZE[1]).unwrap();
    let mut turns = TurnState::default();
    for pass in 0..TRACED_SAMPLES {
        trace.sample(gpu, 1, 11 + pass).unwrap();
        turn_soon(&mut turns, pfx_recipe_scene::UNMEASURED_MS);
    }
    let pixels: Vec<f32> = trace
        .readback(gpu)
        .unwrap()
        .color
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    linear_mean(&pixels)
}

fn linear_mean(rgba: &[f32]) -> [f64; 3] {
    let [w, h] = SIZE;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in (h - CROP) / 2..(h + CROP) / 2 {
        for x in (w - CROP) / 2..(w + CROP) / 2 {
            let at = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                sum[c] += f64::from(rgba[at + c]);
            }
            count += 1.0;
        }
    }
    sum.map(|s| s / count)
}

const TRACED_SAMPLES: u32 = 32;
const TRACED_GAIN: f64 = 0.02;
const TRACED_LEVELS: f64 = 2.0;
const MATTE: &str = "base = [0.6, 0.45, 0.3]\nroughness = 0.6\nspecular = 0.04\n";

fn kind_patch(layer: Option<(&str, f32)>) -> String {
    let layers = match layer {
        Some((kind, frequency)) => format!(
            "layers = [\n  {{ kind = \"{kind}\", frequency = {frequency:?}, amplitude = 0.8, seed = 3 }},\n]\n"
        ),
        None => String::new(),
    };
    let (head, _) = PATCH.split_once("[materials.warm]").unwrap();
    let head = head.replace("fov_y_deg = 40.0", "fov_y_deg = 4.0");
    format!(
        "{head}[materials.warm]\n{MATTE}{layers}\n[[object]]\nname = \"patch\"\nkind = \"plane\"\nsize = [0.12, 0.12]\nrotate = [90.0, 0.0, 0.0]\nmaterial = \"warm\"\n"
    )
}

fn gain(layered: [f64; 3], plain: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|c| layered[c] / plain[c])
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn every_layer_kind_draws_live_as_it_bakes_and_as_it_traces() {
    let gpu = headless().unwrap();
    let plain = emitted_from("kind-plain", &kind_patch(None), false);
    let (plain_live, plain_traced) = (live_linear(&gpu, &plain), traced(&gpu, &plain));
    let mut failed = Vec::new();
    for kind in NoiseKind::ALL {
        if kind == NoiseKind::None || kind == NoiseKind::Flow {
            continue;
        }
        let name = serde_json::to_value(kind).unwrap();
        let name = name.as_str().unwrap();
        let patch = kind_patch(Some((name, 120.0)));
        let layered = emitted_from(&format!("kind-{name}"), &patch, false);
        let traced = gain(traced(&gpu, &layered), plain_traced);
        let baked = emitted_from(&format!("kind-{name}-baked"), &patch, true);
        let baked_gain = gain(live_linear(&gpu, &baked), plain_live);
        eprintln!("{name}: gain baked {baked_gain:.4?} traced {traced:.4?}");
        if (0..3).any(|c| (baked_gain[c] - traced[c]).abs() > TRACED_GAIN) {
            failed.push(format!(
                "{name}: gain baked {baked_gain:.4?} against traced {traced:.4?}"
            ));
        }
        if kind == NoiseKind::CoatWobble {
            continue;
        }
        let live = gain(live_linear(&gpu, &layered), plain_live);
        eprintln!("{name}: gain live {live:.4?}");
        if (0..3).any(|c| (live[c] - traced[c]).abs() > TRACED_GAIN) {
            failed.push(format!(
                "{name}: gain live {live:.4?} against traced {traced:.4?}"
            ));
        }
        let drawn = mean(&live_frame(&gpu, &layered));
        let shown = mean(&live_frame(&gpu, &baked));
        eprintln!("{name}: live {drawn:.2?} baked {shown:.2?}");
        if (0..3).any(|c| (drawn[c] - shown[c]).abs() > 1.0) {
            failed.push(format!(
                "{name}: live {drawn:.2?} against baked {shown:.2?}"
            ));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
