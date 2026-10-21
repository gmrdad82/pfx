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

pub const SCHEMA_VERSION: u32 = 2;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Game,
    Ui,
    Splash,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Ui => "ui",
            Self::Splash => "splash",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub tick: u64,
    pub t_ms: f64,
    pub sim_ms: Option<f64>,
    pub submit_ms: f64,
    pub prepare_ms: Option<f64>,
    pub encode_ms: f64,
    pub poll_ms: f64,
    pub queue_ms: f64,
    pub wait_ms: Option<f64>,
    pub gpu_ms: Option<f64>,
    pub interval_ms: Option<f64>,
    pub kind: FrameKind,
    pub size: [u32; 2],
    pub hud_ms: Option<f64>,
    pub frame_ms: Option<f64>,
    pub acquire_ms: Option<f64>,
    pub source: Source,
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
        out.push_str(",\"prepare_ms\":");
        optional(&mut out, sample.prepare_ms);
        out.push_str(",\"encode_ms\":");
        number(&mut out, sample.encode_ms);
        out.push_str(",\"poll_ms\":");
        number(&mut out, sample.poll_ms);
        out.push_str(",\"queue_ms\":");
        number(&mut out, sample.queue_ms);
        out.push_str(",\"wait_ms\":");
        optional(&mut out, sample.wait_ms);
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
        out.push_str(",\"frame_ms\":");
        optional(&mut out, sample.frame_ms);
        out.push_str(",\"acquire_ms\":");
        optional(&mut out, sample.acquire_ms);
        let _ = write!(out, ",\"source\":\"{}\"}}", sample.source.name());
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
    prepared: Option<Instant>,
    poll_ms: f64,
    queue_ms: f64,
}

struct Presenting {
    begin: Instant,
    source: Source,
    drew: Option<FrameKind>,
    submit_ms: f64,
    prepare_ms: Option<f64>,
    encode_ms: f64,
    poll_ms: f64,
    queue_ms: f64,
    hud_ms: Option<f64>,
    wait_ms: Option<f64>,
    acquire_ms: Option<f64>,
}

