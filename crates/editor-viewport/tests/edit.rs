mod common;

use std::path::{Path, PathBuf};

use glam::DVec3;
use pfx_editor_doc::{Edit, Files, History};
use pfx_editor_viewport::edit::{STARTS_FROM, commit, stale};
use pfx_editor_viewport::gizmo::Pose;
use pfx_editor_viewport::{ScenePatch, Selection};
use pfx_load::scene::{PatchGroup, SceneEdit, migrate};

fn pose(edit: &SceneEdit, name: &str) -> Pose {
    let index = Selection::named(edit.scene(), name).unwrap().index;
    Pose::of(edit.scene(), index).unwrap()
}

fn moved(path: &Path, name: &str, change: impl FnOnce(&mut Pose)) -> (SceneEdit, PatchGroup) {
    let mut edit = SceneEdit::open(path).unwrap();
    let start = pose(&edit, name);
    let mut after = start.clone();
    change(&mut after);
    let group = commit(&mut edit, &format!("move {name}"), &start, &after)
        .unwrap()
        .unwrap();
    (edit, group)
}

fn files_at(path: &Path, text: &str) -> Files {
    let mut files = Files::new();
    files.open(path, text);
    files
}

#[test]
fn the_fixtures_are_the_engines_own_migration_of_format_0() {
    let path = common::format0("edit-migrate");
    let rewrites = migrate(&path, false).unwrap();
    assert_eq!(
        rewrites
            .patches
            .iter()
            .filter(|p| p.before != p.after)
            .count(),
        2
    );
    for name in [common::SCENE, "materials.toml"] {
        let migrated = common::read(&path.with_file_name(name));
        assert!(migrated.starts_with("format = 1\n"), "{migrated}");
        assert_eq!(migrated, common::read(&common::data("stand").join(name)));
    }
}

#[test]
fn a_gizmo_commit_is_one_patch_group_on_the_objects_at_and_writes_nothing() {
    let path = common::fixture("edit-at");
    let before = common::read(&path);
    let (edit, group) = moved(&path, "crate", |pose| pose.at.x += 0.25);
    assert_eq!(group.label, "move crate");
    assert_eq!(group.patches.len(), 1);
    let patch = &group.patches[0];
    assert_eq!(patch.file, path);
    assert_eq!(patch.before, before);
    let expected = before.replace("at = [0.7, 0.2, 0.3]", "at = [0.95, 0.2, 0.3]");
    assert_eq!(patch.after, expected);
    assert_eq!(common::read(&path), before);
    assert_eq!(edit.text(&path), Some(before.as_str()));
    assert_eq!(edit.scene().object("crate").unwrap().at, [0.7, 0.2, 0.3]);
    assert!(!edit.can_undo());
}

#[test]
fn the_adapter_applies_and_undo_restores_the_file_byte_for_byte() {
    let path = common::fixture("edit-undo");
    let before = common::read(&path);
    let (_, group) = moved(&path, "crate", |pose| {
        pose.at = DVec3::new(0.5, 0.25, -0.125);
        pose.rotate.y = 30.0;
    });
    let mut files = files_at(&path, &before);
    let mut history = History::default();
    let changed = history
        .apply(&mut files, ScenePatch::new(group.clone()))
        .unwrap();
    assert_eq!(changed.label, "move crate");
    assert_eq!(changed.files, vec![path.clone()]);
    let after = files.text(&path).unwrap().to_string();
    assert_eq!(after, group.patches[0].after);
    assert!(after.contains("at = [0.5, 0.25, -0.125]\n"), "{after}");
    assert!(after.contains("rotate = [0.0, 30.0, 0.0]\n"), "{after}");
    history.undo(&mut files).unwrap().unwrap();
    assert_eq!(files.text(&path).unwrap(), before);
    history.redo(&mut files).unwrap().unwrap();
    assert_eq!(files.text(&path).unwrap(), after);
    assert_eq!(history.undo_labels(), vec!["move crate"]);
}

#[test]
fn the_inverse_of_the_inverse_is_the_edit() {
    let path = common::fixture("edit-inverse");
    let (_, group) = moved(&path, "floor", |pose| pose.scale.x = 5.0);
    let patch = ScenePatch::new(group.clone());
    let back = patch.inverse();
    assert_eq!(back.label(), "move floor");
    assert_eq!(back.files(), vec![path.clone()]);
    let mut files = files_at(&path, &group.patches[0].after);
    let mut back = back;
    back.apply(&mut files).unwrap();
    assert_eq!(files.text(&path).unwrap(), group.patches[0].before);
    let mut again = back.inverse();
    again.apply(&mut files).unwrap();
    assert_eq!(files.text(&path).unwrap(), group.patches[0].after);
    assert!(group.patches[0].after.contains("scale = [5.0, 0.1, 4.0]\n"));
}

