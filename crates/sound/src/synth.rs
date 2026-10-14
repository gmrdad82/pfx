use crate::channel::Ramp;
use crate::sound::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Square,
    Saw,
    White,
    Pink,
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
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        sample as f32
    }
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

const fn taylor_sine(x: f64) -> f64 {
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
