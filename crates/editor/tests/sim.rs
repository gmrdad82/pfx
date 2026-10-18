use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pfx_core::clock::Tick;
use pfx_editor::session::{Session, Setup};
use pfx_game::Config;
use pfx_input::Input;
use pfx_mod::{Api, Host, Limits, Modules};
use pfx_play::World;

pfx_mod::wasmtime::component::bindgen!({
    path: "tests/sim",
    world: "game",
    wasmtime_crate: pfx_mod::wasmtime,
});

const SIM: &str = include_str!("sim/sim.wat");
const SEED: u64 = 7;
const STEP: f32 = 1.0 / 60.0;
const BEFORE: usize = 20;
const AFTER: usize = 20;
const REBUILT: u32 = 1_000_000;

fn sim(offset: u32) -> Vec<u8> {
    wat::parse_str(SIM.replace("OFFSET", &offset.to_string())).unwrap()
}

#[derive(Default)]
struct Seen {
    started: Cell<u32>,
    ticks: Cell<u64>,
    orders: Cell<u64>,
    events: RefCell<Vec<[u32; 2]>>,
    snapshot: RefCell<Vec<u8>>,
}

struct Simulated {
    modules: Modules<(), Game>,
    seen: Rc<Seen>,
}

impl Simulated {
    fn new(root: &Path, seen: Rc<Seen>) -> Simulated {
        let host = Host::new(Api::new("sim", "1.0.0"), Limits::default()).unwrap();
        let files = pfx_project::Project::open(root)
            .unwrap()
            .modules()
            .into_iter()
            .map(|module| (module.id, module.path));
        let modules = Modules::open(
            host,
            |store, instance| Game::new(store, instance),
            files,
            |_| (),
        );
        Simulated { modules, seen }
    }
}

impl pfx_play::Game for Simulated {
    fn start(&mut self, _world: &mut World) {
        self.seen.started.set(self.seen.started.get() + 1);
        self.modules
            .each(|_, game, store| game.test_sim_sim().call_new(store, SEED));
    }

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        let ticks = self.seen.ticks.get() + 1;
        self.seen.ticks.set(ticks);
        self.modules.tick();
        let order = ticks as u32;
        for (_, called) in self
            .modules
            .each(|_, game, store| game.test_sim_sim().call_step(store, order))
        {
            self.seen
                .orders
                .set(self.seen.orders.get() + u64::from(order));
            let [steps, value] = called.value[..] else {
                panic!("two values in an event, got {:?}", called.value);
            };
            self.seen.events.borrow_mut().push([steps, value]);
            if let Some(object) = world.object("crate") {
                let at = world.at(object);
                world.move_to(object, [steps as f32 * 0.01, at[1], at[2]]);
            }
        }
        if let Some((_, called)) = self
            .modules
            .each(|_, game, store| game.test_sim_sim().call_snapshot(store))
            .pop()
        {
            *self.seen.snapshot.borrow_mut() = called.value;
        }
    }

    fn modules(&mut self) -> Option<&mut dyn pfx_play::Reload> {
        Some(&mut self.modules)
    }
}

fn project(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("editor-sim")
        .join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("modules")).unwrap();
    let scenes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
    for file in [
        "room.scene.toml",
        "lights.scene.toml",
        "materials.toml",
        "ball.gltf",
        "block.gltf",
        "pillar.gltf",
    ] {
        fs::copy(scenes.join(file), root.join(file)).unwrap();
    }
    fs::write(
        root.join("project.toml"),
        "format = 1\n\n[project]\nname = \"sim\"\nscene = \"room.scene.toml\"\n\n[authoring.pfx]\nmodules = [\"modules/sim.wasm\"]\n",
    )
    .unwrap();
    fs::write(root.join("modules/sim.wasm"), sim(0)).unwrap();
    root
}

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                found.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

fn logged(session: &Session) -> Vec<String> {
    session
        .editor
        .log
        .entries
        .iter()
        .map(|entry| entry.text.clone())
        .collect()
}

#[test]
fn a_game_defined_world_plays_in_the_editor_reloads_its_rebuilt_module_and_stop_restores_the_scene()
{
    let root = project("play");
    let module = root.join("modules/sim.wasm");
    let seen = Rc::new(Seen::default());
    let made = seen.clone();
    let at = root.clone();
    let factory = pfx_editor::factory(move || Simulated::new(&at, made.clone()));
    let setup = Setup {
        pgpu: false,
        audio: false,
    };
    let mut session = Session::open_game(
        &root,
        setup,
        factory,
        Config::new("sim", "0.0.0", "room.scene.toml"),
    )
    .unwrap();
    assert!(session.plays_a_game());
    let edited = session.editor.scene().clone();
    let before = files(&root);

    session.play_headless().unwrap();
    assert_eq!(seen.started.get(), 1);
    for _ in 0..BEFORE {
        assert!(session.frame_headless(STEP).unwrap());
    }
    let ticked = seen.ticks.get();
    assert!(ticked >= BEFORE as u64 - 1, "{ticked} ticks");
    let last = *seen.events.borrow().last().unwrap();
    assert_eq!(last, [ticked as u32, (SEED + seen.orders.get()) as u32]);

    fs::write(&module, sim(REBUILT)).unwrap();
    for _ in 0..AFTER {
        assert!(session.frame_headless(STEP).unwrap());
    }
    let ticks = seen.ticks.get();
    assert!(ticks >= ticked + AFTER as u64 - 2, "{ticks} ticks");
    let last = *seen.events.borrow().last().unwrap();
    assert_eq!(
        last,
        [ticks as u32, (SEED + seen.orders.get()) as u32 + REBUILT],
        "the rebuilt module counts on from the old one's state"
    );
    let mut state = (SEED + seen.orders.get()).to_le_bytes().to_vec();
    state.extend_from_slice(&(ticks as u32).to_le_bytes());
    assert_eq!(*seen.snapshot.borrow(), state);

    let world = session.editor.module.session().unwrap().world();
    let moved = world.at(world.object("crate").unwrap());
    assert!((moved[0] - ticks as f32 * 0.01).abs() < 1e-4, "{moved:?}");

    let log = logged(&session);
    for wanted in [
        "modules: watching 1 built module while the game plays",
        "modules: module `sim` started at tick 0",
        "modules/sim.wasm was rebuilt; it swaps in at the next tick",
        "and kept its state (12 bytes through save and restore)",
    ] {
        assert!(
            log.iter().any(|line| line.contains(wanted)),
            "{wanted:?} not in {log:#?}"
        );
    }

    let stopped = session.stop_headless().unwrap();
    assert!(!session.editor.module.is_running());
    assert_eq!(stopped.scene, edited, "stop hands back the edited scene");
    assert_eq!(*session.editor.scene(), edited);
    let mut after = files(&root);
    let mut wanted = before;
    for (path, bytes) in wanted.iter_mut() {
        if *path == module {
            *bytes = sim(REBUILT);
        }
    }
    after.retain(|(path, _)| !path.starts_with(root.join("tmp")));
    assert_eq!(after, wanted, "play writes no file of the project");

    session.play_headless().unwrap();
    assert_eq!(seen.started.get(), 2, "every play makes a new game");
    let _ = fs::remove_dir_all(&root);
}
