use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pfx_core::clock::Tick;
use pfx_input::{Input, Key};
use pfx_mod::wasmtime::component::{Instance, TypedFunc};
use pfx_mod::wasmtime::{self, Store, StoreContextMut};
use pfx_mod::{
    Api, Event, Host, Level, Limits, LogLimits, ModCtx, Modules, Notice, Reload, Swapped, What,
};
use pfx_play::{Game, Label, ObjectId, SceneGame, Ui, UiFrame, World};

pub const WIT: &str = include_str!("../wit/play.wit");
pub const VERSION: &str = "0.1.0";
pub const SCENE: &str = "pfx:play/scene@0.1.0";
pub const INPUT: &str = "pfx:play/input@0.1.0";
pub const CLOCK: &str = "pfx:play/clock@0.1.0";
pub const HUD: &str = "pfx:play/hud@0.1.0";
pub const HUD_LINES: u32 = 8;
pub const TEXT_BYTES: usize = 256;
pub const NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug)]
enum Change {
    Move(u32, [f32; 3]),
    Hide(u32, bool),
}

#[derive(Default)]
struct Bridge {
    names: BTreeMap<String, u32>,
    keys: Vec<(String, Key)>,
    at: Vec<[f32; 3]>,
    found: Vec<bool>,
    held: Vec<Key>,
    ticks: u64,
    step: f32,
    changes: Vec<Change>,
    hud: Vec<String>,
}

impl Bridge {
    fn object(&self, object: u32) -> wasmtime::Result<usize> {
        let index = object as usize;
        if self.found.get(index).copied().unwrap_or(false) {
            Ok(index)
        } else {
            Err(wasmtime::Error::msg(format!(
                "object {object} was never returned by find"
            )))
        }
    }
}

struct Exports {
    start: TypedFunc<(), ()>,
    tick: TypedFunc<(), ()>,
}

fn bind(store: &mut Store<ModCtx<Bridge>>, instance: &Instance) -> wasmtime::Result<Exports> {
    Ok(Exports {
        start: instance.get_typed_func(&mut *store, "start")?,
        tick: instance.get_typed_func(&mut *store, "tick")?,
    })
}

type Cx<'a> = StoreContextMut<'a, ModCtx<Bridge>>;

fn charge(cx: &mut Cx<'_>, bytes: usize) -> wasmtime::Result<()> {
    let fuel = cx.get_fuel()?;
    let Some(left) = fuel.checked_sub(bytes as u64) else {
        cx.set_fuel(0)?;
        return Err(wasmtime::Trap::OutOfFuel.into());
    };
    cx.set_fuel(left)
}

fn capped(what: &str, text: &str, cap: usize) -> wasmtime::Result<()> {
    if text.len() > cap {
        return Err(wasmtime::Error::msg(format!(
            "{what} of {} bytes is longer than the {cap} a module may pass",
            text.len()
        )));
    }
    Ok(())
}

