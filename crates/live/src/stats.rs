use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Instant;

use pfx_gpu::screens::{Device, SystemSource, detect};

pub const SCHEMA_VERSION: u32 = 1;
pub const ENV: &str = "PFX_FRAME_STATS";
pub const CHANNEL: usize = 256;
const KEPT_FRAMES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    Full,
    Partial,
    Reproject,
    Idle,
}

impl FrameKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::Reproject => "reproject",
            Self::Idle => "idle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub tick: u64,
    pub t_ms: f64,
    pub sim_ms: Option<f64>,
    pub submit_ms: f64,
    pub gpu_ms: Option<f64>,
    pub interval_ms: Option<f64>,
    pub kind: FrameKind,
    pub size: [u32; 2],
    pub hud_ms: Option<f64>,
}

struct Line {
    sample: Sample,
    gpu: Option<(u64, f64)>,
    device: Option<&'static str>,
}

fn number(out: &mut String, value: f64) {
    if value.is_finite() {
        let _ = write!(out, "{:.3}", value.max(0.0));
    } else {
        out.push_str("null");
    }
}

fn optional(out: &mut String, value: Option<f64>) {
    match value {
        Some(value) => number(out, value),
        None => out.push_str("null"),
    }
}

impl Line {
    fn json(&self) -> String {
        let sample = &self.sample;
        let mut out = format!(
            "{{\"schema_version\":{SCHEMA_VERSION},\"tick\":{},\"t_ms\":",
            sample.tick
        );
        number(&mut out, sample.t_ms);
        out.push_str(",\"sim_ms\":");
        optional(&mut out, sample.sim_ms);
        out.push_str(",\"submit_ms\":");
        number(&mut out, sample.submit_ms);
        out.push_str(",\"gpu_ms\":");
        optional(&mut out, self.gpu.map(|(_, ms)| ms));
        out.push_str(",\"gpu_tick\":");
        match self.gpu {
            Some((tick, _)) => {
                let _ = write!(out, "{tick}");
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"present_interval_ms\":");
        optional(&mut out, sample.interval_ms);
        let _ = write!(
            out,
            ",\"kind\":\"{}\",\"size\":[{},{}],\"device\":",
            sample.kind.name(),
            sample.size[0],
            sample.size[1]
        );
        match self.device {
            Some(label) => {
                let _ = write!(out, "\"{label}\"");
            }
            None => out.push_str("null"),
        }
        out.push_str(",\"hud_ms\":");
        optional(&mut out, sample.hud_ms);
        out.push('}');
        out
    }
}

fn end_json(frames: u64, dropped: u64) -> String {
    format!(
        "{{\"schema_version\":{SCHEMA_VERSION},\"end\":true,\"frames\":{frames},\"dropped\":{dropped}}}"
    )
}

enum Message {
    Frame(Line),
    End(String),
}

fn write_lines(receiver: Receiver<Message>, mut out: Box<dyn Write + Send>) {
    loop {
        let message = match receiver.try_recv() {
            Ok(message) => message,
            Err(TryRecvError::Empty) => {
                if out.flush().is_err() {
                    return;
                }
                match receiver.recv() {
                    Ok(message) => message,
                    Err(_) => return,
                }
            }
            Err(TryRecvError::Disconnected) => {
                let _ = out.flush();
                return;
            }
        };
        let (text, last) = match message {
            Message::Frame(line) => (line.json(), false),
            Message::End(text) => (text, true),
        };
        if writeln!(out, "{text}").is_err() {
            return;
        }
        if last {
            let _ = out.flush();
            return;
        }
    }
}

struct Open {
    begin: Instant,
    kind: FrameKind,
}

struct State {
    sender: Option<SyncSender<Message>>,
    thread: Option<JoinHandle<()>>,
    epoch: Option<Instant>,
    open: Option<Open>,
    sim_start: Option<Instant>,
    sim_total: Option<f64>,
    last_submit: Option<Instant>,
    pending: Option<Line>,
    frames: u64,
    ticks: VecDeque<(u64, u64)>,
    gpu: Option<(u64, f64)>,
    gpu_reported: bool,
    hud_start: Option<Instant>,
    keep: usize,
    recent: VecDeque<Sample>,
    device: Option<&'static str>,
}

struct Shared {
    state: Mutex<State>,
    dropped: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn emit(&self, state: &mut State, mut line: Line) {
        if !state.gpu_reported {
            line.gpu = state.gpu;
            state.gpu_reported = state.gpu.is_some();
        }
        let Some(sender) = &state.sender else {
            return;
        };
        match sender.try_send(Message::Frame(line)) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn finish(&self) {
        let (thread, sender) = {
            let mut state = self.lock();
            if let Some(line) = state.pending.take() {
                self.emit(&mut state, line);
            }
            let end = end_json(state.frames, self.dropped.load(Ordering::Relaxed));
            if let Some(sender) = &state.sender {
                let _ = sender.send(Message::End(end));
            }
            (state.thread.take(), state.sender.take())
        };
        drop(sender);
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Clone, Default)]
pub struct FrameStats {
    shared: Option<Arc<Shared>>,
}

impl std::fmt::Debug for FrameStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameStats")
            .field("enabled", &self.enabled())
            .finish()
    }
}

