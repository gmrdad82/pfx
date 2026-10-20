use std::collections::VecDeque;

pub const OVERSAMPLE: usize = 4;
const TAPS: usize = 12;
const LATE: usize = TAPS / 2;
const PASSES: usize = 4;
const LN2: f64 = std::f64::consts::LN_2;
const MARGIN: usize = 2 * TAPS;
const SLICE_FRAMES: usize = 1 << 15;
const MAX_SLICES: usize = 16;

const STEP: f32 = 1.0 / 8192.0;

pub const ANNEX: [[i16; TAPS]; OVERSAMPLE] = [
    [
        14, 90, -161, 272, -487, 1125, 7964, -838, 390, -218, 122, -68,
    ],
    [
        -239, 240, -424, 730, -1364, 3810, 6388, -1641, 832, -477, 271, -155,
    ],
    [
        -155, 271, -477, 832, -1641, 6388, 3810, -1364, 730, -424, 240, -239,
    ],
    [
        -68, 122, -218, 390, -838, 7964, 1125, -487, 272, -161, 90, 14,
    ],
];

const fn fir() -> [[f32; TAPS]; OVERSAMPLE] {
    let mut out = [[0.0; TAPS]; OVERSAMPLE];
    let mut p = 0;
    while p < OVERSAMPLE {
        let mut t = 0;
        while t < TAPS {
            out[p][t] = ANNEX[p][t] as f32 * STEP;
            t += 1;
        }
        p += 1;
    }
    out
}

const FIR: [[f32; TAPS]; OVERSAMPLE] = fir();

fn channel(samples: &[f32], channels: usize, c: usize) -> Vec<f32> {
    let mut padded = vec![0.0f32; TAPS - 1];
    padded.extend(samples.iter().skip(c).step_by(channels));
    padded.extend([0.0f32; LATE]);
    padded
}

fn between(window: &[f32]) -> f32 {
    let mut most = 0.0f32;
    for phase in &FIR {
        let mut acc = 0.0f32;
        for (c, x) in phase.iter().zip(window) {
            acc += c * x;
        }
        most = most.max(acc.abs());
    }
    most
}

pub fn oversampled(samples: &[f32]) -> Vec<f32> {
    let padded = channel(samples, 1, 0);
    let mut out = Vec::with_capacity(padded.len() * OVERSAMPLE);
    for window in padded.windows(TAPS) {
        for phase in &FIR {
            out.push(phase.iter().zip(window).map(|(c, x)| c * x).sum());
        }
    }
    out
}

pub fn local_peaks(samples: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(frames / SLICE_FRAMES)
        .min(MAX_SLICES);
    if threads < 2 {
        return scan(samples, channels);
    }
    let slice = frames.div_ceil(threads);
    let mut out = vec![0.0f32; frames];
    std::thread::scope(|scope| {
        for (index, part) in out.chunks_mut(slice).enumerate() {
            let from = index * slice;
            let to = from + part.len();
            let lo = from.saturating_sub(MARGIN);
            let hi = (to + MARGIN).min(frames);
            let window = &samples[lo * channels..hi * channels];
            scope.spawn(move || part.copy_from_slice(&scan(window, channels)[from - lo..to - lo]));
        }
    });
    out
}

fn scan(samples: &[f32], channels: usize) -> Vec<f32> {
    let frames = samples.len() / channels;
    let mut out = vec![0.0f32; frames];
    for c in 0..channels {
        let padded = channel(samples, channels, c);
        for (i, window) in padded.windows(TAPS).enumerate() {
            let over = between(window);
            let left = i.checked_sub(LATE);
            for m in [left, left.map(|m| m + 1)].into_iter().flatten() {
                if let Some(o) = out.get_mut(m) {
                    *o = o.max(over);
                }
            }
        }
        for (o, x) in out.iter_mut().zip(samples.iter().skip(c).step_by(channels)) {
            *o = o.max(x.abs());
        }
    }
    out
}

pub fn true_peak(samples: &[f32], channels: usize) -> f32 {
    local_peaks(samples, channels)
        .into_iter()
        .fold(0.0f32, f32::max)
}

pub fn decibels(peak: f32) -> f32 {
    20.0 * peak.max(1e-20).log10()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limiter {
    pub ceiling: f32,
    pub attack: f32,
    pub hold: f32,
    pub release: f32,
}

impl Limiter {
    pub fn new(ceiling_db: f32) -> Limiter {
        Limiter {
            ceiling: 10f64.powf(f64::from(ceiling_db) / 20.0) as f32,
            attack: 0.005,
            hold: 0.005,
            release: 0.08,
        }
    }

    fn gains(&self, peaks: &[f32], rate: u32) -> Vec<f32> {
        let rate = f64::from(rate.max(1));
        let attack = (f64::from(self.attack.max(0.0)) * rate).ceil() as usize;
        let hold = (f64::from(self.hold.max(0.0)) * rate).ceil() as usize;
        let release = f64::from(self.release.max(0.0)) * rate;
        let slew = LN2 / (release + LN2);
        let window = attack + hold + 1;
        let box_len = attack + 1;
        let ceiling = self.ceiling;
        let need = |n: usize| -> f64 {
            peaks
                .get(n)
                .map_or(1.0, |p| f64::from(ceiling / ceiling.max(*p)))
        };
        let mut lows: VecDeque<(usize, f64)> = VecDeque::new();
        let mut released = 1.0f64;
        let mut boxed: VecDeque<f64> = VecDeque::from(vec![1.0; box_len]);
        let mut sum = box_len as f64;
        let mut out = Vec::with_capacity(peaks.len());
        for n in 0..peaks.len() + attack {
            let g = need(n);
            while lows.back().is_some_and(|(_, v)| *v >= g) {
                lows.pop_back();
            }
            lows.push_back((n, g));
            while lows.front().is_some_and(|(i, _)| i + window <= n) {
                lows.pop_front();
            }
            let held = lows.front().map_or(1.0, |(_, v)| *v);
            released += (held - released) * slew;
            released = released.min(held);
            sum += released - boxed.pop_front().unwrap_or(1.0);
            boxed.push_back(released);
            if n >= attack {
                out.push((sum / box_len as f64) as f32);
            }
        }
        out
    }

    pub fn master(&self, samples: &mut [f32], channels: usize, rate: u32) -> bool {
        let channels = channels.max(1);
        let mut changed = false;
        for _ in 0..PASSES {
            let peaks = local_peaks(samples, channels);
            if peaks.iter().all(|p| *p <= self.ceiling) {
                return changed;
            }
            changed = true;
            let gains = self.gains(&peaks, rate);
            for (frame, g) in samples.chunks_exact_mut(channels).zip(&gains) {
                for s in frame {
                    *s *= g;
                }
            }
        }
        let peak = true_peak(samples, channels);
        if peak > self.ceiling {
            let k = self.ceiling / peak;
            for s in samples.iter_mut() {
                *s *= k;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sliced_scan_reads_the_same_peaks_as_one_pass() {
        let frames = SLICE_FRAMES * 5 + 77;
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let samples: Vec<f32> = (0..frames * 2)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
            })
            .collect();
        assert!(local_peaks(&samples, 2) == scan(&samples, 2));
    }
}
