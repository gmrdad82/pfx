use std::fs;

use pfx_load::scene::{Target, Value, held_text, pack};

use crate::editor::{Editor, Group};
use crate::inspect::Change;
use crate::tests::scene_copy;

#[test]
fn the_editor_edits_a_pfx_scene_in_place_and_undoes_to_the_byte() {
    let toml = scene_copy("pfx-source");
    let folder = toml.with_file_name("packed");
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("room.pfx");
    pack(&toml, &file).unwrap();
    let materials = file.join("materials.toml");
    let before = held_text(&materials).unwrap().unwrap();
    let packed = fs::read(&file).unwrap();

    let mut editor = Editor::open(&file).unwrap();
    editor.change(
        Change {
            target: Target::Material("clay".into()),
            path: vec!["roughness".into()],
            value: Some(Value::Float(0.5)),
        },
        Group::Keep,
    );
    let after = held_text(&materials).unwrap().unwrap();
    assert_eq!(after, before.replace("roughness = 0.9", "roughness = 0.5"));
    assert_eq!(editor.scene().library.get("clay").unwrap().roughness, 0.5);
    assert_eq!(editor.saves, vec![materials.clone()]);
    editor.undo();
    assert_eq!(held_text(&materials).unwrap().unwrap(), before);
    assert_eq!(fs::read(&file).unwrap(), packed);
    editor.reloaded();
    assert_eq!(editor.scene().library.get("clay").unwrap().roughness, 0.9);
}
