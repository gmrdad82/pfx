use std::path::{Path, PathBuf};

use pfx_bake::plate::codec::{from_oct16, from_rgb9e5};
use pfx_bake::plate::{
    PlateCamera, PlateRun, Sampling, ViewKeys, bake_plates, parse_plates, plate_key, plate_size,
    read_plate,
};
use pfx_gpu::Gpu;
use pfx_gpu::pace::Turns;
use pfx_load::scene::Scene;

const RECIPE: &str = "[plates]
scene = \"desk.scene.toml\"
size = [160, 100]
overscan = 0.1
anchors = [12.0, 17.0]
threshold = 0.02
min_samples = 8
max_samples = 64
";

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("plates-bake")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "block.gltf",
        "panel.gltf",
        "pillar.gltf",
        "ball.gltf",
        "screen.png",
    ] {
        std::fs::copy(
            manifest.join("../load/tests/scenes").join(file),
            root.join(file),
        )
        .unwrap();
    }
    for file in ["desk.scene.toml", "desk.materials.toml"] {
        std::fs::copy(manifest.join("tests/plates").join(file), root.join(file)).unwrap();
    }
    std::fs::copy(
        manifest.join("../text/fonts/EBGaramond[wght].ttf"),
        root.join("serif.ttf"),
    )
    .unwrap();
    root
}

