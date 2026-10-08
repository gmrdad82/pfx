pub(crate) const HALF: usize = 16;
pub(crate) const MIN_RATIO: f32 = 1.0 / 16.0;
pub(crate) const MAX_RATIO: f32 = 8.0;
pub(crate) const MAX_TAPS: usize = 2 * HALF * (MAX_RATIO as usize) + 2;

const STEPS: usize = 256;
const CUTOFF: f64 = 0.9;
const BETA: f64 = 7.0;

pub(crate) struct Kernel {
    table: Vec<f32>,
}

pub(crate) struct Taps {
    pub(crate) first: i64,
    pub(crate) count: usize,
    pub(crate) weights: [f32; MAX_TAPS],
}

impl Kernel {
    pub(crate) fn new() -> Self {
        let len = HALF * STEPS + 2;
        let table = (0..len)
            .map(|i| {
                let x = i as f64 / STEPS as f64;
                (windowed_sinc(x)) as f32
            })
            .collect();
        Self { table }
    }

    pub(crate) fn reach(scale: f32) -> u64 {
        (HALF as f32 * scale.max(1.0)).ceil() as u64 + 1
    }

    fn weight(&self, distance: f32) -> f32 {
        let x = distance * STEPS as f32;
        let index = x as usize;
        if index + 1 >= self.table.len() {
            return 0.0;
        }
        let frac = x - index as f32;
        self.table[index] + (self.table[index + 1] - self.table[index]) * frac
    }

    pub(crate) fn taps(&self, position: f64, scale: f32) -> Taps {
        let scale = scale.clamp(1.0, MAX_RATIO);
        let reach = f64::from(HALF as f32 * scale);
        let first = (position - reach).floor() as i64 + 1;
        let last = (position + reach).floor() as i64;
        let count = ((last - first + 1).max(0) as usize).min(MAX_TAPS);
        let inverse = 1.0 / scale;
        let mut weights = [0.0f32; MAX_TAPS];
        let mut sum = 0.0f32;
        for (j, slot) in weights.iter_mut().enumerate().take(count) {
            let distance = ((first + j as i64) as f64 - position).abs() as f32 * inverse;
            let weight = self.weight(distance);
            *slot = weight;
            sum += weight;
        }
        if sum != 0.0 {
            for slot in weights.iter_mut().take(count) {
                *slot /= sum;
            }
        }
        Taps {
            first,
            count,
            weights,
        }
    }
}

fn windowed_sinc(x: f64) -> f64 {
    let half = HALF as f64;
    if x >= half {
        return 0.0;
    }
    let y = CUTOFF * x;
    let sinc = if y == 0.0 {
        1.0
    } else {
        (std::f64::consts::PI * y).sin() / (std::f64::consts::PI * y)
    };
    let t = x / half;
    let window = bessel_i0(BETA * (1.0 - t * t).max(0.0).sqrt()) / bessel_i0(BETA);
    CUTOFF * sinc * window
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let quarter = x * x / 4.0;
    for k in 1..64 {
        term *= quarter / (k * k) as f64;
        sum += term;
        if term < sum * 1e-16 {
            break;
        }
    }
    sum
}

#[cfg(feature = "vorbis")]
pub(crate) struct StreamResampler {
    kernel: Kernel,
    step: f64,
    scale: f32,
    channels: usize,
    pending: Vec<f32>,
    base: u64,
    position: f64,
    passthrough: bool,
}

#[cfg(feature = "vorbis")]
impl StreamResampler {
    pub(crate) fn new(from: u32, to: u32, channels: usize) -> Self {
        let step = f64::from(from) / f64::from(to);
        Self {
            kernel: Kernel::new(),
            step,
            scale: (step as f32).clamp(1.0, MAX_RATIO),
            channels: channels.max(1),
            pending: Vec::new(),
            base: 0,
            position: 0.0,
            passthrough: from == to,
        }
    }

