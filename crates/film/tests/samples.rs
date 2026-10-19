use pfx_core::rate::Rate;
use pfx_film::mix::Mixdown;
use pfx_film::shoot::samples;
use pfx_film::{Drawn, Kinds, Known, RATE, Score, Script};
use pfx_sound::{Clip, decibels, true_peak};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const FPS: Rate = Rate::whole(30);
const BEAT: usize = 22_500;
const OFFSET: usize = 12_000;

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

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, rate: u32, samples: Vec<f32>) {
    let clip = Clip {
        rate,
        channels: 1,
        samples,
    };
    std::fs::write(dir.join(name), clip.to_wav()).unwrap();
}

fn kick(rate: u32) -> Vec<f32> {
    let n = rate as usize / 25;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            0.6 * (std::f32::consts::TAU * 60.0 * t).cos() * (-t * 90.0).exp()
        })
        .collect()
}

fn tone(rate: u32, seconds: f32, hz: f32, amplitude: f32) -> Vec<f32> {
    let n = (rate as f32 * seconds) as usize;
    (0..n)
        .map(|i| amplitude * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin())
        .collect()
}

fn mixdown(score_text: &str, film: &str, dir: &Path) -> Result<Vec<f32>, String> {
    let score = Score::in_dir(score_text, &[], 1, RATE, dir)?;
    let sounds = |name: &str| score.knows(name);
    let known = Known {
        sounds: &sounds,
        marks: &[],
    };
    let mut script: Script<Bare> = Script::parse(film, &known)?;
    script.read_bed(dir)?;
    let mut score = Score::in_dir(score_text, &[], script.film.seed, RATE, dir)?;
    let mut mix = Mixdown::new(&script, RATE, &mut score);
    mix.route()?;
    for i in 0..script.frames(FPS) {
        mix.feed(&[]);
        let due = samples(i + 1, FPS) - samples(i, FPS);
        mix.frame(due as usize, 1.0, 0.0)?;
    }
    Ok(mix.master())
}

const KICKS: &str = r#"
[film]
bpm = 128
offset = 0.25
{head}

[[scene]]
name = "lead"
seconds = 0.25

[[scene]]
name = "one"
seconds = "2bar"

[[scene.sound]]
at = 0
sound = "kick"
every = "1b"

[[scene]]
name = "two"
seconds = "2bar"

[[scene.sound]]
at = 0
sound = "kick"
every = "1b"
"#;

fn kicks(head: &str) -> String {
    KICKS.replace("{head}", head)
}

fn score(layer: &str) -> String {
    format!(
        "limiter = 0.99\nmaster = 1.0\n\n[recipe.kick]\nkind = \"kick\"\nlength = 100\n\n[[recipe.kick.layer]]\n{layer}\n"
    )
}

fn onsets(audio: &[f32]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut quiet = usize::MAX;
    for (i, frame) in audio.chunks_exact(2).enumerate() {
        if frame[0] == 0.0 && frame[1] == 0.0 {
            quiet = quiet.saturating_add(1);
            continue;
        }
        if quiet >= 1000 {
            out.push(i);
        }
        quiet = 0;
    }
    out
}

fn the_grid() -> Vec<usize> {
    (0..16).map(|k| OFFSET + k * BEAT).collect()
}

