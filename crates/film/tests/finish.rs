use pfx_film::grade::{CURVE_CORNER, GLOW_RADIUS, GRAIN_RESPONSE, SCANLINE_PERIOD};
use pfx_film::reel::Roll;
use pfx_film::score::CHANNELS;
use pfx_film::{Camera, Drawn, Encoder, Finish, Grade, Kinds, Known, RATE, Script, Toning};
use pfx_live::flat::look::{
    Aberration, Bloom, Crt, Curvature, Grain, LookPass, Scanlines, SplitTone,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
struct Toy;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Clone, Debug, PartialEq)]
struct Blank;

impl Drawn for Blank {
    fn anchor(&self) -> Option<[f32; 2]> {
        None
    }

    fn flat(&self) -> bool {
        true
    }
}

impl Kinds for Toy {
    type Scene = Nothing;
    type Film = Nothing;
    type Art = Nothing;
    type Drawn = Blank;
    const ART: &'static [&'static str] = &[];
}

fn parse(film: &str) -> Result<Script<Toy>, String> {
    let none = |_: &str| false;
    let known = Known {
        sounds: &none,
        marks: &[],
    };
    Script::parse(
        &format!("[film]\n{film}\n\n[[scene]]\nname = \"a\"\nseconds = 1\n"),
        &known,
    )
}

fn finish(film: &str) -> Finish {
    parse(film).unwrap().film.finish
}

#[test]
fn a_film_without_the_finish_keys_keeps_todays_finish() {
    let f = finish("");
    assert_eq!(f, Finish::default());
    assert_eq!(f.scanlines, 0.0);
    assert_eq!(f.scanline_period, SCANLINE_PERIOD);
    assert_eq!(f.curvature, 0.0);
    assert_eq!(f.grain_response, GRAIN_RESPONSE);
    assert_eq!(GRAIN_RESPONSE, 0.5);
    assert_eq!(f.glow_threshold, 0.0);
    assert_eq!(f.tone, Toning::default());
    assert_eq!(f.tone.shadow, [1.0; 3]);
    assert_eq!(f.tone.highlight, [1.0; 3]);
    assert_eq!(f.tone.range, [0.0, 1.0]);
    assert!(f.tone.split().is_none());
}

#[test]
fn the_finish_keys_parse_into_the_film() {
    let f = finish(
        "scanlines = 0.06\nscanline_period = 2.25\ncurvature = 0.02\ngrain_response = 0.7\n\
         glow_threshold = 0.55\ntone = 0.4\ntone_shadow = \"#2040ff\"\n\
         tone_highlight = \"#ffc080\"\ntone_range = [0.1, 0.9]",
    );
    assert_eq!(f.scanlines, 0.06);
    assert_eq!(f.scanline_period, 2.25);
    assert_eq!(f.curvature, 0.02);
    assert_eq!(f.grain_response, 0.7);
    assert_eq!(f.glow_threshold, 0.55);
    assert_eq!(f.tone.amount, 0.4);
    let byte = |b: u8| f32::from(b) / 255.0;
    for (got, want) in f.tone.shadow.iter().zip([byte(0x20), byte(0x40), 1.0]) {
        assert!((got - want).abs() < 1e-6, "{:?}", f.tone.shadow);
    }
    for (got, want) in f.tone.highlight.iter().zip([1.0, byte(0xc0), byte(0x80)]) {
        assert!((got - want).abs() < 1e-6, "{:?}", f.tone.highlight);
    }
    assert_eq!(f.tone.range, [0.1, 0.9]);
}

#[test]
fn every_finish_key_takes_the_ends_of_its_range() {
    for film in [
        "scanlines = 0.0",
        "scanlines = 1.0",
        "scanline_period = 1.0",
        "scanline_period = 64.0",
        "curvature = 0.0",
        "curvature = 0.25",
        "grain_response = 0.0",
        "grain_response = 1.0",
        "glow_threshold = 0.0",
        "glow_threshold = 1.0",
        "tone = 0.0",
        "tone = 1.0",
        "tone_range = [0.0, 1.0]",
        "tone_range = [0.0, 0.01]",
        "tone_range = [0.99, 1.0]",
        "tone_shadow = \"#000000\"\ntone_highlight = \"#ffffff\"",
        "scanlines = 0.03\nscanline_period = 2.25",
        "scanlines = 0.10\nscanline_period = 2.25\ncurvature = 0.035\ngrain_response = 0.75",
    ] {
        assert!(parse(film).is_ok(), "{film}: {:?}", parse(film).err());
    }
}

