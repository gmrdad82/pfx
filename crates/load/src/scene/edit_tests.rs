use std::path::{Path, PathBuf};

use pfx_materials::Material;

use super::*;
use crate::scene::{SceneWatch, Shadow};

const ROOM: &str = "format = 1\n\n# the room\ninclude = [\"lights.scene.toml\"]\nmaterials = [\"materials.toml\"]\nfallback = \"grey\"\n\n[mesh.block]\nfile = \"block.gltf\"\nid = \"jvp1bak1kx\"\n\n[mesh.pillar]\nfile = \"pillar.gltf\"\nnode = \"Pillar\"\nid = \"6yp2dc01gr\"\n\n[mesh.pillar.nodes.Cap]\nmaterial = \"metal\" # a bright cap\n\n[mesh.panel]\nfile = \"panel.gltf\"\nid = \"jhq72f5by0\"\n\n# the floor\n[[object]]\nname = \"floor\"\nmesh = \"block\"\nat = [0.0, -0.05, 0.0]\nscale = [6.0, 0.1, 6.0]\nmaterial = \"grey\"\nid = \"5wgxc2j4q7\"\n\n[[object]]\nname = \"crate\"\nmesh = \"block\"\nat = [ -0.9,   0.350, 0.0 ]  # by the wall\nrotate = [0.0, 25.0, 0.0]\nscale = 0.7 # small\npick = 7\nid = \"vnrkf8pd3j\"\n\n# the pillars\n[[object]]\nname = \"left pillar\"\nmesh = \"pillar\"\nat = [0.6, 0.0, -0.6]\nid = \"nb8svewvzs\"\n\n[[object]]\nname = \"right pillar\"\nmesh = \"pillar\"\nparent = \"left pillar\"\nat = [1.4, 0.0, -0.6]\nmaterials = { Pillar = \"blue\" }\nid = \"7z24w16eak\"\n\n[[object]]\nname = \"screen\"\nmesh = \"panel\"\nat = [0.0, 1.4, -1.94]\nscale = [1.6, 0.8, 1.0]\ncontent = \"screen\"\nid = \"w61kc2vp5k\"\n\n[content.screen]\nimage = \"screen.png\"\nid = \"8aybh78pph\"\n\n[[mover]]\nname = \"spin\"\nobjects = [\"left pillar\", \"crate\"]\nkind = \"turn\"\naxis = [0.0, 1.0, 0.0]\ntravel = [0.0, 90.0]\nid = \"8w9hahxagx\"\n\n[sky]\nkind = \"room\"\n\n[sky.room]\nwidth = 64\nfloor = [0.18, 0.16, 0.14]\n\n[[sky.room.lights]]\nname = \"window\"\naz = -40.0\nel = 25.0\nwidth = 40.0\nheight = 30.0\npower = 6.0\n\n[sun]\nmodel = \"authored\"\ntoward = [-0.4, 0.8, 0.45]\ncolor = [1.0, 0.92, 0.8]\nirradiance = 2.0\n\n[camera]\nat = [0.0, 1.4, 4.2]\nlook_at = [0.0, 0.7, 0.0]\nfov = 40.0\n";

const LIGHTS: &str = "format = 1\n\n# lights\n[[light]]\nname = \"warm\"\nposition = [-1.6, 2.2, 1.4]\ncolor = [1.0, 0.82, 0.62]\nintensity = 6.0 # strong\nradius = 0.05\nrange = 8.0\nid = \"13cs3nmjxv\"\n\n[[light]]\nname = \"cool\"\nposition = [1.8, 1.6, 1.2]\nintensity = 3.0\nshadow = false\nid = \"a7pncq7x29\"\n";

const MATERIALS: &str = "format = 1\n\n# materials\n[materials.grey]\nbase = [0.5, 0.5, 0.5]\nroughness = 0.8\nid = \"jb3t50pg50\"\n\n[materials.clay]\nbase = [0.7, 0.32, 0.22]\nroughness = 0.90 # dry\nspecular = 0.02\nid = \"krxssptzas\"\n\n[materials.stone]\nbase = [0.42, 0.44, 0.46]\nroughness = 0.7\nid = \"n7798yae8m\"\n\n[materials.metal]\nbase = [0.9, 0.88, 0.82]\nroughness = 0.3\nmetalness = 1.0\nid = \"f4ypmfa8w3\"\n\n[materials.blue]\nbase = [0.15, 0.25, 0.7]\nroughness = 0.5\nid = \"4a39dm2aws\"\n\n[materials.screen]\nbase = [0.02, 0.02, 0.02]\nroughness = 0.4\nid = \"rwqfr6rjjc\"\n\n[materials.screen.content_layer]\nslot = 0\n";

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str) -> Self {
        let root = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp/scene-edit-tests")
                .join(name),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        let folder = Self { root };
        folder.write("room.scene.toml", ROOM);
        folder.write("lights.scene.toml", LIGHTS);
        folder.write("materials.toml", MATERIALS);
        folder
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.path(name), text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).unwrap()
    }

    fn edit(&self) -> SceneEdit {
        SceneEdit::open(self.path("room.scene.toml")).unwrap()
    }

    fn snapshot(&self) -> [String; 3] {
        [
            self.read("room.scene.toml"),
            self.read("lights.scene.toml"),
            self.read("materials.toml"),
        ]
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn id(name: &str) -> pfx_scene::Id {
    pfx_scene::Id::derive(name.as_bytes())
}

fn changed(before: &str, after: &str) -> Vec<(String, String)> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let mut start = 0;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let mut end = 0;
    while end < a.len() - start
        && end < b.len() - start
        && a[a.len() - 1 - end] == b[b.len() - 1 - end]
    {
        end += 1;
    }
    let old = a[start..a.len() - end].join("\n");
    let new = b[start..b.len() - end].join("\n");
    vec![(old, new)]
}

