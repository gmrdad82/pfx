pub const LENGTH: f64 = 2.0;
pub const FADE_IN: f64 = 0.5;
pub const FADE_OUT: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Showing { at: f64, opacity: f32 },
    Waiting,
    Done,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timeline;

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl Timeline {
    pub fn opacity(&self, at: f64) -> f32 {
        smooth(at / FADE_IN).min(smooth((LENGTH - at) / FADE_OUT)) as f32
    }

    pub fn phase(&self, at: f64, loaded: bool) -> Phase {
        if at < LENGTH {
            Phase::Showing {
                at,
                opacity: self.opacity(at),
            }
        } else if loaded {
            Phase::Done
        } else {
            Phase::Waiting
        }
    }
}
