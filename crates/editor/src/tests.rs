use std::fs;
use std::path::{Path, PathBuf};

use glam::DVec3;
use pfx_editor_viewport::gizmo::REACH;
use pfx_editor_viewport::{Lens, Look, Mode};
use pfx_load::scene::{Target, Value};

use crate::editor::{Editor, Group, Play, Request};
use crate::inspect::{Change, Field};
use crate::outline::Item;
use crate::script::{frames, script};

pub fn scene_copy(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../../tmp/editor-tests").join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    for entry in fs::read_dir(manifest.join("tests/scenes")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    std::path::absolute(root.join("room.scene.toml")).unwrap()
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn library(scene: &Path) -> PathBuf {
    scene.with_file_name("materials.toml")
}

fn lights(scene: &Path) -> PathBuf {
    scene.with_file_name("lights.scene.toml")
}

struct Screen {
    ctx: egui::Context,
}

const SIZE: [u32; 2] = [1280, 800];

impl Screen {
    fn new(editor: &mut Editor) -> Screen {
        let screen = Screen {
            ctx: pfx_editor_shell::context(&crate::theme::pfx()),
        };
        screen.drive(editor, &frames(2));
        screen
    }

    fn drive(&self, editor: &mut Editor, text: &str) {
        let script = script(text, &editor.layout).unwrap();
        pfx_editor_shell::drive(&self.ctx, SIZE, &script, |ctx| editor.ui(ctx));
    }
}

fn lens(editor: &Editor) -> Lens {
    Lens::new(
        &editor.viewport.eye().unwrap(),
        editor.viewport.image().unwrap(),
    )
}

fn point(editor: &Editor, at: [f32; 3]) -> egui::Pos2 {
    lens(editor)
        .project(DVec3::from_array(at.map(f64::from)))
        .unwrap()
}

fn handle(editor: &Editor, object: &str, axis: DVec3) -> egui::Pos2 {
    let lens = lens(editor);
    let model = editor.scene().object(object).unwrap().model;
    let centre = DVec3::new(
        f64::from(model[3][0]),
        f64::from(model[3][1]),
        f64::from(model[3][2]),
    );
    let reach = f64::from(REACH) * lens.metres_per_point(centre);
    lens.project(centre + axis * reach * 0.7).unwrap()
}

fn drag_x(screen: &Screen, editor: &mut Editor, by: f32) {
    let from = handle(editor, "crate", DVec3::X);
    screen.drive(
        editor,
        &format!(
            "drag {} {} {} {}\nframe",
            from.x,
            from.y,
            from.x + by,
            from.y
        ),
    );
}

fn type_into(
    screen: &Screen,
    editor: &mut Editor,
    file: &Path,
    after: &str,
    backspaces: usize,
    typed: &str,
) {
    editor.select(Some(Item::File(file.to_path_buf())));
    screen.drive(editor, &frames(1));
    let byte = text(file).find(after).unwrap() + after.len();
    editor.sources.get_mut(file).unwrap().jump(byte);
    screen.drive(
        editor,
        &format!(
            "frame\n{}text {typed}\n{}",
            "key Backspace\n".repeat(backspaces),
            frames(15)
        ),
    );
}

#[test]
fn the_outline_follows_the_scene() {
    let path = scene_copy("outline");
    let editor = Editor::open(&path).unwrap();
    let outline = &editor.outline;
    let objects = outline.section("objects").unwrap();
    let roots: Vec<&str> = objects
        .nodes
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(roots, ["floor", "wall", "crate", "pillar"]);
    let crate_node = &objects.nodes[2];
    assert_eq!(crate_node.children.len(), 1);
    assert_eq!(crate_node.children[0].item, Item::Object("ball".into()));
    let meshes = outline.section("meshes").unwrap();
    let pillar = meshes
        .nodes
        .iter()
        .find(|node| node.item == Item::Mesh("pillar".into()))
        .unwrap();
    let parts: Vec<&str> = pillar
        .children
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert!(
        parts.contains(&"Pillar") && parts.contains(&"Cap"),
        "{parts:?}"
    );
    let lights: Vec<&Item> = outline
        .section("lights")
        .unwrap()
        .nodes
        .iter()
        .map(|node| &node.item)
        .collect();
    assert_eq!(
        lights,
        [&Item::Light("warm".into()), &Item::Emitter("bulb".into())]
    );
    assert_eq!(outline.section("materials").unwrap().nodes.len(), 5);
    let files: Vec<&str> = outline
        .section("files")
        .unwrap()
        .nodes
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    for file in ["room.scene.toml", "lights.scene.toml", "materials.toml"] {
        assert!(files.contains(&file), "{files:?}");
    }
    assert_eq!(
        outline.path_to(&Item::Object("ball".into())),
        [Item::Object("crate".into()), Item::Object("ball".into())]
    );
    assert_eq!(
        outline.step(Some(&Item::Object("floor".into())), 1),
        Some(Item::Object("wall".into()))
    );
    assert_eq!(outline.step(None, -1).unwrap().kind(), "file");
}

#[test]
fn inspector_rows_are_bound_to_scene_edit_paths() {
    let path = scene_copy("rows");
    let mut editor = Editor::open(&path).unwrap();
    editor.select(Some(Item::Object("crate".into())));
    let row = |editor: &Editor, label: &str| {
        editor
            .rows
            .iter()
            .find(|row| row.label == label)
            .cloned()
            .unwrap_or_else(|| panic!("no row {label}"))
    };
    let at = row(&editor, "at");
    assert_eq!(at.target, Target::Object("crate".into()));
    assert_eq!(at.path, ["at"]);
    assert_eq!(at.field, Field::Vector([-0.9, 0.35, 0.0]));
    assert_eq!(row(&editor, "scale").field, Field::Vector([0.7; 3]));
    match row(&editor, "material").field {
        Field::Choice { value, options } => {
            assert_eq!(value, "clay");
            assert_eq!(options[0], "");
            assert!(options.contains(&"blue".to_string()));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(row(&editor, "hidden").field, Field::Toggle(false));
    assert_eq!(row(&editor, "clip").field, Field::Planes(Vec::new()));
    editor.select(Some(Item::Material("clay".into())));
    let roughness = row(&editor, "roughness");
    assert_eq!(roughness.target, Target::Material("clay".into()));
    assert_eq!(roughness.path, ["roughness"]);
    assert_eq!(roughness.field, Field::Number(0.9));
    editor.select(Some(Item::Sun));
    match row(&editor, "model").field {
        Field::Choice { value, .. } => assert_eq!(value, "daylight"),
        other => panic!("{other:?}"),
    }
    assert_eq!(row(&editor, "hour").field, Field::Number(15.0));
    assert!(editor.rows.iter().all(|row| row.label != "toward"));
    editor.select(Some(Item::Light("warm".into())));
    assert_eq!(row(&editor, "intensity").field, Field::Number(6.0));
    editor.select(Some(Item::Camera));
    assert_eq!(row(&editor, "fov").field, Field::Number(40.0));
    editor.select(Some(Item::Sky));
    assert_eq!(row(&editor, "turbidity").field, Field::Number(3.0));
    editor.select(Some(Item::Haze));
    assert!(editor.rows.is_empty());
}

#[test]
fn a_row_change_is_refused_when_unchanged_clamped_in_range_and_unsets_for_none() {
    let path = scene_copy("change");
    let mut editor = Editor::open(&path).unwrap();
    editor.select(Some(Item::Material("clay".into())));
    let roughness = editor
        .rows
        .iter()
        .find(|row| row.label == "roughness")
        .cloned()
        .unwrap();
    assert!(roughness.change(Field::Number(0.9)).is_none());
    assert!(roughness.change(Field::Toggle(true)).is_none());
    let clamped = roughness.change(Field::Number(3.0)).unwrap();
    assert_eq!(clamped.value, Some(Value::Float(1.0)));
    editor.select(Some(Item::Object("crate".into())));
    let material = editor
        .rows
        .iter()
        .find(|row| row.label == "material")
        .cloned()
        .unwrap();
    let Field::Choice { options, .. } = &material.field else {
        panic!();
    };
    let none = material
        .change(Field::Choice {
            value: String::new(),
            options: options.clone(),
        })
        .unwrap();
    assert_eq!(none.value, None);
    let before = text(&path);
    editor.change(none, Group::Keep);
    let after = text(&path);
    assert!(!after.contains("material = \"clay\""), "{after}");
    assert!(after.contains("# the crate sits by the wall"));
    editor.undo();
    assert_eq!(text(&path), before);
}

#[test]
fn an_inspector_edit_writes_the_defining_file_keeps_its_text_and_undoes_to_the_byte() {
    let path = scene_copy("write");
    let materials = library(&path);
    let before_scene = text(&path);
    let before = text(&materials);
    let mut editor = Editor::open(&path).unwrap();
    let revision = editor.revision;
    editor.change(
        Change {
            target: Target::Material("clay".into()),
            path: vec!["roughness".into()],
            value: Some(Value::Float(0.5)),
        },
        Group::Keep,
    );
    let after = text(&materials);
    assert_eq!(after, before.replace("roughness = 0.9", "roughness = 0.5"));
    assert_eq!(text(&path), before_scene);
    assert!(editor.revision > revision);
    assert_eq!(editor.scene().library.get("clay").unwrap().roughness, 0.5);
    assert!(editor.log.last().unwrap().text.contains("clay"));
    assert_eq!(editor.saves, vec![materials.clone()]);
    editor.undo();
    assert_eq!(text(&materials), before);
    editor.redo();
    assert_eq!(text(&materials), after);
    editor.change(
        Change {
            target: Target::Material("clay".into()),
            path: vec!["roughness".into()],
            value: Some(Value::Text("rough".into())),
        },
        Group::Keep,
    );
    assert_eq!(text(&materials), after);
    let refused = editor.log.last().unwrap();
    assert_eq!(refused.level, crate::log::Level::Error);
    assert!(refused.file.is_some());
}

#[test]
fn clicking_the_outline_selects_and_typing_a_number_edits_the_material_then_undo() {
    let path = scene_copy("flow-material");
    let materials = library(&path);
    let before = text(&materials);
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, "click @material:clay\nframe");
    assert_eq!(editor.selection, Some(Item::Material("clay".into())));
    screen.drive(&mut editor, &frames(2));
    screen.drive(
        &mut editor,
        "click @row:roughness\nframe\nkey Backspace\nkey Backspace\nkey Backspace\nkey Backspace\nkey Backspace\ntext 0.25\nkey Enter\nframe\nframe",
    );
    assert_eq!(
        text(&materials),
        before.replace("roughness = 0.9", "roughness = 0.25")
    );
    screen.drive(&mut editor, "key ctrl+Z\nframe");
    assert_eq!(text(&materials), before);
    screen.drive(&mut editor, "key Escape\nframe");
    assert_eq!(editor.selection, None);
}

#[test]
fn the_gizmo_drags_the_crate_along_x_in_one_undo_step() {
    let path = scene_copy("flow-gizmo");
    let before = text(&path);
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, "click @object:crate\nframe");
    assert_eq!(editor.selected_object(), Some("crate"));
    assert_eq!(editor.viewport.selection().unwrap().name, "crate");
    drag_x(&screen, &mut editor, 60.0);
    let moved = editor.scene().object("crate").unwrap().at;
    assert!(moved[0] > -0.9 + 0.05, "{moved:?}");
    assert!(
        (moved[1] - 0.35).abs() < 1e-4 && moved[2].abs() < 1e-4,
        "{moved:?}"
    );
    assert_eq!(editor.history.undo_labels(), vec!["move crate along x"]);
    let after = text(&path);
    assert!(after.contains("# the crate sits by the wall"));
    assert!(after.contains("rotate = [0.0, 25.0, 0.0]"));
    assert_eq!(editor.files.text(&path).unwrap(), after);
    screen.drive(&mut editor, "key ctrl+Z\nframe");
    assert_eq!(text(&path), before);
    assert!(!editor.history.can_undo());
    assert_eq!(
        editor.scene().object("crate").unwrap().at,
        [-0.9, 0.35, 0.0]
    );
}

#[test]
fn a_click_in_the_viewport_picks_the_object_under_it() {
    let path = scene_copy("flow-pick");
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    let pillar = editor.scene().object("pillar").unwrap().model;
    let on = point(&editor, [pillar[3][0], pillar[3][1] + 0.3, pillar[3][2]]);
    screen.drive(&mut editor, &format!("click {} {}\nframe", on.x, on.y));
    assert_eq!(editor.selection, Some(Item::Object("pillar".into())));
    let image = editor.viewport.image().unwrap();
    let (other, found) = (0..16)
        .flat_map(|y| (0..16).map(move |x| (x, y)))
        .map(|(x, y)| {
            image.min
                + egui::vec2(
                    image.width() * (x as f32 + 0.5) / 16.0,
                    image.height() * (y as f32 + 0.5) / 16.0,
                )
        })
        .find_map(|at| {
            let found = editor.viewport.pick_at(at)?;
            (found.name != "pillar").then_some((at, found.name))
        })
        .expect("another object shows");
    screen.drive(
        &mut editor,
        &format!("click {} {}\nframe", other.x, other.y),
    );
    assert_eq!(editor.selection, Some(Item::Object(found)));
    screen.drive(&mut editor, "key Escape\nframe");
    assert_eq!(editor.selection, None);
}

#[test]
fn keys_switch_the_gizmo_the_camera_the_trace_and_the_selection() {
    let path = scene_copy("flow-keys");
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, "move @viewport\nframe\nkey R\nframe");
    assert_eq!(editor.viewport.gizmo().mode, Mode::Rotate);
    screen.drive(&mut editor, "key S\nframe");
    assert_eq!(editor.viewport.gizmo().mode, Mode::Scale);
    screen.drive(&mut editor, "click @button:move");
    assert_eq!(editor.viewport.gizmo().mode, Mode::Move);
    assert_eq!(editor.viewport.look(), Look::Scene);
    screen.drive(&mut editor, "key C\nframe");
    assert!(matches!(editor.viewport.look(), Look::Orbit(_)));
    screen.drive(&mut editor, "key C\nframe");
    assert_eq!(editor.viewport.look(), Look::Scene);
    screen.drive(&mut editor, "move @viewport\nframe\nkey ArrowLeft\nframe");
    assert!(matches!(editor.viewport.look(), Look::Orbit(_)));
    screen.drive(&mut editor, "move 4 4\nframe\nkey T\nframe");
    assert!(editor.trace);
    screen.drive(&mut editor, "key J\nkey J\nframe");
    assert_eq!(editor.selection, Some(Item::Object("wall".into())));
    screen.drive(&mut editor, "key J\nkey J\nframe");
    assert_eq!(editor.selection, Some(Item::Object("ball".into())));
    assert_eq!(editor.viewport.selection().unwrap().name, "ball");
    editor.select(Some(Item::Object("wall".into())));
    screen.drive(&mut editor, "key F\nframe");
    let Look::Orbit(eye) = editor.viewport.look() else {
        panic!("F frames the selection in the editor camera");
    };
    assert!((eye.target[2] - (-2.0)).abs() < 0.2, "{:?}", eye.target);
    screen.drive(&mut editor, "key Escape\nframe");
    assert_eq!(editor.selection, None);
    screen.drive(&mut editor, "key F5\nframe");
    assert_eq!(editor.take_requests(), [Request::Play]);
}

