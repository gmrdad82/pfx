use std::collections::BTreeMap;

use crate::score::Note;
use crate::style::{
    DriveCurve, Harmony, MAX_BEATS, MAX_STEPS, Osc, Patch, Plays, Role, Voicing, Wave,
};
use crate::theory::Key;

use super::harmony::{best, candidates};
use super::melody::{lead, median, monophonic, sorted};
use super::{Part, Piece};

const LOUD: f64 = 0.8;
const BASS_BELOW: f64 = 52.0;
const CYCLE_MATCH: f64 = 0.8;
const MAX_CYCLE: usize = 8;
const MAX_PATTERNS: usize = 4;
const HOLD_GRID: f64 = 48.0;

fn onsets(notes: &[Note]) -> Vec<Vec<Note>> {
    let mut groups: Vec<Vec<Note>> = Vec::new();
    for note in sorted(notes) {
        match groups.last_mut() {
            Some(group) if (note.beat - group[0].beat).abs() < 1e-6 => group.push(note),
            _ => groups.push(vec![note]),
        }
    }
    groups
}

fn bars_spanned(notes: &[Note], bpb: f64) -> f64 {
    let end = notes
        .iter()
        .map(|n| n.beat + n.beats.max(0.0))
        .fold(0.0f64, f64::max);
    (end / bpb).ceil().max(1.0)
}

pub fn plays(part: &Part, beats_per_bar: u32) -> Plays {
    let notes = sorted(&part.notes);
    if notes.is_empty() {
        return Plays::Impact;
    }
    let bpb = f64::from(beats_per_bar.max(1));
    let bars = bars_spanned(&notes, bpb);
    let count = notes.len() as f64;
    let sparse = count <= (bars / 4.0).max(1.0);
    let loud = notes.iter().map(|n| n.vel).sum::<f64>() / count >= LOUD;
    let lengths = median(notes.iter().map(|n| n.beats).collect()).unwrap_or(0.0);
    let rising = notes.iter().filter(|n| n.bend > 0.0).count() as f64 >= count / 2.0;
    if rising && lengths >= 2.0 {
        return Plays::Riser;
    }
    let groups = onsets(&notes);
    if part.pitched {
        let chords = groups.iter().filter(|g| g.len() >= 3).count();
        if chords * 2 >= groups.len() {
            return Plays::Chord;
        }
    }
    if sparse && loud {
        return Plays::Impact;
    }
    if sparse && lengths >= bpb {
        return Plays::Riser;
    }
    if !part.pitched {
        let mut bursts: Vec<usize> = vec![1];
        for pair in groups.windows(2) {
            if pair[1][0].beat - pair[0][0].beat > bpb {
                bursts.push(1);
            } else if let Some(last) = bursts.last_mut() {
                *last += 1;
            }
        }
        if bursts.iter().all(|b| *b >= 4) && (bursts.len() as f64) <= (bars / 4.0).max(1.0) {
            return Plays::Roll;
        }
        return Plays::Hit;
    }
    let pitch = median(notes.iter().map(|n| n.midi).collect()).unwrap_or(60.0);
    if pitch < BASS_BELOW {
        return Plays::Bass;
    }
    let mut classes: Vec<i64> = notes
        .iter()
        .map(|n| (n.midi.round() as i64).rem_euclid(12))
        .collect();
    classes.sort();
    classes.dedup();
    let gaps: Vec<f64> = groups
        .windows(2)
        .map(|w| w[1][0].beat - w[0][0].beat)
        .collect();
    let regular = median(gaps.clone()).is_some_and(|g| {
        gaps.iter().filter(|x| (*x - g).abs() < 1e-3).count() * 4 >= gaps.len() * 3
    });
    if lengths <= 0.5 && classes.len() <= 4 && regular {
        return Plays::Arp;
    }
    Plays::Melody
}

