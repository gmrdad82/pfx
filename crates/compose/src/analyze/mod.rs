pub mod audio;
pub mod cli;
pub mod form;
pub mod harmony;
pub mod key;
pub mod melody;
pub mod roles;
pub mod scores;
pub mod source;
pub mod store;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::Serialize;

use crate::score::Note;
use crate::style::{
    Cue, MAX_BARS, MAX_BEATS, MAX_BEATS_PER_BAR, MAX_BPM, MIN_BEATS, Master, Patch, Plays, Role,
    Style, TEMPO_RATIO,
};
use crate::theory::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Score,
    Film,
    Audio,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub origin: Origin,
    pub name: String,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub beats: f64,
    pub key: Option<String>,
    pub chords: Option<Chords>,
    pub chroma: Option<Chroma>,
    pub parts: Vec<Part>,
    pub spans: Vec<Span>,
    pub hits: Vec<f64>,
    pub patches: BTreeMap<String, Patch>,
    pub cues: Vec<Cue>,
    pub master: Option<Master>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chords {
    pub beats: f64,
    pub numerals: Vec<String>,
    pub sevenths: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chroma {
    pub beats: f64,
    pub frames: Vec<[f64; 12]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub patch: Option<String>,
    pub plays: Option<Plays>,
    pub pitched: bool,
    pub gain: f32,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub name: String,
    pub from: f64,
    pub beats: f64,
    pub tempo: f64,
    pub intensity: Option<[f32; 2]>,
    pub gain_db: f32,
    pub hit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Ranked {
    pub key: String,
    pub correlation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub origins: Vec<Origin>,
    pub pieces: usize,
    pub bpm: [u32; 2],
    pub beats_per_bar: u32,
    pub key: String,
    pub keys: Vec<Ranked>,
    pub chords: usize,
    pub numerals: Vec<String>,
    pub roles: Vec<String>,
    pub notes: usize,
    pub sections: Vec<String>,
    pub coarse: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub style: Style,
    pub report: Report,
}

const KEY_OCTAVE: i32 = 3;
const KEYS_SHOWN: usize = 5;

fn tonal(piece: &Piece) -> [f64; 12] {
    let mut sum = [0.0; 12];
    let mut any = false;
    let parts = if harmony::heard(piece) {
        &[][..]
    } else {
        &piece.parts[..]
    };
    for part in parts.iter().filter(|p| harmony::harmonic(p)) {
        for (s, v) in sum.iter_mut().zip(key::histogram(&part.notes)) {
            *s += v;
            any |= v > 0.0;
        }
    }
    if !any && let Some(chroma) = &piece.chroma {
        for frame in &chroma.frames {
            for (s, v) in sum.iter_mut().zip(frame) {
                if v.is_finite() && *v > 0.0 {
                    *s += v * chroma.beats;
                }
            }
        }
    }
    sum
}

fn declared(piece: &Piece) -> Option<Key> {
    let key = Key::parse(piece.key.as_deref()?, KEY_OCTAVE).ok()?;
    key::MODES.contains(&key.mode.as_str()).then_some(key)
}

fn same(a: &Key, b: &Key) -> bool {
    a.mode == b.mode && (a.root - b.root).rem_euclid(12) == 0
}

fn keep_cues(pieces: &[Piece], roles: &[Role], patch: &mut BTreeMap<String, Patch>) -> Vec<Cue> {
    let mut cues: Vec<Cue> = Vec::new();
    for piece in pieces {
        for cue in &piece.cues {
            if cues.iter().any(|c| c.name == cue.name) || roles.iter().any(|r| r.name == cue.name) {
                continue;
            }
            let mut needed: Vec<(&String, &Patch)> = Vec::new();
            let mut whole = true;
            for voice in &cue.voices {
                match (patch.get(&voice.patch), piece.patches.get(&voice.patch)) {
                    (Some(have), Some(want)) if have != want => whole = false,
                    (Some(_), _) => {}
                    (None, Some(want)) => needed.push((&voice.patch, want)),
                    (None, None) => whole = false,
                }
            }
            if !whole {
                continue;
            }
            for (name, want) in needed {
                patch.insert(name.clone(), want.clone());
            }
            cues.push(cue.clone());
        }
    }
    cues
}

fn within(piece: &str, what: &str, value: f64, lo: f64, hi: f64) -> Result<(), String> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "`{piece}` has {what} {value}; analyze reads {lo} to {hi}"
        ))
    }
}

