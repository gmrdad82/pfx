use std::path::{Path, PathBuf};
use std::sync::Arc;

use pfx_core::clock::Tick;
use pfx_gpu::pace::Pace;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_input::Input;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::{Geometry, Scene};
use pfx_play::{Game, PlaySession, World};

mod common;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 128;
const SEED: u32 = 5;
const FRAME: f32 = 1.0 / 60.0;

const SCENE: &str = r#"format = 1
materials = ["materials.toml"]
fallback = "clay"

[mesh.head]
file = "head.glb"
id = "mrphd00001"

[[object]]
name = "face"
mesh = "head"
id = "mrpfc00001"

[sun]
model = "authored"
toward = [-0.4, 0.8, 0.6]
color = [1.0, 0.92, 0.8]
irradiance = 2.0

[camera]
at = [0.0, 0.4, 2.4]
look_at = [0.0, 0.2, 0.0]
fov = 40.0
"#;

fn folder() -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("play-morph");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let load = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests");
    std::fs::copy(
        load.join("scenes/materials.toml"),
        root.join("materials.toml"),
    )
    .unwrap();
    std::fs::write(root.join("head.glb"), pfx_load::fixture::head()).unwrap();
    std::fs::write(root.join("morph.scene.toml"), SCENE).unwrap();
    root
}

fn unmorphed(scene: &Scene) -> Scene {
    let mut rest = scene.clone();
    for mesh in rest.meshes.values_mut() {
        for part in &mut mesh.parts {
            if part.geometry.morph.is_some() {
                part.geometry = Arc::new(Geometry {
                    morph: None,
                    ..(*part.geometry).clone()
                });
            }
        }
    }
    rest
}

struct Still;

impl Game for Still {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}
}

struct Viewer {
    renderer: Renderer,
    output: OffscreenTarget,
    pace: Pace<pfx_gpu::pace::WallClock>,
}

impl Viewer {
    fn new(gpu: Gpu) -> Self {
        let output = common::output(&gpu, "play morph output", [WIDTH, HEIGHT]);
        let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        Self {
            renderer,
            output,
            pace: Pace::new(),
        }
    }

    fn draw(&mut self, session: &mut PlaySession, staged: &Staged) -> Vec<u16> {
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
        self.renderer.gpu().readback_rgba16(&self.output).unwrap()
    }

    fn play(&mut self, scene: &Scene, weights: &[[f32; 3]]) -> Vec<Vec<u16>> {
        let mut staged = self.renderer.stage(scene).unwrap();
        self.renderer.wait_sky().unwrap();
        let mut session = PlaySession::play(scene, &staged, Box::new(Still)).unwrap();
        let mut frames = Vec::new();
        for set in weights {
            let world = session.world_mut();
            let face = world.object("face").unwrap();
            world.set_morph_weights(face, "head", set).unwrap();
            session.advance(FRAME);
            frames.push(self.draw(&mut session, &staged));
        }
        let stopped = session.stop();
        stopped.restore(&mut staged, &mut self.renderer).unwrap();
        staged.release(&mut self.renderer).unwrap();
        frames
    }
}

fn changed(a: &[u16], b: &[u16]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a[..3] != b[..3])
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_played_head_draws_its_morph_weights_and_zero_weights_draw_the_rest_pose() {
    let scene = Scene::open(folder().join("morph.scene.toml")).unwrap();
    assert!(
        scene
            .meshes
            .values()
            .flat_map(|mesh| &mesh.parts)
            .any(|part| part.geometry.morph.is_some())
    );
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut morphed = Viewer::new(gpu.clone());
    let mut rest = Viewer::new(gpu);
    let played = morphed.play(&scene, &[[0.0, 0.0, 0.0], [1.0, 0.0, 1.0]]);
    let still = rest.play(&unmorphed(&scene), &[[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]);
    assert_eq!(
        changed(&played[0], &still[0]),
        0,
        "zero weights must draw the rest pose"
    );
    let moved = changed(&played[1], &still[1]);
    assert!(
        moved > (WIDTH * HEIGHT / 100) as usize,
        "the open jaw changed {moved} pixels"
    );
    assert!(changed(&played[0], &played[1]) > 0);
}
