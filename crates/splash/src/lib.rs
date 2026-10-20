mod art;
mod chime;
mod compose;
mod licence;
mod settings;
mod timeline;
mod unlock;

#[cfg(test)]
mod tests;

pub use chime::{CEILING_DB, CHANNELS, RATE, chime};
pub use compose::{Composition, Frame};
pub use licence::{Claims, KeyError, PUBLIC_KEY, build_date, built, covers, hidden, token, verify};
pub use settings::{Colour, Settings};
pub use timeline::{FADE_IN, FADE_OUT, LENGTH, Phase, Timeline};
pub use unlock::{Licence, Refusal, Tier, licence, unlock};
