use std::f64::consts::PI;

use super::fft::{Complex, ModifiedRealFft};

const ALMOST_ZERO: f32 = 1e-30;

pub(crate) struct Stft {
    fft: ModifiedRealFft,
    channels: usize,
    block: usize,
    fft_samples: usize,
    bins: usize,
    input_length: usize,
    interval: usize,
    input_pos: usize,
    input: Vec<f32>,
    output_pos: usize,
    output: Vec<f32>,
    window_products: Vec<f32>,
    analysis_window: Vec<f32>,
    synthesis_window: Vec<f32>,
    analysis_offset: usize,
    synthesis_offset: usize,
    spectrum: Vec<Complex>,
    time: Vec<f32>,
}

impl Stft {
    pub(crate) fn new(
        channels: usize,
        block: usize,
        extra_history: usize,
        interval: usize,
    ) -> Stft {
        let fft_samples = ModifiedRealFft::fast_size_above(block.div_ceil(2)) * 2;
        let bins = fft_samples / 2;
        let input_length = block + extra_history;
        let mut stft = Stft {
            fft: ModifiedRealFft::new(fft_samples),
            channels,
            block,
            fft_samples,
            bins,
            input_length,
            interval,
            input_pos: 0,
            input: vec![0.0; input_length * channels],
            output_pos: 0,
            output: vec![0.0; block * channels],
            window_products: vec![0.0; block],
            analysis_window: vec![0.0; block],
            synthesis_window: vec![0.0; block],
            analysis_offset: 0,
            synthesis_offset: 0,
            spectrum: vec![Complex::ZERO; bins * channels],
            time: vec![0.0; fft_samples.max(block)],
        };
        stft.kaiser_windows();
        stft
    }

    fn kaiser_windows(&mut self) {
        let block = self.block;
        let kaiser = Kaiser::with_bandwidth(block as f64 / self.interval as f64);
        kaiser.fill(&mut self.synthesis_window);
        force_perfect_reconstruction(&mut self.synthesis_window, self.interval);
        self.analysis_window.copy_from_slice(&self.synthesis_window);
        self.analysis_offset = block / 2;
        self.synthesis_offset = block / 2;
        for i in 0..block {
            if self.analysis_window[i] > self.analysis_window[self.analysis_offset] {
                self.analysis_offset = i;
            }
            if self.synthesis_window[i] > self.synthesis_window[self.synthesis_offset] {
                self.synthesis_offset = i;
            }
        }
    }

    pub(crate) fn block_samples(&self) -> usize {
        self.block
    }

    pub(crate) fn fft_samples(&self) -> usize {
        self.fft_samples
    }

    pub(crate) fn interval(&self) -> usize {
        self.interval
    }

    pub(crate) fn bands(&self) -> usize {
        self.bins
    }

    pub(crate) fn analysis_latency(&self) -> usize {
        self.block - self.analysis_offset
    }

    pub(crate) fn synthesis_latency(&self) -> usize {
        self.synthesis_offset
    }

    pub(crate) fn bin_to_freq(&self, b: f32) -> f32 {
        (b + 0.5) / self.fft_samples as f32
    }

    pub(crate) fn freq_to_bin(&self, f: f32) -> f32 {
        f * self.fft_samples as f32 - 0.5
    }

    pub(crate) fn reset(&mut self, product_weight: f32) {
        self.input_pos = self.block;
        self.output_pos = 0;
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.spectrum.fill(Complex::ZERO);
        self.window_products.fill(0.0);
        self.add_window_product();
        let interval = self.interval;
        for i in (0..self.block.saturating_sub(interval)).rev() {
            self.window_products[i] += self.window_products[i + interval];
        }
        for v in &mut self.window_products {
            *v = *v * product_weight + ALMOST_ZERO;
        }
        self.move_output(interval);
    }

    pub(crate) fn write_input(&mut self, channel: usize, samples: &[f32]) {
        let len = self.input_length;
        let buffer = &mut self.input[channel * len..(channel + 1) * len];
        let pos = self.input_pos % len;
        for (i, &s) in samples.iter().enumerate() {
            buffer[(pos + i) % len] = s;
        }
    }

    pub(crate) fn move_input(&mut self, samples: usize) {
        self.input_pos = (self.input_pos + samples) % self.input_length;
    }

    pub(crate) fn finish_output(&mut self) {
        let mut max = 0.0f32;
        let block = self.block;
        for i in 0..block {
            let wp = &mut self.window_products[(self.output_pos + i) % block];
            max = std_max(*wp, max);
            *wp += max - *wp;
        }
    }

    pub(crate) fn read_output(&self, channel: usize, offset: usize, out: &mut [f32]) {
        let block = self.block;
        let buffer = &self.output[channel * block..(channel + 1) * block];
        let pos = (self.output_pos + offset) % block;
        for (i, o) in out.iter_mut().enumerate() {
            let i2 = (pos + i) % block;
            *o = buffer[i2] / self.window_products[i2];
        }
    }

    pub(crate) fn read_one(&self, channel: usize) -> f32 {
        let i2 = self.output_pos;
        self.output[channel * self.block + i2] / self.window_products[i2]
    }

