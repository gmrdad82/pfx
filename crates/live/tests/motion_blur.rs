#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::{Path, PathBuf};

use pfx_gpu::{Gpu, OffscreenTarget};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::Staged;
use pfx_load::scene::{Camera, Scene, SceneDiff};

const WIDTH: u32 = 384;
const HEIGHT: u32 = 192;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;
const FPS: f32 = 60.0;

const MATERIALS: &str = "[materials.glow]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [1.0, 1.0, 1.0]
";

const SCENE: &str = "format = 1

materials = [\"blur.materials.toml\"]
fallback = \"glow\"

[mesh.block]
file = \"block.gltf\"

[[object]]
name = \"runner\"
mesh = \"block\"
at = [-2.6, 0.5, 0.0]
scale = 0.5
material = \"glow\"

[[object]]
name = \"post\"
mesh = \"block\"
at = [2.4, -0.7, 0.0]
scale = 0.5
material = \"glow\"

[[mover]]
name = \"dash\"
objects = [\"runner\"]
kind = \"slide\"
axis = [1.0, 0.0, 0.0]
travel = [0.0, 12.0]
period = 1.0
motion = \"once\"

[camera]
at = [0.0, 0.0, 5.0]
look_at = [0.0, 0.0, 0.0]
fov = 40.0
";

fn scene(name: &str, extra: &str) -> Scene {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("motion-blur")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(root.join("blur.materials.toml"), MATERIALS).unwrap();
    let path: PathBuf = root.join("blur.scene.toml");
    std::fs::write(&path, format!("{SCENE}{extra}")).unwrap();
    Scene::open(&path).unwrap()
}

struct Viewer {
    renderer: Renderer,
    staged: Staged,
    output: OffscreenTarget,
    frame: u32,
}

impl Viewer {
    fn new(scene: &Scene) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let output = output(&gpu, WIDTH, HEIGHT);
        let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let staged = renderer.stage(scene).unwrap();
        renderer.wait_sky().unwrap();
        Self {
            renderer,
            staged,
            output,
            frame: 0,
        }
    }

    fn render(&mut self) -> Vec<f32> {
        let time = self.frame as f32 / FPS;
        self.frame += 1;
        let finish = self.staged.finish();
        let drawn = self.staged.frame_at(ASPECT, time, 1.0 / FPS, 3);
        self.renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &self.output.view,
            )
            .unwrap();
        self.renderer
            .gpu()
            .readback_rgba16(&self.output)
            .unwrap()
            .chunks_exact(4)
            .map(|texel| half::f16::from_bits(texel[1]).to_f32())
            .collect()
    }

    fn frames(&mut self, count: u32) -> Vec<f32> {
        let mut last = Vec::new();
        for _ in 0..count {
            last = self.render();
        }
        last
    }

    fn blurred(&self) -> bool {
        self.renderer.last_pass_order().contains(&"motion blur")
    }
}

fn at(image: &[f32], x: u32, y: u32) -> f32 {
    image[(y * WIDTH + x) as usize]
}

fn bright(image: &[f32], xs: std::ops::Range<u32>) -> [u32; 4] {
    let peak = xs
        .clone()
        .flat_map(|x| (0..HEIGHT).map(move |y| (x, y)))
        .map(|(x, y)| at(image, x, y))
        .fold(0.0, f32::max);
    assert!(peak > 0.2, "nothing bright in {xs:?}: {peak}");
    let mut bounds = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..HEIGHT {
        for x in xs.clone() {
            if at(image, x, y) > peak * 0.5 {
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x),
                    bounds[3].max(y),
                ];
            }
        }
    }
    bounds
}