#[test]
fn one_undo_stack_takes_back_the_gizmo_the_inspector_and_the_toml_panel_in_turn() {
    let path = scene_copy("one-stack");
    let materials = library(&path);
    let lights = lights(&path);
    let before = [text(&path), text(&materials), text(&lights)];
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    editor.change(
        Change {
            target: Target::Material("clay".into()),
            path: vec!["roughness".into()],
            value: Some(Value::Float(0.5)),
        },
        Group::Keep,
    );
    let inspected = text(&materials);
    assert_eq!(
        inspected,
        before[1].replace("roughness = 0.9", "roughness = 0.5")
    );
    screen.drive(&mut editor, "click @object:crate\nframe");
    drag_x(&screen, &mut editor, 50.0);
    let dragged = text(&path);
    assert_ne!(dragged, before[0]);
    type_into(&screen, &mut editor, &lights, "intensity = 6", 1, "4");
    assert_eq!(
        text(&lights),
        before[2].replace("intensity = 6.0", "intensity = 4.0")
    );
    assert_eq!(editor.scene().lights["warm"].intensity, 4.0);
    assert_eq!(editor.history.undo_len(), 3);
    assert!(editor.history.undo_labels()[1].starts_with("move crate"));
    editor.select(None);
    screen.drive(&mut editor, "key ctrl+Z\nframe");
    assert_eq!(text(&lights), before[2]);
    assert_eq!(text(&path), dragged);
    assert_eq!(editor.scene().lights["warm"].intensity, 6.0);
    screen.drive(&mut editor, "key ctrl+Z\nframe");
    assert_eq!(text(&path), before[0]);
    assert_eq!(text(&materials), inspected);
    assert_eq!(
        editor.viewport.scene().unwrap().object("crate").unwrap().at,
        [-0.9, 0.35, 0.0]
    );
    screen.drive(&mut editor, "key ctrl+Z\nframe");
    assert_eq!(text(&materials), before[1]);
    assert!(!editor.history.can_undo());
    screen.drive(&mut editor, "key ctrl+Y\nframe");
    assert_eq!(text(&materials), inspected);
    assert_eq!(editor.history.redo_len(), 2);
}

