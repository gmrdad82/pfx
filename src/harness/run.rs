use super::audio;
use super::encode;
use super::image::{Frame, Linear};
use super::loops::{self, Pick};
use super::shots::{self, Shot};
use super::tools::{self, Built};
use crate::protocol::{Event, Hello, Opened, Reply, Request, Sounded, Written};
use exr::prelude::{
    Image as ExrImage, SpecificChannels, Vec2, WritableImage, attribute::Chromaticities, f16,
};
use pfx_run::ending::{self, Process};
use pfx_run::job::{HD_DETAIL, Job, TOOL, stages};
use pfx_run::{archive, encode as master_encode};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SCRATCH: &str = "pfx";
const EARLIER_TOOL: &str = "pito-hd-renderer";
const KIND: &str = "clip";
const STILL: &str = "still";
const CHUNK: u32 = 4;
const STDERR_TAIL: usize = 20;
const MASTER_WHAT: &str = "x264 CRF 12 master, High, yuv420p, BT.709 limited";

struct Scratch {
    path: PathBuf,
    keep: bool,
}

impl Scratch {
    fn new(tmp: &Path, stem: &str, keep: bool) -> Result<Self, String> {
        let mut parts = Path::new(stem).components();
        if !matches!(parts.next(), Some(std::path::Component::Normal(_))) || parts.next().is_some()
        {
            return Err(format!("{stem} is not a scratch folder name"));
        }
        let root = tools::inside(tmp, &tmp.join(SCRATCH))?;
        loop {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis();
            let path = root.join(format!("{stem}.{}-{millis}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path, keep }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

pub enum Source {
    Git(String),
    Dir(PathBuf),
    Adapter(PathBuf),
}

pub struct Options {
    pub app: String,
    pub shot: String,
    pub source: Source,
    pub fixtures: PathBuf,
    pub out: Option<PathBuf>,
    pub fps: Option<Vec<u32>>,
    pub size: Option<String>,
    pub derive: Vec<String>,
    pub seconds: Option<f64>,
    pub seed: u64,
    pub set: Vec<String>,
    pub class: String,
    pub keep_frames: bool,
    pub encode: bool,
    pub no_archive: bool,
    pub force_fixtures: bool,
    pub at: f64,
    pub samples: u32,
    pub web: Vec<u32>,
    pub hdr: bool,
}

struct Session {
    tmp: PathBuf,
    dir: PathBuf,
    tools_dir: PathBuf,
    started: Instant,
    made_at: String,
    stages: BTreeMap<String, f64>,
    built: Built,
    client: Client,
    hello: Hello,
    shot: Shot,
    fixtures: PathBuf,
    set: String,
    recipe: String,
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    queued: bool,
    adapter: Vec<Process>,
    tracked: ending::Tracked,
    tail: Arc<Mutex<VecDeque<String>>>,
    drained: Receiver<()>,
}

impl Client {
    fn start(
        path: &Path,
        class: &str,
        as_name: &str,
        libs: Option<&Path>,
    ) -> Result<Client, String> {
        let pfx_run::queue::Launch {
            command: mut cmd,
            queued,
        } = pfx_run::queue::launch(class, as_name, path);
        if let Some(dir) = libs {
            let key = if cfg!(windows) {
                "PATH"
            } else if cfg!(target_os = "macos") {
                "DYLD_LIBRARY_PATH"
            } else {
                "LD_LIBRARY_PATH"
            };
            let old = std::env::var_os(key).unwrap_or_default();
            let joined = std::env::join_paths(
                std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&old)),
            )
            .map_err(|e| format!("{key}: {e}"))?;
            cmd.env(key, joined);
        }
        Client::spawn(cmd, queued).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn spawn(mut cmd: Command, queued: bool) -> Result<Client, String> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let tracked = ending::track(&child);
        let stdin = child.stdin.take().ok_or("adapter has no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("adapter has no stdout")?);
        let stderr = child.stderr.take().ok_or("adapter has no stderr")?;
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let (done, drained) = mpsc::channel();
        let kept = Arc::clone(&tail);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).split(b'\n') {
                let Ok(bytes) = line else { break };
                let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
                eprintln!("{line}");
                if let Ok(mut lines) = kept.lock() {
                    if lines.len() == STDERR_TAIL {
                        lines.pop_front();
                    }
                    lines.push_back(line);
                }
            }
            let _ = done.send(());
        });
        Ok(Client {
            child,
            stdin: Some(stdin),
            stdout,
            queued,
            adapter: Vec::new(),
            tracked,
            tail,
            drained,
        })
    }

    fn gone(&mut self, what: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => break None,
            }
        };
        let _ = self.drained.recv_timeout(Duration::from_secs(1));
        let lines: Vec<String> = self
            .tail
            .lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default();
        let mut report = format!("{what}: the adapter ");
        match status {
            Some(status) => report.push_str(&format!("ended with {status}")),
            None => report.push_str("closed its output and is still running"),
        }
        if let Some(panic) = lines.iter().rev().find(|line| line.contains("panicked at")) {
            report.push_str(&format!("; it panicked: {}", panic.trim()));
        }
        if lines.is_empty() {
            report.push_str("; it wrote nothing to stderr");
        } else {
            report.push_str("; its last stderr lines:");
            for line in &lines {
                report.push_str(&format!("\n  {line}"));
            }
        }
        report
    }

    fn call(&mut self, request: &Request) -> Result<Value, String> {
        self.call_value(serde_json::to_value(request).map_err(|e| e.to_string())?)
    }

    fn call_value(&mut self, request: Value) -> Result<Value, String> {
        let line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        let stdin = self.stdin.as_mut().ok_or("the adapter has ended")?;
        if let Err(e) = writeln!(stdin, "{line}").and_then(|_| stdin.flush()) {
            return Err(self.gone(&format!("adapter stdin ({e})")));
        }
        let mut reply = String::new();
        loop {
            reply.clear();
            match self.stdout.read_line(&mut reply) {
                Ok(0) => return Err(self.gone("adapter stdout")),
                Ok(_) => {}
                Err(e) => return Err(format!("adapter stdout: {e}")),
            }
            match serde_json::from_str::<Reply>(&reply) {
                Ok(Reply::Ok(v)) => return Ok(v),
                Ok(Reply::Err(e)) => return Err(format!("adapter: {e}")),
                Err(_) => eprintln!("{TOOL}: adapter output: {}", reply.trim_end()),
            }
        }
    }

    fn close(mut self) {
        self.hold();
        if let Err(e) = self.call(&Request::Close) {
            eprintln!("{TOOL}: the adapter did not answer close ({e}); all it gave before stands");
        }
    }

    fn hold(&mut self) {
        if self.queued {
            self.adapter = ending::below(self.child.id());
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _keep = &self.tracked;
        if !self.queued {
            ending::terminate(&mut self.child);
        } else {
            if self.adapter.is_empty() {
                self.hold();
            }
            self.adapter.iter().for_each(Process::end);
        }
        self.stdin = None;
        let _ = self.child.wait();
    }
}

fn typed<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("adapter reply: {e}"))
}

fn build(app: &str, source: &Source, tools: &Path) -> Result<Built, String> {
    let own = || tools::own_app(app, tools::caller_root().as_deref());
    match source {
        Source::Git(reference) => tools::build_from_git(app, reference, tools, &own()?),
        Source::Dir(dir) => tools::build_from_dir(app, dir, tools, &own()?),
        Source::Adapter(path) => {
            let dir = path.parent().ok_or("adapter path has no parent")?;
            Ok(Built {
                path: path.clone(),
                source: path.display().to_string(),
                commit: "prebuilt".into(),
                libs: Some(dir.to_path_buf()),
            })
        }
    }
}

