use crate::math::{add2, dot2, len2, mix, mix2, norm2, scale2, sub2};

pub const SPACING: f32 = 2.4;
pub const KERNEL: f32 = 6.0;
pub const SUBSTEP: f32 = 1.0 / 240.0;
pub const SLOTS: u32 = 32;
pub const MAX_PARTICLES: u32 = 65_536;
pub const MAX_TARGETS: usize = 128;
pub const DRAIN_TIME: f32 = 1.5;
pub const RIM: f32 = 5.0;
const CARRY: [f32; 2] = [40.0, 160.0];
const DROP_PULL: f32 = 1.0;
const SHARPNESS: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct Form {
    pub tilt: f32,
    pub irregular: f32,
    pub seed: f32,
    pub rim: f32,
    pub weight: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub tall: f32,
    pub active: f32,
    pub weight_b: f32,
    pub weight_a: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default, Debug, PartialEq)]
pub struct Target {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub look: [f32; 4],
    pub flow: [f32; 4],
    pub form: [f32; 4],
    pub carry: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default, Debug)]
pub struct Particle {
    pub pos: [f32; 2],
    pub vel: [f32; 2],
    pub owner: u32,
    pub rho: f32,
    pub life: f32,
    pub born: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub pressure: f32,
    pub contain: f32,
    pub cohesion: f32,
    pub damping: f32,
    pub viscosity: f32,
    pub gather: f32,
    pub tension: [f32; 4],
    pub apart: [f32; 4],
    pub life_rate: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            pressure: 60_000.0,
            contain: 4_000.0,
            cohesion: 30.0,
            damping: 8.0,
            viscosity: 100.0,
            gather: 1.5,
            tension: [16.0, 3.0, 0.5, 0.0],
            apart: [40_000.0, 3.0, 0.0, 0.0],
            life_rate: 3.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pour {
    pub key: String,
    pub target: Target,
    pub from: Option<[f32; 2]>,
    pub fill: Option<Target>,
}

impl Target {
    fn with(
        kind: f32,
        a: [f32; 4],
        b: [f32; 4],
        c: [f32; 4],
        form: [f32; 4],
        look: &Look,
        pull: f32,
    ) -> Self {
        Self {
            a,
            b,
            c,
            look: [look.tall, look.active, look.weight_b, look.weight_a],
            flow: [1.0, pull, kind, 1.0],
            form,
            carry: [0.0; 4],
        }
    }

    pub fn drop(center: [f32; 2], radii: [f32; 2], form: &Form, look: &Look) -> Self {
        let r = radii[0].min(radii[1]).max(4.0);
        Self::with(
            0.0,
            [center[0], center[1], radii[0], radii[1]],
            [form.weight[1], SHARPNESS, 0.0, 0.0],
            [0.0; 4],
            [form.tilt, form.irregular, form.seed, form.weight[0]],
            look,
            (100.0 / r.max(30.0)).powi(2) * DROP_PULL,
        )
    }

    pub fn organic(mut self, bend: f32, wobble: f32, seed: f32) -> Self {
        self.form = [bend, wobble, seed, 0.0];
        self
    }

    pub fn outline(center: [f32; 2], radius: f32, waves: [[f32; 2]; 5], look: &Look) -> Self {
        let [w2, w3, w4, w6, w7] = waves;
        Self::with(
            4.0,
            [center[0], center[1], radius, 0.0],
            [w2[0], w3[0], w4[0], w6[0]],
            [w7[0], w7[1], 0.0, 0.0],
            [w2[1], w3[1], w4[1], w6[1]],
            look,
            (100.0 / radius.max(4.0)).powi(2),
        )
    }

    pub fn squash(mut self, height: f32) -> Self {
        self.a[3] = height;
        self
    }

    pub fn outline_radius(&self, p: [f32; 2]) -> f32 {
        let squash = if self.a[3] > 0.0 { self.a[3] } else { 1.0 };
        let t = ((p[1] - self.a[1]) / squash).atan2(p[0] - self.a[0]);
        let wave = |n: f32, amp: f32, phase: f32| amp * (n * t - phase).cos();
        (t.cos().powi(2) + (squash * t.sin()).powi(2)).sqrt()
            * self.a[2]
            * (1.0
                + wave(2.0, self.b[0], self.form[0])
                + wave(3.0, self.b[1], self.form[1])
                + wave(4.0, self.b[2], self.form[2])
                + wave(6.0, self.b[3], self.form[3])
                + wave(7.0, self.c[0], self.c[1]))
    }

