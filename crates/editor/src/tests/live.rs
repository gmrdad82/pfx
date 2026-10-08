use std::fs;
use std::path::{Path, PathBuf};

use pfx_load::scene::{Target, Value};
use pfx_play::{Options, PlaySession, SceneGame};
use pfx_sound::Clip;

use super::{Screen, library, lights, scene_copy, text};
use crate::editor::{Editor, Group, Play, Request};
use crate::inspect::{Change, Field, TUNABLES};
use crate::outline::Item;

const PROJECT: &str = r#"format = 1

[project]
name = "room"

[tunables.jump_height]
type = "float"
default = 1.2   # metres
min = 0.0
max = 4.0
group = "movement"

[tunables.lives]
type = "int"
default = 3
min = 1
max = 9
group = "rules"
"#;

fn live_copy(name: &str) -> PathBuf {
    let path = scene_copy(name);
    let mut scene = text(&path);
    let crate_end = "id = \"vnrkf8pd3j\"\n";
    assert!(scene.contains(crate_end));
    scene = scene.replacen(
        crate_end,
        &format!("{crate_end}\n[object.body]\nfriction = 0.5\n"),
        1,
    );
    scene.push_str("\n[sound.hum]\nfile = \"tone.wav\"\nloop = true\nvolume = 0.5\n");
    fs::write(&path, scene).unwrap();
    let tone = Clip {
        rate: 48_000,
        channels: 1,
        samples: (0..2_400)
            .map(|i| ((i as f32) * 0.05).sin() * 0.5)
            .collect(),
    };
    fs::write(path.with_file_name("tone.wav"), tone.to_wav()).unwrap();
    fs::write(project(&path), PROJECT).unwrap();
    path
}

fn project(scene: &Path) -> PathBuf {
    scene.with_file_name("project.toml")
}

fn change(target: Target, path: &[&str], value: impl Into<Value>) -> Change {
    Change {
        target,
        path: path.iter().map(|key| key.to_string()).collect(),
        value: Some(value.into()),
    }
}

fn play(editor: &mut Editor) -> PlaySession {
    let session = PlaySession::play_with(
        editor.scene(),
        None,
        Box::new(SceneGame),
        Options::default(),
    )
    .unwrap();
    editor.play = Play::Playing;
    session
}

fn send(editor: &mut Editor, session: &mut PlaySession, change: Change) {
    editor.change(change, Group::Keep);
    for request in editor.take_requests() {
        let Request::World(change) = request else {
            panic!("only world edits while playing: {request:?}");
        };
        assert!(editor.play_edit(session, &change), "{}", change.describe());
    }
}

fn snapshot(scene: &Path) -> Vec<String> {
    [
        scene.to_path_buf(),
        library(scene),
        lights(scene),
        project(scene),
    ]
    .iter()
    .map(|file| text(file))
    .collect()
}

fn edits(editor: &mut Editor, session: &mut PlaySession) {
    let crate_object = Target::Object("crate".into());
    send(
        editor,
        session,
        change(Target::Material("clay".into()), &["roughness"], 0.25f32),
    );
    send(
        editor,
        session,
        change(Target::Light("warm".into()), &["intensity"], 2.5f32),
    );
    send(
        editor,
        session,
        change(crate_object.clone(), &["body", "friction"], 0.1f32),
    );
    send(
        editor,
        session,
        change(crate_object, &["body", "mass"], 3.0f32),
    );
    send(
        editor,
        session,
        change(Target::Sound("hum".into()), &["volume"], 0.2f32),
    );
    send(
        editor,
        session,
        change(Target::Scene, &[TUNABLES, "jump_height"], 2.5f32),
    );
    send(
        editor,
        session,
        change(Target::Scene, &[TUNABLES, "lives"], 5.0f32),
    );
}

