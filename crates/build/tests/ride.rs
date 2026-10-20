mod common;

use common::cube;
use glam::{DQuat, DVec3};
use pfx_build::{Built, Call, Generated, Registry, build_with};
use pfx_recipe::parse;
use serde_json::{Map, Value, json};
use std::path::Path;

fn fixed(call: &Call) -> Result<Generated, String> {
    let Value::Object(facts) = json!({ "spin": false }) else {
        return Ok(Generated::default());
    };
    Ok(Generated {
        pieces: vec![cube(&call.object.name, "fake")],
        facts,
        ..Generated::default()
    })
}

fn free(call: &Call) -> Result<Generated, String> {
    Ok(Generated {
        pieces: vec![cube(&call.object.name, "fake")],
        facts: Map::new(),
        ..Generated::default()
    })
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register_with_facts("test/fixed", fixed);
    registry.register_with_facts("test/free", free);
    registry
}

const CURVED: &str = "[[0.0, 0.0, 0.0], [0.0, 0.0, 20.0], [15.0, 0.0, 40.0], [30.0, 0.0, 45.0]]";

fn scene(extra: &str) -> Built {
    let text = format!(
        "recipe = 1\nname = \"t\"\nseed = 3\n\n[timeline]\nfps = 30\nframes = 30\n\n[[route]]\nname = \"main\"\npoints = {CURVED}\n\n[[object]]\nname = \"car\"\nkind = \"box\"\nsize = [1.8, 0.6, 4.0]\nat = [0.0, 0.5, 0.0]\n\n[[follow]]\nobject = \"car\"\nroute = \"main\"\nspeed = [[0.0, 10.0]]\n{extra}"
    );
    let recipe = parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    build_with(&recipe, Path::new("."), &registry()).unwrap()
}

fn keys<'a>(b: &'a Built, node: &str) -> &'a [pfx_motion::Trs] {
    &b.tracks
        .iter()
        .find(|t| t.node == node)
        .unwrap_or_else(|| panic!("no track for {node}"))
        .keys
}

fn turns(b: &Built, node: &str) -> Vec<DQuat> {
    keys(b, node)
        .iter()
        .map(|k| DQuat::from_array(k.rotation))
        .collect()
}

fn spots(b: &Built, node: &str) -> Vec<DVec3> {
    keys(b, node)
        .iter()
        .map(|k| DVec3::from(k.translation))
        .collect()
}

const WHEEL: &str = "\n[[object]]\nname = \"car.wheel_fl\"\nkind = \"generator\"\ngenerator = \"pfx/wheel\"\nparent = \"car\"\nat = [0.8, -0.16, 1.3]\n";

#[test]
fn an_undotted_child_of_the_followed_object_rides_its_pose() {
    let b = scene(
        "\n[[object]]\nname = \"mirror\"\nkind = \"box\"\nsize = [0.1, 0.1, 0.1]\nparent = \"car\"\nat = [0.9, 0.6, 0.5]\n\n[[object]]\nname = \"stalk\"\nkind = \"box\"\nsize = [0.05, 0.05, 0.05]\nparent = \"mirror\"\nat = [0.0, 0.1, 0.0]\n",
    );
    let car_turn = turns(&b, "car");
    let car_at = spots(&b, "car");
    let mirror_at = spots(&b, "mirror");
    let stalk_turn = turns(&b, "stalk");
    assert_eq!(mirror_at.len(), car_turn.len());
    for n in [0, 10, 20, 30] {
        let want = car_at[n] + car_turn[n] * DVec3::new(0.9, 0.6, 0.5);
        assert!((mirror_at[n] - want).length() < 1e-9, "frame {n}");
        assert!(turns(&b, "mirror")[n].angle_between(car_turn[n]) < 1e-6);
        assert!(stalk_turn[n].angle_between(car_turn[n]) < 1e-6);
    }
    assert!((mirror_at[30] - mirror_at[0]).length() > 1.0);
}

#[test]
fn a_child_with_its_own_follow_is_left_to_it() {
    let b = scene(
        "\n[[object]]\nname = \"drone\"\nkind = \"box\"\nsize = [0.1, 0.1, 0.1]\nparent = \"car\"\n\n[[follow]]\nobject = \"drone\"\nroute = \"main\"\nspeed = [[0.0, 5.0]]\n",
    );
    assert_eq!(b.tracks.iter().filter(|t| t.node == "drone").count(), 1);
}

#[test]
fn a_wheels_children_steer_with_it_and_spin_unless_their_facts_say_not() {
    let b = scene(&format!(
        "{WHEEL}\n[[object]]\nname = \"car.wheel_fl.cap\"\nkind = \"generator\"\ngenerator = \"test/free\"\nparent = \"car.wheel_fl\"\nat = [0.1, 0.0, 0.0]\n\n[[object]]\nname = \"car.wheel_fl.caliper\"\nkind = \"generator\"\ngenerator = \"test/fixed\"\nparent = \"car.wheel_fl\"\nat = [0.0, 0.0, 0.2]\n\n[[object]]\nname = \"clip\"\nkind = \"generator\"\ngenerator = \"test/fixed\"\nparent = \"car.wheel_fl.caliper\"\nat = [0.0, 0.05, 0.0]\n"
    ));
    let car = turns(&b, "car");
    let tyre = turns(&b, "car.wheel_fl.tyre");
    let brake = turns(&b, "car.wheel_fl.brake");
    let cap = turns(&b, "car.wheel_fl.cap");
    let caliper = turns(&b, "car.wheel_fl.caliper");
    let steered = (0..car.len()).any(|n| {
        let (axis, angle) = (car[n].inverse() * caliper[n]).to_axis_angle();
        angle > 0.01 && axis.y.abs() > 0.999
    });
    assert!(steered);
    for n in 0..car.len() {
        assert!(cap[n].angle_between(tyre[n]) < 1e-6, "cap spins, frame {n}");
        assert!(
            caliper[n].angle_between(brake[n]) < 1e-6,
            "caliper only steers, frame {n}"
        );
    }
    assert!(cap[15].angle_between(caliper[15]) > 0.1);
    let hub = spots(&b, "car.wheel_fl.tyre");
    let cap_at = spots(&b, "car.wheel_fl.cap");
    for n in [0, 15, 30] {
        let spun = (cap_at[n] - hub[n]).length();
        assert!((spun - 0.1).abs() < 1e-6, "frame {n}: {spun}");
    }
}