fn edit(root: &Path, from: &str, to: &str) {
    let path = root.join("desk.scene.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "the desk has no {from:?}");
    std::fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

fn key_of(root: &Path) -> String {
    let recipe = parse_plates(RECIPE.as_bytes()).unwrap().unwrap();
    let scenes: Vec<(f32, Scene)> = [12.0f32, 17.0]
        .iter()
        .map(|&hour| {
            let scene = Scene::open_at(root.join("desk.scene.toml"), f64::from(hour)).unwrap();
            (hour, scene.static_subset())
        })
        .collect();
    let camera = PlateCamera::new(
        &ViewKeys::default()
            .camera(&scenes[0].1.camera_or_default())
            .unwrap(),
        recipe.size,
        recipe.overscan,
        recipe.scale,
    );
    let sampling = Sampling {
        threshold: recipe.adaptive.threshold,
        min_samples: recipe.adaptive.min_samples,
        max_samples: recipe.adaptive.max_samples,
        growth: recipe.adaptive.growth,
        seed: 7,
    };
    plate_key("main", &camera, &recipe, &sampling, &scenes).unwrap()
}

#[test]
fn the_key_holds_only_the_static_subset() {
    let root = folder("key");
    let before = key_of(&root);
    assert_eq!(key_of(&root), before);
    edit(
        &root,
        "at = [-0.15, 0.785, 0.12]",
        "at = [-0.35, 0.785, 0.1]",
    );
    edit(&root, "travel = [0.0, 50.0]", "travel = [0.0, 70.0]");
    edit(&root, "text = \"Plate\"", "text = \"Other\"");
    assert_eq!(key_of(&root), before, "a dynamic edit keeps the plate");
    edit(
        &root,
        "scale = [0.9, 0.04, 0.2]",
        "scale = [0.8, 0.04, 0.2]",
    );
    let moved = key_of(&root);
    assert_ne!(moved, before, "a static edit changes the key");
    edit(
        &root,
        "material = \"metal\"",
        "material = \"metal\"\ndynamic = true",
    );
    assert_ne!(
        key_of(&root),
        moved,
        "an object turning dynamic changes the key"
    );
}

fn bake(root: &Path, gpu: &Gpu, force: bool) -> Vec<pfx_bake::plate::PlateOutcome> {
    let recipe = parse_plates(RECIPE.as_bytes()).unwrap().unwrap();
    let mut turns = Turns::default();
    let mut between = |ms: f64| turns.add(ms);
    let mut log = |line: &str| println!("{line}");
    bake_plates(PlateRun {
        gpu,
        recipe: &recipe,
        scene: &root.join("desk.scene.toml"),
        anchors: recipe.anchors.as_deref().unwrap(),
        seed: 7,
        max_samples: None,
        out: &root.join("plates"),
        force,
        between: &mut between,
        log: &mut log,
    })
    .unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_plate_bakes_the_static_desk_at_its_anchors_and_keeps_it_while_its_key_holds() {
    let root = folder("bake");
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let outcomes = bake(&root, &gpu, false);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].baked);
    let plate = read_plate(&root.join("plates/main")).unwrap();
    let size = plate_size([160, 100], 0.1, 1.0);
    assert_eq!([plate.manifest.width, plate.manifest.height], size);
    assert_eq!(plate.color.len(), 2);
    let names: Vec<&str> = plate
        .manifest
        .ids
        .iter()
        .map(|entry| entry.object.as_str())
        .collect();
    assert!(names.contains(&"desk") && names.contains(&"orb"));
    assert!(!names.contains(&"card") && !names.contains(&"lid"));
    let ids = plate.id.as_ref().unwrap();
    let scene = Scene::open(root.join("desk.scene.toml")).unwrap();
    let dynamic: Vec<u32> = scene
        .objects
        .iter()
        .enumerate()
        .filter(|(index, _)| scene.dynamic(*index))
        .map(|(_, object)| object.id)
        .collect();
    assert_eq!(dynamic.len(), 3);
    assert!(ids.iter().all(|id| !dynamic.contains(&u32::from(*id))));
    let desk_id = scene.object("desk").unwrap().id as u16;
    let camera = plate.manifest.camera;
    for point in [[-0.1, 0.78, 0.12], [0.4, 0.78, 0.25], [-0.3, 0.78, -0.25]] {
        let (texel, depth) = camera.project(point, size).unwrap();
        let (x, y) = (texel[0] as u32, texel[1] as u32);
        let index = (y * size[0] + x) as usize;
        assert_eq!(ids[index], desk_id, "{point:?} at {x}, {y}");
        let stored = plate.depth_at(x, y).unwrap();
        assert!((stored - depth).abs() < depth * 0.02, "{stored} {depth}");
        let normal = from_oct16(plate.normal.as_ref().unwrap()[index]);
        assert!(normal[1] > 0.99, "{normal:?}");
        for anchor in 0..2 {
            let color = from_rgb9e5(plate.color[anchor][index]);
            let sun = from_rgb9e5(plate.sun.as_ref().unwrap()[anchor][index]);
            assert!(color.iter().all(|v| *v > 0.0), "{color:?}");
            for k in 0..3 {
                assert!(sun[k] <= color[k] * 1.05 + 1e-3, "{sun:?} {color:?}");
            }
        }
    }
    let lit = (0..plate.texels())
        .filter(|&i| from_rgb9e5(plate.sun.as_ref().unwrap()[0][i])[1] > 0.01)
        .count();
    assert!(lit > plate.texels() / 10, "{lit}");
    assert_ne!(plate.color[0], plate.color[1], "the anchors differ");
    let again = bake(&root, &gpu, false);
    assert!(!again[0].baked, "an unchanged key keeps the plate");
    let first = std::fs::read(root.join("plates/main/color-000.bin")).unwrap();
    let forced = bake(&root, &gpu, true);
    assert!(forced[0].baked);
    assert_eq!(
        std::fs::read(root.join("plates/main/color-000.bin")).unwrap(),
        first,
        "the same inputs bake the same bytes"
    );
    edit(
        &root,
        "at = [-0.15, 0.785, 0.12]",
        "at = [-0.35, 0.785, 0.1]",
    );
    assert!(!bake(&root, &gpu, false)[0].baked);
    edit(
        &root,
        "scale = [0.9, 0.04, 0.2]",
        "scale = [0.8, 0.04, 0.2]",
    );
    assert!(bake(&root, &gpu, false)[0].baked);
    println!(
        "plate {}x{}: {} bytes per anchor, {} shared; samples {:?}",
        size[0],
        size[1],
        outcomes[0].bytes_per_anchor,
        outcomes[0].shared_bytes,
        outcomes[0].samples
    );
}
