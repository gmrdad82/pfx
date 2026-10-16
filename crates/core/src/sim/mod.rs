pub mod director;
pub mod journal;
mod math;
mod overlap;
pub mod particles;
mod rng;
pub mod shots;

pub use director::Director;
pub use journal::{Journal, JournalWriter, Schema};
pub use math::{atan2, atan2f, cos, cosf, exp, expf, ln, lnf, pow, powf, sin, sinf, sqrt, sqrtf};
pub use overlap::{Aabb, Aabb3, Hit, Overlaps, Overlaps3};
pub use rng::{Rng, RngState, STATE_BYTES, STREAM_VERSION, SimError, mix64, splitmix64};
pub use shots::Shots;

#[cfg(test)]
mod tests_director;
#[cfg(test)]
mod tests_guard;
#[cfg(test)]
mod tests_journal;
#[cfg(test)]
mod tests_math;
#[cfg(test)]
mod tests_overlap;
#[cfg(test)]
mod tests_particles;
#[cfg(test)]
mod tests_rng;
