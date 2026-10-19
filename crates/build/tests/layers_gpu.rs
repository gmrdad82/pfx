mod common;

use common::scratch;
use pfx_build::{Registry, write};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_recipe_scene::pace::{TurnState, turn_soon};
use pfx_recipe_scene::{SETTLE_FRAMES, headless};

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
    let folder = scratch(&format!("layers-gpu-{name}"));
    let text = if bake {
        format!("{PATCH}bake_layers = {{ size = 256 }}\n")
    } else {
        PATCH.to_string()
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

fn live(gpu: &Gpu, scene: &Scene) -> Vec<u8> {
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
    let layered = mean(&live(&gpu, &emitted("layered", false)));
    let baked = mean(&live(&gpu, &emitted("baked", true)));
    eprintln!("layered {layered:?} baked {baked:?}");
    for c in 0..3 {
        assert!(
            (layered[c] - baked[c]).abs() <= 1.0,
            "channel {c}: layered {layered:?}, baked {baked:?}"
        );
    }
}
