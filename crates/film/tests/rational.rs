use pfx_core::rate::Rate;
use pfx_film::{
    Cast, Crew, Drawn, FilmFrame, Kinds, Known, Play, Posed, RATE, Rig, Scheme, Script, Show,
    Sounds, Take,
};
use pfx_input::InputEvent;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
struct Flat;

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

impl Kinds for Flat {
    type Scene = Nothing;
    type Film = Nothing;
    type Art = Nothing;
    type Drawn = Nothing;
    const ART: &'static [&'static str] = &[];
}

struct Blank;

impl Cast<Flat> for Blank {
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

#[derive(Default)]
struct Counter {
    drawn: Vec<u64>,
}

impl Show for Counter {
    type Kinds = Flat;
    type Cue = At;

    fn plan(&mut self, script: &Script<Flat>) -> Result<Vec<At>, String> {
        Ok(vec![At; script.scenes.len()])
    }

    fn set_theme(&mut self, _scheme: Option<Scheme>) {}

    fn seek(&mut self, _scene: &pfx_film::Scene<Flat>, _take: &Take<At>) -> Result<(), String> {
        Ok(())
    }

    fn aim(&self, _scene: &pfx_film::Scene<Flat>) -> Option<[f32; 2]> {
        None
    }

    fn tick(&mut self, _frame: u64, _input: &[InputEvent]) {}

    fn draw(&mut self, frame: &FilmFrame<'_, Nothing>) -> Result<(), String> {
        self.drawn.push(frame.index);
        Ok(())
    }

    fn audio(&mut self, _out: &mut Vec<f32>) {}

    fn readback(&mut self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct Ledger {
    clock: u64,
    starts: Vec<u64>,
}

impl Sounds for Ledger {
    fn knows(&self, _name: &str) -> bool {
        true
    }

    fn clock(&self) -> u64 {
        self.clock
    }

    fn cue(&mut self, _name: &str, _at: u64, _play: Play) -> Result<(), String> {
        Ok(())
    }

    fn render(&mut self, frames: usize) -> Vec<f32> {
        self.starts.push(self.clock);
        self.clock += frames as u64;
        vec![0.0; frames * 2]
    }
}

const FILM: &str = r#"[film]
bpm = 120

[[scene]]
name = "one"
seconds = "8bar"

[[scene]]
name = "two"
seconds = "16bar+2b"

[[scene]]
name = "three"
seconds = "11bar+2b"

[[scene]]
name = "four"
seconds = "20bar"
"#;

const BEATS: [u64; 5] = [0, 32, 32 + 66, 32 + 66 + 46, 32 + 66 + 46 + 80];

fn script() -> Script<Flat> {
    let sounds = |_: &str| true;
    let known = Known {
        sounds: &sounds,
        marks: &[],
    };
    Script::parse(FILM, &known).unwrap()
}

fn exact_cut(beats: u64, rate: Rate) -> u64 {
    let top = beats * u64::from(rate.num());
    let bottom = 2 * u64::from(rate.den());
    (2 * top + bottom) / (2 * bottom)
}

fn shoot(rate: Rate) -> (Script<Flat>, Counter, Ledger) {
    let script = script();
    let mut show = Counter::default();
    let mut score = Ledger::default();
    let crew = Crew {
        show: &mut show,
        cast: &Blank,
        score: &mut score,
        encoder: None,
    };
    let done = pfx_film::shoot(
        &script,
        Rig {
            size: [8, 4],
            fps: rate,
        },
        crew,
    )
    .unwrap();
    assert_eq!(done.frames, script.frames(rate));
    assert_eq!(done.samples, rate.samples(done.frames, RATE));
    (script, show, score)
}

#[test]
fn a_ntsc_film_keeps_every_frames_audio_start_on_the_exact_rule_and_cuts_on_the_beat_grid() {
    for text in ["24000/1001", "30000/1001"] {
        let rate: Rate = text.parse().unwrap();
        let (script, show, ledger) = shoot(rate);
        let frames = script.frames(rate);
        assert!(frames > 2500, "{text}: {frames}");
        assert_eq!(show.drawn, (0..frames).collect::<Vec<_>>());
        assert_eq!(ledger.starts.len() as u64, frames);
        for (n, start) in ledger.starts.iter().enumerate() {
            let n = n as u64;
            let exact =
                (u128::from(n) * 48_000 * u128::from(rate.den()) / u128::from(rate.num())) as u64;
            assert_eq!(*start, exact, "{text}: frame {n}");
        }
        let end = u128::from(frames) * 48_000 * u128::from(rate.den()) / u128::from(rate.num());
        assert_eq!(u128::from(ledger.clock), end, "{text}");
        let exact: Vec<u64> = BEATS.iter().map(|b| exact_cut(*b, rate)).collect();
        assert_eq!(script.cuts(rate), exact, "{text}");
        assert!(script.off_grid(rate).unwrap().is_empty(), "{text}");
        for (cut, beats) in exact.iter().zip(BEATS) {
            let error = (rate.time(*cut) - beats as f64 * 0.5).abs();
            assert!(error <= 0.5 * rate.step(), "{text}: cut {cut}");
        }
    }
    let film: Rate = "23.976".parse().unwrap();
    assert_eq!(script().cuts(film), [0, 384, 1175, 1726, 2685]);
    assert_eq!(RATE, 48_000);
}

#[test]
fn the_film_frame_clock_depends_only_on_the_frame_and_the_rate() {
    let rate: Rate = "24000/1001".parse().unwrap();
    for frame in [0u64, 1, 1000, 100_000] {
        let t = pfx_film::shoot::clock(frame, 0.0, rate);
        assert_eq!(t, (frame * 1001) as f64 / 24000.0);
    }
    let start = 32.0;
    let cut = exact_cut(64, rate);
    assert_eq!(cut, 767);
    assert_eq!(
        pfx_film::shoot::clock(cut + 5, start, rate),
        ((cut + 5) * 1001) as f64 / 24000.0 - start
    );
    assert_eq!(pfx_film::shoot::samples(1000, rate), 2_002_000);
    let video: Rate = "29.97".parse().unwrap();
    let per_frame: Vec<u64> = (0..5)
        .map(|n| pfx_film::shoot::samples(n + 1, video) - pfx_film::shoot::samples(n, video))
        .collect();
    assert_eq!(per_frame, [1601, 1602, 1601, 1602, 1602]);
    assert_eq!(pfx_film::shoot::samples(30_000, video), 48_048_000);
}
