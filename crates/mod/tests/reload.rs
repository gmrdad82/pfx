mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use pfx_mod::wasmtime::Store;
use pfx_mod::wasmtime::Trap;
use pfx_mod::wasmtime::component::{Instance, TypedFunc};
use pfx_mod::{
    Api, Cause, Fresh, Host, Level, Limits, Mod, ModCtx, Modules, Refusal, Reload, Swapped, What,
};

const EXITS: &str = include_str!("fixtures/exits.wat");

type Run = TypedFunc<(u32, u32), (u32,)>;

fn bind_run(store: &mut Store<ModCtx<()>>, instance: &Instance) -> pfx_mod::wasmtime::Result<Run> {
    instance.get_typed_func::<(u32, u32), (u32,)>(store, "run")
}

fn package(host: &Host<Game>, id: &str, wasm: &[u8]) -> pfx_mod::Package<Game> {
    host.load_module(id, wasm).unwrap()
}

fn bump(m: &mut Mod<Game, Play>, by: u32) -> u32 {
    m.call(|play, store| play.call_bump(store, by))
        .unwrap()
        .value
}

#[test]
fn a_swap_keeps_state_through_save_and_restore() {
    let host = host(limits());
    let mut m = Mod::start(&host, package(&host, "brain", &saving()), bind, game(1));
    assert_eq!(bump(&mut m, 5), 5);
    let swapped = m.reload(&host, package(&host, "brain", &saving())).unwrap();
    assert_eq!(swapped, Swapped::Kept { bytes: 4 });
    assert_eq!(bump(&mut m, 1), 6);
    assert!(swapped.to_string().contains("kept its state"), "{swapped}");
}

#[test]
fn a_module_without_save_or_restore_restarts_clean() {
    let host = host(limits());
    let mut m = Mod::start(
        &host,
        package(&host, "brain", &component(PLAY)),
        bind,
        game(1),
    );
    bump(&mut m, 5);
    let swapped = m.reload(&host, package(&host, "brain", &saving())).unwrap();
    assert_eq!(swapped, Swapped::Fresh(Fresh::NoSave));
    assert_eq!(bump(&mut m, 1), 1);
    bump(&mut m, 3);
    let swapped = m
        .reload(&host, package(&host, "brain", &component(PLAY)))
        .unwrap();
    assert_eq!(swapped, Swapped::Fresh(Fresh::NoRestore));
    assert_eq!(bump(&mut m, 2), 2);
    assert!(swapped.to_string().contains("restarted clean"), "{swapped}");
}

#[test]
fn a_failing_restore_stops_the_module_and_a_fixed_build_starts_it_again() {
    let host = host(limits());
    let mut m = Mod::start(&host, package(&host, "brain", &saving()), bind, game(1));
    bump(&mut m, 5);
    let broken = play_with(&[SAVE, BROKEN_RESTORE], &[LIFT_SAVE, LIFT_RESTORE]);
    let error = m
        .reload(&host, package(&host, "brain", &broken))
        .unwrap_err();
    assert_eq!(error.cause, Cause::Trap(Trap::UnreachableCodeReached));
    assert!(!m.running());
    assert_eq!(
        m.call(|play, store| play.call_bump(store, 1))
            .unwrap_err()
            .cause,
        Cause::Disabled
    );
    let swapped = m.reload(&host, package(&host, "brain", &saving())).unwrap();
    assert_eq!(swapped, Swapped::Fresh(Fresh::Stopped));
    assert_eq!(bump(&mut m, 1), 1);
}

#[test]
fn the_depth_limit_stops_deep_calls_with_a_typed_cause() {
    let host = host(Limits {
        depth: 10,
        ..limits()
    });
    let mut m = start(&host, "deep", 1);
    let error = m.call(|play, store| play.call_fall(store, 9)).unwrap_err();
    assert_eq!(error.cause, Cause::Trap(Trap::UnreachableCodeReached));
    let mut m = start(&host, "deep", 1);
    let error = m.call(|play, store| play.call_fall(store, 10)).unwrap_err();
    assert_eq!(error.cause, Cause::Depth);
    assert!(error.at.is_some());
    assert!(
        error
            .to_string()
            .contains("deeper than its call-depth limit"),
        "{error}"
    );
}