fn only(before: &str, after: &str, old: &str, new: &str) {
    assert_eq!(
        changed(before, after),
        vec![(old.to_string(), new.to_string())],
        "{after}"
    );
}

fn object<'a>(scene: &'a Scene, name: &str) -> &'a super::super::Object {
    scene.object(name).unwrap()
}

#[test]
fn an_opened_scene_is_written_back_byte_for_byte() {
    let folder = Folder::new("round");
    let edit = folder.edit();
    for (name, text) in [
        ("room.scene.toml", ROOM),
        ("lights.scene.toml", LIGHTS),
        ("materials.toml", MATERIALS),
    ] {
        assert_eq!(edit.text(folder.path(name)), Some(text));
    }
    assert_eq!(edit.files().count(), 3);
}

#[test]
fn a_file_that_cannot_be_written_back_exactly_is_not_opened() {
    let folder = Folder::new("crlf");
    folder.write("room.scene.toml", &ROOM.replace('\n', "\r\n"));
    let error = SceneEdit::open(folder.path("room.scene.toml"))
        .err()
        .expect("a CRLF file is refused");
    assert!(error.message.contains("byte for byte"), "{error}");
    assert!(error.file.ends_with("room.scene.toml"));
}

#[test]
fn setting_one_component_of_at_keeps_the_authors_spelling_and_comment() {
    let folder = Folder::new("at");
    let mut edit = folder.edit();
    let before = folder.read("room.scene.toml");
    edit.set_at("crate", [-0.9, 0.35, 1.5]).unwrap();
    let after = folder.read("room.scene.toml");
    only(
        &before,
        &after,
        "at = [ -0.9,   0.350, 0.0 ]  # by the wall",
        "at = [ -0.9,   0.350, 1.5 ]  # by the wall",
    );
    assert_eq!(object(edit.scene(), "crate").at, [-0.9, 0.35, 1.5]);
}

#[test]
fn an_edit_that_changes_nothing_writes_nothing_and_is_no_undo_step() {
    let folder = Folder::new("nothing");
    let mut edit = folder.edit();
    let before = folder.snapshot();
    let group = edit.set_at("crate", [-0.9, 0.35, 0.0]).unwrap();
    assert!(group.is_empty());
    assert_eq!(folder.snapshot(), before);
    assert!(!edit.can_undo());
}

#[test]
fn rotate_scale_and_material_keep_their_neighbours() {
    let folder = Folder::new("rotate");
    let mut edit = folder.edit();
    let before = folder.read("room.scene.toml");
    edit.set_rotate("crate", [0.0, 45.0, 0.0]).unwrap();
    let after = folder.read("room.scene.toml");
    only(
        &before,
        &after,
        "rotate = [0.0, 25.0, 0.0]",
        "rotate = [0.0, 45.0, 0.0]",
    );
    edit.set_scale("crate", [1.0, 2.0, 3.0]).unwrap();
    let scaled = folder.read("room.scene.toml");
    only(
        &after,
        &scaled,
        "scale = 0.7 # small",
        "scale = [1.0, 2.0, 3.0] # small",
    );
    edit.set_scale("crate", 2.0f32).unwrap();
    let uniform = folder.read("room.scene.toml");
    only(
        &scaled,
        &uniform,
        "scale = [1.0, 2.0, 3.0] # small",
        "scale = 2.0 # small",
    );
    edit.set_object_material("crate", Some("clay")).unwrap();
    let material = folder.read("room.scene.toml");
    assert!(
        material.contains("pick = 7\nid = \"vnrkf8pd3j\"\nmaterial = \"clay\"\n"),
        "{material}"
    );
    assert_eq!(
        object(edit.scene(), "crate").material.as_deref(),
        Some("clay")
    );
    edit.set_object_material("crate", None).unwrap();
    assert_eq!(folder.read("room.scene.toml"), uniform);
    assert_eq!(object(edit.scene(), "crate").material, None);
}

#[test]
fn per_node_materials_edit_an_inline_table_in_place() {
    let folder = Folder::new("materials-inline");
    let mut edit = folder.edit();
    let before = folder.read("room.scene.toml");
    edit.set_object_materials("right pillar", &[("Pillar", "clay")])
        .unwrap();
    let after = folder.read("room.scene.toml");
    only(
        &before,
        &after,
        "materials = { Pillar = \"blue\" }",
        "materials = { Pillar = \"clay\" }",
    );
    edit.set_object_materials("right pillar", &[]).unwrap();
    assert!(!folder.read("room.scene.toml").contains("Pillar = "));
    edit.set_object_materials("left pillar", &[("Cap", "metal")])
        .unwrap();
    assert!(
        folder.read("room.scene.toml").contains(
            "at = [0.6, 0.0, -0.6]\nid = \"nb8svewvzs\"\nmaterials = { Cap = \"metal\" }\n"
        )
    );
}