#[test]
fn a_wheels_children_do_not_move_the_wheelbase() {
    let plain = scene(WHEEL);
    let with = scene(&format!(
        "{WHEEL}\n[[object]]\nname = \"car.wheel_fl.cap\"\nkind = \"generator\"\ngenerator = \"test/free\"\nparent = \"car.wheel_fl\"\n"
    ));
    assert_eq!(
        turns(&plain, "car.wheel_fl.tyre"),
        turns(&with, "car.wheel_fl.tyre")
    );
}

fn two_node_wheel() -> Vec<u8> {
    let quad = |half: f32| -> Vec<u8> {
        [
            0.0, -half, -half, 0.0, -half, half, 0.0, half, half, 0.0, half, -half,
        ]
        .iter()
        .flat_map(|v: &f32| v.to_le_bytes())
        .collect()
    };
    let indices: Vec<u8> = [0u16, 1, 2, 0, 2, 3]
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let mut bin = quad(0.3);
    bin.extend(quad(0.05));
    bin.extend(&indices);
    let accessor = |view: usize, half: f32| {
        json!({ "bufferView": view, "componentType": 5126, "count": 4, "type": "VEC3",
                "min": [0.0, -half, -half], "max": [0.0, half, half] })
    };
    let document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0, 1] }],
        "nodes": [
            { "name": "Tyre", "mesh": 0 },
            { "name": "Caliper", "mesh": 1, "translation": [0.0, 0.2, 0.1] },
        ],
        "meshes": [
            { "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 2 }] },
            { "primitives": [{ "attributes": { "POSITION": 1 }, "indices": 2 }] },
        ],
        "accessors": [
            accessor(0, 0.3),
            accessor(1, 0.05),
            { "bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR" },
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 48 },
            { "buffer": 0, "byteOffset": 48, "byteLength": 48 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 12 },
        ],
        "buffers": [{ "byteLength": 108 }],
    });
    pfx_build::library::join_glb(&document, &bin)
}

fn mesh_wheel(name: &str, pieces: &str) -> Result<Built, pfx_build::Error> {
    let dir = common::scratch(name);
    std::fs::write(dir.join("wheel.glb"), two_node_wheel()).unwrap();
    let text = format!(
        "recipe = 1\nname = \"t\"\nseed = 3\n\n[timeline]\nfps = 30\nframes = 30\n\n[[route]]\nname = \"main\"\npoints = {CURVED}\n\n[[object]]\nname = \"car\"\nkind = \"box\"\nsize = [1.8, 0.6, 4.0]\nat = [0.0, 0.5, 0.0]\n\n[[object]]\nname = \"car.wheel_fl\"\nkind = \"mesh\"\npath = \"wheel.glb\"\nparent = \"car\"\nat = [0.8, -0.16, 1.3]\npieces = {{ {pieces} }}\n\n[[follow]]\nobject = \"car\"\nroute = \"main\"\nspeed = [[0.0, 10.0]]\n"
    );
    let recipe = parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    build_with(&recipe, &dir, &registry())
}

#[test]
fn listed_nodes_of_an_imported_wheel_follow_and_steer_but_do_not_spin() {
    let b = mesh_wheel("ride-still-nodes", "Caliper = { spin = false }").unwrap();
    let car = turns(&b, "car");
    let tyre = turns(&b, "car.wheel_fl");
    let caliper = turns(&b, "car.wheel_fl.Caliper");
    let tyre_at = spots(&b, "car.wheel_fl");
    let caliper_at = spots(&b, "car.wheel_fl.Caliper");
    for n in [10, 25] {
        let travelled = 10.0 * n as f64 / 30.0;
        let spun = caliper[n].inverse() * tyre[n];
        let want = DQuat::from_axis_angle(DVec3::X, travelled / 0.3);
        assert!(spun.angle_between(want) < 1e-4, "tyre spins, frame {n}");
        let (axis, angle) = (car[n].inverse() * caliper[n]).to_axis_angle();
        assert!(
            angle < 1e-6 || axis.y.abs() > 0.999,
            "caliper only steers, frame {n}"
        );
        assert!((tyre_at[n] - caliper_at[n]).length() < 1e-6, "frame {n}");
    }
    assert!((caliper_at[25] - caliper_at[10]).length() > 1.0);
    let plain = mesh_wheel("ride-still-nodes-plain", "Tyre = { hidden = false }").unwrap();
    assert!(
        !plain
            .tracks
            .iter()
            .any(|t| t.node == "car.wheel_fl.Caliper")
    );
}

#[test]
fn a_still_node_the_wheel_lacks_is_refused_with_its_nodes() {
    let Err(e) = mesh_wheel("ride-still-nodes-missing", "Brake = { spin = false }") else {
        panic!("built");
    };
    assert_eq!(
        e.to_string(),
        "object[car.wheel_fl].pieces.Brake: wheel.glb has no node `Brake`, expected a node name, a parent/child path or #<index> of: Tyre, Caliper"
    );
}
