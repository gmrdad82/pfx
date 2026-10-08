use std::path::{Path, PathBuf};

use half::f16;
use pfx_core::clock::Tick;
use pfx_gpu::pace::Pace;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_input::Input;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::Scene;
use pfx_play::{Game, Options, PlaySession, Rigs, World};

mod common;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;
const FRAME: f32 = 1.0 / 60.0;

const RIGS: &str = r#"
[[rig]]
name = "chase"
kind = "follow3d"
target = "crate"
arm = [0.0, 1.5, 6.0]
damping = 0.0
blend = 0.0
"#;

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    root
}

struct Mover;

impl Game for Mover {
    fn start(&mut self, world: &mut World) {
        let _ = world.rig("chase");
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let id = world.object("crate").unwrap();
        let mut at = world.at(id);
        at[0] += 0.03;
        at[2] -= 0.01;
        world.move_to(id, at);
    }
}

struct Viewer {
    renderer: Renderer,
    output: OffscreenTarget,
    pace: Pace<pfx_gpu::pace::WallClock>,
}

impl Viewer {
    fn draw(&mut self, session: &mut PlaySession, staged: &Staged) -> Vec<f32> {
        let finish = staged.finish();
        let renderer = &mut self.renderer;
        let drawn = session
            .present(staged, renderer, WIDTH as f32 / HEIGHT as f32, SEED)
            .unwrap();
        let view = &self.output.view;
        self.pace.frame(|| {
            renderer
                .render(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap()
        });
        self.renderer
            .gpu()
            .readback_rgba16(&self.output)
            .unwrap()
            .into_iter()
            .map(|bits| f16::from_bits(bits).to_f32())
            .collect()
    }
}

fn centre(pixels: &[f32]) -> Vec<f32> {
    let mut patch = Vec::new();
    for y in (HEIGHT / 2 - 6)..(HEIGHT / 2 + 6) {
        for x in (WIDTH / 2 - 6)..(WIDTH / 2 + 6) {
            let at = ((y * WIDTH + x) * 4) as usize;
            patch.extend_from_slice(&pixels[at..at + 3]);
        }
    }
    patch
}

fn difference(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum::<f32>() / a.len() as f32
}

fn offset(world: &World) -> (f32, f32) {
    let camera = world.camera;
    let id = world.object("crate").unwrap();
    let at = world.at(id);
    let (forward, right, up) = camera.axes();
    let rel: Vec<f32> = (0..3).map(|axis| at[axis] - camera.at[axis]).collect();
    let dot = |a: [f32; 3]| (0..3).map(|axis| rel[axis] * a[axis]).sum::<f32>();
    let depth = dot(forward);
    (dot(right) / depth, dot(up) / depth)
}

fn follow(rigs: &str, scene: &Scene, viewer: &mut Viewer) -> (Vec<f32>, Vec<f32>, (f32, f32)) {
    let staged = viewer.renderer.stage(scene).unwrap();
    viewer.renderer.wait_sky().unwrap();
    let options = Options {
        rigs: Some(Rigs::parse(rigs, Path::new("rigs.toml")).unwrap()),
        ..Options::default()
    };
    let mut session =
        PlaySession::play_with(scene, Some(&staged), Box::new(Mover), options).unwrap();
    for _ in 0..4 {
        session.advance(FRAME);
    }
    let first = viewer.draw(&mut session, &staged);
    for _ in 0..90 {
        session.advance(FRAME);
    }
    let last = viewer.draw(&mut session, &staged);
    let aim = offset(session.world());
    let stopped = session.stop();
    let mut staged = staged;
    stopped.restore(&mut staged, &mut viewer.renderer).unwrap();
    staged.release(&mut viewer.renderer).unwrap();
    (first, last, aim)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_follow_rig_keeps_a_moving_body_in_the_middle_of_the_frame() {
    let scene = Scene::open(folder("play-rig").join("room.scene.toml")).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = common::output(&gpu, "play rig output", [WIDTH, HEIGHT]);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut viewer = Viewer {
        renderer,
        output,
        pace: Pace::new(),
    };
    let (first, last, aim) = follow(RIGS, &scene, &mut viewer);
    assert!(
        aim.0.abs() < 0.02 && aim.1.abs() < 0.2,
        "the body drifted to {aim:?}"
    );
    let still = RIGS.replace(
        "kind = \"follow3d\"",
        "kind = \"follow3d\"\ndead_zone = [100, 100, 100]",
    );
    let (_, parked, away) = follow(&still, &scene, &mut viewer);
    assert!(away.0.abs() > 0.1, "the control body stayed at {away:?}");
    let followed = difference(&centre(&first), &centre(&last));
    let left_behind = difference(&centre(&first), &centre(&parked));
    assert!(
        followed < left_behind * 0.5,
        "followed {followed}, left behind {left_behind}"
    );
    assert_ne!(first, last);
}
