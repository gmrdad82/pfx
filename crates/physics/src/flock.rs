use crate::math::{add3, clamp3, len3, limit3, norm3, scale3, sub3};
use crate::noise::Rng;

#[derive(Clone, Copy, Debug)]
pub struct Boid {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct FlockParams {
    pub separation: f32,
    pub separation_weight: f32,
    pub alignment: f32,
    pub alignment_weight: f32,
    pub cohesion: f32,
    pub cohesion_weight: f32,
    pub max_speed: f32,
    pub max_force: f32,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub bounds_weight: f32,
    pub margin: f32,
    pub target: Option<[f32; 3]>,
    pub target_weight: f32,
}

impl FlockParams {
    pub fn birds() -> Self {
        Self {
            separation: 1.6,
            separation_weight: 1.8,
            alignment: 4.0,
            alignment_weight: 0.8,
            cohesion: 6.0,
            cohesion_weight: 0.45,
            max_speed: 6.0,
            max_force: 14.0,
            bounds_min: [-24.0, 1.0, -24.0],
            bounds_max: [24.0, 14.0, 24.0],
            bounds_weight: 3.0,
            margin: 2.0,
            target: None,
            target_weight: 1.2,
        }
    }

    pub fn fish() -> Self {
        Self {
            separation: 0.8,
            separation_weight: 2.2,
            alignment: 2.2,
            alignment_weight: 1.1,
            cohesion: 3.0,
            cohesion_weight: 0.7,
            max_speed: 3.5,
            max_force: 10.0,
            bounds_min: [-16.0, -0.6, -16.0],
            bounds_max: [16.0, 0.6, 16.0],
            bounds_weight: 4.0,
            margin: 0.8,
            target: None,
            target_weight: 1.4,
        }
    }
}

pub struct Flock {
    boids: Vec<Boid>,
    params: FlockParams,
}

impl Flock {
    pub fn new(boids: Vec<Boid>, params: FlockParams) -> Self {
        Self { boids, params }
    }

    pub fn seeded(count: usize, seed: u64, params: FlockParams) -> Self {
        let mut rng = Rng::new(seed);
        let mut boids = Vec::with_capacity(count);
        for _ in 0..count {
            let pos = [0, 1, 2].map(|k| rng.range(params.bounds_min[k], params.bounds_max[k]));
            let vel = norm3([
                rng.range(-1.0, 1.0),
                rng.range(-1.0, 1.0),
                rng.range(-1.0, 1.0),
            ]);
            boids.push(Boid {
                pos,
                vel: scale3(vel, params.max_speed * 0.5),
            });
        }
        Self { boids, params }
    }

    pub fn boids(&self) -> &[Boid] {
        &self.boids
    }

    pub fn follow(&mut self, target: Option<[f32; 3]>) {
        self.params.target = target;
    }

    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 || self.boids.is_empty() {
            return;
        }
        let n = self.boids.len();
        let params = self.params;
        let mut acc = vec![[0.0; 3]; n];
        for (i, slot) in acc.iter_mut().enumerate() {
            let mut sep = [0.0; 3];
            let mut align = [0.0; 3];
            let mut centre = [0.0; 3];
            let mut align_n = 0.0;
            let mut cohere_n = 0.0;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let d = sub3(self.boids[i].pos, self.boids[j].pos);
                let dist = len3(d).max(1e-4);
                if dist < params.separation {
                    let push = scale3(norm3(d), (params.separation - dist) / params.separation);
                    sep = add3(sep, push);
                }
                if dist < params.alignment {
                    align = add3(align, self.boids[j].vel);
                    align_n += 1.0;
                }
                if dist < params.cohesion {
                    centre = add3(centre, self.boids[j].pos);
                    cohere_n += 1.0;
                }
            }
            let mut a = scale3(sep, params.separation_weight);
            if align_n > 0.0 {
                let want = scale3(norm3(scale3(align, 1.0 / align_n)), params.max_speed);
                a = add3(
                    a,
                    scale3(sub3(want, self.boids[i].vel), params.alignment_weight),
                );
            }
            if cohere_n > 0.0 {
                let want = scale3(
                    norm3(sub3(scale3(centre, 1.0 / cohere_n), self.boids[i].pos)),
                    params.max_speed,
                );
                a = add3(
                    a,
                    scale3(sub3(want, self.boids[i].vel), params.cohesion_weight),
                );
            }
            a = add3(a, bounds_force(self.boids[i].pos, &params));
            if let Some(target) = params.target {
                let want = scale3(norm3(sub3(target, self.boids[i].pos)), params.max_speed);
                a = add3(
                    a,
                    scale3(sub3(want, self.boids[i].vel), params.target_weight),
                );
            }
            *slot = limit3(a, params.max_force);
        }
        for (boid, a) in self.boids.iter_mut().zip(acc) {
            boid.vel = limit3(add3(boid.vel, scale3(a, dt)), params.max_speed);
            boid.pos = clamp3(
                add3(boid.pos, scale3(boid.vel, dt)),
                params.bounds_min,
                params.bounds_max,
            );
        }
    }
}

