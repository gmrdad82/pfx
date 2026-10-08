use std::path::{Path, PathBuf};

use pfx_gpu::pace::Pace;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Override, Staged};
use pfx_load::scene::Scene;
use pfx_play::{Look, PlaySession, SceneGame};

mod common;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;

const PLAY: &str = r#"format = 1

include = ["room.scene.toml"]

[[object]]
name = "ball"
mesh = "block"
at = [0.4, 1.6, 0.6]
scale = 0.3
material = "blue"

[object.body]
shape = "sphere"
restitution = 0.3

[[mover]]
name = "spin"
objects = ["right pillar"]
kind = "turn"
pivot = [1.4, 0.0, -0.6]
axis = [0.0, 1.0, 0.0]
travel = [0.0, 60.0]
period = 2.0
"#;

fn folder() -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("play-restore");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    std::fs::write(root.join("play.scene.toml"), PLAY).unwrap();
    let room = root.join("room.scene.toml");
    let mut text = std::fs::read_to_string(&room).unwrap();
    let start = text.find("name = \"floor\"\n").unwrap();
    let end = text[start..]
        .find("\n\n")
        .map_or(text.len(), |at| start + at + 1);
    text.insert_str(end, "\n[object.body]\nkind = \"fixed\"\n");
    std::fs::write(&room, text).unwrap();
    root
}

struct Viewer {
    renderer: Renderer,
    output: OffscreenTarget,
    pace: Pace<pfx_gpu::pace::WallClock>,
}

impl Viewer {
    fn pixels(&self) -> Vec<u16> {
        self.renderer.gpu().readback_rgba16(&self.output).unwrap()
    }

    fn edited(&mut self, staged: &mut Staged) -> Vec<u16> {
        let finish = staged.finish();
        let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, 0.0, SEED);
        let renderer = &mut self.renderer;
        let view = &self.output.view;
        self.pace.frame(|| {
            renderer
                .render(&drawn.scene, &drawn.text, &drawn.effects, finish, view)
                .unwrap()
        });
        self.pixels()
    }

    fn played(&mut self, session: &mut PlaySession, staged: &Staged) -> Vec<u16> {
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
        self.pixels()
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn play_pause_step_resume_and_stop_restore_the_rendered_pixels() {
    let root = folder();
    let scene = Scene::open(root.join("play.scene.toml")).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = common::output(&gpu, "play restore output", [WIDTH, HEIGHT]);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(&scene).unwrap();
    staged.set_override(
        "crate",
        Override {
            highlight: Some([0.1, 0.2, 0.5]),
            ..Override::default()
        },
    );
    let mut viewer = Viewer {
        renderer,
        output,
        pace: Pace::new(),
    };
    let edited = viewer.edited(&mut staged);
    let before = scene.clone();

    let mut session = PlaySession::play(&scene, &staged, Box::new(SceneGame)).unwrap();
    let first = viewer.played(&mut session, &staged);
    assert_eq!(first, edited, "play starts from what the editor shows");
    let mut moved = false;
    for _ in 0..40 {
        session.advance(1.0 / 60.0);
        moved |= viewer.played(&mut session, &staged) != edited;
    }
    assert!(moved, "the ball falls and the pillar turns");
    session.pause();
    session.advance(1.0 / 60.0);
    let paused = viewer.played(&mut session, &staged);
    assert!(session.step());
    let stepped = viewer.played(&mut session, &staged);
    assert_ne!(paused, stepped, "a step moves the scene");
    session.resume();
    let wall = session.world().object("wall").unwrap();
    session.world_mut().set_look(
        wall,
        Look {
            color: Some([0.9, 0.1, 0.1]),
            ..Look::default()
        },
    );
    for _ in 0..10 {
        session.advance(1.0 / 60.0);
        viewer.played(&mut session, &staged);
    }
    let stopped = session.stop();
    assert_eq!(stopped.scene, before);
    assert!(stopped.diff.is_empty());
    let applied = stopped.restore(&mut staged, &mut viewer.renderer).unwrap();
    assert!(!applied.touched_device());
    assert_eq!(staged.overrides().len(), 1);
    assert_eq!(
        viewer.edited(&mut staged),
        edited,
        "stop restores the pixels"
    );

    let mut again = PlaySession::play(&stopped.scene, &staged, stopped.game).unwrap();
    assert_eq!(
        viewer.played(&mut again, &staged),
        first,
        "play again starts alike"
    );
    again.advance(1.0 / 60.0);
    viewer.played(&mut again, &staged);
    let stopped = again.stop();
    stopped.restore(&mut staged, &mut viewer.renderer).unwrap();
    assert_eq!(viewer.edited(&mut staged), edited);
}
