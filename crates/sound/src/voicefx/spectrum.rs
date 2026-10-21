use std::f64::consts::PI;

use pfx_core::sim::cos;

use super::fft::{Complex, ModifiedRealFft};

pub fn powers(mono: &[f32], size: usize, hop: usize) -> Vec<f32> {
    if mono.is_empty() || size < 2 || !size.is_power_of_two() || hop == 0 {
        return Vec::new();
    }
    let window: Vec<f32> = (0..size)
        .map(|i| (0.5 - 0.5 * cos(2.0 * PI * i as f64 / size as f64)) as f32)
        .collect();
    let mut fft = ModifiedRealFft::new(size);
    let mut time = vec![0.0f32; size];
    let mut freq = vec![Complex::ZERO; size / 2];
    let count = mono.len().div_ceil(hop);
    let mut out = Vec::with_capacity(count * size / 2);
    for frame in 0..count {
        let start = frame * hop;
        let end = (start + size).min(mono.len());
        let have = end - start;
        for ((slot, s), w) in time.iter_mut().zip(&mono[start..end]).zip(&window) {
            *slot = s * w;
        }
        time[have..].fill(0.0);
        fft.fft(&time, &mut freq);
        out.extend(freq.iter().map(|c| c.norm()));
    }
    out
}

pub fn magnitudes(mono: &[f32], size: usize, hop: usize) -> Vec<Vec<f32>> {
    let bins = size / 2;
    if bins == 0 {
        return Vec::new();
    }
    powers(mono, size, hop)
        .chunks_exact(bins)
        .map(|frame| frame.iter().map(|p| p.sqrt()).collect())
        .collect()
}

pub fn bin_hz(bin: usize, size: usize, rate: u32) -> f32 {
    if size == 0 {
        return 0.0;
    }
    ((bin as f64 + 0.5) * f64::from(rate) / size as f64) as f32
}
