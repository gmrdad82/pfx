use std::path::{Path, PathBuf};

use pfx_recipe::{
    Kind, apply_set, load, parse, reread, save, set_values, set_values_with, value_of,
};

const RECIPE: &str = r#"recipe = 1
name = "t"

[sky]
kind = "analytic"

[materials.red]
base = [1.0, 0.0, 0.0]

[materials.blue]
base = [0.0, 0.0, 1.0]

[[object]]
name = "car"
kind = "box"
size = [1.0, 1.0, 1.0]

[[object]]
name = "car.shell"
kind = "box"
size = [1.0, 1.0, 1.0]
material = "red"
"#;

fn sets(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn material_of(r: &pfx_recipe::Recipe, name: &str) -> Option<String> {
    r.objects
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| o.material.clone())
}

#[test]
fn quoted_and_bracketed_segments_reach_a_dotted_object_name() {
    let mut l = reread(Path::new("t.toml"), RECIPE).unwrap();
    for (set, want) in [
        ("object.\"car.shell\".material=blue", "blue"),
        ("object[car.shell].material=red", "red"),
        ("object[car.shell].material=\"blue\"", "blue"),
    ] {
        let changes = set_values(&mut l, &sets(&[set])).unwrap();
        assert_eq!(changes.len(), 1, "{set}");
        assert_eq!(
            material_of(&l.recipe, "car.shell").as_deref(),
            Some(want),
            "{set}"
        );
        assert_eq!(material_of(&l.recipe, "car"), None, "{set}");
    }
    for key in [
        "object.\"car.shell\".material",
        "object[car.shell].material",
    ] {
        assert_eq!(value_of(&l, key).unwrap().as_str(), Some("blue"), "{key}");
    }
    let recipe = parse(RECIPE).unwrap();
    for set in [
        "object.\"car.shell\".material=blue",
        "object[car.shell].material=blue",
    ] {
        let set = apply_set(&recipe, &sets(&[set])).unwrap();
        assert_eq!(material_of(&set, "car.shell").as_deref(), Some("blue"));
    }
    for bad in [
        "object.\"car.shell.material=blue",
        "object[car.shell.material=blue",
        "object[].material=blue",
        "object..material=blue",
        "object.\"\".material=blue",
    ] {
        assert!(set_values(&mut l, &sets(&[bad])).is_err(), "{bad}");
    }
}

#[test]
fn every_key_takes_a_string_value_the_same_way() {
    for set in [
        "sky.kind=analytic",
        "sky.kind=\"analytic\"",
        "sky.kind= analytic ",
    ] {
        let mut l = reread(Path::new("t.toml"), RECIPE).unwrap();
        set_values(&mut l, &sets(&[set])).unwrap_or_else(|e| panic!("{set}: {e}"));
        assert_eq!(
            value_of(&l, "sky.kind").unwrap().as_str(),
            Some("analytic"),
            "{set}"
        );
        let set = apply_set(&parse(RECIPE).unwrap(), &sets(&[set.trim()]));
        assert!(set.is_ok(), "{set:?}");
    }
}

fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/recipe-tests/names")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    for (file, text) in files {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

const RIG: &str = r#"[sky]
kind = "analytic"
intensity = 1.0

[[light]]
name = "lamp"
position = [0.0, 1.0, 0.0]
intensity = 1.0

[materials.tile]
base = [0.5, 0.5, 0.5]

[[object]]
name = "board"
kind = "box"
size = [1.0, 1.0, 1.0]
material = "tile"
"#;

const SCENE: &str = "recipe = 1\nname = \"s\"\ninclude = [\"rig.toml\"]\n";

fn text(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap()
}

#[test]
fn a_set_writes_an_override_into_the_document_it_was_given() {
    let dir = scratch("over", &[("rig.toml", RIG), ("scene.toml", SCENE)]);
    let mut l = load(&dir.join("scene.toml")).unwrap();
    let changes = set_values(
        &mut l,
        &sets(&[
            "light.lamp.intensity=5.0",
            "materials.tile.base=[0.2, 0.2, 0.2]",
            "sky.intensity=2.0",
            "object.board.size=[2.0, 2.0, 2.0]",
        ]),
    )
    .unwrap();
    assert!(
        changes.iter().all(|c| c.file == "scene.toml"),
        "{changes:?}"
    );
    save(&l).unwrap();
    assert_eq!(text(&dir, "rig.toml"), RIG);
    let again = load(&dir.join("scene.toml")).unwrap().recipe;
    assert_eq!(again.lights.len(), 1);
    assert_eq!(again.lights[0].intensity, 5.0);
    assert_eq!(
        again.materials["tile"]["base"],
        serde_json::json!([0.2, 0.2, 0.2])
    );
    assert_eq!(again.sky.as_ref().unwrap().intensity, 2.0);
    assert_eq!(again.objects.len(), 1);
    assert!(matches!(again.objects[0].kind, Kind::Box { size, .. } if size == [2.0, 2.0, 2.0]));
    let own = text(&dir, "scene.toml");
    assert!(own.contains("name = \"lamp\"") && own.contains("[materials.tile]"));
}

#[test]
fn through_writes_into_the_included_file() {
    let dir = scratch("through", &[("rig.toml", RIG), ("scene.toml", SCENE)]);
    let mut l = load(&dir.join("scene.toml")).unwrap();
    let changes = set_values_with(
        &mut l,
        &sets(&[
            "light.lamp.intensity=5.0",
            "materials.tile.base=[0.2, 0.2, 0.2]",
        ]),
        true,
    )
    .unwrap();
    assert!(changes.iter().all(|c| c.file == "rig.toml"), "{changes:?}");
    save(&l).unwrap();
    assert_eq!(text(&dir, "scene.toml"), SCENE);
    let rig = text(&dir, "rig.toml");
    assert!(rig.contains("intensity = 5.0") && rig.contains("[0.2, 0.2, 0.2]"));
}
