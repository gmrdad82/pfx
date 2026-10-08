mod common;

use std::fs;

use common::*;
use pfx_mod::wasmtime::Trap;
use pfx_mod::{Cause, Fuel, Host, LOG_WIT, Limit, Limits, Mod, ModSet, Refusal};

#[test]
fn loads_a_mods_folder_in_id_order_and_calls_each_mod() {
    let host = host(limits());
    let mods = folder("load");
    let play = component(PLAY);
    write_mod(&mods, "beta", &manifest("beta", "0.1.0"), &play);
    write_mod(&mods, "alpha", &manifest("alpha", "0.1.0"), &play);
    fs::write(mods.join("notes.txt"), "not a mod").unwrap();
    let loaded = host.load_dir(&mods);
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    let ids: Vec<&str> = loaded
        .packages
        .iter()
        .map(|p| p.manifest.id.as_str())
        .collect();
    assert_eq!(ids, ["alpha", "beta"]);
    let (mut set, failed) = ModSet::start(&host, loaded.packages, bind, |_| game(1));
    assert!(failed.is_empty());
    for m in set.mods_mut() {
        let first = m.call(|play, store| play.call_bump(store, 2)).unwrap();
        let second = m.call(|play, store| play.call_bump(store, 3)).unwrap();
        assert_eq!((first.value, second.value), (2, 5));
        assert!(first.fuel > 0);
        assert_eq!(first.fuel, second.fuel);
    }
}

#[test]
fn a_missing_mods_folder_is_an_empty_set() {
    let host = host(limits());
    let loaded = host.load_dir(&folder("missing").join("mods"));
    assert!(loaded.packages.is_empty() && loaded.refused.is_empty());
    let (set, _) = ModSet::start(&host, loaded.packages, bind, |_| game(1));
    assert!(set.is_empty());
}

#[test]
fn refuses_imports_outside_the_world() {
    let host = host(limits());
    let refusal = host
        .load(
            manifest("intruder", "0.1.0").as_bytes(),
            &component(INTRUDER),
        )
        .err()
        .unwrap();
    assert_eq!(
        refusal,
        Refusal::Import {
            name: "pfx-test:files/read@0.1.0".to_string()
        }
    );
    assert!(
        refusal
            .to_string()
            .contains("not part of the game's mod API")
    );
}

#[test]
fn refuses_the_log_import_when_the_game_leaves_it_off() {
    let host = host(Limits {
        log: None,
        ..limits()
    });
    let refusal = host
        .load(manifest("play", "0.1.0").as_bytes(), &component(PLAY))
        .err()
        .unwrap();
    assert_eq!(
        refusal,
        Refusal::Import {
            name: pfx_mod::LOG_INTERFACE.to_string()
        }
    );
}

#[test]
fn refuses_imports_whose_types_differ_from_the_world() {
    let host = host(limits());
    let refusal = host
        .load(
            manifest("mismatch", "0.1.0").as_bytes(),
            &component(MISMATCH),
        )
        .err()
        .unwrap();
    assert!(matches!(refusal, Refusal::Types(_)), "{refusal}");
}

#[test]
fn refuses_a_mismatched_api_version_with_a_clear_message() {
    let host = host(limits());
    let mods = folder("api");
    write_mod(
        &mods,
        "future",
        &manifest("future", "0.2.0"),
        &component(PLAY),
    );
    let loaded = host.load_dir(&mods);
    assert!(loaded.packages.is_empty());
    let message = loaded.refused[0].to_string();
    assert_eq!(
        message,
        format!(
            "mod `future` ({}) was made for version 0.2.0 of the game's `play` mod API, but this game has 0.1.2; it needs a mod made for a version compatible with 0.1.2",
            mods.join("future").display()
        )
    );
}

#[test]
fn refuses_folders_that_do_not_match_their_id_and_missing_entries() {
    let host = host(limits());
    let mods = folder("names");
    let play = component(PLAY);
    write_mod(&mods, "renamed", &manifest("other", "0.1.0"), &play);
    fs::create_dir_all(mods.join("empty")).unwrap();
    fs::write(mods.join("empty/mod.toml"), manifest("empty", "0.1.0")).unwrap();
    let loaded = host.load_dir(&mods);
    assert!(loaded.packages.is_empty());
    assert_eq!(loaded.refused.len(), 2);
    assert!(matches!(*loaded.refused[0].refusal, Refusal::Read(_)));
    assert_eq!(loaded.refused[0].id.as_deref(), Some("empty"));
    assert!(matches!(*loaded.refused[1].refusal, Refusal::Folder { .. }));
}

