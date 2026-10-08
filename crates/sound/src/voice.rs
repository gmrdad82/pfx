use std::collections::VecDeque;
use std::sync::Arc;

use crate::channel::{ChannelId, Ramp};
use crate::envelope::EnvState;
use crate::interp::{Kernel, MAX_RATIO, MIN_RATIO};
use crate::mix::{Level, PcmQueue, VoiceId};
use crate::music::{Curve, Direction};
use crate::pcm::PcmVoice;
use crate::sound::SoundId;
use crate::synth::Synth;

const CHUNK: usize = 64;

pub(crate) struct Voice {
    pub(crate) id: VoiceId,
    pub(crate) channel: ChannelId,
    pub(crate) sound: Option<SoundId>,
    pub(crate) start: u64,
    pub(crate) level: Level,
    pub(crate) curve: Option<Curve>,
    pub(crate) gain: Ramp,
    pub(crate) pitch: Ramp,
    pub(crate) cursor: f64,
    pub(crate) rendered: bool,
    pub(crate) envelope: Option<EnvState>,
    pub(crate) source: Source,
}

pub(crate) enum Source {
    Buffer {
        samples: Arc<Vec<f32>>,
        channels: u16,
        frames: u64,
        looping: Option<(u64, u64)>,
    },
    Stream {
        queue: Arc<PcmQueue>,
        channels: u16,
        history: History,
    },
    Tone(Synth),
    Pcm(PcmVoice),
}

pub(crate) struct History {
    base: u64,
    frames: VecDeque<(f32, f32)>,
    total: Option<u64>,
}

impl History {
    pub(crate) fn new() -> Self {
        Self {
            base: 0,
            frames: VecDeque::new(),
            total: None,
        }
    }

    fn end(&self) -> u64 {
        self.base + self.frames.len() as u64
    }

    fn refresh(&mut self, queue: &PcmQueue, channels: u16) {
        if self.total.is_none() && queue.reader_ended() {
            let waiting = queue.len() / usize::from(channels.max(1));
            self.total = Some(self.end() + waiting as u64);
        }
    }

    fn fill(&mut self, queue: &PcmQueue, channels: u16, need_end: u64) {
        if self.end() >= need_end || self.total.is_some_and(|total| self.end() >= total) {
            return;
        }
        let channels = channels.max(1);
        let want = ((need_end - self.end()) as usize).max(CHUNK);
        let popped = queue.pop_frames(want, usize::from(channels));
        for k in 0..popped.real {
            self.frames
                .push_back(source_pair(&popped.samples, channels, k as u64));
        }
        if popped.ended && self.total.is_none() {
            self.total = Some(self.end() + (popped.available - popped.real) as u64);
        }
    }

    fn trim(&mut self, keep_from: u64) {
        while self.base < keep_from && !self.frames.is_empty() {
            self.frames.pop_front();
            self.base += 1;
        }
    }

    fn get(&self, k: i64) -> (f32, f32) {
        if k < self.base as i64 {
            return (0.0, 0.0);
        }
        self.frames
            .get((k - self.base as i64) as usize)
            .copied()
            .unwrap_or((0.0, 0.0))
    }
}

pub(crate) struct Block<'a> {
    pub(crate) clock: u64,
    pub(crate) frames: usize,
    pub(crate) out_channels: usize,
    pub(crate) rate: u32,
    pub(crate) kernel: &'a Kernel,
    pub(crate) pitch: Option<&'a [f32]>,
}

struct Played {
    left: f32,
    right: f32,
    channels: u16,
    local: u64,
    length: Option<u64>,
}

impl Voice {
    pub(crate) fn new(
        id: VoiceId,
        channel: ChannelId,
        start: u64,
        level: Level,
        source: Source,
    ) -> Self {
        Self {
            id,
            channel,
            sound: None,
            start,
            level,
            curve: None,
            gain: Ramp::new(1.0),
            pitch: Ramp::new(1.0),
            cursor: 0.0,
            rendered: false,
            envelope: None,
            source,
        }
    }

