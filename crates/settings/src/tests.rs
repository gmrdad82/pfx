use crate::{Effect, Effects, Key, Preferences, Quality, Rate, Shedder, Store, Value};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/settings-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn preferences_round_trip_through_the_settings_file_and_keep_its_other_tables() {
    let dir = scratch("round-trip");
    let store = Store::in_dir(&dir);
    let mut document = store.document();
    document["recent"] = toml_edit::value(toml_edit::Array::from_iter(["/a.pitofx"]));
    store.keep(&document).unwrap();
    assert_eq!(store.preferences(), Preferences::default());
    let chosen = Preferences {
        vrr: false,
        frame_rate: Some(Rate::Auto),
        stats: true,
        graphics: crate::Graphics {
            quality: Some(Quality::Performance),
            effects: Effects::ALL_ON
                .with(Effect::Ssr, false)
                .with(Effect::MotionBlur, false),
        },
    };
    store.keep_preferences(&chosen).unwrap();
    store.keep_licence_key(Some("PITO1.a.b\n")).unwrap();
    assert_eq!(store.preferences(), chosen);
    assert_eq!(store.licence_key().as_deref(), Some("PITO1.a.b"));
    let text = store.document().to_string();
    assert!(text.contains("recent = [\"/a.pitofx\"]"), "{text}");
    assert!(text.contains("frame_rate = \"auto\""), "{text}");
    assert!(pfx_file::has_magic(&dir.join(crate::FILE)));
    let older = Store::in_dir(&scratch("older"));
    let mut document = toml_edit::DocumentMut::new();
    document["display"]["vrr"] = toml_edit::value(true);
    document["display"]["frame_rate"] = toml_edit::value("cap60");
    older.keep(&document).unwrap();
    assert_eq!(older.preferences().rate(), Rate::Cap60);
    assert!(!older.preferences().graphics.measured());
}

#[test]
fn every_key_reads_and_sets_by_name_and_refuses_what_it_does_not_take() {
    let mut preferences = Preferences::default();
    for (key, value, shown) in [
        ("vrr", "off", Value::Bool(false)),
        ("stats", "on", Value::Bool(true)),
        ("frame_rate", "120", Value::Text("120")),
        ("frame_rate", "cap60", Value::Text("60")),
        ("frame_rate", "auto", Value::Text("auto")),
        ("quality", "performance", Value::Text("performance")),
        ("effects.taa", "false", Value::Bool(false)),
        ("effects.scene_reflections", "off", Value::Bool(false)),
    ] {
        let key = Key::parse(key).unwrap();
        key.set(&mut preferences, value).unwrap();
        assert_eq!(key.get(&preferences), shown);
    }
    assert!(!preferences.graphics.effects.taa);
    let unknown = Key::parse("effects.chrome").unwrap_err();
    assert!(unknown.starts_with("no setting is named 'effects.chrome'; the settings are vrr, frame_rate, stats, quality, effects.taa"), "{unknown}");
    assert_eq!(
        Key::Vrr.set(&mut preferences, "maybe").unwrap_err(),
        "'maybe' is not a value of vrr; it takes on, off"
    );
    assert_eq!(
        Key::FrameRate.set(&mut preferences, "90").unwrap_err(),
        "'90' is not a value of frame_rate; it takes 60, 120, uncapped, auto"
    );
}

#[test]
fn the_first_run_measurement_picks_the_defaults_once() {
    let mut fast = Preferences::default();
    assert!(fast.measure(6.0));
    assert_eq!(
        (fast.rate(), fast.graphics.quality()),
        (Rate::Uncapped, Quality::Appearance)
    );
    assert!(!fast.measure(40.0));
    assert_eq!(fast.rate(), Rate::Uncapped);
    let mut slow = Preferences::default();
    slow.measure(12.0);
    assert_eq!(
        (slow.rate(), slow.graphics.quality()),
        (Rate::Auto, Quality::Performance)
    );
    let mut chosen = Preferences {
        frame_rate: Some(Rate::Cap60),
        ..Preferences::default()
    };
    chosen.measure(30.0);
    assert_eq!(chosen.rate(), Rate::Cap60);
}

#[test]
fn best_performance_sheds_the_costliest_effects_and_brings_them_back() {
    let mut shedder = Shedder::new();
    let chosen = Effects::ALL_ON.with(Effect::Flare, false);
    for _ in 0..Shedder::OVER * 2 {
        shedder.frame(20.0, false);
    }
    assert!(shedder.shed().is_empty());
    for _ in 0..Shedder::OVER * 2 {
        shedder.frame(20.0, true);
    }
    assert_eq!(shedder.shed(), &[Effect::MotionBlur, Effect::DepthOfField]);
    let kept = shedder.effects(chosen);
    assert!(!kept.motion_blur && !kept.depth_of_field && !kept.flare);
    assert!(kept.taa && kept.ssr && kept.bloom);
    for _ in 0..Shedder::UNDER {
        shedder.frame(3.0, true);
    }
    assert_eq!(shedder.shed(), &[Effect::MotionBlur]);
    assert!(!Effect::SHED.contains(&Effect::Taa));
}
