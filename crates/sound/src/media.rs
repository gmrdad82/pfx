use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use pito_codecs::media::{self, AudioBuffer, AudioStream, Choice, Prefs};
use pito_codecs::{aac, mkv, mp3, mp4};

use crate::decode::ahead_is_full;
use crate::mix::PcmQueue;

#[derive(Clone, Debug)]
pub struct DecodeSpec {
    pub media: PathBuf,
    pub start: f32,
    pub length: f32,
    pub rate: u32,
    pub channels: u16,
    pub tracks: Vec<(usize, f32)>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Container,
    Wav,
    Mp3,
    Adts,
}

fn id3_end(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let size = bytes[6..10]
        .iter()
        .fold(0usize, |n, b| (n << 7) | usize::from(b & 0x7f));
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    (10 + size + footer).min(bytes.len())
}

fn kind(path: &Path, head: &[u8]) -> Result<Kind, String> {
    let refuse = |what: &str| Err(format!("{}: {what}", path.display()));
    let start = id3_end(head);
    let body = &head[start.min(head.len())..];
    if head.get(..4) == Some(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        return Ok(Kind::Wav);
    }
    if head.get(..4) == Some(b"OggS") {
        return refuse(
            "Ogg is not decoded here; pfx-sound's vorbis feature reads Ogg Vorbis (VorbisSource)",
        );
    }
    if head.get(..4) == Some(b"fLaC") {
        return refuse("FLAC is not decoded: convert it to WAV, AAC or Opus");
    }
    if head.get(..4) == Some(&[0x7f, 0xfe, 0x80, 0x01]) {
        return refuse("DTS is not decoded: convert it to AC-3, AAC or WAV");
    }
    match body {
        [0xff, b, ..] if b & 0xf6 == 0xf0 => Ok(Kind::Adts),
        [0xff, b, ..] if b & 0xe0 == 0xe0 && b & 0x06 != 0 => Ok(Kind::Mp3),
        _ if start > 0 => Ok(Kind::Mp3),
        _ => Ok(Kind::Container),
    }
}

fn cpu() -> Prefs {
    Prefs {
        decode: Choice::Cpu,
        encode: Choice::Cpu,
        ..Prefs::default()
    }
}

fn frames_of(seconds: f32, rate: u32) -> usize {
    (f64::from(seconds.max(0.0)) * f64::from(rate)).round() as usize
}

const READ: [&str; 8] = ["aac", "opus", "pcm", "ac3", "eac3", "mp3", "mp2", "mp1"];

fn audio_codecs(path: &Path, head: &[u8]) -> Result<Vec<String>, media::Error> {
    if head.get(..4) == Some(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let file = mkv::Reader::open(path)?;
        return Ok(file
            .audio()
            .into_iter()
            .map(|i| file.tracks[i].codec_name().to_string())
            .collect());
    }
    let file = mp4::Reader::open(path)?;
    Ok(file
        .audio()
        .into_iter()
        .map(|i| {
            file.tracks[i]
                .config()
                .map(|c| c.name())
                .unwrap_or_default()
        })
        .collect())
}

struct Part {
    stream: AudioStream,
    gain: f32,
    pending: VecDeque<f32>,
    ended: bool,
}