pub(crate) fn grid(notes: &[Note], beats_per_bar: u32) -> u32 {
    let bpb = beats_per_bar.max(1);
    let fits = |steps: u32| {
        let step = f64::from(bpb) / f64::from(steps);
        notes.iter().all(|n| {
            let at = n.beat / step;
            (at - at.round()).abs() * step <= 0.1 * step + 1e-6
        })
    };
    for factor in [4, 8, 3, 6, 12, 16] {
        let steps = bpb * factor;
        if steps <= MAX_STEPS && fits(steps) {
            return steps;
        }
    }
    (bpb * 4).min(MAX_STEPS)
}

pub fn patterns(notes: &[Note], beats_per_bar: u32, steps: u32) -> Vec<String> {
    let steps = steps.clamp(1, MAX_STEPS) as i64;
    let bpb = f64::from(beats_per_bar.max(1));
    let mut bars: BTreeMap<i64, Vec<bool>> = BTreeMap::new();
    for note in notes {
        if !note.beat.is_finite() {
            continue;
        }
        let at = (note.beat / bpb * steps as f64).round() as i64;
        let bar = at.div_euclid(steps);
        let slot = at.rem_euclid(steps) as usize;
        bars.entry(bar)
            .or_insert_with(|| vec![false; steps as usize])[slot] = true;
    }
    let mut seen: Vec<(String, usize)> = Vec::new();
    for hits in bars.values() {
        let text: String = hits.iter().map(|h| if *h { 'x' } else { '.' }).collect();
        match seen.iter_mut().find(|(t, _)| *t == text) {
            Some((_, count)) => *count += 1,
            None => seen.push((text, 1)),
        }
    }
    let mut common: Vec<(usize, String, usize)> = seen
        .into_iter()
        .enumerate()
        .map(|(i, (t, c))| (i, t, c))
        .collect();
    common.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    common.truncate(MAX_PATTERNS);
    common.sort_by(|a, b| {
        let density = |t: &str| t.chars().filter(|c| *c == 'x').count();
        density(&a.1).cmp(&density(&b.1)).then(a.0.cmp(&b.0))
    });
    common.into_iter().map(|(_, t, _)| t).collect()
}

fn voiced(chord: &[i32]) -> Vec<i32> {
    let mut root = chord[0];
    while root > 62 {
        root -= 12;
    }
    while root < 55 {
        root += 12;
    }
    let shift = root - chord[0];
    chord.iter().map(|c| c + shift).collect()
}

fn cycle(values: &[i32]) -> Vec<i32> {
    if values.is_empty() {
        return Vec::new();
    }
    for period in 1..=MAX_CYCLE.min(values.len()) {
        let checks = values.len() - period;
        if checks == 0 {
            return values[..period].to_vec();
        }
        let same = (period..values.len())
            .filter(|i| values[*i] == values[i - period])
            .count();
        if same as f64 >= CYCLE_MATCH * checks as f64 {
            return values[..period].to_vec();
        }
    }
    Vec::new()
}

pub fn cycles(notes: &[Note], key: &Key, harmony: &Harmony) -> (Vec<i32>, Vec<i32>) {
    let notes = sorted(notes);
    if notes.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let step = if harmony.beats > 0.0 {
        harmony.beats
    } else {
        4.0
    };
    let options = candidates(key, harmony.sevenths);
    let mut frames: BTreeMap<i64, [f64; 12]> = BTreeMap::new();
    for note in &notes {
        let at = (note.beat / step + 1e-9).floor() as i64;
        let pc = (note.midi.round() as i64).rem_euclid(12) as usize;
        frames.entry(at).or_insert([0.0; 12])[pc] += note.beats.max(1e-3);
    }
    let chords: BTreeMap<i64, Vec<i32>> = frames
        .iter()
        .filter_map(|(at, frame)| {
            let found = best(frame, &options)?;
            Some((*at, key.chord(&found.text, harmony.sevenths).ok()?))
        })
        .collect();
    let mut tones = Vec::new();
    let mut octaves = Vec::new();
    for group in onsets(&notes) {
        let note = group[0];
        let at = (note.beat / step + 1e-9).floor() as i64;
        let Some(chord) = chords.get(&at) else {
            continue;
        };
        let voicing = voiced(chord);
        let midi = note.midi.round() as i32;
        let pc = midi.rem_euclid(12);
        let index = voicing
            .iter()
            .position(|v| v.rem_euclid(12) == pc)
            .unwrap_or(0);
        tones.push(index as i32);
        octaves.push((midi - voicing[index]).div_euclid(12));
    }
    (cycle(&tones), cycle(&octaves))
}