fn bounds_force(p: [f32; 3], params: &FlockParams) -> [f32; 3] {
    let mut a = [0.0; 3];
    for k in 0..3 {
        let lo = params.bounds_min[k] + params.margin;
        let hi = params.bounds_max[k] - params.margin;
        if p[k] < lo {
            a[k] += params.bounds_weight * (lo - p[k]);
        } else if p[k] > hi {
            a[k] -= params.bounds_weight * (p[k] - hi);
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apart(a: [f32; 3], b: [f32; 3]) -> f32 {
        len3(sub3(a, b))
    }

    #[test]
    fn separation_pushes_a_close_pair_apart() {
        let mut params = FlockParams::birds();
        params.separation = 2.0;
        params.separation_weight = 12.0;
        params.alignment_weight = 0.0;
        params.cohesion_weight = 0.0;
        params.target = None;
        params.bounds_min = [-50.0; 3];
        params.bounds_max = [50.0; 3];
        params.max_speed = 6.0;
        params.max_force = 40.0;
        let mut flock = Flock::new(
            vec![
                Boid {
                    pos: [0.0, 0.0, 0.0],
                    vel: [0.0; 3],
                },
                Boid {
                    pos: [0.2, 0.0, 0.0],
                    vel: [0.0; 3],
                },
            ],
            params,
        );
        let start = apart(flock.boids()[0].pos, flock.boids()[1].pos);
        for _ in 0..90 {
            flock.step(1.0 / 60.0);
        }
        let now = apart(flock.boids()[0].pos, flock.boids()[1].pos);
        assert!(now > start, "{now} vs {start}");
        assert!(now > 1.0, "{now}");
    }

    #[test]
    fn the_same_seed_repeats_the_flock() {
        let params = FlockParams::fish();
        let mut a = Flock::seeded(80, 42, params);
        let mut b = Flock::seeded(80, 42, params);
        a.follow(Some([2.0, 0.0, -1.0]));
        b.follow(Some([2.0, 0.0, -1.0]));
        for _ in 0..120 {
            a.step(1.0 / 60.0);
            b.step(1.0 / 60.0);
        }
        for (left, right) in a.boids().iter().zip(b.boids()) {
            assert_eq!(left.pos, right.pos);
            assert_eq!(left.vel, right.vel);
        }
        assert!(a.boids().iter().all(|boid| {
            (0..3)
                .all(|k| boid.pos[k] >= params.bounds_min[k] && boid.pos[k] <= params.bounds_max[k])
        }));
    }

    #[test]
    fn a_flock_follows_its_target_and_stays_in_bounds() {
        let mut params = FlockParams::birds();
        params.target = Some([10.0, 6.0, 0.0]);
        let mut flock = Flock::seeded(200, 7, params);
        let start = flock.boids().iter().map(|b| b.pos[0]).sum::<f32>() / 200.0;
        for _ in 0..180 {
            flock.step(1.0 / 60.0);
        }
        let end = flock.boids().iter().map(|b| b.pos[0]).sum::<f32>() / 200.0;
        assert!(end > start, "{end} vs {start}");
        assert!(flock.boids().iter().all(|boid| {
            boid.pos.iter().all(|v| v.is_finite())
                && (0..3).all(|k| {
                    boid.pos[k] >= params.bounds_min[k] && boid.pos[k] <= params.bounds_max[k]
                })
        }));
    }
}