    pub(crate) fn mix_into(&mut self, out: &mut [f32], block: &Block<'_>) {
        let (pan_l, pan_r) = balance(self.level.pan);
        let mut span = Span {
            out,
            out_channels: block.out_channels,
        };
        let Voice {
            start,
            level,
            curve,
            gain,
            pitch,
            cursor,
            rendered,
            envelope: shaping,
            source,
            ..
        } = self;
        if let Source::Stream {
            queue,
            channels,
            history,
        } = source
        {
            history.refresh(queue, *channels);
        }
        if let Source::Pcm(pcm) = source {
            pcm.fill(block.frames);
        }
        for i in 0..block.frames {
            let at = block.clock + i as u64;
            if at < *start {
                continue;
            }
            let ratio = (pitch.next() * block.pitch.map_or(1.0, |curve| curve[i]))
                .clamp(MIN_RATIO, MAX_RATIO);
            let played = match source {
                Source::Buffer {
                    samples,
                    channels,
                    frames: total,
                    looping,
                } => {
                    if looping.is_none() && *cursor >= *total as f64 {
                        continue;
                    }
                    let direct = ratio == 1.0 && cursor.fract() == 0.0;
                    let (left, right) = if direct {
                        source_pair(samples, *channels, *cursor as u64)
                    } else {
                        let taps = block.kernel.taps(*cursor, ratio.max(1.0));
                        let mut left = 0.0f32;
                        let mut right = 0.0f32;
                        for (j, weight) in taps.weights[..taps.count].iter().enumerate() {
                            let (a, b) = buffer_pair(
                                samples,
                                *channels,
                                *total,
                                *looping,
                                taps.first + j as i64,
                            );
                            left += a * weight;
                            right += b * weight;
                        }
                        (left, right)
                    };
                    let (local, length) = match looping {
                        Some(_) => (at - *start, None),
                        None => (*cursor as u64, Some(*total)),
                    };
                    *cursor += f64::from(ratio);
                    if let Some((loop_start, loop_end)) = *looping
                        && *cursor >= loop_end as f64
                    {
                        let span = (loop_end - loop_start) as f64;
                        *cursor = loop_start as f64 + (*cursor - loop_start as f64) % span;
                    }
                    Played {
                        left,
                        right,
                        channels: *channels,
                        local,
                        length,
                    }
                }
                Source::Tone(synth) => {
                    let sample = synth.next(ratio, block.rate);
                    Played {
                        left: sample,
                        right: sample,
                        channels: 1,
                        local: at - *start,
                        length: None,
                    }
                }
                Source::Pcm(pcm) => {
                    let (left, right) = pcm.next();
                    Played {
                        left,
                        right,
                        channels: pcm.channels(),
                        local: at - *start,
                        length: None,
                    }
                }
                Source::Stream {
                    queue,
                    channels,
                    history,
                } => {
                    let direct = ratio == 1.0 && cursor.fract() == 0.0;
                    let scale = ratio.max(1.0);
                    let reach = if direct { 0 } else { Kernel::reach(scale) };
                    let floor = *cursor as u64;
                    let need = floor + reach + 1;
                    history.fill(queue, *channels, need);
                    let end = history.end();
                    if floor >= end {
                        if history.total.is_some_and(|total| floor >= total) {
                            continue;
                        }
                        break;
                    }
                    if end < need && history.total.is_none_or(|total| end < total) {
                        break;
                    }
                    let (left, right) = if direct {
                        history.get(floor as i64)
                    } else {
                        let taps = block.kernel.taps(*cursor, scale);
                        let mut left = 0.0f32;
                        let mut right = 0.0f32;
                        for (j, weight) in taps.weights[..taps.count].iter().enumerate() {
                            let (a, b) = history.get(taps.first + j as i64);
                            left += a * weight;
                            right += b * weight;
                        }
                        (left, right)
                    };
                    *cursor += f64::from(ratio);
                    Played {
                        left,
                        right,
                        channels: (*channels).max(1),
                        local: floor,
                        length: history.total,
                    }
                }
            };
            *rendered = true;
            let env = envelope(played.local, played.length, level.fade_in, level.fade_out);
            let curve_gain = curve.map_or(1.0, |curve| curve.gain(at))
                * shaping.as_mut().map_or(1.0, EnvState::next);
            add_frame(
                &mut span,
                i,
                played.channels,
                played.left,
                played.right,
                Gain {
                    value: level.volume * env * curve_gain * gain.next(),
                    pan_l,
                    pan_r,
                },
            );
        }
        if let Source::Stream { history, .. } = source {
            let keep = (*cursor as u64).saturating_sub(Kernel::reach(MAX_RATIO) + 2);
            history.trim(keep);
        }
    }

