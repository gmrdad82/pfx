use std::path::Path;

use pfx_load::scene::{BodyShape, Projection, Target, Value};

use super::{BODIES, FRAME, Folder, bits, headless};
use crate::tunables::{Kind, TunableValue, Tunables};
use crate::{Edit, Options, SceneGame};

const TUNABLES: &str = r#"# what a game reads while it plays
[tunables.jump_height]
type = "float"
default = 1.2   # metres
min = 0.0
max = 4.0
group = "movement"

[tunables.lives]
type = "int"
default = 3
min = 1
max = 9
group = "rules"

[tunables.god_mode]
type = "bool"
default = false
group = "rules"

[tunables.gravity]
type = "vector"
default = [0.0, -9.81, 0.0]
min = [-20.0, -20.0, -20.0]
max = [20.0, 20.0, 20.0]
group = "movement"
"#;

fn object(name: &str) -> Target {
    Target::Object(name.to_string())
}

fn edit(target: Target, path: &[&str], value: impl Into<Value>) -> Edit {
    Edit::scene(target, path, value)
}

#[test]
fn material_light_camera_and_exposure_edits_reach_the_running_world() {
    let folder = Folder::new("live world", "");
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let clay = Target::Material("clay".into());
    session
        .edit(edit(clay.clone(), &["base"], [1.0f32, 0.0, 0.0]))
        .unwrap();
    session
        .edit(edit(clay.clone(), &["roughness"], 0.25f32))
        .unwrap();
    let material = session.world().material("clay").unwrap();
    assert_eq!(material.base, [1.0, 0.0, 0.0]);
    assert_eq!(material.roughness, 0.25);
    assert_eq!(
        material.specular,
        scene.library.get("clay").unwrap().specular
    );
    assert!(
        session
            .edit(edit(clay.clone(), &["nonsense"], 1.0f32))
            .is_err()
    );
    assert!(session.edit(edit(clay, &["roughness"], "high")).is_err());
    assert_eq!(session.world().material("clay").unwrap().roughness, 0.25);

    let warm = Target::Light("warm".into());
    session
        .edit(edit(warm.clone(), &["color"], [0.1f32, 0.2, 0.3]))
        .unwrap();
    session
        .edit(edit(warm.clone(), &["intensity"], 2.0f32))
        .unwrap();
    session
        .edit(edit(warm.clone(), &["range"], 3.0f32))
        .unwrap();
    session
        .edit(edit(warm.clone(), &["radius"], 0.2f32))
        .unwrap();
    let light = session.world().light("warm").unwrap();
    assert_eq!(light.color, [0.1, 0.2, 0.3]);
    assert_eq!(
        (light.intensity, light.range, light.radius),
        (2.0, 3.0, 0.2)
    );
    assert!(session.edit(edit(warm, &["range"], -1.0f32)).is_err());
    assert_eq!(scene.lights["warm"].intensity, 6.0);

    session
        .edit(edit(Target::Camera, &["at"], [1.0f32, 2.0, 3.0]))
        .unwrap();
    session
        .edit(edit(Target::Camera, &["look_at"], [0.0f32, 1.0, 0.0]))
        .unwrap();
    session
        .edit(edit(Target::Camera, &["fov"], 55.0f32))
        .unwrap();
    session
        .edit(edit(Target::Finish, &["exposure"], 1.5f32))
        .unwrap();
    let world = session.world();
    assert_eq!(world.camera.at, [1.0, 2.0, 3.0]);
    assert_eq!(world.camera.look_at, [0.0, 1.0, 0.0]);
    assert_eq!(
        world.camera.projection,
        Projection::Perspective { fov: 55.0 }
    );
    assert_eq!(world.exposure(), 1.5);
    assert!(
        session
            .edit(edit(Target::Finish, &["exposure"], 0.0f32))
            .is_err()
    );
    assert!(session.edit(edit(Target::Sun, &["hour"], 3.0f32)).is_err());
}

