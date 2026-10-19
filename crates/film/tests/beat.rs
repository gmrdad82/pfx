use pfx_core::rate::Rate;
use pfx_film::{
    Cast, Crew, Drawn, FilmFrame, Kinds, Known, Posed, RATE, Rig, Scheme, Score, Script, Show,
    Take, frames,
};
use pfx_input::InputEvent;
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
    assert_eq!(frames(at100.scenes[0].seconds, Rate::whole(30)), 144);
    assert_eq!(at100.frames(Rate::whole(60)), 288 + 324);
    let at128 = film("bpm = 128", scenes).unwrap();
    assert_eq!(at128.scenes[0].seconds, 3.75);
    assert_eq!(at128.scenes[0].key[0].at, 0.703125);
    assert_eq!(at128.scenes[1].seconds, 4.21875);
    assert_eq!(at128.frames(Rate::whole(60)), 225 + 253);
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
    assert!(s.off_grid(Rate::whole(60)).unwrap().is_empty());
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
    let misses = warn.off_grid(Rate::whole(60)).unwrap();
    assert_eq!(misses.len(), 2, "{misses:?}");
    assert!(
        misses[0].starts_with("scene 'drop': a key at 1.4b is off the 1/4-beat grid"),
        "{misses:?}"
    );
    assert!(misses[0].contains("1.4b from beat 1"), "{misses:?}");
    assert!(misses[1].contains("scene 'tail'"), "{misses:?}");
    assert!(
        misses[1].contains("isn't a whole number of 1/4 beats"),
        "{misses:?}"
    );
    let strict = film("bpm = 120\noffset = 0.5\nstrict = true", scenes).unwrap();
    let err = strict.off_grid(Rate::whole(60)).unwrap_err();
    assert_eq!(err, misses[0]);
    let fine = film("bpm = 120\noffset = 0.5\nsnap = \"1/8\"", scenes).unwrap();
    assert!(fine.off_grid(Rate::whole(30)).unwrap().is_empty());
    let shifted = film(
        "bpm = 120\noffset = 0.5\nstrict = true",
        &scenes.replace("seconds = 0.5", "seconds = 0.6"),
    )
    .unwrap();
    let err = shifted.off_grid(Rate::whole(60)).unwrap_err();
    assert!(
        err.starts_with("scene 'drop': the cut into it lands 0.2b from beat 1"),
        "{err}"
    );
    assert!(film("bpm = 120\nsnap = \"1/5x\"", scenes).is_err());
    assert!(film("offset = 0.5", scenes).is_err());
}

fn chopped() -> Script<Bare> {
    let lengths = [
        "0.5b", "0.25b", "0.25b", "0.5b", "0.25b", "0.75b", "0.25b", "0.5b",
    ];
    let mut text = String::new();
    for i in 0..96 {
        text.push_str(&format!(
            "[[scene]]\nname = \"s{i}\"\nseconds = \"{}\"\n",
            lengths[i % lengths.len()]
        ));
    }
    text.push_str("[[scene]]\nname = \"drop\"\nseconds = \"2bar\"\n");
    text.push_str("[[scene.key]]\nat = 0\nblack = 1.0\n");
    text.push_str("[[scene.key]]\nat = \"1.25b\"\nease = \"step\"\nblack = 0.0\n");
    film("bpm = 128", &text).unwrap()
}

#[test]
fn half_and_quarter_beat_scenes_keep_every_cut_within_half_a_frame() {
    let s = chopped();
    let fps = Rate::whole(30);
    let starts = s.starts();
    let total = *starts.last().unwrap();
    assert_eq!(s.frames(fps), (total * 30.0).round() as u64);
    let summed: u64 = s.scenes.iter().map(|c| frames(c.seconds, fps)).sum();
    assert!(summed > s.frames(fps) + 10, "{summed} vs {}", s.frames(fps));
    for (cut, start) in s.cuts(fps).iter().zip(&starts) {
        let by = (*cut as f64 - start * 30.0).abs();
        assert!(by <= 0.5 + 1e-9, "a cut {by} frames off at {start}");
    }
    assert!(s.off_grid(fps).unwrap().is_empty(), "{:?}", s.off_grid(fps));
}

#[test]
fn a_cut_off_the_grid_by_more_than_half_a_frame_is_named() {
    let scenes = r#"
[[scene]]
name = "lead"
seconds = 0.52

[[scene]]
name = "hit"
seconds = "4b"
"#;
    let s = film("bpm = 120\noffset = 0.5", scenes).unwrap();
    let misses = s.off_grid(Rate::whole(30)).unwrap();
    assert_eq!(misses.len(), 1, "{misses:?}");
    assert!(
        misses[0].starts_with("scene 'hit': the cut into it lands"),
        "{misses:?}"
    );
    let on = film("bpm = 120\noffset = 0.5", &scenes.replace("0.52", "0.51")).unwrap();
    assert!(on.off_grid(Rate::whole(30)).unwrap().is_empty());
}

struct Lamp {
    black: Vec<f32>,
}

struct Blank;

impl Cast<Bare> for Blank {
    fn fill(&self, text: &str) -> String {
        text.to_string()
    }

    fn pose(&self, _art: &Nothing, _posed: Posed) -> Option<(Nothing, f32)> {
        None
    }
}

#[derive(Clone, Debug)]
struct At;

impl pfx_film::Cue for At {
    fn at(&self) -> u64 {
        0
    }
}

impl Show for Lamp {
    type Kinds = Bare;
    type Cue = At;

    fn plan(&mut self, script: &Script<Bare>) -> Result<Vec<At>, String> {
        Ok(vec![At; script.scenes.len()])
    }

    fn set_theme(&mut self, _scheme: Option<Scheme>) {}

    fn seek(&mut self, _scene: &pfx_film::Scene<Bare>, _take: &Take<At>) -> Result<(), String> {
        Ok(())
    }

    fn aim(&self, _scene: &pfx_film::Scene<Bare>) -> Option<[f32; 2]> {
        None
    }

    fn tick(&mut self, _frame: u64, _input: &[InputEvent]) {}

    fn draw(&mut self, frame: &FilmFrame<'_, Nothing>) -> Result<(), String> {
        self.black.push(frame.cut.grade.black);
        Ok(())
    }

    fn audio(&mut self, _out: &mut Vec<f32>) {}

    fn readback(&mut self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

#[test]
fn a_key_lands_on_the_film_clock_after_drifting_scenes() {
    let s = chopped();
    let mut lamp = Lamp { black: Vec::new() };
    let mut score = Score::new(SCORE, &[], 1, RATE).unwrap();
    let crew = Crew {
        show: &mut lamp,
        cast: &Blank,
        score: &mut score,
        encoder: None,
    };
    let rig = Rig {
        size: [64, 36],
        fps: Rate::whole(30),
    };
    let done = pfx_film::shoot(&s, rig, crew).unwrap();
    assert_eq!(done.frames, s.frames(Rate::whole(30)));
    let drop = s.starts()[s.scenes.len() - 1];
    let grid = s.film.beat.as_ref().unwrap();
    let flip = grid.seconds(grid.beat(drop) + 1.25);
    let cut = s.cuts(Rate::whole(30))[s.scenes.len() - 1] as usize;
    assert_eq!(lamp.black[cut], 1.0);
    let first = (cut + lamp.black[cut..].iter().position(|b| *b == 0.0).unwrap()) as f64;
    assert!(
        first / 30.0 >= flip - 1e-9 && first / 30.0 < flip + 1.0 / 30.0,
        "frame {first} for {flip} s"
    );
}
