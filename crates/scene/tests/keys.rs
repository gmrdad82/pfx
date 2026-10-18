mod common;

use common::{Folder, MATERIALS, PROJECT, show};
use pfx_scene::types::{KeyValue, sample};
use pfx_scene::{SceneEdit, Target, Value};

const SCENE: &str = r#"format = 1
materials = ["materials.toml"]

[mesh.block]
id = "me5h00000b"
file = "block.gltf"

[[object]]
id = "0bj00000c1"
name = "car"
mesh = "block"

[[light]]
id = "11ght0000h"
name = "headlight"
kind = "spot"
parent = "car"
intensity = 2.0
keys = { intensity = [[0.0, 2.0], [1.5, 8.0]], color = [[0.0, [1.0, 0.2, 0.1]]], cone = [[0.0, [20.0, 35.0]], [2.0, [10.0, 20.0]]] }

[[light]]
id = "11ght0000a"
name = "lamp"
position = [0.0, 2.0, 0.0]
intensity = 1.0

[[particles]]
id = "p4rt1c1e01"
name = "spray"
rate_keys = [[0.0, 0.0], [2.0, 400.0]]
keys = { speed = [[0.0, [1.0, 3.0]], [1.0, [2.0, 5.0]]], alpha = [[0.0, 1.0], [4.0, 0.0]], color = [[0.0, [0.8, 0.8, 0.9]]] }

[sun]
model = "daylight"
hour = 18.0
keys = { hour = [[0.0, 18.0], [10.0, 19.5]] }

[sky]
kind = "analytic"
intensity = 1.5
backdrop = 2.0
horizon_color = [1.0, 0.8, 0.6]
horizon_width = 12.0
horizon_amount = 0.5
keys = { intensity = [[0.0, 1.0], [5.0, 2.0]], horizon_amount = [[0.0, 0.0], [5.0, 0.6]] }

[haze]
height = 0.0
falloff = 0.08
keys = { amount = [[0.0, 0.1], [3.0, 0.4]], color = [[0.0, [1.0, 0.7, 0.4]]] }

[finish]
exposure = 1.0
keys = { exposure = [[0.0, 0.5], [2.0, 1.2]], lut_amount = [[0.0, 0.0], [2.0, 1.0]] }

[camera]
at = [0.0, 1.4, 6.0]
look_at = [0.0, 0.5, 0.0]
fstop = 2.8
keys = { fov = [[0.0, 40.0], [3.0, 30.0]], focus = [[0.0, 6.0], [3.0, 2.5]] }
"#;

fn street(name: &str, scene: &str) -> Folder {
    let folder = Folder::assets(name);
    folder.write("project.toml", PROJECT);
    folder.write("room.scene.toml", scene);
    folder.write("materials.toml", MATERIALS);
    folder
}

#[test]
fn keyed_values_read_sample_and_edit() {
    let folder = street("keys", SCENE);
    let diagnostics = folder.check();
    assert!(diagnostics.is_empty(), "{}", show(&diagnostics));
    let mut edit = SceneEdit::open(folder.path("room.scene.toml")).unwrap();
    let scene = edit.scene();
    let light = &scene.light("headlight").unwrap().value;
    let keys = light.keys.as_ref().unwrap();
    assert_eq!(keys.sample("intensity", 0.75), Some(KeyValue::Number(5.0)));
    assert_eq!(keys.sample("intensity", 9.0), Some(KeyValue::Number(8.0)));
    assert_eq!(keys.sample("cone", 1.0), Some(KeyValue::Pair([15.0, 27.5])));
    let spray = &scene.particles("spray").unwrap().value;
    let rate = spray.track("rate").unwrap();
    assert_eq!(sample(&rate, 1.0), Some(KeyValue::Number(200.0)));
    let focus = scene.camera.as_ref().unwrap().track("focus").unwrap();
    assert_eq!(focus[1], (3.0, KeyValue::Number(2.5)));
    let sky = scene.sky.as_ref().unwrap();
    assert_eq!(sky.backdrop, Some(2.0));
    assert_eq!(
        sky.track("horizon_amount").unwrap()[1].1,
        KeyValue::Number(0.6)
    );
    let lamp = Target::Light("lamp".into());
    let track = Value::Array(vec![Value::from([0.0f32, 1.0]), Value::from([2.0f32, 4.0])]);
    edit.set(&lamp, &["keys", "intensity"], track).unwrap();
    edit.set(&Target::Sun, &["keys", "hour", "1", "1"], 20.0f32)
        .unwrap();
    let text = folder.read("room.scene.toml");
    assert!(
        text.contains("intensity = 1.0\nkeys = { intensity = [[0.0, 1.0], [2.0, 4.0]] }"),
        "{text}"
    );
    assert!(
        text.contains("keys = { hour = [[0.0, 18.0], [10.0, 20.0]] }"),
        "{text}"
    );
    let lamp = &edit.scene().light("lamp").unwrap().value;
    assert_eq!(lamp.track("intensity").unwrap().len(), 2);
    assert!(
        edit.set(
            &Target::Light("lamp".into()),
            &["keys", "range", "0"],
            Value::from([0.0f32, -1.0])
        )
        .is_err()
    );
}

#[test]
fn bad_keys_are_refused_where_they_stand() {
    let bad = SCENE
        .replace("[1.5, 8.0]", "[0.0, 8.0]")
        .replace("color = [[0.0, [1.0, 0.2, 0.1]]]", "color = [[0.0, 0.5]]")
        .replace("alpha = [[0.0, 1.0], [4.0, 0.0]]", "alpha = [[0.0, 1.5]]")
        .replace("keys = { hour", "keys = { day = [[0.0, 3.0]], hour")
        .replace("[0.0, 0.0], [5.0, 0.6]", "[0.0, 0.0], [5.0, 1.6]")
        .replace("keys = { speed", "keys = { rate = [[0.0, 1.0]], speed")
        .replace("[3.0, 30.0]", "[3.0, 180.0]");
    let folder = street("keys-bad", &bad);
    let diagnostics = folder.check();
    for (key, line) in [
        ("light.11ght0000h.keys.intensity.1.0", 19),
        ("light.11ght0000h.keys.color.0.1", 19),
        ("particles.p4rt1c1e01.keys.alpha.0.1", 31),
        ("particles.p4rt1c1e01.rate_keys", 30),
        ("sun.keys.day", 36),
        ("sky.keys.horizon_amount.1.1", 45),
        ("camera.keys.fov.1.1", 60),
    ] {
        assert!(
            diagnostics.iter().any(|d| d.key == key && d.line == line),
            "{key} at line {line} not in {}",
            show(&diagnostics)
        );
    }
}
