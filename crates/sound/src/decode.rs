use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::mix::PcmQueue;

pub const AHEAD_SECONDS: u32 = 2;

pub fn ahead_limit(rate: u32, channels: u16) -> usize {
    (rate as usize)
        .saturating_mul(channels as usize)
        .saturating_mul(AHEAD_SECONDS as usize)
}

#[derive(Clone, Debug)]
pub struct DecodeSpec {
    pub ffmpeg: PathBuf,
    pub media: PathBuf,
    pub start: f32,
    pub length: f32,
    pub rate: u32,
    pub channels: u16,
    pub tracks: Vec<(i64, f32)>,
}

#[derive(Clone, Debug)]
pub struct CommandLine {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

pub fn mix_graph(tracks: &[(i64, f32)]) -> String {
    use std::fmt::Write;
    let mut graph = String::new();
    for (k, (stream, volume)) in tracks.iter().enumerate() {
        write!(
            graph,
            "[0:{stream}]volume={volume:.3},aformat=channel_layouts=stereo[t{k}];"
        )
        .unwrap();
    }
    let inputs: String = (0..tracks.len()).map(|k| format!("[t{k}]")).collect();
    write!(
        graph,
        "{inputs}amix=inputs={}:normalize=0[mix]",
        tracks.len()
    )
    .unwrap();
    graph
}

pub fn command_line(spec: &DecodeSpec) -> CommandLine {
    let mut args = Vec::new();
    push(&mut args, "-nostdin");
    push(&mut args, "-v");
    push(&mut args, "error");
    push(&mut args, "-ss");
    push(&mut args, &format!("{:.3}", spec.start.max(0.0)));
    push(&mut args, "-t");
    push(&mut args, &format!("{:.3}", spec.length.max(0.0)));
    push(&mut args, "-i");
    args.push(spec.media.as_os_str().to_os_string());
    if !spec.tracks.is_empty() {
        push(&mut args, "-filter_complex");
        push(&mut args, &mix_graph(&spec.tracks));
        push(&mut args, "-map");
        push(&mut args, "[mix]");
    }
    push(&mut args, "-f");
    push(&mut args, "f32le");
    push(&mut args, "-ac");
    push(&mut args, &decode_channels(spec).to_string());
    push(&mut args, "-ar");
    push(&mut args, &spec.rate.to_string());
    push(&mut args, "-");
    CommandLine {
        program: spec.ffmpeg.clone(),
        args,
    }
}

pub fn decode_channels(spec: &DecodeSpec) -> u16 {
    if spec.tracks.is_empty() {
        spec.channels
    } else {
        2
    }
}

pub(crate) fn spec_at_output(spec: &DecodeSpec, rate: u32, channels: u16) -> DecodeSpec {
    let mut spec = spec.clone();
    spec.rate = rate;
    spec.channels = if spec.tracks.is_empty() { channels } else { 2 };
    spec
}

fn push(args: &mut Vec<OsString>, value: &str) {
    args.push(OsString::from(value));
}

pub struct Decode {
    stop: Arc<AtomicBool>,
    child: Option<Child>,
    queue: Arc<PcmQueue>,
    channels: u16,
    rate: u32,
}

impl Decode {
    pub fn start(spec: &DecodeSpec) -> Result<Decode, String> {
        if !spec.ffmpeg.is_absolute() {
            return Err("ffmpeg path must be absolute".into());
        }
        let channels = decode_channels(spec);
        if spec.rate == 0 || channels == 0 {
            return Err("decode rate and channels must be non-zero".into());
        }
        let line = command_line(spec);
        let mut command = Command::new(&line.program);
        command
            .args(&line.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("ffmpeg returned no audio".into());
        };
        let queue = PcmQueue::new();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_queue = Arc::clone(&queue);
        let thread_stop = Arc::clone(&stop);
        let rate = spec.rate;
        std::thread::spawn(move || pump(stdout, thread_queue, thread_stop, rate, channels));
        Ok(Decode {
            stop,
            child: Some(child),
            queue,
            channels,
            rate,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn ended(&self) -> bool {
        self.queue.reader_ended()
    }

    pub fn buffered(&self) -> usize {
        self.queue.len()
    }

    pub(crate) fn queue(&self) -> Arc<PcmQueue> {
        Arc::clone(&self.queue)
    }
}

impl Drop for Decode {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn pump(
    mut stdout: impl Read,
    queue: Arc<PcmQueue>,
    stop: Arc<AtomicBool>,
    rate: u32,
    channels: u16,
) {
    let mut framer = LeF32::new();
    let mut chunk = vec![0u8; 16_384];
    while !stop.load(Ordering::Relaxed) {
        if ahead_is_full(queue.len(), rate, channels) {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        match stdout.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut decoded = Vec::new();
                framer.push(&chunk[..n], &mut decoded);
                queue.push(&decoded);
            }
        }
    }
    queue.end();
}

struct LeF32 {
    pending: [u8; 4],
    have: usize,
}

impl LeF32 {
    fn new() -> Self {
        Self {
            pending: [0; 4],
            have: 0,
        }
    }

    fn push(&mut self, mut bytes: &[u8], out: &mut Vec<f32>) {
        if self.have > 0 {
            let need = 4 - self.have;
            let take = need.min(bytes.len());
            self.pending[self.have..self.have + take].copy_from_slice(&bytes[..take]);
            self.have += take;
            bytes = &bytes[take..];
            if self.have == 4 {
                out.push(f32::from_le_bytes(self.pending));
                self.have = 0;
            }
        }
        let whole = bytes.len() - bytes.len() % 4;
        for chunk in bytes[..whole].chunks_exact(4) {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(chunk);
            out.push(f32::from_le_bytes(buf));
        }
        let rest = &bytes[whole..];
        self.pending[..rest.len()].copy_from_slice(rest);
        self.have = rest.len();
    }
}

pub(crate) fn ahead_is_full(buffered: usize, rate: u32, channels: u16) -> bool {
    buffered > ahead_limit(rate, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(tracks: Vec<(i64, f32)>, channels: u16) -> DecodeSpec {
        DecodeSpec {
            ffmpeg: PathBuf::from("/opt/ffmpeg"),
            media: PathBuf::from("/media/reel.mov"),
            start: 1.5,
            length: 10.0,
            rate: 48_000,
            channels,
            tracks,
        }
    }

    fn args(line: &CommandLine) -> Vec<String> {
        line.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_decoder_command_matches_the_reference_graph() {
        let line = command_line(&spec(vec![(1, 0.4), (2, 1.0)], 1));
        assert_eq!(line.program, PathBuf::from("/opt/ffmpeg"));
        assert_eq!(
            args(&line),
            [
                "-nostdin",
                "-v",
                "error",
                "-ss",
                "1.500",
                "-t",
                "10.000",
                "-i",
                "/media/reel.mov",
                "-filter_complex",
                "[0:1]volume=0.400,aformat=channel_layouts=stereo[t0];[0:2]volume=1.000,aformat=channel_layouts=stereo[t1];[t0][t1]amix=inputs=2:normalize=0[mix]",
                "-map",
                "[mix]",
                "-f",
                "f32le",
                "-ac",
                "2",
                "-ar",
                "48000",
                "-",
            ]
        );
    }

    #[test]
    fn a_single_file_decodes_at_the_chosen_rate() {
        let mut single = spec(Vec::new(), 1);
        single.media = PathBuf::from("/m/a.wav");
        single.start = -2.5;
        single.length = 1.25;
        single.rate = 44_100;
        let line = command_line(&single);
        assert_eq!(
            args(&line),
            [
                "-nostdin", "-v", "error", "-ss", "0.000", "-t", "1.250", "-i", "/m/a.wav", "-f",
                "f32le", "-ac", "1", "-ar", "44100", "-",
            ]
        );
        let live = spec_at_output(&single, 48_000, 2);
        assert_eq!(live.rate, 48_000);
        assert_eq!(live.channels, 2);
        let live_args = args(&command_line(&live));
        assert!(
            live_args
                .windows(2)
                .any(|pair| pair[0] == "-ar" && pair[1] == "48000")
        );
        assert!(
            live_args
                .windows(2)
                .any(|pair| pair[0] == "-ac" && pair[1] == "2")
        );
    }

    #[test]
    fn the_reader_waits_only_above_two_seconds() {
        assert_eq!(ahead_limit(48_000, 2), 192_000);
        assert_eq!(AHEAD_SECONDS, 2);
        assert!(!ahead_is_full(192_000, 48_000, 2));
        assert!(ahead_is_full(192_001, 48_000, 2));
    }

    #[test]
    fn split_reads_keep_f32le_frames_whole() {
        let mut framer = LeF32::new();
        let mut out = Vec::new();
        let first = 0.5f32.to_le_bytes();
        let second = (-1.0f32).to_le_bytes();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&first);
        bytes.extend_from_slice(&second);
        bytes.push(0x01);
        framer.push(&bytes[..2], &mut out);
        assert!(out.is_empty());
        framer.push(&bytes[2..6], &mut out);
        framer.push(&bytes[6..], &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].to_bits(), 0.5f32.to_bits());
        assert_eq!(out[1].to_bits(), (-1.0f32).to_bits());
        assert_eq!(framer.have, 1);
    }

    #[test]
    fn a_relative_ffmpeg_path_is_refused() {
        let mut spec = spec(Vec::new(), 1);
        spec.ffmpeg = PathBuf::from("ffmpeg");
        assert!(matches!(
            Decode::start(&spec),
            Err(err) if err.contains("absolute")
        ));
    }

    #[test]
    #[ignore = "needs ffmpeg; set PITO_FFMPEG to an absolute path"]
    fn ffmpeg_decodes_a_wav_to_f32() {
        let ffmpeg = std::env::var_os("PITO_FFMPEG").expect("PITO_FFMPEG");
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/sound-ffmpeg");
        std::fs::create_dir_all(&dir).unwrap();
        let media = dir.join("tone.wav");
        let clip = crate::Clip {
            rate: 8_000,
            channels: 1,
            samples: vec![0.0, 0.5, -0.5, 1.0],
        };
        std::fs::write(&media, clip.to_wav()).unwrap();
        let spec = DecodeSpec {
            ffmpeg: PathBuf::from(ffmpeg),
            media,
            start: 0.0,
            length: 1.0,
            rate: 8_000,
            channels: 1,
            tracks: Vec::new(),
        };
        let decode = Decode::start(&spec).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !decode.ended() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(decode.ended());
        let popped = decode.queue().pop_frames(4, 1);
        assert!(popped.ended);
        assert_eq!(popped.real, 4);
        for (got, want) in popped.samples.iter().zip(clip.samples.iter()) {
            assert!((got - want).abs() < 0.0001);
        }
    }
}
