use std::path::{Path, PathBuf};

use super::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes")
}

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/scene-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for entry in std::fs::read_dir(fixtures()).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    fn edit(&self, name: &str, from: &str, to: &str) {
        let text = self.read(name);
        assert!(text.contains(from), "{name} has no {from:?}");
        self.write(name, &text.replacen(from, to, 1));
    }

    fn open(&self) -> Result<Scene, SceneError> {
        Scene::open(self.path("room.scene.toml"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn refused(name: &str, scene: &str) -> SceneError {
    let folder = Folder::new(name);
    let path = folder.write("test.scene.toml", scene);
    Scene::open(&path).expect_err("the scene is refused")
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn the_example_scene_loads_with_its_include_library_meshes_and_content() {
    let scene = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    assert_eq!(scene.meshes.len(), 3);
    assert_eq!(scene.objects.len(), 6);
    assert_eq!(scene.lights.keys().collect::<Vec<_>>(), ["cool", "warm"]);
    assert!(!scene.lights["cool"].shadow);
    assert_eq!(scene.fallback.as_deref(), Some("grey"));
    assert_eq!(scene.library.len(), 6);
    let names: Vec<String> = scene
        .files
        .keys()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for file in [
        "room.scene.toml",
        "lights.scene.toml",
        "materials.toml",
        "block.gltf",
        "pillar.gltf",
        "panel.gltf",
        "screen.png",
    ] {
        assert!(
            names.iter().any(|name| name == file),
            "{file} is not among {names:?}"
        );
    }
    let sky = scene.sky.as_ref().unwrap();
    assert_eq!(sky.kind, "room");
    match &sky.environment {
        Environment::Hdr(sky) => assert_eq!((sky.width, sky.height), (64, 32)),
        Environment::Analytic(_) => panic!("a room sky is a texture"),
    }
    let sun = scene.sun.unwrap();
    assert!(close(dot(sun.direction, sun.direction), 1.0));
    assert!(close(sun.intensity, 2.0));
    let content = &scene.contents["screen"];
    assert_eq!((content.width, content.height), (16, 8));
    assert!(content.srgb);
    assert_eq!(content.rgba.len(), 16 * 8 * 4);
    let camera = scene.camera.unwrap();
    assert_eq!(camera.projection, Projection::Perspective { fov: 40.0 });
}

#[test]
fn a_node_mesh_drops_the_nodes_own_placement_and_keeps_its_children() {
    let scene = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    let pillar = &scene.meshes["pillar"];
    let nodes: Vec<&str> = pillar.parts.iter().map(|part| part.node.as_str()).collect();
    assert_eq!(nodes, ["Pillar", "Cap"]);
    assert_eq!(pillar.parts[0].transform, IDENTITY);
    assert!(close(pillar.parts[1].transform[3][1], 1.25));
    assert!(close(pillar.parts[1].transform[3][0], 0.0));
    assert_eq!(pillar.parts[1].overrides.material.as_deref(), Some("metal"));
    assert_eq!(pillar.parts[0].material.as_deref(), Some("stone"));
}

#[test]
fn draws_resolve_materials_by_the_documented_order() {
    let scene = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    let draws = scene.draws();
    assert_eq!(draws.items.len(), 8);
    let material = |object: &str, node: &str| {
        let draw = draws
            .items
            .iter()
            .find(|draw| draw.object == object && draw.node == node)
            .unwrap();
        draws.names[draw.material as usize].clone()
    };
    assert_eq!(material("floor", "Block"), "grey");
    assert_eq!(material("crate", "Block"), "clay");
    assert_eq!(material("left pillar", "Pillar"), "stone");
    assert_eq!(material("left pillar", "Cap"), "metal");
    assert_eq!(material("right pillar", "Pillar"), "blue");
    assert_eq!(material("right pillar", "Cap"), "metal");
    assert_eq!(material("screen", "Panel"), "screen");
    let mut sorted = draws.names.clone();
    sorted.sort();
    assert_eq!(draws.names, sorted);
    for (name, value) in draws.names.iter().zip(&draws.materials) {
        assert_eq!(scene.library.get(name), Some(value));
    }
    let crate_draw = draws
        .items
        .iter()
        .find(|draw| draw.object == "crate")
        .unwrap();
    assert_eq!(crate_draw.id, 3);
    assert!(close(crate_draw.model[3][0], -0.9));
    let cap = draws
        .items
        .iter()
        .find(|draw| draw.object == "left pillar" && draw.node == "Cap")
        .unwrap();
    assert!(close(cap.model[3][0], 0.6));
    assert!(close(cap.model[3][1], 1.25));
}

#[test]
fn computed_normals_tangents_and_uvs_fill_what_the_gltf_leaves_out() {
    let folder = Folder::new("computed");
    folder.edit(
        "room.scene.toml",
        "file = \"block.gltf\"",
        "file = \"ball.gltf\"",
    );
    let scene = folder.open().unwrap();
    let geometry = &scene.meshes["block"].parts[0].geometry;
    assert_eq!(geometry.normals.len(), geometry.positions.len());
    assert_eq!(geometry.tangents.len(), geometry.positions.len());
    for (normal, position) in geometry.normals.iter().zip(&geometry.positions) {
        assert!(close(dot(*normal, *normal), 1.0));
        if dot(*position, *position) > 0.01 && position[1].abs() < 0.49 {
            assert!(dot(*normal, *position) > 0.0, "{normal:?} at {position:?}");
        }
    }
}

#[test]
fn an_object_parent_and_its_transform_order_compose() {
    let folder = Folder::new("parent");
    let mut text = folder.read("room.scene.toml");
    text.push_str(
        "\n[[object]]\nname = \"child\"\nmesh = \"block\"\nparent = \"crate\"\nat = [0.0, 1.0, 0.0]\n",
    );
    folder.write("room.scene.toml", &text);
    let scene = folder.open().unwrap();
    let crate_model = scene.object("crate").unwrap().model;
    let child = scene.object("child").unwrap().model;
    let expected = multiply(crate_model, model([0.0, 1.0, 0.0], [0.0; 3], [1.0; 3]));
    for (a, b) in child.iter().flatten().zip(expected.iter().flatten()) {
        assert!(close(*a, *b));
    }
    let turned = model([1.0, 2.0, 3.0], [0.0, 90.0, 0.0], [2.0; 3]);
    assert!(close(turned[0][2], -2.0));
    assert!(close(turned[2][0], 2.0));
    assert_eq!(turned[3], [1.0, 2.0, 3.0, 1.0]);
}

#[test]
fn an_unknown_key_is_refused_with_its_file_line_and_the_allowed_keys() {
    let error = refused(
        "unknown-key",
        "materials = [\"materials.toml\"]\n\n[[light]]\nname = \"a\"\nposition = [0.0, 1.0, 0.0]\ncolour = [1.0, 1.0, 1.0]\nintensity = 1.0\n",
    );
    assert!(error.file.ends_with("test.scene.toml"));
    assert_eq!(error.line, Some(6));
    assert!(error.message.contains("colour"), "{error}");
    assert!(error.message.contains("expected one of"), "{error}");
    assert!(error.message.contains("'intensity'"), "{error}");
    assert!(error.to_string().contains("test.scene.toml:6:"), "{error}");
    let top = refused("unknown-top", "materials = []\nlights = 3\n");
    assert_eq!(top.line, Some(2));
    assert!(top.message.contains("lights"), "{top}");
    assert!(top.message.contains("'light'"), "{top}");
}

#[test]
fn an_unknown_key_in_an_include_names_the_included_file() {
    let folder = Folder::new("include-key");
    folder.edit(
        "lights.scene.toml",
        "range = 8.0\nshadow",
        "power = 8.0\nshadow",
    );
    let error = folder.open().unwrap_err();
    assert!(error.file.ends_with("lights.scene.toml"), "{error}");
    assert_eq!(error.line, Some(18));
    assert!(error.message.contains("power"), "{error}");
}

#[test]
fn refusals_name_what_is_wrong() {
    let cases = [
        (
            "bad-shadow",
            "[mesh.b]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"x\"\nmesh = \"b\"\nshadow = \"maybe\"\n",
            "unknown variant 'maybe', expected one of",
            7,
        ),
        (
            "no-mesh",
            "[[object]]\nname = \"x\"\nmesh = \"nothing\"\n",
            "nothing names no mesh of this scene",
            3,
        ),
        ("absolute", "[mesh.b]\nfile = \"/b.gltf\"\n", "absolute", 2),
        (
            "no-material",
            "materials = [\"materials.toml\"]\n[mesh.b]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"x\"\nmesh = \"b\"\nmaterial = \"gold\"\n",
            "gold names no material of the scene's li",
            8,
        ),
        (
            "no-node",
            "[mesh.b]\nfile = \"pillar.gltf\"\nnode = \"Roof\"\n",
            "no node Roof",
            1,
        ),
        (
            "override-node",
            "[mesh.b]\nfile = \"pillar.gltf\"\n\n[mesh.b.nodes.Roof]\nhidden = true\n",
            "node Roof",
            4,
        ),
        (
            "sky-kind",
            "[sky]\nkind = \"cloud\"\n",
            "unknown variant 'cloud', expected one of",
            2,
        ),
        (
            "room-path",
            "[sky]\nkind = \"room\"\npath = \"x.hdr\"\n",
            "[sky.room]",
            1,
        ),
        (
            "prepare-empty",
            "[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\n[sky.prepare]\n",
            "sky.hdr does not exist",
            3,
        ),
        (
            "sun-model",
            "[sun]\nmodel = \"moon\"\n",
            "unknown variant 'moon', expected 'daylig",
            2,
        ),
        (
            "sun-mixed",
            "[sun]\nmodel = \"daylight\"\nhour = 9.0\nirradiance = 2.0\n",
            "not toward",
            2,
        ),
        (
            "sun-hour",
            "[sun]\nmodel = \"daylight\"\nhour = 30.0\n",
            "outside 0 to 24",
            3,
        ),
        (
            "camera-both",
            "[camera]\nfov = 30.0\nfocal = 50.0\n",
            "not both",
            3,
        ),
        (
            "camera-ortho",
            "[camera]\nprojection = \"orthographic\"\n",
            "positive height",
            1,
        ),
        (
            "camera-same",
            "[camera]\nat = [0.0, 0.0, 0.0]\n",
            "must differ",
            1,
        ),
        (
            "light-range",
            "[[light]]\nname = \"l\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\nrange = 0.0\n",
            "positive range",
            1,
        ),
        (
            "finish-key",
            "[finish]\nglow = 1.0\n",
            "unknown key 'glow', expected one of 'fil",
            2,
        ),
        (
            "finish-file",
            "[finish]\nfile = \"look.toml\"\nstyle = \"noir\"\n",
            "file alone",
            2,
        ),
        (
            "clip-three",
            "[mesh.b]\nfile = \"block.gltf\"\n[[object]]\nname = \"x\"\nmesh = \"b\"\nclip = [[0.0, 1.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]\n",
            "at most two",
            3,
        ),
        (
            "content-missing",
            "[mesh.b]\nfile = \"block.gltf\"\n[[object]]\nname = \"x\"\nmesh = \"b\"\ncontent = \"tv\"\n",
            "tv names no content of this scene",
            6,
        ),
        (
            "parent-missing",
            "[mesh.b]\nfile = \"block.gltf\"\n[[object]]\nname = \"x\"\nmesh = \"b\"\nparent = \"y\"\n",
            "y names no object of this scene",
            6,
        ),
        (
            "parent-cycle",
            "[mesh.b]\nfile = \"block.gltf\"\n[[object]]\nname = \"x\"\nmesh = \"b\"\nparent = \"y\"\n[[object]]\nname = \"y\"\nmesh = \"b\"\nparent = \"x\"\n",
            "cycle",
            6,
        ),
        (
            "twice",
            "[mesh.b]\nfile = \"block.gltf\"\n[[object]]\nname = \"x\"\nmesh = \"b\"\n[[object]]\nname = \"x\"\nmesh = \"b\"\n",
            "named twice",
            7,
        ),
        (
            "fallback",
            "fallback = \"gold\"\n",
            "gold names no material of the scene's li",
            1,
        ),
        (
            "missing-file",
            "[mesh.b]\nfile = \"nothing.gltf\"\n",
            "nothing.gltf",
            2,
        ),
        (
            "preset-key",
            "[camera.preset]\norbit = { spin = 1.0 }\n",
            "spin",
            2,
        ),
    ];
    for (name, text, needle, line) in cases {
        let error = refused(name, text);
        assert!(
            error.to_string().contains(needle),
            "{name}: {error} does not say {needle:?}"
        );
        assert_eq!(error.line, Some(line), "{name}: {error}");
    }
}

#[test]
fn an_include_cycle_and_a_second_singleton_are_refused() {
    let folder = Folder::new("include-cycle");
    folder.write("a.scene.toml", "include = [\"b.scene.toml\"]\n");
    folder.write("b.scene.toml", "include = [\"a.scene.toml\"]\n");
    let error = Scene::open(folder.path("a.scene.toml")).unwrap_err();
    assert!(error.message.contains("includes itself"), "{error}");

    let folder = Folder::new("singleton");
    folder.write("sky.scene.toml", "[sky]\nkind = \"analytic\"\n");
    folder.write(
        "main.scene.toml",
        "include = [\"sky.scene.toml\"]\n\n[sky]\nkind = \"analytic\"\n",
    );
    let error = Scene::open(folder.path("main.scene.toml")).unwrap_err();
    assert!(error.file.ends_with("main.scene.toml"));
    assert_eq!(error.line, Some(3));
    assert!(
        error.message.contains("sky.scene.toml sets it already"),
        "{error}"
    );

    let folder = Folder::new("duplicate-light");
    folder.write(
        "main.scene.toml",
        "include = [\"lights.scene.toml\"]\n\n[[light]]\nname = \"warm\"\nposition = [0.0, 0.0, 0.0]\nintensity = 1.0\n",
    );
    let error = Scene::open(folder.path("main.scene.toml")).unwrap_err();
    assert!(
        error.message.contains("light warm is named twice"),
        "{error}"
    );
}

#[test]
fn a_material_in_two_libraries_is_refused() {
    let folder = Folder::new("two-libraries");
    folder.write("more.toml", "[materials.clay]\nbase = [1.0, 0.0, 0.0]\n");
    folder.write(
        "main.scene.toml",
        "materials = [\"materials.toml\", \"more.toml\"]\n",
    );
    let error = Scene::open(folder.path("main.scene.toml")).unwrap_err();
    assert!(error.file.ends_with("more.toml"));
    assert!(
        error.message.contains("material clay is also in"),
        "{error}"
    );
}

#[test]
fn a_daylight_sun_drives_an_analytic_sky_and_finish_and_preset_read() {
    let folder = Folder::new("daylight");
    folder.write("look.toml", "style = \"noir\"\n");
    let path = folder.write(
        "main.scene.toml",
        "[sun]\nmodel = \"daylight\"\nhour = 9.0\n\n[sky]\nkind = \"analytic\"\nturbidity = 4.0\n\n[finish]\nfile = \"look.toml\"\n\n[camera]\nprojection = \"orthographic\"\nheight = 4.0\n\n[camera.preset]\ngain = 0.5\norbit = { yaw = 2.0 }\ndepth = { focus = 3.0, blur = 0.2, floor = 0.1 }\n",
    );
    let scene = Scene::open(&path).unwrap();
    let sun = scene.sun.unwrap();
    assert!(sun.daylight.is_some());
    assert!(close(sun.hour, 9.0));
    match &scene.sky.as_ref().unwrap().environment {
        Environment::Analytic(sky) => {
            assert!(close(sky.turbidity, 4.0));
            for k in 0..3 {
                assert!(close(sky.sun[k], sun.direction[k]));
            }
        }
        Environment::Hdr(_) => panic!("an analytic sky"),
    }
    assert!(scene.finish.is_some());
    let camera = scene.camera.unwrap();
    assert_eq!(camera.projection, Projection::Orthographic { height: 4.0 });
    assert!(close(camera.preset.gain, 0.5));
    assert!(close(camera.preset.orbit.yaw, 2.0 * pfx_core::camera::DEG));
    assert!(camera.preset.depth.is_some());
}

#[test]
fn an_hdr_sky_is_prepared_turned_and_scaled() {
    let folder = Folder::new("hdr");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sky.hdr"),
        folder.path("sky.hdr"),
    )
    .unwrap();
    let plain = folder.write(
        "plain.scene.toml",
        "[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\n",
    );
    let scaled = folder.write(
        "scaled.scene.toml",
        "[sky]\nkind = \"hdr\"\npath = \"sky.hdr\"\nintensity = 2.0\n\n[sky.prepare]\nmean = 0.5\n",
    );
    let plain = Scene::open(&plain).unwrap();
    let scaled = Scene::open(&scaled).unwrap();
    let mean = |scene: &Scene| match &scene.sky.as_ref().unwrap().environment {
        Environment::Hdr(sky) => sky.mean_luminance(),
        Environment::Analytic(_) => panic!("an hdr sky"),
    };
    assert!((mean(&scaled) - 1.0).abs() < 1e-3, "{}", mean(&scaled));
    assert!(mean(&plain) > 0.0);
    assert_ne!(plain.sky, scaled.sky);
}

#[test]
fn the_same_scene_diffs_empty() {
    let a = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    let b = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    assert_eq!(a, b);
    let diff = a.diff(&b);
    assert!(diff.is_empty(), "{diff:?}");
    assert!(!diff.draws());
}

fn diff_after(name: &str, edit: impl Fn(&Folder)) -> SceneDiff {
    let folder = Folder::new(name);
    let before = folder.open().unwrap();
    edit(&folder);
    let after = folder.open().unwrap();
    before.diff(&after)
}

fn only(changes: &Changes, kind: &str, name: &str) {
    let expected = vec![name.to_string()];
    let got = match kind {
        "added" => &changes.added,
        "changed" => &changes.changed,
        _ => &changes.removed,
    };
    assert_eq!(got, &expected, "{changes:?}");
}

#[test]
fn a_material_colour_edit_changes_only_that_material() {
    let diff = diff_after("diff-material", |folder| {
        folder.edit(
            "materials.toml",
            "base = [0.7, 0.32, 0.22]",
            "base = [0.2, 0.6, 0.3]",
        );
    });
    only(&diff.materials, "changed", "clay");
    assert!(diff.meshes.is_empty() && diff.objects.is_empty() && diff.lights.is_empty());
    assert!(!diff.sky && !diff.sun && !diff.camera && !diff.finish);
    assert!(diff.draws());
}

#[test]
fn a_light_power_edit_changes_only_that_light() {
    let diff = diff_after("diff-light", |folder| {
        folder.edit("lights.scene.toml", "intensity = 6.0", "intensity = 12.0");
    });
    only(&diff.lights, "changed", "warm");
    assert!(!diff.draws());
    assert!(diff.materials.is_empty() && !diff.sky);
}

#[test]
fn a_mesh_swap_changes_the_mesh_and_not_the_objects() {
    let diff = diff_after("diff-mesh", |folder| {
        folder.edit(
            "room.scene.toml",
            "file = \"block.gltf\"",
            "file = \"ball.gltf\"",
        );
    });
    only(&diff.meshes, "changed", "block");
    assert!(diff.objects.is_empty());
    let diff = diff_after("diff-gltf", |folder| {
        std::fs::copy(folder.path("ball.gltf"), folder.path("block.gltf")).unwrap();
    });
    only(&diff.meshes, "changed", "block");
}

#[test]
fn a_node_override_changes_its_mesh() {
    let diff = diff_after("diff-override", |folder| {
        folder.edit(
            "room.scene.toml",
            "material = \"metal\"",
            "material = \"clay\"",
        );
    });
    only(&diff.meshes, "changed", "pillar");
}

#[test]
fn objects_are_added_removed_moved_and_reordered() {
    let diff = diff_after("diff-add", |folder| {
        let mut text = folder.read("room.scene.toml");
        text.push_str("\n[[object]]\nname = \"extra\"\nmesh = \"block\"\nat = [2.0, 0.5, 0.0]\n");
        folder.write("room.scene.toml", &text);
    });
    only(&diff.objects, "added", "extra");
    assert!(!diff.order);
    let diff = diff_after("diff-remove", |folder| {
        folder.edit(
            "room.scene.toml",
            "[[object]]\nname = \"crate\"\nmesh = \"block\"\nat = [-0.9, 0.35, 0.0]\nrotate = [0.0, 25.0, 0.0]\nscale = 0.7\nid = \"vnrkf8pd3j\"\n",
            "",
        );
    });
    only(&diff.objects, "removed", "crate");
    let diff = diff_after("diff-move", |folder| {
        folder.edit(
            "room.scene.toml",
            "at = [-0.9, 0.35, 0.0]",
            "at = [-1.2, 0.35, 0.0]",
        );
    });
    only(&diff.objects, "changed", "crate");
    let diff = diff_after("diff-clip", |folder| {
        folder.edit(
            "room.scene.toml",
            "scale = 0.7\n",
            "scale = 0.7\nclip = [[0.0, 1.0, 0.0, 0.4]]\nshadow = \"none\"\ntwo_sided = true\n",
        );
    });
    only(&diff.objects, "changed", "crate");
}

#[test]
fn sky_sun_camera_finish_and_content_changes_are_flagged() {
    let diff = diff_after("diff-sky", |folder| {
        folder.edit("room.scene.toml", "power = 6.0", "power = 9.0");
    });
    assert!(diff.sky && !diff.sun && !diff.draws());
    let diff = diff_after("diff-sun", |folder| {
        folder.edit("room.scene.toml", "irradiance = 2.0", "irradiance = 3.0");
    });
    assert!(diff.sun && !diff.sky);
    let diff = diff_after("diff-camera", |folder| {
        folder.edit("room.scene.toml", "fov = 40.0", "fov = 30.0");
    });
    assert!(diff.camera && !diff.sun);
    let diff = diff_after("diff-finish", |folder| {
        let mut text = folder.read("room.scene.toml");
        text.push_str("\n[finish]\nstyle = \"noir\"\n");
        folder.write("room.scene.toml", &text);
    });
    assert!(diff.finish);
    let diff = diff_after("diff-content", |folder| {
        let mut bytes = std::fs::read(fixtures().join("../data/swatch8.png")).unwrap();
        std::fs::write(folder.path("screen.png"), &mut bytes).unwrap();
    });
    only(&diff.contents, "changed", "screen");
}

#[test]
fn a_trace_table_turns_on_transmissive_shadows_for_the_tracer_alone() {
    let plain = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    assert!(!plain.trace.transmissive_shadows);
    let diff = diff_after("diff-trace", |folder| {
        let mut text = folder.read("room.scene.toml");
        text.push_str("\n[trace]\ntransmissive_shadows = true\n");
        folder.write("room.scene.toml", &text);
    });
    assert!(diff.trace && !diff.draws() && !diff.finish, "{diff:?}");
    let folder = Folder::new("trace-on");
    let mut text = folder.read("room.scene.toml");
    text.push_str("\n[trace]\ntransmissive_shadows = true\n");
    folder.write("room.scene.toml", &text);
    assert!(folder.open().unwrap().trace.transmissive_shadows);
    let error = refused("trace-key", "[trace]\nglass_shadows = true\n");
    assert_eq!(error.line, Some(2));
    assert!(error.message.contains("transmissive_shadows"), "{error}");
    let folder = Folder::new("trace-twice");
    folder.write("trace.scene.toml", "[trace]\ntransmissive_shadows = true\n");
    folder.write(
        "main.scene.toml",
        "include = [\"trace.scene.toml\"]\n\n[trace]\ntransmissive_shadows = false\n",
    );
    let error = Scene::open(folder.path("main.scene.toml")).unwrap_err();
    assert!(
        error.message.contains("trace.scene.toml sets it already"),
        "{error}"
    );
}

#[test]
fn a_trace_table_sets_the_firefly_clamp_and_the_glossy_filter_within_their_ranges() {
    let plain = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    assert_eq!(
        (plain.trace.clamp_indirect, plain.trace.filter_glossy),
        (0.0, 0.0)
    );
    let folder = Folder::new("trace-fireflies");
    let mut text = folder.read("room.scene.toml");
    text.push_str("\n[trace]\nclamp_indirect = 10\nfilter_glossy = 0.15\n");
    folder.write("room.scene.toml", &text);
    let trace = folder.open().unwrap().trace;
    assert_eq!((trace.clamp_indirect, trace.filter_glossy), (10.0, 0.15));
    assert!(!trace.transmissive_shadows);
    for (label, table, key) in [
        (
            "trace-clamp-low",
            "[trace]\nclamp_indirect = -1.0\n",
            "clamp_indirect",
        ),
        (
            "trace-clamp-high",
            "[trace]\nclamp_indirect = 20000.0\n",
            "clamp_indirect",
        ),
        (
            "trace-clamp-nan",
            "[trace]\nclamp_indirect = nan\n",
            "clamp_indirect",
        ),
        (
            "trace-filter-low",
            "[trace]\nfilter_glossy = -0.1\n",
            "filter_glossy",
        ),
        (
            "trace-filter-high",
            "[trace]\nfilter_glossy = 1.5\n",
            "filter_glossy",
        ),
    ] {
        let error = refused(label, table);
        assert_eq!(error.line, Some(2), "{error}");
        assert!(error.message.contains(key), "{error}");
    }
    let diff = diff_after("diff-trace-fireflies", |folder| {
        let mut text = folder.read("room.scene.toml");
        text.push_str("\n[trace]\nfilter_glossy = 0.2\n");
        folder.write("room.scene.toml", &text);
    });
    assert!(diff.trace && !diff.draws() && !diff.finish, "{diff:?}");
}

#[test]
fn an_analytic_sky_follows_a_sun_edit() {
    let folder = Folder::new("diff-analytic-sun");
    folder.write(
        "main.scene.toml",
        "[sun]\nmodel = \"daylight\"\nhour = 9.0\n\n[sky]\nkind = \"analytic\"\n",
    );
    let before = Scene::open(folder.path("main.scene.toml")).unwrap();
    folder.edit("main.scene.toml", "hour = 9.0", "hour = 15.0");
    let after = Scene::open(folder.path("main.scene.toml")).unwrap();
    let diff = before.diff(&after);
    assert!(diff.sun && diff.sky);
}

#[test]
fn an_emitter_is_a_sphere_with_its_own_emissive_material() {
    let folder = Folder::new("emitter");
    let path = folder.write(
        "main.scene.toml",
        "[[emitter]]\nname = \"bulb\"\nposition = [0.0, 2.0, 0.0]\nradius = 0.1\ncolor = [1.0, 0.5, 0.25]\nintensity = 2.0\n",
    );
    let scene = Scene::open(&path).unwrap();
    let draws = scene.draws();
    assert_eq!(draws.items.len(), 1);
    assert_eq!(draws.names, ["emitter bulb"]);
    let triangles = draws.items[0].geometry.indices.len() / 3;
    assert_eq!(triangles, 528);
    let area = std::f32::consts::PI * 0.01;
    assert!(close(draws.materials[0].emission[0], 2.0 / area));
    assert!(close(draws.items[0].model[1][1], 0.1));
    assert_eq!(draws.items[0].shadow, Shadow::None);
}

#[test]
fn a_broken_file_does_not_change_the_watched_scene_and_a_fix_recovers() {
    let folder = Folder::new("watch-broken");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    let good = watch.scene().clone();
    assert!(watch.check().is_none());
    folder.edit("lights.scene.toml", "intensity = 6.0", "intensity = = 6.0");
    let error = watch.check().unwrap().unwrap_err();
    assert!(error.file.ends_with("lights.scene.toml"), "{error}");
    assert!(error.line.is_some());
    assert_eq!(watch.scene(), &good);
    assert!(watch.error().is_some());
    assert!(watch.check().is_none());
    folder.edit("lights.scene.toml", "intensity = = 6.0", "intensity = 7.0");
    let reload = watch.check().unwrap().unwrap();
    assert!(reload.recovered);
    only(&reload.diff.lights, "changed", "warm");
    assert!(watch.error().is_none());
    assert_eq!(watch.scene(), &reload.scene);
}

#[test]
fn a_save_without_a_change_reloads_nothing() {
    let folder = Folder::new("watch-same");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    let text = folder.read("materials.toml");
    folder.write("materials.toml", &text);
    assert!(watch.check().is_none());
    folder.write("materials.toml", &format!("{text}\n# a note\n"));
    assert!(watch.check().is_none());
}

#[test]
fn the_watch_hears_an_edit_through_notify() {
    let folder = Folder::new("watch-notify");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    watch.set_debounce(std::time::Duration::from_millis(20));
    folder.edit(
        "materials.toml",
        "base = [0.15, 0.25, 0.7]",
        "base = [0.7, 0.25, 0.15]",
    );
    let mut reload = None;
    for _ in 0..400 {
        if let Some(result) = watch.poll() {
            reload = Some(result.unwrap());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let reload = reload.expect("the edit is heard within four seconds");
    only(&reload.diff.materials, "changed", "blue");
}

fn opened(name: &str, scene: &str) -> Scene {
    let folder = Folder::new(name);
    let path = folder.write("test.scene.toml", scene);
    Scene::open(&path).unwrap()
}

fn near(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= tolerance)
}

fn transform(matrix: &Matrix, point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| {
        matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row]
    })
}

const BLOCK: &str = "[mesh.b]\nfile = \"block.gltf\"\n";

#[test]
fn round_two_refusals_name_what_is_wrong() {
    let cases = [
        (
            "sun-radius",
            "[sun]\nmodel = \"daylight\"\nhour = 9.0\nradius = 12.0\n",
            "outside 0 to 10",
            4,
        ),
        (
            "finish-pass-style",
            "[finish]\nexposure = 1.0\n\n[[finish.pass]]\nwarmth = 0.1\n\n[[finish.pass]]\nstyle = \"noir\"\n",
            "not style",
            8,
        ),
        (
            "finish-pass-empty",
            "[finish]\n[[finish.pass]]\n",
            "builds no pass",
            2,
        ),
        (
            "finish-pass-key",
            "[finish]\n\n[[finish.pass]]\nglow = 1.0\n",
            "unknown key 'glow', expected one of 'abe",
            4,
        ),
        (
            "camera-ortho-fstop",
            "[camera]\nprojection = \"orthographic\"\nheight = 2.0\nfstop = 2.8\n",
            "not fov, focal, sensor, fstop or focus",
            2,
        ),
        (
            "camera-focus-alone",
            "[camera]\nfocus = 3.0\n",
            "focus needs fstop",
            2,
        ),
        (
            "camera-fstop",
            "[camera]\nfstop = 0.0\n",
            "fstop must be positive",
            2,
        ),
        (
            "camera-lens-key",
            "[camera]\naperture = 1.0\n",
            "unknown key 'aperture', expected one of",
            2,
        ),
        (
            "haze-bounds",
            "[haze]\nlo = [0.0, 0.0, 0.0]\nhi = [1.0, 0.0, 1.0]\n",
            "lo must lie below hi",
            1,
        ),
        (
            "haze-amount",
            "[haze]\nlo = [0.0, 0.0, 0.0]\nhi = [1.0, 1.0, 1.0]\namount = 2.0\n",
            "outside 0 to 1",
            4,
        ),
        (
            "haze-key",
            "[haze]\nlo = [0.0, 0.0, 0.0]\nhi = [1.0, 1.0, 1.0]\ndensity = 1.0\n",
            "unknown key 'density', expected one of '",
            4,
        ),
        (
            "mix-one",
            "[sky]\nkind = \"mix\"\n[[sky.layer]]\nkind = \"analytic\"\n",
            "two or more",
            2,
        ),
        (
            "mix-key",
            "[sky]\nkind = \"mix\"\nintensity = 2.0\n[[sky.layer]]\nkind = \"analytic\"\n[[sky.layer]]\nkind = \"analytic\"\n",
            "only [[sky.layer]]",
            2,
        ),
        (
            "layer-not-mix",
            "[sky]\nkind = \"analytic\"\n[[sky.layer]]\nkind = \"analytic\"\n",
            "only a mix sky",
            3,
        ),
        (
            "layer-weight",
            "[sky]\nkind = \"mix\"\n[[sky.layer]]\nkind = \"analytic\"\nweight = -1.0\n[[sky.layer]]\nkind = \"analytic\"\n",
            "nonnegative",
            5,
        ),
        (
            "sky-weight",
            "[sky]\nkind = \"analytic\"\nweight = 1.0\n",
            "belongs to a [[sky.layer]]",
            3,
        ),
        (
            "alpha-cutoff",
            &format!("{BLOCK}[[object]]\nname = \"x\"\nmesh = \"b\"\nalpha_cutoff = 1.5\n"),
            "outside 0 to 1",
            6,
        ),
        (
            "mover-object",
            &format!(
                "{BLOCK}[[mover]]\nname = \"m\"\nobjects = [\"ghost\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "ghost names no object of this scene",
            5,
        ),
        (
            "mover-kind",
            &format!(
                "{BLOCK}[[mover]]\nname = \"m\"\nobjects = [\"x\"]\nkind = \"spin\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "unknown variant 'spin', expected 'turn'",
            6,
        ),
        (
            "mover-axis",
            &format!(
                "{BLOCK}[[object]]\nname = \"x\"\nmesh = \"b\"\n[[mover]]\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [0.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "nonzero axis",
            6,
        ),
        (
            "mover-twice",
            &format!(
                "{BLOCK}[[object]]\nname = \"x\"\nmesh = \"b\"\n[[mover]]\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\n[[mover]]\nname = \"n\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 1.0]\n"
            ),
            "one at most",
            14,
        ),
        (
            "mover-key",
            &format!(
                "{BLOCK}[[mover]]\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\nspeed = 2.0\n"
            ),
            "unknown key 'speed', expected one of 'id",
            9,
        ),
    ];
    for (name, text, needle, line) in cases {
        let error = refused(name, text);
        assert!(
            error.to_string().contains(needle),
            "{name}: {error} does not say {needle:?}"
        );
        assert_eq!(error.line, Some(line), "{name}: {error}");
    }
    let error = refused(
        "mover-clips",
        &format!(
            "{BLOCK}[[object]]\nname = \"x\"\nmesh = \"b\"\nclip = [[0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]]\n[[mover]]\nname = \"m\"\nobjects = [\"x\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 1.0]\nclip = [[0.0, 0.0, 1.0, 0.0]]\n"
        ),
    );
    assert!(error.message.contains("at most two"), "{error}");
}

#[test]
fn a_sun_radius_is_read_for_both_models() {
    let daylight = opened(
        "sun-radius-daylight",
        "[sun]\nmodel = \"daylight\"\nhour = 9.0\nradius = 1.5\n",
    );
    assert!(close(daylight.sun.unwrap().radius, 1.5));
    let authored = opened(
        "sun-radius-authored",
        "[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\nirradiance = 1.0\nradius = 0.8\n",
    );
    assert!(close(authored.sun.unwrap().radius, 0.8));
    let plain = opened(
        "sun-radius-none",
        "[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\nirradiance = 1.0\n",
    );
    assert_eq!(plain.sun.unwrap().radius, 0.0);
    assert_eq!(Sun::dark().radius, 0.0);
}

#[test]
fn an_authored_sun_follows_the_hour_and_is_as_authored_at_the_reference() {
    let sun = |hour: &str| {
        let label: String = hour.chars().filter(char::is_ascii_digit).collect();
        opened(
            &format!("authored-hour-{label}"),
            &format!("[sun]\nmodel = \"authored\"\ntoward = [-0.4, 0.8, 0.45]\ncolor = [0.9, 0.85, 0.8]\nirradiance = 2.0\n{hour}"),
        )
        .sun
        .unwrap()
    };
    let noon = sun("");
    assert_eq!(noon.direction, unit([-0.4, 0.8, 0.45]));
    assert_eq!(noon.color, [0.9, 0.85, 0.8]);
    assert_eq!(noon.intensity, 2.0);
    let near_noon = sun("hour = 12.01\n");
    assert!(
        near(near_noon.direction, noon.direction, 1e-2),
        "{:?}",
        near_noon.direction
    );
    assert!(
        near(near_noon.color, noon.color, 1e-2),
        "{:?}",
        near_noon.color
    );
    assert!(
        (near_noon.intensity - 2.0).abs() < 1e-2,
        "{}",
        near_noon.intensity
    );
    assert!(close(noon.hour, 12.0));
    let explicit = sun("hour = 12.0\n");
    assert_eq!(noon, explicit);
    let late = sun("hour = 17.5\n");
    assert!(
        late.direction[1] < noon.direction[1] - 0.1,
        "{:?}",
        late.direction
    );
    assert!(!near(late.direction, noon.direction, 0.05));
    assert!(
        late.color[2] / late.color[0] < noon.color[2] / noon.color[0],
        "{:?}",
        late.color
    );
    assert!(late.intensity < noon.intensity);
    assert!(close(late.hour, 17.5));
    let elsewhere = opened(
        "authored-place",
        "[sun]\nmodel = \"authored\"\ntoward = [-0.4, 0.8, 0.45]\nirradiance = 2.0\nhour = 9.0\nlatitude = 60.0\nday = 355.0\nheading = 90.0\nreference_hour = 13.0\n",
    );
    assert!(elsewhere.sun.unwrap().daylight.is_none());
}

#[test]
fn finish_passes_apply_in_the_order_written() {
    let names = |scene: &Scene| -> Vec<&'static str> {
        scene
            .finish
            .as_ref()
            .unwrap()
            .chain
            .passes
            .iter()
            .map(|pass| pass.name())
            .collect()
    };
    let inline = opened("finish-inline", "[finish]\ntone = \"agx\"\nwarmth = 0.2\n");
    let ordered = opened(
        "finish-ordered",
        "[finish]\nexposure = 1.2\n\n[[finish.pass]]\nwarmth = 0.2\n\n[[finish.pass]]\ntone = \"agx\"\n\n[[finish.pass]]\nvignette = 0.3\n",
    );
    let reversed = opened(
        "finish-reversed",
        "[finish]\nexposure = 1.2\n\n[[finish.pass]]\nvignette = 0.3\n\n[[finish.pass]]\ntone = \"agx\"\n\n[[finish.pass]]\nwarmth = 0.2\n",
    );
    assert_eq!(names(&ordered), ["exposure", "warmth", "tone", "vignette"]);
    assert_eq!(names(&reversed), ["exposure", "vignette", "tone", "warmth"]);
    assert_ne!(ordered.finish, reversed.finish);
    assert!(names(&inline).contains(&"tone"));
    let folder = Folder::new("finish-file-pass");
    folder.write("look.toml", "style = \"noir\"\n");
    let path = folder.write(
        "main.scene.toml",
        "[finish]\nfile = \"look.toml\"\n\n[[finish.pass]]\nwarmth = 0.1\n",
    );
    let scene = Scene::open(&path).unwrap();
    assert_eq!(names(&scene).last(), Some(&"warmth"));
    let error = refused(
        "finish-file-inline",
        "[finish]\nfile = \"look.toml\"\nexposure = 1.0\n\n[[finish.pass]]\nwarmth = 0.1\n",
    );
    assert!(error.message.contains("file alone"), "{error}");
}

#[test]
fn the_camera_lens_shifts_and_focuses() {
    let scene = opened(
        "camera-lens",
        "[camera]\nat = [0.0, 1.0, 5.0]\nlook_at = [0.0, 1.0, 0.0]\nfocal = 50.0\nsensor = 24.0\nshift = [0.1, -0.2]\nfstop = 2.0\n",
    );
    let camera = scene.camera.unwrap();
    assert_eq!(camera.shift, [0.1, -0.2]);
    let depth = camera.depth_of_field.unwrap();
    assert!(close(depth.distance, 5.0));
    assert!(close(depth.focal, 0.05));
    assert!(close(depth.aperture(), 0.025));
    let projection = camera.projection(1.5);
    assert!(close(projection[2][0], 0.1) && close(projection[2][1], -0.2));
    let plain = Camera {
        shift: [0.0; 2],
        ..camera
    };
    let clip = |m: &Matrix, p: [f32; 4]| -> [f32; 2] {
        let v: [f32; 4] = std::array::from_fn(|row| (0..4).map(|k| m[k][row] * p[k]).sum());
        [v[0] / v[3], v[1] / v[3]]
    };
    let point = [0.3, 0.2, -4.0, 1.0];
    let shifted = clip(&projection, point);
    let centred = clip(&plain.projection(1.5), point);
    assert!(close(shifted[0], centred[0] - 0.1) && close(shifted[1], centred[1] + 0.2));
    let focused = opened(
        "camera-focus",
        "[camera]\nat = [0.0, 0.0, 5.0]\nfov = 40.0\nfstop = 4.0\nfocus = 2.5\n",
    );
    let depth = focused.camera.unwrap().depth_of_field.unwrap();
    assert!(close(depth.distance, 2.5));
    assert!(close(
        depth.focal,
        0.024 / (2.0 * 20.0_f32.to_radians().tan())
    ));
    let ortho = opened(
        "camera-ortho-shift",
        "[camera]\nprojection = \"orthographic\"\nheight = 2.0\nshift = [0.5, 0.0]\n",
    );
    let projection = ortho.camera.unwrap().projection(1.0);
    assert!(close(projection[3][0], -0.5));
    assert!(
        Scene::open(fixtures().join("room.scene.toml"))
            .unwrap()
            .camera
            .unwrap()
            .depth_of_field
            .is_none()
    );
}

#[test]
fn haze_reads_its_bounds_amount_and_overrides() {
    let scene = opened(
        "haze",
        "[haze]\nlo = [-2.0, 0.0, -2.0]\nhi = [2.0, 3.0, 2.0]\namount = 0.6\nmist = 0.5\nphase = 0.1\nseed = 9\n",
    );
    let haze = scene.haze.unwrap();
    assert_eq!(haze.lo, [-2.0, 0.0, -2.0]);
    assert!(close(haze.fog, 0.1) && close(haze.smoke, 12.0) && close(haze.floor, 1.2));
    assert!(close(haze.mist, 0.5) && close(haze.phase, 0.1));
    assert_eq!(haze.seed, 9);
    let clear = opened(
        "haze-clear",
        "[haze]\nlo = [0.0, 0.0, 0.0]\nhi = [1.0, 1.0, 1.0]\n",
    )
    .haze
    .unwrap();
    assert_eq!(
        (clear.fog, clear.smoke, clear.mist, clear.floor),
        (0.0, 0.0, 0.0, 0.0)
    );
    assert!(close(clear.back, 0.5) && close(clear.reach, 3.0));
    assert_eq!(clear.ambient, [0.03; 3]);
    let folder = Folder::new("haze-diff");
    let before = folder.open().unwrap();
    let mut text = folder.read("room.scene.toml");
    text.push_str("\n[haze]\nlo = [-3.0, 0.0, -3.0]\nhi = [3.0, 3.0, 3.0]\namount = 0.3\n");
    folder.write("room.scene.toml", &text);
    let diff = before.diff(&folder.open().unwrap());
    assert!(diff.haze && !diff.draws());
}

#[test]
fn a_mix_sky_is_the_weighted_sum_of_its_layers() {
    let room = "kind = \"room\"\n[sky.layer.room]\nwidth = 32\nfloor = [0.1, 0.1, 0.1]\nwall = [0.2, 0.3, 0.4]\nceiling = [0.6, 0.6, 0.6]\n";
    let alone = opened(
        "mix-room-alone",
        &format!("[sky]\n{}", room.replace("sky.layer.room", "sky.room")),
    );
    let mixed = opened(
        "mix-room-analytic",
        &format!(
            "[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.8, 0.2]\nirradiance = 3.0\n\n[sky]\nkind = \"mix\"\n\n[[sky.layer]]\nweight = 0.25\n{room}\n[[sky.layer]]\nkind = \"analytic\"\nweight = 0.5\n"
        ),
    );
    let room_sky = match alone.environment() {
        Environment::Hdr(sky) => sky,
        Environment::Analytic(_) => panic!("a room sky is a texture"),
    };
    let sky = match mixed.environment() {
        Environment::Hdr(sky) => sky,
        Environment::Analytic(_) => panic!("a mix sky is a texture"),
    };
    assert_eq!((sky.width, sky.height), (32, 16));
    let sun = mixed.sun.unwrap();
    let analytic = AnalyticSky {
        sun: sun.direction,
        sun_colour: sun.color,
        sun_intensity: sun.intensity,
        ambient: 1.0,
        turbidity: 3.0,
        ground_albedo: [0.2; 3],
    };
    for (index, texel) in sky.texels.iter().enumerate() {
        let (x, y) = (index % 32, index / 32);
        let theta = (y as f32 + 0.5) / 16.0 * std::f32::consts::PI;
        let phi = ((x as f32 + 0.5) / 32.0 - 0.5) * std::f32::consts::TAU;
        let direction = [
            theta.sin() * phi.sin(),
            theta.cos(),
            -theta.sin() * phi.cos(),
        ];
        let diffuse = analytic.diffuse(direction);
        for k in 0..3 {
            let expected = 0.25 * room_sky.texels[index][k] + 0.5 * diffuse[k];
            assert!(
                (texel[k] - expected).abs() < 1e-4,
                "texel {index}: {texel:?}"
            );
        }
    }
    assert_eq!(mixed.sky.as_ref().unwrap().kind, "mix");
    let reweighted = opened(
        "mix-reweighted",
        &format!(
            "[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.8, 0.2]\nirradiance = 3.0\n\n[sky]\nkind = \"mix\"\n\n[[sky.layer]]\nweight = 0.5\n{room}\n[[sky.layer]]\nkind = \"analytic\"\nweight = 0.5\n"
        ),
    );
    assert_ne!(mixed.sky, reweighted.sky);
}

#[test]
fn alpha_cutoff_and_camera_facing_cards_reach_the_draws() {
    let scene = opened(
        "cards",
        &format!(
            "{BLOCK}[[object]]\nname = \"card\"\nmesh = \"b\"\nat = [1.0, 2.0, 0.0]\nrotate = [0.0, 70.0, 0.0]\nscale = [2.0, 1.0, 0.5]\nface_camera = true\nalpha_cutoff = 0.25\n\n[[object]]\nname = \"plain\"\nmesh = \"b\"\n\n[camera]\nat = [4.0, 3.0, 6.0]\nlook_at = [1.0, 2.0, 0.0]\n"
        ),
    );
    assert!(scene.animated());
    let draws = scene.draws();
    let card = draws
        .items
        .iter()
        .find(|draw| draw.object == "card")
        .unwrap();
    assert!(card.face_camera && close(card.alpha_cutoff, 0.25));
    let plain = draws
        .items
        .iter()
        .find(|draw| draw.object == "plain")
        .unwrap();
    assert!(!plain.face_camera && close(plain.alpha_cutoff, ALPHA_CUTOFF));
    let camera = scene.camera_or_default();
    let models = draws.models(&scene.posed(&camera, 0.0));
    let model = models[draws
        .items
        .iter()
        .position(|draw| draw.object == "card")
        .unwrap()];
    let (forward, right, up) = camera.axes();
    let origin = transform(&model, [0.0; 3]);
    assert!(near(origin, [1.0, 2.0, 0.0], 1e-5));
    let along = |axis: [f32; 3]| sub(transform(&model, axis), origin);
    assert!(near(along([1.0, 0.0, 0.0]), right.map(|v| v * 2.0), 1e-5));
    assert!(near(along([0.0, 1.0, 0.0]), up, 1e-5));
    assert!(near(
        along([0.0, 0.0, 1.0]),
        forward.map(|v| -v * 0.5),
        1e-5
    ));
    let moved = Camera {
        at: [-3.0, 2.0, 2.0],
        ..camera
    };
    let turned = draws.models(&scene.posed(&moved, 0.0));
    assert_ne!(turned, models);
    let plain_index = draws
        .items
        .iter()
        .position(|draw| draw.object == "plain")
        .unwrap();
    assert_eq!(turned[plain_index], draws.items[plain_index].model);
    assert!(
        !Scene::open(fixtures().join("room.scene.toml"))
            .unwrap()
            .animated()
    );
}

#[test]
fn movers_turn_and_slide_their_objects_and_children_by_time() {
    let scene = opened(
        "movers",
        &format!(
            "{BLOCK}[[object]]\nname = \"lid\"\nmesh = \"b\"\nat = [1.0, 1.0, 0.0]\n\n[[object]]\nname = \"knob\"\nmesh = \"b\"\nparent = \"lid\"\nat = [1.0, 0.0, 0.0]\n\n[[object]]\nname = \"drawer\"\nmesh = \"b\"\n\n[[mover]]\nname = \"hinge\"\nobjects = [\"lid\"]\nkind = \"turn\"\npivot = [0.0, 1.0, 0.0]\naxis = [0.0, 0.0, 1.0]\ntravel = [0.0, 90.0]\nperiod = 2.0\n\n[[mover]]\nname = \"slider\"\nobjects = [\"drawer\"]\nkind = \"slide\"\naxis = [2.0, 0.0, 0.0]\ntravel = [0.0, 0.5]\nperiod = 4.0\nmotion = \"once\"\nclip = [[1.0, 0.0, 0.0, 0.3]]\n"
        ),
    );
    assert!(scene.animated());
    let camera = scene.camera_or_default();
    let place = |name: &str| scene.objects.iter().position(|o| o.name == name).unwrap();
    let rest = scene.posed(&camera, 0.0);
    for (index, object) in scene.objects.iter().enumerate() {
        assert_eq!(rest[index], object.model, "{} rests at time 0", object.name);
    }
    let open = scene.posed(&camera, 1.0);
    assert!(near(
        transform(&open[place("lid")], [0.0; 3]),
        [0.0, 2.0, 0.0],
        1e-5
    ));
    assert!(near(
        transform(&open[place("knob")], [0.0; 3]),
        [0.0, 3.0, 0.0],
        1e-5
    ));
    let half = scene.posed(&camera, 2.0);
    assert!(near(
        transform(&half[place("drawer")], [0.0; 3]),
        [0.25, 0.0, 0.0],
        1e-5
    ));
    let after = scene.posed(&camera, 9.0);
    assert!(near(
        transform(&after[place("drawer")], [0.0; 3]),
        [0.5, 0.0, 0.0],
        1e-5
    ));
    assert!(near(
        transform(&scene.posed(&camera, 2.0)[place("lid")], [0.0; 3]),
        [1.0, 1.0, 0.0],
        1e-4
    ));
    let draws = scene.draws();
    let drawer = draws
        .items
        .iter()
        .find(|draw| draw.object == "drawer")
        .unwrap();
    assert_eq!(drawer.clip[0], [1.0, 0.0, 0.0, 0.3]);
    let looped = Mover {
        motion: Motion::Loop,
        ..scene.movers["slider"].clone()
    };
    assert!(close(looped.value(5.0), 0.125));
    let folder = Folder::new("mover-diff");
    let before = folder.open().unwrap();
    let mut text = folder.read("room.scene.toml");
    text.push_str("\n[[mover]]\nname = \"spin\"\nobjects = [\"crate\"]\nkind = \"turn\"\npivot = [-0.9, 0.0, 0.0]\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 45.0]\n");
    folder.write("room.scene.toml", &text);
    let diff = before.diff(&folder.open().unwrap());
    only(&diff.movers, "added", "spin");
    assert!(diff.draws());
}

#[test]
fn text_reads_its_family_and_lit() {
    let folder = Folder::new("text-family");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/family.ttf"),
        folder.path("family.ttf"),
    )
    .unwrap();
    let path = folder.write(
        "main.scene.toml",
        "[text.sign]\ntext = \"Open\"\nfont = \"family.ttf\"\nsize = 0.2\nlit = true\n",
    );
    let scene = Scene::open(&path).unwrap();
    let sign = &scene.texts["sign"];
    assert!(sign.lit);
    assert!(!sign.family.is_empty());
}

#[test]
fn a_prefabs_text_follows_its_placement_and_fills_its_bound_tokens() {
    let folder = Folder::new("text-placed");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/family.ttf"),
        folder.path("family.ttf"),
    )
    .unwrap();
    folder.write(
        "sign.prefab.toml",
        "format = 1\n\n[text.label]\ntext = \"v{load-test-version} {load-test-unbound}\"\nfont = \"family.ttf\"\nsize = 0.1\nat = [0.5, 0.25, 0.0]\n",
    );
    let path = folder.write(
        "main.scene.toml",
        "format = 1\n\n[[object]]\nname = \"left\"\nprefab = \"sign.prefab.toml\"\nat = [-2.0, 0.0, 0.0]\n\n[[object]]\nname = \"right\"\nprefab = \"sign.prefab.toml\"\nat = [3.0, 1.0, 0.0]\nrotate = [0.0, 90.0, 0.0]\n\n[text.loose]\ntext = \"v{load-test-version}\"\nfont = \"family.ttf\"\nsize = 0.1\nat = [0.5, 0.25, 0.0]\n",
    );
    bind("load-test-version", "2.0.1");
    let scene = Scene::open(&path).unwrap();
    let left = &scene.texts["left/label"];
    let right = &scene.texts["right/label"];
    let loose = &scene.texts["loose"];
    assert_eq!(left.text, "v2.0.1 {load-test-unbound}");
    assert_eq!(loose.text, "v2.0.1");
    let at = |text: &Text| {
        let model = text.model();
        [model[3][0], model[3][1], model[3][2]]
    };
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
    assert!(near(at(loose), [0.5, 0.25, 0.0]), "{:?}", at(loose));
    assert!(near(at(left), [-1.5, 0.25, 0.0]), "{:?}", at(left));
    assert!(near(at(right), [3.0, 1.25, -0.5]), "{:?}", at(right));
    assert_eq!(loose.place, IDENTITY);
}

#[test]
fn open_with_wakes_the_viewer_after_the_debounce_and_pending_says_when() {
    let folder = Folder::new("watch-wake");
    let (sender, woken) = std::sync::mpsc::channel();
    let mut watch = SceneWatch::open_with(folder.path("room.scene.toml"), move || {
        let _ = sender.send(());
    })
    .unwrap();
    watch.set_debounce(std::time::Duration::from_millis(30));
    assert!(watch.pending().is_none());
    let before = std::time::Instant::now();
    folder.edit(
        "materials.toml",
        "base = [0.15, 0.25, 0.7]",
        "base = [0.7, 0.25, 0.15]",
    );
    woken
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the watch wakes the viewer");
    let due = watch.pending().expect("a change is pending");
    assert!(due >= before + std::time::Duration::from_millis(30));
    let reload = watch.poll().expect("the change is due").unwrap();
    only(&reload.diff.materials, "changed", "blue");
    assert!(watch.pending().is_none());
}

#[test]
fn a_format_0_scene_migrates_in_memory_with_one_warning_per_file() {
    let scene = Scene::open(fixtures().join("legacy.scene.toml")).unwrap();
    assert_eq!(scene.warnings.len(), 1, "{:?}", scene.warnings);
    assert!(scene.warnings[0].file.ends_with("legacy.scene.toml"));
    assert_eq!(scene.object("crate").unwrap().id, 7);
    assert_eq!(scene.object("floor").unwrap().id, 1);
    assert_eq!(scene.bodies["crate"].restitution, 0.4);
    assert!(scene.lights.contains_key("warm"));
    assert_eq!(scene.fallback.as_deref(), Some("grey"));
    let room = Scene::open(fixtures().join("room.scene.toml")).unwrap();
    assert!(room.warnings.is_empty(), "{:?}", room.warnings);
}

#[test]
fn migrate_rewrites_format_0_on_disk_keeping_comments_and_dry_run_writes_nothing() {
    let folder = Folder::new("migrate");
    let path = folder.path("legacy.scene.toml");
    let before = folder.read("legacy.scene.toml");
    let memory = Scene::open(&path).unwrap();
    let planned = migrate(&path, true).unwrap();
    assert_eq!(folder.read("legacy.scene.toml"), before);
    assert_eq!(planned.patches.len(), 1);
    assert!(planned.patches[0].file.ends_with("legacy.scene.toml"));
    let done = migrate(&path, false).unwrap();
    assert_eq!(done, planned);
    let text = folder.read("legacy.scene.toml");
    assert_eq!(text, planned.patches[0].after);
    assert!(
        text.starts_with("format = 1\n\n# a format 0 scene"),
        "{text}"
    );
    assert!(text.contains("scale = 0.7 # small\npick = 7\n"), "{text}");
    assert!(
        text.contains("[object.body]\nrestitution = 0.4\n"),
        "{text}"
    );
    let disk = Scene::open(&path).unwrap();
    assert!(disk.warnings.is_empty(), "{:?}", disk.warnings);
    assert_eq!(disk.objects, memory.objects);
    assert_eq!(disk.bodies, memory.bodies);
    assert!(migrate(&path, false).unwrap().is_empty());
    folder.write(
        "broken.scene.toml",
        "[[object]]\nname = \"x\"\nmesh = \"none\"\n",
    );
    let broken = folder.read("broken.scene.toml");
    assert!(migrate(folder.path("broken.scene.toml"), false).is_err());
    assert_eq!(folder.read("broken.scene.toml"), broken);
}

#[test]
fn fix_writes_the_missing_ids_and_refuses_a_scene_that_does_not_check() {
    let folder = Folder::new("fix");
    let text = "format = 1\n\n# by hand\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"box\"\nmesh = \"block\"\n";
    let path = folder.write("hand.scene.toml", text);
    assert_eq!(Scene::open(&path).unwrap().warnings.len(), 2);
    let planned = fix(&path, true).unwrap();
    assert_eq!(folder.read("hand.scene.toml"), text);
    fix(&path, false).unwrap();
    let fixed = folder.read("hand.scene.toml");
    assert_eq!(fixed, planned.patches[0].after);
    let mesh = pfx_scene::Id::derive(b"hand.scene.toml\nmesh\nblock");
    let object = pfx_scene::Id::derive(b"hand.scene.toml\nobject\nbox");
    assert_eq!(
        fixed,
        format!(
            "format = 1\n\n# by hand\n[mesh.block]\nfile = \"block.gltf\"\nid = \"{mesh}\"\n\n[[object]]\nname = \"box\"\nmesh = \"block\"\nid = \"{object}\"\n"
        )
    );
    assert!(Scene::open(&path).unwrap().warnings.is_empty());
    let broken = format!("{text}colour = 1.0\n");
    folder.write("hand.scene.toml", &broken);
    assert!(fix(&path, false).is_err());
    assert_eq!(folder.read("hand.scene.toml"), broken);
    assert!(fix(folder.path("legacy.scene.toml"), false).is_err());
}

#[test]
fn an_empty_scene_has_nothing_and_matches_its_default() {
    let empty = Scene::empty();
    assert_eq!(empty, Scene::default());
    assert!(empty.objects.is_empty() && empty.meshes.is_empty() && empty.bodies.is_empty());
    assert!(empty.camera.is_none() && empty.sun.is_none() && empty.physics.is_none());
    assert!(empty.files.is_empty() && empty.warnings.is_empty());
    assert_eq!(empty.path, std::path::PathBuf::new());
}
