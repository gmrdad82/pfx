pub mod device;
pub mod mode;
pub mod policy;
pub mod resolution;
#[cfg(test)]
mod tests;

pub use device::{
    DMI_PRODUCT_NAME, DeckModel, Detected, Device, DeviceSource, Evidence, OVERRIDE_VAR,
    SystemSource, deck_model, detect,
};
#[cfg(feature = "window")]
pub use mode::apply_display;
pub use mode::{Change, DEFAULT_WINDOWED, Display, DisplaySettings, WindowMode};
pub use policy::{
    Aspect, BACKDROP, Bars, DECK_SAFE, DECK_UI_SCALE, DEFAULT_LAYOUT_HEIGHT, DEFAULT_TOLERANCE,
    Inset, PerDevice, PolicyError, ScreenPolicy, ScreenReport,
};
pub use resolution::{Budget, DynamicResolution};
