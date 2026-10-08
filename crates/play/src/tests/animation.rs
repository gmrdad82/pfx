use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use pfx_core::anim::{AnimError, Animator, Playback};
use pfx_core::clock::Tick;
use pfx_input::Input;
use pfx_load::scene::Scene;

use super::{FRAME, headless};
use crate::{AnimationEvent, Game, ObjectId, Options, PlaySession, World};

const SCENE: &str = r#"format = 1
materials = ["materials.toml"]
fallback = "grey"

[mesh.column]
file = "column.glb"
id = "anmcm00001"

[mesh.panel]
file = "panel.gltf"
id = "anmpn00001"

[[object]]
name = "walker"
mesh = "column"
id = "anmwk00001"

[object.animation]
clip = "bend"
events = [{ name = "step", clip = "bend", time = 0.5 }]

[[object]]
name = "mixer"
mesh = "column"
at = [2.0, 0.0, 0.0]
id = "anmmx00001"

[object.animation]
blend = [{ clip = "bend", weight = 3.0 }, { clip = "lift", weight = 1.0 }]
loop = "ping-pong"

[[object]]
name = "still"
mesh = "column"
at = [-2.0, 0.0, 0.0]
id = "anmst00001"

[[object]]
name = "card"
mesh = "panel"
material = "screen"
at = [0.0, 1.0, -1.0]
id = "anmcd00001"

[object.animation]
atlas = "atlas.png"
grid = [4, 2]
fps = 10.0
clip = "spin"
events = [{ name = "glint", clip = "spin", frame = 2 }]

[object.animation.clips.spin]
from = 0
to = 3

