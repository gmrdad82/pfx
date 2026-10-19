use pfx_core::rate::Rate;
use pfx_film::{
    Cast, Crew, Drawn, Encoder, FilmFrame, Kinds, Known, Posed, RATE, Rig, Scheme, Score, Script,
    Show, Take,
};
use pfx_input::InputEvent;
use serde::Deserialize;

const SCORE: &str = include_str!("fixtures/score.toml");
const FPS: Rate = Rate::whole(30);

#[derive(Clone, Debug, PartialEq)]
struct Flat;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Paint {
    colour: [u8; 3],
}

impl Drawn for Nothing {
    fn anchor(&self) -> Option<[f32; 2]> {
        None
    }

    fn flat(&self) -> bool {
        true
    }
}

impl Kinds for Flat {
    type Scene = Paint;
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

struct Slate {
    size: [u32; 2],
    colour: [u8; 3],
    frames: u8,
    picture: Vec<u8>,
}

impl Show for Slate {
    type Kinds = Flat;
    type Cue = At;

    fn plan(&mut self, script: &Script<Flat>) -> Result<Vec<At>, String> {
        Ok(vec![At; script.scenes.len()])
    }

    fn set_theme(&mut self, _scheme: Option<Scheme>) {}

    fn seek(&mut self, scene: &pfx_film::Scene<Flat>, _take: &Take<At>) -> Result<(), String> {
        self.colour = scene.app.colour;
        self.frames = 0;
        Ok(())
    }

    fn aim(&self, _scene: &pfx_film::Scene<Flat>) -> Option<[f32; 2]> {
        None
    }

    fn tick(&mut self, _frame: u64, _input: &[InputEvent]) {}

    fn draw(&mut self, _frame: &FilmFrame<'_, Nothing>) -> Result<(), String> {
        let [r, _, b] = self.colour;
        let px = [r, self.frames * 2, b, 255];
        self.frames += 1;
        self.picture = px.repeat((self.size[0] * self.size[1]) as usize);
        Ok(())
    }

    fn audio(&mut self, _out: &mut Vec<f32>) {}

    fn readback(&mut self) -> Result<Vec<u8>, String> {
        Ok(self.picture.clone())
    }
}

#[derive(Default)]
struct Tape {
    frames: Vec<Vec<u8>>,
    audio: Vec<f32>,
}

impl Encoder for Tape {
    fn frame(&mut self, _index: u64, rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        self.frames.push(rgba);
        Ok(())
    }

    fn still(&mut self, _name: &str, _rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        Ok(())
    }

