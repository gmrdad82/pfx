#![allow(dead_code, unused_imports, clippy::new_ret_no_self)]

pub use pfx_gpu::pace::{
    FrameClock as Clock, FrameGpu as Measured, Frames, Sink, TurnReport as Turn, WallClock as Wall,
    frozen_file_ms, sleep_to, turn_wall_ms, unfrozen_ms,
};

use pfx_gpu::pace;

pub struct Record {
    job: Option<pfx_run::job::Job>,
}

impl Record {
    pub fn start(label: &str, pgpu: u32) -> Self {
        let job = pfx_run::job::Job::start(pfx_run::job::TOOL, None, label);
        job.pgpu(Some(pgpu));
        Self { job: Some(job) }
    }

    pub fn planned(&self, total: u64) {
        if let Some(job) = &self.job {
            job.stage("frames", Some(total), "frames");
        }
    }
}

impl Sink for Record {
    fn progress(&mut self, done: u64, _total: u64, _fps: f64) {
        if let Some(job) = &self.job {
            job.progress(done);
        }
    }

    fn end(&mut self, ok: bool) {
        if let Some(job) = self.job.take() {
            let _ = job.end(if ok {
                Ok(())
            } else {
                Err("failed".to_string())
            });
        }
    }
}

pub fn pgpu_pid(get: &impl Fn(&str) -> Option<String>) -> Option<u32> {
    get("GPU_QUEUE_PID")?.trim().parse().ok()
}

pub struct Tally;

impl Tally {
    pub fn none() -> pace::Tally {
        pace::Tally::none()
    }

    pub fn with_sink(sink: Box<dyn Sink>, planned: u64, now_ms: f64) -> pace::Tally {
        pace::Tally::with_sink(sink, planned, now_ms)
    }

    pub fn timed(example: &str, mode: &str, planned: u64, now_ms: f64) -> pace::Tally {
        Self::timed_in(
            |key| std::env::var(key).ok(),
            example,
            mode,
            planned,
            now_ms,
            |label, pgpu| {
                let record = Record::start(label, pgpu);
                record.planned(planned);
                Box::new(record)
            },
        )
    }

    pub fn timed_in(
        get: impl Fn(&str) -> Option<String>,
        example: &str,
        mode: &str,
        planned: u64,
        now_ms: f64,
        start: impl FnOnce(&str, u32) -> Box<dyn Sink>,
    ) -> pace::Tally {
        match pgpu_pid(&get) {
            Some(pgpu) => {
                Self::with_sink(start(&format!("{example} {mode}"), pgpu), planned, now_ms)
            }
            None => Self::none(),
        }
    }
}

pub struct Pace;

impl Pace {
    pub fn new() -> pace::Pace<Wall> {
        pace::Pace::new()
    }

    pub fn with_clock<C: Clock>(clock: C, cap_fps: Option<f64>) -> pace::Pace<C> {
        pace::Pace::with_clock(clock, cap_fps)
    }

    pub fn timed(example: &str, mode: &str, planned: u64) -> pace::Pace<Wall> {
        let mut pace = pace::Pace::new();
        let now = Clock::now_ms(&mut pace.clock);
        pace.with_tally(Tally::timed(example, mode, planned, now))
    }
}
