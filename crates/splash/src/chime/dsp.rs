use pfx_core::sim::{cos, exp, sin, splitmix64};

const SINE_STEPS: usize = 4096;

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

pub fn sine(phase: f64) -> f32 {
    let phase = phase - phase.floor();
    let x = phase * SINE_STEPS as f64;
    let index = (x as usize).min(SINE_STEPS - 1);
    let frac = (x - index as f64) as f32;
    let a = SINE[index];
    a + (SINE[index + 1] - a) * frac
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Osc {
    phase: f64,
}

impl Osc {
    pub fn at(phase: f64) -> Osc {
        Osc { phase }
    }

    pub fn sine(&mut self, hz: f64, rate: f64) -> f32 {
        let out = sine(self.phase);
        self.step(hz, rate);
        out
    }

    pub fn saw(&mut self, hz: f64, rate: f64) -> f32 {
        let dt = (hz / rate).clamp(0.0, 0.45);
        let t = self.phase;
        let out = 2.0 * t - 1.0 - blep(t, dt);
        self.step(hz, rate);
        out as f32
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    fn step(&mut self, hz: f64, rate: f64) {
        self.phase += hz / rate;
        self.phase -= self.phase.floor();
    }
}

fn blep(t: f64, dt: f64) -> f64 {
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

pub struct Noise(u64);

impl Noise {
    pub fn new(seed: u64) -> Noise {
        Noise(seed)
    }

    pub fn next(&mut self) -> f32 {
        let unit = (splitmix64(&mut self.0) >> 40) as f32 / (1u64 << 24) as f32;
        unit * 2.0 - 1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Low,
    Band,
}

pub struct Biquad {
    pass: Pass,
    coef: [f64; 5],
    state: [f64; 2],
}

impl Biquad {
    pub fn new(pass: Pass) -> Biquad {
        Biquad {
            pass,
            coef: [0.0; 5],
            state: [0.0; 2],
        }
    }

    pub fn tune(&mut self, hz: f64, q: f64, rate: f64) {
        let w0 = std::f64::consts::TAU * hz.clamp(10.0, rate * 0.45) / rate;
        let (s, c) = (sin(w0), cos(w0));
        let alpha = s / (2.0 * q.max(0.1));
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match self.pass {
            Pass::Low => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0),
            Pass::Band => (alpha, 0.0, -alpha),
        };
        self.coef = [b0 / a0, b1 / a0, b2 / a0, -2.0 * c / a0, (1.0 - alpha) / a0];
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let [b0, b1, b2, a1, a2] = self.coef;
        let x = f64::from(x);
        let y = b0 * x + self.state[0];
        self.state[0] = b1 * x - a1 * y + self.state[1];
        self.state[1] = b2 * x - a2 * y;
        y as f32
    }
}

pub fn decay(seconds: f64, rate: f64) -> f32 {
    exp(-1.0 / (seconds.max(1e-4) * rate)) as f32
}

pub fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub fn pan(sample: f32, at: f32) -> [f32; 2] {
    let at = at.clamp(-1.0, 1.0);
    let quarter = f64::from(at + 1.0) * 0.125;
    [sample * sine(quarter + 0.25), sample * sine(quarter)]
}