#[test]
fn object_edits_reach_the_running_world_and_move_its_body() {
    let folder = Folder::new("live objects", BODIES);
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    session
        .edit(edit(object("wall"), &["at"], [3.0f32, 1.0, 0.0]))
        .unwrap();
    session
        .edit(edit(object("wall"), &["hidden"], true))
        .unwrap();
    session
        .edit(edit(object("wall"), &["material"], "metal"))
        .unwrap();
    let world = session.world();
    let wall = world.object("wall").unwrap();
    assert_eq!(world.at(wall), [3.0, 1.0, 0.0]);
    assert!(world.hidden(wall));
    assert_eq!(world.look(wall).material.as_deref(), Some("metal"));
    assert!(
        session
            .edit(edit(object("wall"), &["material"], "nothing"))
            .is_err()
    );
    session
        .edit(edit(object("ball"), &["at"], [-1.5f32, 2.0, 1.0]))
        .unwrap();
    let world = session.world();
    let ball = world.object("ball").unwrap();
    let pose = world.physics.pose(world.body(ball).unwrap()).unwrap();
    assert_eq!(pose.position, [-1.5, 2.0, 1.0]);
}

#[test]
fn body_edits_reach_rapier_on_the_next_tick_without_a_restart() {
    let folder = Folder::new("live bodies", BODIES);
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let crate_id = session.world().object("crate").unwrap();
    let body = session.world().body(crate_id).unwrap();
    let crate_object = object("crate");
    session
        .edit(edit(crate_object.clone(), &["body", "mass"], 12.0f32))
        .unwrap();
    assert!((session.world().physics.mass(body).unwrap() - 12.0).abs() < 1e-3);
    let BodyShape::Box { half } = session.world().body_desc(crate_id).unwrap().shape else {
        panic!("the crate's body is a box");
    };
    session
        .edit(edit(
            crate_object.clone(),
            &["body", "half"],
            [half[0] * 2.0, half[1], half[2]],
        ))
        .unwrap();
    assert!((session.world().physics.mass(body).unwrap() - 24.0).abs() < 1e-2);
    for (key, amount) in [("friction", 0.1f32), ("restitution", 0.6)] {
        session
            .edit(edit(crate_object.clone(), &["body", key], amount))
            .unwrap();
    }
    session
        .edit(edit(
            crate_object.clone(),
            &["body", "damping"],
            [0.5f32, 0.25],
        ))
        .unwrap();
    let desc = *session.world().body_desc(crate_id).unwrap();
    assert_eq!(
        (desc.friction, desc.restitution, desc.damping),
        (0.1, 0.6, [0.5, 0.25])
    );
    assert!(
        session
            .edit(edit(crate_object.clone(), &["body", "radius"], 1.0f32))
            .is_err()
    );
    assert!(
        session
            .edit(edit(crate_object.clone(), &["body", "mass"], -1.0f32))
            .is_err()
    );
    let before = session.world().at(crate_id);
    session
        .edit(edit(
            crate_object,
            &["body", "velocity"],
            [0.0f32, 6.0, 0.0],
        ))
        .unwrap();
    assert_eq!(session.world().at(crate_id), before);
    session.pause();
    session.step();
    let after = session.world().at(crate_id);
    assert!(after[1] > before[1] + 0.05, "{before:?} {after:?}");
    let pending = session.pending();
    assert!(pending.contains("object crate body.density"));
    assert!(!pending.contains("object crate body.mass"));
    assert!(pending.contains("object crate body.velocity"));
}

