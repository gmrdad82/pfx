use std::path::Path;

use pfx_core::fx::Comfort;
use pfx_core::rig::Rig;
use pfx_load::scene::Projection;

use super::{FRAME, Folder, headless};
use crate::rigs::{Kind, Rigs};
use crate::{Game, Options, PlayError, PlaySession, World};
use pfx_core::clock::Tick;
use pfx_input::{ActionSpec, Input, InputEvent};

const SCENE: &str = r#"
[body.floor]
kind = "fixed"

[body.wall]
kind = "fixed"

[body.crate]
kind = "fixed"
"#;

const RIGS: &str = r#"
[[rig]]
name = "side"
kind = "follow2d"
target = "crate"
height = 6.0
dead_zone = [0.5, 0.25]
damping = 0.0
look_ahead = 1.0
look_ahead_speed = 2.0
look_ahead_damping = 0.0
blend = 0.0

[[rig]]
name = "zone"
kind = "follow2d"
target = "crate"
height = 6.0
dead_zone = [0.5, 0.25]
damping = 0.0
blend = 0.0

[[rig]]
name = "chase"
kind = "follow3d"
target = "crate"
arm = [0.0, 3.0, 6.0]
damping = 0.0
blend = 0.0

[[rig]]
name = "eyes"
kind = "first_person"
target = "crate"
eye = [0.0, 1.0, 0.0]
pitch_min = -50
pitch_max = 60
blend = 0.0
bob = { stride = 2.0, vertical = 0.05, sway = 0.02 }

[[rig]]
name = "behind"
kind = "third_person"
target = "crate"
eye = [0.0, 1.5, 0.0]
yaw = 180
blend = 0.0
third = { distance = 4.0, radius = 0.25, margin = 0.05, min_distance = 0.4, recover = 0.2 }

[[rig]]
name = "open"
kind = "third_person"
target = "crate"
eye = [0.0, 1.5, 0.0]
yaw = 0
blend = 0.0
third = { distance = 4.0, radius = 0.25 }
"#;

struct Walker {
    step: [f32; 3],
    look: [f32; 2],
    rig: &'static str,
}

impl Walker {
    fn new(rig: &'static str) -> Self {
        Self {
            step: [0.0; 3],
            look: [0.0; 2],
            rig,
        }
    }

    fn walking(mut self, step: [f32; 3]) -> Self {
        self.step = step;
        self
    }

    fn turning(mut self, look: [f32; 2]) -> Self {
        self.look = look;
        self
    }
}

impl Game for Walker {
    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec::stick("look")]
    }

    fn start(&mut self, world: &mut World) {
        assert!(world.rig(self.rig));
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let id = world.object("crate").unwrap();
        let at = world.at(id);
        world.move_to(id, std::array::from_fn(|axis| at[axis] + self.step[axis]));
        world.rig_look(self.look);
    }
}

fn rigs(text: &str) -> Rigs {
    Rigs::parse(text, Path::new("rigs.toml")).unwrap()
}

fn play(game: Walker, text: &str) -> (Folder, PlaySession) {
    let name = std::thread::current()
        .name()
        .unwrap_or("rigs")
        .replace("::", "-");
    let folder = Folder::new(&name, SCENE);
    let scene = folder.scene();
    let options = Options {
        rigs: Some(rigs(text)),
        ..Options::default()
    };
    let session = headless(&scene, Box::new(game), options);
    (folder, session)
}

fn run(session: &mut PlaySession, ticks: usize) {
    for _ in 0..ticks {
        session.advance(FRAME);
    }
}

fn near(a: f32, b: f32, within: f32) {
    assert!((a - b).abs() <= within, "{a} is not within {within} of {b}");
}

fn refused(text: &str) -> String {
    Rigs::parse(text, Path::new("rigs.toml")).unwrap_err()
}

#[test]
fn a_rigs_file_loads_every_kind_with_its_keys() {
    let all = rigs(RIGS);
    assert_eq!(all.len(), 6);
    assert_eq!(all.get("side").unwrap().kind, Kind::Follow2d);
    assert_eq!(all.get("side").unwrap().height, Some(6.0));
    assert_eq!(all.get("eyes").unwrap().kind, Kind::FirstPerson);
    let behind = all.get("behind").unwrap();
    assert!(behind.rig.person().unwrap().is_third());
    assert_eq!(
        behind
            .rig
            .person()
            .unwrap()
            .tuning()
            .third
            .unwrap()
            .distance,
        4.0
    );
    assert!(!all.get("eyes").unwrap().rig.person().unwrap().is_third());
    assert!(Rigs::parse("", Path::new("rigs.toml")).unwrap().is_empty());
}

