use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const PACING_VAR: &str = "PFX_PACING";
pub const SCHEMA_VERSION: u32 = 2;
pub const GUI_BUDGET_NS: u64 = 8_330_000;
pub const TUI_BUDGET_NS: u64 = 8_330_000;
pub const CAP60_BUDGET_NS: u64 = 16_670_000;

const NS_PER_SEC: u64 = 1_000_000_000;
const BUCKET_NS: u64 = 10_000;
const BUCKETS: usize = 20_000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PacingReport {
    pub frames: u64,
    pub intervals: u64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub stddev_ms: f64,
    pub late: u64,
    pub missed: u64,
    pub budget_ms: f64,
    pub acquires: u64,
    pub acquire_p50_ms: f64,
    pub acquire_p99_ms: f64,
    pub acquire_max_ms: f64,
    pub acquire_mean_ms: f64,
    pub present_call_p50_ms: f64,
    pub present_call_p99_ms: f64,
    pub present_call_max_ms: f64,
    pub viewport_p50_ms: f64,
    pub viewport_p99_ms: f64,
    pub viewport_max_ms: f64,
    pub paint_p50_ms: f64,
    pub paint_p99_ms: f64,
    pub paint_max_ms: f64,
    pub shown_intervals: u64,
    pub shown_p50_ms: f64,
    pub shown_p99_ms: f64,
    pub shown_max_ms: f64,
    pub discarded: u64,
}

impl PacingReport {
    pub fn holds(&self) -> bool {
        self.late == 0 && self.p99_ms <= self.budget_ms
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub tag: u64,
    pub at_ns: Option<u64>,
    pub refresh_ns: Option<u64>,
}

const PROBE_WINDOW: u32 = 120;
const PROBE_SLACK_NS: u64 = 300_000;
const PROBE_SPAN: u64 = 8;

#[derive(Clone, Debug, Default)]
pub struct VrrProbe {
    last_ns: Option<u64>,
    monitor_ns: Option<u64>,
    quantized: u32,
    free: u32,
    verdict: Option<bool>,
    reading: Option<bool>,
}

impl VrrProbe {
    pub fn verdict(&self) -> Option<bool> {
        self.verdict
    }

    pub fn reset(&mut self) {
        *self = Self {
            monitor_ns: self.monitor_ns,
            ..Self::default()
        };
    }

    pub fn set_refresh(&mut self, refresh_millihertz: Option<u32>) {
        let monitor_ns = refresh_millihertz
            .filter(|refresh| *refresh > 0)
            .map(|refresh| 1_000_000_000_000 / u64::from(refresh));
        if monitor_ns != self.monitor_ns {
            self.monitor_ns = monitor_ns;
            self.reset();
        }
    }

    pub fn shown(&mut self, shown: Shown) -> bool {
        let Some(at) = shown.at_ns else {
            return false;
        };
        let Some(last) = self.last_ns.replace(at) else {
            return false;
        };
        let interval = at.saturating_sub(last);
        let refresh = match shown.refresh_ns {
            Some(0) => self.monitor_ns.or(Some(0)),
            reported => reported,
        };
        match refresh {
            Some(0) => self.free += 1,
            Some(refresh) if interval > 0 && interval <= refresh.saturating_mul(PROBE_SPAN) => {
                let multiple = ((interval + refresh / 2) / refresh).max(1);
                let off = interval.abs_diff(multiple * refresh);
                if off <= PROBE_SLACK_NS.max(refresh * 15 / 100) {
                    self.quantized += 1;
                } else {
                    self.free += 1;
                }
            }
            _ => return false,
        }
        if self.quantized + self.free < PROBE_WINDOW {
            return false;
        }
        let total = self.quantized + self.free;
        let reading = if self.quantized * 10 >= total * 9 {
            Some(false)
        } else if self.quantized * 2 <= total {
            Some(true)
        } else {
            None
        };
        self.quantized = 0;
        self.free = 0;
        let held = reading.is_some() && reading == self.reading;
        self.reading = reading;
        if !held || reading == self.verdict {
            return false;
        }
        self.verdict = reading;
        true
    }
}

#[derive(Clone, Debug)]
struct Spread {
    buckets: Vec<u32>,
    count: u64,
    max_ns: u64,
    sum_ns: u128,
    squares: f64,
}

impl Spread {
    fn new() -> Self {
        Self {
            buckets: vec![0; BUCKETS],
            count: 0,
            max_ns: 0,
            sum_ns: 0,
            squares: 0.0,
        }
    }

