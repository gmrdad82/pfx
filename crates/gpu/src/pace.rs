use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use crate::wgpu;

mod frames;
pub mod trace;

pub use frames::{
    CAP_FPS, FrameClock, FrameGpu, Frames, Pace, Sink, TURN_AFTER_MS, TURN_MS_VAR, TURN_VAR,
    TURN_WALL_VAR, Tally, TurnReport, UNCAPPED_VAR, WallClock, frozen_file_ms, sleep_to,
    turn_wall_ms, uncapped,
};
pub use trace::TRACE_VAR;
use trace::{Source, Sources};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Work {
    Bands {
        width: u32,
        height: u32,
    },
    Tiles {
        width: u32,
        height: u32,
        tile_width: u32,
    },
    Dispatches {
        count: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slice {
    Pixels {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Dispatches {
        start: u32,
        count: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    Gpu,
    Cpu,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimingSource {
    #[default]
    Unmeasured,
    Gpu,
    Cpu,
    Mixed,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub milliseconds: Vec<f64>,
    pub timings: Vec<Timing>,
    pub wall_ms: f64,
}

impl Stats {
    pub fn count(&self) -> usize {
        self.milliseconds.len()
    }

    pub fn longest_ms(&self) -> f64 {
        self.milliseconds.iter().copied().fold(0.0, f64::max)
    }

    pub fn median_ms(&self) -> f64 {
        let mut values = self.milliseconds.clone();
        values.sort_by(f64::total_cmp);
        if values.is_empty() {
            return 0.0;
        }
        let middle = values.len() / 2;
        if values.len().is_multiple_of(2) {
            (values[middle - 1] + values[middle]) / 2.0
        } else {
            values[middle]
        }
    }

    pub fn percentile_ms(&self, percent: f64) -> f64 {
        let mut values = self.milliseconds.clone();
        values.sort_by(f64::total_cmp);
        if values.is_empty() {
            return 0.0;
        }
        let rank = (percent.clamp(0.0, 100.0) / 100.0 * values.len() as f64).ceil() as usize;
        values[rank.clamp(1, values.len()) - 1]
    }

    pub fn source(&self) -> TimingSource {
        let gpu = self.timings.contains(&Timing::Gpu);
        let cpu = self.timings.contains(&Timing::Cpu);
        match (gpu, cpu) {
            (false, false) => TimingSource::Unmeasured,
            (true, false) => TimingSource::Gpu,
            (false, true) => TimingSource::Cpu,
            (true, true) => TimingSource::Mixed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pacing {
    pub work_ms: f64,
    pub longest_ms: f64,
    pub slices: usize,
    pub untimed: usize,
    pub source: TimingSource,
}

impl Pacing {
    fn add(&mut self, milliseconds: f64, timing: Timing, reported: bool) {
        if !reported {
            self.untimed += 1;
        } else if milliseconds.is_finite() && milliseconds >= 0.0 {
            self.work_ms += milliseconds;
            self.longest_ms = self.longest_ms.max(milliseconds);
        }
        self.slices += 1;
        self.source = match (self.source, timing) {
            (TimingSource::Unmeasured, Timing::Gpu) | (TimingSource::Gpu, Timing::Gpu) => {
                TimingSource::Gpu
            }
            (TimingSource::Unmeasured, Timing::Cpu) | (TimingSource::Cpu, Timing::Cpu) => {
                TimingSource::Cpu
            }
            _ => TimingSource::Mixed,
        };
    }
}

pub trait Turned {
    fn turned(self) -> bool;
}

impl Turned for () {
    fn turned(self) -> bool {
        false
    }
}

impl Turned for bool {
    fn turned(self) -> bool {
        self
    }
}

pub fn turn_args(work_ms: f64, longest_ms: f64) -> [String; 5] {
    [
        "turn".into(),
        "--work-ms".into(),
        format!("{work_ms:.3}"),
        "--longest-ms".into(),
        format!("{longest_ms:.3}"),
    ]
}

pub const FROZEN_VAR: &str = "GPU_QUEUE_FROZEN";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frozen {
    path: Option<PathBuf>,
}

impl Frozen {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var_os(FROZEN_VAR)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        }
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    pub fn none() -> Self {
        Self::default()
    }

    pub fn ms(&self) -> f64 {
        self.path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| text.trim().parse::<f64>().ok())
            .filter(|ms| ms.is_finite() && *ms >= 0.0)
            .unwrap_or(0.0)
    }
}

pub fn unfrozen_ms(wall_ms: f64, frozen_before_ms: f64, frozen_after_ms: f64) -> f64 {
    let frozen = (frozen_after_ms - frozen_before_ms).max(0.0);
    (wall_ms - frozen).max(0.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    pub at: Instant,
    pub frozen_ms: f64,
}

impl Moment {
    pub fn now(frozen: &Frozen) -> Self {
        Self {
            at: Instant::now(),
            frozen_ms: frozen.ms(),
        }
    }

    pub fn ms_since(&self, earlier: Moment) -> f64 {
        unfrozen_ms(
            self.at.saturating_duration_since(earlier.at).as_secs_f64() * 1000.0,
            earlier.frozen_ms,
            self.frozen_ms,
        )
    }

    fn later(self, other: Moment) -> Moment {
        if other.at > self.at { other } else { self }
    }
}

pub fn gpu_ms(passes: &[crate::PassTiming]) -> Option<f64> {
    let total: f64 = passes.iter().map(|pass| pass.milliseconds).sum();
    (!passes.is_empty() && total.is_finite() && total > 0.0).then_some(total)
}

impl crate::FrameTimings {
    pub fn gpu_ms(&self) -> Option<f64> {
        gpu_ms(&self.passes)
    }
}

static TURNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
static TURNS_TAKEN: AtomicU64 = AtomicU64::new(0);
static LAST_TURN: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

fn last_turn_or(now: Instant) -> Instant {
    LAST_TURN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .unwrap_or(now)
}

pub fn turns_taken() -> u64 {
    TURNS_TAKEN.load(Ordering::SeqCst)
}

pub const TURN_WALL_MS: f64 = 400.0;

fn env_ms(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

pub struct Turns {
    queued: bool,
    threshold_ms: f64,
    wall_ms: f64,
    work_ms: f64,
    longest_ms: f64,
    measured: bool,
    untimed: bool,
    last_turn: Instant,
    last_event: Instant,
    interval_ms: f64,
    taken: u32,
    frozen: Frozen,
    site: &'static std::panic::Location<'static>,
    sources: Sources,
}

impl Default for Turns {
    #[track_caller]
    fn default() -> Self {
        let queued =
            std::env::var_os(TURN_VAR).is_some() || std::env::var_os("GPU_QUEUE_PID").is_some();
        Self::new(env_ms(TURN_MS_VAR, 50.0), queued)
            .with_wall_ms(env_ms(TURN_WALL_VAR, TURN_WALL_MS))
    }
}

impl Turns {
    #[track_caller]
    pub fn new(threshold_ms: f64, queued: bool) -> Self {
        trace::arm();
        trace::created(std::panic::Location::caller());
        Self {
            queued,
            threshold_ms: if threshold_ms.is_finite() {
                threshold_ms.max(1.0)
            } else {
                50.0
            },
            wall_ms: TURN_WALL_MS,
            work_ms: 0.0,
            longest_ms: 0.0,
            measured: false,
            untimed: false,
            last_turn: last_turn_or(Instant::now()),
            last_event: Instant::now(),
            interval_ms: 0.0,
            taken: 0,
            frozen: Frozen::from_env(),
            site: std::panic::Location::caller(),
            sources: Sources::default(),
        }
    }

    pub fn with_frozen(mut self, frozen: Frozen) -> Self {
        self.frozen = frozen;
        self
    }

    pub fn with_wall_ms(mut self, wall_ms: f64) -> Self {
        if wall_ms.is_finite() && wall_ms > 0.0 {
            self.wall_ms = wall_ms;
        }
        self
    }

    #[track_caller]
    pub fn add(&mut self, submission_ms: f64) -> bool {
        self.site = std::panic::Location::caller();
        let paced = trace::take_slice();
        let source = match paced {
            _ if !(submission_ms.is_finite() && submission_ms >= 0.0) => Source::Untimed,
            Some(Timing::Gpu) => Source::Gpu,
            Some(Timing::Cpu) => Source::Wall,
            None => Source::Given,
        };
        self.add_sourced(submission_ms, Instant::now(), source)
    }

    #[cfg(test)]
    fn add_at(&mut self, submission_ms: f64, now: Instant) -> bool {
        self.add_sourced(submission_ms, now, Source::Given)
    }

    fn noted(&mut self, now: Instant) {
        self.interval_ms = (now.saturating_duration_since(self.last_event).as_secs_f64() * 1000.0)
            .min(self.wall_ms / 2.0);
        self.last_event = now;
    }

    fn add_sourced(&mut self, submission_ms: f64, now: Instant, source: Source) -> bool {
        self.noted(now);
        self.sources.add(source);
        if submission_ms.is_finite() && submission_ms >= 0.0 {
            self.work_ms += submission_ms;
            self.longest_ms = self.longest_ms.max(submission_ms);
            self.measured = true;
        } else {
            self.untimed = true;
        }
        if !self.due_at(now) {
            return false;
        }
        self.turn_at(now);
        true
    }

    fn due_at(&self, now: Instant) -> bool {
        self.work_ms >= self.threshold_ms
            || now.saturating_duration_since(self.last_turn).as_secs_f64() * 1000.0
                + self.interval_ms
                >= self.wall_ms
    }

    #[track_caller]
    pub fn time<R>(&mut self, submission: impl FnOnce() -> R) -> R {
        self.site = std::panic::Location::caller();
        let began = Moment::now(&self.frozen);
        let turned = turns_taken();
        let result = submission();
        let ended = Moment::now(&self.frozen);
        self.add_timed(began, ended, turns_taken() != turned);
        result
    }

    fn add_timed(&mut self, began: Moment, ended: Moment, turned_inside: bool) -> bool {
        let milliseconds = if turned_inside {
            f64::NAN
        } else {
            ended.ms_since(began)
        };
        let source = if turned_inside {
            Source::Untimed
        } else {
            Source::Wall
        };
        self.add_sourced(milliseconds, ended.at, source)
    }

    #[track_caller]
    pub fn cpu<R>(&mut self, work: impl FnOnce() -> R) -> R {
        self.poll();
        let result = work();
        self.poll();
        result
    }

    #[track_caller]
    pub fn poll(&mut self) -> bool {
        self.site = std::panic::Location::caller();
        let now = Instant::now();
        self.noted(now);
        if !self.due_at(now) {
            return false;
        }
        self.turn_at(now);
        true
    }

    #[track_caller]
    pub fn turn(&mut self) {
        self.site = std::panic::Location::caller();
        self.turn_at(Instant::now());
    }

    pub fn args(&self) -> Vec<String> {
        if self.untimed && !self.measured {
            vec!["turn".into()]
        } else {
            turn_args(self.work_ms, self.longest_ms).to_vec()
        }
    }

    fn turn_at(&mut self, now: Instant) {
        let args = self.args();
        trace::record(
            "turns",
            self.site,
            self.work_ms,
            self.longest_ms,
            self.sources,
        );
        self.sources = Sources::default();
        self.work_ms = 0.0;
        self.longest_ms = 0.0;
        self.measured = false;
        self.untimed = false;
        self.last_turn = now;
        self.last_event = now;
        self.taken += 1;
        TURNS_TAKEN.fetch_add(1, Ordering::SeqCst);
        if self.queued {
            let _held = TURNING
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = std::process::Command::new("pgpu")
                .args(args)
                .stdout(std::process::Stdio::null())
                .status();
        }
        trace::turned();
        *LAST_TURN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
    }

    pub fn pending(&self) -> (f64, f64) {
        (self.work_ms, self.longest_ms)
    }

    pub fn taken(&self) -> u32 {
        self.taken
    }
}

impl Drop for Turns {
    fn drop(&mut self) {
        trace::record(
            "end",
            self.site,
            self.work_ms,
            self.longest_ms,
            self.sources,
        );
    }
}

const DEFAULT_CEILING_MS: f64 = 6.0;
const CUT_SAFETY: f64 = 0.8;
const RECOVERY_SLICES: u32 = 6;
const RECOVERY_GROWTH: f64 = 1.25;
const DEAD_BAND: f64 = 0.15;
const WINDOW: usize = 8;
const DISTINCT: f64 = 0.2;
const REGIME: f64 = 1.5;
const OVERHEAD_FLOOR: f64 = 0.05;
const PER_UNIT_FLOOR: f64 = 0.02;
const UNKNOWN_OVERHEAD: f64 = 0.3;

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

#[derive(Clone, Copy, Debug)]
struct Affine {
    overhead_ms: f64,
    per_unit_ms: f64,
}

impl Affine {
    fn unknown_overhead(weight: f64, elapsed_ms: f64) -> Self {
        Self {
            overhead_ms: elapsed_ms * UNKNOWN_OVERHEAD,
            per_unit_ms: elapsed_ms * (1.0 - UNKNOWN_OVERHEAD) / weight,
        }
    }

    fn at(&self, units: f64) -> f64 {
        self.overhead_ms + self.per_unit_ms * units
    }

    fn units_for(&self, target_ms: f64) -> f64 {
        ((target_ms - self.overhead_ms) / self.per_unit_ms).max(1.0)
    }
}

#[derive(Clone, Copy, Debug)]
struct History {
    points: [(f64, f64); WINDOW],
    len: usize,
    at: usize,
}

impl History {
    fn new() -> Self {
        Self {
            points: [(0.0, 0.0); WINDOW],
            len: 0,
            at: 0,
        }
    }

    fn points(&self) -> &[(f64, f64)] {
        &self.points[..self.len]
    }

    fn push(&mut self, weight: f64, elapsed_ms: f64) {
        self.points[self.at] = (weight, elapsed_ms);
        self.at = (self.at + 1) % WINDOW;
        self.len = (self.len + 1).min(WINDOW);
    }

    fn restart(&mut self, weight: f64, elapsed_ms: f64) {
        *self = Self::new();
        self.push(weight, elapsed_ms);
    }

    fn fit(&self) -> Option<Affine> {
        let points = self.points();
        let mut slopes = Vec::with_capacity(WINDOW * (WINDOW - 1) / 2);
        for (index, &(first_weight, first_ms)) in points.iter().enumerate() {
            for &(second_weight, second_ms) in &points[index + 1..] {
                let widest = first_weight.max(second_weight);
                if (first_weight - second_weight).abs() >= DISTINCT * widest {
                    slopes.push((second_ms - first_ms) / (second_weight - first_weight));
                }
            }
        }
        if slopes.is_empty() {
            return None;
        }
        let steepness = median(&mut slopes);
        let mut weights: Vec<f64> = points.iter().map(|point| point.0).collect();
        let mut times: Vec<f64> = points.iter().map(|point| point.1).collect();
        let fastest = times.iter().copied().fold(f64::MAX, f64::min);
        let floor = PER_UNIT_FLOOR * median(&mut times) / median(&mut weights);
        let per_unit_ms = steepness.max(floor);
        let mut offsets: Vec<f64> = points
            .iter()
            .map(|&(weight, ms)| ms - per_unit_ms * weight)
            .collect();
        Some(Affine {
            overhead_ms: median(&mut offsets).clamp(0.0, fastest),
            per_unit_ms,
        })
    }

    fn expected_ms(&self, weight: f64) -> Option<f64> {
        if let Some(model) = self.fit() {
            return Some(model.at(weight));
        }
        let mut weights: Vec<f64> = self.points().iter().map(|point| point.0).collect();
        if weights.is_empty() {
            return None;
        }
        let usual = median(&mut weights);
        if (usual - weight).abs() >= DISTINCT * usual.max(weight) {
            return None;
        }
        let mut times: Vec<f64> = self.points().iter().map(|point| point.1).collect();
        Some(median(&mut times))
    }

    fn record(&mut self, weight: f64, elapsed_ms: f64, over_ceiling: bool) {
        let shifted = self.expected_ms(weight).is_some_and(|expected| {
            elapsed_ms > expected * REGIME || elapsed_ms * REGIME < expected
        });
        if over_ceiling || shifted {
            self.restart(weight, elapsed_ms);
        } else {
            self.push(weight, elapsed_ms);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Tuning {
    units: u32,
    recovering: u32,
    history: History,
}

#[derive(Clone, Copy, Debug)]
struct Limits {
    target_ms: f64,
    ceiling_ms: f64,
    step: u32,
}

fn on_the_grid(next: u32, used: u32, step: u32, nearest: bool) -> u32 {
    if step <= 1 || next == used {
        return next;
    }
    if next < used || !nearest {
        (next / step * step).max(step)
    } else {
        ((next + step / 2) / step * step).max(step)
    }
}

impl Tuning {
    fn new(units: u32) -> Self {
        Self {
            units,
            recovering: 0,
            history: History::new(),
        }
    }

    fn learn(&mut self, used: u32, weight: f64, elapsed_ms: f64, available: u32, limits: Limits) {
        let available = available.max(1);
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 || weight <= 0.0 {
            self.units = self.units.clamp(1, available);
            return;
        }
        let over_ceiling = elapsed_ms > limits.ceiling_ms;
        self.history.record(weight, elapsed_ms, over_ceiling);
        let fit = self.history.fit();
        let ideal = match fit {
            Some(model) if model.overhead_ms >= OVERHEAD_FLOOR * limits.target_ms => {
                model.units_for(limits.target_ms)
            }
            _ => weight * limits.target_ms / elapsed_ms,
        };
        let next = if over_ceiling {
            self.recovering = RECOVERY_SLICES;
            let next = ((ideal * CUT_SAFETY).floor() as u32)
                .max(1)
                .min(self.units)
                .min(used);
            on_the_grid(next, used, limits.step, false)
        } else {
            let allow_growth = weight * 4.0 >= used as f64;
            let mut next = adapt(used, ideal, allow_growth);
            let mut nearest = true;
            if self.recovering > 0 {
                self.recovering -= 1;
                nearest = false;
                let cap = (self.units as f64 * RECOVERY_GROWTH).ceil() as u32;
                next = next.min(cap.max(self.units));
            }
            let next = on_the_grid(next, used, limits.step, nearest);
            let model = fit.unwrap_or_else(|| Affine::unknown_overhead(weight, elapsed_ms));
            let further = used.saturating_add(limits.step);
            let coarse = limits.step as f64 >= DEAD_BAND * used as f64;
            if limits.step > 1
                && next <= used
                && allow_growth
                && coarse
                && ideal > used as f64
                && model.at(further as f64) <= limits.ceiling_ms
            {
                further
            } else {
                next
            }
        };
        self.units = next.clamp(1, available);
    }
}

pub struct Pacer {
    target_ms: f64,
    ceiling_ms: f64,
    learned: HashMap<String, Tuning>,
    fixed_units: Option<u32>,
    max_units: Option<u32>,
    step: (u32, u32),
    pacing: Pacing,
}

impl Default for Pacer {
    fn default() -> Self {
        Self::new(3.5)
    }
}

impl Pacer {
    pub fn new(target_ms: f64) -> Self {
        let target_ms = if target_ms.is_finite() {
            target_ms.clamp(2.0, 6.0)
        } else {
            3.5
        };
        Self {
            target_ms,
            ceiling_ms: DEFAULT_CEILING_MS.max(target_ms),
            learned: HashMap::new(),
            fixed_units: None,
            max_units: None,
            step: (1, 1),
            pacing: Pacing::default(),
        }
    }

    pub fn set_step(&mut self, rows: u32, columns: u32) {
        self.step = (rows.max(1), columns.max(1));
    }

    pub fn step(&self) -> (u32, u32) {
        self.step
    }

    pub fn set_fixed_units(&mut self, units: Option<u32>) {
        self.fixed_units = units.map(|value| value.max(1));
    }

    pub fn fixed_units(&self) -> Option<u32> {
        self.fixed_units
    }

    pub fn set_max_units(&mut self, units: Option<u32>) {
        self.max_units = units.map(|value| value.max(1));
    }

    pub fn pacing(&self) -> Pacing {
        self.pacing
    }

    pub fn take_pacing(&mut self) -> Pacing {
        std::mem::take(&mut self.pacing)
    }

    pub fn set_ceiling_ms(&mut self, ceiling_ms: f64) {
        if ceiling_ms.is_finite() {
            self.ceiling_ms = ceiling_ms.clamp(self.target_ms, 100.0);
        }
    }

    pub fn ceiling_ms(&self) -> f64 {
        self.ceiling_ms
    }

    pub fn learned_units(&self, label: &str) -> Option<u32> {
        self.learned.get(label).map(|tuning| tuning.units)
    }

    pub fn target_ms(&self) -> f64 {
        self.target_ms
    }

    pub fn teach(&mut self, label: &str, units: u32) {
        self.learned
            .insert(label.to_owned(), Tuning::new(units.max(1)));
    }

    pub fn run<T: Turned>(
        &mut self,
        label: &str,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        work: Work,
        encode: impl FnMut(&mut wgpu::CommandEncoder, Slice),
        between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        let timestamps = device
            .features()
            .contains(
                wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
            )
            .then(|| Timestamp::new(device, queue));
        let mut lane = GpuLane {
            label,
            device,
            queue,
            encode,
            frozen: if timestamps.is_some() {
                Frozen::none()
            } else {
                Frozen::from_env()
            },
            timestamps,
            last_done: None,
        };
        self.drive(label, &mut lane, work, between)
    }

    pub fn run_blocking<T: Turned>(
        &mut self,
        label: &str,
        work: Work,
        mut submit: impl FnMut(Slice) -> Result<(), String>,
        between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        self.run_measured(label, work, |slice| submit(slice).map(|_| None), between)
    }

    pub fn run_measured<T: Turned>(
        &mut self,
        label: &str,
        work: Work,
        submit: impl FnMut(Slice) -> Result<Option<f64>, String>,
        between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        let mut lane = MeasuredLane {
            submit,
            frozen: Frozen::from_env(),
        };
        self.drive(label, &mut lane, work, between)
    }

    fn drive<L: Lane, T: Turned>(
        &mut self,
        label: &str,
        lane: &mut L,
        work: Work,
        mut between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        let mut cursor = Cursor::stepped(work, self.step.0, self.step.1)?;
        let available = cursor.available();
        let limits = Limits {
            target_ms: self.target_ms,
            ceiling_ms: self.ceiling_ms,
            step: cursor.step_rows,
        };
        let mut tuning = self
            .learned
            .get(label)
            .copied()
            .unwrap_or_else(|| Tuning::new(1));
        let fixed = self.fixed_units;
        let mut stats = Stats::default();
        let start = Instant::now();
        let mut pending: Option<Pending<L::Ticket>> = None;
        let mut slot = 0;
        loop {
            let units = cursor.grant(
                fixed
                    .unwrap_or(tuning.units)
                    .min(self.max_units.unwrap_or(u32::MAX)),
            );
            let Some(slice) = cursor.next(units) else {
                break;
            };
            let current = Pending {
                ticket: lane.submit(slot, slice),
                slot,
                weight: cursor.weight(slice),
                units,
            };
            if let Some(previous) = pending.replace(current) {
                let reported = self.settle(
                    label,
                    lane,
                    &previous,
                    &mut tuning,
                    available,
                    limits,
                    &mut stats,
                )?;
                if between(reported).turned() {
                    self.pacing = Pacing::default();
                }
                trace::take_slice();
                lane.resumed();
            }
            slot ^= 1;
        }
        if let Some(last) = pending {
            let reported = self.settle(
                label,
                lane,
                &last,
                &mut tuning,
                available,
                limits,
                &mut stats,
            )?;
            if between(reported).turned() {
                self.pacing = Pacing::default();
            }
            trace::take_slice();
            lane.resumed();
        }
        stats.wall_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(stats)
    }

    #[allow(clippy::too_many_arguments)]
    fn settle<L: Lane>(
        &mut self,
        label: &str,
        lane: &mut L,
        done: &Pending<L::Ticket>,
        tuning: &mut Tuning,
        available: u32,
        limits: Limits,
        stats: &mut Stats,
    ) -> Result<f64, String> {
        let measured = lane.finish(done.slot, &done.ticket)?;
        stats.milliseconds.push(measured.milliseconds);
        stats.timings.push(measured.timing);
        self.pacing
            .add(measured.milliseconds, measured.timing, measured.reported);
        trace::slice_timed(measured.timing);
        if self.fixed_units.is_none() {
            tuning.learn(
                done.units,
                done.weight,
                measured.milliseconds,
                available,
                limits,
            );
            self.learned.insert(label.to_owned(), *tuning);
        }
        Ok(if measured.reported {
            measured.milliseconds
        } else {
            f64::NAN
        })
    }
}

struct Measured {
    milliseconds: f64,
    timing: Timing,
    reported: bool,
}

trait Lane {
    type Ticket;

    fn submit(&mut self, slot: usize, slice: Slice) -> Self::Ticket;

    fn finish(&mut self, slot: usize, ticket: &Self::Ticket) -> Result<Measured, String>;

    fn resumed(&mut self) {}
}

struct Pending<T> {
    ticket: T,
    slot: usize,
    weight: f64,
    units: u32,
}

struct GpuLane<'a, F> {
    label: &'a str,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    timestamps: Option<Timestamp>,
    encode: F,
    frozen: Frozen,
    last_done: Option<Moment>,
}

fn fence_ms(submitted: Moment, last_done: Option<Moment>, done: Moment) -> f64 {
    let began = last_done.map_or(submitted, |last| last.later(submitted));
    done.ms_since(began)
}

impl<F: FnMut(&mut wgpu::CommandEncoder, Slice)> Lane for GpuLane<'_, F> {
    type Ticket = (wgpu::SubmissionIndex, Moment);

    fn submit(&mut self, slot: usize, slice: Slice) -> Self::Ticket {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(self.label),
            });
        if let Some(timestamp) = &self.timestamps {
            encoder.write_timestamp(&timestamp.slots[slot].queries, 0);
        }
        (self.encode)(&mut encoder, slice);
        if let Some(timestamp) = &self.timestamps {
            let timing = &timestamp.slots[slot];
            encoder.write_timestamp(&timing.queries, 1);
            encoder.resolve_query_set(&timing.queries, 0..2, &timing.resolve, 0);
            encoder.copy_buffer_to_buffer(&timing.resolve, 0, &timing.read, 0, 16);
        }
        let submitted = Moment::now(&self.frozen);
        (self.queue.submit(Some(encoder.finish())), submitted)
    }

    fn finish(&mut self, slot: usize, ticket: &Self::Ticket) -> Result<Measured, String> {
        let (index, submitted) = ticket;
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index.clone()),
                timeout: None,
            })
            .map_err(|error| format!("{error:?}"))?;
        let done = Moment::now(&self.frozen);
        let first = self.last_done.is_none();
        let cpu_ms = fence_ms(*submitted, self.last_done, done);
        self.last_done = Some(done);
        let gpu_ms = self
            .timestamps
            .as_ref()
            .and_then(|timestamp| timestamp.read_ms(self.device, slot, index.clone()));
        Ok(match gpu_ms {
            Some(milliseconds) => Measured {
                milliseconds,
                timing: Timing::Gpu,
                reported: true,
            },
            None => Measured {
                milliseconds: cpu_ms,
                timing: Timing::Cpu,
                reported: !first,
            },
        })
    }

    fn resumed(&mut self) {
        if self.last_done.is_some() {
            self.last_done = Some(Moment::now(&self.frozen));
        }
    }
}

struct MeasuredLane<F> {
    submit: F,
    frozen: Frozen,
}

impl<F: FnMut(Slice) -> Result<Option<f64>, String>> Lane for MeasuredLane<F> {
    type Ticket = Result<(Option<f64>, f64), String>;

    fn submit(&mut self, _slot: usize, slice: Slice) -> Self::Ticket {
        let began = Moment::now(&self.frozen);
        let gpu_ms = (self.submit)(slice)?;
        let cpu_ms = Moment::now(&self.frozen).ms_since(began);
        Ok((gpu_ms.filter(|ms| ms.is_finite() && *ms >= 0.0), cpu_ms))
    }

    fn finish(&mut self, _slot: usize, ticket: &Self::Ticket) -> Result<Measured, String> {
        let (gpu_ms, cpu_ms) = ticket.clone()?;
        Ok(match gpu_ms {
            Some(milliseconds) => Measured {
                milliseconds,
                timing: Timing::Gpu,
                reported: true,
            },
            None => Measured {
                milliseconds: cpu_ms,
                timing: Timing::Cpu,
                reported: false,
            },
        })
    }
}

fn adapt(units: u32, ideal: f64, allow_growth: bool) -> u32 {
    let ratio = (ideal / units as f64).clamp(0.5, 2.0);
    if ratio > 1.15 {
        if allow_growth {
            ((units as f64 * ratio).ceil() as u32).clamp(units, units.saturating_mul(2))
        } else {
            units
        }
    } else if ratio < 0.85 {
        ((units as f64 * ratio).floor() as u32).clamp((units / 2).max(1), units)
    } else {
        units
    }
}

struct Cursor {
    work: Work,
    x: u32,
    y: u32,
    band_height: u32,
    step_rows: u32,
}

impl Cursor {
    #[cfg(test)]
    fn new(work: Work) -> Result<Self, String> {
        Self::stepped(work, 1, 1)
    }

    fn stepped(work: Work, rows: u32, columns: u32) -> Result<Self, String> {
        let rows = rows.max(1);
        let columns = columns.max(1);
        match work {
            Work::Bands { width, height } | Work::Tiles { width, height, .. }
                if width == 0 || height == 0 =>
            {
                return Err("pixel work needs nonzero width and height".into());
            }
            Work::Tiles { tile_width: 0, .. } => return Err("tile width must be nonzero".into()),
            Work::Dispatches { count: 0 } => return Err("dispatch count must be nonzero".into()),
            _ => {}
        }
        let work = match work {
            Work::Tiles {
                width,
                height,
                tile_width,
            } => Work::Tiles {
                width,
                height,
                tile_width: (tile_width / columns * columns).max(columns),
            },
            other => other,
        };
        Ok(Self {
            work,
            x: 0,
            y: 0,
            band_height: 0,
            step_rows: match work {
                Work::Dispatches { .. } => 1,
                _ => rows,
            },
        })
    }

    fn grant(&self, units: u32) -> u32 {
        match self.work {
            Work::Bands { .. } | Work::Tiles { .. } => {
                (units / self.step_rows * self.step_rows).max(self.step_rows)
            }
            Work::Dispatches { .. } => units.max(1),
        }
    }

    fn available(&self) -> u32 {
        match self.work {
            Work::Bands { height, .. } | Work::Tiles { height, .. } => height,
            Work::Dispatches { count } => count,
        }
    }

    fn weight(&self, slice: Slice) -> f64 {
        match (self.work, slice) {
            (
                Work::Bands { width, .. },
                Slice::Pixels {
                    width: columns,
                    height,
                    ..
                },
            ) => height as f64 * columns as f64 / width as f64,
            (
                Work::Tiles {
                    width, tile_width, ..
                },
                Slice::Pixels {
                    width: columns,
                    height,
                    ..
                },
            ) => height as f64 * columns as f64 / tile_width.min(width) as f64,
            (_, Slice::Pixels { height, .. }) => height as f64,
            (_, Slice::Dispatches { count, .. }) => count as f64,
        }
    }

    fn next(&mut self, units: u32) -> Option<Slice> {
        match self.work {
            Work::Bands { width, height } => {
                if self.y >= height {
                    return None;
                }
                let rows = self.grant(units).min(height - self.y);
                let slice = Slice::Pixels {
                    x: 0,
                    y: self.y,
                    width,
                    height: rows,
                };
                self.y += rows;
                Some(slice)
            }
            Work::Tiles {
                width,
                height,
                tile_width,
            } => {
                if self.y >= height {
                    return None;
                }
                if self.x == 0 {
                    self.band_height = self.grant(units).min(height - self.y);
                }
                let columns = tile_width.min(width - self.x);
                let slice = Slice::Pixels {
                    x: self.x,
                    y: self.y,
                    width: columns,
                    height: self.band_height,
                };
                self.x += columns;
                if self.x == width {
                    self.x = 0;
                    self.y += self.band_height;
                }
                Some(slice)
            }
            Work::Dispatches { count } => {
                if self.y >= count {
                    return None;
                }
                let size = units.max(1).min(count - self.y);
                let slice = Slice::Dispatches {
                    start: self.y,
                    count: size,
                };
                self.y += size;
                Some(slice)
            }
        }
    }
}

struct Timestamp {
    slots: [TimestampSlot; 2],
    period: f64,
}

struct TimestampSlot {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
}

impl Timestamp {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            slots: std::array::from_fn(|_| TimestampSlot::new(device)),
            period: f64::from(queue.get_timestamp_period()),
        }
    }

    fn read_ms(
        &self,
        device: &wgpu::Device,
        slot: usize,
        index: wgpu::SubmissionIndex,
    ) -> Option<f64> {
        let timing = &self.slots[slot];
        let (send, receive) = mpsc::channel();
        timing
            .read
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: None,
            })
            .ok()?;
        receive.recv().ok()?.ok()?;
        let data = timing.read.slice(..).get_mapped_range();
        let first = u64::from_le_bytes(data[0..8].try_into().ok()?);
        let last = u64::from_le_bytes(data[8..16].try_into().ok()?);
        drop(data);
        timing.read.unmap();
        Some(last.saturating_sub(first) as f64 * self.period / 1_000_000.0)
    }
}

