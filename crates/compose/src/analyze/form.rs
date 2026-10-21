use std::collections::BTreeMap;

use crate::style::{MAX_BARS, MAX_BEATS, MAX_BPM, Master, Plays, Role, Section, TEMPO_RATIO};

use super::{Piece, Span};

const GRID: f64 = 48.0;
const MAIN: &str = "main";

pub fn bpm(pieces: &[Piece]) -> [u32; 2] {
    let mut lo = u32::MAX;
    let mut hi = 0;
    for piece in pieces {
        if !piece.bpm.is_finite() {
            continue;
        }
        let bpm = (piece.bpm.round().max(1.0) as u32).min(MAX_BPM);
        lo = lo.min(bpm);
        hi = hi.max(bpm);
    }
    if hi == 0 { [120, 120] } else { [lo, hi] }
}

fn bar_length(piece: &Piece) -> f64 {
    f64::from(piece.beats_per_bar.max(1))
}

fn length(piece: &Piece) -> f64 {
    let end = piece
        .parts
        .iter()
        .flat_map(|p| &p.notes)
        .map(|n| n.beat + n.beats.max(0.0))
        .filter(|e| e.is_finite())
        .fold(0.0f64, f64::max);
    let beats = if piece.beats.is_finite() {
        piece.beats
    } else {
        0.0
    };
    beats.max(end)
}

fn bar_count(piece: &Piece) -> usize {
    let bars = (length(piece) / bar_length(piece) - 1e-9).ceil().max(1.0);
    bars.min(f64::from(MAX_BARS)) as usize
}

fn round2(v: f64) -> f32 {
    ((v * 100.0).round() / 100.0) as f32
}

pub fn intensity(piece: &Piece) -> Vec<f32> {
    let bpb = bar_length(piece);
    let bars = bar_count(piece);
    let mut onsets = vec![0.0f64; bars];
    let mut loud = vec![0.0f64; bars];
    let mut active: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); bars];
    for (index, part) in piece.parts.iter().enumerate() {
        for note in &part.notes {
            if !(note.beat.is_finite() && note.vel.is_finite()) {
                continue;
            }
            let bar = (note.beat / bpb + 1e-9).floor().max(0.0) as usize;
            if bar < bars {
                onsets[bar] += 1.0;
                loud[bar] += note.vel.max(0.0);
                active[bar].insert(index);
            }
        }
    }
    let top = |v: &[f64]| v.iter().copied().fold(0.0f64, f64::max);
    let (max_onsets, max_loud) = (top(&onsets), top(&loud));
    let max_active = active.iter().map(|a| a.len()).max().unwrap_or(0) as f64;
    let share = |v: f64, max: f64| if max > 0.0 { v / max } else { 0.0 };
    let mut out: Vec<f32> = (0..bars)
        .map(|b| {
            let v = (share(onsets[b], max_onsets)
                + share(loud[b], max_loud)
                + share(active[b].len() as f64, max_active))
                / 3.0;
            round2(v.clamp(0.0, 1.0))
        })
        .collect();
    for span in &piece.spans {
        let Some([i0, i1]) = span.intensity else {
            continue;
        };
        let from = (span.from / bpb).round().max(0.0) as usize;
        let count = (span.beats / bpb).round().max(1.0) as usize;
        for k in 0..count {
            if let Some(v) = out.get_mut(from + k) {
                let t = k as f32 / count as f32;
                *v = (i0 + (i1 - i0) * t).clamp(0.0, 1.0);
            }
        }
    }
    out
}

fn spans(piece: &Piece) -> Vec<Span> {
    if !piece.spans.is_empty() {
        return piece.spans.clone();
    }
    vec![Span {
        name: MAIN.to_string(),
        from: 0.0,
        beats: bar_count(piece) as f64 * bar_length(piece),
        tempo: 1.0,
        intensity: None,
        gain_db: 0.0,
        hit: None,
    }]
}

fn beats_grid(beats: f64) -> f64 {
    ((beats * GRID).round() / GRID).clamp(0.0, MAX_BEATS)
}

struct Gathered {
    bars: [u32; 2],
    roles: Vec<String>,
    intensity: [f64; 2],
    seen: f64,
    round_bars: u32,
    start_roles: usize,
    riser: f64,
    roll: f64,
    impact: bool,
    hit: Option<usize>,
    tempo: f64,
    gain_db: f32,
}

