use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_input::{ActionSpec, Binding, Input, InputEvent, Key};
use pfx_load::scene::{Matrix, Reload, Scene};
use pfx_sound::Clip;

use super::*;

const FRAME: f32 = 1.0 / 60.0;

struct Folder {
    root: PathBuf,
}

impl Folder {
    fn new(name: &str, extra: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/play-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        std::fs::write(root.join("tone.wav"), tone().to_wav()).unwrap();
        let scene = root.join("room.scene.toml");
        let text = with_bodies(&std::fs::read_to_string(&scene).unwrap(), extra);
        std::fs::write(&scene, text).unwrap();
        Self { root }
    }

    fn scene(&self) -> Scene {
        Scene::open(self.root.join("room.scene.toml")).unwrap()
    }

    fn edit(&self, name: &str, from: &str, to: &str) {
        let path = self.root.join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "{name} has no {from:?}");
        std::fs::write(&path, text.replacen(from, to, 1)).unwrap();
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn with_bodies(text: &str, extra: &str) -> String {
    let mut text = text.to_string();
    let mut tables = Vec::new();
    for section in extra.split("\n[").filter(|section| !section.is_empty()) {
        let section = format!("[{section}");
        let found = ["body", "trigger", "character"]
            .into_iter()
            .find_map(|table| {
                section
                    .strip_prefix(&format!("[{table}."))
                    .map(|header| (table, header.to_string()))
            });
        match found {
            Some((table, header)) => {
                let (name, keys) = header
                    .split_once("]\n")
                    .unwrap_or((header.trim_end_matches(']'), ""));
                tables.push((table, name.trim_matches('"').to_string(), keys.to_string()));
            }
            None => {
                text.push('\n');
                text.push_str(&section);
            }
        }
    }
    for (table, name, keys) in tables {
        let start = text
            .find(&format!("name = \"{name}\"\n"))
            .unwrap_or_else(|| panic!("no object {name}"));
        let end = text[start..]
            .find("\n\n")
            .map_or(text.len(), |at| start + at + 1);
        let keys = if keys.ends_with('\n') || keys.is_empty() {
            keys
        } else {
            format!("{keys}\n")
        };
        let lead = if text[..end].ends_with('\n') {
            ""
        } else {
            "\n"
        };
        text.insert_str(end, &format!("{lead}\n[object.{table}]\n{keys}"));
    }
    text
}

fn tone() -> Clip {
    Clip {
        rate: 48_000,
        channels: 1,
        samples: (0..2_400)
            .map(|i| ((i as f32) * 0.05).sin() * 0.5)
            .collect(),
    }
}

const BODIES: &str = r#"
[physics]
gravity = [0.0, -9.81, 0.0]

[body.floor]
kind = "fixed"

[body.crate]
velocity = [0.0, 0.0, 0.0]

[[object]]
name = "ball"
mesh = "block"
at = [0.4, 1.6, 0.6]
scale = 0.3

[body.ball]
shape = "sphere"
restitution = 0.3

[[mover]]
name = "spin"
objects = ["left pillar"]
kind = "turn"
pivot = [0.6, 0.0, -0.6]
axis = [0.0, 1.0, 0.0]
travel = [0.0, 90.0]
period = 2.0

[sound.hum]
file = "tone.wav"
loop = true
volume = 0.25

[sound.knock]
file = "tone.wav"
play = "hit"
object = "ball"

[sound.chime]
file = "tone.wav"
play = "game"
"#;

#[derive(Default)]
struct Pusher {
    hooks: Rc<Cell<[u32; 4]>>,
}

impl Pusher {
    fn count(&self, hook: usize) {
        let mut hooks = self.hooks.get();
        hooks[hook] += 1;
        self.hooks.set(hooks);
    }
}

struct Pushes {
    count: u32,
    draws: Vec<u64>,
}

impl Game for Pusher {
    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec::digital("push").key(Binding::key(Key::D))]
    }

    fn start(&mut self, world: &mut World) {
        self.count(0);
        world.set_state(Pushes {
            count: 0,
            draws: Vec::new(),
        });
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, input: &Input) {
        self.count(1);
        let id = world.object("crate").unwrap();
        let mut at = world.at(id);
        at[1] += 0.01;
        if input.action("push").held {
            at[0] += 0.05;
            let draw = world.rng.next_u64();
            let pushes = world.state_mut::<Pushes>().unwrap();
            pushes.count += 1;
            pushes.draws.push(draw);
        }
        world.move_to(id, at);
    }

    fn frame(&mut self, _world: &World, _alpha: f32) {
        self.count(2);
    }

    fn stop(&mut self) {
        self.count(3);
    }
}