#[test]
fn a_bouncier_ball_bounces_higher_after_a_live_restitution_edit() {
    let folder = Folder::new("live bounce", BODIES);
    let scene = folder.scene();
    let run = |restitution: Option<f32>| {
        let mut session = headless(&scene, Box::new(SceneGame), Options::default());
        if let Some(amount) = restitution {
            session
                .edit(edit(object("ball"), &["body", "restitution"], amount))
                .unwrap();
            session
                .edit(edit(object("floor"), &["body", "restitution"], amount))
                .unwrap();
        }
        let ball = session.world().object("ball").unwrap();
        let mut fell = false;
        let mut peak = f32::NEG_INFINITY;
        let mut last = session.world().at(ball)[1];
        for _ in 0..150 {
            session.advance(FRAME);
            let now = session.world().at(ball)[1];
            if now > last && !fell {
                fell = true;
            }
            if fell {
                peak = peak.max(now);
            }
            last = now;
        }
        peak
    };
    let plain = run(None);
    let bouncy = run(Some(0.95));
    assert!(bouncy > plain + 0.1, "{plain} {bouncy}");
}

#[test]
fn sound_edits_change_the_level_of_the_next_play_and_the_playing_voice() {
    let folder = Folder::new("live sound", BODIES);
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let hum = Target::Sound("hum".into());
    session.edit(edit(hum.clone(), &["pan"], -0.5f32)).unwrap();
    session.edit(edit(hum.clone(), &["loop"], false)).unwrap();
    session
        .edit(edit(hum.clone(), &["volume"], 0.0f32))
        .unwrap();
    let level = session.world().sound_level("hum").unwrap();
    assert_eq!((level.volume, level.pan, level.looping), (0.0, -0.5, false));
    assert!(session.edit(edit(hum, &["pan"], 2.0f32)).is_err());
    session.take_audio();
    for _ in 0..30 {
        session.advance(FRAME);
    }
    let audio = session.take_audio();
    assert!(!audio.is_empty());
    let tail = &audio[audio.len() / 2..];
    assert!(
        tail.iter().all(|sample| sample.abs() < 1e-3),
        "the looping hum is silent at volume 0"
    );
}

#[test]
fn tunables_load_are_range_checked_and_read_by_the_game() {
    let folder = Folder::new("live tunables", "");
    std::fs::write(
        folder.root.join("project.toml"),
        format!("format = 1\n\n[project]\nname = \"room\"\n\n{TUNABLES}"),
    )
    .unwrap();
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let world = session.world();
    assert_eq!(world.tunable::<f32>("jump_height"), Some(1.2));
    assert_eq!(world.tunable::<i64>("lives"), Some(3));
    assert_eq!(world.tunable::<i32>("lives"), Some(3));
    assert_eq!(world.tunable::<f32>("lives"), Some(3.0));
    assert_eq!(world.tunable::<bool>("god_mode"), Some(false));
    assert_eq!(
        world.tunable::<[f32; 3]>("gravity"),
        Some([0.0, -9.81, 0.0])
    );
    assert_eq!(world.tunable::<bool>("jump_height"), None);
    assert_eq!(world.tunable::<f32>("missing"), None);
    let groups: Vec<(&str, Vec<&str>)> = world
        .tunables()
        .groups()
        .into_iter()
        .map(|(group, list)| (group, list.iter().map(|t| t.name.as_str()).collect()))
        .collect();
    assert_eq!(
        groups,
        [
            ("movement", vec!["gravity", "jump_height"]),
            ("rules", vec!["god_mode", "lives"]),
        ]
    );
    session
        .edit(Edit::tunable("jump_height", TunableValue::Float(2.5)))
        .unwrap();
    session
        .edit(Edit::tunable("lives", TunableValue::Int(5)))
        .unwrap();
    assert!(
        session
            .edit(Edit::tunable("jump_height", TunableValue::Float(9.0)))
            .is_err()
    );
    assert!(
        session
            .edit(Edit::tunable("lives", TunableValue::Float(2.5)))
            .is_err()
    );
    assert!(
        session
            .edit(Edit::tunable("god_mode", TunableValue::Int(1)))
            .is_err()
    );
    assert!(
        session
            .edit(Edit::tunable(
                "gravity",
                TunableValue::Vector([0.0, -30.0, 0.0])
            ))
            .is_err()
    );
    assert_eq!(session.world().tunable::<f32>("jump_height"), Some(2.5));
    assert_eq!(session.world().tunable::<i64>("lives"), Some(5));
    let pending: Vec<_> = session.pending().tunables().collect();
    assert_eq!(
        pending,
        [
            ("jump_height", TunableValue::Float(2.5)),
            ("lives", TunableValue::Int(5)),
        ]
    );
    let stopped = session.stop();
    assert_eq!(stopped.pending.len(), 2);
    let again = headless(&stopped.scene, stopped.game, Options::default());
    assert_eq!(again.world().tunable::<f32>("jump_height"), Some(1.2));
}

