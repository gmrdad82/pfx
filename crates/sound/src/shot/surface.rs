use super::RATE;
use super::Surface;
use super::dsp::{Noise, Svf, decay_per_sample, sin_turns};

const STONES: [f32; 4] = [900.0, 1_500.0, 2_400.0, 3_600.0];
const STONE_Q: f32 = 3.0;
const GRAINS_PER_METRE: f32 = 60.0;
const POP_EVERY: f32 = 1.0 / 16.0;
const TICKS_PER_METRE: f32 = 0.3;
const TICK_MODES: [f32; 3] = [1.0, 1.47, 2.09];
const TICK_Q: f32 = 40.0;
const BODY_HZ: f32 = 90.0;
const BODY_Q: f32 = 2.5;
const CLICK_HZ: f32 = 1_800.0;
const KICK_SECONDS: f32 = 0.006;
const JITTER: f32 = 0.05;

pub(crate) fn weights(keys: &[(f32, Surface)], t: f32) -> [f32; 4] {
    let mut w = [0.0; 4];
    let last = keys.len() - 1;
    if t <= keys[0].0 {
        w[keys[0].1 as usize] = 1.0;
        return w;
    }
    if t >= keys[last].0 {
        w[keys[last].1 as usize] = 1.0;
        return w;
    }
    let i = keys.partition_point(|k| k.0 <= t) - 1;
    let (t0, a) = keys[i];
    let (t1, b) = keys[i + 1];
    if a == b || t1 <= t0 {
        w[b as usize] = 1.0;
        return w;
    }
    let s = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    w[a as usize] = sin_turns(0.25 + s * 0.25);
    w[b as usize] += sin_turns(s * 0.25);
    w
}

pub(crate) struct Gravel {
    noise: Noise,
    stones: [Svf; 4],
    kicks: [f32; 4],
    pop: Svf,
    pop_kick: f32,
    ticks: [Svf; 3],
    tick_kick: f32,
}

impl Gravel {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            noise: Noise::new(seed),
            stones: STONES.map(|hz| Svf::new(hz, STONE_Q)),
            kicks: [0.0; 4],
            pop: Svf::new(450.0, 2.0),
            pop_kick: 0.0,
            ticks: TICK_MODES.map(|m| Svf::new(3_000.0 * m, TICK_Q)),
            tick_kick: 0.0,
        }
    }

    pub(crate) fn block(&mut self, speed: f32, load: f32, out: &mut [f32]) -> usize {
        let drive = 0.5 + 0.5 * load.clamp(0.0, 1.0);
        let pace = (speed / 25.0).clamp(0.0, 1.5);
        let bright = 0.7 + 0.5 * pace * drive;
        for (stone, hz) in self.stones.iter_mut().zip(STONES) {
            stone.tune(hz * bright, STONE_Q);
        }
        let grain_chance = GRAINS_PER_METRE * speed.max(0.0) * drive / RATE as f32;
        let tick_chance = TICKS_PER_METRE * speed.max(0.0) / RATE as f32;
        let level = pfx_core::sim::powf(pace, 0.8);
        let mut grains = 0;
        for y in out.iter_mut() {
            if self.noise.unit() < grain_chance {
                grains += 1;
                let amp = self.noise.range(0.3, 1.0);
                if self.noise.unit() < POP_EVERY {
                    self.pop.tune(self.noise.range(300.0, 600.0), 2.0);
                    self.pop_kick += amp * 1.5;
                } else {
                    let i = ((self.noise.unit() * 4.0) as usize).min(3);
                    self.kicks[i] += amp;
                }
            }
            if self.noise.unit() < tick_chance {
                let base = self.noise.range(2_500.0, 4_200.0);
                for (tick, m) in self.ticks.iter_mut().zip(TICK_MODES) {
                    tick.tune(base * m, TICK_Q);
                }
                self.tick_kick += self.noise.range(0.5, 1.0);
            }
            let mut crunch = 0.0;
            for (stone, kick) in self.stones.iter_mut().zip(&mut self.kicks) {
                crunch += stone.band(*kick);
                *kick = 0.0;
            }
            crunch += self.pop.band(self.pop_kick);
            self.pop_kick = 0.0;
            let mut ring = 0.0;
            for tick in &mut self.ticks {
                ring += tick.band(self.tick_kick);
            }
            self.tick_kick = 0.0;
            *y = crunch * level * 1.8 + ring * 0.2;
        }
        grains
    }
}

pub(crate) struct Cobbles {
    noise: Noise,
    spacing: f64,
    distance: f64,
    stone: u64,
    next: f64,
    body: Svf,
    click: Svf,
    kick: f32,
    decay: f32,
}

impl Cobbles {
    pub(crate) fn new(seed: u64, spacing: f32) -> Self {
        let mut noise = Noise::new(seed);
        let spacing = f64::from(spacing.max(0.01));
        let next = spacing * (1.0 + f64::from(JITTER * noise.white()));
        Self {
            noise,
            spacing,
            distance: 0.0,
            stone: 1,
            next,
            body: Svf::new(BODY_HZ, BODY_Q),
            click: Svf::new(CLICK_HZ, 1.0),
            kick: 0.0,
            decay: decay_per_sample(KICK_SECONDS),
        }
    }

    pub(crate) fn block(&mut self, speed: f32, out: &mut [f32]) {
        let step = f64::from(speed.max(0.0)) / f64::from(RATE);
        let level = pfx_core::sim::powf((speed / 25.0).clamp(0.0, 2.0), 0.9);
        for y in out.iter_mut() {
            self.distance += step;
            if self.distance >= self.next {
                self.stone += 1;
                self.next =
                    self.spacing * (self.stone as f64 + f64::from(JITTER * self.noise.white()));

                self.kick = self.noise.range(0.7, 1.0);
            }
            let body = self.body.band(self.kick);
            let click = self.click.band(self.kick * 0.3);
            self.kick *= self.decay;
            *y = (body + click * 0.6) * level;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grains(speed: f32) -> usize {
        let mut gravel = Gravel::new(5);
        let mut out = [0.0; 16];
        (0..2 * RATE as usize / 16)
            .map(|_| gravel.block(speed, 1.0, &mut out))
            .sum()
    }

    #[test]
    fn gravel_grains_come_faster_with_speed() {
        let (slow, fast) = (grains(5.0), grains(20.0));
        assert!(slow > 100, "{slow} grains at 5 m/s");
        let ratio = fast as f32 / slow as f32;
        assert!((3.4..4.6).contains(&ratio), "{slow} then {fast}");
    }
}
