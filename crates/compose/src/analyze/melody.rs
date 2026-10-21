use std::collections::{BTreeMap, BTreeSet};

use crate::score::Note;
use crate::style::{MAX_BEATS, Melody, Plays};
use crate::theory::Key;

use super::harmony::{harmonic, source, timeline};
use super::key::pitch_class;
use super::{Part, Piece};

const GRID: f64 = 48.0;
const CHORD_TONES: f64 = 0.8;
const FULL: f64 = 0.8;
const MOTIFS: [u32; 4] = [1, 2, 4, 8];

pub(crate) fn sorted(notes: &[Note]) -> Vec<Note> {
    let mut out: Vec<Note> = notes
        .iter()
        .filter(|n| n.beat.is_finite() && n.midi.is_finite() && n.beats.is_finite())
        .copied()
        .collect();
    out.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.midi.total_cmp(&b.midi)));
    out
}

pub(crate) fn median(mut values: Vec<f64>) -> Option<f64> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[(values.len() - 1) / 2])
}

pub(crate) fn monophonic(part: &Part) -> bool {
    let notes = sorted(&part.notes);
    if notes.len() < 2 {
        return !notes.is_empty();
    }
    let overlaps = notes
        .windows(2)
        .filter(|w| w[1].beat < w[0].beat + w[0].beats - 1e-6 || w[1].beat - w[0].beat < 1e-6)
        .count();
    overlaps * 10 < notes.len()
}

pub fn lead(pieces: &[Piece]) -> Vec<&Part> {
    let mut out = Vec::new();
    for piece in pieces {
        let named: Vec<&Part> = piece
            .parts
            .iter()
            .filter(|p| p.plays == Some(Plays::Melody))
            .collect();
        if !named.is_empty() {
            out.extend(named);
            continue;
        }
        let mut top: Option<(&Part, f64)> = None;
        for part in &piece.parts {
            if part.plays.is_some() || !harmonic(part) || part.notes.len() < 2 || !monophonic(part)
            {
                continue;
            }
            let Some(pitch) = median(part.notes.iter().map(|n| n.midi).collect()) else {
                continue;
            };
            if top.is_none_or(|(_, p)| pitch > p) {
                top = Some((part, pitch));
            }
        }
        out.extend(top.map(|(p, _)| p));
    }
    out
}

pub(crate) fn scale_index(midi: i32, key: &Key) -> i32 {
    let n = key.degrees.len() as i32;
    let rel = midi - key.root;
    let octave = rel.div_euclid(12);
    let pc = rel.rem_euclid(12);
    let mut best = (0i32, i32::MAX);
    for d in 0..=n {
        let degree = if d == n { 12 } else { key.degrees[d as usize] };
        let distance = (degree - pc).abs();
        if distance < best.1 {
            best = (d, distance);
        }
    }
    octave * n + best.0
}

fn grid(beats: f64) -> u32 {
    let ticks = (beats * GRID).round();
    ticks.clamp(1.0, MAX_BEATS * GRID) as u32
}

fn slots(leads: &[Lead]) -> Vec<u32> {
    let mut filled: BTreeSet<u32> = BTreeSet::new();
    let mut every: BTreeSet<u32> = BTreeSet::new();
    for lead in leads {
        let bpb = f64::from(lead.piece.beats_per_bar.max(1));
        for pair in lead.notes.windows(2) {
            let gap = pair[1].beat - pair[0].beat;
            if gap <= 1e-6 || gap > bpb + 1e-9 {
                continue;
            }
            every.insert(grid(gap));
            if pair[0].beats >= FULL * gap {
                filled.insert(grid(gap));
            }
        }
    }
    if filled.is_empty() { every } else { filled }
        .into_iter()
        .collect()
}

fn snap_up(beats: f64, slots: &[u32]) -> Option<u32> {
    let want = grid(beats.max(1.0 / GRID));
    slots.iter().copied().find(|s| s + 1 >= want)
}

