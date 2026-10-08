use std::sync::Arc;

use pfx_core::sim::splitmix64;

use crate::channel::ChannelId;
use crate::envelope::Envelope;
use crate::mix::Level;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundId(pub(crate) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steal {
    Oldest,
    Quietest,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundSpec {
    pub channel: ChannelId,
    pub level: Level,
    pub max_voices: usize,
    pub steal: Steal,
    pub pitch_jitter: f32,
    pub volume_jitter: f32,
    pub steal_fade: f32,
    pub envelope: Option<Envelope>,
}

impl Default for SoundSpec {
    fn default() -> Self {
        Self {
            channel: ChannelId::EFFECTS,
            level: Level::default(),
            max_voices: usize::MAX,
            steal: Steal::Oldest,
            pitch_jitter: 0.0,
            volume_jitter: 0.0,
            steal_fade: 0.003,
            envelope: None,
        }
    }
}

pub(crate) struct SoundDef {
    pub(crate) samples: Arc<Vec<f32>>,
    pub(crate) channels: u16,
    pub(crate) frames: u64,
    pub(crate) spec: SoundSpec,
}

pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn seed(&mut self) -> u64 {
        self.next()
    }

    fn next(&mut self) -> u64 {
        splitmix64(&mut self.0)
    }

    pub(crate) fn signed(&mut self) -> f32 {
        let unit = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        unit * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_draws_and_they_stay_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let mut c = Rng::new(8);
        let drawn: Vec<f32> = (0..1000).map(|_| a.signed()).collect();
        let again: Vec<f32> = (0..1000).map(|_| b.signed()).collect();
        let other: Vec<f32> = (0..1000).map(|_| c.signed()).collect();
        assert_eq!(drawn, again);
        assert_ne!(drawn, other);
        assert!(drawn.iter().all(|v| (-1.0..1.0).contains(v)));
        let mean = drawn.iter().sum::<f32>() / 1000.0;
        assert!(mean.abs() < 0.1, "{mean}");
    }
}
