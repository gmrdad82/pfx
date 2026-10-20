use std::collections::BTreeMap;

use pfx_core::sim;
use pfx_sound::{Drive, Envelope, Glide as Sweep, NoteRender, Shape, Wave as Voice};

use super::RATE;
use crate::{DriveCurve, Glide, Note, Osc, Patch, Tempo, Wave};

pub struct Stem<'a> {
    pub patch: &'a Patch,
    pub notes: &'a [Note],
    pub lowpass: &'a [Glide],
    pub gain: f32,
    pub seed: u64,
}

pub fn hz(midi: f64) -> f32 {
    (440.0 * sim::pow(2.0, (midi - 69.0) / 12.0)) as f32
}

fn ratio(semitones: f64) -> f64 {
    sim::pow(2.0, semitones / 12.0)
}

fn voice(osc: &Osc) -> Option<Voice> {
    Some(match osc.wave? {
        Wave::Sine => Voice::Sine,
        Wave::Square => Voice::Square,
        Wave::Saw => Voice::Saw,
        Wave::Triangle => Voice::Triangle,
        Wave::Pulse => Voice::Pulse(osc.width.unwrap_or(0.5)),
        Wave::White => Voice::White,
        Wave::Pink => Voice::Pink,
    })
}

pub fn drive(spec: &(DriveCurve, f32, f32)) -> Drive {
    let (curve, drive_db, mix) = *spec;
    Drive {
        drive_db,
        curve: match curve {
            DriveCurve::Soft => pfx_sound::DriveCurve::Soft,
            DriveCurve::Tanh => pfx_sound::DriveCurve::Tanh,
            DriveCurve::Hard => pfx_sound::DriveCurve::Hard,
        },
        mix,
        compensate: false,
        output_db: 0.0,
    }
}

fn envelope(e: [f32; 4], linear: bool) -> Envelope {
    let [attack_ms, decay_ms, sustain, release_ms] = e;
    Envelope {
        attack_ms,
        decay_ms,
        sustain,
        release_ms,
        shape: if linear {
            Shape::Linear
        } else {
            Shape::Exponential
        },
    }
}

pub fn ms(ms: f64) -> u64 {
    (ms.max(0.0) * f64::from(RATE) / 1000.0).round() as u64
}

pub fn tail(patch: &Patch) -> u64 {
    patch
        .osc
        .iter()
        .map(|o| {
            let [a, d, _, r] = o.env.unwrap_or(patch.env);
            ms(f64::from(o.at_ms + a + d + r))
        })
        .max()
        .unwrap_or(0)
}

fn sample(buffer: &mut [f32], source: &[f32], at: u64, step: f64, gain: f32, pan: f32) {
    let length = source.len() / 2;
    let left = (1.0 - pan).min(1.0) * gain;
    let right = (1.0 + pan).min(1.0) * gain;
    let frames = buffer.len() / 2;
    let mut read = 0.0f64;
    let mut write = at as usize;
    while write < frames {
        let i = read as usize;
        if i + 1 >= length {
            break;
        }
        let t = (read - i as f64) as f32;
        let l = source[i * 2] + (source[i * 2 + 2] - source[i * 2]) * t;
        let r = source[i * 2 + 1] + (source[i * 2 + 3] - source[i * 2 + 1]) * t;
        buffer[write * 2] += l * left;
        buffer[write * 2 + 1] += r * right;
        read += step;
        write += 1;
    }
}

pub fn render(
    stem: &Stem,
    frame: &dyn Fn(f64) -> u64,
    frames: usize,
    clips: &BTreeMap<String, Vec<f32>>,
) -> Result<Vec<f32>, String> {
    let patch = stem.patch;
    let mut out = NoteRender::new(RATE, frames);
    if let Some(h) = patch.highpass {
        out.highpass(0, Some(h), 0);
    }
    if let Some(spec) = &patch.drive {
        out.drive(0, Some(drive(spec)));
    }
    let mut events: Vec<(u64, bool, usize)> = Vec::new();
    for (i, glide) in stem.lowpass.iter().enumerate() {
        events.push((frame(glide.beat), false, i));
    }
    for (i, note) in stem.notes.iter().enumerate() {
        events.push((frame(note.beat), true, i));
    }
    events.sort_unstable();
    let mut set: Option<(u64, f64)> = None;
    for (at, is_note, index) in events {
        if !is_note {
            let glide = &stem.lowpass[index];
            if set == Some((at, glide.hz)) {
                continue;
            }
            set = Some((at, glide.hz));
            let span = frame(glide.beat + glide.beats).saturating_sub(at);
            out.lowpass(at, Some(glide.hz as f32), span);
            continue;
        }
        let note = &stem.notes[index];
        let end = frame(note.beat + note.beats).max(at + 1);
        if let Some([from, to]) = patch.sweep {
            let span = patch.sweep_ms.map_or(end - at, |m| ms(f64::from(m)));
            out.lowpass(at, Some(from), 0);
            out.lowpass(at, Some(to), span);
        }
        for (k, osc) in patch.osc.iter().enumerate() {
            let start = at + ms(f64::from(osc.at_ms));
            let gain = note.vel.max(0.0) as f32 * osc.gain * patch.gain * stem.gain;
            let pan = (patch.pan + osc.pan).clamp(-1.0, 1.0);
            let transpose = 12.0 * f64::from(osc.octave) + f64::from(osc.cents) / 100.0;
            let Some(wave) = voice(osc) else {
                let name = osc.sample.as_deref().unwrap_or_default();
                let source = clips
                    .get(name)
                    .ok_or_else(|| format!("sample `{name}` is not in the document"))?;
                let step = ratio(note.midi - 60.0 + transpose);
                sample(out.buffer_mut(), source, start, step, gain, pan);
                continue;
            };
            let f = osc.hz.unwrap_or_else(|| hz(note.midi + transpose));
            let e = osc.env.unwrap_or(patch.env);
            let bend = f64::from(osc.bend) + note.bend;
            let glide = if bend != 0.0 {
                let frames = if osc.glide_ms > 0.0 && osc.to_hz.is_none() {
                    ms(f64::from(osc.glide_ms))
                } else {
                    end - at
                };
                Some(Sweep {
                    after: 0,
                    frames,
                    to_hz: f * ratio(bend) as f32,
                })
            } else {
                osc.to_hz.map(|to_hz| Sweep {
                    after: 0,
                    frames: ms(f64::from(osc.glide_ms)),
                    to_hz,
                })
            };
            let held = if e[2] > 0.0 {
                end - at
            } else {
                ms(f64::from(e[0] + e[1])) + 96
            };
            out.note(&pfx_sound::Note {
                at: start,
                held: held.max(1),
                wave,
                hz: f,
                glide,
                gain,
                pan,
                envelope: envelope(e, patch.linear),
                fade: (note.fade > 0.0).then(|| ms(note.fade * 1000.0)),
                seed: stem
                    .seed
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add(((index as u64) << 8) | k as u64),
            });
        }
    }
    Ok(out.finish())
}

pub fn tempo_frames(tempo: &Tempo) -> impl Fn(f64) -> u64 + '_ {
    move |beat| (tempo.seconds(beat) * f64::from(RATE)).round().max(0.0) as u64
}