#[test]
fn the_outline_and_inspector_show_bodies_sounds_and_tunables_by_group() {
    let path = live_copy("live outline");
    let editor = Editor::open(&path).unwrap();
    let titles: Vec<&str> = editor
        .outline
        .sections
        .iter()
        .map(|section| section.title)
        .collect();
    assert_eq!(
        titles,
        [
            "objects",
            "meshes",
            "lights",
            "sounds",
            "materials",
            "world",
            "tunables",
            "files"
        ]
    );
    assert!(editor.outline.contains(&Item::Tunables("movement".into())));
    assert!(editor.outline.contains(&Item::Tunables("rules".into())));
    assert!(editor.files().contains(&project(&path)));
    let mut editor = editor;
    editor.select(Some(Item::Object("crate".into())));
    let labels: Vec<&str> = editor.rows.iter().map(|row| row.label.as_str()).collect();
    for wanted in [
        "body.mass",
        "body.friction",
        "body.restitution",
        "body.damping",
        "body.velocity",
        "body.half",
    ] {
        assert!(labels.contains(&wanted), "{wanted} in {labels:?}");
    }
    let damping = editor
        .rows
        .iter()
        .find(|row| row.label == "body.damping")
        .unwrap();
    assert!(matches!(damping.field, Field::Pair(_)));
    editor.select(Some(Item::Sound("hum".into())));
    let labels: Vec<&str> = editor.rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, ["volume", "pan", "loop"]);
    editor.select(Some(Item::Tunables("movement".into())));
    assert_eq!(editor.rows.len(), 1);
    assert_eq!(editor.rows[0].label, "jump_height");
    assert_eq!(editor.rows[0].field, Field::Number(1.2));
    assert_eq!(editor.rows[0].range.unwrap().max, 4.0);
    editor.select(Some(Item::Camera));
    let exposure = editor
        .rows
        .iter()
        .find(|row| row.label == "exposure")
        .unwrap();
    assert_eq!(exposure.target, Target::Finish);
    assert_eq!(exposure.field, Field::Number(1.0));
}

#[test]
fn every_kind_of_play_edit_reaches_the_world_marks_its_row_and_writes_nothing() {
    let path = live_copy("live play");
    let before = snapshot(&path);
    let mut editor = Editor::open(&path).unwrap();
    let mut session = play(&mut editor);
    editor.select(Some(Item::Material("clay".into())));
    edits(&mut editor, &mut session);
    let world = session.world();
    assert_eq!(world.material("clay").unwrap().roughness, 0.25);
    assert_eq!(world.light("warm").unwrap().intensity, 2.5);
    let crate_id = world.object("crate").unwrap();
    assert_eq!(world.body_desc(crate_id).unwrap().friction, 0.1);
    let body = world.body(crate_id).unwrap();
    assert!((world.physics.mass(body).unwrap() - 3.0).abs() < 1e-3);
    assert_eq!(world.sound_level("hum").unwrap().volume, 0.2);
    assert_eq!(world.tunable::<f32>("jump_height"), Some(2.5));
    assert_eq!(world.tunable::<i64>("lives"), Some(5));
    let roughness = editor
        .rows
        .iter()
        .find(|row| row.label == "roughness")
        .unwrap();
    assert!(roughness.pending);
    assert_eq!(roughness.field, Field::Number(0.25));
    assert!(
        !editor
            .rows
            .iter()
            .find(|row| row.label == "metalness")
            .unwrap()
            .pending
    );
    editor.select(Some(Item::Tunables("rules".into())));
    assert!(editor.rows[0].pending);
    assert_eq!(editor.rows[0].field, Field::Number(5.0));
    editor.change(
        change(Target::Light("warm".into()), &["intensity"], -1.0f32),
        Group::Keep,
    );
    let refused = editor.take_requests();
    let Request::World(bad) = &refused[0] else {
        panic!("a world edit");
    };
    assert!(!editor.play_edit(&mut session, bad));
    assert_eq!(snapshot(&path), before);
    assert!(editor.saves.is_empty());
    assert!(!editor.history.can_undo());
    assert_eq!(session.pending().len(), 7);
}