pub struct SliceTimer {
    stamp: Timestamp,
}

impl SliceTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        device
            .features()
            .contains(
                wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
            )
            .then(|| Self {
                stamp: Timestamp::new(device, queue),
            })
    }

    pub fn begin(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(&self.stamp.slots[0].queries, 0);
    }

    pub fn end(&self, encoder: &mut wgpu::CommandEncoder) {
        let timing = &self.stamp.slots[0];
        encoder.write_timestamp(&timing.queries, 1);
        encoder.resolve_query_set(&timing.queries, 0..2, &timing.resolve, 0);
        encoder.copy_buffer_to_buffer(&timing.resolve, 0, &timing.read, 0, 16);
    }

    pub fn read_ms(&self, device: &wgpu::Device, index: wgpu::SubmissionIndex) -> Option<f64> {
        self.stamp.read_ms(device, 0, index)
    }
}

impl TimestampSlot {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pacer timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pacer timestamp resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pacer timestamp read"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_and_tiles_cover_every_pixel_in_order() {
        for work in [
            Work::Bands {
                width: 7,
                height: 11,
            },
            Work::Tiles {
                width: 7,
                height: 11,
                tile_width: 3,
            },
        ] {
            let mut cursor = Cursor::new(work).unwrap();
            let mut slices = Vec::new();
            let mut pixels = Vec::new();
            while let Some(
                slice @ Slice::Pixels {
                    x,
                    y,
                    width,
                    height,
                },
            ) = cursor.next(4)
            {
                slices.push(slice);
                for row in y..y + height {
                    for column in x..x + width {
                        pixels.push((column, row));
                    }
                }
            }
            assert_eq!(pixels.len(), 77);
            pixels.sort_unstable();
            let expected: Vec<_> = (0..11).flat_map(|y| (0..7).map(move |x| (x, y))).collect();
            let mut expected_sorted = expected.clone();
            expected_sorted.sort_unstable();
            assert_eq!(pixels, expected_sorted);
            let expected_slices = match work {
                Work::Bands { .. } => vec![
                    Slice::Pixels {
                        x: 0,
                        y: 0,
                        width: 7,
                        height: 4,
                    },
                    Slice::Pixels {
                        x: 0,
                        y: 4,
                        width: 7,
                        height: 4,
                    },
                    Slice::Pixels {
                        x: 0,
                        y: 8,
                        width: 7,
                        height: 3,
                    },
                ],
                Work::Tiles { .. } => [
                    (0, 0, 3, 4),
                    (3, 0, 3, 4),
                    (6, 0, 1, 4),
                    (0, 4, 3, 4),
                    (3, 4, 3, 4),
                    (6, 4, 1, 4),
                    (0, 8, 3, 3),
                    (3, 8, 3, 3),
                    (6, 8, 1, 3),
                ]
                .into_iter()
                .map(|(x, y, width, height)| Slice::Pixels {
                    x,
                    y,
                    width,
                    height,
                })
                .collect(),
                _ => unreachable!(),
            };
            assert_eq!(slices, expected_slices);
        }
    }

    #[test]
    fn dispatch_slices_have_absolute_starts() {
        let mut cursor = Cursor::new(Work::Dispatches { count: 10 }).unwrap();
        assert_eq!(
            cursor.next(4),
            Some(Slice::Dispatches { start: 0, count: 4 })
        );
        assert_eq!(
            cursor.next(4),
            Some(Slice::Dispatches { start: 4, count: 4 })
        );
        assert_eq!(
            cursor.next(4),
            Some(Slice::Dispatches { start: 8, count: 2 })
        );
        assert_eq!(cursor.next(4), None);
    }

    #[test]
    fn adaptation_converges_and_stays_bounded() {
        assert_eq!(Pacer::default().target_ms, 3.5);
        assert_eq!(Pacer::new(1.0).target_ms, 2.0);
        assert_eq!(Pacer::new(9.0).target_ms, 6.0);
        for initial in [1, 256] {
            let mut units = initial;
            for _ in 0..30 {
                let next = adapt(units, 3.5 / 0.1, true);
                assert!(next <= units.saturating_mul(2));
                assert!(next >= (units / 2).max(1));
                units = next;
            }
            assert!((30..=40).contains(&units), "{units}");
        }
    }

    struct Synthetic<C> {
        cost: C,
        timing: Timing,
        submissions: u32,
        units_done: u32,
        trace: Vec<(u32, f64)>,
    }

    impl<C: FnMut(u32, u32, f64) -> f64> Synthetic<C> {
        fn new(timing: Timing, cost: C) -> Self {
            Self {
                cost,
                timing,
                submissions: 0,
                units_done: 0,
                trace: Vec::new(),
            }
        }
    }

    impl<C: FnMut(u32, u32, f64) -> f64> Lane for Synthetic<C> {
        type Ticket = f64;

        fn submit(&mut self, _slot: usize, slice: Slice) -> f64 {
            let weight = match slice {
                Slice::Pixels { width, height, .. } => (width * height) as f64 / 1000.0,
                Slice::Dispatches { count, .. } => count as f64,
            };
            let ms = (self.cost)(self.submissions, self.units_done, weight);
            self.submissions += 1;
            self.units_done += weight as u32;
            self.trace.push((weight as u32, ms));
            ms
        }

        fn finish(&mut self, _slot: usize, ticket: &f64) -> Result<Measured, String> {
            Ok(Measured {
                milliseconds: *ticket,
                timing: self.timing,
                reported: true,
            })
        }
    }

    const OVERHEAD_MS: f64 = 0.02;
    const STEP_NOISE: f64 = 1.25;
    const TOTAL: u32 = 60_000;

    fn stepping(outliers: bool) -> impl FnMut(u32, u32, f64) -> f64 {
        move |submission, done, weight| {
            let per_unit = if (20_000..40_000).contains(&done) {
                0.2
            } else {
                0.05
            };
            let spike = if outliers && submission % 40 == 39 {
                3.0
            } else {
                1.0
            };
            (OVERHEAD_MS + weight * per_unit) * spike
        }
    }

    fn unsliced_ms() -> f64 {
        OVERHEAD_MS + 40_000.0 * 0.05 + 20_000.0 * 0.2
    }

    fn legacy(work_units: u32, cost: impl FnMut(u32, u32, f64) -> f64) -> Stats {
        fn old_adapt(units: u32, elapsed_ms: f64, target_ms: f64) -> u32 {
            let ratio = (target_ms / elapsed_ms).clamp(0.5, 2.0);
            if ratio > 1.15 {
                ((units as f64 * ratio).ceil() as u32).clamp(units, units.saturating_mul(2))
            } else if ratio < 0.85 {
                ((units as f64 * ratio).floor() as u32).clamp((units / 2).max(1), units)
            } else {
                units
            }
        }
        let mut lane = Synthetic::new(Timing::Gpu, cost);
        let mut cursor = Cursor::new(Work::Dispatches { count: work_units }).unwrap();
        let mut stats = Stats::default();
        let mut units = 1;
        let mut pending: Option<(f64, u32, u32)> = None;
        while let Some(slice) = cursor.next(units) {
            let observed = match slice {
                Slice::Dispatches { count, .. } => count,
                _ => unreachable!(),
            };
            let current = (lane.submit(0, slice), units, observed);
            if let Some((ms, used, seen)) = pending.replace(current) {
                stats.milliseconds.push(ms);
                if seen == used {
                    units = old_adapt(used, ms, 3.5);
                }
            }
        }
        if let Some((ms, ..)) = pending {
            stats.milliseconds.push(ms);
        }
        stats.wall_ms = stats.milliseconds.iter().sum();
        stats
    }

    fn paced(
        work_units: u32,
        cost: impl FnMut(u32, u32, f64) -> f64,
    ) -> (Stats, Synthetic<impl FnMut(u32, u32, f64) -> f64>) {
        let mut lane = Synthetic::new(Timing::Gpu, cost);
        let mut pacer = Pacer::default();
        let mut stats = pacer
            .drive(
                "t",
                &mut lane,
                Work::Dispatches { count: work_units },
                |_| {},
            )
            .unwrap();
        stats.wall_ms = stats.milliseconds.iter().sum();
        (stats, lane)
    }

    #[test]
    fn truncated_slice_teaches_the_pacer() {
        let limits = Limits {
            target_ms: 3.5,
            ceiling_ms: 6.0,
            step: 1,
        };
        let mut tuning = Tuning::new(12);
        tuning.learn(12, 6.0, 0.6, 1000, limits);
        assert_eq!(tuning.units, 24);
        let mut tuning = Tuning::new(24);
        tuning.learn(24, 12.0, 4.8, 1000, limits);
        assert_eq!(tuning.units, 12);
        let mut tuning = Tuning::new(24);
        tuning.learn(24, 3.0, 0.1, 1000, limits);
        assert_eq!(tuning.units, 24);
    }

    #[test]
    fn learned_units_never_exceed_the_available_work() {
        let limits = Limits {
            target_ms: 3.5,
            ceiling_ms: 6.0,
            step: 1,
        };
        let mut tuning = Tuning::new(12);
        tuning.learn(12, 5.0, 0.5, 5, limits);
        assert_eq!(tuning.units, 5);
        tuning.learn(5, 5.0, f64::NAN, 3, limits);
        assert_eq!(tuning.units, 3);
    }

    #[test]
    fn a_smaller_work_item_reteaches_the_pacer() {
        let mut pacer = Pacer::default();
        let mut big = Synthetic::new(Timing::Gpu, |_, _, weight| weight * 0.05);
        pacer
            .drive("t", &mut big, Work::Dispatches { count: 4000 }, |_| {})
            .unwrap();
        let before = pacer.learned_units("t").unwrap();
        assert!((60..=80).contains(&before), "{before}");
        let mut small = Synthetic::new(Timing::Gpu, |_, _, weight| weight * 0.2);
        pacer
            .drive("t", &mut small, Work::Dispatches { count: 40 }, |_| {})
            .unwrap();
        let after = pacer.learned_units("t").unwrap();
        assert!(after <= 40, "{after}");
        assert!(after < before);
        let mut again = Synthetic::new(Timing::Gpu, |_, _, weight| weight * 0.2);
        pacer
            .drive("t", &mut again, Work::Dispatches { count: 4000 }, |_| {})
            .unwrap();
        let settled = pacer.learned_units("t").unwrap();
        assert!((14..=20).contains(&settled), "{settled}");
    }

    #[test]
    fn tile_weights_discount_the_narrow_edge_tile() {
        let cursor = Cursor::new(Work::Tiles {
            width: 10,
            height: 8,
            tile_width: 4,
        })
        .unwrap();
        let full = Slice::Pixels {
            x: 0,
            y: 0,
            width: 4,
            height: 8,
        };
        let edge = Slice::Pixels {
            x: 8,
            y: 0,
            width: 2,
            height: 8,
        };
        assert_eq!(cursor.weight(full), 8.0);
        assert_eq!(cursor.weight(edge), 4.0);
    }

    #[test]
    fn the_ceiling_cuts_after_an_outlier_then_grows_back_slowly() {
        let mut pacer = Pacer::default();
        let mut lane = Synthetic::new(Timing::Gpu, |submission, _, weight| {
            let spike = if submission == 60 { 3.0 } else { 1.0 };
            weight * 0.05 * spike
        });
        let stats = pacer
            .drive("t", &mut lane, Work::Dispatches { count: 30_000 }, |_| {})
            .unwrap();
        let ceiling = pacer.ceiling_ms();
        let outlier = stats
            .milliseconds
            .iter()
            .position(|ms| *ms > ceiling)
            .expect("the spike exceeds the ceiling");
        assert_eq!(outlier, 60);
        assert!(
            stats.milliseconds[outlier + 2] < ceiling,
            "{:?}",
            &stats.milliseconds[outlier..outlier + 4]
        );
        assert!(stats.milliseconds[outlier + 2] < 3.5);
        for pair in lane.trace[outlier + 2..outlier + 8].windows(2) {
            assert!(pair[1].0 as f64 <= (pair[0].0 as f64 * 1.25).ceil() + 1.0);
        }
        let later = &stats.milliseconds[outlier + 40..];
        assert!(
            later.iter().all(|ms| *ms < ceiling),
            "{:?}",
            later.iter().copied().fold(0.0, f64::max)
        );
        let tail = lane.trace[lane.trace.len() - 10].0;
        assert!((55..=80).contains(&tail), "{tail}");
    }

    #[test]
    fn the_ceiling_is_settable_and_never_below_the_target() {
        let mut pacer = Pacer::new(4.0);
        assert_eq!(pacer.ceiling_ms(), 6.0);
        pacer.set_ceiling_ms(8.0);
        assert_eq!(pacer.ceiling_ms(), 8.0);
        pacer.set_ceiling_ms(1.0);
        assert_eq!(pacer.ceiling_ms(), 4.0);
        pacer.set_ceiling_ms(f64::NAN);
        assert_eq!(pacer.ceiling_ms(), 4.0);
        assert_eq!(Pacer::new(6.0).ceiling_ms(), 6.0);
    }

    #[test]
    fn stats_say_how_every_slice_was_timed() {
        for (timing, source) in [
            (Timing::Gpu, TimingSource::Gpu),
            (Timing::Cpu, TimingSource::Cpu),
        ] {
            let mut lane = Synthetic::new(timing, |_, _, weight| weight * 0.05);
            let stats = Pacer::default()
                .drive("t", &mut lane, Work::Dispatches { count: 500 }, |_| {})
                .unwrap();
            assert_eq!(stats.source(), source);
            assert_eq!(stats.timings.len(), stats.count());
            assert!(stats.timings.iter().all(|seen| *seen == timing));
        }
        let mixed = Stats {
            milliseconds: vec![1.0, 2.0],
            timings: vec![Timing::Gpu, Timing::Cpu],
            wall_ms: 3.0,
        };
        assert_eq!(mixed.source(), TimingSource::Mixed);
        assert_eq!(Stats::default().source(), TimingSource::Unmeasured);
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let stats = Stats {
            milliseconds: (1..=100).map(f64::from).collect(),
            ..Stats::default()
        };
        assert_eq!(stats.percentile_ms(99.0), 99.0);
        assert_eq!(stats.percentile_ms(100.0), 100.0);
        assert_eq!(stats.percentile_ms(0.0), 1.0);
        assert_eq!(Stats::default().percentile_ms(99.0), 0.0);
    }

    #[test]
    fn a_steady_workload_behaves_as_before() {
        let steady = |_: u32, _: u32, weight: f64| weight * 0.1;
        let before = legacy(20_000, steady);
        let (after, _) = paced(20_000, steady);
        assert_eq!(after.count(), before.count());
        for (new, old) in after.milliseconds.iter().zip(&before.milliseconds) {
            assert!((new - old).abs() < 1e-9, "{new} {old}");
        }
        let late = &after.milliseconds[after.count() / 2..after.count() - 1];
        assert!(
            late.iter().all(|ms| (2.8..=4.0).contains(ms)),
            "{:?}",
            late.iter().copied().fold(0.0, f64::max)
        );
    }

    #[test]
    fn fixed_units_skip_learning_and_the_ceiling() {
        let mut pacer = Pacer::default();
        pacer.set_fixed_units(Some(50));
        let mut lane = Synthetic::new(Timing::Gpu, |_, _, weight| weight * 1.0);
        let stats = pacer
            .drive("t", &mut lane, Work::Dispatches { count: 200 }, |_| {})
            .unwrap();
        assert_eq!(stats.count(), 4);
        assert_eq!(pacer.learned_units("t"), None);
    }

    #[test]
    #[ignore = "prints the before and after of the synthetic step and outlier workload"]
    fn synthetic_step_and_outliers() {
        for (name, outliers) in [("steps", false), ("steps+outliers", true)] {
            let before = legacy(TOTAL, stepping(outliers));
            let (after, _) = paced(TOTAL, stepping(outliers));
            for (which, stats) in [("before", &before), ("after", &after)] {
                let calm: Vec<f64> = stats
                    .milliseconds
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !outliers || index % 40 != 39)
                    .map(|(_, ms)| *ms)
                    .collect();
                println!(
                    "{name} {which}: slices {} median {:.2} p99 {:.2} longest {:.2} over6 {} calm_longest {:.2} calm_over6 {} throughput {:.3}",
                    stats.count(),
                    stats.median_ms(),
                    stats.percentile_ms(99.0),
                    stats.longest_ms(),
                    stats.milliseconds.iter().filter(|ms| **ms > 6.0).count(),
                    calm.iter().copied().fold(0.0, f64::max),
                    calm.iter().filter(|ms| **ms > 6.0).count(),
                    unsliced_ms() / stats.wall_ms,
                );
            }
        }
    }

    #[test]
    fn the_step_workload_stays_near_the_target_and_under_the_ceiling() {
        let (after, lane) = paced(TOTAL, stepping(true));
        let before = legacy(TOTAL, stepping(true));
        let calm_over = |stats: &Stats| {
            stats
                .milliseconds
                .iter()
                .enumerate()
                .filter(|(index, ms)| index % 40 != 39 && **ms > 6.0)
                .count()
        };
        assert!(calm_over(&after) <= calm_over(&before));
        assert!(calm_over(&after) <= 2);
        assert!(after.count() < 2 * before.count());
        let over: Vec<usize> = (0..after.count())
            .filter(|index| after.milliseconds[*index] > 6.0)
            .collect();
        for index in &over {
            let sizing = lane.trace.get(index + 2).map(|(_, ms)| *ms);
            if let Some(ms) = sizing {
                let spiked = (index + 2) % 40 == 39;
                assert!(spiked || ms < 6.0, "{index} {ms}");
            }
        }
        assert!(unsliced_ms() / after.wall_ms > 0.7);
    }

    #[test]
    fn pacing_sums_work_and_longest_until_a_turn() {
        let costs = [2.0, 5.0, 3.0, 4.0, 1.5, 6.5];
        let mut lane = Synthetic::new(Timing::Gpu, |submission, _, _| costs[submission as usize]);
        let mut pacer = Pacer::default();
        pacer.set_fixed_units(Some(1));
        let mut seen = Vec::new();
        pacer
            .drive("turns", &mut lane, Work::Dispatches { count: 4 }, |ms| {
                seen.push(ms);
                seen.len() == 3
            })
            .unwrap();
        assert_eq!(seen, [2.0, 5.0, 3.0, 4.0]);
        assert_eq!(
            pacer.pacing(),
            Pacing {
                work_ms: 4.0,
                longest_ms: 4.0,
                slices: 1,
                untimed: 0,
                source: TimingSource::Gpu,
            }
        );
        let mut lane = Synthetic::new(Timing::Cpu, |submission, _, _| {
            costs[4 + submission as usize]
        });
        pacer
            .drive("turns", &mut lane, Work::Dispatches { count: 2 }, |_| {})
            .unwrap();
        assert_eq!(
            pacer.take_pacing(),
            Pacing {
                work_ms: 12.0,
                longest_ms: 6.5,
                slices: 3,
                untimed: 0,
                source: TimingSource::Mixed,
            }
        );
        assert_eq!(pacer.pacing(), Pacing::default());
    }

    #[test]
    fn turns_sum_work_keep_the_longest_and_reset() {
        let mut turns = Turns::new(50.0, false);
        assert!(!turns.add(20.0));
        assert!(!turns.add(f64::NAN));
        assert!(!turns.add(29.5));
        assert_eq!(turns.pending(), (49.5, 29.5));
        assert!(turns.add(4.0));
        assert_eq!(turns.pending(), (0.0, 0.0));
        assert_eq!(turns.taken(), 1);
        assert_eq!(turns.time(|| 7), 7);
        assert!(turns.pending().0 >= 0.0);
        turns.turn();
        assert_eq!(turns.taken(), 2);
        assert_eq!(
            turn_args(53.5, 4.0004),
            ["turn", "--work-ms", "53.500", "--longest-ms", "4.000"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn turns_from_two_threads_never_overlap() {
        if std::env::var_os("PFX_TURN_CHILD").is_some() {
            return;
        }
        let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("pace-turns-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let pgpu = scratch.join("pgpu");
        std::fs::write(
            &pgpu,
            "#!/bin/sh\nmkdir \"$TURN_LOG.held\" 2>/dev/null || echo overlap >> \"$TURN_LOG\"\nsleep 0.1\nprintf '%s\\n' \"$*\" >> \"$TURN_LOG\"\nrmdir \"$TURN_LOG.held\" 2>/dev/null\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&pgpu, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let log = scratch.join("turns");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![scratch.clone()];
        paths.extend(std::env::split_paths(&path));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "pace::tests::turns_child", "--test-threads", "1"])
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("PFX_TURN_CHILD", "1")
            .env("TURN_LOG", &log)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        let lines = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_dir_all(&scratch).unwrap();
        assert!(status.success());
        let mut lines: Vec<_> = lines.lines().collect();
        lines.sort();
        assert_eq!(
            lines,
            [
                "turn --work-ms 1.000 --longest-ms 1.000",
                "turn --work-ms 2.000 --longest-ms 2.000",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn turns_child() {
        if std::env::var_os("PFX_TURN_CHILD").is_none() {
            return;
        }
        let together: Vec<_> = [1.0, 2.0]
            .into_iter()
            .map(|ms| {
                std::thread::spawn(move || {
                    let mut turns = Turns::new(50.0, true);
                    turns.add(ms);
                    turns.turn();
                })
            })
            .collect();
        for thread in together {
            thread.join().unwrap();
        }
    }

    #[test]
    fn the_unit_limit_caps_every_slice() {
        let mut lane = Synthetic::new(Timing::Gpu, |_, _, weight| weight * 0.01);
        let mut pacer = Pacer::default();
        pacer.set_max_units(Some(64));
        let stats = pacer
            .drive("cap", &mut lane, Work::Dispatches { count: 5000 }, |_| {})
            .unwrap();
        assert!(lane.trace.iter().all(|&(units, _)| units <= 64));
        assert_eq!(
            lane.trace.iter().map(|&(units, _)| units).sum::<u32>(),
            5000
        );
        assert!(stats.count() >= 5000 / 64);
        pacer.set_max_units(Some(0));
        let mut lane = Synthetic::new(Timing::Gpu, |_, _, _| 1.0);
        pacer
            .drive("cap", &mut lane, Work::Dispatches { count: 3 }, |_| {})
            .unwrap();
        assert!(lane.trace.iter().all(|&(units, _)| units == 1));
    }

    #[test]
    fn a_blocking_run_covers_the_work_and_passes_errors_on() {
        let mut pacer = Pacer::default();
        let mut slices = Vec::new();
        let stats = pacer
            .run_blocking(
                "blocking",
                Work::Dispatches { count: 9 },
                |slice| {
                    slices.push(slice);
                    Ok(())
                },
                |_| {},
            )
            .unwrap();
        assert_eq!(stats.source(), TimingSource::Cpu);
        assert_eq!(stats.count(), slices.len());
        let mut next = 0;
        for slice in slices {
            let Slice::Dispatches { start, count } = slice else {
                unreachable!()
            };
            assert_eq!(start, next);
            next += count;
        }
        assert_eq!(next, 9);
        assert_eq!(pacer.pacing().slices, stats.count());
        let failed = pacer.run_blocking(
            "blocking",
            Work::Dispatches { count: 9 },
            |_| Err("lost".to_string()),
            |_| {},
        );
        assert_eq!(failed.err().as_deref(), Some("lost"));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn paced_dispatches_cover_everything_and_report_their_timing() {
        const GROUPS: u32 = 600;
        const ROUNDS: u32 = 2000;
        let gpu = pollster::block_on(crate::Gpu::headless()).unwrap();
        let device = &gpu.device;
        let timed = device.features().contains(
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pacer test"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "@group(0) @binding(0) var<storage, read_write> out: array<u32>;
                     @group(0) @binding(1) var<uniform> start: vec4<u32>;
                     @compute @workgroup_size(64)
                     fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
                         let index = start.x * 64u + id.x;
                         var a = index;
                         for (var i = 0u; i < {ROUNDS}u; i = i + 1u) {{
                             a = a * 1664525u + 1013904223u;
                         }}
                         out[index] = a;
                     }}"
                )
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pacer test"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let bytes = u64::from(GROUPS) * 64 * 4;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut pacer = Pacer::default();
        let mut slices = Vec::new();
        let stats = pacer
            .run(
                "gpu test",
                device,
                &gpu.queue,
                Work::Dispatches { count: GROUPS },
                |encoder, slice| {
                    let Slice::Dispatches { start, count } = slice else {
                        unreachable!()
                    };
                    slices.push((start, count));
                    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("start"),
                        size: 16,
                        usage: wgpu::BufferUsages::UNIFORM,
                        mapped_at_creation: true,
                    });
                    uniform.slice(..).get_mapped_range_mut()[0..4]
                        .copy_from_slice(&start.to_le_bytes());
                    uniform.unmap();
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: out.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: uniform.as_entire_binding(),
                            },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(count, 1, 1);
                },
                |_| {},
            )
            .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&out, 0, &read, 0, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = read.slice(..).get_mapped_range();
        for index in 0..GROUPS * 64 {
            let mut a = index;
            for _ in 0..ROUNDS {
                a = a.wrapping_mul(1664525).wrapping_add(1013904223);
            }
            let at = index as usize * 4;
            assert_eq!(u32::from_le_bytes(data[at..at + 4].try_into().unwrap()), a);
        }
        assert_eq!(stats.count(), slices.len());
        assert_eq!(stats.timings.len(), stats.count());
        assert_eq!(
            stats.source(),
            if timed {
                TimingSource::Gpu
            } else {
                TimingSource::Cpu
            }
        );
        assert!(
            stats
                .milliseconds
                .iter()
                .all(|ms| ms.is_finite() && *ms >= 0.0)
        );
        println!(
            "gpu pacer: {} slices, source {:?}, median {:.3} ms, longest {:.3} ms",
            stats.count(),
            stats.source(),
            stats.median_ms(),
            stats.longest_ms()
        );
    }

    fn pixel_slices(
        cursor: &mut Cursor,
        units: impl Fn(usize) -> u32,
    ) -> Vec<(u32, u32, u32, u32)> {
        let mut slices = Vec::new();
        while let Some(Slice::Pixels {
            x,
            y,
            width,
            height,
        }) = cursor.next(units(slices.len()))
        {
            slices.push((x, y, width, height));
        }
        slices
    }

    fn assert_exact_tiling(slices: &[(u32, u32, u32, u32)], width: u32, height: u32) {
        let mut covered = vec![0u8; width as usize * height as usize];
        for &(x, y, columns, rows) in slices {
            assert!(x + columns <= width && y + rows <= height);
            for row in y..y + rows {
                let at = row as usize * width as usize;
                for cell in &mut covered[at + x as usize..at + (x + columns) as usize] {
                    *cell += 1;
                }
            }
        }
        assert!(covered.iter().all(|count| *count == 1));
    }

    #[test]
    fn steps_of_eight_tile_a_4k_image_exactly() {
        for (work, columns) in [
            (
                Work::Bands {
                    width: 3840,
                    height: 2160,
                },
                8,
            ),
            (
                Work::Tiles {
                    width: 3840,
                    height: 2160,
                    tile_width: 500,
                },
                8,
            ),
        ] {
            for sizes in [[1u32, 5, 13, 64, 100], [8, 9, 7, 300, 2]] {
                let mut cursor = Cursor::stepped(work, 8, columns).unwrap();
                let slices = pixel_slices(&mut cursor, |index| sizes[index % sizes.len()]);
                assert_exact_tiling(&slices, 3840, 2160);
                for &(x, y, width, height) in &slices {
                    assert_eq!((x % 8, y % 8), (0, 0));
                    assert_eq!(height % 8, 0);
                    assert_eq!(width % 8, 0);
                }
            }
        }
    }

    #[test]
    fn the_edge_slices_are_the_partial_ones() {
        let mut cursor = Cursor::stepped(
            Work::Bands {
                width: 3843,
                height: 2163,
            },
            8,
            8,
        )
        .unwrap();
        let slices = pixel_slices(&mut cursor, |_| 100);
        assert_exact_tiling(&slices, 3843, 2163);
        let (last, rest) = slices.split_last().unwrap();
        assert_eq!(last.1 + last.3, 2163);
        assert_eq!(last.3 % 8, 2163 % 8);
        assert!(
            rest.iter()
                .all(|slice| slice.3 % 8 == 0 && slice.1 % 8 == 0)
        );
        assert!(rest.iter().all(|slice| slice.2 == 3843));

        let mut cursor = Cursor::stepped(
            Work::Tiles {
                width: 3843,
                height: 2163,
                tile_width: 500,
            },
            8,
            8,
        )
        .unwrap();
        let slices = pixel_slices(&mut cursor, |_| 100);
        assert_exact_tiling(&slices, 3843, 2163);
        for &(x, y, width, height) in &slices {
            assert_eq!((x % 8, y % 8), (0, 0));
            assert!(width % 8 == 0 || x + width == 3843);
            assert!(height % 8 == 0 || y + height == 2163);
            assert!(width <= 496);
        }
        assert!(slices.iter().any(|slice| slice.2 % 8 == 3843 % 8));
        assert!(slices.last().is_some_and(|slice| slice.3 % 8 == 2163 % 8));
    }

    #[test]
    fn a_step_larger_than_the_image_sends_the_image() {
        let mut cursor = Cursor::stepped(
            Work::Tiles {
                width: 5,
                height: 3,
                tile_width: 2,
            },
            8,
            8,
        )
        .unwrap();
        let slices = pixel_slices(&mut cursor, |_| 1);
        assert_eq!(slices, [(0, 0, 5, 3)]);
    }

    fn slice_heights(
        work: Work,
        step: Option<(u32, u32)>,
        cost: impl FnMut(u32, u32, f64) -> f64,
    ) -> (Stats, Vec<(u32, f64)>, Pacer) {
        let mut lane = Synthetic::new(Timing::Gpu, cost);
        let mut pacer = Pacer::default();
        if let Some((rows, columns)) = step {
            pacer.set_step(rows, columns);
        }
        let mut stats = pacer.drive("t", &mut lane, work, |_| {}).unwrap();
        stats.wall_ms = stats.milliseconds.iter().sum();
        (stats, lane.trace, pacer)
    }

    #[test]
    fn the_default_step_cuts_bands_and_tiles_like_dispatches() {
        let steady = |_: u32, _: u32, weight: f64| weight * 0.1;
        for work in [
            Work::Bands {
                width: 1000,
                height: 20_000,
            },
            Work::Tiles {
                width: 1000,
                height: 20_000,
                tile_width: 1000,
            },
        ] {
            let (_, reference, _) = slice_heights(Work::Dispatches { count: 20_000 }, None, steady);
            let (_, plain, pacer) = slice_heights(work, None, steady);
            assert_eq!(pacer.step(), (1, 1));
            assert_eq!(plain, reference);
            let (_, explicit, _) = slice_heights(work, Some((1, 1)), steady);
            assert_eq!(explicit, reference);
            let (_, stepped_trace, _) = slice_heights(
                Work::Bands {
                    width: 1000,
                    height: 20_000,
                },
                None,
                stepping(true),
            );
            let (_, dispatch_trace, _) =
                slice_heights(Work::Dispatches { count: 20_000 }, None, stepping(true));
            assert_eq!(stepped_trace, dispatch_trace);
        }
    }

    #[test]
    fn a_step_changes_nothing_for_dispatches() {
        let steady = |_: u32, _: u32, weight: f64| weight * 0.1;
        let (_, plain, _) = slice_heights(Work::Dispatches { count: 5000 }, None, steady);
        let (_, stepped, _) = slice_heights(Work::Dispatches { count: 5000 }, Some((8, 8)), steady);
        assert_eq!(plain, stepped);
    }

    #[test]
    fn the_step_and_outlier_workloads_stay_under_the_ceiling_with_a_step_of_eight() {
        let work = Work::Bands {
            width: 1000,
            height: TOTAL,
        };
        let (after, trace, pacer) = slice_heights(work, Some((8, 8)), stepping(true));
        let (flat, ..) = slice_heights(work, None, stepping(true));
        assert!(
            trace
                .iter()
                .take(trace.len() - 1)
                .all(|(rows, _)| rows % 8 == 0)
        );
        let ceiling = pacer.ceiling_ms();
        let calm_over = |stats: &Stats| {
            stats
                .milliseconds
                .iter()
                .enumerate()
                .filter(|(index, ms)| index % 40 != 39 && **ms > ceiling)
                .count()
        };
        assert!(
            calm_over(&after) <= calm_over(&flat) + 1,
            "{}",
            calm_over(&after)
        );
        assert!(calm_over(&after) <= 2, "{}", calm_over(&after));
        for index in 0..after.count() {
            if after.milliseconds[index] > ceiling
                && let Some((_, ms)) = trace.get(index + 2)
            {
                assert!((index + 2) % 40 == 39 || *ms < ceiling, "{index} {ms}");
            }
        }
        assert!(unsliced_ms() / after.wall_ms > 0.7);
        let (calm, _, _) = slice_heights(work, Some((8, 8)), stepping(false));
        assert!(calm.milliseconds.iter().filter(|ms| **ms > ceiling).count() <= 2);
        assert!(unsliced_ms() / calm.wall_ms > 0.9);
    }

    #[test]
    fn one_step_alone_over_the_ceiling_is_sent_and_reported() {
        let work = Work::Bands {
            width: 1000,
            height: 200,
        };
        let (stats, trace, pacer) = slice_heights(work, Some((8, 8)), |_, _, weight| weight);
        assert!(trace.iter().all(|(rows, _)| *rows == 8));
        assert_eq!(stats.count(), 25);
        let pacing = pacer.pacing();
        assert_eq!(pacing.slices, 25);
        assert_eq!(pacing.longest_ms, 8.0);
        assert!(pacing.longest_ms > pacer.ceiling_ms());
        assert_eq!(pacer.learned_units("t"), Some(8));
    }

    #[test]
    fn steps_are_never_below_one() {
        let mut pacer = Pacer::default();
        pacer.set_step(0, 0);
        assert_eq!(pacer.step(), (1, 1));
        pacer.set_step(8, 16);
        assert_eq!(pacer.step(), (8, 16));
    }

    #[test]
    fn learning_lands_on_the_grid() {
        let limits = Limits {
            target_ms: 3.5,
            ceiling_ms: 6.0,
            step: 8,
        };
        let mut tuning = Tuning::new(8);
        tuning.learn(8, 8.0, 0.5, 1000, limits);
        assert_eq!(tuning.units, 16);
        let mut tuning = Tuning::new(16);
        tuning.learn(16, 16.0, 9.0, 1000, limits);
        assert_eq!(tuning.units, 8);
        let mut tuning = Tuning::new(64);
        tuning.learn(64, 64.0, 2.0, 1000, limits);
        assert_eq!(tuning.units % 8, 0);
    }

    fn latency_bound(overhead_ms: f64, per_row_ms: f64) -> impl FnMut(u32, u32, f64) -> f64 {
        move |_, _, rows| overhead_ms + per_row_ms * rows
    }

    fn rows_and_ms(trace: &[(u32, f64)]) -> Vec<(u32, f64)> {
        trace.to_vec()
    }

    #[test]
    #[ignore = "prints the slices a latency-bound workload takes to reach the target band"]
    fn latency_bound_convergence() {
        for (name, step) in [("step 8", Some((8, 8))), ("step 1", None)] {
            let work = Work::Bands {
                width: 1000,
                height: 20_000,
            };
            let (_, trace, _) = slice_heights(work, step, latency_bound(3.0, 0.012));
            let in_band = trace
                .iter()
                .position(|&(_, ms)| (3.0..=4.2).contains(&ms) && ms >= 3.3);
            println!(
                "{name}: first slice at 3.3 ms or more {in_band:?}, rows {:?}",
                trace.iter().take(14).map(|t| t.0).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_latency_bound_band_grows_to_the_target() {
        let work = Work::Bands {
            width: 1000,
            height: 20_000,
        };
        let (stats, trace, pacer) = slice_heights(work, Some((8, 8)), latency_bound(3.0, 0.012));
        let trace = rows_and_ms(&trace);
        let reached = trace
            .iter()
            .position(|&(rows, ms)| rows >= 32 && ms >= 3.3)
            .expect("the bands reach the target band");
        assert!(reached <= 8, "{reached} {:?}", &trace[..12]);
        for &(rows, ms) in &trace[reached..trace.len() - 1] {
            assert!((3.3..=4.2).contains(&ms), "{rows} {ms}");
            assert_eq!(rows % 8, 0);
        }
        assert!(stats.longest_ms() < pacer.ceiling_ms());
        assert!(matches!(pacer.learned_units("t"), Some(32..=56)));
    }

    #[test]
    fn a_latency_bound_band_regrows_after_an_outlier() {
        let work = Work::Bands {
            width: 1000,
            height: 20_000,
        };
        let mut spiked = latency_bound(3.0, 0.012);
        let (stats, trace, pacer) =
            slice_heights(work, Some((8, 8)), move |submission, done, rows| {
                let spike = if submission == 30 { 3.0 } else { 1.0 };
                spiked(submission, done, rows) * spike
            });
        assert!(stats.milliseconds[30] > pacer.ceiling_ms());
        let after = &trace[31..];
        let regrown = after
            .iter()
            .position(|&(rows, _)| rows >= 32)
            .expect("the bands grow back");
        assert!(regrown <= 12, "{regrown} {:?}", &after[..14]);
        for &(_, ms) in &after[2..after.len() - 1] {
            assert!(ms < pacer.ceiling_ms());
        }
    }

    #[test]
    fn a_latency_bound_band_sizes_to_the_nearest_step() {
        let limits = Limits {
            target_ms: 3.5,
            ceiling_ms: 6.0,
            step: 8,
        };
        let mut tuning = Tuning::new(16);
        tuning.history.push(8.0, 3.0 + 0.012 * 8.0);
        tuning.learn(16, 16.0, 3.0 + 0.012 * 16.0, 1000, limits);
        assert_eq!(tuning.units, 32);
        tuning.learn(32, 32.0, 3.0 + 0.012 * 32.0, 1000, limits);
        assert_eq!(tuning.units, 40);
        tuning.learn(40, 40.0, 3.0 + 0.012 * 40.0, 1000, limits);
        assert_eq!(tuning.units, 48);
        tuning.learn(48, 48.0, 3.0 + 0.012 * 48.0, 1000, limits);
        assert_eq!(tuning.units, 48);
    }

    #[test]
    fn a_single_size_grows_by_a_step_when_one_more_stays_under_the_ceiling() {
        let limits = Limits {
            target_ms: 3.5,
            ceiling_ms: 6.0,
            step: 8,
        };
        let mut tuning = Tuning::new(8);
        tuning.learn(8, 8.0, 3.096, 1000, limits);
        assert_eq!(tuning.units, 16);
        let mut tuning = Tuning::new(16);
        tuning.learn(16, 16.0, 3.2, 1000, limits);
        assert_eq!(tuning.units, 24);
        let mut tuning = Tuning::new(8);
        tuning.learn(8, 8.0, 5.0, 1000, limits);
        assert_eq!(tuning.units, 8);
        let mut tuning = Tuning::new(64);
        tuning.learn(64, 64.0, 3.2, 1000, limits);
        assert_eq!(tuning.units, 64);
        let mut tuning = Tuning::new(8);
        tuning.learn(8, 8.0, 3.096, 8, limits);
        assert_eq!(tuning.units, 8);
    }

    #[test]
    fn recovery_grows_by_a_step_when_the_cost_is_overhead() {
        let limits = Limits {
            target_ms: 3.5,
            ceiling_ms: 6.0,
            step: 8,
        };
        let mut tuning = Tuning::new(16);
        tuning.recovering = RECOVERY_SLICES;
        tuning.history.push(8.0, 3.096);
        tuning.learn(16, 16.0, 3.192, 1000, limits);
        assert!(tuning.units >= 24, "{}", tuning.units);
    }

    #[test]
    fn the_fit_finds_the_overhead_and_shrugs_off_an_outlier() {
        let mut history = History::new();
        for rows in [8.0, 16.0, 24.0, 16.0, 40.0, 24.0, 32.0] {
            history.push(rows, 3.0 + 0.012 * rows);
        }
        history.push(24.0, 3.0 + 0.012 * 24.0 + 9.0);
        let model = history.fit().unwrap();
        assert!((model.overhead_ms - 3.0).abs() < 0.05, "{model:?}");
        assert!((model.per_unit_ms - 0.012).abs() < 0.002, "{model:?}");
        let mut same = History::new();
        same.push(8.0, 3.0);
        same.push(8.0, 3.1);
        same.push(9.0, 3.1);
        assert!(same.fit().is_none());
        let mut linear = History::new();
        for rows in [8.0, 16.0, 32.0] {
            linear.push(rows, rows * 0.1);
        }
        assert!(linear.fit().unwrap().overhead_ms < 1e-9);
    }

    #[test]
    fn a_shift_in_cost_forgets_the_old_fit() {
        let mut history = History::new();
        for rows in [8.0, 16.0, 24.0, 32.0] {
            history.record(rows, 3.0 + 0.012 * rows, false);
        }
        assert_eq!(history.points().len(), 4);
        history.record(32.0, 0.6, false);
        assert_eq!(history.points(), [(32.0, 0.6)]);
        history.record(32.0, 20.0, true);
        assert_eq!(history.points(), [(32.0, 20.0)]);
    }

    #[test]
    fn a_linear_workload_on_a_step_of_eight_stays_within_a_step_of_the_target() {
        let work = Work::Bands {
            width: 1000,
            height: 20_000,
        };
        let (stats, trace, pacer) = slice_heights(work, Some((8, 8)), |_, _, rows| rows * 0.1);
        let tail = &trace[trace.len() / 2..trace.len() - 1];
        assert!(
            tail.iter()
                .all(|&(rows, ms)| matches!(rows, 32 | 40) && ms <= 4.0 + 1e-9)
        );
        assert!(stats.longest_ms() <= pacer.ceiling_ms());
    }

    #[test]
    fn the_wall_fallback_turns_a_light_job_four_tenths_of_a_second_apart() {
        let began = Instant::now();
        let mut turns = Turns::new(50.0, false);
        turns.last_turn = began;
        turns.last_event = began;
        let mut last = began;
        let mut gaps = Vec::new();
        for tick in 1..=500u32 {
            let now = began + std::time::Duration::from_millis(u64::from(tick) * 10);
            if turns.add_at(0.5, now) {
                gaps.push(now.duration_since(last));
                last = now;
            }
        }
        assert_eq!(turns.taken(), 12);
        assert!(
            gaps.iter()
                .all(|gap| *gap == std::time::Duration::from_millis(390))
        );
        let slow = Turns::new(50.0, false).with_wall_ms(1000.0);
        assert!(!slow.due_at(slow.last_turn + std::time::Duration::from_millis(999)));
        assert!(slow.due_at(slow.last_turn + std::time::Duration::from_millis(1000)));
        let kept = Turns::new(50.0, false).with_wall_ms(f64::NAN);
        assert_eq!(kept.wall_ms, TURN_WALL_MS);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn sliced_path_tracer_stand_in_on_the_workgroup_grid() {
        const WIDTH: u32 = 3840;
        const HEIGHT: u32 = 2160;
        let gpu = pollster::block_on(crate::Gpu::headless()).unwrap();
        let device = &gpu.device;
        for scale in [1u32, 2] {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pacer step test"),
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var<storage, read_write> out: array<u32>;
                 @group(0) @binding(1) var<uniform> slice: vec4<u32>;
                 @group(0) @binding(2) var<uniform> image: vec4<u32>;
                 @compute @workgroup_size(8, 8)
                 fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                     if (id.x >= slice.z || id.y >= slice.w) {
                         return;
                     }
                     let px = slice.x + id.x;
                     let py = slice.y + id.y;
                     let rounds = SCALEu * (1200u + (((px / 24u) * 31u + (py / 24u) * 17u) % 61u) * 360u
                         + select(0u, 18000u, ((px / 96u) + (py / 96u)) % 5u == 0u));
                     var a = py * image.x + px;
                     for (var i = 0u; i < rounds; i = i + 1u) {
                         a = a * 1664525u + 1013904223u;
                     }
                     out[py * image.x + px] = a;
                 }"
                .replace("SCALE", &scale.to_string())
                .into(),
            ),
        });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pacer step test"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let bytes = u64::from(WIDTH) * u64::from(HEIGHT) * 4;
            let out = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("out"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("read"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let uniform = |values: [u32; 4]| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("uniform"),
                    size: 16,
                    usage: wgpu::BufferUsages::UNIFORM,
                    mapped_at_creation: true,
                });
                for (at, value) in values.iter().enumerate() {
                    buffer.slice(..).get_mapped_range_mut()[at * 4..at * 4 + 4]
                        .copy_from_slice(&value.to_le_bytes());
                }
                buffer.unmap();
                buffer
            };
            let mut turns = Turns::default();
            let mut run = |label: &str, step: u32, fixed: Option<u32>| {
                let mut pacer = Pacer::default();
                pacer.set_step(step, step);
                pacer.set_fixed_units(fixed);
                let mut heights = Vec::new();
                let stats = pacer
                    .run(
                        label,
                        device,
                        &gpu.queue,
                        Work::Bands {
                            width: WIDTH,
                            height: HEIGHT,
                        },
                        |encoder, slice| {
                            let Slice::Pixels {
                                x,
                                y,
                                width,
                                height,
                            } = slice
                            else {
                                unreachable!()
                            };
                            heights.push((y, height));
                            let slice_uniform = uniform([x, y, width, height]);
                            let image_uniform = uniform([WIDTH, HEIGHT, 0, 0]);
                            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: None,
                                layout: &pipeline.get_bind_group_layout(0),
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: out.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: slice_uniform.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: image_uniform.as_entire_binding(),
                                    },
                                ],
                            });
                            let mut pass = encoder.begin_compute_pass(&Default::default());
                            pass.set_pipeline(&pipeline);
                            pass.set_bind_group(0, &group, &[]);
                            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
                        },
                        |ms| turns.add(ms),
                    )
                    .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.copy_buffer_to_buffer(&out, 0, &read, 0, bytes);
                gpu.queue.submit(Some(encoder.finish()));
                read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                let data = read.slice(..).get_mapped_range().to_vec();
                read.unmap();
                (stats, heights, data)
            };
            let mut best_unsliced = f64::MAX;
            let mut reference = Vec::new();
            for round in 0..3 {
                let (stats, _, data) = run("step test unsliced", 1, Some(HEIGHT));
                assert_eq!(stats.count(), 1);
                if round > 0 {
                    best_unsliced = best_unsliced.min(stats.milliseconds[0]);
                }
                reference = data;
            }
            println!("scale {scale}: unsliced {best_unsliced:.3} ms");
            let mut ratios = Vec::new();
            for step in [1u32, 8] {
                let label = format!("step test {step}");
                let mut best = f64::MAX;
                for round in 0..2 {
                    let (stats, heights, data) = run(&label, step, None);
                    assert!(data == reference);
                    assert!(
                        stats.source() == TimingSource::Gpu || stats.source() == TimingSource::Cpu
                    );
                    for &(y, height) in &heights[..heights.len() - 1] {
                        assert_eq!((y % step, height % step), (0, 0));
                    }
                    let sum: f64 = stats.milliseconds.iter().sum();
                    println!(
                        "scale {scale} step {step} run {round}: {} slices, summed {:.3} ms, ratio {:.3}, median {:.3} ms, longest {:.3} ms, source {:?}",
                        stats.count(),
                        sum,
                        sum / best_unsliced,
                        stats.median_ms(),
                        stats.longest_ms(),
                        stats.source()
                    );
                    if round > 0 {
                        best = best.min(sum);
                    }
                }
                ratios.push(best / best_unsliced);
            }
            println!(
                "scale {scale} summed GPU time over unsliced: step 1 {:.3}, step 8 {:.3}",
                ratios[0], ratios[1]
            );
            assert!(ratios[1] <= ratios[0] * STEP_NOISE);
        }
    }

    fn moment(base: Instant, ms: u64, frozen_ms: f64) -> Moment {
        Moment {
            at: base + std::time::Duration::from_millis(ms),
            frozen_ms,
        }
    }

    #[test]
    fn frozen_time_comes_out_of_every_wall_interval() {
        assert_eq!(unfrozen_ms(100.0, 10.0, 70.0), 40.0);
        assert_eq!(unfrozen_ms(100.0, 10.0, 500.0), 0.0);
        assert_eq!(unfrozen_ms(100.0, 70.0, 10.0), 100.0);
        let base = Instant::now();
        let after = moment(base, 127_004, 127_000.0);
        assert_eq!(after.ms_since(moment(base, 0, 0.0)), 4.0);
        assert_eq!(moment(base, 0, 0.0).ms_since(after), 0.0);
        let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("pace-frozen-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let file = scratch.join("frozen");
        let frozen = Frozen::at(&file);
        assert_eq!(frozen.ms(), 0.0);
        std::fs::write(&file, "4413000\n").unwrap();
        assert_eq!(frozen.ms(), 4_413_000.0);
        std::fs::write(&file, "soon").unwrap();
        assert_eq!(frozen.ms(), 0.0);
        std::fs::write(&file, "-1").unwrap();
        assert_eq!(frozen.ms(), 0.0);
        std::fs::remove_dir_all(&scratch).unwrap();
        assert_eq!(Frozen::none().ms(), 0.0);
    }

    #[test]
    fn a_fence_timed_slice_leaves_out_freezes_and_turns() {
        let base = Instant::now();
        let submitted = moment(base, 0, 0.0);
        assert_eq!(
            fence_ms(submitted, None, moment(base, 120_010, 120_000.0)),
            10.0
        );
        let resumed = moment(base, 3_300, 0.0);
        assert_eq!(
            fence_ms(submitted, Some(resumed), moment(base, 3_305, 0.0)),
            5.0
        );
        assert_eq!(
            fence_ms(
                moment(base, 50, 0.0),
                Some(moment(base, 20, 0.0)),
                moment(base, 54, 0.0)
            ),
            4.0
        );
    }

    #[test]
    fn a_timed_wait_reports_its_unfrozen_time_and_nothing_across_a_turn() {
        let base = Instant::now();
        let mut turns = Turns::new(50.0, false).with_frozen(Frozen::none());
        turns.last_turn = base;
        assert!(!turns.add_timed(moment(base, 0, 0.0), moment(base, 1, 0.0), false));
        turns.last_turn = base + std::time::Duration::from_secs(90);
        assert!(!turns.add_timed(moment(base, 1, 0.0), moment(base, 90_005, 89_996.0), false));
        assert_eq!(turns.pending(), (9.0, 8.0));
        let mut nested = Turns::new(50.0, false)
            .with_frozen(Frozen::none())
            .with_wall_ms(1e9);
        let mut inner = Turns::new(50.0, false);
        let value = nested.time(|| {
            inner.add(30.0);
            inner.turn();
            5
        });
        assert_eq!(value, 5);
        assert_eq!(nested.pending(), (0.0, 0.0));
        assert_eq!(nested.args(), ["turn"]);
        nested.add(2.0);
        assert_eq!(
            nested.args(),
            ["turn", "--work-ms", "2.000", "--longest-ms", "2.000"]
        );
        nested.turn();
        assert_eq!(
            nested.args(),
            ["turn", "--work-ms", "0.000", "--longest-ms", "0.000"]
        );
    }

    #[test]
    fn a_measured_run_reports_gpu_time_and_hands_untimed_slices_on_as_bare() {
        let mut pacer = Pacer::default();
        pacer.set_fixed_units(Some(1));
        let mut seen = Vec::new();
        let stats = pacer
            .run_measured(
                "measured",
                Work::Dispatches { count: 4 },
                |slice| {
                    let Slice::Dispatches { start, .. } = slice else {
                        unreachable!()
                    };
                    Ok(match start {
                        0 => Some(2.5),
                        1 => None,
                        2 => Some(f64::NAN),
                        _ => Some(4.0),
                    })
                },
                |ms| seen.push(ms),
            )
            .unwrap();
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[0], 2.5);
        assert!(seen[1].is_nan() && seen[2].is_nan());
        assert_eq!(seen[3], 4.0);
        assert_eq!(
            stats.timings,
            [Timing::Gpu, Timing::Cpu, Timing::Cpu, Timing::Gpu]
        );
        assert_eq!(stats.milliseconds[0], 2.5);
        assert_eq!(stats.milliseconds[3], 4.0);
        let pacing = pacer.pacing();
        assert_eq!(pacing.work_ms, 6.5);
        assert_eq!(pacing.longest_ms, 4.0);
        assert_eq!(pacing.slices, 4);
        assert_eq!(pacing.untimed, 2);
        assert_eq!(pacing.source, TimingSource::Mixed);
        let mut turns = Turns::new(50.0, false).with_wall_ms(1e9);
        let mut blocking = Pacer::default();
        blocking
            .run_blocking(
                "blocking",
                Work::Dispatches { count: 3 },
                |_| Ok(()),
                |ms| turns.add(ms),
            )
            .unwrap();
        assert_eq!(turns.pending(), (0.0, 0.0));
        assert_eq!(turns.args(), ["turn"]);
        assert_eq!(blocking.pacing().untimed, blocking.pacing().slices);
        assert_eq!(blocking.pacing().work_ms, 0.0);
    }

    #[cfg(unix)]
    #[test]
    fn frozen_reports_reach_pgpu_without_the_freeze() {
        if std::env::var_os("PFX_TURN_CHILD").is_some() {
            return;
        }
        let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("pace-frozen-turns-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let pgpu = scratch.join("pgpu");
        std::fs::write(&pgpu, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$TURN_LOG\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&pgpu, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let log = scratch.join("turns");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![scratch.clone()];
        paths.extend(std::env::split_paths(&path));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "pace::tests::frozen_child",
                "--test-threads",
                "1",
            ])
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("PFX_TURN_CHILD", "1")
            .env("TURN_LOG", &log)
            .env_remove(FROZEN_VAR)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        let lines = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_dir_all(&scratch).unwrap();
        assert!(status.success());
        assert_eq!(
            lines.lines().collect::<Vec<_>>(),
            [
                "turn --work-ms 4.000 --longest-ms 4.000",
                "turn --work-ms 0.000 --longest-ms 0.000",
                "turn",
                "turn",
                "turn --work-ms 60.000 --longest-ms 30.000",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn frozen_child() {
        if std::env::var_os("PFX_TURN_CHILD").is_none() {
            return;
        }
        let base = Instant::now();
        let mut held = Turns::new(50.0, true);
        held.last_turn = base;
        held.add_timed(moment(base, 0, 0.0), moment(base, 90_004, 90_000.0), false);
        assert_eq!(held.taken(), 1);
        let mut outer = Turns::new(50.0, true).with_wall_ms(1e9);
        let mut inner = Turns::new(50.0, true);
        outer.time(|| inner.turn());
        outer.turn();
        let mut blocking = Turns::new(50.0, true).with_wall_ms(1e9);
        Pacer::default()
            .run_blocking(
                "blocking",
                Work::Dispatches { count: 3 },
                |_| Ok(()),
                |ms| blocking.add(ms),
            )
            .unwrap();
        blocking.turn();
        let mut measured = Turns::new(50.0, true).with_wall_ms(1e9);
        let mut pacer = Pacer::default();
        pacer.set_fixed_units(Some(1));
        pacer
            .run_measured(
                "measured",
                Work::Dispatches { count: 2 },
                |_| Ok(Some(30.0)),
                |ms| measured.add(ms),
            )
            .unwrap();
        assert_eq!(measured.taken(), 1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_blocking_run_times_its_slices_with_timestamps() {
        const GROUPS: u32 = 600;
        let gpu = pollster::block_on(crate::Gpu::headless()).unwrap();
        let device = &gpu.device;
        let compile = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("measured test"),
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var<storage, read_write> out: array<u32>;
                 @compute @workgroup_size(64)
                 fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                     var a = id.x;
                     for (var i = 0u; i < 4000u; i = i + 1u) {
                         a = a * 1664525u + 1013904223u;
                     }
                     out[id.x] = a;
                 }"
                .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("measured test"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let compile_ms = compile.elapsed().as_secs_f64() * 1000.0;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: u64::from(GROUPS) * 64 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: out.as_entire_binding(),
            }],
        });
        let timer = SliceTimer::new(device, &gpu.queue);
        let mut walls = Vec::new();
        let mut seen = Vec::new();
        let mut pacer = Pacer::default();
        let stats = pacer
            .run_measured(
                "measured test",
                Work::Dispatches { count: GROUPS },
                |slice| {
                    let Slice::Dispatches { count, .. } = slice else {
                        unreachable!()
                    };
                    let began = Instant::now();
                    let mut encoder = device.create_command_encoder(&Default::default());
                    if let Some(timer) = &timer {
                        timer.begin(&mut encoder);
                    }
                    {
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(&pipeline);
                        pass.set_bind_group(0, &group, &[]);
                        pass.dispatch_workgroups(count, 1, 1);
                    }
                    if let Some(timer) = &timer {
                        timer.end(&mut encoder);
                    }
                    let index = gpu.queue.submit(Some(encoder.finish()));
                    let measured = match &timer {
                        Some(timer) => timer.read_ms(device, index),
                        None => {
                            device
                                .poll(wgpu::PollType::Wait {
                                    submission_index: Some(index),
                                    timeout: None,
                                })
                                .map_err(|error| format!("{error:?}"))?;
                            None
                        }
                    };
                    walls.push(began.elapsed().as_secs_f64() * 1000.0);
                    Ok(measured)
                },
                |ms| seen.push(ms),
            )
            .unwrap();
        let timed = timer.is_some();
        assert_eq!(seen.len(), stats.count());
        if timed {
            assert_eq!(stats.source(), TimingSource::Gpu);
            assert_eq!(seen, stats.milliseconds);
            assert_eq!(pacer.pacing().untimed, 0);
            for (gpu_ms, wall_ms) in stats.milliseconds.iter().zip(&walls) {
                assert!(*gpu_ms > 0.0 && gpu_ms <= wall_ms, "{gpu_ms} {wall_ms}");
            }
        } else {
            assert_eq!(stats.source(), TimingSource::Cpu);
            assert!(seen.iter().all(|ms| ms.is_nan()));
        }
        println!(
            "measured lane: timestamps {timed}, pipeline created in {compile_ms:.3} ms, first slice {:.3} ms GPU in {:.3} ms wall, {} slices, median {:.3} ms, longest {:.3} ms",
            stats.milliseconds[0],
            walls[0],
            stats.count(),
            stats.median_ms(),
            stats.longest_ms()
        );
    }

    #[test]
    fn fixed_units_read_back_as_set() {
        let mut pacer = Pacer::default();
        assert_eq!(pacer.fixed_units(), None);
        pacer.set_fixed_units(Some(0));
        assert_eq!(pacer.fixed_units(), Some(1));
        pacer.set_fixed_units(Some(12));
        assert_eq!(pacer.fixed_units(), Some(12));
    }
}
