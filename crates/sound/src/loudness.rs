use pfx_core::sim::{cos, ln, pow, sin};

use crate::peak::{decibels, true_peak};

const ABSOLUTE_GATE: f64 = 0.000_000_117_246_530_458_229_81;
const RELATIVE_GATE_DB: f64 = -10.0;
const MOMENTARY_BLOCKS: usize = 4;
const SHORT_TERM_BLOCKS: usize = 30;
const SURROUND: f64 = 1.41;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    pub short_term_max: f64,
    pub momentary_max: f64,
    pub integrated: f64,
    pub true_peak_db: f32,
}

#[derive(Clone, Debug)]
pub struct Loudness {
    channels: usize,
    b: [f64; 5],
    a: [f64; 5],
    state: Vec<[f64; 5]>,
    weights: Vec<f64>,
    hundred: usize,
    window: usize,
    data: Vec<f64>,
    index: usize,
    needed: usize,
    sums: Vec<f64>,
    partial: Vec<f64>,
    gated: Vec<f64>,
}

impl Loudness {
    pub fn new(channels: usize, rate: u32) -> Loudness {
        let channels = channels.max(1);
        let rate = rate.max(1);
        let hundred = ((rate as usize + 5) / 10).max(1);
        let window = SHORT_TERM_BLOCKS * hundred;
        let (b, a) = coefficients(f64::from(rate));
        Loudness {
            channels,
            b,
            a,
            state: vec![[0.0; 5]; channels],
            weights: weights(channels),
            hundred,
            window,
            data: vec![0.0; window * channels],
            index: 0,
            needed: hundred * MOMENTARY_BLOCKS,
            sums: vec![0.0; SHORT_TERM_BLOCKS * channels],
            partial: vec![0.0; channels],
            gated: Vec::new(),
        }
    }

    pub fn add(&mut self, samples: &[f32]) {
        let frames = samples.len() / self.channels;
        let mut done = 0;
        while done < frames {
            let take = (frames - done).min(self.needed);
            self.filter(&samples[done * self.channels..(done + take) * self.channels]);
            done += take;
            self.needed -= take;
            if self.needed == 0 {
                let energy = self.energy(self.hundred * MOMENTARY_BLOCKS);
                if energy >= ABSOLUTE_GATE {
                    self.gated.push(energy);
                }
                self.needed = self.hundred;
            }
        }
    }

    pub fn momentary(&self) -> f64 {
        lufs(self.energy(self.hundred * MOMENTARY_BLOCKS))
    }

    pub fn short_term(&self) -> f64 {
        lufs(self.energy(self.window))
    }

    pub fn integrated(&self) -> f64 {
        if self.gated.is_empty() {
            return f64::NEG_INFINITY;
        }
        let mean = self.gated.iter().sum::<f64>() / self.gated.len() as f64;
        let threshold = mean * pow(10.0, RELATIVE_GATE_DB / 10.0);
        let mut count = 0usize;
        let mut sum = 0.0;
        for energy in self.gated.iter().filter(|e| **e >= threshold) {
            count += 1;
            sum += energy;
        }
        if count == 0 {
            return f64::NEG_INFINITY;
        }
        lufs(sum / count as f64)
    }

    fn filter(&mut self, samples: &[f32]) {
        let channels = self.channels;
        let frames = samples.len() / channels;
        let (h, window) = (self.hundred, self.window);
        let [b0, b1, b2, b3, b4] = self.b;
        let [_, a1, a2, a3, a4] = self.a;
        let mut n = 0;
        while n < frames {
            let at = self.index + n;
            let stop = frames.min(n + h - at % h);
            for (m, frame) in samples[n * channels..stop * channels]
                .chunks_exact(channels)
                .enumerate()
            {
                for (c, x) in frame.iter().enumerate() {
                    if self.weights[c] == 0.0 {
                        continue;
                    }
                    let s = &mut self.state[c];
                    s[0] = f64::from(*x) - (a1 * s[1] + (a2 * s[2] + a3 * s[3] + a4 * s[4]));
                    let y = b0 * s[0] + b1 * s[1] + b2 * s[2] + b3 * s[3] + b4 * s[4];
                    s[4] = s[3];
                    s[3] = s[2];
                    s[2] = s[1];
                    s[1] = s[0];
                    self.data[c * window + at + m] = y;
                    self.partial[c] += y * y;
                }
            }
            n = stop;
            if (self.index + n).is_multiple_of(h) {
                let block = (self.index + n) / h - 1;
                for c in 0..channels {
                    self.sums[c * SHORT_TERM_BLOCKS + block] = self.partial[c];
                    self.partial[c] = 0.0;
                }
                for state in &mut self.state {
                    for v in state.iter_mut() {
                        if v.abs() < f64::EPSILON {
                            *v = 0.0;
                        }
                    }
                }
            }
        }
        self.index += frames;
        if self.index == window {
            self.index = 0;
        }
    }

    fn energy(&self, frames: usize) -> f64 {
        let h = self.hundred;
        let current = self.index / h;
        let rest = frames - self.index % h;
        let (full, left) = (rest / h, rest % h);
        let mut sum = 0.0;
        for c in 0..self.channels {
            let weight = self.weights[c];
            if weight == 0.0 {
                continue;
            }
            let lane = &self.data[c * self.window..(c + 1) * self.window];
            let sums = &self.sums[c * SHORT_TERM_BLOCKS..(c + 1) * SHORT_TERM_BLOCKS];
            let mut lane_sum = self.partial[c];
            for k in 1..=full {
                lane_sum += sums[(current + SHORT_TERM_BLOCKS - k) % SHORT_TERM_BLOCKS];
            }
            if left > 0 {
                let end = (current + SHORT_TERM_BLOCKS - full - 1) % SHORT_TERM_BLOCKS * h + h;
                for v in &lane[end - left..end] {
                    lane_sum += v * v;
                }
            }
            sum += lane_sum * weight;
        }
        sum / frames as f64
    }
}

