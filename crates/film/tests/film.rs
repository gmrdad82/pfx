use pfx_film::score::CHANNELS;
use pfx_film::{
    Camera, Cast, Crew, Drawn, Encoder, FilmFrame, Grade, Kinds, Known, Letter, Marks, Overlay,
    Play, Posed, RATE, Rig, Scheme, Score, Script, Sheet, Show, Sounds, Take,
};
use pfx_input::InputEvent;
use pfx_live::flat::{Draw, Icons};
use serde::Deserialize;

const FILM: &str = include_str!("fixtures/film.toml");
const SCORE: &str = include_str!("fixtures/score.toml");
const MARK: &str = include_str!("fixtures/mark.svg");
const FPS: u32 = 30;

#[derive(Clone, Debug, PartialEq)]
struct Toy;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dot {
    kind: String,
    radius: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Disc {
    centre: [f32; 2],
    radius: f32,
}

impl Drawn for Disc {
    fn anchor(&self) -> Option<[f32; 2]> {
        Some(self.centre)
    }

    fn flat(&self) -> bool {
        false
    }
}

impl Kinds for Toy {
    type Scene = Nothing;
    type Film = Nothing;
    type Art = Dot;
    type Drawn = Disc;
    const ART: &'static [&'static str] = &["dot"];
}

struct Words;

impl Cast<Toy> for Words {
    fn fill(&self, text: &str) -> String {
        text.replace("{thing}", "film")
    }

    fn pose(&self, art: &Dot, posed: Posed) -> Option<(Disc, f32)> {
        let disc = Disc {
            centre: posed.centre,
            radius: art.radius,
        };
        Some((disc, 1.0))
    }
}

#[derive(Default)]
struct Page {
    draws: Vec<Draw>,
    letters: Vec<Letter>,
    groups: u16,
}

impl Sheet for Page {
    type Drawn = Disc;

    fn size(&self) -> [f32; 2] {
        pfx_film::WIDE
    }

    fn draws(&mut self) -> &mut Vec<Draw> {
        &mut self.draws
    }

    fn group(&mut self, opacity: f32) -> Option<u16> {
        (opacity < 0.999).then(|| {
            self.groups += 1;
            self.groups - 1
        })
    }

    fn letter(&mut self, letter: Letter) {
        self.letters.push(letter);
    }

    fn width(&mut self, text: &str, size: f32, _weight: u16) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }

    fn check(&self, centre: [f32; 2], height: f32) -> Option<Draw> {
        Some(Draw::circle(centre, height * 0.3))
    }

    fn art(&mut self, art: &Disc) {
        self.draws.push(Draw::circle(art.centre, art.radius));
    }
}

#[derive(Clone, Debug)]
struct Moment(u64);

impl pfx_film::Cue for Moment {
    fn at(&self) -> u64 {
        self.0
    }
}

struct Toybox {
    overlay: Overlay,
    clock: f64,
    phase: f64,
    pending: Vec<f32>,
    frame: Vec<u8>,
    keys: u32,
    scheme: Option<Scheme>,
}

impl Toybox {
    fn new() -> Toybox {
        let marks = Marks::load(0, &[("mark.svg", MARK)]).unwrap();
        Toybox {
            overlay: Overlay::new(marks),
            clock: 0.0,
            phase: 0.0,
            pending: Vec::new(),
            frame: Vec::new(),
            keys: 0,
            scheme: None,
        }
    }
}

impl Show for Toybox {
    type Kinds = Toy;
    type Cue = Moment;

    fn plan(&mut self, script: &Script<Toy>) -> Result<Vec<Moment>, String> {
        Ok(script
            .scenes
            .iter()
            .map(|s| Moment((s.from * 60.0).round() as u64))
            .collect())
    }

    fn set_theme(&mut self, scheme: Option<Scheme>) {
        self.scheme = scheme;
    }

    fn seek(&mut self, _scene: &pfx_film::Scene<Toy>, take: &Take<Moment>) -> Result<(), String> {
        self.clock = take.cue.0 as f64 / 60.0;
        Ok(())
    }

    fn aim(&self, _scene: &pfx_film::Scene<Toy>) -> Option<[f32; 2]> {
        Some([960.0 + self.clock.sin() as f32 * 200.0, 540.0])
    }

    fn tick(&mut self, _frame: u64, input: &[InputEvent]) {
        self.keys += input.len() as u32;
    }