fn extent(image: &[f32], along: impl Iterator<Item = (u32, u32)>, floor: f32) -> u32 {
    along.filter(|&(x, y)| at(image, x, y) > floor).count() as u32
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_moving_object_blurs_along_its_motion_and_a_still_one_stays_byte_identical() {
    const FRAMES: u32 = 13;
    let off = Viewer::new(&scene("off", "")).frames(FRAMES);
    let mut zero = Viewer::new(&scene("zero", "\n[motion_blur]\nshutter = 0.0\n"));
    let closed = zero.frames(FRAMES);
    assert!(!zero.blurred(), "a closed shutter runs no pass");
    assert!(off == closed, "a closed shutter changes no byte");

    let on_scene = scene(
        "on",
        "\n[motion_blur]\nshutter = 1.0\nmax_px = 16.0\nsamples = 16\n",
    );
    let mut on = Viewer::new(&on_scene);
    let open = on.frames(FRAMES);
    assert!(on.blurred(), "an open shutter runs the pass");

    let runner = bright(&off, 0..WIDTH / 2 + 40);
    let post = bright(&off, WIDTH / 2 + 60..WIDTH);
    for y in post[1].saturating_sub(24)..(post[3] + 24).min(HEIGHT) {
        for x in post[0] - 24..(post[2] + 24).min(WIDTH) {
            assert_eq!(
                at(&open, x, y).to_bits(),
                at(&off, x, y).to_bits(),
                "the still object changed at ({x},{y})"
            );
        }
    }
    let row = (runner[1] + runner[3]) / 2;
    let column = (runner[0] + runner[2]) / 2;
    let floor = 0.05;
    let across = |image: &[f32]| extent(image, (0..WIDTH / 2 + 40).map(|x| (x, row)), floor);
    let down = |image: &[f32]| extent(image, (0..HEIGHT).map(|y| (column, y)), floor);
    println!(
        "runner {runner:?}: {} px wide sharp, {} px blurred; {} px tall sharp, {} px blurred",
        across(&off),
        across(&open),
        down(&off),
        down(&open)
    );
    assert!(
        across(&open) >= across(&off) + 6,
        "the runner streaks along its motion"
    );
    assert!(
        down(&open) <= down(&off) + 1,
        "the runner does not smear across its motion"
    );

    let mut live = Viewer::new(&scene("off-live", ""));
    live.frames(FRAMES - 1);
    let diff = SceneDiff {
        motion_blur: true,
        ..SceneDiff::default()
    };
    let applied = live
        .staged
        .apply(&mut live.renderer, &on_scene, &diff)
        .unwrap();
    assert!(applied.motion_blur);
    let edited = live.render();
    assert!(edited == open, "a scene edit turns blur on as staging does");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_cameras_own_motion_blurs_a_still_object() {
    let mut runs = Vec::new();
    for (name, extra) in [
        ("pan-off", ""),
        (
            "pan-on",
            "\n[motion_blur]\nshutter = 1.0\nmax_px = 16.0\nsamples = 16\n",
        ),
    ] {
        let scene = scene(name, extra);
        let mut viewer = Viewer::new(&scene);
        let mut last = Vec::new();
        for frame in 0..8 {
            let mut panned = scene.clone();
            let camera = scene.camera_or_default();
            let step = 0.3 * frame as f32;
            panned.camera = Some(Camera {
                at: [camera.at[0] + step, camera.at[1], camera.at[2]],
                look_at: [
                    camera.look_at[0] + step,
                    camera.look_at[1],
                    camera.look_at[2],
                ],
                ..camera
            });
            let diff = SceneDiff {
                camera: true,
                ..SceneDiff::default()
            };
            viewer
                .staged
                .apply(&mut viewer.renderer, &panned, &diff)
                .unwrap();
            viewer.frame = 0;
            last = viewer.render();
        }
        runs.push(last);
    }
    let post = bright(&runs[0], 0..WIDTH);
    let row = (post[1] + post[3]) / 2;
    let across = |image: &[f32]| extent(image, (0..WIDTH).map(|x| (x, row)), 0.05);
    println!(
        "panned post {post:?}: {} px wide sharp, {} px blurred",
        across(&runs[0]),
        across(&runs[1])
    );
    assert!(
        across(&runs[1]) >= across(&runs[0]) + 6,
        "the pan streaks the still post"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu --class bench"]
fn the_pass_cost_at_2160x3840() {
    let (width, height) = (2160, 3840);
    let scene = scene(
        "cost",
        "\n[motion_blur]\nshutter = 0.5\nmax_px = 64.0\nsamples = 8\n",
    );
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let target = output(&gpu, width, height);
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut renderer = turns.cpu(|| Renderer::new(gpu, width, height).unwrap());
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = turns.cpu(|| renderer.stage(&scene).unwrap());
    renderer.wait_sky().unwrap();
    let aspect = width as f32 / height as f32;
    let mut blur = [Vec::new(), Vec::new()];
    let mut frame = [Vec::new(), Vec::new()];
    let camera = scene.camera_or_default();
    for number in 0..180u32 {
        turns.poll();
        let panning = number >= 90;
        if panning {
            let step = 0.02 * (number - 90) as f32;
            let mut panned = scene.clone();
            panned.camera = Some(Camera {
                look_at: [
                    camera.look_at[0] + step,
                    camera.look_at[1],
                    camera.look_at[2],
                ],
                ..camera
            });
            let diff = SceneDiff {
                camera: true,
                ..SceneDiff::default()
            };
            staged.apply(&mut renderer, &panned, &diff).unwrap();
        }
        let finish = staged.finish();
        let drawn = staged.frame_at(aspect, (number % 90) as f32 / FPS, 1.0 / FPS, 3);
        let timings = renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
        turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
        let run = usize::from(panning);
        if number % 90 >= 30 && !timings.is_empty() {
            frame[run].push(timings.iter().map(|t| t.milliseconds).sum::<f64>());
            blur[run].extend(
                timings
                    .iter()
                    .filter(|t| t.label == "motion blur")
                    .map(|t| t.milliseconds),
            );
        }
    }
    turns.turn();
    let median = |values: &mut Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
    };
    for (run, name) in ["one object moving", "the camera turning"]
        .iter()
        .enumerate()
    {
        let (blur_ms, frame_ms) = (median(&mut blur[run]), median(&mut frame[run]));
        println!(
            "motion blur at {width}x{height} (max_px 64, samples 8), {name}: {blur_ms:.3} ms of a {frame_ms:.3} ms frame over {} frames",
            blur[run].len()
        );
        assert!(!blur[run].is_empty(), "the pass was timed");
    }
}
