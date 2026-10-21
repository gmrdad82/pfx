use std::collections::BTreeMap;

use crate::style::{Harmony, MAX_BEATS, Plays};
use crate::theory::Key;
use crate::theory::numeral;

use super::key::pitch_class;
use super::{Chords, Origin, Part, Piece};

const QUANTUM: f64 = 0.25;
const SEVENTH_FLOOR: f64 = 0.08;
const ABSENT: f64 = 0.03;
const SEVENTH_SHARE: f64 = 0.25;

pub(crate) struct Candidate {
    pub text: String,
    pub tones: [bool; 12],
    root: usize,
    seventh: Option<usize>,
    altered: bool,
}

pub(crate) fn harmonic(part: &Part) -> bool {
    part.pitched && !part.plays.is_some_and(|p| p == Plays::Hit || p.event())
}

pub(crate) fn same_key(text: &str, key: &Key) -> bool {
    let Ok(other) = Key::parse(text, 3) else {
        return false;
    };
    other.mode == key.mode && (other.root - key.root).rem_euclid(12) == 0
}

pub(crate) fn source<'a>(piece: &'a Piece, key: &Key) -> Option<&'a Chords> {
    let chords = piece.chords.as_ref()?;
    let ok = !chords.numerals.is_empty()
        && chords.beats > 0.0
        && chords.beats.is_finite()
        && piece.key.as_deref().is_some_and(|k| same_key(k, key))
        && chords
            .numerals
            .iter()
            .all(|n| key.chord(n, chords.sevenths).is_ok());
    ok.then_some(chords)
}

fn steps_of(beats: f64, step: f64) -> usize {
    if !(beats > 0.0 && step > 0.0 && beats.is_finite()) {
        return 0;
    }
    ((beats / step) - 1e-9).ceil().max(1.0) as usize
}

pub(crate) fn heard(piece: &Piece) -> bool {
    piece.origin == Origin::Audio
        && piece
            .chroma
            .as_ref()
            .is_some_and(|c| c.beats > 0.0 && c.beats.is_finite() && !c.frames.is_empty())
}

pub fn chroma(piece: &Piece, step: f64) -> Vec<[f64; 12]> {
    let count = steps_of(piece.beats, step);
    let mut out = vec![[0.0; 12]; count];
    if count == 0 {
        return out;
    }
    let mut lowest: Vec<Option<(f64, usize, f64)>> = vec![None; count];
    let mut any = false;
    let notes = if heard(piece) {
        &[][..]
    } else {
        &piece.parts[..]
    };
    for part in notes.iter().filter(|p| harmonic(p)) {
        for note in &part.notes {
            let Some(pc) = pitch_class(note.midi) else {
                continue;
            };
            if !(note.beat.is_finite() && note.beats.is_finite() && note.beats > 0.0) {
                continue;
            }
            let start = note.beat.max(0.0);
            let end = note.beat + note.beats.min(MAX_BEATS);
            let first = (start / step).floor() as usize;
            let mut i = first;
            while i < count {
                let lo = i as f64 * step;
                let hi = lo + step;
                if lo >= end {
                    break;
                }
                let overlap = end.min(hi) - start.max(lo);
                if overlap > 0.0 {
                    out[i][pc] += overlap;
                    any = true;
                    let low = lowest[i];
                    if low.is_none_or(|(m, _, _)| note.midi < m) {
                        lowest[i] = Some((note.midi, pc, overlap));
                    }
                }
                i += 1;
            }
        }
    }
    if any {
        for (frame, low) in out.iter_mut().zip(&lowest) {
            if let Some((_, pc, weight)) = low {
                frame[*pc] += weight;
            }
        }
        return out;
    }
    let Some(source) = piece.chroma.as_ref().filter(|c| c.beats > 0.0) else {
        return out;
    };
    for (i, frame) in out.iter_mut().enumerate() {
        let lo = i as f64 * step;
        let hi = lo + step;
        let first = (lo / source.beats).floor() as usize;
        let mut j = first;
        while j < source.frames.len() {
            let a = j as f64 * source.beats;
            let b = a + source.beats;
            if a >= hi {
                break;
            }
            let overlap = hi.min(b) - lo.max(a);
            if overlap > 0.0 {
                for (o, v) in frame.iter_mut().zip(&source.frames[j]) {
                    if v.is_finite() && *v > 0.0 {
                        *o += v * overlap;
                    }
                }
            }
            j += 1;
        }
    }
    out
}

