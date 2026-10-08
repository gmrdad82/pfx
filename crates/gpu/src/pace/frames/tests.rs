use std::cell::RefCell;
use std::rc::Rc;

use super::*;

struct Fake {
    now: f64,
    slept: Vec<f64>,
    turns: u32,
    reports: Vec<TurnReport>,
}

impl Fake {
    fn new() -> Self {
        Self {
            now: 100.0,
            slept: Vec::new(),
            turns: 0,
            reports: Vec::new(),
        }
    }
}

impl FrameClock for Fake {
    fn now_ms(&mut self) -> f64 {
        self.now
    }

    fn sleep_ms(&mut self, ms: f64) {
        self.slept.push(ms);
        self.now += ms;
    }

    fn turn(&mut self, turn: TurnReport) {
        self.turns += 1;
        self.reports.push(turn);
    }
}

#[test]
fn sleep_runs_to_the_next_boundary_and_never_negative() {
    assert_eq!(sleep_to(116.0, 110.0), 6.0);
    assert_eq!(sleep_to(116.0, 116.0), 0.0);
    assert_eq!(sleep_to(116.0, 130.0), 0.0);
}

#[test]
fn a_capped_loop_sleeps_to_each_frame_boundary() {
    let period = 1000.0 / 60.0;
    let mut pace = Pace::with_clock(Fake::new(), Some(60.0));
    for _ in 0..3 {
        pace.clock.now += 10.0;
        pace.after(10.0);
    }
    let slept = &pace.clock.slept;
    assert_eq!(slept.len(), 3);
    assert!((slept[0] - (period - 10.0)).abs() < 1e-9);
    assert!((slept[1] - (period - 10.0)).abs() < 1e-9);
    assert!((pace.clock.now - (100.0 + 3.0 * period)).abs() < 1e-9);
}

#[test]
fn a_late_frame_does_not_owe_a_burst() {
    let period = 1000.0 / 60.0;
    let mut pace = Pace::with_clock(Fake::new(), Some(60.0));
    pace.clock.now += 40.0;
    pace.after(40.0);
    assert!(pace.clock.slept.is_empty());
    pace.clock.now += 10.0;
    pace.after(10.0);
    assert_eq!(pace.clock.slept.len(), 1);
    assert!((pace.clock.slept[0] - (period - 10.0)).abs() < 1e-9);
}

#[test]
fn uncapped_never_sleeps_and_still_turns() {
    let mut pace = Pace::with_clock(Fake::new(), None);
    pace.clock.now += 12.0;
    pace.after(12.0);
    for _ in 0..10 {
        pace.clock.now += 12.0;
        pace.after(12.0);
    }
    assert!(pace.clock.slept.is_empty());
    assert_eq!(pace.clock.turns, 2);
}

#[test]
fn turns_after_fifty_ms_of_work_or_four_tenths_of_a_second() {
    let mut pace = Pace::with_clock(Fake::new(), Some(60.0));
    pace.clock.now += 10.0;
    pace.after(10.0);
    for _ in 0..4 {
        pace.clock.now += 10.0;
        pace.after(10.0);
    }
    assert_eq!(pace.clock.turns, 0);
    pace.clock.now += 10.0;
    pace.after(10.0);
    assert_eq!(pace.clock.turns, 1);
    pace.clock.now += 390.0;
    pace.after(1.0);
    assert_eq!(pace.clock.turns, 1);
    pace.clock.now += 20.0;
    pace.after(1.0);
    assert_eq!(pace.clock.turns, 2);
}

#[test]
fn the_wall_fallback_reads_its_env_and_defaults_to_four_tenths_of_a_second() {
    let none = |_: &str| None;
    assert_eq!(turn_wall_ms(&none), 400.0);
    let set = |value: &'static str| move |_: &str| Some(value.to_string());
    assert_eq!(turn_wall_ms(&set("250")), 250.0);
    assert_eq!(turn_wall_ms(&set(" 1500 ")), 1500.0);
    assert_eq!(turn_wall_ms(&set("0")), 400.0);
    assert_eq!(turn_wall_ms(&set("-5")), 400.0);
    assert_eq!(turn_wall_ms(&set("soon")), 400.0);
}

#[test]
fn frame_times_its_own_work_without_the_sleep() {
    let mut pace = Pace::with_clock(Fake::new(), Some(60.0));
    let value = pace.frame(Vec::<crate::PassTiming>::new);
    assert!(value.is_empty());
    assert_eq!(pace.clock.slept.len(), 1);
}

