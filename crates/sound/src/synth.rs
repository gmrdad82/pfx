use crate::channel::Ramp;
use crate::sound::Rng;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Square,
    Saw,
    White,
    Pink,
    Triangle,
    Pulse(f32),
}

pub(crate) struct Synth {
    wave: Wave,
    pub(crate) frequency: Ramp,
    phase: f64,
    rng: Rng,
    pink: [f32; 7],
}

impl Synth {
    pub(crate) fn new(wave: Wave, frequency: f32, seed: u64) -> Self {
        Self {
            wave,
            frequency: Ramp::new(frequency.max(0.0)),
            phase: 0.0,
            rng: Rng::new(seed),
            pink: [0.0; 7],
        }
    }

    pub(crate) fn next(&mut self, ratio: f32, rate: u32) -> f32 {
        let rate = f64::from(rate.max(1));
        let hz = (f64::from(self.frequency.next()) * f64::from(ratio)).clamp(0.0, rate * 0.45);
        let dt = hz / rate;
        let phase = self.phase;
        let sample = match self.wave {
            Wave::Sine => f64::from(sine(phase)),
            Wave::White => f64::from(self.rng.signed()),
            Wave::Pink => f64::from(self.pink_sample()),
            Wave::Saw => 2.0 * phase - 1.0 - poly_blep(phase, dt),
            Wave::Square => {
                let raw = if phase < 0.5 { 1.0 } else { -1.0 };
                raw + poly_blep(phase, dt) - poly_blep((phase + 0.5).fract(), dt)
            }
            Wave::Triangle => triangle(phase, dt),
            Wave::Pulse(width) => pulse(phase, dt, width),
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        sample as f32
    }
}

impl Synth {
    pub(crate) fn fill(&mut self, out: &mut [f32], rate: u32) {
        let mut i = 0;
        while i < out.len() && !self.frequency.is_settled() {
            out[i] = self.next(1.0, rate);
            i += 1;
        }
        let rest = &mut out[i..];
        let per_second = f64::from(rate.max(1));
        let hz = f64::from(self.frequency.current()).clamp(0.0, per_second * 0.45);
        let dt = hz / per_second;
        let phase = self.phase;
        self.phase = match self.wave {
            Wave::Sine => run(rest, phase, dt, |p| f64::from(sine(p))),
            Wave::Saw => run(rest, phase, dt, |p| 2.0 * p - 1.0 - poly_blep(p, dt)),
            Wave::Square => run(rest, phase, dt, |p| {
                let raw = if p < 0.5 { 1.0 } else { -1.0 };
                raw + poly_blep(p, dt) - poly_blep((p + 0.5).fract(), dt)
            }),
            Wave::Triangle => run(rest, phase, dt, |p| triangle(p, dt)),
            Wave::Pulse(width) => run(rest, phase, dt, |p| pulse(p, dt, width)),
            Wave::White | Wave::Pink => {
                for sample in rest.iter_mut() {
                    *sample = self.next(1.0, rate);
                }
                self.phase
            }
        };
    }
}

fn run(out: &mut [f32], mut phase: f64, dt: f64, wave: impl Fn(f64) -> f64) -> f64 {
    for sample in out.iter_mut() {
        *sample = wave(phase) as f32;
        phase += dt;
        if phase >= 1.0 {
            phase -= 1.0;
        }
    }
    phase
}

impl Synth {
    fn pink_sample(&mut self) -> f32 {
        let white = self.rng.signed();
        let b = &mut self.pink;
        b[0] = 0.99886 * b[0] + white * 0.055_517_9;
        b[1] = 0.99332 * b[1] + white * 0.075_075_9;
        b[2] = 0.96900 * b[2] + white * 0.153_852;
        b[3] = 0.86650 * b[3] + white * 0.310_485_6;
        b[4] = 0.55000 * b[4] + white * 0.532_952_2;
        b[5] = -0.7616 * b[5] - white * 0.016_898;
        let sum = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.5362;
        b[6] = white * 0.115_926;
        sum * PINK_SCALE
    }
}

const PINK_SCALE: f32 = 0.11;

const SINE_STEPS: usize = 2048;

static SINE: [f32; SINE_STEPS + 1] = sine_table();

const fn sine_table() -> [f32; SINE_STEPS + 1] {
    let mut table = [0.0f32; SINE_STEPS + 1];
    let mut i = 0;
    while i <= SINE_STEPS {
        table[i] = taylor_sine(std::f64::consts::TAU * i as f64 / SINE_STEPS as f64) as f32;
        i += 1;
    }
    table
}

pub(crate) const fn taylor_sine(x: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let x = if x > 1.5 * pi {
        x - 2.0 * pi
    } else if x > 0.5 * pi {
        pi - x
    } else {
        x
    };
    let square = x * x;
    let mut term = x;
    let mut sum = x;
    let mut n = 1;
    while n < 14 {
        term = -term * square / ((2 * n) * (2 * n + 1)) as f64;
        sum += term;
        n += 1;
    }
    sum
}

pub(crate) fn sine(phase: f64) -> f32 {
    let x = phase * SINE_STEPS as f64;
    let index = (x as usize).min(SINE_STEPS - 1);
    let frac = (x - index as f64) as f32;
    let a = SINE[index];
    a + (SINE[index + 1] - a) * frac
}

fn poly_blep(t: f64, dt: f64) -> f64 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

const PULSE_MIN: f32 = 0.01;
const PULSE_MAX: f32 = 0.99;

pub(crate) fn pulse(phase: f64, dt: f64, width: f32) -> f64 {
    let width = if width.is_nan() {
        0.5
    } else {
        f64::from(width.clamp(PULSE_MIN, PULSE_MAX))
    };
    let raw = if phase < width { 1.0 } else { -1.0 };
    raw + poly_blep(phase, dt) - poly_blep((phase + 1.0 - width).fract(), dt) + 1.0 - 2.0 * width
}

pub(crate) fn triangle(phase: f64, dt: f64) -> f64 {
    let raw = if phase < 0.25 {
        4.0 * phase
    } else if phase < 0.75 {
        2.0 - 4.0 * phase
    } else {
        4.0 * phase - 4.0
    };
    raw + 8.0 * dt * (poly_blamp(phase - 0.75, dt) - poly_blamp(phase - 0.25, dt))
}

fn poly_blamp(offset: f64, dt: f64) -> f64 {
    if dt <= 0.0 {
        return 0.0;
    }
    let offset = offset - (offset + 0.5).floor();
    let x = offset / dt;
    if x <= -1.0 || x >= 1.0 {
        0.0
    } else if x < 0.0 {
        let y = 1.0 + x;
        y * y * y / 6.0
    } else {
        let y = 1.0 - x;
        y * y * y / 6.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliasing(wave: impl Fn(f64, f64) -> f64, bin: usize) -> f64 {
        let size = 4_800;
        let dt = bin as f64 / size as f64;
        let samples: Vec<f64> = (0..size)
            .map(|n| wave((n as f64 * dt).fract(), dt))
            .collect();
        let total: f64 = samples.iter().map(|s| s * s).sum();
        let mut harmonics = 0.0;
        let mut k = bin;
        while k < size / 2 {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, s) in samples.iter().enumerate() {
                let angle = std::f64::consts::TAU * (k * n % size) as f64 / size as f64;
                re += s * angle.cos();
                im -= s * angle.sin();
            }
            harmonics += 2.0 * (re * re + im * im) / size as f64;
            k += 2 * bin;
        }
        (total - harmonics) / total
    }

    #[test]
    fn the_triangle_is_band_limited_and_a_half_pulse_is_the_square() {
        for bin in [123, 431] {
            let naive = aliasing(|p, _| triangle(p, 0.0), bin);
            let blamp = aliasing(triangle, bin);
            assert!(
                blamp < naive * 0.5,
                "bin {bin}: {blamp} against naive {naive}"
            );
        }
        let mut square = Synth::new(Wave::Square, 3_000.0, 0);
        let mut pulse = Synth::new(Wave::Pulse(0.5), 3_000.0, 0);
        for _ in 0..1_000 {
            assert_eq!(square.next(1.0, 48_000), pulse.next(1.0, 48_000));
        }
    }
}
