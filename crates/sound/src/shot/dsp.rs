use super::RATE;
use crate::sound::Rng;
use crate::synth::{Synth, Wave, sine};

pub(crate) struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    tuned: [f32; 2],
}

pub(crate) struct Bands {
    pub(crate) low: f32,
    pub(crate) band: f32,
    pub(crate) high: f32,
}

impl Svf {
    pub(crate) fn new(hz: f32, q: f32) -> Self {
        let mut svf = Self {
            ic1: 0.0,
            ic2: 0.0,
            a1: 1.0,
            a2: 0.0,
            a3: 0.0,
            k: 1.0,
            tuned: [f32::NAN; 2],
        };
        svf.tune(hz, q);
        svf
    }

    pub(crate) fn tune(&mut self, hz: f32, q: f32) {
        if self.tuned == [hz, q] {
            return;
        }
        self.tuned = [hz, q];
        let hz = hz.clamp(10.0, RATE as f32 * 0.45);
        let w = std::f64::consts::PI * f64::from(hz) / f64::from(RATE);
        let g = (pfx_core::sim::sin(w) / pfx_core::sim::cos(w)) as f32;
        let k = 1.0 / q.max(0.05);
        self.k = k;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub(crate) fn tick(&mut self, x: f32) -> Bands {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        Bands {
            low: v2,
            band: v1,
            high: x - self.k * v1 - v2,
        }
    }

    pub(crate) fn low(&mut self, x: f32) -> f32 {
        self.tick(x).low
    }

    pub(crate) fn band(&mut self, x: f32) -> f32 {
        self.tick(x).band
    }

    pub(crate) fn high(&mut self, x: f32) -> f32 {
        self.tick(x).high
    }
}

pub(crate) struct OnePole {
    alpha: f32,
    last: f32,
}

impl OnePole {
    pub(crate) fn new(hz: f32) -> Self {
        let w = std::f32::consts::TAU * hz / RATE as f32;
        Self {
            alpha: w / (w + 1.0),
            last: 0.0,
        }
    }

    pub(crate) fn tick(&mut self, x: f32) -> f32 {
        self.last += (x - self.last) * self.alpha;
        self.last
    }
}

pub(crate) struct Noise {
    rng: Rng,
}

impl Noise {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
        }
    }

    pub(crate) fn white(&mut self) -> f32 {
        self.rng.signed()
    }

    pub(crate) fn unit(&mut self) -> f32 {
        self.rng.signed() * 0.5 + 0.5
    }

    pub(crate) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    pub(crate) fn seed(&mut self) -> u64 {
        self.rng.seed()
    }
}

pub(crate) struct Pink(Synth);

impl Pink {
    pub(crate) fn new(seed: u64) -> Self {
        Self(Synth::new(Wave::Pink, 0.0, seed))
    }

    pub(crate) fn tick(&mut self) -> f32 {
        self.0.next(1.0, RATE)
    }
}

pub(crate) struct Wander {
    noise: Noise,
    filter: Svf,
    scale: f32,
}

impl Wander {
    pub(crate) fn new(seed: u64, hz: f32) -> Self {
        Self {
            noise: Noise::new(seed),
            filter: Svf::new(hz, 0.5),
            scale: 2.2 * pfx_core::sim::sqrtf(RATE as f32 / hz.max(0.01)),
        }
    }

    pub(crate) fn tick(&mut self) -> f32 {
        let x = self.noise.white();
        (self.filter.low(x) * self.scale).clamp(-1.0, 1.0)
    }
}

pub(crate) fn pan(p: f32) -> (f32, f32) {
    let phase = f64::from((p.clamp(-1.0, 1.0) + 1.0) * 0.125);
    (sine(phase + 0.25), sine(phase))
}

pub(crate) fn sin_turns(turns: f32) -> f32 {
    sine(f64::from(fract(turns)))
}

pub(crate) fn fract(x: f32) -> f32 {
    let f = x - x as i32 as f32;
    if f < 0.0 { f + 1.0 } else { f }
}

pub(crate) fn exp_seconds(x: f32) -> f32 {
    pfx_core::sim::expf(x)
}

pub(crate) fn decay_per_sample(seconds: f32) -> f32 {
    exp_seconds(-1.0 / (seconds.max(1e-4) * RATE as f32))
}

pub(crate) fn smooth_step(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}