#[test]
fn refuses_components_over_the_size_cap() {
    let host = host(Limits {
        component_bytes: 256,
        ..limits()
    });
    let mods = folder("size");
    write_mod(&mods, "big", &manifest("big", "0.1.0"), &component(PLAY));
    let loaded = host.load_dir(&mods);
    assert!(matches!(
        *loaded.refused[0].refusal,
        Refusal::Size {
            what: "component",
            cap: 256,
            ..
        }
    ));
}

#[test]
fn randomness_comes_only_from_the_games_seeded_rng() {
    let host = host(limits());
    let rolls = |seed| {
        let mut m = start(&host, "dice", seed);
        (0..3)
            .map(|_| m.call(|play, store| play.call_roll(store)).unwrap().value)
            .collect::<Vec<u64>>()
    };
    let mut rng = pfx_core::sim::Rng::new(7);
    let expected: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
    assert_eq!(rolls(7), expected);
    assert_ne!(rolls(8), expected);
}

#[test]
fn the_log_is_rate_limited_per_tick() {
    let host = host(limits());
    let mut m = start(&host, "chatty", 1);
    m.call(|play, store| play.call_chat(store, 5)).unwrap();
    assert_eq!(m.take_log(), vec!["hello from a mod"; 3]);
    assert_eq!(m.dropped_lines(), 2);
    m.call(|play, store| play.call_chat(store, 1)).unwrap();
    assert!(m.take_log().is_empty());
    m.tick();
    m.call(|play, store| play.call_chat(store, 1)).unwrap();
    assert_eq!(m.take_log(), vec!["hello from a mod"]);
    assert_eq!(m.dropped_lines(), 3);
}

#[test]
fn running_out_of_fuel_stops_only_that_mod() {
    let host = host(limits());
    let packages = ["calm", "greedy"]
        .iter()
        .map(|id| {
            host.load(manifest(id, "0.1.0").as_bytes(), &component(PLAY))
                .unwrap()
        })
        .collect();
    let (mut set, _) = ModSet::start(&host, packages, bind, |_| game(1));
    let greedy = set.get_mut("greedy").unwrap();
    let error = greedy
        .call(|play, store| play.call_burn(store, u32::MAX))
        .unwrap_err();
    assert_eq!(error.id, "greedy");
    assert_eq!(error.cause, Cause::Fuel);
    assert_eq!(error.fuel, BUDGET);
    assert!(error.at.is_some());
    assert!(
        error
            .to_string()
            .starts_with("mod `greedy` ran out of fuel")
    );
    assert!(!greedy.running());
    assert_eq!(greedy.stopped(), Some(&error));
    let again = greedy
        .call(|play, store| play.call_bump(store, 1))
        .unwrap_err();
    assert_eq!(again.cause, Cause::Disabled);
    let calm = set.get_mut("calm").unwrap();
    assert_eq!(
        calm.call(|play, store| play.call_bump(store, 1))
            .unwrap()
            .value,
        1
    );
}

#[test]
fn a_per_tick_budget_spans_the_tick_and_refills_on_the_next() {
    let host = host(Limits {
        fuel: Fuel::PerTick(BUDGET),
        ..limits()
    });
    let mut m = start(&host, "ticker", 1);
    m.tick();
    let one = m
        .call(|play, store| play.call_burn(store, 1000))
        .unwrap()
        .fuel;
    assert_eq!(m.fuel_left(), Some(BUDGET - one));
    m.call(|play, store| play.call_burn(store, 1000)).unwrap();
    assert_eq!(m.fuel_left(), Some(BUDGET - 2 * one));
    m.tick();
    assert_eq!(m.fuel_left(), Some(BUDGET));
}

#[test]
fn growing_memory_past_the_cap_stops_the_mod() {
    let host = host(limits());
    let mut m = start(&host, "hungry", 1);
    assert_eq!(
        m.call(|play, store| play.call_grow(store, 1))
            .unwrap()
            .value,
        1
    );
    let error = m
        .call(|play, store| play.call_grow(store, 100))
        .unwrap_err();
    assert_eq!(
        error.cause,
        Cause::Limit(Limit::Memory { bytes: 102 * 65536 })
    );
    assert!(!m.running());
}

#[test]
fn traps_name_the_mod_and_the_point() {
    let host = host(limits());
    let mut m = start(&host, "faller", 1);
    let error = m.call(|play, store| play.call_fall(store, 5)).unwrap_err();
    assert_eq!(error.cause, Cause::Trap(Trap::UnreachableCodeReached));
    assert!(error.at.is_some_and(|at| at.offset.is_some()));
    assert!(error.fuel > 0 && error.fuel < BUDGET);
}