    fn finish(&mut self, audio: Option<&[f32]>, _rate: u32, _channels: u16) -> Result<(), String> {
        self.audio = audio.unwrap_or_default().to_vec();
        Ok(())
    }
}

fn script(text: &str) -> Result<Script<Flat>, String> {
    let score = Score::new(SCORE, &[], 1, RATE)?;
    let sounds = |name: &str| score.knows(name);
    let known = Known {
        sounds: &sounds,
        marks: &[],
    };
    Script::parse(text, &known)
}

fn film(transition: &str) -> String {
    format!(
        r#"[film]
bpm = 120

[[scene]]
name = "out"
seconds = "2b"
colour = [200, 0, 0]

[[scene.sound]]
at = 0
sound = "thump"
every = "1b"

[[scene]]
name = "in"
seconds = "4b"
colour = [0, 0, 100]
{transition}
"#
    )
}

fn shoot(text: &str, size: [u32; 2]) -> (Script<Flat>, Tape) {
    let script = script(text).unwrap();
    let mut show = Slate {
        size,
        colour: [0; 3],
        frames: 0,
        picture: Vec::new(),
    };
    let mut score = Score::new(SCORE, &[], 1, RATE).unwrap();
    let mut tape = Tape::default();
    let crew = Crew {
        show: &mut show,
        cast: &Blank,
        score: &mut score,
        encoder: Some(&mut tape),
    };
    let rig = Rig { size, fps: FPS };
    let done = pfx_film::shoot(&script, rig, crew).unwrap();
    assert_eq!(done.frames, tape.frames.len() as u64);
    (script, tape)
}

fn px(frame: &[u8], x: usize) -> [u8; 4] {
    [
        frame[x * 4],
        frame[x * 4 + 1],
        frame[x * 4 + 2],
        frame[x * 4 + 3],
    ]
}

#[test]
fn a_dissolve_mixes_both_scenes_evenly_at_its_middle_and_keeps_the_cuts() {
    let (plain_script, plain) = shoot(&film(""), [8, 4]);
    let (s, tape) = shoot(
        &film("[scene.transition]\nkind = \"dissolve\"\nseconds = \"2b\""),
        [8, 4],
    );
    assert_eq!(s.cuts(FPS), vec![0, 30, 90]);
    assert_eq!(s.cuts(FPS), plain_script.cuts(FPS));
    assert_eq!(tape.frames.len(), 90);
    assert_eq!(tape.audio, plain.audio);
    assert!(s.off_grid(FPS).unwrap().is_empty());
    assert_eq!(tape.frames[29], plain.frames[29]);
    assert_eq!(px(&tape.frames[30], 0), [200, 60, 0, 255]);
    let mid = &tape.frames[45];
    let outgoing = [200u8, 45 * 2, 0, 255];
    let incoming = [0u8, 15 * 2, 100, 255];
    let even: Vec<u8> = (0..4)
        .map(|c| ((f32::from(outgoing[c]) + f32::from(incoming[c])) / 2.0).round() as u8)
        .collect();
    assert!(
        mid.chunks_exact(4).all(|p| p == even.as_slice()),
        "{:?}",
        &mid[..4]
    );
    for f in 60..90 {
        assert_eq!(tape.frames[f], plain.frames[f]);
    }
}

#[test]
fn a_push_slides_the_outgoing_scene_out_as_the_incoming_comes_in() {
    let (_, tape) = shoot(
        &film("[scene.transition]\nkind = \"push\"\nseconds = \"2b\"\ntowards = \"left\""),
        [8, 4],
    );
    assert_eq!(tape.frames.len(), 90);
    let mid = &tape.frames[45];
    for x in 0..4 {
        assert_eq!(px(mid, x)[0], 200, "column {x}");
    }
    for x in 4..8 {
        assert_eq!(px(mid, x)[2], 100, "column {x}");
    }
    let (_, whip) = shoot(
        &film("[scene.transition]\nkind = \"whip\"\nseconds = \"1b\""),
        [64, 2],
    );
    assert_eq!(whip.frames.len(), 90);
    let streaked = (30..45).any(|f| {
        let frame = &whip.frames[f];
        (0..64).any(|x| {
            let p = px(frame, x);
            p[0] > 0 && p[0] < 200 && p[2] > 0 && p[2] < 100
        })
    });
    assert!(streaked, "the whip blurs along its move");
    assert_eq!(whip.frames[45][0], 0);
}

#[test]
fn transitions_are_checked_and_placed_on_the_grid() {
    let first = "[film]\nbpm = 120\n[[scene]]\nname = \"a\"\nseconds = 1\ncolour = [0, 0, 0]\n[scene.transition]\nkind = \"dissolve\"\nseconds = 0.5\n";
    assert!(script(first).unwrap_err().contains("first scene"));
    for bad in [
        "[scene.transition]\nkind = \"dissolve\"\nseconds = \"5b\"",
        "[scene.transition]\nkind = \"spin\"\nseconds = \"1b\"",
        "[scene.transition]\nkind = \"push\"\nseconds = \"1b\"\ntowards = \"back\"",
        "[scene.transition]\nkind = \"push\"\nseconds = \"1b\"\nease = \"wobble\"",
    ] {
        assert!(script(&film(bad)).is_err(), "{bad}");
    }
    let off = script(&film(
        "[scene.transition]\nkind = \"dissolve\"\nseconds = 0.6",
    ))
    .unwrap();
    let misses = off.off_grid(FPS).unwrap();
    assert!(
        misses
            .iter()
            .any(|m| m.starts_with("scene 'in': the dissolve into it ends")),
        "{misses:?}"
    );
}
