use super::super::sqrt;
use super::emit::round;

#[must_use]
pub fn toward<const N: usize>(from: [i32; N], to: [i32; N], step: i32) -> [i32; N] {
    if step <= 0 {
        return from;
    }
    let d: [i64; N] = std::array::from_fn(|i| i64::from(to[i]) - i64::from(from[i]));
    let dist2: i128 = d.iter().map(|&c| i128::from(c) * i128::from(c)).sum();
    if dist2 <= i128::from(step) * i128::from(step) {
        return to;
    }
    let scale = f64::from(step) / sqrt(dist2 as f64);
    std::array::from_fn(|i| from[i].saturating_add(round(d[i] as f64 * scale)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Member<const N: usize> {
    pub pos: [i32; N],
    pub offset: [i32; N],
    pub limit: Option<i32>,
}

impl<const N: usize> Member<N> {
    pub fn slot(&self, leader: [i32; N]) -> [i32; N] {
        std::array::from_fn(|i| leader[i].saturating_add(self.offset[i]))
    }

    pub fn step(&mut self, leader: [i32; N]) {
        let slot = self.slot(leader);
        self.pos = match self.limit {
            Some(limit) => toward(self.pos, slot, limit),
            None => slot,
        };
    }
}

pub fn follow<const N: usize>(members: &mut [Member<N>], leader: [i32; N]) {
    for member in members {
        member.step(leader);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Slow {
    pub num: u32,
    pub den: u32,
    pub ticks: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Mover<const N: usize> {
    pub pos: [i32; N],
    pub top: i32,
    stun: u32,
    slows: Vec<Slow>,
}

impl<const N: usize> Mover<N> {
    pub fn new(pos: [i32; N], top: i32) -> Self {
        Self {
            pos,
            top,
            stun: 0,
            slows: Vec::new(),
        }
    }

    pub fn stun(&mut self, ticks: u32) {
        self.stun = self.stun.max(ticks);
    }

    pub fn slow(&mut self, num: u32, den: u32, ticks: u32) {
        if den == 0 || ticks == 0 {
            return;
        }
        let same =
            |s: &Slow| u64::from(s.num) * u64::from(den) == u64::from(num) * u64::from(s.den);
        match self.slows.iter_mut().find(|s| same(s)) {
            Some(slow) => slow.ticks = slow.ticks.max(ticks),
            None => self.slows.push(Slow { num, den, ticks }),
        }
    }

    pub fn clear_effects(&mut self) {
        self.stun = 0;
        self.slows.clear();
    }

    pub const fn stunned(&self) -> u32 {
        self.stun
    }

    pub fn slows(&self) -> &[Slow] {
        &self.slows
    }

    pub fn speed(&self) -> i32 {
        if self.stun > 0 {
            return 0;
        }
        let strongest = self.slows.iter().min_by(|a, b| {
            (u64::from(a.num) * u64::from(b.den)).cmp(&(u64::from(b.num) * u64::from(a.den)))
        });
        match strongest {
            Some(slow) => {
                let speed = i64::from(self.top) * i64::from(slow.num) / i64::from(slow.den);
                speed.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
            }
            None => self.top,
        }
    }

    pub fn step(&mut self, target: [i32; N]) -> [i32; N] {
        self.pos = toward(self.pos, target, self.speed());
        self.stun = self.stun.saturating_sub(1);
        for slow in &mut self.slows {
            slow.ticks -= 1;
        }
        self.slows.retain(|slow| slow.ticks > 0);
        self.pos
    }
}