#[test]
fn a_rigs_file_refuses_what_it_cannot_use_by_name() {
    let unknown = refused("[[rig]]\nname = \"a\"\nkind = \"follow3d\"\nzoom = 2\n");
    assert!(unknown.contains("zoom"), "{unknown}");
    let kind = refused("[[rig]]\nname = \"a\"\nkind = \"orbit\"\n");
    assert!(
        kind.contains("a") && kind.contains("kind") && kind.contains("orbit"),
        "{kind}"
    );
    let wrong = refused("[[rig]]\nname = \"a\"\nkind = \"first_person\"\ndead_zone = [1, 1]\n");
    assert!(
        wrong.contains("dead_zone") && wrong.contains("first_person"),
        "{wrong}"
    );
    let wrong = refused("[[rig]]\nname = \"a\"\nkind = \"follow3d\"\nbob = {}\n");
    assert!(wrong.contains("bob"), "{wrong}");
    let range = refused("[[rig]]\nname = \"a\"\nkind = \"follow3d\"\ndamping = -1.0\n");
    assert!(range.contains("damping"), "{range}");
    let bounds = refused(
        "[[rig]]\nname = \"a\"\nkind = \"follow2d\"\nbounds = { min = [5, 0], max = [1, 1] }\n",
    );
    assert!(bounds.contains("bounds"), "{bounds}");
    let pitch =
        refused("[[rig]]\nname = \"a\"\nkind = \"first_person\"\npitch_min = 50\npitch_max = 10\n");
    assert!(pitch.contains("pitch_min"), "{pitch}");
    let twice = refused(
        "[[rig]]\nname = \"a\"\nkind = \"follow3d\"\n[[rig]]\nname = \"a\"\nkind = \"follow3d\"\n",
    );
    assert!(twice.contains("share the name"), "{twice}");
    let third = refused("[[rig]]\nname = \"a\"\nkind = \"first_person\"\nthird = {}\n");
    assert!(third.contains("third"), "{third}");
    let side = refused("[[rig]]\nname = \"a\"\nkind = \"follow3d\"\nheight = 4\n");
    assert!(side.contains("height"), "{side}");
}

#[test]
fn a_project_names_its_rigs_file_or_keeps_rigs_toml() {
    let folder = Folder::new("rigs file", SCENE);
    let scene = folder.root.join("room.scene.toml");
    assert!(Rigs::of_scene(&scene).unwrap().is_empty());
    std::fs::write(folder.root.join("rigs.toml"), RIGS).unwrap();
    assert_eq!(Rigs::of_scene(&scene).unwrap().len(), 6);
    std::fs::write(
        folder.root.join("project.toml"),
        "rigs = \"cameras.toml\"\n",
    )
    .unwrap();
    assert!(Rigs::of_scene(&scene).unwrap().is_empty());
    std::fs::write(
        folder.root.join("cameras.toml"),
        "[[rig]]\nname = \"one\"\nkind = \"follow3d\"\n",
    )
    .unwrap();
    let named = Rigs::of_scene(&scene).unwrap();
    assert_eq!(named.names().collect::<Vec<_>>(), ["one"]);
    std::fs::write(folder.root.join("cameras.toml"), "[[rig]]\nkind = 3\n").unwrap();
    assert!(Rigs::of_scene(&scene).is_err());
    let error = PlayError::Rigs("broken".into());
    assert_eq!(error.to_string(), "broken");
}

#[test]
fn a_game_drives_the_camera_with_a_rig_and_stop_leaves_the_scene_alone() {
    let (_folder, mut session) = play(Walker::new("chase").walking([0.05, 0.0, 0.0]), RIGS);
    let scene_camera = session.world().camera;
    run(&mut session, 60);
    assert_eq!(session.world().active_rig(), Some("chase"));
    let id = session.world().object("crate").unwrap();
    let hero = session.world().at(id);
    let camera = session.world().camera;
    near(camera.look_at[0], hero[0], 0.06);
    near(camera.at[1] - camera.look_at[1], 3.0, 1e-4);
    near(camera.at[2] - camera.look_at[2], 6.0, 1e-4);
    assert_ne!(camera.at, scene_camera.at);
    assert_eq!(camera.projection, scene_camera.projection);
    session.world_mut().rig_off();
    assert_eq!(session.world().active_rig(), None);
    let stopped = session.stop();
    assert_eq!(stopped.scene.camera_or_default().at, scene_camera.at);
}

#[test]
fn a_two_d_rig_goes_orthographic_with_a_dead_zone_and_look_ahead() {
    let (_folder, mut session) = play(Walker::new("zone"), RIGS);
    run(&mut session, 3);
    assert_eq!(
        session.world().camera.projection,
        Projection::Orthographic { height: 6.0 }
    );
    let id = session.world().object("crate").unwrap();
    let start = session.world().at(id);
    let focus = session.world().camera.look_at;
    near(focus[0], start[0], 1e-4);
    let mut at = start;
    at[0] += 0.4;
    at[1] += 0.2;
    session.world_mut().move_to(id, at);
    run(&mut session, 5);
    assert_eq!(session.world().camera.look_at, focus);
    at[0] += 2.0;
    session.world_mut().move_to(id, at);
    run(&mut session, 5);
    assert!(session.world().camera.look_at[0] > focus[0] + 1.0);
}

