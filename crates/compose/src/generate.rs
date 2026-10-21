use pfx_core::sim::{Rng, ln, pow};

use crate::score::{CueTrack, Fire, Glide, Note, Placed, Score, ScoreCue, StyleRef, Tempo, Track};
use crate::style::{
    Cue, CuePlays, Develop, MAX_BARS, MAX_BPM, MAX_NOTE_BEATS, MAX_NOTES, MAX_SECONDS, Plays, Role,
    Section, Style, Voicing, midi_of,
};
use crate::theory::{Key, spread, voicing};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub seed: u64,
    pub seconds: f64,
    pub hits: Vec<f64>,
    pub bpm: Option<u32>,
    pub key: Option<String>,
    pub plan: Option<String>,
}

impl Request {
    pub fn check(&self) -> Result<(), String> {
        if !self.seconds.is_finite() || self.seconds <= 0.0 || self.seconds > MAX_SECONDS {
            return Err(format!(
                "the length is {} s; it wants above 0 up to {MAX_SECONDS}",
                self.seconds
            ));
        }
        let mut last = None;
        for hit in &self.hits {
            if !hit.is_finite() || *hit < 0.0 || *hit >= self.seconds {
                return Err(format!(
                    "a hit at {hit} s falls outside the length, 0 up to {} s",
                    self.seconds
                ));
            }
            if last.is_some_and(|last| hit <= last) {
                return Err(format!(
                    "the hit at {hit} s comes before the one ahead of it; hits must rise"
                ));
            }
            last = Some(hit);
        }
        if let Some(bpm) = self.bpm
            && !(1..=MAX_BPM).contains(&bpm)
        {
            return Err(format!("the tempo is {bpm} bpm; it wants 1 to {MAX_BPM}"));
        }
        Ok(())
    }
}

const KEY_OCTAVE: i32 = 3;

fn label(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn stream(rng: &Rng, names: &[&str]) -> Rng {
    names
        .iter()
        .fold(rng.clone(), |r, name| r.fork(label(name)))
}

fn round(value: f64, scale: f64) -> f64 {
    let r = (value * scale).round() / scale;
    if r == 0.0 { 0.0 } else { r }
}

fn log2(x: f64) -> f64 {
    ln(x) / ln(2.0)
}

fn pick<T: Clone>(rng: &mut Rng, items: &[(T, u32)]) -> T {
    let weights: Vec<u32> = items.iter().map(|(_, w)| *w).collect();
    let at = rng.weighted_index_u32(&weights).unwrap_or(items.len() - 1);
    items[at].0.clone()
}

fn between(rng: &mut Rng, lo: u32, hi: u32) -> u32 {
    if lo >= hi {
        lo
    } else {
        rng.range_u64(u64::from(lo), u64::from(hi) + 1) as u32
    }
}

fn bar_seconds(style: &Style, bpm: u32) -> Vec<f64> {
    let bpb = f64::from(style.beats_per_bar);
    style
        .sections
        .iter()
        .map(|s| 60.0 * bpb / (f64::from(bpm) * s.tempo))
        .collect()
}

fn miss(style: &Style, bpm: u32, request: &Request, spans: &[(u32, u32)]) -> f64 {
    let lengths = bar_seconds(style, bpm);
    let mut at = 0.0;
    let mut cost = 0.0;
    for ((spec, (_, bars)), len) in style.sections.iter().zip(spans).zip(&lengths) {
        if let Some(hit) = spec.hit.and_then(|h| request.hits.get(h)) {
            cost += (at - hit).abs();
        }
        at += f64::from(*bars) * len;
    }
    cost + (request.seconds - at).abs() * 0.01
}

type Spans = Vec<(u32, u32)>;

fn choose(style: &Style, request: &Request, form: &Rng, rng: &mut Rng) -> (u32, Spans) {
    let [lo, hi] = style.bpm;
    let placed = |bpm| {
        place(
            style,
            bpm,
            request.seconds,
            &request.hits,
            &mut form.clone(),
        )
    };
    if request.hits.is_empty() {
        let bpm = between(rng, lo, hi);
        return (bpm, placed(bpm));
    }
    let mut best: Option<(f64, u32, Spans)> = None;
    for bpm in lo..=hi {
        let spans = placed(bpm);
        let bars: u32 = spans.iter().map(|(_, b)| *b).fold(0, u32::saturating_add);
        if bars > MAX_BARS {
            if best.is_none() {
                best = Some((f64::INFINITY, bpm, spans));
            }
            break;
        }
        let cost = miss(style, bpm, request, &spans);
        if best.as_ref().is_none_or(|b| cost < b.0 - 1e-9) {
            best = Some((cost, bpm, spans));
        }
    }
    best.map_or_else(|| (lo, placed(lo)), |b| (b.1, b.2))
}

fn fit(group: &[Section], lengths: &[f64], target: f64, within: bool, rng: &mut Rng) -> Vec<u32> {
    let mut got: Vec<u32> = group.iter().map(|s| s.bars[0]).collect();
    let mut cur: f64 = got
        .iter()
        .zip(lengths)
        .map(|(b, l)| f64::from(*b) * l)
        .sum();
    let growable: Vec<usize> = (0..group.len())
        .filter(|k| group[*k].bars[0] < group[*k].bars[1])
        .collect();
    let mut bars: u32 = got.iter().fold(0, |a, b| a.saturating_add(*b));
    let mut open: Vec<usize> = Vec::with_capacity(group.len());
    loop {
        if bars > MAX_BARS {
            return got;
        }
        let helps = |k: &usize| {
            if within {
                cur + lengths[*k] <= target + 1e-9
            } else {
                cur + lengths[*k] / 2.0 < target
            }
        };
        open.clear();
        open.extend(
            (0..group.len())
                .filter(|k| got[*k] < group[*k].bars[1])
                .filter(helps),
        );
        if open.is_empty() && (0..group.len()).all(|k| got[k] >= group[k].bars[1]) {
            if growable.is_empty() {
                open.extend((0..group.len()).filter(helps));
            } else {
                open.extend(growable.iter().copied().filter(helps));
            }
        }
        if open.is_empty() {
            break;
        }
        let k = open[rng.index(open.len())];
        got[k] += 1;
        bars += 1;
        cur += lengths[k];
    }
    while let Some(k) = (0..group.len())
        .filter(|k| {
            got[*k] > 1
                && if within {
                    cur > target + 1e-9
                } else {
                    cur - lengths[*k] / 2.0 > target
                }
        })
        .max_by_key(|k| (got[*k], std::cmp::Reverse(*k)))
    {
        got[k] -= 1;
        cur -= lengths[k];
    }
    refine(group, lengths, target, within, got)
}

fn refine(
    group: &[Section],
    lengths: &[f64],
    target: f64,
    within: bool,
    mut got: Vec<u32>,
) -> Vec<u32> {
    let lo: Vec<u32> = group
        .iter()
        .zip(&got)
        .map(|(s, g)| s.bars[0].min(*g).max(1))
        .collect();
    let hi: Vec<u32> = group
        .iter()
        .zip(&got)
        .map(|(s, g)| s.bars[1].max(*g))
        .collect();
    let error = |cur: f64| {
        if within {
            if cur > target + 1e-9 {
                f64::MAX
            } else {
                target - cur
            }
        } else {
            (cur - target).abs()
        }
    };
    let n = group.len();
    let mut cur: f64 = got
        .iter()
        .zip(lengths)
        .map(|(b, l)| f64::from(*b) * l)
        .sum();
    for _ in 0..4 * MAX_BARS {
        let mut best = (error(cur) - 1e-9, None);
        for up in 0..=n {
            for down in 0..=n {
                if up == down || up < n && got[up] >= hi[up] || down < n && got[down] <= lo[down] {
                    continue;
                }
                let mut next = cur;
                if up < n {
                    next += lengths[up];
                }
                if down < n {
                    next -= lengths[down];
                }
                let e = error(next);
                if e < best.0 {
                    best = (e, Some((up, down, next)));
                }
            }
        }
        let Some((up, down, next)) = best.1 else {
            break;
        };
        if up < n {
            got[up] += 1;
        }
        if down < n {
            got[down] -= 1;
        }
        cur = next;
    }
    got
}

fn place(style: &Style, bpm: u32, seconds: f64, hits: &[f64], rng: &mut Rng) -> Spans {
    let sections = &style.sections;
    let n = sections.len();
    let lengths = bar_seconds(style, bpm);
    let mut bars: Vec<u32> = sections.iter().map(|s| s.bars[0]).collect();
    if sections.iter().all(|s| s.bars[0] == s.bars[1]) {
        return bars
            .iter()
            .scan(0, |at, b| {
                let from = *at;
                *at += b;
                Some((from, *b))
            })
            .collect();
    }
    let pins: Vec<Option<f64>> = sections
        .iter()
        .enumerate()
        .map(|(k, s)| {
            if k == 0 {
                None
            } else {
                s.hit.and_then(|h| hits.get(h).copied())
            }
        })
        .collect();
    let mut elapsed = 0.0;
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && pins[j].is_none() {
            j += 1;
        }
        let end = if j < n {
            pins[j].unwrap_or(seconds)
        } else {
            seconds
        };
        let got = fit(&sections[i..j], &lengths[i..j], end - elapsed, j == n, rng);
        for (k, b) in got.iter().enumerate() {
            bars[i + k] = *b;
            elapsed += f64::from(*b) * lengths[i + k];
        }
        i = j;
    }
    bars.iter()
        .scan(0, |at, b| {
            let from = *at;
            *at += b;
            Some((from, *b))
        })
        .collect()
}

