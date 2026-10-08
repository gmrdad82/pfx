use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::trace::{self, Source, Sources};
use super::{Frozen, TURN_WALL_MS, gpu_ms, turn_args, unfrozen_ms};
use crate::PassTiming;

pub const CAP_FPS: f64 = 60.0;
pub const TURN_AFTER_MS: f64 = 50.0;
pub const UNCAPPED_VAR: &str = "PFX_UNCAPPED";
pub const TURN_WALL_VAR: &str = "PFX_TURN_WALL_MS";
pub const TURN_MS_VAR: &str = "PFX_TURN_MS";
pub const TURN_VAR: &str = "GPU_QUEUE_TURN";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnReport {
    pub work_ms: f64,
    pub longest_ms: f64,
}

impl TurnReport {
    pub fn args(&self) -> [String; 5] {
        turn_args(self.work_ms, self.longest_ms)
    }
}

pub trait FrameClock {
    fn now_ms(&mut self) -> f64;
    fn sleep_ms(&mut self, ms: f64);
    fn turn(&mut self, report: TurnReport);
    fn frozen_ms(&mut self) -> f64 {
        0.0
    }
}

pub fn frozen_file_ms(path: Option<&std::path::Path>) -> f64 {
    match path {
        Some(path) => Frozen::at(path).ms(),
        None => 0.0,
    }
}

pub trait FrameGpu {
    fn gpu_ms(&self) -> Option<f64>;
}

impl FrameGpu for () {
    fn gpu_ms(&self) -> Option<f64> {
        None
    }
}

impl FrameGpu for Vec<PassTiming> {
    fn gpu_ms(&self) -> Option<f64> {
        gpu_ms(self)
    }
}

impl FrameGpu for [PassTiming] {
    fn gpu_ms(&self) -> Option<f64> {
        gpu_ms(self)
    }
}

pub trait Sink {
    fn progress(&mut self, done: u64, total: u64, fps: f64);
    fn end(&mut self, ok: bool);
}

pub struct Tally {
    sink: Option<Box<dyn Sink>>,
    planned: u64,
    done: u64,
    started_ms: f64,
}

impl Tally {
    pub fn none() -> Self {
        Self {
            sink: None,
            planned: 0,
            done: 0,
            started_ms: 0.0,
        }
    }

    pub fn with_sink(sink: Box<dyn Sink>, planned: u64, now_ms: f64) -> Self {
        Self {
            sink: Some(sink),
            planned,
            done: 0,
            started_ms: now_ms,
        }
    }

    pub fn frame(&mut self) {
        self.done += 1;
    }

    pub fn report(&mut self, now_ms: f64) {
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        let elapsed_ms = now_ms - self.started_ms;
        let fps = if elapsed_ms > 0.0 {
            self.done as f64 * 1000.0 / elapsed_ms
        } else {
            0.0
        };
        sink.progress(self.done, self.planned, fps);
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        if let Some(sink) = self.sink.as_mut() {
            sink.end(!std::thread::panicking());
        }
    }
}

pub struct WallClock {
    start: Instant,
    frozen: Option<PathBuf>,
}

impl WallClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            frozen: std::env::var_os(super::FROZEN_VAR)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        }
    }
}

impl Default for WallClock {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameClock for WallClock {
    fn now_ms(&mut self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn sleep_ms(&mut self, ms: f64) {
        std::thread::sleep(Duration::from_secs_f64(ms / 1000.0));
    }

    fn turn(&mut self, report: TurnReport) {
        if std::env::var_os(TURN_VAR).is_some() {
            let _ = std::process::Command::new("pgpu")
                .args(report.args())
                .stdout(std::process::Stdio::null())
                .status();
        }
    }

    fn frozen_ms(&mut self) -> f64 {
        frozen_file_ms(self.frozen.as_deref())
    }
}

pub fn sleep_to(boundary_ms: f64, now_ms: f64) -> f64 {
    (boundary_ms - now_ms).max(0.0)
}

pub fn turn_wall_ms(get: &impl Fn(&str) -> Option<String>) -> f64 {
    get(TURN_WALL_VAR)
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(TURN_WALL_MS)
}

pub fn uncapped(get: &impl Fn(&str) -> Option<String>) -> bool {
    get(UNCAPPED_VAR).is_some_and(|value| value == "1")
}

pub struct Pace<C: FrameClock> {
    pub clock: C,
    wall_ms: f64,
    tally: Tally,
    period_ms: Option<f64>,
    boundary_ms: f64,
    pending_ms: f64,
    longest_ms: f64,
    last_turn_ms: f64,
    mark_ms: f64,
    mark_frozen_ms: f64,
    frames: u64,
    sources: Sources,
}

impl Pace<WallClock> {
    pub fn new() -> Self {
        let get = |key: &str| std::env::var(key).ok();
        let cap = if uncapped(&get) { None } else { Some(CAP_FPS) };
        let mut pace = Self::with_clock(WallClock::new(), cap);
        pace.wall_ms = turn_wall_ms(&get);
        pace
    }
}

impl Default for Pace<WallClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: FrameClock> Pace<C> {
    pub fn with_clock(mut clock: C, cap_fps: Option<f64>) -> Self {
        trace::arm();
        let now = clock.now_ms();
        let frozen = clock.frozen_ms();
        Self {
            clock,
            wall_ms: TURN_WALL_MS,
            tally: Tally::none(),
            period_ms: cap_fps
                .filter(|fps| fps.is_finite() && *fps > 0.0)
                .map(|fps| 1000.0 / fps),
            boundary_ms: now,
            pending_ms: 0.0,
            longest_ms: 0.0,
            last_turn_ms: now,
            mark_ms: now,
            mark_frozen_ms: frozen,
            frames: 0,
            sources: Sources::default(),
        }
    }