pub fn list(app: &str, source: &Source, class: &str) -> Result<String, String> {
    let tools = tools::cache()?;
    let built = build(app, source, &tools)?;
    let mut client = Client::start(&built.path, class, app, built.libs.as_deref())?;
    let book = shots::book(client.call(&Request::Shots)?.as_str().unwrap_or_default())?;
    client.close();
    Ok(book
        .iter()
        .map(|(name, shot)| {
            format!(
                "{name}  {}  ({} s, fps {:?}, size {:?})\n",
                shot.line, shot.seconds, shot.fps, shot.size
            )
        })
        .collect())
}

#[derive(Serialize)]
struct Manifest {
    tool: ToolRecord,
    product: String,
    kind: &'static str,
    source: SourceRecord,
    params: Value,
    device: String,
    colour: Value,
    outputs: Value,
    seconds: f64,
    made: String,
    files: Vec<Value>,
    version: Option<String>,
    pinned: bool,
    adapter: AdapterRecord,
    shot: String,
    shot_sha256: String,
    resolved: Shot,
    fixtures: FixtureRecord,
    driver: String,
    ffmpeg: Option<EncoderRecord>,
    master: Target,
    targets: Vec<Target>,
    stages: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kept_frames: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<AudioRecord>,
}

#[derive(Serialize, Clone)]
struct AudioRecord {
    codec: &'static str,
    bitrate: &'static str,
    rate: u32,
    channels: u16,
    seconds: f64,
    pcm_format: &'static str,
    pcm_frames: u64,
    pcm_sha256: String,
}

