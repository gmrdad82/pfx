mod art;
mod chime;
mod compose;
mod licence;
mod lockup;
mod player;
mod settings;
mod timeline;
mod unlock;

#[cfg(test)]
mod tests;

pub use chime::{CEILING_DB, CHANNELS, RATE, chime};
pub use compose::{Composition, Frame};
pub use licence::{
    Claims, Group, KeyError, PUBLIC_KEY, build_date, built, covers, hidden, token, verify,
};
pub use lockup::Lockup;
pub use pfx_live::flat::Srgba;
pub use player::Player;
pub use settings::{Colour, Settings, Show};
pub use timeline::{FADE_IN, FADE_OUT, LENGTH, Phase, Timeline};
pub use unlock::{Badge, Licence, Refusal, Tier, licence, unlock};
