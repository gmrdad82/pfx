mod emit;
mod follow;
mod pool;

pub use emit::{
    Aim2, Aim3, Burst, Emitter, FULL_TURN, Fire, Pattern, Phase, Rate, Telegraph, intercept,
};
pub use follow::{Member, Mover, Slow, follow, toward};
pub use pool::{End, Shot, ShotId, Shots, Span};

#[cfg(test)]
mod tests;
