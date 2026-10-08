use crate::anim::math::lerp3;
use crate::fx::Comfort;
use crate::motion::smoothstep;
use crate::sim::{cosf, expf, sinf};

pub type Vec3 = [f32; 3];

const UP: Vec3 = [0.0, 1.0, 0.0];
const TAU: f32 = std::f32::consts::TAU;

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Vec3, by: f32) -> Vec3 {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn length(a: Vec3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn response(dt: f32, time: f32) -> f32 {
    if time.is_finite() && time > 1e-6 {
        1.0 - expf(-dt / time)
    } else {
        1.0
    }
}

fn damped(from: f32, to: f32, dt: f32, time: f32) -> f32 {
    from + (to - from) * response(dt, time)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub at: Vec3,
    pub look_at: Vec3,
    pub up: Vec3,
}

impl View {
    pub fn lerp(self, to: View, t: f32) -> View {
        View {
            at: lerp3(self.at, to.at, t),
            look_at: lerp3(self.look_at, to.look_at, t),
            up: lerp3(self.up, to.up, t),
        }
    }

    pub fn forward(&self) -> Vec3 {
        let direction = sub(self.look_at, self.at);
        let long = length(direction);
        if long > 1e-9 {
            scale(direction, 1.0 / long)
        } else {
            [0.0, 0.0, -1.0]
        }
    }
}

pub trait Probe {
    fn clear(&self, from: Vec3, direction: Vec3, radius: f32, max: f32) -> f32;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Open;

impl Probe for Open {
    fn clear(&self, _from: Vec3, _direction: Vec3, _radius: f32, max: f32) -> f32 {
        max
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    pub target: Vec3,
    pub look: [f32; 2],
    pub mouse: [f32; 2],
    pub grounded: bool,
}

impl Input {
    pub fn at(target: Vec3) -> Self {
        Self {
            target,
            look: [0.0; 2],
            mouse: [0.0; 2],
            grounded: true,
        }
    }

    pub fn looking(mut self, look: [f32; 2]) -> Self {
        self.look = look;
        self
    }

    pub fn mouse(mut self, mouse: [f32; 2]) -> Self {
        self.mouse = mouse;
        self
    }

    pub fn airborne(mut self) -> Self {
        self.grounded = false;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn clamp(&self, point: Vec3, margin: Vec3) -> Vec3 {
        std::array::from_fn(|axis| {
            let low = self.min[axis] + margin[axis];
            let high = self.max[axis] - margin[axis];
            if low > high {
                (low + high) * 0.5
            } else {
                point[axis].clamp(low, high)
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowTuning {
    pub plane: bool,
    pub offset: Vec3,
    pub arm: Vec3,
    pub dead_zone: Vec3,
    pub look_ahead: f32,
    pub look_ahead_speed: f32,
    pub look_ahead_damping: f32,
    pub damping: Vec3,
    pub bounds: Option<Bounds>,
    pub handover: f32,
    pub pixels_per_unit: Option<f32>,
}

impl FollowTuning {
    pub fn follow3() -> Self {
        Self {
            plane: false,
            offset: [0.0; 3],
            arm: [0.0, 4.0, 8.0],
            dead_zone: [0.0; 3],
            look_ahead: 0.0,
            look_ahead_speed: 4.0,
            look_ahead_damping: 0.25,
            damping: [0.2; 3],
            bounds: None,
            handover: 0.6,
            pixels_per_unit: None,
        }
    }

    pub fn follow2() -> Self {
        Self {
            plane: true,
            arm: [0.0, 0.0, 10.0],
            ..Self::follow3()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Handover {
    from: Vec3,
    elapsed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Follow {
    tuning: FollowTuning,
    margin: Vec3,
    focus: Vec3,
    previous: Vec3,
    ahead: Vec3,
    last: Option<Vec3>,
    started: bool,
    handover: Option<Handover>,
}

impl Follow {
    pub fn new(tuning: FollowTuning) -> Self {
        Self {
            tuning,
            margin: [0.0; 3],
            focus: [0.0; 3],
            previous: [0.0; 3],
            ahead: [0.0; 3],
            last: None,
            started: false,
            handover: None,
        }
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn tuning(&self) -> &FollowTuning {
        &self.tuning
    }

    pub fn tuning_mut(&mut self) -> &mut FollowTuning {
        &mut self.tuning
    }

    pub fn focus(&self) -> Vec3 {
        self.focus
    }

    pub fn set_view_half(&mut self, half: [f32; 2]) {
        self.margin = [half[0].max(0.0), half[1].max(0.0), 0.0];
    }

    pub fn hand_over(&mut self) {
        self.last = None;
        if !self.started {
            return;
        }
        if self.tuning.handover > 0.0 {
            self.handover = Some(Handover {
                from: self.focus,
                elapsed: 0.0,
            });
        } else {
            self.started = false;
        }
    }

    pub fn restart(&mut self) {
        self.started = false;
        self.handover = None;
        self.last = None;
    }

    fn bounded(&self, point: Vec3) -> Vec3 {
        match &self.tuning.bounds {
            Some(bounds) => bounds.clamp(point, self.margin),
            None => point,
        }
    }

    pub fn tick(&mut self, dt: f32, input: &Input, comfort: Comfort) {
        let tuning = self.tuning;
        let target = add(input.target, tuning.offset);
        if !self.started {
            self.focus = self.bounded(target);
            self.previous = self.focus;
            self.ahead = [0.0; 3];
            self.last = Some(input.target);
            self.started = true;
            self.handover = None;
            return;
        }
        let mut velocity = match self.last {
            Some(last) if dt > 0.0 => scale(sub(input.target, last), 1.0 / dt),
            _ => [0.0; 3],
        };
        self.last = Some(input.target);
        if tuning.plane {
            velocity[2] = 0.0;
        }
        let speed = length(velocity);
        let amount = comfort.amount();
        let wanted = if tuning.look_ahead > 0.0 && amount > 0.0 && speed > 1e-4 {
            let reach = (speed / tuning.look_ahead_speed.max(1e-4)).min(1.0);
            scale(velocity, tuning.look_ahead * amount * reach / speed)
        } else {
            [0.0; 3]
        };
        let follow = response(dt, tuning.look_ahead_damping);
        self.ahead = std::array::from_fn(|axis| {
            self.ahead[axis] + (wanted[axis] - self.ahead[axis]) * follow
        });
        let desired = add(target, self.ahead);
        self.previous = self.focus;
        match &mut self.handover {
            Some(handover) => {
                handover.elapsed += dt;
                let t = (handover.elapsed / tuning.handover.max(1e-6)).min(1.0);
                let from = handover.from;
                self.focus = lerp3(from, desired, smoothstep(t));
                if t >= 1.0 {
                    self.handover = None;
                }
            }
            None => {
                for (axis, &wanted) in desired.iter().enumerate() {
                    let gap = wanted - self.focus[axis];
                    let zone = tuning.dead_zone[axis].max(0.0);
                    let excess = if gap > zone {
                        gap - zone
                    } else if gap < -zone {
                        gap + zone
                    } else {
                        0.0
                    };
                    self.focus[axis] +=
                        excess * response(dt, finite(tuning.damping[axis], 0.0).max(0.0));
                }
            }
        }
        if tuning.plane {
            self.focus[2] = target[2];
            self.previous[2] = target[2];
        }
        self.focus = self.bounded(self.focus);
    }

    pub fn pose(&self, alpha: f32) -> View {
        let alpha = alpha.clamp(0.0, 1.0);
        let mut focus = lerp3(self.previous, self.focus, alpha);
        if self.tuning.plane
            && let Some(pixels) = self.tuning.pixels_per_unit
            && pixels > 0.0
        {
            for axis in focus.iter_mut().take(2) {
                *axis = (*axis * pixels).round() / pixels;
            }
        }
        View {
            at: add(focus, self.tuning.arm),
            look_at: focus,
            up: UP,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bob {
    pub stride: f32,
    pub vertical: f32,
    pub sway: f32,
    pub full_speed: f32,
    pub ease: f32,
}

impl Default for Bob {
    fn default() -> Self {
        Self {
            stride: 2.2,
            vertical: 0.04,
            sway: 0.02,
            full_speed: 5.0,
            ease: 0.12,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Third {
    pub distance: f32,
    pub radius: f32,
    pub margin: f32,
    pub min_distance: f32,
    pub recover: f32,
    pub shoulder: [f32; 2],
}

impl Default for Third {
    fn default() -> Self {
        Self {
            distance: 4.0,
            radius: 0.25,
            margin: 0.05,
            min_distance: 0.4,
            recover: 0.35,
            shoulder: [0.0; 2],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PersonTuning {
    pub eye: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub mouse: f32,
    pub stick: f32,
    pub invert_x: bool,
    pub invert_y: bool,
    pub smoothing: f32,
    pub pitch_range: [f32; 2],
    pub bob: Option<Bob>,
    pub third: Option<Third>,
    pub handover: f32,
}

impl Default for PersonTuning {
    fn default() -> Self {
        Self {
            eye: [0.0, 1.6, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            mouse: 0.1,
            stick: 180.0,
            invert_x: false,
            invert_y: false,
            smoothing: 0.0,
            pitch_range: [-89.0, 89.0],
            bob: None,
            third: None,
            handover: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Carry {
    offset: Vec3,
    elapsed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    tuning: PersonTuning,
    yaw: f32,
    pitch: f32,
    residual: [f32; 2],
    phase: f32,
    amplitude: f32,
    arm: f32,
    third: bool,
    last: Option<Vec3>,
    started: bool,
    changed: bool,
    carry: Option<Carry>,
    previous: View,
    current: View,
}

const FLAT: View = View {
    at: [0.0; 3],
    look_at: [0.0, 0.0, -1.0],
    up: UP,
};

impl Person {
    pub fn new(tuning: PersonTuning) -> Self {
        let mut person = Self {
            yaw: tuning.yaw.to_radians(),
            pitch: 0.0,
            tuning,
            residual: [0.0; 2],
            phase: 0.0,
            amplitude: 0.0,
            arm: 0.0,
            third: false,
            last: None,
            started: false,
            changed: false,
            carry: None,
            previous: FLAT,
            current: FLAT,
        };
        person.pitch = person.clamped_pitch(tuning.pitch.to_radians());
        person
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn tuning(&self) -> &PersonTuning {
        &self.tuning
    }

    pub fn tuning_mut(&mut self) -> &mut PersonTuning {
        &mut self.tuning
    }

    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    pub fn arm(&self) -> f32 {
        self.arm
    }

    pub fn bob_amplitude(&self) -> f32 {
        self.amplitude
    }

    pub fn is_third(&self) -> bool {
        self.third
    }

    pub fn set_third(&mut self, third: bool) {
        self.third = third && self.tuning.third.is_some();
    }

    pub fn set_look(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = self.clamped_pitch(pitch);
        self.residual = [0.0; 2];
    }

    pub fn hand_over(&mut self) {
        self.last = None;
        self.changed = self.started;
    }

    pub fn restart(&mut self) {
        self.started = false;
        self.changed = false;
        self.carry = None;
        self.last = None;
        self.residual = [0.0; 2];
    }

    fn clamped_pitch(&self, pitch: f32) -> f32 {
        let [low, high] = self.tuning.pitch_range;
        let low = low.clamp(-89.9, 89.9).to_radians();
        let high = high.clamp(-89.9, 89.9).to_radians();
        pitch.clamp(low.min(high), high.max(low))
    }

    fn directions(&self) -> (Vec3, Vec3, Vec3) {
        let (sin_yaw, cos_yaw) = (sinf(self.yaw), cosf(self.yaw));
        let (sin_pitch, cos_pitch) = (sinf(self.pitch), cosf(self.pitch));
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let right = [cos_yaw, 0.0, -sin_yaw];
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        (forward, right, up)
    }

    pub fn tick(&mut self, dt: f32, input: &Input, comfort: Comfort, probe: &dyn Probe) {
        let tuning = self.tuning;
        let sign = |inverted: bool| if inverted { -1.0 } else { 1.0 };
        let turn = [
            -(input.mouse[0] * tuning.mouse + input.look[0] * tuning.stick * dt)
                * sign(tuning.invert_x),
            (-input.mouse[1] * tuning.mouse + input.look[1] * tuning.stick * dt)
                * sign(tuning.invert_y),
        ];
        let smoothing = response(dt, tuning.smoothing);
        let mut steps = [0.0; 2];
        for ((residual, step), turn) in self.residual.iter_mut().zip(&mut steps).zip(turn) {
            *residual += finite(turn, 0.0).to_radians();
            *step = *residual * smoothing;
            *residual -= *step;
        }
        self.yaw += steps[0];
        self.pitch += steps[1];
        self.yaw = self.yaw.rem_euclid(TAU);
        self.pitch = self.clamped_pitch(self.pitch);
        let planar = match self.last {
            Some(last) if dt > 0.0 => {
                let moved = sub(input.target, last);
                (moved[0] * moved[0] + moved[2] * moved[2]).sqrt() / dt
            }
            _ => 0.0,
        };
        self.last = Some(input.target);
        let (forward, right, up) = self.directions();
        let third = tuning.third.filter(|_| self.third);
        let reach = tuning.third.map_or(0.0, |third| third.distance.max(0.0));
        let blend = if reach > 0.0 {
            (self.arm / reach).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut pivot = add(input.target, tuning.eye);
        if let Some(third) = third {
            pivot = add(
                pivot,
                add(
                    scale(right, third.shoulder[0] * blend),
                    scale(up, third.shoulder[1] * blend),
                ),
            );
        }
        match third {
            Some(third) => {
                let wanted = third.distance.max(0.0);
                let back = scale(forward, -1.0);
                let free = probe
                    .clear(pivot, back, third.radius.max(0.0), wanted + third.margin)
                    .min(wanted + third.margin)
                    - third.margin;
                let limit = free.max(third.min_distance.min(wanted)).min(wanted);
                if limit < self.arm {
                    self.arm = limit;
                } else {
                    self.arm = damped(self.arm, limit, dt, third.recover);
                }
            }
            None => {
                let recover = tuning.third.map_or(0.0, |third| third.recover);
                self.arm = damped(self.arm, 0.0, dt, recover);
                if self.arm < 1e-4 {
                    self.arm = 0.0;
                }
            }
        }
        let bob = match tuning.bob {
            Some(bob) if bob.stride > 0.0 => {
                let moving = input.grounded && planar > 0.05;
                if moving {
                    self.phase = (self.phase + planar * dt / bob.stride * TAU).rem_euclid(TAU);
                }
                let wanted = if moving {
                    (planar / bob.full_speed.max(1e-4)).min(1.0)
                } else {
                    0.0
                };
                self.amplitude = damped(self.amplitude, wanted, dt, bob.ease);
                let fade = 1.0 - blend;
                let gain = self.amplitude * comfort.amount() * fade;
                add(
                    scale(right, cosf(self.phase) * bob.sway * gain),
                    scale(up, sinf(self.phase * 2.0) * bob.vertical * gain),
                )
            }
            _ => {
                self.amplitude = 0.0;
                [0.0; 3]
            }
        };
        let mut at = add(add(pivot, scale(forward, -self.arm)), bob);
        let mut look = add(at, forward);
        if !self.started {
            self.started = true;
            self.carry = None;
            self.changed = false;
            let view = View {
                at,
                look_at: look,
                up: UP,
            };
            self.previous = view;
            self.current = view;
            return;
        }
        if self.changed {
            self.changed = false;
            if tuning.handover > 0.0 {
                self.carry = Some(Carry {
                    offset: sub(self.current.at, at),
                    elapsed: 0.0,
                });
            } else {
                self.carry = None;
                self.current.at = at;
            }
        }
        if let Some(carry) = &mut self.carry {
            carry.elapsed += dt;
            let t = (carry.elapsed / tuning.handover.max(1e-6)).min(1.0);
            let shift = scale(carry.offset, 1.0 - smoothstep(t));
            at = add(at, shift);
            look = add(look, shift);
            if t >= 1.0 {
                self.carry = None;
            }
        }
        self.previous = self.current;
        self.current = View {
            at,
            look_at: look,
            up: UP,
        };
    }

    pub fn pose(&self, alpha: f32) -> View {
        self.previous.lerp(self.current, alpha.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rig {
    Follow(Follow),
    Person(Person),
}

impl Rig {
    pub fn tick(&mut self, dt: f32, input: &Input, comfort: Comfort, probe: &dyn Probe) {
        match self {
            Rig::Follow(follow) => follow.tick(dt, input, comfort),
            Rig::Person(person) => person.tick(dt, input, comfort, probe),
        }
    }

    pub fn pose(&self, alpha: f32) -> View {
        match self {
            Rig::Follow(follow) => follow.pose(alpha),
            Rig::Person(person) => person.pose(alpha),
        }
    }

    pub fn hand_over(&mut self) {
        match self {
            Rig::Follow(follow) => follow.hand_over(),
            Rig::Person(person) => person.hand_over(),
        }
    }

    pub fn restart(&mut self) {
        match self {
            Rig::Follow(follow) => follow.restart(),
            Rig::Person(person) => person.restart(),
        }
    }

    pub fn started(&self) -> bool {
        match self {
            Rig::Follow(follow) => follow.started(),
            Rig::Person(person) => person.started(),
        }
    }

    pub fn follow(&self) -> Option<&Follow> {
        match self {
            Rig::Follow(follow) => Some(follow),
            Rig::Person(_) => None,
        }
    }

    pub fn follow_mut(&mut self) -> Option<&mut Follow> {
        match self {
            Rig::Follow(follow) => Some(follow),
            Rig::Person(_) => None,
        }
    }

    pub fn person(&self) -> Option<&Person> {
        match self {
            Rig::Person(person) => Some(person),
            Rig::Follow(_) => None,
        }
    }

    pub fn person_mut(&mut self) -> Option<&mut Person> {
        match self {
            Rig::Person(person) => Some(person),
            Rig::Follow(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    from: View,
    elapsed: f32,
    duration: f32,
}

impl Blend {
    pub fn new(from: View, duration: f32) -> Self {
        Self {
            from,
            elapsed: 0.0,
            duration,
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.elapsed += dt;
    }

    pub fn done(&self) -> bool {
        self.duration <= 0.0 || self.elapsed >= self.duration
    }

    pub fn apply(&self, to: View) -> View {
        if self.done() {
            return to;
        }
        self.from.lerp(to, smoothstep(self.elapsed / self.duration))
    }
}

#[cfg(test)]
mod tests;