fn fold(mut midi: i32, range: [i32; 2]) -> i32 {
    while midi > range[1] {
        midi -= 12;
    }
    while midi < range[0] {
        midi += 12;
    }
    midi
}

fn root_voicing(chord: &[i32], octave: i32) -> Vec<i32> {
    let mut root = chord[0];
    while root > 62 {
        root -= 12;
    }
    while root < 55 {
        root += 12;
    }
    let shift = root - chord[0] + 12 * octave;
    chord.iter().map(|c| c + shift).collect()
}

fn voice_lead(prev: &[i32], chord: &[i32], octave: i32) -> Vec<i32> {
    let base = root_voicing(chord, octave);
    if prev.len() != base.len() {
        return base;
    }
    let mut best = base.clone();
    let mut cost = i32::MAX;
    for inv in 0..base.len() {
        for shift in [-12, 0, 12] {
            let mut v = base.clone();
            for x in v.iter_mut().take(inv) {
                *x += 12;
            }
            for x in &mut v {
                *x += shift;
            }
            v.sort();
            let c: i32 = v.iter().zip(prev).map(|(a, b)| (a - b).abs()).sum();
            if c < cost {
                cost = c;
                best = v;
            }
        }
    }
    best
}

fn voice(role: &Role, prev: &[i32], chord: &[i32]) -> Vec<i32> {
    match role.voicing {
        Voicing::Root => root_voicing(chord, role.octave),
        Voicing::Lead => voice_lead(prev, chord, role.octave),
        Voicing::Top => {
            let intervals: Vec<i32> = chord.iter().map(|c| c - chord[0]).collect();
            let [lo, hi] = role.range.unwrap_or([48, 72]);
            let range = [lo + 12 * role.octave, hi + 12 * role.octave];
            voicing::symbol(&intervals)
                .and_then(|s| voicing::top_note_diff(voicing::search(chord[0], s, range), prev))
                .unwrap_or_else(|| voice_lead(prev, chord, role.octave))
        }
    }
}

fn nearest_tone(midi: i32, chord: &[i32]) -> i32 {
    let mut best = midi;
    let mut dist = i32::MAX;
    for &c in chord {
        for o in -4..=4 {
            let m = c + 12 * o;
            if (m - midi).abs() < dist {
                dist = (m - midi).abs();
                best = m;
            }
        }
    }
    best
}

fn nearest_toward(midi: i32, chord: &[i32], dir: i32) -> i32 {
    let mut best = (i32::MAX, midi);
    for &c in chord {
        for o in -4..=4 {
            let m = c + 12 * o;
            let rank = 2 * (m - midi).abs() + i32::from((m - midi).signum() != dir.signum());
            if rank < best.0 {
                best = (rank, m);
            }
        }
    }
    best.1
}

fn clashes(midi: f64, chord: &[i32], strict: bool) -> bool {
    let m = midi.round() as i32;
    if chord.iter().any(|t| (m - t).rem_euclid(12) == 0) {
        return false;
    }
    strict
        || chord
            .iter()
            .any(|t| matches!((m - t).rem_euclid(12), 1 | 11))
}

fn pattern(role: &Role, intensity: f64, rotate: u32) -> Vec<bool> {
    let at = |len: usize| ((intensity * (len as f64 - 1.0)).round().max(0.0) as usize).min(len - 1);
    if !role.patterns.is_empty() {
        return role.patterns[at(role.patterns.len())]
            .chars()
            .map(|c| c == 'x')
            .collect();
    }
    let [k, n] = role.euclid[at(role.euclid.len())];
    spread::spread(k, n, rotate)
}

fn motif(
    style: &Style,
    key: &Key,
    rng: &mut Rng,
    beats: f64,
    degree: &mut i32,
    octave: i32,
) -> Vec<(f64, i32, f64)> {
    let m = &style.melody;
    let mut out = Vec::new();
    let mut b = 0.0;
    while b < beats - 1e-9 {
        let len = pick(rng, &m.rhythm).min(beats - b);
        if b == 0.0 || rng.next_f64() >= m.rest {
            *degree += pick(rng, &m.steps);
            if m.chord_tones {
                *degree = (*degree).clamp(0, 4);
            } else {
                let midi = key.pitch(*degree);
                let mid = (m.range[0] + m.range[1]) / 2 - 12 * octave;
                if (midi - mid).abs() > 7 {
                    *degree -= (midi - mid).signum() * 2;
                }
            }
            out.push((b, *degree, len * 0.9));
        }
        b += len;
    }
    out
}

fn ring(line: &mut [(f64, i32, f64)], beats: f64) {
    for k in 0..line.len() {
        let end = line.get(k + 1).map_or(beats, |n| n.0);
        let (b, _, l) = &mut line[k];
        if *b + *l > end + 1e-9 {
            *l = (end - *b).max(0.0) * 0.9;
        }
    }
}

fn fragment(line: &[(f64, i32, f64)], beats: f64, step: i32) -> Vec<(f64, i32, f64)> {
    let half = beats / 2.0;
    let mut head: Vec<(f64, i32, f64)> =
        line.iter().filter(|n| n.0 < half - 1e-9).copied().collect();
    if head.is_empty() {
        return line.to_vec();
    }
    ring(&mut head, half);
    let again = head.iter().map(|&(b, d, l)| (b + half, d + step, l));
    let mut out: Vec<(f64, i32, f64)> = head.iter().copied().chain(again).collect();
    ring(&mut out, beats);
    out
}

fn split(line: &[(f64, i32, f64)], beats: f64) -> Vec<(f64, i32, f64)> {
    let mut out = Vec::with_capacity(line.len() * 2);
    for (k, &(b, d, l)) in line.iter().enumerate() {
        let next = line.get(k + 1);
        let slot = next.map_or(beats, |n| n.0) - b;
        if slot < 1.0 - 1e-9 || l < 0.9 - 1e-9 {
            out.push((b, d, l));
            continue;
        }
        let toward = next.map_or(-1, |n| (n.1 - d).signum());
        let toward = if toward == 0 { 1 } else { toward };
        out.push((b, d, 0.45));
        out.push((b + 0.5, d + toward, (l - 0.45).min((slot - 0.5) * 0.9)));
    }
    out
}

fn next_hit(pat: &[bool], i: usize, bar_beat: f64, step: f64, bpb: f64) -> f64 {
    match pat.iter().skip(i + 1).position(|on| *on) {
        Some(k) => bar_beat + (i + 1 + k) as f64 * step,
        None => bar_beat + bpb + pat.iter().position(|on| *on).unwrap_or(0) as f64 * step,
    }
}

fn counter_cost(
    m: i32,
    tone: bool,
    strong: bool,
    lead: Option<i32>,
    prev: Option<(i32, Option<i32>)>,
    back: Option<i32>,
) -> f64 {
    let dissonance = if strong { 1.0 } else { 0.3 };
    let mut cost = if tone { 0.0 } else { dissonance };
    if let Some(l) = lead {
        let gap = l - m;
        if gap < 3 {
            cost += 2.0;
        }
        let class = gap.rem_euclid(12);
        cost += match class {
            3 | 4 | 8 | 9 => 0.0,
            0 | 7 => 0.1,
            _ => dissonance,
        };
        if let Some((pm, Some(pl))) = prev {
            let was = (pl - pm).rem_euclid(12);
            if (class == 0 || class == 7) && class == was && pm != m && pl != l {
                cost += 0.5;
            }
            let (dm, dl) = ((m - pm).signum(), (l - pl).signum());
            if dm != 0 && dm == dl {
                cost += 0.1;
            }
            if dm != 0 && dm == -dl {
                cost -= 0.1;
            }
        }
    }
    if let Some((pm, _)) = prev {
        let leap = (m - pm).abs();
        if leap > 2 {
            cost += 0.04 * f64::from(leap - 2);
        }
        if leap == 0 {
            cost += 0.4;
        }
        if matches!(leap, 6 | 10 | 11) || leap > 12 {
            cost += 1.0;
        }
        if !tone && leap > 2 {
            cost += 1.0;
        }
        if back == Some(m) && leap > 0 {
            cost += 0.1;
        }
    }
    cost
}

struct Frame<'a> {
    style: &'a Style,
    key: Key,
    bpm: u32,
    bpb: f64,
    sections: Vec<Placed>,
    specs: Vec<&'a Section>,
    chords: Vec<Chord>,
    cadence: f64,
}

struct Chord {
    beat: f64,
    tones: Vec<i32>,
    name: String,
    applied: bool,
}

