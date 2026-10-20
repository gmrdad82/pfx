pub mod master;
pub mod music;
pub mod patch;
pub mod store;

use std::collections::BTreeMap;

use pfx_sound::Clip;

use crate::{Score, ScoreCue, Style, Tempo};
use master::{Chain, Mastered, Sidechain, Stem};

pub const RATE: u32 = 48_000;
pub const CUES: &str = "cue";

pub type Samples<'a> = dyn Fn(&str) -> Result<Option<Vec<u8>>, String> + 'a;

pub struct Rendered {
    pub stems: Vec<Stem>,
    pub cues: Vec<Stem>,
    pub master: Mastered,
    pub render_ms: f64,
    pub master_ms: f64,
}

pub fn wav(samples: &[f32]) -> Vec<u8> {
    Clip {
        rate: RATE,
        channels: 2,
        samples: samples.to_vec(),
    }
    .to_wav()
}

fn clips(style: &Style, samples: &Samples) -> Result<BTreeMap<String, Vec<f32>>, String> {
    let mut clips = BTreeMap::new();
    for patch in style.patch.values() {
        for name in patch.osc.iter().filter_map(|o| o.sample.as_deref()) {
            if clips.contains_key(name) {
                continue;
            }
            let bytes =
                samples(name)?.ok_or_else(|| format!("sample `{name}` is not in the document"))?;
            let clip = Clip::from_wav(&bytes)
                .ok_or_else(|| format!("sample `{name}` is no WAV pfx reads"))?;
            clips.insert(name.to_string(), clip.at_rate(RATE, 2));
        }
    }
    Ok(clips)
}

fn patch<'a>(style: &'a Style, name: &str, of: &str) -> Result<&'a crate::Patch, String> {
    style
        .patch
        .get(name)
        .ok_or_else(|| format!("{of} names patch `{name}`, which the style lacks"))
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
    let mut length = frame(cue.beats);
    for voice in &cue.voices {
        let p = patch(style, &voice.patch, &format!("cue `{}`", cue.name))?;
        let last = voice
            .notes
            .iter()
            .map(|n| frame(n.beat + n.beats))
            .max()
            .unwrap_or(0);
        length = length.max(last + patch::tail(p));
    }
    let frames = length as usize;
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
        )?;
        for (s, v) in samples.iter_mut().zip(rendered) {
            *s += v;
        }
    }
    Ok(Stem {
        name: cue.name.clone(),
        samples,
    })
}

