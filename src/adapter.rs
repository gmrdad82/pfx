use crate::protocol::{
    AUDIO_CHANNELS, AUDIO_RATE, AudioFormat, Event, Hello, Opened, PROTOCOL, Reply, Request,
    Sounded, Written,
};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Report {
    pub work_ms: f64,
    pub longest_ms: f64,
}

impl Report {
    pub fn args(&self) -> [String; 5] {
        [
            "turn".into(),
            "--work-ms".into(),
            format!("{:.3}", self.work_ms),
            "--longest-ms".into(),
            format!("{:.3}", self.longest_ms),
        ]
    }
}

pub const TURN_WALL_MS: f64 = 400.0;

pub const TURN_MS_VAR: &str = "PFX_TURN_MS";

pub const TURN_WALL_VAR: &str = "PFX_TURN_WALL_MS";

pub const TRACE_VAR: &str = "PFX_TURN_TRACE";

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
    measured: bool,
    untimed: bool,
    last_turn: Instant,
    frozen: Frozen,
    frozen_at_turn: f64,
}

impl Default for TurnState {
    fn default() -> Self {
        Self::new(env_ms(TURN_MS_VAR, 50.0)).with_wall_ms(env_ms(TURN_WALL_VAR, TURN_WALL_MS))
    }
}

impl TurnState {
    pub fn new(threshold_ms: f64) -> Self {
        Self {
            threshold_ms: if threshold_ms.is_finite() {
                threshold_ms.max(1.0)
            } else {
                50.0
            },
            wall_ms: TURN_WALL_MS,
            accumulated_ms: 0.0,
            longest_ms: 0.0,
            measured: false,
            untimed: false,
            last_turn: Instant::now(),
            frozen: Frozen::none(),
            frozen_at_turn: 0.0,
        }
        .with_frozen(Frozen::from_env())
    }

    pub fn with_frozen(mut self, frozen: Frozen) -> Self {
        self.frozen_at_turn = frozen.ms();
        self.frozen = frozen;
        self
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
            self.measured = true;
        } else {
            self.untimed = true;
        }
    }

    pub fn bare(&self) -> bool {
        self.untimed && !self.measured
    }

    pub fn args(&self) -> Vec<String> {
        if self.bare() {
            vec!["turn".into()]
        } else {
            self.report().args().to_vec()
        }
    }

    pub fn since_turn_ms(&self) -> f64 {
        self.since_turn_ms_at(Instant::now(), self.frozen.ms())
    }

    pub fn since_turn_ms_at(&self, now: Instant, frozen_ms: f64) -> f64 {
        unfrozen_ms(
            now.saturating_duration_since(self.last_turn).as_secs_f64() * 1000.0,
            self.frozen_at_turn,
            frozen_ms,
        )
    }

    pub fn due(&self) -> bool {
        self.due_at(Instant::now())
    }

    pub fn due_at(&self, now: Instant) -> bool {
        self.accumulated_ms >= self.threshold_ms
            || now.saturating_duration_since(self.last_turn).as_secs_f64() * 1000.0 >= self.wall_ms
    }

    pub fn take(&mut self) -> Report {
        self.take_at(Instant::now())
    }

    pub fn take_at(&mut self, now: Instant) -> Report {
        let report = self.report();
        self.reset_at(now);
        report
    }

    pub fn take_args(&mut self) -> Vec<String> {
        let args = self.args();
        self.reset();
        args
    }

    pub fn reset(&mut self) {
        self.reset_at(Instant::now());
    }

    pub fn reset_at(&mut self, now: Instant) {
        self.accumulated_ms = 0.0;
        self.longest_ms = 0.0;
        self.measured = false;
        self.untimed = false;
        self.last_turn = now;
        self.frozen_at_turn = self.frozen.ms();
    }
}

pub struct About {
    pub app: String,
    pub rev: String,
    pub depth: u8,
    pub cutouts: bool,
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u16>,
    pub ground: Option<[f32; 3]>,
}

pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u16>,
}

impl Image {
    pub fn from_rgba8(width: u32, height: u32, rgba: &[u8]) -> Image {
        Image {
            width,
            height,
            rgba: rgba.iter().map(|&v| v as u16 * 257).collect(),
            ground: None,
        }
    }
}