pub(crate) fn candidates(key: &Key, sevenths: bool) -> Vec<Candidate> {
    let n = key.degrees.len();
    let mut out = Vec::new();
    let suffixes: &[&str] = if sevenths { &["", "7"] } else { &[""] };
    for suffix in suffixes {
        for alt in ["", "b", "#"] {
            for degree in 0..n.min(7) {
                let third = (key.degrees[(degree + 2) % n] - key.degrees[degree]).rem_euclid(12);
                let upper = ["I", "II", "III", "IV", "V", "VI", "VII"][degree];
                let lower = upper.to_lowercase();
                let cases = if third == 4 {
                    [upper.to_string(), lower]
                } else {
                    [lower, upper.to_string()]
                };
                for case in cases {
                    let text = format!("{alt}{case}{suffix}");
                    let Ok(chord) = key.chord(&text, false) else {
                        continue;
                    };
                    let mut tones = [false; 12];
                    for m in &chord {
                        tones[m.rem_euclid(12) as usize] = true;
                    }
                    out.push(Candidate {
                        text,
                        tones,
                        root: chord[0].rem_euclid(12) as usize,
                        seventh: chord.get(3).map(|m| m.rem_euclid(12) as usize),
                        altered: !alt.is_empty(),
                    });
                }
            }
        }
    }
    out
}

pub(crate) fn best<'a>(chroma: &[f64; 12], candidates: &'a [Candidate]) -> Option<&'a Candidate> {
    let total: f64 = chroma.iter().filter(|v| v.is_finite() && **v > 0.0).sum();
    if total <= 1e-12 || total.is_nan() {
        return None;
    }
    let c: Vec<f64> = chroma
        .iter()
        .map(|v| {
            if v.is_finite() && *v > 0.0 {
                v / total
            } else {
                0.0
            }
        })
        .collect();
    let mut found: Option<(&Candidate, f64)> = None;
    for candidate in candidates {
        if candidate.seventh.is_some_and(|s| c[s] < SEVENTH_FLOOR) {
            continue;
        }
        let mut inside = 0.0;
        let mut missing = 0.0;
        for (pc, on) in candidate.tones.iter().enumerate() {
            if *on {
                inside += c[pc];
                if c[pc] < ABSENT {
                    missing += 1.0;
                }
            }
        }
        let mut score = inside - (1.0 - inside) + 0.5 * c[candidate.root] - 0.15 * missing;
        if candidate.altered {
            score -= 0.1;
        }
        if candidate.seventh.is_some() {
            score -= 0.1;
        }
        if found.is_none_or(|(_, s)| score > s) {
            found = Some((candidate, score));
        }
    }
    found.map(|(c, _)| c)
}

pub fn numeral(chroma: &[f64; 12], key: &Key, sevenths: bool) -> Option<String> {
    best(chroma, &candidates(key, sevenths)).map(|c| c.text.clone())
}

fn detect_step(piece: &Piece) -> f64 {
    let pitched = !heard(piece)
        && piece
            .parts
            .iter()
            .any(|p| harmonic(p) && !p.notes.is_empty());
    match &piece.chroma {
        Some(c) if !pitched && c.beats > 0.0 && c.beats.is_finite() => c.beats,
        _ => 1.0,
    }
}

pub(crate) fn timeline(piece: &Piece, key: &Key, sevenths: bool) -> (f64, Vec<Option<String>>) {
    if let Some(chords) = source(piece, key) {
        return (
            chords.beats,
            chords.numerals.iter().cloned().map(Some).collect(),
        );
    }
    let step = detect_step(piece);
    let candidates = candidates(key, sevenths);
    let found = chroma(piece, step)
        .iter()
        .map(|frame| best(frame, &candidates).map(|c| c.text.clone()))
        .collect();
    (step, found)
}

fn runs(piece: &Piece, key: &Key) -> Vec<(String, f64)> {
    let (step, line) = timeline(piece, key, true);
    let mut out: Vec<(String, f64)> = Vec::new();
    for numeral in line {
        match (numeral, out.last_mut()) {
            (Some(n), Some((last, len))) if *last == n => *len += step,
            (Some(n), _) => out.push((n, step)),
            (None, Some((_, len))) => *len += step,
            (None, None) => {}
        }
    }
    out
}

fn plain(text: &str) -> String {
    match numeral::parse(text) {
        Some(n) if n.chord_type == "7" => text[..text.len() - 1].to_string(),
        _ => text.to_string(),
    }
}

fn quantize(beats: f64) -> f64 {
    ((beats / QUANTUM).round() * QUANTUM).clamp(QUANTUM, MAX_BEATS)
}

struct Walked {
    beats: f64,
    sevenths: bool,
    walks: Vec<Vec<String>>,
}

