mod emit;
mod follow;
mod pool;

#[cfg(feature = "serde")]
mod arr;
#[cfg(feature = "serde")]
mod save;

pub use emit::{
    Aim2, Aim3, Burst, Emitter, FULL_TURN, Fire, Pattern, Phase, Rate, Telegraph, intercept,
};
pub use follow::{Member, Mover, Slow, follow, follow_scaled, toward};
pub use pool::{End, Shot, ShotId, Shots, Span, Strike};

#[cfg(test)]
mod tests;
