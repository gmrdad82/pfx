use crate::math::{add3, scale3, sub3};
use crate::noise::{Rng, noise3};
use crate::wind::Wind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleKind {
    Petal,
    Dust,
    Steam,
}

#[derive(Clone, Copy, Debug)]
pub struct Speck {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub age: f32,
    pub life: f32,
    pub seed: u32,
}

pub struct Emitter {
    kind: ParticleKind,
    origin: [f32; 3],
    rate: f32,
    carry: f32,
    max: usize,
    rng: Rng,
    specks: Vec<Speck>,
}

impl Emitter {
    pub fn new(kind: ParticleKind, origin: [f32; 3], rate: f32, seed: u64) -> Self {
        Self {
            kind,
            origin,
            rate,
            carry: 0.0,
            max: 256,
            rng: Rng::new(seed),
            specks: Vec::new(),
        }
    }

    pub fn specks(&self) -> &[Speck] {
        &self.specks
    }

    pub fn origin(&self) -> [f32; 3] {
        self.origin
    }

    pub fn set_origin(&mut self, origin: [f32; 3]) {
        if origin.iter().all(|value| value.is_finite()) {
            self.origin = origin;
        }
    }

    pub fn step(&mut self, dt: f32, time: f32, wind: &Wind) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.carry += self.rate * dt;
        while self.carry >= 1.0 && self.specks.len() < self.max {
            self.carry -= 1.0;
            self.spawn();
        }
        let kind = self.kind;
        for speck in &mut self.specks {
            let blow = wind.sample(speck.pos, time);
            let (drag, gravity) = match kind {
                ParticleKind::Petal => (1.6, [0.0, -1.4, 0.0]),
                ParticleKind::Dust => (6.0, [0.0, -0.02, 0.0]),
                ParticleKind::Steam => (2.2, [0.0, 0.9, 0.0]),
            };
            let mut vel = pull(speck.vel, blow, drag, dt);
            vel = add3(vel, scale3(gravity, dt));
            if kind == ParticleKind::Petal {
                let n = noise3([
                    speck.pos[0] * 3.0 + speck.seed as f32 * 0.001,
                    time * 2.0,
                    speck.pos[2] * 3.0,
                ]);
                let m = noise3([
                    speck.pos[2] * 3.0,
                    time * 1.7 + 4.0,
                    speck.pos[0] * 3.0 + speck.seed as f32 * 0.002,
                ]);
                vel[0] += (n - 0.5) * dt;
                vel[2] += (m - 0.5) * dt;
            }
            speck.vel = vel;
            speck.pos = add3(speck.pos, scale3(vel, dt));
            speck.age += dt;
        }
        self.specks.retain(|speck| speck.age < speck.life);
    }

    fn spawn(&mut self) {
        let (life, spread, vel) = match self.kind {
            ParticleKind::Petal => (
                self.rng.range(4.0, 7.0),
                0.15,
                [
                    self.rng.range(-0.2, 0.2),
                    self.rng.range(-0.15, 0.05),
                    self.rng.range(-0.2, 0.2),
                ],
            ),
            ParticleKind::Dust => (
                self.rng.range(8.0, 12.0),
                0.4,
                [
                    self.rng.range(-0.05, 0.05),
                    self.rng.range(-0.02, 0.04),
                    self.rng.range(-0.05, 0.05),
                ],
            ),
            ParticleKind::Steam => (
                self.rng.range(1.4, 2.8),
                0.04,
                [
                    self.rng.range(-0.05, 0.05),
                    self.rng.range(0.15, 0.35),
                    self.rng.range(-0.05, 0.05),
                ],
            ),
        };
        let seed = self.rng.next_u32();
        let pos = [
            self.origin[0] + self.rng.range(-spread, spread),
            self.origin[1] + self.rng.range(-spread * 0.25, spread * 0.25),
            self.origin[2] + self.rng.range(-spread, spread),
        ];
        self.specks.push(Speck {
            pos,
            vel,
            age: 0.0,
            life,
            seed,
        });
    }
}

fn pull(vel: [f32; 3], toward: [f32; 3], drag: f32, dt: f32) -> [f32; 3] {
    let k = 1.0 - (-drag * dt).exp();
    add3(vel, scale3(sub3(toward, vel), k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(kind: ParticleKind, seed: u64) -> Vec<Speck> {
        let wind = Wind::new(11, [1.0, 0.0, 0.2], 1.2);
        let mut emitter = Emitter::new(kind, [0.0, 2.0, 0.0], 8.0, seed);
        let dt = 1.0 / 60.0;
        let mut time = 0.0;
        for _ in 0..240 {
            emitter.step(dt, time, &wind);
            time += dt;
        }
        emitter.specks().to_vec()
    }

    #[test]
    fn the_same_seed_repeats_every_kind() {
        for kind in [ParticleKind::Petal, ParticleKind::Dust, ParticleKind::Steam] {
            let a = run(kind, 99);
            let b = run(kind, 99);
            assert!(!a.is_empty());
            assert_eq!(a.len(), b.len());
            for (left, right) in a.iter().zip(&b) {
                assert_eq!(left.pos, right.pos);
                assert_eq!(left.vel, right.vel);
                assert_eq!(left.age, right.age);
                assert!(left.pos.iter().all(|v| v.is_finite()));
            }
        }
    }

    #[test]
    fn moved_origin_spawns_at_the_new_place() {
        let wind = Wind::new(4, [1.0, 0.0, 0.0], 0.0);
        let mut emitter = Emitter::new(ParticleKind::Petal, [0.0, 2.0, 0.0], 60.0, 8);
        emitter.step(0.1, 0.0, &wind);
        let before = emitter.specks().len();
        emitter.set_origin([5.0, 2.0, -3.0]);
        emitter.set_origin([f32::NAN, 0.0, 0.0]);
        assert_eq!(emitter.origin(), [5.0, 2.0, -3.0]);
        emitter.step(0.1, 0.1, &wind);
        let fresh = &emitter.specks()[before..];
        assert!(!fresh.is_empty());
        for speck in fresh {
            assert!((speck.pos[0] - 5.0).abs() < 0.5);
            assert!((speck.pos[2] + 3.0).abs() < 0.5);
        }
    }

    #[test]
    fn petals_fall_and_steam_rises() {
        let wind = Wind::new(4, [1.0, 0.0, 0.0], 0.4);
        let origin = [0.0, 3.0, 0.0];
        let mut petals = Emitter::new(ParticleKind::Petal, origin, 6.0, 5);
        let mut steam = Emitter::new(ParticleKind::Steam, origin, 10.0, 5);
        let dt = 1.0 / 60.0;
        let mut time = 0.0;
        for _ in 0..180 {
            petals.step(dt, time, &wind);
            steam.step(dt, time, &wind);
            time += dt;
        }
        let petal = petals
            .specks()
            .iter()
            .max_by(|a, b| a.age.total_cmp(&b.age))
            .unwrap();
        let puff = steam
            .specks()
            .iter()
            .max_by(|a, b| a.age.total_cmp(&b.age))
            .unwrap();
        assert!(petal.pos[1] < origin[1], "{}", petal.pos[1]);
        assert!(puff.pos[1] > origin[1], "{}", puff.pos[1]);
    }
}
