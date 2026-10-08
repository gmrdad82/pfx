mod flock;
mod fluid;
mod math;
mod noise;
mod particles;
#[cfg(feature = "rigid")]
pub mod rigid;
mod ripple;
mod rope;
mod shape;
mod wind;

pub use flock::{Boid, Flock, FlockParams};
pub use fluid::{Fluid, FluidDesc, FluidState, Outline, Reference, Smoothing, Wire};
pub use particles::{Emitter, ParticleKind, Speck};
pub use ripple::{DAMP, DROPS, Drop, FIXED_DT, RIPPLE_WGSL, Ripple, RippleGpu};
pub use rope::{Chain, ChainParams, Drift, Rope, SUBSTEP as ROPE_SUBSTEP};
pub use shape::{
    Form, KERNEL, Look, Params, Particle, Pour, SPACING, SUBSTEP, Target, rest_density,
};
pub use wind::{BEND_WGSL, Wind, sway};

#[cfg(test)]
mod gpu_test;
