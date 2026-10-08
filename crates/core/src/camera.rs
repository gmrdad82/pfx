use crate::motion::{Cascade, Rest, STEP, Stepper};

pub const DEG: f32 = std::f32::consts::PI / 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Full,
    Subtle,
    Off,
}

impl Level {
    pub fn amount(self) -> f32 {
        match self {
            Level::Full => 1.0,
            Level::Subtle => 0.5,
            Level::Off => 0.0,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Level::Off),
            "subtle" => Some(Level::Subtle),
            "full" => Some(Level::Full),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harmonic {
    pub weight: f32,
    pub rate: f32,
    pub phase: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drift {
    pub yaw: [Harmonic; 3],
    pub yaw_gain: f32,
    pub pitch: [Harmonic; 3],
    pub pitch_gain: f32,
    pub roll: Harmonic,
    pub roll_gain: f32,
    pub offset: Harmonic,
    pub offset_gain: f32,
    pub calmed: bool,
    pub quiet_roll: bool,
}

impl Default for Drift {
    fn default() -> Self {
        Self {
            yaw: [
                Harmonic {
                    weight: 0.55,
                    rate: 0.043,
                    phase: 0.0,
                },
                Harmonic {
                    weight: 0.3,
                    rate: 0.071,
                    phase: 1.3,
                },
                Harmonic {
                    weight: 0.15,
                    rate: 0.113,
                    phase: 2.1,
                },
            ],
            yaw_gain: 0.25 * DEG,
            pitch: [
                Harmonic {
                    weight: 0.5,
                    rate: 0.037,
                    phase: 0.7,
                },
                Harmonic {
                    weight: 0.35,
                    rate: 0.083,
                    phase: 2.4,
                },
                Harmonic {
                    weight: 0.15,
                    rate: 0.127,
                    phase: 0.0,
                },
            ],
            pitch_gain: 0.16 * DEG,
            roll: Harmonic {
                weight: 1.0,
                rate: 0.029,
                phase: 0.4,
            },
            roll_gain: 0.04 * DEG,
            offset: Harmonic {
                weight: 1.0,
                rate: 0.061,
                phase: 0.0,
            },
            offset_gain: 0.002,
            calmed: true,
            quiet_roll: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub stiffness: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub hold: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub stiffness: f32,
    pub lean: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub sign: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub stiffness: f32,
    pub hold: f32,
    pub max: f32,
    pub rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wander {
    pub idle: f32,
    pub every: f32,
    pub hold: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hush {
    pub floor: f32,
    pub stiffness: f32,
    pub blocks_cursor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Depth {
    pub focus: f32,
    pub blur: f32,
    pub floor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nudge {
    pub stiffness: f32,
    pub pitch: f32,
    pub hold: f32,
    pub floor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Redraw {
    Always,
    Exact,
    Threshold {
        angle: f32,
        offset: f32,
        zoom: f32,
        focus: f32,
        blur: f32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stepping {
    #[default]
    Shared,
    PerFrame,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub gain: f32,
    pub orbit: Orbit,
    pub look: Look,
    pub zoom: Zoom,
    pub wander: Wander,
    pub hush: Option<Hush>,
    pub depth: Option<Depth>,
    pub nudge: Nudge,
    pub drift: Drift,
    pub redraw: Redraw,
    pub rest: Rest,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            gain: 1.0,
            orbit: Orbit {
                stiffness: 4.0,
                yaw: 0.5 * DEG,
                pitch: 0.25 * DEG,
                hold: 2.0,
            },
            look: Look {
                stiffness: 3.0,
                lean: 0.2,
                yaw: 0.6 * DEG,
                pitch: 0.4 * DEG,
                sign: 1.0,
            },
            zoom: Zoom {
                stiffness: 3.0,
                hold: 2.0,
                max: 2.0,
                rate: 1.1,
            },
            wander: Wander {
                idle: 8.0,
                every: 12.0,
                hold: 3.0,
            },
            hush: None,
            depth: None,
            nudge: Nudge {
                stiffness: 30.0,
                pitch: 0.1 * DEG,
                hold: 0.2,
                floor: 0.0,
            },
            drift: Drift::default(),
            redraw: Redraw::Exact,
            rest: Rest::COARSE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub focus: f32,
    pub blur: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub offset: f32,
    pub zoom: f32,
    pub focus: f32,
    pub blur: f32,
    pub look: [f32; 2],
    pub moved: bool,
}

impl Pose {
    pub fn rest(focus: f32, blur: f32) -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            offset: 0.0,
            zoom: 1.0,
            focus,
            blur,
            look: [0.0, 0.0],
            moved: false,
        }
    }

    pub fn from_spec(spec: &str) -> Self {
        let mut pose = Self::rest(0.0, 0.0);
        for pair in spec.split(',') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let Ok(parsed) = value.trim().parse::<f32>() else {
                continue;
            };
            match key.trim() {
                "yaw" => pose.yaw = parsed * DEG,
                "pitch" => pose.pitch = parsed * DEG,
                "roll" => pose.roll = parsed * DEG,
                "dolly" => pose.offset = parsed,
                "zoom" => pose.zoom = parsed.max(1.0),
                "focus" => pose.focus = parsed,
                "blur" => pose.blur = parsed,
                "lookx" => pose.look[0] = parsed * DEG,
                "looky" => pose.look[1] = parsed * DEG,
                _ => {}
            }
        }
        pose
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Director {
    level: Level,
    preset: Preset,
    stepping: Stepping,
    frame_time: f32,
    clock: Stepper,
    ticks: u64,
    orbit_yaw: Cascade<f32>,
    orbit_pitch: Cascade<f32>,
    look_yaw: Cascade<f32>,
    look_pitch: Cascade<f32>,
    zoom: Cascade<f32>,
    focus: Cascade<f32>,
    blur: Cascade<f32>,
    nudge: Cascade<f32>,
    calm: Cascade<f32>,
    nudge_until: f64,
    mouse: [f32; 2],
    mouse_at: f64,
    busy: bool,
    hushed: bool,
    base: Option<Aim>,
    glance: Option<(Aim, f64)>,
    rest_focus: f32,
    rest_blur: f32,
    user_zoom: f32,
    zoom_at: f64,
    input_at: f64,
    wander: Option<Aim>,
    wandered_at: f64,
}

fn eased(base: f32, gain: f32, value: f32, rest: Rest) -> Cascade<f32> {
    Cascade::new(base * gain, value).with_rest(rest)
}

fn wave(term: Harmonic, t: f32) -> f32 {
    term.weight * (std::f32::consts::TAU * term.rate * t + term.phase).sin()
}

impl Director {
    pub fn new(level: Level, preset: Preset, rest_focus: f32, rest_blur: f32) -> Self {
        let gain = preset.gain;
        let focus_k = preset.depth.map(|depth| depth.focus).unwrap_or(1.0);
        let blur_k = preset.depth.map(|depth| depth.blur).unwrap_or(1.0);
        let calm_k = preset.hush.map(|hush| hush.stiffness).unwrap_or(1.0);
        let focus0 = if preset.depth.is_some() {
            rest_focus
        } else {
            0.0
        };
        let blur0 = if preset.depth.is_some() {
            rest_blur
        } else {
            0.0
        };
        Self {
            level,
            preset,
            stepping: Stepping::Shared,
            frame_time: 0.0,
            clock: Stepper::new(),
            ticks: 0,
            orbit_yaw: eased(preset.orbit.stiffness, gain, 0.0, preset.rest),
            orbit_pitch: eased(preset.orbit.stiffness, gain, 0.0, preset.rest),
            look_yaw: eased(preset.look.stiffness, gain, 0.0, preset.rest),
            look_pitch: eased(preset.look.stiffness, gain, 0.0, preset.rest),
            zoom: eased(preset.zoom.stiffness, gain, 1.0, preset.rest),
            focus: eased(focus_k, gain, focus0, preset.rest),
            blur: eased(blur_k, gain, blur0, preset.rest),
            nudge: eased(preset.nudge.stiffness, gain, 0.0, preset.rest),
            calm: eased(calm_k, gain, 1.0, preset.rest),
            nudge_until: f64::NEG_INFINITY,
            mouse: [0.0; 2],
            mouse_at: f64::NEG_INFINITY,
            busy: false,
            hushed: false,
            base: None,
            glance: None,
            rest_focus,
            rest_blur,
            user_zoom: 1.0,
            zoom_at: f64::NEG_INFINITY,
            input_at: 0.0,
            wander: None,
            wandered_at: 0.0,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn stepping(&self) -> Stepping {
        self.stepping
    }

    pub fn set_stepping(&mut self, stepping: Stepping) {
        if self.stepping == stepping {
            return;
        }
        match stepping {
            Stepping::Shared => {
                self.ticks += u64::from(self.clock.advance_to(self.frame_time));
            }
            Stepping::PerFrame => {
                self.frame_time = self.clock.time() as f32;
            }
        }
        self.stepping = stepping;
    }

    pub fn with_stepping(mut self, stepping: Stepping) -> Self {
        self.set_stepping(stepping);
        self
    }

    pub fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    pub fn set_rest_focus(&mut self, focus: f32) {
        self.rest_focus = focus;
    }

    pub fn set_rest_blur(&mut self, blur: f32) {
        self.rest_blur = blur;
    }

    pub fn set_wander(&mut self, target: Option<Aim>) {
        self.wander = target;
    }

    pub fn hush(&mut self, reading: bool) {
        self.hushed = reading;
    }

    pub fn cursor(&mut self, x: f32, y: f32, busy: bool) {
        self.mouse = [x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0)];
        self.mouse_at = self.now();
        self.busy = busy;
        self.touch();
    }

    pub fn attend(&mut self, target: Aim) {
        self.base = Some(target);
        self.touch();
    }

    pub fn release(&mut self) {
        self.base = None;
        self.touch();
    }

    pub fn glance(&mut self, target: Aim, hold: f32) {
        self.glance = Some((target, self.deadline(hold)));
        self.touch();
    }

    pub fn nudge(&mut self) {
        let amount = self.level.amount().max(self.preset.nudge.floor);
        self.nudge.lead.target = -self.preset.nudge.pitch * amount;
        self.nudge_until = self.deadline(self.preset.nudge.hold);
        self.touch();
    }

    pub fn zoom_by(&mut self, steps: f32) {
        let next = self.user_zoom * self.preset.zoom.rate.powf(steps);
        self.user_zoom = next.clamp(1.0, self.preset.zoom.max);
        self.zoom_at = self.now();
        self.touch();
    }

    pub fn step(&mut self, dt: f32) -> Pose {
        match self.stepping {
            Stepping::Shared => {
                let ticks = self.clock.advance(dt);
                self.advance(ticks)
            }
            Stepping::PerFrame => {
                self.frame_time += dt;
                self.integrate_per_frame(dt);
                self.pose()
            }
        }
    }

    pub fn advance_to(&mut self, to: f32) -> Pose {
        match self.stepping {
            Stepping::Shared => {
                let ticks = self.clock.advance_to(to);
                self.advance(ticks)
            }
            Stepping::PerFrame => {
                if to.is_finite() && to >= self.frame_time {
                    self.step(to - self.frame_time)
                } else {
                    self.pose()
                }
            }
        }
    }

    pub fn settled(&self) -> bool {
        self.level == Level::Off
            && self.base.is_none()
            && self.glance.is_none()
            && self.user_zoom == 1.0
            && self.orbit_yaw.settled()
            && self.orbit_pitch.settled()
            && self.look_yaw.settled()
            && self.look_pitch.settled()
            && self.zoom.settled()
            && self.focus.settled()
            && self.blur.settled()
            && self.nudge.settled()
            && self.calm.settled()
    }

    fn touch(&mut self) {
        self.input_at = self.now();
    }

    fn now(&self) -> f64 {
        match self.stepping {
            Stepping::Shared => self.ticks as f64 * f64::from(STEP),
            Stepping::PerFrame => f64::from(self.frame_time),
        }
    }

    fn deadline(&self, hold: f32) -> f64 {
        match self.stepping {
            Stepping::Shared => self.now() + f64::from(hold),
            Stepping::PerFrame => f64::from(self.frame_time + hold),
        }
    }

    fn advance(&mut self, ticks: u32) -> Pose {
        for _ in 0..ticks {
            self.ticks += 1;
            self.integrate(self.now());
        }
        self.pose()
    }

    fn integrate(&mut self, time: f64) {
        let preset = self.preset;
        let amount = self.level.amount();
        let hushed = self.hushed;
        self.calm.lead.target = match preset.hush {
            Some(hush) if hushed => hush.floor,
            _ => 1.0,
        };
        let blocked = preset.hush.is_some_and(|hush| hush.blocks_cursor) && hushed;
        let mousing = time - self.mouse_at < f64::from(preset.orbit.hold) && !self.busy && !blocked;
        let (mx, my) = if mousing {
            (self.mouse[0], self.mouse[1])
        } else {
            (0.0, 0.0)
        };
        self.orbit_yaw.lead.target = -mx * preset.orbit.yaw * amount;
        self.orbit_pitch.lead.target = my * preset.orbit.pitch * amount;
        let aim = self.intent(time);
        if time - self.zoom_at > f64::from(preset.zoom.hold) {
            self.user_zoom = 1.0;
        }
        let (look_yaw, look_pitch, zoom, focus, blur) = self.aimed(aim, amount);
        self.look_yaw.lead.target = look_yaw;
        self.look_pitch.lead.target = look_pitch;
        self.zoom.lead.target = zoom;
        self.focus.lead.target = focus;
        self.blur.lead.target = blur;
        if time > self.nudge_until {
            self.nudge.lead.target = 0.0;
        }
        self.orbit_yaw.step(STEP);
        self.orbit_pitch.step(STEP);
        self.look_yaw.step(STEP);
        self.look_pitch.step(STEP);
        self.zoom.step(STEP);
        self.focus.step(STEP);
        self.blur.step(STEP);
        self.nudge.step(STEP);
        self.calm.step(STEP);
    }

    fn integrate_per_frame(&mut self, dt: f32) {
        let time = self.frame_time;
        let preset = self.preset;
        let amount = self.level.amount();
        self.calm.lead.target = if self.hushed {
            preset.hush.map(|hush| hush.floor).unwrap_or(1.0)
        } else {
            1.0
        };
        let blocked = preset.hush.is_some_and(|hush| hush.blocks_cursor) && self.hushed;
        let mousing = time - (self.mouse_at as f32) < preset.orbit.hold && !self.busy && !blocked;
        let (mx, my) = if mousing {
            (self.mouse[0], self.mouse[1])
        } else {
            (0.0, 0.0)
        };
        self.orbit_yaw.lead.target = -mx * (preset.orbit.yaw / DEG) * DEG * amount;
        self.orbit_pitch.lead.target = my * (preset.orbit.pitch / DEG) * DEG * amount;
        let aim = self.intent_per_frame();
        if time - (self.zoom_at as f32) > preset.zoom.hold {
            self.user_zoom = 1.0;
        }
        let (look_yaw, look_pitch, zoom, focus, blur) = self.aimed(aim, amount);
        self.look_yaw.lead.target = look_yaw;
        self.look_pitch.lead.target = look_pitch;
        self.zoom.lead.target = zoom;
        self.focus.lead.target = focus;
        self.blur.lead.target = blur;
        if time > self.nudge_until as f32 {
            self.nudge.lead.target = 0.0;
        }
        self.orbit_yaw.step(dt);
        self.orbit_pitch.step(dt);
        self.look_yaw.step(dt);
        self.look_pitch.step(dt);
        self.zoom.step(dt);
        self.focus.step(dt);
        self.blur.step(dt);
        self.nudge.step(dt);
        self.calm.step(dt);
    }

    fn aimed(&self, aim: Option<Aim>, amount: f32) -> (f32, f32, f32, f32, f32) {
        let preset = self.preset;
        let zoom = match aim {
            Some(aim) => self.user_zoom.max(1.0 + (aim.zoom - 1.0) * amount),
            None => self.user_zoom,
        };
        let (look_yaw, look_pitch) = match aim {
            Some(aim) => {
                let yaw = (preset.look.sign * aim.yaw * preset.look.lean)
                    .clamp(-preset.look.yaw, preset.look.yaw)
                    * amount;
                let pitch = (aim.pitch * preset.look.lean)
                    .clamp(-preset.look.pitch, preset.look.pitch)
                    * amount;
                (yaw, pitch)
            }
            None => (0.0, 0.0),
        };
        let (focus, blur) = if let Some(depth) = preset.depth {
            let scale = amount.max(depth.floor);
            match aim {
                Some(aim) => (aim.focus, aim.blur * scale),
                None => (self.rest_focus, self.rest_blur * scale),
            }
        } else {
            (0.0, 0.0)
        };
        (look_yaw, look_pitch, zoom, focus, blur)
    }

    fn intent(&mut self, time: f64) -> Option<Aim> {
        if let Some((target, until)) = self.glance {
            if time < until {
                return Some(target);
            }
            self.glance = None;
        }
        if let Some(target) = self.base {
            return Some(target);
        }
        let wander = self.preset.wander;
        let idle = f64::from(wander.idle);
        if self.level == Level::Full && time - self.input_at > idle {
            if time - self.wandered_at > f64::from(wander.every) {
                self.wandered_at = time;
            }
            if time - self.wandered_at < f64::from(wander.hold) {
                return self.wander;
            }
        } else if time - self.input_at <= idle {
            self.wandered_at = time;
        }
        None
    }

    fn intent_per_frame(&mut self) -> Option<Aim> {
        let time = self.frame_time;
        if let Some((target, until)) = self.glance {
            if time < until as f32 {
                return Some(target);
            }
            self.glance = None;
        }
        if self.base.is_some() {
            return self.base;
        }
        let wander = self.preset.wander;
        if self.level == Level::Full && time - (self.input_at as f32) > wander.idle {
            if time - (self.wandered_at as f32) > wander.every {
                self.wandered_at = f64::from(time);
            }
            if time - (self.wandered_at as f32) < wander.hold {
                return self.wander;
            }
        } else if time - (self.input_at as f32) <= wander.idle {
            self.wandered_at = f64::from(time);
        }
        None
    }

    fn pose(&self) -> Pose {
        let calm = self.calm.value();
        let scale = self.level.amount() * if self.preset.drift.calmed { calm } else { 1.0 };
        let hush = self.preset.hush.map(|hush| hush.floor).unwrap_or(0.0);
        let quiet = if self.preset.drift.quiet_roll {
            let span = 1.0 - hush;
            if span > 0.0 {
                ((calm - hush) / span).clamp(0.0, 1.0)
            } else {
                0.0
            }
        } else {
            1.0
        };
        let t = self.now() as f32;
        let drift = &self.preset.drift;
        let yaw_sum = wave(drift.yaw[0], t) + wave(drift.yaw[1], t) + wave(drift.yaw[2], t);
        let pitch_sum = wave(drift.pitch[0], t) + wave(drift.pitch[1], t) + wave(drift.pitch[2], t);
        let (yaw, pitch, roll) = if self.stepping == Stepping::PerFrame {
            (
                yaw_sum * (drift.yaw_gain / DEG) * DEG * scale,
                pitch_sum * (drift.pitch_gain / DEG) * DEG * scale,
                wave(drift.roll, t) * (drift.roll_gain / DEG) * DEG * scale * quiet,
            )
        } else {
            (
                yaw_sum * drift.yaw_gain * scale,
                pitch_sum * drift.pitch_gain * scale,
                wave(drift.roll, t) * drift.roll_gain * scale * quiet,
            )
        };
        let offset = wave(drift.offset, t) * drift.offset_gain * scale;
        let pose = Pose {
            yaw: self.orbit_yaw.value() + yaw,
            pitch: self.orbit_pitch.value() + pitch + self.nudge.value(),
            roll,
            offset,
            zoom: self.zoom.value(),
            focus: self.focus.value(),
            blur: self.blur.value(),
            look: [self.look_yaw.value(), self.look_pitch.value()],
            moved: false,
        };
        Pose {
            moved: self.moved(&pose),
            ..pose
        }
    }

    fn moved(&self, pose: &Pose) -> bool {
        match self.preset.redraw {
            Redraw::Always => true,
            Redraw::Exact => {
                pose.yaw != 0.0
                    || pose.pitch != 0.0
                    || pose.roll != 0.0
                    || pose.offset != 0.0
                    || (pose.zoom - 1.0).abs() >= 1e-6
                    || pose.look != [0.0, 0.0]
            }
            Redraw::Threshold {
                angle,
                offset,
                zoom,
                focus,
                blur,
            } => {
                let angles = pose.yaw.abs()
                    + pose.pitch.abs()
                    + pose.roll.abs()
                    + pose.look[0].abs()
                    + pose.look[1].abs();
                angles > angle
                    || pose.offset.abs() > offset
                    || (pose.zoom - 1.0).abs() > zoom
                    || (pose.focus - self.rest_focus).abs() > focus
                    || pose.blur.abs() > blur
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FrameSpring {
        lead: (f32, f32),
        main: (f32, f32),
        stiffness: f32,
    }

    impl FrameSpring {
        fn new(stiffness: f32, gain: f32, value: f32) -> Self {
            Self {
                lead: (value, 0.0),
                main: (value, 0.0),
                stiffness: stiffness * gain,
            }
        }

        fn step(&mut self, target: f32, dt: f32) -> f32 {
            let spring = |state: &mut (f32, f32), target: f32| {
                let steps = (dt / (1.0 / 240.0)).ceil().max(1.0) as u32;
                let h = dt / steps as f32;
                let damping = 2.0 * self.stiffness.sqrt();
                for _ in 0..steps {
                    let force = -self.stiffness * (state.0 - target) - damping * state.1;
                    state.1 += force * h;
                    state.0 += state.1 * h;
                }
                if (state.0 - target).abs() < 1e-4 && state.1.abs() < 1e-3 {
                    state.0 = target;
                    state.1 = 0.0;
                }
            };
            spring(&mut self.lead, target);
            spring(&mut self.main, self.lead.0);
            self.main.0
        }
    }

    fn aim(yaw: f32, pitch: f32, zoom: f32, focus: f32, blur: f32) -> Aim {
        Aim {
            yaw,
            pitch,
            zoom,
            focus,
            blur,
        }
    }

    fn depth() -> Preset {
        Preset {
            depth: Some(Depth {
                focus: 5.0,
                blur: 2.0,
                floor: 0.0,
            }),
            hush: Some(Hush {
                floor: 0.25,
                stiffness: 2.0,
                blocks_cursor: true,
            }),
            redraw: Redraw::Always,
            ..Preset::default()
        }
    }

    fn open() -> Director {
        Director::new(Level::Full, depth(), 1.2, 4.0)
    }

    fn hold(director: &mut Director, seconds: f32) -> Pose {
        let mut pose = Pose::rest(0.0, 0.0);
        let frames = (seconds * 60.0).round() as usize;
        for _ in 0..frames {
            pose = director.step(1.0 / 60.0);
        }
        pose
    }

    fn until_settled(director: &mut Director) -> Pose {
        let mut pose = director.step(1.0 / 60.0);
        for _ in 0..60 * 30 {
            if director.settled() {
                return pose;
            }
            pose = director.step(1.0 / 60.0);
        }
        panic!("camera did not settle, zoom {}", pose.zoom);
    }

    fn still(pose: &Pose) {
        assert_eq!(pose.yaw, 0.0);
        assert_eq!(pose.pitch, 0.0);
        assert_eq!(pose.roll, 0.0);
        assert_eq!(pose.offset, 0.0);
        assert_eq!(pose.look, [0.0, 0.0]);
    }

    fn per_frame() -> Preset {
        let mut preset = depth();
        preset.gain = 1.5;
        preset.orbit.stiffness = 4.0;
        preset.orbit.yaw = 0.4 * DEG;
        preset.orbit.pitch = 0.2 * DEG;
        preset.zoom.stiffness = 2.0;
        preset.zoom.rate = 1.2;
        preset.zoom.max = 1.8;
        preset.zoom.hold = 10.0;
        preset.depth = Some(Depth {
            focus: 5.0,
            blur: 2.0,
            floor: 1.0,
        });
        preset.nudge = Nudge {
            stiffness: 20.0,
            pitch: 0.05 * DEG,
            hold: 0.15,
            floor: 0.4,
        };
        preset
    }

    #[test]
    fn per_frame_steps_once_from_the_preset() {
        let preset = per_frame();
        let mut director =
            Director::new(Level::Off, preset, 1.2, 4.0).with_stepping(Stepping::PerFrame);
        assert_eq!(director.stepping(), Stepping::PerFrame);
        director.attend(aim(0.2, -0.1, 1.1, 2.4, 8.0));
        director.zoom_by(3.0);
        director.nudge();
        let mut zoom = FrameSpring::new(preset.zoom.stiffness, preset.gain, 1.0);
        let mut focus = FrameSpring::new(preset.depth.unwrap().focus, preset.gain, 1.2);
        let mut blur = FrameSpring::new(preset.depth.unwrap().blur, preset.gain, 4.0);
        let mut nudge = FrameSpring::new(preset.nudge.stiffness, preset.gain, 0.0);
        let mut time = 0.0f32;
        let zoom_target = preset.zoom.rate.powf(3.0).min(preset.zoom.max);
        for dt in [0.007, 0.013, 1.0 / 60.0, 0.031, 0.004, 0.052, 0.2] {
            time += dt;
            let expected_zoom = zoom.step(zoom_target, dt);
            let expected_focus = focus.step(2.4, dt);
            let expected_blur = blur.step(8.0, dt);
            let nudge_target = if time > preset.nudge.hold {
                0.0
            } else {
                -preset.nudge.pitch * preset.nudge.floor
            };
            let expected_nudge = nudge.step(nudge_target, dt);
            let pose = director.step(dt);
            assert_eq!(pose.zoom.to_bits(), expected_zoom.to_bits());
            assert_eq!(pose.focus.to_bits(), expected_focus.to_bits());
            assert_eq!(pose.blur.to_bits(), expected_blur.to_bits());
            assert_eq!(pose.pitch.to_bits(), expected_nudge.to_bits());
            assert!(pose.moved);
        }
        let mut shared = Director::new(Level::Off, preset, 1.2, 4.0);
        shared.zoom_by(3.0);
        assert_ne!(shared.step(0.007).zoom.to_bits(), zoom.main.0.to_bits());
    }

    #[test]
    fn per_frame_orbit_and_drift_read_the_preset() {
        let preset = per_frame();
        let mut director =
            Director::new(Level::Full, preset, 1.2, 4.0).with_stepping(Stepping::PerFrame);
        director.cursor(0.73, -0.41, false);
        let mut orbit_yaw = FrameSpring::new(preset.orbit.stiffness, preset.gain, 0.0);
        let mut orbit_pitch = FrameSpring::new(preset.orbit.stiffness, preset.gain, 0.0);
        let mut time = 0.0f32;
        for dt in [0.007, 0.019, 1.0 / 60.0, 0.031, 0.11, 0.02] {
            time += dt;
            let yaw = orbit_yaw.step(-0.73 * (preset.orbit.yaw / DEG) * DEG, dt);
            let pitch = orbit_pitch.step(-0.41 * (preset.orbit.pitch / DEG) * DEG, dt);
            let drift = &preset.drift;
            let idle_yaw =
                (wave(drift.yaw[0], time) + wave(drift.yaw[1], time) + wave(drift.yaw[2], time))
                    * (drift.yaw_gain / DEG)
                    * DEG;
            let idle_pitch = (wave(drift.pitch[0], time)
                + wave(drift.pitch[1], time)
                + wave(drift.pitch[2], time))
                * (drift.pitch_gain / DEG)
                * DEG;
            let pose = director.step(dt);
            assert_eq!(pose.yaw.to_bits(), (yaw + idle_yaw).to_bits());
            assert_eq!(pose.pitch.to_bits(), (pitch + idle_pitch).to_bits());
        }
    }

    #[test]
    fn a_default_preset_is_neutral() {
        let preset = Preset::default();
        assert_eq!(preset.gain, 1.0);
        assert_eq!(preset.look.sign, 1.0);
        assert!(preset.hush.is_none());
        assert!(preset.depth.is_none());
        assert!(matches!(preset.redraw, Redraw::Exact));
        assert_eq!(preset.rest, Rest::COARSE);
        assert!(preset.drift.calmed && preset.drift.quiet_roll);
        let mut fine = preset;
        fine.rest = Rest::FINE;
        let director = Director::new(Level::Full, fine, 0.0, 0.0);
        assert_eq!(director.zoom.follow.rest, Rest::FINE);
        assert_eq!(director.orbit_yaw.lead.rest, Rest::FINE);
    }

    #[test]
    fn a_director_settles_to_rest() {
        let mut director = Director::new(Level::Off, depth(), 1.2, 4.0);
        let pose = until_settled(&mut director);
        still(&pose);
        assert_eq!(pose.zoom, 1.0);
        assert_eq!(pose.focus, 1.2);
        assert_eq!(pose.blur, 0.0);
        assert!(director.settled());
        director.zoom_by(4.0);
        assert!(!director.settled());
        let back = until_settled(&mut director);
        still(&back);
        assert!((back.zoom - 1.0).abs() < 1e-4);

        let mut plain = Director::new(Level::Off, Preset::default(), 0.8, 3.0);
        let pose = until_settled(&mut plain);
        still(&pose);
        assert_eq!(pose.focus, 0.0);
        assert_eq!(pose.blur, 0.0);
        assert!(!pose.moved);
    }

    #[test]
    fn a_lean_follows_the_cursor_and_returns() {
        let mut director = open();
        for _ in 0..180 {
            director.cursor(1.0, -1.0, false);
            let _ = director.step(1.0 / 60.0);
        }
        let leaning = director.step(1.0 / 60.0);
        assert!(leaning.yaw < -0.2 * DEG, "yaw {}", leaning.yaw);
        assert!(leaning.pitch < -0.1 * DEG, "pitch {}", leaning.pitch);
        let back = hold(&mut director, 6.0);
        assert!(back.yaw.abs() < leaning.yaw.abs());
        assert!(back.pitch.abs() < leaning.pitch.abs());
    }

    #[test]
    fn zoom_stays_within_the_preset_cap() {
        let mut preset = depth();
        preset.zoom.stiffness = 12.0;
        preset.zoom.hold = 8.0;
        let mut director = Director::new(Level::Full, preset, 1.2, 4.0);
        let max = preset.zoom.max;
        for _ in 0..40 {
            director.zoom_by(1.0);
        }
        director.zoom_by(-80.0);
        director.zoom_by(8.0);
        let mut peak = 1.0f32;
        for _ in 0..240 {
            let zoom = director.step(1.0 / 60.0).zoom;
            assert!(zoom <= max + 1e-4, "zoom {zoom}");
            assert!(zoom >= 1.0 - 1e-4, "zoom {zoom}");
            peak = peak.max(zoom);
        }
        assert!((peak - max).abs() < 1e-2, "peak {peak}");
        let back = hold(&mut director, 12.0);
        assert!(back.zoom < 1.01);
    }

    #[test]
    fn frame_patterns_meet_at_the_same_time() {
        let make = open();
        let script = |director: &mut Director| {
            director.cursor(0.6, -0.25, false);
            director.zoom_by(2.5);
            director.attend(aim(0.15, -0.05, 1.04, 1.7, 6.0));
            director.glance(aim(-0.35, 0.12, 1.02, 1.2, 3.0), 0.4);
            director.nudge();
        };
        let mut timed = make;
        script(&mut timed);
        let pose = timed.advance_to(1.0);
        for frames in [
            vec![1.0 / 60.0; 60],
            vec![1.0 / 240.0; 240],
            vec![1.0 / 120.0; 120],
            vec![1.0 / 30.0; 30],
        ] {
            let mut director = make;
            script(&mut director);
            let mut got = Pose::rest(0.0, 0.0);
            for dt in frames {
                got = director.step(dt);
            }
            assert_eq!(got, pose);
        }
    }

    #[test]
    fn level_off_holds_the_camera_still() {
        let mut always = Director::new(Level::Off, depth(), 1.2, 4.0);
        let mut exact = Director::new(Level::Off, Preset::default(), 0.0, 0.0);
        for _ in 0..180 {
            let posed = always.step(1.0 / 60.0);
            let quiet = exact.step(1.0 / 60.0);
            still(&posed);
            still(&quiet);
            assert_eq!(posed.zoom, 1.0);
            assert!(posed.moved);
            assert!(!quiet.moved);
        }
        always.cursor(1.0, 1.0, false);
        always.attend(aim(0.4, -0.3, 1.2, 2.0, 8.0));
        exact.cursor(-1.0, 1.0, true);
        exact.attend(aim(0.2, 0.2, 1.05, 4.0, 8.0));
        for _ in 0..120 {
            still(&always.step(1.0 / 60.0));
            let pose = exact.step(1.0 / 60.0);
            still(&pose);
            assert_eq!(pose.focus, 0.0);
            assert_eq!(pose.blur, 0.0);
        }
    }

    #[test]
    fn off_ignores_motion_events_but_keeps_explicit_zoom() {
        for stepping in [Stepping::Shared, Stepping::PerFrame] {
            let mut director =
                Director::new(Level::Off, Preset::default(), 0.8, 0.0).with_stepping(stepping);
            director.cursor(1.0, -1.0, false);
            director.attend(aim(0.4, -0.3, 1.2, 2.0, 6.0));
            director.glance(aim(-0.5, 0.2, 1.1, 1.0, 4.0), 0.5);
            director.set_wander(Some(aim(0.3, 0.4, 1.1, 1.0, 4.0)));
            director.nudge();
            for _ in 0..120 {
                still(&director.step(1.0 / 60.0));
            }
            director.zoom_by(3.0);
            let mut peak = 1.0f32;
            for _ in 0..720 {
                let pose = director.step(1.0 / 60.0);
                still(&pose);
                peak = peak.max(pose.zoom);
            }
            assert!(peak > 1.1);
        }
    }

    #[test]
    fn a_glance_returns_to_the_base() {
        let mut director = open();
        let player = aim(0.05, 0.02, 1.04, 1.4, 6.0);
        let cards = aim(-0.2, 0.0, 1.02, 1.2, 5.0);
        director.attend(player);
        let on_player = hold(&mut director, 4.0);
        director.glance(cards, 1.0);
        let on_cards = hold(&mut director, 0.9);
        let back = hold(&mut director, 5.0);
        assert!(on_player.zoom > 1.03);
        assert!(on_cards.look[0] < back.look[0]);
        assert!((back.look[0] - on_player.look[0]).abs() < 1e-3);
        assert!((back.focus - player.focus).abs() < 1e-2);
        director.release();
        let home = hold(&mut director, 4.0);
        assert!(home.look[0].abs() < 1e-3);
    }

    #[test]
    fn look_sign_follows_or_opposes_the_aim() {
        let far = aim(-0.3, 0.2, 1.03, 0.7, 4.0);
        let mut positive = depth();
        positive.look.sign = 1.0;
        let mut director = Director::new(Level::Full, positive, 1.2, 4.0);
        director.attend(far);
        let posed = hold(&mut director, 5.0);
        assert!((posed.look[0] + positive.look.yaw).abs() < 1e-3);
        assert!((posed.look[1] - positive.look.pitch).abs() < 1e-3);
        let mut negative = positive;
        negative.look.sign = -1.0;
        let mut director = Director::new(Level::Full, negative, 1.2, 4.0);
        director.attend(far);
        let posed = hold(&mut director, 5.0);
        assert!((posed.look[0] - negative.look.yaw).abs() < 1e-3);
        assert!((posed.look[1] - negative.look.pitch).abs() < 1e-3);
    }

    #[test]
    fn reading_quiets_the_drift_and_stops_the_roll() {
        let mut preset = depth();
        preset.hush = Some(Hush {
            floor: 0.25,
            stiffness: 12.0,
            blocks_cursor: true,
        });
        let mut director = Director::new(Level::Full, preset, 1.2, 4.0);
        director.hush(true);
        let _ = hold(&mut director, 5.0);
        for _ in 0..60 {
            let pose = director.step(1.0 / 60.0);
            assert!(pose.yaw.abs() < 0.1 * DEG);
            assert!(pose.roll.abs() < 1e-5);
        }
        let mut loose = Preset::default();
        loose.drift.calmed = false;
        loose.drift.quiet_roll = false;
        loose.redraw = Redraw::Always;
        let mut director = Director::new(Level::Full, loose, 0.0, 0.0);
        director.hush(true);
        let _ = hold(&mut director, 5.0);
        let mut rolled = false;
        for _ in 0..240 {
            if director.step(1.0 / 60.0).roll.abs() > 1e-5 {
                rolled = true;
            }
        }
        assert!(rolled);
    }

    #[test]
    fn wander_leans_for_a_full_level_only() {
        let target = aim(0.5, -0.4, 1.02, 1.4, 3.0);
        let mut director = open();
        director.set_wander(Some(target));
        let _ = hold(&mut director, 21.0);
        let wandering = hold(&mut director, 1.0);
        assert!(wandering.look[0] > 0.3 * DEG);
        assert!(wandering.look[1] < -0.2 * DEG);
        let mut subtle = open();
        subtle.set_level(Level::Subtle);
        subtle.set_wander(Some(target));
        let pose = hold(&mut subtle, 22.0);
        assert!(pose.look[0].abs() < 1e-3);
        assert!(pose.look[1].abs() < 1e-3);
    }

    #[test]
    fn level_scales_a_clamped_look() {
        let far = aim(1.0, 1.0, 1.0, 1.2, 4.0);
        let preset = depth();
        let mut full = Director::new(Level::Full, preset, 1.2, 4.0);
        let mut subtle = Director::new(Level::Subtle, preset, 1.2, 4.0);
        full.attend(far);
        subtle.attend(far);
        let full = hold(&mut full, 5.0);
        let subtle = hold(&mut subtle, 5.0);
        assert!((full.look[0] - preset.look.yaw).abs() < 1e-3);
        assert!((subtle.look[0] - 0.5 * preset.look.yaw).abs() < 1e-3);
        assert!((full.look[1] - preset.look.pitch).abs() < 1e-3);
        assert!((subtle.look[1] - 0.5 * preset.look.pitch).abs() < 1e-3);
    }

    #[test]
    fn a_busy_cursor_does_not_lean() {
        let mut director = open();
        for _ in 0..90 {
            director.cursor(1.0, 1.0, true);
            let pose = director.step(1.0 / 60.0);
            assert!(pose.yaw.abs() < 0.3 * DEG);
        }
    }

    #[test]
    fn a_nudge_dips_and_returns() {
        let mut preset = depth();
        preset.nudge.floor = 0.5;
        let mut director = Director::new(Level::Off, preset, 1.2, 4.0);
        director.nudge();
        let mut low = 0.0f32;
        for _ in 0..20 {
            low = low.min(director.step(1.0 / 60.0).pitch);
        }
        assert!(low < 0.0);
        let pose = hold(&mut director, 2.0);
        assert!(pose.pitch.abs() < 1e-4);
    }

    #[test]
    fn idle_drift_stays_under_a_third_of_a_degree() {
        let mut director = open();
        for _ in 0..(30 * 60) {
            let pose = director.step(1.0 / 60.0);
            assert!(pose.yaw.abs() < 0.3 * DEG);
            assert!(pose.pitch.abs() < 0.2 * DEG);
            assert!(pose.roll.abs() < 0.05 * DEG);
        }
    }

    #[test]
    fn level_parse_reads_the_three_names() {
        assert_eq!(Level::parse("off"), Some(Level::Off));
        assert_eq!(Level::parse("subtle"), Some(Level::Subtle));
        assert_eq!(Level::parse("full"), Some(Level::Full));
        assert_eq!(Level::parse("0"), None);
        assert_eq!(Level::parse("nope"), None);
    }

    #[test]
    fn a_spec_parses_a_pose() {
        let pose = Pose::from_spec(
            "yaw=10, pitch=-2, roll=0.5, dolly=0.02, zoom=0.4, focus=1.6, blur=3, lookx=4, looky=-1, nope=1",
        );
        assert!((pose.yaw - 10.0 * DEG).abs() < 1e-6);
        assert!((pose.pitch + 2.0 * DEG).abs() < 1e-6);
        assert!((pose.roll - 0.5 * DEG).abs() < 1e-6);
        assert_eq!(pose.offset, 0.02);
        assert_eq!(pose.zoom, 1.0);
        assert_eq!(pose.focus, 1.6);
        assert_eq!(pose.blur, 3.0);
        assert!((pose.look[0] - 4.0 * DEG).abs() < 1e-6);
        assert!((pose.look[1] + DEG).abs() < 1e-6);
        assert!(!pose.moved);
        let blank = Pose::from_spec("broken, zoom=two");
        assert_eq!(blank.zoom, 1.0);
        assert_eq!(blank.focus, 0.0);
    }

    #[test]
    fn a_preset_without_depth_ignores_focus_and_blur() {
        let mut director = Director::new(Level::Full, Preset::default(), 1.0, 8.0);
        director.attend(aim(0.0, 0.0, 1.0, 3.0, 40.0));
        let pose = hold(&mut director, 3.0);
        assert_eq!(pose.focus, 0.0);
        assert_eq!(pose.blur, 0.0);
    }

    #[test]
    fn set_level_off_brings_the_rig_back_to_rest() {
        let mut director = open();
        director.cursor(1.0, 0.5, false);
        let _ = hold(&mut director, 1.0);
        director.set_level(Level::Off);
        let pose = hold(&mut director, 6.0);
        still(&pose);
        assert_eq!(pose.zoom, 1.0);
    }
}
