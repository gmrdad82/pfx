use crate::clock::HitStop;
use crate::ease::Ease;
use crate::sim::mix64;

pub const FLASHES_PER_SECOND: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub low: f32,
    pub high: f32,
    pub ease: Ease,
}

impl Curve {
    pub const ONE: Self = Self::constant(1.0);

    pub const fn constant(value: f32) -> Self {
        Self {
            low: value,
            high: value,
            ease: Ease::Linear,
        }
    }

    pub const fn new(low: f32, high: f32, ease: Ease) -> Self {
        Self { low, high, ease }
    }

    pub fn at(&self, intensity: f32) -> f32 {
        let t = if intensity.is_finite() {
            intensity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.low + (self.high - self.low) * self.ease.at(t)
    }
}

impl Default for Curve {
    fn default() -> Self {
        Self::ONE
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Response {
    pub gain: Curve,
    pub rate: Curve,
}

impl Response {
    pub const STEADY: Self = Self {
        gain: Curve::ONE,
        rate: Curve::ONE,
    };

    pub fn gain(&self, intensity: f32) -> f32 {
        self.gain.at(intensity).max(0.0)
    }

    pub fn rate(&self, intensity: f32) -> f32 {
        self.rate.at(intensity).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comfort {
    pub motion: f32,
    pub glitch_floor: f32,
}

impl Default for Comfort {
    fn default() -> Self {
        Self::FULL
    }
}

impl Comfort {
    pub const FULL: Self = Self {
        motion: 1.0,
        glitch_floor: 0.5,
    };

    pub fn reduced(motion: f32) -> Self {
        Self {
            motion,
            ..Self::FULL
        }
    }

    pub fn amount(&self) -> f32 {
        if self.motion.is_finite() {
            self.motion.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    pub fn shake(&self, value: f32) -> f32 {
        value * self.amount()
    }

    pub fn blur(&self, displacement: [f32; 2]) -> [f32; 2] {
        let amount = self.amount();
        displacement.map(|value| value * amount)
    }

    pub fn flash(&self, alpha: f32) -> f32 {
        alpha * self.amount()
    }

    pub fn hit_stop(&self, stop: HitStop) -> HitStop {
        let floor = stop.floor.clamp(0.0, 1.0);
        HitStop {
            floor: 1.0 - (1.0 - floor) * self.amount(),
            ..stop
        }
    }

    pub fn glitch(&self, strength: f32) -> f32 {
        let floor = if self.glitch_floor.is_finite() {
            self.glitch_floor.clamp(0.0, 1.0)
        } else {
            1.0
        };
        strength * (floor + (1.0 - floor) * self.amount())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShakeTuning {
    pub translation: [f32; 2],
    pub roll: f32,
    pub frequency: f32,
    pub decay: f32,
    pub seed: u64,
}

impl Default for ShakeTuning {
    fn default() -> Self {
        Self {
            translation: [0.0; 2],
            roll: 0.0,
            frequency: 0.0,
            decay: 1.0,
            seed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShakeOffset {
    pub translation: [f32; 2],
    pub roll: f32,
}

impl ShakeOffset {
    pub fn is_zero(&self) -> bool {
        self.translation == [0.0; 2] && self.roll == 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shake {
    pub tuning: ShakeTuning,
    trauma: f32,
    phase: f64,
}

impl Shake {
    pub fn new(tuning: ShakeTuning) -> Self {
        Self {
            tuning,
            trauma: 0.0,
            phase: 0.0,
        }
    }

    pub fn trauma(&self) -> f32 {
        self.trauma
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    pub fn add(&mut self, trauma: f32) {
        if trauma.is_finite() {
            self.trauma = (self.trauma + trauma).clamp(0.0, 1.0);
        }
    }

    pub fn clear(&mut self) {
        self.trauma = 0.0;
    }

    pub fn step(&mut self, dt: f32, rate: f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let decay = self.tuning.decay.max(0.0);
        self.trauma = (self.trauma - decay * dt).max(0.0);
        let speed = f64::from(self.tuning.frequency.max(0.0)) * f64::from(rate.max(0.0));
        self.phase += f64::from(dt) * speed;
    }

    pub fn offset(&self, gain: f32) -> ShakeOffset {
        let amount = self.trauma * self.trauma * gain.max(0.0);
        if amount.is_nan() || amount <= 0.0 {
            return ShakeOffset::default();
        }
        let seed = self.tuning.seed;
        let noise = |lane: u64| {
            smooth_noise(
                mix64(seed ^ lane.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
                self.phase,
            )
        };
        ShakeOffset {
            translation: [
                amount * self.tuning.translation[0] * noise(1),
                amount * self.tuning.translation[1] * noise(2),
            ],
            roll: amount * self.tuning.roll * noise(3),
        }
    }
}

fn lattice(seed: u64, cell: i64) -> f32 {
    let bits = mix64(seed ^ (cell as u64).wrapping_mul(0xd1b5_4a32_d192_ed03));
    ((bits >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
}

pub fn smooth_noise(seed: u64, x: f64) -> f32 {
    let cell = x.floor();
    let f = (x - cell) as f32;
    let index = cell as i64;
    let g0 = lattice(seed, index);
    let g1 = lattice(seed, index.wrapping_add(1));
    let a = g0 * f;
    let b = g1 * (f - 1.0);
    let fade = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    (2.0 * (a + (b - a) * fade)).clamp(-1.0, 1.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlashGate {
    starts: [f64; FLASHES_PER_SECOND],
    next: usize,
}

impl Default for FlashGate {
    fn default() -> Self {
        Self::new()
    }
}

impl FlashGate {
    pub fn new() -> Self {
        Self {
            starts: [f64::NEG_INFINITY; FLASHES_PER_SECOND],
            next: 0,
        }
    }

    pub fn allows(&self, now: f64) -> bool {
        now - self.starts[self.next] >= 1.0
    }

    pub fn admit(&mut self, now: f64) -> bool {
        if !now.is_finite() || !self.allows(now) {
            return false;
        }
        self.starts[self.next] = now;
        self.next = (self.next + 1) % FLASHES_PER_SECOND;
        true
    }
}

#[cfg(test)]
mod tests;