#[test]
fn a_tunables_file_refuses_bad_entries_by_name() {
    let file = Path::new("project.toml");
    let refused = |text: &str, says: &str| {
        let text = format!("[project]\nname = \"room\"\n\n{text}");
        let error = Tunables::parse(&text, file).unwrap_err();
        assert!(error.contains(says), "{error}");
    };
    refused(
        "[tunables.speed]\ntype = \"float\"\ndefault = 5.0\nmax = 2.0\n",
        "above its max",
    );
    refused(
        "[tunables.speed]\ntype = \"float\"\ndefault = 1.0\nmin = 3.0\nmax = 2.0\n",
        "min 3 is above max 2",
    );
    refused(
        "[tunables.speed]\ntype = \"number\"\ndefault = 1.0\n",
        "unknown variant `number`",
    );
    refused(
        "[tunables.count]\ntype = \"int\"\ndefault = 1.5\n",
        "default 1.5 is not a int",
    );
    refused(
        "[tunables.on]\ntype = \"bool\"\ndefault = true\nmin = 0\n",
        "a bool takes no min or max",
    );
    refused(
        "[tunables.at]\ntype = \"vector\"\ndefault = [1.0, 2.0]\n",
        "invalid length 2, expected an array of length 3",
    );
    refused(
        "[tunables.speed]\ntype = \"float\"\ndefault = 1.0\nstep = 0.1\n",
        "step",
    );
    let fine = Tunables::parse(
        "[project]\nname = \"room\"\n\n[tunables.speed]\ntype = \"float\"\ndefault = 1\n",
        file,
    )
    .unwrap();
    let speed = fine.get("speed").unwrap();
    assert_eq!(
        (speed.kind, speed.default),
        (Kind::Float, TunableValue::Float(1.0))
    );
    assert_eq!(speed.group, "");
    assert_eq!(fine.file(), Some(file));
}

#[test]
fn a_project_declares_its_tunables_in_project_toml_and_keep_writes_a_default_with_the_text_kept() {
    let folder = Folder::new("live tunables project", "");
    std::fs::write(
        folder.root.join("tunables.toml"),
        "[tunables.ignored]\ntype = \"bool\"\ndefault = true\n",
    )
    .unwrap();
    std::fs::write(
        folder.root.join("project.toml"),
        format!("format = 1\n\n[project]\nname = \"room\"\n\n{TUNABLES}"),
    )
    .unwrap();
    let found = Tunables::of_scene(&folder.root.join("room.scene.toml")).unwrap();
    assert_eq!(
        found.file(),
        Some(folder.root.join("project.toml").as_path())
    );
    assert_eq!(found.len(), 4);
    assert!(found.get("ignored").is_none());
    let scene = folder.scene();
    let session = headless(&scene, Box::new(SceneGame), Options::default());
    assert_eq!(session.world().tunable::<i64>("lives"), Some(3));
    let written = Tunables::written(TUNABLES, "jump_height", TunableValue::Float(2.5)).unwrap();
    assert_eq!(
        written,
        TUNABLES.replace("default = 1.2   # metres", "default = 2.5   # metres")
    );
    let vector =
        Tunables::written(TUNABLES, "gravity", TunableValue::Vector([0.0, -4.5, 0.0])).unwrap();
    assert!(vector.contains("default = [0.0, -4.5, 0.0]"), "{vector}");
    assert!(Tunables::written(TUNABLES, "missing", TunableValue::Bool(true)).is_err());
}