#[test]
fn a_kick_sample_lands_on_every_beat_to_the_sample() {
    let dir = folder("samples-grid");
    write(&dir, "kick.wav", RATE, kick(RATE));
    write(&dir, "kick-44k.wav", 44_100, kick(44_100));
    for (layer, late) in [
        ("sample = \"kick.wav\"\nvolume = 1.0", 0),
        ("sample = \"kick.wav\"\nvolume = 0.7\npitch = 7", 0),
        (
            "sample = \"kick.wav\"\nvolume = 1.0\npitch = -12\nend = 30",
            0,
        ),
        (
            "sample = \"kick.wav\"\nvolume = 1.0\nstart = 2\nrelease = 10",
            0,
        ),
        ("sample = \"kick-44k.wav\"\nvolume = 1.0\ncents = 30", 0),
        ("sample = \"kick.wav\"\nvolume = 1.0\nattack = 1", 1),
    ] {
        let audio = mixdown(&score(layer), &kicks(""), &dir).unwrap();
        let grid: Vec<usize> = the_grid().into_iter().map(|at| at + late).collect();
        assert_eq!(onsets(&audio), grid, "{layer}");
        assert!(decibels(true_peak(&audio, 2)) <= -1.0 + 1e-3, "{layer}");
    }
    let plain = mixdown(
        &score("sample = \"kick.wav\"\nvolume = 1.0"),
        &kicks(""),
        &dir,
    )
    .unwrap();
    let source = kick(RATE);
    for (k, at) in the_grid().into_iter().enumerate().take(3) {
        assert_eq!(plain[at * 2], source[0], "beat {k}");
        assert_eq!(plain[at * 2], plain[at * 2 + 1]);
    }
    let again = mixdown(
        &score("sample = \"kick.wav\"\nvolume = 1.0"),
        &kicks(""),
        &dir,
    )
    .unwrap();
    assert_eq!(plain, again);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_hot_sample_and_bed_master_under_the_true_peak_ceiling() {
    let dir = folder("samples-hot");
    write(&dir, "kick.wav", RATE, kick(RATE));
    write(&dir, "bed.wav", RATE, tone(RATE, 20.0, 110.0, 0.9));
    let layer = "sample = \"kick.wav\"\nvolume = 4.0";
    for (head, ceiling) in [
        (
            "limit = 0.99\nceiling = 1.0\nbed = { file = \"bed.wav\", gain = 2.0 }",
            -1.0,
        ),
        (
            "limit = 0.99\nceiling = 1.0\ntrue_peak = -3\nbed = { file = \"bed.wav\", gain = 4.0, from = 1.5 }",
            -3.0,
        ),
    ] {
        let audio = mixdown(&score(layer), &kicks(head), &dir).unwrap();
        let peak = decibels(true_peak(&audio, 2));
        assert!(peak <= ceiling + 1e-3, "{head}: {peak} dBTP");
        assert!(peak > ceiling - 1.0, "{head}: {peak} dBTP");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

const TALK: &str = r#"
limiter = 0.99
master = 1.0

[recipe.line]
kind = "dialogue"
length = 600

[[recipe.line.layer]]
sample = "line.wav"
volume = 1.0

[recipe.wash]
length = 4000

[[recipe.wash.layer]]
sample = "bed.wav"
volume = 1.0
"#;

fn talk(head: &str, wash: bool) -> String {
    let wash = if wash {
        "\n[[scene.sound]]\nat = 0\nsound = \"wash\"\n"
    } else {
        ""
    };
    format!(
        "[film]\nlimit = 0.99\nceiling = 1.0\n{head}\n\n[[scene]]\nname = \"talk\"\nseconds = 3.0\n{wash}\n[[scene.sound]]\nat = 1.0\nsound = \"line\"\n"
    )
}

fn rms(audio: &[f32], from: f32, to: f32) -> f32 {
    let at = |s: f32| (s * RATE as f32) as usize * 2;
    let part = &audio[at(from)..at(to)];
    let power = part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32;
    10.0 * power.max(1e-20).log10()
}

fn less(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x - y).collect()
}

#[test]
fn the_bed_ducks_under_dialogue_as_a_score_bed_does() {
    let dir = folder("samples-bed");
    write(&dir, "bed.wav", RATE, tone(RATE, 4.0, 220.0, 0.1));
    write(&dir, "line.wav", RATE, tone(RATE, 0.6, 330.0, 0.3));
    let duck = "[duck]\nunder = [\"dialogue\"]\nbeds = [\"wash\"]\n";
    let bed = "bed = { file = \"bed.wav\" }";
    let film = |head: String, wash: bool| format!("{duck}\n{}", talk(&head, wash));
    let with_bed = mixdown(TALK, &film(bed.into(), false), &dir).unwrap();
    let alone = mixdown(TALK, &film(String::new(), false), &dir).unwrap();
    let with_wash = mixdown(TALK, &film(String::new(), true), &dir).unwrap();
    let ducked = less(&with_bed, &alone);
    let before = rms(&ducked, 0.3, 0.9);
    let under = rms(&ducked, 1.2, 1.5);
    let after = rms(&ducked, 2.5, 2.9);
    assert!((before - 20.0 * (0.1f32 / 2f32.sqrt()).log10()).abs() < 0.05);
    assert!(
        (before - under - 12.0).abs() < 1.0,
        "the bed drops {:.2} dB under the line",
        before - under
    );
    assert!(
        (before - after).abs() < 0.2,
        "and comes back: {before} {after}"
    );
    let worst = with_bed
        .iter()
        .zip(&with_wash)
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(
        worst < 1e-6,
        "the WAV bed and a score bed differ by {worst}"
    );
    let open = mixdown(TALK, &talk(bed, false), &dir).unwrap();
    let dry = less(&open, &mixdown(TALK, &talk("", false), &dir).unwrap());
    assert!((rms(&dry, 1.2, 1.5) - before).abs() < 0.05);
    for audio in [&with_bed, &alone, &with_wash, &open] {
        assert!(decibels(true_peak(audio, 2)) <= -1.0 + 1e-3);
    }
    let late = mixdown(
        TALK,
        &film(
            "bed = { file = \"bed.wav\", gain = 0.5, from = 0.5 }".into(),
            false,
        ),
        &dir,
    )
    .unwrap();
    let late = less(&late, &alone);
    let source = tone(RATE, 4.0, 220.0, 0.1);
    let first = (0.5 * RATE as f32) as usize;
    assert!((late[0] - 0.5 * source[first]).abs() < 1e-7);
    assert!((late[200] - 0.5 * source[first + 100]).abs() < 1e-7);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn samples_and_beds_are_refused_with_their_reason() {
    let dir = folder("samples-refused");
    write(&dir, "kick.wav", RATE, kick(RATE));
    std::fs::write(dir.join("text.wav"), b"not a wav").unwrap();
    let open = |layer: &str| {
        Score::in_dir(&score(layer), &[], 1, RATE, &dir)
            .err()
            .unwrap_or_default()
    };
    for (layer, says) in [
        (
            "sample = \"kick.wav\"\nwave = \"sine\"\nhz = 50\nvolume = 1",
            "a wave or a sample",
        ),
        ("volume = 1", "a wave or a sample"),
        (
            "sample = \"kick.wav\"\nnote = 40\nvolume = 1",
            "takes no hz, note",
        ),
        ("sample = \"kick.wav\"\nvolume = 5", "volume is in [0, 4]"),
        (
            "sample = \"kick.wav\"\nvolume = 1\npitch = 30",
            "within 24 semitones",
        ),
        (
            "sample = \"kick.wav\"\nvolume = 1\nstart = 10\nend = 5",
            "end comes after its start",
        ),
        (
            "sample = \"kick.wav\"\nvolume = 1\nstart = 900",
            "its start is past its end",
        ),
        ("sample = \"gone.wav\"\nvolume = 1", "gone.wav"),
        ("sample = \"text.wav\"\nvolume = 1", "not a WAV"),
        (
            "wave = \"sine\"\nhz = 50\nvolume = 1\ndecay = 5\nend = 9",
            "a sample layer's",
        ),
    ] {
        let error = open(layer);
        assert!(error.contains(says), "{layer}: {error}");
    }
    let score_text = score("sample = \"kick.wav\"\nvolume = 1");
    let parse = |head: &str| {
        let score = Score::in_dir(&score_text, &[], 1, RATE, &dir).unwrap();
        let sounds = |name: &str| score.knows(name);
        let known = Known {
            sounds: &sounds,
            marks: &[],
        };
        Script::<Bare>::parse(&kicks(head), &known)
            .map(|_| ())
            .err()
            .unwrap_or_default()
    };
    assert!(parse("bed = { file = \"a.wav\", gain = 5 }").contains("bed gain"));
    assert!(parse("bed = { file = \"a.wav\", from = -1 }").contains("bed from"));
    assert!(parse("bed = { file = \"\" }").contains("names a WAV"));
    assert!(parse("bed = { file = \"a.wav\", loud = 1 }").contains("loud"));
    let score = Score::in_dir(&score_text, &[], 1, RATE, &dir).unwrap();
    let sounds = |name: &str| score.knows(name);
    let known = Known {
        sounds: &sounds,
        marks: &[],
    };
    let mut script: Script<Bare> =
        Script::parse(&kicks("bed = { file = \"gone.wav\" }"), &known).unwrap();
    let mut played = Score::in_dir(&score_text, &[], 1, RATE, &dir).unwrap();
    {
        let mut mix = Mixdown::new(&script, RATE, &mut played);
        assert!(mix.route().unwrap_err().contains("read_bed"));
    }
    assert!(script.read_bed(&dir).unwrap_err().contains("gone.wav"));
    std::fs::remove_dir_all(&dir).unwrap();
}
