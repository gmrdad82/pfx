use std::time::Instant;

use pfx_gpu::pace::{FrameClock, TURN_MS_VAR, TURN_WALL_VAR, TurnReport, WallClock};

pub type Report = TurnReport;

const TURN_MS: f64 = 50.0;
const TURN_WALL_MS: f64 = 400.0;

fn env_ms(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

pub struct TurnState {
    threshold_ms: f64,
    wall_ms: f64,
    accumulated_ms: f64,
    longest_ms: f64,
    last_turn: Instant,
}

impl Default for TurnState {
    fn default() -> Self {
        Self::new(env_ms(TURN_MS_VAR, TURN_MS)).with_wall_ms(env_ms(TURN_WALL_VAR, TURN_WALL_MS))
    }
}

impl TurnState {
    pub fn new(threshold_ms: f64) -> Self {
        Self {
            threshold_ms: if threshold_ms.is_finite() {
                threshold_ms.max(1.0)
            } else {
                TURN_MS
            },
            wall_ms: TURN_WALL_MS,
            accumulated_ms: 0.0,
            longest_ms: 0.0,
            last_turn: Instant::now(),
        }
    }

    pub fn with_wall_ms(mut self, wall_ms: f64) -> Self {
        if wall_ms.is_finite() && wall_ms > 0.0 {
            self.wall_ms = wall_ms;
        }
        self
    }

    pub fn pending_ms(&self) -> f64 {
        self.accumulated_ms
    }

    pub fn report(&self) -> Report {
        Report {
            work_ms: self.accumulated_ms,
            longest_ms: self.longest_ms,
        }
    }

    pub fn add(&mut self, slice_ms: f64) {
        if slice_ms.is_finite() && slice_ms >= 0.0 {
            self.accumulated_ms += slice_ms;
            self.longest_ms = self.longest_ms.max(slice_ms);
        }
    }

    pub fn due(&self) -> bool {
        self.accumulated_ms >= self.threshold_ms
            || self.last_turn.elapsed().as_secs_f64() * 1000.0 >= self.wall_ms
    }

    pub fn take(&mut self) -> Report {
        let report = self.report();
        self.reset_at(Instant::now());
        report
    }

    pub fn reset_at(&mut self, now: Instant) {
        self.accumulated_ms = 0.0;
        self.longest_ms = 0.0;
        self.last_turn = now;
    }
}

pub fn turn_reporting(report: Report) {
    WallClock::new().turn(report);
}

pub fn turn() {
    turn_reporting(Report::default());
}

pub fn turn_now(state: &mut TurnState) {
    turn_reporting(state.take());
}

pub fn turn_soon(state: &mut TurnState, slice_ms: f64) -> bool {
    state.add(slice_ms);
    if !state.due() {
        return false;
    }
    turn_now(state);
    true
}
