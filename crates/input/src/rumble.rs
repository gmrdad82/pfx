use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::device::{Motors, PadId};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rumble {
    pub low: f32,
    pub high: f32,
    pub duration_ms: u32,
    pub attack_ms: u32,
    pub release_ms: u32,
}

impl Rumble {
    pub fn new(strength: f32, duration_ms: u32) -> Self {
        Self::motors(strength, strength, duration_ms)
    }

    pub fn motors(low: f32, high: f32, duration_ms: u32) -> Self {
        Self {
            low: unit(low),
            high: unit(high),
            duration_ms,
            attack_ms: 0,
            release_ms: 0,
        }
    }

    pub fn attack(mut self, ms: u32) -> Self {
        self.attack_ms = ms;
        self
    }

    pub fn release(mut self, ms: u32) -> Self {
        self.release_ms = ms;
        self
    }

    pub fn level_at(&self, elapsed_us: u64) -> [f32; 2] {
        let duration = u64::from(self.duration_ms) * 1000;
        if elapsed_us >= duration {
            return [0.0, 0.0];
        }
        let mut gain = 1.0f32;
        let attack = u64::from(self.attack_ms) * 1000;
        if attack > 0 && elapsed_us < attack {
            gain = gain.min(elapsed_us as f32 / attack as f32);
        }
        let release = u64::from(self.release_ms) * 1000;
        let left = duration - elapsed_us;
        if release > 0 && left < release {
            gain = gain.min(left as f32 / release as f32);
        }
        [self.low * gain, self.high * gain]
    }
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotorCommand {
    pub pad: PadId,
    pub low: u16,
    pub high: u16,
    pub hold_ms: u32,
}

#[derive(Clone, Copy, Debug)]
struct Playing {
    rumble: Rumble,
    elapsed_us: u64,
}

#[derive(Clone, Debug)]
pub struct Rumbler {
    intensity: u8,
    step_us: u32,
    playing: BTreeMap<PadId, (Motors, Vec<Playing>)>,
    sent: BTreeMap<PadId, (u16, u16)>,
}

impl Rumbler {
    pub fn new(step_us: u32, intensity: u8) -> Self {
        Self {
            intensity: intensity.min(100),
            step_us: step_us.max(1),
            playing: BTreeMap::new(),
            sent: BTreeMap::new(),
        }
    }

    pub fn intensity(&self) -> u8 {
        self.intensity
    }

    pub fn set_intensity(&mut self, percent: u8) {
        self.intensity = percent.min(100);
        if self.intensity == 0 {
            self.playing.clear();
        }
    }

    pub fn play(&mut self, pad: PadId, motors: Motors, rumble: Rumble) -> bool {
        if self.intensity == 0 || motors == Motors::None || rumble.duration_ms == 0 {
            return false;
        }
        if rumble.low <= 0.0 && rumble.high <= 0.0 {
            return false;
        }
        let entry = self
            .playing
            .entry(pad)
            .or_insert_with(|| (motors, Vec::new()));
        entry.0 = motors;
        entry.1.push(Playing {
            rumble,
            elapsed_us: 0,
        });
        true
    }

    pub fn stop(&mut self, pad: PadId) {
        self.playing.remove(&pad);
    }

    pub fn forget(&mut self, pad: PadId) {
        self.playing.remove(&pad);
        self.sent.remove(&pad);
    }

    pub fn is_playing(&self, pad: PadId) -> bool {
        self.playing.contains_key(&pad)
    }

    pub fn scale(&self, value: f32) -> u16 {
        let scaled = unit(value) * f32::from(self.intensity) / 100.0;
        (scaled * 65535.0).round() as u16
    }

    pub fn step(&mut self) -> Vec<MotorCommand> {
        let mut commands = Vec::new();
        let mut levels = BTreeMap::new();
        for (pad, (motors, events)) in &mut self.playing {
            let mut low = 0.0f32;
            let mut high = 0.0f32;
            let mut hold_us = u64::MAX;
            for event in events.iter() {
                let [l, h] = event.rumble.level_at(event.elapsed_us);
                low = low.max(l);
                high = high.max(h);
                let duration = u64::from(event.rumble.duration_ms) * 1000;
                hold_us = hold_us.min(duration.saturating_sub(event.elapsed_us));
            }
            if *motors == Motors::One {
                let both = low.max(high);
                low = both;
                high = both;
            }
            let step = u64::from(self.step_us);
            for event in events.iter_mut() {
                event.elapsed_us += step;
            }
            events.retain(|event| event.elapsed_us < u64::from(event.rumble.duration_ms) * 1000);
            let hold_ms = (hold_us.min(u64::from(u32::MAX)) + step).div_ceil(1000) as u32;
            levels.insert(*pad, (low, high, hold_ms));
        }
        self.playing.retain(|_, (_, events)| !events.is_empty());
        let sounding: Vec<PadId> = levels.keys().copied().collect();
        for (pad, (low, high, hold_ms)) in levels {
            let value = (self.scale(low), self.scale(high));
            if self.sent.get(&pad) != Some(&value) {
                commands.push(MotorCommand {
                    pad,
                    low: value.0,
                    high: value.1,
                    hold_ms,
                });
                self.sent.insert(pad, value);
            }
        }
        let silent: Vec<PadId> = self
            .sent
            .iter()
            .filter(|(pad, value)| **value != (0, 0) && !sounding.contains(pad))
            .map(|(pad, _)| *pad)
            .collect();
        for pad in silent {
            commands.push(MotorCommand {
                pad,
                low: 0,
                high: 0,
                hold_ms: 0,
            });
            self.sent.insert(pad, (0, 0));
        }
        commands.sort_by_key(|command| command.pad);
        commands
    }
}