    pub fn capsule(from: [f32; 2], to: [f32; 2], radii: [f32; 2], look: &Look) -> Self {
        let r = radii[0].max(radii[1]).max(2.0);
        Self::with(
            1.0,
            [from[0], from[1], to[0], to[1]],
            [radii[0], radii[1], 0.0, 0.0],
            [0.0; 4],
            [0.0; 4],
            look,
            (100.0 / r).powi(2) * 0.6,
        )
    }

    pub fn curve(points: [[f32; 2]; 4], radii: [f32; 3], look: &Look) -> Self {
        let r = radii[0].max(radii[1]).max(2.0);
        Self::with(
            2.0,
            [points[0][0], points[0][1], points[1][0], points[1][1]],
            [points[2][0], points[2][1], points[3][0], points[3][1]],
            [radii[0], radii[1], radii[2], 0.0],
            [0.0; 4],
            look,
            (100.0 / r).powi(2) * 0.6,
        )
    }

    pub fn bead(center: [f32; 2], radius: f32, look: &Look) -> Self {
        Self::with(
            3.0,
            [center[0], center[1], radius, 0.0],
            [0.0; 4],
            [0.0; 4],
            [0.0; 4],
            look,
            (100.0 / radius.max(2.0)).powi(2) * 0.8,
        )
    }

    pub fn drifting(mut self, speed: f32) -> Self {
        self.form[3] = speed;
        self
    }

    pub fn flare(mut self, power: f32) -> Self {
        self.form[0] = power;
        self
    }

    pub fn drain(mut self, amount: f32) -> Self {
        self.form[1] = amount;
        self
    }

    pub fn pull(mut self, factor: f32) -> Self {
        self.flow[1] *= factor;
        self
    }

    pub fn mass(mut self, mass: f32) -> Self {
        self.flow[0] = mass;
        self
    }

    pub fn gathered(mut self, to: [f32; 2], pull: f32, scale: f32) -> Self {
        let toward = |x: f32, y: f32| {
            [
                to[0] + (x - to[0]) * (1.0 - pull),
                to[1] + (y - to[1]) * (1.0 - pull),
            ]
        };
        match self.kind() {
            1 => {
                let girth = 0.5 + 0.5 * scale;
                let a = toward(self.a[0], self.a[1]);
                let b = toward(
                    self.a[0] + (self.a[2] - self.a[0]) * scale,
                    self.a[1] + (self.a[3] - self.a[1]) * scale,
                );
                self.a = [a[0], a[1], b[0], b[1]];
                self.b[0] *= girth;
                self.b[1] *= girth;
                self.form[0] *= scale;
                self.flow[0] *= scale * girth;
            }
            2 => {
                let p0 = toward(self.a[0], self.a[1]);
                let p1 = toward(self.a[2], self.a[3]);
                let p2 = toward(self.b[0], self.b[1]);
                let p3 = toward(self.b[2], self.b[3]);
                self.a = [p0[0], p0[1], p1[0], p1[1]];
                self.b = [p2[0], p2[1], p3[0], p3[1]];
                self.c[0] *= scale;
                self.c[1] *= scale;
                self.c[2] *= scale;
                self.flow[0] *= scale * scale;
            }
            _ => {
                let c = toward(self.a[0], self.a[1]);
                self.a[0] = c[0];
                self.a[1] = c[1];
                self.a[2] *= scale;
                if self.kind() != 4 {
                    self.a[3] *= scale;
                }
                self.flow[0] *= scale * scale;
            }
        }
        self
    }

    pub fn shifted(mut self, by: [f32; 2]) -> Self {
        let move_by = |x: &mut f32, y: &mut f32| {
            *x += by[0];
            *y += by[1];
        };
        match self.kind() {
            1 => {
                let [a0, a1, a2, a3] = &mut self.a;
                move_by(a0, a1);
                move_by(a2, a3);
            }
            2 => {
                let [a0, a1, a2, a3] = &mut self.a;
                move_by(a0, a1);
                move_by(a2, a3);
                let [b0, b1, b2, b3] = &mut self.b;
                move_by(b0, b1);
                move_by(b2, b3);
            }
            _ => {
                let [a0, a1, _, _] = &mut self.a;
                move_by(a0, a1);
            }
        }
        self
    }

