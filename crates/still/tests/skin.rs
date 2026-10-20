use pfx_build::Registry;
use pfx_recipe::Pick;
use pfx_still::{Reel, Request, Seed, Step, Take, View, looked_with};
use std::path::{Path, PathBuf};

const RECIPE: &str = r#"recipe = 1
name = "skin"

[timeline]
fps = 4
frames = 4

[sun]
model = "authored"
toward = [0.4, 0.8, 0.3]
radius_deg = 0.5
color = [1.0, 0.95, 0.9]
irradiance = 3.0

[[camera]]
name = "main"
position = [0.0, 1.6, 6.0]
target = [0.0, 1.4, 0.0]
fov_y_deg = 45.0

[materials.grey]
base = [0.5, 0.5, 0.5]
roughness = 0.8

[[object]]
name = "floor"
kind = "plane"
size = [6.0, 6.0]
material = "grey"

[[object]]
name = "column"
kind = "mesh"
path = "column.glb"
material = "grey"
smooth = true
clip = "bend"
loop = false
"#;

const SIZE: [u32; 2] = [160, 120];

fn scene(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/still-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("column.glb"), pfx_load::fixture::skinned_column(3)).unwrap();
    std::fs::write(dir.join("skin.toml"), RECIPE).unwrap();
    dir.join("skin.toml")
}

fn traced(recipe: &Path, time: f64) -> Vec<u8> {
    let request = Request {
        recipe,
        camera: "main",
        size: SIZE,
        samples: 4,
        noise: None,
        seed: None,
        view: View::Lens,
        variant: None,
        set: &[],
        pick: &Pick::default(),
        crop: None,
        denoise: true,
        time: Some(time),
        transparent: false,
    };
    let rendered = pfx_still::render(&request, &Registry::new(), &mut |_| {}).unwrap();
    looked_with(&rendered.film, (&rendered.finish, &rendered.overlay), true).unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_clip_frame_of_a_skinned_mesh_is_the_render_posed_at_its_time() {
    let recipe = scene("skin-traced");
    let mut reel = Reel::open(&recipe, None, &[], None, &Registry::new()).unwrap();
    let mut frames = Vec::new();
    for time in [0.0, 0.5] {
        reel.pose(time).unwrap();
        let camera = reel.recipe.cameras[0].clone();
        let take = Take {
            size: SIZE,
            samples: 4,
            noise: None,
            seed: Seed::Locked,
        };
        let rendered = reel.frame(&camera, take, &mut |_: Step| {}).unwrap();
        let rgba =
            looked_with(&rendered.film, (&rendered.finish, &rendered.overlay), true).unwrap();
        assert!(
            rgba == traced(&recipe, time),
            "the traced frame at {time} s"
        );
        frames.push(rgba);
    }
    assert!(frames[0] != frames[1]);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_live_frame_of_a_skinned_mesh_matches_the_shot_posed_at_its_time() {
    let recipe = scene("skin-live");
    let registry = Registry::new();
    let (r, built) = pfx_recipe_scene::load(&recipe, None, &[], &registry).unwrap();
    let gpu = pfx_recipe_scene::headless().unwrap();
    let mut reel = pfx_recipe_scene::Reel::new(&gpu, &r, &built.parts, SIZE).unwrap();
    let camera = pfx_recipe_scene::camera(&r, "main").unwrap().clone();
    let mut frames = Vec::new();
    for time in [0.0, 0.5] {
        let fps = r.timeline.map_or(pfx_build::Rate::whole(1), |t| t.fps);
        reel.set_poses(pfx_recipe_scene::poses(&built, fps, time));
        reel.reshape(&built.skinned_parts(time)).unwrap();
        let (rgba, _) = reel.shoot(&reel.eye(&camera)).unwrap();
        let (posed_recipe, posed) =
            pfx_recipe_scene::load_at(&recipe, None, &[], &Pick::default(), &registry, Some(time))
                .unwrap();
        let shot =
            pfx_recipe_scene::shoot(&gpu, &posed_recipe, &posed.parts, "main", SIZE).unwrap();
        let close = pfx_recipe_scene::psnr(&rgba, &shot).unwrap();
        assert!(close > 40.0, "the live frame at {time} s: {close:.1} dB");
        frames.push(rgba);
    }
    let apart = pfx_recipe_scene::psnr(&frames[0], &frames[1]).unwrap();
    assert!(apart < 35.0, "the two times look alike: {apart:.1} dB");
}
