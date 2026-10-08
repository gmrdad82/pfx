use std::fs;
use std::path::PathBuf;

use pfx_load::scene::{Target, Value};
use pfx_play::{Options, PlaySession, SceneGame};

use super::{scene_copy, text};
use crate::editor::{Editor, Group, Play, Request};
use crate::inspect::{Change, Field};
use crate::outline::Item;

const CRATE_END: &str = "id = \"vnrkf8pd3j\"\n";

const CHARACTER: &str = "\n[object.character]\njump_speed = 5.0 # metres per second\ncoyote = 6\n";

fn copy(name: &str) -> PathBuf {
    let path = scene_copy(name);
    let scene = text(&path);
    assert!(scene.contains(CRATE_END));
    fs::write(
        &path,
        scene.replacen(CRATE_END, &format!("{CRATE_END}{CHARACTER}"), 1),
    )
    .unwrap();
    path
}

fn change(path: &[&str], value: impl Into<Value>) -> Change {
    Change {
        target: Target::Object("crate".into()),
        path: path.iter().map(|key| key.to_string()).collect(),
        value: Some(value.into()),
    }
}

fn row<'a>(editor: &'a Editor, label: &str) -> &'a crate::inspect::Row {
    editor
        .rows
        .iter()
        .find(|row| row.label == label)
        .unwrap_or_else(|| panic!("no row {label}"))
}

#[test]
fn the_inspector_shows_an_objects_character_table() {
    let path = copy("characters rows");
    let mut editor = Editor::open(&path).unwrap();
    assert_eq!(editor.scene().characters["crate"].jump_speed, 5.0);
    editor.select(Some(Item::Object("crate".into())));
    assert_eq!(
        row(&editor, "character.jump_speed").field,
        Field::Number(5.0)
    );
    assert_eq!(
        row(&editor, "character.jump_buffer").field,
        Field::Number(6.0)
    );
    assert_eq!(row(&editor, "character.jump_buffer").unit, "ticks");
    assert_eq!(row(&editor, "character.max_climb").unit, "°");
    assert!(matches!(
        &row(&editor, "character.kind").field,
        Field::Choice { value, .. } if value == "3d"
    ));
    let labels: Vec<&str> = editor.rows.iter().map(|row| row.label.as_str()).collect();
    for absent in [
        "character.plane",
        "character.wall_slide",
        "character.variable_jump",
    ] {
        assert!(!labels.contains(&absent), "{absent} on a 3d character");
    }
    editor.select(Some(Item::Object("floor".into())));
    assert!(
        editor
            .rows
            .iter()
            .all(|row| !row.label.starts_with("character."))
    );
}

#[test]
fn a_stopped_edit_writes_the_scene_with_its_text_kept() {
    let path = copy("characters stopped");
    let before = text(&path);
    let mut editor = Editor::open(&path).unwrap();
    editor.select(Some(Item::Object("crate".into())));
    editor.change(change(&["character", "jump_speed"], 3.5f32), Group::Keep);
    let written = text(&path);
    assert!(
        written.contains("jump_speed = 3.5 # metres per second"),
        "{written}"
    );
    assert!(written.contains("# the crate sits by the wall"));
    assert_eq!(editor.scene().characters["crate"].jump_speed, 3.5);
    assert_eq!(
        row(&editor, "character.jump_speed").field,
        Field::Number(3.5)
    );
    editor.change(change(&["character", "radius"], -1.0f32), Group::Keep);
    assert_eq!(text(&path), written, "a bad value writes nothing");
    editor.undo();
    assert_eq!(text(&path), before);
}

#[test]
fn a_play_edit_reaches_the_world_and_keep_writes_it_to_the_scene() {
    let path = copy("characters play");
    let before = text(&path);
    let mut editor = Editor::open(&path).unwrap();
    editor.select(Some(Item::Object("crate".into())));
    let mut session = PlaySession::play_with(
        editor.scene(),
        None,
        Box::new(SceneGame),
        Options::default(),
    )
    .unwrap();
    editor.play = Play::Playing;
    editor.change(change(&["character", "snap"], 0.1f32), Group::Keep);
    for request in editor.take_requests() {
        let Request::World(change) = request else {
            panic!("only world edits while playing: {request:?}");
        };
        assert!(editor.play_edit(&mut session, &change));
    }
    let world = session.world();
    let crate_id = world.object("crate").unwrap();
    assert_eq!(world.character_desc(crate_id).unwrap().snap, 0.1);
    let snap = row(&editor, "character.snap");
    assert!(snap.pending);
    assert_eq!(snap.field, Field::Number(0.1));
    assert_eq!(text(&path), before, "play writes nothing");
    let stopped = session.stop();
    editor.stopped(Some(&stopped.pending));
    let written = text(&path);
    assert!(written.contains("snap = 0.1\n"), "{written}");
    assert!(
        written.contains("jump_speed = 5.0 # metres per second"),
        "{written}"
    );
    assert_eq!(editor.history.undo_label(), Some("keep 1 play edit"));
    editor.undo();
    assert_eq!(text(&path), before);
}