#[test]
fn every_way_out_of_a_function_leaves_the_depth_count_balanced() {
    let host = Host::<()>::new(
        Api::new("exits", "0.1.0"),
        Limits {
            depth: 10,
            ..Limits::default()
        },
    )
    .unwrap();
    let manifest = manifest("exits", "0.1.0");
    let package = host.load(manifest.as_bytes(), &component(EXITS)).unwrap();
    let mut m = Mod::start(&host, package.clone(), bind_run, ());
    let run = |m: &mut Mod<(), Run>, k: u32, d: u32| m.call(|run, store| run.call(store, (k, d)));
    assert_eq!(run(&mut m, 0, 8).unwrap().value, (8,));
    assert_eq!(run(&mut m, 6, 0).unwrap().value, (2082,));
    assert_eq!(run(&mut m, 60, 8).unwrap().value, (20828,));
    assert_eq!(run(&mut m, 600, 8).unwrap().value, (208_208,));
    assert_eq!(run(&mut m, 0, 9).unwrap_err().cause, Cause::Depth);
    let mut m = Mod::start(&host, package, bind_run, ());
    assert_eq!(run(&mut m, 0, 8).unwrap().value, (8,));
}

const TINY_WIT: &str = r#"
package pfx-test:tiny@0.1.0;

world tiny {
  export bump: func(by: u32) -> u32;
}
"#;

const TINY_CORE: &str = r#"
(module
  (global $count (mut i32) (i32.const 0))
  (func (export "bump") (param $by i32) (result i32)
    (global.set $count (i32.add (global.get $count) (local.get $by)))
    (global.get $count))
)
"#;

#[test]
fn a_core_module_with_its_component_type_loads_as_a_gameplay_module() {
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve.push_str("tiny.wit", TINY_WIT).unwrap();
    let world = resolve.select_world(&[package], Some("tiny")).unwrap();
    let mut core = wat::parse_str(TINY_CORE).unwrap();
    wit_component::embed_component_metadata(
        &mut core,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    let host = Host::<()>::new(Api::new("tiny", "0.1.0"), Limits::default()).unwrap();
    let loaded = host.load_module("tiny", &core).unwrap();
    assert_eq!(loaded.manifest.id, "tiny");
    let bind = |store: &mut Store<ModCtx<()>>, instance: &Instance| {
        instance.get_typed_func::<(u32,), (u32,)>(store, "bump")
    };
    let mut m = Mod::start(&host, loaded, bind, ());
    let mut call = |by| m.call(|f, store| f.call(store, (by,))).unwrap().value.0;
    assert_eq!((call(2), call(3)), (2, 5));
    let bare = wat::parse_str(TINY_CORE).unwrap();
    assert!(matches!(
        host.load_module("tiny", &bare),
        Err(Refusal::Module(_))
    ));
    assert!(matches!(
        host.load_module("Not An Id", &core),
        Err(Refusal::Id { .. })
    ));
}

fn files(dir: &std::path::Path, ids: &[&str]) -> Vec<(String, PathBuf)> {
    ids.iter()
        .map(|id| (id.to_string(), dir.join(format!("{id}.wasm"))))
        .collect()
}

fn modules(dir: &std::path::Path, ids: &[&str]) -> Modules<Game, Play> {
    Modules::open(host(limits()), bind, files(dir, ids), |_| game(1))
}

fn bumps(modules: &mut Modules<Game, Play>, by: u32) -> Vec<(String, u32)> {
    modules
        .each(|_, play, store| play.call_bump(store, by))
        .into_iter()
        .map(|(id, called)| (id, called.value))
        .collect()
}

#[test]
fn gameplay_modules_load_from_their_files_and_swap_at_a_tick_boundary() {
    let dir = folder("gameplay-swap");
    fs::write(dir.join("brain.wasm"), saving()).unwrap();
    let mut modules = modules(&dir, &["brain", "absent"]);
    let opened = modules.take_events();
    assert_eq!(opened.len(), 2, "{opened:?}");
    assert!(
        opened
            .iter()
            .any(|e| e.id == "brain" && e.what == What::Started)
    );
    assert!(
        opened
            .iter()
            .any(|e| e.id == "absent" && matches!(e.what, What::Missing(Refusal::Read(_))))
    );
    modules.tick();
    assert_eq!(bumps(&mut modules, 4), [("brain".to_string(), 4)]);
    fs::write(dir.join("brain.wasm"), saving()).unwrap();
    modules.changed(&dir.join("brain.wasm"));
    assert_eq!(bumps(&mut modules, 1), [("brain".to_string(), 5)]);
    assert!(modules.take_events().is_empty(), "nothing swaps mid-tick");
    modules.tick();
    let swapped = modules.take_events();
    assert!(
        swapped.is_empty(),
        "the same bytes do not swap: {swapped:?}"
    );
    let rebuilt = play_with(
        &[SAVE, RESTORE, "    (func (export \"spare\"))\n"],
        &[LIFT_SAVE, LIFT_RESTORE],
    );
    fs::write(dir.join("brain.wasm"), &rebuilt).unwrap();
    modules.changed(&dir.join("brain.wasm"));
    modules.tick();
    let swapped = modules.take_events();
    assert_eq!(
        swapped,
        [pfx_mod::Event {
            tick: 2,
            id: "brain".to_string(),
            what: What::Swapped(Swapped::Kept { bytes: 4 }),
        }]
    );
    assert_eq!(bumps(&mut modules, 1), [("brain".to_string(), 6)]);
    assert_eq!(
        modules.files(),
        files(&dir, &["brain", "absent"])
            .into_iter()
            .map(|(_, p)| p)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_gameplay_module_without_save_and_restore_restarts_clean_with_a_notice() {
    let dir = folder("gameplay-clean");
    fs::write(dir.join("brain.wasm"), component(PLAY)).unwrap();
    let mut modules = modules(&dir, &["brain"]);
    modules.tick();
    bumps(&mut modules, 7);
    let rebuilt = play_with(&["    (func (export \"spare\"))\n"], &[]);
    fs::write(dir.join("brain.wasm"), &rebuilt).unwrap();
    modules.changed(&dir.join("brain.wasm"));
    modules.notices();
    modules.tick();
    let notices = modules.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].level, Level::Warn);
    assert!(
        notices[0].text.contains("restarted clean") && notices[0].text.contains("no `save`"),
        "{}",
        notices[0].text
    );
    assert_eq!(bumps(&mut modules, 1), [("brain".to_string(), 1)]);
}

#[test]
fn a_failing_gameplay_module_is_disabled_and_the_game_keeps_its_others() {
    let dir = folder("gameplay-fail");
    fs::write(dir.join("brain.wasm"), component(PLAY)).unwrap();
    fs::write(dir.join("legs.wasm"), component(PLAY)).unwrap();
    let mut modules = modules(&dir, &["brain", "legs"]);
    modules.tick();
    modules.take_events();
    let failed = modules
        .call("brain", |play, store| play.call_fall(store, 3))
        .unwrap()
        .unwrap_err();
    assert_eq!(failed.cause, Cause::Trap(Trap::UnreachableCodeReached));
    let events = modules.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level(), Level::Error);
    assert_eq!(events[0].what, What::Failed(failed));
    assert!(!modules.running("brain"));
    modules.tick();
    assert_eq!(bumps(&mut modules, 2), [("legs".to_string(), 2)]);
    assert!(modules.take_events().is_empty());
    fs::write(dir.join("brain.wasm"), component(PLAY)).unwrap();
    modules.changed(&dir.join("brain.wasm"));
    modules.tick();
    assert!(modules.running("brain"), "{:?}", modules.take_events());
}

