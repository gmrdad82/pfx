use super::super::{Rng, atan2, cos, sin, sqrt};

pub const FULL_TURN: i32 = 360_000;

const PI: f64 = f64::from_bits(0x4009_21FB_5444_2D18);
const RADIANS: f64 = PI / 180_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Pattern {
    Single,
    Fan { count: u32, arc: i32 },
    Ring { count: u32 },
    Spiral { arms: u32, step: i32 },
    Crown { count: u32, tilt: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rate {
    pub charge: u32,
    pub cost: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Burst {
    pub count: u32,
    pub gap: u32,
    pub cooldown: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Fire {
    pub pattern: Pattern,
    pub rate: Rate,
    pub burst: Option<Burst>,
    pub windup: u32,
    pub speed: i32,
    pub spread: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Telegraph {
    left: u32,
    total: u32,
}

impl Telegraph {
    pub const fn new() -> Self {
        Self { left: 0, total: 0 }
    }

    pub fn start(&mut self, ticks: u32) {
        self.total = ticks.max(1);
        self.left = self.total;
    }

    pub fn cancel(&mut self) -> bool {
        let was = self.is_winding();
        self.left = 0;
        self.total = 0;
        was
    }

    #[must_use]
    pub fn tick(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        if self.left == 0 {
            self.total = 0;
            return true;
        }
        false
    }

    pub const fn is_winding(&self) -> bool {
        self.left > 0
    }

    pub const fn left(&self) -> u32 {
        self.left
    }

    pub const fn total(&self) -> u32 {
        self.total
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Phase {
    Ready,
    Winding { left: u32, total: u32 },
    Burst { left: u32, wait: u32 },
    Cooling { left: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Aim2 {
    Angle(i32),
    At { target: [i32; 2], vel: [i32; 2] },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Aim3 {
    Basis {
        forward: [i32; 3],
        up: [i32; 3],
    },
    At {
        target: [i32; 3],
        vel: [i32; 3],
        up: [i32; 3],
    },
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Emitter {
    pub fire: Fire,
    charge: u32,
    phase: Phase,
    telegraph: Telegraph,
    spin: i32,
    #[cfg_attr(feature = "serde", serde(skip))]
    locals: Vec<(f64, f64)>,
}

impl Emitter {
    pub fn new(fire: Fire) -> Self {
        Self::with_charge(fire, 0)
    }

    pub fn with_charge(fire: Fire, charge: u32) -> Self {
        Self {
            fire,
            charge,
            phase: Phase::Ready,
            telegraph: Telegraph::new(),
            spin: 0,
            locals: Vec::new(),
        }
    }

    pub fn phase(&self) -> Phase {
        match self.phase {
            Phase::Winding { .. } => Phase::Winding {
                left: self.telegraph.left(),
                total: self.telegraph.total(),
            },
            phase => phase,
        }
    }

    pub const fn charge(&self) -> u32 {
        self.charge
    }

    pub const fn spin(&self) -> i32 {
        self.spin
    }

    pub fn cancel(&mut self) -> bool {
        if self.telegraph.cancel() {
            self.phase = Phase::Ready;
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.charge = 0;
        self.phase = Phase::Ready;
        self.telegraph.cancel();
        self.spin = 0;
    }

    #[must_use]
    pub fn tick(&mut self, trigger: bool) -> u32 {
        let cost = self.fire.rate.cost.max(1);
        match self.phase {
            Phase::Cooling { left } => {
                self.phase = if left <= 1 {
                    Phase::Ready
                } else {
                    Phase::Cooling { left: left - 1 }
                };
                0
            }
            Phase::Winding { .. } => {
                if self.telegraph.tick() {
                    self.release()
                } else {
                    0
                }
            }
            Phase::Burst { left, wait } => {
                if wait > 1 {
                    self.phase = Phase::Burst {
                        left,
                        wait: wait - 1,
                    };
                    return 0;
                }
                self.after_volley(left - 1);
                1
            }
            Phase::Ready => {
                if !trigger {
                    if self.charge < cost {
                        self.charge = self.charge.saturating_add(self.fire.rate.charge).min(cost);
                    }
                    return 0;
                }
                self.charge = self.charge.saturating_add(self.fire.rate.charge);
                if self.charge < cost {
                    return 0;
                }
                if self.fire.burst.is_none() && self.fire.windup == 0 {
                    let volleys = self.charge / cost;
                    self.charge %= cost;
                    return volleys;
                }
                self.charge -= cost;
                if self.fire.windup > 0 {
                    self.telegraph.start(self.fire.windup);
                    self.phase = Phase::Winding {
                        left: self.fire.windup,
                        total: self.fire.windup,
                    };
                    return 0;
                }
                self.release()
            }
        }
    }

    pub fn tick2(
        &mut self,
        origin: [i32; 2],
        aim: Aim2,
        trigger: bool,
        rng: &mut Rng,
        out: &mut Vec<[i32; 2]>,
    ) -> u32 {
        let volleys = self.tick(trigger);
        for _ in 0..volleys {
            self.volley2(origin, aim, rng, out);
        }
        volleys
    }

    pub fn tick3(
        &mut self,
        origin: [i32; 3],
        aim: Aim3,
        trigger: bool,
        rng: &mut Rng,
        out: &mut Vec<[i32; 3]>,
    ) -> u32 {
        let volleys = self.tick(trigger);
        for _ in 0..volleys {
            self.volley3(origin, aim, rng, out);
        }
        volleys
    }

    pub fn volley2(&mut self, origin: [i32; 2], aim: Aim2, rng: &mut Rng, out: &mut Vec<[i32; 2]>) {
        let base = match aim {
            Aim2::Angle(angle) => f64::from(angle) * RADIANS,
            Aim2::At { target, vel } => {
                let dir = aim_point(origin, target, vel, self.fire.speed);
                if dir == [0.0; 2] {
                    0.0
                } else {
                    atan2(dir[1], dir[0])
                }
            }
        };
        self.fill_locals(true);
        let speed = f64::from(self.fire.speed);
        for &(tilt, _) in &self.locals {
            let mut angle = base + tilt * RADIANS;
            if self.fire.spread > 0 {
                angle +=
                    f64::from(rng.range_i32(-self.fire.spread, self.fire.spread + 1)) * RADIANS;
            }
            out.push([round(cos(angle) * speed), round(sin(angle) * speed)]);
        }
    }

    pub fn volley3(&mut self, origin: [i32; 3], aim: Aim3, rng: &mut Rng, out: &mut Vec<[i32; 3]>) {
        let (forward, up) = match aim {
            Aim3::Basis { forward, up } => (forward.map(f64::from), up),
            Aim3::At { target, vel, up } => (aim_point(origin, target, vel, self.fire.speed), up),
        };
        let (f, s, u) = basis(forward, up.map(f64::from));
        self.fill_locals(false);
        let speed = f64::from(self.fire.speed);
        let cone = f64::from(self.fire.spread) * RADIANS;
        for &(tilt, roll) in &self.locals {
            let (t, r) = (tilt * RADIANS, roll * RADIANS);
            let (ct, st, cr, sr) = (cos(t), sin(t), cos(r), sin(r));
            let mut d: [f64; 3] = std::array::from_fn(|i| ct * f[i] + st * (cr * s[i] + sr * u[i]));
            if self.fire.spread > 0 {
                let ca = 1.0 - rng.next_f64() * (1.0 - cos(cone));
                let sa = sqrt((1.0 - ca * ca).max(0.0));
                let b = rng.next_f64() * (2.0 * PI);
                let (_, p, q) = basis(d, u);
                let (cb, sb) = (cos(b), sin(b));
                d = std::array::from_fn(|i| ca * d[i] + sa * (cb * p[i] + sb * q[i]));
            }
            out.push(d.map(|c| round(c * speed)));
        }
    }

    fn release(&mut self) -> u32 {
        match self.fire.burst {
            Some(burst) if burst.count > 1 && burst.gap == 0 => {
                self.after_volley(0);
                burst.count
            }
            Some(burst) if burst.count > 1 => {
                self.after_volley(burst.count - 1);
                1
            }
            _ => {
                self.after_volley(0);
                1
            }
        }
    }

    fn after_volley(&mut self, left: u32) {
        let burst = self.fire.burst.unwrap_or(Burst {
            count: 1,
            gap: 0,
            cooldown: 0,
        });
        self.phase = if left > 0 {
            Phase::Burst {
                left,
                wait: burst.gap,
            }
        } else if burst.cooldown > 0 {
            Phase::Cooling {
                left: burst.cooldown,
            }
        } else {
            Phase::Ready
        };
    }

    fn fill_locals(&mut self, flat: bool) {
        self.locals.clear();
        let even = |i: u32, count: u32| f64::from(i) * f64::from(FULL_TURN) / f64::from(count);
        match self.fire.pattern {
            Pattern::Single => self.locals.push((0.0, 0.0)),
            Pattern::Fan { count, arc } => {
                if count == 1 {
                    self.locals.push((0.0, 0.0));
                }
                if count > 1 {
                    let arc = f64::from(arc);
                    let gaps = f64::from(count - 1);
                    self.locals
                        .extend((0..count).map(|i| (f64::from(i) * arc / gaps - arc / 2.0, 0.0)));
                }
            }
            Pattern::Ring { count } => self
                .locals
                .extend((0..count).map(|i| (even(i, count), 0.0))),
            Pattern::Spiral { arms, step } => {
                let spin = f64::from(self.spin);
                self.locals
                    .extend((0..arms).map(|i| (spin + even(i, arms), 0.0)));
                self.spin = wrap(i64::from(self.spin) + i64::from(step));
            }
            Pattern::Crown { count, tilt } => {
                let tilt = f64::from(tilt);
                if flat {
                    self.locals.extend(
                        [(tilt, 0.0), (-tilt, 0.0)]
                            .into_iter()
                            .take(count.min(2) as usize),
                    );
                } else {
                    self.locals
                        .extend((0..count).map(|i| (tilt, even(i, count))));
                }
            }
        }
    }
}

pub fn intercept<const N: usize>(
    from: [i32; N],
    target: [i32; N],
    vel: [i32; N],
    speed: i32,
) -> Option<f64> {
    let d: [f64; N] = std::array::from_fn(|i| f64::from(target[i]) - f64::from(from[i]));
    let v = vel.map(f64::from);
    let dot = |x: &[f64; N], y: &[f64; N]| (0..N).fold(0.0, |sum, i| sum + x[i] * y[i]);
    let s = f64::from(speed);
    let a = dot(&v, &v) - s * s;
    let b = 2.0 * dot(&d, &v);
    let c = dot(&d, &d);
    if c == 0.0 || speed <= 0 {
        return None;
    }
    if a == 0.0 {
        return (b < 0.0).then(|| -c / b);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let root = sqrt(disc);
    let t1 = (-b - root) / (2.0 * a);
    let t2 = (-b + root) / (2.0 * a);
    let (low, high) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
    if low > 0.0 {
        Some(low)
    } else if high > 0.0 {
        Some(high)
    } else {
        None
    }
}

fn aim_point<const N: usize>(
    from: [i32; N],
    target: [i32; N],
    vel: [i32; N],
    speed: i32,
) -> [f64; N] {
    let t = intercept(from, target, vel, speed).unwrap_or(0.0);
    std::array::from_fn(|i| f64::from(target[i]) - f64::from(from[i]) + f64::from(vel[i]) * t)
}

fn basis(forward: [f64; 3], up: [f64; 3]) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let f = normalize(forward).unwrap_or([1.0, 0.0, 0.0]);
    let side = [up, [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]
        .into_iter()
        .find_map(|hint| normalize(cross(f, hint)))
        .unwrap_or([0.0, 1.0, 0.0]);
    (f, side, cross(side, f))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    let len = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    (len > 0.0 && len.is_finite()).then(|| v.map(|c| c / len))
}

fn wrap(angle: i64) -> i32 {
    let turn = i64::from(FULL_TURN);
    let mut angle = angle % turn;
    if angle < 0 {
        angle += turn;
    }
    angle as i32
}

pub(super) fn round(x: f64) -> i32 {
    x.round() as i32
}
