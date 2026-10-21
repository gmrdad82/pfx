use super::Rng;

pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const STEP: f32 = 1.0 / 120.0;
pub const PREWARM_MAX: f32 = 30.0;
pub const AIR: f32 = 1.225;
pub const REST_SPEED: f32 = 0.05;
pub const SPIN_MAX: f32 = 30.0;

pub type Quat = [f32; 4];

pub const UPRIGHT: Quat = [0.0, 0.0, 0.0, 1.0];

const LIFT_ATTACHED: f32 = 5.2;
const LIFT_STALLED: f32 = 0.95;
const DRAG_EDGE: f32 = 0.1;
const DRAG_ATTACHED: f32 = 5.0;
const DRAG_BROADSIDE: f32 = 1.9;
const PRESSURE_EDGE: f32 = 0.3;
const PRESSURE_FALL: f32 = 3.5;
const PRESSURE_BROADSIDE: f32 = 0.2;
const STALL: f32 = 14.0 * std::f32::consts::PI / 180.0;
const STALL_WIDTH: f32 = 6.0 * std::f32::consts::PI / 180.0;
const LUMP_DRAG: f32 = 1.0;

pub trait Field {
    fn velocity(&self, at: [f32; 3]) -> [f32; 3];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub height: f32,
    pub bounce: f32,
    pub friction: f32,
    pub keep: bool,
    pub lift: f32,
}

