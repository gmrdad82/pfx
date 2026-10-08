use std::fs;
use std::path::{Path, PathBuf};

use glam::DVec3;

use super::{Screen, point, text};
use crate::editor::Editor;
use crate::level::{Shape, Stroke, Tool};
use crate::outline::Item;
use crate::script::frames;
use crate::snap::Surfaces;

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub fn level_copy(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../../tmp/editor-tests/level").join(name);
    let _ = fs::remove_dir_all(&root);
    copy(&manifest.join("../load/tests/level"), &root);
    std::path::absolute(root).unwrap()
}

fn open(name: &str) -> (Editor, PathBuf) {
    let root = level_copy(name);
    let mut editor = Editor::open(root.join("level.scene.toml")).unwrap();
    editor.set_project(pfx_project::Project::folder(&root).unwrap());
    (editor, root)
}

fn at(editor: &Editor, name: &str) -> [f32; 3] {
    editor.scene().object(name).unwrap().at
}

fn near(a: [f32; 3], b: [f32; 3], within: f32) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= within)
}

fn open_content(screen: &Screen, editor: &mut Editor) {
    screen.drive(editor, "click @button:content\nframe\nframe");
    assert!(
        editor.layout.find("asset:block.prefab.toml").is_some(),
        "{:?}",
        editor.layout.outline.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_tile_layer_shows_as_its_layers_not_its_cells() {
    let (mut editor, root) = open("outline");
    assert_eq!(editor.scene().objects.len(), 10);
    let tiles = editor.outline.section("tiles").unwrap();
    assert_eq!(tiles.nodes.len(), 1);
    assert_eq!(tiles.nodes[0].item, Item::Layer("ground".into()));
    assert_eq!(tiles.nodes[0].label, "ground  ·  7 cells");
    let roots: Vec<&str> = editor
        .outline
        .section("objects")
        .unwrap()
        .nodes
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(roots, ["floor", "ledge"]);
    assert_eq!(editor.layers().len(), 1);
    assert!(editor.files().contains(&root.join("level.scene.toml")));
    assert!(editor.is_cell("ground[0,0]/block"));
    assert!(!editor.is_cell("ledge"));
    editor.select(Some(Item::Layer("ground".into())));
    assert_eq!(editor.level.brush.layer.as_deref(), Some("ground"));
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, &frames(1));
    assert!(editor.layout.buttons.contains_key("paint"));
    assert!(editor.problems.is_empty(), "{:?}", editor.problems);
}

#[test]
fn dragging_a_prefab_from_the_content_panel_places_it_on_the_floor_in_one_undo_step() {
    let (mut editor, root) = open("drop");
    let scene = root.join("level.scene.toml");
    let before = text(&scene);
    let screen = Screen::new(&mut editor);
    open_content(&screen, &mut editor);
    let steps = editor.history.undo_len();
    let target = point(&editor, [-1.0, 0.0, 1.5]);
    screen.drive(
        &mut editor,
        &format!(
            "drag @asset:block.prefab.toml {} {}\nframe",
            target.x, target.y
        ),
    );
    assert_eq!(editor.history.undo_len(), steps + 1);
    assert_eq!(editor.history.undo_label(), Some("place block"));
    let placed = at(&editor, "block");
    assert!(near(placed, [-1.0, 0.0, 1.5], 0.02), "{placed:?}");
    assert_eq!(editor.selection, Some(Item::Object("block".into())));
    let written = text(&scene);
    assert!(
        written.contains("prefab = \"prefabs/block.prefab.toml\""),
        "{written}"
    );
    assert!(written.starts_with(&before[..before.find("[sky]").unwrap()]));
    assert!(editor.scene().object("block/block").is_some());
    editor.undo();
    assert_eq!(text(&scene), before);
    assert!(editor.scene().object("block").is_none());
}

#[test]
fn grid_snapping_and_the_place_key_set_a_stamp_on_the_grid() {
    let (mut editor, root) = open("grid");
    let scene = root.join("level.scene.toml");
    let screen = Screen::new(&mut editor);
    editor.select(Some(Item::Asset(root.join("prefabs/block.prefab.toml"))));
    assert_eq!(
        editor.level.stamp,
        Some(root.join("prefabs/block.prefab.toml"))
    );
    screen.drive(&mut editor, "key N\nframe");
    assert!(editor.level.snapping.grid);
    assert_eq!(editor.viewport.gizmo().snap.distance, 0.5);
    screen.drive(
        &mut editor,
        "key OpenBracket\nframe\nkey CloseBracket\nframe",
    );
    assert_eq!(editor.level.snapping.step, 0.5);
    assert!(editor.layout.buttons.contains_key("grid"));
    let steps = editor.history.undo_len();
    let target = point(&editor, [0.37, 0.0, 1.12]);
    screen.drive(
        &mut editor,
        &format!("move {} {}\nkey P\nframe", target.x, target.y),
    );
    assert_eq!(editor.history.undo_len(), steps + 1);
    assert_eq!(at(&editor, "block"), [0.5, 0.0, 1.0]);
    assert!(text(&scene).contains("at = [0.5, 0.0, 1.0]"));
    screen.drive(&mut editor, "key N\nframe");
    assert_eq!(editor.viewport.gizmo().snap.distance, 0.01);
    editor.undo();
    assert!(editor.scene().object("block").is_none());
}

#[test]
fn surface_snapping_places_onto_the_surface_under_the_pointer_and_aligns_to_its_normal() {
    let (mut editor, root) = open("surface");
    let screen = Screen::new(&mut editor);
    let block = root.join("prefabs/block.prefab.toml");
    let top = point(&editor, [2.1, 1.0, -0.9]);
    let name = editor.place(&block, top).unwrap();
    assert!(
        near(at(&editor, &name), [2.1, 1.0, -0.9], 0.02),
        "{:?}",
        at(&editor, &name)
    );
    let steps = editor.history.undo_len();
    editor.level.snapping.align = true;
    let front = point(&editor, [1.8, 0.4, -0.5]);
    let name = editor.place(&block, front).unwrap();
    assert_eq!(editor.history.undo_len(), steps + 1);
    let object = editor.scene().object(&name).unwrap().clone();
    assert!(near(object.at, [1.8, 0.4, -0.5], 0.02), "{:?}", object.at);
    assert_eq!(object.rotate, [90.0, 0.0, 0.0]);
    editor.level.snapping.surface = false;
    editor.level.snapping.align = false;
    let name = editor.place(&block, top).unwrap();
    assert!(
        at(&editor, &name)[1].abs() < 1e-4,
        "{:?}",
        at(&editor, &name)
    );
    screen.drive(&mut editor, &frames(1));
    assert!(editor.layout.buttons.contains_key("surface"));
}

#[test]
fn vertex_snapping_finds_the_corner_nearest_the_pointer() {
    let (mut editor, _root) = open("vertex");
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, &frames(1));
    let corner = [2.5_f32, 1.0, -0.5];
    let seen = point(&editor, corner);
    let lens = editor.lens().unwrap();
    let surfaces = Surfaces::of(editor.scene());
    let found = surfaces
        .vertex(&lens, seen + egui::vec2(4.0, -3.0), 12.0, &[])
        .unwrap();
    assert!(
        (found - DVec3::from_array(corner.map(f64::from))).length() < 1e-4,
        "{found}"
    );
    assert!(
        surfaces
            .vertex(&lens, egui::pos2(-50.0, -50.0), 12.0, &[])
            .is_none()
    );
    editor.level.snapping.vertex = true;
    editor.level.snapping.surface = false;
    let block = editor.project_root().join("meshes/block.gltf");
    let name = editor.place(&block, seen + egui::vec2(4.0, -3.0)).unwrap();
    assert!(
        near(at(&editor, &name), [2.5, 1.5, -0.5], 1e-3),
        "{:?}",
        at(&editor, &name)
    );
    assert_eq!(editor.scene().object(&name).unwrap().mesh, "block");
}

