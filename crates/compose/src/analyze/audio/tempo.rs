use std::f64::consts::PI;

use pfx_core::sim::{cos, ln};

use super::onset::Strength;

const AC_SECONDS: f64 = 8.0;
const START_BPM: f64 = 120.0;
const STD_BPM: f64 = 1.0;
const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 200.0;
const PERIOD_STEP: f64 = 0.01;
const PHASE_STEP: f64 = 0.25;
const METER_MARGIN: f64 = 1.1;
const MIN_METER_BEATS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    pub bpm: f64,
    pub offset: f64,
    pub confidence: f64,
}

pub fn estimate(strength: &Strength) -> Option<Beat> {
    let x: Vec<f64> = strength.values.iter().map(|&v| f64::from(v)).collect();
    if x.len() < 2 || strength.hop == 0 || strength.rate == 0 || !x.iter().any(|&v| v > 0.0) {
        return None;
    }
    let frames_per_minute = 60.0 * f64::from(strength.rate) / strength.hop as f64;
    let win =
        ((AC_SECONDS * f64::from(strength.rate) / strength.hop as f64).floor() as usize).max(2);
    let tempogram = tempogram(&x, win);
    let mut best: Option<(usize, f64)> = None;
    for (lag, &value) in tempogram.iter().enumerate().skip(1) {
        let bpm = frames_per_minute / lag as f64;
        if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            continue;
        }
        let octaves = (ln(bpm) - ln(START_BPM)) / ln(2.0) / STD_BPM;
        let score = ln(1.0 + 1e6 * value) - 0.5 * octaves * octaves;
        if best.is_none_or(|(_, top)| score > top) {
            best = Some((lag, score));
        }
    }
    let (lag, _) = best?;
    let confidence = tempogram[lag].clamp(0.0, 1.0);
    if confidence <= 0.0 {
        return None;
    }
    let shortest = frames_per_minute / MAX_BPM;
    let longest = frames_per_minute / MIN_BPM;
    let smooth = smoothed(&x);
    let mut fit = (lag as f64, 0.0, f64::NEG_INFINITY);
    let steps = (2.0 / PERIOD_STEP).round() as usize;
    for step in 0..=steps {
        let period = lag as f64 - 1.0 + step as f64 * PERIOD_STEP;
        if period < shortest || period > longest || period <= 0.0 {
            continue;
        }
        let (phase, score) = best_phase(&smooth, period);
        if score > fit.2 {
            fit = (period, phase, score);
        }
    }
    let (period, phase, _) = fit;
    Some(Beat {
        bpm: frames_per_minute / period,
        offset: phase * strength.hop as f64 / f64::from(strength.rate),
        confidence,
    })
}

pub fn beats_per_bar(strength: &Strength, beat: &Beat) -> u32 {
    if beat.bpm <= 0.0 || strength.hop == 0 || strength.rate == 0 {
        return 4;
    }
    let frames_per_second = f64::from(strength.rate) / strength.hop as f64;
    let period = 60.0 / beat.bpm * frames_per_second;
    let start = beat.offset * frames_per_second;
    let x = &strength.values;
    let mut accents = Vec::new();
    let mut k = 0usize;
    loop {
        let at = (start + k as f64 * period).round() as usize;
        if at >= x.len() {
            break;
        }
        let from = at.saturating_sub(1);
        let to = (at + 2).min(x.len());
        accents.push(x[from..to].iter().copied().fold(0.0f32, f32::max) as f64);
        k += 1;
    }
    if accents.len() < MIN_METER_BEATS {
        return 4;
    }
    let mean = accents.iter().sum::<f64>() / accents.len() as f64;
    if mean <= 0.0 {
        return 4;
    }
    let contrast = |m: usize| {
        (0..m)
            .map(|j| {
                let group: Vec<f64> = accents.iter().skip(j).step_by(m).copied().collect();
                group.iter().sum::<f64>() / group.len().max(1) as f64
            })
            .fold(0.0f64, f64::max)
            / mean
    };
    if contrast(3) > contrast(4) * METER_MARGIN {
        3
    } else {
        4
    }
}

fn tempogram(x: &[f64], win: usize) -> Vec<f64> {
    let n = x.len();
    let half = win / 2;
    let first = x[0];
    let last = x[n - 1];
    let mut padded = Vec::with_capacity(n + 2 * half);
    padded.extend((0..half).map(|i| first * i as f64 / half as f64));
    padded.extend_from_slice(x);
    padded.extend((0..half).map(|i| last * (half - 1 - i) as f64 / half as f64));
    let window: Vec<f64> = (0..win)
        .map(|i| 0.5 - 0.5 * cos(2.0 * PI * i as f64 / win as f64))
        .collect();
    let mut mean = vec![0.0f64; win];
    let mut frame = vec![0.0f64; win];
    for t in 0..n {
        for (i, slot) in frame.iter_mut().enumerate() {
            *slot = padded.get(t + i).copied().unwrap_or(0.0) * window[i];
        }
        let zero: f64 = frame.iter().map(|v| v * v).sum();
        if zero <= f64::MIN_POSITIVE {
            continue;
        }
        for (lag, total) in mean.iter_mut().enumerate() {
            let ac: f64 = frame[..win - lag]
                .iter()
                .zip(&frame[lag..])
                .map(|(a, b)| a * b)
                .sum();
            *total += ac / zero;
        }
    }
    for value in &mut mean {
        *value /= n as f64;
    }
    mean
}

fn smoothed(x: &[f64]) -> Vec<f64> {
    (0..x.len())
        .map(|i| {
            let before = if i > 0 { x[i - 1] } else { 0.0 };
            let after = x.get(i + 1).copied().unwrap_or(0.0);
            0.25 * before + 0.5 * x[i] + 0.25 * after
        })
        .collect()
}

fn best_phase(x: &[f64], period: f64) -> (f64, f64) {
    let mut best = (0.0, f64::NEG_INFINITY);
    let steps = (period / PHASE_STEP).ceil() as usize;
    for step in 0..steps {
        let phase = step as f64 * PHASE_STEP;
        let mut sum = 0.0;
        let mut count = 0usize;
        let mut at = phase;
        while at < (x.len() - 1) as f64 {
            let i = at.floor() as usize;
            let f = at - i as f64;
            sum += x[i] * (1.0 - f) + x[i + 1] * f;
            count += 1;
            at += period;
        }
        if count == 0 {
            continue;
        }
        let score = sum / count as f64;
        if score > best.1 {
            best = (phase, score);
        }
    }
    best
}
