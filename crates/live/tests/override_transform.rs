#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::{Path, PathBuf};

use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_live::frame::Matrix;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Override, Staged};
use pfx_load::scene::Scene;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const MOVED: [f32; 3] = [0.3, 0.1, -0.2];

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("override-transform")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    root
}

fn translation(by: [f32; 3]) -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [by[0], by[1], by[2], 1.0],
    ]
}

struct Viewer {
    renderer: Renderer,
    staged: Staged,
    output: OffscreenTarget,
    turns: Turns,
}

impl Viewer {
    fn new(scene: &Scene) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let output = output(&gpu, WIDTH, HEIGHT);
        let mut turns = Turns::default();
        let mut renderer = turns.cpu(|| Renderer::new(gpu, WIDTH, HEIGHT).unwrap());
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let staged = turns.cpu(|| renderer.stage(scene).unwrap());
        renderer.wait_sky().unwrap();
        Self {
            renderer,
            staged,
            output,
            turns,
        }
    }

    fn frames(&mut self, count: u32) -> Vec<f32> {
        let aspect = WIDTH as f32 / HEIGHT as f32;
        for frame in 0..count {
            self.turns.poll();
            let finish = self.staged.finish();
            let drawn = self.staged.frame(aspect, frame as f32 / 60.0, 7);
            let timings = self
                .renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &self.output.view,
                )
                .unwrap();
            self.turns
                .add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
        }
        self.turns.turn();
        self.renderer
            .gpu()
            .readback_rgba16(&self.output)
            .unwrap()
            .into_iter()
            .map(|value| half::f16::from_bits(value).to_f32())
            .collect()
    }

    fn models(&self) -> Vec<Matrix> {
        self.staged
            .instances()
            .iter()
            .map(|instance| instance.model)
            .collect()
    }
}

fn largest_difference(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_transform_override_draws_the_pixels_of_a_restaged_moved_copy_and_unset_changes_none() {
    let rest = Scene::open(folder("rest").join("room.scene.toml")).unwrap();
    let crate_at = rest.object("crate").unwrap().at;
    let moved_root = folder("moved");
    let path = moved_root.join("room.scene.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let from = "at = [-0.9, 0.35, 0.0]";
    assert!(text.contains(from));
    let to = format!(
        "at = [{:?}, {:?}, {:?}]",
        crate_at[0] + MOVED[0],
        crate_at[1] + MOVED[1],
        crate_at[2] + MOVED[2]
    );
    std::fs::write(&path, text.replacen(from, &to, 1)).unwrap();
    let moved = Scene::open(&path).unwrap();

    let mut overridden = Viewer::new(&rest);
    let mut restaged = Viewer::new(&rest);
    let untouched = overridden.models();
    let plain = overridden.frames(4);
    assert!(plain.iter().any(|value| *value > 0.01), "the scene draws");
    restaged.frames(4);

    assert!(overridden.staged.set_override(
        "crate",
        Override {
            highlight: Some([0.0, 0.0, 0.0]),
            ..Override::default()
        },
    ));
    assert_eq!(overridden.models(), untouched, "unset moves nothing");
    let unset = overridden.frames(4);
    let alike = restaged.frames(4);
    assert_eq!(
        largest_difference(&unset, &alike),
        0.0,
        "unset changes no pixel"
    );
    overridden.staged.clear_overrides();

    assert!(overridden.staged.set_override(
        "crate",
        Override {
            transform: Some(translation(MOVED)),
            ..Override::default()
        },
    ));
    let diff = rest.diff(&moved);
    restaged
        .staged
        .apply(&mut restaged.renderer, &moved, &diff)
        .unwrap();
    for (got, want) in overridden.models().iter().zip(restaged.models()) {
        for (a, b) in got.iter().flatten().zip(want.iter().flatten()) {
            assert!((a - b).abs() < 1e-5, "{got:?} against {want:?}");
        }
    }
    let previewed = overridden.frames(8);
    let reference = restaged.frames(8);
    assert!(
        largest_difference(&plain, &reference) > 0.05,
        "moving the crate shows"
    );
    let left = largest_difference(&previewed, &reference);
    println!("override against restaged copy: up to {left}");
    assert!(left <= 1e-3, "the override draws as the restaged copy");

    overridden.staged.clear_override("crate");
    assert_eq!(overridden.models(), untouched);
}
