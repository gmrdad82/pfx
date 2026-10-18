use std::sync::Arc;

use crate::clip::Clip;
use crate::sound::Rng;
use crate::synth::sine;

pub const CONTROL_FRAMES: u64 = 16;

const HELD_FRAMES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ease {
    #[default]
    Linear,
    In(i32),
    Out(i32),
    InOut(i32),
}

impl Ease {
    pub fn apply(self, x: f64) -> f64 {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        match self {
            Ease::Linear => x,
            Ease::In(power) => powi(x, power),
            Ease::Out(power) => 1.0 - powi(1.0 - x, power),
            Ease::InOut(power) => {
                let x = x * 2.0;
                if x < 1.0 {
                    0.5 * powi(x, power)
                } else {
                    0.5 * (1.0 - powi(2.0 - x, power)) + 0.5
                }
            }
        }
    }
}

fn powi(x: f64, power: i32) -> f64 {
    let mut out = 1.0;
    for _ in 0..power.clamp(1, 16) {
        out *= x;
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct StemGain {
    pub keys: Vec<(f32, f32)>,
    pub power: bool,
}

impl StemGain {
    pub fn constant(gain: f32) -> Self {
        Self {
            keys: vec![(0.0, gain)],
            power: false,
        }
    }

    pub fn above(threshold: f32, width: f32) -> Self {
        Self {
            keys: vec![(threshold, 0.0), (threshold + width.max(0.0), 1.0)],
            power: true,
        }
    }

    pub fn below(threshold: f32, width: f32) -> Self {
        Self {
            keys: vec![(threshold - width.max(0.0), 1.0), (threshold, 0.0)],
            power: true,
        }
    }

    pub fn gain(&self, intensity: f32) -> f32 {
        let Some(&(first_at, first)) = self.keys.first() else {
            return 1.0;
        };
        if intensity <= first_at {
            return first;
        }
        for pair in self.keys.windows(2) {
            let (a_at, a) = pair[0];
            let (b_at, b) = pair[1];
            if intensity > b_at {
                continue;
            }
            if b_at <= a_at {
                return b;
            }
            let t = f64::from((intensity - a_at) / (b_at - a_at));
            if !self.power {
                return a + (b - a) * t as f32;
            }
            return if b >= a {
                a + (b - a) * sine(t * 0.25)
            } else {
                b + (a - b) * sine(0.25 + t * 0.25)
            };
        }
        self.keys.last().map_or(1.0, |key| key.1)
    }

    fn check(&self, stem: &str) -> Result<(), String> {
        for pair in self.keys.windows(2) {
            if pair[1].0 < pair[0].0 {
                return Err(format!(
                    "stem `{stem}`: gain keys go backwards at intensity {}",
                    pair[1].0
                ));
            }
        }
        if self
            .keys
            .iter()
            .any(|(at, gain)| !at.is_finite() || !gain.is_finite() || *gain < 0.0)
        {
            return Err(format!(
                "stem `{stem}`: gain keys must be finite, gains not negative"
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Stem {
    pub name: String,
    pub clip: Clip,
    pub gain: StemGain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Repeat,
    Section(String),
    Stop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub name: String,
    pub variants: Vec<(u32, u32)>,
    pub next: Next,
    pub avoid_repeat: bool,
}

impl Section {
    pub fn new(name: &str, bars: (u32, u32)) -> Self {
        Self {
            name: name.to_string(),
            variants: vec![bars],
            next: Next::Repeat,
            avoid_repeat: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantize {
    Now,
    Beat,
    Bar,
    End,
}

#[derive(Clone, Debug)]
pub struct AdaptiveTrack {
    pub stems: Vec<Stem>,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub offset: f64,
    pub sections: Vec<Section>,
    pub start: Option<String>,
    pub intensity: f32,
    pub ease: Ease,
    pub transition: f32,
}

impl AdaptiveTrack {
    pub fn new(stems: Vec<Stem>, bpm: f64, beats_per_bar: u32) -> Self {
        Self {
            stems,
            bpm,
            beats_per_bar,
            offset: 0.0,
            sections: Vec::new(),
            start: None,
            intensity: 0.0,
            ease: Ease::Linear,
            transition: 0.0,
        }
    }

    pub fn frames(&self) -> u64 {
        self.stems.first().map_or(0, |stem| stem.clip.frames())
    }

    pub fn prepare(&self, rate: u32) -> Result<PreparedAdaptive, String> {
        let plan = self.plan()?;
        let stems = self
            .stems
            .iter()
            .map(|stem| (stem.clip.for_rate(rate), stem.gain.clone()))
            .collect();
        Ok(PreparedAdaptive {
            score: Arc::new(Score::build(stems, plan, rate)?),
        })
    }

    fn plan(&self) -> Result<Plan, String> {
        let first = self.stems.first().ok_or("an adaptive track needs a stem")?;
        let (rate, frames) = (first.clip.rate, first.clip.frames());
        if rate == 0 || frames == 0 {
            return Err(format!("stem `{}` is empty", first.name));
        }
        for stem in &self.stems {
            if stem.clip.rate != rate {
                return Err(format!(
                    "stem `{}` is at {} Hz, `{}` at {rate} Hz: every stem has one rate",
                    stem.name, stem.clip.rate, first.name
                ));
            }
            if stem.clip.frames() != frames {
                return Err(format!(
                    "stem `{}` is {} frames, `{}` {frames}: every stem has one length",
                    stem.name,
                    stem.clip.frames(),
                    first.name
                ));
            }
            stem.gain.check(&stem.name)?;
        }
        if !self.bpm.is_finite() || self.bpm <= 0.0 {
            return Err(format!("bpm must be above 0, not {}", self.bpm));
        }
        if self.beats_per_bar == 0 {
            return Err("beats_per_bar must be at least 1".into());
        }
        if !self.offset.is_finite() || self.offset < 0.0 {
            return Err(format!(
                "offset must be 0 or more seconds, not {}",
                self.offset
            ));
        }
        let seconds = frames as f64 / f64::from(rate);
        let mut sections = Vec::new();
        let named = |name: &str| self.sections.iter().position(|s| s.name == name);
        for (index, section) in self.sections.iter().enumerate() {
            if named(&section.name) != Some(index) {
                return Err(format!("section `{}` is named twice", section.name));
            }
            if section.variants.is_empty() {
                return Err(format!("section `{}` has no bars", section.name));
            }
            for &(from, to) in &section.variants {
                if to <= from {
                    return Err(format!(
                        "section `{}`: bars {from} to {to} are empty",
                        section.name
                    ));
                }
                let at =
                    self.offset + f64::from(from) * f64::from(self.beats_per_bar) * 60.0 / self.bpm;
                if at >= seconds {
                    return Err(format!(
                        "section `{}`: bar {from} starts past the stems' end",
                        section.name
                    ));
                }
            }
            let next = match &section.next {
                Next::Repeat => Flow::Repeat,
                Next::Stop => Flow::Stop,
                Next::Section(name) => Flow::Go(named(name).ok_or_else(|| {
                    format!("section `{}`: next `{name}` is no section", section.name)
                })?),
            };
            sections.push(PlanSection {
                name: section.name.clone(),
                bars: section.variants.clone(),
                next,
                avoid_repeat: section.avoid_repeat,
            });
        }
        if sections.is_empty() {
            sections.push(PlanSection {
                name: "main".into(),
                bars: Vec::new(),
                next: Flow::Repeat,
                avoid_repeat: false,
            });
        }
        let start = match &self.start {
            None => 0,
            Some(name) => sections
                .iter()
                .position(|section| section.name == *name)
                .ok_or_else(|| format!("start `{name}` is no section"))?,
        };
        Ok(Plan {
            bpm: self.bpm,
            beats_per_bar: self.beats_per_bar,
            offset: self.offset,
            sections,
            start,
            intensity: clamp_intensity(self.intensity),
            ease: self.ease,
            transition: self.transition,
        })
    }
}

#[derive(Clone, Debug)]
struct Plan {
    bpm: f64,
    beats_per_bar: u32,
    offset: f64,
    sections: Vec<PlanSection>,
    start: usize,
    intensity: f32,
    ease: Ease,
    transition: f32,
}

#[derive(Clone, Debug)]
struct PlanSection {
    name: String,
    bars: Vec<(u32, u32)>,
    next: Flow,
    avoid_repeat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Repeat,
    Go(usize),
    Stop,
}

#[derive(Debug)]
struct StemData {
    samples: Vec<f32>,
    channels: usize,
    gain: StemGain,
}

#[derive(Debug)]
struct Score {
    rate: u32,
    frames: u64,
    beat: f64,
    offset: f64,
    stems: Vec<StemData>,
    variants: Vec<Vec<(u64, u64)>>,
    plan: Plan,
    transition: u64,
}

impl Score {
    fn build(stems: Vec<(Clip, StemGain)>, plan: Plan, rate: u32) -> Result<Score, String> {
        let frames = stems
            .iter()
            .map(|(clip, _)| clip.frames())
            .min()
            .unwrap_or(0);
        if frames == 0 {
            return Err("the stems are empty at this rate".into());
        }
        let beat = 60.0 * f64::from(rate) / plan.bpm;
        let offset = plan.offset * f64::from(rate);
        let bar = |index: u32| -> u64 {
            let beats = f64::from(index) * f64::from(plan.beats_per_bar);
            ((offset + beats * beat).round() as u64).min(frames)
        };
        let variants = plan
            .sections
            .iter()
            .map(|section| {
                if section.bars.is_empty() {
                    return vec![(0, frames)];
                }
                section
                    .bars
                    .iter()
                    .map(|&(from, to)| {
                        let start = bar(from).min(frames - 1);
                        (start, bar(to).clamp(start + 1, frames))
                    })
                    .collect()
            })
            .collect();
        let transition = seconds_to_frames(plan.transition, rate);
        Ok(Score {
            rate,
            frames,
            beat,
            offset,
            stems: stems
                .into_iter()
                .map(|(clip, gain)| StemData {
                    channels: usize::from(clip.channels.max(1)),
                    samples: clip.samples,
                    gain,
                })
                .collect(),
            variants,
            plan,
            transition,
        })
    }

    fn at_rate(&self, rate: u32) -> Result<Score, String> {
        let stems = self
            .stems
            .iter()
            .map(|stem| {
                let clip = Clip {
                    rate: self.rate,
                    channels: stem.channels as u16,
                    samples: stem.samples.clone(),
                };
                (clip.for_rate(rate), stem.gain.clone())
            })
            .collect();
        Score::build(stems, self.plan.clone(), rate)
    }

    fn grid_frame(&self, beats: u64) -> u64 {
        (self.offset + beats as f64 * self.beat).round() as u64
    }

    fn next_grid(&self, cursor: u64, step: u64) -> u64 {
        let step = step.max(1);
        let span = self.beat * step as f64;
        let into = (cursor as f64 - self.offset) / span;
        let whole = if into > 0.0 { into as u64 } else { 0 };
        let mut k = whole.saturating_sub(1);
        loop {
            let frame = self.grid_frame(k * step);
            if frame >= cursor {
                return frame;
            }
            k += 1;
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedAdaptive {
    score: Arc<Score>,
}

impl PreparedAdaptive {
    pub fn rate(&self) -> u32 {
        self.score.rate
    }

    pub fn frames(&self) -> u64 {
        self.score.frames
    }

    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.score.plan.sections.iter().map(|s| s.name.as_str())
    }

    pub fn for_rate(&self, rate: u32) -> PreparedAdaptive {
        if self.score.rate == rate {
            return self.clone();
        }
        match self.score.at_rate(rate) {
            Ok(score) => PreparedAdaptive {
                score: Arc::new(score),
            },
            Err(_) => self.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    section: usize,
    at: u64,
}

#[derive(Clone, Copy, Debug)]
struct Fade {
    cursor: u64,
    done: u64,
}

#[derive(Clone, Copy, Debug)]
struct Ramp {
    from: f32,
    to: f32,
    start: u64,
    len: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Segment {
    g0: f32,
    g1: f32,
}

pub(crate) struct Adaptive {
    score: Arc<Score>,
    rng: Rng,
    tick: u64,
    cursor: u64,
    section: usize,
    variant: usize,
    end: u64,
    last: Vec<Option<usize>>,
    pending: Option<Pending>,
    fade: Option<Fade>,
    ramp: Ramp,
    seg_start: u64,
    seg_end: u64,
    gains: Vec<Segment>,
    stopped: bool,
    out: Vec<f32>,
    spare: Vec<f32>,
}

impl Adaptive {
    pub(crate) fn new(track: &PreparedAdaptive, seed: u64) -> Self {
        let score = Arc::clone(&track.score);
        let intensity = score.plan.intensity;
        let stems = score.stems.len();
        let sections = score.plan.sections.len();
        let mut adaptive = Self {
            rng: Rng::new(seed),
            tick: 0,
            cursor: 0,
            section: score.plan.start,
            variant: 0,
            end: 0,
            last: vec![None; sections],
            pending: None,
            fade: None,
            ramp: Ramp {
                from: intensity,
                to: intensity,
                start: 0,
                len: 0,
            },
            seg_start: 0,
            seg_end: 0,
            gains: vec![Segment::default(); stems],
            stopped: false,
            out: Vec::with_capacity(HELD_FRAMES * 2),
            spare: Vec::with_capacity(HELD_FRAMES * 2),
            score,
        };
        adaptive.enter(adaptive.section);
        adaptive.segment(0, true);
        adaptive
    }

    pub(crate) fn intensity(&self) -> f32 {
        self.intensity_at(self.tick)
    }

    pub(crate) fn section(&self) -> &str {
        &self.score.plan.sections[self.section].name
    }

    pub(crate) fn position(&self) -> u64 {
        self.cursor
    }

    pub(crate) fn finished(&self) -> bool {
        self.stopped && self.fade.is_none()
    }

    pub(crate) fn set_intensity(&mut self, value: f32, frames: u64) {
        let from = self.intensity_at(self.tick);
        self.ramp = Ramp {
            from,
            to: clamp_intensity(value),
            start: self.tick,
            len: frames,
        };
        self.segment(self.tick, false);
    }

    pub(crate) fn request(&mut self, name: &str, at: Quantize) -> bool {
        let Some(section) = self.score.plan.sections.iter().position(|s| s.name == name) else {
            return false;
        };
        if self.stopped {
            return false;
        }
        let score = &self.score;
        let bar = u64::from(score.plan.beats_per_bar);
        let frame = match at {
            Quantize::Now => self.cursor,
            Quantize::Beat => score.next_grid(self.cursor, 1),
            Quantize::Bar => score.next_grid(self.cursor, bar),
            Quantize::End => self.end,
        };
        self.pending = Some(Pending {
            section,
            at: frame.min(self.end),
        });
        true
    }

    pub(crate) fn frame(&self, index: usize) -> (f32, f32) {
        let base = index * 2;
        match self.out.get(base..base + 2) {
            Some(pair) => (pair[0], pair[1]),
            None => (0.0, 0.0),
        }
    }

    pub(crate) fn fill(&mut self, first: usize, frames: usize) {
        self.out.clear();
        self.out.resize(frames * 2, 0.0);
        let mut i = first.min(frames);
        while i < frames {
            if self.finished() {
                break;
            }
            if self.tick >= self.seg_end {
                self.segment(self.tick, true);
            }
            let mut span = (frames - i) as u64;
            span = span.min(self.seg_end - self.tick);
            if !self.stopped {
                if let Some(pending) = self.pending
                    && pending.at <= self.cursor
                {
                    self.pending = None;
                    self.jump(pending.section);
                    continue;
                }
                if self.cursor >= self.end {
                    self.advance();
                    continue;
                }
                span = span.min(self.end - self.cursor);
                if let Some(pending) = self.pending {
                    span = span.min(pending.at - self.cursor);
                }
            }
            if let Some(fade) = self.fade {
                span = span.min(self.score.transition - fade.done);
            }
            let span = span.max(1) as usize;
            self.render(i, span);
            let step = span as u64;
            self.tick += step;
            if !self.stopped {
                self.cursor += step;
            }
            if let Some(fade) = self.fade.as_mut() {
                fade.cursor += step;
                fade.done += step;
                if fade.done >= self.score.transition {
                    self.fade = None;
                }
            }
            i += span;
        }
    }

    fn render(&mut self, at: usize, span: usize) {
        let from = self.tick - self.seg_start;
        let len = (self.seg_end - self.seg_start).max(1) as f32;
        let region = at * 2..(at + span) * 2;
        if !self.stopped {
            mix_stems(
                &mut self.out[region.clone()],
                &self.score,
                &self.gains,
                self.cursor,
                from,
                len,
            );
        }
        let Some(fade) = self.fade else {
            return;
        };
        let total = self.score.transition.max(1) as f64;
        if !self.stopped {
            for (k, pair) in self.out[region.clone()].chunks_exact_mut(2).enumerate() {
                let g = sine((fade.done + k as u64) as f64 / total * 0.25);
                pair[0] *= g;
                pair[1] *= g;
            }
        }
        let mut spare = std::mem::take(&mut self.spare);
        spare.clear();
        spare.resize(span * 2, 0.0);
        mix_stems(&mut spare, &self.score, &self.gains, fade.cursor, from, len);
        for (k, (slot, pair)) in self.out[region]
            .chunks_exact_mut(2)
            .zip(spare.chunks_exact(2))
            .enumerate()
        {
            let g = sine(0.25 + (fade.done + k as u64) as f64 / total * 0.25);
            slot[0] += pair[0] * g;
            slot[1] += pair[1] * g;
        }
        self.spare = spare;
    }

    fn advance(&mut self) {
        match self.score.plan.sections[self.section].next {
            Flow::Repeat => self.jump(self.section),
            Flow::Go(next) => self.jump(next),
            Flow::Stop => {
                if self.score.transition > 0 {
                    self.fade = Some(Fade {
                        cursor: self.cursor,
                        done: 0,
                    });
                }
                self.stopped = true;
                self.pending = None;
            }
        }
    }

    fn jump(&mut self, section: usize) {
        let old = self.cursor;
        self.enter(section);
        if self.cursor != old && self.score.transition > 0 {
            self.fade = Some(Fade {
                cursor: old,
                done: 0,
            });
        }
    }

    fn enter(&mut self, section: usize) {
        let variants = &self.score.variants[section];
        let count = variants.len();
        let avoid = self.score.plan.sections[section].avoid_repeat;
        let last = self.last[section].filter(|_| avoid && count > 1);
        let choices = if last.is_some() { count - 1 } else { count };
        let mut pick = if choices > 1 {
            (((self.rng.seed() >> 32) * choices as u64) >> 32) as usize
        } else {
            0
        };
        if let Some(last) = last
            && pick >= last
        {
            pick += 1;
        }
        self.section = section;
        self.variant = pick;
        self.last[section] = Some(pick);
        let (start, end) = variants[pick];
        self.cursor = start;
        self.end = end;
    }

    fn intensity_at(&self, tick: u64) -> f32 {
        let ramp = self.ramp;
        if ramp.len == 0 || tick >= ramp.start + ramp.len {
            return ramp.to;
        }
        let t = (tick.saturating_sub(ramp.start)) as f64 / ramp.len as f64;
        let eased = self.score.plan.ease.apply(t);
        ramp.from + (ramp.to - ramp.from) * eased as f32
    }

    fn segment(&mut self, tick: u64, natural: bool) {
        let end = (tick / CONTROL_FRAMES + 1) * CONTROL_FRAMES;
        let target = self.intensity_at(end);
        let now = self.intensity_at(tick);
        let from = tick.saturating_sub(self.seg_start);
        let len = (self.seg_end - self.seg_start).max(1) as f32;
        for (segment, stem) in self.gains.iter_mut().zip(&self.score.stems) {
            let g0 = if natural {
                if self.seg_end == tick && tick > 0 {
                    segment.g1
                } else {
                    stem.gain.gain(now)
                }
            } else {
                segment.g0 + (segment.g1 - segment.g0) / len * from as f32
            };
            *segment = Segment {
                g0,
                g1: stem.gain.gain(target),
            };
        }
        self.seg_start = tick;
        self.seg_end = end;
    }
}

fn mix_stems(out: &mut [f32], score: &Score, gains: &[Segment], cursor: u64, from: u64, len: f32) {
    let span = out.len() / 2;
    let available = score.frames.saturating_sub(cursor).min(span as u64) as usize;
    if available == 0 {
        return;
    }
    let cursor = cursor as usize;
    for (stem, segment) in score.stems.iter().zip(gains) {
        let step = (segment.g1 - segment.g0) / len;
        if segment.g0 == 0.0 && segment.g1 == 0.0 {
            continue;
        }
        let channels = stem.channels;
        let right = usize::from(channels > 1);
        let frames = &stem.samples[cursor * channels..(cursor + available) * channels];
        let pairs = out[..available * 2].chunks_exact_mut(2);
        if step == 0.0 {
            let g = segment.g0;
            for (pair, frame) in pairs.zip(frames.chunks_exact(channels)) {
                pair[0] += frame[0] * g;
                pair[1] += frame[right] * g;
            }
        } else {
            for (k, (pair, frame)) in pairs.zip(frames.chunks_exact(channels)).enumerate() {
                let g = segment.g0 + step * (from + k as u64) as f32;
                pair[0] += frame[0] * g;
                pair[1] += frame[right] * g;
            }
        }
    }
}

fn clamp_intensity(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn seconds_to_frames(seconds: f32, rate: u32) -> u64 {
    if seconds.is_nan() || seconds <= 0.0 {
        return 0;
    }
    (f64::from(seconds) * f64::from(rate)).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mix::{Level, Mixer};

    const RATE: u32 = 1000;

    fn clip(frames: u64, value: impl Fn(u64) -> f32) -> Clip {
        Clip {
            rate: RATE,
            channels: 1,
            samples: (0..frames).map(value).collect(),
        }
    }

    fn stem(name: &str, clip: Clip, gain: StemGain) -> Stem {
        Stem {
            name: name.into(),
            clip,
            gain,
        }
    }

    fn index_track(frames: u64) -> AdaptiveTrack {
        let ramp = clip(frames, |k| k as f32 / frames as f32);
        AdaptiveTrack::new(vec![stem("ramp", ramp, StemGain::constant(1.0))], 600.0, 4)
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.iter().step_by(2).copied().collect()
    }

    fn frame_of(value: f32, frames: u64) -> u64 {
        (value * frames as f32).round() as u64
    }

    fn play(track: &AdaptiveTrack, seed: u64) -> Mixer {
        let mut mixer = Mixer::new(RATE, 2);
        mixer.set_seed(seed);
        let prepared = track.prepare(RATE).unwrap();
        mixer.play_adaptive(&prepared, 0.0, Level::default());
        mixer
    }

    #[test]
    fn a_stem_gain_follows_its_intensity_keys() {
        let drums = StemGain::above(0.3, 0.1);
        assert_eq!(drums.gain(0.0), 0.0);
        assert_eq!(drums.gain(0.3), 0.0);
        assert!((drums.gain(0.35) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert_eq!(drums.gain(0.4), 1.0);
        assert_eq!(drums.gain(1.0), 1.0);
        let calm = StemGain::below(0.4, 0.1);
        for step in 0..=100 {
            let at = step as f32 / 100.0;
            let power = drums.gain(at).powi(2) + calm.gain(at).powi(2);
            assert!((power - 1.0).abs() < 1e-4, "{at}: {power}");
        }
        let lead = StemGain {
            keys: vec![(0.7, 0.0), (0.9, 0.5)],
            power: false,
        };
        assert!((lead.gain(0.8) - 0.25).abs() < 1e-6);
        assert_eq!(lead.gain(1.0), 0.5);
        assert_eq!(Ease::InOut(2).apply(0.25), 0.125);
        assert_eq!(Ease::Out(2).apply(0.5), 0.75);
        assert_eq!(Ease::In(3).apply(0.5), 0.125);
    }

    #[test]
    fn stems_cross_by_intensity_eased_over_the_given_time() {
        let pads = clip(4000, |_| 0.25);
        let drums = clip(4000, |_| 0.5);
        let mut track = AdaptiveTrack::new(
            vec![
                stem("pads", pads, StemGain::constant(1.0)),
                stem("drums", drums, StemGain::above(0.3, 0.1)),
            ],
            600.0,
            4,
        );
        track.ease = Ease::InOut(2);
        let mut mixer = play(&track, 1);
        assert_eq!(mixer.music_intensity(), Some(0.0));
        let calm = left(&mixer.render(100));
        assert!(calm.iter().all(|value| *value == 0.25));
        mixer.set_music_intensity(1.0, 0.2);
        let ramp = left(&mixer.render(200));
        assert_eq!(mixer.music_intensity(), Some(1.0));
        assert!(ramp.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(ramp[..60].iter().all(|value| *value == 0.25));
        assert!(ramp[150..].iter().all(|value| *value == 0.75));
        let full = left(&mixer.render(100));
        assert!(full.iter().all(|value| *value == 0.75));
        mixer.set_music_intensity(0.0, 0.0);
        let back = left(&mixer.render(40));
        assert!(
            back[CONTROL_FRAMES as usize..]
                .iter()
                .all(|value| *value == 0.25)
        );
    }

    #[test]
    fn every_stem_stays_in_sample_exact_sync() {
        let frames = 6400;
        let up = clip(frames, |k| ((k * 7919) % 1000) as f32 / 1000.0 - 0.5);
        let down = clip(frames, |k| 0.5 - ((k * 7919) % 1000) as f32 / 1000.0);
        let curve = StemGain::above(0.2, 0.6);
        let mut track = AdaptiveTrack::new(
            vec![stem("up", up, curve.clone()), stem("down", down, curve)],
            600.0,
            4,
        );
        track.sections = vec![
            Section {
                variants: vec![(0, 4), (4, 8), (8, 12)],
                ..Section::new("a", (0, 4))
            },
            Section::new("b", (12, 16)),
        ];
        track.transition = 0.05;
        track.intensity = 0.5;
        let mut mixer = play(&track, 3);
        let mut out = Vec::new();
        for round in 0..40u64 {
            if round % 7 == 3 {
                mixer.set_music_intensity((round % 5) as f32 / 4.0, 0.13);
            }
            if round % 11 == 5 {
                mixer.request_music_section(if round % 2 == 0 { "a" } else { "b" }, Quantize::Beat);
            }
            out.extend(mixer.render(97 + (round as usize % 13)));
        }
        assert!(out.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn a_section_change_waits_for_its_beat_bar_or_end() {
        let frames = 4000;
        let mut track = index_track(frames);
        track.sections = vec![Section::new("a", (0, 4)), Section::new("b", (8, 10))];
        for (at, from, to) in [
            (Quantize::Now, 150, 150),
            (Quantize::Beat, 150, 200),
            (Quantize::Bar, 150, 400),
            (Quantize::End, 150, 1600),
        ] {
            let mut mixer = play(&track, 1);
            mixer.render(from as usize);
            assert_eq!(mixer.music_section(), Some("a"));
            assert!(mixer.request_music_section("b", at));
            let got = left(&mixer.render(2000));
            let switch = (to - from) as usize;
            if switch > 0 {
                assert_eq!(frame_of(got[switch - 1], frames), to - 1, "{at:?}");
            }
            assert_eq!(frame_of(got[switch], frames), 3200, "{at:?}");
            assert_eq!(frame_of(got[switch + 1], frames), 3201, "{at:?}");
            assert_eq!(mixer.music_section(), Some("b"));
        }
        let mut mixer = play(&track, 1);
        assert!(!mixer.request_music_section("c", Quantize::Bar));
    }

    #[test]
    fn sections_flow_to_their_next_and_stop() {
        let frames = 4000;
        let mut track = index_track(frames);
        track.sections = vec![
            Section {
                next: Next::Section("loop".into()),
                ..Section::new("intro", (0, 1))
            },
            Section::new("loop", (2, 3)),
            Section {
                next: Next::Stop,
                ..Section::new("outro", (9, 10))
            },
        ];
        let mut mixer = play(&track, 1);
        let got = left(&mixer.render(1300));
        let at = |index: usize| frame_of(got[index], frames);
        assert_eq!((at(0), at(399), at(400), at(799)), (0, 399, 800, 1199));
        assert_eq!((at(800), at(1200)), (800, 800));
        assert_eq!(mixer.music_section(), Some("loop"));
        mixer.request_music_section("outro", Quantize::End);
        let tail = left(&mixer.render(800));
        let next = 400 - (1300 - 400) % 400;
        assert_eq!(frame_of(tail[next], frames), 3600);
        assert_eq!(frame_of(tail[next + 399], frames), 3999);
        assert!(tail[next + 400..].iter().all(|v| *v == 0.0));
        mixer.render(1);
        assert_eq!(mixer.music(), None);
    }

    fn variant_starts(seed: u64, avoid_repeat: bool) -> Vec<u64> {
        let frames = 4000;
        let mut track = index_track(frames);
        track.sections = vec![Section {
            variants: vec![(0, 1), (1, 2), (2, 3), (3, 4)],
            avoid_repeat,
            ..Section::new("loop", (0, 1))
        }];
        let mut mixer = play(&track, seed);
        let got = left(&mixer.render(400 * 40));
        got.iter()
            .step_by(400)
            .map(|value| frame_of(*value, frames))
            .collect()
    }

    #[test]
    fn the_seed_picks_the_variants_and_avoid_repeat_never_plays_one_twice() {
        let a = variant_starts(5, false);
        assert_eq!(a, variant_starts(5, false));
        assert_ne!(a, variant_starts(6, false));
        assert!(a.iter().all(|start| start % 400 == 0 && *start < 1600));
        for start in [0, 400, 800, 1200] {
            assert!(a.contains(&start));
        }
        assert!(a.windows(2).any(|pair| pair[0] == pair[1]));
        let b = variant_starts(5, true);
        assert!(b.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn the_block_size_does_not_change_a_sample() {
        let frames = 4000;
        let tone = clip(frames, |k| ((k % 37) as f32 / 37.0) - 0.5);
        let beat = clip(frames, |k| if k % 100 < 10 { 0.4 } else { 0.0 });
        let mut track = AdaptiveTrack::new(
            vec![
                stem("tone", tone, StemGain::constant(0.8)),
                stem("beat", beat, StemGain::above(0.3, 0.2)),
            ],
            600.0,
            4,
        );
        track.sections = vec![Section::new("a", (0, 2)), Section::new("b", (4, 8))];
        track.transition = 0.03;
        track.ease = Ease::InOut(3);
        let run = |sizes: &[usize]| {
            let mut mixer = play(&track, 2);
            let mut out = Vec::new();
            let mut done = 0;
            let mut turn = 0;
            while done < 3000 {
                if done >= 500 && turn == 0 {
                    mixer.set_music_intensity(1.0, 0.4);
                    mixer.request_music_section("b", Quantize::Bar);
                    turn = 1;
                }
                let size = sizes[done % sizes.len()].min(3000 - done);
                let size = if done < 500 {
                    size.min(500 - done)
                } else {
                    size
                };
                out.extend(mixer.render(size));
                done += size;
            }
            out
        };
        let whole = run(&[500, 2500]);
        let bits = |out: &[f32]| out.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&run(&[1, 7, 31, 64, 3])), bits(&whole));
        assert_eq!(bits(&run(&[13])), bits(&whole));
    }

    pub(crate) fn eight_stems(rate: u32, seconds: u64) -> AdaptiveTrack {
        let frames = u64::from(rate) * seconds;
        let curves = [
            StemGain::below(0.5, 0.3),
            StemGain::constant(0.6),
            StemGain::above(0.2, 0.1),
            StemGain::above(0.3, 0.2),
            StemGain::above(0.5, 0.1),
            StemGain::above(0.7, 0.1),
            StemGain {
                keys: vec![(0.0, 0.2), (0.5, 0.8), (1.0, 0.4)],
                power: false,
            },
            StemGain::above(0.9, 0.1),
        ];
        let stems = curves
            .into_iter()
            .enumerate()
            .map(|(n, gain)| {
                let hz = 55.0 * (n as f64 + 1.0) * 1.5;
                let channels = 1 + (n % 2) as u16;
                let samples = (0..frames)
                    .flat_map(|k| {
                        let phase = (k as f64 * hz / f64::from(rate)).fract();
                        let value = 0.1 * sine(phase);
                        let right = 0.1 * sine((phase + 0.25).fract());
                        [value, right].into_iter().take(usize::from(channels))
                    })
                    .collect();
                stem(
                    &format!("stem{n}"),
                    Clip {
                        rate,
                        channels,
                        samples,
                    },
                    gain,
                )
            })
            .collect();
        let mut track = AdaptiveTrack::new(stems, 120.0, 4);
        track.sections = vec![
            Section {
                variants: vec![(0, 1), (1, 2)],
                ..Section::new("calm", (0, 1))
            },
            Section {
                variants: vec![(2, 3), (3, 4)],
                avoid_repeat: true,
                ..Section::new("fight", (2, 3))
            },
        ];
        track.ease = Ease::InOut(2);
        track.transition = 0.02;
        track
    }

    fn fnv(samples: &[f32]) -> u64 {
        samples
            .iter()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, sample| {
                (hash ^ u64::from(sample.to_bits())).wrapping_mul(0x0100_0000_01b3)
            })
    }

    #[test]
    fn a_seeded_intensity_ramp_keeps_its_bits_on_every_platform() {
        let track = eight_stems(48_000, 8);
        let prepared = track.prepare(48_000).unwrap();
        let mut mixer = Mixer::new(48_000, 2);
        mixer.set_seed(11);
        mixer.play_adaptive(&prepared, 0.0, Level::default());
        let mut out = Vec::new();
        for tick in 0..600u64 {
            match tick {
                30 => mixer.set_music_intensity(0.4, 1.5),
                150 => {
                    mixer.set_music_intensity(1.0, 3.0);
                    mixer.request_music_section("fight", Quantize::Bar);
                }
                420 => {
                    mixer.set_music_intensity(0.1, 2.0);
                    mixer.request_music_section("calm", Quantize::Beat);
                }
                _ => {}
            }
            out.extend(mixer.render(800));
        }
        assert_eq!(mixer.music_section(), Some("calm"));
        let peak = out.iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
        assert!(peak > 0.3 && peak <= 1.0, "{peak}");
        assert_eq!(fnv(&out), 16_505_630_198_141_952_554);
    }

    #[test]
    fn a_track_with_mismatched_stems_is_refused() {
        let mut track = index_track(1000);
        track
            .stems
            .push(stem("short", clip(999, |_| 0.0), StemGain::constant(1.0)));
        assert!(track.prepare(RATE).unwrap_err().contains("one length"));
        let mut track = index_track(1000);
        track.sections = vec![Section {
            next: Next::Section("nowhere".into()),
            ..Section::new("a", (0, 1))
        }];
        assert!(track.prepare(RATE).unwrap_err().contains("nowhere"));
        let mut track = index_track(1000);
        track.sections = vec![Section::new("a", (3, 4))];
        assert!(
            track
                .prepare(RATE)
                .unwrap_err()
                .contains("past the stems' end")
        );
    }
}