pub fn derive(pieces: &[Piece], roles: &[Role]) -> Vec<Section> {
    let plays: BTreeMap<&str, Plays> = roles.iter().map(|r| (r.name.as_str(), r.plays)).collect();
    let mut order: Vec<String> = Vec::new();
    let mut gathered: BTreeMap<String, Gathered> = BTreeMap::new();
    for piece in pieces {
        let bpb = bar_length(piece);
        let curve = intensity(piece);
        for span in spans(piece) {
            let bars = if span.beats <= 0.0 {
                0
            } else {
                ((span.beats / bpb).round().max(1.0) as u32).min(MAX_BARS)
            };
            let end = span.from + span.beats;
            let inside = |beat: f64| beat >= span.from - 1e-6 && beat < end - 1e-6;
            let mut entries: Vec<(String, u32)> = Vec::new();
            let mut riser: f64 = 0.0;
            let mut roll: f64 = 0.0;
            let mut impact = false;
            for part in &piece.parts {
                let kind = plays.get(part.name.as_str()).copied();
                let first = part
                    .notes
                    .iter()
                    .map(|n| n.beat)
                    .filter(|b| b.is_finite() && inside(*b))
                    .fold(f64::INFINITY, f64::min);
                if !first.is_finite() {
                    continue;
                }
                match kind {
                    Some(Plays::Riser) | Some(Plays::Reverse) => {
                        riser = riser.max(beats_grid(end - first));
                    }
                    Some(Plays::Roll) => roll = roll.max(beats_grid(end - first)),
                    Some(Plays::Impact) => impact |= first < span.from + 0.5,
                    Some(_) => {
                        let entry = ((first - span.from) / bpb + 1e-9).floor().max(0.0) as u32;
                        entries.push((part.name.clone(), entry));
                    }
                    None => {}
                }
            }
            entries.sort_by_key(|(_, e)| *e);
            let start_roles = entries.iter().filter(|(_, e)| *e == 0).count();
            let round_bars = match entries.iter().find(|(_, e)| *e > 0) {
                Some((_, e)) if start_roles > 0 => *e,
                _ => 0,
            };
            let from_bar = (span.from / bpb).round().max(0.0) as usize;
            let last_bar = from_bar + bars.max(1) as usize - 1;
            let at = |bar: usize| f64::from(curve.get(bar).copied().unwrap_or(0.0));
            let level = match span.intensity {
                Some([a, b]) => [f64::from(a), f64::from(b)],
                None => [at(from_bar), at(last_bar)],
            };
            let tempo = if span.tempo.is_finite() {
                span.tempo.clamp(TEMPO_RATIO[0], TEMPO_RATIO[1])
            } else {
                1.0
            };
            let gain_db = if span.gain_db.is_finite() {
                span.gain_db
            } else {
                0.0
            };
            match gathered.get_mut(&span.name) {
                Some(g) => {
                    g.bars = [g.bars[0].min(bars), g.bars[1].max(bars)];
                    for (name, _) in &entries {
                        if !g.roles.contains(name) {
                            g.roles.push(name.clone());
                        }
                    }
                    g.intensity[0] += level[0];
                    g.intensity[1] += level[1];
                    g.seen += 1.0;
                    g.riser = g.riser.max(riser);
                    g.roll = g.roll.max(roll);
                    g.impact |= impact;
                    g.hit = g.hit.or(span.hit);
                }
                None => {
                    order.push(span.name.clone());
                    gathered.insert(
                        span.name.clone(),
                        Gathered {
                            bars: [bars, bars],
                            roles: entries.iter().map(|(n, _)| n.clone()).collect(),
                            intensity: level,
                            seen: 1.0,
                            round_bars,
                            start_roles: if round_bars > 0 { start_roles } else { 0 },
                            riser,
                            roll,
                            impact,
                            hit: span.hit,
                            tempo,
                            gain_db,
                        },
                    );
                }
            }
        }
    }
    let mut sections: Vec<Section> = order
        .iter()
        .filter_map(|name| {
            let g = gathered.remove(name)?;
            let level = g.intensity.map(|v| round2(v / g.seen).clamp(0.0, 1.0));
            Some(Section {
                name: name.clone(),
                bars: [g.bars[0], g.bars[1].max(1)],
                roles: g.roles,
                intensity: level,
                round_bars: g.round_bars,
                start_roles: g.start_roles,
                riser: g.riser,
                roll: g.roll,
                impact: g.impact,
                hit: g.hit,
                tail: 0.0,
                tempo: g.tempo,
                pitch: 1.0,
                stutter: 0,
                lift: 0,
                cue: None,
                gain_db: g.gain_db,
                stop: 0.0,
                through: Vec::new(),
                secondary: None,
                triads: false,
                plans: Vec::new(),
                level: None,
            })
        })
        .collect();
    let mut last = None;
    for (index, section) in sections.iter_mut().enumerate() {
        let keep = section
            .hit
            .filter(|h| index > 0 && last.is_none_or(|l| *h > l));
        section.hit = keep;
        if keep.is_some() {
            last = keep;
        }
    }
    let fixed = sections.iter().all(|s| s.bars[0] == s.bars[1]);
    if fixed {
        let hits: Vec<usize> = sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.hit.is_some())
            .map(|(i, _)| i)
            .collect();
        for index in hits {
            let before = &mut sections[index - 1];
            let [lo, hi] = before.bars;
            before.bars = [lo.saturating_sub(1).max(1), (hi + 1).min(MAX_BARS)];
        }
    }
    sections
}

pub fn master(pieces: &[Piece], roles: &[Role]) -> Master {
    let found = pieces.iter().find_map(|p| p.master.clone());
    let Some(mut master) = found else {
        return Master {
            lufs: -16.0,
            true_peak: -1.0,
            room: [0.0; 3],
            sidechain: None,
            fade: 0.0,
        };
    };
    if let Some(sidechain) = &mut master.sidechain {
        let known = |name: &String| roles.iter().any(|r| r.name == *name);
        sidechain.keys.retain(known);
        sidechain.beds.retain(known);
        if sidechain.keys.is_empty() || sidechain.beds.is_empty() {
            master.sidechain = None;
        }
    }
    master
}
