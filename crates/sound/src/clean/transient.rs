use std::f32::consts::{PI, TAU};

use super::spectrum::Spectra;
use crate::voicefx::fft::Complex;
use pfx_core::sim::mix64;

const RANGE_DB: f32 = 80.0;
const SPREAD: f32 = 0.08;
const MIN_SPREAD: usize = 2;
const BACKGROUND_SECONDS: f32 = 1.0;
const NOVEL_DB: f32 = 12.0;
const FRACTION: f32 = 0.01;
const TAIL_DB: f32 = 6.0;
const BAND_LOW_HZ: f32 = 100.0;
const BAND_HIGH_HZ: f32 = 12000.0;
const BRIDGE: usize = 2;
const PAD_BEFORE: usize = 2;
const PAD_AFTER: usize = 4;
const HIDDEN_DB: f32 = 20.0;
const MIN_DECAY_DB: f32 = 1.0;
const EDGE: usize = 2;
const MAX_EVENT_SECONDS: f32 = 2.0;
const INPAINT_DB: f32 = 6.0;
const FOOTPRINT_BINS: usize = 2;
const BELOW: f32 = std::f32::consts::FRAC_1_SQRT_2;
const STEADY_RADIANS: f32 = 0.5;
const CLUSTER_BINS: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) first: usize,
    pub(crate) last: usize,
}

pub(crate) struct Found {
    pub(crate) spans: Vec<Span>,
    pub(crate) gains: Vec<f32>,
    pub(crate) fill: Vec<(usize, Complex)>,
}

fn phase(c: Complex) -> f32 {
    c.im.atan2(c.re)
}

fn wrap(x: f32) -> f32 {
    x - TAU * ((x + PI) / TAU).floor()
}

fn advance(spectra: &Spectra, earlier: usize, later: usize, k: usize) -> f32 {
    let g = spectra.grid;
    let expected = TAU * (k as f32 + 0.5) * g.hop as f32 / g.size as f32;
    let a = spectra.data[earlier * g.bins + k];
    let b = spectra.data[later * g.bins + k];
    expected + wrap(phase(b) - phase(a) - expected)
}

fn levels(mags: &[f32]) -> Vec<f32> {
    let db: Vec<f32> = mags.iter().map(|m| 20.0 * (m + 1e-12).log10()).collect();
    let top = db.iter().copied().fold(f32::MIN, f32::max);
    db.into_iter().map(|d| d.max(top - RANGE_DB)).collect()
}

fn dilate(row: &[f32], reach: impl Fn(usize) -> usize, out: &mut [f32]) {
    let n = row.len();
    let mut table = vec![row.to_vec()];
    let mut span = 1;
    while span * 2 <= n {
        let prev = table.last().map(Vec::as_slice).unwrap_or(row);
        let level: Vec<f32> = (0..=n - span * 2)
            .map(|i| prev[i].max(prev[i + span]))
            .collect();
        table.push(level);
        span *= 2;
    }
    for (k, o) in out.iter_mut().enumerate() {
        let r = reach(k);
        let lo = k.saturating_sub(r);
        let hi = (k + r).min(n - 1);
        let len = hi - lo + 1;
        let p = (usize::BITS - 1 - len.leading_zeros()) as usize;
        let level = &table[p];
        *o = level[lo].max(level[hi + 1 - (1 << p)]);
    }
}

fn reflect(i: isize, n: usize) -> usize {
    let period = 2 * n as isize;
    let m = i.rem_euclid(period);
    if m < n as isize {
        m as usize
    } else {
        (period - 1 - m) as usize
    }
}

fn median_in_time(rows: &[f32], frames: usize, bins: usize, width: usize) -> Vec<f32> {
    let half = (width / 2) as isize;
    let mut out = vec![0.0f32; rows.len()];
    let mut window: Vec<f32> = Vec::with_capacity(width);
    for k in 0..bins {
        window.clear();
        for i in -half..=half {
            window.push(rows[reflect(i, frames) * bins + k]);
        }
        window.sort_by(f32::total_cmp);
        for t in 0..frames {
            out[t * bins + k] = window[width / 2];
            let old = rows[reflect(t as isize - half, frames) * bins + k];
            let new = rows[reflect(t as isize + half + 1, frames) * bins + k];
            let at = window.partition_point(|v| v.total_cmp(&old).is_lt());
            window.remove(at);
            let to = window.partition_point(|v| v.total_cmp(&new).is_lt());
            window.insert(to, new);
        }
    }
    out
}

