pub mod bounds;
pub mod game;
pub mod master;
pub mod music;
pub mod patch;
pub mod store;

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use pfx_sound::Clip;

use crate::{Score, ScoreCue, Style, Tempo};
use master::{Chain, Mastered, Sidechain, Stem};

pub use master::Target;

pub const RATE: u32 = 48_000;
pub const CUES: &str = "cue";
pub const RENDER: [&str; 3] = [
    "d8fff70dda5bc8fc23ab9d5457d31a50718382a4c4f18b43873a3f8fdbff8654",
    "38c11307a4bea258b304c80e590108bf4e0dab091890728710d13d287c7185d4",
    "0000000000000000000000000000000000000000000000000000000000000000",
];

pub type Samples<'a> = dyn Fn(&str) -> Result<Option<Vec<u8>>, String> + 'a;

pub struct Rendered {
    pub stems: Vec<Stem>,
    pub cues: Vec<Stem>,
    pub master: Mastered,
    pub render_ms: f64,
    pub master_ms: f64,
}

pub fn wav(samples: Vec<f32>) -> Vec<u8> {
    Clip {
        rate: RATE,
        channels: 2,
        samples,
    }
    .to_wav()
}

pub fn threads(jobs: usize) -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(jobs)
        .max(1)
}

fn clips(
    style: &Style,
    samples: &Samples,
    mut held: u64,
) -> Result<BTreeMap<String, Vec<f32>>, String> {
    let mut clips = BTreeMap::new();
    for name in bounds::samples(style) {
        let bytes =
            samples(name)?.ok_or_else(|| format!("sample `{name}` is not in the document"))?;
        bounds::sample(name, &bytes, &mut held)?;
        let clip =
            Clip::from_wav(&bytes).ok_or_else(|| format!("sample `{name}` is no WAV pfx reads"))?;
        drop(bytes);
        clips.insert(name.to_string(), clip.at_rate(RATE, 2));
    }
    Ok(clips)
}

fn patch<'a>(style: &'a Style, name: &str, of: &str) -> Result<&'a crate::Patch, String> {
    style
        .patch
        .get(name)
        .ok_or_else(|| format!("{of} names patch `{name}`, which the style lacks"))
}

pub fn cue_frames(style: &Style, score: &Score, cue: &ScoreCue) -> Result<u64, String> {
    let tempo = Tempo::new(f64::from(score.bpm), score.beats_per_bar, &[]);
    let frame = patch::tempo_frames(&tempo);
    let mut length = frame(cue.beats);
    for voice in &cue.voices {
        let p = patch(style, &voice.patch, &format!("cue `{}`", cue.name))?;
        let last = voice
            .notes
            .iter()
            .map(|n| frame(n.beat + n.beats))
            .max()
            .unwrap_or(0);
        length = length.max(last.saturating_add(patch::tail(p)));
    }
    bounds::frames(&format!("cue `{}`", cue.name), length).map(|f| f as u64)
}

fn cue(
    style: &Style,
    score: &Score,
    cue: &ScoreCue,
    seed: u64,
    clips: &BTreeMap<String, Vec<f32>>,
) -> Result<Stem, String> {
    let tempo = Tempo::new(f64::from(score.bpm), score.beats_per_bar, &[]);
    let frame = patch::tempo_frames(&tempo);
    let frames = bounds::frames(
        &format!("cue `{}`", cue.name),
        cue_frames(style, score, cue)?,
    )?;
    let mut samples = vec![0.0f32; frames * 2];
    for (vi, voice) in cue.voices.iter().enumerate() {
        let p = patch(style, &voice.patch, &format!("cue `{}`", cue.name))?;
        let rendered = patch::render(
            &patch::Stem {
                patch: p,
                notes: &voice.notes,
                lowpass: &[],
                gain: voice.gain,
                seed: seed.wrapping_add(vi as u64 + 1),
            },
            &frame,
            frames,
            clips,
        )
        .map_err(|e| format!("cue `{}`: {e}", cue.name))?;
        for (s, v) in samples.iter_mut().zip(rendered) {
            *s += v;
        }
    }
    Ok(Stem {
        name: cue.name.clone(),
        samples,
    })
}

