mod math;
mod rng;

pub use math::{atan2, atan2f, cos, cosf, exp, expf, ln, lnf, pow, powf, sin, sinf, sqrt, sqrtf};
pub use rng::{Rng, RngState, STATE_BYTES, STREAM_VERSION, SimError, mix64, splitmix64};

#[cfg(test)]
mod tests_guard;
#[cfg(test)]
mod tests_math;
#[cfg(test)]
mod tests_rng;
