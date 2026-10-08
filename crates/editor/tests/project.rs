use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use glam::DVec3;
use pfx_core::clock::Tick;
use pfx_editor::editor::Play;
use pfx_editor::outline::Item;
use pfx_editor::script::frames;
use pfx_editor::session::{Session, Setup};
use pfx_editor_shell::{Script, shot};
use pfx_input::Input;
use pfx_play::{Game, World};

const SIZE: [u32; 2] = [1280, 800];

#[derive(Default)]
struct Calls {
    started: Cell<u32>,
    ticks: Cell<u64>,
    stopped: Cell<u32>,
}

struct Hider {
    calls: Rc<Calls>,
}

impl Game for Hider {
    fn start(&mut self, _world: &mut World) {
        self.calls.started.set(self.calls.started.get() + 1);
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        self.calls.ticks.set(self.calls.ticks.get() + 1);
        for name in ["incoming/badge", "incoming/wordmark", "grid"] {
            if let Some(object) = world.object(name) {
                world.set_hidden(object, true);
            }
        }
    }

    fn stop(&mut self) {
        self.calls.stopped.set(self.calls.stopped.get() + 1);
    }
}

fn project(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("editor-projects")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    pfx_project::scaffold("sample-game", &root, "0.0.0").unwrap();
    root
}

fn setup() -> Setup {
    Setup {
        pgpu: false,
        audio: false,
    }
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    for relative in [
        "project.toml",
        "content/first.scene.toml",
        "content/materials/base.materials.toml",
        "content/materials/incoming.materials.toml",
        "content/prefabs/incoming.prefab.toml",
    ] {
        found.push((relative.into(), fs::read(root.join(relative)).unwrap()));
    }
    found
}

#[derive(Debug)]
struct Seen {
    light: usize,
    violet: usize,
    red: usize,
}

fn seen(session: &Session) -> Seen {
    let pixels = session.pixels().unwrap();
    let mut out = Seen {
        light: 0,
        violet: 0,
        red: 0,
    };
    for pixel in pixels.chunks_exact(4) {
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]];
        if r > 170 && g > 170 && b > 170 {
            out.light += 1;
        }
        if u16::from(b) > u16::from(g) + 40 && b > 80 && u16::from(b) > u16::from(r) + 10 {
            out.violet += 1;
        }
        if u16::from(r) > u16::from(g) + 60 && u16::from(r) > u16::from(b) + 50 {
            out.red += 1;
        }
    }
    out
}

fn save(name: &str, png: &[u8]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/editor-shots");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{name}.png")), png).unwrap();
}

fn take(session: &mut Session, name: &str, script: &str) {
    let png = shot(session, SIZE, &Script::parse(script).unwrap()).unwrap();
    save(name, &png);
}

const MEAN_TOLERANCE: f64 = 1.5;
const WORST_TOLERANCE: u8 = 96;

fn decode(png_bytes: &[u8]) -> ([u32; 2], Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    ([info.width, info.height], buffer)
}