impl FrameStats {
    pub fn off() -> Self {
        Self::default()
    }

    pub fn from_env() -> Self {
        Self::from_value(std::env::var_os(ENV).as_deref())
    }

    pub fn from_value(value: Option<&OsStr>) -> Self {
        let Some(path) = value.filter(|value| !value.is_empty()) else {
            return Self::off();
        };
        match Self::to_file(Path::new(path)) {
            Ok(stats) => stats,
            Err(error) => {
                eprintln!("pfx: {ENV}: {error}");
                Self::off()
            }
        }
    }

    pub fn to_file(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let stats = Self::to_writer(Box::new(BufWriter::new(file)), CHANNEL);
        stats.set_device(Some(detect(&SystemSource).device));
        Ok(stats)
    }

    pub fn set_device(&self, device: Option<Device>) {
        if let Some(shared) = &self.shared {
            shared.lock().device = device.map(Device::label);
        }
    }

    pub fn device(&self) -> Option<&'static str> {
        self.shared.as_ref().and_then(|shared| shared.lock().device)
    }

    pub fn to_writer(out: Box<dyn Write + Send>, channel: usize) -> Self {
        let (sender, receiver) = sync_channel(channel.max(1));
        let thread = std::thread::Builder::new()
            .name("pfx frame stats".into())
            .spawn(move || write_lines(receiver, out))
            .ok();
        let sender = thread.is_some().then_some(sender);
        Self::with(sender, thread)
    }

    pub fn measure() -> Self {
        Self::with(None, None)
    }

    fn with(sender: Option<SyncSender<Message>>, thread: Option<JoinHandle<()>>) -> Self {
        Self {
            shared: Some(Arc::new(Shared {
                state: Mutex::new(State {
                    sender,
                    thread,
                    epoch: None,
                    open: None,
                    sim_start: None,
                    sim_total: None,
                    last_submit: None,
                    pending: None,
                    frames: 0,
                    ticks: VecDeque::new(),
                    gpu: None,
                    gpu_reported: true,
                    hud_start: None,
                    keep: 0,
                    recent: VecDeque::new(),
                    device: None,
                }),
                dropped: AtomicU64::new(0),
            })),
        }
    }

    pub fn enabled(&self) -> bool {
        self.shared.is_some()
    }

    pub fn writes(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|shared| shared.lock().sender.is_some())
    }

    pub fn keep_recent(&self, frames: usize) {
        if let Some(shared) = &self.shared {
            let mut state = shared.lock();
            state.keep = state.keep.max(frames);
        }
    }

    pub fn recent(&self) -> Vec<Sample> {
        self.with_recent(<[Sample]>::to_vec)
    }

    pub(crate) fn with_recent<R>(&self, read: impl FnOnce(&[Sample]) -> R) -> R {
        match &self.shared {
            Some(shared) => read(shared.lock().recent.make_contiguous()),
            None => read(&[]),
        }
    }

    pub fn hud_begin(&self) {
        if self.shared.is_some() {
            self.hud_begin_at(Instant::now());
        }
    }

    pub fn hud_end(&self) {
        if self.shared.is_some() {
            self.hud_end_at(Instant::now());
        }
    }

    pub fn hud_begin_at(&self, now: Instant) {
        if let Some(shared) = &self.shared {
            shared.lock().hud_start = Some(now);
        }
    }

    pub fn hud_end_at(&self, now: Instant) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        let Some(start) = state.hud_start.take() else {
            return;
        };
        let span = millis(start, now);
        let Some(line) = state.pending.as_mut() else {
            return;
        };
        let total = line.sample.hud_ms.unwrap_or(0.0) + span;
        line.sample.hud_ms = Some(total);
        let tick = line.sample.tick;
        if let Some(sample) = state.recent.back_mut().filter(|sample| sample.tick == tick) {
            sample.hud_ms = Some(total);
        }
    }

    pub fn frames(&self) -> u64 {
        self.shared
            .as_ref()
            .map_or(0, |shared| shared.lock().frames)
    }

    pub fn dropped(&self) -> u64 {
        self.shared
            .as_ref()
            .map_or(0, |shared| shared.dropped.load(Ordering::Relaxed))
    }

    pub fn finish(&self) {
        if let Some(shared) = &self.shared {
            shared.finish();
        }
    }

    pub fn sim_begin(&self) {
        if self.shared.is_some() {
            self.sim_begin_at(Instant::now());
        }
    }

    pub fn sim_end(&self) {
        if self.shared.is_some() {
            self.sim_end_at(Instant::now());
        }
    }

    pub fn sim_begin_at(&self, now: Instant) {
        if let Some(shared) = &self.shared {
            shared.lock().sim_start = Some(now);
        }
    }

    pub fn sim_end_at(&self, now: Instant) {
        if let Some(shared) = &self.shared {
            let mut state = shared.lock();
            if let Some(start) = state.sim_start.take() {
                let span = millis(start, now);
                state.sim_total = Some(state.sim_total.unwrap_or(0.0) + span);
            }
        }
    }

    pub fn begin(&self, kind: FrameKind) {
        if self.shared.is_some() {
            self.begin_at(kind, Instant::now());
        }
    }

    pub fn begin_at(&self, kind: FrameKind, now: Instant) {
        if let Some(shared) = &self.shared {
            let mut state = shared.lock();
            state.epoch.get_or_insert(now);
            state.open = Some(Open { begin: now, kind });
        }
    }

    pub fn discard(&self) {
        if let Some(shared) = &self.shared {
            shared.lock().open = None;
        }
    }

    pub fn submitted(&self, frame: u64, size: [u32; 2]) {
        if self.shared.is_some() {
            self.submitted_at(frame, size, Instant::now());
        }
    }

    pub fn submitted_at(&self, frame: u64, size: [u32; 2], now: Instant) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        let Some(open) = state.open.take() else {
            return;
        };
        if let Some(line) = state.pending.take() {
            shared.emit(&mut state, line);
        }
        let epoch = state.epoch.unwrap_or(open.begin);
        let tick = state.frames;
        state.frames += 1;
        let interval_ms = state.last_submit.map(|last| millis(last, now));
        state.last_submit = Some(now);
        let sim_ms = state.sim_total.take();
        state.ticks.push_back((frame, tick));
        while state.ticks.len() > KEPT_FRAMES {
            state.ticks.pop_front();
        }
        let sample = Sample {
            tick,
            t_ms: millis(epoch, open.begin),
            sim_ms,
            submit_ms: millis(open.begin, now),
            gpu_ms: None,
            interval_ms,
            kind: open.kind,
            size,
            hud_ms: None,
        };
        if state.keep > 0 {
            state.recent.push_back(sample);
            while state.recent.len() > state.keep {
                state.recent.pop_front();
            }
        }
        state.pending = Some(Line {
            sample,
            gpu: None,
            device: state.device,
        });
    }

    pub fn gpu(&self, frame: u64, milliseconds: f64) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        let Some(tick) = state
            .ticks
            .iter()
            .find(|(submitted, _)| *submitted == frame)
            .map(|(_, tick)| *tick)
        else {
            return;
        };
        if state.gpu.is_none_or(|(latest, _)| latest < tick) {
            state.gpu = Some((tick, milliseconds));
            state.gpu_reported = false;
        }
        if let Some(sample) = state
            .recent
            .iter_mut()
            .rev()
            .find(|sample| sample.tick == tick)
        {
            sample.gpu_ms = Some(milliseconds);
        }
    }
}

