mod common;

use std::path::{Path, PathBuf};

use common::{Folder, LAMP, LIGHTS, MATERIALS, ROOM, show};
use pfx_scene::{Id, PatchGroup, SceneEdit, code};

const LAMPS: &str = "\n[[object]]\nid = \"1eft1amp00\"\nname = \"left lamp\"\nprefab = \"props/lamp.prefab.toml\"\nat = [-1.2, 0.0, 0.4]\n";

const PLATES: &str = "\n[plates]\ndir = \"plates/main\"\nproxies = \"plates/room.proxies.toml\"\n";

const PROXIES: &str = "format = 1\n\n[[proxy]]\nobject = \"floor\"\nkind = \"box\"\nat = [0.0, -0.05, 0.0]\nsize = [6.0, 0.1, 6.0]\n";

fn derived(file: &str, kind: &str, name: &str) -> Id {
    Id::derive(format!("{file}\n{kind}\n{name}").as_bytes())
}

fn unfixed(name: &str) -> Folder {
    let folder = Folder::room(name);
    let room = ROOM.replace("id = \"0bj000000c\"\n", "");
    folder.write("room.scene.toml", &format!("{room}{LAMPS}{PLATES}"));
    folder.write(
        "lights.scene.toml",
        &LIGHTS.replace("id = \"11ght00002\"\n", ""),
    );
    folder.write(
        "materials.toml",
        &MATERIALS.replace("id = \"mat0000003\"\n", ""),
    );
    folder.write(
        "props/lamp.prefab.toml",
        &LAMP.replace("id = \"1amp000002\"\n", ""),
    );
    folder.write("plates/room.proxies.toml", PROXIES);
    folder
}

