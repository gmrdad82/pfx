use std::path::Path;

use pfx_editor_doc::toml_edit::{Item, Table, value};
use pfx_editor_doc::{Edit, Files, History, Op, TomlPatch, edit_doc, parse_path};

const COMMENTED: &str = include_str!("data/commented.toml");
const FULL: &str = include_str!("data/full.toml");
const STAND: &str = include_str!("data/stand.scene.toml");

fn lines_touched(before: &str, after: &str) -> usize {
    let a: Vec<&str> = before.split_inclusive('\n').collect();
    let b: Vec<&str> = after.split_inclusive('\n').collect();
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - head;
    let tail = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    a.len().max(b.len()) - head - tail
}

fn table(pairs: &[(&str, &str)]) -> Item {
    let mut t = Table::new();
    for (key, text) in pairs {
        t.insert(key, value(*text));
    }
    Item::Table(t)
}

fn set(path: &str, item: Item) -> TomlPatch {
    TomlPatch::set("sample.toml", parse_path(path).unwrap(), item)
}

fn cases(text: &str) -> Vec<(TomlPatch, usize)> {
    let p = |s: &str| parse_path(s).unwrap();
    if text == COMMENTED {
        vec![
            (set("object[crate].at[1]", value(0.5)), 1),
            (set("object[mv_knob].radius", value(0.02)), 1),
            (set("object[crate].bevel.width", value(0.005)), 1),
            (set("plate.width", value(120)), 1),
            (set("seed", value(9)), 1),
            (set("object[crate].name", value("box")), 1),
            (set("camera[main].fov_y_deg", value(25.0)), 1),
            (
                TomlPatch::unset("sample.toml", p("object[lid].materials")),
                1,
            ),
            (
                TomlPatch::push("sample.toml", p("object[lid].materials"), value("paint")),
                1,
            ),
            (
                TomlPatch::insert("sample.toml", p("object[lid].materials[0]"), value("wood")),
                1,
            ),
            (TomlPatch::remove("sample.toml", p("object[mv_knob]")), 9),
            (
                TomlPatch::push(
                    "sample.toml",
                    p("object"),
                    table(&[("name", "new"), ("kind", "box")]),
                ),
                4,
            ),
        ]
    } else if text == FULL {
        vec![
            (set("variant.bare.set.\"sun.irradiance\"", value(0.5)), 1),
            (set("sky.shade.hours.6[2]", value(0.75)), 1),
            (set("object[slab].outline.union[1].segments", value(64)), 1),
            (set("object[note].spans[1].italic", value(false)), 1),
            (set("object[drum].materials[1]", value("tin")), 1),
            (set("sky.room.lights[key].power", value(80.0)), 1),
            (set("bake.volume[0].spacing", value(0.5)), 1),
            (set("materials.oak.roughness", value(0.6)), 1),
            (set("output.hierarchy", value(false)), 1),
            (TomlPatch::unset("sample.toml", p("look.grade.haze")), 1),
            (TomlPatch::remove("sample.toml", p("light[fill]")), 5),
            (TomlPatch::remove("sample.toml", p("camera[1]")), 7),
            (
                TomlPatch::push("sample.toml", p("mover[1].parts"), value("pill")),
                1,
            ),
        ]
    } else {
        vec![
            (set("object[ball].at[1]", value(1.0)), 1),
            (set("object[plinth].scale", value(0.6)), 1),
            (set("mesh.ball.file", value("orb.gltf")), 1),
            (set("camera.fov", value(45.0)), 1),
            (set("fallback", value("clay")), 1),
            (
                TomlPatch::push("sample.toml", p("materials"), value("extra.toml")),
                1,
            ),
            (
                TomlPatch::unset("sample.toml", p("object[crate].rotate")),
                1,
            ),
        ]
    }
}

#[test]
fn every_sample_round_trips_through_its_document() {
    for text in [COMMENTED, FULL, STAND] {
        let mut files = Files::new();
        files.open("sample.toml", text);
        let doc = files
            .doc(Path::new("sample.toml"))
            .expect("the sample parses");
        assert_eq!(doc.to_string(), text);
    }
}

#[test]
fn a_patch_touches_only_its_own_lines_and_its_inverse_restores_the_file() {
    for text in [COMMENTED, FULL, STAND] {
        for (patch, most) in cases(text) {
            let label = patch.label().to_string();
            let mut files = Files::new();
            files.open("sample.toml", text);
            let mut history = History::default();
            let changed = history
                .apply(&mut files, patch)
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(changed.files.len(), 1, "{label}");
            let after = files.text(Path::new("sample.toml")).unwrap().to_string();
            let touched = lines_touched(text, &after);
            assert!(
                touched >= 1 && touched <= most,
                "{label}: {touched} lines\n{after}"
            );
            history.undo(&mut files).unwrap().unwrap();
            assert_eq!(
                files.text(Path::new("sample.toml")).unwrap(),
                text,
                "{label}"
            );
            history.redo(&mut files).unwrap().unwrap();
            assert_eq!(
                files.text(Path::new("sample.toml")).unwrap(),
                after,
                "{label}"
            );
        }
    }
}

#[test]
fn a_value_sets_own_inverse_restores_the_text_without_the_exact_copy() {
    for text in [COMMENTED, FULL, STAND] {
        for (patch, _) in cases(text) {
            let Some(op @ Op::Set { .. }) = patch.ops().first() else {
                continue;
            };
            let mut doc = text
                .parse::<pfx_editor_doc::toml_edit::DocumentMut>()
                .unwrap();
            let back = edit_doc(&mut doc, op).unwrap();
            assert_ne!(doc.to_string(), text);
            edit_doc(&mut doc, &back).unwrap();
            assert_eq!(doc.to_string(), text, "{}", patch.label());
        }
    }
}

#[test]
fn every_patch_then_undo_all_restores_each_sample() {
    for text in [COMMENTED, FULL, STAND] {
        let mut files = Files::new();
        files.open("sample.toml", text);
        let mut history = History::default();
        let mut applied = 0;
        for (patch, _) in cases(text) {
            if history.apply(&mut files, patch).is_ok() {
                applied += 1;
            }
        }
        assert!(applied >= 5);
        while history.undo(&mut files).unwrap().is_some() {}
        assert_eq!(files.text(Path::new("sample.toml")).unwrap(), text);
    }
}