#[test]
fn bad_finish_values_are_refused_in_one_line_that_names_the_key() {
    for (film, key) in [
        ("scanlines = -0.01", "scanlines"),
        ("scanlines = 1.5", "scanlines"),
        ("scanlines = nan", "scanlines"),
        ("scanline_period = 0.5", "scanline_period"),
        ("scanline_period = 65.0", "scanline_period"),
        ("scanline_period = inf", "scanline_period"),
        ("curvature = -0.01", "curvature"),
        ("curvature = 0.3", "curvature"),
        ("grain_response = -0.1", "grain_response"),
        ("grain_response = 1.1", "grain_response"),
        ("glow_threshold = -1.0", "glow_threshold"),
        ("glow_threshold = 1.5", "glow_threshold"),
        ("tone = -0.5", "tone"),
        ("tone = 2.0", "tone"),
        ("tone_range = [0.8, 0.2]", "tone_range"),
        ("tone_range = [0.5, 0.5]", "tone_range"),
        ("tone_range = [-0.1, 1.0]", "tone_range"),
        ("tone_range = [0.0, 1.2]", "tone_range"),
        ("tone_shadow = \"blue\"", "tone_shadow"),
        ("tone_highlight = \"#12345\"", "tone_highlight"),
    ] {
        let e = parse(film).expect_err(film);
        assert!(e.starts_with("[film] "), "{film}: {e}");
        assert!(e.contains(key), "{film}: {e}");
        assert!(!e.contains('\n'), "{film}: {e}");
    }
    for film in [
        "scanlines = \"strong\"",
        "tone_range = [0.5]",
        "tone_shadow = 3",
        "scan_lines = 0.1",
    ] {
        assert!(parse(film).is_err(), "{film}");
    }
}

fn look_grade(finish: Finish) -> Grade {
    Grade {
        desat: 0.4,
        tint: [1.0, 0.9, 0.8],
        contrast: 0.2,
        dim: 0.3,
        vignette: 0.5,
        grain: 0.3,
        glow: 0.45,
        split: 2.0,
        flash: [1.0, 1.0, 1.0, 0.5],
        black: 0.2,
        bars: 2.39,
        finish,
        ..Grade::default()
    }
}

#[test]
fn the_default_finish_makes_the_same_looks_as_before_it() {
    let grade = look_grade(Finish::default());
    let look = grade.game_look([1920, 1080]);
    assert_eq!(look.passes.len(), 6);
    for pass in &look.passes {
        if let LookPass::Grade(tone) = pass {
            assert_eq!(tone.split, SplitTone::default());
        }
    }
    assert_eq!(look.passes[4], LookPass::Grain(Grain::new(0.3 / 3.0, 1.0)));
    let LookPass::Grain(grain) = &look.passes[4] else {
        panic!("{:?}", look.passes[4]);
    };
    assert_eq!(grain.response, 0.5);
    assert_eq!(
        look.passes[5],
        LookPass::Crt(Crt {
            aberration: Some(Aberration { shift: 2.0 }),
            ..Crt::default()
        })
    );
    let over = grade.over_look([1920, 1080]);
    assert_eq!(
        over.passes[0],
        LookPass::Bloom(Bloom {
            radius: GLOW_RADIUS,
            strength: 0.45,
            tint: [1.0; 4],
            threshold: 0.0,
        })
    );
    assert_eq!(over.passes.len(), 4);
    let quiet = Grade {
        finish: Finish {
            tone: Toning {
                shadow: [0.2, 0.3, 1.0],
                highlight: [1.0, 0.8, 0.2],
                range: [0.2, 0.7],
                ..Toning::default()
            },
            scanline_period: 9.0,
            grain_response: 0.9,
            glow_threshold: 0.7,
            ..Finish::default()
        },
        ..Grade::default()
    };
    assert!(quiet.game_look([1920, 1080]).is_identity());
    assert!(quiet.over_look([1920, 1080]).is_identity());
}

