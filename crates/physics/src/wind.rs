use crate::math::{norm3, scale3, smoothstep};
use crate::noise::{fbm, pcg};

pub const BEND_WGSL: &str = include_str!("bend.wgsl");

#[derive(Clone, Copy, Debug)]
pub struct Wind {
    seed: u32,
    direction: [f32; 3],
    strength: f32,
}

impl Wind {
    pub fn new(seed: u32, direction: [f32; 3], strength: f32) -> Self {
        let len = crate::math::len3(direction);
        let direction = if len < 1e-8 {
            [1.0, 0.0, 0.0]
        } else {
            scale3(direction, 1.0 / len)
        };
        Self {
            seed,
            direction,
            strength,
        }
    }

    pub fn direction(&self) -> [f32; 3] {
        self.direction
    }

    pub fn strength(&self) -> f32 {
        self.strength
    }

    pub fn gust_at(&self, p: [f32; 3], time: f32) -> f32 {
        let envelope = 0.55 + 0.45 * (time * 0.21 + 0.4).sin() * (time * 0.13 + 1.7).sin();
        let n = fbm([
            p[0] * 0.37 + time * 0.05,
            p[1] * 0.37 + 2.0,
            p[2] * 0.37 + self.seed as f32 * 0.13,
        ]);
        (envelope * (0.35 + 0.65 * n)).max(0.0)
    }

    pub fn sample(&self, p: [f32; 3], time: f32) -> [f32; 3] {
        scale3(self.direction, self.strength * self.gust_at(p, time))
    }
}

pub fn sway(p: [f32; 3], obj: u32, time: f32, wind: &Wind) -> [f32; 3] {
    let h = (pcg(obj.wrapping_mul(747796405).wrapping_add(11)) >> 8) as f32 / 16_777_216.0;
    let reach = 0.4
        + 0.6
            * smoothstep(
                0.0,
                0.35,
                ((p[0] - 0.4).powi(2) + (p[2] + 0.12).powi(2)).sqrt(),
            );
    let gust = wind.gust_at(p, time);
    let bough = [
        (time * 0.8 + p[0] * 3.0).sin() * 0.006,
        0.35 * (time * 0.6 + p[0] * 2.0).sin() * 0.006,
        0.5 * (time * 0.7 + p[0] * 2.5).cos() * 0.006,
    ];
    let flutter = [
        (time * 3.1 + h * 37.0).sin() * 0.0015,
        (time * 3.7 + h * 21.0).sin() * 0.0015,
        (time * 2.9 + h * 29.0).cos() * 0.0015,
    ];
    let amp = wind.strength * gust;
    let local = [
        (bough[0] * reach + flutter[0]) * amp,
        (bough[1] * reach + flutter[1]) * amp,
        (bough[2] * reach + flutter[2]) * amp,
    ];
    let flat = [wind.direction[0], 0.0, wind.direction[2]];
    let dir = norm3([flat[0] + 1e-5, 0.0, flat[2]]);
    let side = [-dir[2], 0.0, dir[0]];
    [
        dir[0] * local[0] + side[0] * local[2],
        local[1],
        dir[2] * local[0] + side[2] * local[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gust_is_deterministic_and_sway_follows_strength() {
        let wind = Wind::new(3, [2.0, 0.0, 0.4], 2.0);
        let p = [0.5, 0.3, -0.1];
        assert_eq!(wind.gust_at(p, 1.25), wind.gust_at(p, 1.25));
        let once = sway(p, 4, 1.25, &wind);
        let twice = sway(p, 4, 1.25, &wind);
        assert_eq!(once, twice);
        assert!(once.iter().any(|v| v.abs() > 0.0));
        let still = Wind::new(3, [2.0, 0.0, 0.4], 0.0);
        assert_eq!(sway(p, 4, 1.25, &still), [0.0; 3]);
        let blow = wind.sample(p, 1.25);
        let along = blow[0] * wind.direction()[0] + blow[2] * wind.direction()[2];
        assert!(along > 0.0);
        assert!(blow[1].abs() < 1e-6);
        assert!(BEND_WGSL.contains("fn bend("));
    }
}