fn headless(scene: &Scene, game: Box<dyn Game>, options: Options) -> PlaySession {
    PlaySession::play_with(scene, None, game, options).unwrap()
}

fn bits(models: &[Matrix]) -> Vec<u32> {
    models
        .iter()
        .flatten()
        .flatten()
        .map(|value| value.to_bits())
        .collect()
}

fn hashes(scene: &Scene) -> BTreeMap<PathBuf, Vec<u8>> {
    scene
        .files
        .keys()
        .map(|path| (path.clone(), std::fs::read(path).unwrap()))
        .collect()
}

fn key(pressed: bool) -> InputEvent {
    InputEvent::Key {
        key: Key::D,
        pressed,
    }
}

#[test]
fn a_test_game_moves_an_object_by_its_input() {
    let folder = Folder::new("moves", "");
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(Pusher::default()), Options::default());
    let crate_id = session.world().object("crate").unwrap();
    let rest = session.world().at(crate_id);
    assert_eq!(
        session.world().model(crate_id),
        scene.object("crate").unwrap().model
    );
    for _ in 0..10 {
        session.advance(FRAME);
    }
    session.feed(key(true));
    for _ in 0..5 {
        session.advance(FRAME);
    }
    let ticks = session.world().ticks();
    assert!((9..=16).contains(&ticks), "{ticks}");
    let at = session.world().at(crate_id);
    assert!((at[1] - (rest[1] + 0.01 * ticks as f32)).abs() < 1e-4);
    let pushes = session.world().state::<Pushes>().unwrap().count;
    assert!(pushes >= 4, "{pushes}");
    assert!((at[0] - (rest[0] + 0.05 * pushes as f32)).abs() < 1e-4);
    let stopped = session.stop();
    assert_eq!(stopped.ticks, ticks);
}

#[test]
fn play_pause_step_resume_and_stop_restore_the_scene_exactly() {
    let folder = Folder::new("pause", BODIES);
    let scene = folder.scene();
    let before = scene.clone();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    for _ in 0..30 {
        session.advance(FRAME);
    }
    let ran = session.world().ticks();
    assert!(ran >= 29, "{ran}");
    session.pause();
    assert!(session.paused());
    for _ in 0..10 {
        let tick = session.advance(FRAME);
        assert_eq!(tick.ticks, 0);
    }
    assert_eq!(session.world().ticks(), ran);
    let paused = bits(&session.world().models());
    assert!(session.step());
    assert_eq!(session.world().ticks(), ran + 1);
    assert_ne!(
        bits(&session.world().models()),
        paused,
        "a step moves the scene"
    );
    session.resume();
    assert!(!session.step(), "step runs only while paused");
    for _ in 0..30 {
        session.advance(FRAME);
    }
    assert!(session.world().ticks() >= ran + 30);
    let ball = session.world().object("ball").unwrap();
    assert_ne!(
        session.world().model(ball),
        scene.object("ball").unwrap().model
    );
    let stopped = session.stop();
    assert_eq!(stopped.scene, before);
    assert!(stopped.diff.is_empty());
    assert!(stopped.errors.is_empty());
    assert_eq!(*stopped.snapshot, before);
}