pub trait Adapter {
    fn about(&self) -> About;
    fn shots(&self) -> &str;
    fn open(
        &mut self,
        shot: &str,
        fixtures: &Path,
        scale: f32,
        app: &Value,
    ) -> Result<Opened, String>;
    fn input(&mut self, event: &Event) -> Result<(), String>;
    fn advance(&mut self, to: f64) -> Result<(), String>;
    fn sample(&mut self, spp: u32, moving: u32, seed: u64) -> Result<(), String>;
    fn frame(&mut self, ground: Option<[f32; 3]>, linear: bool) -> Result<Image, String>;
    fn supports_hdr(&self) -> bool {
        false
    }
    fn hdr(&mut self) -> Result<HdrImage, String> {
        Err("unsupported".into())
    }
    fn audio_format(&self) -> Option<AudioFormat> {
        None
    }
    fn audio(&mut self, _seconds: f64) -> Result<Vec<f32>, String> {
        Err("unsupported".into())
    }
    fn close(&mut self) {}
    fn turns(&mut self) -> Option<&mut TurnState> {
        None
    }
}

fn write_values(path: &str, width: u32, height: u32, rgba: &[u16]) -> Result<(), String> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "frame holds {} values, expected {expected}",
            rgba.len()
        ));
    }
    let mut bytes = Vec::with_capacity(expected * 2);
    for v in rgba {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(|e| format!("{path}: {e}"))
}