#[test]
fn the_toml_panel_lists_pfx_scene_problems_at_line_and_column_and_refuses_them() {
    let path = scene_copy("toml-problems");
    let lights = lights(&path);
    let before = text(&lights);
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    type_into(&screen, &mut editor, &lights, "range = 8.0", 0, "# far");
    assert!(text(&lights).contains("range = 8.0# far"));
    let undo = editor.history.undo_len();
    let line = before
        .lines()
        .position(|line| line.starts_with("range"))
        .unwrap()
        + 2;
    editor
        .sources
        .get_mut(&lights)
        .unwrap()
        .jump(text(&lights).find("range").unwrap());
    screen.drive(
        &mut editor,
        &format!("frame\ntext wide = 1\nkey Enter\n{}", frames(15)),
    );
    let source = &editor.sources[&lights];
    let problem = source
        .problems()
        .iter()
        .find(|problem| problem.message.contains("wide"))
        .unwrap_or_else(|| panic!("{:?}", source.problems()));
    assert_eq!(problem.file, lights);
    assert_eq!((problem.line, problem.column), (line as u32 - 1, 1));
    assert!(source.unapplied());
    assert!(!text(&lights).contains("wide"));
    assert_eq!(editor.history.undo_len(), undo);
    assert!(editor.layout.find("source").is_some());
}