#[test]
fn a_rebuilt_module_that_does_not_load_leaves_the_running_one() {
    let dir = folder("gameplay-refused");
    fs::write(dir.join("brain.wasm"), saving()).unwrap();
    let mut modules = modules(&dir, &["brain"]);
    modules.tick();
    bumps(&mut modules, 3);
    modules.take_events();
    fs::write(dir.join("brain.wasm"), &saving()[..40]).unwrap();
    modules.changed(&dir.join("brain.wasm"));
    modules.tick();
    let events = modules.take_events();
    assert!(
        matches!(events.as_slice(), [e] if matches!(e.what, What::Refused(_))),
        "{events:?}"
    );
    assert!(events[0].to_string().contains("the running one carries on"));
    assert_eq!(bumps(&mut modules, 1), [("brain".to_string(), 4)]);
}

#[test]
fn a_missing_module_starts_once_its_file_appears() {
    let dir = folder("gameplay-late");
    let mut modules = modules(&dir, &["brain"]);
    modules.tick();
    assert!(bumps(&mut modules, 1).is_empty());
    let notices = modules.notices();
    assert_eq!(notices[0].level, Level::Warn);
    fs::write(dir.join("brain.wasm"), component(PLAY)).unwrap();
    modules.changed(&dir.join("brain.wasm"));
    modules.tick();
    assert_eq!(
        modules.take_events()[0].what,
        What::Started,
        "a module that was never running starts"
    );
    assert_eq!(bumps(&mut modules, 2), [("brain".to_string(), 2)]);
}
