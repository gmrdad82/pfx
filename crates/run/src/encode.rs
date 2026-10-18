use pito_codecs::aac;
use pito_codecs::filter::{ColorSpec, Order, Packed, packed_to_yuv};
use pito_codecs::h264::{self, Preset, Profile};
use pito_codecs::media::{
    self, AudioBuffer, Bitstream, Choice, Frame, PixelFormat, Prefs, Rate, TimeBase,
};
use pito_codecs::mp4::{AvcConfig, SampleEntry, TrackConfig, Writer};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;

pub const PITO_CODECS: &str = "1.14.3";
pub const COLOR: &str = "BT.709 primaries, transfer and matrix, limited range (SDR)";
pub const CRF: u8 = 12;
pub const B_FRAMES: u8 = 2;
pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_BITRATE: &str = "320k";
pub const AUDIO_CODEC: &str = "AAC-LC";
const AUDIO_BITS: u32 = 320_000;
const AUDIO_CHANNELS: u16 = 2;
const AHEAD: usize = 2;

pub fn settings(fps: u32) -> serde_json::Value {
    serde_json::json!({
        "encoder": "pito-codecs H.264, CPU",
        "profile": "High",
        "preset": "Quality",
        "crf": CRF,
        "gop": fps,
        "b_frames": B_FRAMES,
        "matrix": "BT.709 limited, left-sited 4:2:0 chroma, from 8-bit RGB",
        "container": "MP4, faststart",
    })
}

pub fn record() -> serde_json::Value {
    serde_json::json!({"name": "pito-codecs", "version": PITO_CODECS, "route": "cpu"})
}

fn fail(what: &Path) -> impl Fn(media::Error) -> String + '_ {
    move |e| format!("{}: {e}", what.display())
}

fn audio_samples(frames: u64, fps: u32) -> usize {
    let num = u128::from(frames) * u128::from(AUDIO_RATE);
    let den = u128::from(fps.max(1));
    usize::try_from((2 * num + den) / (2 * den)).unwrap_or(usize::MAX)
}

struct Stream {
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    encoder: h264::Encoder,
    writer: Writer,
    track: usize,
    yuv: Frame,
    frames: u64,
}

impl Stream {
    fn encode(&mut self, rgba: &[u8]) -> Result<(), String> {
        let show = fail(&self.path);
        let packed = Packed::new(self.width, self.height, Order::Rgba, rgba);
        packed_to_yuv(
            packed,
            ColorSpec::BT709,
            PixelFormat::Yuv420p8,
            &mut self.yuv,
        )
        .map_err(&show)?;
        self.yuv.pts = i64::try_from(self.frames).unwrap_or(i64::MAX);
        self.yuv.time_base = TimeBase::new(1, self.fps);
        for packet in self.encoder.encode(&self.yuv).map_err(&show)? {
            self.writer.push(self.track, &packet).map_err(&show)?;
        }
        self.frames += 1;
        Ok(())
    }

    fn close(mut self) -> Result<Writer, String> {
        let show = fail(&self.path);
        for packet in self.encoder.flush().map_err(&show)? {
            self.writer.push(self.track, &packet).map_err(&show)?;
        }
        Ok(self.writer)
    }

    fn run(mut self, frames: Receiver<Vec<u8>>, spent: Sender<Vec<u8>>) -> Result<Writer, String> {
        for rgba in frames {
            self.encode(&rgba)?;
            let _ = spent.send(rgba);
        }
        self.close()
    }
}

pub struct Master {
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    frames: u64,
    send: Option<SyncSender<Vec<u8>>>,
    spent: Receiver<Vec<u8>>,
    worker: Option<JoinHandle<Result<Writer, String>>>,
}

impl Master {
    pub fn create(width: u32, height: u32, fps: u32, out: &Path) -> Result<Master, String> {
        Master::with_threads(width, height, fps, out, 0)
    }