impl Frame<'_> {
    fn section_at(&self, beat: f64) -> usize {
        let bar = (beat / self.bpb).floor().max(0.0) as u32;
        self.sections
            .partition_point(|s| s.from <= bar)
            .saturating_sub(1)
    }

    fn chord_index(&self, beat: f64) -> usize {
        let within = beat + 1e-9 * self.style.harmony.beats;
        self.chords
            .partition_point(|c| c.beat <= within)
            .saturating_sub(1)
    }

    fn intensity(&self, beat: f64) -> f64 {
        let sec = &self.sections[self.section_at(beat)];
        let [i0, i1] = sec.intensity.map(f64::from);
        let from = f64::from(sec.from) * self.bpb;
        let beats = f64::from(sec.bars) * self.bpb;
        if beats <= 0.0 {
            return i0;
        }
        i0 + (i1 - i0) * ((beat - from) / beats).clamp(0.0, 1.0)
    }

    fn chord_at(&self, beat: f64) -> &[i32] {
        &self.chords[self.chord_index(beat)].tones
    }

    fn triads(&self, beat: f64) -> bool {
        self.specs[self.section_at(beat)].triads
            || (self.style.harmony.land
                && self.chords.last().is_some_and(|c| beat >= c.beat - 1e-9))
    }

    fn voiced(&self, beat: f64) -> &[i32] {
        let c = self.chord_at(beat);
        if self.triads(beat) {
            &c[..c.len().min(3)]
        } else {
            c
        }
    }

    fn line_tones(&self, beat: f64) -> &[i32] {
        let c = &self.chords[self.chord_index(beat)];
        if (c.applied && !self.style.harmony.sevenths) || self.triads(beat) {
            &c.tones[..c.tones.len().min(3)]
        } else {
            &c.tones
        }
    }

    fn split(&self, k: usize) -> bool {
        k > 0 && k < self.chords.len() && (self.chords[k].applied || self.chords[k - 1].applied)
    }
}

fn harmony(frame: &mut Frame, total_beats: f64, base: &Rng, applied: &Rng) -> Result<(), String> {
    let h = &frame.style.harmony;
    let mut rngs: Vec<Rng> = frame
        .specs
        .iter()
        .map(|s| stream(base, &[&s.name]))
        .collect();
    let starts: Vec<(String, u32)> = h.start.iter().map(|s| (s.clone(), 1)).collect();
    let mut current = pick(&mut rngs[0], &starts);
    let steps = ((total_beats / h.beats).round() as usize).max(1);
    if steps > MAX_NOTES {
        return Err(format!(
            "the harmony changes {steps} times; a score holds at most {MAX_NOTES} chords, so lengthen `harmony.beats` or shorten the music"
        ));
    }
    let phrase = h.phrase as usize;
    let cadence_from = steps.saturating_sub(h.cadence.len());
    frame.cadence = cadence_from as f64 * h.beats;
    let turn = |i: usize| -> &[String] {
        if phrase == 0 || h.turns.is_empty() {
            &h.turn
        } else {
            &h.turns[(i / phrase) % h.turns.len()]
        }
    };
    let turn_slot =
        |i: usize| phrase > 0 && i < cadence_from && i % phrase + turn(i).len() >= phrase;
    let mut names = Vec::with_capacity(steps);
    for i in 0..steps {
        if turn_slot(i) {
            current = turn(i)[i % phrase + turn(i).len() - phrase].clone();
        }
        if steps - i <= h.cadence.len() {
            current = h.cadence[h.cadence.len() - (steps - i)].clone();
        }
        frame.key.chord(&current, h.sevenths)?;
        names.push(current.clone());
        let rng = &mut rngs[frame.section_at(i as f64 * h.beats)];
        if let Some(next) = h.markov.get(&current) {
            current = pick(rng, next);
            let ahead = i + 2;
            let forced = if phrase == 0 || turn_slot(i + 1) {
                None
            } else if ahead == cadence_from {
                h.cadence.first()
            } else if turn_slot(ahead) {
                turn(ahead).first()
            } else {
                None
            };
            if forced.is_some() && forced == Some(&current) {
                let others: Vec<(String, u32)> = next
                    .iter()
                    .filter(|(n, w)| *w > 0 && Some(n) != forced)
                    .cloned()
                    .collect();
                if !others.is_empty() {
                    current = pick(&mut applied.fork((1 << 32) | i as u64), &others);
                }
            }
        }
    }
    for (i, name) in names.iter().enumerate() {
        let beat = i as f64 * h.beats;
        let mut slots = vec![(beat, name.clone())];
        let chance = frame.specs[frame.section_at(beat)]
            .secondary
            .unwrap_or(h.secondary);
        if chance > 0.0 && h.beats >= 2.0 && i + 1 < steps && !turn_slot(i) {
            let target = &names[i + 1];
            if target != name
                && let Some(dominant) = frame.key.secondary(target)
                && applied.fork((2 << 32) | i as u64).next_f64() < chance
            {
                slots.push((beat + h.beats / 2.0, dominant));
            }
        }
        for (at, numeral) in slots {
            let lift = frame.specs[frame.section_at(at)].lift;
            let tones: Vec<i32> = frame
                .key
                .chord(&numeral, h.sevenths)?
                .iter()
                .map(|m| m + lift)
                .collect();
            frame.chords.push(Chord {
                beat: at,
                tones,
                applied: numeral.contains('/'),
                name: numeral,
            });
        }
    }
    Ok(())
}

fn entry_bar(frame: &Frame, role: &Role, si: usize) -> Option<u32> {
    let sec = &frame.sections[si];
    let spec = frame.specs[si];
    let rank = if spec.roles.iter().any(|r| r == "*") {
        frame.style.roles.iter().position(|r| r.name == role.name)?
    } else {
        spec.roles.iter().position(|r| *r == role.name)?
    };
    let entry = if spec.round_bars > 0 && rank >= spec.start_roles {
        sec.from + (rank - spec.start_roles + 1) as u32 * spec.round_bars
    } else {
        sec.from
    };
    (entry < sec.from + sec.bars).then_some(entry)
}

struct Part<'a> {
    frame: &'a Frame<'a>,
    role: &'a Role,
    room: usize,
    notes: Vec<Note>,
    lowpass: Vec<Glide>,
    voicing: Vec<i32>,
    seed: Rng,
    theme: Vec<(f64, i32, f64)>,
    contour: Vec<i32>,
    slot: usize,
    sequence: i32,
    direction: i32,
    bass: Vec<i32>,
    leader: &'a [Note],
    last_chord: usize,
}