    fn add(&mut self, ns: u64) {
        let bucket = usize::try_from(ns / BUCKET_NS)
            .unwrap_or(usize::MAX)
            .min(BUCKETS - 1);
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.count += 1;
        self.sum_ns += u128::from(ns);
        self.squares += (ns as f64 / 1e6).powi(2);
        self.max_ns = self.max_ns.max(ns);
    }

    fn mean_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum_ns as f64 / self.count as f64 / 1e6
        }
    }

    fn percentile_ms(&self, fraction: f64) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let rank = ((fraction * self.count as f64).ceil() as u64).clamp(1, self.count);
        let mut seen = 0u64;
        for (index, count) in self.buckets.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                return ((index as u64 + 1) * BUCKET_NS).min(self.max_ns) as f64 / 1e6;
            }
        }
        self.max_ns as f64 / 1e6
    }

    fn stddev_ms(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let mean = self.mean_ms();
        (self.squares / self.count as f64 - mean * mean)
            .max(0.0)
            .sqrt()
    }

    fn max_ms(&self) -> f64 {
        self.max_ns as f64 / 1e6
    }
}

#[derive(Clone, Debug)]
pub struct Presents {
    budget_ns: u64,
    last_ns: Option<u64>,
    frames: u64,
    late: u64,
    missed: u64,
    intervals: Spread,
    acquires: Spread,
    calls: Spread,
    viewports: Spread,
    paints: Spread,
    shown: Spread,
    last_shown: Option<(u64, u64)>,
    discarded: u64,
}

impl Presents {
    pub fn new(budget_ns: u64) -> Self {
        Self {
            budget_ns: budget_ns.max(1),
            last_ns: None,
            frames: 0,
            late: 0,
            missed: 0,
            intervals: Spread::new(),
            acquires: Spread::new(),
            calls: Spread::new(),
            viewports: Spread::new(),
            paints: Spread::new(),
            shown: Spread::new(),
            last_shown: None,
            discarded: 0,
        }
    }

    pub fn presented(&mut self, now_ns: u64) {
        self.frames += 1;
        if let Some(last) = self.last_ns.replace(now_ns) {
            let interval = now_ns.saturating_sub(last);
            self.intervals.add(interval);
            if interval > self.budget_ns {
                self.late += 1;
            }
            if interval.saturating_mul(2) >= self.budget_ns.saturating_mul(3) {
                self.missed += 1;
            }
        }
    }

    pub fn acquired(&mut self, wait_ns: u64) {
        self.acquires.add(wait_ns);
    }

    pub fn present_call(&mut self, ns: u64) {
        self.calls.add(ns);
    }

    pub fn viewport(&mut self, ns: u64) {
        self.viewports.add(ns);
    }

    pub fn paint(&mut self, ns: u64) {
        self.paints.add(ns);
    }

    pub fn shown(&mut self, shown: Shown) {
        let Some(at) = shown.at_ns else {
            self.discarded += 1;
            return;
        };
        if let Some((tag, last)) = self.last_shown
            && tag == shown.tag
            && at > last
        {
            self.shown.add(at - last);
        }
        self.last_shown = Some((shown.tag, at));
    }

    pub fn idle(&mut self) {
        self.last_ns = None;
    }