fn container(spec: &DecodeSpec, head: &[u8], channels: u16) -> Result<Vec<Part>, String> {
    let path = &spec.media;
    let show = |e: media::Error| format!("{}: {e}", path.display());
    let codecs = audio_codecs(path, head).map_err(show)?;
    if codecs.is_empty() {
        return Err(format!("{}: no audio track", path.display()));
    }
    let mut gains: Vec<(usize, f32)> = Vec::new();
    if spec.tracks.is_empty() {
        gains.push((0, 1.0));
    }
    for (track, gain) in &spec.tracks {
        match gains.iter_mut().find(|(t, _)| t == track) {
            Some((_, g)) => *g += gain,
            None => gains.push((*track, *gain)),
        }
    }
    let from = f64::from(spec.start.max(0.0));
    let prefs = cpu();
    gains
        .into_iter()
        .map(|(track, gain)| {
            let Some(codec) = codecs.get(track) else {
                return Err(format!(
                    "{}: no audio track {track}; it has {}",
                    path.display(),
                    codecs.len()
                ));
            };
            if !READ.contains(&codec.as_str()) {
                return Err(format!(
                    "{}: audio track {track} is {}, which pito-codecs does not decode (AAC, Opus, AC-3, E-AC-3, MP1, MP2, MP3 and PCM are read)",
                    path.display(),
                    if codec.is_empty() { "unknown" } else { codec }
                ));
            }
            let stream = media::audio_stream(path, track, from, spec.rate, channels, &prefs)
                .map_err(show)?;
            Ok(Part {
                stream,
                gain,
                pending: VecDeque::new(),
                ended: false,
            })
        })
        .collect()
}

fn mp3_pcm(bytes: &[u8]) -> Result<AudioBuffer, String> {
    let mut decoder = mp3::Decoder::new();
    let mut samples = Vec::new();
    decoder
        .decode_packet(&bytes[id3_end(bytes)..], &mut samples)
        .map_err(|e| e.to_string())?;
    if decoder.rate() == 0 {
        return Err("no MPEG audio frame".into());
    }
    let mut out = AudioBuffer::new(decoder.rate(), decoder.channels());
    out.samples = samples;
    Ok(out)
}

fn adts_pcm(bytes: &[u8]) -> Result<AudioBuffer, String> {
    let frames = aac::split_adts(bytes).map_err(|e| e.to_string())?;
    let (first, _) = frames.first().ok_or("no ADTS frame")?;
    let mut decoder = aac::Decoder::from_adts(first).map_err(|e| e.to_string())?;
    let mut out = AudioBuffer::new(decoder.rate(), decoder.channels());
    for (_, frame) in &frames {
        out.samples.extend(
            decoder
                .decode_frame(frame)
                .map_err(|e| e.to_string())?
                .samples,
        );
    }
    Ok(out)
}

fn whole(spec: &DecodeSpec, kind: Kind, channels: u16) -> Result<Vec<f32>, String> {
    let path = &spec.media;
    let show = |e: String| format!("{}: {e}", path.display());
    let bytes = std::fs::read(path).map_err(|e| show(e.to_string()))?;
    let pcm = match kind {
        Kind::Wav => media::wav::decode(&bytes).map_err(|e| show(e.to_string()))?,
        Kind::Mp3 => mp3_pcm(&bytes).map_err(show)?,
        _ => adts_pcm(&bytes).map_err(show)?,
    };
    let mut pcm = match channels {
        c if c == pcm.channels => pcm,
        1 => aac::mono(&pcm),
        2 => aac::downmix_stereo(&pcm),
        c => {
            return Err(show(format!(
                "{c} channels from a {}-channel file: ask for 1, 2 or {}",
                pcm.channels, pcm.channels
            )));
        }
    };
    if !spec.tracks.is_empty() {
        if let Some((track, _)) = spec.tracks.iter().find(|(track, _)| *track > 0) {
            return Err(show(format!("no audio track {track}; the file holds one")));
        }
        let gain: f32 = spec.tracks.iter().map(|(_, gain)| gain).sum();
        pcm.samples.iter_mut().for_each(|s| *s *= gain);
    }
    let pcm = if pcm.rate == spec.rate {
        pcm
    } else {
        aac::resample(&pcm, spec.rate)
    };
    let ch = usize::from(channels);
    let skip = (frames_of(spec.start, spec.rate) * ch).min(pcm.samples.len());
    let take = (frames_of(spec.length, spec.rate) * ch).min(pcm.samples.len() - skip);
    Ok(pcm.samples[skip..skip + take].to_vec())
}

