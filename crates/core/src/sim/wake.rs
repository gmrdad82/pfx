use std::collections::VecDeque;

use super::particles::{Field, Matrix, length, point};

pub const STRIDE: u64 = 4;
pub const REACH: f32 = 3.0;

const SQRT_PI: f32 = 1.772_453_9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recipe {
    pub at: [f32; 3],
    pub strength: f32,
    pub radius: f32,
    pub grow: f32,
    pub decay: f32,
    pub spread: f32,
    pub rise: f32,
    pub length: f32,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            at: [0.0; 3],
            strength: 0.5,
            radius: 1.0,
            grow: 0.6,
            decay: 0.8,
            spread: 0.3,
            rise: 0.15,
            length: 3.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    position: [f32; 3],
    velocity: [f32; 3],
    time: f32,
    span: f32,
}

#[derive(Clone, Debug)]
pub struct Wake {
    recipe: Recipe,
    samples: VecDeque<Sample>,
    time: f32,
    steps: u64,
}

impl Wake {
    pub fn new(recipe: Recipe) -> Self {
        Self {
            recipe,
            samples: VecDeque::new(),
            time: 0.0,
            steps: 0,
        }
    }

    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.time = 0.0;
        self.steps = 0;
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn samples(&self) -> usize {
        self.samples.len()
    }

    pub fn step(&mut self, dt: f32, from: Matrix, to: Matrix) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let start = point(from, self.recipe.at);
        let end = point(to, self.recipe.at);
        self.time += dt;
        self.steps += 1;
        let velocity: [f32; 3] = std::array::from_fn(|i| (end[i] - start[i]) / dt);
        if self.steps.is_multiple_of(STRIDE) && length(velocity) > 1e-4 {
            self.samples.push_back(Sample {
                position: end,
                velocity,
                time: self.time,
                span: dt * STRIDE as f32,
            });
        }
        let oldest = self.time - self.recipe.length.max(0.0);
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.time < oldest)
        {
            self.samples.pop_front();
        }
    }

    pub fn velocity(&self, at: [f32; 3]) -> [f32; 3] {
        let recipe = &self.recipe;
        let radius = recipe.radius.max(1e-3);
        let decay = recipe.decay.max(1e-3);
        let mut sum = [0.0; 3];
        for sample in &self.samples {
            let age = (self.time - sample.time).max(0.0);
            let reach = radius + recipe.grow.max(0.0) * age;
            let offset: [f32; 3] = std::array::from_fn(|i| at[i] - sample.position[i]);
            let far = offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2];
            if far > REACH * REACH * reach * reach {
                continue;
            }
            let speed = length(sample.velocity);
            if speed.is_nan() || speed <= 1e-4 {
                continue;
            }
            let ratio = radius / reach;
            let line = speed * sample.span / (SQRT_PI * reach);
            let w = (-age / decay).exp() * (-far / (reach * reach)).exp() * ratio * ratio * line;
            let heading = sample.velocity.map(|v| v / speed);
            let ahead = offset[0] * heading[0] + offset[1] * heading[1] + offset[2] * heading[2];
            let mut out: [f32; 3] = std::array::from_fn(|i| offset[i] - heading[i] * ahead);
            out[1] = 0.0;
            let wide = length(out);
            for i in 0..3 {
                let outward = if wide > 1e-6 { out[i] / wide } else { 0.0 };
                let rise = if i == 1 { recipe.rise } else { 0.0 };
                sum[i] += w
                    * (recipe.strength * sample.velocity[i]
                        + (recipe.spread * outward + rise) * speed);
            }
        }
        sum
    }
}

impl Field for Wake {
    fn velocity(&self, at: [f32; 3]) -> [f32; 3] {
        Wake::velocity(self, at)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Wakes<'a>(pub &'a [Wake]);

impl Field for Wakes<'_> {
    fn velocity(&self, at: [f32; 3]) -> [f32; 3] {
        let mut sum = [0.0; 3];
        for wake in self.0 {
            let v = wake.velocity(at);
            for i in 0..3 {
                sum[i] += v[i];
            }
        }
        sum
    }
}