#[test]
fn look_ahead_leads_a_walking_target_in_its_direction() {
    let (_folder, mut session) = play(Walker::new("side").walking([0.04, 0.0, 0.0]), RIGS);
    run(&mut session, 60);
    let id = session.world().object("crate").unwrap();
    let hero = session.world().at(id)[0];
    let lead = session.world().camera.look_at[0] - hero;
    assert!(lead > 0.4 && lead <= 1.0 + 1e-3, "{lead}");
}

#[test]
fn the_camera_is_interpolated_between_ticks_by_the_alpha() {
    let (_folder, mut session) = play(Walker::new("chase").walking([0.2, 0.0, 0.0]), RIGS);
    run(&mut session, 10);
    let rate = session.world().clock().tick_rate() as f32;
    let mut last = session.world().camera.at[0];
    let mut moved = 0;
    for _ in 0..40 {
        session.advance(0.25 / rate);
        let now = session.world().camera.at[0];
        assert!(now >= last - 1e-5, "{now} after {last}");
        if now > last + 1e-6 {
            moved += 1;
        }
        last = now;
    }
    assert!(moved >= 30, "{moved}");
}

#[test]
fn the_same_ticks_give_the_same_camera_whatever_the_frame_times() {
    let pose = |frames: &[f32]| {
        let (_folder, mut session) = play(
            Walker::new("eyes")
                .walking([0.04, 0.0, 0.02])
                .turning([6.0, -3.0]),
            RIGS,
        );
        for frame in frames {
            session.advance(*frame);
        }
        let ticks = session.world().ticks();
        let rig = session.world().rig_state("eyes").unwrap();
        let pose = rig.pose(1.0);
        (ticks, [pose.at, pose.look_at])
    };
    let even = pose(&[FRAME; 120]);
    let mut uneven = Vec::new();
    for _ in 0..40 {
        uneven.extend([FRAME * 0.5, FRAME * 2.0, FRAME * 0.5]);
    }
    let split = pose(&uneven);
    assert_eq!(even.0, 120);
    assert_eq!(split.0, 120);
    assert_eq!(even, split);
    assert_eq!(even, pose(&[FRAME; 120]));
}

#[test]
fn first_person_look_clamps_the_pitch_and_bobs_while_walking() {
    let (_folder, mut session) = play(
        Walker::new("eyes")
            .walking([0.08, 0.0, 0.0])
            .turning([0.0, -300.0]),
        RIGS,
    );
    run(&mut session, 90);
    let rig = session.world().rig_state("eyes").unwrap().person().unwrap();
    near(rig.pitch(), 60.0f32.to_radians(), 1e-4);
    assert!(rig.pitch() <= 60.0f32.to_radians() + 1e-5);
    assert!(rig.pitch() >= -50.0f32.to_radians() - 1e-5);
    assert!(rig.bob_amplitude() > 0.5);
    let id = session.world().object("crate").unwrap();
    let hero = session.world().at(id);
    let mut spread: f32 = 0.0;
    let low = hero[1] + 1.0;
    for _ in 0..90 {
        session.advance(FRAME);
        spread = spread.max((session.world().camera.at[1] - low).abs());
    }
    assert!(spread > 0.01, "{spread}");
}

#[test]
fn reduced_motion_turns_off_head_bob_and_look_ahead() {
    let bob = |comfort: Comfort| {
        let (_folder, mut session) = play(Walker::new("eyes").walking([0.08, 0.0, 0.0]), RIGS);
        session.world_mut().fx.comfort = comfort;
        run(&mut session, 90);
        let id = session.world().object("crate").unwrap();
        let low = session.world().at(id)[1] + 1.0;
        let mut spread: f32 = 0.0;
        for _ in 0..60 {
            session.advance(FRAME);
            spread = spread.max((session.world().camera.at[1] - low).abs());
        }
        spread
    };
    assert!(bob(Comfort::FULL) > 0.01);
    assert_eq!(bob(Comfort::reduced(0.0)), 0.0);
    let lead = |comfort: Comfort| {
        let (_folder, mut session) = play(Walker::new("side").walking([0.06, 0.0, 0.0]), RIGS);
        session.world_mut().fx.comfort = comfort;
        run(&mut session, 90);
        let id = session.world().object("crate").unwrap();
        let hero = session.world().at(id)[0];
        session.world().camera.look_at[0] - hero
    };
    assert!(lead(Comfort::FULL) > 0.4);
    assert!(lead(Comfort::reduced(0.0)) < 0.0);
}