#[test]
fn a_file_that_moved_on_refuses_and_changes_nothing() {
    let path = common::fixture("edit-refused");
    let (_, group) = moved(&path, "crate", |pose| pose.at.y = 0.5);
    let other = group.patches[0]
        .before
        .replace("intensity = 4.0", "intensity = 5.0");
    let mut files = files_at(&path, &other);
    let mut history = History::default();
    let generation = files.generation();
    let error = history
        .apply(&mut files, ScenePatch::new(group))
        .unwrap_err();
    assert!(error.to_string().ends_with(STARTS_FROM), "{error}");
    assert_eq!(files.text(&path).unwrap(), other);
    assert_eq!(files.generation(), generation);
    assert_eq!(history.undo_len(), 0);
}

#[test]
fn a_closed_file_opens_at_the_text_the_patch_starts_from() {
    let path = common::fixture("edit-closed");
    let (_, group) = moved(&path, "ball", |pose| pose.at.y = 1.0);
    let mut files = Files::new();
    let mut history = History::default();
    history
        .apply(&mut files, ScenePatch::new(group.clone()))
        .unwrap();
    assert_eq!(files.text(&path).unwrap(), group.patches[0].after);
    history.undo(&mut files).unwrap();
    assert_eq!(files.text(&path).unwrap(), group.patches[0].before);
}

#[test]
fn a_uniform_scale_keeps_its_single_number_and_comments_stay() {
    let path = common::fixture("edit-scale");
    let text = common::read(&path).replace("scale = 0.4\n", "scale = 0.4 # by hand\n");
    std::fs::write(&path, &text).unwrap();
    let (_, group) = moved(&path, "crate", |pose| pose.scale = DVec3::splat(0.6));
    assert_eq!(
        group.patches[0].after,
        text.replace("scale = 0.4 # by hand", "scale = 0.6 # by hand")
    );
    std::fs::write(&path, &group.patches[0].after).unwrap();
    let (_, group) = moved(&path, "crate", |pose| pose.scale.x = 0.8);
    assert!(
        group.patches[0].after.contains("scale = [0.8, 0.6, 0.6]"),
        "{}",
        group.patches[0].after
    );
}

#[test]
fn nothing_moved_is_no_patch() {
    let path = common::fixture("edit-none");
    let mut edit = SceneEdit::open(&path).unwrap();
    let start = pose(&edit, "crate");
    let mut nudged = start.clone();
    nudged.at.x += 1e-9;
    assert_eq!(commit(&mut edit, "nothing", &start, &nudged).unwrap(), None);
    assert!(!edit.can_undo());
}

#[test]
fn a_scene_the_app_opened_by_a_relative_path_keeps_one_entry_in_files() {
    let path = common::fixture("edit-relative");
    let relative = Path::new("../../tmp/viewport-tests/edit-relative").join(common::SCENE);
    let before = common::read(&path);
    let (edit, group) = moved(&relative, "crate", |pose| pose.at.x += 0.25);
    let mut files = files_at(&relative, &before);
    let mut history = History::default();
    let patch = ScenePatch::onto(&files, group);
    let changed = history.apply(&mut files, patch).unwrap();
    assert_eq!(changed.files, vec![relative.clone()]);
    assert_eq!(files.len(), 1);
    let expected = before.replace("at = [0.7, 0.2, 0.3]", "at = [0.95, 0.2, 0.3]");
    assert_eq!(files.text(&relative), Some(expected.as_str()));
    let typed = files_at(&relative, &before.replace("0.7", "0.8"));
    assert_eq!(stale(&edit, &typed).len(), 1);
}

#[test]
fn stale_names_the_files_the_editor_holds_otherwise() {
    let path = common::fixture("edit-stale");
    let edit = SceneEdit::open(&path).unwrap();
    let text = common::read(&path);
    assert!(stale(&edit, &files_at(&path, &text)).is_empty());
    let changed = files_at(&path, &text.replace("0.7", "0.8"));
    assert_eq!(stale(&edit, &changed), vec![PathBuf::from(&path)]);
}
