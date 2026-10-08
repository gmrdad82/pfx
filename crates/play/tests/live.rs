use std::path::{Path, PathBuf};

use pfx_gpu::pace::Pace;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::{Scene, Target};
use pfx_play::{Edit, PlaySession, SceneGame};

mod common;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;

fn folder(name: &str, edits: &[(&str, &str, &str)]) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    for (file, from, to) in edits {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{file} has no {from:?}");
        std::fs::write(&path, text.replacen(from, to, 1)).unwrap();
    }
    root
}

struct Viewer {
    renderer: Renderer,
    output: OffscreenTarget,
    pace: Pace<pfx_gpu::pace::WallClock>,
}

impl Viewer {
    fn stage(&mut self, scene: &Scene) -> Staged {
        let staged = self.renderer.stage(scene).unwrap();
        self.renderer.wait_sky().unwrap();
        staged
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
        self.renderer.gpu().readback_rgba16(&self.output).unwrap()
    }

    fn first(&mut self, scene: &Scene, edits: &[Edit]) -> Vec<u16> {
        let staged = self.stage(scene);
        let mut session = PlaySession::play(scene, &staged, Box::new(SceneGame)).unwrap();
        for edit in edits {
            session.edit(edit.clone()).unwrap();
        }
        let pixels = self.played(&mut session, &staged);
        let stopped = session.stop();
        let mut staged = staged;
        stopped.restore(&mut staged, &mut self.renderer).unwrap();
        staged.release(&mut self.renderer).unwrap();
        pixels
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn live_material_and_light_edits_draw_as_the_edited_files_do_and_stop_undoes_them() {
    let plain = Scene::open(folder("play-live-plain", &[]).join("room.scene.toml")).unwrap();
    let edited = Scene::open(
        folder(
            "play-live-edited",
            &[
                (
                    "materials.toml",
                    "base = [0.42, 0.44, 0.46]\nroughness = 0.7",
                    "base = [0.9, 0.1, 0.1]\nroughness = 0.3",
                ),
                ("lights.scene.toml", "intensity = 6.0", "intensity = 1.5"),
            ],
        )
        .join("room.scene.toml"),
    )
    .unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = common::output(&gpu, "play live edits output", [WIDTH, HEIGHT]);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut viewer = Viewer {
        renderer,
        output,
        pace: Pace::new(),
    };
    let stone = Target::Material("stone".into());
    let edits = [
        Edit::scene(stone.clone(), &["base"], [0.9f32, 0.1, 0.1]),
        Edit::scene(stone, &["roughness"], 0.3f32),
        Edit::scene(Target::Light("warm".into()), &["intensity"], 1.5f32),
    ];
    let before = viewer.first(&plain, &[]);
    let live = viewer.first(&plain, &edits);
    let after = viewer.first(&plain, &[]);
    let staged = viewer.first(&edited, &[]);
    assert_ne!(live, before, "the live edits change the pixels");
    assert_eq!(live, staged, "the live edits draw as the edited files do");
    assert_eq!(after, before, "stop puts the lights and materials back");
}
