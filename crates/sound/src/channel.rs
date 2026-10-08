use crate::filter::{Filter, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelId(u32);

impl ChannelId {
    pub const MUSIC: ChannelId = ChannelId(0);
    pub const EFFECTS: ChannelId = ChannelId(1);
    pub const DIALOG: ChannelId = ChannelId(2);

    const FIRST_CUSTOM: u32 = 3;

    pub const fn custom(index: u32) -> ChannelId {
        ChannelId(Self::FIRST_CUSTOM.saturating_add(index))
    }
}

pub(crate) const BUILT_IN: [ChannelId; 3] =
    [ChannelId::MUSIC, ChannelId::EFFECTS, ChannelId::DIALOG];

pub const RAMP_SECONDS: f32 = 0.01;

pub(crate) fn ramp_frames(rate: u32) -> u32 {
    ((rate as f32 * RAMP_SECONDS) as u32).max(1)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ramp {
    current: f32,
    target: f32,
    step: f32,
}

impl Ramp {
    pub(crate) fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
        }
    }

    pub(crate) fn current(&self) -> f32 {
        self.current
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.current == self.target
    }

    pub(crate) fn snap(&mut self, value: f32) {
        *self = Self::new(value);
    }

    pub(crate) fn to(&mut self, target: f32, frames: u32) {
        self.target = target;
        self.step = (target - self.current) / frames.max(1) as f32;
    }

    pub(crate) fn next(&mut self) -> f32 {
        if self.current != self.target {
            self.current += self.step;
            let passed = if self.step > 0.0 {
                self.current >= self.target
            } else {
                self.current <= self.target
            };
            if passed || self.step == 0.0 {
                self.current = self.target;
            }
        }
        self.current
    }

    pub(crate) fn skip(&mut self, frames: usize) {
        if self.current == self.target {
            return;
        }
        let moved = self.current + self.step * frames as f32;
        let passed = if self.step > 0.0 {
            moved >= self.target
        } else {
            moved <= self.target
        };
        self.current = if passed || self.step == 0.0 {
            self.target
        } else {
            moved
        };
    }
}

pub(crate) struct Channel {
    pub(crate) volume: f32,
    pub(crate) muted: bool,
    pub(crate) gain: Ramp,
    pub(crate) pitch: Ramp,
    pub(crate) pitch_curve: Vec<f32>,
    pub(crate) pitch_on: bool,
    pub(crate) lowpass: Filter,
    pub(crate) highpass: Filter,
    pub(crate) bus: Vec<f32>,
    pub(crate) touched: bool,
}

impl Channel {
    pub(crate) fn new() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            gain: Ramp::new(1.0),
            pitch: Ramp::new(1.0),
            pitch_curve: Vec::new(),
            pitch_on: false,
            lowpass: Filter::new(Kind::LowPass),
            highpass: Filter::new(Kind::HighPass),
            bus: Vec::new(),
            touched: false,
        }
    }

    pub(crate) fn prepare_pitch(&mut self, frames: usize) {
        self.pitch_on = !(self.pitch.is_settled() && self.pitch.current() == 1.0);
        if self.pitch_on {
            self.pitch_curve.clear();
            for _ in 0..frames {
                self.pitch_curve.push(self.pitch.next());
            }
        }
    }

    pub(crate) fn effective(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }
}

pub(crate) fn clamp_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        0.0
    } else {
        volume.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ducking {
    pub trigger: ChannelId,
    pub target: ChannelId,
    pub depth_db: f32,
    pub attack: f32,
    pub release: f32,
}

impl Ducking {
    pub fn dialog_over_music(depth_db: f32, attack: f32, release: f32) -> Self {
        Self {
            trigger: ChannelId::DIALOG,
            target: ChannelId::MUSIC,
            depth_db,
            attack,
            release,
        }
    }
}

pub(crate) struct DuckState {
    pub(crate) config: Ducking,
    pub(crate) reduction_db: f32,
}

impl DuckState {
    pub(crate) fn new(config: Ducking) -> Self {
        Self {
            config,
            reduction_db: 0.0,
        }
    }

    pub(crate) fn step(&mut self, active: bool, rate: u32) -> f32 {
        let depth = self.config.depth_db.max(0.0);
        let (goal, seconds) = if active {
            (depth, self.config.attack)
        } else {
            (0.0, self.config.release)
        };
        let frames = seconds.max(0.0) * rate as f32;
        let travel = if frames < 1.0 { depth } else { depth / frames };
        if self.reduction_db < goal {
            self.reduction_db = (self.reduction_db + travel).min(goal);
        } else if self.reduction_db > goal {
            self.reduction_db = (self.reduction_db - travel).max(goal);
        }
        if self.reduction_db == 0.0 {
            1.0
        } else {
            10f32.powf(-self.reduction_db / 20.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_channels_never_collide_with_the_built_ins() {
        assert_ne!(ChannelId::custom(0), ChannelId::DIALOG);
        assert_ne!(ChannelId::custom(0), ChannelId::custom(1));
        assert_eq!(ChannelId::custom(7), ChannelId::custom(7));
    }

    #[test]
    fn a_ramp_lands_exactly_on_its_target() {
        let mut ramp = Ramp::new(0.0);
        ramp.to(1.0, 4);
        let seen: Vec<f32> = (0..6).map(|_| ramp.next()).collect();
        assert_eq!(seen, vec![0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
        ramp.to(0.0, 2);
        ramp.skip(1);
        assert_eq!(ramp.current(), 0.5);
        ramp.skip(5);
        assert_eq!(ramp.current(), 0.0);
    }
}
