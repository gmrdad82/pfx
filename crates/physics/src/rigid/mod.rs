mod body;
mod character;
mod collider;
mod convert;
mod drive;
mod event;
mod gravity;
mod query;
mod world;

pub use body::{BodyDesc, BodyState, Kind, Locks, Mass};
pub use character::{Character, CharacterDesc, Intent, Plane, TICKS};
pub use collider::{ColliderDesc, Combine, Friction, Layers, Shape};
pub use drive::{Drive, Ease};
pub use event::Event;
pub use gravity::{Falloff, Region, Source, SourceId};
pub use query::{Filter, Hit};
pub use world::{World, WorldDesc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}

impl Pose {
    pub const IDENTITY: Self = Self {
        position: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
    };

    pub fn at(position: [f32; 3]) -> Self {
        Self {
            position,
            ..Self::IDENTITY
        }
    }

    pub fn new(position: [f32; 3], rotation: [f32; 4]) -> Self {
        Self { position, rotation }
    }

    pub fn turned(self, axis: [f32; 3], angle: f32) -> Self {
        let turn = convert::quat_axis_angle(axis, angle);
        Self {
            position: self.position,
            rotation: (turn * convert::quat(self.rotation)).normalize().to_array(),
        }
    }
}

impl Default for Pose {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId {
    index: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColliderId {
    index: u32,
    generation: u32,
}

#[cfg(test)]
mod character_tests;
#[cfg(test)]
mod tests;
