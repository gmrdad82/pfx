use crate::clip::Clip;

#[derive(Clone, Debug)]
pub struct MusicTrack {
    pub clip: Clip,
    pub looped: bool,
    pub loop_start: u64,
    pub loop_end: u64,
}

impl MusicTrack {
    pub fn once(clip: Clip) -> Self {
        let loop_end = clip.frames();
        Self {
            clip,
            looped: false,
            loop_start: 0,
            loop_end,
        }
    }

    pub fn looping(clip: Clip) -> Self {
        Self {
            looped: true,
            ..Self::once(clip)
        }
    }

    pub fn with_loop(clip: Clip, loop_start: u64, loop_end: u64) -> Self {
        Self {
            looped: true,
            loop_start,
            loop_end,
            ..Self::once(clip)
        }
    }

    pub(crate) fn loop_frames(&self, rate: u32) -> Option<(u64, u64)> {
        if !self.looped {
            return None;
        }
        scale_loop(
            self.loop_start,
            self.loop_end,
            self.clip.frames(),
            self.clip.rate,
            rate,
        )
    }
}

pub(crate) fn scale_loop(
    start: u64,
    end: u64,
    frames: u64,
    from_rate: u32,
    to_rate: u32,
) -> Option<(u64, u64)> {
    if from_rate == 0 {
        return None;
    }
    let scale = |value: u64| -> u64 {
        let scaled = u128::from(value) * u128::from(to_rate) / u128::from(from_rate);
        u64::try_from(scaled).unwrap_or(u64::MAX)
    };
    let total = scale(frames);
    let end = scale(end.min(frames)).min(total);
    let start = scale(start);
    if end == 0 || start >= end {
        return (total > 0).then_some((0, total));
    }
    Some((start, end))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Direction {
    In,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Curve {
    pub(crate) start: u64,
    pub(crate) len: u64,
    pub(crate) direction: Direction,
}

impl Curve {
    pub(crate) fn gain(&self, at: u64) -> f32 {
        if at < self.start {
            return match self.direction {
                Direction::In => 0.0,
                Direction::Out => 1.0,
            };
        }
        let past = at - self.start;
        if past >= self.len {
            return match self.direction {
                Direction::In => 1.0,
                Direction::Out => 0.0,
            };
        }
        let t = past as f64 / self.len as f64;
        let angle = t * std::f64::consts::FRAC_PI_2;
        match self.direction {
            Direction::In => angle.sin() as f32,
            Direction::Out => angle.cos() as f32,
        }
    }

    pub(crate) fn spent(&self, clock: u64) -> bool {
        self.direction == Direction::Out && clock >= self.start.saturating_add(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_keeps_its_points_across_a_rate_change() {
        let clip = Clip {
            rate: 24_000,
            channels: 1,
            samples: vec![0.0; 1000],
        };
        let track = MusicTrack::with_loop(clip.clone(), 100, 900);
        assert_eq!(track.loop_frames(24_000), Some((100, 900)));
        assert_eq!(track.loop_frames(48_000), Some((200, 1800)));
        assert_eq!(MusicTrack::once(clip.clone()).loop_frames(24_000), None);
        assert_eq!(
            MusicTrack::looping(clip.clone()).loop_frames(24_000),
            Some((0, 1000))
        );
        let backwards = MusicTrack::with_loop(clip, 900, 100);
        assert_eq!(backwards.loop_frames(24_000), Some((0, 1000)));
    }

    #[test]
    fn crossfade_curves_keep_power_constant() {
        let fade_in = Curve {
            start: 10,
            len: 100,
            direction: Direction::In,
        };
        let fade_out = Curve {
            direction: Direction::Out,
            ..fade_in
        };
        for at in 0..130 {
            let sum = fade_in.gain(at).powi(2) + fade_out.gain(at).powi(2);
            assert!((sum - 1.0).abs() < 1e-5, "frame {at}: {sum}");
        }
        assert!(fade_out.spent(110));
        assert!(!fade_out.spent(109));
        assert!(!fade_in.spent(1000));
    }
}