fn compare(name: &str, png: &[u8]) {
    let stored = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/shots")
        .join(format!("{name}.png"));
    if std::env::var_os("PFX_EDIT_BLESS").is_some() {
        fs::write(&stored, png).unwrap();
        return;
    }
    let reference = fs::read(&stored)
        .unwrap_or_else(|error| panic!("{name}: no stored shot at {} ({error})", stored.display()));
    let (size, want) = decode(&reference);
    let (got_size, got) = decode(png);
    assert_eq!(size, got_size, "{name}: size");
    let mut total = 0u64;
    let mut worst = 0u8;
    for (a, b) in want.iter().zip(&got) {
        let d = a.abs_diff(*b);
        total += u64::from(d);
        worst = worst.max(d);
    }
    let mean = total as f64 / want.len().max(1) as f64;
    assert!(
        mean <= MEAN_TOLERANCE && worst <= WORST_TOLERANCE,
        "{name}: mean {mean:.3}, worst {worst} against {}",
        stored.display()
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_editor_opens_a_scaffolded_project_and_f5_plays_the_games_own_logic() {
    let root = project("play");
    let before = snapshot(&root);
    let open = frames(14);
    let started = format!("{open}key F5\n{}", frames(30));

    let mut still = Session::open_project(&root, setup(), None).unwrap();
    let png = shot(&mut still, SIZE, &Script::parse(&open).unwrap()).unwrap();
    save("project-open", &png);
    compare("project-scene-camera", &png);
    assert!(
        still.editor.viewport.frame_aspect().is_some()
            && still.editor.viewport.passepartout().is_some(),
        "looking through the scene's camera shows its 16:9 frame"
    );
    let lockup = seen(&still);
    assert_eq!(
        still.editor.selection,
        Some(Item::Object("incoming".to_string()))
    );
    let viewport = &still.editor.viewport;
    let image = viewport.image().unwrap();
    let lens = viewport.lens().unwrap();
    let gizmo = lens.project(DVec3::new(0.0, 0.495, 0.0)).unwrap();
    let centre = image.center();
    assert!(
        (gizmo.x - centre.x).abs() < image.width() * 0.05
            && (gizmo.y - centre.y).abs() < image.height() * 0.05,
        "the group's gizmo sits in the middle of the view: {gizmo:?} in {image:?}"
    );
    assert!(
        lockup.light > 300 && lockup.violet > 2_000 && lockup.red > 200,
        "the first scene shows the wordmark, the mark and the X axis: {lockup:?}"
    );
    let mut orbit = Session::open_project(&root, setup(), None).unwrap();
    take(
        &mut orbit,
        "project-editor-camera",
        &format!("key C\n{}", frames(14)),
    );
    assert_eq!(
        orbit.editor.viewport.image(),
        still.editor.viewport.image(),
        "the 3D view covers the same rect through the editor's camera and the scene's"
    );
    assert_eq!(
        orbit.editor.viewport.passepartout(),
        None,
        "the editor's camera shows no frame"
    );
    let mut scene_game = Session::open_project(&root, setup(), None).unwrap();
    take(&mut scene_game, "project-scene-game", &started);
    assert_eq!(scene_game.editor.play, Play::Playing);
    let shown = seen(&scene_game);
    assert!(
        shown.violet > 1_000 && shown.red > 200,
        "without a game, F5 plays the scene as it is: {shown:?}"
    );
    let [width, height] = scene_game.view.as_ref().unwrap().size();
    let pixels = scene_game.pixels().unwrap();
    let framed = (height as f32 * 16.0 / 9.0 - width as f32).abs() > width as f32 * 0.02;
    assert!(
        !framed || (pixels[..3] == [0, 0, 0] && pixels[pixels.len() - 4..][..3] == [0, 0, 0]),
        "play shows the project's 16:9 frame with the policy's bars around it"
    );

    let calls = Rc::new(Calls::default());
    let made = calls.clone();
    let factory = pfx_editor::factory(move || Hider {
        calls: made.clone(),
    });
    let mut playing = Session::open_project(&root, setup(), Some(factory)).unwrap();
    assert!(playing.plays_a_game());
    take(&mut playing, "project-playing", &started);
    assert_eq!(playing.editor.play, Play::Playing);
    assert_eq!(calls.started.get(), 1);
    assert!(calls.ticks.get() >= 10, "{} ticks", calls.ticks.get());
    let hidden = seen(&playing);
    assert!(
        hidden.violet < 50 && hidden.red < 50,
        "the game's own tick hid the mark, the wordmark and the grid: {hidden:?}"
    );
    assert!(
        playing
            .editor
            .log
            .entries
            .iter()
            .any(|entry| entry.text.starts_with("play: the game's own logic runs"))
    );
    drop(playing);
    assert_eq!(calls.stopped.get(), 1, "closing the editor stops the game");

    let calls = Rc::new(Calls::default());
    let made = calls.clone();
    let factory = pfx_editor::factory(move || Hider {
        calls: made.clone(),
    });
    let mut stopped = Session::open_project(&root, setup(), Some(factory)).unwrap();
    take(
        &mut stopped,
        "project-stopped",
        &format!("{started}key F9\nframe\nkey F8\n{}", frames(15)),
    );
    assert_eq!(stopped.editor.play, Play::Stopped);
    assert_eq!(calls.stopped.get(), 1);
    let back = seen(&stopped);
    assert!(
        back.light.abs_diff(lockup.light) <= lockup.light / 50
            && back.violet.abs_diff(lockup.violet) <= lockup.violet / 50,
        "stop restores the edited frame: {back:?} against {lockup:?}"
    );
    let recordings: Vec<_> = fs::read_dir(root.join("tmp/pfx/recordings"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(recordings.len(), 1, "F9 saved the game's input");
    assert_eq!(snapshot(&root), before, "play writes no file");
    let _ = fs::remove_dir_all(&root);
}

pfx_mod::wasmtime::component::bindgen!({
    path: "tests/wit",
    world: "turns",
    wasmtime_crate: pfx_mod::wasmtime,
});

const TURNS: &str = include_str!("turns.wat");
const REBUILD_AT: u64 = 12;

fn turns(offset: u32) -> Vec<u8> {
    wat::parse_str(TURNS.replace("OFFSET", &offset.to_string())).unwrap()
}

#[derive(Default)]
struct Turned {
    ticks: Cell<u64>,
    seen: Cell<u32>,
}

struct Turner {
    modules: pfx_mod::Modules<(), Turns>,
    turned: Rc<Turned>,
    rebuild: Option<(PathBuf, Vec<u8>)>,
}

impl Turner {
    fn new(root: &Path, turned: Rc<Turned>) -> Turner {
        let host = pfx_mod::Host::new(
            pfx_mod::Api::new("turns", "0.1.0"),
            pfx_mod::Limits::default(),
        )
        .unwrap();
        let files = pfx_project::Project::open(root)
            .unwrap()
            .modules()
            .into_iter()
            .map(|module| (module.id, module.path));
        let modules = pfx_mod::Modules::open(
            host,
            |store, instance| Turns::new(store, instance),
            files,
            |_| (),
        );
        Turner {
            modules,
            turned,
            rebuild: Some((root.join("modules/turns.wasm"), turns(1000))),
        }
    }
}

impl Game for Turner {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let ticks = self.turned.ticks.get() + 1;
        self.turned.ticks.set(ticks);
        if ticks == REBUILD_AT
            && let Some((path, bytes)) = self.rebuild.take()
        {
            fs::write(path, bytes).unwrap();
        }
        self.modules.tick();
        for (_, called) in self.modules.each(|_, turns, store| turns.call_turn(store)) {
            self.turned.seen.set(called.value);
            if called.value >= 1000 {
                for name in ["incoming/badge", "incoming/wordmark", "grid"] {
                    if let Some(object) = world.object(name) {
                        world.set_hidden(object, true);
                    }
                }
            }
        }
    }

    fn modules(&mut self) -> Option<&mut dyn pfx_play::Reload> {
        Some(&mut self.modules)
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_editor_swaps_a_rebuilt_gameplay_module_during_play_and_keeps_its_state() {
    reload_during_play();
}

fn reload_during_play() {
    let root = project("reload");
    let file = root.join("project.toml");
    let text = fs::read_to_string(&file).unwrap().replacen(
        "[authoring.pfx]\n",
        "[authoring.pfx]\nmodules = [\"modules/turns.wasm\"]\n",
        1,
    );
    fs::write(&file, text).unwrap();
    fs::create_dir_all(root.join("modules")).unwrap();
    fs::write(root.join("modules/turns.wasm"), turns(0)).unwrap();

    let turned = Rc::new(Turned::default());
    let made = turned.clone();
    let at = root.clone();
    let factory = pfx_editor::factory(move || Turner::new(&at, made.clone()));
    let mut session = Session::open_project(&root, setup(), Some(factory)).unwrap();
    take(
        &mut session,
        "project-module-reload",
        &format!("{}key F5\n{}", frames(14), frames(40)),
    );
    assert_eq!(session.editor.play, Play::Playing);
    let ticks = turned.ticks.get();
    assert!(ticks > REBUILD_AT + 4, "{ticks} ticks");
    assert_eq!(
        u64::from(turned.seen.get()),
        1000 + ticks,
        "the rebuilt module counts on from the old one's state"
    );
    let log: Vec<&str> = session
        .editor
        .log
        .entries
        .iter()
        .map(|entry| entry.text.as_str())
        .collect();
    for wanted in [
        "modules: watching 1 built module while the game plays",
        "modules: module `turns` started at tick 0",
        "was rebuilt; it swaps in at the next tick",
        "and kept its state (4 bytes through save and restore)",
    ] {
        assert!(
            log.iter().any(|line| line.contains(wanted)),
            "{wanted:?} not in {log:#?}"
        );
    }
    let hidden = seen(&session);
    assert!(
        hidden.violet < 50 && hidden.red < 50,
        "the rebuilt module's turn hid the mark: {hidden:?}"
    );
}
