use pfx_core::sim::{exp, ln};
use pfx_sound::voicefx::spectrum::bin_hz;

use super::tempo::Beat;
use super::{Banks, HOP, Power, SIZE};

const A0_HZ: f64 = 27.5;
const CENTRE_OCTAVE: f64 = 5.0;
const OCTAVE_WIDTH: f64 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Frames {
    pub hop: usize,
    pub rate: u32,
    pub frames: Vec<[f64; 12]>,
}

pub fn frames(mono: &[f32], rate: u32) -> Frames {
    frames_of(&Power::of(mono), &Banks::new(rate), rate)
}

pub(crate) fn frame(row: &[f32], bank: &[[f64; 12]]) -> [f64; 12] {
    let mut chroma = [0.0f64; 12];
    for (&power, column) in row.iter().zip(bank) {
        let power = f64::from(power);
        if power == 0.0 {
            continue;
        }
        for (c, w) in chroma.iter_mut().zip(column) {
            *c += w * power;
        }
    }
    let top = chroma.iter().copied().fold(0.0f64, f64::max);
    if top > f64::MIN_POSITIVE {
        for c in &mut chroma {
            *c /= top;
        }
    } else {
        chroma = [0.0; 12];
    }
    chroma
}

pub(crate) fn frames_of(power: &Power, banks: &Banks, rate: u32) -> Frames {
    Frames {
        hop: HOP,
        rate,
        frames: power.rows().map(|row| frame(row, &banks.chroma)).collect(),
    }
}

pub fn per_beat(frames: &Frames, beat: &Beat, beats: f64) -> Vec<[f64; 12]> {
    if frames.frames.is_empty() || frames.rate == 0 || beat.bpm <= 0.0 || beats <= 0.0 {
        return Vec::new();
    }
    let step = beats * 60.0 / beat.bpm;
    let seconds = |t: usize| (t * frames.hop) as f64 / f64::from(frames.rate);
    let end = seconds(frames.frames.len());
    if end <= beat.offset {
        return Vec::new();
    }
    let count = ((end - beat.offset) / step).ceil().max(1.0) as usize;
    let mut sums = vec![[0.0f64; 12]; count];
    let mut counts = vec![0usize; count];
    for (t, frame) in frames.frames.iter().enumerate() {
        let at = seconds(t) - beat.offset;
        if at < 0.0 {
            continue;
        }
        let i = ((at / step).floor() as usize).min(count - 1);
        for (s, v) in sums[i].iter_mut().zip(frame) {
            *s += v;
        }
        counts[i] += 1;
    }
    sums.iter()
        .zip(&counts)
        .map(|(sum, &n)| {
            if n == 0 {
                return [0.0; 12];
            }
            let mut mean = *sum;
            for v in &mut mean {
                *v /= n as f64;
            }
            mean
        })
        .collect()
}

pub(crate) fn bank(rate: u32) -> Vec<[f64; 12]> {
    let bins = SIZE / 2;
    let octs: Vec<f64> = (0..bins)
        .map(|bin| 12.0 * ln(f64::from(bin_hz(bin, SIZE, rate)) / A0_HZ) / ln(2.0))
        .collect();
    (0..bins)
        .map(|bin| {
            let width = octs
                .get(bin + 1)
                .map_or(1.0, |next| (next - octs[bin]).max(1.0));
            let mut column = [0.0f64; 12];
            for (c, slot) in column.iter_mut().enumerate() {
                let d = (octs[bin] - c as f64 + 6.0 + 120.0).rem_euclid(12.0) - 6.0;
                let z = 2.0 * d / width;
                *slot = exp(-0.5 * z * z);
            }
            let norm = column.iter().map(|w| w * w).sum::<f64>().sqrt();
            let octave = (octs[bin] / 12.0 - CENTRE_OCTAVE) / OCTAVE_WIDTH;
            let scale = exp(-0.5 * octave * octave);
            for w in &mut column {
                *w = if norm > 0.0 { *w / norm * scale } else { 0.0 };
            }
            column.rotate_left(3);
            column
        })
        .collect()
}