pub fn wav_bytes(format: AudioFormat, samples: &[f32]) -> Vec<u8> {
    let (tag, bits): (u16, u16) = match format {
        AudioFormat::S16 => (1, 16),
        AudioFormat::F32 => (3, 32),
    };
    let width = u32::from(bits / 8);
    let block = AUDIO_CHANNELS * (bits / 8);
    let data = samples.len() as u32 * width;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&AUDIO_CHANNELS.to_le_bytes());
    out.extend_from_slice(&AUDIO_RATE.to_le_bytes());
    out.extend_from_slice(&(AUDIO_RATE * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for &v in samples {
        match format {
            AudioFormat::S16 => {
                let scaled = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                out.extend_from_slice(&scaled.to_le_bytes());
            }
            AudioFormat::F32 => out.extend_from_slice(&v.to_le_bytes()),
        }
    }
    out
}

fn write_audio<A: Adapter>(adapter: &mut A, path: &str, seconds: f64) -> Result<Value, String> {
    let format = adapter
        .audio_format()
        .ok_or_else(|| "unsupported".to_string())?;
    let samples = adapter.audio(seconds)?;
    if samples.len() % usize::from(AUDIO_CHANNELS) != 0 {
        return Err(format!(
            "audio holds {} values, not a whole number of stereo frames",
            samples.len()
        ));
    }
    std::fs::write(path, wav_bytes(format, &samples)).map_err(|e| format!("{path}: {e}"))?;
    Ok(json(&Sounded {
        frames: (samples.len() / usize::from(AUDIO_CHANNELS)) as u64,
    }))
}

fn json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn handle<A: Adapter>(adapter: &mut A, request: Request) -> Result<Value, String> {
    match request {
        Request::Hello => {
            let about = adapter.about();
            let mut hello = json(&Hello {
                protocol: PROTOCOL,
                app: about.app,
                rev: about.rev,
                depth: about.depth,
                cutouts: about.cutouts,
                audio: adapter.audio_format().is_some(),
                audio_format: adapter.audio_format(),
            });
            hello["hdr"] = Value::Bool(adapter.supports_hdr());
            Ok(hello)
        }
        Request::Shots => Ok(Value::String(adapter.shots().to_string())),
        Request::Open {
            shot,
            fixtures,
            scale,
            app,
        } => adapter
            .open(&shot, Path::new(&fixtures), scale, &app)
            .map(|o| json(&o)),
        Request::Input { event } => adapter.input(&event).map(|_| Value::Null),
        Request::Advance { to } => adapter.advance(to).map(|_| Value::Null),
        Request::Sample { spp, moving, seed } => {
            adapter.sample(spp, moving, seed).map(|_| Value::Null)
        }
        Request::Frame {
            path,
            ground,
            linear,
        } => {
            let image = adapter.frame(ground, linear)?;
            write_values(&path, image.width, image.height, &image.rgba)?;
            Ok(json(&Written {
                size: [image.width, image.height],
                ground: image.ground,
            }))
        }
        Request::Audio { path, seconds } => write_audio(adapter, &path, seconds),
        Request::Close => {
            adapter.close();
            Ok(Value::Null)
        }
    }
}

fn handle_hdr<A: Adapter>(adapter: &mut A, path: &str) -> Result<Value, String> {
    let image = adapter.hdr()?;
    write_values(path, image.width, image.height, &image.rgba)?;
    Ok(json(&Written {
        size: [image.width, image.height],
        ground: None,
    }))
}

static TURNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
static TURNS_TAKEN: AtomicU64 = AtomicU64::new(0);

pub fn turns_taken() -> u64 {
    TURNS_TAKEN.load(Ordering::SeqCst)
}

struct Traced {
    epoch: Instant,
    last: Instant,
    frozen_ms: f64,
}

static TRACED: std::sync::Mutex<Option<Traced>> = std::sync::Mutex::new(None);

fn trace_turn(site: &std::panic::Location<'_>, args: &[String]) {
    let Some(path) = std::env::var_os(TRACE_VAR).filter(|path| !path.is_empty()) else {
        return;
    };
    let now = Instant::now();
    let frozen_ms = Frozen::from_env().ms();
    let (gap_ms, at_ms) = {
        let mut traced = TRACED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = traced.get_or_insert(Traced {
            epoch: now,
            last: now,
            frozen_ms,
        });
        let gap = unfrozen_ms(
            now.saturating_duration_since(before.last).as_secs_f64() * 1000.0,
            before.frozen_ms,
            frozen_ms,
        );
        let at = now.saturating_duration_since(before.epoch).as_secs_f64() * 1000.0;
        (gap, at)
    };
    let line = trace_row(at_ms, gap_ms, site, args);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn trace_row(at_ms: f64, gap_ms: f64, site: &std::panic::Location<'_>, args: &[String]) -> String {
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|at| args.get(at + 1))
            .and_then(|value| value.parse::<f64>().ok())
    };
    let untimed = u32::from(flag("--work-ms").is_none());
    format!(
        "{at_ms:.1}\tadapter\t{}:{}\t{:.3}\t{:.3}\t{gap_ms:.1}\t1\t0\t0\t{}\t{untimed}\t{}\n",
        site.file(),
        site.line(),
        flag("--work-ms").unwrap_or(0.0),
        flag("--longest-ms").unwrap_or(0.0),
        1 - untimed,
        std::thread::current().name().unwrap_or("-"),
    )
}

fn trace_turned() {
    let mut traced = TRACED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(traced) = traced.as_mut() {
        traced.last = Instant::now();
        traced.frozen_ms = Frozen::from_env().ms();
    }
}

#[track_caller]
pub fn pgpu_turn(args: &[String], quiet: bool) -> std::io::Result<std::process::ExitStatus> {
    trace_turn(std::panic::Location::caller(), args);
    let _held = TURNING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TURNS_TAKEN.fetch_add(1, Ordering::SeqCst);
    let mut command = std::process::Command::new("pgpu");
    command.args(args);
    if quiet {
        command.stdout(std::process::Stdio::null());
    }
    let status = command.status();
    trace_turned();
    status
}

#[track_caller]
pub fn turn() {
    if std::env::var_os("GPU_QUEUE_TURN").is_some() {
        let _ = pgpu_turn(&["turn".into()], true);
    }
}

#[track_caller]
pub fn turn_reporting(report: Report) {
    if std::env::var_os("GPU_QUEUE_TURN").is_some() {
        let _ = pgpu_turn(&report.args(), true);
    }
}

#[track_caller]
pub fn turn_now(state: &mut TurnState) {
    let args = state.take_args();
    if std::env::var_os("GPU_QUEUE_TURN").is_some() {
        let _ = pgpu_turn(&args, true);
    }
}

