mod common;

use std::path::Path;

use common::{Folder, LIGHTS, MATERIALS, ROOM};
use pfx_scene::types::{Material, Shadow};
use pfx_scene::{EditError, Id, Kind, PatchGroup, Project, SceneEdit, Target, Value};

fn id(bits: u64) -> Id {
    Id::from_bits(0x5eed_0000 + bits)
}

fn edit(folder: &Folder) -> SceneEdit {
    SceneEdit::open(folder.path("room.scene.toml")).unwrap()
}

fn snapshot(folder: &Folder) -> [String; 3] {
    [
        folder.read("room.scene.toml"),
        folder.read("lights.scene.toml"),
        folder.read("materials.toml"),
    ]
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

fn object<'a>(edit: &'a SceneEdit, name: &str) -> &'a pfx_scene::types::Object {
    &edit.scene().object(name).unwrap().value
}

#[test]
fn an_opened_scene_is_written_back_byte_for_byte() {
    let folder = Folder::room("edit-round");
    let edit = edit(&folder);
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
    let folder = Folder::room("edit-crlf");
    folder.write("room.scene.toml", &ROOM.replace('\n', "\r\n"));
    let error = SceneEdit::open(folder.path("room.scene.toml"))
        .err()
        .expect("a CRLF file is refused");
    assert!(error.message.contains("byte for byte"), "{error}");
    assert!(error.file.ends_with("room.scene.toml"));
}

#[test]
fn a_format_0_file_is_migrated_before_it_is_edited() {
    let folder = Folder::room("edit-old");
    folder.write("lights.scene.toml", &LIGHTS.replace("format = 1\n\n", ""));
    let error = SceneEdit::open(folder.path("room.scene.toml"))
        .err()
        .expect("a format 0 file is refused");
    assert!(error.message.contains("migrate"), "{error}");
    assert!(error.file.ends_with("lights.scene.toml"));
}

#[test]
fn setting_one_component_of_at_keeps_the_authors_spelling_and_comment() {
    let folder = Folder::room("edit-at");
    let mut edit = edit(&folder);
    let before = folder.read("room.scene.toml");
    edit.set_at("crate", [-0.9, 0.35, 1.5]).unwrap();
    let after = folder.read("room.scene.toml");
    only(
        &before,
        &after,
        "at = [ -0.9,   0.350, 0.0 ]  # by the wall",
        "at = [ -0.9,   0.350, 1.5 ]  # by the wall",
    );
    assert_eq!(object(&edit, "crate").at, [-0.9, 0.35, 1.5]);
}

#[test]
fn an_entry_is_targeted_by_its_id_as_well_as_its_name() {
    let folder = Folder::room("edit-by-id");
    let mut edit = edit(&folder);
    let before = folder.read("room.scene.toml");
    edit.set(
        &Target::Object("0bj000000c".into()),
        &["at"],
        [-0.9, 0.35, 2.0],
    )
    .unwrap();
    only(
        &before,
        &folder.read("room.scene.toml"),
        "at = [ -0.9,   0.350, 0.0 ]  # by the wall",
        "at = [ -0.9,   0.350, 2.0 ]  # by the wall",
    );
    edit.set(
        &Target::Material("mat0000002".into()),
        &["roughness"],
        0.5f32,
    )
    .unwrap();
    assert!(
        folder
            .read("materials.toml")
            .contains("roughness = 0.5 # dry")
    );
    edit.set(&Target::Light("11ght00002".into()), &["intensity"], 4.0f32)
        .unwrap();
    assert!(
        folder
            .read("lights.scene.toml")
            .contains("intensity = 4.0\nshadow = false")
    );
}

#[test]
fn an_edit_that_changes_nothing_writes_nothing_and_is_no_undo_step() {
    let folder = Folder::room("edit-nothing");
    let mut edit = edit(&folder);
    let before = snapshot(&folder);
    let group = edit.set_at("crate", [-0.9, 0.35, 0.0]).unwrap();
    assert!(group.is_empty());
    assert_eq!(snapshot(&folder), before);
    assert!(!edit.can_undo());
}

