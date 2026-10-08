use std::path::{Path, PathBuf};

use pfx_core::clock::Tick;
use pfx_input::{ActionSpec, Binding, Input, InputEvent, Key};
use pfx_load::scene::{Scene, Target};
use pfx_physics::rigid::{CharacterDesc, Pose};

use super::Folder;
use crate::{Edit, Game, Options, PlayError, PlaySession, World};

const FRAME: f32 = 1.0 / 60.0;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/characters")
        .join(name)
        .join(format!("{name}.scene.toml"))
}

fn scene(name: &str) -> Scene {
    Scene::open(fixture(name)).unwrap()
}

struct Runner {
    speed: f32,
    acceleration: f32,
}

impl Runner {
    fn of(name: &str) -> Runner {
        match name {
            "platformer" => Runner {
                speed: 6.0,
                acceleration: 60.0,
            },
            _ => Runner {
                speed: 4.5,
                acceleration: 40.0,
            },
        }
    }
}

impl Game for Runner {
    fn actions(&self) -> Vec<ActionSpec> {
        vec![
            ActionSpec::digital("left").key(Binding::key(Key::A)),
            ActionSpec::digital("right").key(Binding::key(Key::D)),
            ActionSpec::digital("forward").key(Binding::key(Key::W)),
            ActionSpec::digital("back").key(Binding::key(Key::S)),
            ActionSpec::digital("jump").key(Binding::key(Key::Space)),
        ]
    }

    fn start(&mut self, world: &mut World) {
        let player = world.object("player").unwrap();
        let desc = CharacterDesc {
            speed: self.speed,
            acceleration: self.acceleration,
            ..*world.character(player).unwrap().desc()
        };
        world.set_character_desc(player, desc).unwrap();
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, input: &Input) {
        let axis = |plus: &str, minus: &str| {
            f32::from(u8::from(input.action(plus).held))
                - f32::from(u8::from(input.action(minus).held))
        };
        let direction = [axis("right", "left"), 0.0, axis("back", "forward")];
        let jump = input.action("jump").held;
        if let Some(lift) = world.object("lift").and_then(|id| world.body(id)) {
            let phase = (world.ticks().saturating_sub(60) % 240) as f32 / 120.0;
            let swing = if phase < 1.0 { phase } else { 2.0 - phase };
            world
                .physics
                .move_to(lift, Pose::at([-3.0, -0.2 + 2.0 * swing, 0.0]));
        }
        let player = world.object("player").unwrap();
        world.character(player).unwrap().drive(direction, jump);
    }
}

fn press(session: &mut PlaySession, key: Key, pressed: bool) {
    session.feed(InputEvent::Key { key, pressed });
}

fn feet(session: &PlaySession) -> [f32; 3] {
    let world = session.world();
    world
        .character_of(world.object("player").unwrap())
        .unwrap()
        .feet()
}

fn play(name: &str) -> PlaySession {
    PlaySession::play_with(
        &scene(name),
        None,
        Box::new(Runner::of(name)),
        Options::default(),
    )
    .unwrap()
}

fn player(session: &PlaySession) -> &pfx_physics::rigid::Character {
    let world = session.world();
    world.character_of(world.object("player").unwrap()).unwrap()
}

fn run(session: &mut PlaySession, ticks: u32) {
    for _ in 0..ticks {
        session.advance(FRAME);
    }
}

#[test]
fn the_platformer_player_walks_the_step_and_ramp_in_its_plane() {
    let mut session = play("platformer");
    assert_eq!(
        player(&session).desc().plane,
        Some(pfx_physics::rigid::Plane::Xy)
    );
    press(&mut session, Key::D, true);
    let mut on_step = false;
    let mut on_ramp = false;
    for _ in 0..170 {
        session.advance(FRAME);
        let [x, y, z] = feet(&session);
        assert_eq!(z.to_bits(), 0.0f32.to_bits());
        if (3.6..4.4).contains(&x) {
            on_step |= player(&session).grounded() && (y - 0.25).abs() < 0.03;
        }
        if (8.5..9.5).contains(&x) {
            on_ramp |= player(&session).grounded() && y > 0.5;
        }
        assert!(x < 11.5, "the ledge stops a walk: {x}");
    }
    assert!(on_step, "stood on the 0.25 step");
    assert!(on_ramp, "walked up the 20° ramp");
    let object = session.world().object("player").unwrap();
    assert_eq!(session.world().at(object), feet(&session));
}