    pub(crate) fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.pending.extend_from_slice(input);
        self.emit(out, false);
        self.trim();
    }

    pub(crate) fn finish(&mut self, out: &mut Vec<f32>) {
        if self.passthrough {
            return;
        }
        self.emit(out, true);
        self.pending.clear();
    }

    fn emit(&mut self, out: &mut Vec<f32>, flush: bool) {
        let frames = (self.pending.len() / self.channels) as u64;
        let end = self.base + frames;
        let reach = Kernel::reach(self.scale);
        loop {
            let floor = self.position.floor() as u64;
            if flush {
                if self.position >= end as f64 {
                    break;
                }
            } else if floor + reach + 1 > end {
                break;
            }
            let taps = self.kernel.taps(self.position, self.scale);
            for channel in 0..self.channels {
                let mut sum = 0.0f32;
                for (j, weight) in taps.weights[..taps.count].iter().enumerate() {
                    let k = taps.first + j as i64;
                    if k < self.base as i64 || k >= end as i64 {
                        continue;
                    }
                    let index = (k as u64 - self.base) as usize * self.channels + channel;
                    sum += self.pending[index] * weight;
                }
                out.push(sum);
            }
            self.position += self.step;
        }
    }

    fn trim(&mut self) {
        let reach = Kernel::reach(self.scale);
        let keep = (self.position.floor() as u64).saturating_sub(reach + 1);
        if keep > self.base {
            let drop = ((keep - self.base) as usize).min(self.pending.len() / self.channels);
            self.pending.drain(..drop * self.channels);
            self.base += drop as u64;
        }
    }
}

#[cfg(test)]
pub(crate) fn tone_level(samples: &[f32], rate: f32, frequency: f32) -> f32 {
    let n = samples.len();
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    let mut window_sum = 0.0f64;
    for (i, sample) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let phase = std::f64::consts::TAU * f64::from(frequency) * i as f64 / f64::from(rate);
        re += f64::from(*sample) * window * phase.cos();
        im += f64::from(*sample) * window * phase.sin();
        window_sum += window;
    }
    (2.0 * (re * re + im * im).sqrt() / window_sum) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_weights_sum_to_one_at_every_phase() {
        let kernel = Kernel::new();
        for scale in [1.0, 1.5, 2.0, 4.0] {
            for k in 0..20 {
                let taps = kernel.taps(100.0 + f64::from(k) / 20.0, scale);
                let sum: f32 = taps.weights[..taps.count].iter().sum();
                assert!((sum - 1.0).abs() < 1e-5, "{sum}");
            }
        }
    }

    #[test]
    fn a_half_frame_position_reads_between_the_samples() {
        let kernel = Kernel::new();
        let tone = |t: f64| (std::f64::consts::TAU * 1000.0 * t / 48_000.0).sin();
        let taps = kernel.taps(500.25, 1.0);
        let value: f64 = (0..taps.count)
            .map(|j| tone((taps.first + j as i64) as f64) * f64::from(taps.weights[j]))
            .sum();
        assert!((value - tone(500.25)).abs() < 1e-3, "{value}");
    }

    #[cfg(feature = "vorbis")]
    #[test]
    fn a_stream_resample_matches_the_tone_it_carries() {
        let rate_in = 44_100u32;
        let rate_out = 48_000u32;
        let input: Vec<f32> = (0..20_000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / rate_in as f32).sin())
            .collect();
        let mut resampler = StreamResampler::new(rate_in, rate_out, 1);
        let mut out = Vec::new();
        for chunk in input.chunks(777) {
            resampler.push(chunk, &mut out);
        }
        resampler.finish(&mut out);
        let expected = 20_000.0 * 48_000.0 / 44_100.0;
        assert!((out.len() as f64 - expected).abs() < 2.0, "{}", out.len());
        let worst = out
            .iter()
            .enumerate()
            .skip(64)
            .take(out.len() - 128)
            .map(|(i, s)| {
                let want =
                    0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / rate_out as f32).sin();
                (s - want).abs()
            })
            .fold(0.0, f32::max);
        assert!(worst < 2e-3, "{worst}");
    }
}
