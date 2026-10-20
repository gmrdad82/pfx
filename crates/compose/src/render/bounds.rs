use pfx_sound::Clip;

use super::RATE;
use super::master::ROOM_STEM;
use crate::{Note, Patch, Score, Style};

pub const MAX_SECONDS: f64 = 600.0;
pub const MAX_FRAMES: u64 = MAX_SECONDS as u64 * RATE as u64;
pub const SAMPLE_RATES: [u32; 2] = [8_000, 192_000];
const MAX_MS: f64 = 60_000.0;
const MAX_BPM: u32 = 400;
const MAX_BEATS_PER_BAR: u32 = 32;
const TEMPO: [f64; 2] = [0.25, 4.0];

fn within(what: &str, value: f64, low: f64, high: f64) -> Result<(), String> {
    if value >= low && value <= high {
        Ok(())
    } else {
        Err(format!(
            "`{what}` is {value}; the render wants {low} to {high}"
        ))
    }
}

fn finite(what: &str, values: &[f64]) -> Result<(), String> {
    match values.iter().find(|v| !v.is_finite()) {
        Some(v) => Err(format!("`{what}` is {v}; the render wants a finite number")),
        None => Ok(()),
    }
}

fn env(what: &str, env: [f32; 4]) -> Result<(), String> {
    let [attack, decay, sustain, release] = env.map(f64::from);
    for stage in [attack, decay, release] {
        within(what, stage, 0.0, MAX_MS)?;
    }
    within(what, sustain, 0.0, 1.0)
}

fn patch(patch: &Patch) -> Result<(), String> {
    env("env", patch.env)?;
    within(
        "sweep_ms",
        f64::from(patch.sweep_ms.unwrap_or(0.0)),
        0.0,
        MAX_MS,
    )?;
    finite("gain", &[f64::from(patch.gain), f64::from(patch.send)])?;
    for (k, osc) in patch.osc.iter().enumerate() {
        let at = |e: String| format!("osc {}: {e}", k + 1);
        within("at_ms", f64::from(osc.at_ms), 0.0, MAX_MS).map_err(at)?;
        within("glide_ms", f64::from(osc.glide_ms), 0.0, MAX_MS).map_err(at)?;
        if let Some(e) = osc.env {
            env("env", e).map_err(at)?;
        }
        finite(
            "cents, gain or bend",
            &[
                f64::from(osc.cents),
                f64::from(osc.gain),
                f64::from(osc.bend),
            ],
        )
        .map_err(at)?;
    }
    Ok(())
}

fn notes(notes: &[Note]) -> Result<(), String> {
    for note in notes {
        let row = [
            note.beat, note.midi, note.beats, note.vel, note.bend, note.fade,
        ];
        if row.iter().any(|v| !v.is_finite()) || note.beat < 0.0 || note.beats < 0.0 {
            return Err(format!(
                "the note {:?} wants finite values and a beat and length from 0 up",
                note.row()
            ));
        }
        within("midi", note.midi + note.bend, -128.0, 256.0)?;
        within("fade", note.fade, 0.0, MAX_SECONDS)?;
    }
    Ok(())
}

pub fn check(style: &Style, score: &Score) -> Result<(), String> {
    within("seconds", score.seconds, 1.0 / f64::from(RATE), MAX_SECONDS)?;
    if !(1..=MAX_BPM).contains(&score.bpm) {
        return Err(format!(
            "`bpm` is {}; the render wants 1 to {MAX_BPM}",
            score.bpm
        ));
    }
    if !(1..=MAX_BEATS_PER_BAR).contains(&score.beats_per_bar) {
        return Err(format!(
            "`beats_per_bar` is {}; the render wants 1 to {MAX_BEATS_PER_BAR}",
            score.beats_per_bar
        ));
    }
    for s in &score.sections {
        within("tempo", s.tempo, TEMPO[0], TEMPO[1])
            .map_err(|e| format!("[[section]] `{}`: {e}", s.name))?;
    }
    for track in &score.tracks {
        notes(&track.notes).map_err(|e| format!("[[track]] `{}`: {e}", track.role))?;
        for glide in &track.lowpass {
            finite("lowpass", &[glide.beat, glide.hz, glide.beats])
                .map_err(|e| format!("[[track]] `{}`: {e}", track.role))?;
        }
    }
    for cue in &score.cues {
        let at = |e: String| format!("[[cue]] `{}`: {e}", cue.name);
        within("beats", cue.beats, 0.0, MAX_SECONDS).map_err(at)?;
        for voice in &cue.voices {
            finite("gain", &[f64::from(voice.gain)]).map_err(at)?;
            notes(&voice.notes).map_err(at)?;
        }
    }
    for fire in &score.fires {
        within("at", fire.at, 0.0, score.seconds).map_err(|e| format!("[[fire]]: {e}"))?;
    }
    if let Some(role) = style.roles.iter().find(|r| r.name == ROOM_STEM) {
        return Err(format!(
            "role `{}` shares its name with the room stem; rename the role",
            role.name
        ));
    }
    for (name, p) in &style.patch {
        patch(p).map_err(|e| format!("[patch.{name}]: {e}"))?;
    }
    let master = &style.master;
    finite(
        "master",
        &[
            master.lufs,
            f64::from(master.true_peak),
            f64::from(master.room[0]),
            f64::from(master.room[1]),
            f64::from(master.room[2]),
        ],
    )?;
    within("master.fade", master.fade, 0.0, MAX_SECONDS)?;
    if let Some(sidechain) = &master.sidechain {
        finite("master.sidechain.depth", &[f64::from(sidechain.depth)])?;
        within(
            "master.sidechain.release_ms",
            f64::from(sidechain.release_ms),
            0.0,
            MAX_MS,
        )?;
    }
    Ok(())
}

pub fn frames(what: &str, frames: u64) -> Result<usize, String> {
    if frames > MAX_FRAMES {
        return Err(format!(
            "{what} would render {:.1} s; the render holds at most {MAX_SECONDS} s",
            frames as f64 / f64::from(RATE)
        ));
    }
    usize::try_from(frames).map_err(|_| format!("{what} is too long to render"))
}

pub fn sample(name: &str, clip: &Clip) -> Result<(), String> {
    let [low, high] = SAMPLE_RATES;
    if !(low..=high).contains(&clip.rate) || clip.channels == 0 {
        return Err(format!(
            "sample `{name}` is at {} Hz with {} channels; the render reads {low} to {high} Hz",
            clip.rate, clip.channels
        ));
    }
    let frames = clip.samples.len() as u64 / u64::from(clip.channels);
    let resampled = u128::from(frames) * u128::from(RATE) / u128::from(clip.rate);
    if resampled > u128::from(MAX_FRAMES) {
        return Err(format!(
            "sample `{name}` lasts over {MAX_SECONDS} s; the render holds at most that"
        ));
    }
    Ok(())
}