#[test]
fn rotate_scale_and_material_keep_their_neighbours() {
    let folder = Folder::room("edit-rotate");
    let mut edit = edit(&folder);
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
        material.contains("pick = 7\nmaterial = \"clay\"\n"),
        "{material}"
    );
    assert_eq!(object(&edit, "crate").material.as_deref(), Some("clay"));
    edit.set_object_material("crate", None).unwrap();
    assert_eq!(folder.read("room.scene.toml"), uniform);
    assert_eq!(object(&edit, "crate").material, None);
}

#[test]
fn per_node_materials_edit_an_inline_table_in_place() {
    let folder = Folder::room("edit-materials-inline");
    let mut edit = edit(&folder);
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
        folder
            .read("room.scene.toml")
            .contains("at = [0.6, 0.0, -0.6]\nmaterials = { Cap = \"metal\" }\n")
    );
}

#[test]
fn shadow_two_sided_hidden_clip_and_parent_are_set_and_unset() {
    let folder = Folder::room("edit-flags");
    let mut edit = edit(&folder);
    let before = folder.read("room.scene.toml");
    edit.set_shadow("floor", "only").unwrap();
    edit.set_two_sided("floor", true).unwrap();
    edit.set_hidden("floor", true).unwrap();
    edit.set_clip("floor", &[[0.0, 1.0, 0.0, 2.0], [1.0, 0.0, 0.0, 3.0]])
        .unwrap();
    edit.set_parent("floor", Some("crate")).unwrap();
    let floor = object(&edit, "floor");
    assert_eq!(floor.shadow, Some(Shadow::Only));
    assert!(floor.two_sided && floor.hidden);
    assert_eq!(floor.clip.len(), 2);
    assert_eq!(floor.parent.as_deref(), Some("crate"));
    let after = folder.read("room.scene.toml");
    assert!(after.contains("material = \"grey\"\nshadow = \"only\"\ntwo_sided = true\nhidden = true\nclip = [[0.0, 1.0, 0.0, 2.0], [1.0, 0.0, 0.0, 3.0]]\nparent = \"crate\"\n"), "{after}");
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
    let folder = Folder::room("edit-refused");
    let mut edit = edit(&folder);
    let before = snapshot(&folder);
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
                id(1),
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
                id(2),
                "ghost",
                &[("mesh", "nothing".into())],
                None,
            ),
            "object ghost",
            "nothing",
        ),
        (
            edit.add(
                Kind::Object,
                Id::parse("0bj000000c").unwrap(),
                "twin",
                &[("mesh", "block".into())],
                None,
            ),
            "object twin",
            "taken",
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
    assert_eq!(snapshot(&folder), before);
    assert_eq!(edit.scene(), &scene);
    assert!(!edit.can_undo());
}