#[test]
fn deep_recursion_stops_at_the_depth_limit_and_the_stack_limit_stays_beneath_it() {
    let host = host(limits());
    let mut m = start(&host, "deep", 1);
    let error = m
        .call(|play, store| play.call_fall(store, u32::MAX))
        .unwrap_err();
    assert_eq!(error.cause, Cause::Depth);
    let host = common::host(Limits {
        depth: u32::MAX,
        ..limits()
    });
    let mut m = start(&host, "deep", 1);
    let error = m
        .call(|play, store| play.call_fall(store, u32::MAX))
        .unwrap_err();
    assert_eq!(error.cause, Cause::Stack);
}

#[test]
fn start_functions_run_under_the_budget() {
    let host = host(limits());
    let package = host
        .load(manifest("spinner", "0.1.0").as_bytes(), &component(SPINNER))
        .unwrap();
    let (set, failed) = ModSet::start(&host, vec![package], bind, |_| game(1));
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].cause, Cause::Fuel);
    assert!(!set.mods()[0].running());
}

#[test]
fn a_module_swaps_between_calls_and_restarts_a_stopped_mod() {
    let host = host(limits());
    let mut m = start(&host, "swapper", 1);
    m.call(|play, store| play.call_bump(store, 4)).unwrap();
    let fresh = host
        .load(manifest("swapper", "0.1.0").as_bytes(), &component(PLAY))
        .unwrap();
    m.swap(&host, fresh.clone()).unwrap();
    assert_eq!(
        m.call(|play, store| play.call_bump(store, 1))
            .unwrap()
            .value,
        1
    );
    m.call(|play, store| play.call_burn(store, u32::MAX))
        .unwrap_err();
    m.swap(&host, fresh).unwrap();
    assert!(m.running());
    let other = host
        .load(manifest("other", "0.1.0").as_bytes(), &component(PLAY))
        .unwrap();
    assert!(m.swap(&host, other).is_err());
    assert_eq!(m.id(), "swapper");
}

#[test]
fn the_games_state_survives_a_stop() {
    let host = host(limits());
    let mut m = start(&host, "keeper", 3);
    m.call(|play, store| play.call_roll(store)).unwrap();
    m.call(|play, store| play.call_fall(store, 1)).unwrap_err();
    let mut rng = pfx_core::sim::Rng::new(3);
    rng.next_u64();
    assert_eq!(m.game_mut().rng.next_u64(), rng.next_u64());
}

#[test]
fn fingerprints_follow_the_manifest_and_the_component() {
    let host = host(limits());
    let play = component(PLAY);
    let base = host
        .load(manifest("print", "0.1.0").as_bytes(), &play)
        .unwrap();
    let again = host
        .load(manifest("print", "0.1.0").as_bytes(), &play)
        .unwrap();
    assert_eq!(base.fingerprint, again.fingerprint);
    let edited = manifest("print", "0.1.0").replace("1.0.0", "1.0.1");
    let other_manifest = host.load(edited.as_bytes(), &play).unwrap();
    assert_ne!(base.fingerprint, other_manifest.fingerprint);
    let mut grown = play.clone();
    grown.extend_from_slice(&[0, 5, 4, b'n', b'o', b't', b'e']);
    let other_component = host
        .load(manifest("print", "0.1.0").as_bytes(), &grown)
        .unwrap();
    assert_ne!(base.fingerprint, other_component.fingerprint);

    let mods = folder("print");
    write_mod(&mods, "print", &manifest("print", "0.1.0"), &play);
    let loaded = host.load_dir(&mods);
    assert_eq!(loaded.packages[0].fingerprint, base.fingerprint);

    let (empty, _) = ModSet::start(&host, Vec::new(), bind, |_| game(1));
    let (one, _) = ModSet::start(&host, vec![base], bind, |_| game(1));
    let (other, _) = ModSet::start(&host, vec![other_component], bind, |_| game(1));
    assert_ne!(empty.fingerprint(), one.fingerprint());
    assert_ne!(one.fingerprint(), other.fingerprint());
    assert_eq!(one.fingerprint().hex().len(), 64);
}

#[test]
fn the_test_world_carries_pfx_logs_wit_unchanged() {
    assert_eq!(
        include_str!("wit/deps/pfx-mod/log.wit").replace("\r\n", "\n"),
        LOG_WIT.replace("\r\n", "\n")
    );
}

#[test]
fn a_mod_can_be_started_on_its_own() {
    let host: Host<Game> = host(limits());
    let package = host
        .load(manifest("solo", "0.1.0").as_bytes(), &component(PLAY))
        .unwrap();
    let mut m = Mod::start(&host, package, bind, game(1));
    assert_eq!(m.manifest().api, "0.1.0");
    assert_eq!(
        m.call(|play, store| play.call_bump(store, 9))
            .unwrap()
            .value,
        9
    );
}