pub fn render(
    style: &Style,
    score: &Score,
    samples: &Samples,
    clock: &dyn Fn() -> f64,
) -> Result<Rendered, String> {
    let started = clock();
    let frames = (score.seconds * f64::from(RATE)).round() as usize;
    let tempo = score.tempo();
    let clips = clips(style, samples)?;
    let mut jobs = Vec::new();
    for (ri, role) in style.roles.iter().enumerate() {
        let p = patch(style, &role.patch, &format!("role `{}`", role.name))?;
        let track = score.track(&role.name);
        jobs.push((ri, role, p, track));
    }
    let rendered: Vec<Result<Vec<f32>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(ri, role, p, track)| {
                let tempo = &tempo;
                let clips = &clips;
                let seed = score.seed.wrapping_add((*ri as u64 + 1).wrapping_mul(7919));
                scope.spawn(move || {
                    let frame = patch::tempo_frames(tempo);
                    patch::render(
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
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("a stem's render panicked".into()))
            })
            .collect()
    });
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
        .filter_map(|fire| {
            let stem = cues.iter().find(|c| c.name == fire.cue)?;
            Some(((fire.at * f64::from(RATE)).round() as u64, stem))
        })
        .collect();
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
    let (mastered, gain) = master::chain(&mut stems, &sends, &fired, &chain, frames);
    for c in &mut cues {
        for v in &mut c.samples {
            *v *= gain;
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

pub fn cue_file(score: &str, cue: &str) -> String {
    format!("{}/{score}/{CUES}/{cue}.wav", store::FOLDER)
}

pub fn music_table(style: &Style, score: &Score, name: &str, rendered: &Rendered) -> toml::Table {
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
    let lines: Vec<music::Line> = rendered
        .stems
        .iter()
        .map(|stem| {
            let notes = score
                .track(&stem.name)
                .map_or(&[][..], |t| t.notes.as_slice());
            let room = style.role(&stem.name).is_none();
            music::Line {
                name: stem.name.clone(),
                file: store::stem_file(name, &stem.name),
                plays: score
                    .sections
                    .iter()
                    .map(|s| {
                        let from = f64::from(s.from) * bpb;
                        let end = f64::from(s.from + s.bars) * bpb;
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
    music::table(
        f64::from(score.bpm),
        score.beats_per_bar,
        score.seconds,
        &parts,
        &lines,
        &stings,
    )
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

    fn hash(rendered: &Rendered) -> String {
        let mut all = wav(&rendered.master.mix);
        for stem in rendered.stems.iter().chain(&rendered.cues) {
            all.extend(stem.name.as_bytes());
            all.extend(wav(&stem.samples));
        }
        pfx_file::sha256::hex(&pfx_file::sha256::sha256(&all))
    }

    fn rendered(
        text: &str,
        style: &str,
        seed: u64,
        seconds: f64,
        hits: &[f64],
    ) -> (Score, Rendered) {
        let doc = crate::Compose::parse(text).unwrap();
        let name = style;
        let style = doc.style(name).unwrap();
        let score = crate::compose(
            name,
            style,
            &crate::Request {
                seed,
                seconds,
                hits: hits.to_vec(),
                bpm: None,
                key: None,
            },
        )
        .unwrap();
        let none = |_: &str| Ok(None);
        let out = render(style, &score, &none, &|| 0.0).unwrap();
        (score, out)
    }

    #[test]
    fn the_test_style_renders_the_same_bytes_for_the_same_seed() {
        let (score, out) = rendered(TEST_STYLE, "test", 3, 8.0, &[4.0]);
        assert_eq!(out.master.mix.len(), 8 * RATE as usize * 2);
        assert!(out.stems.iter().any(|s| s.name == master::ROOM_STEM));
        assert_eq!(score.fires.len(), 1);
        assert_eq!(out.cues.len(), 1);
        assert!(out.cues[0].samples.iter().any(|v| v.abs() > 1e-4));
        assert!(out.stems.iter().all(|s| s.name != "sting"));
        assert!(
            (out.master.short_term_max + 16.0).abs() < 0.5,
            "{}",
            out.master.short_term_max
        );
        assert!(out.master.true_peak_db <= -1.0 + 1e-3);
        let again = rendered(TEST_STYLE, "test", 3, 8.0, &[4.0]).1;
        assert_eq!(hash(&out), hash(&again));
        assert_ne!(
            hash(&out),
            hash(&rendered(TEST_STYLE, "test", 4, 8.0, &[4.0]).1)
        );
        assert_eq!(
            hash(&out),
            "f6768ba01718af5c638bc86b44beb7a8cb8a26bd4ef11c503c9f59c59610406f"
        );
    }

    #[test]
    fn the_sample_build_drop_style_renders_its_pinned_bytes() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/chase.pitofx");
        let doc = store::Document::open(&path).unwrap();
        let (score, out) = rendered(&doc.text, "build_drop", 11, 30.0, &[18.0, 26.0]);
        assert_eq!(score.bpm, 120);
        assert!(
            (out.master.short_term_max + 16.0).abs() < 0.5,
            "{}",
            out.master.short_term_max
        );
        assert!(out.master.true_peak_db <= -1.0 + 1e-3);
        assert_eq!(
            hash(&out),
            "5ba7b108a4a2f91a51d9b868c3305a4aa469ebc2a3fa2f99e05f60d918b97e2b"
        );
    }
}
