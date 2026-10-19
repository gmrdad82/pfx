use super::*;

fn rate(text: &str) -> Rate {
    text.parse().unwrap()
}

#[test]
fn every_accepted_form_reads_to_its_exact_reduced_rational() {
    for (text, num, den) in [
        ("24", 24, 1),
        ("24.0", 24, 1),
        ("24000/1001", 24000, 1001),
        ("30000/1001", 30000, 1001),
        ("60000/1001", 60000, 1001),
        ("48/2", 24, 1),
        ("23.976", 24000, 1001),
        ("29.97", 30000, 1001),
        ("29.970", 30000, 1001),
        ("47.952", 48000, 1001),
        ("59.94", 60000, 1001),
        ("119.88", 120000, 1001),
    ] {
        let found = rate(text);
        assert_eq!((found.num(), found.den()), (num, den), "{text}");
    }
    assert_eq!(rate("24000/1001").to_string(), "24000/1001");
    assert_eq!(rate("23.976").label(), "23.976");
    assert_eq!(rate("24").to_string(), "24");
}

#[test]
fn other_decimals_and_junk_are_refused_naming_the_rational_to_use() {
    let refused = |text: &str| text.parse::<Rate>().unwrap_err();
    assert!(refused("23.97").contains("use 24000/1001"));
    assert!(refused("29.976").contains("use 30000/1001"));
    assert!(refused("12.5").contains("use 25/2"));
    for junk in [
        "", "abc", "24/", "/1001", "24/0", "0", "-24", "24.5.1", "1e3", "24 fps",
    ] {
        assert!(junk.parse::<Rate>().is_err(), "{junk:?}");
    }
    assert!(!refused("x").contains('\n'));
    assert!(Rate::parse_within("241", 1, 240).is_err());
    assert!(Rate::parse_within("1/2", 1, 240).is_err());
    assert!(Rate::parse_within("240000/1001", 1, 240).is_ok());
    assert!(Rate::parse_within("240", 1, 240).is_ok());
}

#[test]
fn frame_times_at_ntsc_film_are_exact_for_any_frame() {
    let film = rate("24000/1001");
    assert_eq!(film.time(0), 0.0);
    assert_eq!(film.time(1), 1001.0 / 24000.0);
    assert_eq!(film.time(1000), 1_001_000.0 / 24000.0);
    assert_eq!(film.time(1000), 41.708333333333336);
    assert_eq!(film.time(100_000), 4170.833333333333);
    assert_eq!(film.time(24000), 1001.0);
    let mut stepped = 0.0f64;
    for _ in 0..100_000 {
        stepped += film.step();
    }
    assert_ne!(stepped, film.time(100_000));
    for frame in [1u64, 1000, 100_000] {
        assert_eq!(film.frame_at(film.time(frame)), Some(frame));
        assert_eq!(film.before(film.time(frame)), film.time(frame - 1));
    }
    assert_eq!(film.frame_at(film.time(10) + 0.01), None);
}

#[test]
fn audio_samples_per_frame_follow_the_exact_cadence() {
    let film = rate("23.976");
    let per_frame: Vec<u64> = (0..6)
        .map(|k| film.samples(k + 1, 48_000) - film.samples(k, 48_000))
        .collect();
    assert_eq!(per_frame, [2002, 2002, 2002, 2002, 2002, 2002]);
    assert_eq!(film.samples(500, 48_000), 1_001_000);
    assert_eq!(film.samples(24_000, 48_000), 48_048_000);
    let video = rate("29.97");
    assert_eq!(video.samples(5, 48_000), 8008);
    assert_eq!(Rate::whole(7).samples_nearest(1, 48_000), 6857);
    assert_eq!(Rate::whole(7).samples(1, 48_000), 6857);
}

#[test]
fn whole_rates_keep_their_old_arithmetic() {
    for fps in [1u32, 24, 25, 30, 60, 240] {
        let r = Rate::whole(fps);
        for frame in [0u64, 1, 7, 1000] {
            assert_eq!(r.time(frame), frame as f64 / f64::from(fps));
        }
        assert_eq!(r.step(), 1.0 / f64::from(fps));
        assert_eq!(r.whole_fps(), Some(fps));
    }
}

#[cfg(feature = "serde")]
#[test]
fn serde_writes_whole_rates_as_numbers_and_others_as_ratios() {
    assert_eq!(serde_json::to_string(&Rate::whole(24)).unwrap(), "24");
    assert_eq!(
        serde_json::to_string(&rate("23.976")).unwrap(),
        "\"24000/1001\""
    );
    for text in ["24", "\"24000/1001\"", "23.976", "\"23.976\""] {
        let found: Rate = serde_json::from_str(text).unwrap();
        assert!(
            found == Rate::whole(24) || found == rate("24000/1001"),
            "{text}"
        );
    }
    assert!(serde_json::from_str::<Rate>("23.97").is_err());
    let written = serde_json::to_string(&rate("24000/1001").fps()).unwrap();
    assert_eq!(
        serde_json::from_str::<Rate>(&written).unwrap(),
        rate("23.976")
    );
}

#[test]
fn the_short_decimals_cameras_print_read_as_the_exact_ntsc_rates() {
    assert_eq!(
        "23.98".parse::<Rate>().unwrap(),
        "24000/1001".parse::<Rate>().unwrap()
    );
    assert_eq!(
        Rate::from_f64(23.98).unwrap(),
        "24000/1001".parse::<Rate>().unwrap()
    );
    assert_eq!(
        "47.95".parse::<Rate>().unwrap(),
        "48000/1001".parse::<Rate>().unwrap()
    );
    assert_eq!(
        "47.952".parse::<Rate>().unwrap(),
        "48000/1001".parse::<Rate>().unwrap()
    );
}