#[test]
fn a_save_is_a_dry_run_landed_on_the_history_and_written_once() {
    let path = scene_copy("dry-save");
    let materials = library(&path);
    let before = text(&materials);
    let mut editor = Editor::open(&path).unwrap();
    let change = Change {
        target: Target::Material("clay".into()),
        path: vec!["roughness".into()],
        value: Some(Value::Float(0.4)),
    };
    let group = editor.dry(&change).unwrap();
    assert_eq!(text(&materials), before, "a dry run writes nothing");
    assert_eq!(group.patches.len(), 1);
    assert_eq!(group.patches[0].file, materials);
    assert!(editor.saves.is_empty());
    let reloads = editor.viewport.reloads();
    editor.land(group);
    assert_eq!(editor.saves, vec![materials.clone()]);
    assert_eq!(
        text(&materials),
        before.replace("roughness = 0.9", "roughness = 0.4")
    );
    assert_eq!(editor.viewport.reloads(), reloads + 1);
    assert_eq!(
        editor
            .viewport
            .scene()
            .unwrap()
            .library
            .get("clay")
            .unwrap()
            .roughness,
        0.4
    );
    editor.viewport.check();
    editor.events();
    assert_eq!(
        editor.viewport.reloads(),
        reloads + 1,
        "its own write reloads once"
    );
    assert!(
        editor
            .log
            .entries
            .iter()
            .all(|entry| !entry.text.contains("outside"))
    );
    assert_eq!(editor.history.undo_len(), 1);
    editor.saves.clear();
    for (group, value) in [(Group::Begin, 0.3), (Group::Keep, 0.2), (Group::End, 0.1)] {
        editor.change(
            Change {
                value: Some(Value::Float(value)),
                ..change.clone()
            },
            group,
        );
    }
    assert_eq!(editor.saves, vec![materials.clone(); 3], "one write a step");
    assert_eq!(editor.history.undo_len(), 2, "a drag is one undo step");
    editor.saves.clear();
    editor.undo();
    assert_eq!(editor.saves, vec![materials.clone()]);
    assert_eq!(
        text(&materials),
        before.replace("roughness = 0.9", "roughness = 0.4")
    );
}