#[test]
fn a_refused_edit_in_a_library_names_the_library() {
    let folder = Folder::room("edit-refused-library");
    let mut edit = edit(&folder);
    let error = edit
        .set_material("clay", &["roughness"], "rough")
        .expect_err("a word is no roughness");
    assert_eq!(error.file, folder.path("materials.toml"));
    assert_eq!(error.code, "bad-type");
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn an_object_is_added_after_the_others_and_removed_again() {
    let folder = Folder::room("edit-add-object");
    let mut edit = edit(&folder);
    let before = folder.read("room.scene.toml");
    edit.add(
        Kind::Object,
        Id::parse("ba110000aa").unwrap(),
        "ball",
        &[
            ("scale", 0.5f32.into()),
            ("mesh", "block".into()),
            ("at", [1.0, 2.0, 3.0].into()),
        ],
        None,
    )
    .unwrap();
    let after = folder.read("room.scene.toml");
    assert!(
        after.contains("content = \"screen\"\n\n[[object]]\nid = \"ba110000aa\"\nname = \"ball\"\nmesh = \"block\"\nat = [1.0, 2.0, 3.0]\nscale = 0.5\n\n[content.screen]"),
        "{after}"
    );
    assert_eq!(
        edit.scene().objects.last().map(|entry| entry.key.as_str()),
        Some("ball")
    );
    assert_eq!(
        edit.scene().object("ball").unwrap().id.as_deref(),
        Some("ba110000aa")
    );
    edit.remove(&Target::Object("ball".into())).unwrap();
    assert_eq!(folder.read("room.scene.toml"), before);
    assert!(edit.scene().object("ball").is_none());
}

#[test]
fn an_object_is_duplicated_beside_its_original_with_a_new_id() {
    let folder = Folder::room("edit-duplicate");
    let mut edit = edit(&folder);
    edit.duplicate_object("crate", "crate two", Id::parse("c0py00000c").unwrap())
        .unwrap();
    let after = folder.read("room.scene.toml");
    assert!(
        after.contains("pick = 7\n\n[[object]]\nid = \"c0py00000c\"\nname = \"crate two\"\nmesh = \"block\"\nat = [ -0.9,   0.350, 0.0 ]  # by the wall\nrotate = [0.0, 25.0, 0.0]\nscale = 0.7 # small\n\n# the pillars\n[[object]]\nid = \"0bj00000p1\""),
        "{after}"
    );
    let names: Vec<&str> = edit
        .scene()
        .objects
        .iter()
        .map(|entry| entry.key.as_str())
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
    let copy = object(&edit, "crate two");
    assert_eq!(copy.at, object(&edit, "crate").at);
    assert_eq!(copy.pick, None);
    edit.undo().unwrap();
    assert_eq!(folder.read("room.scene.toml"), ROOM);
}

#[test]
fn an_object_with_sub_tables_is_duplicated_with_them() {
    let folder = Folder::room("edit-duplicate-sub");
    let screen = "[[object]]\nid = \"0bj00000s1\"\nname = \"screen\"\nmesh = \"panel\"\nat = [0.0, 1.4, -1.94]\nscale = [1.6, 0.8, 1.0]\ncontent = \"screen\"\n";
    assert!(ROOM.contains(screen));
    let text = ROOM.replace(
        screen,
        "[[object]]\nid = \"0bj00000s1\"\nname = \"screen\"\nmesh = \"panel\"\ncontent = \"screen\"\n\n[object.materials]\nPanel = \"clay\"\n\n[[object]]\nid = \"0bj0000a0f\"\nname = \"after\"\nmesh = \"block\"\n",
    );
    folder.write("room.scene.toml", &text);
    let mut edit = edit(&folder);
    edit.duplicate_object("screen", "screen two", id(3))
        .unwrap();
    let after = folder.read("room.scene.toml");
    assert_eq!(after.matches("[object.materials]").count(), 2, "{after}");
    let copy = after.find("name = \"screen two\"").unwrap();
    let second = after[copy..].find("[object.materials]").unwrap();
    let next = after[copy..].find("name = \"after\"").unwrap();
    assert!(second < next, "{after}");
    let screen = object(&edit, "screen");
    let two = object(&edit, "screen two");
    assert_eq!(screen.materials, two.materials);
    assert_eq!(two.materials["Panel"], "clay");
    edit.undo().unwrap();
    assert_eq!(folder.read("room.scene.toml"), text);
}

#[test]
fn renaming_an_object_updates_its_children_and_movers() {
    let folder = Folder::room("edit-rename");
    let mut edit = edit(&folder);
    edit.rename_object("left pillar", "west pillar").unwrap();
    let after = folder.read("room.scene.toml");
    assert!(after.contains("name = \"west pillar\""));
    assert!(after.contains("parent = \"west pillar\""));
    assert!(after.contains("objects = [\"west pillar\", \"crate\"]"));
    assert!(!after.contains("left pillar"));
    assert_eq!(
        object(&edit, "right pillar").parent.as_deref(),
        Some("west pillar")
    );
    assert_eq!(
        edit.scene().mover("spin").unwrap().value.objects[0],
        "west pillar"
    );
    let error = edit.rename_object("west pillar", "crate").unwrap_err();
    assert!(error.message.contains("crate"), "{error}");
    let error = edit.rename_object("nobody", "somebody").unwrap_err();
    assert!(error.message.contains("nobody"), "{error}");
    assert_eq!(folder.read("room.scene.toml"), after);
}

#[test]
fn a_material_edit_lands_in_its_library_and_leaves_the_scene_file_alone() {
    let folder = Folder::room("edit-library");
    let mut edit = edit(&folder);
    let scene_before = folder.read("room.scene.toml");
    edit.set_material("clay", &["roughness"], 0.5f32).unwrap();
    assert_eq!(folder.read("room.scene.toml"), scene_before);
    only(
        MATERIALS,
        &folder.read("materials.toml"),
        "roughness = 0.90 # dry",
        "roughness = 0.5 # dry",
    );
    let clay = |edit: &SceneEdit| edit.scene().material("clay").unwrap().value.clone();
    assert!((clay(&edit).roughness - 0.5).abs() < 1e-6);
    edit.set_material("clay", &["clearcoat"], 0.4f32).unwrap();
    edit.set_material("clay", &["transmission"], 0.25f32)
        .unwrap();
    edit.set_material("clay", &["base"], [0.1, 0.2, 0.3])
        .unwrap();
    assert!((clay(&edit).clearcoat - 0.4).abs() < 1e-6);
    assert_eq!(clay(&edit).base, [0.1, 0.2, 0.3]);
    for _ in 0..4 {
        edit.undo().unwrap();
    }
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn material_layers_and_params_are_edited_as_tables_and_lists() {
    let folder = Folder::room("edit-layers");
    let mut edit = edit(&folder);
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
        text.contains("specular = 0.02\n\n[[materials.clay.layers]]\nkind = \"fbm\"\nfrequency = 8.0\namplitude = 0.3\nseed = 1\n"),
        "{text}"
    );
    edit.push(
        &Target::Material("clay".into()),
        &["layers"],
        layer("value", 2.0),
    )
    .unwrap();
    let clay = |edit: &SceneEdit| edit.scene().material("clay").unwrap().value.clone();
    assert_eq!(clay(&edit).layers.len(), 2);
    edit.set_material("clay", &["layers", "1", "frequency"], 4.0f32)
        .unwrap();
    edit.set_material("clay", &["layers", "0", "amplitude"], 0.6f32)
        .unwrap();
    assert!((clay(&edit).layers[1].frequency - 4.0).abs() < 1e-6);
    assert!((clay(&edit).layers[0].amplitude - 0.6).abs() < 1e-6);
    edit.set_material(
        "clay",
        &["layers", "0", "params"],
        [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    )
    .unwrap();
    edit.set_material("clay", &["layers", "0", "params", "1"], 9.0f32)
        .unwrap();
    assert_eq!(clay(&edit).layers[0].params[..3], [1.0, 9.0, 3.0]);
    edit.unset(&Target::Material("clay".into()), &["layers", "1"])
        .unwrap();
    assert_eq!(clay(&edit).layers.len(), 1);
    while edit.can_undo() {
        edit.undo().unwrap();
    }
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn a_material_is_added_to_a_library_from_a_value() {
    let folder = Folder::room("edit-add-material");
    let mut edit = edit(&folder);
    let material = Material {
        roughness: 0.35,
        ..Material::default()
    };
    edit.add_material("felt", Id::parse("fe1t000001").unwrap(), &material, None)
        .unwrap();
    let text = folder.read("materials.toml");
    assert!(text.starts_with(MATERIALS), "{text}");
    assert!(
        text.ends_with("\n[materials.felt]\nid = \"fe1t000001\"\nroughness = 0.35\n"),
        "{text}"
    );
    let felt = &edit.scene().material("felt").unwrap().value;
    assert!((felt.roughness - 0.35).abs() < 1e-6);
    let error = edit
        .add_material("felt", Id::parse("fe1t000002").unwrap(), &material, None)
        .unwrap_err();
    assert!(error.message.contains("exists"), "{error}");
    edit.undo().unwrap();
    assert_eq!(folder.read("materials.toml"), MATERIALS);
}

#[test]
fn a_light_edit_lands_in_the_include_that_defines_it() {
    let folder = Folder::room("edit-include");
    let mut edit = edit(&folder);
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
    let cool = &edit.scene().light("cool").unwrap().value;
    assert_eq!(cool.shadow, Some(true));
    assert_eq!(cool.position, [0.0, 3.0, 0.0]);
    edit.add(
        Kind::Light,
        Id::parse("11ght00003").unwrap(),
        "rim",
        &[
            ("intensity", 2.0f32.into()),
            ("position", [0.0, 1.0, 0.0].into()),
        ],
        Some(Path::new("lights.scene.toml")),
    )
    .unwrap();
    assert!(
        folder.read("lights.scene.toml").ends_with(
            "\n[[light]]\nid = \"11ght00003\"\nname = \"rim\"\nposition = [0.0, 1.0, 0.0]\nintensity = 2.0\n"
        )
    );
    assert_eq!(folder.read("room.scene.toml"), scene_before);
    assert!(edit.scene().light("rim").is_some());
}

#[test]
fn an_emitter_is_added_edited_and_removed() {
    let folder = Folder::room("edit-emitter");
    let mut edit = edit(&folder);
    edit.add(
        Kind::Emitter,
        id(4),
        "bulb",
        &[
            ("position", [0.0, 2.0, 0.0].into()),
            ("radius", 0.1f32.into()),
            ("intensity", 4.0f32.into()),
        ],
        None,
    )
    .unwrap();
    let expected = format!(
        "\n[[emitter]]\nid = \"{}\"\nname = \"bulb\"\nposition = [0.0, 2.0, 0.0]\nradius = 0.1\nintensity = 4.0\n",
        id(4)
    );
    assert!(folder.read("room.scene.toml").ends_with(&expected));
    edit.set(&Target::Emitter("bulb".into()), &["radius"], 0.2f32)
        .unwrap();
    assert!((edit.scene().emitter("bulb").unwrap().value.radius - 0.2).abs() < 1e-6);
    edit.remove(&Target::Emitter("bulb".into())).unwrap();
    assert_eq!(folder.read("room.scene.toml"), ROOM);
}

#[test]
fn sun_sky_camera_and_finish_fields_are_set_where_they_live() {
    let folder = Folder::room("edit-sections");
    let mut edit = edit(&folder);
    let before = folder.read("room.scene.toml");
    edit.set(&Target::Sun, &["irradiance"], 3.5f32).unwrap();
    only(
        &before,
        &folder.read("room.scene.toml"),
        "irradiance = 2.0",
        "irradiance = 3.5",
    );
    edit.set(&Target::Sun, &["radius"], 0.5f32).unwrap();
    assert_eq!(edit.scene().sun.as_ref().unwrap().radius, Some(0.5));
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
    assert_eq!(edit.scene().camera.as_ref().unwrap().fstop, Some(2.8));
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
    assert_eq!(edit.scene().finish.as_ref().unwrap().pass.len(), 1);
    edit.set(&Target::Scene, &["fallback"], "clay").unwrap();
    assert_eq!(edit.scene().fallback.as_deref(), Some("clay"));
    edit.set(&Target::Haze, &["lo"], [-1.0, 0.0, -1.0])
        .unwrap_err();
    edit.set(&Target::Trace, &["transmissive_shadows"], true)
        .unwrap();
    assert!(edit.scene().trace.unwrap().transmissive_shadows);
    edit.set(&Target::Trace, &["filter_glossy"], 0.25f32)
        .unwrap();
    assert_eq!(edit.scene().trace.unwrap().filter_glossy, Some(0.25));
    edit.set(&Target::Trace, &["clamp_indirect"], -1.0f32)
        .unwrap_err();
    assert_eq!(edit.scene().trace.unwrap().clamp_indirect, None);
    edit.set(&Target::Physics, &["rate"], 120.0f32).unwrap();
    assert_eq!(edit.scene().physics.as_ref().unwrap().rate, Some(120.0));
}

#[test]
fn a_mesh_is_added_and_a_node_override_set() {
    let folder = Folder::room("edit-mesh");
    let mut edit = edit(&folder);
    edit.add_mesh(Id::parse("me5h0000ba").unwrap(), "ball", "ball.gltf", None)
        .unwrap();
    assert!(
        folder
            .read("room.scene.toml")
            .contains("[mesh.panel]\nid = \"me5h0000p2\"\nfile = \"panel.gltf\"\n\n[mesh.ball]\nid = \"me5h0000ba\"\nfile = \"ball.gltf\"\n")
    );
    assert!(edit.scene().mesh("ball").is_some());
    edit.add(
        Kind::Object,
        id(5),
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
    let error = edit
        .set_node("pillar", "Cap", "shadow", "dark")
        .unwrap_err();
    assert_eq!(error.key, "mesh pillar node Cap shadow");
    let error = edit
        .add_mesh(id(6), "bad", "missing.gltf", None)
        .unwrap_err();
    assert!(error.message.contains("missing.gltf"), "{error}");
    let error = edit.add_mesh(id(7), "ball", "ball.gltf", None).unwrap_err();
    assert!(error.message.contains("exists"), "{error}");
    edit.remove(&Target::Object("ball one".into())).unwrap();
    edit.remove(&Target::Mesh("ball".into())).unwrap();
    assert!(edit.scene().mesh("ball").is_none());
}

#[test]
fn undo_and_redo_restore_exact_bytes() {
    let folder = Folder::room("edit-undo");
    let mut edit = edit(&folder);
    let start = snapshot(&folder);
    let mut states = vec![start.clone()];
    edit.set_at("crate", [1.0, 1.0, 1.0]).unwrap();
    states.push(snapshot(&folder));
    edit.set_material("clay", &["roughness"], 0.2f32).unwrap();
    states.push(snapshot(&folder));
    edit.set(&Target::Light("warm".into()), &["range"], 12.0f32)
        .unwrap();
    states.push(snapshot(&folder));
    edit.rename_object("crate", "box").unwrap();
    states.push(snapshot(&folder));
    edit.duplicate_object("box", "box 2", id(8)).unwrap();
    states.push(snapshot(&folder));
    assert_eq!(edit.undo_label(), Some("duplicate object box as box 2"));
    for expected in states.iter().rev().skip(1) {
        let undone = edit.undo().unwrap().unwrap();
        assert!(!undone.is_empty());
        assert_eq!(&snapshot(&folder), expected);
    }
    assert!(edit.undo().unwrap().is_none());
    assert_eq!(snapshot(&folder), start);
    assert!(edit.can_redo());
    for expected in states.iter().skip(1) {
        edit.redo().unwrap().unwrap();
        assert_eq!(&snapshot(&folder), expected);
    }
    assert!(edit.redo().unwrap().is_none());
    edit.undo().unwrap();
    edit.set_hidden("floor", true).unwrap();
    assert!(!edit.can_redo());
    let fresh = Project::open(&folder.root)
        .unwrap()
        .scene(&folder.path("room.scene.toml"))
        .unwrap();
    assert_eq!(edit.scene().objects, fresh.objects);
}

#[test]
fn a_group_of_edits_is_one_undo_step() {
    let folder = Folder::room("edit-group");
    let mut edit = edit(&folder);
    let start = snapshot(&folder);
    edit.begin("move the crate").unwrap();
    assert!(edit.begin("again").is_err());
    for step in 1..=20 {
        edit.set_at("crate", [step as f32 * 0.1, 0.35, 0.0])
            .unwrap();
    }
    edit.set(&Target::Light("warm".into()), &["intensity"], 1.0f32)
        .unwrap();
    assert!(edit.undo().is_err());
    assert_eq!(object(&edit, "crate").at[0], 2.0);
    let group = edit.end().unwrap();
    assert_eq!(group.label(), "move the crate");
    assert_eq!(group.patches.len(), 2);
    assert_eq!(group.files().count(), 2);
    assert_ne!(snapshot(&folder), start);
    assert_eq!(edit.undo_label(), Some("move the crate"));
    edit.undo().unwrap();
    assert_eq!(snapshot(&folder), start);
    assert!(!edit.can_undo());
    edit.redo().unwrap();
    assert_eq!(object(&edit, "crate").at[0], 2.0);
    edit.undo().unwrap();
    edit.begin("drag").unwrap();
    edit.set_at("crate", [5.0, 0.0, 0.0]).unwrap();
    edit.set_hidden("crate", true).unwrap();
    edit.cancel().unwrap();
    assert_eq!(snapshot(&folder), start);
    assert!(!edit.can_undo());
    edit.begin("nothing").unwrap();
    edit.set_at("crate", [5.0, 0.35, 0.0]).unwrap();
    edit.set_at("crate", [-0.9, 0.35, 0.0]).unwrap();
    assert!(edit.end().is_none());
    assert_eq!(snapshot(&folder), start);
}

#[test]
fn a_patch_group_is_a_value_with_its_inverse_label_and_files() {
    let folder = Folder::room("edit-values");
    let mut edit = edit(&folder);
    let start = snapshot(&folder);
    let group = edit.set_at("crate", [3.0, 0.0, 0.0]).unwrap();
    assert_eq!(group.label(), "set object crate at");
    let patch = &group.patches[0];
    assert_eq!(patch.file(), folder.path("room.scene.toml"));
    assert_eq!(patch.label(), "set object crate at");
    assert_eq!(patch.inverse().inverse(), *patch);
    assert_eq!(group.inverse().inverse(), group);
    let moved = snapshot(&folder);
    edit.apply(&group.inverse()).unwrap();
    assert_eq!(snapshot(&folder), start);
    assert!(edit.apply(&group.inverse()).is_err());
    edit.apply(&group).unwrap();
    assert_eq!(snapshot(&folder), moved);
    assert!(edit.can_undo());
}

#[test]
fn a_file_changed_on_disk_is_not_overwritten() {
    let folder = Folder::room("edit-stale");
    let mut edit = edit(&folder);
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
    let folder = Folder::room("edit-broken");
    folder.write(
        "room.scene.toml",
        "format = 1\n\n[[object]]\nname = \"x\"\n",
    );
    assert!(SceneEdit::open(folder.path("room.scene.toml")).is_err());
}

#[test]
fn the_first_write_to_a_file_adds_the_ids_its_entries_lack() {
    let folder = Folder::room("edit-fix");
    let lights = LIGHTS
        .replace("id = \"11ght00001\"\n", "")
        .replace("id = \"11ght00002\"\n", "");
    folder.write("lights.scene.toml", &lights);
    let mut edit = edit(&folder);
    let warnings: Vec<&str> = edit.scene().warnings.iter().map(|d| d.code).collect();
    assert_eq!(warnings, ["missing-id", "missing-id"]);
    edit.set_at("crate", [0.0, 0.35, 0.0]).unwrap();
    assert_eq!(folder.read("lights.scene.toml"), lights);
    edit.set(&Target::Light("cool".into()), &["intensity"], 4.0f32)
        .unwrap();
    let after = folder.read("lights.scene.toml");
    let derived =
        |name: &str| Id::derive(format!("lights.scene.toml\nlight\n{name}").as_bytes()).to_string();
    assert!(
        after.contains(&format!("range = 8.0\nid = \"{}\"\n", derived("warm"))),
        "{after}"
    );
    assert!(
        after.contains(&format!(
            "intensity = 4.0\nshadow = false\nid = \"{}\"\n",
            derived("cool")
        )),
        "{after}"
    );
    assert!(edit.scene().warnings.is_empty());
    edit.undo().unwrap();
    assert_eq!(folder.read("lights.scene.toml"), lights);
}

#[test]
fn a_dry_run_returns_the_merged_group_checked_as_a_whole_and_writes_nothing() {
    let folder = Folder::room("edit-dry");
    let mut edit = edit(&folder);
    let start = snapshot(&folder);
    let felt = Material {
        roughness: 0.35,
        ..Material::default()
    };
    let group = edit
        .dry_run(|edit| {
            edit.set_at("crate", [1.0, 0.35, 0.0])?;
            edit.set_at("crate", [2.0, 0.35, 0.0])?;
            edit.set_object_material("crate", Some("felt"))?;
            edit.add_material("felt", id(1), &felt, None)?;
            edit.set(&Target::Light("warm".into()), &["intensity"], 1.0f32)
        })
        .unwrap();
    assert_eq!(snapshot(&folder), start);
    assert!(!edit.can_undo());
    assert_eq!(object(&edit, "crate").at[0], -0.9);
    assert_eq!(edit.text(folder.path("room.scene.toml")), Some(ROOM));
    assert_eq!(group.label(), "set object crate at");
    let files: Vec<&Path> = group.files().collect();
    assert_eq!(
        files,
        [
            folder.path("room.scene.toml"),
            folder.path("materials.toml"),
            folder.path("lights.scene.toml"),
        ]
    );
    assert_eq!(group.patches[0].before, ROOM);
    assert!(
        group.patches[0]
            .after
            .contains("at = [ 2.0,   0.350, 0.0 ]  # by the wall\n")
    );
    let added = format!("\n[materials.felt]\nid = \"{}\"\nroughness = 0.35\n", id(1));
    assert!(group.patches[1].after.ends_with(&added));
    let scene = edit
        .project()
        .scene_with(&edit.path(), group.after())
        .unwrap();
    let crate_material = &scene.object("crate").unwrap().value.material;
    assert_eq!(crate_material.as_deref(), Some("felt"));
    assert_eq!(scene.light("warm").unwrap().value.intensity, 1.0);
    edit.apply(&group).unwrap();
    for patch in &group.patches {
        assert_eq!(std::fs::read_to_string(patch.file()).unwrap(), patch.after);
    }
    assert_eq!(object(&edit, "crate").at[0], 2.0);
    edit.apply(&group.inverse()).unwrap();
    assert_eq!(snapshot(&folder), start);
}

#[test]
fn a_refused_dry_run_writes_nothing_and_leaves_the_editor_as_it_was() {
    let folder = Folder::room("edit-dry-refused");
    let mut edit = edit(&folder);
    let start = snapshot(&folder);
    let error = edit
        .dry_run(|edit| {
            edit.set_at("crate", [1.0, 0.35, 0.0])?;
            edit.set_object_material("crate", Some("nothing"))
        })
        .unwrap_err();
    assert_eq!(error.key, "set object crate at");
    assert_eq!(error.file, folder.path("room.scene.toml"));
    assert!(error.message.contains("names no material"), "{error}");
    let error = edit
        .dry_run(|edit| {
            edit.set_at("crate", [1.0, 0.35, 0.0])?;
            edit.set_at("nobody", [1.0, 0.35, 0.0])
        })
        .unwrap_err();
    assert!(error.message.contains("no object nobody"), "{error}");
    let inside = edit.dry_run(|edit| {
        assert!(edit.begin("drag").is_err());
        assert!(edit.undo().is_err());
        assert!(edit.redo().is_err());
        assert!(edit.reload().is_err());
        assert!(edit.dry_run(|_| Ok(())).is_err());
        let group = edit.set_at("crate", [3.0, 0.35, 0.0])?;
        assert!(edit.apply(&group).is_err());
        Ok(())
    });
    assert!(inside.is_ok());
    assert_eq!(snapshot(&folder), start);
    assert!(!edit.can_undo());
    edit.begin("drag").unwrap();
    assert!(edit.dry_run(|_| Ok(())).is_err());
    edit.cancel().unwrap();
    assert!(edit.dry_run(|_| Ok(())).unwrap().is_empty());
    let group = edit.set_at("crate", [4.0, 0.35, 0.0]).unwrap();
    assert_eq!(group.patches[0].before, ROOM);
    assert_eq!(edit.undo_label(), Some("set object crate at"));
}

#[test]
fn ids_made_in_a_dry_run_are_taken_by_its_later_edits() {
    let folder = Folder::room("edit-dry-ids");
    let mut edit = edit(&folder);
    let ball = [("mesh", Value::from("ball"))];
    let error = edit
        .dry_run(|edit| {
            edit.add_mesh(id(1), "ball", "ball.gltf", None)?;
            edit.add(Kind::Object, id(1), "ball", &ball, None)
        })
        .unwrap_err();
    assert!(error.message.contains("is taken"), "{error}");
    let group = edit
        .dry_run(|edit| {
            edit.add(Kind::Object, id(2), "ball", &ball, None)?;
            edit.add_mesh(id(1), "ball", "ball.gltf", None)
        })
        .unwrap();
    assert_eq!(group.label(), "add object ball");
    assert_eq!(group.patches.len(), 1);
    edit.apply(&group).unwrap();
    assert!(edit.scene().object("ball").is_some());
    assert!(edit.scene().mesh("ball").is_some());
}