#[test]
fn the_third_person_camera_stops_at_a_wall_the_world_physics_knows() {
    let (_folder, mut session) = play(Walker::new("behind"), RIGS);
    run(&mut session, 120);
    let id = session.world().object("crate").unwrap();
    let hero = session.world().at(id);
    let camera = session.world().camera;
    let rig = session
        .world()
        .rig_state("behind")
        .unwrap()
        .person()
        .unwrap();
    assert!(rig.arm() < 1.8, "arm {}", rig.arm());
    assert!(rig.arm() >= 0.4);
    assert!(camera.at[2] > -2.0, "the camera is at {:?}", camera.at);
    assert!(camera.at[2] < hero[2]);
    near(camera.look_at[2] - camera.at[2], 1.0, 1e-3);

    let (_folder, mut session) = play(Walker::new("open"), RIGS);
    run(&mut session, 240);
    let rig = session.world().rig_state("open").unwrap().person().unwrap();
    near(rig.arm(), 4.0, 0.02);
}

#[test]
fn the_third_person_camera_ignores_the_character_it_follows() {
    let (_folder, mut session) = play(Walker::new("open"), RIGS);
    run(&mut session, 120);
    let rig = session.world().rig_state("open").unwrap().person().unwrap();
    assert!(
        rig.arm() > 3.9,
        "the character's own body held the camera at {}",
        rig.arm()
    );
}

#[test]
fn switching_rigs_blends_the_camera_and_an_unknown_rig_changes_nothing() {
    let text = format!(
        "{RIGS}\n[[rig]]\nname = \"slow\"\nkind = \"follow3d\"\ntarget = \"crate\"\narm = [0, 8, 0]\nblend = 1.0\n"
    );
    let (_folder, mut session) = play(Walker::new("chase"), &text);
    run(&mut session, 5);
    let before = session.world().camera;
    assert!(!session.world_mut().rig("nowhere"));
    assert_eq!(session.world().active_rig(), Some("chase"));
    assert!(session.world_mut().rig("slow"));
    run(&mut session, 1);
    let during = session.world().camera;
    let travelled = (during.at[1] - before.at[1]).abs();
    assert!(travelled < 1.0, "{travelled}");
    run(&mut session, 90);
    let after = session.world().camera;
    near(after.at[1] - after.look_at[1], 8.0, 1e-3);
    assert!(after.at[1] > during.at[1]);
}

#[test]
fn a_rig_hands_over_to_another_target_without_a_jump() {
    let (_folder, mut session) = play(Walker::new("chase"), RIGS);
    run(&mut session, 5);
    let steady = session.world().camera.look_at;
    assert!(session.world_mut().rig_target("chase", Some("wall")));
    assert!(!session.world_mut().rig_target("chase", Some("nothing")));
    let mut last = steady;
    let mut biggest = 0.0f32;
    for _ in 0..60 {
        session.advance(FRAME);
        let now = session.world().camera.look_at;
        let step: f32 = (0..3)
            .map(|axis| (now[axis] - last[axis]).powi(2))
            .sum::<f32>()
            .sqrt();
        biggest = biggest.max(step);
        last = now;
    }
    let wall = session.world().object("wall").unwrap();
    let there = session.world().at(wall);
    near(last[0], there[0], 0.2);
    near(last[2], there[2], 0.2);
    assert!(biggest < 0.6, "{biggest}");
}

#[test]
fn the_pointer_turns_a_first_person_view_through_its_moves() {
    let (_folder, mut session) = play(Walker::new("eyes"), RIGS);
    run(&mut session, 3);
    let yaw = session
        .world()
        .rig_state("eyes")
        .unwrap()
        .person()
        .unwrap()
        .yaw();
    session.feed(InputEvent::PointerAt {
        layout: [100.0, 100.0],
        inside: true,
    });
    run(&mut session, 2);
    session.feed(InputEvent::PointerAt {
        layout: [150.0, 100.0],
        inside: true,
    });
    run(&mut session, 2);
    let turned = session
        .world()
        .rig_state("eyes")
        .unwrap()
        .person()
        .unwrap()
        .yaw();
    assert!((turned - yaw).abs() > 0.01);
}

#[test]
fn a_rig_state_is_reachable_for_a_game_to_toggle_the_view() {
    let (_folder, mut session) = play(Walker::new("behind"), RIGS);
    run(&mut session, 5);
    match session.world_mut().rig_state_mut("behind").unwrap() {
        Rig::Person(person) => person.set_third(false),
        Rig::Follow(_) => panic!("a person rig"),
    }
    run(&mut session, 240);
    let rig = session
        .world()
        .rig_state("behind")
        .unwrap()
        .person()
        .unwrap();
    assert!(!rig.is_third());
    assert_eq!(rig.arm(), 0.0);
}