    fn with_threads(
        width: u32,
        height: u32,
        fps: u32,
        out: &Path,
        threads: usize,
    ) -> Result<Master, String> {
        if fps == 0 {
            return Err("a master needs a frame rate above 0".into());
        }
        let show = fail(out);
        let encoder = h264::Encoder::new(h264::EncoderConfig {
            width,
            height,
            fps: (fps, 1),
            profile: Profile::High,
            preset: Preset::Quality,
            rate: Rate::Crf(CRF),
            gop: fps,
            b_frames: B_FRAMES,
            threads,
            bitstream: Bitstream::LengthPrefixed,
        })
        .map_err(&show)?;
        let config = AvcConfig::parse(&encoder.avcc()).map_err(&show)?;
        let mut writer = Writer::create(out).map_err(&show)?;
        let entry = SampleEntry::h264(config, width as u16, height as u16);
        let track = writer
            .add_track(TrackConfig::new(entry, fps))
            .map_err(&show)?;
        let stream = Stream {
            path: out.to_path_buf(),
            width,
            height,
            fps,
            encoder,
            writer,
            track,
            yuv: Frame::new(
                PixelFormat::Yuv420p8,
                width,
                height,
                0,
                TimeBase::new(1, fps),
            ),
            frames: 0,
        };
        let (send, frames) = mpsc::sync_channel(AHEAD);
        let (spend, spent) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("pfx-master".into())
            .spawn(move || stream.run(frames, spend))
            .map_err(|e| format!("{}: {e}", out.display()))?;
        Ok(Master {
            path: out.to_path_buf(),
            width,
            height,
            fps,
            frames: 0,
            send: Some(send),
            spent,
            worker: Some(worker),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    fn buffer(&self) -> Vec<u8> {
        self.spent
            .try_recv()
            .unwrap_or_else(|_| vec![0; self.bytes()])
    }

    fn refused(&self, what: &str, len: usize) -> String {
        format!(
            "{}: a frame of {len} {what} for a {}x{} master",
            self.path.display(),
            self.width,
            self.height
        )
    }

    pub fn push(&mut self, rgba: &[u16]) -> Result<(), String> {
        if rgba.len() != self.bytes() {
            return Err(self.refused("samples", rgba.len()));
        }
        let mut frame = self.buffer();
        for (out, v) in frame.iter_mut().zip(rgba) {
            *out = ((u32::from(*v) + 128) / 257) as u8;
        }
        self.send(frame)
    }

    pub fn push8(&mut self, rgba: &[u8]) -> Result<(), String> {
        if rgba.len() != self.bytes() {
            return Err(self.refused("bytes", rgba.len()));
        }
        let mut frame = self.buffer();
        frame.copy_from_slice(rgba);
        self.send(frame)
    }

    fn send(&mut self, frame: Vec<u8>) -> Result<(), String> {
        let sent = self
            .send
            .as_ref()
            .is_some_and(|send| send.send(frame).is_ok());
        if sent {
            self.frames += 1;
            return Ok(());
        }
        self.send = None;
        match self.join() {
            Err(e) => Err(e),
            Ok(_) => Err(format!("{}: the encoder stopped", self.path.display())),
        }
    }

    fn join(&mut self) -> Result<Writer, String> {
        self.send = None;
        self.worker
            .take()
            .ok_or_else(|| format!("{}: the encoder stopped", self.path.display()))?
            .join()
            .map_err(|_| format!("{}: the encoder failed", self.path.display()))?
    }

    pub fn finish(mut self, wav: Option<&Path>) -> Result<(), String> {
        let path = self.path.clone();
        let show = fail(&path);
        if self.frames == 0 {
            return Err(format!("{}: no frames to encode", path.display()));
        }
        let mut writer = self.join()?;
        if let Some(wav) = wav {
            let bytes = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
            let pcm = media::wav::decode(&bytes).map_err(fail(wav))?;
            let total = audio_samples(self.frames, self.fps);
            let pcm = fit(pcm, total);
            let mut aac = aac::Encoder::new(aac::EncoderConfig {
                rate: AUDIO_RATE,
                channels: AUDIO_CHANNELS,
                bitrate: AUDIO_BITS,
            })
            .map_err(&show)?;
            let mut packets = aac.encode(&pcm).map_err(&show)?;
            packets.extend(aac.flush());
            let priming = i64::from(aac.priming());
            let mut config = TrackConfig::new(
                SampleEntry::aac(
                    aac.extradata(),
                    u32::from(AUDIO_CHANNELS),
                    AUDIO_RATE,
                    AUDIO_BITS,
                ),
                AUDIO_RATE,
            );
            config.media_start = Some(priming);
            let track = writer.add_track(config).map_err(&show)?;
            if !packets.is_empty() {
                writer.set_movie_timescale(AUDIO_RATE);
            }
            let end = priming.saturating_add(i64::try_from(total).unwrap_or(i64::MAX));
            let count = packets.len();
            for (k, packet) in packets.iter().enumerate() {
                if k + 1 == count {
                    let duration =
                        u32::try_from(end.saturating_sub(packet.pts).max(1)).unwrap_or(u32::MAX);
                    writer.push_timed(track, packet, duration).map_err(&show)?;
                } else {
                    writer.push(track, packet).map_err(&show)?;
                }
            }
        }
        writer.finish_faststart().map_err(&show)?;
        Ok(())
    }
}

impl Drop for Master {
    fn drop(&mut self) {
        self.send = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn fit(pcm: AudioBuffer, total: usize) -> AudioBuffer {
    let stereo = if pcm.channels == AUDIO_CHANNELS {
        pcm
    } else {
        aac::downmix_stereo(&pcm)
    };
    let mut out = if stereo.rate == AUDIO_RATE {
        stereo
    } else {
        aac::resample(&stereo, AUDIO_RATE)
    };
    out.samples.resize(total * usize::from(AUDIO_CHANNELS), 0.0);
    out
}

fn cpu() -> Prefs {
    Prefs {
        decode: Choice::Cpu,
        encode: Choice::Cpu,
        ..Prefs::default()
    }
}

pub fn readable(input: &Path) -> Result<(u32, u32), String> {
    let probe = media::probe(input, &cpu()).map_err(fail(input))?;
    if probe.video_native() {
        return Ok((probe.width, probe.height));
    }
    let name = probe.video_codec_name;
    if name.to_ascii_lowercase().contains("ffv1") {
        return Err(format!(
            "{} is an FFV1 master, and pito-codecs reads no FFV1: render the clip again for an H.264 master",
            input.display()
        ));
    }
    Err(format!(
        "{} holds {} video, which pito-codecs does not decode",
        input.display(),
        if name.is_empty() { "unknown" } else { &name }
    ))
}

pub fn from_file(input: &Path, fps: u32, out: &Path, poster: Option<&Path>) -> Result<(), String> {
    let (width, height) = readable(input)?;
    let show = fail(input);
    let video = media::decode_video(input, 0.0, None, &cpu()).map_err(&show)?;
    let mut master = Master::create(width, height, fps, out)?;
    for frame in video {
        let rgba = media::to_rgba(&frame.map_err(&show)?);
        if master.frames() == 0
            && let Some(poster) = poster
        {
            save8(poster, rgba.width, rgba.height, &rgba.data)?;
        }
        master.push8(&rgba.data)?;
    }
    master.finish(None)
}

fn save8(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let show = |e: &dyn Display| format!("{}: {e}", path.display());
    let file = std::fs::File::create(path).map_err(|e| show(&e))?;
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .map_err(|e| show(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pito_codecs::media::Packet;
    use pito_codecs::{mkv, mp4};

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn checker(size: usize, frame: usize) -> Vec<u16> {
        let mut out = Vec::with_capacity(size * size * 4);
        for y in 0..size {
            for x in 0..size {
                let light = (x / 8 + y / 8 + frame).is_multiple_of(2);
                let value = if light { u16::MAX } else { 0 };
                out.extend([value, value, value, u16::MAX]);
            }
        }
        out
    }

    fn decode_all(path: &Path) -> (Vec<Frame>, Option<h264::ColorInfo>) {
        let file = mp4::Reader::open(path).unwrap();
        let video = file.video().unwrap();
        let track = &file.tracks[video];
        let mut decoder = h264::Decoder::with_extradata(&track.extradata().unwrap()).unwrap();
        let mut frames = Vec::new();
        for index in 0..track.samples.len() {
            frames.extend(decoder.decode(&file.packet(video, index).unwrap()).unwrap());
        }
        frames.extend(decoder.flush().unwrap());
        (frames, decoder.color())
    }

    #[test]
    fn the_pinned_tag_is_the_recorded_version() {
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains(&format!("tag = \"v{PITO_CODECS}\"")));
    }

    #[test]
    fn a_master_is_high_h264_tagged_bt709_limited_with_a_key_at_least_every_second() {
        let dir = scratch("master-checker");
        let output = dir.join("checker.master.mp4");
        let mut master = Master::create(64, 64, 4, &output).unwrap();
        for frame in 0..9 {
            master.push(&checker(64, frame)).unwrap();
        }
        master.finish(None).unwrap();
        let file = mp4::Reader::open(&output).unwrap();
        let track = &file.tracks[file.video().unwrap()];
        assert_eq!(track.codec(), Some(media::Codec::H264));
        assert_eq!(track.extradata().unwrap()[1], 100);
        assert_eq!(track.samples.len(), 9);
        let keys: Vec<usize> = (0..track.samples.len())
            .filter(|&i| track.samples[i].keyframe)
            .collect();
        assert_eq!(keys[0], 0);
        let mut last = 0;
        for i in 1..9 {
            if keys.contains(&i) {
                last = i;
            }
            assert!(i - last < 4, "no keyframe in the second before frame {i}");
        }
        let (frames, color) = decode_all(&output);
        assert_eq!(frames.len(), 9);
        let color = color.unwrap();
        assert!(!color.full_range);
        assert_eq!([color.primaries, color.transfer, color.matrix], [1, 1, 1]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_swatch_keeps_its_bt709_limited_values() {
        let dir = scratch("master-swatch");
        let output = dir.join("swatch.master.mp4");
        let mut master = Master::create(16, 16, 4, &output).unwrap();
        let swatch: Vec<u16> = (0..16 * 16)
            .flat_map(|_| [192u16 * 257, 64 * 257, 32 * 257, u16::MAX])
            .collect();
        for _ in 0..2 {
            master.push(&swatch).unwrap();
        }
        master.finish(None).unwrap();
        let (frames, _) = decode_all(&output);
        let [y, u, v] = &frames[0].planes;
        let got = [
            y.data[8 * y.stride + 8],
            u.data[4 * u.stride + 4],
            v.data[4 * v.stride + 4],
        ];
        for (got, want) in got.into_iter().zip([92u8, 101, 186]) {
            assert!(got.abs_diff(want) <= 1, "{got} for {want}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn frames(width: usize, height: usize, count: usize) -> Vec<Vec<u16>> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..count)
            .map(|frame| {
                let mut out = Vec::with_capacity(width * height * 4);
                for y in 0..height {
                    for x in 0..width {
                        state = state
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        let noise = (state >> 48) as u16 & 0x03ff;
                        let r = ((x * 65535 / width) as u16).wrapping_add(noise);
                        let g = ((y * 65535 / height) as u16).wrapping_add((frame * 2111) as u16);
                        let b = (((x + y + frame * 5) * 997) as u16) ^ noise;
                        out.extend([r, g, b, u16::MAX]);
                    }
                }
                out
            })
            .collect()
    }

    fn encode_with(threads: usize, frames: &[Vec<u16>], out: &Path) -> Vec<u8> {
        let mut master = Master::with_threads(320, 180, 6, out, threads).unwrap();
        for frame in frames {
            master.push(frame).unwrap();
        }
        master.finish(None).unwrap();
        std::fs::read(out).unwrap()
    }

    #[test]
    fn the_same_frames_give_the_same_bytes_on_any_thread_count() {
        let dir = scratch("master-threads");
        let input = frames(320, 180, 12);
        let out = dir.join("a.master.mp4");
        let first = encode_with(0, &input, &out);
        assert!(!first.is_empty());
        assert!(first == encode_with(0, &input, &out), "two runs differ");
        assert!(first == encode_with(1, &input, &out), "one thread differs");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_ffv1_master_is_refused_by_name() {
        let dir = scratch("master-ffv1");
        let input = dir.join("old.master.mkv");
        let mut writer = mkv::Writer::create(&input).unwrap();
        let mut config =
            mkv::TrackConfig::new(mkv::TrackKind::Video, "V_FFV1", TimeBase::new(1, 4));
        config.width = 16;
        config.height = 16;
        let track = writer.add_track(config).unwrap();
        let packet = Packet {
            data: vec![0; 64],
            pts: 0,
            dts: 0,
            keyframe: true,
        };
        writer.push(track, &packet).unwrap();
        writer.finish().unwrap();
        let err = from_file(&input, 4, &dir.join("new.master.mp4"), None).unwrap_err();
        assert!(err.contains("FFV1"), "{err}");
        assert!(!dir.join("new.master.mp4").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