fn cut(text: &str, cap: usize) -> String {
    let mut end = text.len().min(cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn link(host: &mut Host<Bridge>) -> wasmtime::Result<()> {
    let linker = host.linker();
    let mut scene = linker.instance(SCENE)?;
    scene.func_wrap("find", |mut cx: Cx<'_>, (name,): (String,)| {
        charge(&mut cx, name.len())?;
        capped("an object name", &name, NAME_BYTES)?;
        let bridge = &mut cx.data_mut().game;
        let found = bridge.names.get(&name).copied();
        if let Some(object) = found {
            bridge.found[object as usize] = true;
        }
        Ok((found,))
    })?;
    scene.func_wrap("position", |cx: Cx<'_>, (object,): (u32,)| {
        let bridge = &cx.data().game;
        let [x, y, z] = bridge.at[bridge.object(object)?];
        Ok(((x, y, z),))
    })?;
    scene.func_wrap(
        "move-to",
        |mut cx: Cx<'_>, (object, x, y, z): (u32, f32, f32, f32)| {
            let bridge = &mut cx.data_mut().game;
            let index = bridge.object(object)?;
            if !(x.is_finite() && y.is_finite() && z.is_finite()) {
                return Err(wasmtime::Error::msg(format!(
                    "move-to object {object}: a position needs finite numbers"
                )));
            }
            bridge.at[index] = [x, y, z];
            bridge.changes.push(Change::Move(object, [x, y, z]));
            Ok(())
        },
    )?;
    scene.func_wrap(
        "set-hidden",
        |mut cx: Cx<'_>, (object, hidden): (u32, bool)| {
            let bridge = &mut cx.data_mut().game;
            bridge.object(object)?;
            bridge.changes.push(Change::Hide(object, hidden));
            Ok(())
        },
    )?;
    let mut input = linker.instance(INPUT)?;
    input.func_wrap("held", |mut cx: Cx<'_>, (key,): (String,)| {
        charge(&mut cx, key.len())?;
        let bridge = &cx.data().game;
        let Some((_, key)) = bridge.keys.iter().find(|(name, _)| *name == key) else {
            return Err(wasmtime::Error::msg(format!(
                "held: unknown key `{}`; keys are named as pfx names them, such as Left, Space or A",
                cut(&key, NAME_BYTES)
            )));
        };
        Ok((bridge.held.contains(key),))
    })?;
    let mut clock = linker.instance(CLOCK)?;
    clock.func_wrap("ticks", |cx: Cx<'_>, (): ()| Ok((cx.data().game.ticks,)))?;
    clock.func_wrap("step", |cx: Cx<'_>, (): ()| Ok((cx.data().game.step,)))?;
    let mut hud = linker.instance(HUD)?;
    hud.func_wrap("text", |mut cx: Cx<'_>, (line, text): (u32, String)| {
        charge(&mut cx, text.len())?;
        if line >= HUD_LINES {
            return Err(wasmtime::Error::msg(format!(
                "hud line {line} is past the {HUD_LINES} lines a module has"
            )));
        }
        let lines = &mut cx.data_mut().game.hud;
        let line = line as usize;
        if lines.len() <= line {
            lines.resize(line + 1, String::new());
        }
        lines[line] = cut(&text, TEXT_BYTES);
        Ok(())
    })?;
    Ok(())
}

pub fn api() -> Api {
    Api::new("play", VERSION)
        .import(SCENE)
        .import(INPUT)
        .import(CLOCK)
        .import(HUD)
}

pub fn limits() -> Limits {
    Limits {
        log: Some(LogLimits::default()),
        ..Limits::default()
    }
}

pub fn embed(core: &[u8]) -> Result<Vec<u8>, String> {
    let mut resolve = wit_parser::Resolve::default();
    resolve
        .push_str("log.wit", pfx_mod::LOG_WIT)
        .map_err(|error| format!("{error:#}"))?;
    let package = resolve
        .push_str("play.wit", WIT)
        .map_err(|error| format!("{error:#}"))?;
    let world = resolve
        .select_world(&[package], Some("game"))
        .map_err(|error| format!("{error:#}"))?;
    let mut bytes = core.to_vec();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .map_err(|error| format!("{error:#}"))?;
    Ok(bytes)
}

pub fn module_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let id: String = stem
        .chars()
        .take(64)
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '-' | '_') => c,
            _ => '-',
        })
        .collect();
    if pfx_mod::valid_id(&id) {
        id
    } else {
        "game".to_string()
    }
}

pub struct ModuleGame {
    id: String,
    path: PathBuf,
    modules: Modules<Bridge, Exports>,
    ids: Vec<ObjectId>,
    family: Option<String>,
    notices: Vec<Notice>,
    refused: Option<String>,
}

impl ModuleGame {
    pub fn open(path: impl Into<PathBuf>) -> Result<ModuleGame, String> {
        ModuleGame::open_with(path, limits())
    }

    pub fn open_with(path: impl Into<PathBuf>, limits: Limits) -> Result<ModuleGame, String> {
        let path = path.into();
        let id = module_id(&path);
        let mut host = Host::new(api(), limits).map_err(|error| error.to_string())?;
        link(&mut host).map_err(|error| format!("{error:#}"))?;
        let modules = Modules::open(host, bind, [(id.clone(), path.clone())], |_| {
            Bridge::default()
        });
        let mut game = ModuleGame {
            id,
            path,
            modules,
            ids: Vec::new(),
            family: None,
            notices: Vec::new(),
            refused: None,
        };
        game.events();
        Ok(game)
    }

