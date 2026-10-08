use pfx_core::camera::{
    DEG, Depth, Drift, Harmonic, Hush, Look, Nudge, Orbit, Preset, Redraw, Wander, Zoom,
};
use pfx_core::motion::Rest;

fn harmonic(weight: f32, rate: f32, phase: f32) -> Harmonic {
    Harmonic {
        weight,
        rate,
        phase,
    }
}

pub fn drift() -> Drift {
    Drift {
        yaw: [
            harmonic(0.6, 0.05, 0.0),
            harmonic(0.3, 0.09, 1.0),
            harmonic(0.1, 0.15, 2.0),
        ],
        yaw_gain: 0.2 * DEG,
        pitch: [
            harmonic(0.6, 0.04, 0.5),
            harmonic(0.3, 0.07, 1.5),
            harmonic(0.1, 0.13, 2.5),
        ],
        pitch_gain: 0.1 * DEG,
        roll: harmonic(1.0, 0.03, 0.0),
        roll_gain: 0.02 * DEG,
        offset: harmonic(1.0, 0.05, 0.0),
        offset_gain: 0.001,
        calmed: true,
        quiet_roll: true,
    }
}

pub fn preset() -> Preset {
    Preset {
        gain: 2.0,
        orbit: Orbit {
            stiffness: 4.0,
            yaw: 0.5 * DEG,
            pitch: 0.25 * DEG,
            hold: 1.5,
        },
        look: Look {
            stiffness: 2.5,
            lean: 0.2,
            yaw: 0.6 * DEG,
            pitch: 0.4 * DEG,
            sign: 1.0,
        },
        zoom: Zoom {
            stiffness: 2.5,
            hold: 4.0,
            max: 1.5,
            rate: 1.1,
        },
        wander: Wander {
            idle: 20.0,
            every: 30.0,
            hold: 4.0,
        },
        hush: Some(Hush {
            floor: 0.25,
            stiffness: 2.0,
            blocks_cursor: false,
        }),
        depth: Some(Depth {
            focus: 5.0,
            blur: 3.0,
            floor: 0.4,
        }),
        nudge: Nudge {
            stiffness: 50.0,
            pitch: 0.05 * DEG,
            hold: 0.1,
            floor: 0.2,
        },
        drift: drift(),
        redraw: Redraw::Exact,
        rest: Rest {
            position: 1e-5,
            velocity: 1e-4,
        },
    }
}
