use std::path::{Path, PathBuf};

use pfx_bench::{Plan, actions, run_plan};
use pfx_input::{Actions, Input, InputEvent, Key};
use serde_json::Value;

const SCENE: &str = r#"
include = ["room.scene.toml"]

[[object]]
name = "ball"
mesh = "block"
at = [0.3, 1.8, 0.7]
scale = 0.35
material = "blue"

[body.ball]
shape = "sphere"

[[mover]]
name = "spin"
objects = ["right pillar"]
kind = "turn"
pivot = [1.4, 0.0, -0.6]
axis = [0.0, 1.0, 0.0]
travel = [0.0, 90.0]
period = 3.0
"#;

fn folder() -> PathBuf {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/bench-gpu");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    for entry in std::fs::read_dir(fixtures).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
    }
    let room = folder.join("room.scene.toml");
    let text = std::fs::read_to_string(&room).unwrap();
    let floor = "material = \"grey\"\nid = \"5wgxc2j4q7\"\n";
    assert!(text.contains(floor));
    std::fs::write(
        &room,
        text.replacen(
            floor,
            &format!("{floor}\n[object.body]\nkind = \"fixed\"\n"),
            1,
        ),
    )
    .unwrap();
    std::fs::write(folder.join("bench.scene.toml"), SCENE).unwrap();
    folder
}

fn columns(path: &Path) -> Vec<(u64, String, Value)> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line.get("end").is_none())
        .map(|line| {
            (
                line["tick"].as_u64().unwrap(),
                line["kind"].as_str().unwrap().to_string(),
                line["size"].clone(),
            )
        })
        .collect()
}

fn plan(folder: &Path, name: &str) -> Plan {
    Plan {
        scene: folder.join("bench.scene.toml"),
        seconds: 2.0,
        camera: None,
        replay: None,
        size: Some((160, 96)),
        stats: Some(folder.join(format!("{name}.jsonl"))),
        seed: 5,
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn two_benches_of_the_same_scene_give_the_same_ticks_inputs_and_frame_kinds() {
    let folder = folder();
    let none = || Err("no caller tree".to_string());
    let one = run_plan(&plan(&folder, "one"), none).unwrap();
    let two = run_plan(&plan(&folder, "two"), none).unwrap();
    assert_eq!(one.summary.frames, 120);
    assert_eq!(one.summary.frames, two.summary.frames);
    assert_eq!(one.ticks, two.ticks);
    assert_eq!(one.inputs, two.inputs);
    assert_eq!(one.camera, two.camera);
    assert_eq!(columns(&one.stats), columns(&two.stats));
    let lines = columns(&one.stats);
    assert_eq!(lines.len(), 120);
    assert!(
        lines
            .iter()
            .all(|(_, kind, size)| kind == "full" && size == &serde_json::json!([160, 96]))
    );
    assert!(
        lines
            .iter()
            .enumerate()
            .all(|(at, line)| line.0 == at as u64)
    );
    assert!(one.summary.submit.is_some() && one.summary.sim.is_some());
    let summary: Value = serde_json::from_str(&one.line).unwrap();
    assert_eq!(summary["frames"], 120);
    assert_eq!(summary["size"], serde_json::json!([160, 96]));
    assert_eq!(summary["stats"], one.stats.to_string_lossy().as_ref());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_replayed_recording_moves_the_benched_camera() {
    let folder = folder();
    let mut input = Input::new(Actions::new(actions()).unwrap(), 16_667);
    input.start_recording();
    for step in 0..120 {
        if step == 10 {
            input.feed(InputEvent::Key {
                key: Key::D,
                pressed: true,
            });
        }
        if step == 70 {
            input.feed(InputEvent::Key {
                key: Key::D,
                pressed: false,
            });
        }
        input.step();
    }
    let recording = input.stop_recording().unwrap();
    std::fs::write(
        folder.join("run.json"),
        serde_json::to_string(&recording).unwrap(),
    )
    .unwrap();
    let none = || Err("no caller tree".to_string());
    let still = run_plan(&plan(&folder, "still"), none).unwrap();
    let moved = run_plan(
        &Plan {
            replay: Some(folder.join("run.json")),
            ..plan(&folder, "moved")
        },
        none,
    )
    .unwrap();
    assert_ne!(still.camera, moved.camera);
    assert_eq!(still.inputs, 0);
    assert_eq!(moved.inputs, 2);
    assert_eq!(still.ticks, moved.ticks);
    assert_eq!(columns(&still.stats), columns(&moved.stats));
}