#[test]
fn the_platformer_player_jumps_to_the_ledge_slides_the_wall_and_wall_jumps() {
    let mut session = play("platformer");
    press(&mut session, Key::D, true);
    let mut ledge = false;
    let mut slid = false;
    let mut kicked = false;
    for tick in 0..300 {
        if tick % 50 == 30 {
            press(&mut session, Key::Space, true);
        }
        if tick % 50 == 45 {
            press(&mut session, Key::Space, false);
        }
        session.advance(FRAME);
        let [x, y, _] = feet(&session);
        let character = player(&session);
        ledge |= character.grounded() && (11.5..14.5).contains(&x) && (y - 1.5).abs() < 0.1;
        if character.on_wall().is_some() {
            slid |= (character.velocity()[1] + 2.0).abs() < 1.0e-4;
        }
        kicked |= character.velocity()[0] < -4.9 && character.velocity()[1] > 6.0;
    }
    assert!(ledge, "landed on the ledge");
    assert!(slid, "slid down the wall at its wall_slide speed");
    assert!(kicked, "kicked off the wall");
}

#[test]
fn the_platformer_lift_carries_the_player_up() {
    let mut session = play("platformer");
    press(&mut session, Key::A, true);
    run(&mut session, 25);
    press(&mut session, Key::A, false);
    let mut top = 0.0f32;
    for _ in 0..160 {
        session.advance(FRAME);
        let world = session.world();
        let lift = world.at(world.object("lift").unwrap());
        let [x, y, _] = feet(&session);
        assert!((x + 3.0).abs() < 1.0, "on the lift: {x}");
        if player(&session).grounded() {
            assert!((y - (lift[1] + 0.25)).abs() < 0.04, "{y} on {lift:?}");
        }
        top = top.max(y);
    }
    assert!(top > 2.0, "the lift carried the player to {top}");
}

#[test]
fn the_room_player_climbs_the_stairs_and_walls_stop_it() {
    let mut session = play("room");
    press(&mut session, Key::W, true);
    run(&mut session, 60);
    press(&mut session, Key::W, false);
    press(&mut session, Key::D, true);
    let mut heights = Vec::new();
    for _ in 0..140 {
        session.advance(FRAME);
        if player(&session).grounded() {
            let y = feet(&session)[1];
            for stair in [0.2f32, 0.4, 0.6] {
                if (y - stair - 0.02).abs() < 0.01 && !heights.contains(&stair) {
                    heights.push(stair);
                }
            }
        }
    }
    assert_eq!(heights, [0.2, 0.4, 0.6], "each 0.2 stair is stepped");
    let [x, y, _] = feet(&session);
    assert!((x - (6.0 - 0.35 - 0.02)).abs() < 0.01, "the east wall: {x}");
    assert!(y < 0.03);
}

#[test]
fn the_room_player_pushes_the_crate() {
    let mut session = play("room");
    let start = {
        let world = session.world();
        world.at(world.object("crate").unwrap())
    };
    press(&mut session, Key::S, true);
    run(&mut session, 90);
    let world = session.world();
    let crate_at = world.at(world.object("crate").unwrap());
    assert!(crate_at[2] > start[2] + 0.5, "{start:?} {crate_at:?}");
    assert!(feet(&session)[2] < crate_at[2]);
}

fn scripted(session: &mut PlaySession) {
    press(session, Key::D, true);
    for tick in 0..240 {
        if tick % 45 == 20 {
            press(session, Key::Space, true);
        }
        if tick % 45 == 30 {
            press(session, Key::Space, false);
        }
        session.advance(FRAME);
    }
}

#[test]
fn a_recorded_run_replays_to_the_same_positions_bit_for_bit() {
    let options = Options {
        record: true,
        ..Options::default()
    };
    let mut session = PlaySession::play_with(
        &scene("platformer"),
        None,
        Box::new(Runner::of("platformer")),
        options,
    )
    .unwrap();
    scripted(&mut session);
    let recording = session.take_recording().unwrap();
    let first: Vec<u32> = session
        .world()
        .models()
        .iter()
        .flatten()
        .flatten()
        .map(|v| v.to_bits())
        .collect();
    let replay = Options {
        replay: Some(recording),
        ..Options::default()
    };
    let mut again = PlaySession::play_with(
        &scene("platformer"),
        None,
        Box::new(Runner::of("platformer")),
        replay,
    )
    .unwrap();
    for tick in 0..240 {
        again.advance(if tick % 3 == 0 {
            FRAME * 2.0
        } else {
            FRAME * 0.5
        });
    }
    while again.world().ticks() < session.world().ticks() {
        again.advance(FRAME);
    }
    let second: Vec<u32> = again
        .world()
        .models()
        .iter()
        .flatten()
        .flatten()
        .map(|v| v.to_bits())
        .collect();
    assert_eq!(again.world().ticks(), session.world().ticks());
    assert_eq!(first, second);
}