fn fill(mut ticks: u32, slots: &[u32]) -> Vec<u32> {
    let mut out = Vec::new();
    for &slot in slots.iter().rev() {
        while slot > 0 && ticks >= slot {
            ticks -= slot;
            out.push(slot);
        }
    }
    if ticks > 1 {
        out.push(ticks);
    }
    out
}

fn chord_tones_of(piece: &Piece, key: &Key, sevenths: bool) -> Option<Vec<(f64, Option<Vec<i32>>)>> {
    let (step, line) = timeline(piece, key, false);
    if line.iter().all(Option::is_none) {
        return None;
    }
    let mut chords: Vec<(f64, Option<String>)> = line
        .into_iter()
        .enumerate()
        .map(|(i, n)| (i as f64 * step, n))
        .collect();
    if let Some(sourced) = source(piece, key) {
        chords.extend(sourced.applied.iter().map(|(at, n)| (*at, Some(n.clone()))));
        chords.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    Some(
        chords
            .into_iter()
            .map(|(at, n)| (at, n.and_then(|n| key.chord(&n, sevenths).ok())))
            .collect(),
    )
}

fn chord_index(midi: i32, chord: &[i32]) -> Option<i32> {
    let mut root = chord[0];
    while root > 62 {
        root -= 12;
    }
    while root < 55 {
        root += 12;
    }
    let shift = root - chord[0];
    let voiced: Vec<i32> = chord.iter().map(|c| c + shift).collect();
    let n = voiced.len() as i32;
    let pc = midi.rem_euclid(12);
    let at = voiced.iter().position(|v| v.rem_euclid(12) == pc)? as i32;
    let octave = (midi - voiced[at as usize]).div_euclid(12);
    Some(octave * n + at)
}

struct Lead<'a> {
    piece: &'a Piece,
    notes: Vec<Note>,
}

fn motif_fit(leads: &[Lead], key: &Key) -> (u32, f64) {
    let mut best = (1u32, 0.0f64);
    for length in MOTIFS {
        let mut same = 0usize;
        let mut pairs = 0usize;
        for lead in leads {
            let bpb = f64::from(lead.piece.beats_per_bar.max(1));
            let span = bpb * f64::from(length);
            let mut chunks: BTreeMap<i64, Vec<(u32, i32)>> = BTreeMap::new();
            let mut first: BTreeMap<i64, i32> = BTreeMap::new();
            for note in &lead.notes {
                let chunk = (note.beat / span + 1e-9).floor() as i64;
                let index = scale_index(note.midi.round() as i32, key);
                let base = *first.entry(chunk).or_insert(index);
                let at = (note.beat - chunk as f64 * span).max(0.0);
                chunks
                    .entry(chunk)
                    .or_default()
                    .push(((at * GRID).round() as u32, index - base));
            }
            let mut last: Option<(i64, &Vec<(u32, i32)>)> = None;
            for (at, tokens) in &chunks {
                if let Some((before, previous)) = last
                    && before + 1 == *at
                {
                    pairs += 1;
                    if previous == tokens {
                        same += 1;
                    }
                }
                last = Some((*at, tokens));
            }
        }
        if pairs == 0 {
            continue;
        }
        let ratio = same as f64 / pairs as f64;
        if ratio > best.1 + 1e-9 {
            best = (length, ratio);
        }
    }
    best
}

pub fn derive(pieces: &[Piece], key: &Key) -> Melody {
    derive_for(pieces, key, super::harmony::derive(pieces, key).sevenths)
}