    pub fn font(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn running(&self) -> bool {
        self.modules.running(&self.id)
    }

    pub fn hud(&self) -> Vec<String> {
        self.modules
            .get(&self.id)
            .map(|module| module.game().hud.clone())
            .unwrap_or_default()
    }

    fn events(&mut self) -> bool {
        let mut fresh = false;
        for event in self.modules.take_events() {
            match &event.what {
                What::Started | What::Swapped(Swapped::Fresh(_)) => fresh = true,
                What::Missing(_) | What::Failed(_) if !self.running() => {
                    self.refused.get_or_insert_with(|| event.to_string());
                }
                _ => {}
            }
            self.notice(&event);
        }
        fresh
    }

    fn notice(&mut self, event: &Event) {
        self.notices.push(Notice {
            level: event.level(),
            text: event.to_string(),
        });
    }

    fn fill(&mut self, world: &World, tick: Option<&Tick>, all: bool) {
        let Some(module) = self.modules.get_mut(&self.id) else {
            return;
        };
        let bridge = module.game_mut();
        if all {
            bridge.names = self
                .ids
                .iter()
                .enumerate()
                .map(|(index, id)| (world.name(*id).to_string(), index as u32))
                .collect();
            bridge.keys = Key::ALL
                .iter()
                .map(|key| (format!("{key:?}"), *key))
                .collect();
            bridge.found = vec![false; self.ids.len()];
            bridge.at = self.ids.iter().map(|id| world.at(*id)).collect();
        } else {
            for (index, id) in self.ids.iter().enumerate() {
                if bridge.found[index] {
                    bridge.at[index] = world.at(*id);
                }
            }
        }
        bridge.ticks = world.ticks();
        bridge.step = match tick {
            Some(tick) => tick.dt,
            None => (1.0 / world.clock().tick_rate()) as f32,
        };
    }

    fn held(&mut self, input: &Input) {
        if let Some(module) = self.modules.get_mut(&self.id) {
            let bridge = module.game_mut();
            bridge.held = Key::ALL
                .iter()
                .copied()
                .filter(|key| input.key_held(*key))
                .collect();
        }
    }

    fn apply(&mut self, world: &mut World) {
        let Some(module) = self.modules.get_mut(&self.id) else {
            return;
        };
        for change in std::mem::take(&mut module.game_mut().changes) {
            match change {
                Change::Move(object, at) => {
                    world.move_to(self.ids[object as usize], at);
                }
                Change::Hide(object, hidden) => {
                    world.set_hidden(self.ids[object as usize], hidden);
                }
            }
        }
        for line in module.take_log() {
            self.notices.push(Notice {
                level: Level::Info,
                text: format!("module `{}`: {line}", self.id),
            });
        }
    }

    fn call(&mut self, world: &mut World, start: bool) {
        let called = self.modules.call(&self.id, |exports, store| {
            if start {
                exports.start.call(store, ())
            } else {
                exports.tick.call(store, ())
            }
        });
        self.apply(world);
        self.events();
        if let Some(Err(error)) = called {
            world.fail(error);
        }
    }

    fn begin(&mut self, world: &mut World, tick: Option<&Tick>) {
        self.fill(world, tick, true);
        self.call(world, true);
    }
}

impl Game for ModuleGame {
    fn start(&mut self, world: &mut World) {
        SceneGame.start(world);
        self.ids = world.ids().collect();
        if !self.running() {
            let refused = self
                .refused
                .clone()
                .unwrap_or_else(|| format!("module `{}` did not start", self.id));
            world.fail(refused);
            return;
        }
        self.begin(world, None);
    }

    fn tick(&mut self, world: &mut World, tick: &Tick, input: &Input) {
        SceneGame.tick(world, tick, input);
        self.modules.tick();
        if self.events() && self.running() {
            self.begin(world, Some(tick));
        }
        if !self.running() {
            let error = self
                .modules
                .get(&self.id)
                .and_then(|module| module.stopped().cloned());
            match error {
                Some(error) => world.fail(error),
                None => world.fail(format!("module `{}` is not running", self.id)),
            }
            return;
        }
        self.fill(world, Some(tick), false);
        self.held(input);
        self.call(world, false);
    }

    fn ui(&mut self, _world: &World, frame: &UiFrame) -> Ui {
        let mut ui = Ui::new();
        let Some(family) = &self.family else {
            return ui;
        };
        let size = 28.0 * frame.ui_scale;
        let [left, top, _, _] = frame.safe;
        for (line, text) in self.hud().iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let at = [left + size, top + size * (1.0 + 1.4 * line as f32)];
            ui.label(Label::new(text.clone(), family.clone(), size, at));
        }
        ui
    }

    fn modules(&mut self) -> Option<&mut dyn Reload> {
        Some(self)
    }
}

impl Reload for ModuleGame {
    fn files(&self) -> Vec<PathBuf> {
        self.modules.files()
    }

    fn changed(&mut self, file: &Path) {
        self.modules.changed(file);
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.events();
        std::mem::take(&mut self.notices)
    }
}