#[test]
fn an_outside_edit_reloads_the_files_and_drops_the_steps_it_breaks() {
    let path = scene_copy("outside");
    let mut editor = Editor::open(&path).unwrap();
    editor.change(
        Change {
            target: Target::Light("warm".into()),
            path: vec!["intensity".into()],
            value: Some(Value::Float(9.0)),
        },
        Group::Keep,
    );
    assert!(editor.history.can_undo());
    editor.reloaded();
    assert!(editor.history.can_undo());
    let lights = lights(&path);
    fs::write(
        &lights,
        text(&lights).replace("intensity = 9.0", "intensity = 2.0"),
    )
    .unwrap();
    editor.outside = true;
    editor.viewport.check();
    editor.events();
    assert!(!editor.history.can_undo());
    assert_eq!(editor.scene().lights["warm"].intensity, 2.0);
    assert_eq!(editor.files.text(&lights).unwrap(), text(&lights));
    assert_eq!(editor.log.last().unwrap().level, crate::log::Level::Warn);
}

#[test]
fn while_playing_edits_go_to_the_world_and_no_file_is_written() {
    let path = scene_copy("playing");
    let before = text(&path);
    let mut editor = Editor::open(&path).unwrap();
    editor.select(Some(Item::Object("crate".into())));
    editor.play = Play::Playing;
    let change = Change {
        target: Target::Object("crate".into()),
        path: vec!["at".into()],
        value: Some(Value::from([0.0f32, 1.0, 0.0])),
    };
    editor.change(change.clone(), Group::Keep);
    assert_eq!(editor.take_requests(), [Request::World(change)]);
    editor.undo();
    assert_eq!(text(&path), before);
    assert!(editor.saves.is_empty());
    editor.play_pressed();
    assert_eq!(editor.take_requests(), [Request::Pause]);
    editor.play = Play::Paused;
    editor.play_pressed();
    assert_eq!(editor.take_requests(), [Request::Resume]);
}

#[test]
fn f9_asks_for_the_input_recording_only_while_a_scene_plays() {
    let path = scene_copy("record");
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, "key F9\nframe");
    assert_eq!(editor.take_requests(), []);
    editor.play = Play::Playing;
    screen.drive(&mut editor, "key F9\nframe");
    assert_eq!(editor.take_requests(), [Request::Record]);
    editor.play = Play::Paused;
    screen.drive(&mut editor, "key F9\nframe");
    assert_eq!(editor.take_requests(), [Request::Record]);
}

#[test]
fn one_frame_asks_for_one_recording_however_often_its_pass_runs() {
    let path = scene_copy("record-once");
    let mut editor = Editor::open(&path).unwrap();
    editor.play = Play::Playing;
    editor.request(Request::Record);
    editor.request(Request::Record);
    assert_eq!(editor.take_requests(), vec![Request::Record]);
    editor.request(Request::Record);
    assert_eq!(editor.take_requests(), vec![Request::Record]);
}

#[test]
fn run_wants_one_scene_file() {
    assert_eq!(crate::run(&[]).unwrap_err(), crate::USAGE);
    assert_eq!(crate::run(&["--x".to_string()]).unwrap_err(), crate::USAGE);
    assert!(
        crate::run(&["tmp/no-such.scene.toml".to_string()])
            .unwrap_err()
            .contains("no such scene file")
    );
}

