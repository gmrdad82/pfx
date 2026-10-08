mod config;
mod driver;
#[cfg(test)]
mod gpu_paint_tests;
#[cfg(test)]
mod gpu_tests;
pub mod headless;
mod host;
pub mod launcher;
#[cfg(feature = "modules")]
pub mod modules {
    pub use pfx_mod::*;
}
mod painter;
pub mod prompt;
mod start;
#[cfg(feature = "steam")]
pub mod steam;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_flat;
#[cfg(test)]
mod tests_host;
#[cfg(test)]
mod tests_input;

pub use config::{Config, Crash, EnvironmentTexels, Pads, WARM_BUDGET};
pub use driver::{Driver, Event, present_preference};
pub use headless::{Count, Headless, StandIn, WindowCall};
pub use host::run;
pub use launcher::{
    CommandError, Commands, Decision, Factory, Outcome, Unmatched, decide, factory, launch,
};
pub use painter::{Drawn, Painter, Warmup};
pub use pfx_core::clock::Tick;
pub use pfx_gpu::screens::{Device, ScreenPolicy};
pub use pfx_input::{ActionSpec, Control, Glyph, Input, Rumble};
pub use pfx_live::flat;
pub use pfx_live::renderer::Exposure;
pub use pfx_live::renderer::Renderer;
pub use pfx_play::{
    Anchor, Audio, Cap, Exit, FrameTime, Game, GameError, Label, Level, Notice, Pacing, PaintFrame,
    PcmStream, Prompt, PromptOf, Reload, RumbleRequest, SceneGame, Settings, Ui, UiFrame, UiLook,
    Warm, Warming, World,
};
pub use prompt::Part;