fn common_beats(detected: &[Option<Vec<(String, f64)>>], pieces: &[Piece]) -> f64 {
    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    for (_, len) in detected.iter().flatten().flatten() {
        *counts.entry((quantize(*len) / QUANTUM) as u64).or_default() += 1;
    }
    let mut top: Option<(u64, usize)> = None;
    for (len, count) in counts {
        if top.is_none_or(|(_, c)| count >= c) {
            top = Some((len, count));
        }
    }
    match top {
        Some((len, _)) => len as f64 * QUANTUM,
        None => f64::from(pieces.first().map_or(4, |p| p.beats_per_bar)),
    }
}

fn walked(pieces: &[Piece], key: &Key) -> Walked {
    let sources: Vec<Option<&Chords>> = pieces.iter().map(|p| source(p, key)).collect();
    let detected: Vec<Option<Vec<(String, f64)>>> = pieces
        .iter()
        .zip(&sources)
        .map(|(piece, sourced)| match sourced {
            Some(_) => None,
            None => Some(runs(piece, key)),
        })
        .collect();
    let mut sevenths_long = 0.0;
    let mut total_long = 0.0;
    for (n, len) in detected.iter().flatten().flatten() {
        total_long += len;
        if numeral::parse(n).is_some_and(|p| p.chord_type == "7") {
            sevenths_long += len;
        }
    }
    let seventh_found = total_long > 0.0 && sevenths_long >= SEVENTH_SHARE * total_long;
    let sevenths = sources.iter().flatten().any(|c| c.sevenths) || seventh_found;
    let common = sources
        .iter()
        .flatten()
        .map(|c| ((c.beats / QUANTUM).round() as u64).max(1))
        .reduce(gcd)
        .map(|q| q as f64 * QUANTUM);
    let beats = common
        .unwrap_or_else(|| common_beats(&detected, pieces))
        .clamp(QUANTUM, MAX_BEATS);
    let walks = sources
        .iter()
        .zip(detected)
        .map(|(sourced, runs)| match (sourced, runs) {
            (Some(chords), _) => {
                let copies = ((chords.beats / beats).round() as usize).max(1);
                let mut walk = Vec::new();
                for n in &chords.numerals {
                    walk.extend(std::iter::repeat_n(n.clone(), copies));
                }
                walk
            }
            (None, Some(runs)) => {
                let mut walk = Vec::new();
                for (n, len) in runs {
                    let copies = ((len / beats).round() as usize).max(1);
                    walk.extend(std::iter::repeat_n(plain(&n), copies));
                }
                walk
            }
            (None, None) => Vec::new(),
        })
        .collect();
    Walked {
        beats,
        sevenths,
        walks,
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub fn walk(pieces: &[Piece], key: &Key) -> Vec<Vec<String>> {
    walked(pieces, key).walks
}

pub(crate) fn tonic_numeral(key: &Key) -> String {
    if key.mode == "major" { "I" } else { "i" }.to_string()
}

pub fn derive(pieces: &[Piece], key: &Key) -> Harmony {
    derive_walked(pieces, key).0
}

pub(crate) fn derive_walked(pieces: &[Piece], key: &Key) -> (Harmony, Vec<Vec<String>>) {
    let Walked {
        beats,
        sevenths,
        walks,
    } = walked(pieces, key);
    let mut start: Vec<String> = Vec::new();
    for walk in &walks {
        if let Some(first) = walk.first()
            && !start.contains(first)
        {
            start.push(first.clone());
        }
    }
    let mut counts: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for walk in &walks {
        for pair in walk.windows(2) {
            let next = counts.entry(pair[0].clone()).or_default();
            *next.entry(pair[1].clone()).or_default() += 1;
        }
        for n in walk {
            if !order.contains(n) {
                order.push(n.clone());
            }
        }
    }
    if start.is_empty() {
        let tonic = tonic_numeral(key);
        start.push(tonic.clone());
        order.push(tonic);
    }
    let mut markov: BTreeMap<String, Vec<(String, u32)>> = counts
        .into_iter()
        .map(|(from, next)| (from, next.into_iter().collect()))
        .collect();
    for n in &order {
        markov
            .entry(n.clone())
            .or_insert_with(|| vec![(n.clone(), 1)]);
    }
    let tails: Vec<&[String]> = walks
        .iter()
        .filter(|w| w.len() >= 2)
        .map(|w| &w[w.len() - 2..])
        .collect();
    let cadence = match tails.first() {
        Some(first) if tails.iter().all(|t| t == first) => first.to_vec(),
        _ => Vec::new(),
    };
    let harmony = Harmony {
        beats,
        start,
        cadence,
        sevenths,
        markov,
        secondary: 0.0,
        phrase: 0,
        turn: Vec::new(),
        turns: Vec::new(),
        land: false,
    };
    (harmony, walks)
}