fn finite(piece: &str, what: &str, value: f32) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!(
            "`{piece}` has a {what} of {value}; analyze reads finite values"
        ))
    }
}

fn bounded(piece: &Piece) -> Result<Piece, String> {
    let name = piece.name.as_str();
    if !(1..=MAX_BEATS_PER_BAR).contains(&piece.beats_per_bar) {
        return Err(format!(
            "`{name}` has {} beats per bar; a style wants 1 to {MAX_BEATS_PER_BAR}",
            piece.beats_per_bar
        ));
    }
    let bpb = f64::from(piece.beats_per_bar.max(1));
    let most = f64::from(MAX_BARS) * bpb;
    within(name, "bpm", piece.bpm, 1.0, f64::from(MAX_BPM))?;
    within(name, "a length in beats of", piece.beats, MIN_BEATS, most)?;
    let [t0, t1] = TEMPO_RATIO;
    if piece.spans.len() > MAX_BARS as usize {
        return Err(format!(
            "`{name}` has {} sections; analyze reads at most {MAX_BARS}",
            piece.spans.len()
        ));
    }
    for span in &piece.spans {
        let at = format!("{name}` section `{}", span.name);
        within(&at, "a start beat of", span.from, 0.0, piece.beats)?;
        within(&at, "a length in beats of", span.beats, 0.0, piece.beats)?;
        within(&at, "a tempo ratio of", span.tempo, t0, t1)?;
        finite(&at, "gain_db", span.gain_db)?;
        if let Some([a, b]) = span.intensity {
            finite(&at, "intensity", a)?;
            finite(&at, "intensity", b)?;
        }
    }
    if let Some(chords) = &piece.chords {
        within(
            name,
            "a chord length in beats of",
            chords.beats,
            MIN_BEATS,
            MAX_BEATS,
        )?;
    }
    if let Some(chroma) = &piece.chroma {
        within(
            name,
            "a chroma step in beats of",
            chroma.beats,
            MIN_BEATS,
            most,
        )?;
    }
    let mut parts = Vec::with_capacity(piece.parts.len());
    for part in &piece.parts {
        finite(&format!("{name}` part `{}", part.name), "gain", part.gain)?;
        let mut notes = Vec::with_capacity(part.notes.len());
        for note in &part.notes {
            let row = [note.beat, note.midi, note.beats, note.vel];
            if row.iter().any(|v| !v.is_finite()) || note.beat < 0.0 || note.beat > piece.beats {
                continue;
            }
            if !(0.0..=127.0).contains(&note.midi) {
                return Err(format!(
                    "`{name}` part `{}` has a note at MIDI {}; analyze reads 0 to 127",
                    part.name, note.midi
                ));
            }
            let mut note = *note;
            note.beats = note.beats.clamp(0.0, MAX_BEATS);
            notes.push(note);
        }
        parts.push(Part {
            notes,
            ..part.clone()
        });
    }
    Ok(Piece {
        parts,
        hits: piece
            .hits
            .iter()
            .copied()
            .filter(|h| h.is_finite() && (0.0..=piece.beats).contains(h))
            .collect(),
        ..piece.clone()
    })
}