#[test]
fn no_file_is_written_during_play() {
    let folder = Folder::new("no writes", BODIES);
    let scene = folder.scene();
    let files = hashes(&scene);
    let times: BTreeMap<PathBuf, std::time::SystemTime> = files
        .keys()
        .map(|path| {
            (
                path.clone(),
                std::fs::metadata(path).unwrap().modified().unwrap(),
            )
        })
        .collect();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    for frame in 0..90 {
        session.advance(FRAME);
        if frame == 20 {
            let world = session.world_mut();
            let wall = world.object("wall").unwrap();
            world.set_look(
                wall,
                Look {
                    color: Some([1.0, 0.0, 0.0]),
                    ..Look::default()
                },
            );
            world.move_to(wall, [0.0, 2.0, -2.0]);
            world.set_hidden(world.object("screen").unwrap(), true);
            world.play_sound("chime");
        }
    }
    let stopped = session.stop();
    assert!(stopped.diff.is_empty());
    assert_eq!(hashes(&scene), files);
    for (path, time) in times {
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), time);
    }
    let again = folder.scene();
    assert_eq!(again, scene);
}

#[test]
fn inspector_edits_during_play_are_discarded_on_stop() {
    let folder = Folder::new("inspector", "");
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let first = bits(&session.world().models());
    let world = session.world_mut();
    let wall = world.object("wall").unwrap();
    world.move_to(wall, [3.0, 1.0, 0.0]);
    world.set_look(
        wall,
        Look {
            material: Some("metal".into()),
            ..Look::default()
        },
    );
    assert!(world.set_hidden(wall, true));
    let stopped = session.stop();
    assert_eq!(stopped.scene, scene);
    let again = headless(&stopped.scene, stopped.game, Options::default());
    assert_eq!(bits(&again.world().models()), first);
    let wall = again.world().object("wall").unwrap();
    assert!(again.world().look(wall).is_plain());
    assert!(!again.world().hidden(wall));
}

#[test]
fn file_changes_during_play_queue_and_apply_on_stop() {
    let folder = Folder::new("queue", "");
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    session.advance(FRAME);
    folder.edit(
        "materials.toml",
        "base = [0.7, 0.32, 0.22]",
        "base = [0.2, 0.6, 0.3]",
    );
    let edited = folder.scene();
    session.queue(Ok(Reload {
        diff: scene.diff(&edited),
        scene: edited.clone(),
        recovered: false,
    }));
    folder.edit("lights.scene.toml", "intensity = 6.0", "intensity = 14.0");
    let later = folder.scene();
    session.queue(Ok(Reload {
        diff: edited.diff(&later),
        scene: later.clone(),
        recovered: false,
    }));
    folder.edit("room.scene.toml", "[[object]]", "[[object]\n");
    let broken = Scene::open(folder.root.join("room.scene.toml")).unwrap_err();
    session.queue(Err(broken.clone()));
    assert_eq!(session.world().scene(), &scene, "play keeps the snapshot");
    assert_eq!(
        session.world().scene().library.get("clay").unwrap().base,
        [0.7, 0.32, 0.22]
    );
    for _ in 0..10 {
        session.advance(FRAME);
    }
    let stopped = session.stop();
    assert_eq!(stopped.scene, later);
    assert_eq!(stopped.diff.materials.changed, ["clay"]);
    assert_eq!(stopped.diff.lights.changed, ["warm"]);
    assert_eq!(stopped.errors, [broken]);
    assert_eq!(*stopped.snapshot, scene);
}

#[test]
fn a_good_reload_clears_the_errors_queued_before_it() {
    let folder = Folder::new("recover", "");
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    folder.edit("room.scene.toml", "[[object]]", "[[object]\n");
    session.queue(Err(
        Scene::open(folder.root.join("room.scene.toml")).unwrap_err()
    ));
    folder.edit("room.scene.toml", "[[object]\n", "[[object]]");
    let fixed = folder.scene();
    session.queue(Ok(Reload {
        diff: scene.diff(&fixed),
        scene: fixed,
        recovered: true,
    }));
    let stopped = session.stop();
    assert!(stopped.errors.is_empty());
    assert!(stopped.diff.is_empty());
}