impl Default for Ground {
    fn default() -> Self {
        Self {
            height: 0.0,
            bounce: 0.1,
            friction: 0.8,
            keep: true,
            lift: 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub mass: f32,
    pub area: f32,
    pub flat: f32,
    pub tumble: [f32; 2],
    pub ground: Option<Ground>,
}

impl Default for Body {
    fn default() -> Self {
        Self {
            mass: 0.0005,
            area: 0.002,
            flat: 1.0,
            tumble: [0.0, 6.0],
            ground: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub position: [f32; 3],
    pub yaw: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub position: [f32; 3],
    pub previous: [f32; 3],
    pub velocity: [f32; 3],
    pub orientation: Quat,
    pub previous_orientation: Quat,
    pub angular: [f32; 3],
    pub size: f32,
    pub colour: [f32; 3],
    pub alpha: f32,
    pub resting: bool,
}

impl Piece {
    pub fn matrix(&self) -> Matrix {
        placement(self.position, self.orientation, self.size)
    }

    pub fn previous_matrix(&self) -> Matrix {
        placement(self.previous, self.previous_orientation, self.size)
    }

    pub fn back(&self, span: f32) -> Matrix {
        if self.resting || span.is_nan() || span <= 0.0 {
            return self.matrix();
        }
        let position = std::array::from_fn(|i| self.position[i] - self.velocity[i] * span);
        let turn = self.angular.map(|w| -w * span);
        placement(position, turned(self.orientation, turn), self.size)
    }
}

struct Spawn<'a> {
    dt: f32,
    late: f32,
    start: [f32; 3],
    end: [f32; 3],
    parent: [f32; 3],
    axis: [[f32; 3]; 2],
    field: Option<&'a dyn Field>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    #[default]
    Alpha,
    Add,
}

pub trait Lerp: Copy {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for [f32; 2] {
    fn lerp(self, other: Self, t: f32) -> Self {
        std::array::from_fn(|i| self[i] + (other[i] - self[i]) * t)
    }
}

impl Lerp for [f32; 3] {
    fn lerp(self, other: Self, t: f32) -> Self {
        std::array::from_fn(|i| self[i] + (other[i] - self[i]) * t)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient<T> {
    keys: Vec<(f32, T)>,
}

impl<T: Lerp> Gradient<T> {
    pub fn constant(value: T) -> Self {
        Self {
            keys: vec![(0.0, value)],
        }
    }

    pub fn new(mut keys: Vec<(f32, T)>, fallback: T) -> Self {
        keys.retain(|(t, _)| t.is_finite());
        if keys.is_empty() {
            return Self::constant(fallback);
        }
        keys.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { keys }
    }

    pub fn keys(&self) -> &[(f32, T)] {
        &self.keys
    }

    pub fn sample(&self, ratio: f32) -> T {
        match self.keys.binary_search_by(|key| key.0.total_cmp(&ratio)) {
            Ok(mut index) => {
                while index > 0 && self.keys[index - 1].0 == self.keys[index].0 {
                    index -= 1;
                }
                self.keys[index].1
            }
            Err(0) => self.keys[0].1,
            Err(upper) if upper >= self.keys.len() => self.keys[upper - 1].1,
            Err(upper) => {
                let (t0, v0) = self.keys[upper - 1];
                let (t1, v1) = self.keys[upper];
                v0.lerp(v1, (ratio - t0) / (t1 - t0))
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeKeys {
    pub rate: Option<Gradient<f32>>,
    pub speed: Option<Gradient<[f32; 2]>>,
    pub size: Option<Gradient<[f32; 2]>>,
    pub colour: Option<Gradient<[f32; 3]>>,
    pub alpha: Option<Gradient<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub rate: f32,
    pub keys: RecipeKeys,
    pub burst: u32,
    pub max: u32,
    pub lifetime: [f32; 2],
    pub speed: [f32; 2],
    pub direction: [f32; 3],
    pub cone: f32,
    pub inherit: f32,
    pub gravity: [f32; 3],
    pub drag: f32,
    pub size: [f32; 2],
    pub alpha: Gradient<f32>,
    pub colour: Gradient<[f32; 3]>,
    pub spin: [f32; 2],
    pub stretch: f32,
    pub blend: Blend,
    pub lit: bool,
    pub seed: u64,
    pub prewarm: f32,
    pub body: Option<Body>,
    pub placed: Vec<Placed>,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            rate: 0.0,
            keys: RecipeKeys::default(),
            burst: 0,
            max: 1024,
            lifetime: [1.0, 1.0],
            speed: [1.0, 1.0],
            direction: [0.0, 1.0, 0.0],
            cone: 0.0,
            inherit: 0.0,
            gravity: [0.0; 3],
            drag: 0.0,
            size: [0.1, 0.1],
            alpha: Gradient::new(vec![(0.0, 1.0), (1.0, 0.0)], 1.0),
            colour: Gradient::constant([1.0; 3]),
            spin: [0.0, 0.0],
            stretch: 0.0,
            blend: Blend::Alpha,
            lit: true,
            seed: 0,
            prewarm: 0.0,
            body: None,
            placed: Vec::new(),
        }
    }
}

impl Recipe {
    pub fn rate_at(&self, time: f32) -> f32 {
        self.keys
            .rate
            .as_ref()
            .map_or(self.rate, |keys| keys.sample(time))
            .max(0.0)
    }

    pub fn speed_at(&self, time: f32) -> [f32; 2] {
        self.keys
            .speed
            .as_ref()
            .map_or(self.speed, |keys| keys.sample(time))
    }

    pub fn prewarm_steps(&self) -> u64 {
        if !(self.prewarm.is_finite() && self.prewarm > 0.0) {
            return 0;
        }
        (self.prewarm.min(PREWARM_MAX) / STEP + 1e-3).floor() as u64
    }

    pub fn size_at(&self, time: f32) -> [f32; 2] {
        self.keys
            .size
            .as_ref()
            .map_or(self.size, |keys| keys.sample(time))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub previous: [f32; 3],
    pub velocity: [f32; 3],
    pub angle: f32,
    pub spin: f32,
    pub age: f32,
    pub life: f32,
    pub orientation: Quat,
    pub previous_orientation: Quat,
    pub angular: [f32; 3],
    pub resting: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub position: [f32; 3],
    pub previous: [f32; 3],
    pub velocity: [f32; 3],
    pub size: f32,
    pub angle: f32,
    pub colour: [f32; 3],
    pub alpha: f32,
}

#[derive(Clone, Debug)]
pub struct Emitter {
    recipe: Recipe,
    at: [f32; 3],
    rng: Rng,
    remainder: f32,
    time: f32,
    steps: u64,
    started: bool,
    warmed: bool,
    dropped: u64,
    particles: Vec<Particle>,
}

impl Emitter {
    pub fn new(recipe: Recipe, at: [f32; 3]) -> Self {
        let rng = Rng::new(recipe.seed);
        let capacity = (recipe.max as usize).min(1 << 16);
        Self {
            recipe,
            at,
            rng,
            remainder: 0.0,
            time: 0.0,
            steps: 0,
            started: false,
            warmed: false,
            dropped: 0,
            particles: Vec::with_capacity(capacity),
        }
    }

    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    pub fn at(&self) -> [f32; 3] {
        self.at
    }

    pub fn reset(&mut self) {
        self.rng = Rng::new(self.recipe.seed);
        self.remainder = 0.0;
        self.time = 0.0;
        self.steps = 0;
        self.started = false;
        self.warmed = false;
        self.dropped = 0;
        self.particles.clear();
    }

    pub fn skip_prewarm(&mut self) {
        self.warmed = true;
    }

    pub fn warm(&mut self, at: Matrix) {
        if self.warmed {
            return;
        }
        self.warmed = true;
        for _ in 0..self.recipe.prewarm_steps() {
            self.advance(STEP, at, at, true, None);
        }
    }

    pub fn time(&self) -> f32 {
        self.time
    }

    pub fn live(&self) -> usize {
        self.particles.len()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    pub fn step(&mut self, dt: f32, from: Matrix, to: Matrix) {
        self.step_in(dt, from, to, None);
    }

    pub fn step_in(&mut self, dt: f32, from: Matrix, to: Matrix, field: Option<&dyn Field>) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        self.warm(from);
        self.advance(dt, from, to, false, field);
        self.time += dt;
        self.steps += 1;
    }

    fn advance(
        &mut self,
        dt: f32,
        from: Matrix,
        to: Matrix,
        held: bool,
        field: Option<&dyn Field>,
    ) {
        let keep = (-self.recipe.drag.max(0.0) * dt).exp();
        let gravity = self.recipe.gravity;
        let body = self.recipe.body;
        let kept = body
            .and_then(|body| body.ground)
            .is_some_and(|ground| ground.keep);
        let rng = &mut self.rng;
        let particles = &mut self.particles;
        let mut index = 0;
        while index < particles.len() {
            let particle = &mut particles[index];
            particle.previous = particle.position;
            particle.previous_orientation = particle.orientation;
            if !(kept && particle.resting) {
                particle.age += dt;
            }
            if particle.age >= particle.life {
                particles.swap_remove(index);
                continue;
            }
            match &body {
                Some(body) => fly(particle, body, gravity, dt, field, rng),
                None => integrate(particle, gravity, keep, dt, field),
            }
            index += 1;
        }
        let rate = if held {
            self.recipe.rate_at(self.time)
        } else {
            0.5 * (self.recipe.rate_at(self.time) + self.recipe.rate_at(self.time + dt))
        };
        let late = if held { 0.0 } else { dt };
        self.remainder += rate * dt;
        let count = self.remainder.floor();
        self.remainder -= count;
        let start = point(from, self.at);
        let end = point(to, self.at);
        let parent = std::array::from_fn(|i| (end[i] - start[i]) / dt);
        let axis = [
            unit(vector(from, self.recipe.direction)),
            unit(vector(to, self.recipe.direction)),
        ];
        let spawn = Spawn {
            dt,
            late,
            start,
            end,
            parent,
            axis,
            field,
        };
        if !self.started {
            self.started = true;
            for _ in 0..self.recipe.burst {
                self.spawn(0.0, &spawn);
            }
            for index in 0..self.recipe.placed.len() {
                self.place(index, from);
            }
        }
        let count = count as u32;
        for i in 0..count {
            let born = (i as f32 + 0.5) / count as f32;
            self.spawn(born, &spawn);
        }
    }

    fn spawn(&mut self, born: f32, at: &Spawn<'_>) {
        if self.particles.len() >= self.recipe.max as usize {
            self.dropped += 1;
            return;
        }
        let recipe = &self.recipe;
        let rng = &mut self.rng;
        let direction = cone(rng, unit(at.axis[0].lerp(at.axis[1], born)), recipe.cone);
        let speed = between(rng, recipe.speed_at(self.time + at.late * born));
        let life = between(rng, recipe.lifetime).max(1e-3);
        let angle = rng.next_f32() * std::f32::consts::TAU;
        let spin = between(rng, recipe.spin) * if rng.next_bool() { 1.0 } else { -1.0 };
        let (orientation, angular) = match &recipe.body {
            Some(body) => {
                let orientation = random_turn(rng);
                let axis = random_axis(rng);
                let rate = between(rng, body.tumble);
                (orientation, axis.map(|w| w * rate))
            }
            None => (UPRIGHT, [0.0; 3]),
        };
        let position = at.start.lerp(at.end, born);
        let velocity =
            std::array::from_fn(|i| direction[i] * speed + at.parent[i] * recipe.inherit);
        let rest = at.dt * (1.0 - born);
        let mut particle = Particle {
            position,
            previous: position,
            velocity,
            angle,
            spin,
            age: rest,
            life,
            orientation,
            previous_orientation: orientation,
            angular,
            resting: false,
        };
        if particle.age >= particle.life {
            return;
        }
        if rest > 0.0 {
            match &recipe.body {
                Some(body) => fly(&mut particle, body, recipe.gravity, rest, at.field, rng),
                None => {
                    let keep = (-recipe.drag.max(0.0) * rest).exp();
                    integrate(&mut particle, recipe.gravity, keep, rest, at.field);
                }
            }
        }
        self.particles.push(particle);
    }

    fn place(&mut self, index: usize, from: Matrix) {
        if self.particles.len() >= self.recipe.max as usize {
            self.dropped += 1;
            return;
        }
        let placed = self.recipe.placed[index];
        let position = point(from, placed.position);
        let half = 0.5 * placed.yaw.to_radians();
        let orientation = [0.0, half.sin(), 0.0, half.cos()];
        let life = between(&mut self.rng, self.recipe.lifetime).max(1e-3);
        let resting = self.recipe.body.is_some_and(|body| body.ground.is_some());
        self.particles.push(Particle {
            position,
            previous: position,
            velocity: [0.0; 3],
            angle: 0.0,
            spin: 0.0,
            age: 0.0,
            life,
            orientation,
            previous_orientation: orientation,
            angular: [0.0; 3],
            resting,
        });
    }

    pub fn seek(&mut self, time: f32, step: f32, mut parent: impl FnMut(f32) -> Matrix) {
        if !(step.is_finite() && step > 0.0 && time.is_finite()) {
            return;
        }
        let target = (time.max(0.0) / step + 1e-3).floor() as u64;
        if target < self.steps {
            self.reset();
        }
        if self.steps == 0 {
            self.warm(parent(0.0));
        }
        while self.steps < target {
            let t0 = self.steps as f32 * step;
            let from = parent(t0);
            let to = parent(t0 + step);
            self.step(step, from, to);
        }
    }

    pub fn sprites(&self, out: &mut Vec<Sprite>) {
        self.shown(|particle, size, colour, alpha| {
            out.push(Sprite {
                position: particle.position,
                previous: particle.previous,
                velocity: particle.velocity,
                size,
                angle: particle.angle,
                colour,
                alpha,
            });
        });
    }

    pub fn pieces(&self, out: &mut Vec<Piece>) {
        self.shown(|particle, size, colour, alpha| {
            out.push(Piece {
                position: particle.position,
                previous: particle.previous,
                velocity: particle.velocity,
                orientation: particle.orientation,
                previous_orientation: particle.previous_orientation,
                angular: particle.angular,
                size,
                colour,
                alpha,
                resting: particle.resting,
            });
        });
    }

    fn shown(&self, mut visit: impl FnMut(&Particle, f32, [f32; 3], f32)) {
        let recipe = &self.recipe;
        let sizes = recipe.size_at(self.time);
        let fade = recipe
            .keys
            .alpha
            .as_ref()
            .map(|keys| keys.sample(self.time));
        let tint = recipe
            .keys
            .colour
            .as_ref()
            .map(|keys| keys.sample(self.time));
        for particle in &self.particles {
            let t = (particle.age / particle.life).clamp(0.0, 1.0);
            let size = sizes[0].lerp(sizes[1], t);
            let alpha = match fade {
                Some(fade) => recipe.alpha.sample(t) * fade,
                None => recipe.alpha.sample(t),
            }
            .clamp(0.0, 1.0);
            if !(size > 0.0 && alpha > 0.0) {
                continue;
            }
            visit(
                particle,
                size,
                tint.unwrap_or_else(|| recipe.colour.sample(t)),
                alpha,
            );
        }
    }
}

fn integrate(
    particle: &mut Particle,
    gravity: [f32; 3],
    keep: f32,
    dt: f32,
    field: Option<&dyn Field>,
) {
    let wind = field.map(|field| field.velocity(particle.position));
    for (axis, pull) in gravity.iter().enumerate() {
        let velocity = particle.velocity[axis] + pull * dt;
        particle.velocity[axis] = match wind {
            Some(wind) => wind[axis] + (velocity - wind[axis]) * keep,
            None => velocity * keep,
        };
        particle.position[axis] += particle.velocity[axis] * dt;
    }
    particle.angle += particle.spin * dt;
}

fn fly(
    particle: &mut Particle,
    body: &Body,
    gravity: [f32; 3],
    dt: f32,
    field: Option<&dyn Field>,
    rng: &mut Rng,
) {
    let wind = field.map_or([0.0; 3], |field| field.velocity(particle.position));
    if particle.resting {
        if length(wind) <= body.ground.map_or(0.0, |ground| ground.lift) {
            return;
        }
        particle.resting = false;
        let axis = random_axis(rng);
        let rate = between(rng, body.tumble);
        particle.angular = axis.map(|w| w * rate);
    }
    let mass = body.mass.max(1e-6);
    let area = body.area.max(0.0);
    let flat = body.flat.clamp(0.0, 1.0);
    let chord = area.sqrt().max(1e-4);
    let k = 0.5 * AIR * area / mass;
    let air = sub(particle.velocity, wind);
    let speed = length(air);
    let mut drag = LUMP_DRAG * (1.0 - flat);
    let mut lift = [0.0; 3];
    let mut torque = [0.0; 3];
    if speed > 1e-6 {
        let along = air.map(|v| v / speed);
        let normal = rotate(particle.orientation, [0.0, 1.0, 0.0]);
        let facing = dot(normal, along);
        let attack = facing.abs().min(1.0).asin();
        let sin = attack.sin();
        let attached = 0.5 * (1.0 - ((attack - STALL) / STALL_WIDTH).tanh());
        let cd = attached * (DRAG_EDGE + DRAG_ATTACHED * sin * sin)
            + (1.0 - attached) * DRAG_BROADSIDE * sin * sin;
        let cl =
            attached * LIFT_ATTACHED * sin + (1.0 - attached) * LIFT_STALLED * (2.0 * attack).sin();
        let pressure = attached * (PRESSURE_EDGE - PRESSURE_FALL * attack * attack)
            + (1.0 - attached) * PRESSURE_BROADSIDE * (1.0 - attack / std::f32::consts::FRAC_PI_2);
        drag += flat * cd;
        let side = sub(normal, along.map(|v| v * facing));
        let across = length(side);
        if across > 1e-6 && flat > 0.0 {
            let sign = if facing >= 0.0 { -1.0 } else { 1.0 };
            let push = (flat * cl * k * speed * speed * dt).min(0.5 * speed);
            lift = side.map(|v| v * sign * push / across);
        }
        let chordwise = sub(along, normal.map(|v| v * facing));
        let reach = length(chordwise);
        if reach > 1e-6 && flat > 0.0 {
            let lead = chordwise.map(|v| v / reach);
            let force: [f32; 3] =
                std::array::from_fn(|i| -k * drag * speed * air[i] + lift[i] / dt);
            let arm = 12.0 * flat * pressure / chord;
            torque = cross(lead, force).map(|v| v * arm);
        }
    }
    let damping = 1.0 + k * drag * speed * dt;
    let spun = 12.0 * AIR * DRAG_BROADSIDE * chord * chord * chord / (64.0 * mass);
    for axis in 0..3 {
        let next = (air[axis] + gravity[axis] * dt + lift[axis]) / damping;
        particle.velocity[axis] = wind[axis] + next;
        particle.angular[axis] += torque[axis] * dt;
    }
    let rate = length(particle.angular);
    let slowed = 1.0 / (1.0 + spun * rate * dt);
    let capped = if rate * slowed > SPIN_MAX {
        SPIN_MAX / rate
    } else {
        slowed
    };
    particle.angular = particle.angular.map(|w| w * capped);
    particle.orientation = turned(particle.orientation, particle.angular.map(|w| w * dt));
    for axis in 0..3 {
        particle.position[axis] += particle.velocity[axis] * dt;
    }
    let Some(ground) = body.ground else {
        return;
    };
    if particle.position[1] > ground.height {
        return;
    }
    particle.position[1] = ground.height;
    if particle.velocity[1] < 0.0 {
        particle.velocity[1] = -particle.velocity[1] * ground.bounce.clamp(0.0, 1.0);
    }
    let friction = ground.friction.clamp(0.0, 1.0);
    let keep = 1.0 - friction;
    particle.velocity[0] *= keep;
    particle.velocity[2] *= keep;
    particle.angular = particle.angular.map(|w| w * keep);
    let flat = laid(particle.orientation);
    particle.orientation = blend(particle.orientation, flat, friction);
    if length(particle.velocity) < REST_SPEED && length(wind) <= ground.lift {
        particle.resting = true;
        particle.velocity = [0.0; 3];
        particle.angular = [0.0; 3];
        particle.orientation = flat;
    }
}

pub fn placement(position: [f32; 3], orientation: Quat, size: f32) -> Matrix {
    let x = rotate(orientation, [1.0, 0.0, 0.0]).map(|v| v * size);
    let y = rotate(orientation, [0.0, 1.0, 0.0]).map(|v| v * size);
    let z = rotate(orientation, [0.0, 0.0, 1.0]).map(|v| v * size);
    [
        [x[0], x[1], x[2], 0.0],
        [y[0], y[1], y[2], 0.0],
        [z[0], z[1], z[2], 0.0],
        [position[0], position[1], position[2], 1.0],
    ]
}

pub fn rotate(q: Quat, v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let s = cross(u, t);
    std::array::from_fn(|i| v[i] + q[3] * t[i] + s[i])
}

pub fn turned(q: Quat, by: [f32; 3]) -> Quat {
    let angle = length(by);
    if angle.is_nan() || angle <= 1e-9 {
        return q;
    }
    let (sin, cos) = (0.5 * angle).sin_cos();
    let d = [
        by[0] / angle * sin,
        by[1] / angle * sin,
        by[2] / angle * sin,
        cos,
    ];
    normal(multiply(d, q))
}

pub fn laid(q: Quat) -> Quat {
    let x = rotate(q, [1.0, 0.0, 0.0]);
    let yaw = if x[0].abs() + x[2].abs() > 1e-4 {
        (-x[2]).atan2(x[0])
    } else {
        let z = rotate(q, [0.0, 0.0, 1.0]);
        z[0].atan2(z[2])
    };
    let (sin, cos) = (0.5 * yaw).sin_cos();
    let upright = [0.0, sin, 0.0, cos];
    if rotate(q, [0.0, 1.0, 0.0])[1] >= 0.0 {
        upright
    } else {
        multiply(upright, [1.0, 0.0, 0.0, 0.0])
    }
}

fn blend(a: Quat, b: Quat, t: f32) -> Quat {
    let sign = if a.iter().zip(&b).map(|(x, y)| x * y).sum::<f32>() < 0.0 {
        -1.0
    } else {
        1.0
    };
    normal(std::array::from_fn(|i| a[i] + (sign * b[i] - a[i]) * t))
}

fn multiply(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn normal(q: Quat) -> Quat {
    let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length > 1e-12 {
        q.map(|v| v / length)
    } else {
        UPRIGHT
    }
}

fn random_turn(rng: &mut Rng) -> Quat {
    let u = rng.next_f32();
    let (a, b) = ((1.0 - u).sqrt(), u.sqrt());
    let (s1, c1) = (rng.next_f32() * std::f32::consts::TAU).sin_cos();
    let (s2, c2) = (rng.next_f32() * std::f32::consts::TAU).sin_cos();
    [a * s1, a * c1, b * s2, b * c2]
}

fn random_axis(rng: &mut Rng) -> [f32; 3] {
    let z = 2.0 * rng.next_f32() - 1.0;
    let (s, c) = (rng.next_f32() * std::f32::consts::TAU).sin_cos();
    let r = (1.0 - z * z).max(0.0).sqrt();
    [r * c, r * s, z]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn length(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}
fn between(rng: &mut Rng, range: [f32; 2]) -> f32 {
    range[0] + (range[1] - range[0]) * rng.next_f32()
}

fn cone(rng: &mut Rng, n: [f32; 3], degrees: f32) -> [f32; 3] {
    let u = rng.next_f32();
    let v = rng.next_f32();
    let half = degrees.clamp(0.0, 180.0).to_radians();
    let cos_theta = 1.0 - u * (1.0 - half.cos());
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = v * std::f32::consts::TAU;
    let (tangent, bitangent) = basis(n);
    let (s, c) = phi.sin_cos();
    std::array::from_fn(|i| (tangent[i] * c + bitangent[i] * s) * sin_theta + n[i] * cos_theta)
}

pub fn basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let sign = if n[2] >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (sign + n[2]);
    let b = n[0] * n[1] * a;
    (
        [1.0 + sign * n[0] * n[0] * a, sign * b, -sign * n[0]],
        [b, sign + n[1] * n[1] * a, -n[1]],
    )
}

pub fn point(m: Matrix, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

fn vector(m: Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 1e-12 {
        v.map(|x| x / length)
    } else {
        [0.0, 1.0, 0.0]
    }
}