#[track_caller]
pub fn turn_soon(state: &mut TurnState, slice_ms: f64) -> bool {
    state.add(slice_ms);
    if !state.due() {
        return false;
    }
    turn_now(state);
    true
}

struct Timed {
    began: Instant,
    frozen_ms: f64,
    turns: u64,
}

impl Timed {
    fn start(frozen: &Frozen) -> Self {
        Self {
            began: Instant::now(),
            frozen_ms: frozen.ms(),
            turns: turns_taken(),
        }
    }

    fn unturned_ms(&self, frozen: &Frozen) -> Option<f64> {
        self.unturned_ms_at(Instant::now(), frozen.ms())
    }

    fn unturned_ms_at(&self, now: Instant, frozen_ms: f64) -> Option<f64> {
        (turns_taken() == self.turns).then(|| {
            unfrozen_ms(
                now.saturating_duration_since(self.began).as_secs_f64() * 1000.0,
                self.frozen_ms,
                frozen_ms,
            )
        })
    }
}

fn turn_after<A: Adapter>(adapter: &mut A, request_ms: Option<f64>) {
    match (adapter.turns(), request_ms) {
        (Some(state), _) => turn_now(state),
        (None, Some(request_ms)) => turn_reporting(Report {
            work_ms: request_ms,
            longest_ms: request_ms,
        }),
        (None, None) => turn(),
    }
}

pub fn serve<A: Adapter>(adapter: &mut A) -> Result<(), String> {
    let frozen = Frozen::from_env();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| format!("stdin: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let (reply, closing) = match serde_json::from_str::<Value>(&line) {
            Err(e) => (Reply::Err(format!("not a request: {e}")), false),
            Ok(value) if value.get("op").and_then(Value::as_str) == Some("hdr") => {
                let request = Timed::start(&frozen);
                let reply = value
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("hdr needs a path".to_string())
                    .and_then(|path| handle_hdr(adapter, path))
                    .map_or_else(Reply::Err, Reply::Ok);
                turn_after(adapter, request.unturned_ms(&frozen));
                (reply, false)
            }
            Ok(value) => match serde_json::from_value::<Request>(value) {
                Err(e) => (Reply::Err(format!("not a request: {e}")), false),
                Ok(request) => {
                    let closing = request == Request::Close;
                    let sampled = matches!(request, Request::Sample { .. } | Request::Frame { .. });
                    let timed = Timed::start(&frozen);
                    let reply = handle(adapter, request).map_or_else(Reply::Err, Reply::Ok);
                    if sampled {
                        turn_after(adapter, timed.unturned_ms(&frozen));
                    }
                    (reply, closing)
                }
            },
        };
        let text = serde_json::to_string(&reply).map_err(|e| e.to_string())?;
        writeln!(out, "{text}")
            .and_then(|_| out.flush())
            .map_err(|e| format!("stdout: {e}"))?;
        if closing {
            end();
        }
    }
    adapter.close();
    end()
}

