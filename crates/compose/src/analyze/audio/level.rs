use pfx_sound::voicefx::spectrum::bin_hz;

use super::tempo::Beat;
use super::{BINS, Power, SIZE, rms};

const LOW_HZ: f32 = 250.0;
const HIGH_HZ: f32 = 4000.0;

pub fn per_bar(mono: &[f32], rate: u32, beat: &Beat, beats_per_bar: u32) -> Vec<f32> {
    if mono.is_empty() || rate == 0 || beat.bpm <= 0.0 || beats_per_bar == 0 {
        return vec![0.0];
    }
    let bar = f64::from(beats_per_bar) * 60.0 / beat.bpm;
    let rate = f64::from(rate);
    let seconds = mono.len() as f64 / rate;
    let bars = ((seconds - beat.offset) / bar).ceil().max(1.0) as usize;
    let sample = |s: f64| ((s * rate).round().max(0.0) as usize).min(mono.len());
    let levels: Vec<f64> = (0..bars)
        .map(|i| {
            let from = if i == 0 {
                0
            } else {
                sample(beat.offset + i as f64 * bar)
            };
            let to = if i + 1 == bars {
                mono.len()
            } else {
                sample(beat.offset + (i + 1) as f64 * bar)
            };
            rms(&mono[from..to.max(from)])
        })
        .collect();
    let top = levels.iter().copied().fold(0.0f64, f64::max);
    levels
        .iter()
        .map(|&l| if top > 0.0 { (l / top) as f32 } else { 0.0 })
        .collect()
}

pub fn bands(mono: &[f32], rate: u32) -> [f32; 3] {
    bands_of(&Power::of(mono), rate)
}

pub(crate) fn bands_of(power: &Power, rate: u32) -> [f32; 3] {
    let edge = |hz: f32| {
        (0..BINS)
            .find(|&bin| bin_hz(bin, SIZE, rate) >= hz)
            .unwrap_or(BINS)
    };
    let low = edge(LOW_HZ);
    let high = edge(HIGH_HZ).max(low);
    let mut energy = [0.0f64; 3];
    for row in power.rows() {
        let sum = |part: &[f32]| part.iter().map(|&p| f64::from(p)).sum::<f64>();
        energy[0] += sum(&row[..low]);
        energy[1] += sum(&row[low..high]);
        energy[2] += sum(&row[high..]);
    }
    let total: f64 = energy.iter().sum();
    if total <= 0.0 {
        return [0.0; 3];
    }
    energy.map(|e| (e / total) as f32)
}