#[test]
fn an_ogg_vorbis_sound_plays_as_a_wav_does() {
    let bodies = BODIES.replace(
        "[sound.chime]\nfile = \"tone.wav\"",
        "[sound.chime]\nfile = \"tone.ogg\"",
    );
    assert_ne!(bodies, BODIES);
    let folder = Folder::new("ogg sound", &bodies);
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    session.take_audio();
    assert!(session.world_mut().play_sound("chime"));
    for _ in 0..6 {
        session.advance(FRAME);
    }
    let audio = session.take_audio();
    assert!(audio.iter().any(|sample| sample.abs() > 0.01));
    assert!(
        session
            .world()
            .sounds
            .log()
            .iter()
            .any(|(_, name)| name == "chime")
    );
}

#[test]
fn the_scene_game_animates_movers_simulates_bodies_and_plays_sounds() {
    let folder = Folder::new("scene game", BODIES);
    let scene = folder.scene();
    let mut session = headless(&scene, Box::new(SceneGame), Options::default());
    let world = session.world();
    let pillar = world.object("left pillar").unwrap();
    let ball = world.object("ball").unwrap();
    let crate_id = world.object("crate").unwrap();
    let floor = world.object("floor").unwrap();
    assert_eq!(world.sounds.log(), [(0, "hum".to_string())]);
    let ball_rest = world.at(ball);
    let crate_rest = world.at(crate_id);
    let mut lowest = f32::INFINITY;
    for _ in 0..90 {
        session.advance(FRAME);
        lowest = lowest.min(session.world().at(ball)[1]);
    }
    let world = session.world();
    let time = world.time() as f32;
    let posed = scene.posed(&scene.camera_or_default(), time);
    let model = world.model(pillar);
    for column in 0..4 {
        for row in 0..4 {
            assert!((model[column][row] - posed[pillar.index()][column][row]).abs() < 1e-5);
        }
    }
    assert_ne!(model, scene.object("left pillar").unwrap().model);
    assert!(lowest < ball_rest[1] - 0.5, "the ball falls: {lowest}");
    let resting = world.at(ball)[1];
    assert!(
        resting > 0.05 && resting < 0.4,
        "the ball rests on the floor: {resting}"
    );
    let crate_at = world.at(crate_id);
    assert!(
        (crate_at[1] - crate_rest[1]).abs() < 0.05,
        "the crate stays on the floor: {crate_at:?}"
    );
    assert_eq!(world.model(floor), scene.object("floor").unwrap().model);
    assert!(
        world
            .sounds
            .log()
            .iter()
            .any(|(tick, name)| name == "knock" && *tick > 0)
    );
    assert!(!world.sounds.log().iter().any(|(_, name)| name == "chime"));
    let audio = session.take_audio();
    assert!(!audio.is_empty());
    assert!(audio.iter().any(|sample| sample.abs() > 0.01));
    assert!(session.take_audio().is_empty());
}

#[test]
fn the_same_scene_seed_and_input_recording_give_the_same_ticks_and_world() {
    let folder = Folder::new("determinism", BODIES);
    let scene = folder.scene();
    let options = Options {
        seed: 7,
        record: true,
        ..Options::default()
    };
    let mut live = headless(&scene, Box::new(Pusher::default()), options);
    let real = [FRAME, FRAME * 0.5, FRAME * 2.5, FRAME, FRAME * 1.5];
    for frame in 0..120 {
        if frame % 17 == 3 {
            live.feed(key(frame % 34 == 3));
        }
        live.advance(real[frame % real.len()]);
    }
    let recording = live.take_recording().unwrap();
    let ticks = live.world().ticks();
    assert_eq!(recording.len() as u64, ticks);
    let models = bits(&live.world().models());
    let pushes = live.world().state::<Pushes>().unwrap().draws.clone();
    assert!(!pushes.is_empty());
    let log = live.world().sounds.log().to_vec();
    for _ in 0..2 {
        let mut replay = headless(
            &scene,
            Box::new(Pusher::default()),
            Options {
                seed: 7,
                replay: Some(recording.clone()),
                ..Options::default()
            },
        );
        while replay.world().ticks() < ticks {
            replay.feed(key(true));
            replay.advance(FRAME * 3.0);
        }
        assert_eq!(replay.world().ticks(), ticks);
        assert_eq!(bits(&replay.world().models()), models);
        assert_eq!(replay.world().state::<Pushes>().unwrap().draws, pushes);
        assert_eq!(replay.world().sounds.log(), log);
    }
}

