use super::AnimError;
use super::math::{lerp3, nlerp, quat_normalize, slerp};
use super::pose::Pose;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Step,
    Linear,
    CubicSpline,
}

impl Interpolation {
    fn stride(self) -> usize {
        match self {
            Interpolation::CubicSpline => 3,
            Interpolation::Step | Interpolation::Linear => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

impl Property {
    pub fn width(self) -> usize {
        match self {
            Property::Rotation => 4,
            Property::Translation | Property::Scale => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub joint: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

fn check_times(times: &[f32]) -> Result<(), String> {
    if times.is_empty() {
        return Err("a channel has no keys".into());
    }
    if times.iter().any(|time| !time.is_finite() || *time < 0.0) {
        return Err("a key time is negative or not finite".into());
    }
    if times.windows(2).any(|pair| pair[1] <= pair[0]) {
        return Err("key times do not rise".into());
    }
    Ok(())
}

impl Channel {
    pub fn check(&self) -> Result<(), String> {
        check_times(&self.times)?;
        let expected = self.times.len() * self.interpolation.stride() * self.property.width();
        if self.values.len() != expected {
            return Err(format!(
                "a channel holds {} values where its {} keys need {expected}",
                self.values.len(),
                self.times.len()
            ));
        }
        if self.values.iter().any(|value| !value.is_finite()) {
            return Err("a key value is not finite".into());
        }
        Ok(())
    }

    fn key(&self, index: usize, part: usize) -> [f32; 4] {
        let width = self.property.width();
        let start = (index * self.interpolation.stride() + part) * width;
        let mut out = [0.0; 4];
        out[..width].copy_from_slice(&self.values[start..start + width]);
        out
    }

    fn value(&self, index: usize) -> [f32; 4] {
        match self.interpolation {
            Interpolation::CubicSpline => self.key(index, 1),
            Interpolation::Step | Interpolation::Linear => self.key(index, 0),
        }
    }

    pub fn sample(&self, time: f32) -> [f32; 4] {
        let after = self.times.partition_point(|key| *key <= time);
        let last = self.times.len() - 1;
        let raw = if after == 0 {
            self.value(0)
        } else if after > last {
            self.value(last)
        } else {
            let k0 = after - 1;
            let k1 = after;
            let t0 = self.times[k0];
            let t1 = self.times[k1];
            let span = t1 - t0;
            let u = ((time - t0) / span).clamp(0.0, 1.0);
            match self.interpolation {
                Interpolation::Step => self.value(k0),
                Interpolation::Linear => {
                    let a = self.value(k0);
                    let b = self.value(k1);
                    if self.property == Property::Rotation {
                        slerp(a, b, u)
                    } else {
                        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * u)
                    }
                }
                Interpolation::CubicSpline => {
                    let v0 = self.key(k0, 1);
                    let b0 = self.key(k0, 2);
                    let a1 = self.key(k1, 0);
                    let v1 = self.key(k1, 1);
                    let u2 = u * u;
                    let u3 = u2 * u;
                    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
                    let h10 = u3 - 2.0 * u2 + u;
                    let h01 = -2.0 * u3 + 3.0 * u2;
                    let h11 = u3 - u2;
                    std::array::from_fn(|k| {
                        h00 * v0[k] + h10 * span * b0[k] + h01 * v1[k] + h11 * span * a1[k]
                    })
                }
            }
        };
        if self.property == Property::Rotation {
            quat_normalize(raw)
        } else {
            raw
        }
    }

    pub fn end(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MorphChannel {
    pub morph: usize,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

impl MorphChannel {
    pub fn width(&self) -> usize {
        self.values
            .len()
            .checked_div(self.times.len() * self.interpolation.stride())
            .unwrap_or(0)
    }

    pub fn check(&self) -> Result<(), String> {
        check_times(&self.times)?;
        let keys = self.times.len() * self.interpolation.stride();
        if self.values.is_empty() || !self.values.len().is_multiple_of(keys) {
            return Err(format!(
                "a weights channel holds {} values, not a whole number of weights for its {} keys",
                self.values.len(),
                self.times.len()
            ));
        }
        if self.values.iter().any(|value| !value.is_finite()) {
            return Err("a key value is not finite".into());
        }
        Ok(())
    }

    pub fn sample_into(&self, time: f32, out: &mut [f32]) {
        sample_weights(
            self.interpolation,
            &self.times,
            &self.values,
            time,
            |target, value| {
                if let Some(slot) = out.get_mut(target) {
                    *slot = value;
                }
            },
        );
    }

    pub fn end(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }
}

pub fn sample_weights(
    interpolation: Interpolation,
    times: &[f32],
    values: &[f32],
    time: f32,
    mut write: impl FnMut(usize, f32),
) {
    let stride = interpolation.stride();
    let keys = times.len() * stride;
    if keys == 0 || values.len() < keys {
        return;
    }
    let width = values.len() / keys;
    let at =
        |index: usize, part: usize, target: usize| values[(index * stride + part) * width + target];
    let middle = match interpolation {
        Interpolation::CubicSpline => 1,
        Interpolation::Step | Interpolation::Linear => 0,
    };
    let after = times.partition_point(|key| *key <= time);
    let last = times.len() - 1;
    if after == 0 || after > last {
        let index = if after == 0 { 0 } else { last };
        for target in 0..width {
            write(target, at(index, middle, target));
        }
        return;
    }
    let k0 = after - 1;
    let k1 = after;
    let span = times[k1] - times[k0];
    let u = ((time - times[k0]) / span).clamp(0.0, 1.0);
    for target in 0..width {
        let value = match interpolation {
            Interpolation::Step => at(k0, 0, target),
            Interpolation::Linear => {
                let a = at(k0, 0, target);
                a + (at(k1, 0, target) - a) * u
            }
            Interpolation::CubicSpline => {
                let u2 = u * u;
                let u3 = u2 * u;
                (2.0 * u3 - 3.0 * u2 + 1.0) * at(k0, 1, target)
                    + (u3 - 2.0 * u2 + u) * span * at(k0, 2, target)
                    + (-2.0 * u3 + 3.0 * u2) * at(k1, 1, target)
                    + (u3 - u2) * span * at(k1, 0, target)
            }
        };
        write(target, value);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipEvent {
    pub time: f32,
    pub name: String,
}

impl ClipEvent {
    pub fn new(time: f32, name: impl Into<String>) -> Self {
        Self {
            time,
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    name: String,
    channels: Vec<Channel>,
    weights: Vec<MorphChannel>,
    events: Vec<ClipEvent>,
    duration: f32,
}

impl Clip {
    pub fn new(name: impl Into<String>, channels: Vec<Channel>) -> Result<Self, AnimError> {
        let name = name.into();
        for channel in &channels {
            channel.check().map_err(|reason| AnimError::Channel {
                clip: name.clone(),
                reason,
            })?;
        }
        let duration = channels.iter().map(Channel::end).fold(0.0f32, f32::max);
        Ok(Self {
            name,
            channels,
            weights: Vec::new(),
            events: Vec::new(),
            duration,
        })
    }

    pub fn with_weights(mut self, weights: Vec<MorphChannel>) -> Result<Self, AnimError> {
        for channel in &weights {
            channel.check().map_err(|reason| AnimError::Channel {
                clip: self.name.clone(),
                reason,
            })?;
        }
        self.duration = weights
            .iter()
            .map(MorphChannel::end)
            .fold(self.duration, f32::max);
        self.weights = weights;
        Ok(self)
    }

    pub(crate) fn empty(name: &str) -> Self {
        Self {
            name: name.to_string(),
            channels: Vec::new(),
            weights: Vec::new(),
            events: Vec::new(),
            duration: 0.0,
        }
    }

    pub fn with_duration(mut self, duration: f32) -> Result<Self, AnimError> {
        if !duration.is_finite() || duration < self.duration {
            return Err(AnimError::Channel {
                clip: self.name.clone(),
                reason: format!(
                    "a duration of {duration} is shorter than the clip's keys ({})",
                    self.duration
                ),
            });
        }
        self.duration = duration;
        Ok(self)
    }

    pub fn with_events(mut self, mut events: Vec<ClipEvent>) -> Result<Self, AnimError> {
        for event in &events {
            if !event.time.is_finite() || event.time < 0.0 || event.time > self.duration {
                return Err(AnimError::Event {
                    clip: self.name.clone(),
                    event: event.name.clone(),
                    reason: format!(
                        "its time {} is outside the clip's 0 to {}",
                        event.time, self.duration
                    ),
                });
            }
        }
        events.sort_by(|a, b| a.time.total_cmp(&b.time).then_with(|| a.name.cmp(&b.name)));
        self.events = events;
        Ok(self)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    pub fn weights(&self) -> &[MorphChannel] {
        &self.weights
    }

    pub fn apply_weights(&self, time: f32, morphs: &mut [Vec<f32>], weight: f32) {
        if weight <= 0.0 {
            return;
        }
        for channel in &self.weights {
            let Some(set) = morphs.get_mut(channel.morph) else {
                continue;
            };
            sample_weights(
                channel.interpolation,
                &channel.times,
                &channel.values,
                time,
                |target, value| {
                    if let Some(slot) = set.get_mut(target) {
                        *slot = if weight >= 1.0 {
                            value
                        } else {
                            *slot + (value - *slot) * weight
                        };
                    }
                },
            );
        }
    }

    pub fn events(&self) -> &[ClipEvent] {
        &self.events
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn sample(&self, time: f32, pose: &mut Pose) {
        self.apply(time, pose, 1.0);
    }

    pub fn apply(&self, time: f32, pose: &mut Pose, weight: f32) {
        if weight <= 0.0 {
            return;
        }
        let locals = pose.locals_mut();
        for channel in &self.channels {
            let Some(local) = locals.get_mut(channel.joint) else {
                continue;
            };
            let value = channel.sample(time);
            let three = [value[0], value[1], value[2]];
            match channel.property {
                Property::Translation if weight >= 1.0 => local.translation = three,
                Property::Translation => {
                    local.translation = lerp3(local.translation, three, weight)
                }
                Property::Scale if weight >= 1.0 => local.scale = three,
                Property::Scale => local.scale = lerp3(local.scale, three, weight),
                Property::Rotation if weight >= 1.0 => local.rotation = value,
                Property::Rotation => local.rotation = nlerp(local.rotation, value, weight),
            }
        }
    }
}