#[test]
fn the_finish_reaches_the_game_and_over_looks() {
    let finish = Finish {
        scanlines: 0.06,
        scanline_period: 2.25,
        curvature: 0.02,
        grain_response: 0.7,
        glow_threshold: 0.55,
        tone: Toning {
            amount: 0.4,
            shadow: [0.2, 0.3, 1.0],
            highlight: [1.0, 0.8, 0.2],
            range: [0.1, 0.9],
        },
    };
    let grade = look_grade(finish);
    for (out, k) in [([1920, 1080], 1.0f32), ([1080, 1920], 1920.0 / 1080.0)] {
        let look = grade.game_look(out);
        look.validate().unwrap();
        assert_eq!(look.passes.len(), 7);
        let LookPass::Grade(toned) = &look.passes[2] else {
            panic!("{:?}", look.passes[2]);
        };
        assert_eq!(toned.saturation, 1.0);
        assert_eq!(toned.contrast, 1.0);
        assert_eq!(toned.tint[3], 0.0);
        assert_eq!(toned.split.shadow[3], 0.4);
        assert_eq!(toned.split.highlight[3], 0.4);
        assert_eq!(toned.split.range, [0.1, 0.9]);
        let LookPass::Grain(grain) = &look.passes[5] else {
            panic!("{:?}", look.passes[5]);
        };
        assert_eq!(grain.response, 0.7);
        assert!((grain.size - k).abs() < 1e-5);
        let LookPass::Crt(crt) = &look.passes[6] else {
            panic!("{:?}", look.passes[6]);
        };
        let lines = crt.scanlines.unwrap();
        assert_eq!(lines.strength, 0.06);
        assert!((lines.period - 2.25 * k).abs() < 1e-5);
        assert_eq!(
            crt.curvature,
            Some(Curvature {
                amount: 0.02,
                corner: CURVE_CORNER,
                bezel: [0.0, 0.0, 0.0, 1.0],
            })
        );
        assert!((crt.aberration.unwrap().shift - 2.0 * k).abs() < 1e-5);
        assert!(crt.glow.is_none());
        let over = grade.over_look(out);
        over.validate().unwrap();
        let LookPass::Bloom(bloom) = &over.passes[0] else {
            panic!("{:?}", over.passes[0]);
        };
        assert_eq!(bloom.threshold, 0.55);
    }
    let lines_only = Grade {
        finish: Finish {
            scanlines: 0.1,
            ..Finish::default()
        },
        ..Grade::default()
    };
    assert_eq!(
        lines_only.game_look([1920, 1080]).passes,
        vec![LookPass::Crt(Crt {
            scanlines: Some(Scanlines {
                period: SCANLINE_PERIOD,
                strength: 0.1,
            }),
            ..Crt::default()
        })]
    );
}

#[test]
fn a_split_tone_takes_the_films_tint_curve() {
    let toning = Toning {
        amount: 0.4,
        shadow: [0.2, 0.5, 1.0],
        highlight: [1.0, 0.7, 0.1],
        range: [0.25, 0.75],
    };
    let split = toning.split().unwrap();
    assert_eq!(split.range, [0.25, 0.75]);
    let factor = |t: f32| (1.0 - 0.4 + 0.4 * t).powf(1.0 / 2.2);
    let dark = split.scale(0.0);
    let light = split.scale(1.0);
    for i in 0..3 {
        assert!(
            (dark[i] - factor(toning.shadow[i])).abs() < 1e-5,
            "{dark:?}"
        );
        assert!(
            (light[i] - factor(toning.highlight[i])).abs() < 1e-5,
            "{light:?}"
        );
    }
    let mid = split.scale(0.5);
    for i in 0..3 {
        assert!(
            (mid[i] - (dark[i] + light[i]) * 0.5).abs() < 1e-5,
            "{mid:?}"
        );
    }
    assert!(
        Toning {
            amount: 0.0,
            ..toning
        }
        .split()
        .is_none()
    );
    let full = Toning {
        amount: 1.0,
        ..toning
    }
    .split()
    .unwrap();
    assert_eq!(full.shadow[3], 1.0);
    let edge = full.scale(0.0);
    assert!((edge[2] - 1.0).abs() < 1e-6 && edge[0] < 1.0);
}