enum Source {
    Streams { parts: Vec<Part>, left: usize },
    Whole { samples: Vec<f32>, at: usize },
}

impl Source {
    fn open(spec: &DecodeSpec, channels: u16) -> Result<Source, String> {
        let mut head = vec![0u8; 64 * 1024];
        let read = {
            use std::io::Read;
            let mut file = std::fs::File::open(&spec.media)
                .map_err(|e| format!("{}: {e}", spec.media.display()))?;
            let mut n = 0;
            while n < head.len() {
                match file.read(&mut head[n..]) {
                    Ok(0) => break,
                    Ok(k) => n += k,
                    Err(e) => return Err(format!("{}: {e}", spec.media.display())),
                }
            }
            n
        };
        head.truncate(read);
        let left = frames_of(spec.length, spec.rate) * usize::from(channels);
        match kind(&spec.media, &head)? {
            Kind::Container => Ok(Source::Streams {
                parts: container(spec, &head, channels)?,
                left,
            }),
            kind => Ok(Source::Whole {
                samples: whole(spec, kind, channels)?,
                at: 0,
            }),
        }
    }

    fn next(&mut self, block: usize) -> Option<Result<Vec<f32>, String>> {
        match self {
            Source::Streams { parts, left } => {
                if *left == 0 {
                    return None;
                }
                let want = block.min(*left);
                for part in parts.iter_mut() {
                    while !part.ended && part.pending.len() < want {
                        match part.stream.next() {
                            Some(Ok(buffer)) => part.pending.extend(buffer.samples),
                            Some(Err(e)) => return Some(Err(e.to_string())),
                            None => part.ended = true,
                        }
                    }
                }
                let n = parts
                    .iter()
                    .map(|part| part.pending.len().min(want))
                    .max()
                    .unwrap_or(0);
                if n == 0 {
                    return None;
                }
                let mut out = vec![0.0f32; n];
                for part in parts.iter_mut() {
                    let m = part.pending.len().min(n);
                    for (o, s) in out.iter_mut().zip(part.pending.drain(..m)) {
                        *o += s * part.gain;
                    }
                }
                *left -= n;
                Some(Ok(out))
            }
            Source::Whole { samples, at } => {
                if *at >= samples.len() {
                    return None;
                }
                let end = (*at + block).min(samples.len());
                let out = samples[*at..end].to_vec();
                *at = end;
                Some(Ok(out))
            }
        }
    }
}

pub struct Decode {
    stop: Arc<AtomicBool>,
    queue: Arc<PcmQueue>,
    channels: u16,
    rate: u32,
}