#[test]
fn shadow_two_sided_hidden_clip_and_parent_are_set_and_unset() {
    let folder = Folder::new("flags");
    let mut edit = folder.edit();
    let before = folder.read("room.scene.toml");
    edit.set_shadow("floor", "only").unwrap();
    edit.set_two_sided("floor", true).unwrap();
    edit.set_hidden("floor", true).unwrap();
    edit.set_clip("floor", &[[0.0, 1.0, 0.0, 2.0], [1.0, 0.0, 0.0, 3.0]])
        .unwrap();
    edit.set_parent("floor", Some("crate")).unwrap();
    let floor = object(edit.scene(), "floor");
    assert_eq!(floor.shadow, Shadow::Only);
    assert!(floor.two_sided && floor.hidden);
    assert_eq!(floor.clip.len(), 2);
    assert_eq!(floor.parent.as_deref(), Some("crate"));
    let after = folder.read("room.scene.toml");
    assert!(after.contains("material = \"grey\"\nid = \"5wgxc2j4q7\"\nshadow = \"only\"\ntwo_sided = true\nhidden = true\nclip = [[0.0, 1.0, 0.0, 2.0], [1.0, 0.0, 0.0, 3.0]]\nparent = \"crate\"\n"), "{after}");
    edit.set_parent("floor", None).unwrap();
    edit.set_clip("floor", &[]).unwrap();
    edit.set_hidden("floor", false).unwrap();
    edit.set_two_sided("floor", false).unwrap();
    edit.set_shadow("floor", "cast").unwrap();
    for _ in 0..5 {
        edit.undo().unwrap();
    }
    assert!(
        folder
            .read("room.scene.toml")
            .contains("parent = \"crate\"")
    );
    while edit.can_undo() {
        edit.undo().unwrap();
    }
    assert_eq!(folder.read("room.scene.toml"), before);
}