#[test]
fn the_brush_paints_a_line_of_tiles_in_one_undo_step_and_erases_and_picks() {
    let (mut editor, root) = open("brush");
    let scene = root.join("level.scene.toml");
    let before = text(&scene);
    let screen = Screen::new(&mut editor);
    screen.drive(&mut editor, "key B\nframe");
    assert!(editor.level.brush.on);
    let layer = editor.brush_layer().unwrap();
    let from = point(&editor, layer.position([0, 2]));
    let to = point(&editor, layer.position([5, 2]));
    assert_eq!(editor.cell_under(from), Some([0, 2]));
    let steps = editor.history.undo_len();
    screen.drive(
        &mut editor,
        &format!("drag {} {} {} {} shift\nframe", from.x, from.y, to.x, to.y),
    );
    assert_eq!(editor.history.undo_len(), steps + 1);
    assert_eq!(editor.history.undo_label(), Some("paint 6 tiles in ground"));
    let painted = editor.brush_layer().unwrap();
    assert_eq!(painted.rows.as_ref().unwrap()[0], "######");
    assert_eq!(painted.cells().len(), 13);
    assert!(text(&scene).contains("# a side-view strip"));
    assert_eq!(editor.scene().objects.len(), 10 + 6);
    assert!(editor.scene().object("ground[5,2]/block").is_some());
    assert_eq!(
        editor.outline.section("tiles").unwrap().nodes[0].label,
        "ground  ·  13 cells"
    );
    editor.level.brush.tool = Tool::Erase;
    editor.level.brush.stroke = Some(Stroke::new(Shape::Free, [5, 2]));
    editor.finish_stroke();
    assert!(editor.scene().object("ground[5,2]/block").is_none());
    assert_eq!(editor.history.undo_label(), Some("erase 1 tile in ground"));
    editor.level.brush.tool = Tool::Pick;
    editor.level.brush.stroke = Some(Stroke::new(Shape::Free, [4, 1]));
    editor.finish_stroke();
    assert_eq!(
        editor.level.brush.prefab.as_deref(),
        Some("prefabs/post.prefab.toml")
    );
    assert_eq!(editor.level.brush.tool, Tool::Paint);
    editor.undo();
    editor.undo();
    assert_eq!(text(&scene), before);
    assert_eq!(editor.scene().objects.len(), 10);
    editor.select(Some(Item::Object("ground[0,0]/block".into())));
    editor.level.brush.on = false;
    screen.drive(&mut editor, &frames(1));
}