    pub(crate) fn add_output(&mut self, channel: usize, samples: &[f32]) {
        let block = self.block;
        let len = samples.len().min(block);
        let buffer = &mut self.output[channel * block..(channel + 1) * block];
        for (i, &s) in samples[..len].iter().enumerate() {
            let i2 = (self.output_pos + i) % block;
            buffer[i2] += s * self.window_products[i2];
        }
    }

    pub(crate) fn move_output(&mut self, samples: usize) {
        let block = self.block;
        for i in 0..samples {
            let i2 = (self.output_pos + i) % block;
            for c in 0..self.channels {
                self.output[i2 + c * block] = 0.0;
            }
            self.window_products[i2] = ALMOST_ZERO;
        }
        self.output_pos = (self.output_pos + samples) % block;
    }

    pub(crate) fn spectrum(&self, channel: usize) -> &[Complex] {
        &self.spectrum[channel * self.bins..(channel + 1) * self.bins]
    }

    pub(crate) fn spectrum_mut(&mut self, channel: usize) -> &mut [Complex] {
        &mut self.spectrum[channel * self.bins..(channel + 1) * self.bins]
    }

    pub(crate) fn analyse(&mut self, channel: usize, samples_in_past: usize) {
        let len = self.input_length;
        let block = self.block;
        let offset = self.analysis_offset;
        let at = (len * 2 + self.input_pos - block - samples_in_past) % len;
        let size = self.time.len();
        let buffer = &self.input[channel * len..(channel + 1) * len];
        for i in 0..offset {
            self.time[i + size - offset] = buffer[(at + i) % len] * -self.analysis_window[i];
        }
        for i in offset..block {
            self.time[i - offset] = buffer[(at + i) % len] * self.analysis_window[i];
        }
        for i in block - offset..self.fft_samples - offset {
            self.time[i] = 0.0;
        }
        let bins = self.bins;
        self.fft.fft(
            &self.time,
            &mut self.spectrum[channel * bins..(channel + 1) * bins],
        );
    }

    pub(crate) fn synthesise(&mut self) {
        self.add_window_product();
        let block = self.block;
        let offset = self.synthesis_offset;
        let bins = self.bins;
        let fft_samples = self.fft_samples;
        for channel in 0..self.channels {
            self.fft.ifft(
                &self.spectrum[channel * bins..(channel + 1) * bins],
                &mut self.time,
            );
            let buffer = &mut self.output[channel * block..(channel + 1) * block];
            for i in 0..offset {
                let w = -self.synthesis_window[i];
                buffer[(self.output_pos + i) % block] += self.time[i + fft_samples - offset] * w;
            }
            for i in offset..block {
                let w = self.synthesis_window[i];
                buffer[(self.output_pos + i) % block] += self.time[i - offset] * w;
            }
        }
    }

    fn add_window_product(&mut self) {
        let shift = self.synthesis_offset as isize - self.analysis_offset as isize;
        let block = self.block as isize;
        let w_min = shift.max(0);
        let w_max = block.min(block + shift);
        let scaling = self.fft_samples as f32;
        for i in w_min..w_max {
            let wa = self.analysis_window[(i - shift) as usize];
            let ws = self.synthesis_window[i as usize];
            let bi = (self.output_pos + i as usize) % self.block;
            self.window_products[bi] += wa * ws * scaling;
        }
    }
}

pub(crate) fn std_max(a: f32, b: f32) -> f32 {
    if a < b { b } else { a }
}

pub(crate) fn std_min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

struct Kaiser {
    beta: f64,
    inv_b0: f64,
}

impl Kaiser {
    fn bessel0(x: f64) -> f64 {
        let mut result = 0.0;
        let mut term = 1.0;
        let mut m = 0.0;
        while term > 1e-4 {
            result += term;
            m += 1.0;
            term *= (x * x) / (4.0 * m * m);
        }
        result
    }

    fn with_bandwidth(bandwidth: f64) -> Kaiser {
        let bandwidth = bandwidth
            + 8.0 / ((bandwidth + 3.0) * (bandwidth + 3.0))
            + 0.25 * (3.0 - bandwidth).max(0.0);
        let bandwidth = bandwidth.max(2.0);
        let alpha = (bandwidth * bandwidth * 0.25 - 1.0).sqrt();
        let beta = alpha * PI;
        Kaiser {
            beta,
            inv_b0: 1.0 / Self::bessel0(beta),
        }
    }

    fn fill(&self, data: &mut [f32]) {
        let size = data.len();
        let inv_size = 1.0 / size as f64;
        let offset = if size & 1 == 1 { 1 } else { 0 };
        for (i, d) in data.iter_mut().enumerate() {
            let r = (2 * i + offset) as f64 * inv_size - 1.0;
            let arg = (1.0 - r * r).sqrt();
            *d = (Self::bessel0(self.beta * arg) * self.inv_b0) as f32;
        }
    }
}

fn force_perfect_reconstruction(data: &mut [f32], interval: usize) {
    for i in 0..interval {
        let mut sum2 = 0.0f64;
        for index in (i..data.len()).step_by(interval) {
            sum2 += (data[index] * data[index]) as f64;
        }
        let factor = 1.0 / sum2.sqrt();
        for index in (i..data.len()).step_by(interval) {
            data[index] = (data[index] as f64 * factor) as f32;
        }
    }
}