fn osc(wave: Wave) -> Osc {
    Osc {
        wave: Some(wave),
        sample: None,
        width: None,
        cents: 0.0,
        octave: 0,
        gain: 1.0,
        hz: None,
        to_hz: None,
        glide_ms: 0.0,
        bend: 0.0,
        pan: 0.0,
        at_ms: 0.0,
        env: None,
    }
}

fn patch(osc: Vec<Osc>, env: [f32; 4]) -> Patch {
    Patch {
        osc,
        env,
        linear: false,
        lowpass: None,
        highpass: None,
        sweep: None,
        sweep_ms: None,
        drive: None,
        send: 0.0,
        gain: 1.0,
        pan: 0.0,
    }
}

pub fn default_patch(plays: Plays) -> Patch {
    match plays {
        Plays::Chord => Patch {
            lowpass: Some([600.0, 4000.0]),
            send: 0.3,
            gain: 0.5,
            ..patch(
                vec![
                    Osc {
                        cents: -7.0,
                        ..osc(Wave::Saw)
                    },
                    Osc {
                        cents: 7.0,
                        ..osc(Wave::Saw)
                    },
                ],
                [40.0, 400.0, 0.7, 300.0],
            )
        },
        Plays::Bass => Patch {
            lowpass: Some([300.0, 1500.0]),
            drive: Some((DriveCurve::Tanh, 2.0, 0.3)),
            ..patch(
                vec![
                    osc(Wave::Saw),
                    Osc {
                        gain: 0.8,
                        ..osc(Wave::Sine)
                    },
                ],
                [2.0, 220.0, 0.6, 60.0],
            )
        },
        Plays::Arp => Patch {
            lowpass: Some([1200.0, 6000.0]),
            send: 0.2,
            gain: 0.6,
            ..patch(
                vec![Osc {
                    width: Some(0.3),
                    ..osc(Wave::Pulse)
                }],
                [2.0, 150.0, 0.2, 80.0],
            )
        },
        Plays::Melody | Plays::Counter | Plays::Answer => Patch {
            send: 0.2,
            gain: 0.7,
            ..patch(
                vec![
                    osc(Wave::Triangle),
                    Osc {
                        cents: 5.0,
                        gain: 0.4,
                        ..osc(Wave::Saw)
                    },
                ],
                [10.0, 200.0, 0.6, 150.0],
            )
        },
        Plays::Hit => Patch {
            highpass: Some(2000.0),
            gain: 0.5,
            ..patch(vec![osc(Wave::White)], [1.0, 60.0, 0.0, 30.0])
        },
        Plays::Riser | Plays::Reverse => Patch {
            sweep: Some([300.0, 8000.0]),
            sweep_ms: Some(4000.0),
            gain: 0.4,
            ..patch(
                vec![
                    osc(Wave::Pink),
                    Osc {
                        gain: 0.3,
                        ..osc(Wave::Saw)
                    },
                ],
                [2000.0, 100.0, 1.0, 200.0],
            )
        },
        Plays::Roll => Patch {
            highpass: Some(400.0),
            gain: 0.5,
            ..patch(vec![osc(Wave::White)], [1.0, 90.0, 0.0, 40.0])
        },
        Plays::Impact => Patch {
            lowpass: Some([120.0, 2000.0]),
            send: 0.4,
            drive: Some((DriveCurve::Soft, 1.5, 0.5)),
            ..patch(
                vec![
                    Osc {
                        octave: -1,
                        ..osc(Wave::Sine)
                    },
                    Osc {
                        gain: 0.5,
                        ..osc(Wave::White)
                    },
                ],
                [1.0, 1200.0, 0.0, 800.0],
            )
        },
    }
}

