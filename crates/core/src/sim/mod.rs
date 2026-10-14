mod math;
mod overlap;
mod rng;
mod shots;

pub use math::{atan2, atan2f, cos, cosf, exp, expf, ln, lnf, pow, powf, sin, sinf, sqrt, sqrtf};
pub use overlap::{Aabb, Hit, Overlaps};
pub use rng::{Rng, RngState, STATE_BYTES, STREAM_VERSION, SimError, mix64, splitmix64};
pub use shots::{
    Aim2, Aim3, Burst, Emitter, End, FULL_TURN, Fire, Member, Mover, Pattern, Phase, Rate, Shot,
    ShotId, Shots, Slow, Span, Telegraph, follow, intercept, toward,
};

#[cfg(test)]
mod tests_guard;
#[cfg(test)]
mod tests_math;
#[cfg(test)]
mod tests_overlap;
#[cfg(test)]
mod tests_rng;
