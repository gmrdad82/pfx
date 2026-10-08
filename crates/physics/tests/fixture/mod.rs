use pfx_physics::{ChainParams, Drift, FluidDesc, Params};

pub fn chain() -> ChainParams {
    ChainParams {
        pull: 24.0,
        drag: 2.5,
        flow: 12.0,
        slack: 1.1,
        cord: 12.0,
        drift: Drift::default(),
    }
}

pub fn fluid() -> FluidDesc {
    FluidDesc {
        width: 960,
        height: 540,
        max_particles: 32_768,
        max_targets: 64,
        params: Params {
            pressure: 50_000.0,
            contain: 4_000.0,
            cohesion: 30.0,
            damping: 8.0,
            viscosity: 100.0,
            gather: 1.5,
            tension: [15.0, 2.0, 0.5, 0.0],
            apart: [40_000.0, 2.0, 0.0, 0.0],
            life_rate: 3.0,
        },
        settle: 2.0,
    }
}
