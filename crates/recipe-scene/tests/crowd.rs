use pfx_build::{Built, Part, Rate, Registry, build_with};
use pfx_recipe::Recipe;
use pfx_recipe_scene::pace::turn;
use pfx_recipe_scene::{Eye, Keyed, Stage, camera, headless, poses, psnr};
use std::path::Path;

const SIZE: [u32; 2] = [320, 256];
const FPS: u32 = 24;

const SCENE: &str = r#"recipe = 1
name = "crowd"
seed = 3

[timeline]
fps = 24
frames = 48

[[object]]
name = "ground"
kind = "plane"
size = [12.0, 12.0]
material = "ground"

[[object]]
name = "leaf"
kind = "box"
size = [0.16, 0.02, 0.1]
material = "leaf"
hidden = true

[[scatter]]
name = "leaves"
object = "leaf"
on = "ground"
seed = 9
density = 6.0
rotate_max = [0.0, 360.0, 0.0]
scale_min = 0.6
scale_max = 1.4

[[object]]
name = "crate"
kind = "box"
size = [0.6, 0.6, 0.6]
material = "leaf"
at = [-1.0, 0.3, 0.0]
keys = { at = [[0.0, [-1.0, 0.3, 0.0]], [2.0, [1.0, 1.3, 1.0]]], rotate = [[0.0, [0.0, 0.0, 0.0]], [2.0, [0.0, 270.0, 0.0]]] }

[[object]]
name = "lid"
kind = "box"
size = [0.7, 0.05, 0.7]
material = "ground"
parent = "crate"
at = [0.0, 0.35, 0.0]

[[object]]
name = "post"
kind = "box"
size = [0.2, 1.4, 0.2]
material = "ground"
at = [1.5, 0.7, -1.0]
keys = { rotate = [[0.0, [0.0, 0.0, 0.0]], [2.0, [0.0, 0.0, 30.0]]] }

[[object]]
name = "ramp"
kind = "box"
size = [3.0, 0.2, 2.0]
material = "ground"
at = [2.5, 0.4, 1.5]
rotate = [0.0, 0.0, 20.0]

[[scatter]]
name = "pebbles"
object = "leaf"
on = "ramp"
seed = 4
density = 8.0
align = true
rotate_max = [0.0, 360.0, 0.0]

[[camera]]
name = "main"
position = [0.0, 3.5, -6.0]
target = [0.0, 0.0, 0.5]
fov_y_deg = 50.0

[sun]
model = "authored"
toward = [0.5, 0.7, -0.3]
radius_deg = 0.27
color = [1.0, 0.96, 0.9]
irradiance = 3.0

[materials.ground]
family = "plain"
base = [0.35, 0.33, 0.3]
roughness = 0.9

[materials.leaf]
family = "plain"
base = [0.6, 0.35, 0.12]
roughness = 0.7
"#;

fn scene() -> (Recipe, Built) {
    let parsed = pfx_recipe::parse(SCENE).unwrap();
    let built = build_with(&parsed, Path::new("."), &Registry::new()).unwrap();
    (built.recipe(&parsed), built)
}

fn baked(parts: &[Part]) -> Vec<Part> {
    parts
        .iter()
        .cloned()
        .map(|part| Part {
            scattered: None,
            ..part
        })
        .collect()
}

struct Shot {
    stage: Stage,
    target: pfx_gpu::OffscreenTarget,
    eye: Eye,
}

impl Shot {
    fn new(gpu: &pfx_gpu::Gpu, recipe: &Recipe, parts: &[Part]) -> Shot {
        let mut stage = Stage::with_parts(gpu.clone(), recipe, parts, SIZE).unwrap();
        let main = camera(recipe, "main").unwrap();
        stage.look_through(Some(main));
        let target = gpu
            .offscreen(SIZE[0], SIZE[1], stage.output_format())
            .unwrap();
        let eye = Eye::from_camera(main, f64::from(SIZE[0]) / f64::from(SIZE[1]));
        Shot { stage, target, eye }
    }

    fn draw(&mut self, time: f64) -> Vec<u8> {
        self.stage
            .render(&self.eye, time as f32, &self.target.view)
            .unwrap();
        turn();
        self.stage.gpu().readback_rgba8(&self.target).unwrap()
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn keyed_frames_in_one_batch_are_the_bytes_the_named_poses_drew() {
    let (recipe, built) = scene();
    let gpu = headless().unwrap();
    let fps = Rate::whole(FPS);
    let mut named = Shot::new(&gpu, &recipe, &built.parts);
    let mut batched = Shot::new(&gpu, &recipe, &built.parts);
    let mut keyed = Keyed::of(&built, fps);
    batched.stage.bind_keys(keyed.names());
    assert_eq!(keyed.names(), ["crate", "lid", "post"]);
    for frame in 0..12 {
        let time = f64::from(frame) / f64::from(FPS) * 4.0;
        named.stage.set_poses(poses(&built, fps, time));
        batched.stage.set_keyed(keyed.sample(&built, time));
        let (a, b) = (named.draw(time), batched.draw(time));
        assert!(a == b, "frame {frame} differs");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_crowd_draws_what_its_baked_copies_drew_and_repeats_to_the_byte() {
    let (recipe, built) = scene();
    let gpu = headless().unwrap();
    let mut crowd = Shot::new(&gpu, &recipe, &built.parts);
    let mut copies = Shot::new(&gpu, &recipe, &baked(&built.parts));
    let placed = crowd.stage.crowd_copies();
    assert!(placed > 500, "{placed}");
    assert_eq!(copies.stage.crowd_copies(), 0);
    assert_eq!(
        copies.stage.instance_count(),
        crowd.stage.instance_count() + placed
    );
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for frame in 0..24 {
        a = crowd.draw(f64::from(frame) / 60.0);
        b = copies.draw(f64::from(frame) / 60.0);
    }
    let close = psnr(&a, &b).unwrap();
    assert!(
        close > 40.0,
        "the crowd drew {close:.1} dB from its baked copies"
    );
    let apart = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| x.iter().zip(*y).any(|(p, q)| p.abs_diff(*q) > 4))
        .count();
    let pixels = a.len() / 4;
    assert!(
        apart * 200 <= pixels,
        "{apart} of {pixels} pixels differ by more than 4/255 from the baked copies"
    );
    let mut again = Shot::new(&gpu, &recipe, &built.parts);
    let mut c = Vec::new();
    for frame in 0..24 {
        c = again.draw(f64::from(frame) / 60.0);
    }
    assert!(a == c, "the same crowd drew different bytes");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_frame_draws_no_more_instances_for_more_copies() {
    let gpu = headless().unwrap();
    let gpu_scene = |density: f64| {
        let text = SCENE.replace("density = 6.0", &format!("density = {density}"));
        let parsed = pfx_recipe::parse(&text).unwrap();
        let built = build_with(&parsed, Path::new("."), &Registry::new()).unwrap();
        let mut shot = Shot::new(&gpu, &built.recipe(&parsed), &built.parts);
        shot.draw(0.0);
        (shot.stage.crowd_copies(), shot.stage.instance_count())
    };
    let (few, few_instances) = gpu_scene(1.0);
    let (many, many_instances) = gpu_scene(12.0);
    assert!(many > few * 8, "{few} {many}");
    assert_eq!(few_instances, many_instances);
}