#[test]
fn the_mute_button_and_key_toggle_mute_for_the_session() {
    let path = scene_copy("mute");
    let mut editor = Editor::open(&path).unwrap();
    let screen = Screen::new(&mut editor);
    assert!(!editor.muted);
    assert!(editor.layout.find("button:mute").is_none());
    editor.sound_available = true;
    screen.drive(&mut editor, &frames(2));
    screen.drive(&mut editor, "click @button:mute");
    assert!(editor.muted);
    screen.drive(&mut editor, "key M");
    assert!(!editor.muted);
    screen.drive(&mut editor, "key M");
    assert!(editor.muted);
}

pub fn project_copy(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../../tmp/editor-tests").join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = std::path::absolute(root).unwrap();
    pfx_project::scaffold("sample-game", &root, "0.0.0").unwrap();
    let first = fs::read_to_string(root.join("content/first.scene.toml")).unwrap();
    let second: String = first
        .lines()
        .filter(|line| !line.starts_with("id = "))
        .map(|line| format!("{}\n", line.replace("intensity = 9.0", "intensity = 5.0")))
        .collect();
    fs::write(root.join("content/second.scene.toml"), second).unwrap();
    root
}

fn quiet() -> crate::session::Setup {
    crate::session::Setup {
        pgpu: false,
        audio: false,
    }
}

fn labels(editor: &Editor, section: &str) -> Vec<String> {
    editor
        .outline
        .section(section)
        .unwrap()
        .nodes
        .iter()
        .map(|node| node.label.clone())
        .collect()
}

