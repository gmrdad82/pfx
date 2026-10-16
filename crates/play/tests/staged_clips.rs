use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::fixture::{TIMELINE_CLIP, TIMELINE_NODE, timeline_glb, timeline_x};
use pfx_load::scene::{DepthOfField, Scene};
use pfx_play::{PlaySession, SceneGame};

mod common;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;
const TICKS: [u32; 2] = [15, 30];
const AT: [f32; 3] = [0.0, 0.6, 0.5];
const LAMP: f32 = 0.4;

fn scene_text(particles: bool) -> String {
    let mut text = format!(
        "format = 1\n\ninclude = [\"room.scene.toml\"]\n\n[mesh.timeline]\nfile = \"timeline.glb\"\n\n[[object]]\nname = \"runner\"\nmesh = \"timeline\"\nat = {AT:?}\nmaterial = \"clay\"\n\n[object.animation]\nclip = \"{TIMELINE_CLIP}\"\n\n[[light]]\nname = \"lamp\"\nparent = \"runner\"\nnode = \"{TIMELINE_NODE}\"\nat = [0.0, {LAMP:?}, 0.0]\ncolor = [1.0, 0.6, 0.3]\nintensity = 4.0\nrange = 3.0\n"
    );
    if particles {
        text.push_str(&format!(
            "\n[[particles]]\nname = \"trail\"\nparent = \"runner\"\nnode = \"{TIMELINE_NODE}\"\nrate = 400.0\nlifetime = [0.02, 0.02]\nspeed = [0.0, 0.0]\nsize = [0.05, 0.05]\n"
        ));
    }
    text
}

fn scene(name: &str, particles: bool) -> Scene {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    std::fs::write(root.join("timeline.glb"), timeline_glb()).unwrap();
    std::fs::write(root.join("clip.scene.toml"), scene_text(particles)).unwrap();
    Scene::open(root.join("clip.scene.toml")).unwrap()
}

fn renderer(gpu: &Gpu) -> Renderer {
    let mut renderer = Renderer::new(gpu.clone(), WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    renderer
}

fn rate(scene: &Scene) -> f32 {
    scene.physics_or_default().rate.round().max(1.0)
}

fn played(gpu: &Gpu, scene: &Scene, output: &OffscreenTarget) -> Vec<Vec<u16>> {
    let mut renderer = renderer(gpu);
    let staged = renderer.stage(scene).unwrap();
    renderer.wait_sky().unwrap();
    let finish = staged.finish();
    let mut session = PlaySession::play(scene, &staged, Box::new(SceneGame)).unwrap();
    session.pause();
    let mut done = 0;
    let aspect = WIDTH as f32 / HEIGHT as f32;
    TICKS
        .iter()
        .map(|&ticks| {
            while done < ticks {
                assert!(session.step());
                done += 1;
            }
            assert_eq!(session.time(), ticks as f32 / rate(scene));
            let drawn = session
                .present(&staged, &mut renderer, aspect, SEED)
                .unwrap();
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
            gpu.readback_rgba16(output).unwrap()
        })
        .collect()
}

fn staged(gpu: &Gpu, scene: &Scene, output: &OffscreenTarget) -> Vec<Vec<u16>> {
    let mut renderer = renderer(gpu);
    let mut staged = renderer.stage(scene).unwrap();
    renderer.wait_sky().unwrap();
    renderer.resize(WIDTH, HEIGHT).unwrap();
    let finish = staged.finish();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    staged.pose(TICKS[0] as f32 / rate(scene));
    TICKS
        .iter()
        .map(|&ticks| {
            let drawn = staged.frame(aspect, ticks as f32 / rate(scene), SEED);
            renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
                )
                .unwrap();
            gpu.readback_rgba16(output).unwrap()
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_staged_frame_poses_a_node_clip_as_play_does() {
    let scene = scene("staged-clip", false);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = common::output(&gpu, "staged clip output", [WIDTH, HEIGHT]);
    let play = played(&gpu, &scene, &output);
    let view = staged(&gpu, &scene, &output);
    assert_ne!(
        view[0], view[1],
        "the clip moves the box between the frames"
    );
    assert!(view[0] == play[0], "the first frame differs from play's");
    assert!(view[1] == play[1], "the second frame differs from play's");
}

fn near(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_attached_light_and_a_parented_spray_follow_a_staged_clip() {
    let mut scene = scene("staged-clip-follow", true);
    let mut camera = scene.camera_or_default();
    camera.depth_of_field = Some(DepthOfField {
        fstop: 2.8,
        distance: 3.0,
        focal: 0.05,
        keys: vec![[0.25, 2.0], [0.75, 4.0]],
    });
    scene.camera = Some(camera);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = renderer(&gpu);
    let mut staged = renderer.stage(&scene).unwrap();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    for time in [0.25f32, 0.75] {
        let x = AT[0] + timeline_x(time);
        let drawn = staged.frame(aspect, time, SEED);
        let lens = drawn
            .effects
            .lens
            .expect("focus keys set the lens each frame");
        assert!(
            near(lens.focus_m, 2.0 + 4.0 * (time - 0.25), 1e-5),
            "{lens:?}"
        );
        let lights = drawn
            .effects
            .lights
            .expect("the frame carries its posed lights");
        assert!(
            lights.iter().any(|light| near(light.position[0], x, 1e-4)
                && near(light.position[1], AT[1] + LAMP, 1e-4)
                && near(light.position[2], AT[2], 1e-4)),
            "no light rides the node at {time}: {lights:?}"
        );
        let sprays = drawn.effects.sprays;
        assert!(!sprays.is_empty(), "the trail spawns at {time}");
        let mean = sprays
            .iter()
            .map(|spray| spray.position_size[0])
            .sum::<f32>()
            / sprays.len() as f32;
        assert!(
            near(mean, x, 0.05),
            "the trail sits at {mean}, the node at {x}"
        );
    }
    staged.release(&mut renderer).unwrap();
}
