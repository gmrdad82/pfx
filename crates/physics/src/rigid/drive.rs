use pfx_core::motion::Spring;

use super::Pose;
use super::convert::{Iso, iso, lerp_iso};

#[derive(Clone, Copy, Debug)]
pub struct Ease {
    pub from: Pose,
    pub to: Pose,
    pub duration: f32,
    pub elapsed: f32,
    pub curve: fn(f32) -> f32,
}

impl Ease {
    pub fn new(from: Pose, to: Pose, duration: f32, curve: fn(f32) -> f32) -> Self {
        Self {
            from,
            to,
            duration,
            elapsed: 0.0,
            curve,
        }
    }

    pub fn done(&self) -> bool {
        self.elapsed >= self.duration
    }

    fn at(&self) -> Iso {
        let t = if self.duration > 0.0 {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        lerp_iso(&iso(self.from), &iso(self.to), (self.curve)(t))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Drive {
    Spring(Spring<[f32; 3]>),
    Ease(Ease),
}

impl Drive {
    pub fn spring(stiffness: f32, position: [f32; 3]) -> Self {
        Self::Spring(Spring::at(stiffness, position))
    }

    pub fn ease(from: Pose, to: Pose, duration: f32, curve: fn(f32) -> f32) -> Self {
        Self::Ease(Ease::new(from, to, duration, curve))
    }

    pub fn settled(&self) -> bool {
        match self {
            Self::Spring(spring) => spring.settled(),
            Self::Ease(ease) => ease.done(),
        }
    }

    pub(crate) fn advance(&mut self, dt: f32, current: &Iso) -> Iso {
        match self {
            Self::Spring(spring) => {
                spring.step(dt);
                let [x, y, z] = spring.value;
                Iso::from_parts(super::convert::Vec3::new(x, y, z), current.rotation)
            }
            Self::Ease(ease) => {
                ease.elapsed = (ease.elapsed + dt).min(ease.duration.max(0.0));
                ease.at()
            }
        }
    }
}