fn files(folder: &Folder, group: &PatchGroup) -> Vec<String> {
    let mut files: Vec<String> = group
        .files()
        .map(|file| {
            file.strip_prefix(&folder.root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect();
    files.sort();
    files
}

fn after<'a>(folder: &Folder, group: &'a PatchGroup, file: &str) -> &'a str {
    let path = folder.path(file);
    let patch = group.patches.iter().find(|patch| patch.file() == path);
    &patch.unwrap().after
}

#[test]
fn a_fix_fills_the_missing_ids_of_every_file_the_scene_reads_as_the_migration_derives_them() {
    let folder = unfixed("fix-all");
    let project = folder.project();
    let scene = folder.path("room.scene.toml");
    let warnings = project.scene(&scene).unwrap().warnings;
    assert_eq!(warnings.len(), 5, "{}", show(&warnings));
    let group = project.fix(&scene).unwrap();
    assert_eq!(group.label(), "fix ids");
    assert_eq!(
        files(&folder, &group),
        [
            "lights.scene.toml",
            "materials.toml",
            "plates/room.proxies.toml",
            "props/lamp.prefab.toml",
            "room.scene.toml",
        ]
    );
    let expected = [
        (
            "room.scene.toml",
            "pick = 7\nid",
            derived("room.scene.toml", "object", "crate"),
        ),
        (
            "lights.scene.toml",
            "shadow = false\nid",
            derived("lights.scene.toml", "light", "cool"),
        ),
        (
            "materials.toml",
            "roughness = 0.7\nid",
            derived("materials.toml", "materials", "stone"),
        ),
        (
            "props/lamp.prefab.toml",
            "scale = 0.2\nid",
            derived("props/lamp.prefab.toml", "object", "stem"),
        ),
        (
            "plates/room.proxies.toml",
            "size = [6.0, 0.1, 6.0]\nid",
            derived("plates/room.proxies.toml", "proxy", "floor\n0"),
        ),
    ];
    for (file, before, id) in expected {
        let text = after(&folder, &group, file);
        assert!(text.contains(&format!("{before} = \"{id}\"\n")), "{text}");
        assert_eq!(
            text.matches("id = ").count(),
            folder.read(file).matches("id = ").count() + 1
        );
    }
    let fixed = project.scene_with(&scene, group.after()).unwrap();
    assert!(fixed.warnings.is_empty(), "{}", show(&fixed.warnings));
    assert_eq!(
        fixed.object("crate").unwrap().id.as_deref(),
        Some(
            derived("room.scene.toml", "object", "crate")
                .to_string()
                .as_str()
        )
    );
    assert!(folder.read("room.scene.toml").contains("pick = 7\n\n"));
    group.write().unwrap();
    for patch in &group.patches {
        assert_eq!(std::fs::read_to_string(patch.file()).unwrap(), patch.after);
    }
    let project = folder.project();
    assert!(project.check().is_empty(), "{}", show(&project.check()));
    assert!(project.fix(&scene).unwrap().is_empty());
    assert!(group.write().is_err());
    group.inverse().write().unwrap();
    assert!(folder.read("room.scene.toml").contains("pick = 7\n\n"));
}

#[test]
fn a_fix_of_a_prefab_fills_the_prefab_and_its_library() {
    let folder = unfixed("fix-prefab");
    let group = folder
        .project()
        .fix(Path::new("props/lamp.prefab.toml"))
        .unwrap();
    assert_eq!(
        files(&folder, &group),
        ["materials.toml", "props/lamp.prefab.toml"]
    );
}

#[test]
fn a_derived_id_taken_elsewhere_in_the_project_is_suffixed() {
    let folder = unfixed("fix-taken");
    let taken = derived("room.scene.toml", "object", "crate");
    folder.write(
        "other.scene.toml",
        &format!("format = 1\n\n[mesh.thing]\nid = \"{taken}\"\nfile = \"block.gltf\"\n"),
    );
    let project = folder.project();
    let group = project.fix(&folder.path("room.scene.toml")).unwrap();
    let id = Id::derive(b"room.scene.toml\nobject\ncrate\n1");
    let text = after(&folder, &group, "room.scene.toml");
    assert!(
        text.contains(&format!("pick = 7\nid = \"{id}\"\n")),
        "{text}"
    );
    let mut overlay = group.after();
    overlay.insert(
        PathBuf::from("other.scene.toml"),
        folder.read("other.scene.toml"),
    );
    let diagnostics = project.check_with(overlay);
    assert!(
        diagnostics.iter().all(|d| d.code != code::DUPLICATE_ID),
        "{}",
        show(&diagnostics)
    );
}

#[test]
fn a_fix_writes_nothing_and_refuses_an_old_file_or_a_scene_that_does_not_load() {
    let folder = unfixed("fix-refused");
    let old = LIGHTS.replace("format = 1\n\n", "");
    folder.write("lights.scene.toml", &old);
    let error = folder
        .project()
        .fix(&folder.path("room.scene.toml"))
        .unwrap_err();
    assert_eq!(error.file, folder.path("lights.scene.toml"));
    assert_eq!(error.key, "fix ids");
    assert!(error.message.contains("migrate"), "{error}");
    folder.write("lights.scene.toml", LIGHTS);
    let broken = folder
        .read("room.scene.toml")
        .replace("material = \"grey\"", "material = \"nothing\"");
    folder.write("room.scene.toml", &broken);
    let error = folder
        .project()
        .fix(&folder.path("room.scene.toml"))
        .unwrap_err();
    assert_eq!(error.code, code::BAD_REFERENCE);
    assert_eq!(error.file, folder.path("room.scene.toml"));
    assert_eq!(folder.read("room.scene.toml"), broken);
}

#[test]
fn a_fix_group_lands_through_the_editor_when_it_touches_only_the_files_it_edits() {
    let folder = unfixed("fix-editor");
    folder.write(
        "room.scene.toml",
        &ROOM.replace("id = \"0bj000000c\"\n", ""),
    );
    let group = folder
        .project()
        .fix(&folder.path("room.scene.toml"))
        .unwrap();
    assert_eq!(
        files(&folder, &group),
        ["lights.scene.toml", "materials.toml", "room.scene.toml"]
    );
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    assert_eq!(edit.scene().warnings.len(), 3);
    edit.apply(&group).unwrap();
    assert!(edit.scene().warnings.is_empty());
    edit.apply(&group.inverse()).unwrap();
    assert_eq!(edit.scene().warnings.len(), 3);
}