fn millis(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Sink {
        fn lines(&self) -> Vec<serde_json::Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }

    struct Gate {
        sink: Sink,
        open: mpsc::Receiver<()>,
        passed: bool,
    }

    impl Write for Gate {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.passed {
                let _ = self.open.recv();
                self.passed = true;
            }
            self.sink.write(bytes)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn ms(base: Instant, milliseconds: u64) -> Instant {
        base + Duration::from_millis(milliseconds)
    }

    fn frame(stats: &FrameStats, base: Instant, number: u64, at: u64, kind: FrameKind) {
        stats.begin_at(kind, ms(base, at));
        stats.submitted_at(number, [1280, 800], ms(base, at + 2));
    }

    #[test]
    fn lines_parse_in_order_and_the_end_line_closes_the_file() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        frame(&stats, base, 0, 0, FrameKind::Full);
        frame(&stats, base, 1, 17, FrameKind::Partial);
        frame(&stats, base, 2, 33, FrameKind::Reproject);
        frame(&stats, base, 3, 50, FrameKind::Idle);
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines.len(), 5);
        for (index, line) in lines[..4].iter().enumerate() {
            assert_eq!(line["schema_version"], 1);
            assert_eq!(line["tick"], index as u64);
            assert_eq!(line["size"], serde_json::json!([1280, 800]));
            assert!(line["device"].is_null());
            assert!(line["hud_ms"].is_null());
            assert!(line["sim_ms"].is_null());
            assert!(line["gpu_ms"].is_null());
            assert!(line["gpu_tick"].is_null());
            assert_eq!(line["submit_ms"], 2.0);
        }
        let kinds: Vec<_> = lines[..4]
            .iter()
            .map(|line| line["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["full", "partial", "reproject", "idle"]);
        assert_eq!(lines[0]["t_ms"], 0.0);
        assert_eq!(lines[3]["t_ms"], 50.0);
        assert!(lines[0]["present_interval_ms"].is_null());
        assert_eq!(lines[1]["present_interval_ms"], 17.0);
        assert_eq!(lines[3]["present_interval_ms"], 17.0);
        assert_eq!(
            lines[4],
            serde_json::json!({"schema_version": 1, "end": true, "frames": 4, "dropped": 0})
        );
    }

    #[test]
    fn sim_time_goes_to_the_next_frame_and_gpu_time_to_the_next_line() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        stats.sim_begin_at(ms(base, 0));
        stats.sim_end_at(ms(base, 3));
        stats.sim_begin_at(ms(base, 4));
        stats.sim_end_at(ms(base, 5));
        frame(&stats, base, 10, 6, FrameKind::Full);
        frame(&stats, base, 11, 20, FrameKind::Full);
        stats.gpu(10, 4.5);
        frame(&stats, base, 12, 40, FrameKind::Full);
        frame(&stats, base, 13, 60, FrameKind::Full);
        stats.gpu(12, 5.5);
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines[0]["sim_ms"], 4.0);
        assert!(lines[1]["sim_ms"].is_null());
        assert!(lines[0]["gpu_ms"].is_null());
        assert_eq!(lines[1]["gpu_ms"], 4.5);
        assert_eq!(lines[1]["gpu_tick"], 0);
        assert!(lines[2]["gpu_ms"].is_null());
        assert_eq!(lines[3]["gpu_ms"], 5.5);
        assert_eq!(lines[3]["gpu_tick"], 2);
    }

    #[test]
    fn gpu_time_of_a_frame_that_was_never_presented_is_ignored() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        frame(&stats, base, 4, 0, FrameKind::Full);
        stats.gpu(3, 9.0);
        frame(&stats, base, 5, 20, FrameKind::Full);
        stats.finish();
        let lines = sink.lines();
        assert!(lines[0]["gpu_ms"].is_null());
        assert!(lines[1]["gpu_ms"].is_null());
    }

    #[test]
    fn a_full_channel_drops_lines_and_counts_them() {
        let sink = Sink::default();
        let (open, gate) = mpsc::channel();
        let stats = FrameStats::to_writer(
            Box::new(Gate {
                sink: sink.clone(),
                open: gate,
                passed: false,
            }),
            2,
        );
        let base = Instant::now();
        for number in 0..10 {
            frame(&stats, base, number, number * 17, FrameKind::Full);
        }
        let dropped = stats.dropped();
        assert!(dropped >= 5, "dropped {dropped}");
        open.send(()).unwrap();
        stats.finish();
        let lines = sink.lines();
        let end = lines.last().unwrap();
        assert_eq!(end["end"], true);
        assert_eq!(end["frames"], 10);
        let dropped = end["dropped"].as_u64().unwrap();
        assert!(dropped >= 5);
        assert_eq!(lines.len() as u64 - 1 + dropped, 10);
    }

    #[test]
    fn nothing_is_written_when_unset() {
        let stats = FrameStats::from_value(None);
        assert!(!stats.enabled());
        let blank = FrameStats::from_value(Some(OsStr::new("")));
        assert!(!blank.enabled());
        let base = Instant::now();
        frame(&stats, base, 0, 0, FrameKind::Full);
        stats.sim_begin_at(base);
        stats.sim_end_at(ms(base, 1));
        stats.gpu(0, 1.0);
        stats.finish();
        assert_eq!(stats.frames(), 0);
        assert_eq!(stats.dropped(), 0);
    }

    #[test]
    fn a_frame_without_a_begin_is_not_counted() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 4);
        let base = Instant::now();
        stats.submitted_at(0, [8, 8], ms(base, 1));
        stats.begin_at(FrameKind::Full, ms(base, 2));
        stats.discard();
        stats.submitted_at(1, [8, 8], ms(base, 3));
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["frames"], 0);
    }

    #[test]
    fn hud_time_lands_on_the_frame_it_drew_over_and_is_null_otherwise() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        frame(&stats, base, 0, 0, FrameKind::Full);
        stats.hud_begin_at(ms(base, 3));
        stats.hud_end_at(ms(base, 4));
        frame(&stats, base, 1, 17, FrameKind::Full);
        frame(&stats, base, 2, 33, FrameKind::Full);
        stats.hud_begin_at(ms(base, 36));
        stats.hud_end_at(ms(base, 38));
        stats.hud_end_at(ms(base, 40));
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines[0]["hud_ms"], 1.0);
        assert!(lines[1]["hud_ms"].is_null());
        assert_eq!(lines[2]["hud_ms"], 2.0);
        let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        let first = text.lines().next().unwrap();
        assert!(
            first.ends_with(",\"device\":null,\"hud_ms\":1.000}"),
            "{first}"
        );
    }

    #[test]
    fn measuring_keeps_recent_frames_without_writing() {
        let stats = FrameStats::measure();
        assert!(stats.enabled());
        assert!(!stats.writes());
        let base = Instant::now();
        frame(&stats, base, 0, 0, FrameKind::Full);
        assert!(stats.recent().is_empty());
        stats.keep_recent(3);
        stats.keep_recent(2);
        stats.sim_begin_at(ms(base, 10));
        stats.sim_end_at(ms(base, 12));
        frame(&stats, base, 1, 17, FrameKind::Partial);
        stats.hud_begin_at(ms(base, 20));
        stats.hud_end_at(ms(base, 21));
        frame(&stats, base, 2, 33, FrameKind::Idle);
        stats.gpu(1, 3.5);
        frame(&stats, base, 3, 50, FrameKind::Full);
        frame(&stats, base, 4, 67, FrameKind::Full);
        let recent = stats.recent();
        assert_eq!(
            recent.iter().map(|sample| sample.tick).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert_eq!(recent[0].kind, FrameKind::Idle);
        assert_eq!(recent[0].interval_ms, Some(16.0));
        assert!(recent[0].gpu_ms.is_none());
        stats.gpu(3, 4.25);
        let recent = stats.recent();
        assert_eq!(recent[1].gpu_ms, Some(4.25));
        assert_eq!(recent[1].submit_ms, 2.0);
        let early = FrameStats::measure();
        early.keep_recent(4);
        frame(&early, base, 0, 0, FrameKind::Full);
        early.sim_begin_at(ms(base, 5));
        early.sim_end_at(ms(base, 7));
        frame(&early, base, 1, 17, FrameKind::Full);
        early.hud_begin_at(ms(base, 20));
        early.hud_end_at(ms(base, 23));
        early.gpu(0, 1.5);
        let recent = early.recent();
        assert_eq!(recent[0].gpu_ms, Some(1.5));
        assert!(recent[0].hud_ms.is_none());
        assert_eq!(recent[1].sim_ms, Some(2.0));
        assert_eq!(recent[1].hud_ms, Some(3.0));
        stats.finish();
        assert_eq!(stats.frames(), 5);
    }

    #[test]
    fn the_env_value_opens_the_file_and_appends() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("frame-stats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stats.jsonl");
        let _ = std::fs::remove_file(&path);
        for _ in 0..2 {
            let stats = FrameStats::from_value(Some(path.as_os_str()));
            assert!(stats.enabled());
            let base = Instant::now();
            frame(&stats, base, 0, 0, FrameKind::Full);
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1]["end"], true);
        assert_eq!(lines[3]["end"], true);
        assert_eq!(lines[0]["device"], detect(&SystemSource).device.label());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_device_label_fills_every_line_once_set() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        assert_eq!(stats.device(), None);
        let base = Instant::now();
        frame(&stats, base, 0, 0, FrameKind::Full);
        stats.set_device(Some(Device::SteamDeck {
            model: pfx_gpu::screens::DeckModel::Oled,
        }));
        assert_eq!(stats.device(), Some("steam-deck-oled"));
        frame(&stats, base, 1, 17, FrameKind::Full);
        frame(&stats, base, 2, 33, FrameKind::Full);
        stats.finish();
        let lines = sink.lines();
        assert!(lines[0]["device"].is_null());
        assert_eq!(lines[1]["device"], "steam-deck-oled");
        assert_eq!(lines[2]["device"], "steam-deck-oled");
        assert!(FrameStats::off().device().is_none());
    }
}
