mod art;
mod chime;
mod compose;
mod licence;
mod player;
mod settings;
mod timeline;
mod unlock;

#[cfg(test)]
mod tests;

pub use chime::{CEILING_DB, CHANNELS, RATE, chime};
pub use compose::{Composition, Frame};
pub use licence::{Claims, KeyError, PUBLIC_KEY, build_date, built, covers, hidden, token, verify};
pub use pfx_live::flat::Srgba;
pub use player::Player;
pub use settings::{Colour, Settings, Show};
pub use timeline::{FADE_IN, FADE_OUT, LENGTH, Phase, Timeline};
pub use unlock::{Licence, Refusal, Tier, licence, unlock};