    fn draw(&mut self, frame: &FilmFrame<'_, Disc>) -> Result<(), String> {
        let step = frame.seconds();
        self.clock += step;
        let out = [640, 360];
        frame.game_look(out).validate()?;
        frame.over_look(out).validate()?;
        let mut page = Page::default();
        let grade = &frame.cut.grade;
        self.overlay
            .paint(&mut page, &frame.cut.pieces, grade, false);
        self.frame =
            format!("{:?}{:?}{:?}", frame.view(out), page.draws, page.letters).into_bytes();
        let n = (step * f64::from(RATE)).round() as usize;
        for _ in 0..n {
            self.phase += 220.0 / f64::from(RATE);
            let v = (self.phase * std::f64::consts::TAU).sin() as f32 * 0.2;
            self.pending.extend([v, v]);
        }
        Ok(())
    }

    fn audio(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.pending);
    }

    fn readback(&mut self) -> Result<Vec<u8>, String> {
        Ok(self.frame.clone())
    }
}

#[derive(Debug, Default)]
struct Tape {
    frames: Vec<Vec<u8>>,
    stills: Vec<(String, Vec<u8>)>,
    audio: Vec<f32>,
}

impl Encoder for Tape {
    fn frame(&mut self, _index: u64, rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        self.frames.push(rgba);
        Ok(())
    }

    fn still(&mut self, name: &str, rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        self.stills.push((name.to_string(), rgba));
        Ok(())
    }

    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String> {
        assert_eq!((rate, channels), (RATE, CHANNELS));
        self.audio = audio.unwrap_or_default().to_vec();
        Ok(())
    }
}

fn script(text: &str) -> Result<Script<Toy>, String> {
    let score = Score::new(SCORE, &[], 1, RATE)?;
    let sounds = |name: &str| score.knows(name);
    let known = Known {
        sounds: &sounds,
        marks: &["mark.svg"],
    };
    Script::parse(text, &known)
}

fn roll() -> (Tape, u32) {
    shot(FILM, SCORE).unwrap()
}

fn shot(film: &str, sounds: &str) -> Result<(Tape, u32), String> {
    let script = script(film)?;
    let mut show = Toybox::new();
    let mut score = Score::new(sounds, &[], script.film.seed, RATE)?;
    let mut tape = Tape::default();
    let crew = Crew {
        show: &mut show,
        cast: &Words,
        score: &mut score,
        encoder: Some(&mut tape),
    };
    let rig = Rig {
        size: [640, 360],
        fps: FPS,
    };
    let done = pfx_film::shoot(&script, rig, crew)?;
    assert_eq!(done.frames, script.frames(FPS));
    assert_eq!(done.stills, 1);
    assert_eq!(done.samples * u64::from(CHANNELS), tape.audio.len() as u64);
    Ok((tape, show.keys))
}