#[test]
fn every_finish_at_its_limits_makes_looks_pfx_accepts() {
    for finish in [
        Finish::default(),
        Finish {
            scanlines: 1.0,
            scanline_period: 64.0,
            curvature: 0.25,
            grain_response: 1.0,
            glow_threshold: 1.0,
            tone: Toning {
                amount: 1.0,
                shadow: [0.0; 3],
                highlight: [1.0; 3],
                range: [0.99, 1.0],
            },
        },
        Finish {
            scanlines: 0.03,
            scanline_period: 1.0,
            grain_response: 0.0,
            tone: Toning {
                amount: 0.01,
                shadow: [1.0, 0.0, 0.0],
                highlight: [0.0, 0.0, 1.0],
                range: [0.0, 0.01],
            },
            ..Finish::default()
        },
    ] {
        let grade = Grade {
            camera: Camera {
                zoom: 8.0,
                ..Camera::default()
            },
            ..look_grade(finish)
        };
        for size in [[1920, 1080], [2560, 1440], [1280, 800], [1080, 1920]] {
            grade.game_look(size).validate().unwrap();
            grade.over_look(size).validate().unwrap();
        }
    }
}

const SIZE: [u32; 2] = [320, 180];

#[derive(Default)]
struct Tape {
    frames: Vec<Vec<u8>>,
}

impl Encoder for Tape {
    fn frame(&mut self, _index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        assert_eq!(size, SIZE);
        self.frames.push(rgba);
        Ok(())
    }

    fn still(&mut self, _name: &str, _rgba: Vec<u8>, _size: [u32; 2]) -> Result<(), String> {
        Ok(())
    }

