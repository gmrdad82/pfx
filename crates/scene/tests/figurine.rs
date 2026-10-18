mod common;

use common::{Folder, MATERIALS, show};
use pfx_scene::{SceneEdit, Target};

const SKIN: &str = "roughness = 0.8\nfabric = { color = [0.9, 0.8, 0.7], roughness = 0.6 }\nanisotropy = { strength = 0.5, rotation = 1.2 }\nscatter = { color = [0.9, 0.4, 0.3], radius = 0.02, amount = 0.7 }\n";

fn figurine(name: &str, skin: &str) -> Folder {
    let folder = Folder::room(name);
    folder.write(
        "materials.toml",
        &MATERIALS.replacen("roughness = 0.8\n", skin, 1),
    );
    folder
}

#[test]
fn the_figurine_keys_read_check_and_edit() {
    let folder = figurine("figurine", SKIN);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    let grey = edit.scene().material("grey").unwrap().value.clone();
    assert_eq!(grey.fabric.unwrap().color, Some([0.9, 0.8, 0.7]));
    assert_eq!(grey.anisotropy.unwrap().rotation, Some(1.2));
    assert_eq!(grey.scatter.unwrap().radius, Some(0.02));
    let target = Target::Material("grey".into());
    edit.set(&target, &["fabric", "roughness"], 0.25f32)
        .unwrap();
    let grey = &edit.scene().material("grey").unwrap().value;
    assert_eq!(grey.fabric.unwrap().roughness, Some(0.25));
    let text = folder.read("materials.toml");
    assert!(
        text.contains("fabric = { color = [0.9, 0.8, 0.7], roughness = 0.25 }"),
        "{text}"
    );
    assert!(edit.set(&target, &["scatter", "radius"], 0.0f32).is_err());
}

#[test]
fn out_of_range_figurine_keys_are_reported() {
    let bad = SKIN
        .replace("[0.9, 0.8, 0.7]", "[1.5, 0.8, 0.7]")
        .replace("strength = 0.5", "strength = -0.1")
        .replace("rotation = 1.2", "rotation = inf")
        .replace("radius = 0.02", "radius = 0.0")
        .replace("amount = 0.7", "amount = 2.0");
    let diagnostics = figurine("figurine-bad", &bad).check();
    for key in [
        "fabric.color",
        "anisotropy.strength",
        "anisotropy.rotation",
        "scatter.radius",
        "scatter.amount",
    ] {
        assert!(
            diagnostics.iter().any(|d| d.key.ends_with(key)),
            "{key} not in {}",
            show(&diagnostics)
        );
    }
}