#[test]
fn keep_writes_every_play_edit_as_one_undo_step() {
    let path = live_copy("live keep");
    let before = snapshot(&path);
    let mut editor = Editor::open(&path).unwrap();
    let mut session = play(&mut editor);
    edits(&mut editor, &mut session);
    let stopped = session.stop();
    editor.stopped(Some(&stopped.pending));
    assert_eq!(editor.play, Play::Stopped);
    assert!(editor.live.is_empty());
    assert_eq!(editor.history.undo_len(), 1);
    assert_eq!(editor.history.undo_label(), Some("keep 7 play edits"));
    let scene = editor.scene();
    assert_eq!(scene.library.get("clay").unwrap().roughness, 0.25);
    assert_eq!(scene.lights["warm"].intensity, 2.5);
    let body = scene.bodies["crate"];
    assert_eq!(body.friction, 0.1);
    assert!((body.density * pfx_play::body_volume(body.shape) - 3.0).abs() < 1e-3);
    assert_eq!(scene.sounds["hum"].volume, 0.2);
    assert_eq!(
        editor.tunables.get("jump_height").unwrap().default,
        pfx_play::TunableValue::Float(2.5)
    );
    let written = text(&project(&path));
    assert!(written.contains("default = 2.5   # metres"), "{written}");
    assert!(written.contains("default = 5\n"), "{written}");
    assert!(text(&path).contains("# the crate sits by the wall"));
    editor.undo();
    assert_eq!(snapshot(&path), before);
    assert!(!editor.history.can_undo());
    assert_eq!(
        editor.tunables.get("lives").unwrap().default,
        pfx_play::TunableValue::Int(3)
    );
}

#[test]
fn stop_discards_the_play_edits_and_the_marks() {
    let path = live_copy("live stop");
    let before = snapshot(&path);
    let mut editor = Editor::open(&path).unwrap();
    let mut session = play(&mut editor);
    editor.select(Some(Item::Material("clay".into())));
    edits(&mut editor, &mut session);
    assert!(editor.rows.iter().any(|row| row.pending));
    let stopped = session.stop();
    assert_eq!(stopped.pending.len(), 7);
    editor.stopped(None);
    assert!(editor.live.is_empty());
    assert!(editor.rows.iter().all(|row| !row.pending));
    let roughness = editor
        .rows
        .iter()
        .find(|row| row.label == "roughness")
        .unwrap();
    assert_eq!(roughness.field, Field::Number(0.9));
    assert_eq!(snapshot(&path), before);
    assert!(!editor.history.can_undo());
}

#[test]
fn f10_and_the_keep_button_ask_to_keep_only_while_play_holds_edits() {
    let path = live_copy("live keep key");
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, "key F10\nframe");
    assert_eq!(editor.take_requests(), []);
    assert!(editor.layout.find("button:keep").is_none());
    let mut session = play(&mut editor);
    screen.drive(&mut editor, &crate::script::frames(2));
    assert!(editor.layout.find("button:keep").is_none());
    send(
        &mut editor,
        &mut session,
        change(Target::Light("warm".into()), &["intensity"], 2.5f32),
    );
    screen.drive(&mut editor, &crate::script::frames(2));
    assert!(editor.layout.find("button:keep").is_some());
    screen.drive(&mut editor, "click @button:keep\nframe");
    assert_eq!(editor.take_requests(), [Request::Keep]);
    screen.drive(&mut editor, "key F10\nframe");
    assert_eq!(editor.take_requests(), [Request::Keep]);
}

#[test]
fn stopped_a_tunable_row_writes_its_default_and_a_mass_row_writes_the_density() {
    let path = live_copy("live stopped rows");
    let before = text(&project(&path));
    let mut editor = Editor::open(&path).unwrap();
    editor.select(Some(Item::Tunables("movement".into())));
    let row = editor.rows[0].clone();
    let edit = row.change(Field::Number(3.0)).unwrap();
    editor.change(edit, Group::Keep);
    assert_eq!(
        text(&project(&path)),
        before.replace("default = 1.2   # metres", "default = 3.0   # metres")
    );
    assert_eq!(editor.rows[0].field, Field::Number(3.0));
    let clamped = editor.rows[0].change(Field::Number(9.0)).unwrap();
    editor.change(clamped, Group::Keep);
    assert_eq!(
        editor.tunables.get("jump_height").unwrap().default,
        pfx_play::TunableValue::Float(4.0)
    );
    editor.undo();
    editor.undo();
    assert_eq!(text(&project(&path)), before);
    editor.select(Some(Item::Object("crate".into())));
    let mass = editor
        .rows
        .iter()
        .find(|row| row.label == "body.mass")
        .unwrap()
        .clone();
    editor.change(mass.change(Field::Number(2.0)).unwrap(), Group::Keep);
    let body = editor.scene().bodies["crate"];
    assert!((body.density * pfx_play::body_volume(body.shape) - 2.0).abs() < 1e-4);
    assert!(text(&path).contains("density = "));
    assert!(!text(&path).contains("mass = "));
}
