use pfx_core::sim::{Rng, ln, pow};

use crate::score::{CueTrack, Fire, Glide, Note, Placed, Score, ScoreCue, StyleRef, Tempo, Track};
use crate::style::{
    Cue, CuePlays, MAX_BARS, MAX_BPM, MAX_NOTE_BEATS, MAX_NOTES, MAX_SECONDS, Plays, Role, Section,
    Style, Voicing, midi_of,
};
use crate::theory::{Key, spread, voicing};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub seed: u64,
    pub seconds: f64,
    pub hits: Vec<f64>,
    pub bpm: Option<u32>,
    pub key: Option<String>,
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

struct Frame<'a> {
    style: &'a Style,
    key: Key,
    bpm: u32,
    bpb: f64,
    sections: Vec<Placed>,
    specs: Vec<&'a Section>,
    chords: Vec<(Vec<i32>, String)>,
}

impl Frame<'_> {
    fn section_at(&self, beat: f64) -> usize {
        let bar = (beat / self.bpb).floor().max(0.0) as u32;
        self.sections
            .partition_point(|s| s.from <= bar)
            .saturating_sub(1)
    }

    fn chord_index(&self, beat: f64) -> usize {
        ((beat / self.style.harmony.beats + 1e-9).floor().max(0.0) as usize)
            .min(self.chords.len() - 1)
    }

    fn chord_at(&self, beat: f64) -> &[i32] {
        &self.chords[self.chord_index(beat)].0
    }
}

fn harmony(frame: &mut Frame, total_beats: f64, base: &Rng) -> Result<(), String> {
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
    for i in 0..steps {
        let from_end = steps - i;
        if from_end <= h.cadence.len() {
            current = h.cadence[h.cadence.len() - from_end].clone();
        }
        let si = frame.section_at(i as f64 * h.beats);
        let rng = &mut rngs[si];
        let lift = frame.specs[si].lift;
        let chord: Vec<i32> = frame
            .key
            .chord(&current, h.sevenths)?
            .iter()
            .map(|m| m + lift)
            .collect();
        frame.chords.push((chord, current.clone()));
        if let Some(next) = h.markov.get(&current) {
            current = pick(rng, next);
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
}

impl Part<'_> {
    fn push(&mut self, si: usize, note: Note) -> Result<(), String> {
        if self.notes.len() >= self.room {
            return Err(format!(
                "[[role]] `{}` in section `{}` takes the score past {MAX_NOTES} notes; shorten the music or thin its roles",
                self.role.name, self.frame.specs[si].name
            ));
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
            let moved = change as f64 * step;
            let moved = (change < frame.chords.len() && moved < end - 1e-9).then_some(moved);
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
            let voicing = voice(role, &self.voicing, &frame.chords[chord].0);
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
                        let voicing = voice(role, &self.voicing, frame.chord_at(beat));
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
                    if in_motif == 0 {
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
                        let c = frame.chord_at(beat);
                        let midi = if frame.style.melody.chord_tones {
                            let n = c.len() as i32;
                            let root = root_voicing(c, 0);
                            fold(
                                root[d.rem_euclid(n) as usize]
                                    + 12 * (d.div_euclid(n) + role.octave),
                                frame.style.melody.range,
                            )
                        } else {
                            let lift = frame.specs[frame.section_at(beat)].lift;
                            let m = frame.key.pitch(d) + 12 * role.octave + lift;
                            let m = if b.fract() == 0.0 {
                                let shifted: Vec<i32> =
                                    c.iter().map(|x| x + 12 * role.octave).collect();
                                nearest_tone(m, &shifted)
                            } else {
                                m
                            };
                            fold(m, frame.style.melody.range)
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
                        let c = frame.chord_at(beat);
                        let accent = if b.fract() == 0.0 {
                            1.0
                        } else {
                            1.0 - f64::from(role.accent)
                        };
                        let jitter = rng.next_f64() * 0.08;
                        let midi = match role.plays {
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

    fn finish(mut self, humanize: &mut [Rng]) -> Track {
        let frame = self.frame;
        let plays = self.role.plays;
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
                let step = frame.style.harmony.beats;
                let change = frame.chord_index(note.beat) + 1;
                if change < frame.chords.len() {
                    note.beats = note.beats.min(change as f64 * step - note.beat);
                }
            }
        }
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
        let eighth = |b: f64| (b * 2.0 + 1e-6).floor() as i64;
        self.notes.retain(|n| {
            let s = i64::from(frame.specs[frame.section_at(n.beat)].stutter);
            s == 0 || eighth(n.beat).rem_euclid(s) != s - 1
        });
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
    };
    harmony(
        &mut frame,
        f64::from(total) * bpb,
        &stream(&root, &["harmony"]),
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
    let mut tracks = Vec::with_capacity(style.roles.len());
    for role in &style.roles {
        let lane = stream(&root, &["role", &role.name]);
        let mut humanize: Vec<Rng> = frame
            .specs
            .iter()
            .map(|s| stream(&lane, &["humanize", &s.name]))
            .collect();
        let mut part = Part {
            frame: &frame,
            role,
            room: MAX_NOTES.saturating_sub(used),
            notes: Vec::new(),
            lowpass: Vec::new(),
            voicing: Vec::new(),
        };
        for (si, spec) in frame.specs.iter().enumerate() {
            if role.plays.event() {
                part.events(si)?;
            }
            part.section(si, &mut stream(&lane, &["section", &spec.name]))?;
        }
        let track = part.finish(&mut humanize);
        used += track.notes.len();
        tracks.push(track);
    }
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
        bpm,
        beats_per_bar: style.beats_per_bar,
        key: frame.key.name(),
        bars: total,
        chords: frame.chords.iter().map(|c| c.1.clone()).collect(),
        sections: frame.sections,
        tracks,
        cues,
        fires,
    };
    score.check()?;
    score.check_style(style)?;
    Ok(score)
}