#[test]
fn a_character_comes_only_from_its_object_character() {
    let mut session = play("room");
    let world = session.world_mut();
    let floor = world.object("floor").unwrap();
    let error = world.character(floor).unwrap_err();
    assert_eq!(
        error,
        PlayError::Body("object floor has no [object.character]".to_string())
    );
    let crate_id = world.object("crate").unwrap();
    world
        .set_character_desc(crate_id, CharacterDesc::default())
        .unwrap();
    let error = world.character(crate_id).unwrap_err().to_string();
    assert!(error.contains("has a body"), "{error}");
    let player = world.object("player").unwrap();
    let body = world.character(player).unwrap().body();
    assert_eq!(
        world.character(player).unwrap().body(),
        body,
        "spawned once"
    );
    assert_eq!(world.characters().count(), 1);
    assert!(world.remove_character(player));
    assert!(world.character_of(player).is_none());
}

#[test]
fn the_scene_gives_each_character_its_keys_and_defaults() {
    let platformer = scene("platformer");
    let player = &platformer.characters["player"];
    assert_eq!(player.plane, Some(pfx_load::scene::CharacterPlane::Xy));
    assert_eq!((player.jump_speed, player.variable_jump), (6.3, 0.5));
    assert_eq!(player.wall_jump, Some([5.0, 7.0]));
    let room = scene("room");
    let player = &room.characters["player"];
    assert_eq!(player.plane, None);
    assert_eq!((player.max_climb, player.max_step), (40.0, 0.25));
    assert_eq!(
        (player.snap, player.coyote, player.jump_buffer),
        (0.2, 6, 6)
    );
    assert_eq!(player.variable_jump, 0.0);

    let folder = Folder::new(
        "character defaults",
        r#"
[[object]]
name = "walker"
mesh = "block"
at = [0.0, 1.0, 2.0]
scale = [0.8, 2.0, 0.5]

[character.walker]
layer = "hero"

[[object]]
name = "hopper"
mesh = "block"
at = [2.0, 1.0, 2.0]
scale = [0.6, 1.6, 0.6]

[character.hopper]
kind = "2d"
plane = "yz"
"#,
    );
    std::fs::write(
        folder.root.join("project.toml"),
        "[project]\n\n[layers]\nhero = [\"world\"]\nworld = [\"hero\", \"world\"]\n",
    )
    .unwrap();
    let scene = folder.scene();
    let walker = &scene.characters["walker"];
    assert_eq!(
        (walker.radius, walker.height),
        (0.4, 2.0),
        "from the mesh bounds"
    );
    assert_eq!(
        (walker.max_climb, walker.max_step, walker.snap),
        (45.0, 0.3, 0.2)
    );
    assert_eq!(
        (walker.coyote, walker.jump_buffer, walker.jump_speed),
        (6, 6, 5.0)
    );
    assert_eq!(walker.layer.as_deref(), Some("hero"));
    let hopper = &scene.characters["hopper"];
    assert_eq!(hopper.plane, Some(pfx_load::scene::CharacterPlane::Yz));
    let mut session =
        PlaySession::play_with(&scene, None, Box::new(Idle), Options::default()).unwrap();
    let world = session.world_mut();
    let walker = world.object("walker").unwrap();
    let layers = world.character(walker).unwrap().desc().layers;
    assert_eq!((layers.member, layers.mask), (0b01, 0b10));
    let hopper = world.object("hopper").unwrap();
    let desc = *world.character(hopper).unwrap().desc();
    assert_eq!(desc.plane, Some(pfx_physics::rigid::Plane::Yz));
    assert_eq!((desc.layers.member, desc.layers.mask), (u32::MAX, u32::MAX));
    let feet = world.character(hopper).unwrap().feet();
    run(&mut session, 30);
    let after = session
        .world()
        .character_of(session.world().object("hopper").unwrap())
        .unwrap()
        .feet();
    assert_eq!(
        after[0].to_bits(),
        feet[0].to_bits(),
        "a yz character keeps its x"
    );
    let flat = Folder::new(
        "character too flat",
        "\n[[object]]\nname = \"slab\"\nmesh = \"block\"\nat = [0.0, 1.0, 2.0]\nscale = [2.0, 0.5, 2.0]\n\n[character.slab]\n",
    );
    let error = Scene::open(flat.root.join("room.scene.toml"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("object slab: its character's height 0.5 is less than twice its radius 1"),
        "{error}"
    );
}

struct Idle;

impl Game for Idle {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let hopper = world.object("hopper").unwrap();
        world
            .character(hopper)
            .unwrap()
            .drive([1.0, 0.0, 1.0], false);
    }
}

