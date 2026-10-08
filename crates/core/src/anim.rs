mod animator;
mod clip;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod math;
mod pose;
mod skeleton;
mod skin;
mod sprite;

pub use animator::{Animator, Fired, MAX_MIXES, MAX_REPEATS, Playback};
pub use clip::{Channel, Clip, ClipEvent, Interpolation, Property};
pub use math::{Mat4, Quat, Trs};
pub use pose::Pose;
pub use skeleton::{Joint, MAX_JOINTS, Rig, Skeleton};
pub use skin::{Skinned, blend_matrix, check_skin, normalize_weights, skin};
pub use sprite::{Atlas, FrameEvent, SpriteClip, SpritePlayer, SpriteSheet};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_sprite;

#[derive(Clone, Debug, PartialEq)]
pub enum AnimError {
    NoJoints,
    TooManyJoints(usize),
    InverseBinds {
        joints: usize,
        binds: usize,
    },
    ParentOrder(String),
    NotFinite(String),
    DuplicateClip(String),
    UnknownClip(String),
    NotPlaying(String),
    NotAnimated(String),
    Channel {
        clip: String,
        reason: String,
    },
    Event {
        clip: String,
        event: String,
        reason: String,
    },
    Weight {
        clip: String,
        weight: f32,
    },
    Speed(f32),
    Fade(f32),
    Start(f32),
    Skin(String),
    Atlas(String),
    Sprite {
        clip: String,
        reason: String,
    },
}

impl std::fmt::Display for AnimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnimError::NoJoints => write!(f, "a skeleton needs at least one joint"),
            AnimError::TooManyJoints(count) => {
                write!(f, "a skeleton of {count} joints is over {MAX_JOINTS}")
            }
            AnimError::InverseBinds { joints, binds } => {
                write!(f, "{binds} inverse bind matrices for {joints} joints")
            }
            AnimError::ParentOrder(joint) => {
                write!(f, "joint {joint} comes before its parent")
            }
            AnimError::NotFinite(what) => write!(f, "{what} is not finite"),
            AnimError::DuplicateClip(clip) => write!(f, "two clips are named {clip}"),
            AnimError::UnknownClip(clip) => write!(f, "no clip is named {clip}"),
            AnimError::NotPlaying(clip) => write!(f, "clip {clip} is not playing"),
            AnimError::NotAnimated(reason) => f.write_str(reason),
            AnimError::Channel { clip, reason } => write!(f, "clip {clip}: {reason}"),
            AnimError::Event {
                clip,
                event,
                reason,
            } => write!(f, "clip {clip}, event {event}: {reason}"),
            AnimError::Weight { clip, weight } => {
                write!(f, "clip {clip}: a weight of {weight} is outside 0 to 1")
            }
            AnimError::Speed(speed) => write!(f, "a speed of {speed} is below 0 or not finite"),
            AnimError::Fade(seconds) => {
                write!(f, "a fade of {seconds} seconds is below 0 or not finite")
            }
            AnimError::Start(seconds) => {
                write!(f, "a start of {seconds} seconds is below 0 or not finite")
            }
            AnimError::Skin(reason) => write!(f, "skin: {reason}"),
            AnimError::Atlas(reason) => write!(f, "atlas: {reason}"),
            AnimError::Sprite { clip, reason } => write!(f, "sprite clip {clip}: {reason}"),
        }
    }
}

impl std::error::Error for AnimError {}
