use std::path::{Path, PathBuf};

use pfx_input::{Actions, Input, InputEvent, Key, Recording};
use pfx_load::scene::Scene;
use pfx_play::{Options, PlaySession};

use crate::game::{BenchGame, actions};
use crate::{Plan, Spread, parse, run_plan, summarize};

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

[[object]]
name = "eye"
mesh = "block"
at = [0.0, 1.2, 3.0]
scale = 0.01
hidden = true
"#;

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

pub fn folder(name: &str) -> PathBuf {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/bench-tests")
        .join(name);
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

pub fn recording(held: &[(usize, usize, Key)], steps: usize) -> Recording {
    let mut input = Input::new(Actions::new(actions()).unwrap(), 16_667);
    input.start_recording();
    for step in 0..steps {
        for &(from, to, key) in held {
            if step == from {
                input.feed(InputEvent::Key { key, pressed: true });
            }
            if step == to {
                input.feed(InputEvent::Key {
                    key,
                    pressed: false,
                });
            }
        }
        input.step();
    }
    input.stop_recording().unwrap()
}

fn drive(
    scene: &Scene,
    camera: Option<&str>,
    replay: Option<Recording>,
    frames: u32,
) -> PlaySession {
    let mut session = PlaySession::play_with(
        scene,
        None,
        Box::new(BenchGame::new(camera.map(str::to_string))),
        Options {
            seed: 3,
            replay,
            ..Options::default()
        },
    )
    .unwrap();
    for _ in 0..frames {
        session.advance(1.0 / 60.0);
    }
    session
}

fn camera(session: &PlaySession) -> ([f32; 3], [f32; 3]) {
    (session.world().camera.at, session.world().camera.look_at)
}

#[test]
fn the_command_takes_a_scene_and_seconds_and_refuses_the_rest() {
    let plan = parse(&words(
        "room.scene.toml --seconds 2.5 --camera eye --replay run.json --size 1280x720 --stats out.jsonl --seed 7",
    ))
    .unwrap();
    assert_eq!(
        plan,
        Plan {
            scene: "room.scene.toml".into(),
            seconds: 2.5,
            camera: Some("eye".into()),
            replay: Some("run.json".into()),
            size: Some((1280, 720)),
            stats: Some("out.jsonl".into()),
            seed: 7,
        }
    );
    let plain = parse(&words("room.scene.toml --seconds=1")).unwrap();
    assert_eq!((plain.size, plain.stats, plain.seed), (None, None, 0));
    for bad in [
        "--seconds 2",
        "room.scene.toml",
        "room.scene.toml --seconds 0",
        "room.scene.toml --seconds soon",
        "room.scene.toml --seconds 2 --size 1280",
        "room.scene.toml --seconds 2 --size 0x720",
        "room.scene.toml --seconds 2 --seed -1",
        "room.scene.toml --seconds 2 --colour red",
        "room.scene.toml other.toml --seconds 2",
        "room.scene.toml --seconds",
    ] {
        assert!(parse(&words(bad)).is_err(), "{bad}");
    }
}

#[test]
fn a_missing_scene_recording_or_camera_is_refused_before_the_gpu() {
    let folder = folder("refusals");
    let scene = folder.join("bench.scene.toml");
    let plan = |scene: &Path| Plan {
        scene: scene.to_path_buf(),
        seconds: 1.0,
        camera: None,
        replay: None,
        size: Some((64, 36)),
        stats: Some(folder.join("refused.jsonl")),
        seed: 0,
    };
    let none = || Err("the gpu was not wanted".to_string());

    let error = run_plan(&plan(&folder.join("absent.scene.toml")), none).unwrap_err();
    assert!(error.contains("no such scene file"), "{error}");

    let error = run_plan(
        &Plan {
            replay: Some(folder.join("absent.json")),
            ..plan(&scene)
        },
        none,
    )
    .unwrap_err();
    assert!(error.contains("no such recording"), "{error}");

    std::fs::write(folder.join("broken.json"), "{}").unwrap();
    let error = run_plan(
        &Plan {
            replay: Some(folder.join("broken.json")),
            ..plan(&scene)
        },
        none,
    )
    .unwrap_err();
    assert!(error.contains("not a recording"), "{error}");

    let error = run_plan(
        &Plan {
            camera: Some("nowhere".into()),
            ..plan(&scene)
        },
        none,
    )
    .unwrap_err();
    assert!(error.contains("no object named \"nowhere\""), "{error}");
    assert!(!folder.join("refused.jsonl").exists());
}

#[test]
fn without_a_recording_the_camera_stays_where_the_scene_put_it() {
    let folder = folder("fixed");
    let scene = Scene::open(folder.join("bench.scene.toml")).unwrap();
    let rest = scene.camera_or_default();
    let session = drive(&scene, None, None, 60);
    assert_eq!(camera(&session).0, rest.at);
    assert_eq!(session.world().ticks(), 60);
}

#[test]
fn a_named_object_stands_the_camera() {
    let folder = folder("named");
    let scene = Scene::open(folder.join("bench.scene.toml")).unwrap();
    let session = drive(&scene, Some("eye"), None, 4);
    let (at, look_at) = camera(&session);
    assert_eq!(at, [0.0, 1.2, 3.0]);
    assert!(look_at[2] < at[2], "looks down -Z: {look_at:?}");
}

#[test]
fn a_replayed_recording_drives_the_camera_and_the_same_recording_drives_it_the_same() {
    let folder = folder("replay");
    let scene = Scene::open(folder.join("bench.scene.toml")).unwrap();
    let rest = scene.camera_or_default();
    let held = [(5, 35, Key::W), (10, 25, Key::Right)];
    let one = drive(&scene, None, Some(recording(&held, 60)), 60);
    let two = drive(&scene, None, Some(recording(&held, 60)), 60);
    let (at, look_at) = camera(&one);
    assert_ne!(at, rest.at);
    assert_ne!(look_at, rest.look_at);
    assert_eq!(camera(&one), camera(&two));
    assert_eq!(one.world().ticks(), two.world().ticks());
}

#[test]
fn a_recording_that_holds_nothing_leaves_the_camera_alone() {
    let folder = folder("idle");
    let scene = Scene::open(folder.join("bench.scene.toml")).unwrap();
    let rest = scene.camera_or_default();
    let session = drive(&scene, None, Some(recording(&[], 30)), 30);
    assert_eq!(camera(&session), (rest.at, rest.look_at));
}

#[test]
fn spreads_use_the_nearest_rank() {
    let values: Vec<f64> = (1..=100).map(f64::from).collect();
    let spread = Spread::of(&values).unwrap();
    assert_eq!(
        (spread.p50, spread.p95, spread.p99, spread.max),
        (50.0, 95.0, 99.0, 100.0)
    );
    assert!(Spread::of(&[]).is_none());
}

#[test]
fn the_summary_reads_the_stats_lines_after_the_first_frame() {
    let line = |tick: u32, sim: &str, submit: f64, gpu: &str| {
        format!(
            "{{\"schema_version\":1,\"tick\":{tick},\"t_ms\":0.0,\"sim_ms\":{sim},\"submit_ms\":{submit},\"gpu_ms\":{gpu},\"gpu_tick\":null,\"present_interval_ms\":null,\"kind\":\"full\",\"size\":[8,8],\"device\":null,\"hud_ms\":null}}\n"
        )
    };
    let text = [
        line(0, "0.1", 300.0, "null"),
        line(1, "0.2", 2.0, "1.5"),
        line(2, "null", 3.0, "20.0"),
        line(3, "0.4", 17.0, "null"),
        "{\"schema_version\":1,\"end\":true,\"frames\":4,\"dropped\":1}\n".to_string(),
    ]
    .concat();
    let summary = summarize(&text).unwrap();
    assert_eq!((summary.frames, summary.dropped, summary.over), (4, 1, 2));
    assert_eq!(summary.submit.unwrap().max, 17.0);
    assert_eq!(summary.sim.unwrap().max, 0.4);
    assert_eq!(summary.gpu.unwrap().max, 20.0);
    assert!(summarize("not json\n").is_err());
}
