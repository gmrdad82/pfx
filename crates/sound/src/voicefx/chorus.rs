use super::line::Line;
use crate::DriveCurve;
use crate::drive::shape;
use crate::interp::hermite;
use pfx_core::sim::{Rng, mix64};

const VOICES: usize = 4;
const OCTAVES: usize = 8;
const ROUGHNESS: f32 = 0.45;
const INTERVAL: f64 = 0.01;
const RATE_SPREAD: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chorus {
    pub separation: f32,
    pub depth: f32,
    pub rate: f32,
    pub feedback: f32,
    pub mix: f32,
    pub seed: u64,
}

impl Default for Chorus {
    fn default() -> Self {
        Self {
            separation: 0.015,
            depth: 0.005,
            rate: 0.5,
            feedback: 0.0,
            mix: 0.8,
            seed: 0,
        }
    }
}

pub fn chorus(samples: &[f32], channels: usize, rate: u32, settings: &Chorus) -> Vec<f32> {
    let channels = channels.max(1);
    let frames_per_second = rate.max(1) as f32;
    let separation = settings.separation.max(0.0);
    let depth = settings.depth.max(0.0);
    let shortest = separation * frames_per_second;
    let longest = (separation * VOICES as f32 + depth) * frames_per_second;
    let feedback = settings.feedback.clamp(-1.0, 1.0);
    let mix = settings.mix.clamp(0.0, 1.0);
    let seeds = [
        settings.seed,
        mix64(settings.seed),
        mix64(mix64(settings.seed)),
        mix64(settings.seed ^ 0xfedcba),
    ];
    let voices: Vec<Fractal> = seeds.iter().map(|seed| Fractal::new(*seed)).collect();
    let delays_at = |point: u64| -> [f32; VOICES] {
        let t = point as f64 * INTERVAL;
        std::array::from_fn(|k| {
            let base = separation * (k + 1) as f32;
            let speed = f64::from(settings.rate.max(0.0) + RATE_SPREAD * k as f32);
            let swing = voices[k].at(t * speed) * 0.5 + 0.5;
            (base + depth * swing) * frames_per_second
        })
    };
    let interval = (INTERVAL * f64::from(rate.max(1))).max(1.0);
    let mut lines: Vec<Line> = (0..channels).map(|_| Line::new(longest)).collect();
    let mut looped = vec![0.0f32; channels];
    let mut point = 0u64;
    let mut from = delays_at(0);
    let mut to = delays_at(1);
    let mut out = Vec::with_capacity(samples.len());
    for (n, frame) in samples.chunks(channels).enumerate() {
        let position = n as f64 / interval;
        let at = position.floor() as u64;
        if at != point {
            point = at;
            from = delays_at(point);
            to = delays_at(point + 1);
        }
        let t = (position - at as f64) as f32;
        let delays: [f32; VOICES] = std::array::from_fn(|k| from[k] + (to[k] - from[k]) * t);
        for ((x, line), value) in frame.iter().zip(lines.iter_mut()).zip(looped.iter_mut()) {
            line.write(x + *value);
            let wet = delays
                .iter()
                .map(|delay| line.read(*delay, shortest))
                .sum::<f32>()
                / VOICES as f32;
            line.advance();
            *value = shape(DriveCurve::Tanh, feedback * wet);
            out.push((1.0 - mix) * x + mix * wet);
        }
    }
    out
}

struct Fractal {
    octaves: [(f32, u64, f32); OCTAVES],
    total: f32,
}

impl Fractal {
    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let mut weight = 1.0f32;
        let mut total = 0.0f32;
        let octaves = std::array::from_fn(|_| {
            let offset = rng.next_f32();
            let seed = rng.next_u64();
            let octave = (offset, seed, weight);
            total += weight;
            weight *= ROUGHNESS;
            octave
        });
        Self { octaves, total }
    }

    fn at(&self, x: f64) -> f32 {
        let mut frequency = 1.0f64;
        let mut sum = 0.0f32;
        for (offset, seed, weight) in self.octaves {
            sum += weight * spline_noise(seed, x * frequency + f64::from(offset));
            frequency *= 2.0;
        }
        sum / self.total
    }
}

fn spline_noise(seed: u64, x: f64) -> f32 {
    let x = x + unit(mix64(seed)) as f64;
    let floor = x.floor();
    let dx = (x - floor) as f32;
    let index = floor as i64;
    let point =
        |i: i64| unit(mix64(seed ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15))) * 2.0 - 1.0;
    hermite(
        point(index.wrapping_sub(1)),
        point(index),
        point(index.wrapping_add(1)),
        point(index.wrapping_add(2)),
        dx,
    ) / 1.25
}

fn unit(bits: u64) -> f32 {
    (bits >> 40) as f32 / (1u64 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_chorus_and_another_seed_moves_it() {
        let input: Vec<f32> = (0..24_000)
            .flat_map(|i| [(std::f32::consts::TAU * 220.0 * i as f32 / 48_000.0).sin(); 2])
            .collect();
        let settings = Chorus {
            seed: 7,
            ..Chorus::default()
        };
        let a = chorus(&input, 2, 48_000, &settings);
        let b = chorus(&input, 2, 48_000, &settings);
        let c = chorus(
            &input,
            2,
            48_000,
            &Chorus {
                seed: 8,
                ..settings
            },
        );
        assert_eq!(a.len(), input.len());
        assert_eq!(a, b);
        assert_ne!(a, c);
        for frame in a.chunks(2) {
            assert_eq!(frame[0], frame[1]);
        }
    }
}