pub(crate) fn derive_for(pieces: &[Piece], key: &Key, sevenths: bool) -> Melody {
    let mut leads: Vec<Lead> = Vec::new();
    for piece in pieces {
        for part in lead(std::slice::from_ref(piece)) {
            let notes = sorted(&part.notes);
            if !notes.is_empty() {
                leads.push(Lead { piece, notes });
            }
        }
    }
    if leads.is_empty() {
        return Melody::default();
    }
    let mut lo = i32::MAX;
    let mut hi = i32::MIN;
    let mut in_chord = 0usize;
    let mut checked = 0usize;
    let mut chord_steps: BTreeMap<i32, u32> = BTreeMap::new();
    let mut scale_steps: BTreeMap<i32, u32> = BTreeMap::new();
    let mut rhythm: BTreeMap<u32, u32> = BTreeMap::new();
    let mut notes = 0usize;
    let slots = slots(&leads);
    let mut rests = 0usize;
    for lead in &leads {
        let bpb = f64::from(lead.piece.beats_per_bar.max(1));
        let chords = chord_tones_of(lead.piece, key, sevenths);
        let chord_at = |beat: f64| -> Option<&Vec<i32>> {
            let line = chords.as_ref()?;
            let i = line.partition_point(|(at, _)| *at <= beat + 1e-9);
            line.get(i.saturating_sub(1))?.1.as_ref()
        };
        let mut previous: Option<(i32, Option<i32>)> = None;
        for (i, note) in lead.notes.iter().enumerate() {
            let midi = note.midi.round() as i32;
            lo = lo.min(midi);
            hi = hi.max(midi);
            let chord = chord_at(note.beat);
            let index = chord.and_then(|c| chord_index(midi, c));
            if let Some(chord) = chord {
                checked += 1;
                if chord
                    .iter()
                    .any(|c| c.rem_euclid(12) as usize == pitch_class(note.midi).unwrap_or(12))
                {
                    in_chord += 1;
                }
            }
            let scale = scale_index(midi, key);
            if let Some((before, before_chord)) = previous {
                let step = scale - before;
                if step.abs() <= 24 {
                    *scale_steps.entry(step).or_default() += 1;
                }
                if let (Some(a), Some(b)) = (before_chord, index)
                    && (b - a).abs() <= 24
                {
                    *chord_steps.entry(b - a).or_default() += 1;
                }
            }
            previous = Some((scale, index));
            notes += 1;
            let next = lead.notes.get(i + 1).map(|n| n.beat - note.beat);
            match next {
                Some(gap) if gap <= 1e-6 => {}
                Some(gap) if gap <= bpb + 1e-9 && note.beats < FULL * gap => {
                    let whole = grid(gap);
                    let slot = snap_up(note.beats, &slots)
                        .unwrap_or_else(|| grid(note.beats.max(1.0 / GRID)))
                        .min(whole);
                    *rhythm.entry(slot).or_default() += 1;
                    for rest in fill(whole - slot, &slots) {
                        *rhythm.entry(rest).or_default() += 1;
                        rests += 1;
                    }
                }
                Some(gap) if gap <= bpb + 1e-9 => {
                    *rhythm.entry(grid(gap)).or_default() += 1;
                }
                _ => {
                    let length = note.beats.clamp(1.0 / GRID, bpb);
                    let slot = snap_up(length, &slots)
                        .or(slots.last().copied())
                        .unwrap_or_else(|| grid(length));
                    *rhythm.entry(slot).or_default() += 1;
                }
            }
        }
    }
    let chord_tones = checked > 0 && in_chord as f64 >= CHORD_TONES * checked as f64;
    let lo = lo.clamp(0, 126);
    let hi = hi.clamp(lo + 1, 127);
    let steps = if chord_tones {
        chord_steps
    } else {
        scale_steps
    };
    let fallback = Melody::default();
    let steps: Vec<(i32, u32)> = if steps.is_empty() {
        fallback.steps
    } else {
        steps.into_iter().collect()
    };
    let rhythm: Vec<(f64, u32)> = if rhythm.is_empty() {
        fallback.rhythm
    } else {
        rhythm
            .into_iter()
            .map(|(ticks, w)| (f64::from(ticks) / GRID, w))
            .collect()
    };
    let rest = if notes + rests == 0 {
        0.0
    } else {
        (rests as f64 / (notes + rests) as f64 * 100.0).round() / 100.0
    };
    let (motif_bars, repeat) = motif_fit(&leads, key);
    Melody {
        range: [lo, hi],
        steps,
        rhythm,
        rest: rest.clamp(0.0, 1.0),
        motif_bars,
        repeat: ((repeat * 100.0).round() / 100.0).clamp(0.0, 1.0),
        chord_tones,
        ..Melody::default()
    }
}