#[test]
fn a_rectangle_paints_and_a_new_prefab_joins_the_palette_and_a_paint_grows_the_layer_down() {
    let (mut editor, root) = open("rectangle");
    editor.level.brush.on = true;
    editor.level.brush.prefab = Some("prefabs/post.prefab.toml".into());
    let mut stroke = Stroke::new(Shape::Rectangle, [10, 0]);
    stroke.reach([11, 1]);
    editor.level.brush.stroke = Some(stroke);
    let steps = editor.history.undo_len();
    editor.finish_stroke();
    assert_eq!(editor.history.undo_len(), steps + 1);
    let layer = editor.brush_layer().unwrap();
    for cell in [[10, 0], [11, 0], [10, 1], [11, 1]] {
        assert_eq!(
            pfx_load::scene::tiles::get(&layer, cell).as_deref(),
            Some("p"),
            "{cell:?}"
        );
    }
    fs::write(
        root.join("prefabs/step.prefab.toml"),
        "format = 1\n\nmaterials = [\"materials.toml\"]\n\n[mesh.step]\nfile = \"meshes/block.gltf\"\n\n[[object]]\nname = \"step\"\nmesh = \"step\"\nat = [0.0, 0.25, 0.0]\nscale = [1.0, 0.5, 1.0]\nmaterial = \"stone\"\n",
    )
    .unwrap();
    editor.level.brush.prefab = Some("prefabs/step.prefab.toml".into());
    editor.level.brush.stroke = Some(Stroke::new(Shape::Free, [-1, -1]));
    editor.finish_stroke();
    assert_eq!(editor.history.undo_len(), steps + 2);
    let layer = editor.brush_layer().unwrap();
    assert_eq!(
        layer.palette.get("@").map(String::as_str),
        Some("prefabs/step.prefab.toml")
    );
    assert_eq!(layer.origin, [-4.5, -1.0, -2.0]);
    assert_eq!(
        pfx_load::scene::tiles::get(&layer, [0, 0]).as_deref(),
        Some("@")
    );
    assert!(editor.scene().object("ground[0,0]/step").is_some());
    let block = editor.scene().object("ground[1,1]/block").unwrap();
    assert_eq!(block.at, [-3.5, 0.5, -2.0]);
}