    pub fn with_tally(mut self, tally: Tally) -> Self {
        self.tally = tally;
        self
    }

    pub fn recording(mut self, sink: Box<dyn Sink>, planned: u64) -> Self {
        let now = self.clock.now_ms();
        self.tally = Tally::with_sink(sink, planned, now);
        self
    }

    #[track_caller]
    pub fn frame<R: FrameGpu>(&mut self, work: impl FnOnce() -> R) -> R {
        let frozen_before = self.clock.frozen_ms();
        let before = self.clock.now_ms();
        let result = work();
        let after = self.clock.now_ms();
        let work_ms = unfrozen_ms(after - before, frozen_before, self.clock.frozen_ms());
        self.after_gpu(work_ms, result.gpu_ms());
        result
    }

    #[track_caller]
    pub fn after(&mut self, frame_ms: f64) {
        self.after_gpu(frame_ms, None);
    }

    #[track_caller]
    pub fn after_gpu(&mut self, frame_ms: f64, gpu_ms: Option<f64>) {
        let site = std::panic::Location::caller();
        let since_mark = unfrozen_ms(
            self.clock.now_ms() - self.mark_ms,
            self.mark_frozen_ms,
            self.clock.frozen_ms(),
        );
        let first = self.frames == 0;
        self.frames += 1;
        let gpu_ms = gpu_ms.filter(|ms| ms.is_finite() && *ms > 0.0);
        let work_ms = match gpu_ms {
            Some(ms) => ms,
            None if first => 0.0,
            None => frame_ms.min(since_mark),
        };
        self.sources.add(match gpu_ms {
            Some(_) => Source::Gpu,
            None if first => Source::Untimed,
            None => Source::Wall,
        });
        if work_ms.is_finite() && work_ms > 0.0 {
            self.pending_ms += work_ms;
            self.longest_ms = self.longest_ms.max(work_ms);
        }
        self.tally.frame();
        let now = self.clock.now_ms();
        if self.pending_ms >= TURN_AFTER_MS || now - self.last_turn_ms >= self.wall_ms {
            let report = TurnReport {
                work_ms: self.pending_ms,
                longest_ms: self.longest_ms,
            };
            trace::record(
                "pace",
                site,
                report.work_ms,
                report.longest_ms,
                self.sources,
            );
            self.sources = Sources::default();
            self.pending_ms = 0.0;
            self.longest_ms = 0.0;
            self.clock.turn(report);
            trace::turned();
            self.last_turn_ms = self.clock.now_ms();
            self.tally.report(self.last_turn_ms);
        }
        if let Some(period) = self.period_ms {
            let now = self.clock.now_ms();
            let boundary = self.boundary_ms + period;
            let wait = sleep_to(boundary, now);
            if wait > 0.0 {
                self.clock.sleep_ms(wait);
                self.boundary_ms = boundary;
            } else {
                self.boundary_ms = now;
            }
        }
        self.mark_ms = self.clock.now_ms();
        self.mark_frozen_ms = self.clock.frozen_ms();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frames {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub fps: f64,
}

impl Frames {
    pub fn of(sorted_ms: &[f64]) -> Self {
        let at =
            |fraction: f64| sorted_ms[((sorted_ms.len() - 1) as f64 * fraction).round() as usize];
        let p50 = at(0.50);
        Self {
            p50,
            p95: at(0.95),
            p99: at(0.99),
            max: sorted_ms[sorted_ms.len() - 1],
            fps: 1000.0 / p50,
        }
    }
}

#[cfg(test)]
mod tests;
