mod director;
mod math;
mod overlap;
mod rng;

pub use director::{
    DIRECTOR_VERSION, Director, DirectorError, Entry, Lull, MAX_ORDERS_PER_TICK, NO_SPOT, ONE,
    PICKS, POSITIONS, Phase, Ramp, Region, Schedule, Shape, SpawnLull, SpawnOrder, Spots, TIMING,
    Table, UNCAPPED, Window,
};
pub use math::{atan2, atan2f, cos, cosf, exp, expf, ln, lnf, pow, powf, sin, sinf, sqrt, sqrtf};
pub use overlap::{Aabb, Hit, Overlaps};
pub use rng::{Rng, RngState, STATE_BYTES, STREAM_VERSION, SimError, mix64, splitmix64};

#[cfg(test)]
mod tests_director;
#[cfg(test)]
mod tests_guard;
#[cfg(test)]
mod tests_math;
#[cfg(test)]
mod tests_overlap;
#[cfg(test)]
mod tests_rng;
