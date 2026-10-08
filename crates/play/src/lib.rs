pub mod characters;
mod events;
mod game;
mod hooks;
mod layers;
mod live;
mod math;
mod paint;
mod present;
pub mod rigs;
mod session;
mod settings;
pub mod tunables;
mod ui;
mod world;

#[cfg(test)]
mod tests;

pub use events::{Phase, TriggerEvent, WorldEvent};
pub use game::{FrameTime, Game, GameError, SceneGame};
pub use hooks::{NoHooks, PlayHooks};
pub use layers::Layers;
pub use live::{Edit, Pending, Stamped};
pub use paint::{NoWarmer, PaintFrame, Warm, Warmer, Warming};
pub use pfx_core::modules::{Level, Notice, Reload};
pub use present::PlayFrame;
pub use rigs::{RigDesc, Rigs};
pub use session::{MOTORS_KEPT, Options, PlaySession, Stopped};
pub use settings::{Audio, Cap, Pacing, Settings};
pub use tunables::{Kind, Tunable, TunableValue, Tunables};
pub use ui::{Anchor, Label, Prompt, PromptOf, Ui, UiFrame, UiLook};
pub use world::{
    Animate, AnimationEvent, Contact, Exit, Fx, Hit, Look, ObjectId, Overlap, PcmStream, Query,
    QueryError, RumbleRequest, SoundLevel, Sounds, World, body_volume,
};

#[derive(Clone, Debug, PartialEq)]
pub enum PlayError {
    Clock(String),
    Actions(pfx_input::ActionsError),
    Sound(String),
    Body(String),
    Render(String),
    Tunables(String),
    Rigs(String),
    Layers(String),
    Animation(String),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Clock(message) => write!(f, "play clock: {message}"),
            Self::Actions(error) => write!(f, "play input actions: {error:?}"),
            Self::Sound(message)
            | Self::Body(message)
            | Self::Render(message)
            | Self::Tunables(message)
            | Self::Rigs(message)
            | Self::Layers(message)
            | Self::Animation(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PlayError {}