    pub fn origin(&self) -> [f32; 2] {
        [self.a[0], self.a[1]]
    }

    pub fn revealed(mut self, to: f32) -> Self {
        self.carry[2] = to;
        self.carry[3] = 1.0;
        self
    }

    pub fn kind(&self) -> u32 {
        self.flow[2] as u32
    }

    pub(crate) fn carried(mut self, before: &Target, dt: f32) -> Self {
        self.carry = [0.0, 0.0, self.carry[2], self.carry[3]];
        if let (Some(now), Some(was), true) = (
            self.anchor(),
            before.anchor(),
            dt > 1e-4 && self.kind() == before.kind(),
        ) {
            let v = [(now[0] - was[0]) / dt, (now[1] - was[1]) / dt];
            let speed = v[0].hypot(v[1]);
            let [slow, fast] = CARRY;
            let k = ((speed - slow) / (fast - slow)).clamp(0.0, 1.0);
            let k = k * k * (3.0 - 2.0 * k);
            if speed < 4000.0 {
                self.carry = [v[0] * k, v[1] * k, self.carry[2], self.carry[3]];
            }
        }
        self
    }

    fn anchor(&self) -> Option<[f32; 2]> {
        match self.kind() {
            1 => Some([(self.a[0] + self.a[2]) * 0.5, (self.a[1] + self.a[3]) * 0.5]),
            2 => None,
            _ => Some([self.a[0], self.a[1]]),
        }
    }

