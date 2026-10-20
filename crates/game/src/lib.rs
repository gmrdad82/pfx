mod clipboard;
mod config;
mod cursor;
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
    #[doc(no_inline)]
    pub use pfx_mod::*;
}
mod painter;
pub mod prompt;
mod scheme;
mod start;
#[cfg(feature = "steam")]
pub mod steam;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_clipboard;
#[cfg(test)]
mod tests_flat;
#[cfg(test)]
mod tests_host;
#[cfg(test)]
mod tests_input;
#[cfg(feature = "modules")]
pub mod wasm;

pub use config::{Config, Crash, EnvironmentTexels, Pads, Redraw, WARM_BUDGET};
pub use driver::{Driver, Event, present_preference};
pub use headless::{Count, Headless, StandIn, WindowCall};
pub use host::run;
pub use launcher::{
    CommandError, Commands, Decision, Factory, Outcome, Unmatched, decide, factory, launch,
};
pub use painter::{Drawn, FRAMES_IN_FLIGHT, Painter, Warmup};
pub use pfx_core::clock::Tick;
pub use pfx_gpu::screens::{Device, ScreenPolicy};
pub use pfx_input::{ActionSpec, Control, Glyph, Input, Rumble};
pub use pfx_live::flat;
pub use pfx_live::renderer::Exposure;
pub use pfx_live::renderer::Renderer;
pub use pfx_play::{
    Anchor, Audio, Clipboard, Cursor, Exit, FrameRate, FrameTime, Game, GameError, Label, Level,
    Notice, Pacing, PaintFrame, PcmStream, Present, Prompt, PromptOf, Reload, RumbleRequest,
    SceneGame, Scheme, Settings, Ui, UiFrame, UiLook, Warm, Warming, World,
};
pub use pfx_play::{MenuLook, SettingsMenu, menu};
pub use pfx_settings::{Effect, Effects, Graphics, Quality};
#[cfg(feature = "splash")]
pub use pfx_splash::{Colour as SplashColour, Settings as SplashSettings, Show as SplashShow};
pub use prompt::Part;
pub use scheme::system_scheme;