[object.animation.clips.burst]
from = 4
to = 7
loop = "once"
"#;

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str, scene: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/play-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
        for file in ["materials.toml", "panel.gltf"] {
            std::fs::copy(fixtures.join(file), root.join(file)).unwrap();
        }
        std::fs::write(
            root.join("column.glb"),
            pfx_load::fixture::skinned_column(3),
        )
        .unwrap();
        std::fs::write(
            root.join("atlas.png"),
            pfx_load::fixture::atlas_png(4, 2, 8),
        )
        .unwrap();
        std::fs::write(root.join("anim.scene.toml"), scene).unwrap();
        Self { root }
    }

    fn open(&self) -> Result<Scene, pfx_load::scene::SceneError> {
        Scene::open(self.root.join("anim.scene.toml"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

type Heard = Rc<RefCell<Vec<(u64, String, String)>>>;

struct Listener {
    heard: Heard,
    script: fn(&mut World),
}

impl Game for Listener {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        (self.script)(world);
    }

    fn animated(&mut self, world: &mut World, events: &[AnimationEvent]) {
        for event in events {
            self.heard.borrow_mut().push((
                world.ticks(),
                world.name(event.object).to_string(),
                event.event.clone(),
            ));
        }
    }
}

fn session(folder: &Folder, script: fn(&mut World)) -> (PlaySession, Heard) {
    let scene = folder.open().unwrap();
    let heard: Heard = Rc::default();
    let game = Listener {
        heard: heard.clone(),
        script,
    };
    (headless(&scene, Box::new(game), Options::default()), heard)
}

fn id(world: &World, name: &str) -> ObjectId {
    world.object(name).unwrap()
}

fn rig(world: &World, name: &str) -> Arc<pfx_core::anim::Rig> {
    world
        .animator(id(world, name))
        .unwrap()
        .shared_rig()
        .clone()
}

#[test]
fn scene_objects_play_their_clips_from_the_start_and_fire_events_by_tick() {
    let folder = Folder::new("anim scene", SCENE);
    let (mut session, heard) = session(&folder, |_| {});
    for _ in 0..100 {
        session.advance(FRAME);
    }
    let world = session.world();
    let mut reference = Animator::new(rig(world, "walker"), 60);
    reference.play("bend", 0).unwrap();
    assert_eq!(
        world.palette(id(world, "walker")).unwrap(),
        reference.palette(world.ticks())
    );
    let mut mixer = Animator::new(rig(world, "mixer"), 60);
    mixer
        .play_blend(&[("bend", 3.0), ("lift", 1.0)], 0)
        .unwrap();
    mixer.set_playback("bend", Playback::PingPong).unwrap();
    mixer.set_playback("lift", Playback::PingPong).unwrap();
    assert_eq!(
        world.palette(id(world, "mixer")).unwrap(),
        mixer.palette(world.ticks())
    );
    let still = world.animator(id(world, "still")).unwrap();
    assert!(!still.is_playing());
    let steps: Vec<u64> = heard
        .borrow()
        .iter()
        .filter(|(_, object, event)| object == "walker" && event == "step")
        .map(|(tick, _, _)| *tick)
        .collect();
    assert_eq!(steps, [30, 90]);
    let glints: Vec<u64> = heard
        .borrow()
        .iter()
        .filter(|(_, object, _)| object == "card")
        .map(|(tick, _, _)| *tick)
        .collect();
    assert_eq!(glints, [12, 36, 60, 84]);
}

#[test]
fn a_sprite_card_steps_through_its_atlas_and_a_game_switches_its_clip() {
    let folder = Folder::new("anim sprite", SCENE);
    let (mut session, _) = session(&folder, |world| {
        if world.ticks() == 40 {
            let card = id(world, "card");
            world.play_clip(card, "burst").unwrap();
        }
    });
    let card = id(session.world(), "card");
    assert_eq!(
        session.world().sprite_frame(card),
        Some([0.0, 0.0, 0.25, 0.5])
    );
    for _ in 0..7 {
        session.advance(FRAME);
    }
    assert_eq!(
        session.world().sprite_frame(card),
        Some([0.25, 0.0, 0.5, 0.5])
    );
    for _ in 7..40 {
        session.advance(FRAME);
    }
    assert_eq!(
        session.world().sprite_frame(card),
        Some([0.0, 0.5, 0.25, 1.0])
    );
    for _ in 40..200 {
        session.advance(FRAME);
    }
    assert_eq!(
        session.world().sprite_frame(card),
        Some([0.75, 0.5, 1.0, 1.0])
    );
    assert!(session.world().sprite(card).unwrap().finished(200));
}

#[test]
fn a_game_animates_blends_and_crossfades_by_tick() {
    let folder = Folder::new("anim game", SCENE);
    let (mut session, _) = session(&folder, |world| {
        let walker = id(world, "walker");
        match world.ticks() {
            1 => world
                .animate(walker, "lift")
                .blend("bend", 0.3)
                .done()
                .unwrap(),
            20 => world
                .animate(walker, "wave")
                .fade(0.25)
                .speed(1.5)
                .done()
                .unwrap(),
            _ => {}
        }
    });
    for _ in 0..30 {
        session.advance(FRAME);
    }
    let world = session.world();
    let mut reference = Animator::new(rig(world, "walker"), 60);
    reference.play("bend", 0).unwrap();
    reference.play("lift", 1).unwrap();
    reference.blend("bend", 0.3, 1).unwrap();
    reference.crossfade("wave", 0.25, 20).unwrap();
    reference.set_speed("wave", 1.5, 20).unwrap();
    assert_eq!(
        world.palette(id(world, "walker")).unwrap(),
        reference.palette(world.ticks())
    );
    assert_eq!(
        world.animator(id(world, "walker")).unwrap().playing(),
        vec![("wave", 1.0)]
    );
}

#[test]
fn bad_clip_names_are_refused_and_reported() {
    let folder = Folder::new("anim errors", SCENE);
    let (mut session, _) = session(&folder, |world| {
        let walker = id(world, "walker");
        let still = id(world, "still");
        if world.ticks() == 2 {
            let error = world.animate(walker, "fly").blend("bend", 0.5).done();
            assert_eq!(error, Err(AnimError::UnknownClip("fly".into())));
            assert_eq!(world.animation_errors().len(), 1);
            let card = id(world, "card");
            assert!(matches!(
                world.animate(card, "spin").blend("burst", 0.5).done(),
                Err(AnimError::NotAnimated(_))
            ));
            assert!(world.play_clip(still, "wave").is_ok());
        }
    });
    for _ in 0..4 {
        session.advance(FRAME);
    }
    assert!(session.world().animation_errors().is_empty());
}

#[test]
fn the_loader_refuses_unknown_clips_and_frames_past_the_atlas() {
    for (from, to, says) in [
        (
            "clip = \"bend\"\nevents",
            "clip = \"fly\"\nevents",
            Some("has no clip fly"),
        ),
        (
            "clip = \"bend\", time",
            "clip = \"jog\", time",
            Some("marks clip jog"),
        ),
        ("to = 7", "to = 8", None),
        (
            "grid = [4, 2]",
            "frames = [[0, 0, 8, 8], [8, 0, 8, 8], [16, 0, 8, 8], [24, 0, 8, 8], [0, 8, 8, 8], [8, 8, 8, 8], [16, 8, 8, 8], [30, 8, 8, 8]]",
            Some("outside"),
        ),
    ] {
        let folder = Folder::new("anim refusals", &SCENE.replacen(from, to, 1));
        let error = folder.open().expect_err(to);
        if let Some(says) = says {
            assert!(error.message.contains(says), "{says}: {}", error.message);
        }
        assert!(error.line.is_some(), "{to}: {error:?}");
    }
}

fn run_hash() -> (u64, Vec<(u64, String, String)>) {
    let folder = Folder::new("anim determinism", SCENE);
    let (mut session, heard) = session(&folder, |world| {
        let walker = id(world, "walker");
        let mixer = id(world, "mixer");
        match world.ticks() {
            45 => world.animate(walker, "twist").fade(0.5).done().unwrap(),
            70 => world.animate(mixer, "wave").once().done().unwrap(),
            90 => world
                .animate(walker, "bend")
                .blend("lift", 0.4)
                .done()
                .unwrap(),
            _ => {}
        }
    });
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for frame in 0..240 {
        session.advance(if frame % 3 == 0 {
            FRAME * 2.0
        } else {
            FRAME * 0.5
        });
        let posed = session.world().posed();
        for palette in posed.palettes.values() {
            for value in palette.as_flattened().as_flattened() {
                hash ^= u64::from(value.to_bits());
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    let heard = heard.borrow().clone();
    for (tick, object, event) in &heard {
        for byte in tick
            .to_le_bytes()
            .into_iter()
            .chain(object.bytes())
            .chain(event.bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    assert!(heard.len() > 10, "{} events", heard.len());
    (hash, heard)
}

#[test]
fn the_same_ticks_give_the_same_poses_and_events_on_every_platform() {
    let (first, heard) = run_hash();
    let (second, again) = run_hash();
    assert_eq!(first, second);
    assert_eq!(heard, again);
    assert_eq!(first, GOLDEN, "{first:#x}");
}

const GOLDEN: u64 = 0xea4b_6ef8_79cf_6ed7;

#[test]
fn a_posed_snapshot_carries_models_palettes_and_sprite_frames() {
    let folder = Folder::new("anim posed", SCENE);
    let (mut session, _) = session(&folder, |_| {});
    for _ in 0..10 {
        session.advance(FRAME);
    }
    let world = session.world();
    let posed = world.posed();
    assert_eq!(posed.models.len(), world.len());
    let walker = id(world, "walker").index();
    assert_eq!(
        posed.palettes[&walker],
        world.palette(id(world, "walker")).unwrap()
    );
    let card = id(world, "card").index();
    assert_eq!(
        posed.frames[&card],
        world.sprite_frame(id(world, "card")).unwrap()
    );
    assert!(!posed.palettes.contains_key(&card));
}