pub(crate) fn find(spectra: &Spectra, mags: &[f32]) -> Found {
    let grid = spectra.grid;
    let bins = grid.bins;
    let frames = grid.frames;
    let level = levels(mags);
    let mut wide = vec![0.0f32; level.len()];
    let mut near = vec![0.0f32; level.len()];
    for t in 0..frames {
        let row = &level[t * bins..(t + 1) * bins];
        dilate(
            row,
            |k| MIN_SPREAD.max((k as f32 * SPREAD).round() as usize),
            &mut wide[t * bins..(t + 1) * bins],
        );
        dilate(row, |_| 1, &mut near[t * bins..(t + 1) * bins]);
    }
    let width = ((BACKGROUND_SECONDS * grid.rate as f32 / grid.hop as f32).round() as usize) | 1;
    let background = median_in_time(&wide, frames, bins, width);
    let low = grid.bin(BAND_LOW_HZ).max(0.0).ceil() as usize;
    let high = (grid.bin(BAND_HIGH_HZ).floor().max(0.0) as usize).min(bins - 1);
    let band = (high + 1).saturating_sub(low).max(1);
    let novel = |t: usize, db: f32| {
        (low..=high)
            .filter(|&k| level[t * bins + k] - background[t * bins + k] > db)
            .count() as f32
            / band as f32
    };
    let inside = |t: usize| t >= EDGE && t + EDGE < frames;
    let mut flagged: Vec<bool> = (0..frames)
        .map(|t| inside(t) && novel(t, NOVEL_DB) > FRACTION)
        .collect();
    let clustered = |t: usize, db: f32, from: usize| -> Option<usize> {
        let novel = |k: usize| level[t * bins + k] - background[t * bins + k] > db;
        (from..=high).find(|&k| {
            novel(k) && {
                let from = k.saturating_sub(CLUSTER_BINS);
                let to = (k + CLUSTER_BINS).min(bins - 1);
                (from..=to).filter(|&j| j != k && novel(j)).count() >= CLUSTER_BINS
            }
        })
    };
    let tail: Vec<bool> = (0..frames)
        .map(|t| inside(t) && clustered(t, TAIL_DB, low).is_some())
        .collect();
    for t in 1..frames {
        if flagged[t - 1] && tail[t] {
            flagged[t] = true;
        }
    }
    for t in (0..frames.saturating_sub(1)).rev() {
        if flagged[t + 1] && tail[t] {
            flagged[t] = true;
        }
    }
    let mut runs: Vec<Span> = Vec::new();
    for (t, &on) in flagged.iter().enumerate() {
        if !on {
            continue;
        }
        match runs.last_mut() {
            Some(run) if t <= run.last + BRIDGE + 1 => run.last = t,
            _ => runs.push(Span { first: t, last: t }),
        }
    }
    let longest = (MAX_EVENT_SECONDS * grid.rate as f32 / grid.hop as f32) as usize;
    let mut spans: Vec<Span> = Vec::new();
    let peak = |t: usize| {
        (low..=high)
            .map(|k| level[t * bins + k] - background[t * bins + k])
            .fold(f32::MIN, f32::max)
    };
    for run in runs {
        let loudest = (run.first..=run.last)
            .max_by(|a, b| peak(*a).total_cmp(&peak(*b)))
            .unwrap_or(run.first);
        let last = peak(run.last);
        let slope = ((peak(loudest) - last) / (run.last - loudest).max(1) as f32).max(MIN_DECAY_DB);
        let hidden = ((last + HIDDEN_DB) / slope).ceil().max(0.0) as usize;
        let after = hidden.clamp(PAD_AFTER, (run.last - run.first + 1).max(PAD_AFTER));
        let span = Span {
            first: run.first.saturating_sub(PAD_BEFORE),
            last: (run.last + after).min(frames - 1),
        };
        if span.last - span.first + 1 > longest {
            continue;
        }
        match spans.last_mut() {
            Some(prev) if span.first <= prev.last + 1 => prev.last = span.last,
            _ => spans.push(span),
        }
    }
    let mut gains = vec![1.0f32; level.len()];
    let mut fill = Vec::new();
    let edge = |from: usize, to: usize, k: usize| -> Option<f32> {
        if from > to {
            return None;
        }
        let sum: f32 = (from..=to).map(|t| near[t * bins + k]).sum();
        Some(sum / (to - from + 1) as f32)
    };
    for (n, span) in spans.iter().enumerate() {
        let Some(lowest) = (span.first..=span.last)
            .filter_map(|t| clustered(t, INPAINT_DB, 0))
            .min()
        else {
            continue;
        };
        let lowest = (grid.bin(grid.hz(lowest as f32) * BELOW).floor().max(0.0) as usize)
            .min(lowest.saturating_sub(FOOTPRINT_BINS));
        let left_from = span.first.saturating_sub(EDGE);
        let right_to = (span.last + EDGE).min(frames - 1);
        let gap = (span.last - span.first + 2) as f32;
        for k in lowest..bins {
            let left = (span.first > 0)
                .then(|| edge(left_from, span.first - 1, k))
                .flatten();
            let right = (span.last + 1 < frames)
                .then(|| edge(span.last + 1, right_to, k))
                .flatten();
            let (left, right) = match (left, right) {
                (Some(a), Some(b)) => (a, b),
                (Some(a), None) => (a, a),
                (None, Some(b)) => (b, b),
                (None, None) => continue,
            };
            let anchor = if span.first >= 3 {
                let p = span.first - 1;
                let step = advance(spectra, p - 1, p, k);
                let steady = wrap(step - advance(spectra, p - 2, p - 1, k)).abs() < STEADY_RADIANS;
                Some((p, phase(spectra.data[p * bins + k]), step, steady))
            } else if span.last + 3 < frames {
                let q = span.last + 1;
                let step = advance(spectra, q, q + 1, k);
                let steady = wrap(step - advance(spectra, q + 1, q + 2, k)).abs() < STEADY_RADIANS;
                Some((q, phase(spectra.data[q * bins + k]), step, steady))
            } else {
                None
            };
            for t in span.first..=span.last {
                let i = t * bins + k;
                let f = (t - span.first + 1) as f32 / gap;
                let target = 10f32.powf((left + (right - left) * f) / 20.0);
                gains[i] = 0.0;
                let angle = match anchor {
                    Some((at, start, step, true)) => wrap(start + (t as f32 - at as f32) * step),
                    _ => {
                        let draw = mix64(((n as u64) << 40) ^ i as u64);
                        (draw >> 40) as f32 / (1u64 << 24) as f32 * TAU
                    }
                };
                fill.push((i, Complex::polar(angle).scale(target.min(mags[i]))));
            }
        }
    }
    fill.sort_by_key(|(i, _)| *i);
    Found { spans, gains, fill }
}
