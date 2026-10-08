use std::sync::Arc;

use super::AnimError;
use super::math::Mat4;
use super::pose::Pose;
use super::skeleton::Rig;

pub const MAX_MIXES: usize = 4;
pub const MAX_REPEATS: u64 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Playback {
    #[default]
    Loop,
    Once,
    PingPong,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    pub clip: String,
    pub event: String,
    pub time: f32,
    pub tick: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Layer {
    clip: usize,
    start: u64,
    offset: f64,
    begin: f64,
    speed: f32,
    weight: f32,
    playback: Playback,
    heard: Option<f64>,
}

impl Layer {
    fn new(clip: usize, tick: u64) -> Self {
        Self {
            clip,
            start: tick,
            offset: 0.0,
            begin: 0.0,
            speed: 1.0,
            weight: 1.0,
            playback: Playback::Loop,
            heard: None,
        }
    }

    fn elapsed(&self, tick: u64, rate: u32) -> f64 {
        let ticks = tick.saturating_sub(self.start) as f64;
        self.offset + ticks * self.speed as f64 / rate as f64
    }

    fn rebase(&mut self, tick: u64, rate: u32) {
        self.offset = self.elapsed(tick, rate);
        self.start = tick.max(self.start);
    }
}

fn local_time(elapsed: f64, duration: f32, playback: Playback) -> f32 {
    let duration = duration as f64;
    let time = match playback {
        Playback::Once => elapsed.min(duration),
        _ if duration <= 0.0 => 0.0,
        Playback::Loop => elapsed - (elapsed / duration).floor() * duration,
        Playback::PingPong => {
            let period = 2.0 * duration;
            let at = elapsed - (elapsed / period).floor() * period;
            if at > duration { period - at } else { at }
        }
    };
    time as f32
}

#[derive(Clone, Debug, PartialEq)]
struct Mix {
    layers: Vec<Layer>,
    start: u64,
    fade: u64,
}

impl Mix {
    fn progress(&self, tick: u64) -> f32 {
        if self.fade == 0 {
            return 1.0;
        }
        let done = tick.saturating_sub(self.start).min(self.fade);
        (done as f64 / self.fade as f64) as f32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animator {
    rig: Arc<Rig>,
    rate: u32,
    mixes: Vec<Mix>,
}

impl Animator {
    pub fn new(rig: Arc<Rig>, rate: u32) -> Self {
        Self {
            rig,
            rate: rate.max(1),
            mixes: Vec::new(),
        }
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    pub fn shared_rig(&self) -> &Arc<Rig> {
        &self.rig
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    fn find(&self, clip: &str) -> Result<usize, AnimError> {
        self.rig
            .clip(clip)
            .ok_or_else(|| AnimError::UnknownClip(clip.to_string()))
    }

    fn layer_mut(&mut self, clip: &str) -> Result<&mut Layer, AnimError> {
        let index = self.find(clip)?;
        self.mixes
            .last_mut()
            .and_then(|mix| mix.layers.iter_mut().find(|layer| layer.clip == index))
            .ok_or_else(|| AnimError::NotPlaying(clip.to_string()))
    }

    pub fn play(&mut self, clip: &str, tick: u64) -> Result<(), AnimError> {
        self.crossfade(clip, 0.0, tick)
    }

    pub fn crossfade(&mut self, clip: &str, seconds: f32, tick: u64) -> Result<(), AnimError> {
        let index = self.find(clip)?;
        let fade = fade_ticks(seconds, self.rate)?;
        self.mixes.push(Mix {
            layers: vec![Layer::new(index, tick)],
            start: tick,
            fade,
        });
        if self.mixes.len() > MAX_MIXES {
            let extra = self.mixes.len() - MAX_MIXES;
            self.mixes.drain(..extra);
        }
        Ok(())
    }

    pub fn set_fade(&mut self, seconds: f32) -> Result<(), AnimError> {
        let fade = fade_ticks(seconds, self.rate)?;
        if let Some(mix) = self.mixes.last_mut() {
            mix.fade = fade;
        }
        Ok(())
    }

    pub fn blend(&mut self, clip: &str, weight: f32, tick: u64) -> Result<(), AnimError> {
        let index = self.find(clip)?;
        let weight = check_weight(clip, weight)?;
        let Some(mix) = self.mixes.last_mut() else {
            self.mixes.push(Mix {
                layers: vec![Layer {
                    weight,
                    ..Layer::new(index, tick)
                }],
                start: tick,
                fade: 0,
            });
            return Ok(());
        };
        match mix.layers.iter_mut().find(|layer| layer.clip == index) {
            Some(layer) => layer.weight = weight,
            None => mix.layers.push(Layer {
                weight,
                ..Layer::new(index, tick)
            }),
        }
        Ok(())
    }

    pub fn set_weight(&mut self, clip: &str, weight: f32) -> Result<(), AnimError> {
        let weight = check_weight(clip, weight)?;
        self.layer_mut(clip)?.weight = weight;
        Ok(())
    }

    pub fn set_speed(&mut self, clip: &str, speed: f32, tick: u64) -> Result<(), AnimError> {
        if !speed.is_finite() || speed < 0.0 {
            return Err(AnimError::Speed(speed));
        }
        let rate = self.rate;
        let layer = self.layer_mut(clip)?;
        layer.rebase(tick, rate);
        layer.speed = speed;
        Ok(())
    }

    pub fn set_start(&mut self, clip: &str, seconds: f32, tick: u64) -> Result<(), AnimError> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(AnimError::Start(seconds));
        }
        let layer = self.layer_mut(clip)?;
        layer.start = tick;
        layer.offset = seconds as f64;
        layer.begin = seconds as f64;
        layer.heard = None;
        Ok(())
    }

    pub fn play_blend(&mut self, clips: &[(&str, f32)], tick: u64) -> Result<(), AnimError> {
        let Some((first, _)) = clips.first() else {
            return Err(AnimError::UnknownClip(String::new()));
        };
        for (clip, weight) in clips {
            self.find(clip)?;
            if !weight.is_finite() || *weight < 0.0 {
                return Err(AnimError::Weight {
                    clip: clip.to_string(),
                    weight: *weight,
                });
            }
        }
        self.play(first, tick)?;
        let mut sum = 0.0f32;
        for (index, (clip, weight)) in clips.iter().enumerate() {
            sum += weight;
            let share = if sum > 0.0 { weight / sum } else { 0.0 };
            if index == 0 {
                self.set_weight(clip, if *weight > 0.0 { 1.0 } else { 0.0 })?;
            } else {
                self.blend(clip, share.clamp(0.0, 1.0), tick)?;
            }
        }
        Ok(())
    }

    pub fn set_playback(&mut self, clip: &str, playback: Playback) -> Result<(), AnimError> {
        self.layer_mut(clip)?.playback = playback;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.mixes.clear();
    }

    pub fn is_playing(&self) -> bool {
        !self.mixes.is_empty()
    }

    pub fn playing(&self) -> Vec<(&str, f32)> {
        self.mixes
            .last()
            .map(|mix| {
                mix.layers
                    .iter()
                    .map(|layer| (self.rig.clips()[layer.clip].name(), layer.weight))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn time(&self, clip: &str, tick: u64) -> Option<f32> {
        let index = self.rig.clip(clip)?;
        let layer = self
            .mixes
            .last()?
            .layers
            .iter()
            .find(|layer| layer.clip == index)?;
        Some(local_time(
            layer.elapsed(tick, self.rate),
            self.rig.clips()[index].duration(),
            layer.playback,
        ))
    }

    pub fn finished(&self, clip: &str, tick: u64) -> bool {
        let Some(index) = self.rig.clip(clip) else {
            return false;
        };
        self.mixes.last().is_some_and(|mix| {
            mix.layers.iter().any(|layer| {
                layer.clip == index
                    && layer.playback == Playback::Once
                    && layer.elapsed(tick, self.rate) >= self.rig.clips()[index].duration() as f64
            })
        })
    }

    fn mix_pose(&self, mix: &Mix, tick: u64, out: &mut Pose) {
        out.reset(self.rig.skeleton());
        for layer in &mix.layers {
            let clip = &self.rig.clips()[layer.clip];
            let time = local_time(
                layer.elapsed(tick, self.rate),
                clip.duration(),
                layer.playback,
            );
            clip.apply(time, out, layer.weight);
        }
    }

    pub fn evaluate(&self, tick: u64, pose: &mut Pose) {
        let skeleton = self.rig.skeleton();
        pose.reset(skeleton);
        let mut layer = Pose::rest(skeleton);
        let first = self
            .mixes
            .iter()
            .rposition(|mix| mix.progress(tick) >= 1.0)
            .unwrap_or(0);
        for (index, mix) in self.mixes.iter().enumerate().skip(first) {
            if index == first {
                self.mix_pose(mix, tick, pose);
            } else {
                self.mix_pose(mix, tick, &mut layer);
                pose.mix(&layer, mix.progress(tick));
            }
        }
    }

    pub fn pose(&self, tick: u64) -> Pose {
        let mut pose = Pose::rest(self.rig.skeleton());
        self.evaluate(tick, &mut pose);
        pose
    }

    pub fn palette(&self, tick: u64) -> Vec<Mat4> {
        self.pose(tick).palette(self.rig.skeleton())
    }

    pub fn advance(&mut self, tick: u64) -> Vec<Fired> {
        if let Some(done) = self.mixes.iter().rposition(|mix| mix.progress(tick) >= 1.0) {
            self.mixes.drain(..done);
        }
        let rate = self.rate;
        let rig = &self.rig;
        let mut fired = Vec::new();
        for mix in &mut self.mixes {
            for layer in &mut mix.layers {
                let clip = &rig.clips()[layer.clip];
                let now = layer.elapsed(tick, rate);
                let heard = layer.heard.replace(now);
                if layer.weight <= 0.0 || clip.events().is_empty() {
                    continue;
                }
                let mut found: Vec<(f64, usize)> = Vec::new();
                for (order, event) in clip.events().iter().enumerate() {
                    for at in occurrences(
                        event.time,
                        clip.duration(),
                        layer.playback,
                        heard,
                        layer.begin,
                        now,
                    ) {
                        found.push((at, order));
                    }
                }
                found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                fired.extend(found.into_iter().map(|(_, order)| {
                    let event = &clip.events()[order];
                    Fired {
                        clip: clip.name().to_string(),
                        event: event.name.clone(),
                        time: event.time,
                        tick,
                    }
                }));
            }
        }
        fired
    }
}

fn occurrences(
    time: f32,
    duration: f32,
    playback: Playback,
    heard: Option<f64>,
    begin: f64,
    now: f64,
) -> Vec<f64> {
    let time = time as f64;
    let duration = duration as f64;
    let lower = heard.unwrap_or(begin);
    let after = |at: f64| match heard {
        Some(heard) => at > heard,
        None => at >= begin,
    } && at <= now;
    let series = |base: f64, period: f64| -> Vec<f64> {
        if now < base {
            return Vec::new();
        }
        let last = ((now - base) / period).floor() as u64;
        let first = if lower <= base {
            0
        } else {
            ((lower - base) / period).floor() as u64
        };
        if first > last {
            return Vec::new();
        }
        let first = first.max(last.saturating_sub(MAX_REPEATS - 1));
        (first..=last)
            .map(|k| base + k as f64 * period)
            .filter(|&at| after(at))
            .collect()
    };
    let mut found = match playback {
        Playback::Once => return if after(time) { vec![time] } else { Vec::new() },
        _ if duration <= 0.0 => return if after(time) { vec![time] } else { Vec::new() },
        Playback::Loop => series(time, duration),
        Playback::PingPong => {
            let mut both = series(time, 2.0 * duration);
            both.extend(series(2.0 * duration - time, 2.0 * duration));
            both
        }
    };
    found.sort_by(f64::total_cmp);
    found.dedup();
    found
}

fn fade_ticks(seconds: f32, rate: u32) -> Result<u64, AnimError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(AnimError::Fade(seconds));
    }
    Ok((seconds as f64 * rate as f64).round() as u64)
}

fn check_weight(clip: &str, weight: f32) -> Result<f32, AnimError> {
    if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
        return Err(AnimError::Weight {
            clip: clip.to_string(),
            weight,
        });
    }
    Ok(weight)
}