fn kind(plays: Plays) -> &'static str {
    match plays {
        Plays::Chord => "chord",
        Plays::Bass => "bass",
        Plays::Arp => "arp",
        Plays::Melody => "melody",
        Plays::Hit => "hit",
        Plays::Riser => "riser",
        Plays::Reverse => "reverse",
        Plays::Roll => "roll",
        Plays::Impact => "impact",
        Plays::Counter => "counter",
        Plays::Answer => "answer",
    }
}

fn place(table: &mut BTreeMap<String, Patch>, name: &str, patch: &Patch) -> String {
    let mut candidate = name.to_string();
    let mut n = 2;
    loop {
        match table.get(&candidate) {
            None => {
                table.insert(candidate.clone(), patch.clone());
                return candidate;
            }
            Some(existing) if existing == patch => return candidate,
            Some(_) => {
                candidate = format!("{name}_{n}");
                n += 1;
            }
        }
    }
}

fn octave_of(plays: Plays, notes: &[Note], key: &Key) -> i32 {
    let Some(pitch) = median(notes.iter().map(|n| n.midi).collect()) else {
        return 0;
    };
    let octave = match plays {
        Plays::Bass | Plays::Melody | Plays::Counter | Plays::Answer => {
            ((pitch - f64::from(key.root)) / 12.0).floor()
        }
        Plays::Chord | Plays::Arp => ((pitch - 62.0) / 12.0).round(),
        _ => 0.0,
    };
    (octave as i32).clamp(-8, 8)
}

fn hold_of(notes: &[Note]) -> f64 {
    let Some(length) = median(notes.iter().map(|n| n.beats).collect()) else {
        return 0.0;
    };
    let length = (length * HOLD_GRID).round() / HOLD_GRID;
    if length < 1.0 / HOLD_GRID {
        0.0
    } else {
        length.min(MAX_BEATS)
    }
}

fn stabs(notes: &[Note]) -> bool {
    let groups = onsets(notes);
    let gaps: Vec<f64> = groups
        .windows(2)
        .map(|w| w[1][0].beat - w[0][0].beat)
        .collect();
    let (Some(gap), Some(length)) = (
        median(gaps),
        median(notes.iter().map(|n| n.beats).collect()),
    ) else {
        return false;
    };
    length < 0.5 * gap
}

struct Line<'a> {
    name: &'a str,
    parts: Vec<(&'a Piece, &'a Part)>,
}

fn classify(piece: &Piece, part: &Part, leads: &[&Part]) -> Plays {
    if let Some(plays) = part.plays {
        return plays;
    }
    if leads.iter().any(|l| std::ptr::eq(*l, part)) {
        return Plays::Melody;
    }
    let found = plays(part, piece.beats_per_bar);
    if found != Plays::Melody || !part.pitched || !monophonic(part) {
        return found;
    }
    let pitch = |p: &Part| median(p.notes.iter().map(|n| n.midi).collect());
    let mine = pitch(part);
    let lowest = piece
        .parts
        .iter()
        .filter(|p| p.plays.is_none() && p.pitched && !p.notes.is_empty() && monophonic(p))
        .filter_map(|p| pitch(p).map(|m| (p, m)))
        .fold(None::<(&Part, f64)>, |low, (p, m)| match low {
            Some((_, l)) if l <= m => low,
            _ => Some((p, m)),
        });
    match (lowest, mine) {
        (Some((low, _)), Some(m)) if std::ptr::eq(low, part) && m < 60.0 => Plays::Bass,
        _ => found,
    }
}

