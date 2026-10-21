use std::f64::consts::LN_10;

use pfx_core::sim::{exp, ln};
use pfx_sound::voicefx::spectrum::bin_hz;

use super::{Banks, HOP, Power, SIZE};

const MELS: usize = 128;
const LAG: usize = 1;
const AMIN: f64 = 1e-10;
const TOP_DB: f64 = 80.0;
const PRE_MAX: f64 = 0.03;
const POST_MAX: f64 = 0.0;
const PRE_AVG: f64 = 0.10;
const POST_AVG: f64 = 0.10;
const WAIT: f64 = 0.03;
const DELTA: f32 = 0.07;

#[derive(Clone, Debug, PartialEq)]
pub struct Strength {
    pub hop: usize,
    pub rate: u32,
    pub values: Vec<f32>,
}

impl Strength {
    pub fn seconds(&self, frame: usize) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        (frame * self.hop) as f64 / f64::from(self.rate)
    }
}

pub fn strength(mono: &[f32], rate: u32) -> Strength {
    strength_of(&Power::of(mono), &Banks::new(rate), rate)
}

pub(crate) fn strength_of(spectrogram: &Power, banks: &Banks, rate: u32) -> Strength {
    let mut db: Vec<Vec<f64>> = spectrogram
        .rows()
        .map(|row| {
            banks
                .mel
                .iter()
                .map(|(start, weights)| {
                    let power: f64 = weights
                        .iter()
                        .zip(&row[*start..])
                        .map(|(w, &p)| w * f64::from(p))
                        .sum();
                    10.0 * ln(power.max(AMIN)) / LN_10
                })
                .collect()
        })
        .collect();
    let peak = db
        .iter()
        .flatten()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    for value in db.iter_mut().flatten() {
        *value = value.max(peak - TOP_DB);
    }
    let frames = db.len();
    let pad = LAG + SIZE / (2 * HOP);
    let mut values = vec![0.0f32; pad.min(frames)];
    for t in LAG..frames {
        if values.len() >= frames {
            break;
        }
        let flux: f64 = db[t]
            .iter()
            .zip(&db[t - LAG])
            .map(|(now, before)| (now - before).max(0.0))
            .sum();
        values.push((flux / MELS as f64) as f32);
    }
    Strength {
        hop: HOP,
        rate,
        values,
    }
}

pub fn peaks(strength: &Strength) -> Vec<usize> {
    let x = &strength.values;
    if x.is_empty() || strength.hop == 0 {
        return Vec::new();
    }
    let low = x.iter().copied().fold(f32::INFINITY, f32::min);
    let high = x.iter().copied().fold(f32::NEG_INFINITY, f32::max) - low;
    if high <= 0.0 {
        return Vec::new();
    }
    let x: Vec<f32> = x
        .iter()
        .map(|v| (v - low) / (high + f32::MIN_POSITIVE))
        .collect();
    let frames =
        |seconds: f64| (seconds * f64::from(strength.rate) / strength.hop as f64).floor() as usize;
    let pre_max = frames(PRE_MAX);
    let post_max = frames(POST_MAX) + 1;
    let pre_avg = frames(PRE_AVG);
    let post_avg = frames(POST_AVG) + 1;
    let wait = frames(WAIT);
    let len = x.len();
    let max_of = |from: usize, to: usize| {
        x[from..to.min(len)]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
    };
    let mean_of = |from: usize, to: usize| {
        let window = &x[from..to.min(len)];
        window.iter().sum::<f32>() / window.len() as f32
    };
    let mut peaks = Vec::new();
    let mut n = 1;
    if x[0] >= max_of(0, post_max) && x[0] >= mean_of(0, post_avg) + DELTA {
        peaks.push(0);
        n = wait + 1;
    }
    while n < len {
        if x[n] != max_of(n.saturating_sub(pre_max), n + post_max) {
            n += 1;
            continue;
        }
        if x[n] < mean_of(n.saturating_sub(pre_avg), n + post_avg) + DELTA {
            n += 1;
            continue;
        }
        peaks.push(n);
        n += wait + 1;
    }
    peaks
}

pub(crate) fn mel_bank(rate: u32) -> Vec<(usize, Vec<f64>)> {
    let bins = SIZE / 2;
    let top = hz_to_mel(f64::from(rate) / 2.0);
    let edges: Vec<f64> = (0..MELS + 2)
        .map(|i| mel_to_hz(top * i as f64 / (MELS + 1) as f64))
        .collect();
    (0..MELS)
        .map(|i| {
            let lower_width = edges[i + 1] - edges[i];
            let upper_width = edges[i + 2] - edges[i + 1];
            let norm = 2.0 / (edges[i + 2] - edges[i]);
            let weights: Vec<(usize, f64)> = (0..bins)
                .filter_map(|bin| {
                    let hz = f64::from(bin_hz(bin, SIZE, rate));
                    let lower = (hz - edges[i]) / lower_width;
                    let upper = (edges[i + 2] - hz) / upper_width;
                    let w = lower.min(upper).max(0.0);
                    (w > 0.0).then_some((bin, w * norm))
                })
                .collect();
            let start = weights.first().map_or(0, |&(bin, _)| bin);
            let mut dense = vec![0.0; weights.last().map_or(0, |&(bin, _)| bin + 1 - start)];
            for (bin, w) in weights {
                dense[bin - start] = w;
            }
            (start, dense)
        })
        .collect()
}

const F_SP: f64 = 200.0 / 3.0;
const MIN_LOG_HZ: f64 = 1000.0;
const MIN_LOG_MEL: f64 = MIN_LOG_HZ / F_SP;

fn log_step() -> f64 {
    ln(6.4) / 27.0
}

fn hz_to_mel(hz: f64) -> f64 {
    if hz >= MIN_LOG_HZ {
        MIN_LOG_MEL + ln(hz / MIN_LOG_HZ) / log_step()
    } else {
        hz / F_SP
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel >= MIN_LOG_MEL {
        MIN_LOG_HZ * exp(log_step() * (mel - MIN_LOG_MEL))
    } else {
        F_SP * mel
    }
}