#[test]
fn play_stop_and_play_again_start_from_the_same_world() {
    let folder = Folder::new("again", BODIES);
    let scene = folder.scene();
    let mut first = headless(&scene, Box::new(SceneGame), Options::default());
    let start = bits(&first.world().models());
    for _ in 0..40 {
        first.advance(FRAME);
    }
    let stopped = first.stop();
    let mut second = headless(&stopped.scene, stopped.game, Options::default());
    assert_eq!(bits(&second.world().models()), start);
    let mut third = headless(&scene, Box::new(SceneGame), Options::default());
    for _ in 0..40 {
        second.advance(FRAME);
        third.advance(FRAME);
    }
    assert_eq!(
        bits(&second.world().models()),
        bits(&third.world().models())
    );
}

#[test]
fn the_game_hooks_run_once_per_play_tick_frame_and_stop() {
    let folder = Folder::new("hooks", "");
    let scene = folder.scene();
    let hooks = Rc::new(Cell::new([0; 4]));
    let game = Pusher {
        hooks: hooks.clone(),
    };
    let mut session = headless(&scene, Box::new(game), Options::default());
    assert_eq!(hooks.get(), [1, 0, 0, 0]);
    session.advance(FRAME * 3.0);
    let ticks = session.world().ticks() as u32;
    assert_eq!(hooks.get(), [1, ticks, 1, 0]);
    session.pause();
    session.advance(FRAME);
    session.step();
    assert_eq!(hooks.get(), [1, ticks + 1, 3, 0]);
    let stopped = session.stop();
    assert_eq!(hooks.get(), [1, ticks + 1, 3, 1]);
    let again = headless(&stopped.scene, stopped.game, Options::default());
    assert_eq!(hooks.get(), [2, ticks + 1, 3, 1]);
    drop(again);
}

struct Calls(Rc<std::cell::RefCell<Vec<String>>>);

impl PlayHooks for Calls {
    fn on_play(&mut self, scene: &Scene) {
        let name = scene.path.file_name().unwrap().to_string_lossy();
        self.0.borrow_mut().push(format!("play {name}"));
    }

    fn on_pause(&mut self) {
        self.0.borrow_mut().push("pause".into());
    }

    fn on_resume(&mut self) {
        self.0.borrow_mut().push("resume".into());
    }

    fn on_stop(&mut self) {
        self.0.borrow_mut().push("stop".into());
    }
}

#[test]
fn the_session_calls_its_hooks_on_play_pause_resume_and_stop() {
    let folder = Folder::new("hooks calls", "");
    let scene = folder.scene();
    let calls = Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut session = PlaySession::play_hooked(
        &scene,
        None,
        Box::new(SceneGame),
        Options::default(),
        Box::new(Calls(calls.clone())),
    )
    .unwrap();
    session.advance(FRAME);
    session.pause();
    session.pause();
    session.step();
    session.resume();
    session.resume();
    let stopped = session.stop();
    assert_eq!(
        *calls.borrow(),
        ["play room.scene.toml", "pause", "resume", "stop"]
    );
    let again = PlaySession::play_hooked(
        &stopped.scene,
        None,
        stopped.game,
        Options::default(),
        stopped.hooks,
    )
    .unwrap();
    drop(again);
    assert_eq!(calls.borrow().len(), 5);
    let plain = headless(&scene, Box::new(SceneGame), Options::default());
    plain.stop();
    assert_eq!(calls.borrow().len(), 5);
}