#[test]
fn a_project_lists_its_scenes_and_content_folders_in_the_outliner() {
    let root = project_copy("project-outline");
    let session = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    let editor = &session.editor;
    assert_eq!(editor.path(), root.join("content/first.scene.toml"));
    assert_eq!(
        labels(editor, "scenes"),
        [
            "content/first.scene.toml  ·  open",
            "content/second.scene.toml"
        ]
    );
    assert_eq!(
        labels(editor, "content"),
        [
            "content",
            "content/fonts",
            "content/materials",
            "content/meshes",
            "content/prefabs"
        ]
    );
    let meshes = &editor.outline.section("content").unwrap().nodes[3];
    assert_eq!(
        meshes
            .children
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>(),
        ["badge.glb", "grid.glb", "wordmark.glb"]
    );
    assert_eq!(
        editor.selection,
        Some(Item::Object("incoming".to_string())),
        "the project's select names the object the editor opens on"
    );
    assert_eq!(
        editor.viewport.selection().map(|found| found.name.as_str()),
        Some("incoming")
    );
    let objects = &editor.outline.section("objects").unwrap().nodes;
    let tree: Vec<(String, Vec<String>)> = objects
        .iter()
        .map(|node| {
            (
                node.label.clone(),
                node.children
                    .iter()
                    .map(|child| child.label.clone())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        tree,
        [
            (
                "incoming".to_string(),
                vec![
                    "incoming/badge".to_string(),
                    "incoming/wordmark".to_string()
                ]
            ),
            ("grid".to_string(), Vec::new())
        ],
        "the outliner shows the parent, the badge and the wordmark"
    );
    assert_eq!(
        labels(editor, "materials"),
        [
            "axis_x", "axis_z", "dome", "grid", "lime", "lines", "tile", "violet", "white",
            "wordmark"
        ]
    );
    let titles: Vec<&str> = editor
        .outline
        .sections
        .iter()
        .map(|section| section.title)
        .collect();
    assert_eq!(
        titles,
        [
            "scenes",
            "content",
            "objects",
            "meshes",
            "lights",
            "materials",
            "world",
            "files"
        ]
    );
    assert!(editor.scene().object("incoming/wordmark").is_some());
    assert!(
        editor
            .log
            .entries
            .iter()
            .any(|entry| entry.text == "project sample-game: 2 scenes")
    );
    assert!(!session.plays_a_game());
}

#[test]
fn enter_or_a_double_click_on_a_scene_opens_it_and_the_project_stays() {
    let root = project_copy("project-open");
    let first = root.join("content/first.scene.toml");
    let second = root.join("content/second.scene.toml");
    let mut session = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    let screen = Screen::new(&mut session.editor);
    screen.drive(
        &mut session.editor,
        "click @scene:first.scene.toml\nframe\nkey Enter\nframe",
    );
    assert_eq!(session.editor.selection, Some(Item::Scene(first.clone())));
    assert!(session.editor.take_requests().is_empty());
    screen.drive(
        &mut session.editor,
        "click @scene:second.scene.toml\nframe\nkey Enter\nframe",
    );
    assert_eq!(
        session.editor.take_requests(),
        vec![Request::Open(second.clone())]
    );
    screen.drive(&mut session.editor, "click @button:open\nframe");
    assert_eq!(
        session.editor.take_requests(),
        vec![Request::Open(second.clone())]
    );
    session.switch(&second).unwrap();
    assert_eq!(session.editor.path(), second);
    assert!(session.project().is_some());
    assert_eq!(
        labels(&session.editor, "scenes"),
        [
            "content/first.scene.toml",
            "content/second.scene.toml  ·  open"
        ]
    );
    assert_eq!(session.editor.scene().lights["key"].intensity, 5.0);
    assert!(
        session
            .editor
            .log
            .entries
            .iter()
            .any(|entry| entry.text == "project sample-game: 2 scenes"),
        "the log carries over"
    );
    let screen = Screen::new(&mut session.editor);
    screen.drive(
        &mut session.editor,
        "click @scene:first.scene.toml\nclick @scene:first.scene.toml\nframe",
    );
    assert_eq!(session.editor.take_requests(), vec![Request::Open(first)]);
}

#[test]
fn a_folder_without_scenes_is_refused_and_a_bare_fixture_folder_opens() {
    let empty = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/editor-tests/project-empty");
    let _ = fs::remove_dir_all(&empty);
    fs::create_dir_all(&empty).unwrap();
    let error = crate::session::Session::open_project(&empty, quiet(), None)
        .err()
        .unwrap();
    assert!(error.contains("no *.scene.toml"), "{error}");
    let fixtures = scene_copy("project-bare");
    let folder = fixtures.parent().unwrap();
    let session = crate::session::Session::open_project(folder, quiet(), None).unwrap();
    assert_eq!(
        labels(&session.editor, "scenes").len(),
        2,
        "{:?}",
        labels(&session.editor, "scenes")
    );
    assert!(
        crate::run(&[empty.join("missing").to_string_lossy().into_owned()])
            .unwrap_err()
            .contains("no such scene file or project folder")
    );
}

#[test]
fn a_flat_project_with_no_scene_opens_on_an_empty_world() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/editor-tests/project-flat");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("project.toml"),
        "format = 1\n\n[project]\nname = \"flat-game\"\n\n[authoring.pfx]\nflat = true\n",
    )
    .unwrap();
    let session = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    assert!(session.flat());
    let world = root.join(crate::session::FLAT_SCENE);
    assert_eq!(
        std::path::absolute(session.editor.path()).unwrap(),
        std::path::absolute(&world).unwrap()
    );
    assert!(session.editor.scene().objects.is_empty());
    assert_eq!(
        session.editor.project_root(),
        session.project().unwrap().root()
    );
    assert!(labels(&session.editor, "scenes").is_empty());
    assert!(
        session
            .editor
            .log
            .entries
            .iter()
            .any(|entry| entry.text.contains("is flat and has no scene")),
        "{:?}",
        session.editor.log.entries
    );
    drop(session);
    assert_eq!(fs::read_to_string(&world).unwrap(), "format = 1\n");
    let again = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    assert!(again.flat());
    drop(again);

    fs::write(
        root.join("project.toml"),
        "format = 1\n\n[project]\nname = \"flat-game\"\n",
    )
    .unwrap();
    fs::remove_dir_all(root.join("tmp")).unwrap();
    let error = crate::session::Session::open_project(&root, quiet(), None)
        .err()
        .unwrap();
    assert!(error.contains("no *.scene.toml"), "{error}");
    let game = crate::factory(|| pfx_play::SceneGame);
    let config = pfx_game::Config::flat("flat-game", "0.0.0");
    let session = crate::session::Session::open_game(&root, quiet(), game, config).unwrap();
    assert!(session.flat() && session.plays_a_game());
    assert_eq!(session.editor.app, "flat-game v0.0.0");
    assert!(world.is_file());
}

#[test]
fn the_version_sits_dim_at_the_right_end_of_the_bottom_bar() {
    let path = scene_copy("version");
    let mut editor = Editor::open(&path).unwrap();
    assert_eq!(editor.app, format!("pfx v{}", env!("CARGO_PKG_VERSION")));
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, &frames(1));
    let version = editor.layout.version.unwrap();
    let viewport = editor.layout.viewport.unwrap();
    assert!(version.right() > SIZE[0] as f32 - 24.0, "{version:?}");
    assert!(
        version.top() >= viewport.bottom(),
        "{version:?} {viewport:?}"
    );
}

#[test]
fn the_scaffolds_lockup_shows_the_games_version_and_moves_with_its_group() {
    let root = project_copy("project-lockup-version");
    let session = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    let scene = session.editor.scene();
    let version = &scene.texts["incoming/version"];
    assert!(
        version.text.starts_with('v') && !version.text.contains('{'),
        "{}",
        version.text
    );
    assert_eq!(version.place, scene.object("incoming").unwrap().model);
}