    pub(crate) fn spawns(&self, p: [f32; 2]) -> bool {
        if !self.inside(p) {
            return false;
        }
        if self.c[3] < 0.5 {
            return true;
        }
        if self.kind() == 2 {
            let (distance, radius, u) = self.curve_near(p);
            return distance > radius - self.form[3].max(SPACING)
                && u > self.form[2]
                && u < 1.0 - self.form[2];
        }
        if self.kind() == 0 && self.b[3] > 0.0 {
            let angle = (p[1] - self.a[1]).atan2(p[0] - self.a[0]);
            let off = (angle - self.b[2] + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            if off.abs() < self.b[3] {
                return false;
            }
        }
        let band = if self.kind() == 0 {
            RIM
        } else {
            self.form[3].max(SPACING)
        };
        match self.kind() {
            1 => {
                let ba = [self.a[2] - self.a[0], self.a[3] - self.a[1]];
                let pa = [p[0] - self.a[0], p[1] - self.a[1]];
                let h = ((pa[0] * ba[0] + pa[1] * ba[1])
                    / (ba[0] * ba[0] + ba[1] * ba[1]).max(1e-4))
                .clamp(0.0, 1.0);
                let length = ba[0].hypot(ba[1]).max(1e-3);
                let normal = [-ba[1] / length, ba[0] / length];
                let lift = self.form[0] * (std::f32::consts::PI * h).sin();
                let axis = [
                    self.a[0] + ba[0] * h + normal[0] * lift,
                    self.a[1] + ba[1] * h + normal[1] * lift,
                ];
                let wobble = 0.6 * (std::f32::consts::TAU * h * 1.3 + self.form[2]).sin()
                    + 0.4 * (std::f32::consts::TAU * h * 2.7 + self.form[2] * 1.7).sin();
                let r = (self.b[0] + (self.b[1] - self.b[0]) * (1.0 - (1.0 - h) * (1.0 - h)))
                    * (1.0 + self.form[1] * wobble);
                (p[0] - axis[0]).hypot(p[1] - axis[1]) < r
            }
            3 => (p[0] - self.a[0]).hypot(p[1] - self.a[1]) > self.a[2] - band,
            _ => {
                let (s, c) = (-self.form[0]).sin_cos();
                let x = p[0] - self.a[0];
                let y = p[1] - self.a[1];
                let local = [c * x - s * y, s * x + c * y];
                let k = (local[0] / (self.a[2] * 0.92)).hypot(local[1] / (self.a[3] * 0.92));
                k > 1.0 - band / (self.a[2].min(self.a[3]) * 0.92)
            }
        }
    }

    fn curve_near(&self, p: [f32; 2]) -> (f32, f32, f32) {
        let mut best = (f32::MAX, 0.0, 0.0);
        let mut prev = bezier(self, 0.0);
        for i in 1..=16 {
            let next = bezier(self, i as f32 / 16.0);
            let ba = sub2(next, prev);
            let pa = sub2(p, prev);
            let h = (dot2(pa, ba) / dot2(ba, ba).max(1e-4)).clamp(0.0, 1.0);
            let d = len2(sub2(pa, scale2(ba, h)));
            if d < best.0 {
                let u = (i as f32 - 1.0 + h) / 16.0;
                let e = (2.0 * u - 1.0).abs();
                let power = if self.form[0] > 0.0 {
                    self.form[0]
                } else {
                    2.0
                };
                let r = self.c[2]
                    + (self.c[0] + (self.c[1] - self.c[0]) * u - self.c[2]) * e.powf(power);
                best = (d, r, u);
            }
            prev = next;
        }
        best
    }

    pub(crate) fn inside(&self, p: [f32; 2]) -> bool {
        let segment = |a: [f32; 2], b: [f32; 2]| {
            let ba = sub2(b, a);
            let pa = sub2(p, a);
            let h = (dot2(pa, ba) / dot2(ba, ba).max(1e-4)).clamp(0.0, 1.0);
            (len2(sub2(pa, scale2(ba, h))), h)
        };
        match self.kind() {
            1 => {
                let (d, h) = segment([self.a[0], self.a[1]], [self.a[2], self.a[3]]);
                d < self.b[0] + (self.b[1] - self.b[0]) * (1.0 - (1.0 - h) * (1.0 - h))
            }
            2 => {
                let mut prev = bezier(self, 0.0);
                for i in 1..=16 {
                    let next = bezier(self, i as f32 / 16.0);
                    let (d, h) = segment(prev, next);
                    let u = (i as f32 - 1.0 + h) / 16.0;
                    let e = (2.0 * u - 1.0).abs();
                    let power = if self.form[0] > 0.0 {
                        self.form[0]
                    } else {
                        2.0
                    };
                    let r = self.c[2]
                        + (self.c[0] + (self.c[1] - self.c[0]) * u - self.c[2]) * e.powf(power);
                    if d < r.max(SPACING * 0.75) {
                        return true;
                    }
                    prev = next;
                }
                false
            }
            3 => (p[0] - self.a[0]).hypot(p[1] - self.a[1]) < self.a[2],
            4 => (p[0] - self.a[0]).hypot(p[1] - self.a[1]) < self.outline_radius(p) * 0.97,
            _ => {
                let (s, c) = (-self.form[0]).sin_cos();
                let x = p[0] - self.a[0];
                let y = p[1] - self.a[1];
                let local = [c * x - s * y, s * x + c * y];
                (local[0] / (self.a[2] * 0.92)).hypot(local[1] / (self.a[3] * 0.92)) < 1.0
            }
        }
    }

    pub(crate) fn bounds(&self) -> ([f32; 2], [f32; 2]) {
        match self.kind() {
            1 => {
                let r = self.b[0].max(self.b[1]);
                (
                    [self.a[0].min(self.a[2]) - r, self.a[1].min(self.a[3]) - r],
                    [self.a[0].max(self.a[2]) + r, self.a[1].max(self.a[3]) + r],
                )
            }
            2 => {
                let r = self.c[0].max(self.c[1]).max(self.c[2]);
                let xs = [self.a[0], self.a[2], self.b[0], self.b[2]];
                let ys = [self.a[1], self.a[3], self.b[1], self.b[3]];
                (
                    [
                        xs.into_iter().fold(f32::MAX, f32::min) - r,
                        ys.into_iter().fold(f32::MAX, f32::min) - r,
                    ],
                    [
                        xs.into_iter().fold(f32::MIN, f32::max) + r,
                        ys.into_iter().fold(f32::MIN, f32::max) + r,
                    ],
                )
            }
            3 => (
                [self.a[0] - self.a[2], self.a[1] - self.a[2]],
                [self.a[0] + self.a[2], self.a[1] + self.a[2]],
            ),
            4 => {
                let r = self.a[2] * 1.4;
                (
                    [self.a[0] - r, self.a[1] - r],
                    [self.a[0] + r, self.a[1] + r],
                )
            }
            _ => {
                let r = self.a[2].max(self.a[3]) * 1.2;
                (
                    [self.a[0] - r, self.a[1] - r],
                    [self.a[0] + r, self.a[1] + r],
                )
            }
        }
    }
}

pub fn rest_density() -> f32 {
    let h = KERNEL;
    let reach = (h / SPACING).ceil() as i32;
    let mut sum = 0.0;
    for y in -reach..=reach {
        for x in -reach..=reach {
            let r2 = ((x * x + y * y) as f32) * SPACING * SPACING;
            if r2 < h * h {
                let d = h * h - r2;
                sum += 4.0 / (std::f32::consts::PI * h.powi(8)) * d * d * d;
            }
        }
    }
    sum
}

pub(crate) fn spawn(
    target: &Target,
    slot: usize,
    from: Option<[f32; 2]>,
    seed: u32,
) -> Vec<Particle> {
    let (lo, hi) = target.bounds();
    let mut out = Vec::new();
    let mut rng = seed.wrapping_mul(2_654_435_761) | 1;
    let mut jitter = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        ((rng >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * SPACING * 0.3
    };
    let mut y = lo[1];
    while y <= hi[1] {
        let mut x = lo[0];
        while x <= hi[0] {
            if target.spawns([x, y]) {
                let mut at = [x + jitter(), y + jitter()];
                if let Some(from) = from {
                    at = add2(from, scale2(sub2(at, from), 0.3));
                }
                out.push(Particle {
                    pos: at,
                    vel: [0.0, 0.0],
                    owner: slot as u32,
                    rho: 1.0,
                    life: 0.0,
                    born: 1.0,
                });
            }
            x += SPACING;
        }
        y += SPACING;
    }
    out
}

pub(crate) fn cells_for(table: [f32; 2]) -> [usize; 2] {
    [
        (table[0] / KERNEL).ceil().max(1.0) as usize,
        (table[1] / KERNEL).ceil().max(1.0) as usize,
    ]
}

pub(crate) fn poly6(r2: f32) -> f32 {
    let h2 = KERNEL * KERNEL;
    if r2 >= h2 {
        return 0.0;
    }
    let d = h2 - r2;
    4.0 / (std::f32::consts::PI * KERNEL.powi(8)) * d * d * d
}

pub(crate) fn spiky(r: f32) -> f32 {
    if r >= KERNEL || r <= 1e-5 {
        return 0.0;
    }
    let d = KERNEL - r;
    -30.0 / (std::f32::consts::PI * KERNEL.powi(5)) * d * d
}

pub(crate) fn region(target: &Target, p: [f32; 2], time: f32, calm: f32) -> [f32; 3] {
    match target.kind() {
        1 => {
            let a = [target.a[0], target.a[1]];
            let b = [target.a[2], target.a[3]];
            let s = segment(p, a, b);
            let span = sub2(b, a);
            let normal = scale2([-span[1], span[0]], 1.0 / len2(span).max(1e-3));
            let u = s[1];
            let axis = add2(
                mix2(a, b, u),
                scale2(normal, target.form[0] * (std::f32::consts::PI * u).sin()),
            );
            let drift = time * calm * target.form[3];
            let wobble = 0.6 * (std::f32::consts::TAU * u * 1.3 + target.form[2] + drift).sin()
                + 0.4
                    * (std::f32::consts::TAU * u * 2.7 + target.form[2] * 1.7 - drift * 0.7).sin();
            let radius = mix(target.b[0], target.b[1], 1.0 - (1.0 - u) * (1.0 - u))
                * (1.0 + target.form[1] * wobble);
            [len2(sub2(p, axis)) - radius, axis[0], axis[1]]
        }
        2 => {
            let mut best = [1e5, p[0], p[1]];
            let mut prev = bezier(target, 0.0);
            for i in 1..=16 {
                let u1 = i as f32 / 16.0;
                let next = bezier(target, u1);
                let s = segment(p, prev, next);
                let u = (i as f32 - 1.0 + s[1]) / 16.0;
                let e = (2.0 * u - 1.0).abs();
                let power = if target.form[0] > 0.0 {
                    target.form[0]
                } else {
                    2.0
                };
                let radius = mix(target.c[2], mix(target.c[0], target.c[1], u), e.powf(power));
                if s[0] - radius < best[0] {
                    let at = mix2(prev, next, s[1]);
                    best = [s[0] - radius, at[0], at[1]];
                }
                prev = next;
            }
            best
        }
        3 => {
            let centre = [target.a[0], target.a[1]];
            [len2(sub2(p, centre)) - target.a[2], centre[0], centre[1]]
        }
        4 => {
            let squash = if target.a[3] > 0.0 { target.a[3] } else { 1.0 };
            let d = [p[0] - target.a[0], (p[1] - target.a[1]) / squash];
            let th = d[1].atan2(d[0]);
            let wave = target.b[0] * (2.0 * th - target.form[0]).cos()
                + target.b[1] * (3.0 * th - target.form[1]).cos()
                + target.b[2] * (4.0 * th - target.form[2]).cos()
                + target.b[3] * (6.0 * th - target.form[3]).cos()
                + target.c[0] * (7.0 * th - target.c[1]).cos();
            [
                (len2(d) - target.a[2] * (1.0 + wave)) * squash.min(1.0),
                target.a[0],
                target.a[1],
            ]
        }
        _ => {
            let local = turn(sub2(p, [target.a[0], target.a[1]]), -target.form[0]);
            let r = [target.a[2].max(1.0), target.a[3].max(1.0)];
            let theta = (local[1] / r[1]).atan2(local[0] / r[0]);
            let c = (theta + target.form[0] - target.form[3]).cos().max(0.0);
            let s = 1.0
                + target.form[1] * harmonic(theta, target.form[2])
                + target.b[0] * c.powf(target.b[1].max(2.0));
            let shaped = [r[0] * s, r[1] * s];
            let k0 = len2([local[0] / shaped[0], local[1] / shaped[1]]);
            let k1 = len2([
                local[0] / (shaped[0] * shaped[0]),
                local[1] / (shaped[1] * shaped[1]),
            ]);
            [k0 * (k0 - 1.0) / k1.max(1e-5), target.a[0], target.a[1]]
        }
    }
}

pub(crate) fn along(target: &Target, p: [f32; 2]) -> [f32; 4] {
    let mut total = 0.0;
    let mut prev = bezier(target, 0.0);
    for i in 1..=16 {
        let next = bezier(target, i as f32 / 16.0);
        total += len2(sub2(next, prev));
        prev = next;
    }
    let mut best = 1e9;
    let mut out = [0.5, 1.0, 0.0, total];
    prev = bezier(target, 0.0);
    for i in 1..=16 {
        let next = bezier(target, i as f32 / 16.0);
        let s = segment(p, prev, next);
        if s[0] < best {
            best = s[0];
            let dir = norm2(add2(sub2(next, prev), [1e-5, 0.0]));
            out = [(i as f32 - 1.0 + s[1]) / 16.0, dir[0], dir[1], total];
        }
        prev = next;
    }
    out
}

fn bezier(target: &Target, u: f32) -> [f32; 2] {
    let v = 1.0 - u;
    let w = [v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u];
    [
        w[0] * target.a[0] + w[1] * target.a[2] + w[2] * target.b[0] + w[3] * target.b[2],
        w[0] * target.a[1] + w[1] * target.a[3] + w[2] * target.b[1] + w[3] * target.b[3],
    ]
}

fn segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let ba = sub2(b, a);
    let h = (dot2(sub2(p, a), ba) / dot2(ba, ba).max(1e-4)).clamp(0.0, 1.0);
    [len2(sub2(p, add2(a, scale2(ba, h)))), h]
}

fn turn(p: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, c) = angle.sin_cos();
    [c * p[0] - s * p[1], s * p[0] + c * p[1]]
}

fn harmonic(theta: f32, seed: f32) -> f32 {
    let square = (seed * 0.618).fract() * 0.9;
    0.46 * (2.0 * theta + seed * 1.7).sin()
        + 0.36 * (3.0 * theta + seed * 2.9 + 1.0).sin()
        + square * 0.40 * (4.0 * theta + seed * 0.7).cos()
        + 0.12 * (5.0 * theta + seed * 4.3 + 2.0).sin()
}