impl Presenting {
    fn new(begin: Instant, source: Source) -> Self {
        Self {
            begin,
            source,
            drew: None,
            submit_ms: 0.0,
            prepare_ms: None,
            encode_ms: 0.0,
            poll_ms: 0.0,
            queue_ms: 0.0,
            hud_ms: None,
            wait_ms: None,
            acquire_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Resolved {
    tick: u64,
    ms: f64,
    count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Submission {
    submitter: u64,
    frame: u64,
    tick: u64,
    timed: bool,
}

fn rank(kind: FrameKind) -> u8 {
    match kind {
        FrameKind::Idle => 0,
        FrameKind::Reproject => 1,
        FrameKind::Partial => 2,
        FrameKind::Full => 3,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Poll,
    Queue,
    Wait,
    Acquire,
}

struct State {
    sender: Option<SyncSender<Message>>,
    thread: Option<JoinHandle<()>>,
    epoch: Option<Instant>,
    open: Option<Open>,
    presenting: Option<Presenting>,
    sim_start: Option<Instant>,
    sim_total: Option<f64>,
    last_submit: Option<Instant>,
    pending: Option<Line>,
    frames: u64,
    ticks: VecDeque<Submission>,
    gpu: Option<Resolved>,
    gpu_reported: bool,
    superseded: Option<Resolved>,
    hud_start: Option<Instant>,
    keep: usize,
    recent: VecDeque<Sample>,
    device: Option<&'static str>,
}

impl State {
    fn submissions(&self, tick: u64) -> usize {
        self.ticks
            .iter()
            .filter(|kept| kept.tick == tick && kept.timed)
            .count()
    }

    fn forget(&mut self, tick: u64) {
        self.ticks.retain(|kept| kept.tick != tick);
        if self.gpu.is_some_and(|resolved| resolved.tick == tick) {
            self.gpu = None;
            self.gpu_reported = true;
        }
        if self
            .superseded
            .is_some_and(|resolved| resolved.tick == tick)
        {
            self.superseded = None;
        }
    }

    fn written(&mut self) -> Option<Resolved> {
        let held = self.gpu.filter(|resolved| {
            !self.gpu_reported && resolved.count >= self.submissions(resolved.tick)
        });
        if held.is_some() {
            self.gpu_reported = true;
        }
        let written = held.or(self.superseded);
        self.superseded = None;
        let resolved = written?;
        if let Some(sample) = self
            .recent
            .iter_mut()
            .rev()
            .find(|sample| sample.tick == resolved.tick)
        {
            sample.gpu_ms = Some(resolved.ms);
        }
        Some(resolved)
    }
}

struct Shared {
    state: Mutex<State>,
    dropped: AtomicU64,
    submitters: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn emit(&self, state: &mut State, mut line: Line) {
        if let Some(resolved) = state.written() {
            line.gpu = Some((resolved.tick, resolved.ms));
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

    fn line(&self, state: &mut State, mut sample: Sample, begin: Instant, now: Instant) {
        if let Some(line) = state.pending.take() {
            self.emit(state, line);
        }
        let epoch = state.epoch.unwrap_or(begin);
        state.frames += 1;
        sample.t_ms = millis(epoch, begin);
        sample.interval_ms = state.last_submit.map(|last| millis(last, now));
        state.last_submit = Some(now);
        sample.sim_ms = state.sim_total.take();
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

    fn finish(&self) {
        let (thread, sender) = {
            let mut state = self.lock();
            if let Some(line) = state.pending.take() {
                self.emit(&mut state, line);
            }
            let end = end_json(state.frames, self.dropped.load(Ordering::Relaxed));
            if let Some(sender) = state.sender.as_ref().filter(|_| state.frames > 0) {
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
    submitter: u64,
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
                    presenting: None,
                    sim_start: None,
                    sim_total: None,
                    last_submit: None,
                    pending: None,
                    frames: 0,
                    ticks: VecDeque::new(),
                    gpu: None,
                    gpu_reported: true,
                    superseded: None,
                    hud_start: None,
                    keep: 0,
                    recent: VecDeque::new(),
                    device: None,
                }),
                dropped: AtomicU64::new(0),
                submitters: AtomicU64::new(1),
            })),
            submitter: 0,
        }
    }

    pub fn joined(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            submitter: self.shared.as_ref().map_or(0, |shared| {
                shared.submitters.fetch_add(1, Ordering::Relaxed)
            }),
        }
    }

    pub fn submitter(&self) -> u64 {
        self.submitter
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
        if let Some(presenting) = state.presenting.as_mut() {
            presenting.hud_ms = Some(presenting.hud_ms.unwrap_or(0.0) + span);
            return;
        }
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
            state.open = Some(Open {
                begin: now,
                kind,
                prepared: None,
                poll_ms: 0.0,
                queue_ms: 0.0,
            });
        }
    }

    pub fn now(&self) -> Option<Instant> {
        self.shared.as_ref().map(|_| Instant::now())
    }

    pub fn prepared(&self) {
        if self.shared.is_some() {
            self.prepared_at(Instant::now());
        }
    }

    pub fn prepared_at(&self, now: Instant) {
        if let Some(shared) = &self.shared
            && let Some(open) = shared.lock().open.as_mut()
        {
            open.prepared = Some(now);
        }
    }

    pub fn span(&self, span: Span, from: Option<Instant>) {
        if let Some(from) = from {
            self.span_at(span, from, Instant::now());
        }
    }

    pub fn span_at(&self, span: Span, from: Instant, to: Instant) {
        let Some(shared) = &self.shared else {
            return;
        };
        let ms = millis(from, to);
        let mut state = shared.lock();
        match span {
            Span::Poll => {
                if let Some(open) = state.open.as_mut() {
                    open.poll_ms += ms;
                }
            }
            Span::Queue => {
                if let Some(open) = state.open.as_mut() {
                    open.queue_ms += ms;
                }
            }
            Span::Wait => {
                if let Some(presenting) = state.presenting.as_mut() {
                    presenting.wait_ms = Some(presenting.wait_ms.unwrap_or(0.0) + ms);
                    return;
                }
                let Some(line) = state.pending.as_mut() else {
                    return;
                };
                let total = line.sample.wait_ms.unwrap_or(0.0) + ms;
                line.sample.wait_ms = Some(total);
                let tick = line.sample.tick;
                if let Some(sample) = state.recent.back_mut().filter(|sample| sample.tick == tick) {
                    sample.wait_ms = Some(total);
                }
            }
            Span::Acquire => {
                if let Some(presenting) = state.presenting.as_mut() {
                    presenting.acquire_ms = Some(presenting.acquire_ms.unwrap_or(0.0) + ms);
                }
            }
        }
    }

    pub fn discard(&self) {
        if let Some(shared) = &self.shared {
            shared.lock().open = None;
        }
    }

    pub fn submitted(&self, frame: u64, size: [u32; 2]) {
        self.submitted_as(frame, size, true);
    }

    pub fn submitted_as(&self, frame: u64, size: [u32; 2], timed: bool) {
        if self.shared.is_some() {
            self.submitted_as_at(frame, size, timed, Instant::now());
        }
    }

    pub fn submitted_at(&self, frame: u64, size: [u32; 2], now: Instant) {
        self.submitted_as_at(frame, size, true, now);
    }

    pub fn submitted_as_at(&self, frame: u64, size: [u32; 2], timed: bool, now: Instant) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        let Some(open) = state.open.take() else {
            return;
        };
        let tick = state.frames;
        state.ticks.push_back(Submission {
            submitter: self.submitter,
            frame,
            tick,
            timed,
        });
        while state.ticks.len() > KEPT_FRAMES {
            state.ticks.pop_front();
        }
        let submit_ms = millis(open.begin, now);
        let prepare_ms = open.prepared.map(|prepared| millis(open.begin, prepared));
        let encode_ms =
            (submit_ms - prepare_ms.unwrap_or(0.0) - open.poll_ms - open.queue_ms).max(0.0);
        if let Some(presenting) = state.presenting.as_mut() {
            presenting.submit_ms += submit_ms;
            presenting.prepare_ms = match (presenting.prepare_ms, prepare_ms) {
                (Some(held), Some(more)) => Some(held + more),
                (held, more) => held.or(more),
            };
            presenting.encode_ms += encode_ms;
            presenting.poll_ms += open.poll_ms;
            presenting.queue_ms += open.queue_ms;
            presenting.drew = Some(match presenting.drew {
                Some(kind) if rank(kind) >= rank(open.kind) => kind,
                _ => open.kind,
            });
            return;
        }
        let sample = Sample {
            tick,
            t_ms: 0.0,
            sim_ms: None,
            submit_ms,
            prepare_ms,
            encode_ms,
            poll_ms: open.poll_ms,
            queue_ms: open.queue_ms,
            wait_ms: None,
            gpu_ms: None,
            interval_ms: None,
            kind: open.kind,
            size,
            hud_ms: None,
            frame_ms: None,
            acquire_ms: None,
            source: Source::Game,
        };
        shared.line(&mut state, sample, open.begin, now);
    }

    pub fn present_begin(&self, source: Source) {
        if self.shared.is_some() {
            self.present_begin_at(source, Instant::now());
        }
    }

    pub fn present_begin_at(&self, source: Source, now: Instant) {
        if let Some(shared) = &self.shared {
            let mut state = shared.lock();
            state.epoch.get_or_insert(now);
            state.open = None;
            if state.presenting.is_some() {
                let tick = state.frames;
                state.forget(tick);
            }
            state.presenting = Some(Presenting::new(now, source));
        }
    }

    pub fn presenting(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|shared| shared.lock().presenting.is_some())
    }