    pub fn report(&self) -> PacingReport {
        PacingReport {
            frames: self.frames,
            intervals: self.intervals.count,
            p50_ms: self.intervals.percentile_ms(0.50),
            p99_ms: self.intervals.percentile_ms(0.99),
            max_ms: self.intervals.max_ms(),
            mean_ms: self.intervals.mean_ms(),
            stddev_ms: self.intervals.stddev_ms(),
            late: self.late,
            missed: self.missed,
            budget_ms: self.budget_ns as f64 / 1e6,
            acquires: self.acquires.count,
            acquire_p50_ms: self.acquires.percentile_ms(0.50),
            acquire_p99_ms: self.acquires.percentile_ms(0.99),
            acquire_max_ms: self.acquires.max_ms(),
            acquire_mean_ms: self.acquires.mean_ms(),
            present_call_p50_ms: self.calls.percentile_ms(0.50),
            present_call_p99_ms: self.calls.percentile_ms(0.99),
            present_call_max_ms: self.calls.max_ms(),
            viewport_p50_ms: self.viewports.percentile_ms(0.50),
            viewport_p99_ms: self.viewports.percentile_ms(0.99),
            viewport_max_ms: self.viewports.max_ms(),
            paint_p50_ms: self.paints.percentile_ms(0.50),
            paint_p99_ms: self.paints.percentile_ms(0.99),
            paint_max_ms: self.paints.max_ms(),
            shown_intervals: self.shown.count,
            shown_p50_ms: self.shown.percentile_ms(0.50),
            shown_p99_ms: self.shown.percentile_ms(0.99),
            shown_max_ms: self.shown.max_ms(),
            discarded: self.discarded,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub host: &'static str,
    pub present_mode: String,
    pub frame_latency: u32,
    pub vrr: bool,
    pub vrr_in_effect: Option<bool>,
    pub frame_rate: &'static str,
}

fn budget_for(budget_ns: u64, frame_rate: &str) -> u64 {
    if frame_rate == "cap60" {
        budget_ns.max(CAP60_BUDGET_NS)
    } else {
        budget_ns
    }
}

pub struct Pacing {
    path: Option<PathBuf>,
    budget_ns: u64,
    done: Vec<(Setting, PacingReport)>,
    setting: Option<Setting>,
    current: Presents,
    written_ns: Option<u64>,
    run: u64,
    section_run: u64,
}

impl Pacing {
    pub fn off() -> Self {
        Self::at(None, GUI_BUDGET_NS)
    }

    pub fn from_env(budget_ns: u64) -> Self {
        let path = std::env::var_os(PACING_VAR)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        Self::at(path, budget_ns)
    }

    pub fn to_file(path: &Path, budget_ns: u64) -> Self {
        Self::at(Some(path.to_path_buf()), budget_ns)
    }

    fn at(path: Option<PathBuf>, budget_ns: u64) -> Self {
        Self {
            path,
            budget_ns,
            done: Vec::new(),
            setting: None,
            current: Presents::new(budget_ns),
            written_ns: None,
            run: 0,
            section_run: 0,
        }
    }

    pub fn on(&self) -> bool {
        self.path.is_some()
    }

    pub fn tag(&self) -> Option<u64> {
        self.on().then_some(self.run)
    }

    pub fn set(&mut self, setting: Setting) {
        if self.setting.as_ref() == Some(&setting) {
            return;
        }
        let budget = budget_for(self.budget_ns, setting.frame_rate);
        let current = std::mem::replace(&mut self.current, Presents::new(budget));
        self.run += 1;
        self.section_run = self.run;
        if let Some(previous) = self.setting.replace(setting)
            && current.frames > 0
        {
            self.done.push((previous, current.report()));
        }
    }

    pub fn presented(&mut self, now_ns: u64) {
        if !self.on() {
            return;
        }
        self.current.presented(now_ns);
        let due = self
            .written_ns
            .is_none_or(|written| now_ns.saturating_sub(written) >= NS_PER_SEC);
        if due {
            self.written_ns = Some(now_ns);
            self.write();
        }
    }

    pub fn acquired(&mut self, wait_ns: u64) {
        if self.on() {
            self.current.acquired(wait_ns);
        }
    }

    pub fn present_call(&mut self, ns: u64) {
        if self.on() {
            self.current.present_call(ns);
        }
    }

    pub fn viewport(&mut self, ns: u64) {
        if self.on() {
            self.current.viewport(ns);
        }
    }

    pub fn paint(&mut self, ns: u64) {
        if self.on() {
            self.current.paint(ns);
        }
    }

    pub fn shown(&mut self, shown: Shown) {
        if self.on() && shown.tag >= self.section_run {
            self.current.shown(shown);
        }
    }

    pub fn idle(&mut self) {
        self.current.idle();
        self.run += 1;
    }

    pub fn sections(&self) -> Vec<(Setting, PacingReport)> {
        let mut sections = self.done.clone();
        if let Some(setting) = &self.setting
            && self.current.frames > 0
        {
            sections.push((setting.clone(), self.current.report()));
        }
        sections
    }

    pub fn lines(&self) -> String {
        let mut out = String::new();
        for (setting, report) in self.sections() {
            let shown = |ms: f64| {
                if report.shown_intervals == 0 {
                    "null".to_string()
                } else {
                    format!("{ms:.3}")
                }
            };
            let _ = writeln!(
                out,
                "{{\"schema_version\":{SCHEMA_VERSION},\"host\":\"{}\",\"present_mode\":\"{}\",\"frame_latency\":{},\"vrr\":{},\"vrr_in_effect\":{},\"frame_rate\":\"{}\",\"budget_ms\":{:.3},\"frames\":{},\"intervals\":{},\"p50_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"mean_ms\":{:.3},\"stddev_ms\":{:.3},\"late\":{},\"missed\":{},\"acquire_p50_ms\":{:.3},\"acquire_p99_ms\":{:.3},\"acquire_max_ms\":{:.3},\"acquire_mean_ms\":{:.3},\"present_call_p50_ms\":{:.3},\"present_call_p99_ms\":{:.3},\"present_call_max_ms\":{:.3},\"viewport_p50_ms\":{:.3},\"viewport_p99_ms\":{:.3},\"viewport_max_ms\":{:.3},\"paint_p50_ms\":{:.3},\"paint_p99_ms\":{:.3},\"paint_max_ms\":{:.3},\"shown_intervals\":{},\"shown_p50_ms\":{},\"shown_p99_ms\":{},\"shown_max_ms\":{},\"discarded\":{}}}",
                setting.host,
                setting.present_mode,
                setting.frame_latency,
                setting.vrr,
                setting
                    .vrr_in_effect
                    .map_or("null", |on| if on { "true" } else { "false" }),
                setting.frame_rate,
                report.budget_ms,
                report.frames,
                report.intervals,
                report.p50_ms,
                report.p99_ms,
                report.max_ms,
                report.mean_ms,
                report.stddev_ms,
                report.late,
                report.missed,
                report.acquire_p50_ms,
                report.acquire_p99_ms,
                report.acquire_max_ms,
                report.acquire_mean_ms,
                report.present_call_p50_ms,
                report.present_call_p99_ms,
                report.present_call_max_ms,
                report.viewport_p50_ms,
                report.viewport_p99_ms,
                report.viewport_max_ms,
                report.paint_p50_ms,
                report.paint_p99_ms,
                report.paint_max_ms,
                report.shown_intervals,
                shown(report.shown_p50_ms),
                shown(report.shown_p99_ms),
                shown(report.shown_max_ms),
                report.discarded,
            );
        }
        out
    }

    pub fn finish(&mut self) {
        self.write();
    }

    fn write(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut part = path.clone().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        let written =
            std::fs::write(&part, self.lines()).and_then(|()| std::fs::rename(&part, path));
        if let Err(error) = written {
            eprintln!("{PACING_VAR}: cannot write {}: {error}", path.display());
        }
    }
}

impl Drop for Pacing {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    fn setting(mode: &str, vrr: bool) -> Setting {
        Setting {
            host: "wayland",
            present_mode: mode.to_string(),
            frame_latency: 1,
            vrr,
            vrr_in_effect: if vrr { None } else { Some(false) },
            frame_rate: "uncapped",
        }
    }

    #[test]
    fn intervals_count_only_between_chained_presents_and_late_is_over_the_budget() {
        let mut presents = Presents::new(GUI_BUDGET_NS);
        let mut now = 0;
        for _ in 0..99 {
            presents.presented(now);
            now += 4 * MS;
        }
        presents.presented(now);
        presents.idle();
        now += 5_000 * MS;
        presents.presented(now);
        now += 17 * MS;
        presents.presented(now);
        now += GUI_BUDGET_NS;
        presents.presented(now);
        let report = presents.report();
        assert_eq!(report.frames, 103);
        assert_eq!(report.intervals, 101);
        assert_eq!(report.late, 1);
        assert!((report.p50_ms - 4.0).abs() < 0.011);
        assert!((report.p99_ms - 8.33).abs() < 0.011);
        assert!((report.max_ms - 17.0).abs() < 1e-9);
        assert!(!report.holds());
        assert!((report.budget_ms - 8.33).abs() < 1e-9);
    }

    #[test]
    fn a_steady_run_holds_a_whole_missed_refresh_counts_apart_and_an_empty_run_is_zeros() {
        let mut presents = Presents::new(GUI_BUDGET_NS);
        for frame in 0..1_000u64 {
            presents.presented(frame * 4_167_000 + (frame % 3) * 20_000);
        }
        let report = presents.report();
        assert!(report.holds());
        assert!(report.p99_ms <= report.max_ms);
        assert_eq!(Presents::new(TUI_BUDGET_NS).report().p99_ms, 0.0);
        let mut gap = Presents::new(GUI_BUDGET_NS);
        gap.presented(0);
        gap.presented(12_600_000);
        gap.presented(22_600_000);
        let report = gap.report();
        assert_eq!((report.late, report.missed), (2, 1));
    }

    #[test]
    fn each_setting_is_its_own_line_and_the_file_is_replaced_whole() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/gpu-tests/pacing");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pacing.jsonl");
        let _ = std::fs::remove_file(&path);
        let mut pacing = Pacing::to_file(&path, GUI_BUDGET_NS);
        pacing.set(setting("Fifo", false));
        pacing.presented(0);
        pacing.presented(16 * MS);
        pacing.set(setting("Fifo", false));
        pacing.presented(32 * MS);
        pacing.set(setting("Mailbox", true));
        pacing.presented(40 * MS);
        pacing.presented(44 * MS);
        drop(pacing);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "{\"schema_version\":2,\"host\":\"wayland\",\"present_mode\":\"Fifo\",\"frame_latency\":1,\"vrr\":false,\"vrr_in_effect\":false,\"frame_rate\":\"uncapped\",\"budget_ms\":8.330,\"frames\":3,\"intervals\":2,\"p50_ms\":16.000,\"p99_ms\":16.000,\"max_ms\":16.000,\"mean_ms\":16.000,\"stddev_ms\":0.000,\"late\":2,\"missed\":2,\"acquire_p50_ms\":0.000,\"acquire_p99_ms\":0.000,\"acquire_max_ms\":0.000,\"acquire_mean_ms\":0.000,\"present_call_p50_ms\":0.000,\"present_call_p99_ms\":0.000,\"present_call_max_ms\":0.000,\"viewport_p50_ms\":0.000,\"viewport_p99_ms\":0.000,\"viewport_max_ms\":0.000,\"paint_p50_ms\":0.000,\"paint_p99_ms\":0.000,\"paint_max_ms\":0.000,\"shown_intervals\":0,\"shown_p50_ms\":null,\"shown_p99_ms\":null,\"shown_max_ms\":null,\"discarded\":0}"
        );
        assert!(lines[1].contains("\"present_mode\":\"Mailbox\",\"frame_latency\":1,\"vrr\":true"));
        assert!(lines[1].contains("\"frames\":2,\"intervals\":1,\"p50_ms\":4.000"));
        assert!(!dir.join("pacing.jsonl.part").exists());
    }

    fn shown(tag: u64, at_ns: u64, refresh_ns: u64) -> Shown {
        Shown {
            tag,
            at_ns: Some(at_ns),
            refresh_ns: Some(refresh_ns),
        }
    }

    #[test]
    fn shown_intervals_chain_within_a_run_and_a_discarded_frame_is_counted_apart() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/gpu-tests/shown");
        std::fs::create_dir_all(&dir).unwrap();
        let mut pacing = Pacing::to_file(&dir.join("pacing.jsonl"), GUI_BUDGET_NS);
        pacing.set(setting("Mailbox", true));
        let run = pacing.tag().unwrap();
        pacing.presented(0);
        pacing.acquired(MS / 2);
        pacing.present_call(MS / 4);
        for (frame, at) in [0, 8, 16, 33].into_iter().enumerate() {
            pacing.shown(shown(run, at * MS, 4_166_667));
            pacing.presented((frame as u64 + 1) * 8 * MS);
        }
        pacing.shown(Shown {
            tag: run,
            at_ns: None,
            refresh_ns: None,
        });
        pacing.idle();
        let next = pacing.tag().unwrap();
        assert_ne!(next, run);
        pacing.shown(shown(next, 5_000 * MS, 4_166_667));
        pacing.shown(shown(next, 5_008 * MS, 4_166_667));
        let report = pacing.sections()[0].1;
        assert_eq!((report.shown_intervals, report.discarded), (4, 1));
        assert!((report.shown_max_ms - 17.0).abs() < 1e-9);
        assert!((report.acquire_max_ms - 0.5).abs() < 1e-9);
        assert!((report.present_call_max_ms - 0.25).abs() < 1e-9);
        pacing.set(setting("Fifo", false));
        pacing.shown(shown(next, 5_016 * MS, 4_166_667));
        pacing.presented(6_000 * MS);
        let lines = pacing.lines();
        let last = lines.lines().last().unwrap();
        assert!(last.contains("\"shown_intervals\":0,\"shown_p50_ms\":null"));
        assert!(last.contains("\"vrr\":false,\"vrr_in_effect\":false"));
    }

    #[test]
    fn a_fixed_refresh_reads_as_vrr_not_in_effect_and_a_free_one_as_in_effect() {
        let refresh = 4_166_667;
        let mut fixed = VrrProbe::default();
        let mut at = 0;
        let mut changed = 0;
        for frame in 0..=240u64 {
            at += refresh * (1 + frame % 3 / 2) + (frame % 5) * 40_000;
            changed += usize::from(fixed.shown(shown(0, at, refresh)));
        }
        assert_eq!((fixed.verdict(), changed), (Some(false), 1));
        let mut free = VrrProbe::default();
        let mut at = 0;
        for frame in 0..=240u64 {
            at += 5_300_000 + (frame % 7) * 310_000;
            free.shown(shown(0, at, refresh));
        }
        assert_eq!(free.verdict(), Some(true));
        let mut variable = VrrProbe::default();
        for frame in 0..=240u64 {
            variable.shown(shown(0, frame * 8 * MS, 0));
        }
        assert_eq!(variable.verdict(), Some(true));
        let mut headless = VrrProbe::default();
        headless.set_refresh(Some(240_000));
        for frame in 0..=240u64 {
            headless.shown(shown(0, frame * 4_166_667 + (frame % 4) * 200_000, 0));
        }
        assert_eq!(headless.verdict(), Some(false));
        variable.reset();
        assert_eq!(variable.verdict(), None);
        let mut flapping = VrrProbe::default();
        let mut at = 0;
        for frame in 0..480u64 {
            at += if frame / 120 % 2 == 0 {
                refresh
            } else {
                5_300_000
            };
            flapping.shown(shown(0, at, refresh));
        }
        assert_eq!(flapping.verdict(), None);
    }

    #[test]
    fn off_writes_nothing() {
        let mut pacing = Pacing::off();
        assert!(!pacing.on());
        pacing.set(setting("Fifo", true));
        pacing.presented(0);
        assert!(pacing.sections().is_empty());
    }
}
