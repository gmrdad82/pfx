use std::collections::BTreeSet;

use pfx_sound::WavInfo;

use super::master::ROOM_STEM;
use super::{RATE, patch as play};
use crate::{Note, Patch, Score, Style};

pub const MAX_SECONDS: f64 = 600.0;
pub const MAX_FRAMES: u64 = MAX_SECONDS as u64 * RATE as u64;
pub const SAMPLE_RATES: [u32; 2] = [8_000, 192_000];
const MAX_MS: f64 = 60_000.0;
const MAX_BPM: u32 = 400;
const MAX_BEATS_PER_BAR: u32 = 32;
const TEMPO: [f64; 2] = [0.25, 4.0];
pub const MAX_GAIN: f64 = 64.0;
pub const OCTAVES: i32 = 8;
pub const MAX_ROLES: usize = 64;
pub const MAX_PATCHES: usize = 256;
pub const MAX_OSCS: usize = 16;
pub const MAX_SECTIONS: usize = 256;
pub const MAX_NOTES: usize = 200_000;
pub const MAX_CUES: usize = 64;
pub const MAX_VOICES: usize = 16;
pub const MAX_FIRES: usize = 4_096;
pub const MAX_SAMPLES: usize = 64;
pub const MAX_CHANNELS: u16 = 8;
pub const MAX_BYTES: u64 = 6 << 30;
pub const MAX_WORK: u64 = 1 << 35;
pub const MAX_SAMPLE_BYTES: u64 = 64 << 20;
pub const MAX_PART_BYTES: u64 = 256 << 20;
const STEREO: u64 = 8;
const WORKING: u64 = 8;
const GIB: f64 = (1u64 << 30) as f64;

fn count(what: &str, n: usize, max: usize) -> Result<(), String> {
    if n > max {
        Err(format!(
            "{what}: {n} of them; the render takes at most {max}"
        ))
    } else {
        Ok(())
    }
}

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

fn gain(what: &str, gain: f32) -> Result<(), String> {
    within(what, f64::from(gain), -MAX_GAIN, MAX_GAIN)
}

fn env(what: &str, env: [f32; 4]) -> Result<(), String> {
    let [attack, decay, sustain, release] = env.map(f64::from);
    for stage in [attack, decay, release] {
        within(what, stage, 0.0, MAX_MS)?;
    }
    within(what, sustain, 0.0, 1.0)
}