type Job<'a> = (
    usize,
    &'a crate::Role,
    &'a crate::Patch,
    Option<&'a crate::Track>,
);

fn stems(
    jobs: &[Job],
    score: &Score,
    clips: &BTreeMap<String, Vec<f32>>,
    frames: usize,
) -> Vec<Result<Vec<f32>, String>> {
    let tempo = score.tempo();
    let next = AtomicUsize::new(0);
    let slots: Vec<OnceLock<Result<Vec<f32>, String>>> =
        jobs.iter().map(|_| OnceLock::new()).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads(jobs.len()))
            .map(|_| {
                scope.spawn(|| {
                    let frame = patch::tempo_frames(&tempo);
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some((ri, role, p, track)) = jobs.get(index) else {
                            break;
                        };
                        let seed = score.seed.wrapping_add((*ri as u64 + 1).wrapping_mul(7919));
                        let done = patch::render(
                            &patch::Stem {
                                patch: p,
                                notes: track.map_or(&[][..], |t| t.notes.as_slice()),
                                lowpass: track.map_or(&[][..], |t| t.lowpass.as_slice()),
                                gain: role.gain,
                                seed,
                            },
                            &frame,
                            frames,
                            clips,
                        )
                        .map_err(|e| format!("role `{}`: {e}", role.name));
                        let _ = slots[index].set(done);
                    }
                })
            })
            .collect();
        for worker in workers {
            let _ = worker.join();
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(|| Err("a stem's render panicked".into()))
        })
        .collect()
}

