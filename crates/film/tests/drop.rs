use pfx_film::mix::Mixdown;
use pfx_film::shoot::samples;
use pfx_film::{Drawn, Kinds, Known, RATE, Score, Script};
use serde::Deserialize;

const DROP: &str = include_str!("fixtures/drop.toml");
const FPS: u32 = 30;
const BEAT: usize = 22_500;

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

fn mixdown(text: &str) -> Result<Vec<f32>, String> {
    let score = Score::new(DROP, &[], 1, RATE)?;
    let sounds = |name: &str| score.knows(name);
    let known = Known {
        sounds: &sounds,
        marks: &[],
    };
    let script: Script<Bare> = Script::parse(text, &known)?;
    let mut score = Score::new(DROP, &[], script.film.seed, RATE)?;
    let mut mix = Mixdown::new(&script, RATE, &mut score);
    mix.route()?;
    for i in 0..script.frames(FPS) {
        mix.feed(&[]);
        let due = samples(i + 1, FPS) - samples(i, FPS);
        mix.frame(due as usize, 1.0, 0.0)?;
    }
    Ok(mix.master())
}

fn rms(audio: &[f32], frames: std::ops::Range<usize>) -> f32 {
    let part = &audio[frames.start * 2..frames.end * 2];
    let power = part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32;
    10.0 * power.max(1e-20).log10()
}

const SCENES: &str = r#"
[[scene]]
name = "build"
seconds = "2bar"
{build}

[[scene.sound]]
at = 0
sound = "pad"
every = "2bar"

[[scene.sound]]
at = 0
sound = "hat"
every = "0.5b"

[[scene.sound]]
at = 0
sound = "bass"
every = "1b"

[[scene]]
name = "drop"
seconds = "2bar"

[[scene.sound]]
at = 0
sound = "pad"
every = "2bar"

[[scene.sound]]
at = 0
sound = "hat"
every = "0.5b"

[[scene.sound]]
at = 0
sound = "bass"
every = "1b"

[[scene.sound]]
at = 0
sound = "kick"
every = "1b"
"#;

fn film(head: &str, build: &str) -> String {
    format!(
        "[film]\nbpm = 128\nceiling = 1.0\nlimit = 0.99\n{head}\n{}",
        SCENES.replace("{build}", build)
    )
}

#[test]
fn a_quieter_build_makes_the_drop_hit_through_the_limiters() {
    let level = |text: String| {
        let audio = mixdown(&text).unwrap();
        let bar = 8 * BEAT;
        assert!(audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        (rms(&audio, 4800..bar), rms(&audio, bar + 4800..2 * bar))
    };
    let quieter = "\n[scene.score]\ngain_db = -6\n";
    let (build, drop) = level(film("", ""));
    let (quiet, loud) = level(film("", quieter));
    assert!((loud - drop).abs() < 0.05, "{drop} {loud}");
    assert!(
        loud - quiet > drop - build + 5.0,
        "build {build} drop {drop}, quieted {quiet} {loud}"
    );
    assert!(loud - quiet > 7.0, "{quiet} {loud}");
    let hot = |limits: &str| {
        film("", quieter)
            .replace("ceiling = 1.0\nlimit = 0.99\n", limits)
            .replace("sound = \"kick\"", "sound = \"kick\"\ngain = 3")
    };
    let (open_build, open_drop) = level(hot("ceiling = 1.0\nlimit = 0.99\n"));
    let (held_build, held_drop) = level(hot(""));
    let (open, held) = (open_drop - open_build, held_drop - held_build);
    assert!(
        held > open - 0.5,
        "the limiters took {open} dB to {held} dB"
    );
}

#[test]
fn the_bass_dips_under_each_kick_and_comes_back_before_the_next() {
    let side = |depth: f32| {
        format!(
            "[duck.bass]\nunder = [\"kick\"]\nbeds = [\"bass\"]\ndepth_db = {depth}\nattack = 2\nrelease = 40\nlookahead = 2\n"
        )
    };
    let ducked = mixdown(&film(&side(12.0), "")).unwrap();
    let plain = mixdown(&film(&side(0.0), "")).unwrap();
    assert_eq!(ducked, mixdown(&film(&side(12.0), "")).unwrap());
    let alone = film(&side(0.0), "")
        .replace("sound = \"pad\"", "sound = \"pad\"\ngain = 0")
        .replace("sound = \"hat\"", "sound = \"hat\"\ngain = 0")
        .replace("sound = \"kick\"", "sound = \"kick\"\ngain = 0");
    let bass = mixdown(&alone).unwrap();
    let span = |v: &[f32], at: &std::ops::Range<usize>| v[at.start * 2..at.end * 2].to_vec();
    let dip = |at: std::ops::Range<usize>| {
        let lost: Vec<f32> = span(&plain, &at)
            .iter()
            .zip(span(&ducked, &at))
            .map(|(p, d)| p - d)
            .collect();
        rms(&lost, 0..lost.len() / 2) - rms(&bass, at)
    };
    let bar = 8 * BEAT;
    let full = 20.0 * (1.0 - 10f32.powf(-12.0 / 20.0)).log10();
    for k in 0..8 {
        let kick = bar + k * BEAT;
        let under = kick + 480..kick + 2400;
        let next = kick + BEAT - 1440..kick + BEAT - 240;
        assert!(
            (dip(under.clone()) - full).abs() < 0.5,
            "kick {k}: {}",
            dip(under)
        );
        assert!(dip(next.clone()) < full - 20.0, "kick {k}: {}", dip(next));
    }
    let build = 0..bar - 96;
    assert!(span(&ducked, &build) == span(&plain, &build));
}

#[test]
fn named_sidechains_refuse_what_the_score_lacks() {
    let bad = |duck: &str| mixdown(&film(duck, "")).unwrap_err();
    assert!(bad("[duck.bass]\nunder = [\"snare\"]\nbeds = [\"bass\"]").contains("'snare'"));
    assert!(bad("[duck.bass]\nunder = [\"kick\"]").contains("[duck.bass]"));
    assert!(bad("[duck.bass]\nunder = [\"kick\"]\nbeds = [\"kick\"]").contains("named once"));
    assert!(
        mixdown(&film(
            "[duck.bass]\nunder = [\"kick\"]\nbeds = [\"bass\"]",
            ""
        ))
        .is_ok()
    );
}
