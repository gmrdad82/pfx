use std::path::{Path, PathBuf};

use pfx_gpu::pace::{Pacer, Turns};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_live::scene::{Applied, Staged};
use pfx_load::scene::{Scene, SceneWatch};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("scene-watch")
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

fn edit(root: &Path, name: &str, from: &str, to: &str) {
    let path = root.join(name);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{name} has no {from:?}");
    std::fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

fn edits(root: &Path) {
    edit(
        root,
        "materials.toml",
        "base = [0.7, 0.32, 0.22]",
        "base = [0.2, 0.6, 0.3]",
    );
    edit(
        root,
        "lights.scene.toml",
        "intensity = 6.0",
        "intensity = 14.0",
    );
    edit(
        root,
        "room.scene.toml",
        "file = \"block.gltf\"",
        "file = \"ball.gltf\"",
    );
    edit(root, "room.scene.toml", "power = 6.0", "power = 3.0");
    let path = root.join("room.scene.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(
        "\n[[object]]\nname = \"extra\"\nmesh = \"pillar\"\nat = [-1.8, 0.0, -1.0]\nclip = [[0.0, 1.0, 0.0, 0.8]]\n",
    );
    std::fs::write(&path, text).unwrap();
}

fn output(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene watch output"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format: wgpu::TextureFormat::Rgba16Float,
        width: WIDTH,
        height: HEIGHT,
    }
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
        let output = output(&gpu);
        let mut turns = Turns::default();
        let mut renderer = turns.cpu(|| Renderer::new(gpu, WIDTH, HEIGHT).unwrap());
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let staged = turns.cpu(|| renderer.stage(scene).unwrap());
        Self {
            renderer,
            staged,
            output,
            turns,
        }
    }

    fn frames(&mut self, start: f32, count: u32) -> Vec<f32> {
        let aspect = WIDTH as f32 / HEIGHT as f32;
        for frame in 0..count {
            self.turns.poll();
            let finish = self.staged.finish();
            let drawn = self.staged.frame(aspect, start + frame as f32 / 60.0, 7);
            let renderer = &mut self.renderer;
            let output = &self.output;
            let timings = renderer
                .render(
                    &drawn.scene,
                    &drawn.text,
                    &drawn.effects,
                    finish,
                    &output.view,
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
fn an_edit_through_scene_watch_draws_the_same_pixels_as_the_edited_scene_loaded_fresh() {
    let root = folder("watched");
    let mut watch = SceneWatch::open(root.join("room.scene.toml")).unwrap();
    let mut watched = Viewer::new(watch.scene());
    let before = watched.frames(0.0, 4);
    assert!(before.iter().any(|value| *value > 0.01), "the scene draws");

    let stats = watched.renderer.mesh_stats();
    let same = watch.scene().diff(watch.scene());
    let scene = watch.scene().clone();
    let applied = watched
        .staged
        .apply(&mut watched.renderer, &scene, &same)
        .unwrap();
    assert_eq!(applied, Applied::default());
    assert!(!applied.touched_device());
    assert_eq!(watched.renderer.mesh_stats(), stats);

    edits(&root);
    let reload = watch.check().unwrap().unwrap();
    assert!(!reload.recovered);
    assert_eq!(reload.diff.materials.changed, ["clay"]);
    assert_eq!(reload.diff.lights.changed, ["warm"]);
    assert_eq!(reload.diff.meshes.changed, ["block"]);
    assert_eq!(reload.diff.objects.added, ["extra"]);
    assert!(reload.diff.sky);
    let applied = watched
        .staged
        .apply(&mut watched.renderer, &reload.scene, &reload.diff)
        .unwrap();
    assert_eq!(applied.replaced, 1);
    assert_eq!(applied.released, 0);
    assert!(applied.lights && applied.sky && applied.draws);
    assert!(applied.sky_request.is_some());
    watched.renderer.wait_sky().unwrap();

    let after = watched.frames(10.0, 4);

    let fresh_scene = Scene::open(root.join("room.scene.toml")).unwrap();
    let mut fresh = Viewer::new(&fresh_scene);
    fresh.frames(0.0, 4);
    let reference = fresh.frames(10.0, 4);
    assert_eq!(watched.staged.meshes(), fresh.staged.meshes());
    assert_eq!(watched.staged.materials(), fresh.staged.materials());

    let moved = largest_difference(&before, &after);
    let left = largest_difference(&after, &reference);
    println!("edit changed pixels by up to {moved}, watched against fresh up to {left}");
    assert!(moved > 0.05, "the edits show");
    assert!(left <= 1e-3, "the watched scene draws as the fresh one");

    edit(
        &root,
        "materials.toml",
        "roughness = 0.9",
        "roughness = = 0.9",
    );
    let error = watch.check().unwrap().unwrap_err();
    assert!(error.file.ends_with("materials.toml"), "{error}");
    let kept = watched.frames(20.0, 1);
    assert!(kept.iter().any(|value| *value > 0.01));
    edit(
        &root,
        "materials.toml",
        "roughness = = 0.9",
        "roughness = 0.9",
    );
    let reload = watch.check().unwrap().unwrap();
    assert!(reload.recovered && reload.diff.is_empty());
    let applied = watched
        .staged
        .apply(&mut watched.renderer, &reload.scene, &reload.diff)
        .unwrap();
    assert!(!applied.touched_device());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_example_scene_traces_from_the_same_source() {
    let root = folder("traced");
    let scene = Scene::open(root.join("room.scene.toml")).unwrap();
    let staged = pfx_trace::stage::scene(&scene, WIDTH as f32 / HEIGHT as f32).unwrap();
    assert_eq!(staged.detail.instances.len(), scene.draws().items.len());
    assert_eq!(staged.detail.lights.len(), 2);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut trace = staged.trace(&gpu, WIDTH, HEIGHT).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..8 {
        trace
            .sample_paced(&gpu, 1, 3 + pass, &mut pacer, |ms| turns.add(ms))
            .unwrap();
    }
    turns.turn();
    let colour = trace.readback(&gpu).unwrap().color;
    let lit = colour
        .chunks_exact(16)
        .filter(|pixel| f32::from_le_bytes(pixel[0..4].try_into().unwrap()) > 0.01)
        .count();
    assert!(lit > (WIDTH * HEIGHT / 4) as usize, "{lit} pixels are lit");
}