#[test]
fn a_games_editor_shows_the_games_version_across_scenes() {
    assert_eq!(
        crate::editor::version_line("sample-game", "v1.2.0"),
        "sample-game v1.2.0"
    );
    let root = project_copy("project-version");
    let second = root.join("content/second.scene.toml");
    let game = crate::factory(|| pfx_play::SceneGame);
    let config = pfx_game::Config::flat("sample-game", "1.2.0");
    let mut session = crate::session::Session::open_game(&root, quiet(), game, config).unwrap();
    assert_eq!(session.editor.app, "sample-game v1.2.0");
    session.switch(&second).unwrap();
    assert_eq!(session.editor.app, "sample-game v1.2.0");
}

mod characters;
mod level;
mod live;

#[test]
fn the_scene_cameras_frame_follows_the_projects_screen_policy_for_the_device() {
    use pfx_gpu::screens::{DeckModel, Device};
    let root = project_copy("project-frame");
    let project = pfx_project::Project::open(&root).unwrap();
    let frame = |device| crate::editor::frame_aspect(&project, device).unwrap();
    assert_eq!(frame(Device::Desktop), Some(16.0 / 9.0));
    assert_eq!(
        frame(Device::SteamDeck {
            model: DeckModel::Lcd
        }),
        Some(16.0 / 10.0)
    );
    let session = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    assert!(session.editor.viewport.frame_aspect().is_some());
    let text = fs::read_to_string(root.join("project.toml")).unwrap();
    let start = text.find("\n[authoring.pfx.screen]").unwrap();
    fs::write(root.join("project.toml"), &text[..start]).unwrap();
    let bare = pfx_project::Project::open(&root).unwrap();
    assert_eq!(
        crate::editor::frame_aspect(&bare, Device::Desktop).unwrap(),
        None
    );
    let session = crate::session::Session::open_project(&root, quiet(), None).unwrap();
    assert_eq!(session.editor.viewport.frame_aspect(), None);
    fs::write(
        root.join("project.toml"),
        format!(
            "{}\n[authoring.pfx.screen]\ndesktop = [\"0:9\"]\n",
            &text[..start]
        ),
    )
    .unwrap();
    let broken = pfx_project::Project::open(&root).unwrap();
    let error = crate::editor::frame_aspect(&broken, Device::Desktop).unwrap_err();
    assert!(error.contains("[authoring.pfx.screen]"), "{error}");
}

fn cyclic(root: &std::path::Path, kind: &str) -> std::path::PathBuf {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root.join("content")).unwrap();
    let write = |relative: &str, text: &str| std::fs::write(root.join(relative), text).unwrap();
    write(
        "project.toml",
        "format = 1\n\n[project]\nname = \"loop\"\nscene = \"content/main.scene.toml\"\n",
    );
    match kind {
        "prefab" => {
            write(
                "content/main.scene.toml",
                "format = 1\n\n[[object]]\nname = \"group\"\nprefab = \"content/loop.prefab.toml\"\n",
            );
            write(
                "content/loop.prefab.toml",
                "format = 1\n\n[[object]]\nname = \"again\"\nprefab = \"content/loop.prefab.toml\"\n",
            );
        }
        "include" => {
            write(
                "content/main.scene.toml",
                "format = 1\n\ninclude = [\"content/other.scene.toml\"]\n",
            );
            write(
                "content/other.scene.toml",
                "format = 1\n\ninclude = [\"content/main.scene.toml\"]\n",
            );
        }
        _ => {
            write(
                "content/main.scene.toml",
                "format = 1\n\n[[object]]\nname = \"group\"\nprefab = \"content/p0.prefab.toml\"\n",
            );
            for at in 0..70 {
                write(
                    &format!("content/p{at}.prefab.toml"),
                    &format!(
                        "format = 1\n\n[[object]]\nname = \"next\"\nprefab = \"content/p{}.prefab.toml\"\n",
                        at + 1
                    ),
                );
            }
            write("content/p70.prefab.toml", "format = 1\n");
        }
    }
    root.join("content/main.scene.toml")
}

const FINDINGS: [(&str, &str); 3] = [
    ("prefab", "which places itself"),
    ("include", "includes itself"),
    ("deep", "nests prefabs more than 64 deep"),
];

#[test]
fn opening_a_cyclic_or_too_deep_project_is_a_clean_error() {
    for (kind, says) in FINDINGS {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/editor-tests")
            .join(format!("cyclic-{kind}"));
        let scene = cyclic(&root, kind);
        let error = crate::session::Session::open_project(&root, quiet(), None)
            .err()
            .unwrap();
        assert!(error.contains(says), "{kind}: {error}");
        let error = crate::session::Session::open(&scene, quiet())
            .err()
            .unwrap();
        assert!(error.contains(says), "{kind}: {error}");
        if kind == "include" {
            for file in [
                "main.scene.toml:3:12: see here",
                "other.scene.toml:3:12: see here",
            ] {
                assert!(error.contains(file), "the chain names {file}: {error}");
            }
        }
    }
}