fn passes(ms: &[f64]) -> Vec<crate::PassTiming> {
    ms.iter()
        .map(|&milliseconds| crate::PassTiming {
            label: "pass".into(),
            milliseconds,
        })
        .collect()
}

#[test]
fn turns_report_gpu_work_and_the_longest_frame() {
    let mut pace = Pace::with_clock(Fake::new(), None);
    pace.frame(|| passes(&[4.0, 6.0]));
    pace.frame(|| passes(&[10.0, 9.0]));
    pace.after_gpu(30.0, passes(&[1.0, 2.0]).gpu_ms());
    assert!(pace.clock.reports.is_empty());
    pace.frame(|| passes(&[20.0, 2.0]));
    assert_eq!(
        pace.clock.reports,
        [TurnReport {
            work_ms: 54.0,
            longest_ms: 22.0,
        }]
    );
    pace.clock.now += 12.5;
    pace.after_gpu(12.5, None);
    pace.clock.now += 2500.0;
    pace.after_gpu(8.0, passes(&[]).gpu_ms());
    assert_eq!(
        pace.clock.reports[1],
        TurnReport {
            work_ms: 20.5,
            longest_ms: 12.5,
        }
    );
}

#[test]
fn a_turn_formats_pgpus_flags() {
    assert_eq!(
        TurnReport {
            work_ms: 50.0,
            longest_ms: 8.0626,
        }
        .args(),
        ["turn", "--work-ms", "50.000", "--longest-ms", "8.063"]
    );
}

#[test]
fn fps_is_one_thousand_over_the_median_frame_time() {
    let sorted = [1.0, 2.0, 3.0, 4.0, 5.0];
    let frames = Frames::of(&sorted);
    assert_eq!(frames.p50, 3.0);
    assert_eq!(frames.max, 5.0);
    assert_eq!(frames.fps, 1000.0 / 3.0);
    assert_eq!(Frames::of(&[8.0]).fps, 125.0);
}

#[derive(Default)]
struct Seen {
    reports: Vec<(u64, u64, f64)>,
    ended: Vec<bool>,
}

struct Watch(Rc<RefCell<Seen>>);

impl Sink for Watch {
    fn progress(&mut self, done: u64, total: u64, fps: f64) {
        self.0.borrow_mut().reports.push((done, total, fps));
    }

    fn end(&mut self, ok: bool) {
        self.0.borrow_mut().ended.push(ok);
    }
}

#[test]
fn turns_report_frames_done_over_planned_and_the_rate() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    {
        let mut pace =
            Pace::with_clock(Fake::new(), None).recording(Box::new(Watch(Rc::clone(&seen))), 100);
        for _ in 0..4 {
            pace.clock.now += 10.0;
            pace.after_gpu(10.0, Some(10.0));
        }
        assert!(seen.borrow().reports.is_empty());
        pace.clock.now += 10.0;
        pace.after_gpu(10.0, Some(10.0));
        assert_eq!(seen.borrow().reports, [(5, 100, 100.0)]);
        for _ in 0..5 {
            pace.clock.now += 20.0;
            pace.after_gpu(20.0, Some(20.0));
        }
        assert_eq!(seen.borrow().reports.len(), 2);
        assert_eq!(seen.borrow().reports[1], (8, 100, 8.0 * 1000.0 / 110.0));
        assert!(seen.borrow().ended.is_empty());
    }
    assert_eq!(seen.borrow().ended, [true]);
}

#[test]
fn a_pace_without_a_sink_reports_nothing() {
    let mut pace = Pace::with_clock(Fake::new(), None);
    for _ in 0..10 {
        pace.clock.now += 12.0;
        pace.after_gpu(12.0, Some(12.0));
    }
    assert_eq!(pace.clock.turns, 2);
}

struct Held {
    now: std::rc::Rc<std::cell::Cell<f64>>,
    frozen: std::rc::Rc<std::cell::Cell<f64>>,
    turn_wait_ms: f64,
    reports: Vec<TurnReport>,
}

impl Held {
    fn new() -> (
        Self,
        std::rc::Rc<std::cell::Cell<f64>>,
        std::rc::Rc<std::cell::Cell<f64>>,
    ) {
        let now = std::rc::Rc::new(std::cell::Cell::new(100.0));
        let frozen = std::rc::Rc::new(std::cell::Cell::new(0.0));
        (
            Self {
                now: now.clone(),
                frozen: frozen.clone(),
                turn_wait_ms: 0.0,
                reports: Vec::new(),
            },
            now,
            frozen,
        )
    }
}

impl FrameClock for Held {
    fn now_ms(&mut self) -> f64 {
        self.now.get()
    }