pub fn derive(
    pieces: &[Piece],
    key: &Key,
    harmony: &Harmony,
) -> (Vec<Role>, BTreeMap<String, Patch>) {
    let mut lines: Vec<Line> = Vec::new();
    for piece in pieces {
        for part in &piece.parts {
            match lines.iter_mut().find(|l| l.name == part.name) {
                Some(line) => line.parts.push((piece, part)),
                None => lines.push(Line {
                    name: &part.name,
                    parts: vec![(piece, part)],
                }),
            }
        }
    }
    let leads = lead(pieces);
    let mut table: BTreeMap<String, Patch> = BTreeMap::new();
    let mut roles = Vec::with_capacity(lines.len());
    for line in &lines {
        let (piece, part) = line.parts[0];
        let plays = line
            .parts
            .iter()
            .find_map(|(_, p)| p.plays)
            .unwrap_or_else(|| classify(piece, part, &leads));
        let bpb = piece.beats_per_bar.max(1);
        let mut notes: Vec<Note> = Vec::new();
        let mut offset = 0.0;
        for (piece, part) in &line.parts {
            let bar = f64::from(piece.beats_per_bar.max(1));
            notes.extend(part.notes.iter().map(|n| Note {
                beat: n.beat + offset,
                ..*n
            }));
            let length = piece
                .beats
                .max(bars_spanned(&part.notes, bar) * bar)
                .max(0.0);
            offset += (length / bar).ceil() * bar;
        }
        let copied = line.parts.iter().find_map(|(piece, part)| {
            let name = part.patch.as_ref()?;
            Some((name.as_str(), piece.patches.get(name)?))
        });
        let patch = match copied {
            Some((name, patch)) => place(&mut table, name, patch),
            None => place(&mut table, kind(plays), &default_patch(plays)),
        };
        let mut role = Role {
            name: line.name.to_string(),
            patch,
            plays,
            euclid: Vec::new(),
            patterns: Vec::new(),
            rotate: Vec::new(),
            octave: octave_of(plays, &notes, key),
            gain: if part.gain.is_finite() {
                part.gain
            } else {
                1.0
            },
            hold: 0.0,
            accent: 0.0,
            tones: Vec::new(),
            octaves: Vec::new(),
            semis: Vec::new(),
            voicing: Voicing::Lead,
            range: None,
            ramp: 0.0,
            follows: None,
            delay: 0.0,
            interval: 0,
            walk: 0.0,
            inversions: 0.0,
            enter: None,
        };
        let steps = grid(&notes, bpb);
        match plays {
            Plays::Chord => {
                role.hold = hold_of(&notes);
                if stabs(&notes) {
                    role.patterns = patterns(&notes, bpb, steps);
                }
            }
            Plays::Arp => {
                role.patterns = patterns(&notes, bpb, steps);
                let (tones, octaves) = cycles(&notes, key, harmony);
                let base = role.octave;
                role.tones = tones;
                role.octaves = octaves.iter().map(|o| (o - base).clamp(-8, 8)).collect();
                if role.octaves.iter().all(|o| *o == 0) {
                    role.octaves.clear();
                }
            }
            Plays::Bass => {
                role.patterns = patterns(&notes, bpb, steps);
                let raw: Vec<i32> = onsets(&notes)
                    .iter()
                    .map(|g| {
                        let rel = g[0].midi.round() as i32 - key.root;
                        (rel.div_euclid(12) - role.octave).clamp(-8, 8)
                    })
                    .collect();
                role.octaves = cycle(&raw);
                if role.octaves.iter().all(|o| *o == 0) {
                    role.octaves.clear();
                }
            }
            Plays::Hit => role.patterns = patterns(&notes, bpb, steps),
            _ => {}
        }
        if plays.patterned() && role.patterns.is_empty() {
            role.patterns = vec![".".repeat(steps as usize)];
        }
        roles.push(role);
    }
    let leader = roles
        .iter()
        .find(|r| r.plays == Plays::Melody)
        .or_else(|| roles.iter().find(|r| !r.follower() && !r.plays.event()))
        .map(|r| r.name.clone());
    for role in roles.iter_mut().filter(|r| r.follower()) {
        match &leader {
            Some(name) => role.follows = Some(name.clone()),
            None => role.plays = Plays::Melody,
        }
    }
    (roles, table)
}