impl Part<'_> {
    fn full(&self, si: usize) -> String {
        format!(
            "[[role]] `{}` in section `{}` takes the score past {MAX_NOTES} notes; shorten the music or thin its roles",
            self.role.name, self.frame.specs[si].name
        )
    }

    fn push(&mut self, si: usize, note: Note) -> Result<(), String> {
        if self.notes.len() >= self.room {
            return Err(self.full(si));
        }
        self.notes.push(note);
        Ok(())
    }

    fn events(&mut self, si: usize) -> Result<(), String> {
        let frame = self.frame;
        let sec = &frame.sections[si];
        let spec = frame.specs[si];
        let start = f64::from(sec.from) * frame.bpb;
        let beats = f64::from(sec.bars) * frame.bpb;
        let end = start + beats;
        if sec.bars == 0 {
            return Ok(());
        }
        let lift = f64::from(spec.lift);
        let from = |beats: f64| (end - beats).max(start);
        match self.role.plays {
            Plays::Riser if spec.riser > 0.0 => {
                let at = from(spec.riser);
                self.push(
                    si,
                    Note {
                        bend: 12.0,
                        ..Note::new(at, 40.0 + lift, end - at, 1.0)
                    },
                )?;
            }
            Plays::Reverse if spec.riser > 0.0 => {
                let at = from(2.0);
                self.push(si, Note::new(at, 60.0, end - at, 1.0))?;
            }
            Plays::Roll if spec.roll > 0.0 => {
                let from = from(spec.roll);
                let span = end - from;
                let mut b = from;
                while b < end - 1e-6 {
                    let t = (b - from) / span;
                    self.push(si, Note::new(b, 60.0, 0.1, 0.3 + 0.7 * t * t))?;
                    b += if t > 0.75 {
                        0.125
                    } else if t > 0.5 {
                        0.25
                    } else {
                        0.5
                    };
                }
            }
            Plays::Impact if spec.impact => {
                let root = f64::from(frame.chord_at(start)[0]);
                self.push(si, Note::new(start, root, 4.0, 1.0))?;
            }
            _ => {}
        }
        Ok(())
    }

    fn held(&mut self, si: usize, entry: u32) -> Result<(), String> {
        let frame = self.frame;
        let role = self.role;
        let sec = &frame.sections[si];
        let spec = frame.specs[si];
        let bpb = frame.bpb;
        let start = f64::from(sec.from) * bpb;
        let beats = f64::from(sec.bars) * bpb;
        let end = start + beats;
        let last_bar = end - bpb;
        let [i0, i1] = sec.intensity.map(f64::from);
        let step = frame.style.harmony.beats;
        let hold = if role.hold > 0.0 { role.hold } else { step };
        let entry = f64::from(entry) * bpb;
        let onset = |k: u32, change: usize| {
            let held = entry + f64::from(k) * hold;
            let held = (held < end - 1e-9).then_some(held);
            let moved = frame.chords.get(change).map(|c| c.beat);
            let moved = moved.filter(|m| *m < end - 1e-9);
            match (held, moved) {
                (Some(h), Some(m)) if (h - m).abs() < 1e-9 => Some((m, change, k + 1, change + 1)),
                (Some(h), Some(m)) if m < h => Some((m, change, k, change + 1)),
                (Some(h), _) => Some((h, frame.chord_index(h), k + 1, change)),
                (None, Some(m)) => Some((m, change, k, change + 1)),
                (None, None) => None,
            }
        };
        let mut at = onset(0, frame.chord_index(entry) + 1);
        while let Some((beat, chord, k, change)) = at {
            let next = onset(k, change);
            let t = (beat - start) / beats;
            let intensity = i0 + (i1 - i0) * t;
            let swell = 1.0 + f64::from(role.ramp) * (t - 0.5);
            let len = match next {
                Some((next, ..)) => next - beat,
                None if spec.tail > 0.0 => {
                    ((last_bar - beat).max(0.0) + spec.tail).min(MAX_NOTE_BEATS)
                }
                None => hold.min(end - beat),
            };
            let tones = &frame.chords[chord].tones;
            let tones = if frame.triads(beat) {
                &tones[..tones.len().min(3)]
            } else {
                tones
            };
            let voicing = voice(role, &self.voicing, tones);
            let vel = (0.6 + 0.3 * intensity) * swell;
            for &m in &voicing {
                self.push(si, Note::new(beat, f64::from(m), len, vel))?;
            }
            self.voicing = voicing;
            at = next;
        }
        Ok(())
    }

    fn section(&mut self, si: usize, rng: &mut Rng) -> Result<(), String> {
        let frame = self.frame;
        let role = self.role;
        let Some(entry) = entry_bar(frame, role, si) else {
            return Ok(());
        };
        let sec = &frame.sections[si];
        let spec = frame.specs[si];
        let bpb = frame.bpb;
        let sec_start = f64::from(sec.from) * bpb;
        let sec_beats = f64::from(sec.bars) * bpb;
        let end_bar = sec.from + sec.bars;
        let end_beat = f64::from(end_bar) * bpb;
        let [i0, i1] = sec.intensity.map(f64::from);
        if let Some([lo, hi]) = frame.style.patch.get(&role.patch).and_then(|p| p.lowpass) {
            let (lo, hi) = (f64::from(lo), f64::from(hi));
            let entry_beat = f64::from(entry) * bpb;
            let a = lo * pow(hi / lo, i0);
            let b = lo * pow(hi / lo, i1);
            self.lowpass.push(Glide {
                beat: entry_beat,
                hz: round(a, 100.0),
                beats: 0.0,
            });
            self.lowpass.push(Glide {
                beat: entry_beat,
                hz: round(b, 100.0),
                beats: round(end_beat - entry_beat, 1e6),
            });
        }
        match role.plays {
            Plays::Counter => return self.counter(si, entry, rng),
            Plays::Answer => return self.answer(si, entry),
            _ => {}
        }
        let rotate = if role.rotate.is_empty() {
            0
        } else {
            role.rotate[rng.index(role.rotate.len())]
        };
        let mut motif_notes: Vec<(f64, i32, f64)> = Vec::new();
        let mut degree = 0i32;
        let mut hit_n = 0usize;
        let cycle = |v: &[i32], n: usize| if v.is_empty() { 0 } else { v[n % v.len()] };
        if role.plays == Plays::Chord && role.patterns.is_empty() && role.euclid.is_empty() {
            return self.held(si, entry);
        }
        for bar in entry..end_bar {
            let bar_beat = f64::from(bar) * bpb;
            let t = (bar_beat - sec_start) / sec_beats;
            let intensity = i0 + (i1 - i0) * t;
            let swell = 1.0 + f64::from(role.ramp) * (t - 0.5);
            match role.plays {
                Plays::Chord => {
                    let last = spec.tail > 0.0 && bar + 1 == end_bar;
                    let pat = pattern(role, intensity, rotate);
                    let step = bpb / pat.len() as f64;
                    for (i, on) in pat.iter().enumerate() {
                        if !on {
                            continue;
                        }
                        let beat = bar_beat + i as f64 * step;
                        let voicing = voice(role, &self.voicing, frame.voiced(beat));
                        let len = if last { spec.tail } else { role.hold.max(step) };
                        for &m in &voicing {
                            self.push(
                                si,
                                Note::new(beat, f64::from(m), len, (0.7 + 0.3 * intensity) * swell),
                            )?;
                        }
                        self.voicing = voicing;
                    }
                }
                Plays::Melody => {
                    let mb = frame.style.melody.motif_bars.max(1);
                    let in_motif = (bar - entry) % mb;
                    let developed = !frame.style.melody.develop.is_empty();
                    if in_motif == 0 && developed {
                        motif_notes = self.develop(f64::from(mb) * bpb, bar_beat);
                    } else if in_motif == 0 {
                        let reuse =
                            !motif_notes.is_empty() && rng.next_f64() < frame.style.melody.repeat;
                        if !reuse {
                            motif_notes = motif(
                                frame.style,
                                &frame.key,
                                rng,
                                f64::from(mb) * bpb,
                                &mut degree,
                                role.octave,
                            );
                        }
                    }
                    let lo = f64::from(in_motif) * bpb;
                    for &(b, d, len) in &motif_notes {
                        if b < lo || b >= lo + bpb {
                            continue;
                        }
                        let beat = bar_beat + b - lo;
                        let melody = &frame.style.melody;
                        let midi = if developed {
                            d
                        } else {
                            fold(self.line_pitch(beat, b, d, 0), melody.range)
                        };
                        self.push(
                            si,
                            Note::new(
                                beat,
                                f64::from(midi),
                                len,
                                (0.55 + 0.35 * intensity) * swell,
                            ),
                        )?;
                    }
                }
                Plays::Bass | Plays::Arp | Plays::Hit => {
                    let pat = pattern(role, intensity, rotate);
                    let step = bpb / pat.len() as f64;
                    for (i, on) in pat.iter().enumerate() {
                        if !on {
                            continue;
                        }
                        let b = i as f64 * step;
                        let beat = bar_beat + b;
                        let c = frame.voiced(beat);
                        let accent = if b.fract() == 0.0 {
                            1.0
                        } else {
                            1.0 - f64::from(role.accent)
                        };
                        let jitter = rng.next_f64() * 0.08;
                        let midi = match role.plays {
                            Plays::Bass if role.walk > 0.0 || role.inversions > 0.0 => {
                                let next = next_hit(&pat, i, bar_beat, step, bpb);
                                self.walking(beat, next, hit_n, rng)
                            }
                            Plays::Bass => {
                                c[0] + 12 * (role.octave + cycle(&role.octaves, hit_n))
                                    + cycle(&role.semis, hit_n)
                            }
                            Plays::Arp => {
                                let n = c.len() as i32;
                                let root = root_voicing(c, 0);
                                let idx = if role.tones.is_empty() {
                                    hit_n as i32
                                } else {
                                    cycle(&role.tones, hit_n)
                                };
                                root[idx.rem_euclid(n) as usize]
                                    + 12 * (idx.div_euclid(n)
                                        + role.octave
                                        + cycle(&role.octaves, hit_n))
                            }
                            _ => 60,
                        };
                        let len = if role.hold > 0.0 { role.hold } else { step };
                        self.push(
                            si,
                            Note::new(
                                beat,
                                f64::from(midi),
                                len,
                                ((0.75 + 0.25 * intensity) * accent - jitter) * swell,
                            ),
                        )?;
                        hit_n += 1;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn line_pitch(&self, beat: f64, b: f64, d: i32, shift: i32) -> i32 {
        let frame = self.frame;
        let octave = self.role.octave;
        let c = frame.line_tones(beat);
        let lift = frame.specs[frame.section_at(beat)].lift;
        if frame.style.melody.chord_tones {
            let n = c.len() as i32;
            let root = root_voicing(c, 0);
            let m = root[d.rem_euclid(n) as usize] + 12 * (d.div_euclid(n) + octave);
            if shift == 0 {
                return m;
            }
            let m = frame.key.pitch(frame.key.index(m - lift) + shift) + lift;
            if b.fract() == 0.0 {
                nearest_toward(m, c, shift)
            } else {
                m
            }
        } else {
            let m = frame.key.pitch(d + shift) + 12 * octave + lift;
            if b.fract() == 0.0 {
                let shifted: Vec<i32> = c.iter().map(|x| x + 12 * octave).collect();
                nearest_tone(m, &shifted)
            } else {
                m
            }
        }
    }

    fn develop(&mut self, beats: f64, start: f64) -> Vec<(f64, i32, f64)> {
        let frame = self.frame;
        let style = frame.style;
        let m = &style.melody;
        let octave = self.role.octave;
        let step_of = |part: &Self, b: f64, d: i32| {
            let beat = start + b;
            let lift = frame.specs[frame.section_at(beat)].lift;
            frame.key.index(part.line_pitch(beat, b, d, 0) - lift)
        };
        if self.theme.is_empty() {
            let mut degree = 0;
            self.theme = motif(
                style,
                &frame.key,
                &mut self.seed.fork(0),
                beats,
                &mut degree,
                octave,
            );
            if m.chord_tones {
                self.contour = self
                    .theme
                    .iter()
                    .map(|&(b, d, _)| step_of(self, b, d))
                    .collect();
            }
        }
        let plan = m.develop[self.slot % m.develop.len()];
        let mut rng = self.seed.fork(1 + self.slot as u64);
        self.slot += 1;
        let line: Vec<(f64, i32, f64)> = if m.chord_tones {
            self.theme
                .iter()
                .zip(&self.contour)
                .map(|(&(b, _, l), &i)| (b, i, l))
                .collect()
        } else {
            self.theme.clone()
        };
        let axis = line.iter().map(|n| n.1).min().unwrap_or(0)
            + line.iter().map(|n| n.1).max().unwrap_or(0);
        let level = frame.intensity(start);
        let reaches = |at: Option<f64>| at.is_some_and(|a| level >= a - 1e-9);
        let fragments = plan == Develop::Sequence && reaches(m.fragment);
        let mut shift = 0;
        let out: Vec<(f64, i32, f64)> = match plan {
            Develop::Sequence => {
                let leaves = |s: i32, direction: i32| {
                    let heard = if fragments {
                        fragment(&line, beats, m.sequence * direction)
                    } else {
                        line.clone()
                    };
                    self.realize(&heard, start, s)
                        .iter()
                        .any(|&(_, p, _)| p < m.range[0] || p > m.range[1])
                };
                let mut next = self.sequence + m.sequence * self.direction;
                if leaves(next, self.direction) {
                    self.direction = -self.direction;
                    next = self.sequence + m.sequence * self.direction;
                }
                self.sequence = next;
                shift = next;
                line
            }
            Develop::Invert => line.iter().map(|&(b, d, l)| (b, axis - d, l)).collect(),
            Develop::Vary => {
                let half = beats / 2.0;
                let mut kept: Vec<(f64, i32, f64)> =
                    line.iter().filter(|n| n.0 < half - 1e-9).copied().collect();
                let first = self.theme.first().map_or(0, |n| n.1);
                let mut degree = self
                    .theme
                    .iter()
                    .rfind(|n| n.0 < half - 1e-9)
                    .map_or(first, |n| n.1);
                let tail = motif(
                    style,
                    &frame.key,
                    &mut rng,
                    beats - half,
                    &mut degree,
                    octave,
                );
                for (b, d, l) in tail {
                    let b = b + half;
                    let d = if m.chord_tones {
                        step_of(self, b, d)
                    } else {
                        d
                    };
                    kept.push((b, d, l));
                }
                ring(&mut kept, beats);
                kept
            }
            Develop::State | Develop::Repeat | Develop::Return => line,
        };
        let out = if fragments {
            fragment(&out, beats, m.sequence * self.direction)
        } else {
            out
        };
        let out = if reaches(m.split) {
            split(&out, beats)
        } else {
            out
        };
        if plan != Develop::Sequence {
            self.sequence = 0;
            self.direction = 1;
        }
        let notes = self.realize(&out, start, shift);
        self.register(notes, start)
    }

    fn realize(&self, notes: &[(f64, i32, f64)], start: f64, shift: i32) -> Vec<(f64, i32, f64)> {
        let frame = self.frame;
        if !frame.style.melody.chord_tones {
            return notes
                .iter()
                .map(|&(b, d, l)| (b, self.line_pitch(start + b, b, d, shift), l))
                .collect();
        }
        let key = &frame.key;
        let mut prev: Option<i32> = None;
        notes
            .iter()
            .map(|&(b, i, l)| {
                let beat = start + b;
                let lift = frame.specs[frame.section_at(beat)].lift;
                let c = frame.line_tones(beat);
                let heard = frame.voiced(beat);
                let at = i + shift;
                let p = key.pitch(at) + lift;
                let dir = if shift != 0 {
                    shift.signum()
                } else {
                    prev.map_or(1, |q| (p - q).signum())
                };
                let dir = if dir == 0 { 1 } else { dir };
                let strong = b.fract() == 0.0;
                let fits = |q: i32| {
                    if strong {
                        c.iter().any(|t| (q - t).rem_euclid(12) == 0)
                    } else {
                        !clashes(f64::from(q), heard, false)
                    }
                };
                let q = if fits(p) {
                    p
                } else {
                    [at + dir, at - dir]
                        .into_iter()
                        .map(|j| key.pitch(j) + lift)
                        .find(|q| fits(*q))
                        .unwrap_or_else(|| nearest_toward(p, c, dir))
                };
                prev = Some(q);
                (b, q, l)
            })
            .collect()
    }

    fn register(&self, notes: Vec<(f64, i32, f64)>, start: f64) -> Vec<(f64, i32, f64)> {
        let frame = self.frame;
        let [lo, hi] = frame.style.melody.range;
        let outside = |k: i32| -> i32 {
            notes
                .iter()
                .map(|&(_, p, _)| (lo - p - 12 * k).max(0) + (p + 12 * k - hi).max(0))
                .sum()
        };
        let mut best = (outside(0), 0);
        for k in [-1, 1, -2, 2] {
            let o = outside(k);
            if o < best.0 {
                best = (o, k);
            }
        }
        let k = best.1;
        notes
            .into_iter()
            .map(|(b, p, l)| {
                (
                    b,
                    self.inside(start + b, b.fract() == 0.0, p + 12 * k, [lo, hi]),
                    l,
                )
            })
            .collect()
    }

    fn bass_note(&mut self, ci: usize) -> i32 {
        let frame = self.frame;
        let role = self.role;
        let base = frame.key.root + 12 * role.octave;
        let near = |tone: i32, to: i32| {
            let mut m = tone + 12 * (to - tone).div_euclid(12);
            if (m + 12 - to).abs() < (m - to).abs() {
                m += 12;
            }
            while m > base + 11 {
                m -= 12;
            }
            while m < base - 1 {
                m += 12;
            }
            m
        };
        while self.bass.len() <= ci {
            let k = self.bass.len();
            let c = &frame.chords[k].tones;
            let root = c[0] + 12 * role.octave;
            let note = match self.bass.last() {
                None => root,
                Some(&prev) => {
                    let home = near(c[0], prev);
                    let mut rng = self.seed.fork((1 << 32) | k as u64);
                    let cadence = frame.chords[k].beat >= frame.cadence - 1e-9;
                    if k + 1 < frame.chords.len()
                        && c.len() >= 3
                        && !cadence
                        && rng.next_f64() < role.inversions
                    {
                        let third = near(c[1], prev);
                        let fifth = near(c[2], prev);
                        let after = near(frame.chords[k + 1].tones[0], fifth);
                        if (1..=4).contains(&(third - prev).abs()) {
                            third
                        } else if (1..=2).contains(&(fifth - prev).abs())
                            && (1..=2).contains(&(after - fifth).abs())
                        {
                            fifth
                        } else {
                            home
                        }
                    } else {
                        home
                    }
                }
            };
            self.bass.push(note);
        }
        self.bass[ci]
    }

    fn walking(&mut self, beat: f64, next: f64, hit: usize, rng: &mut Rng) -> i32 {
        let frame = self.frame;
        let role = self.role;
        let ci = frame.chord_index(beat);
        let here = self.bass_note(ci);
        let first = self.last_chord != ci;
        self.last_chord = ci;
        let cn = frame.chord_index(next);
        if cn != ci && !first && rng.next_f64() < role.walk {
            let target = self.bass_note(cn);
            let lift = frame.specs[frame.section_at(next)].lift;
            let at = frame.key.index(target - lift);
            let below = frame.key.pitch(at - 1) + lift;
            let above = frame.key.pitch(at + 1) + lift;
            let (pick, other) = if below == here
                || (above != here && (above - here).abs() < (below - here).abs())
            {
                (above, below)
            } else {
                (below, above)
            };
            let now = frame.voiced(beat);
            let fits = |m: i32| m != here && !clashes(f64::from(m), now, false);
            if fits(pick) {
                return pick;
            }
            if fits(other) {
                return other;
            }
            let lead = nearest_tone(target, now);
            return if lead != target && (lead - target).abs() <= 2 {
                lead
            } else {
                here
            };
        }
        if first {
            return here;
        }
        let semis = if role.semis.is_empty() {
            0
        } else {
            role.semis[hit % role.semis.len()]
        };
        nearest_tone(here + semis, frame.voiced(beat))
    }

    fn sounding(&self, beat: f64) -> Option<&Note> {
        self.leader
            .iter()
            .rev()
            .find(|n| n.beat <= beat + 1e-9 && n.beat + n.beats > beat + 1e-9)
    }

    fn range(&self) -> [i32; 2] {
        let [lo, hi] = self.role.range.unwrap_or([55, 72]);
        [lo + 12 * self.role.octave, hi + 12 * self.role.octave]
    }

    fn counter(&mut self, si: usize, entry: u32, rng: &mut Rng) -> Result<(), String> {
        let frame = self.frame;
        let role = self.role;
        let sec = &frame.sections[si];
        let bpb = frame.bpb;
        let sec_start = f64::from(sec.from) * bpb;
        let sec_beats = f64::from(sec.bars) * bpb;
        let [i0, i1] = sec.intensity.map(f64::from);
        let [lo, hi] = self.range();
        let mut prev: Option<(i32, Option<i32>)> = self
            .notes
            .last()
            .map(|n| (n.midi as i32, self.sounding(n.beat).map(|l| l.midi as i32)));
        let mut back = self
            .notes
            .len()
            .checked_sub(2)
            .map(|i| self.notes[i].midi as i32);
        for bar in entry..sec.from + sec.bars {
            let bar_beat = f64::from(bar) * bpb;
            let t = (bar_beat - sec_start) / sec_beats;
            let intensity = i0 + (i1 - i0) * t;
            let swell = 1.0 + f64::from(role.ramp) * (t - 0.5);
            let pat = if role.patterns.is_empty() && role.euclid.is_empty() {
                vec![true, false]
            } else {
                pattern(role, intensity, 0)
            };
            let step = bpb / pat.len() as f64;
            let hits: Vec<usize> = (0..pat.len()).filter(|i| pat[*i]).collect();
            for (k, &i) in hits.iter().enumerate() {
                let beat = bar_beat + i as f64 * step;
                let until = hits.get(k + 1).map_or(bpb, |j| *j as f64 * step);
                let len = if role.hold > 0.0 {
                    role.hold
                } else {
                    until - i as f64 * step
                };
                let c = frame.line_tones(beat);
                let strong = beat.fract() == 0.0;
                let lift = frame.specs[frame.section_at(beat)].lift;
                let lead = self.sounding(beat).map(|n| n.midi.round() as i32);
                let mut candidates: Vec<(i32, bool)> = (lo..=hi)
                    .filter(|m| c.iter().any(|t| (m - t).rem_euclid(12) == 0))
                    .map(|m| (m, true))
                    .collect();
                if !strong {
                    candidates.extend(
                        (lo..=hi)
                            .filter(|m| frame.key.pitch(frame.key.index(m - lift)) + lift == *m)
                            .filter(|m| c.iter().all(|t| (m - t).rem_euclid(12) != 0))
                            .map(|m| (m, false)),
                    );
                }
                if let Some(l) = lead
                    && candidates.iter().any(|(m, _)| l - m >= 3)
                {
                    candidates.retain(|(m, _)| l - m >= 3);
                }
                let mut best = (f64::MAX, None);
                for (m, tone) in candidates {
                    let cost =
                        counter_cost(m, tone, strong, lead, prev, back) + rng.next_f64() * 0.05;
                    if cost < best.0 {
                        best = (cost, Some(m));
                    }
                }
                let Some(m) = best.1 else {
                    continue;
                };
                self.push(
                    si,
                    Note::new(beat, f64::from(m), len, (0.5 + 0.3 * intensity) * swell),
                )?;
                back = prev.map(|p| p.0);
                prev = Some((m, lead));
            }
        }
        Ok(())
    }

    fn answer(&mut self, si: usize, entry: u32) -> Result<(), String> {
        let frame = self.frame;
        let role = self.role;
        let sec = &frame.sections[si];
        let bpb = frame.bpb;
        let sec_start = f64::from(sec.from) * bpb;
        let sec_beats = f64::from(sec.bars) * bpb;
        let from = f64::from(entry) * bpb;
        let end = f64::from(sec.from + sec.bars) * bpb;
        let [i0, i1] = sec.intensity.map(f64::from);
        let [lo, hi] = self.range();
        let onsets: Vec<f64> = self.leader.iter().map(|n| n.beat).collect();
        let statement = f64::from(frame.style.melody.motif_bars.max(1)) * bpb;
        let origin = role
            .follows
            .as_deref()
            .and_then(|name| frame.style.roles.iter().find(|r| r.name == name))
            .and_then(|leader| entry_bar(frame, leader, si))
            .map_or(sec_start, |bar| f64::from(bar) * bpb);
        let mut group: Option<i64> = None;
        let mut start = self.notes.len();
        for l in self.leader {
            let beat = l.beat + role.delay;
            if beat < from - 1e-9 || beat >= end - 1e-9 {
                continue;
            }
            if onsets.iter().any(|o| (o - beat).abs() < 0.2) {
                continue;
            }
            let g = ((l.beat - origin) / statement + 1e-9).floor() as i64;
            if group != Some(g) {
                self.place(start, [lo, hi]);
                start = self.notes.len();
                group = Some(g);
            }
            let c = frame.line_tones(beat);
            let lift = frame.specs[frame.section_at(beat)].lift;
            let at = frame
                .key
                .index(l.midi.round() as i32 + 12 * role.octave - lift)
                + role.interval;
            let mut m = frame.key.pitch(at) + lift;
            let heard = frame.voiced(beat);
            if beat.fract() == 0.0 {
                m = nearest_tone(m, c);
            } else if clashes(f64::from(m), heard, false) {
                m = [at - 1, at + 1]
                    .into_iter()
                    .map(|j| frame.key.pitch(j) + lift)
                    .find(|q| !clashes(f64::from(*q), heard, false))
                    .unwrap_or_else(|| nearest_tone(m, c));
            }
            if let Some(n) = self.sounding(beat)
                && (n.midi.round() as i32 - m).rem_euclid(12) == 0
            {
                m = nearest_tone(m - 3, c);
            }
            let next = onsets
                .iter()
                .filter(|o| **o > beat + 1e-9)
                .fold(end, |a, b| a.min(*b));
            let t = (beat - sec_start) / sec_beats;
            let intensity = i0 + (i1 - i0) * t;
            let swell = 1.0 + f64::from(role.ramp) * (t - 0.5);
            self.push(
                si,
                Note::new(
                    beat,
                    f64::from(m),
                    l.beats.min(next - beat).max(0.25),
                    (0.45 + 0.3 * intensity) * swell,
                ),
            )?;
        }
        self.place(start, [lo, hi]);
        Ok(())
    }

    fn place(&mut self, from: usize, [lo, hi]: [i32; 2]) {
        let notes = &self.notes[from.min(self.notes.len())..];
        let outside = |k: i32| -> i32 {
            notes
                .iter()
                .map(|n| {
                    let p = n.midi.round() as i32 + 12 * k;
                    (lo - p).max(0) + (p - hi).max(0)
                })
                .sum()
        };
        let mut best = (outside(0), 0);
        for k in [-1, 1, -2, 2, -3, 3, -4, 4] {
            let o = outside(k);
            if o < best.0 {
                best = (o, k);
            }
        }
        let k = best.1;
        for i in from..self.notes.len() {
            let n = &self.notes[i];
            let q = n.midi.round() as i32 + 12 * k;
            let m = self.inside(n.beat, n.beat.fract() == 0.0, q, [lo, hi]);
            self.notes[i].midi = f64::from(m);
        }
    }

    fn inside(&self, beat: f64, strong: bool, q: i32, [lo, hi]: [i32; 2]) -> i32 {
        if (lo..=hi).contains(&q) {
            return q;
        }
        let frame = self.frame;
        let lift = frame.specs[frame.section_at(beat)].lift;
        let c = frame.line_tones(beat);
        let heard = frame.voiced(beat);
        let fits = |x: &i32| {
            if strong {
                c.iter().any(|t| (x - t).rem_euclid(12) == 0)
            } else {
                frame.key.pitch(frame.key.index(x - lift)) + lift == *x
                    && !clashes(f64::from(*x), heard, false)
            }
        };
        let lead = self.sounding(beat).map(|n| n.midi.round() as i32);
        let apart = |x: &i32| lead.is_none_or(|p| (x - p).rem_euclid(12) != 0);
        (lo..=hi)
            .filter(|x| fits(x) && apart(x))
            .min_by_key(|x| (x - q).abs())
            .or_else(|| (lo..=hi).filter(fits).min_by_key(|x| (x - q).abs()))
            .unwrap_or_else(|| fold(q, [lo, hi]))
    }

    fn last_bar(&self) -> Option<f64> {
        let last = self.frame.sections.last()?;
        Some(f64::from(last.from + last.bars).max(1.0) * self.frame.bpb - self.frame.bpb)
    }

    fn land(&mut self) {
        let frame = self.frame;
        let Some(last_bar) = self.last_bar() else {
            return;
        };
        let Some(last) = frame.chords.last() else {
            return;
        };
        let bar = last_bar.max(last.beat);
        let end = last_bar + frame.bpb;
        let (sec, spec) = (
            &frame.sections[frame.sections.len() - 1],
            frame.specs[frame.specs.len() - 1],
        );
        let until = if spec.stop > 0.0 && !spec.through.contains(&self.role.name) {
            (end - spec.stop).max(f64::from(sec.from) * frame.bpb)
        } else {
            end
        };
        if until - bar <= 1e-9 {
            return;
        }
        let Some(first) = self
            .notes
            .iter()
            .filter(|n| n.beat >= bar - 1e-9)
            .min_by(|a, b| a.beat.total_cmp(&b.beat))
            .cloned()
        else {
            return;
        };
        let root = last.tones[0];
        let midi = first.midi.round() as i32;
        let vel = self
            .notes
            .iter()
            .filter(|n| n.beat >= bar - 1e-9)
            .fold(0.0f64, |v, n| v.max(n.vel));
        self.notes.retain(|n| n.beat < bar - 1e-9);
        for n in &mut self.notes {
            if n.beat + n.beats > bar {
                n.beats = bar - n.beat;
            }
        }
        let target = if self.role.plays == Plays::Bass {
            let m = nearest_tone(midi, &[root]);
            if self.role.range.is_some() {
                fold(m, self.range())
            } else {
                m
            }
        } else {
            self.cadence(bar, root, midi)
        };
        self.notes.push(Note {
            beat: bar,
            midi: f64::from(target),
            beats: until - bar,
            vel,
            ..first
        });
    }

    fn cadence(&mut self, bar: f64, root: i32, near: i32) -> i32 {
        let frame = self.frame;
        let key = &frame.key;
        let range = if self.role.plays == Plays::Melody {
            frame.style.melody.range
        } else {
            self.range()
        };
        let finals: Vec<i32> = (range[0]..=range[1])
            .filter(|m| (m - root).rem_euclid(12) == 0)
            .collect();
        let mut order: Vec<usize> = (0..self.notes.len()).collect();
        order.sort_by(|a, b| self.notes[*a].beat.total_cmp(&self.notes[*b].beat));
        let Some(&at) = order.last() else {
            return nearest_tone(near, &[root]);
        };
        if finals.is_empty() {
            return nearest_tone(near, &[root]);
        }
        let pen = self.notes[at].midi.round() as i32;
        let pen_beat = self.notes[at].beat;
        let ante = order
            .len()
            .checked_sub(2)
            .map(|i| self.notes[order[i]].midi.round() as i32);
        let lift = frame.specs[frame.section_at(bar)].lift;
        let step = |m: i32| key.index(m - lift);
        let lead = |beat: f64| self.sounding(beat).map(|n| n.midi.round() as i32);
        let (lead_final, lead_pen) = (lead(bar), lead(pen_beat));
        let pen_chord = frame.voiced(pen_beat);
        let mut best = (f64::MAX, finals[0], pen);
        for &f in &finals {
            let up = key.pitch(step(f) + 1) + lift;
            let down = key.pitch(step(f) - 1) + lift;
            for p in [pen, up, down] {
                if p != pen && (p < range[0] || p > range[1]) {
                    continue;
                }
                let mut cost = if (step(f) - step(p)).abs() == 1 {
                    -1.0
                } else {
                    0.7
                };
                if let (Some(lf), Some(lp)) = (lead_final, lead_pen) {
                    let (dl, dc) = ((lf - lp).signum(), (f - p).signum());
                    cost += if dl != 0 && dc == -dl { -1.0 } else { 0.1 };
                    if lf - f < 3 {
                        cost += 2.0;
                    }
                    let (was, now) = ((lp - p).rem_euclid(12), (lf - f).rem_euclid(12));
                    if (now == 0 || now == 7) && now == was && p != f && lp != lf {
                        cost += 0.5;
                    }
                }
                if let Some(a) = ante {
                    cost += if a == p {
                        0.1
                    } else if (step(p) - step(a)).abs() < 2 {
                        -1.0
                    } else {
                        0.7
                    };
                }
                if let Some(lp) = lead_pen {
                    if lp - p < 3 {
                        cost += 2.0;
                    }
                    if !matches!((lp - p).rem_euclid(12), 0 | 3 | 4 | 7 | 8 | 9) {
                        cost += 1.0;
                    }
                }
                if p != pen {
                    cost += 0.5;
                    if pen_chord.iter().all(|t| (p - t).rem_euclid(12) != 0) {
                        cost += 1.0;
                    }
                }
                cost += 0.01 * f64::from((f - near).abs());
                if cost < best.0 - 1e-9 {
                    best = (cost, f, p);
                }
            }
        }
        self.notes[at].midi += f64::from(best.2 - pen);
        best.1
    }

    fn passing(&mut self) -> Result<(), String> {
        let frame = self.frame;
        let m = &frame.style.melody;
        if self.role.plays != Plays::Melody || !m.chord_tones || m.passing <= 0.0 {
            return Ok(());
        }
        let key = &frame.key;
        let mut rng = self.seed.fork(2 << 32);
        self.notes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        let mut added = Vec::new();
        for k in 1..self.notes.len() {
            let (a, b) = (&self.notes[k - 1], &self.notes[k]);
            let mid = (a.beat + b.beat) / 2.0;
            let gap = b.beat - a.beat;
            if !(1.0 - 1e-9..=2.0 + 1e-9).contains(&gap)
                || (mid.fract() - 0.5).abs() > 1e-9
                || a.beat + a.beats < mid - 0.05
                || frame.section_at(a.beat) != frame.section_at(b.beat)
            {
                continue;
            }
            let lift = frame.specs[frame.section_at(mid)].lift;
            let (pa, pb) = (a.midi.round() as i32, b.midi.round() as i32);
            let (ia, ib) = (key.index(pa - lift), key.index(pb - lift));
            if (ib - ia).abs() != 2
                || key.pitch(ia) + lift != pa
                || key.pitch(ib) + lift != pb
                || rng.next_f64() >= m.passing
            {
                continue;
            }
            let p = key.pitch((ia + ib) / 2) + lift;
            if clashes(f64::from(p), frame.voiced(mid), false) {
                continue;
            }
            if self.notes.len() + added.len() >= self.room {
                return Err(self.full(frame.section_at(mid)));
            }
            added.push((
                k - 1,
                Note::new(mid, f64::from(p), (b.beat - mid) * 0.9, a.vel * 0.85),
            ));
        }
        for (k, note) in &added {
            let a = &mut self.notes[*k];
            a.beats = a.beats.min(note.beat - a.beat);
        }
        self.notes.extend(added.into_iter().map(|(_, n)| n));
        self.notes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        Ok(())
    }

    fn trim(&mut self) {
        let frame = self.frame;
        let role = self.role;
        let plays = role.plays;
        if plays == Plays::Chord {
            self.notes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            let onsets: Vec<f64> = self.notes.iter().map(|n| n.beat).collect();
            let mut next = 0;
            for note in &mut self.notes {
                while next < onsets.len() && onsets[next] <= note.beat + 1e-9 {
                    next += 1;
                }
                if let Some(at) = onsets.get(next) {
                    note.beats = note.beats.min(at - note.beat);
                }
                if let Some(change) = frame.chords.get(frame.chord_index(note.beat) + 1) {
                    note.beats = note.beats.min(change.beat - note.beat);
                }
            }
        }
        if !plays.event() {
            for (sec, spec) in frame.sections.iter().zip(&frame.specs) {
                if spec.stop <= 0.0 || sec.bars == 0 || spec.through.contains(&role.name) {
                    continue;
                }
                let end = f64::from(sec.from + sec.bars) * frame.bpb;
                let from = (end - spec.stop).max(f64::from(sec.from) * frame.bpb);
                self.notes
                    .retain(|n| n.beat < from - 1e-9 || n.beat >= end - 1e-9);
                for n in &mut self.notes {
                    if n.beat < from && n.beat + n.beats > from + 1e-9 {
                        n.beats = from - n.beat;
                    }
                }
            }
        }
        if !plays.event() && plays != Plays::Hit {
            let every = plays == Plays::Chord;
            for note in &mut self.notes {
                for k in 1..frame.chords.len() {
                    let at = frame.chords[k].beat;
                    if (every || frame.split(k))
                        && note.beat < at - 1e-9
                        && note.beat + note.beats > at + 1e-9
                        && clashes(note.midi, frame.voiced(at), every)
                    {
                        note.beats = at - note.beat;
                        break;
                    }
                }
            }
        }
        let eighth = |b: f64| (b * 2.0 + 1e-6).floor() as i64;
        self.notes.retain(|n| {
            let s = i64::from(frame.specs[frame.section_at(n.beat)].stutter);
            s == 0 || eighth(n.beat).rem_euclid(s) != s - 1
        });
        if !frame.style.harmony.land {
            return;
        }
        match plays {
            Plays::Melody | Plays::Counter | Plays::Bass => self.land(),
            Plays::Answer => {
                if let Some(bar) = self.last_bar() {
                    self.notes.retain(|n| n.beat < bar - 1e-9);
                    for n in &mut self.notes {
                        if n.beat + n.beats > bar {
                            n.beats = bar - n.beat;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(mut self, humanize: &mut [Rng]) -> Track {
        let frame = self.frame;
        let plays = self.role.plays;
        for note in &mut self.notes {
            let spec = frame.specs[frame.section_at(note.beat)];
            if spec.pitch != 1.0 {
                note.midi += 12.0 * log2(spec.pitch);
            }
            while note.midi + note.bend.max(0.0) > 127.0 && note.midi >= 12.0 {
                note.midi -= 12.0;
            }
            while note.midi + note.bend.min(0.0) < 0.0 && note.midi <= 115.0 {
                note.midi += 12.0;
            }
            if !matches!(plays, Plays::Riser | Plays::Roll | Plays::Reverse) {
                note.vel *= pow(10.0, f64::from(spec.gain_db) / 20.0);
            }
        }
        let style = frame.style;
        let homes: Vec<usize> = self
            .notes
            .iter()
            .map(|n| frame.section_at(n.beat))
            .collect();
        if style.swing > 0.0 {
            for note in &mut self.notes {
                let e = note.beat * 2.0;
                if (e - e.round()).abs() < 1e-6 && (e.round() as i64) % 2 == 1 {
                    note.beat += style.swing * 0.5;
                }
            }
        }
        let total = frame
            .sections
            .last()
            .map_or(0.0, |s| f64::from(s.from + s.bars) * frame.bpb);
        if style.humanize_ms > 0.0 {
            for (note, &si) in self.notes.iter_mut().zip(&homes) {
                let beat_ms = 60_000.0 / (f64::from(frame.bpm) * frame.specs[si].tempo);
                let jitter = (humanize[si].next_f64() - 0.5) * style.humanize_ms / beat_ms;
                note.beat = (note.beat + jitter).max(0.0);
            }
        }
        for note in &mut self.notes {
            note.beat = note.beat.min(total);
        }
        for note in &mut self.notes {
            note.beat = round(note.beat, 1e6);
            note.midi = round(note.midi, 1e4);
            note.beats = round(note.beats, 1e6);
            note.vel = round(note.vel.max(0.0), 1e4);
            note.bend = round(note.bend, 1e4);
            note.fade = round(note.fade, 1e4);
        }
        self.notes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        Track {
            role: self.role.name.clone(),
            notes: self.notes,
            lowpass: self.lowpass,
        }
    }
}

fn stinger(cue: &Cue, key: &Key) -> Result<ScoreCue, String> {
    let mut voices = Vec::with_capacity(cue.voices.len());
    let mut length: f64 = 0.0;
    for voice in &cue.voices {
        let mut notes = Vec::new();
        let at = voice.at;
        match voice.plays {
            CuePlays::Hit => {
                notes.push(Note::new(at, voice.midi.unwrap_or(60.0), voice.beats, 1.0))
            }
            CuePlays::Glide => {
                let [a, b] = voice.hz.unwrap_or([440.0, 440.0]);
                let from = midi_of(a);
                notes.push(Note {
                    bend: midi_of(b) - from,
                    fade: voice.fade,
                    ..Note::new(at, from, voice.beats, 1.0)
                });
            }
            CuePlays::Burst => {
                for k in 0..voice.count {
                    let vel = if voice.vels.is_empty() {
                        1.0
                    } else {
                        voice.vels[k as usize % voice.vels.len()]
                    };
                    let beat = at + f64::from(k) * voice.every;
                    notes.push(Note::new(
                        beat,
                        voice.midi.unwrap_or(60.0),
                        voice.every,
                        vel,
                    ));
                }
            }
            CuePlays::Chord => {
                let numeral = voice.numeral.as_deref().unwrap_or("I");
                for m in key.chord(numeral, false)? {
                    notes.push(Note::new(
                        at,
                        f64::from(m + 12 * voice.octave),
                        voice.beats,
                        1.0,
                    ));
                }
            }
        }
        for note in &mut notes {
            note.beat = round(note.beat, 1e6);
            note.midi = round(note.midi, 1e4);
            note.beats = round(note.beats, 1e6);
            note.vel = round(note.vel.max(0.0), 1e4);
            note.bend = round(note.bend, 1e4);
            note.fade = round(note.fade, 1e4);
            length = length.max(note.beat + note.beats);
        }
        voices.push(CueTrack {
            patch: voice.patch.clone(),
            gain: voice.gain,
            notes,
        });
    }
    Ok(ScoreCue {
        name: cue.name.clone(),
        event: cue.event().to_string(),
        beats: round(length, 1e6),
        voices,
    })
}

pub fn compose(name: &str, style: &Style, request: &Request) -> Result<Score, String> {
    style.check()?;
    let planned = request.plan.as_deref().map(|p| style.plan(p)).transpose()?;
    let style = planned.as_ref().unwrap_or(style);
    let resolved = style.resolved();
    let style = resolved.as_ref().unwrap_or(style);
    if style.sections.is_empty() {
        return Err("every [[section]] is in a plan; compose one plan of the style".into());
    }
    request.check()?;
    if let Some(need) = style.sections.iter().filter_map(|s| s.hit).max()
        && !request.hits.is_empty()
        && request.hits.len() <= need
    {
        return Err(format!(
            "the style pins sections to {} hits but {} came; give every hit, or none to compose without game events",
            need + 1,
            request.hits.len()
        ));
    }
    let root = Rng::new(request.seed);
    let form = stream(&root, &["form"]);
    let (bpm, spans) = match request.bpm {
        Some(bpm) => (
            bpm,
            place(
                style,
                bpm,
                request.seconds,
                &request.hits,
                &mut form.clone(),
            ),
        ),
        None => choose(style, request, &form, &mut stream(&root, &["bpm"])),
    };
    let mut keys = stream(&root, &["key"]);
    let tonic = pick(&mut keys, &style.tonics);
    let mode = pick(&mut keys, &style.modes);
    let key = match &request.key {
        Some(text) => Key::parse(text, KEY_OCTAVE)?,
        None => Key::new(&tonic, &mode, KEY_OCTAVE)?,
    };
    let bpb = f64::from(style.beats_per_bar);
    let total: u32 = spans.iter().map(|(_, bars)| bars).sum();
    if total > MAX_BARS {
        return Err(format!(
            "the form runs to {total} bars at {bpm} bpm; at most {MAX_BARS} fit"
        ));
    }
    let mut sections: Vec<Placed> = spans
        .iter()
        .zip(&style.sections)
        .map(|((from, bars), spec)| Placed {
            name: spec.name.clone(),
            from: *from,
            bars: *bars,
            at: 0.0,
            tempo: spec.tempo,
            intensity: spec.intensity,
        })
        .collect();
    let tempo = Tempo::new(f64::from(bpm), style.beats_per_bar, &sections);
    for s in &mut sections {
        s.at = round(tempo.seconds(f64::from(s.from) * bpb), 1e6);
    }
    let mut frame = Frame {
        style,
        key,
        bpm,
        bpb,
        sections,
        specs: style.sections.iter().collect(),
        chords: Vec::new(),
        cadence: f64::MAX,
    };
    harmony(
        &mut frame,
        f64::from(total) * bpb,
        &stream(&root, &["harmony"]),
        &stream(&root, &["applied"]),
    )?;
    let cues = style
        .cues
        .iter()
        .map(|cue| stinger(cue, &frame.key))
        .collect::<Result<Vec<_>, _>>()?;
    let mut used: usize = cues
        .iter()
        .flat_map(|c| &c.voices)
        .map(|v| v.notes.len())
        .sum();
    let mut tracks: Vec<Option<Track>> = style.roles.iter().map(|_| None).collect();
    let mut raw: Vec<(&str, Vec<Note>)> = Vec::new();
    for followers in [false, true] {
        for (ri, role) in style.roles.iter().enumerate() {
            if role.follower() != followers {
                continue;
            }
            let lane = stream(&root, &["role", &role.name]);
            let mut humanize: Vec<Rng> = frame
                .specs
                .iter()
                .map(|s| stream(&lane, &["humanize", &s.name]))
                .collect();
            let leader = role
                .follows
                .as_deref()
                .and_then(|name| raw.iter().find(|(n, _)| *n == name))
                .map_or(&[][..], |(_, notes)| notes.as_slice());
            let mut part = Part {
                frame: &frame,
                role,
                room: MAX_NOTES.saturating_sub(used),
                notes: Vec::new(),
                lowpass: Vec::new(),
                voicing: Vec::new(),
                seed: stream(&lane, &["develop"]),
                theme: Vec::new(),
                contour: Vec::new(),
                slot: 0,
                sequence: 0,
                direction: 1,
                bass: Vec::new(),
                leader,
                last_chord: usize::MAX,
            };
            for (si, spec) in frame.specs.iter().enumerate() {
                if role.plays.event() {
                    part.events(si)?;
                }
                part.section(si, &mut stream(&lane, &["section", &spec.name]))?;
            }
            part.passing()?;
            part.trim();
            let notes = (!followers).then(|| part.notes.clone());
            let track = part.finish(&mut humanize);
            used += track.notes.len();
            tracks[ri] = Some(track);
            if let Some(notes) = notes {
                raw.push((role.name.as_str(), notes));
            }
        }
    }
    let tracks: Vec<Track> = tracks.into_iter().flatten().collect();
    let slot = style.harmony.beats;
    let split = frame
        .chords
        .iter()
        .enumerate()
        .any(|(i, c)| (c.beat - i as f64 * slot).abs() > 1e-9);
    let chord_beats = if split {
        frame.chords.iter().map(|c| round(c.beat, 1e6)).collect()
    } else {
        Vec::new()
    };
    let fires = style
        .sections
        .iter()
        .filter_map(|spec| {
            let cue = spec.cue.clone()?;
            let at = *request.hits.get(spec.hit?)?;
            Some(Fire { cue, at })
        })
        .collect();
    let score = Score {
        style: StyleRef::Name(name.to_string()),
        seed: request.seed,
        seconds: request.seconds,
        hits: request.hits.clone(),
        plan: request.plan.clone(),
        bpm,
        beats_per_bar: style.beats_per_bar,
        key: frame.key.name(),
        bars: total,
        chords: frame.chords.iter().map(|c| c.name.clone()).collect(),
        chord_beats,
        sections: frame.sections,
        tracks,
        cues,
        fires,
    };
    score.check()?;
    score.check_style(style)?;
    Ok(score)
}
