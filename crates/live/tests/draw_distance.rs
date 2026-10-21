#[path = "support/output.rs"]
mod output;

use output::output;
use std::path::{Path, PathBuf};

use pfx_gpu::Gpu;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const SEED: u32 = 7;

fn room(name: &str, edit: impl Fn(&str) -> String) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("draw-distance")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    let file = root.join("room.scene.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, edit(&text)).unwrap();
    file
}

fn culled(file: &Path) -> (u32, u32, usize) {
    let scene = Scene::open(file).unwrap();
    let limited = scene
        .objects
        .iter()
        .filter(|object| object.max_distance.is_finite())
        .count();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let output = output(&gpu, WIDTH, HEIGHT);
    let mut renderer = Renderer::new(gpu, WIDTH, HEIGHT).unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(&scene).unwrap();
    let finish = staged.finish();
    let drawn = staged.frame(WIDTH as f32 / HEIGHT as f32, 0.0, SEED);
    renderer
        .submit(
            &drawn.scene,
            &drawn.text,
            &drawn.effects,
            finish,
            &output.view,
        )
        .unwrap();
    renderer.wait_frames(0).unwrap();
    let stats = renderer.cull_stats();
    (stats.drawable, stats.culled(), limited)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_scene_objects_draw_distance_culls_it_from_the_live_frame() {
    let plain = room("plain", str::to_string);
    let near = room("near", |text| {
        text.replace("[[object]]\n", "[[object]]\nmax_distance = 0.05\n")
    });
    let wide = room("wide", |text| {
        text.replace("[[object]]\n", "[[object]]\nmax_distance = 1000000.0\n")
    });
    let (drawable, base, none) = culled(&plain);
    assert_eq!(none, 0);
    assert!(drawable > 0);
    let (near_drawable, near_culled, limited) = culled(&near);
    assert!(limited > 0);
    assert_eq!(near_drawable, drawable);
    assert!(
        near_culled > base,
        "{near_culled} culled with {limited} limited objects, {base} without"
    );
    let (_, wide_culled, _) = culled(&wide);
    assert_eq!(wide_culled, base);
}
