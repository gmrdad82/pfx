pub(crate) fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let shift = (s >> 28).wrapping_add(4);
    let w = ((s >> shift) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

pub(crate) fn hash3(p: [i32; 3]) -> f32 {
    let h = (p[0] as u32).wrapping_mul(73856093)
        ^ (p[1] as u32).wrapping_mul(19349663)
        ^ (p[2] as u32).wrapping_mul(83492791);
    (pcg(h) >> 8) as f32 / 16_777_216.0
}

pub(crate) fn noise3(p: [f32; 3]) -> f32 {
    let i = [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ];
    let f = [p[0].fract(), p[1].fract(), p[2].fract()];
    let u = f.map(|t| t * t * (3.0 - 2.0 * t));
    let h = |x: i32, y: i32, z: i32| {
        hash3([
            i[0].wrapping_add(x),
            i[1].wrapping_add(y),
            i[2].wrapping_add(z),
        ])
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let a = lerp(
        lerp(h(0, 0, 0), h(1, 0, 0), u[0]),
        lerp(h(0, 1, 0), h(1, 1, 0), u[0]),
        u[1],
    );
    let b = lerp(
        lerp(h(0, 0, 1), h(1, 0, 1), u[0]),
        lerp(h(0, 1, 1), h(1, 1, 1), u[0]),
        u[1],
    );
    lerp(a, b, u[2])
}

pub(crate) fn fbm(p: [f32; 3]) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.55;
    let mut q = p;
    for _ in 0..4 {
        sum += noise3(q) * amp;
        q = [q[0] * 2.07 + 1.7, q[1] * 2.07 + 9.2, q[2] * 2.07 + 3.1];
        amp *= 0.5;
    }
    sum
}

pub(crate) struct Rng {
    state: u64,
}

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    pub(crate) fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let x = self.state;
        let rot = (x >> 59) as u32;
        let mixed = (((x >> 18) ^ x) >> 27) as u32;
        mixed.rotate_right(rot)
    }

    pub(crate) fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }

    pub(crate) fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_wind_seed_does_not_overflow() {
        let wind = crate::wind::Wind::new(4_000_000_000, [1.0, 0.0, 0.0], 1.0);
        let first = wind.gust_at([0.0; 3], 0.0);
        assert!(first.is_finite());
        assert_eq!(first, wind.gust_at([0.0; 3], 0.0));
        assert_eq!(noise3([1.0, 2.0, 3.0]), hash3([1, 2, 3]));
    }
}
