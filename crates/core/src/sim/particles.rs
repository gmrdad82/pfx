use super::Rng;

pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const STEP: f32 = 1.0 / 120.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    #[default]
    Alpha,
    Add,
}

pub trait Lerp: Copy {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for [f32; 3] {
    fn lerp(self, other: Self, t: f32) -> Self {
        std::array::from_fn(|i| self[i] + (other[i] - self[i]) * t)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient<T> {
    keys: Vec<(f32, T)>,
}

impl<T: Lerp> Gradient<T> {
    pub fn constant(value: T) -> Self {
        Self {
            keys: vec![(0.0, value)],
        }
    }

    pub fn new(mut keys: Vec<(f32, T)>, fallback: T) -> Self {
        keys.retain(|(t, _)| t.is_finite());
        if keys.is_empty() {
            return Self::constant(fallback);
        }
        keys.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { keys }
    }

    pub fn keys(&self) -> &[(f32, T)] {
        &self.keys
    }

    pub fn sample(&self, ratio: f32) -> T {
        match self.keys.binary_search_by(|key| key.0.total_cmp(&ratio)) {
            Ok(mut index) => {
                while index > 0 && self.keys[index - 1].0 == self.keys[index].0 {
                    index -= 1;
                }
                self.keys[index].1
            }
            Err(0) => self.keys[0].1,
            Err(upper) if upper >= self.keys.len() => self.keys[upper - 1].1,
            Err(upper) => {
                let (t0, v0) = self.keys[upper - 1];
                let (t1, v1) = self.keys[upper];
                v0.lerp(v1, (ratio - t0) / (t1 - t0))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub rate: f32,
    pub rate_keys: Option<Gradient<f32>>,
    pub burst: u32,
    pub max: u32,
    pub lifetime: [f32; 2],
    pub speed: [f32; 2],
    pub direction: [f32; 3],
    pub cone: f32,
    pub inherit: f32,
    pub gravity: [f32; 3],
    pub drag: f32,
    pub size: [f32; 2],
    pub alpha: Gradient<f32>,
    pub colour: Gradient<[f32; 3]>,
    pub spin: [f32; 2],
    pub stretch: f32,
    pub blend: Blend,
    pub lit: bool,
    pub seed: u64,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            rate: 0.0,
            rate_keys: None,
            burst: 0,
            max: 1024,
            lifetime: [1.0, 1.0],
            speed: [1.0, 1.0],
            direction: [0.0, 1.0, 0.0],
            cone: 0.0,
            inherit: 0.0,
            gravity: [0.0; 3],
            drag: 0.0,
            size: [0.1, 0.1],
            alpha: Gradient::new(vec![(0.0, 1.0), (1.0, 0.0)], 1.0),
            colour: Gradient::constant([1.0; 3]),
            spin: [0.0, 0.0],
            stretch: 0.0,
            blend: Blend::Alpha,
            lit: true,
            seed: 0,
        }
    }
}

impl Recipe {
    pub fn rate_at(&self, time: f32) -> f32 {
        self.rate_keys
            .as_ref()
            .map_or(self.rate, |keys| keys.sample(time))
            .max(0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub previous: [f32; 3],
    pub velocity: [f32; 3],
    pub angle: f32,
    pub spin: f32,
    pub age: f32,
    pub life: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub position: [f32; 3],
    pub previous: [f32; 3],
    pub velocity: [f32; 3],
    pub size: f32,
    pub angle: f32,
    pub colour: [f32; 3],
    pub alpha: f32,
}

#[derive(Clone, Debug)]
pub struct Emitter {
    recipe: Recipe,
    at: [f32; 3],
    rng: Rng,
    remainder: f32,
    time: f32,
    steps: u64,
    started: bool,
    dropped: u64,
    particles: Vec<Particle>,
}

impl Emitter {
    pub fn new(recipe: Recipe, at: [f32; 3]) -> Self {
        let rng = Rng::new(recipe.seed);
        let capacity = (recipe.max as usize).min(1 << 16);
        Self {
            recipe,
            at,
            rng,
            remainder: 0.0,
            time: 0.0,
            steps: 0,
            started: false,
            dropped: 0,
            particles: Vec::with_capacity(capacity),
        }
    }

    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    pub fn at(&self) -> [f32; 3] {
        self.at
    }

    pub fn reset(&mut self) {
        self.rng = Rng::new(self.recipe.seed);
        self.remainder = 0.0;
        self.time = 0.0;
        self.steps = 0;
        self.started = false;
        self.dropped = 0;
        self.particles.clear();
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn live(&self) -> usize {
        self.particles.len()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    pub fn step(&mut self, dt: f32, from: Matrix, to: Matrix) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let keep = (-self.recipe.drag.max(0.0) * dt).exp();
        let gravity = self.recipe.gravity;
        let mut index = 0;
        while index < self.particles.len() {
            let particle = &mut self.particles[index];
            particle.previous = particle.position;
            particle.age += dt;
            if particle.age >= particle.life {
                self.particles.swap_remove(index);
                continue;
            }
            integrate(particle, gravity, keep, dt);
            index += 1;
        }
        let rate = 0.5 * (self.recipe.rate_at(self.time) + self.recipe.rate_at(self.time + dt));
        self.remainder += rate * dt;
        let count = self.remainder.floor();
        self.remainder -= count;
        let start = point(from, self.at);
        let end = point(to, self.at);
        let parent = std::array::from_fn(|i| (end[i] - start[i]) / dt);
        let axis = [
            unit(vector(from, self.recipe.direction)),
            unit(vector(to, self.recipe.direction)),
        ];
        if !self.started {
            self.started = true;
            for _ in 0..self.recipe.burst {
                self.spawn(0.0, dt, start, end, parent, axis);
            }
        }
        let count = count as u32;
        for i in 0..count {
            let born = (i as f32 + 0.5) / count as f32;
            self.spawn(born, dt, start, end, parent, axis);
        }
        self.time += dt;
        self.steps += 1;
    }

    fn spawn(
        &mut self,
        born: f32,
        dt: f32,
        start: [f32; 3],
        end: [f32; 3],
        parent: [f32; 3],
        axis: [[f32; 3]; 2],
    ) {
        if self.particles.len() >= self.recipe.max as usize {
            self.dropped += 1;
            return;
        }
        let recipe = &self.recipe;
        let rng = &mut self.rng;
        let direction = cone(rng, unit(axis[0].lerp(axis[1], born)), recipe.cone);
        let speed = between(rng, recipe.speed);
        let life = between(rng, recipe.lifetime).max(1e-3);
        let angle = rng.next_f32() * std::f32::consts::TAU;
        let spin = between(rng, recipe.spin) * if rng.next_bool() { 1.0 } else { -1.0 };
        let position = start.lerp(end, born);
        let velocity = std::array::from_fn(|i| direction[i] * speed + parent[i] * recipe.inherit);
        let rest = dt * (1.0 - born);
        let mut particle = Particle {
            position,
            previous: position,
            velocity,
            angle,
            spin,
            age: rest,
            life,
        };
        if particle.age >= particle.life {
            return;
        }
        if rest > 0.0 {
            let keep = (-recipe.drag.max(0.0) * rest).exp();
            integrate(&mut particle, recipe.gravity, keep, rest);
        }
        self.particles.push(particle);
    }

    pub fn seek(&mut self, time: f32, step: f32, mut parent: impl FnMut(f32) -> Matrix) {
        if !(step.is_finite() && step > 0.0 && time.is_finite()) {
            return;
        }
        let target = (time.max(0.0) / step + 1e-3).floor() as u64;
        if target < self.steps {
            self.reset();
        }
        while self.steps < target {
            let t0 = self.steps as f32 * step;
            let from = parent(t0);
            let to = parent(t0 + step);
            self.step(step, from, to);
        }
    }

    pub fn sprites(&self, out: &mut Vec<Sprite>) {
        let recipe = &self.recipe;
        for particle in &self.particles {
            let t = (particle.age / particle.life).clamp(0.0, 1.0);
            let size = recipe.size[0].lerp(recipe.size[1], t);
            let alpha = recipe.alpha.sample(t).clamp(0.0, 1.0);
            if !(size > 0.0 && alpha > 0.0) {
                continue;
            }
            out.push(Sprite {
                position: particle.position,
                previous: particle.previous,
                velocity: particle.velocity,
                size,
                angle: particle.angle,
                colour: recipe.colour.sample(t),
                alpha,
            });
        }
    }
}

fn integrate(particle: &mut Particle, gravity: [f32; 3], keep: f32, dt: f32) {
    for (axis, pull) in gravity.iter().enumerate() {
        particle.velocity[axis] = (particle.velocity[axis] + pull * dt) * keep;
        particle.position[axis] += particle.velocity[axis] * dt;
    }
    particle.angle += particle.spin * dt;
}

fn between(rng: &mut Rng, range: [f32; 2]) -> f32 {
    range[0] + (range[1] - range[0]) * rng.next_f32()
}

fn cone(rng: &mut Rng, n: [f32; 3], degrees: f32) -> [f32; 3] {
    let u = rng.next_f32();
    let v = rng.next_f32();
    let half = degrees.clamp(0.0, 180.0).to_radians();
    let cos_theta = 1.0 - u * (1.0 - half.cos());
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = v * std::f32::consts::TAU;
    let (tangent, bitangent) = basis(n);
    let (s, c) = phi.sin_cos();
    std::array::from_fn(|i| (tangent[i] * c + bitangent[i] * s) * sin_theta + n[i] * cos_theta)
}

pub fn basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let sign = if n[2] >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (sign + n[2]);
    let b = n[0] * n[1] * a;
    (
        [1.0 + sign * n[0] * n[0] * a, sign * b, -sign * n[0]],
        [b, sign + n[1] * n[1] * a, -n[1]],
    )
}

pub fn point(m: Matrix, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

fn vector(m: Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 1e-12 {
        v.map(|x| x / length)
    } else {
        [0.0, 1.0, 0.0]
    }
}