fn patch(patch: &Patch) -> Result<(), String> {
    count("`osc`", patch.osc.len(), MAX_OSCS)?;
    env("env", patch.env)?;
    within(
        "sweep_ms",
        f64::from(patch.sweep_ms.unwrap_or(0.0)),
        0.0,
        MAX_MS,
    )?;
    gain("gain", patch.gain)?;
    finite(
        "send or pan",
        &[f64::from(patch.send), f64::from(patch.pan)],
    )?;
    let filters = patch.lowpass.into_iter().flatten();
    let filters = filters.chain(patch.sweep.into_iter().flatten());
    let filters: Vec<f64> = filters.chain(patch.highpass).map(f64::from).collect();
    finite("lowpass, highpass or sweep", &filters)?;
    if let Some((_, drive, mix)) = patch.drive {
        finite("drive", &[f64::from(drive), f64::from(mix)])?;
    }
    for (k, osc) in patch.osc.iter().enumerate() {
        let at = |e: String| format!("osc {}: {e}", k + 1);
        within("at_ms", f64::from(osc.at_ms), 0.0, MAX_MS).map_err(at)?;
        within("glide_ms", f64::from(osc.glide_ms), 0.0, MAX_MS).map_err(at)?;
        if let Some(e) = osc.env {
            env("env", e).map_err(at)?;
        }
        gain("gain", osc.gain).map_err(at)?;
        if !(-OCTAVES..=OCTAVES).contains(&osc.octave) {
            return Err(at(format!(
                "`octave` is {}; the render wants -{OCTAVES} to {OCTAVES}",
                osc.octave
            )));
        }
        finite(
            "cents, bend, pan, width, hz or to_hz",
            &[
                f64::from(osc.cents),
                f64::from(osc.bend),
                f64::from(osc.pan),
                f64::from(osc.width.unwrap_or(0.5)),
                f64::from(osc.hz.unwrap_or(1.0)),
                f64::from(osc.to_hz.unwrap_or(1.0)),
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
    count("[[role]]", style.roles.len(), MAX_ROLES)?;
    count("[patch]", style.patch.len(), MAX_PATCHES)?;
    count("[[section]]", score.sections.len(), MAX_SECTIONS)?;
    count("[[cue]]", score.cues.len(), MAX_CUES)?;
    count("[[fire]]", score.fires.len(), MAX_FIRES)?;
    count("samples", samples(style).len(), MAX_SAMPLES)?;
    let tracked = score.tracks.iter().map(|t| t.notes.len());
    let cued = score
        .cues
        .iter()
        .flat_map(|c| c.voices.iter().map(|v| v.notes.len()));
    count("notes", tracked.chain(cued).sum(), MAX_NOTES)?;
    for role in &style.roles {
        gain("gain", role.gain).map_err(|e| format!("[[role]] `{}`: {e}", role.name))?;
        finite(
            "accent or ramp",
            &[f64::from(role.accent), f64::from(role.ramp)],
        )
        .map_err(|e| format!("[[role]] `{}`: {e}", role.name))?;
    }
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
        count("[[cue.voice]]", cue.voices.len(), MAX_VOICES).map_err(at)?;
        for voice in &cue.voices {
            gain("gain", voice.gain).map_err(at)?;
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

pub fn samples(style: &Style) -> BTreeSet<&str> {
    style
        .patch
        .values()
        .flat_map(|p| p.osc.iter().filter_map(|o| o.sample.as_deref()))
        .collect()
}

fn audio(what: &str, raw: usize, most: u64, info: Option<WavInfo>) -> Result<WavInfo, String> {
    if raw as u64 > most {
        return Err(format!(
            "{what} holds {raw} bytes; the render reads at most {most}"
        ));
    }
    let info = info.ok_or_else(|| format!("{what} is no WAV pfx reads"))?;
    let [low, high] = SAMPLE_RATES;
    if !(low..=high).contains(&info.rate) || !(1..=MAX_CHANNELS).contains(&info.channels) {
        return Err(format!(
            "{what} is at {} Hz with {} channels; the render reads {low} to {high} Hz and 1 to {MAX_CHANNELS} channels",
            info.rate, info.channels
        ));
    }
    let resampled = u128::from(info.frames()) * u128::from(RATE) / u128::from(info.rate);
    if info.samples > MAX_FRAMES * 2 || resampled > u128::from(MAX_FRAMES) {
        return Err(format!(
            "{what} lasts over {MAX_SECONDS} s of stereo; the render holds at most that"
        ));
    }
    Ok(info)
}

pub fn sample(name: &str, bytes: &[u8], held: &mut u64) -> Result<(), String> {
    let what = format!("sample `{name}`");
    let info = audio(
        &what,
        bytes.len(),
        MAX_SAMPLE_BYTES,
        pfx_sound::Clip::wav_info(bytes),
    )?;
    let resampled = u128::from(info.frames()) * u128::from(RATE) / u128::from(info.rate);
    let kept = u64::try_from(resampled)
        .unwrap_or(u64::MAX)
        .saturating_mul(STEREO);
    let decoded = info.samples.saturating_mul(4);
    within_budget(&what, held.saturating_add(kept).saturating_add(decoded))?;
    *held = held.saturating_add(kept);
    Ok(())
}

pub fn part(file: &str, bytes: &[u8], held: &mut u64) -> Result<(), String> {
    let what = format!("`{file}`");
    let info = audio(
        &what,
        bytes.len(),
        MAX_PART_BYTES,
        pfx_sound::Clip::wav_info(bytes),
    )?;
    let decoded = info.samples.saturating_mul(4);
    within_budget(&what, held.saturating_add(decoded))?;
    *held = held.saturating_add(decoded);
    Ok(())
}

fn within_budget(what: &str, bytes: u64) -> Result<(), String> {
    if bytes > MAX_BYTES {
        return Err(format!(
            "{what}: the render would hold {:.2} GiB at once; it holds at most {:.0} GiB",
            bytes as f64 / GIB,
            MAX_BYTES as f64 / GIB
        ));
    }
    Ok(())
}

fn curves(patch: &Patch) -> u64 {
    let envs: BTreeSet<[u32; 4]> = patch
        .osc
        .iter()
        .filter(|o| o.sample.is_none())
        .map(|o| o.env.unwrap_or(patch.env).map(f32::to_bits))
        .collect();
    envs.iter()
        .map(|e| {
            let [a, d, _, r] = e.map(f32::from_bits).map(f64::from);
            play::ms(a)
                .saturating_add(play::ms(d))
                .saturating_add(play::ms(r))
                .saturating_mul(4)
        })
        .fold(0u64, u64::saturating_add)
}

fn work(patch: &Patch, notes: &[Note], frame: &dyn Fn(f64) -> u64, most: u64) -> u64 {
    let tail = play::tail(patch);
    let oscs = patch.osc.len() as u64;
    notes
        .iter()
        .map(|n| {
            let at = frame(n.beat);
            let end = frame(n.beat + n.beats).max(at.saturating_add(1));
            (end - at)
                .saturating_add(tail)
                .min(most)
                .saturating_mul(oscs)
        })
        .fold(0u64, u64::saturating_add)
}

pub fn budget(style: &Style, score: &Score, frames: usize) -> Result<u64, String> {
    let most = frames as u64;
    let tempo = score.tempo();
    let frame = play::tempo_frames(&tempo);
    let buffers = style.roles.len() as u64 + WORKING;
    let mut bytes = most.saturating_mul(STEREO).saturating_mul(buffers);
    let mut done = 0u64;
    let cue_tempo = crate::Tempo::new(f64::from(score.bpm), score.beats_per_bar, &[]);
    let cue_frame = play::tempo_frames(&cue_tempo);
    for role in &style.roles {
        let Some(p) = style.patch.get(&role.patch) else {
            continue;
        };
        bytes = bytes.saturating_add(curves(p));
        let notes = score
            .track(&role.name)
            .map_or(&[][..], |t| t.notes.as_slice());
        done = done.saturating_add(work(p, notes, &frame, most));
    }
    for cue in &score.cues {
        let length = super::cue_frames(style, score, cue)?;
        bytes = bytes.saturating_add(length.saturating_mul(STEREO));
        for voice in &cue.voices {
            let Some(p) = style.patch.get(&voice.patch) else {
                continue;
            };
            bytes = bytes.saturating_add(curves(p));
            done = done.saturating_add(work(p, &voice.notes, &cue_frame, length));
        }
        let fires = score.fires.iter().filter(|f| f.cue == cue.name).count() as u64;
        done = done.saturating_add(fires.saturating_mul(length));
    }
    within_budget("the score", bytes)?;
    if done > MAX_WORK {
        return Err(format!(
            "the score would render {done} oscillator frames; the render takes at most {MAX_WORK}"
        ));
    }
    Ok(bytes)
}
