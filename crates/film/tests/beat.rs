use pfx_film::{Drawn, Kinds, Known, RATE, Score, Script, frames};
use serde::Deserialize;

const SCORE: &str = include_str!("fixtures/score.toml");

#[derive(Clone, Debug, PartialEq)]
struct Bare;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

impl Drawn for Nothing {
    fn anchor(&self) -> Option<[f32; 2]> {
        None
    }

    fn flat(&self) -> bool {
        true
    }
}

impl Kinds for Bare {
    type Scene = Nothing;
    type Film = Nothing;
    type Art = Nothing;
    type Drawn = Nothing;
    const ART: &'static [&'static str] = &[];
}

fn script(text: &str) -> Result<Script<Bare>, String> {
    let score = Score::new(SCORE, &[], 1, RATE)?;
    let sounds = |name: &str| score.knows(name);
    let known = Known {
        sounds: &sounds,
        marks: &[],
    };
    Script::parse(text, &known)
}

fn film(film: &str, scenes: &str) -> Result<Script<Bare>, String> {
    script(&format!("[film]\n{film}\n{scenes}"))
}

#[test]
fn beat_times_convert_exactly_at_100_and_128_bpm() {
    let scenes = r#"
[[scene]]
name = "a"
seconds = "8b"

[[scene.key]]
at = "1.5b"
zoom = 1.2

[[scene.sound]]
at = "1bar"
sound = "thump"
every = "1b"

[[scene]]
name = "b"
seconds = "2bar+1b"
"#;
    let at100 = film("bpm = 100", scenes).unwrap();
    assert_eq!(at100.scenes[0].seconds, 4.8);
    assert_eq!(at100.scenes[0].key[0].at, 0.9);
    assert_eq!(at100.scenes[0].sound[0].at, 2.4);
    assert_eq!(at100.scenes[0].sound[0].every, Some(0.6));
    assert_eq!(at100.scenes[1].seconds, 5.4);
    assert_eq!(frames(at100.scenes[0].seconds, 30), 144);
    assert_eq!(at100.frames(60), 288 + 324);
    let at128 = film("bpm = 128", scenes).unwrap();
    assert_eq!(at128.scenes[0].seconds, 3.75);
    assert_eq!(at128.scenes[0].key[0].at, 0.703125);
    assert_eq!(at128.scenes[1].seconds, 4.21875);
    assert_eq!(at128.frames(60), 225 + 253);
    let waltz = film("bpm = 100\nsignature = [3, 4]", scenes).unwrap();
    assert_eq!(waltz.scenes[1].seconds, 7.0 * 0.6);
    assert!(film("bpm = 100\nsignature = [3, 4]\nbeats_per_bar = 3", scenes).is_err());
    let err = script(scenes).unwrap_err();
    assert!(err.contains("needs [film] bpm"), "{err}");
}

#[test]
fn a_tempo_map_changes_the_beat_from_its_bar() {
    let scenes = r#"
[[scene]]
name = "slow"
seconds = "1bar"

[[scene]]
name = "fast"
seconds = "1bar"

[[scene.flash]]
at = "1b"
"#;
    let s = film(
        "bpm = 100\n[[film.tempo]]\nat = \"1bar\"\nbpm = 128",
        scenes,
    )
    .unwrap();
    assert_eq!(s.scenes[0].seconds, 2.4);
    assert_eq!(s.scenes[1].seconds, 1.875);
    assert_eq!(s.scenes[1].flash[0].at, 0.46875);
    assert!(s.off_grid(60).unwrap().is_empty());
    let late = film(
        "bpm = 100\n[[film.tempo]]\nat = 1.2\nbpm = 128\n[[film.tempo]]\nat = 1.0\nbpm = 90",
        scenes,
    );
    assert!(late.is_err());
}

#[test]
fn off_grid_times_name_the_scene_and_the_beat() {
    let scenes = r#"
[[scene]]
name = "intro"
seconds = 0.5

[[scene]]
name = "drop"
seconds = "4b"

[[scene.key]]
at = 0.7
zoom = 1.3

[[scene]]
name = "tail"
seconds = 2.1
"#;
    let warn = film("bpm = 120\noffset = 0.5", scenes).unwrap();
    let misses = warn.off_grid(60).unwrap();
    assert_eq!(misses.len(), 2, "{misses:?}");
    assert!(
        misses[0].starts_with("scene 'drop': a key at 1.4b is off the 1/4-beat grid"),
        "{misses:?}"
    );
    assert!(misses[0].contains("1.4b from beat 1"), "{misses:?}");
    assert!(misses[1].contains("scene 'tail'"), "{misses:?}");
    assert!(
        misses[1].contains("isn't a whole number of beats"),
        "{misses:?}"
    );
    let strict = film("bpm = 120\noffset = 0.5\nstrict = true", scenes).unwrap();
    let err = strict.off_grid(60).unwrap_err();
    assert_eq!(err, misses[0]);
    let fine = film("bpm = 120\noffset = 0.5\nsnap = \"1/8\"", scenes).unwrap();
    assert_eq!(fine.off_grid(30).unwrap().len(), 1);
    let shifted = film(
        "bpm = 120\noffset = 0.5\nstrict = true",
        &scenes.replace("seconds = 0.5", "seconds = 0.6"),
    )
    .unwrap();
    let err = shifted.off_grid(60).unwrap_err();
    assert!(
        err.starts_with("scene 'drop': the cut into it lands 0.2b from beat 1"),
        "{err}"
    );
    assert!(film("bpm = 120\nsnap = \"1/5x\"", scenes).is_err());
    assert!(film("offset = 0.5", scenes).is_err());
}