#[test]
fn live_edits_reach_the_character_and_are_held_as_pending() {
    let mut session = play("room");
    let player_target = || Target::Object("player".into());
    session
        .edit(Edit::scene(
            player_target(),
            &["character", "jump_speed"],
            2.0f32,
        ))
        .unwrap();
    run(&mut session, 10);
    press(&mut session, Key::Space, true);
    run(&mut session, 2);
    let rise = player(&session).velocity()[1];
    assert!(rise > 1.5 && rise < 2.0, "{rise}");
    assert_eq!(player(&session).desc().jump_speed, 2.0);
    session
        .edit(Edit::scene(
            player_target(),
            &["character", "height"],
            1.2f32,
        ))
        .unwrap();
    assert_eq!(player(&session).desc().height, 1.2);
    let refused = session
        .edit(Edit::scene(
            player_target(),
            &["character", "radius"],
            0.0f32,
        ))
        .unwrap_err();
    assert!(
        refused.contains("radius needs a number above 0"),
        "{refused}"
    );
    let refused = session
        .edit(Edit::scene(
            player_target(),
            &["character", "wall_slide"],
            1.0f32,
        ))
        .unwrap_err();
    assert!(
        refused.contains("a 3d character takes no such key"),
        "{refused}"
    );
    let refused = session
        .edit(Edit::scene(
            Target::Object("floor".into()),
            &["character", "jump_speed"],
            1.0f32,
        ))
        .unwrap_err();
    assert!(refused.contains("has no [object.character]"), "{refused}");
    assert_eq!(session.pending().len(), 2);
    let stopped = session.stop();
    let again = PlaySession::play_with(
        &stopped.scene,
        None,
        Box::new(Runner::of("room")),
        Options::default(),
    )
    .unwrap();
    assert_eq!(
        player(&again).desc().jump_speed,
        4.4,
        "stop drops live edits"
    );
}

fn poses(world: &World) -> Vec<u32> {
    let mut bits = Vec::new();
    for body in world.physics.bodies() {
        let pose = world.physics.pose(body).unwrap();
        bits.extend(pose.position.map(f32::to_bits));
        bits.extend(pose.rotation.map(f32::to_bits));
    }
    for (_, character) in world.characters() {
        bits.extend(character.feet().map(f32::to_bits));
        bits.extend(character.velocity().map(f32::to_bits));
    }
    bits
}

fn fingerprint(bits: &[u32]) -> u64 {
    bits.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, word| {
        word.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
    })
}

fn recorded(name: &str) -> pfx_input::Recording {
    let options = Options {
        record: true,
        ..Options::default()
    };
    let mut session =
        PlaySession::play_with(&scene(name), None, Box::new(Runner::of(name)), options).unwrap();
    let keys = [Key::D, Key::W, Key::A, Key::S];
    for tick in 0..360u32 {
        if tick % 60 == 0 {
            let at = (tick / 60) as usize;
            press(&mut session, keys[at % 4], false);
            press(&mut session, keys[(at + 1) % 4], true);
        }
        if tick % 37 == 10 {
            press(&mut session, Key::Space, true);
        }
        if tick % 37 == 22 {
            press(&mut session, Key::Space, false);
        }
        session.advance(FRAME);
    }
    session.take_recording().unwrap()
}

fn replayed(name: &str, recording: &pfx_input::Recording) -> Vec<u32> {
    let options = Options {
        replay: Some(recording.clone()),
        ..Options::default()
    };
    let mut session =
        PlaySession::play_with(&scene(name), None, Box::new(Runner::of(name)), options).unwrap();
    while (session.world().ticks() as usize) < recording.steps.len() {
        session.advance(FRAME);
    }
    poses(session.world())
}

#[test]
fn a_recorded_run_gives_every_pose_bit_for_bit_on_every_platform() {
    let mut prints = Vec::new();
    for name in ["platformer", "room"] {
        let recording = recorded(name);
        assert_eq!(recording.steps.len(), 360);
        let first = replayed(name, &recording);
        assert_eq!(first, replayed(name, &recording), "{name}");
        prints.push(fingerprint(&first));
    }
    assert_eq!(
        prints,
        [0xc482_571f_1989_ba6e, 0x0b57_cda6_6eec_5367],
        "the same natively and in the Windows phase; a change here changes every recorded run"
    );
}