#[test]
fn edits_recorded_by_tick_replay_to_the_same_run() {
    let folder = Folder::new("live determinism", BODIES);
    let scene = folder.scene();
    let options = Options {
        seed: 5,
        ..Options::default()
    };
    let mut live = headless(&scene, Box::new(SceneGame), options.clone());
    let real = [FRAME, FRAME * 0.5, FRAME * 2.5, FRAME, FRAME * 1.5];
    let edits = [
        (
            4,
            edit(object("ball"), &["body", "velocity"], [1.5f32, 2.0, 0.0]),
        ),
        (9, edit(object("crate"), &["body", "friction"], 0.05f32)),
        (9, edit(object("ball"), &["body", "radius"], 0.2f32)),
        (17, edit(object("floor"), &["body", "restitution"], 0.8f32)),
        (23, edit(object("crate"), &["at"], [-0.9f32, 1.2, 0.0])),
        (31, edit(object("ball"), &["body", "mass"], 4.0f32)),
        (40, edit(Target::Sound("hum".into()), &["volume"], 0.5f32)),
    ];
    for frame in 0..120 {
        for (at, edit) in &edits {
            if *at == frame {
                live.edit(edit.clone()).unwrap();
            }
        }
        live.advance(real[frame % real.len()]);
    }
    let recorded = live.edits().to_vec();
    assert_eq!(recorded.len(), edits.len());
    assert!(recorded.windows(2).all(|pair| pair[0].tick <= pair[1].tick));
    let ticks = live.world().ticks();
    let models = bits(&live.world().models());
    let log = live.world().sounds.log().to_vec();
    let mut plain = headless(&scene, Box::new(SceneGame), options.clone());
    plain.pause();
    for _ in 0..ticks {
        plain.step();
    }
    assert_ne!(bits(&plain.world().models()), models);
    for _ in 0..2 {
        let mut replay = headless(
            &scene,
            Box::new(SceneGame),
            Options {
                edits: recorded.clone(),
                ..options.clone()
            },
        );
        assert!(
            replay
                .edit(edit(object("crate"), &["hidden"], true))
                .is_err()
        );
        replay.pause();
        for _ in 0..ticks {
            replay.step();
        }
        assert_eq!(replay.world().ticks(), ticks);
        assert_eq!(bits(&replay.world().models()), models);
        assert_eq!(replay.world().sounds.log(), log);
        assert_eq!(replay.edits(), recorded.as_slice());
    }
}

#[test]
fn stop_hands_back_the_pending_patch_and_the_next_play_starts_clean() {
    let folder = Folder::new("live pending", BODIES);
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let clay = Target::Material("clay".into());
    session
        .edit(edit(clay.clone(), &["roughness"], 0.2f32))
        .unwrap();
    session
        .edit(edit(clay.clone(), &["roughness"], 0.4f32))
        .unwrap();
    session
        .edit(edit(Target::Light("cool".into()), &["intensity"], 1.0f32))
        .unwrap();
    let pending = session.pending().clone();
    assert_eq!(pending.len(), 2);
    assert_eq!(
        pending.get("material clay roughness"),
        Some(&edit(clay, &["roughness"], 0.4f32))
    );
    let stopped = session.stop();
    assert_eq!(stopped.pending, pending);
    assert_eq!(stopped.edits.len(), 3);
    assert_eq!(stopped.scene, scene);
    let again = headless(&stopped.scene, stopped.game, Options::default());
    assert!(again.pending().is_empty());
    assert_eq!(
        again.world().material("clay"),
        scene.library.get("clay").copied()
    );
    assert_eq!(again.world().light("cool"), scene.lights.get("cool"));
}