#[derive(Serialize)]
struct ToolRecord {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct SourceRecord {
    path: String,
    sha256: Option<String>,
    commit: String,
}

#[derive(Serialize)]
struct AdapterRecord {
    reported_rev: String,
    sha256: String,
    depth: u8,
}

#[derive(Serialize)]
struct FixtureRecord {
    name: String,
    files: BTreeMap<String, String>,
}

#[derive(Serialize, Clone)]
struct Target {
    fps: u32,
    size: String,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
struct EncoderRecord {
    url: String,
    sha256: String,
}

struct Made {
    path: PathBuf,
    what: String,
    encoded: Option<Value>,
}

fn linear_made(path: PathBuf) -> Made {
    Made {
        path,
        what: "scene-linear RGBA half-float OpenEXR, BT.709 primaries".into(),
        encoded: Some(json!({"role": "linear"})),
    }
}

fn request_hdr(client: &mut Client, raw: &Path, width: u32, height: u32) -> Result<(), String> {
    let written: Written =
        typed(client.call_value(json!({"op": "hdr", "path": raw.display().to_string()}))?)?;
    if written.size != [width, height] {
        return Err(format!(
            "HDR capture is {}x{}, expected {width}x{height}",
            written.size[0], written.size[1]
        ));
    }
    let expected = width as usize * height as usize * 8;
    let length = std::fs::metadata(raw)
        .map_err(|e| format!("{}: {e}", raw.display()))?
        .len();
    if length != expected as u64 {
        return Err(format!(
            "HDR capture has {length} bytes, expected {expected}"
        ));
    }
    Ok(())
}

fn write_linear_exr(bytes: &[u8], output: &Path, width: u32, height: u32) -> Result<(), String> {
    let expected = width as usize * height as usize * 8;
    if bytes.len() != expected {
        return Err(format!(
            "HDR capture has {} bytes, expected {expected}",
            bytes.len()
        ));
    }
    let halves: Vec<f16> = bytes
        .chunks_exact(2)
        .map(|pair| f16::from_bits(u16::from_le_bytes([pair[0], pair[1]])))
        .collect();
    let channels = SpecificChannels::rgba(|Vec2(x, y)| {
        let index = (y * width as usize + x) * 4;
        (
            halves[index],
            halves[index + 1],
            halves[index + 2],
            halves[index + 3],
        )
    });
    let mut image = ExrImage::from_channels((width as usize, height as usize), channels);
    image.attributes.chromaticities = Some(Chromaticities {
        red: Vec2(0.64, 0.33),
        green: Vec2(0.30, 0.60),
        blue: Vec2(0.15, 0.06),
        white: Vec2(0.3127, 0.3290),
    });
    image
        .write()
        .to_file(output)
        .map_err(|e| format!("{}: {e}", output.display()))
}

fn linear_pick(
    source: &Path,
    pick: Pick,
    width: u32,
    height: u32,
    output: &Path,
) -> Result<(), String> {
    let read =
        |i: usize| std::fs::read(source.join(format!("{i:05}.rgba"))).map_err(|e| e.to_string());
    let bytes = match pick {
        Pick::One(i) => read(i)?,
        Pick::Blend(a, b, t) => {
            let mut first = read(a)?;
            let second = read(b)?;
            if first.len() != second.len() {
                return Err("HDR crossfade frames have different sizes".into());
            }
            for (left, right) in first.chunks_exact_mut(2).zip(second.chunks_exact(2)) {
                let a = f16::from_bits(u16::from_le_bytes([left[0], left[1]])).to_f32();
                let b = f16::from_bits(u16::from_le_bytes([right[0], right[1]])).to_f32();
                left.copy_from_slice(&f16::from_f32(a + (b - a) * t).to_bits().to_le_bytes());
            }
            first
        }
    };
    write_linear_exr(&bytes, output, width, height)
}

fn made(path: PathBuf, what: &str) -> Made {
    Made {
        path,
        what: what.to_string(),
        encoded: None,
    }
}

fn entry(dir: &Path, m: &Made) -> Result<Value, String> {
    let bytes = std::fs::metadata(&m.path)
        .map_err(|e| format!("{}: {e}", m.path.display()))?
        .len();
    let file = m
        .path
        .strip_prefix(dir)
        .unwrap_or(&m.path)
        .to_string_lossy()
        .replace('\\', "/");
    let mut v = json!({
        "file": file,
        "sha256": tools::hash_file(&m.path)?,
        "bytes": bytes,
        "what": m.what,
    });
    if let Some(Value::Object(extra)) = m.encoded.as_ref() {
        for (k, x) in extra {
            if k != "file" && k != "bytes" {
                v[k] = x.clone();
            }
        }
    }
    Ok(v)
}

fn colour(depth: u8) -> Value {
    json!({
        "master": "sRGB, 16-bit RGB",
        "delivery": "BT.709 primaries, transfer and matrix, limited range, SDR",
        "adapter_depth": depth,
    })
}

fn outputs() -> Value {
    json!({
        "master": MASTER_WHAT,
        "poster": "8-bit sRGB PNG with an sRGB chunk, the first frame",
        "resampling": "Lanczos3 in linear light, premultiplied",
    })
}

pub fn version(source: &Source, commit: &str) -> Result<String, String> {
    if let Source::Git(reference) = source
        && reference.strip_prefix('v').is_some_and(|numbers| {
            let parts: Vec<&str> = numbers.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
        })
    {
        return Ok(reference.clone());
    }
    short_version(commit)
}

fn pinned(source: &Source, commit: &str) -> bool {
    matches!(source, Source::Git(_)) && !commit.ends_with("+dirty")
}

fn short_version(commit: &str) -> Result<String, String> {
    let raw = commit.strip_suffix("+dirty").unwrap_or(commit);
    if raw.len() < 12 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("the archive version needs a Git commit of at least 12 hex digits".into());
    }
    let short = &raw[..12];
    if commit.ends_with("+dirty") {
        Ok(format!("{short}-dirty"))
    } else {
        Ok(short.to_string())
    }
}

fn fresh(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn matte(black: &Frame, white: &Frame, bg: [f32; 3], wg: [f32; 3]) -> Frame {
    let (b, w) = (black.linear(), white.linear());
    let span = ((wg[0] - bg[0]) + (wg[1] - bg[1]) + (wg[2] - bg[2])) / 3.0;
    let mut rgba = Vec::with_capacity(b.rgba.len());
    for (pb, pw) in b.rgba.chunks_exact(4).zip(w.rgba.chunks_exact(4)) {
        let diff = ((pw[0] - pb[0]) + (pw[1] - pb[1]) + (pw[2] - pb[2])) / 3.0;
        let alpha = (1.0 - diff / span.max(1e-4)).clamp(0.0, 1.0);
        let un = |c: f32, g: f32| {
            if alpha > 1e-4 {
                ((c - g * (1.0 - alpha)) / alpha).max(0.0)
            } else {
                0.0
            }
        };
        rgba.extend_from_slice(&[un(pb[0], bg[0]), un(pb[1], bg[1]), un(pb[2], bg[2]), alpha]);
    }
    Linear {
        width: b.width,
        height: b.height,
        rgba,
    }
    .frame()
}

fn mix_cutouts(a: &Linear, b: &Linear, t: f32) -> Linear {
    let mut rgba = Vec::with_capacity(a.rgba.len());
    for (left, right) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
        let alpha = left[3] * (1.0 - t) + right[3] * t;
        let inv = if alpha > 1e-6 { 1.0 / alpha } else { 0.0 };
        for channel in 0..3 {
            rgba.push((left[channel] * left[3] * (1.0 - t) + right[channel] * right[3] * t) * inv);
        }
        rgba.push(alpha);
    }
    Linear {
        width: a.width,
        height: a.height,
        rgba,
    }
}

fn resolve(o: &Options, found: &Shot) -> Result<Shot, String> {
    let mut shot = shots::set(found, &o.set)?;
    shots::daylight(&mut shot)?;
    if let Some(fps) = &o.fps {
        shot.fps = fps.clone();
    }
    if o.size.is_some() || !o.derive.is_empty() {
        let largest = match &o.size {
            Some(s) => shots::named(s)?,
            None => shot.master()?.1,
        };
        let mut sizes = vec![largest];
        for d in &o.derive {
            let n = shots::named(d)?;
            if !sizes.contains(&n) {
                sizes.push(n);
            }
        }
        shot.size = sizes;
    }
    if let Some(seconds) = o.seconds {
        shot.seconds = seconds;
    }
    Ok(shot)
}

fn session(o: &Options, kind: &str, job: &mut Job) -> Result<Session, String> {
    let tmp = tools::caller_tmp()?;
    let root = o.out.clone().unwrap_or_else(|| tmp.join("renders"));
    let dir = tools::inside(&tmp, &root.join(&o.app).join(kind))?;
    job.out(&dir);
    let tools_dir = tools::cache()?;
    let started = Instant::now();
    let made_at = archive::local_made();
    let mut stages: BTreeMap<String, f64> = BTreeMap::new();
    job.stage(stages::BUILD, None, "steps");
    let built = build(&o.app, &o.source, &tools_dir)?;
    stages.insert("build".into(), started.elapsed().as_secs_f64());
    job.stage(
        stages::TRACE,
        None,
        if kind == STILL { "samples" } else { "frames" },
    );
    let mut client = Client::start(&built.path, &o.class, &o.app, built.libs.as_deref())?;
    job.pgpu(client.queued.then(|| client.child.id()));
    let hello_reply = client.call(&Request::Hello)?;
    let hdr_supported = hello_reply["hdr"].as_bool().unwrap_or(false);
    if o.hdr && !hdr_supported {
        return Err(format!("{}'s adapter does not support HDR capture", o.app));
    }
    let hello: Hello = typed(hello_reply)?;
    let book = shots::book(client.call(&Request::Shots)?.as_str().unwrap_or_default())?;
    let found = book.get(&o.shot).ok_or_else(|| {
        format!(
            "{} has no shot '{}'; it has {}",
            o.app,
            o.shot,
            book.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;
    let shot = resolve(o, found)?;
    job.detail(format!(
        "{HD_DETAIL}{} {} @{} {}",
        o.app,
        o.shot,
        built.commit.chars().take(12).collect::<String>(),
        shot.size.join("→")
    ));
    let fixtures = o
        .fixtures
        .canonicalize()
        .map_err(|e| format!("{}: {e}", o.fixtures.display()))?;
    let set = fixtures
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if set != shot.fixtures && !o.force_fixtures {
        return Err(format!(
            "the shot expects fixture set '{}', got a folder named '{set}' (--force-fixtures to override)",
            shot.fixtures
        ));
    }
    let recipe = shots::recipe(&o.app, &o.shot, &shot);
    Ok(Session {
        tmp,
        dir,
        tools_dir,
        started,
        made_at,
        stages,
        built,
        client,
        hello,
        shot,
        fixtures,
        set,
        recipe,
    })
}

fn sample(
    client: &mut Client,
    total: u32,
    moving: u32,
    recipe: &str,
    base: u64,
    frame: u64,
    sampled: &dyn Fn(u32),
) -> Result<(), String> {
    let mut done = 0;
    let mut pass = 0;
    while done < total.max(1) {
        let n = CHUNK.min(total.max(1) - done);
        client.call(&Request::Sample {
            spp: n,
            moving: if pass == 0 { moving } else { 0 },
            seed: shots::seed(recipe, base, frame, pass),
        })?;
        done += n;
        pass += 1;
        sampled(done);
    }
    Ok(())
}

fn timed_inputs(shot: &Shot) -> Result<Vec<(f64, Event)>, String> {
    let mut inputs: Vec<(f64, Event)> = shot
        .inputs
        .iter()
        .map(|i| i.event().map(|e| (i.at, e)))
        .collect::<Result<_, _>>()?;
    inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(inputs)
}

fn opened(
    client: &mut Client,
    o: &Options,
    shot: &Shot,
    fixtures: &Path,
    size: &str,
) -> Result<Opened, String> {
    let (w, h, scale) = shots::size(size)?;
    let opened: Opened = typed(client.call(&Request::Open {
        shot: o.shot.clone(),
        fixtures: fixtures.display().to_string(),
        scale,
        app: shot.app_json(),
    })?)?;
    if opened.size != [w, h] {
        return Err(format!(
            "the adapter opened {}x{}, expected {w}x{h} for {size}",
            opened.size[0], opened.size[1]
        ));
    }
    Ok(opened)
}

fn device(opened: &Opened) -> String {
    match &opened.backend {
        Some(b) if !b.is_empty() => format!("{b}: {}", opened.gpu),
        _ => opened.gpu.clone(),
    }
}

pub fn still(o: &Options) -> Result<PathBuf, String> {
    let mut job = Job::start(TOOL, Some(&o.app), &o.shot);
    job.plan(stages::APP_STILL);
    let made = render_still(o, &mut job);
    job.end(made)
}

fn render_still(o: &Options, job: &mut Job) -> Result<PathBuf, String> {
    let Session {
        tmp,
        dir,
        started,
        made_at,
        mut stages,
        built,
        mut client,
        hello,
        shot,
        fixtures,
        set,
        recipe,
        ..
    } = session(o, STILL, job)?;
    let (fps, master_size) = shot.master()?;
    let (mw, mh, _) = shots::size(&master_size)?;
    if let Some(px) = o.web.iter().find(|px| **px * 2 > mw) {
        return Err(format!(
            "a web copy of {px} px (and its @2x, {} px) would be wider than the {mw} px master; renders never upscale",
            px * 2
        ));
    }
    let opened = opened(&mut client, o, &shot, &fixtures, &master_size)?;
    job.device(&device(&opened));
    job.stage(stages::TRACE, Some(o.samples.max(1) as u64), "samples");
    let inputs = timed_inputs(&shot)?;
    let steps = (o.at.max(0.0) * fps as f64).round() as u64;
    let t1 = Instant::now();
    let mut next = 0;
    for i in 0..=steps {
        let t = i as f64 / fps as f64;
        while next < inputs.len() && inputs[next].0 <= t {
            client.call(&Request::Input {
                event: inputs[next].1.clone(),
            })?;
            next += 1;
        }
        client.call(&Request::Advance { to: t })?;
    }
    sample(&mut client, o.samples, 0, &recipe, o.seed, steps, &|done| {
        job.progress(done as u64)
    })?;
    let scratch = Scratch::new(&tmp, &format!("{}-{STILL}-{}", o.app, o.shot), false)?;
    let raw = scratch.path.join("frame.rgba");
    let _: Written = typed(client.call(&Request::Frame {
        path: raw.display().to_string(),
        ground: None,
        linear: false,
    })?)?;
    let frame = Frame::read_raw(&raw, mw, mh)?;
    std::fs::remove_file(&raw).ok();
    let master_stem = format!("{}-{STILL}-{}-{mw}x{mh}", o.app, o.shot);
    let linear = if o.hdr {
        let output = dir.join(format!("{master_stem}.linear.exr"));
        request_hdr(&mut client, &raw, mw, mh)?;
        write_linear_exr(
            &std::fs::read(&raw).map_err(|e| e.to_string())?,
            &output,
            mw,
            mh,
        )?;
        std::fs::remove_file(&raw).map_err(|e| e.to_string())?;
        Some(linear_made(output))
    } else {
        None
    };
    client.close();
    job.pgpu(None);
    stages.insert("trace".into(), t1.elapsed().as_secs_f64());
    let t2 = Instant::now();
    let mut sizes: Vec<(u32, u32)> = shot
        .size
        .iter()
        .map(|s| shots::size(s).map(|(w, h, _)| (w, h)))
        .collect::<Result<_, _>>()?;
    sizes.sort_by(|a, b| b.cmp(a));
    sizes.dedup();
    job.stage(
        stages::DERIVE,
        Some((1 + usize::from(o.hdr) + sizes.len() + 2 * o.web.len()) as u64),
        "outputs",
    );
    let stem = |w: u32, h: u32| format!("{}-{STILL}-{}-{w}x{h}", o.app, o.shot);
    let mut made: Vec<Made> = linear.into_iter().collect();
    let master = dir.join(format!("{master_stem}.master.png"));
    frame.save(&master)?;
    made.push(Made {
        path: master,
        what: "lossless master, 16-bit sRGB PNG".into(),
        encoded: None,
    });
    job.progress(made.len() as u64);
    let lin = frame.linear();
    for (w, h) in &sizes {
        let path = dir.join(format!("{}.png", stem(*w, *h)));
        lin.resize(*w, *h).frame().save8(&path)?;
        made.push(Made {
            path,
            what: "delivery, 8-bit sRGB PNG".into(),
            encoded: None,
        });
        job.progress(made.len() as u64);
    }
    for px in &o.web {
        for (factor, suffix) in [(1, ""), (2, "@2x")] {
            let w = px * factor;
            let h = ((w as f64) * mh as f64 / mw as f64).round().max(1.0) as u32;
            let path = dir.join(format!("{master_stem}-web-{px}{suffix}.png"));
            lin.resize(w, h).frame().save8(&path)?;
            made.push(Made {
                path,
                what: format!("web copy, {w}x{h}, 8-bit sRGB PNG"),
                encoded: None,
            });
            job.progress(made.len() as u64);
        }
    }
    stages.insert("derive".into(), t2.elapsed().as_secs_f64());
    let files = made
        .iter()
        .map(|m| entry(&dir, m))
        .collect::<Result<Vec<_>, _>>()?;
    let manifest = json!({
        "tool": { "name": TOOL, "version": env!("CARGO_PKG_VERSION") },
        "product": o.app,
        "kind": STILL,
        "source": { "path": built.source, "sha256": null, "commit": built.commit },
        "params": {
            "size": [mw, mh],
            "derive": sizes.iter().skip(1).map(|(w, h)| [*w, *h]).collect::<Vec<_>>(),
            "web": o.web,
            "at": o.at,
            "samples": o.samples,
            "fps": fps,
            "seed": o.seed,
            "set": o.set,
            "class": o.class,
            "hdr": o.hdr,
        },
        "device": device(&opened),
        "colour": {
            "master": "sRGB, 16-bit RGB",
            "delivery": "8-bit sRGB PNG with an sRGB chunk",
            "adapter_depth": hello.depth,
        },
        "outputs": {
            "delivery": "8-bit sRGB PNG with an sRGB chunk",
            "master": "lossless 16-bit sRGB PNG with an sRGB chunk",
            "resampling": "Lanczos3 in linear light, premultiplied",
        },
        "seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        "made": made_at,
        "files": files,
        "version": version(&o.source, &built.commit).ok(),
        "pinned": pinned(&o.source, &built.commit),
        "adapter": {
            "reported_rev": hello.rev,
            "sha256": tools::hash_file(&built.path)?,
            "depth": hello.depth,
        },
        "shot": o.shot,
        "shot_sha256": recipe,
        "resolved": shot,
        "fixtures": { "name": set, "files": tools::hash_tree(&fixtures)? },
        "driver": opened.driver,
        "stages": stages,
    });
    let manifest_path = dir.join(format!("{master_stem}.json"));
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    job.archiving(archive::reason(&manifest_path, o.no_archive).is_none());
    if archive::archive(&manifest_path, o.no_archive) == archive::State::Queued {
        job.note("archive queued: share offline");
    }
    Ok(manifest_path)
}

pub fn shot(o: &Options) -> Result<PathBuf, String> {
    let mut job = Job::start(TOOL, Some(&o.app), &o.shot);
    job.plan(stages::APP_CLIP);
    let made = render_clip(o, &mut job);
    job.end(made)
}

fn render_clip(o: &Options, job: &mut Job) -> Result<PathBuf, String> {
    if !o.derive.is_empty() {
        return Err("clip --derive belongs to pconv; use pconv for more video sizes".into());
    }
    let Session {
        tmp,
        dir,
        tools_dir,
        started,
        made_at,
        mut stages,
        built,
        mut client,
        hello,
        shot,
        fixtures,
        set,
        recipe,
    } = session(o, KIND, job)?;
    if shot.cutouts && !hello.cutouts {
        return Err(format!(
            "the shot asks for cutouts, and {}'s adapter cannot draw them yet",
            o.app
        ));
    }
    let closure = shot.closure()?;
    let (master_fps, master_size) = shot.master()?;
    let (mw, mh, scale) = shots::size(&master_size)?;
    let count = loops::needed(closure, shot.seconds, master_fps, &[master_fps]);
    let opened: Opened = typed(client.call(&Request::Open {
        shot: o.shot.clone(),
        fixtures: fixtures.display().to_string(),
        scale,
        app: shot.app_json(),
    })?)?;
    if opened.size != [mw, mh] {
        return Err(format!(
            "the adapter opened {}x{}, expected {mw}x{mh} for {master_size}",
            opened.size[0], opened.size[1]
        ));
    }
    job.device(&device(&opened));
    job.stage(stages::TRACE, Some(count as u64), "frames");
    let master_stem = encode::stem(&o.app, &o.shot, mw, mh, master_fps);
    let scratch = Scratch::new(&tmp, &master_stem, o.keep_frames)?;
    let frames_dir = scratch.path.join("frames");
    std::fs::create_dir(&frames_dir).map_err(|e| format!("{}: {e}", frames_dir.display()))?;
    let linear_source = scratch.path.join("linear-original");
    let linear_dir = dir.join(format!("{master_stem}.linear"));
    if o.hdr {
        std::fs::create_dir(&linear_source).map_err(|e| e.to_string())?;
        fresh(&linear_dir)?;
    }
    let cut_dir = dir.join(format!("{master_stem}.cutouts"));
    if shot.cutouts {
        fresh(&cut_dir)?;
    }
    let mut outputs_made: Vec<Made> = Vec::new();
    let raw = scratch.path.join("frame.rgba");
    let raw_s = raw.display().to_string();
    let inputs = timed_inputs(&shot)?;
    let mut next = 0;
    let t1 = Instant::now();
    for i in 0..count {
        let t = i as f64 / master_fps as f64;
        while next < inputs.len() && inputs[next].0 <= t {
            client.call(&Request::Input {
                event: inputs[next].1.clone(),
            })?;
            next += 1;
        }
        client.call(&Request::Advance { to: t })?;
        let (spp, moving) = if i == 0 {
            (shot.warmup, 0)
        } else {
            (shot.per_frame, shot.moving)
        };
        sample(&mut client, spp, moving, &recipe, o.seed, i as u64, &|_| {})?;
        let _: Written = typed(client.call(&Request::Frame {
            path: raw_s.clone(),
            ground: None,
            linear: false,
        })?)?;
        Frame::read_raw(&raw, mw, mh)?.save(&frames_dir.join(format!("{i:05}.png")))?;
        if o.hdr {
            request_hdr(
                &mut client,
                &linear_source.join(format!("{i:05}.rgba")),
                mw,
                mh,
            )?;
        }
        if shot.cutouts {
            let b: Written = typed(client.call(&Request::Frame {
                path: raw_s.clone(),
                ground: Some([0.0; 3]),
                linear: true,
            })?)?;
            let black = Frame::read_raw(&raw, mw, mh)?;
            let w: Written = typed(client.call(&Request::Frame {
                path: raw_s.clone(),
                ground: Some([1.0; 3]),
                linear: true,
            })?)?;
            let white = Frame::read_raw(&raw, mw, mh)?;
            let cut = cut_dir.join(format!("{i:05}.png"));
            matte(
                &black,
                &white,
                b.ground.unwrap_or([0.0; 3]),
                w.ground.unwrap_or([1.0; 3]),
            )
            .save(&cut)?;
        }
        job.progress(i as u64 + 1);
        if i % 30 == 29 || i + 1 == count {
            eprintln!(
                "{TOOL}: {} frame {}/{count} ({:.1} s)",
                o.shot,
                i + 1,
                t1.elapsed().as_secs_f64()
            );
        }
    }
    let _ = std::fs::remove_file(&raw);
    let picks = loops::picks(closure, shot.seconds, master_fps, master_fps);
    let clip_seconds = picks.len() as f64 / master_fps as f64;
    let wav = scratch.path.join("audio.wav");
    let pcm = match (o.encode, hello.audio, hello.audio_format) {
        (true, true, Some(format)) => {
            let sounded: Sounded = typed(client.call(&Request::Audio {
                path: wav.display().to_string(),
                seconds: clip_seconds,
            })?)?;
            let bytes = std::fs::read(&wav).map_err(|e| format!("{}: {e}", wav.display()))?;
            let pcm = audio::read(&bytes, format)?;
            if pcm.frames != sounded.frames {
                return Err(format!(
                    "the adapter said it wrote {} audio frames, and its WAV holds {}",
                    sounded.frames, pcm.frames
                ));
            }
            let held = pcm.frames as f64 / f64::from(pcm.rate);
            if (held - clip_seconds).abs() > 1.0 / master_fps as f64 {
                eprintln!(
                    "{TOOL}: the adapter's audio runs {held:.3} s for a {clip_seconds:.3} s clip; the master pads or cuts it to fit"
                );
            }
            Some(pcm)
        }
        (true, true, None) => {
            return Err(format!(
                "{}'s adapter announced audio without naming its format",
                o.app
            ));
        }
        _ => None,
    };
    client.close();
    job.pgpu(None);
    stages.insert("trace".into(), t1.elapsed().as_secs_f64());
    let t2 = Instant::now();
    let ffmpeg = if o.encode {
        Some(tools::ffmpeg(&tools_dir)?)
    } else {
        None
    };
    let master_target = Target {
        fps: master_fps,
        size: master_size.clone(),
        width: mw,
        height: mh,
    };
    if shot.cutouts {
        let source = scratch.path.join("cutouts-original");
        std::fs::rename(&cut_dir, &source).map_err(|e| e.to_string())?;
        fresh(&cut_dir)?;
        for (n, pick) in picks.iter().enumerate() {
            let load = |i: usize| {
                Frame::load(&source.join(format!("{i:05}.png"))).map(|frame| frame.linear())
            };
            let frame = match *pick {
                Pick::One(i) => load(i)?,
                Pick::Blend(a, b, blend) => mix_cutouts(&load(a)?, &load(b)?, blend),
            }
            .frame();
            let path = cut_dir.join(format!("{n:05}.png"));
            frame.save(&path)?;
            outputs_made.push(made(path, "cutout frame, 16-bit RGBA PNG"));
        }
        std::fs::remove_dir_all(source).map_err(|e| e.to_string())?;
    }
    let master = dir.join(encode::master(&master_stem));
    let poster = dir.join(encode::poster(&master_stem));
    let encode_started = o.encode.then(Instant::now);
    let with_audio = pcm.is_some();
    let audio_record = pcm.map(|pcm| AudioRecord {
        codec: master_encode::AUDIO_CODEC,
        bitrate: master_encode::AUDIO_BITRATE,
        rate: master_encode::AUDIO_RATE,
        channels: pcm.channels,
        seconds: clip_seconds,
        pcm_format: pcm.format,
        pcm_frames: pcm.frames,
        pcm_sha256: pcm.sha256,
    });
    let silent = scratch.path.join("silent.master.mp4");
    let video_out = if with_audio { &silent } else { &master };
    let mut encoder = if let Some((ff, _)) = &ffmpeg {
        Some(encode::from_raw(ff, mw, mh, master_fps, video_out)?)
    } else {
        None
    };
    let loose = if encoder.is_none() {
        let path = dir.join(format!("{master_stem}.frames"));
        fresh(&path)?;
        Some(path)
    } else {
        None
    };
    job.stage(stages::MASTER, Some(picks.len() as u64), "frames");
    for (n, pick) in picks.iter().enumerate() {
        if o.hdr {
            let output = linear_dir.join(format!("{n:05}.exr"));
            linear_pick(&linear_source, *pick, mw, mh, &output)?;
            outputs_made.push(linear_made(output));
        }
        let load = |i: usize| {
            Frame::load(&frames_dir.join(format!("{i:05}.png"))).map(|frame| frame.linear())
        };
        let frame = match *pick {
            Pick::One(i) => load(i)?,
            Pick::Blend(a, b, blend) => load(a)?.mix(&load(b)?, blend),
        }
        .resize(mw, mh)
        .frame();
        if n == 0 {
            frame.save8(&poster)?;
            outputs_made.push(made(
                poster.clone(),
                "poster, the first frame, 8-bit sRGB PNG",
            ));
        }
        if let Some(path) = &loose {
            let output = path.join(format!("{n:05}.png"));
            frame.save(&output)?;
            outputs_made.push(made(output, "delivery frame, 16-bit sRGB PNG"));
        }
        if let Some(child) = &mut encoder {
            child
                .stdin
                .as_mut()
                .ok_or("encoder stdin")?
                .write_all(&frame.raw())
                .map_err(|e| format!("encoder: {e}"))?;
        }
        job.progress(n as u64 + 1);
    }
    if let Some(mut child) = encoder {
        drop(child.stdin.take());
        if !child.wait().map_err(|e| e.to_string())?.success() {
            return Err(format!("ffmpeg failed encoding {}", master.display()));
        }
        if with_audio {
            let ff = &ffmpeg.as_ref().ok_or("ffmpeg unavailable")?.0;
            master_encode::mux_audio(ff, &silent, &wav, clip_seconds, &master)?;
        }
        job.stage(
            stages::ENCODE,
            Some(if shot.cutouts { 2 } else { 1 }),
            "outputs",
        );
        let size = format!("{mw}x{mh}");
        let args = master_encode::master_args(master_fps);
        outputs_made.push(Made {
            path: master,
            what: MASTER_WHAT.into(),
            encoded: Some(json!({"tier": "master", "size": size, "fps": master_fps, "args": args, "depth": 8, "chroma": "4:2:0", "color": master_encode::COLOR, "audio": audio_record.as_ref().map(|a| a.codec)})),
        });
        job.progress(1);
        if shot.cutouts {
            let alpha = dir.join(format!("{master_stem}.alpha.master.mov"));
            let ff = &ffmpeg.as_ref().ok_or("ffmpeg unavailable")?.0;
            master_encode::alpha_sequence(ff, &cut_dir, master_fps, &alpha)?;
            outputs_made.push(Made {
                path: alpha,
                what: "ProRes 4444 alpha master".into(),
                encoded: Some(json!({"tier": "alpha master", "fps": master_fps, "args": master_encode::alpha_args(), "depth": 10, "chroma": "4:4:4"})),
            });
            job.progress(2);
        }
    }
    if let Some(started) = encode_started {
        stages.insert("encode".into(), started.elapsed().as_secs_f64());
    }
    stages.insert("master".into(), t2.elapsed().as_secs_f64());
    if o.keep_frames {
        eprintln!("{TOOL}: kept the traced frames in {}", frames_dir.display());
    }
    let files = outputs_made
        .iter()
        .map(|m| entry(&dir, m))
        .collect::<Result<Vec<_>, _>>()?;
    let manifest = Manifest {
        tool: ToolRecord {
            name: TOOL,
            version: env!("CARGO_PKG_VERSION"),
        },
        audio: audio_record,
        product: o.app.clone(),
        kind: KIND,
        source: SourceRecord {
            path: built.source.clone(),
            sha256: None,
            commit: built.commit.clone(),
        },
        params: json!({
            "size": [mw, mh], "fps": master_fps, "seconds": shot.seconds,
            "seed": o.seed, "set": o.set, "warmup": shot.warmup,
            "per_frame": shot.per_frame, "moving": shot.moving,
            "frames": count, "closure": shot.closure, "encode": o.encode, "class": o.class,
            "hdr": o.hdr,
        }),
        device: device(&opened),
        colour: colour(hello.depth),
        outputs: outputs(),
        seconds: (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        made: made_at,
        files,
        version: version(&o.source, &built.commit).ok(),
        pinned: pinned(&o.source, &built.commit),
        adapter: AdapterRecord {
            reported_rev: hello.rev,
            sha256: tools::hash_file(&built.path)?,
            depth: hello.depth,
        },
        shot: o.shot.clone(),
        shot_sha256: recipe,
        resolved: shot.clone(),
        fixtures: FixtureRecord {
            name: set,
            files: tools::hash_tree(&fixtures)?,
        },
        driver: opened.driver,
        ffmpeg: ffmpeg.map(|(_, pin)| EncoderRecord {
            url: pin.url,
            sha256: pin.sha256,
        }),
        master: master_target.clone(),
        targets: vec![master_target],
        stages,
        kept_frames: o.keep_frames.then(|| frames_dir.display().to_string()),
    };
    let manifest_path = dir.join(format!("{master_stem}.json"));
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    job.archiving(archive::reason(&manifest_path, o.no_archive).is_none());
    if archive::archive(&manifest_path, o.no_archive) == archive::State::Queued {
        job.note("archive queued: share offline");
    }
    Ok(manifest_path)
}

pub fn reencode(path: &Path) -> Result<(), String> {
    reencode_mode(path, true)
}

fn reencode_mode(path: &Path, archive_enabled: bool) -> Result<(), String> {
    let tmp = tools::caller_tmp()?;
    let file = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !file.starts_with(&tmp) {
        return Err(format!(
            "{} is outside the caller's tmp/ ({})",
            file.display(),
            tmp.display()
        ));
    }
    let dir = file.parent().ok_or("the manifest has no folder")?;
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&file).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let tool_name = manifest["tool"]["name"].as_str().unwrap_or("");
    if tool_name != TOOL && tool_name != EARLIER_TOOL {
        return Err(format!("{} is not a {TOOL} manifest", file.display()));
    }
    let product = manifest["product"].as_str().unwrap_or_default().to_string();
    let shot = manifest["shot"]
        .as_str()
        .ok_or("the manifest names no shot")?
        .to_string();
    let converted_version = if manifest["version"].is_null() {
        Some(short_version(
            manifest["source"]["commit"].as_str().unwrap_or_default(),
        )?)
    } else {
        None
    };
    let mut job = if archive_enabled {
        Job::start(TOOL, Some(&product), &shot)
    } else {
        Job::disabled()
    };
    job.plan(stages::APP_ENCODE);
    job.detail(format!("{HD_DETAIL}encode {product} {shot}"));
    job.out(dir);
    let result = (|| -> Result<(), String> {
        let old = manifest["files"]
            .as_array()
            .and_then(|files| {
                files
                    .iter()
                    .filter_map(|f| f["file"].as_str())
                    .find(|name| name.ends_with(".master.mkv"))
            })
            .ok_or("the manifest lists no .master.mkv")?
            .to_string();
        let old_path = dir.join(&old);
        if !old_path.is_file() {
            return Err(format!("pfx encode needs {}", old_path.display()));
        }
        let stem = old.trim_end_matches(".master.mkv");
        let fps = manifest["master"]["fps"]
            .as_u64()
            .or_else(|| manifest["params"]["fps"].as_u64())
            .unwrap_or(60) as u32;
        let (ffmpeg, pin) = tools::ffmpeg(&tools::cache()?)?;
        let new_path = dir.join(encode::master(stem));
        job.stage(stages::ENCODE, Some(1), "outputs");
        encode::from_file(&ffmpeg, &old_path, fps, &new_path)?;
        let mut files: Vec<Value> = manifest["files"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| {
                let name = f["file"].as_str().unwrap_or_default();
                !name.ends_with(".master.mkv")
                    && !name.ends_with(".hevc.mp4")
                    && !name.ends_with(".youtube.mp4")
                    && !name.ends_with(".av1.mp4")
                    && !name.ends_with(".h264.mp4")
                    && !name.ends_with(".web.hevc.mp4")
            })
            .collect();
        files.push(entry(dir, &Made {
            path: new_path,
            what: MASTER_WHAT.into(),
            encoded: Some(json!({"tier": "master", "fps": fps, "args": master_encode::master_args(fps), "depth": 8, "chroma": "4:2:0", "color": master_encode::COLOR})),
        })?);
        let poster = dir.join(encode::poster(stem));
        if !poster.exists() {
            encode::first_frame(&ffmpeg, &dir.join(encode::master(stem)), &poster)?;
            files.push(entry(
                dir,
                &made(poster, "poster, the first frame, 8-bit sRGB PNG"),
            )?);
        }
        let cutouts = dir.join(format!("{stem}.cutouts"));
        if cutouts.is_dir() {
            let alpha = dir.join(format!("{stem}.alpha.master.mov"));
            master_encode::alpha_sequence(&ffmpeg, &cutouts, fps, &alpha)?;
            files.push(entry(dir, &Made {
                path: alpha,
                what: "ProRes 4444 alpha master".into(),
                encoded: Some(json!({"tier": "alpha master", "fps": fps, "args": master_encode::alpha_args(), "depth": 10, "chroma": "4:4:4"})),
            })?);
        }
        manifest["files"] = json!(files);
        manifest["outputs"] = outputs();
        manifest["ffmpeg"] = json!({"url": pin.url, "sha256": pin.sha256});
        manifest["encoded_by"] = json!({"name": TOOL, "version": env!("CARGO_PKG_VERSION")});
        if let Some(value) = &converted_version {
            manifest["version"] = json!(value);
        }
        std::fs::write(
            &file,
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::remove_file(old_path).map_err(|e| e.to_string())?;
        job.progress(1);
        if archive_enabled {
            job.archiving(archive::reason(&file, false).is_none());
            if archive::archive(&file, false) == archive::State::Queued {
                job.note("archive queued: share offline");
            }
        }
        Ok(())
    })();
    job.end(result)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn linear_exr_round_trips_half_pixels_and_primaries() {
        let path = PathBuf::from(format!("tmp/hdr-exr-roundtrip-{}.exr", std::process::id()));
        std::fs::create_dir_all("tmp").unwrap();
        let pixels = [
            [0x0000u16, 0x3800, 0x3c00, 0x3c00],
            [0x4000, 0xbc00, 0x3400, 0x3800],
            [0x3c00, 0x4000, 0x4200, 0x3c00],
            [0x3400, 0x3800, 0x3c00, 0x4000],
        ];
        let bytes: Vec<u8> = pixels
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        write_linear_exr(&bytes, &path, 2, 2).unwrap();
        let read = exr::prelude::read_first_rgba_layer_from_file(
            &path,
            |size, _| vec![[f16::from_f32(0.0); 4]; size.width() * size.height()],
            |out, position, (r, g, b, a): (f16, f16, f16, f16)| {
                out[position.y() * 2 + position.x()] = [r, g, b, a];
            },
        )
        .unwrap();
        let actual: Vec<[u16; 4]> = read
            .layer_data
            .channel_data
            .pixels
            .iter()
            .map(|pixel| pixel.map(f16::to_bits))
            .collect();
        assert_eq!(actual, pixels);
        assert_eq!(
            read.attributes.chromaticities.unwrap().red,
            Vec2(0.64, 0.33)
        );
        let listing = entry(path.parent().unwrap(), &linear_made(path.clone())).unwrap();
        assert_eq!(listing["role"], "linear");
        assert_eq!(listing["file"], path.file_name().unwrap().to_str().unwrap());
        std::fs::remove_file(path).unwrap();
    }
    use std::time::{Duration, Instant};

    #[test]
    fn versions_follow_tags_commits_and_dirty_trees() {
        assert_eq!(
            version(&Source::Git("v1.2.3".into()), "aabbccddeeff9988").unwrap(),
            "v1.2.3"
        );
        assert_eq!(
            version(&Source::Git("main".into()), "aabbccddeeff9988").unwrap(),
            "aabbccddeeff"
        );
        assert_eq!(
            version(&Source::Git("vpreview".into()), "aabbccddeeff9988").unwrap(),
            "aabbccddeeff"
        );
        assert_eq!(
            version(&Source::Dir(PathBuf::from(".")), "aabbccddeeff9988+dirty").unwrap(),
            "aabbccddeeff-dirty"
        );
        assert!(version(&Source::Adapter(PathBuf::from("adapter")), "prebuilt").is_err());
    }

    #[test]
    fn only_a_clean_revision_of_the_app_is_pinned() {
        assert!(pinned(&Source::Git("v1.2.3".into()), "aabbccddeeff9988"));
        assert!(pinned(&Source::Git("main".into()), "aabbccddeeff9988"));
        assert!(!pinned(
            &Source::Dir(PathBuf::from(".")),
            "aabbccddeeff9988"
        ));
        assert!(!pinned(
            &Source::Dir(PathBuf::from(".")),
            "aabbccddeeff9988+dirty"
        ));
        assert!(!pinned(
            &Source::Adapter(PathBuf::from("adapter")),
            "prebuilt"
        ));
    }

    #[test]
    fn cutout_crossfade_uses_premultiplied_colour() {
        let visible = Linear {
            width: 1,
            height: 1,
            rgba: vec![1.0, 0.0, 0.0, 1.0],
        };
        let invisible = Linear {
            width: 1,
            height: 1,
            rgba: vec![0.0, 0.0, 1.0, 0.0],
        };
        let mixed = mix_cutouts(&visible, &invisible, 0.5);
        assert_eq!(mixed.rgba, [1.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn older_ffv1_manifest_becomes_x264_without_writing_an_archive() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("ffv1-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("alpha-clip-fixture-64x64-4fps.master.mkv");
        let (ffmpeg, _) = tools::ffmpeg(&tools::cache().unwrap()).unwrap();
        let status = Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=4",
                "-frames:v",
                "4",
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "gbrp16le",
            ])
            .arg(&old)
            .status()
            .unwrap();
        assert!(status.success());
        let manifest = dir.join("alpha-clip-fixture-64x64-4fps.json");
        let old_name = old.file_name().unwrap().to_string_lossy().to_string();
        std::fs::write(
            &manifest,
            serde_json::to_vec(&json!({
                "tool": {"name": "pito-hd-renderer"},
                "product": "alpha", "kind": "clip", "shot": "fixture",
                "source": {"commit": "aabbccddeeff9988"},
                "master": {"fps": 4}, "params": {"fps": 4},
                "files": [{"file": old_name, "sha256": tools::hash_file(&old).unwrap()}]
            }))
            .unwrap(),
        )
        .unwrap();
        reencode_mode(&manifest, false).unwrap();
        assert!(!old.exists());
        let converted: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        assert_eq!(converted["version"], "aabbccddeeff");
        assert!(converted["archive"].is_null());
        assert!(
            dir.join("alpha-clip-fixture-64x64-4fps.master.mp4")
                .is_file()
        );
        assert!(
            converted["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["file"] == "alpha-clip-fixture-64x64-4fps.master.mp4")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_adapter_that_lingers_after_close_is_ended_before_its_teardown() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("linger-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("adapter.sh");
        std::fs::write(
            &script,
            "while read line; do echo '{\"ok\":null}'; case \"$line\" in *close*) exec sleep 60;; esac; done\n",
        )
        .unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg(&script);
        let mut client = Client::spawn(cmd, false).unwrap();
        client.call(&Request::Hello).unwrap();
        let asked = Instant::now();
        client.close();
        assert!(asked.elapsed() < Duration::from_secs(10));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn scripted(name: &str, body: &str) -> (PathBuf, Client) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("adapter.sh");
        std::fs::write(&script, body).unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg(&script);
        (dir, Client::spawn(cmd, false).unwrap())
    }

    #[test]
    fn stray_stdout_lines_before_and_between_replies_are_skipped() {
        let (dir, mut client) = scripted(
            "stray",
            "echo 'amdgpu: context priority changed to -512'\nread line; echo '{\"ok\":1}'\nread line; echo 'noise'; echo '{\"broken'; echo '{\"ok\":2}'\nread line; echo '{\"err\":\"no\"}'\n",
        );
        assert_eq!(client.call(&Request::Hello).unwrap(), json!(1));
        assert_eq!(client.call(&Request::Shots).unwrap(), json!(2));
        assert_eq!(client.call(&Request::Shots).unwrap_err(), "adapter: no");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_panicking_adapter_is_reported_by_its_panic_and_never_by_a_stray_line() {
        let (dir, mut client) = scripted(
            "panicked",
            "echo 'amdgpu: context priority changed to -512'\nread line\necho 'thread main panicked at src/lib.rs:7:3:' >&2\necho 'the frame was empty' >&2\nexit 101\n",
        );
        let error = client.call(&Request::Hello).unwrap_err();
        assert!(error.contains("exit status: 101"), "{error}");
        assert!(
            error.contains("it panicked: thread main panicked at src/lib.rs:7:3:"),
            "{error}"
        );
        assert!(error.contains("the frame was empty"), "{error}");
        assert!(!error.contains("amdgpu"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_adapter_that_dies_silently_says_so() {
        let (dir, mut client) = scripted("silent", "read line\nexit 3\n");
        let error = client.call(&Request::Hello).unwrap_err();
        assert!(error.contains("exit status: 3"), "{error}");
        assert!(error.contains("wrote nothing to stderr"), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_render_that_stops_on_an_error_ends_its_adapter_too() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("stopped-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("adapter.sh");
        std::fs::write(&script, "read line; echo '{\"ok\":null}'; exec sleep 60\n").unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg(&script);
        let mut client = Client::spawn(cmd, false).unwrap();
        client.call(&Request::Hello).unwrap();
        let asked = Instant::now();
        drop(client);
        assert!(asked.elapsed() < Duration::from_secs(10));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    fn pgpu_log_of(label: &str, args: &[&str]) -> String {
        use std::os::unix::fs::PermissionsExt;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let profile = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let pfx = profile.join("pfx");
        let checker = profile.join("examples").join("checker");
        assert!(checker.is_file() && pfx.is_file());
        let base = root
            .join("tmp")
            .join(format!("as-{label}-{}", std::process::id()));
        let tools = base.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        let log = base.join("pgpu.log");
        let pgpu = tools.join("pgpu");
        std::fs::write(&pgpu, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$PGPU_LOG\"\n").unwrap();
        std::fs::set_permissions(&pgpu, std::fs::Permissions::from_mode(0o700)).unwrap();
        Command::new(&pfx)
            .env("PATH", &tools)
            .env("PGPU_LOG", &log)
            .env("PFX_CACHE", base.join("cache"))
            .env_remove("PFX_DIRECT")
            .current_dir(&root)
            .args(args.iter().map(|a| {
                if *a == "{checker}" {
                    checker.display().to_string()
                } else {
                    a.to_string()
                }
            }))
            .status()
            .unwrap();
        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_dir_all(&base).ok();
        logged
    }

    #[cfg(unix)]
    #[test]
    fn an_app_render_asks_pgpu_to_run_as_its_product() {
        let logged = pgpu_log_of(
            "app",
            &[
                "list",
                "--shots",
                "--app",
                "beta",
                "--class",
                "interactive",
                "--adapter",
                "{checker}",
            ],
        );
        let first = logged.lines().next().unwrap_or_default();
        assert!(
            first.starts_with("run --class interactive --as beta -- "),
            "{first}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_asset_render_asks_pgpu_to_run_pfx_as_pfx() {
        let logged = pgpu_log_of(
            "asset",
            &[
                "render",
                "--icons",
                "shapes",
                "--product",
                "sample",
                "--recipes",
                "crates/assets/tests/recipes",
                "--no-archive",
            ],
        );
        let first = logged.lines().next().unwrap_or_default();
        assert!(
            first.starts_with("run --class truth --as pfx -- ")
                && first.contains("pfx render --icons shapes"),
            "{first}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_queued_wrapper_gets_term_when_pfx_does() {
        for (label, args) in [
            (
                "adapter",
                vec![
                    "list",
                    "--shots",
                    "--app",
                    "beta",
                    "--class",
                    "clip",
                    "--adapter",
                    "{pfx}",
                ],
            ),
            (
                "asset",
                vec![
                    "render",
                    "--icons",
                    "shapes",
                    "--product",
                    "sample",
                    "--recipes",
                    "crates/assets/tests/recipes",
                    "--no-archive",
                ],
            ),
        ] {
            term_reaches(label, &args);
        }
    }

    #[cfg(unix)]
    fn term_reaches(label: &str, args: &[&str]) {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::ExitStatusExt;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let profile = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let pfx = profile.join("pfx");
        assert!(pfx.is_file(), "{}", pfx.display());
        let base = root
            .join("tmp")
            .join(format!("term-{label}-{}", std::process::id()));
        let tools = base.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        let started = base.join("started");
        let got = base.join("got");
        let pgpu = tools.join("pgpu");
        std::fs::write(
            &pgpu,
            "#!/usr/bin/python3\nimport os, signal, time\ndef stop(signum, frame):\n    open(os.environ['GOT'], 'w').write('term')\n    raise SystemExit\nsignal.signal(signal.SIGTERM, stop)\nopen(os.environ['STARTED'], 'w').write(str(os.getpid()))\nwhile True:\n    time.sleep(30)\n",
        )
        .unwrap();
        std::fs::set_permissions(&pgpu, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = std::fs::File::create(base.join("err")).unwrap();
        let mut child = Command::new(&pfx)
            .env("PATH", &tools)
            .env("STARTED", &started)
            .env("GOT", &got)
            .env("PFX_CACHE", base.join("cache"))
            .env_remove("PFX_DIRECT")
            .env_remove("PFX_SLICE")
            .current_dir(&root)
            .args(args.iter().map(|arg| {
                if *arg == "{pfx}" {
                    pfx.display().to_string()
                } else {
                    (*arg).to_string()
                }
            }))
            .stdout(Stdio::null())
            .stderr(err)
            .spawn()
            .unwrap();
        let asked = Instant::now();
        while !started.exists() && asked.elapsed() < std::time::Duration::from_secs(20) {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            started.exists(),
            "{label}: pgpu never started\n{}",
            std::fs::read_to_string(base.join("err")).unwrap_or_default()
        );
        let pid = loop {
            let text = std::fs::read_to_string(&started).unwrap_or_default();
            let text = text.trim();
            if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
                break text.to_string();
            }
            assert!(
                asked.elapsed() < std::time::Duration::from_secs(20),
                "{label}: pgpu wrote no pid"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let status = Command::new("kill")
            .args(["-s", "TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let sent = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                sent.elapsed() < std::time::Duration::from_secs(8),
                "{label}: pfx did not exit\n{}",
                std::fs::read_to_string(base.join("err")).unwrap_or_default()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let reflects =
            status.signal() == Some(libc::SIGTERM) || status.code() == Some(128 + libc::SIGTERM);
        assert!(reflects, "{label}: pfx status {status}");
        assert!(
            !std::path::Path::new("/proc").join(&pid).exists(),
            "{label}: wrapper {pid} still running"
        );
        assert_eq!(
            std::fs::read_to_string(&got).unwrap_or_default(),
            "term",
            "{label}"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn simultaneous_clips_keep_every_frame_in_separate_scratch() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let profile = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let checker = profile.join("examples").join("checker");
        let pfx = profile.join("pfx");
        assert!(checker.is_file() && pfx.is_file());
        let base = root
            .join("tmp")
            .join(format!("parallel-clips-{}", std::process::id()));
        let fixtures = base.join("none");
        std::fs::create_dir_all(&fixtures).unwrap();
        std::fs::write(fixtures.join("readme.txt"), "empty set").unwrap();
        let mut children = Vec::new();
        for name in ["a", "b"] {
            let out = base.join(name);
            let child = Command::new(&pfx)
                .env("PFX_DIRECT", "1")
                .env("PFX_CACHE", base.join("cache"))
                .current_dir(&root)
                .args([
                    "render",
                    "--app",
                    "checker",
                    "--shot",
                    "slide",
                    "--clip",
                    "--adapter",
                ])
                .arg(&checker)
                .arg("--fixtures")
                .arg(&fixtures)
                .arg("--out")
                .arg(&out)
                .args([
                    "--size",
                    "720p",
                    "--seconds",
                    "0.1",
                    "--no-encode",
                    "--keep-frames",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            children.push((name, child));
        }
        let mut paths = Vec::new();
        for (name, child) in children {
            let result = child.unwrap().wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let manifest_path = String::from_utf8(result.stdout).unwrap();
            let manifest: Value =
                serde_json::from_slice(&std::fs::read(manifest_path.trim()).unwrap()).unwrap();
            assert_eq!(
                manifest["adapter"]["sha256"],
                tools::hash_file(&checker).unwrap()
            );
            let frames = PathBuf::from(manifest["kept_frames"].as_str().unwrap());
            let count = manifest["params"]["frames"].as_u64().unwrap();
            assert!(frames.starts_with(root.join("tmp/pfx")));
            for i in 0..count {
                assert!(
                    frames.join(format!("{i:05}.png")).is_file(),
                    "{name} frame {i}"
                );
            }
            paths.push(frames);
        }
        assert_ne!(paths[0], paths[1]);
        std::fs::remove_dir_all(&base).unwrap();
        for frames in paths {
            std::fs::remove_dir_all(frames.parent().unwrap()).unwrap();
        }
    }
}