impl Decode {
    pub fn start(spec: &DecodeSpec) -> Result<Decode, String> {
        let channels = decode_channels(spec);
        if spec.rate == 0 || channels == 0 {
            return Err("decode rate and channels must be non-zero".into());
        }
        let source = Source::open(spec, channels)?;
        let queue = PcmQueue::new();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_queue = Arc::clone(&queue);
        let thread_stop = Arc::clone(&stop);
        let rate = spec.rate;
        std::thread::spawn(move || pump(source, thread_queue, thread_stop, rate, channels));
        Ok(Decode {
            stop,
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
    }
}

fn pump(mut source: Source, queue: Arc<PcmQueue>, stop: Arc<AtomicBool>, rate: u32, channels: u16) {
    let block = rate as usize * channels as usize / 10;
    while !stop.load(Ordering::Relaxed) {
        if ahead_is_full(queue.len(), rate, channels) {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        match source.next(block.max(1)) {
            Some(Ok(samples)) => queue.push(&samples),
            Some(Err(_)) | None => break,
        }
    }
    queue.end();
}

#[cfg(test)]
mod tests {
    use super::*;
    use pito_codecs::mp4::{SampleEntry, TrackConfig, Writer};

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("sound-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn spec(media: PathBuf, start: f32, length: f32, rate: u32, channels: u16) -> DecodeSpec {
        DecodeSpec {
            media,
            start,
            length,
            rate,
            channels,
            tracks: Vec::new(),
        }
    }

    fn drain(spec: &DecodeSpec) -> Vec<f32> {
        let decode = Decode::start(spec).unwrap();
        let channels = usize::from(decode.channels());
        let queue = decode.queue();
        let mut out = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let pop = queue.pop_frames(queue.len() / channels, channels);
            out.extend(pop.samples);
            if pop.ended && pop.real == 0 {
                return out;
            }
            assert!(std::time::Instant::now() < deadline, "the decode stalled");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
    }

    fn tone_mp4(path: &Path) {
        let mut aac = aac::Encoder::new(aac::EncoderConfig {
            rate: 48_000,
            channels: 2,
            bitrate: 192_000,
        })
        .unwrap();
        let mut pcm = AudioBuffer::new(48_000, 2);
        pcm.samples = (0..48_000)
            .flat_map(|k| {
                let v = (std::f32::consts::TAU * 440.0 * k as f32 / 48_000.0).sin() * 0.5;
                [v, v]
            })
            .collect();
        let mut packets = aac.encode(&pcm).unwrap();
        packets.extend(aac.flush());
        let mut writer = Writer::create(path).unwrap();
        let mut config = TrackConfig::new(
            SampleEntry::aac(aac.extradata(), 2, 48_000, 192_000),
            48_000,
        );
        config.media_start = Some(i64::from(aac.priming()));
        let track = writer.add_track(config).unwrap();
        for packet in &packets {
            writer.push(track, packet).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn a_wav_is_cut_resampled_and_shaped_to_the_spec() {
        let dir = scratch("wav");
        let media = dir.join("level.wav");
        let clip = crate::Clip {
            rate: 8_000,
            channels: 1,
            samples: vec![0.5; 8_000],
        };
        std::fs::write(&media, clip.to_wav()).unwrap();
        let out = drain(&spec(media, 0.25, 0.5, 16_000, 2));
        assert_eq!(out.len(), 16_000);
        assert!(out[4_000..12_000].iter().all(|s| (s - 0.5).abs() < 0.01));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_mp4_tone_decodes_to_its_length_and_its_track_gain() {
        let dir = scratch("mp4");
        let media = dir.join("tone.m4a");
        tone_mp4(&media);
        let plain = drain(&spec(media.clone(), 0.0, 0.5, 48_000, 2));
        assert_eq!(plain.len(), 48_000);
        let level = rms(&plain[4_800..]);
        assert!((level - 0.5 / 2f32.sqrt()).abs() < 0.02, "rms {level}");
        let mut mixed = spec(media.clone(), 0.0, 0.5, 48_000, 1);
        mixed.tracks = vec![(0, 0.5)];
        let half = drain(&mixed);
        assert_eq!(half.len(), 48_000);
        assert!((rms(&half[4_800..]) - level / 2.0).abs() < 0.01);
        let mut missing = spec(media, 0.0, 0.5, 48_000, 2);
        missing.tracks = vec![(1, 1.0)];
        let err = Decode::start(&missing).err().unwrap();
        assert!(err.contains("no audio track 1"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ogg_flac_and_dts_are_refused_by_name() {
        let dir = scratch("refused");
        let ogg = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sine.ogg");
        let err = Decode::start(&spec(ogg, 0.0, 1.0, 48_000, 2))
            .err()
            .unwrap();
        assert!(err.contains("vorbis feature"), "{err}");
        for (name, head, word) in [
            ("a.flac", b"fLaC\0\0\0\0".to_vec(), "FLAC"),
            ("a.dts", vec![0x7f, 0xfe, 0x80, 0x01, 0, 0, 0, 0], "DTS"),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, head).unwrap();
            let err = Decode::start(&spec(path, 0.0, 1.0, 48_000, 2))
                .err()
                .unwrap();
            assert!(err.contains(word), "{err}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