    fn sleep_ms(&mut self, ms: f64) {
        self.now.set(self.now.get() + ms);
    }

    fn turn(&mut self, turn: TurnReport) {
        self.reports.push(turn);
        self.now.set(self.now.get() + self.turn_wait_ms);
    }

    fn frozen_ms(&mut self) -> f64 {
        self.frozen.get()
    }
}

#[test]
fn a_freeze_inside_a_cpu_timed_frame_never_reaches_a_turn() {
    let (clock, now, frozen) = Held::new();
    let mut pace = Pace::with_clock(clock, None);
    pace.after(0.0);
    for _ in 0..4 {
        pace.frame(|| {
            now.set(now.get() + 12.0);
        });
    }
    pace.frame(|| {
        now.set(now.get() + 73.0 * 60_000.0 + 12.0);
        frozen.set(frozen.get() + 73.0 * 60_000.0);
    });
    assert_eq!(
        pace.clock.reports,
        [TurnReport {
            work_ms: 60.0,
            longest_ms: 12.0,
        }]
    );
}

#[test]
fn a_freeze_before_a_frame_timed_by_its_loop_is_bounded_out() {
    let (clock, now, frozen) = Held::new();
    let mut pace = Pace::with_clock(clock, None);
    pace.after(0.0);
    now.set(now.get() + 9_000.0 + 20.0);
    frozen.set(9_000.0);
    pace.after(9_020.0);
    assert_eq!(
        pace.clock.reports,
        [TurnReport {
            work_ms: 20.0,
            longest_ms: 20.0,
        }]
    );
}

#[test]
fn a_turn_that_waits_is_never_counted_in_the_next_frame() {
    let (mut clock, now, _) = Held::new();
    clock.turn_wait_ms = 3_200.0;
    let mut pace = Pace::with_clock(clock, None);
    pace.after(0.0);
    for _ in 0..10 {
        now.set(now.get() + 25.0);
        pace.after(f64::MAX);
    }
    assert_eq!(pace.clock.reports.len(), 5);
    for report in &pace.clock.reports {
        assert_eq!(
            *report,
            TurnReport {
                work_ms: 50.0,
                longest_ms: 25.0,
            }
        );
    }
}

#[test]
fn gpu_time_is_reported_whatever_the_wall_says() {
    let (clock, now, frozen) = Held::new();
    let mut pace = Pace::with_clock(clock, None);
    now.set(now.get() + 127_000.0);
    frozen.set(126_000.0);
    pace.after_gpu(127_000.0, Some(4.5));
    pace.after_gpu(1.0, Some(46.0));
    pace.after_gpu(90_000.0, Some(4.0));
    assert_eq!(
        pace.clock.reports,
        [
            TurnReport {
                work_ms: 4.5,
                longest_ms: 4.5,
            },
            TurnReport {
                work_ms: 50.0,
                longest_ms: 46.0,
            },
        ]
    );
}

#[test]
fn a_first_cpu_timed_frame_holds_first_use_compile_and_is_left_out() {
    let (clock, now, _) = Held::new();
    let mut pace = Pace::with_clock(clock, None);
    pace.frame(|| now.set(now.get() + 6_423.0));
    pace.frame(|| now.set(now.get() + 25.0));
    pace.frame(|| now.set(now.get() + 25.0));
    assert_eq!(
        pace.clock.reports,
        [
            TurnReport {
                work_ms: 0.0,
                longest_ms: 0.0,
            },
            TurnReport {
                work_ms: 50.0,
                longest_ms: 25.0,
            },
        ]
    );
}

#[test]
fn only_one_lifts_the_cap() {
    let set =
        |value: &'static str| move |key: &str| (key == UNCAPPED_VAR).then(|| value.to_string());
    assert!(uncapped(&set("1")));
    assert!(!uncapped(&set("0")));
    assert!(!uncapped(&set("true")));
    assert!(!uncapped(&|_: &str| None));
}

#[test]
fn frozen_file_ms_reads_a_number_and_ignores_anything_else() {
    let dir = std::env::temp_dir().join(format!("pito-gpu-frozen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("frozen");
    std::fs::write(&file, " 1250.5\n").unwrap();
    assert_eq!(frozen_file_ms(Some(&file)), 1250.5);
    std::fs::write(&file, "soon").unwrap();
    assert_eq!(frozen_file_ms(Some(&file)), 0.0);
    std::fs::write(&file, "-3").unwrap();
    assert_eq!(frozen_file_ms(Some(&file)), 0.0);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(frozen_file_ms(Some(&file)), 0.0);
    assert_eq!(frozen_file_ms(None), 0.0);
}
