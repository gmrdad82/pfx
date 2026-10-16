use std::f64::consts::PI;

use crate::voicefx::fft::{Complex, ModifiedRealFft};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub(crate) size: usize,
    pub(crate) hop: usize,
    pub(crate) bins: usize,
    pub(crate) frames: usize,
    pub(crate) rate: u32,
    pub(crate) len: usize,
}

impl Grid {
    pub(crate) fn new(size: usize, rate: u32, len: usize) -> Grid {
        let hop = size / 4;
        Grid {
            size,
            hop,
            bins: size / 2,
            frames: len.div_ceil(hop) + 1,
            rate,
            len,
        }
    }

    pub(crate) fn hz(&self, bin: f32) -> f32 {
        (bin + 0.5) * self.rate as f32 / self.size as f32
    }

    pub(crate) fn bin(&self, hz: f32) -> f32 {
        hz * self.size as f32 / self.rate as f32 - 0.5
    }

    pub(crate) fn bin_hz(&self) -> f32 {
        self.rate as f32 / self.size as f32
    }

    pub(crate) fn seconds(&self, frame: f32) -> f32 {
        frame * self.hop as f32 / self.rate as f32
    }

    pub(crate) fn frame(&self, seconds: f32) -> f32 {
        seconds * self.rate as f32 / self.hop as f32
    }
}

pub(crate) struct Spectra {
    pub(crate) grid: Grid,
    pub(crate) data: Vec<Complex>,
}

impl Spectra {
    pub(crate) fn magnitudes(&self) -> Vec<f32> {
        self.data.iter().map(|c| c.norm().sqrt()).collect()
    }
}

fn hann(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / size as f64).cos()) as f32)
        .collect()
}

pub(crate) fn analyse(signal: &[f32], grid: Grid) -> Spectra {
    let window = hann(grid.size);
    let mut fft = ModifiedRealFft::new(grid.size);
    let half = grid.size / 2;
    let mut time = vec![0.0f32; grid.size];
    let mut data = vec![Complex::ZERO; grid.frames * grid.bins];
    for t in 0..grid.frames {
        let start = (t * grid.hop) as isize - half as isize;
        for (i, slot) in time.iter_mut().enumerate() {
            let at = start + i as isize;
            let sample = if at >= 0 && (at as usize) < signal.len() {
                signal[at as usize]
            } else {
                0.0
            };
            *slot = sample * window[i];
        }
        fft.fft(&time, &mut data[t * grid.bins..(t + 1) * grid.bins]);
    }
    Spectra { grid, data }
}

pub(crate) fn synthesise(spectra: &Spectra, gains: &[f32], fill: &[(usize, Complex)]) -> Vec<f32> {
    let grid = spectra.grid;
    let window = hann(grid.size);
    let mut fft = ModifiedRealFft::new(grid.size);
    let half = grid.size / 2;
    let mut out = vec![0.0f64; grid.len];
    let mut weight = vec![0.0f64; grid.len];
    let mut freq = vec![Complex::ZERO; grid.bins];
    let mut time = vec![0.0f32; grid.size];
    let scale = 1.0 / grid.size as f64;
    let mut extra = fill.iter().peekable();
    for t in 0..grid.frames {
        let row = &spectra.data[t * grid.bins..(t + 1) * grid.bins];
        let gain = &gains[t * grid.bins..(t + 1) * grid.bins];
        for ((f, c), g) in freq.iter_mut().zip(row).zip(gain) {
            *f = c.scale(*g);
        }
        while let Some((i, c)) = extra.next_if(|(i, _)| *i < (t + 1) * grid.bins) {
            let f = &mut freq[i - t * grid.bins];
            *f = f.add(*c);
        }
        fft.ifft(&freq, &mut time);
        let start = (t * grid.hop) as isize - half as isize;
        for (i, &w) in window.iter().enumerate() {
            let at = start + i as isize;
            if at >= 0 && (at as usize) < grid.len {
                out[at as usize] += f64::from(time[i] * w) * scale;
                weight[at as usize] += f64::from(w * w);
            }
        }
    }
    out.iter()
        .zip(&weight)
        .map(|(o, w)| if *w > 1e-9 { (o / w) as f32 } else { 0.0 })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_gains_give_the_input_back_at_its_length() {
        let signal: Vec<f32> = (0..9_001)
            .map(|i| ((i as f32 * 0.013).sin() + (i as f32 * 0.31).cos() * 0.25) * 0.5)
            .collect();
        for size in [1024, 4096] {
            let grid = Grid::new(size, 48_000, signal.len());
            let spectra = analyse(&signal, grid);
            let back = synthesise(&spectra, &vec![1.0; grid.frames * grid.bins], &[]);
            assert_eq!(back.len(), signal.len());
            let worst = back
                .iter()
                .zip(&signal)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-4, "{size}: {worst}");
        }
    }
}