    pub fn present_abandon(&self) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        state.open = None;
        if state.presenting.take().is_some() {
            let tick = state.frames;
            state.forget(tick);
        }
    }

    pub fn presented(&self, size: [u32; 2]) {
        if self.shared.is_some() {
            self.presented_at(size, Instant::now());
        }
    }

    pub fn presented_at(&self, size: [u32; 2], now: Instant) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        state.open = None;
        let Some(presenting) = state.presenting.take() else {
            return;
        };
        let source = match (presenting.drew, presenting.source) {
            (Some(_), Source::Ui) => Source::Game,
            (_, source) => source,
        };
        let sample = Sample {
            tick: state.frames,
            t_ms: 0.0,
            sim_ms: None,
            submit_ms: presenting.submit_ms,
            prepare_ms: presenting.prepare_ms,
            encode_ms: presenting.encode_ms,
            poll_ms: presenting.poll_ms,
            queue_ms: presenting.queue_ms,
            wait_ms: presenting.wait_ms,
            gpu_ms: None,
            interval_ms: None,
            kind: presenting.drew.unwrap_or(FrameKind::Full),
            size,
            hud_ms: presenting.hud_ms,
            frame_ms: Some(millis(presenting.begin, now)),
            acquire_ms: presenting.acquire_ms,
            source,
        };
        shared.line(&mut state, sample, presenting.begin, now);
    }

    pub fn gpu(&self, frame: u64, milliseconds: f64) {
        let Some(shared) = &self.shared else {
            return;
        };
        let mut state = shared.lock();
        let Some(tick) = state
            .ticks
            .iter()
            .find(|kept| kept.submitter == self.submitter && kept.frame == frame)
            .map(|kept| kept.tick)
        else {
            return;
        };
        match state.gpu {
            Some(held) if held.tick == tick => {
                if !state.gpu_reported {
                    state.gpu = Some(Resolved {
                        ms: held.ms + milliseconds,
                        count: held.count + 1,
                        ..held
                    });
                }
            }
            Some(held) if held.tick > tick => {}
            held => {
                if let Some(held) = held.filter(|_| !state.gpu_reported) {
                    state.superseded = Some(held);
                }
                state.gpu = Some(Resolved {
                    tick,
                    ms: milliseconds,
                    count: 1,
                });
                state.gpu_reported = false;
            }
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
            assert_eq!(line["schema_version"], 2);
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
            serde_json::json!({"schema_version": 2, "end": true, "frames": 4, "dropped": 0})
        );
    }

    #[test]
    fn submit_time_splits_into_prepare_encode_poll_and_queue_and_waits_land_on_the_frame() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        stats.begin_at(FrameKind::Full, ms(base, 0));
        stats.prepared_at(ms(base, 1));
        stats.span_at(Span::Poll, ms(base, 2), ms(base, 3));
        stats.span_at(Span::Queue, ms(base, 4), ms(base, 6));
        stats.submitted_at(0, [1280, 800], ms(base, 7));
        stats.span_at(Span::Wait, ms(base, 8), ms(base, 12));
        stats.begin_at(FrameKind::Full, ms(base, 17));
        stats.submitted_at(1, [1280, 800], ms(base, 19));
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines[0]["submit_ms"], 7.0);
        assert_eq!(lines[0]["prepare_ms"], 1.0);
        assert_eq!(lines[0]["encode_ms"], 3.0);
        assert_eq!(lines[0]["poll_ms"], 1.0);
        assert_eq!(lines[0]["queue_ms"], 2.0);
        assert_eq!(lines[0]["wait_ms"], 4.0);
        assert!(lines[1]["prepare_ms"].is_null());
        assert_eq!(lines[1]["encode_ms"], 2.0);
        assert_eq!(lines[1]["poll_ms"], 0.0);
        assert!(lines[1]["wait_ms"].is_null());
        let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        let first = text.lines().next().unwrap();
        assert!(
            first.contains(",\"submit_ms\":7.000,\"prepare_ms\":1.000,\"encode_ms\":3.000,\"poll_ms\":1.000,\"queue_ms\":2.000,\"wait_ms\":4.000,\"gpu_ms\":"),
            "{first}"
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
        assert_eq!(stats.frames(), 0);
        assert!(sink.lines().is_empty());
    }

    #[test]
    fn every_presented_frame_has_one_line_with_its_source_and_the_renderer_frames_fold_in() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        let size = [2560, 1440];
        stats.present_begin_at(Source::Splash, ms(base, 0));
        assert!(stats.presenting());
        stats.presented_at(size, ms(base, 3));
        assert!(!stats.presenting());
        stats.sim_begin_at(ms(base, 5));
        stats.sim_end_at(ms(base, 6));
        stats.present_begin_at(Source::Ui, ms(base, 8));
        stats.hud_begin_at(ms(base, 9));
        stats.hud_end_at(ms(base, 10));
        stats.presented_at(size, ms(base, 10));
        stats.present_begin_at(Source::Ui, ms(base, 16));
        stats.sim_begin_at(ms(base, 16));
        stats.sim_end_at(ms(base, 17));
        stats.begin_at(FrameKind::Partial, ms(base, 17));
        stats.span_at(Span::Poll, ms(base, 17), ms(base, 18));
        stats.submitted_at(7, [1280, 720], ms(base, 19));
        stats.begin_at(FrameKind::Full, ms(base, 20));
        stats.prepared_at(ms(base, 20));
        stats.span_at(Span::Queue, ms(base, 20), ms(base, 22));
        stats.submitted_at(8, [1280, 720], ms(base, 22));
        stats.begin_at(FrameKind::Idle, ms(base, 22));
        stats.discard();
        stats.span_at(Span::Acquire, ms(base, 22), ms(base, 23));
        stats.span_at(Span::Wait, ms(base, 23), ms(base, 24));
        stats.presented_at(size, ms(base, 24));
        stats.gpu(7, 2.5);
        stats.gpu(8, 1.5);
        stats.present_begin_at(Source::Ui, ms(base, 30));
        stats.begin_at(FrameKind::Full, ms(base, 31));
        stats.submitted_at(9, [1280, 720], ms(base, 32));
        stats.present_abandon();
        stats.presented_at(size, ms(base, 33));
        stats.present_begin_at(Source::Ui, ms(base, 40));
        stats.presented_at(size, ms(base, 41));
        stats.gpu(9, 1.0);
        frame(&stats, base, 10, 50, FrameKind::Reproject);
        stats.span_at(Span::Acquire, ms(base, 52), ms(base, 53));
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines.len(), 6);
        let column = |key: &str| -> Vec<serde_json::Value> {
            lines[..5].iter().map(|line| line[key].clone()).collect()
        };
        let null = serde_json::Value::Null;
        assert_eq!(column("tick"), [0, 1, 2, 3, 4]);
        assert_eq!(column("source"), ["splash", "ui", "game", "ui", "game"]);
        assert_eq!(
            column("kind"),
            ["full", "full", "full", "full", "reproject"]
        );
        assert_eq!(column("t_ms"), [0.0, 8.0, 16.0, 40.0, 50.0]);
        assert_eq!(column("submit_ms"), [0.0, 0.0, 4.0, 0.0, 2.0]);
        assert_eq!(
            column("prepare_ms"),
            [
                null.clone(),
                null.clone(),
                0.0.into(),
                null.clone(),
                null.clone()
            ]
        );
        assert_eq!(column("encode_ms"), [0.0, 0.0, 1.0, 0.0, 2.0]);
        assert_eq!(column("poll_ms"), [0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(column("queue_ms"), [0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_eq!(column("frame_ms")[..4], [3.0, 2.0, 8.0, 1.0]);
        assert!(lines[4]["frame_ms"].is_null());
        assert_eq!(column("acquire_ms")[2], 1.0);
        assert!(
            [0, 1, 3, 4]
                .iter()
                .all(|&index| lines[index]["acquire_ms"].is_null())
        );
        assert!(lines[1]["wait_ms"].is_null());
        assert_eq!(lines[2]["wait_ms"], 1.0);
        assert_eq!(
            column("sim_ms"),
            [
                null.clone(),
                1.0.into(),
                1.0.into(),
                null.clone(),
                null.clone()
            ]
        );
        let host = lines[2]["frame_ms"].as_f64().unwrap();
        let renderer = lines[2]["submit_ms"].as_f64().unwrap();
        let sim = lines[2]["sim_ms"].as_f64().unwrap();
        assert!(renderer + sim < host, "{renderer} + {sim} against {host}");
        assert_eq!(column("size")[0], serde_json::json!([2560, 1440]));
        assert_eq!(column("size")[4], serde_json::json!([1280, 800]));
        assert!(lines[0]["present_interval_ms"].is_null());
        assert_eq!(column("present_interval_ms")[1..], [7.0, 14.0, 17.0, 11.0]);
        assert!(lines[0]["hud_ms"].is_null());
        assert_eq!(lines[1]["hud_ms"], 1.0);
        assert!(lines[2]["hud_ms"].is_null());
        assert_eq!(lines[2]["gpu_ms"], 4.0);
        assert_eq!(lines[2]["gpu_tick"], 2);
        assert!(lines[3]["gpu_ms"].is_null());
        assert!(lines[4]["gpu_ms"].is_null());
        assert_eq!(
            lines[5],
            serde_json::json!({"schema_version": 2, "end": true, "frames": 5, "dropped": 0})
        );
        let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        let first = text.lines().next().unwrap();
        assert!(
            first.starts_with("{\"schema_version\":2,\"tick\":0,")
                && first.ends_with(",\"kind\":\"full\",\"size\":[2560,1440],\"device\":null,\"hud_ms\":null,\"frame_ms\":3.000,\"acquire_ms\":null,\"source\":\"splash\"}"),
            "{first}"
        );
    }

    #[test]
    fn a_presented_frames_gpu_time_is_the_sum_of_its_renderer_frames_once_they_all_resolved() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        stats.keep_recent(8);
        let base = Instant::now();
        let size = [64, 64];
        stats.present_begin_at(Source::Ui, ms(base, 0));
        frame(&stats, base, 1, 1, FrameKind::Full);
        frame(&stats, base, 2, 3, FrameKind::Full);
        stats.presented_at(size, ms(base, 6));
        stats.gpu(1, 2.0);
        stats.present_begin_at(Source::Ui, ms(base, 10));
        stats.presented_at(size, ms(base, 11));
        stats.gpu(2, 3.0);
        stats.present_begin_at(Source::Ui, ms(base, 20));
        stats.presented_at(size, ms(base, 21));
        stats.present_begin_at(Source::Ui, ms(base, 30));
        frame(&stats, base, 3, 31, FrameKind::Full);
        frame(&stats, base, 4, 33, FrameKind::Full);
        stats.presented_at(size, ms(base, 36));
        stats.gpu(3, 1.0);
        stats.present_begin_at(Source::Ui, ms(base, 40));
        frame(&stats, base, 5, 41, FrameKind::Full);
        stats.presented_at(size, ms(base, 44));
        stats.gpu(5, 6.0);
        stats.gpu(4, 8.0);
        stats.finish();
        let recent: Vec<_> = stats.recent().iter().map(|sample| sample.gpu_ms).collect();
        assert_eq!(recent, [Some(5.0), None, None, None, Some(6.0)]);
        let lines = sink.lines();
        assert_eq!(lines.len(), 6);
        let gpu: Vec<_> = lines[..5]
            .iter()
            .map(|line| (line["gpu_tick"].as_u64(), line["gpu_ms"].as_f64()))
            .collect();
        assert_eq!(
            gpu,
            [
                (None, None),
                (Some(0), Some(5.0)),
                (None, None),
                (None, None),
                (Some(4), Some(6.0)),
            ]
        );
        let sources: Vec<_> = lines[..5]
            .iter()
            .map(|line| line["source"].clone())
            .collect();
        assert_eq!(sources, ["game", "ui", "ui", "game", "game"]);
        assert!(lines[..5].iter().all(|line| line["kind"] == "full"));
        assert_eq!(lines[0]["submit_ms"], 4.0);
    }

    fn shown(stats: &FrameStats, base: Instant, at: u64, frames: &[(&FrameStats, u64, bool)]) {
        stats.present_begin_at(Source::Ui, ms(base, at));
        for (by, number, timed) in frames {
            by.begin_at(FrameKind::Full, ms(base, at + 1));
            by.submitted_as_at(*number, [64, 64], *timed, ms(base, at + 2));
        }
        stats.presented_at([64, 64], ms(base, at + 3));
    }

    fn written_gpu(sink: &Sink, stats: &FrameStats) -> Vec<(Option<u64>, Option<f64>)> {
        let lines = sink.lines();
        let (_, frames) = lines.split_last().unwrap();
        let written: Vec<_> = frames
            .iter()
            .map(|line| (line["gpu_tick"].as_u64(), line["gpu_ms"].as_f64()))
            .collect();
        let recent = stats.recent();
        for sample in &recent {
            let line = written
                .iter()
                .find(|(tick, _)| *tick == Some(sample.tick))
                .and_then(|(_, ms)| *ms);
            assert_eq!(
                sample.gpu_ms, line,
                "tick {}: the HUD and the file",
                sample.tick
            );
        }
        written
    }

    #[test]
    fn two_renderers_on_one_handle_count_their_own_frame_numbers() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        stats.keep_recent(8);
        let a = stats.joined();
        let b = stats.joined();
        assert_eq!(stats.submitter(), 0);
        assert!(a.submitter() != 0 && b.submitter() != 0 && a.submitter() != b.submitter());
        assert_eq!(FrameStats::off().joined().submitter(), 0);
        let base = Instant::now();
        shown(&stats, base, 0, &[(&a, 0, true)]);
        shown(&stats, base, 10, &[(&a, 1, true)]);
        shown(&stats, base, 20, &[(&b, 0, true), (&a, 2, true)]);
        a.gpu(0, 1.0);
        shown(&stats, base, 30, &[]);
        b.gpu(0, 4.0);
        a.gpu(2, 0.5);
        shown(&stats, base, 40, &[]);
        stats.finish();
        assert_eq!(
            written_gpu(&sink, &stats),
            [
                (None, None),
                (None, None),
                (Some(0), Some(1.0)),
                (Some(2), Some(4.5)),
                (None, None),
            ]
        );
    }

    #[test]
    fn a_renderer_frame_that_times_nothing_or_never_resolves_holds_no_gpu_time_back() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        stats.keep_recent(8);
        let base = Instant::now();
        shown(&stats, base, 0, &[(&stats, 0, true), (&stats, 1, false)]);
        shown(&stats, base, 10, &[(&stats, 2, true), (&stats, 3, true)]);
        stats.gpu(0, 2.0);
        shown(&stats, base, 20, &[]);
        stats.gpu(2, 1.0);
        shown(&stats, base, 30, &[(&stats, 4, true), (&stats, 5, true)]);
        stats.gpu(4, 3.0);
        stats.gpu(3, 9.0);
        shown(&stats, base, 40, &[]);
        stats.gpu(5, 0.5);
        stats.finish();
        assert_eq!(
            written_gpu(&sink, &stats),
            [
                (None, None),
                (Some(0), Some(2.0)),
                (None, None),
                (Some(1), Some(1.0)),
                (Some(3), Some(3.5)),
            ]
        );
        let recent: Vec<_> = stats.recent().iter().map(|sample| sample.gpu_ms).collect();
        assert_eq!(recent, [Some(2.0), Some(1.0), None, Some(3.5), None]);
    }

    #[test]
    fn presented_frames_that_never_opened_or_opened_twice_write_only_what_was_shown() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 16);
        let base = Instant::now();
        stats.presented_at([8, 8], ms(base, 1));
        stats.present_abandon();
        assert_eq!(stats.frames(), 0);
        stats.present_begin_at(Source::Ui, ms(base, 2));
        stats.begin_at(FrameKind::Full, ms(base, 3));
        stats.submitted_at(1, [8, 8], ms(base, 4));
        stats.gpu(1, 7.0);
        stats.present_begin_at(Source::Splash, ms(base, 10));
        stats.begin_at(FrameKind::Idle, ms(base, 11));
        stats.submitted_at(2, [8, 8], ms(base, 11));
        stats.begin_at(FrameKind::Reproject, ms(base, 12));
        stats.submitted_at(3, [8, 8], ms(base, 12));
        stats.presented_at([8, 8], ms(base, 14));
        stats.gpu(1, 9.0);
        stats.present_begin_at(Source::Ui, ms(base, 20));
        stats.begin_at(FrameKind::Partial, ms(base, 21));
        stats.submitted_at(4, [8, 8], ms(base, 21));
        stats.begin_at(FrameKind::Idle, ms(base, 22));
        stats.submitted_at(5, [8, 8], ms(base, 22));
        stats.presented_at([8, 8], ms(base, 23));
        stats.presented_at([8, 8], ms(base, 24));
        stats.finish();
        let lines = sink.lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["tick"], 0);
        assert_eq!(lines[0]["t_ms"], 8.0);
        assert_eq!(lines[0]["submit_ms"], 0.0);
        assert_eq!(lines[0]["frame_ms"], 4.0);
        assert_eq!(lines[0]["source"], "splash");
        assert_eq!(lines[0]["kind"], "reproject");
        assert!(lines[0]["gpu_ms"].is_null());
        assert!(lines[1]["gpu_ms"].is_null());
        assert_eq!(lines[1]["source"], "game");
        assert_eq!(lines[1]["kind"], "partial");
        assert_eq!(lines[2]["frames"], 2);
    }

    #[test]
    fn a_stream_that_never_presents_a_frame_writes_nothing_not_even_its_end_line() {
        let sink = Sink::default();
        let stats = FrameStats::to_writer(Box::new(sink.clone()), 4);
        let base = Instant::now();
        stats.present_begin_at(Source::Splash, ms(base, 0));
        stats.begin_at(FrameKind::Full, ms(base, 1));
        stats.submitted_at(0, [8, 8], ms(base, 2));
        stats.gpu(0, 1.0);
        stats.present_abandon();
        stats.span_at(Span::Acquire, ms(base, 3), ms(base, 4));
        stats.span_at(Span::Wait, ms(base, 3), ms(base, 4));
        stats.finish();
        assert_eq!(stats.frames(), 0);
        assert!(sink.0.lock().unwrap().is_empty());
        let dropped = Sink::default();
        let second = FrameStats::to_writer(Box::new(dropped.clone()), 4);
        drop(second);
        assert!(dropped.0.lock().unwrap().is_empty());
    }

    #[test]
    fn an_off_handle_presents_nothing_and_sources_name_themselves() {
        let stats = FrameStats::off();
        let base = Instant::now();
        stats.present_begin_at(Source::Splash, base);
        assert!(!stats.presenting());
        stats.presented_at([8, 8], ms(base, 1));
        stats.present_begin(Source::Ui);
        stats.present_abandon();
        stats.presented([8, 8]);
        assert_eq!(stats.frames(), 0);
        assert!(stats.recent().is_empty());
        assert_eq!(Source::default(), Source::Game);
        let names = [Source::Splash, Source::Ui, Source::Game].map(Source::name);
        assert_eq!(names, ["splash", "ui", "game"]);
    }

    #[test]
    fn a_measuring_handle_keeps_the_source_of_presented_frames() {
        let stats = FrameStats::measure();
        stats.keep_recent(4);
        let base = Instant::now();
        stats.present_begin_at(Source::Splash, ms(base, 0));
        stats.presented_at([64, 64], ms(base, 1));
        stats.present_begin_at(Source::Ui, ms(base, 8));
        stats.presented_at([64, 64], ms(base, 9));
        frame(&stats, base, 0, 16, FrameKind::Full);
        let sources: Vec<Source> = stats.recent().iter().map(|sample| sample.source).collect();
        assert_eq!(sources, [Source::Splash, Source::Ui, Source::Game]);
        assert_eq!(stats.frames(), 3);
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
            first.ends_with(",\"device\":null,\"hud_ms\":1.000,\"frame_ms\":null,\"acquire_ms\":null,\"source\":\"game\"}"),
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
        assert!(stats.recent().iter().all(|sample| sample.gpu_ms.is_none()));
        frame(&stats, base, 5, 84, FrameKind::Full);
        let recent = stats.recent();
        assert_eq!(recent[0].tick, 3);
        assert_eq!(recent[0].gpu_ms, Some(4.25));
        assert_eq!(recent[0].submit_ms, 2.0);
        let early = FrameStats::measure();
        early.keep_recent(4);
        frame(&early, base, 0, 0, FrameKind::Full);
        early.sim_begin_at(ms(base, 5));
        early.sim_end_at(ms(base, 7));
        frame(&early, base, 1, 17, FrameKind::Full);
        early.hud_begin_at(ms(base, 20));
        early.hud_end_at(ms(base, 23));
        early.gpu(0, 1.5);
        frame(&early, base, 2, 33, FrameKind::Full);
        let recent = early.recent();
        assert_eq!(recent[0].gpu_ms, Some(1.5));
        assert!(recent[0].hud_ms.is_none());
        assert_eq!(recent[1].sim_ms, Some(2.0));
        assert_eq!(recent[1].hud_ms, Some(3.0));
        stats.finish();
        assert_eq!(stats.frames(), 6);
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
            let idle = FrameStats::from_value(Some(path.as_os_str()));
            assert!(idle.enabled());
            idle.present_begin(Source::Ui);
            idle.present_abandon();
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