pub fn loudness(samples: &[f32], channels: usize, rate: u32) -> Measure {
    let channels = channels.max(1);
    let mut meter = Loudness::new(channels, rate);
    let mut short_term_max = f64::NEG_INFINITY;
    let mut momentary_max = f64::NEG_INFINITY;
    for block in samples.chunks(meter.hundred * channels) {
        meter.add(block);
        short_term_max = short_term_max.max(meter.short_term());
        momentary_max = momentary_max.max(meter.momentary());
    }
    Measure {
        short_term_max,
        momentary_max,
        integrated: meter.integrated(),
        true_peak_db: decibels(true_peak(samples, channels)),
    }
}

fn lufs(energy: f64) -> f64 {
    if energy <= 0.0 {
        return f64::NEG_INFINITY;
    }
    10.0 * (ln(energy) / std::f64::consts::LN_10) - 0.691
}

fn weights(channels: usize) -> Vec<f64> {
    match channels {
        4 => vec![1.0, 1.0, SURROUND, SURROUND],
        5 => vec![1.0, 1.0, 1.0, SURROUND, SURROUND],
        _ => {
            let map = [1.0, 1.0, 1.0, 0.0, SURROUND, SURROUND];
            (0..channels)
                .map(|c| map.get(c).copied().unwrap_or(0.0))
                .collect()
        }
    }
}

fn tan(x: f64) -> f64 {
    sin(x) / cos(x)
}

fn coefficients(rate: f64) -> ([f64; 5], [f64; 5]) {
    let f0 = 1681.974450955533;
    let gain = 3.999843853973347;
    let q = 0.7071752369554196;
    let k = tan(std::f64::consts::PI * f0 / rate);
    let vh = pow(10.0, gain / 20.0);
    let vb = pow(vh, 0.4996667741545416);
    let rb = [1.0, -2.0, 1.0];
    let a0 = 1.0 + k / q + k * k;
    let pb = [
        (vh + vb * k / q + k * k) / a0,
        2.0 * (k * k - vh) / a0,
        (vh - vb * k / q + k * k) / a0,
    ];
    let pa = [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0];
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = tan(std::f64::consts::PI * f0 / rate);
    let ra = [
        1.0,
        2.0 * (k * k - 1.0) / (1.0 + k / q + k * k),
        (1.0 - k / q + k * k) / (1.0 + k / q + k * k),
    ];
    (
        [
            pb[0] * rb[0],
            pb[0] * rb[1] + pb[1] * rb[0],
            pb[0] * rb[2] + pb[1] * rb[1] + pb[2] * rb[0],
            pb[1] * rb[2] + pb[2] * rb[1],
            pb[2] * rb[2],
        ],
        [
            pa[0] * ra[0],
            pa[0] * ra[1] + pa[1] * ra[0],
            pa[0] * ra[2] + pa[1] * ra[1] + pa[2] * ra[0],
            pa[1] * ra[2] + pa[2] * ra[1],
            pa[2] * ra[2],
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ebur128::{EbuR128, Mode};

    fn signal(channels: usize, rate: u32) -> Vec<f32> {
        let mut state = 0x5eed_u64;
        let frames = rate as usize * 9;
        let mut out = Vec::with_capacity(frames * channels);
        for n in 0..frames {
            let t = n as f64 / f64::from(rate);
            let level = match n * 10 / frames {
                0..=1 => 0.5,
                2 => 0.0,
                3..=5 => 0.05,
                6 => 0.000_01,
                _ => 0.8,
            };
            for c in 0..channels {
                let noise =
                    (pfx_core::sim::splitmix64(&mut state) >> 40) as f64 / (1u64 << 24) as f64;
                let tone = sin(std::f64::consts::TAU * (220.0 + 110.0 * c as f64) * t);
                out.push((level * (0.6 * tone + 0.4 * (noise * 2.0 - 1.0))) as f32);
            }
        }
        out
    }

    #[test]
    fn the_meter_reads_what_ebur128_reads() {
        for (channels, rate) in [(2usize, 48_000u32), (1, 44_100), (6, 48_000)] {
            let samples = signal(channels, rate);
            let mut ours = Loudness::new(channels, rate);
            let mut theirs =
                EbuR128::new(channels as u32, rate, Mode::M | Mode::S | Mode::I).unwrap();
            for block in samples.chunks(1_237 * channels) {
                ours.add(block);
                theirs.add_frames_f32(block).unwrap();
                let pairs = [
                    (ours.momentary(), theirs.loudness_momentary().unwrap()),
                    (ours.short_term(), theirs.loudness_shortterm().unwrap()),
                    (ours.integrated(), theirs.loudness_global().unwrap()),
                ];
                for (a, b) in pairs {
                    assert!(
                        a == b || (a - b).abs() < 1e-6 || (a < -100.0 && b < -100.0),
                        "{channels} ch at {rate}: {a} against {b}"
                    );
                }
            }
        }
        let measure = loudness(&signal(2, 48_000), 2, 48_000);
        assert!(measure.integrated.is_finite());
        assert!(measure.short_term_max >= measure.integrated);
        assert!(measure.true_peak_db <= 0.0);
    }
}
