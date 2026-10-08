use crate::math::{add2, len2, scale2, sub2};

pub const SUBSTEP: f32 = 1.0 / 240.0;
const PER: usize = 5;
const MAX_WIRES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drift {
    pub x_main: [f32; 2],
    pub x_side: [f32; 3],
    pub y_main: [f32; 2],
    pub y_side: [f32; 2],
    pub side: f32,
}

impl Default for Drift {
    fn default() -> Self {
        Self {
            x_main: [0.02, 0.5],
            x_side: [0.012, 0.3, 0.006],
            y_main: [0.02, 0.4],
            y_side: [0.016, 0.25],
            side: 0.5,
        }
    }
}

impl Drift {
    pub fn at(&self, p: [f32; 2], time: f32) -> [f32; 2] {
        [
            (p[1] * self.x_main[0] + time * self.x_main[1]).sin()
                + self.side
                    * (p[0] * self.x_side[0] - time * self.x_side[1] + p[1] * self.x_side[2]).sin(),
            (p[0] * self.y_main[0] - time * self.y_main[1]).cos()
                + self.side * (p[1] * self.y_side[0] + time * self.y_side[1]).cos(),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainParams {
    pub pull: f32,
    pub drag: f32,
    pub flow: f32,
    pub slack: f32,
    pub cord: f32,
    pub drift: Drift,
}

impl Default for ChainParams {
    fn default() -> Self {
        Self {
            pull: 24.0,
            drag: 2.5,
            flow: 20.0,
            slack: 1.1,
            cord: 12.0,
            drift: Drift::default(),
        }
    }
}

pub struct Chain {
    params: ChainParams,
    nodes: Vec<[f32; 2]>,
    prev: Vec<[f32; 2]>,
    cords: Vec<[f32; 2]>,
    cords_prev: Vec<[f32; 2]>,
    clock: f32,
    inputs: Vec<[f32; 2]>,
    asleep: bool,
}

impl Default for Chain {
    fn default() -> Self {
        Self::new(ChainParams::default())
    }
}

impl Chain {
    pub fn new(params: ChainParams) -> Self {
        Self {
            params,
            nodes: Vec::new(),
            prev: Vec::new(),
            cords: Vec::new(),
            cords_prev: Vec::new(),
            clock: 0.0,
            inputs: Vec::new(),
            asleep: false,
        }
    }

    pub fn nodes(&self) -> &[[f32; 2]] {
        &self.nodes
    }

    pub fn cords(&self) -> &[[f32; 2]] {
        &self.cords
    }

    pub fn asleep(&self) -> bool {
        self.asleep
    }

    pub fn bead(&self, index: usize) -> Option<[f32; 2]> {
        self.nodes.get((index + 1) * PER).copied()
    }

    pub fn snap(&mut self, anchor: [f32; 2], beads: &[[f32; 2]], cords: &[[f32; 2]]) {
        let point = |k: usize| if k == 0 { anchor } else { beads[k - 1] };
        self.nodes = vec![anchor];
        for k in 0..beads.len() {
            let (a, b) = (point(k), point(k + 1));
            for j in 1..=PER {
                let t = j as f32 / PER as f32;
                self.nodes
                    .push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        self.prev = self.nodes.clone();
        self.cords = cords.to_vec();
        self.cords_prev = cords.to_vec();
    }

    pub fn step(
        &mut self,
        dt: f32,
        time: f32,
        anchor: [f32; 2],
        beads: &[[f32; 2]],
        cords: &[[f32; 2]],
        idle: f32,
    ) {
        let params = self.params;
        let wanted = if beads.is_empty() {
            0
        } else {
            beads.len() * PER + 1
        };
        while self.nodes.len() < wanted {
            let from = self.nodes.last().copied().unwrap_or(anchor);
            self.nodes.push(from);
            self.prev.push(from);
        }
        self.nodes.truncate(wanted);
        self.prev.truncate(wanted);
        while self.cords.len() < cords.len() {
            let i = self.cords.len();
            let from = self.nodes.get((i + 2) * PER).copied().unwrap_or(anchor);
            self.cords.push(from);
            self.cords_prev.push(from);
        }
        self.cords.truncate(cords.len());
        self.cords_prev.truncate(cords.len());
        if wanted == 0 {
            return;
        }
        let inputs: Vec<[f32; 2]> = std::iter::once(anchor)
            .chain(beads.iter().copied())
            .chain(cords.iter().copied())
            .collect();
        let same = inputs.len() == self.inputs.len()
            && inputs
                .iter()
                .zip(&self.inputs)
                .all(|(a, b)| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01);
        self.inputs = inputs;
        if !same {
            self.asleep = false;
        }
        if self.asleep {
            self.clock = 0.0;
            return;
        }
        let point = |k: usize| if k == 0 { anchor } else { beads[k - 1] };
        let rest: Vec<f32> = (0..beads.len())
            .map(|k| {
                let (a, b) = (point(k), point(k + 1));
                (b[0] - a[0]).hypot(b[1] - a[1]) / PER as f32 * params.slack
            })
            .collect();
        let h = SUBSTEP;
        self.clock = (self.clock + dt).min(h * 60.0);
        let steps = (self.clock / h).floor() as usize;
        self.clock -= steps as f32 * h;
        let keep = (-params.drag * h).exp();
        let flow = params.flow * idle;
        for step in 0..steps {
            let t = time + h * step as f32;
            self.nodes[0] = anchor;
            self.prev[0] = anchor;
            for j in 1..self.nodes.len() {
                let p = self.nodes[j];
                let v = [
                    (p[0] - self.prev[j][0]) * keep,
                    (p[1] - self.prev[j][1]) * keep,
                ];
                let w = params.drift.at(p, t);
                let mut f = [w[0] * flow, w[1] * flow];
                if j % PER == 0 {
                    let goal = point(j / PER);
                    f[0] += params.pull * (goal[0] - p[0]);
                    f[1] += params.pull * (goal[1] - p[1]);
                }
                self.prev[j] = p;
                self.nodes[j] = [p[0] + v[0] + f[0] * h * h, p[1] + v[1] + f[1] * h * h];
            }
            for (i, goal) in cords.iter().enumerate() {
                let p = self.cords[i];
                let v = [
                    (p[0] - self.cords_prev[i][0]) * keep,
                    (p[1] - self.cords_prev[i][1]) * keep,
                ];
                let w = params.drift.at(p, t + 3.0);
                let f = [
                    w[0] * flow + params.pull * 1.6 * (goal[0] - p[0]),
                    w[1] * flow + params.pull * 1.6 * (goal[1] - p[1]),
                ];
                self.cords_prev[i] = p;
                self.cords[i] = [p[0] + v[0] + f[0] * h * h, p[1] + v[1] + f[1] * h * h];
            }
            for _ in 0..12 {
                for j in 0..self.nodes.len() - 1 {
                    let (a, b) = (self.nodes[j], self.nodes[j + 1]);
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let len = d[0].hypot(d[1]).max(1e-4);
                    let target = rest[(j / PER).min(rest.len() - 1)];
                    let fix = (len - target) / len;
                    if j == 0 {
                        self.nodes[1] = [b[0] - d[0] * fix, b[1] - d[1] * fix];
                    } else {
                        self.nodes[j] = [a[0] + d[0] * fix * 0.5, a[1] + d[1] * fix * 0.5];
                        self.nodes[j + 1] = [b[0] - d[0] * fix * 0.5, b[1] - d[1] * fix * 0.5];
                    }
                }
                for i in 0..self.cords.len() {
                    let Some(a) = self.nodes.get((i + 2) * PER).copied() else {
                        continue;
                    };
                    let b = self.cords[i];
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let len = d[0].hypot(d[1]).max(1e-4);
                    let fix = (len - params.cord) / len;
                    self.cords[i] = [b[0] - d[0] * fix, b[1] - d[1] * fix];
                }
            }
        }
        let moving = |now: &[[f32; 2]], was: &[[f32; 2]]| {
            now.iter()
                .zip(was)
                .any(|(a, b)| (a[0] - b[0]).abs() > 0.01 || (a[1] - b[1]).abs() > 0.01)
        };
        if steps > 0 && !moving(&self.nodes, &self.prev) && !moving(&self.cords, &self.cords_prev) {
            self.prev = self.nodes.clone();
            self.cords_prev = self.cords.clone();
            self.asleep = true;
        }
    }

    pub fn wires(&self) -> Vec<[f32; 4]> {
        let mut out = Vec::with_capacity(MAX_WIRES);
        if self.nodes.is_empty() {
            return out;
        }
        let at = |i: isize| self.nodes[i.clamp(0, self.nodes.len() as isize - 1) as usize];
        for i in 0..self.nodes.len().saturating_sub(1) {
            if out.len() == MAX_WIRES {
                return out;
            }
            let (p0, p1, p2, p3) = (
                at(i as isize - 1),
                at(i as isize),
                at(i as isize + 1),
                at(i as isize + 2),
            );
            let mut prev = p1;
            for k in 1..=3 {
                if out.len() == MAX_WIRES {
                    return out;
                }
                let t = k as f32 / 3.0;
                let (t2, t3) = (t * t, t * t * t);
                let f = |a: f32, b: f32, c: f32, d: f32| {
                    0.5 * (2.0 * b
                        + (-a + c) * t
                        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                        + (-a + 3.0 * b - 3.0 * c + d) * t3)
                };
                let next = [f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])];
                out.push([prev[0], prev[1], next[0], next[1]]);
                prev = next;
            }
        }
        for (i, end) in self.cords.iter().enumerate() {
            if out.len() == MAX_WIRES {
                break;
            }
            if let Some(start) = self.nodes.get((i + 2) * PER) {
                out.push([start[0], start[1], end[0], end[1]]);
            }
        }
        out
    }
}

pub struct Rope {
    pos: Vec<[f32; 2]>,
    prev: Vec<[f32; 2]>,
    rest: Vec<f32>,
    pins: Vec<Option<[f32; 2]>>,
    drag: f32,
    iterations: u32,
    clock: f32,
    drift: Drift,
}

impl Rope {
    pub fn between(start: [f32; 2], end: [f32; 2], segments: usize, slack: f32) -> Self {
        let segments = segments.max(1);
        let mut pos = Vec::with_capacity(segments + 1);
        for i in 0..=segments {
            let t = i as f32 / segments as f32;
            pos.push(add2(start, scale2(sub2(end, start), t)));
        }
        let rest_len = len2(sub2(end, start)) / segments as f32 * slack;
        let mut pins = vec![None; pos.len()];
        pins[0] = Some(start);
        pins[segments] = Some(end);
        Self {
            prev: pos.clone(),
            pos,
            rest: vec![rest_len; segments],
            pins,
            drag: 3.0,
            iterations: 12,
            clock: 0.0,
            drift: Drift::default(),
        }
    }

    pub fn drifting(mut self, drift: Drift) -> Self {
        self.drift = drift;
        self
    }

    pub fn pin(&mut self, index: usize, at: [f32; 2]) {
        if let Some(slot) = self.pins.get_mut(index) {
            *slot = Some(at);
            self.pos[index] = at;
            self.prev[index] = at;
        }
    }

    pub fn nodes(&self) -> &[[f32; 2]] {
        &self.pos
    }

    pub fn length_error(&self) -> f32 {
        let mut worst: f32 = 0.0;
        for (j, rest) in self.rest.iter().enumerate() {
            let len = len2(sub2(self.pos[j + 1], self.pos[j]));
            worst = worst.max((len - rest).abs());
        }
        worst
    }

    pub fn step(&mut self, dt: f32, gravity: [f32; 2]) {
        self.step_at(dt, 0.0, gravity, 0.0);
    }

    pub fn step_at(&mut self, dt: f32, time: f32, gravity: [f32; 2], flow: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.clock = (self.clock + dt).min(SUBSTEP * 60.0);
        let steps = (self.clock / SUBSTEP).floor() as usize;
        self.clock -= steps as f32 * SUBSTEP;
        let h = SUBSTEP;
        let keep = (-self.drag * h).exp();
        for step in 0..steps {
            let t = time + h * step as f32;
            for j in 0..self.pos.len() {
                if let Some(pin) = self.pins[j] {
                    self.pos[j] = pin;
                    self.prev[j] = pin;
                    continue;
                }
                let p = self.pos[j];
                let v = scale2(sub2(p, self.prev[j]), keep);
                let w = self.drift.at(p, t);
                let f = add2(gravity, scale2(w, flow));
                self.prev[j] = p;
                self.pos[j] = add2(add2(p, v), scale2(f, h * h));
            }
            for _ in 0..self.iterations {
                for j in 0..self.rest.len() {
                    let a_pin = self.pins[j].is_some();
                    let b_pin = self.pins[j + 1].is_some();
                    if a_pin && b_pin {
                        continue;
                    }
                    let a = self.pos[j];
                    let b = self.pos[j + 1];
                    let d = sub2(b, a);
                    let len = len2(d).max(1e-4);
                    let fix = (len - self.rest[j]) / len;
                    if a_pin {
                        self.pos[j + 1] = sub2(b, scale2(d, fix));
                    } else if b_pin {
                        self.pos[j] = add2(a, scale2(d, fix));
                    } else {
                        self.pos[j] = add2(a, scale2(d, fix * 0.5));
                        self.pos[j + 1] = sub2(b, scale2(d, fix * 0.5));
                    }
                }
                for j in 0..self.pos.len() {
                    if let Some(pin) = self.pins[j] {
                        self.pos[j] = pin;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct ReferenceChain {
        nodes: Vec<[f32; 2]>,
        prev: Vec<[f32; 2]>,
        cords: Vec<[f32; 2]>,
        cords_prev: Vec<[f32; 2]>,
        clock: f32,
        inputs: Vec<[f32; 2]>,
        asleep: bool,
    }

    const REFERENCE: ChainParams = ChainParams {
        pull: 28.0,
        drag: 2.5,
        flow: 22.0,
        slack: 1.08,
        cord: 14.0,
        drift: Drift {
            x_main: [0.023, 0.43],
            x_side: [0.011, 0.33, 0.009],
            y_main: [0.017, 0.37],
            y_side: [0.019, 0.29],
            side: 0.55,
        },
    };

    fn reference_drift(p: [f32; 2], time: f32) -> [f32; 2] {
        [
            (p[1] * 0.023 + time * 0.43).sin()
                + 0.55 * (p[0] * 0.011 - time * 0.33 + p[1] * 0.009).sin(),
            (p[0] * 0.017 - time * 0.37).cos() + 0.55 * (p[1] * 0.019 + time * 0.29).cos(),
        ]
    }

    impl ReferenceChain {
        fn step(
            &mut self,
            dt: f32,
            time: f32,
            anchor: [f32; 2],
            beads: &[[f32; 2]],
            cords: &[[f32; 2]],
            idle: f32,
        ) {
            let [pull, drag, flow, slack, cord] = [28.0, 2.5, 22.0, 1.08, 14.0];
            let flow = flow * idle;
            let wanted = if beads.is_empty() {
                0
            } else {
                beads.len() * PER + 1
            };
            while self.nodes.len() < wanted {
                let from = self.nodes.last().copied().unwrap_or(anchor);
                self.nodes.push(from);
                self.prev.push(from);
            }
            self.nodes.truncate(wanted);
            self.prev.truncate(wanted);
            while self.cords.len() < cords.len() {
                let i = self.cords.len();
                let from = self.nodes.get((i + 2) * PER).copied().unwrap_or(anchor);
                self.cords.push(from);
                self.cords_prev.push(from);
            }
            self.cords.truncate(cords.len());
            self.cords_prev.truncate(cords.len());
            if wanted == 0 {
                return;
            }
            let inputs: Vec<[f32; 2]> = std::iter::once(anchor)
                .chain(beads.iter().copied())
                .chain(cords.iter().copied())
                .collect();
            let same = inputs.len() == self.inputs.len()
                && inputs
                    .iter()
                    .zip(&self.inputs)
                    .all(|(a, b)| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01);
            self.inputs = inputs;
            if !same {
                self.asleep = false;
            }
            if self.asleep {
                self.clock = 0.0;
                return;
            }
            let point = |k: usize| if k == 0 { anchor } else { beads[k - 1] };
            let rest: Vec<f32> = (0..beads.len())
                .map(|k| {
                    let (a, b) = (point(k), point(k + 1));
                    (b[0] - a[0]).hypot(b[1] - a[1]) / PER as f32 * slack
                })
                .collect();
            let h = 1.0 / 240.0;
            self.clock = (self.clock + dt).min(h * 60.0);
            let steps = (self.clock / h).floor() as usize;
            self.clock -= steps as f32 * h;
            let keep = (-drag * h).exp();
            for step in 0..steps {
                let t = time + h * step as f32;
                self.nodes[0] = anchor;
                self.prev[0] = anchor;
                for j in 1..self.nodes.len() {
                    let p = self.nodes[j];
                    let v = [
                        (p[0] - self.prev[j][0]) * keep,
                        (p[1] - self.prev[j][1]) * keep,
                    ];
                    let w = reference_drift(p, t);
                    let mut f = [w[0] * flow, w[1] * flow];
                    if j % PER == 0 {
                        let goal = point(j / PER);
                        f[0] += pull * (goal[0] - p[0]);
                        f[1] += pull * (goal[1] - p[1]);
                    }
                    self.prev[j] = p;
                    self.nodes[j] = [p[0] + v[0] + f[0] * h * h, p[1] + v[1] + f[1] * h * h];
                }
                for (i, goal) in cords.iter().enumerate() {
                    let p = self.cords[i];
                    let v = [
                        (p[0] - self.cords_prev[i][0]) * keep,
                        (p[1] - self.cords_prev[i][1]) * keep,
                    ];
                    let w = reference_drift(p, t + 3.0);
                    let f = [
                        w[0] * flow + pull * 1.6 * (goal[0] - p[0]),
                        w[1] * flow + pull * 1.6 * (goal[1] - p[1]),
                    ];
                    self.cords_prev[i] = p;
                    self.cords[i] = [p[0] + v[0] + f[0] * h * h, p[1] + v[1] + f[1] * h * h];
                }
                for _ in 0..12 {
                    for j in 0..self.nodes.len() - 1 {
                        let (a, b) = (self.nodes[j], self.nodes[j + 1]);
                        let d = [b[0] - a[0], b[1] - a[1]];
                        let len = d[0].hypot(d[1]).max(1e-4);
                        let target = rest[(j / PER).min(rest.len() - 1)];
                        let fix = (len - target) / len;
                        if j == 0 {
                            self.nodes[1] = [b[0] - d[0] * fix, b[1] - d[1] * fix];
                        } else {
                            self.nodes[j] = [a[0] + d[0] * fix * 0.5, a[1] + d[1] * fix * 0.5];
                            self.nodes[j + 1] = [b[0] - d[0] * fix * 0.5, b[1] - d[1] * fix * 0.5];
                        }
                    }
                    for i in 0..self.cords.len() {
                        let Some(a) = self.nodes.get((i + 2) * PER).copied() else {
                            continue;
                        };
                        let b = self.cords[i];
                        let d = [b[0] - a[0], b[1] - a[1]];
                        let len = d[0].hypot(d[1]).max(1e-4);
                        let fix = (len - cord) / len;
                        self.cords[i] = [b[0] - d[0] * fix, b[1] - d[1] * fix];
                    }
                }
            }
            let moving = |now: &[[f32; 2]], was: &[[f32; 2]]| {
                now.iter()
                    .zip(was)
                    .any(|(a, b)| (a[0] - b[0]).abs() > 0.01 || (a[1] - b[1]).abs() > 0.01)
            };
            if steps > 0
                && !moving(&self.nodes, &self.prev)
                && !moving(&self.cords, &self.cords_prev)
            {
                self.prev = self.nodes.clone();
                self.cords_prev = self.cords.clone();
                self.asleep = true;
            }
        }

        fn bead(&self, k: usize) -> Option<[f32; 2]> {
            self.nodes.get((k + 1) * PER).copied()
        }

        fn snap(&mut self, anchor: [f32; 2], beads: &[[f32; 2]], cords: &[[f32; 2]]) {
            let point = |k: usize| if k == 0 { anchor } else { beads[k - 1] };
            self.nodes = vec![anchor];
            for k in 0..beads.len() {
                let (a, b) = (point(k), point(k + 1));
                for j in 1..=PER {
                    let t = j as f32 / PER as f32;
                    self.nodes
                        .push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                }
            }
            self.prev = self.nodes.clone();
            self.cords = cords.to_vec();
            self.cords_prev = cords.to_vec();
        }

        fn wires(&self, out: &mut Vec<[f32; 4]>) {
            let at = |i: isize| self.nodes[i.clamp(0, self.nodes.len() as isize - 1) as usize];
            for i in 0..self.nodes.len().saturating_sub(1) {
                let (p0, p1, p2, p3) = (
                    at(i as isize - 1),
                    at(i as isize),
                    at(i as isize + 1),
                    at(i as isize + 2),
                );
                let mut prev = p1;
                for k in 1..=3 {
                    let t = k as f32 / 3.0;
                    let (t2, t3) = (t * t, t * t * t);
                    let f = |a: f32, b: f32, c: f32, d: f32| {
                        0.5 * (2.0 * b
                            + (-a + c) * t
                            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                            + (-a + 3.0 * b - 3.0 * c + d) * t3)
                    };
                    let next = [f(p0[0], p1[0], p2[0], p3[0]), f(p0[1], p1[1], p2[1], p3[1])];
                    out.push([prev[0], prev[1], next[0], next[1]]);
                    prev = next;
                }
            }
            for (i, end) in self.cords.iter().enumerate() {
                if let Some(start) = self.nodes.get((i + 2) * PER) {
                    out.push([start[0], start[1], end[0], end[1]]);
                }
            }
        }
    }

    #[test]
    fn pinned_rope_keeps_its_length() {
        let start = [0.0, 0.0];
        let end = [10.0, 0.0];
        let mut rope = Rope::between(start, end, 10, 1.15);
        let dt = SUBSTEP * 4.0;
        for frame in 0..180 {
            rope.step_at(dt, frame as f32 * dt, [0.0, -400.0], 6.0);
        }
        assert_eq!(rope.nodes()[0], start);
        assert_eq!(*rope.nodes().last().unwrap(), end);
        let err = rope.length_error();
        assert!(err < 0.03, "length error {err}");
    }

    #[test]
    fn chain_matches_reference_step_for_step() {
        let mut chain = Chain::new(REFERENCE);
        let mut reference = ReferenceChain::default();
        let anchor = [20.0, 40.0];
        let mut beads = vec![[55.0, 45.0], [89.0, 60.0]];
        let mut cords = vec![[95.0, 85.0]];
        let dt = 1.0 / 60.0;
        let mut slept = false;
        for frame in 0..4000 {
            if frame == 30 {
                beads.push([120.0, 75.0]);
                cords.push([125.0, 100.0]);
            }
            if frame == 60 {
                beads.remove(1);
                cords.pop();
            }
            if frame == 90 {
                beads[0] = [62.0, 53.0];
                beads[1] = [117.0, 72.0];
                cords[0] = [118.0, 95.0];
                chain.snap(anchor, &beads, &cords);
                reference.snap(anchor, &beads, &cords);
            }
            let idle = if frame < 90 { 0.7 } else { 0.0 };
            let time = frame as f32 * dt;
            chain.step(dt, time, anchor, &beads, &cords, idle);
            reference.step(dt, time, anchor, &beads, &cords, idle);
            assert_eq!(chain.nodes().len(), reference.nodes.len(), "frame {frame}");
            assert_eq!(chain.cords().len(), reference.cords.len(), "frame {frame}");
            for (index, (actual, expected)) in
                chain.nodes().iter().zip(&reference.nodes).enumerate()
            {
                for axis in 0..2 {
                    assert!(
                        (actual[axis] - expected[axis]).abs() < 0.01,
                        "frame {frame} node {index} axis {axis}: {} != {}",
                        actual[axis],
                        expected[axis]
                    );
                }
            }
            for (index, (actual, expected)) in
                chain.cords().iter().zip(&reference.cords).enumerate()
            {
                for axis in 0..2 {
                    assert!(
                        (actual[axis] - expected[axis]).abs() < 0.01,
                        "frame {frame} cord {index} axis {axis}: {} != {}",
                        actual[axis],
                        expected[axis]
                    );
                }
            }
            assert_eq!(chain.asleep(), reference.asleep, "frame {frame}");
            for index in 0..beads.len() {
                let actual = chain.bead(index).unwrap();
                let from_node = chain.nodes()[(index + 1) * PER];
                assert_eq!(actual, from_node);
                assert_eq!(actual, reference.bead(index).unwrap());
            }
            let mut expected_wires = Vec::new();
            reference.wires(&mut expected_wires);
            assert_eq!(chain.wires(), expected_wires);
            if frame >= 90 && chain.asleep() {
                slept = true;
                break;
            }
        }
        assert!(slept);
        beads[0][0] += 0.02;
        chain.step(0.0, 70.0, anchor, &beads, &cords, 0.0);
        reference.step(0.0, 70.0, anchor, &beads, &cords, 0.0);
        assert_eq!(chain.asleep(), reference.asleep);
        assert!(!chain.asleep());
    }

    #[test]
    fn chain_wire_counts_and_cap() {
        let mut chain = Chain::default();
        let beads = [[10.0, 0.0], [20.0, 0.0]];
        let cords = [[20.0, 16.0]];
        assert!(chain.wires().is_empty());
        chain.snap([0.0, 0.0], &beads, &cords);
        assert_eq!(chain.wires().len(), 31);
        assert_eq!(chain.wires()[0][0..2], [0.0, 0.0]);
        assert_eq!(chain.wires()[2][2..4], [2.0, 0.0]);
        assert_eq!(chain.wires()[30], [20.0, 0.0, 20.0, 16.0]);
        let many: Vec<_> = (1..=18).map(|i| [i as f32 * 10.0, 0.0]).collect();
        chain.snap([0.0, 0.0], &many, &[]);
        assert_eq!(chain.wires().len(), 256);
    }

    #[test]
    fn chain_is_deterministic() {
        let params = ChainParams::default();
        let mut a = Chain::new(params);
        let mut b = Chain::new(params);
        let beads = [[15.0, 9.0], [30.0, 20.0]];
        let cords = [[32.0, 40.0]];
        for frame in 0..120 {
            let dt = if frame % 3 == 0 { 0.033 } else { 0.011 };
            let time = frame as f32 * 0.019;
            a.step(dt, time, [0.0, 0.0], &beads, &cords, 0.6);
            b.step(dt, time, [0.0, 0.0], &beads, &cords, 0.6);
            assert_eq!(a.nodes(), b.nodes());
            assert_eq!(a.cords(), b.cords());
            assert_eq!(a.asleep(), b.asleep());
        }
    }
}