pub fn render(
    style: &Style,
    score: &Score,
    samples: &Samples,
    clock: &dyn Fn() -> f64,
    target: Target,
) -> Result<Rendered, String> {
    let started = clock();
    bounds::check(style, score)?;
    let frames = bounds::frames(
        "the score",
        (score.seconds * f64::from(RATE)).round() as u64,
    )?;
    let held = bounds::budget(style, score, frames)?;
    let clips = clips(style, samples, held)?;
    let mut jobs: Vec<Job> = Vec::new();
    for (ri, role) in style.roles.iter().enumerate() {
        let p = patch(style, &role.patch, &format!("role `{}`", role.name))?;
        jobs.push((ri, role, p, score.track(&role.name)));
    }
    let rendered = stems(&jobs, score, &clips, frames);
    let mut stems = Vec::with_capacity(rendered.len());
    let mut sends = Vec::with_capacity(rendered.len());
    for ((_, role, p, _), samples) in jobs.iter().zip(rendered) {
        stems.push(Stem {
            name: role.name.clone(),
            samples: samples?,
        });
        sends.push(p.send);
    }
    let mut cues = Vec::with_capacity(score.cues.len());
    for (ci, c) in score.cues.iter().enumerate() {
        let seed = score.seed ^ (0xC0E5 + ci as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        cues.push(cue(style, score, c, seed, &clips)?);
    }
    let fired: Vec<(u64, &Stem)> = score
        .fires
        .iter()
        .map(|fire| {
            let stem = cues.iter().find(|c| c.name == fire.cue).ok_or_else(|| {
                format!(
                    "the fire at {} s names cue `{}`, which the score lacks",
                    fire.at, fire.cue
                )
            })?;
            Ok(((fire.at * f64::from(RATE)).round() as u64, stem))
        })
        .collect::<Result<_, String>>()?;
    let rendered_at = clock();
    let master = &style.master;
    let chain = Chain {
        lufs: master.lufs,
        true_peak: master.true_peak,
        room: master.room,
        sidechain: master.sidechain.as_ref().map(|s| Sidechain {
            keys: &s.keys,
            beds: &s.beds,
            depth: s.depth,
            release_ms: s.release_ms,
        }),
        fade: master.fade,
    };
    let (mastered, gain) = master::chain(&mut stems, &sends, &fired, &chain, frames, target);
    if target == Target::Game {
        for c in &mut cues {
            master::limit_cue(&mut c.samples, gain, master.true_peak);
        }
    }
    let done = clock();
    Ok(Rendered {
        stems,
        cues,
        master: mastered,
        render_ms: (rendered_at - started) * 1000.0,
        master_ms: (done - rendered_at) * 1000.0,
    })
}

pub fn recipe(style: &Style, score: &Score, samples: &Samples) -> Result<String, String> {
    bounds::check(style, score)?;
    let mut all = format!("{RENDER:?}\n{style:?}\n{score:?}\n").into_bytes();
    for name in bounds::samples(style) {
        let bytes = samples(name)?.unwrap_or_default();
        all.extend(name.as_bytes());
        all.extend((bytes.len() as u64).to_le_bytes());
        all.extend(pfx_file::sha256::sha256(&bytes));
    }
    Ok(pfx_file::sha256::hex(&pfx_file::sha256::sha256(&all)))
}

pub fn marks_music(text: &str, name: &str) -> bool {
    let table: Option<toml::Table> = text.parse().ok();
    table
        .as_ref()
        .and_then(|t| t.get("music"))
        .and_then(toml::Value::as_table)
        .and_then(|m| m.get(pfx_sound::manifest::SCORE))
        .and_then(toml::Value::as_str)
        == Some(name)
}

pub fn cue_file(score: &str, cue: &str) -> String {
    format!("{}/{score}/{CUES}/{cue}.wav", store::FOLDER)
}

pub fn written(style: &Style, score: &Score, name: &str) -> Vec<String> {
    stem_names(style)
        .iter()
        .map(|stem| store::stem_file(name, stem))
        .chain(score.cues.iter().map(|c| cue_file(name, &c.name)))
        .collect()
}

pub fn stem_names(style: &Style) -> Vec<String> {
    let mut names: Vec<String> = style.roles.iter().map(|r| r.name.clone()).collect();
    let sends = style
        .roles
        .iter()
        .any(|r| style.patch.get(&r.patch).is_some_and(|p| p.send > 0.0));
    if style.master.room[2] > 0.0 && sends {
        names.push(master::ROOM_STEM.to_string());
    }
    names
}

pub fn music_table(style: &Style, score: &Score, name: &str) -> toml::Table {
    let bpb = f64::from(score.beats_per_bar);
    let parts: Vec<music::Part> = score
        .sections
        .iter()
        .map(|s| music::Part {
            name: s.name.clone(),
            at: s.at,
            intensity: s.intensity,
        })
        .collect();
    let lines: Vec<music::Line> = stem_names(style)
        .into_iter()
        .map(|stem| {
            let notes = score.track(&stem).map_or(&[][..], |t| t.notes.as_slice());
            let room = style.role(&stem).is_none();
            music::Line {
                file: store::stem_file(name, &stem),
                name: stem,
                plays: score
                    .sections
                    .iter()
                    .map(|s| {
                        let from = f64::from(s.from) * bpb;
                        let end = f64::from(s.from.saturating_add(s.bars)) * bpb;
                        room || notes.iter().any(|n| n.beat >= from - 1e-9 && n.beat < end)
                    })
                    .collect(),
            }
        })
        .collect();
    let stings: Vec<music::Sting> = score
        .cues
        .iter()
        .map(|c| music::Sting {
            name: c.name.clone(),
            event: c.event.clone(),
            file: cue_file(name, &c.name),
        })
        .collect();
    let mut table = music::table(
        f64::from(score.bpm),
        score.beats_per_bar,
        score.seconds,
        &parts,
        &lines,
        &stings,
    );
    table.insert(
        pfx_sound::manifest::SCORE.into(),
        toml::Value::String(name.to_string()),
    );
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_STYLE: &str = r#"
[compose.style.test]
bpm = [100, 110]
tonics = [["C", 1], ["G", 1]]
modes = [["major", 2], ["dorian", 1]]

[compose.style.test.harmony]
beats = 4
start = ["I"]
cadence = ["V", "I"]
markov = { I = [["IV", 2], ["vi", 1]], IV = [["V", 2]], V = [["I", 2]], vi = [["IV", 1]] }

[compose.style.test.melody]
range = [62, 79]
steps = [[1, 3], [-1, 3], [2, 1], [-2, 1]]
rhythm = [[0.5, 2], [1.0, 1]]
rest = 0.1
motif_bars = 1
repeat = 0.5

[[compose.style.test.role]]
name = "kick"
patch = "kick"
plays = "hit"
patterns = ["x...x...x...x..."]
gain = 0.5

[[compose.style.test.role]]
name = "pad"
patch = "pad"
plays = "chord"
octave = -1
gain = 0.05

[[compose.style.test.role]]
name = "lead"
patch = "lead"
plays = "melody"
octave = 1
gain = 0.08

[compose.style.test.patch.kick]
osc = [{ wave = "sine", hz = 150, to_hz = 55, glide_ms = 40 }, { wave = "white", gain = 0.2, env = [1, 5, 0.0, 0] }]
env = [1, 300, 0.0, 0]
drive = ["tanh", 3.0, 0.3]

[compose.style.test.patch.pad]
osc = [{ wave = "saw", cents = -8, pan = -0.4 }, { wave = "triangle", cents = 8, pan = 0.4 }]
env = [30, 0, 0.8, 200]
lowpass = [900, 2400]
send = 0.2

[compose.style.test.patch.lead]
osc = [{ wave = "pulse", width = 0.3 }, { wave = "sine", octave = -1, gain = 0.5 }]
env = [3, 250, 0.0, 0]
sweep = [5000, 1200]
sweep_ms = 300
send = 0.1

[[compose.style.test.section]]
name = "calm"
bars = [2, 3]
roles = ["kick", "pad", "lead"]
intensity = [0.0, 0.4]
round_bars = 1
start_roles = 1

[[compose.style.test.section]]
name = "lift"
bars = [2, 3]
roles = ["kick", "pad", "lead"]
intensity = [0.7, 1.0]
hit = 0
cue = "sting"

[[compose.style.test.cue]]
name = "sting"
event = "level_up"

[[compose.style.test.cue.voice]]
patch = "lead"
plays = "glide"
hz = [660, 990]
beats = 1
fade = 0.2

[compose.style.test.master]
lufs = -16
true_peak = -1.0
room = [0.6, 0.4, 0.15]
fade = 0.5
sidechain = { keys = ["kick"], beds = ["pad"], depth = 0.4, release_ms = 150 }
"#;

    fn hash(film: &Rendered, game: Option<&Rendered>) -> String {
        let mut all = wav(film.master.mix.clone());
        for stem in game.iter().flat_map(|g| g.stems.iter().chain(&g.cues)) {
            all.extend(stem.name.as_bytes());
            all.extend(wav(stem.samples.clone()));
        }
        pfx_file::sha256::hex(&pfx_file::sha256::sha256(&all))
    }

    fn scored(text: &str, style: &str, seed: u64, seconds: f64, hits: &[f64]) -> Score {
        let doc = crate::Compose::parse(text).unwrap();
        crate::compose(
            style,
            doc.style(style).unwrap(),
            &crate::Request {
                seed,
                seconds,
                hits: hits.to_vec(),
                bpm: None,
                key: None,
            },
        )
        .unwrap()
    }

    fn rendered(text: &str, style: &str, score: &Score, target: Target) -> Rendered {
        let doc = crate::Compose::parse(text).unwrap();
        let none = |_: &str| Ok(None);
        render(doc.style(style).unwrap(), score, &none, &|| 0.0, target).unwrap()
    }

    fn under_the_ceiling(out: &Rendered, ceiling: f32) {
        let sum = master::sum(&out.stems, out.master.mix.len() / 2);
        let stems = pfx_sound::decibels(pfx_sound::true_peak(&sum, 2));
        assert!(stems <= ceiling + 1e-3, "the stems sum to {stems} dBTP");
        for cue in &out.cues {
            let peak = pfx_sound::decibels(pfx_sound::true_peak(&cue.samples, 2));
            assert!(
                peak <= ceiling + 1e-3,
                "cue `{}` peaks at {peak} dBTP",
                cue.name
            );
        }
    }

    #[test]
    fn the_test_style_renders_the_same_bytes_for_the_same_seed() {
        let score = scored(TEST_STYLE, "test", 3, 8.0, &[4.0]);
        let film = rendered(TEST_STYLE, "test", &score, Target::Film);
        assert_eq!(film.master.mix.len(), 8 * RATE as usize * 2);
        assert!(film.stems.iter().any(|s| s.name == master::ROOM_STEM));
        assert_eq!(score.fires.len(), 1);
        assert_eq!(film.cues.len(), 1);
        assert!(film.cues[0].samples.iter().any(|v| v.abs() > 1e-4));
        assert!(film.stems.iter().all(|s| s.name != "sting"));
        assert!(
            (film.master.short_term_max + 16.0).abs() < 0.5,
            "{}",
            film.master.short_term_max
        );
        assert!(film.master.true_peak_db <= -1.0 + 1e-3);
        let game = rendered(TEST_STYLE, "test", &score, Target::Game);
        under_the_ceiling(&game, -1.0);
        let style = crate::Compose::parse(TEST_STYLE).unwrap();
        let style = style.style("test").unwrap();
        let text = crate::write_score(TEST_STYLE, "kept", &score).unwrap();
        let text = store::set_table(&text, "music", music_table(style, &score, "kept")).unwrap();
        let bytes = pfx_file::Writer::new()
            .named(
                pfx_file::SCNE,
                "main.toml",
                text.as_bytes(),
                pfx_file::Codec::Auto,
            )
            .to_vec()
            .unwrap();
        let refused = pfx_sound::manifest::music_bytes(&bytes, None)
            .err()
            .unwrap();
        assert!(
            refused.contains("pfx_compose::render::game::music"),
            "{refused}"
        );
        let music = game::music(&bytes, None).unwrap();
        assert_eq!(music.track.stems.len(), game.stems.len());
        for stem in &game.stems {
            let loaded = music
                .track
                .stems
                .iter()
                .find(|s| s.name == stem.name)
                .unwrap();
            assert!(loaded.clip.samples == stem.samples, "stem `{}`", stem.name);
        }
        assert_eq!(music.stingers.len(), 1);
        assert!(music.stingers[0].clip.samples == game.cues[0].samples);
        let other = scored(TEST_STYLE, "test", 4, 8.0, &[4.0]);
        assert_ne!(
            hash(&film, None),
            hash(&rendered(TEST_STYLE, "test", &other, Target::Film), None)
        );
        assert_eq!(hash(&film, Some(&game)), RENDER[0]);
    }

    #[test]
    fn the_sample_build_drop_style_renders_its_pinned_bytes() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/chase.pitofx");
        let doc = store::Document::open(&path).unwrap();
        let score = scored(&doc.text, "build_drop", 11, 30.0, &[18.0, 26.0]);
        assert_eq!(score.bpm, 120);
        let out = rendered(&doc.text, "build_drop", &score, Target::Film);
        assert!(
            (out.master.short_term_max + 16.0).abs() < 0.5,
            "{}",
            out.master.short_term_max
        );
        assert!(out.master.true_peak_db <= -1.0 + 1e-3);
        assert_eq!(hash(&out, None), RENDER[1]);
    }

    fn kick_style() -> String {
        TEST_STYLE.replace(
            r#"osc = [{ wave = "sine", hz = 150, to_hz = 55, glide_ms = 40 }, { wave = "white", gain = 0.2, env = [1, 5, 0.0, 0] }]"#,
            r#"osc = [{ sample = "kick.wav" }]"#,
        )
    }

    fn kick(rate: u32) -> Vec<u8> {
        Clip {
            rate,
            channels: 1,
            samples: (0..2_400)
                .map(|i| pfx_core::sim::sinf(i as f32 * 0.03) * (1.0 - i as f32 / 2_400.0))
                .collect(),
        }
        .to_wav()
    }

    #[test]
    fn a_sample_oscillator_plays_its_chunk_and_a_missing_or_odd_chunk_is_refused() {
        let text = kick_style();
        assert_ne!(text, TEST_STYLE);
        let doc = crate::Compose::parse(&text).unwrap();
        let style = doc.style("test").unwrap();
        let score = scored(&text, "test", 3, 4.0, &[2.0]);
        let wav = kick(24_000);
        let samples = |name: &str| Ok((name == "kick.wav").then(|| wav.clone()));
        let out = render(style, &score, &samples, &|| 0.0, Target::Film).unwrap();
        let stem = out.stems.iter().find(|s| s.name == "kick").unwrap();
        assert!(stem.samples.iter().any(|v| v.abs() > 1e-3));
        assert_eq!(hash(&out, None), RENDER[2]);
        let none = |_: &str| Ok(None);
        let refused = render(style, &score, &none, &|| 0.0, Target::Film)
            .err()
            .unwrap();
        assert!(
            refused.contains("`kick.wav` is not in the document"),
            "{refused}"
        );
        let slow = kick(1);
        let samples = |name: &str| Ok((name == "kick.wav").then(|| slow.clone()));
        let refused = render(style, &score, &samples, &|| 0.0, Target::Film)
            .err()
            .unwrap();
        assert!(
            refused.contains("sample `kick.wav` is at 1 Hz"),
            "{refused}"
        );
    }

    #[test]
    fn a_score_past_the_render_bounds_is_refused_before_it_allocates() {
        let doc = crate::Compose::parse(TEST_STYLE).unwrap();
        let style = doc.style("test").unwrap();
        let score = scored(TEST_STYLE, "test", 3, 4.0, &[2.0]);
        let none = |_: &str| Ok(None);
        let refused = |score: &Score| {
            render(style, score, &none, &|| 0.0, Target::Film)
                .err()
                .unwrap()
        };
        let mut long = score.clone();
        long.seconds = f64::INFINITY;
        assert!(refused(&long).contains("`seconds`"));
        long.seconds = 1e7;
        assert!(refused(&long).contains("`seconds`"));
        let mut still = score.clone();
        still.bpm = 0;
        assert!(refused(&still).contains("`bpm`"));
        let mut cue = score.clone();
        cue.cues[0].beats = 1e12;
        assert!(refused(&cue).contains("`beats`"));
        let mut note = score.clone();
        note.cues[0].voices[0].notes[0].beats = 1e12;
        assert!(refused(&note).contains("would render"));
        let mut nan = score.clone();
        nan.sections[0].tempo = f64::NAN;
        assert!(refused(&nan).contains("`tempo`"));
        let roomy = TEST_STYLE.replace("name = \"lead\"", "name = \"room\"");
        let roomy = crate::Compose::parse(&roomy).unwrap();
        let err = render(
            roomy.style("test").unwrap(),
            &score,
            &none,
            &|| 0.0,
            Target::Film,
        )
        .err()
        .unwrap();
        assert!(err.contains("room stem"), "{err}");
    }

    fn wav8(rate: u32, channels: u16, bytes: usize) -> Vec<u8> {
        let mut out = b"RIFF".to_vec();
        out.extend(((36 + bytes) as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(channels)).to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(8u16.to_le_bytes());
        out.extend(b"data");
        out.extend((bytes as u32).to_le_bytes());
        out.resize(out.len() + bytes, 128);
        out
    }

    #[test]
    fn a_sample_chunk_too_large_or_too_long_is_refused_before_it_decodes() {
        let mut held = 0;
        bounds::sample("kick.wav", &kick(48_000), &mut held).unwrap();
        assert!(held > 0);
        let long = wav8(8_000, 1, (bounds::MAX_FRAMES / 6 + 1) as usize);
        let refused = bounds::sample("long.wav", &long, &mut 0).err().unwrap();
        assert!(
            refused.contains("sample `long.wav` lasts over"),
            "{refused}"
        );
        let wide = wav8(48_000, bounds::MAX_CHANNELS + 1, 64);
        let refused = bounds::sample("wide.wav", &wide, &mut 0).err().unwrap();
        assert!(refused.contains("channels"), "{refused}");
        let mut full = bounds::MAX_BYTES;
        let refused = bounds::sample("kick.wav", &kick(48_000), &mut full)
            .err()
            .unwrap();
        assert!(refused.contains("GiB"), "{refused}");
        let refused = bounds::part("compose/a/kick.wav", &long, &mut 0)
            .err()
            .unwrap();
        assert!(refused.contains("lasts over"), "{refused}");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/compose-tests/capped.pitofx");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        pfx_file::Writer::new()
            .named(
                pfx_file::SCNE,
                "main.toml",
                kick_style().as_bytes(),
                pfx_file::Codec::Auto,
            )
            .named(
                pfx_file::FILE,
                "kick.wav",
                vec![0u8; 4_096].as_slice(),
                pfx_file::Codec::Auto,
            )
            .write(&path)
            .unwrap();
        let stored = store::Document::open(&path).unwrap();
        let refused = stored.file_within("kick.wav", 1_024).err().unwrap();
        assert!(refused.contains("`kick.wav` holds 4096 bytes"), "{refused}");
        assert_eq!(stored.sample("kick.wav").unwrap().unwrap().len(), 4_096);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_style_or_score_past_the_render_counts_is_refused_with_its_key() {
        let doc = crate::Compose::parse(TEST_STYLE).unwrap();
        let base = doc.style("test").unwrap();
        let score = scored(TEST_STYLE, "test", 3, 4.0, &[2.0]);
        let none = |_: &str| Ok(None);
        let refused = |style: &Style, score: &Score| {
            render(style, score, &none, &|| 0.0, Target::Film)
                .err()
                .unwrap()
        };
        let role = base.roles[0].clone();
        let mut crowded = base.clone();
        crowded.roles = (0..=bounds::MAX_ROLES)
            .map(|i| crate::Role {
                name: format!("r{i}"),
                ..role.clone()
            })
            .collect();
        let err = refused(&crowded, &score);
        assert!(err.contains("[[role]]"), "{err}");
        let mut wide = base.clone();
        let pad = wide.patch.get_mut("pad").unwrap();
        pad.osc = vec![pad.osc[0].clone(); bounds::MAX_OSCS + 1];
        let err = refused(&wide, &score);
        assert!(
            err.contains("[patch.pad]") && err.contains("`osc`"),
            "{err}"
        );
        let mut heavy = base.clone();
        let pad = heavy.patch.get_mut("pad").unwrap();
        pad.osc = (0..bounds::MAX_OSCS)
            .map(|k| crate::Osc {
                env: Some([60_000.0, 60_000.0, 0.5, 60_000.0 - k as f32]),
                ..pad.osc[0].clone()
            })
            .collect();
        heavy.roles = (0..bounds::MAX_ROLES)
            .map(|i| crate::Role {
                name: format!("r{i}"),
                patch: "pad".into(),
                ..role.clone()
            })
            .collect();
        let err = refused(&heavy, &score);
        assert!(err.contains("GiB"), "{err}");
        let mut busy = score.clone();
        busy.fires = vec![
            crate::Fire {
                cue: "sting".into(),
                at: 0.0,
            };
            bounds::MAX_FIRES + 1
        ];
        let err = refused(base, &busy);
        assert!(err.contains("[[fire]]"), "{err}");
        let mut nan = base.clone();
        nan.patch.get_mut("pad").unwrap().pan = f32::NAN;
        let err = refused(&nan, &score);
        assert!(err.contains("[patch.pad]") && err.contains("pan"), "{err}");
    }

    #[test]
    fn the_recipe_key_follows_every_input_the_render_reads() {
        let text = kick_style();
        let doc = crate::Compose::parse(&text).unwrap();
        let style = doc.style("test").unwrap();
        let score = scored(&text, "test", 3, 4.0, &[2.0]);
        let wav = kick(24_000);
        let samples = |name: &str| Ok((name == "kick.wav").then(|| wav.clone()));
        let key = recipe(style, &score, &samples).unwrap();
        assert_eq!(key, recipe(style, &score, &samples).unwrap());
        let mut louder = score.clone();
        louder
            .tracks
            .iter_mut()
            .find(|t| !t.notes.is_empty())
            .unwrap()
            .notes[0]
            .vel += 0.01;
        assert_ne!(key, recipe(style, &louder, &samples).unwrap());
        let mut bent = wav.clone();
        let last = bent.len() - 1;
        bent[last] ^= 1;
        let changed = |name: &str| Ok((name == "kick.wav").then(|| bent.clone()));
        assert_ne!(key, recipe(style, &score, &changed).unwrap());
        let none = |_: &str| Ok(None);
        assert_ne!(key, recipe(style, &score, &none).unwrap());
    }
}