#[derive(Clone, Default)]
struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_ticks_of_a_frame_report_their_time_to_the_frame_stats() {
    use pfx_live::stats::{FrameKind, FrameStats};
    let folder = Folder::new("stats", "");
    let scene = folder.scene();
    let sink = Sink::default();
    let stats = FrameStats::to_writer(Box::new(sink.clone()), 8);
    let mut session = headless(&scene, Box::new(Pusher::default()), Options::default());
    session.set_frame_stats(stats.clone());
    session.advance(FRAME);
    stats.begin(FrameKind::Full);
    stats.submitted(0, [64, 64]);
    stats.begin(FrameKind::Full);
    stats.submitted(1, [64, 64]);
    stats.finish();
    let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("\"tick\":0"));
    assert!(lines[0].contains("\"sim_ms\":") && !lines[0].contains("\"sim_ms\":null"));
    assert!(lines[1].contains("\"sim_ms\":null"));
}

mod animation;
mod characters;
mod live;
mod queries;
mod rig;

#[test]
fn a_backdrop_alone_is_not_an_empty_ui_and_the_frame_defaults_to_no_bars() {
    let mut ui = Ui::new();
    assert!(ui.is_empty());
    ui.backdrop(pfx_live::flat::Draw::rect([10.0, 10.0], [20.0, 20.0], 0.0));
    assert!(!ui.is_empty());
    assert_eq!(ui.backdrop.len(), 1);
    assert!(ui.draws.is_empty());
    let frame = UiFrame::default();
    assert_eq!(frame.window_units, frame.layout);
    assert_eq!(frame.content, [0.0, 0.0, frame.layout[0], frame.layout[1]]);
    assert!(!frame.backdrop);
}

#[test]
fn labels_take_a_box_a_line_a_transform_and_fallbacks_and_the_ui_composes_effects() {
    use pfx_live::flat::effects::{Effects, EffectsDesc, Flash};
    use pfx_live::flat::{Draw, Light, Srgba, place};

    let plain = Label::new("score", "DM Sans", 40.0, [100.0, 50.0]);
    assert_eq!(plain.line_height(), 50.0);
    assert_eq!(
        (plain.wrap, plain.height, plain.transform),
        (None, None, None)
    );
    assert!(plain.fallbacks.is_empty());
    let label = plain
        .clone()
        .line(60.0)
        .boxed([400.0, 300.0], Anchor::Center, Anchor::End)
        .transform(place([0.0, 0.0], 0.5))
        .fallbacks(["Noto Sans SC"]);
    assert_eq!(label.line_height(), 60.0);
    assert_eq!(label.wrap, Some(400.0));
    assert_eq!(label.height, Some(300.0));
    assert_eq!(
        (label.anchor, label.vertical),
        (Anchor::Center, Anchor::End)
    );
    assert_eq!(label.transform, Some(place([0.0, 0.0], 0.5)));
    assert_eq!(label.fallbacks, vec!["Noto Sans SC".to_string()]);
    assert_eq!(plain.clone().wrap(320.0).wrap, Some(320.0));

    let mut ui = Ui::new();
    assert_eq!((ui.clear, ui.look.is_none()), (None, true));
    ui.clear(Srgba::WHITE);
    assert_eq!(ui.clear, Some(Srgba::WHITE));
    let card = Draw::rect([200.0, 100.0], [120.0, 60.0], 8.0).fill(Srgba([0.5, 0.5, 0.5, 1.0]));
    ui.draw(card);
    let mut effects = Effects::new(EffectsDesc::new([1920.0, 1080.0], 3));
    let before = ui.clone();
    ui.effects(&mut effects);
    assert_eq!(ui.draws, before.draws);
    assert_eq!(ui.light, Light::default());
    assert!(effects.flash(Flash::new(Srgba::WHITE, 0.5, 0.0, 1.0)));
    ui.effects(&mut effects);
    assert_eq!(ui.draws[0], card);
    assert_eq!(ui.draws.len(), 2);

    let look = pfx_live::flat::look::Look::new();
    let base = pfx_live::flat::look::TokenSet::new();
    let still = UiLook::still(look.clone(), base.clone());
    ui.look(&still.frame());
    assert_eq!(ui.look, Some(still));
    assert_eq!(
        Prompt::glyph(pfx_input::Glyph::Touchpad, [0.0; 2], 48.0)
            .floor(12.0)
            .floor,
        12.0
    );
}