    fn finish(&mut self, _audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String> {
        assert_eq!((rate, channels), (RATE, CHANNELS));
        Ok(())
    }
}

fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn card(name: &str, film: &str, scene: &str, colour: &str) -> Vec<Vec<u8>> {
    let at = dir(name);
    let text = format!(
        "[film]\nsize = [{}, {}]\nfps = 30\nlayout = [{}, {}]\nseed = 5\n{film}\n\n\
         [[scene]]\nname = \"card\"\nkind = \"card\"\ncolour = \"{colour}\"\nseconds = 0.1\n{scene}\n",
        SIZE[0], SIZE[1], SIZE[0], SIZE[1]
    );
    let path = at.join("card.film.toml");
    std::fs::write(&path, text).unwrap();
    let roll = Roll::open(&path).unwrap();
    let mut tape = Tape::default();
    roll.roll(Some(&mut tape)).unwrap();
    assert_eq!(tape.frames.len() as u64, roll.frames());
    tape.frames
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [i32; 3] {
    let at = ((y * SIZE[0] + x) * 4) as usize;
    [frame[at], frame[at + 1], frame[at + 2]].map(i32::from)
}

const LOOKED: &str = "[[scene.key]]\nat = 0.0\ndesat = 0.3\ncontrast = 0.2\ngrain = 0.4\nvignette = 0.5\nsplit = 3.0\n";
const DEFAULTS: &str = "scanlines = 0.0\nscanline_period = 3.0\ncurvature = 0.0\ngrain_response = 0.5\n\
                        glow_threshold = 0.0\ntone = 0.0\ntone_shadow = \"#ffffff\"\n\
                        tone_highlight = \"#ffffff\"\ntone_range = [0.0, 1.0]";

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_film_with_the_default_finish_renders_the_same_bytes_as_one_without_it() {
    let plain = card("finish-plain", "", LOOKED, "#6a8fb0");
    let again = card("finish-again", "", LOOKED, "#6a8fb0");
    let spelled = card("finish-spelled", DEFAULTS, LOOKED, "#6a8fb0");
    assert!(plain == again, "two renders are equal");
    assert!(plain == spelled, "the default finish keys change no byte");
    let untouched = card("finish-untouched", "", "", "#6a8fb0");
    assert!(plain != untouched, "the looks themselves draw");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn scanlines_darken_rows_on_their_period_and_curvature_bends_the_corners() {
    let plain = card("lines-plain", "", "", "#808080");
    let lined = card(
        "lines-on",
        "scanlines = 0.3\nscanline_period = 3.0",
        "",
        "#808080",
    );
    let again = card(
        "lines-again",
        "scanlines = 0.3\nscanline_period = 3.0",
        "",
        "#808080",
    );
    assert!(lined == again, "two renders are equal");
    let (base, lines) = (&plain[1], &lined[1]);
    for x in [8u32, 160, 311] {
        for y in 3..SIZE[1] - 3 {
            let p = pixel(lines, x, y)[1];
            let flat = pixel(base, x, y)[1];
            if y % 3 == 1 {
                assert!(
                    (p - flat).abs() <= 1,
                    "row {y} keeps its light: {p} vs {flat}"
                );
            } else {
                assert!(p <= flat - 8, "row {y} darkens: {p} vs {flat}");
            }
            let twin = pixel(lines, x, y + 3)[1];
            assert!((p - twin).abs() <= 1, "rows repeat every period: {y}");
        }
    }
    let bent = card("curve-on", "curvature = 0.2", "", "#808080");
    let corner = pixel(&bent[1], 0, 0);
    assert!(
        corner.iter().all(|c| *c <= 2),
        "the corner falls off the screen: {corner:?}"
    );
    let centre = pixel(&bent[1], 160, 90);
    let flat = pixel(base, 160, 90);
    assert!(
        (0..3).all(|i| (centre[i] - flat[i]).abs() <= 1),
        "{centre:?} vs {flat:?}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn grain_response_one_keeps_white_clean_while_zero_grains_it() {
    let grain = "[[scene.key]]\nat = 0.0\ngrain = 1.0\n";
    let clean = card("grain-clean", "grain_response = 1.0", grain, "#ffffff");
    for frame in &clean {
        assert!(
            frame
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c >= 254)),
            "a response of 1 keeps white clean"
        );
    }
    let grained = card("grain-full", "grain_response = 0.0", grain, "#ffffff");
    let speckled = grained[1].chunks_exact(4).filter(|p| p[1] < 250).count();
    assert!(
        speckled * 10 > (SIZE[0] * SIZE[1]) as usize,
        "{speckled} grains"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_split_tone_moves_shadows_and_highlights_toward_their_tints() {
    let tone = "tone = 1.0\ntone_shadow = \"#5070ff\"\ntone_highlight = \"#ffb060\"\ntone_range = [0.3, 0.6]";
    let dark_plain = card("tone-dark-plain", "", "", "#202020");
    let dark = card("tone-dark", tone, "", "#202020");
    let light_plain = card("tone-light-plain", "", "", "#e0e0e0");
    let light = card("tone-light", tone, "", "#e0e0e0");
    let d = pixel(&dark[1], 160, 90);
    let dp = pixel(&dark_plain[1], 160, 90);
    assert!(d[2] > d[0] + 4, "shadows lean blue: {d:?}");
    assert!(
        (d[2] - dp[2]).abs() <= 1,
        "the shadow tint keeps its full channel: {d:?} {dp:?}"
    );
    let l = pixel(&light[1], 160, 90);
    let lp = pixel(&light_plain[1], 160, 90);
    assert!(l[0] > l[2] + 4, "highlights lean warm: {l:?}");
    assert!(
        (l[0] - lp[0]).abs() <= 1,
        "the highlight tint keeps its full channel: {l:?} {lp:?}"
    );
}