#[test]
fn a_film_renders_the_same_frames_stills_and_audio_every_time() {
    let (one, keys) = roll();
    let (two, _) = roll();
    assert_eq!(keys, 2);
    assert_eq!(one.frames.len(), 30);
    assert!(one.frames == two.frames && one.stills == two.stills);
    assert_eq!(one.audio, two.audio);
    assert!(one.audio.iter().any(|s| s.abs() > 1e-3));
    assert!(one.audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    let still = String::from_utf8_lossy(&one.stills[0].1).to_string();
    assert!(still.contains("Icon"), "the mark is drawn");
    let caption = String::from_utf8_lossy(&one.frames[10]).to_string();
    assert!(caption.contains("\"F\""), "the caption is set in caps");
}

#[test]
fn scripts_refuse_unknown_keys_kinds_and_paths() {
    let base = "[[scene]]\nname = \"a\"\nseconds = 1\n";
    assert!(script(base).is_ok());
    for bad in [
        format!("{base}colour = 1"),
        format!("{base}still = \"../x\""),
        format!("{base}[[scene.art]]\nkind = \"blob\"\nat = 0"),
        format!("{base}[[scene.art]]\nkind = \"dot\"\nat = 0"),
        format!("{base}[[scene.art]]\nkind = \"mark\"\nfile = \"x.svg\"\nat = 0"),
        format!("{base}[[scene.sound]]\nat = 0\nsound = \"nope\""),
        format!("[film]\ncast = 2\n{base}"),
        format!("[film]\nlayout = [0, 1080]\n{base}"),
    ] {
        assert!(script(&bad).is_err(), "{bad}");
    }
}

#[test]
fn every_key_at_its_limits_makes_looks_pfx_accepts() {
    for (desat, contrast, split, grain, glow, black, bars) in [
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        (2.0, -1.0, 40.0, 2.0, 2.0, 2.0, 4.0),
        (0.75, 2.0, 0.5, 0.045, 0.45, 1.0, 2.39),
    ] {
        let grade = Grade {
            camera: Camera {
                zoom: 8.0,
                roll: 45.0,
                ..Camera::default()
            },
            desat,
            contrast,
            split,
            grain,
            glow,
            black,
            bars,
            flash: [1.0, 0.5, 0.0, 1.0],
            ..Grade::default()
        };
        for size in [[1920, 1080], [2560, 1440], [1280, 800], [1080, 1920]] {
            grade.game_look(size).validate().unwrap();
            grade.over_look(size).validate().unwrap();
        }
    }
}

#[test]
fn every_score_sound_sounds_and_stays_in_range() {
    let names: Vec<String> = Score::new(SCORE, &[], 1, RATE)
        .unwrap()
        .names()
        .into_iter()
        .map(str::to_string)
        .collect();
    for name in names {
        let mut score = Score::new(SCORE, &[], 7, RATE).unwrap();
        let play = Play {
            seconds: Some(0.6),
            ..Play::default()
        };
        Sounds::cue(&mut score, &name, 300, play).unwrap();
        let block = Sounds::render(&mut score, RATE as usize);
        assert!(block.iter().any(|s| s.abs() > 1e-3), "{name}");
        assert!(
            block.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
            "{name}"
        );
    }
    let mut score = Score::new(SCORE, &[], 1, RATE).unwrap();
    assert!(Sounds::cue(&mut score, "nope", 0, Play::default()).is_err());
}

#[test]
fn a_mark_with_arcs_bakes_into_icons() {
    let marks = Marks::load(0, &[("mark.svg", MARK)]).unwrap();
    assert_eq!(marks.names(), ["mark.svg"]);
    Icons::bake(&marks.shapes).unwrap();
}

fn fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn the_fixture_film_keeps_its_hash() {
    let (tape, _) = roll();
    let mut h = 0xcbf2_9ce4_8422_2325;
    for f in &tape.frames {
        h = fnv(h, f);
    }
    for (name, s) in &tape.stills {
        h = fnv(fnv(h, name.as_bytes()), s);
    }
    for s in &tape.audio {
        h = fnv(h, &s.to_le_bytes());
    }
    assert_eq!(h, 17_911_994_450_967_581_415);
}

#[test]
fn the_bed_ducks_under_the_score_deterministically() {
    let (plain, _) = roll();
    let ducked = format!("[duck]\nunder = [\"score\"]\ndepth_db = 12\n{FILM}");
    let (one, _) = shot(&ducked, SCORE).unwrap();
    let (two, _) = shot(&ducked, SCORE).unwrap();
    assert_eq!(one.audio, two.audio);
    assert_eq!(one.frames, plain.frames);
    assert_ne!(one.audio, plain.audio);
    let off = format!("[duck]\ndepth_db = 0\n{FILM}");
    assert_eq!(shot(&off, SCORE).unwrap().0.audio, plain.audio);
    let scene = FILM.replacen(
        "seconds = 0.5\n",
        "seconds = 0.5\n\n[scene.duck]\ndepth_db = 0\n",
        1,
    );
    let (opened, _) = shot(&format!("[duck]\n{scene}"), SCORE).unwrap();
    assert_ne!(opened.audio, one.audio);
    let hits = SCORE.replace("[recipe.thump]\n", "[recipe.thump]\nkind = \"hit\"\n");
    let keyed = format!("[duck]\nunder = [\"hit\"]\nbeds = [\"pad\"]\n{FILM}");
    let (kind, _) = shot(&keyed, &hits).unwrap();
    assert!(kind.audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert_ne!(kind.audio, one.audio);
    let err = shot(&format!("[duck]\nunder = \"nope\"\n{FILM}"), SCORE).unwrap_err();
    assert!(err.contains("'nope'"), "{err}");
    assert!(script(&format!("[duck]\nbeds = [\"nope\"]\n{FILM}")).is_err());
    assert!(script(&format!("[duck]\ndepth_db = 90\n{FILM}")).is_err());
}