#[test]
fn refused_edits_write_nothing_and_name_the_key_and_file() {
    let folder = Folder::new("refused");
    let mut edit = folder.edit();
    let before = folder.snapshot();
    let scene = edit.scene().clone();
    let cases: Vec<(Result<PatchGroup, EditError>, &str, &str)> = vec![
        (
            edit.set_object_material("crate", Some("nothing")),
            "object crate material",
            "names no material",
        ),
        (
            edit.set_shadow("crate", "dark"),
            "object crate shadow",
            "dark",
        ),
        (
            edit.set_at("crate", [f32::NAN, 0.0, 0.0]),
            "object crate at",
            "not finite",
        ),
        (
            edit.set_clip(
                "crate",
                &[[0.0; 4], [1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
            ),
            "object crate clip",
            "at most two",
        ),
        (
            edit.set_parent("left pillar", Some("right pillar")),
            "object left pillar parent",
            "cycle",
        ),
        (
            edit.set(&Target::Object("crate".into()), &["colour"], 1.0f32),
            "object crate colour",
            "colour",
        ),
        (
            edit.set(&Target::Object("crate".into()), &["at"], "up"),
            "object crate at",
            "",
        ),
        (
            edit.set_at("ghost", [0.0; 3]),
            "object ghost at",
            "no object ghost",
        ),
        (
            edit.set_material("clay", &["roughness"], "rough"),
            "material clay roughness",
            "",
        ),
        (
            edit.set_material("clay", &["gloss"], 1.0f32),
            "material clay gloss",
            "gloss",
        ),
        (
            edit.set(&Target::Camera, &["fov"], "wide"),
            "camera fov",
            "",
        ),
        (
            edit.add(
                Kind::Object,
                id("crate"),
                "crate",
                &[("mesh", "block".into())],
                None,
            ),
            "object crate",
            "crate",
        ),
        (
            edit.add(
                Kind::Object,
                id("ghost"),
                "ghost",
                &[("mesh", "nothing".into())],
                None,
            ),
            "object ghost",
            "nothing",
        ),
        (
            edit.remove(&Target::Object("left pillar".into())),
            "object left pillar",
            "left pillar",
        ),
    ];
    for (result, key, message) in cases {
        let error = result.expect_err(key);
        assert_eq!(error.key, key, "{error}");
        assert!(error.message.contains(message), "{error}");
        assert!(
            error.file.extension().is_some_and(|ext| ext == "toml"),
            "{error}"
        );
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(folder.snapshot(), before);
    assert_eq!(edit.scene(), &scene);
    assert!(!edit.can_undo());
}

#[test]
fn a_refused_edit_in_a_library_names_the_library() {
    let folder = Folder::new("refused-library");
    let mut edit = folder.edit();
    let error = edit
        .set_material("clay", &["roughness"], "rough")
        .expect_err("a word is no roughness");
    assert_eq!(error.file, folder.path("materials.toml"));
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn an_object_is_added_after_the_others_and_removed_again() {
    let folder = Folder::new("add-object");
    let mut edit = folder.edit();
    let before = folder.read("room.scene.toml");
    edit.add(
        Kind::Object,
        id("ball"),
        "ball",
        &[
            ("mesh", "block".into()),
            ("at", [1.0, 2.0, 3.0].into()),
            ("scale", 0.5f32.into()),
        ],
        None,
    )
    .unwrap();
    let after = folder.read("room.scene.toml");
    assert!(
        after.contains(&format!("\n\n[[object]]\nid = \"{}\"\nname = \"ball\"\nmesh = \"block\"\nat = [1.0, 2.0, 3.0]\nscale = 0.5\n\n[content.screen]", id("ball"))),
        "{after}"
    );
    assert_eq!(
        edit.scene()
            .objects
            .last()
            .map(|object| object.name.as_str()),
        Some("ball")
    );
    edit.remove(&Target::Object("ball".into())).unwrap();
    assert_eq!(folder.read("room.scene.toml"), before);
    assert!(edit.scene().object("ball").is_none());
}

#[test]
fn an_object_is_duplicated_beside_its_original() {
    let folder = Folder::new("duplicate");
    let mut edit = folder.edit();
    edit.duplicate_object("crate", "crate two", id("crate two"))
        .unwrap();
    let after = folder.read("room.scene.toml");
    assert!(
        after.contains(&format!("pick = 7\nid = \"vnrkf8pd3j\"\n\n[[object]]\nname = \"crate two\"\nmesh = \"block\"\nat = [ -0.9,   0.350, 0.0 ]  # by the wall\nrotate = [0.0, 25.0, 0.0]\nscale = 0.7 # small\nid = \"{}\"\n\n# the pillars\n[[object]]\nname = \"left pillar\"", id("crate two"))),
        "{after}"
    );
    let names: Vec<&str> = edit
        .scene()
        .objects
        .iter()
        .map(|object| object.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "floor",
            "crate",
            "crate two",
            "left pillar",
            "right pillar",
            "screen"
        ]
    );
    let copy = object(edit.scene(), "crate two");
    assert_eq!(copy.at, object(edit.scene(), "crate").at);
    assert_eq!(copy.id, 3);
    edit.undo().unwrap();
    assert_eq!(folder.read("room.scene.toml"), ROOM);
}

#[test]
fn an_object_with_sub_tables_is_duplicated_with_them() {
    let folder = Folder::new("duplicate-sub");
    let screen = "[[object]]\nname = \"screen\"\nmesh = \"panel\"\nat = [0.0, 1.4, -1.94]\nscale = [1.6, 0.8, 1.0]\ncontent = \"screen\"\n";
    assert!(ROOM.contains(screen));
    let text = ROOM.replace(
        screen,
        "[[object]]\nname = \"screen\"\nmesh = \"panel\"\ncontent = \"screen\"\n\n[object.materials]\nPanel = \"clay\"\n\n[[object]]\nname = \"after\"\nmesh = \"block\"\n",
    );
    folder.write("room.scene.toml", &text);
    let mut edit = folder.edit();
    edit.duplicate_object("screen", "screen two", id("screen two"))
        .unwrap();
    let after = folder.read("room.scene.toml");
    assert_eq!(after.matches("[object.materials]").count(), 2, "{after}");
    let copy = after.find("name = \"screen two\"").unwrap();
    let second = after[copy..].find("[object.materials]").unwrap();
    let next = after[copy..].find("name = \"after\"").unwrap();
    assert!(second < next, "{after}");
    let one = &edit.scene().objects;
    let screen = one.iter().find(|o| o.name == "screen").unwrap();
    let two = one.iter().find(|o| o.name == "screen two").unwrap();
    assert_eq!(screen.materials, two.materials);
    assert_eq!(two.materials["Panel"], "clay");
    edit.undo().unwrap();
    assert_eq!(folder.read("room.scene.toml"), text);
}

#[test]
fn renaming_an_object_updates_its_children_and_movers() {
    let folder = Folder::new("rename");
    let mut edit = folder.edit();
    edit.rename_object("left pillar", "west pillar").unwrap();
    let after = folder.read("room.scene.toml");
    assert!(after.contains("name = \"west pillar\""));
    assert!(after.contains("parent = \"west pillar\""));
    assert!(after.contains("objects = [\"west pillar\", \"crate\"]"));
    assert!(!after.contains("left pillar"));
    assert_eq!(
        object(edit.scene(), "right pillar").parent.as_deref(),
        Some("west pillar")
    );
    assert_eq!(edit.scene().movers["spin"].objects[0], "west pillar");
    let error = edit.rename_object("west pillar", "crate").unwrap_err();
    assert!(error.message.contains("crate"), "{error}");
    let error = edit.rename_object("nobody", "somebody").unwrap_err();
    assert!(error.message.contains("nobody"), "{error}");
    assert_eq!(folder.read("room.scene.toml"), after);
}

#[test]
fn a_material_edit_lands_in_its_library_and_leaves_the_scene_file_alone() {
    let folder = Folder::new("library");
    let mut edit = folder.edit();
    let scene_before = folder.read("room.scene.toml");
    edit.set_material("clay", &["roughness"], 0.5f32).unwrap();
    assert_eq!(folder.read("room.scene.toml"), scene_before);
    only(
        MATERIALS,
        &folder.read("materials.toml"),
        "roughness = 0.90 # dry",
        "roughness = 0.5 # dry",
    );
    assert!((edit.scene().library.get("clay").unwrap().roughness - 0.5).abs() < 1e-6);
    edit.set_material("clay", &["clearcoat"], 0.4f32).unwrap();
    edit.set_material("clay", &["transmission"], 0.25f32)
        .unwrap();
    edit.set_material("clay", &["base"], [0.1, 0.2, 0.3])
        .unwrap();
    let clay = edit.scene().library.get("clay").unwrap();
    assert!((clay.clearcoat - 0.4).abs() < 1e-6);
    assert_eq!(clay.base[..3], [0.1, 0.2, 0.3]);
    edit.undo().unwrap();
    edit.undo().unwrap();
    edit.undo().unwrap();
    edit.undo().unwrap();
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn material_layers_and_params_are_edited_as_tables_and_lists() {
    let folder = Folder::new("layers");
    let mut edit = folder.edit();
    let layer = |kind: &str, frequency: f32| {
        Value::Table(vec![
            ("kind".to_string(), kind.into()),
            ("frequency".to_string(), frequency.into()),
            ("amplitude".to_string(), 0.3f32.into()),
            ("seed".to_string(), 1.into()),
        ])
    };
    edit.push(
        &Target::Material("clay".into()),
        &["layers"],
        layer("fbm", 8.0),
    )
    .unwrap();
    let text = folder.read("materials.toml");
    assert!(
        text.contains("specular = 0.02\nid = \"krxssptzas\"\n\n[[materials.clay.layers]]\nkind = \"fbm\"\nfrequency = 8.0\namplitude = 0.3\nseed = 1\n"),
        "{text}"
    );
    edit.push(
        &Target::Material("clay".into()),
        &["layers"],
        layer("value", 2.0),
    )
    .unwrap();
    let used = |edit: &SceneEdit| {
        edit.scene()
            .library
            .get("clay")
            .unwrap()
            .layers
            .iter()
            .filter(|layer| layer.amplitude > 0.0)
            .count()
    };
    assert_eq!(used(&edit), 2);
    edit.set_material("clay", &["layers", "1", "frequency"], 4.0f32)
        .unwrap();
    edit.set_material("clay", &["layers", "0", "amplitude"], 0.6f32)
        .unwrap();
    let clay = edit.scene().library.get("clay").unwrap();
    assert!((clay.layers[1].frequency - 4.0).abs() < 1e-6);
    assert!((clay.layers[0].amplitude - 0.6).abs() < 1e-6);
    edit.set_material(
        "clay",
        &["layers", "0", "params"],
        [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
    .unwrap();
    edit.set_material("clay", &["layers", "0", "params", "1"], 9.0f32)
        .unwrap();
    let params = edit.scene().library.get("clay").unwrap().layers[0].params;
    assert_eq!(params[..3], [1.0, 9.0, 3.0]);
    edit.unset(&Target::Material("clay".into()), &["layers", "1"])
        .unwrap();
    assert_eq!(used(&edit), 1);
    while edit.can_undo() {
        edit.undo().unwrap();
    }
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn a_material_is_added_to_a_library_from_a_value() {
    let folder = Folder::new("add-material");
    let mut edit = folder.edit();
    let material = Material {
        roughness: 0.35,
        ..Material::default()
    };
    edit.add_material("felt", id("felt"), &material, None)
        .unwrap();
    let text = folder.read("materials.toml");
    assert!(text.starts_with(MATERIALS), "{text}");
    assert!(text.contains("\n[materials.felt]\n"), "{text}");
    assert!((edit.scene().library.get("felt").unwrap().roughness - 0.35).abs() < 1e-6);
    let error = edit
        .add_material("felt", id("felt two"), &material, None)
        .unwrap_err();
    assert!(error.message.contains("exists"), "{error}");
    edit.undo().unwrap();
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn a_light_edit_lands_in_the_include_that_defines_it() {
    let folder = Folder::new("include");
    let mut edit = folder.edit();
    let scene_before = folder.read("room.scene.toml");
    edit.set(&Target::Light("warm".into()), &["intensity"], 9.5f32)
        .unwrap();
    assert_eq!(folder.read("room.scene.toml"), scene_before);
    only(
        LIGHTS,
        &folder.read("lights.scene.toml"),
        "intensity = 6.0 # strong",
        "intensity = 9.5 # strong",
    );
    edit.set(&Target::Light("cool".into()), &["shadow"], true)
        .unwrap();
    edit.set(
        &Target::Light("cool".into()),
        &["position"],
        [0.0, 3.0, 0.0],
    )
    .unwrap();
    assert!(edit.scene().lights["cool"].shadow);
    assert_eq!(edit.scene().lights["cool"].position, [0.0, 3.0, 0.0]);
    edit.add(
        Kind::Light,
        id("rim"),
        "rim",
        &[
            ("position", [0.0, 1.0, 0.0].into()),
            ("intensity", 2.0f32.into()),
        ],
        Some(Path::new("lights.scene.toml")),
    )
    .unwrap();
    assert!(folder.read("lights.scene.toml").ends_with(&format!(
        "\n[[light]]\nid = \"{}\"\nname = \"rim\"\nposition = [0.0, 1.0, 0.0]\nintensity = 2.0\n",
        id("rim")
    )));
    assert_eq!(folder.read("room.scene.toml"), scene_before);
    assert!(edit.scene().lights.contains_key("rim"));
}

#[test]
fn an_emitter_is_added_edited_and_removed() {
    let folder = Folder::new("emitter");
    let mut edit = folder.edit();
    edit.add(
        Kind::Emitter,
        id("bulb"),
        "bulb",
        &[
            ("position", [0.0, 2.0, 0.0].into()),
            ("radius", 0.1f32.into()),
            ("intensity", 4.0f32.into()),
        ],
        None,
    )
    .unwrap();
    assert!(folder.read("room.scene.toml").ends_with(&format!("\n[[emitter]]\nid = \"{}\"\nname = \"bulb\"\nposition = [0.0, 2.0, 0.0]\nradius = 0.1\nintensity = 4.0\n", id("bulb"))));
    edit.set(&Target::Emitter("bulb".into()), &["radius"], 0.2f32)
        .unwrap();
    assert!((edit.scene().emitters["bulb"].radius - 0.2).abs() < 1e-6);
    edit.remove(&Target::Emitter("bulb".into())).unwrap();
    assert_eq!(folder.read("room.scene.toml"), ROOM);
}

#[test]
fn sun_sky_camera_and_finish_fields_are_set_where_they_live() {
    let folder = Folder::new("sections");
    let mut edit = folder.edit();
    let before = folder.read("room.scene.toml");
    edit.set(&Target::Sun, &["irradiance"], 3.5f32).unwrap();
    only(
        &before,
        &folder.read("room.scene.toml"),
        "irradiance = 2.0",
        "irradiance = 3.5",
    );
    edit.set(&Target::Sun, &["radius"], 0.5f32).unwrap();
    assert!((edit.scene().sun.unwrap().radius - 0.5).abs() < 1e-6);
    edit.set(&Target::Sky, &["room", "floor"], [0.2, 0.16, 0.14])
        .unwrap();
    edit.set(&Target::Sky, &["room", "lights", "0", "power"], 8.0f32)
        .unwrap();
    edit.set(&Target::Camera, &["fov"], 55.0f32).unwrap();
    edit.set(&Target::Camera, &["shift"], [0.1, 0.0]).unwrap();
    edit.set(&Target::Camera, &["preset", "orbit", "yaw"], 12.0f32)
        .unwrap();
    edit.set(&Target::Camera, &["fstop"], 2.8f32).unwrap();
    edit.set(&Target::Camera, &["focus"], 4.0f32).unwrap();
    let camera = edit.scene().camera.unwrap();
    assert!(camera.depth_of_field.is_some());
    let text = folder.read("room.scene.toml");
    assert!(
        text.contains("fov = 55.0\nshift = [0.1, 0.0]\nfstop = 2.8\nfocus = 4.0\n"),
        "{text}"
    );
    assert!(
        text.contains("\n[camera.preset.orbit]\nyaw = 12.0\n"),
        "{text}"
    );
    edit.set(&Target::Finish, &["exposure"], 1.1f32).unwrap();
    assert!(
        folder
            .read("room.scene.toml")
            .ends_with("\n[finish]\nexposure = 1.1\n")
    );
    edit.push(
        &Target::Finish,
        &["pass"],
        Value::Table(vec![("warmth".into(), 0.2f32.into())]),
    )
    .unwrap();
    assert!(edit.scene().finish.is_some());
    edit.set(&Target::Scene, &["fallback"], "clay").unwrap();
    assert_eq!(edit.scene().fallback.as_deref(), Some("clay"));
    edit.set(&Target::Haze, &["lo"], [-1.0, 0.0, -1.0])
        .unwrap_err();
    edit.set(&Target::Trace, &["transmissive_shadows"], true)
        .unwrap();
    assert!(edit.scene().trace.transmissive_shadows);
    edit.set(&Target::Trace, &["filter_glossy"], 0.15f32)
        .unwrap();
    assert_eq!(edit.scene().trace.filter_glossy, 0.15);
    edit.set(&Target::Trace, &["clamp_indirect"], -2.0f32)
        .unwrap_err();
}

#[test]
fn a_mesh_is_added_and_a_node_override_set() {
    let folder = Folder::new("mesh");
    let mut edit = folder.edit();
    edit.add_mesh(id("mesh ball"), "ball", "ball.gltf", None)
        .unwrap();
    assert!(
        folder
            .read("room.scene.toml")
            .contains(&format!("[mesh.panel]\nfile = \"panel.gltf\"\nid = \"jhq72f5by0\"\n\n[mesh.ball]\nid = \"{}\"\nfile = \"ball.gltf\"\n", id("mesh ball")))
    );
    assert!(edit.scene().meshes.contains_key("ball"));
    edit.add(
        Kind::Object,
        id("ball one"),
        "ball one",
        &[("mesh", "ball".into())],
        None,
    )
    .unwrap();
    edit.set_node("pillar", "Cap", "material", "clay").unwrap();
    assert!(
        folder
            .read("room.scene.toml")
            .contains("material = \"clay\" # a bright cap")
    );
    edit.set_node("pillar", "Cap", "hidden", true).unwrap();
    assert!(
        folder.read("room.scene.toml").contains(
            "[mesh.pillar.nodes.Cap]\nmaterial = \"clay\" # a bright cap\nhidden = true\n"
        )
    );
    let text = folder.read("room.scene.toml");
    let modified = std::fs::metadata(folder.path("room.scene.toml"))
        .unwrap()
        .modified()
        .unwrap();
    let error = edit
        .set_node("pillar", "Nothing", "hidden", true)
        .unwrap_err();
    assert_eq!(error.key, "mesh pillar node Nothing hidden");
    assert!(error.message.contains("Nothing"), "{error}");
    assert_eq!(folder.read("room.scene.toml"), text);
    assert_eq!(
        std::fs::metadata(folder.path("room.scene.toml"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    let error = edit
        .add_mesh(id("mesh bad"), "bad", "missing.gltf", None)
        .unwrap_err();
    assert!(error.message.contains("missing.gltf"), "{error}");
    let error = edit
        .add_mesh(id("mesh ball two"), "ball", "ball.gltf", None)
        .unwrap_err();
    assert!(error.message.contains("exists"), "{error}");
    edit.remove(&Target::Object("ball one".into())).unwrap();
    edit.remove(&Target::Mesh("ball".into())).unwrap();
    assert!(!edit.scene().meshes.contains_key("ball"));
}

#[test]
fn undo_and_redo_restore_exact_bytes() {
    let folder = Folder::new("undo");
    let mut edit = folder.edit();
    let start = folder.snapshot();
    let mut states = vec![start.clone()];
    edit.set_at("crate", [1.0, 1.0, 1.0]).unwrap();
    states.push(folder.snapshot());
    edit.set_material("clay", &["roughness"], 0.2f32).unwrap();
    states.push(folder.snapshot());
    edit.set(&Target::Light("warm".into()), &["range"], 12.0f32)
        .unwrap();
    states.push(folder.snapshot());
    edit.rename_object("crate", "box").unwrap();
    states.push(folder.snapshot());
    edit.duplicate_object("box", "box 2", id("box 2")).unwrap();
    states.push(folder.snapshot());
    assert_eq!(edit.undo_label(), Some("duplicate object box as box 2"));
    for expected in states.iter().rev().skip(1) {
        let undone = edit.undo().unwrap().unwrap();
        assert!(!undone.is_empty());
        assert_eq!(&folder.snapshot(), expected);
    }
    assert!(edit.undo().unwrap().is_none());
    assert_eq!(folder.snapshot(), start);
    assert!(edit.can_redo());
    for expected in states.iter().skip(1) {
        edit.redo().unwrap().unwrap();
        assert_eq!(&folder.snapshot(), expected);
    }
    assert!(edit.redo().unwrap().is_none());
    edit.undo().unwrap();
    edit.set_hidden("floor", true).unwrap();
    assert!(!edit.can_redo());
    assert_eq!(
        edit.scene(),
        &Scene::open(folder.path("room.scene.toml")).unwrap()
    );
}

#[test]
fn a_group_of_edits_is_one_undo_step() {
    let folder = Folder::new("group");
    let mut edit = folder.edit();
    let start = folder.snapshot();
    edit.begin("move the crate").unwrap();
    assert!(edit.begin("again").is_err());
    for step in 1..=20 {
        edit.set_at("crate", [step as f32 * 0.1, 0.35, 0.0])
            .unwrap();
    }
    edit.set(&Target::Light("warm".into()), &["intensity"], 1.0f32)
        .unwrap();
    assert!(edit.undo().is_err());
    assert_eq!(edit.scene().objects[1].at[0], 2.0);
    let group = edit.end().unwrap();
    assert_eq!(group.label(), "move the crate");
    assert_eq!(group.patches.len(), 2);
    let files: Vec<&Path> = group.files().collect();
    assert_eq!(files.len(), 2);
    assert_ne!(folder.snapshot(), start);
    assert_eq!(edit.undo_label(), Some("move the crate"));
    edit.undo().unwrap();
    assert_eq!(folder.snapshot(), start);
    assert!(!edit.can_undo());
    edit.redo().unwrap();
    assert_eq!(edit.scene().objects[1].at[0], 2.0);
    edit.undo().unwrap();
    edit.begin("drag").unwrap();
    edit.set_at("crate", [5.0, 0.0, 0.0]).unwrap();
    edit.set_hidden("crate", true).unwrap();
    edit.cancel().unwrap();
    assert_eq!(folder.snapshot(), start);
    assert!(!edit.can_undo());
    edit.begin("nothing").unwrap();
    edit.set_at("crate", [5.0, 0.35, 0.0]).unwrap();
    edit.set_at("crate", [-0.9, 0.35, 0.0]).unwrap();
    assert!(edit.end().is_none());
    assert_eq!(folder.snapshot(), start);
}

#[test]
fn a_patch_group_is_a_value_with_its_inverse_label_and_files() {
    let folder = Folder::new("values");
    let mut edit = folder.edit();
    let start = folder.snapshot();
    let group = edit.set_at("crate", [3.0, 0.0, 0.0]).unwrap();
    assert_eq!(group.label(), "set object crate at");
    let patch = &group.patches[0];
    assert_eq!(patch.file(), folder.path("room.scene.toml"));
    assert_eq!(patch.label(), "set object crate at");
    assert_eq!(patch.inverse().inverse(), *patch);
    assert_eq!(group.inverse().inverse(), group);
    let moved = folder.snapshot();
    edit.apply(&group.inverse()).unwrap();
    assert_eq!(folder.snapshot(), start);
    assert!(edit.apply(&group.inverse()).is_err());
    edit.apply(&group).unwrap();
    assert_eq!(folder.snapshot(), moved);
    assert!(edit.can_undo());
}

#[test]
fn a_file_changed_on_disk_is_not_overwritten() {
    let folder = Folder::new("stale");
    let mut edit = folder.edit();
    let other = format!("{ROOM}\n# by hand\n");
    folder.write("room.scene.toml", &other);
    let error = edit.set_at("crate", [3.0, 0.0, 0.0]).unwrap_err();
    assert!(error.message.contains("changed on disk"), "{error}");
    assert_eq!(folder.read("room.scene.toml"), other);
    edit.reload().unwrap();
    edit.set_at("crate", [3.0, 0.0, 0.0]).unwrap();
    assert!(folder.read("room.scene.toml").contains("# by hand"));
}

#[test]
fn a_scene_that_does_not_load_is_not_opened() {
    let folder = Folder::new("broken");
    folder.write("room.scene.toml", "[[object]]\nname = \"x\"\n");
    assert!(SceneEdit::open(folder.path("room.scene.toml")).is_err());
}

#[test]
fn the_watch_sees_an_object_edit_and_only_that_object() {
    let folder = Folder::new("watch-object");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    let mut edit = folder.edit();
    assert!(watch.check().is_none());
    edit.set_at("crate", [2.0, 0.35, 0.0]).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.changed, ["crate"]);
    assert!(reload.diff.objects.added.is_empty() && reload.diff.objects.removed.is_empty());
    assert!(reload.diff.meshes.is_empty());
    assert!(reload.diff.materials.is_empty());
    assert!(reload.diff.lights.is_empty());
    assert!(!reload.diff.order && !reload.diff.sky && !reload.diff.sun && !reload.diff.camera);
    assert_eq!(&reload.scene, edit.scene());
    edit.undo().unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.changed, ["crate"]);
    assert_eq!(watch.scene(), edit.scene());
}

#[test]
fn the_watch_sees_a_material_a_light_and_a_camera_edit_each_alone() {
    let folder = Folder::new("watch-others");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    let mut edit = folder.edit();
    edit.set_material("clay", &["roughness"], 0.4f32).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.materials.changed, ["clay"]);
    assert!(reload.diff.objects.is_empty() && reload.diff.lights.is_empty());
    assert!(reload.diff.meshes.is_empty() && !reload.diff.camera);
    edit.set(&Target::Light("cool".into()), &["intensity"], 1.0f32)
        .unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.lights.changed, ["cool"]);
    assert!(reload.diff.materials.is_empty() && reload.diff.objects.is_empty());
    edit.set(&Target::Camera, &["fov"], 60.0f32).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert!(reload.diff.camera);
    assert!(reload.diff.lights.is_empty() && reload.diff.materials.is_empty());
    assert!(reload.diff.objects.is_empty() && !reload.diff.sun && !reload.diff.sky);
    edit.set(&Target::Sun, &["irradiance"], 1.0f32).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert!(reload.diff.sun && !reload.diff.camera);
    edit.set(&Target::Sky, &["room", "width"], 32).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert!(reload.diff.sky && !reload.diff.sun);
}

#[test]
fn the_watch_sees_adds_removes_and_renames_as_those_names() {
    let folder = Folder::new("watch-structure");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    let mut edit = folder.edit();
    edit.add(
        Kind::Object,
        id("ball"),
        "ball",
        &[("mesh", "block".into())],
        None,
    )
    .unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.added, ["ball"]);
    assert!(reload.diff.objects.changed.is_empty() && reload.diff.objects.removed.is_empty());
    assert!(reload.diff.meshes.is_empty());
    edit.duplicate_object("crate", "crate two", id("crate two"))
        .unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.added, ["crate two"]);
    assert!(reload.diff.meshes.is_empty());
    edit.remove(&Target::Object("ball".into())).unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.removed, ["ball"]);
    edit.rename_object("screen", "monitor").unwrap();
    let reload = watch.check().unwrap().unwrap();
    assert_eq!(reload.diff.objects.added, ["monitor"]);
    assert_eq!(reload.diff.objects.removed, ["screen"]);
    assert!(reload.diff.contents.is_empty() && reload.diff.meshes.is_empty());
}

#[test]
fn the_watch_hears_an_edit_through_notify() {
    let folder = Folder::new("watch-notify");
    let mut watch = SceneWatch::open(folder.path("room.scene.toml")).unwrap();
    watch.set_debounce(std::time::Duration::from_millis(20));
    let mut edit = folder.edit();
    edit.set_hidden("floor", true).unwrap();
    let start = std::time::Instant::now();
    let reload = loop {
        if let Some(result) = watch.poll() {
            break result.unwrap();
        }
        assert!(start.elapsed().as_secs() < 10, "the watch heard nothing");
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(reload.diff.objects.changed, ["floor"]);
}

type Edit = fn(&mut SceneEdit) -> Result<PatchGroup, EditError>;

#[test]
fn a_dry_run_writes_nothing_and_its_group_applied_gives_the_direct_edits_bytes() {
    let edits: [(&str, Edit); 10] = [
        ("set", |edit| edit.set_at("crate", [1.0, 0.35, 0.0])),
        ("unset", |edit| edit.set_object_material("floor", None)),
        ("push", |edit| {
            edit.push(
                &Target::Material("clay".into()),
                &["layers"],
                Value::Table(vec![
                    ("kind".into(), "fbm".into()),
                    ("frequency".into(), 8.0f32.into()),
                    ("amplitude".into(), 0.3f32.into()),
                    ("seed".into(), 1.into()),
                ]),
            )
        }),
        ("remove", |edit| edit.remove(&Target::Light("cool".into()))),
        ("add", |edit| {
            edit.add(
                Kind::Object,
                id("ball"),
                "ball",
                &[("mesh", "block".into())],
                None,
            )
        }),
        ("add material", |edit| {
            edit.add_material("felt", id("felt"), &Material::default(), None)
        }),
        ("add mesh", |edit| {
            edit.add_mesh(id("mesh ball"), "ball", "ball.gltf", None)
        }),
        ("duplicate", |edit| {
            edit.duplicate_object("crate", "crate two", id("crate two"))
        }),
        ("rename", |edit| edit.rename_object("left pillar", "pillar")),
        ("node", |edit| {
            edit.set_node("pillar", "Cap", "hidden", true)
        }),
    ];
    for (name, make) in edits {
        let direct = Folder::new(&format!("dry-direct-{name}"));
        make(&mut direct.edit()).unwrap();
        let wanted = direct.snapshot();
        let folder = Folder::new(&format!("dry-run-{name}"));
        let mut edit = folder.edit();
        let before = folder.snapshot();
        let scene = edit.scene().clone();
        let group = edit.dry_run(make).unwrap();
        assert_eq!(folder.snapshot(), before, "{name}");
        assert_eq!(edit.scene(), &scene, "{name}");
        assert!(!edit.can_undo(), "{name}");
        assert!(!group.is_empty(), "{name}");
        edit.apply(&group).unwrap();
        assert_eq!(folder.snapshot(), wanted, "{name}");
        assert_eq!(
            edit.scene(),
            &Scene::open(folder.path("room.scene.toml")).unwrap()
        );
    }
}

#[test]
fn a_dry_run_gathers_several_edits_into_one_group_and_refuses_what_the_check_refuses() {
    let direct = Folder::new("dry-many-direct");
    let mut edit = direct.edit();
    edit.set_at("crate", [1.0, 0.35, 0.0]).unwrap();
    edit.set_at("crate", [2.0, 0.35, 0.0]).unwrap();
    edit.set(&Target::Light("warm".into()), &["intensity"], 2.0f32)
        .unwrap();
    let wanted = direct.snapshot();
    let folder = Folder::new("dry-many");
    let mut edit = folder.edit();
    let before = folder.snapshot();
    let group = edit
        .dry_run(|edit| {
            edit.set_at("crate", [1.0, 0.35, 0.0])?;
            edit.set_at("crate", [2.0, 0.35, 0.0])?;
            edit.set(&Target::Light("warm".into()), &["intensity"], 2.0f32)
        })
        .unwrap();
    assert_eq!(group.label(), "set object crate at");
    assert_eq!(group.patches.len(), 2);
    assert_eq!(folder.snapshot(), before);
    assert!(edit.begin("inside").is_ok());
    assert!(edit.dry_run(|edit| edit.set_at("crate", [0.0; 3])).is_err());
    edit.cancel().unwrap();
    let error = edit
        .dry_run(|edit| edit.set_shadow("crate", "dark"))
        .unwrap_err();
    assert!(error.message.contains("dark"), "{error}");
    assert_eq!(folder.snapshot(), before);
    edit.apply(&group).unwrap();
    assert_eq!(folder.snapshot(), wanted);
}