fn end() -> ! {
    #[cfg(unix)]
    unsafe {
        libc::_exit(0)
    }
    #[cfg(not(unix))]
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct Scratch {
        closed: u32,
    }

    impl Adapter for Scratch {
        fn about(&self) -> About {
            About {
                app: "scratch".into(),
                rev: "0".into(),
                depth: 16,
                cutouts: false,
            }
        }
        fn shots(&self) -> &str {
            ""
        }
        fn open(&mut self, _: &str, _: &Path, _: f32, _: &Value) -> Result<Opened, String> {
            Err("never opened".into())
        }
        fn input(&mut self, _: &Event) -> Result<(), String> {
            Ok(())
        }
        fn advance(&mut self, _: f64) -> Result<(), String> {
            Ok(())
        }
        fn sample(&mut self, _: u32, _: u32, _: u64) -> Result<(), String> {
            Ok(())
        }
        fn frame(&mut self, _: Option<[f32; 3]>, _: bool) -> Result<Image, String> {
            Err("never framed".into())
        }
        fn close(&mut self) {
            self.closed += 1;
        }
    }

    #[test]
    fn an_adapter_cleans_up_before_close_is_answered() {
        let mut adapter = Scratch { closed: 0 };
        assert_eq!(
            handle(&mut adapter, Request::Hello).map(|_| adapter.closed),
            Ok(0)
        );
        assert_eq!(handle(&mut adapter, Request::Hello).unwrap()["hdr"], false);
        assert_eq!(
            handle_hdr(&mut adapter, "tmp/unused.rgba"),
            Err("unsupported".into())
        );
        assert_eq!(handle(&mut adapter, Request::Close), Ok(Value::Null));
        assert_eq!(adapter.closed, 1);
    }

    #[test]
    fn a_traced_turn_row_carries_its_flags_and_site() {
        let site = std::panic::Location::caller();
        let full = trace_row(
            12.5,
            40.0,
            site,
            &Report {
                work_ms: 7.5,
                longest_ms: 2.25,
            }
            .args(),
        );
        let fields: Vec<&str> = full.trim_end().split('\t').collect();
        assert_eq!(fields.len(), 12);
        assert_eq!(&fields[1], &"adapter");
        assert!(fields[2].contains("adapter.rs:"));
        assert_eq!(&fields[3..6], &["7.500", "2.250", "40.0"]);
        assert_eq!(&fields[6..11], &["1", "0", "0", "1", "0"]);
        let bare = trace_row(0.0, 0.0, site, &["turn".to_string()]);
        let fields: Vec<&str> = bare.trim_end().split('\t').collect();
        assert_eq!(&fields[9..11], &["0", "1"]);
    }

    #[test]
    fn an_adapter_without_audio_says_so_and_refuses_the_request() {
        let mut adapter = Scratch { closed: 0 };
        let hello = handle(&mut adapter, Request::Hello).unwrap();
        assert_eq!(hello["audio"], false);
        assert_eq!(hello["audio_format"], Value::Null);
        assert_eq!(
            handle(
                &mut adapter,
                Request::Audio {
                    path: "tmp/unused.wav".into(),
                    seconds: 1.0
                }
            ),
            Err("unsupported".into())
        );
    }

    struct Tone;

    impl Adapter for Tone {
        fn about(&self) -> About {
            About {
                app: "tone".into(),
                rev: "r".into(),
                depth: 16,
                cutouts: false,
            }
        }
        fn shots(&self) -> &str {
            ""
        }
        fn open(&mut self, _: &str, _: &Path, _: f32, _: &Value) -> Result<Opened, String> {
            Err("never opened".into())
        }
        fn input(&mut self, _: &Event) -> Result<(), String> {
            Ok(())
        }
        fn advance(&mut self, _: f64) -> Result<(), String> {
            Ok(())
        }
        fn sample(&mut self, _: u32, _: u32, _: u64) -> Result<(), String> {
            Ok(())
        }
        fn frame(&mut self, _: Option<[f32; 3]>, _: bool) -> Result<Image, String> {
            Err("never framed".into())
        }
        fn audio_format(&self) -> Option<AudioFormat> {
            Some(AudioFormat::S16)
        }
        fn audio(&mut self, seconds: f64) -> Result<Vec<f32>, String> {
            Ok(vec![0.5; (seconds * 48_000.0) as usize * 2])
        }
    }

    #[test]
    fn an_adapter_with_audio_announces_it_and_writes_a_wav() {
        let mut adapter = Tone;
        let hello = handle(&mut adapter, Request::Hello).unwrap();
        assert_eq!(hello["audio"], true);
        assert_eq!(hello["audio_format"], "s16");
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("tone-{}.wav", std::process::id()));
        let reply = handle(
            &mut adapter,
            Request::Audio {
                path: path.display().to_string(),
                seconds: 0.25,
            },
        )
        .unwrap();
        assert_eq!(reply["frames"], 12_000);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 12_000 * 4);
        assert_eq!(&bytes[44..46], &16_384i16.to_le_bytes());
    }

    #[test]
    fn turn_soon_accumulates_slice_time() {
        let mut state = TurnState::new(50.0);
        assert!(!turn_soon(&mut state, 20.0));
        assert!(!turn_soon(&mut state, 29.0));
        assert_eq!(state.pending_ms(), 49.0);
        assert_eq!(
            state.report(),
            Report {
                work_ms: 49.0,
                longest_ms: 29.0,
            }
        );
        assert!(turn_soon(&mut state, 1.0));
        assert_eq!(state.pending_ms(), 0.0);
        assert_eq!(state.report(), Report::default());
        assert!(!turn_soon(&mut state, 4.0));
        assert!(!turn_soon(&mut state, f64::NAN));
        assert_eq!(state.report().longest_ms, 4.0);
        state.last_turn = Instant::now() - Duration::from_millis(400);
        assert!(turn_soon(&mut state, 1.0));
        assert_eq!(state.report(), Report::default());
    }

    #[test]
    fn a_light_job_turns_at_most_four_tenths_of_a_second_apart() {
        let began = Instant::now();
        let mut state = TurnState::new(50.0);
        state.reset_at(began);
        let mut last = began;
        let mut gaps = Vec::new();
        for tick in 1..=500u32 {
            let now = began + Duration::from_millis(u64::from(tick) * 10);
            state.add(0.5);
            if state.due_at(now) {
                gaps.push(now.duration_since(last));
                state.take_at(now);
                last = now;
            }
        }
        assert_eq!(gaps.len(), 12);
        assert!(gaps.iter().all(|gap| *gap <= Duration::from_millis(400)));
        assert!(gaps.iter().all(|gap| *gap >= Duration::from_millis(390)));
    }

    #[test]
    fn the_wall_fallback_is_four_tenths_of_a_second_and_settable() {
        let began = Instant::now();
        let mut state = TurnState::new(50.0);
        state.reset_at(began);
        assert!(!state.due_at(began + Duration::from_millis(399)));
        assert!(state.due_at(began + Duration::from_millis(400)));
        let mut slow = TurnState::new(50.0).with_wall_ms(1000.0);
        slow.reset_at(began);
        assert!(!slow.due_at(began + Duration::from_millis(999)));
        assert!(slow.due_at(began + Duration::from_millis(1000)));
        let ignored = TurnState::new(50.0)
            .with_wall_ms(f64::NAN)
            .with_wall_ms(-3.0);
        assert_eq!(ignored.wall_ms, TURN_WALL_MS);
        assert!(!state.due_at(began));
        state.add(50.0);
        assert!(state.due_at(began));
    }

    #[test]
    fn the_time_since_a_turn_leaves_out_frozen_time() {
        let began = Instant::now();
        let mut state = TurnState::new(50.0).with_frozen(Frozen::none());
        state.reset_at(began);
        assert_eq!(
            state.since_turn_ms_at(began + Duration::from_millis(4_413_012), 4_413_000.0),
            12.0
        );
        assert_eq!(
            state.since_turn_ms_at(began + Duration::from_millis(30), 0.0),
            30.0
        );
        assert_eq!(unfrozen_ms(10.0, 0.0, 50.0), 0.0);
        assert!(!state.bare());
        state.add(f64::INFINITY);
        assert!(state.bare());
        assert_eq!(state.args(), ["turn"]);
        state.add(3.0);
        assert!(!state.bare());
        assert_eq!(
            state.take_args(),
            ["turn", "--work-ms", "3.000", "--longest-ms", "3.000"]
        );
        assert_eq!(
            state.args(),
            ["turn", "--work-ms", "0.000", "--longest-ms", "0.000"]
        );
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("adapter-frozen-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let file = scratch.join("frozen");
        assert_eq!(Frozen::at(&file).ms(), 0.0);
        std::fs::write(&file, "250\n").unwrap();
        assert_eq!(Frozen::at(&file).ms(), 250.0);
        std::fs::write(&file, "x").unwrap();
        assert_eq!(Frozen::at(&file).ms(), 0.0);
        std::fs::remove_dir_all(&scratch).unwrap();
    }

    #[test]
    fn a_report_formats_pgpus_flags() {
        assert_eq!(
            Report {
                work_ms: 51.25,
                longest_ms: 3.0004,
            }
            .args(),
            ["turn", "--work-ms", "51.250", "--longest-ms", "3.000"]
        );
    }

    #[test]
    fn turns_reach_pgpu_with_their_pacing() {
        if std::env::var_os("PFX_TURN_CHILD").is_some() {
            return;
        }
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("turn-args-{}", std::process::id()));
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
            .args([
                "--exact",
                "adapter::tests::turn_child",
                "--test-threads",
                "1",
            ])
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("GPU_QUEUE_TURN", "1")
            .env_remove(FROZEN_VAR)
            .env("PFX_TURN_CHILD", "1")
            .env("TURN_LOG", &log)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        let lines = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_dir_all(&scratch).unwrap();
        assert!(status.success());
        assert_eq!(
            lines.lines().collect::<Vec<_>>(),
            [
                "turn --work-ms 51.500 --longest-ms 31.500",
                "turn --work-ms 4.000 --longest-ms 4.000",
                "turn --work-ms 12.250 --longest-ms 12.250",
                "turn --work-ms 0.000 --longest-ms 0.000",
                "turn --work-ms 4.000 --longest-ms 4.000",
                "turn",
                "turn",
                "turn",
                "turn --work-ms 2.000 --longest-ms 2.000",
                "turn --work-ms 1.000 --longest-ms 1.000",
                "turn --work-ms 1.000 --longest-ms 1.000",
            ]
        );
    }

    struct Pacing {
        turns: TurnState,
    }

    impl Adapter for Pacing {
        fn about(&self) -> About {
            Scratch { closed: 0 }.about()
        }
        fn shots(&self) -> &str {
            ""
        }
        fn open(&mut self, _: &str, _: &Path, _: f32, _: &Value) -> Result<Opened, String> {
            Err("never opened".into())
        }
        fn input(&mut self, _: &Event) -> Result<(), String> {
            Ok(())
        }
        fn advance(&mut self, _: f64) -> Result<(), String> {
            Ok(())
        }
        fn sample(&mut self, _: u32, _: u32, _: u64) -> Result<(), String> {
            Ok(())
        }
        fn frame(&mut self, _: Option<[f32; 3]>, _: bool) -> Result<Image, String> {
            Err("never framed".into())
        }
        fn turns(&mut self) -> Option<&mut TurnState> {
            Some(&mut self.turns)
        }
    }

    #[test]
    fn turn_child() {
        if std::env::var_os("PFX_TURN_CHILD").is_none() {
            return;
        }
        let mut state = TurnState::new(50.0);
        assert!(!turn_soon(&mut state, 20.0));
        assert!(turn_soon(&mut state, 31.5));
        assert!(!turn_soon(&mut state, 4.0));
        turn_now(&mut state);
        turn_after(&mut Scratch { closed: 0 }, Some(12.25));
        let mut paced = Pacing {
            turns: TurnState::new(50.0),
        };
        turn_after(&mut paced, Some(900.0));
        let base = Instant::now();
        let held = Timed {
            began: base,
            frozen_ms: 0.0,
            turns: turns_taken(),
        };
        let request_ms = held.unturned_ms_at(base + Duration::from_millis(90_004), 90_000.0);
        assert_eq!(request_ms, Some(4.0));
        turn_after(&mut Scratch { closed: 0 }, request_ms);
        let nested = Timed::start(&Frozen::none());
        turn();
        let request_ms = nested.unturned_ms(&Frozen::none());
        assert_eq!(request_ms, None);
        turn_after(&mut Scratch { closed: 0 }, request_ms);
        let mut untimed = TurnState::new(50.0).with_wall_ms(1e9);
        assert!(!turn_soon(&mut untimed, f64::NAN));
        turn_now(&mut untimed);
        assert!(!turn_soon(&mut untimed, f64::NAN));
        assert!(!turn_soon(&mut untimed, 2.0));
        turn_now(&mut untimed);
        let together: Vec<_> = (0..2)
            .map(|_| {
                std::thread::spawn(|| {
                    turn_reporting(Report {
                        work_ms: 1.0,
                        longest_ms: 1.0,
                    })
                })
            })
            .collect();
        for thread in together {
            thread.join().unwrap();
        }
    }
}