pub fn style(pieces: &[Piece]) -> Result<Analysis, String> {
    let pieces = pieces
        .iter()
        .map(bounded)
        .collect::<Result<Vec<Piece>, String>>()?;
    let pieces = pieces.as_slice();
    let first = pieces.first().ok_or("analyze needs at least one piece")?;
    if let Some(other) = pieces
        .iter()
        .find(|p| p.beats_per_bar != first.beats_per_bar)
    {
        return Err(format!(
            "`{}` has {} beats per bar but `{}` has {}; one style holds one meter",
            first.name, first.beats_per_bar, other.name, other.beats_per_bar
        ));
    }
    let histograms: Vec<[f64; 12]> = pieces.iter().map(tonal).collect();
    let mut total = [0.0; 12];
    for h in &histograms {
        for (t, v) in total.iter_mut().zip(h) {
            *t += v;
        }
    }
    let ranked = key::best(&total);
    let declared_keys: Vec<Option<Key>> = pieces.iter().map(declared).collect();
    let agreed = match declared_keys.first() {
        Some(Some(k))
            if declared_keys
                .iter()
                .all(|d| d.as_ref().is_some_and(|d| same(d, k))) =>
        {
            Some(k.clone())
        }
        _ => None,
    };
    let found = match agreed {
        Some(k) => k,
        None => {
            let top = &ranked[0];
            Key::new(&top.tonic, &top.mode, KEY_OCTAVE)?
        }
    };
    let is_found =
        |r: &key::Found| Key::new(&r.tonic, &r.mode, KEY_OCTAVE).is_ok_and(|k| same(&k, &found));
    let mut shown: Vec<&key::Found> = ranked.iter().filter(|r| is_found(r)).take(1).collect();
    shown.extend(ranked.iter().filter(|r| !is_found(r)).take(KEYS_SHOWN - 1));
    let mut tonics = vec![(found.tonic.clone(), 1)];
    for (histogram, declared) in histograms.iter().zip(&declared_keys) {
        let best = match declared {
            Some(k) => Some(k.clone()),
            None if histogram.iter().any(|v| *v > 0.0) => {
                let top = &key::best(histogram)[0];
                Key::new(&top.tonic, &top.mode, KEY_OCTAVE).ok()
            }
            None => None,
        };
        if let Some(best) = best
            && (best.root - found.root).rem_euclid(12) != 0
            && tonics.len() < 2
        {
            tonics.push((best.tonic.clone(), 1));
        }
    }
    let (harmony, walks) = harmony::derive_walked(pieces, &found);
    let melody = melody::derive_for(pieces, &found, harmony.sevenths);
    let (roles, mut patch) = roles::derive(pieces, &found, &harmony);
    let sections = form::derive(pieces, &roles);
    let master = form::master(pieces, &roles);
    let cues = keep_cues(pieces, &roles, &mut patch);
    let style = Style {
        bpm: form::bpm(pieces),
        beats_per_bar: first.beats_per_bar,
        swing: 0.0,
        humanize_ms: 0.0,
        tonics,
        modes: vec![(found.mode.clone(), 1)],
        harmony,
        melody,
        roles,
        patch,
        sections,
        cues,
        master,
        levels: None,
    };
    style
        .check()
        .map_err(|e| format!("the derived style fails its own check, a bug in analyze: {e}"))?;
    let mut numerals: Vec<String> = Vec::new();
    for n in walks.iter().flatten() {
        if !numerals.contains(n) {
            numerals.push(n.clone());
        }
    }
    let mut origins: Vec<Origin> = pieces.iter().map(|p| p.origin).collect();
    origins.sort();
    origins.dedup();
    let report = Report {
        coarse: origins.contains(&Origin::Audio),
        origins,
        pieces: pieces.len(),
        bpm: style.bpm,
        beats_per_bar: style.beats_per_bar,
        key: format!("{} {}", found.tonic, found.mode),
        keys: shown
            .into_iter()
            .map(|r| Ranked {
                key: r.name(),
                correlation: (r.correlation * 1e4).round() / 1e4,
            })
            .collect(),
        chords: walks.iter().map(Vec::len).sum(),
        numerals,
        roles: style.roles.iter().map(|r| r.name.clone()).collect(),
        notes: pieces
            .iter()
            .flat_map(|p| &p.parts)
            .map(|p| p.notes.len())
            .sum(),
        sections: style.sections.iter().map(|s| s.name.clone()).collect(),
    };
    Ok(Analysis { style, report })
}