#[test]
fn shift_clicks_pick_several_and_align_distribute_and_duplicate_are_one_undo_step_each() {
    let (mut editor, root) = open("arrange");
    let scene = root.join("level.scene.toml");
    let block = root.join("prefabs/block.prefab.toml");
    let a = editor.place_at(&block, [0.0, 0.0, 1.0], [0.0; 3]).unwrap();
    let b = editor.place_at(&block, [1.0, 0.5, 2.0], [0.0; 3]).unwrap();
    let c = editor.place_at(&block, [3.0, 0.0, 0.0], [0.0; 3]).unwrap();
    assert_eq!(
        [a.as_str(), b.as_str(), c.as_str()],
        ["block", "block 2", "block 3"]
    );
    let before = text(&scene);
    let screen = Screen::new(&mut editor);
    screen.drive(
        &mut editor,
        "click @object:floor\nframe\npress @object:ledge shift\nrelease @object:ledge\nframe",
    );
    assert_eq!(editor.level.picked, ["floor", "ledge"]);
    assert_eq!(editor.selection, Some(Item::Object("ledge".into())));
    screen.drive(&mut editor, &format!("click @object:{a}\nframe"));
    assert!(editor.level.picked.is_empty());
    editor.pick_more(&b);
    editor.pick_more(&c);
    assert_eq!(editor.level.picked, [a.clone(), b.clone(), c.clone()]);
    assert_eq!(editor.selection, Some(Item::Object(c.clone())));
    screen.drive(&mut editor, &frames(1));
    assert!(editor.layout.buttons.contains_key("distribute x"));
    let steps = editor.history.undo_len();
    assert!(editor.align(1, false));
    assert_eq!(editor.history.undo_len(), steps + 1);
    assert_eq!(at(&editor, &b), [1.0, 0.0, 2.0]);
    assert!(editor.distribute(0));
    assert_eq!(editor.history.undo_len(), steps + 2);
    assert_eq!(at(&editor, &b), [1.5, 0.0, 2.0]);
    assert!(editor.align(2, true));
    assert_eq!(at(&editor, &a)[2], 0.0);
    assert_eq!(at(&editor, &b)[2], 0.0);
    editor.level.offset = [0.0, 0.0, 2.0];
    let made = editor.duplicate();
    assert_eq!(made, ["block 4", "block 5", "block 6"]);
    assert_eq!(editor.history.undo_len(), steps + 4);
    assert_eq!(editor.history.undo_label(), Some("duplicate 3 objects"));
    assert_eq!(at(&editor, "block 5"), [1.5, 0.0, 2.0]);
    assert_eq!(editor.level.picked, made);
    screen.drive(
        &mut editor,
        &format!("click @object:{a}\nframe\nkey shift+D\nframe"),
    );
    assert!(editor.scene().object("block 7").is_some());
    for _ in 0..5 {
        editor.undo();
    }
    assert_eq!(text(&scene), before);
    editor.level.picked = vec!["ground[0,0]/block".into(), a.clone()];
    assert!(!editor.align(0, false));
    assert!(
        editor
            .log
            .entries
            .iter()
            .any(|entry| entry.text.contains("is a tile"))
    );
}
