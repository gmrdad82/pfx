#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use pfx_core::sim::Rng;
use pfx_mod::wasmtime::Store;
use pfx_mod::wasmtime::component::{HasSelf, Instance};
use pfx_mod::{Api, Fuel, Host, Limits, LogLimits, Mod, ModCtx};

pfx_mod::wasmtime::component::bindgen!({
    path: "tests/wit",
    world: "play",
    wasmtime_crate: pfx_mod::wasmtime,
});

pub const GAME_VERSION: &str = "0.1.2";
pub const GAME: &str = "pfx-test:play/game@0.1.0";
pub const PLAY: &str = include_str!("../fixtures/play.wat");
pub const INTRUDER: &str = include_str!("../fixtures/intruder.wat");
pub const MISMATCH: &str = include_str!("../fixtures/mismatch.wat");
pub const SPINNER: &str = include_str!("../fixtures/spinner.wat");
pub const BUDGET: u64 = 1_000_000;

pub struct Game {
    pub rng: Rng,
}

impl pfx_test::play::game::Host for Game {
    fn seed(&mut self) -> u64 {
        self.rng.next_u64()
    }
}

pub fn game(seed: u64) -> Game {
    Game {
        rng: Rng::new(seed),
    }
}

pub fn limits() -> Limits {
    Limits {
        fuel: Fuel::PerCall(BUDGET),
        memory_bytes: 1 << 20,
        stack_bytes: 256 << 10,
        log: Some(LogLimits {
            lines_per_tick: 3,
            line_bytes: 64,
        }),
        ..Limits::default()
    }
}

pub fn host(limits: Limits) -> Host<Game> {
    let mut host = Host::new(Api::new("play", GAME_VERSION).import(GAME), limits).unwrap();
    pfx_test::play::game::add_to_linker::<_, HasSelf<Game>>(
        host.linker(),
        |cx: &mut ModCtx<Game>| &mut cx.game,
    )
    .unwrap();
    host
}

pub fn bind(
    store: &mut Store<ModCtx<Game>>,
    instance: &Instance,
) -> pfx_mod::wasmtime::Result<Play> {
    Play::new(store, instance)
}

pub fn component(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap()
}

pub fn manifest(id: &str, api: &str) -> String {
    format!("id = \"{id}\"\nversion = \"1.0.0\"\napi = \"{api}\"\nentry = \"{id}.wasm\"\n")
}

pub fn start(host: &Host<Game>, id: &str, seed: u64) -> Mod<Game, Play> {
    let package = host
        .load(manifest(id, "0.1.0").as_bytes(), &component(PLAY))
        .unwrap();
    let started = Mod::start(host, package, bind, game(seed));
    assert!(started.running(), "{:?}", started.stopped());
    started
}

pub fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("pfx-mod")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

pub fn write_mod(mods: &Path, id: &str, manifest: &str, component: &[u8]) {
    let dir = mods.join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("mod.toml"), manifest).unwrap();
    fs::write(dir.join(format!("{id}.wasm")), component).unwrap();
}

pub const SAVE: &str = r#"
    (func (export "save") (result i32)
      (i32.store (i32.const 128) (global.get $count))
      (i32.store (i32.const 64) (i32.const 128))
      (i32.store (i32.const 68) (i32.const 4))
      (i32.const 64))
"#;

pub const RESTORE: &str = r#"
    (func (export "restore") (param $at i32) (param $len i32)
      (if (i32.eq (local.get $len) (i32.const 4))
        (then (global.set $count (i32.load (local.get $at))))))
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      (i32.const 256))
"#;

pub const BROKEN_RESTORE: &str = r#"
    (func (export "restore") (param $at i32) (param $len i32)
      unreachable)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      (i32.const 256))
"#;

pub const LIFT_SAVE: &str = r#"
  (func (export "save") (result (list u8))
    (canon lift (core func $main "save") (memory $memory "memory")))
"#;

pub const LIFT_RESTORE: &str = r#"
  (func (export "restore") (param "state" (list u8))
    (canon lift (core func $main "restore") (memory $memory "memory") (realloc (func $main "realloc"))))
"#;

pub fn play_with(core: &[&str], lifts: &[&str]) -> Vec<u8> {
    let anchor = "  )\n  (core instance $main";
    let at = PLAY.find(anchor).unwrap();
    let end = PLAY.trim_end().rfind(')').unwrap();
    let text = format!(
        "{}{}{}{})\n",
        &PLAY[..at],
        core.concat(),
        &PLAY[at..end],
        lifts.concat()
    );
    component(&text)
}

pub fn saving() -> Vec<u8> {
    play_with(&[SAVE, RESTORE], &[LIFT_SAVE, LIFT_RESTORE])
}