    pub(crate) fn audible_span(&self, clock: u64, block_end: u64) -> (u64, u64) {
        match &self.source {
            Source::Buffer {
                frames, looping, ..
            } => {
                if looping.is_some() {
                    return (self.start, u64::MAX);
                }
                let left = (*frames as f64 - self.cursor).max(0.0);
                let ratio = f64::from(self.pitch.current().clamp(MIN_RATIO, MAX_RATIO));
                let from = self.start.max(clock);
                (
                    self.start,
                    from.saturating_add((left / ratio).ceil() as u64),
                )
            }
            Source::Tone(_) => (self.start, u64::MAX),
            Source::Pcm(pcm) => {
                if pcm.finished() {
                    (self.start, self.start)
                } else {
                    (self.start, block_end)
                }
            }
            Source::Stream {
                queue,
                channels,
                history,
            } => {
                if queue.is_finished(*channels) && self.cursor >= history.end() as f64 {
                    (self.start, self.start)
                } else {
                    (self.start, block_end)
                }
            }
        }
    }

    pub(crate) fn finished(&self, clock: u64) -> bool {
        if self.envelope.as_ref().is_some_and(EnvState::done) {
            return true;
        }
        if self.curve.is_some_and(|curve| curve.spent(clock)) {
            return true;
        }
        match &self.source {
            Source::Buffer {
                frames, looping, ..
            } => *frames == 0 || (looping.is_none() && self.cursor >= *frames as f64),
            Source::Stream {
                queue,
                channels,
                history,
            } => queue.is_finished(*channels) && self.cursor >= history.end() as f64,
            Source::Tone(_) => false,
            Source::Pcm(pcm) => pcm.finished(),
        }
    }

    pub(crate) fn fading(&self) -> bool {
        self.curve
            .is_some_and(|curve| curve.direction == Direction::Out)
            || self.envelope.as_ref().is_some_and(EnvState::releasing)
    }

    pub(crate) fn loudness(&self, clock: u64) -> f32 {
        let left = match &self.source {
            Source::Buffer {
                frames, looping, ..
            } if looping.is_none() && *frames > 0 => {
                (1.0 - self.cursor / *frames as f64).clamp(0.0, 1.0) as f32
            }
            _ => 1.0,
        };
        self.level.volume
            * self.gain.current()
            * left
            * self.curve.map_or(1.0, |c| c.gain(clock))
            * self.envelope.as_ref().map_or(1.0, EnvState::level)
    }
}

fn buffer_pair(
    samples: &[f32],
    channels: u16,
    total: u64,
    looping: Option<(u64, u64)>,
    k: i64,
) -> (f32, f32) {
    if k < 0 {
        return (0.0, 0.0);
    }
    let mut k = k as u64;
    match looping {
        Some((start, end)) => {
            if k >= end {
                k = start + (k - start) % (end - start);
            }
        }
        None => {
            if k >= total {
                return (0.0, 0.0);
            }
        }
    }
    source_pair(samples, channels, k)
}

pub(crate) fn balance(pan: f32) -> (f32, f32) {
    let pan = pan.clamp(-1.0, 1.0);
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

pub(crate) fn envelope(local: u64, length: Option<u64>, fade_in: u64, fade_out: u64) -> f32 {
    let mut gain = 1.0f32;
    if fade_in > 0 && local < fade_in {
        gain *= (local as f64 / fade_in as f64) as f32;
    }
    if let Some(length) = length
        && fade_out > 0
        && length > 0
    {
        let start = length.saturating_sub(fade_out);
        if local >= start {
            gain *= (length.saturating_sub(local) as f64 / fade_out as f64) as f32;
        }
    }
    gain
}

pub(crate) fn source_pair(samples: &[f32], channels: u16, frame: u64) -> (f32, f32) {
    let channels = channels as usize;
    if channels == 0 {
        return (0.0, 0.0);
    }
    let Some(base) = (frame as usize).checked_mul(channels) else {
        return (0.0, 0.0);
    };
    let left = samples.get(base).copied().unwrap_or(0.0);
    if channels == 1 {
        (left, left)
    } else {
        (left, samples.get(base + 1).copied().unwrap_or(0.0))
    }
}

struct Span<'a> {
    out: &'a mut [f32],
    out_channels: usize,
}

struct Gain {
    value: f32,
    pan_l: f32,
    pan_r: f32,
}

fn add_frame(
    span: &mut Span<'_>,
    frame: usize,
    src_channels: u16,
    left: f32,
    right: f32,
    gain: Gain,
) {
    let Some(base) = frame.checked_mul(span.out_channels) else {
        return;
    };
    if span.out_channels <= 1 {
        let sample = if src_channels <= 1 {
            left
        } else {
            (left + right) * 0.5
        };
        if let Some(slot) = span.out.get_mut(base) {
            *slot += sample * gain.value;
        }
        return;
    }
    if let Some(slot) = span.out.get_mut(base) {
        *slot += left * gain.value * gain.pan_l;
    }
    if let Some(slot) = span.out.get_mut(base + 1) {
        *slot += right * gain.value * gain.pan_r;
    }
}
