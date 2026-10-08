use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lewton::inside_ogg::OggStreamReader;

use crate::channel::ChannelId;
use crate::clip::Clip;
use crate::decode::ahead_limit;
use crate::interp::StreamResampler;
use crate::mix::{Level, Mixer, PcmQueue, VoiceId};

trait Source: Read + Seek + Send {}

impl<T: Read + Seek + Send> Source for T {}

type Reader = OggStreamReader<Box<dyn Source>>;

#[derive(Clone, Debug)]
pub enum VorbisSource {
    Bytes(Arc<[u8]>),
    File(PathBuf),
}

impl VorbisSource {
    fn open(&self) -> Result<Reader, String> {
        let source: Box<dyn Source> = match self {
            VorbisSource::Bytes(bytes) => Box::new(Cursor::new(Arc::clone(bytes))),
            VorbisSource::File(path) => {
                let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
                Box::new(BufReader::new(file))
            }
        };
        OggStreamReader::new(source).map_err(|e| format!("vorbis: {e}"))
    }
}

fn unit(sample: i16) -> f32 {
    f32::from(sample) / 32768.0
}

impl Clip {
    pub fn from_vorbis(bytes: &[u8]) -> Result<Clip, String> {
        let mut reader =
            OggStreamReader::new(Cursor::new(bytes)).map_err(|e| format!("vorbis: {e}"))?;
        let rate = reader.ident_hdr.audio_sample_rate;
        let channels = u16::from(reader.ident_hdr.audio_channels);
        if rate == 0 || channels == 0 {
            return Err("vorbis: no audio".into());
        }
        let mut samples = Vec::new();
        while let Some(packet) = reader
            .read_dec_packet_itl()
            .map_err(|e| format!("vorbis: {e}"))?
        {
            samples.extend(packet.into_iter().map(unit));
        }
        if let Some(end) = reader.get_last_absgp() {
            let keep = usize::try_from(end).unwrap_or(usize::MAX);
            samples.truncate(keep.saturating_mul(usize::from(channels)));
        }
        Ok(Clip {
            rate,
            channels,
            samples,
        })
    }
}

struct Pump {
    source: VorbisSource,
    reader: Reader,
    looping: Option<(u64, u64)>,
    channels: usize,
    ahead: usize,
    queue: Arc<PcmQueue>,
    resampler: StreamResampler,
    held: Option<Vec<i16>>,
    done: u64,
    discard_to: u64,
    emitted_this_pass: bool,
}

impl Pump {
    fn run(mut self) {
        loop {
            if Arc::strong_count(&self.queue) <= 1 {
                return;
            }
            if self.queue.len() >= self.ahead {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            match self.reader.read_dec_packet_itl() {
                Ok(Some(packet)) => {
                    if let Some(previous) = self.held.replace(packet) {
                        self.take(&previous);
                    }
                    if self.looping.is_some_and(|(_, end)| self.done >= end) && !self.restart() {
                        return;
                    }
                }
                Ok(None) | Err(_) => {
                    if let Some(last) = self.held.take() {
                        let last = self.trimmed(last);
                        self.take(&last);
                    }
                    if self.looping.is_some() && self.restart() {
                        continue;
                    }
                    self.conclude();
                    return;
                }
            }
        }
    }

    fn trimmed(&self, mut last: Vec<i16>) -> Vec<i16> {
        let Some(end) = self.reader.get_last_absgp() else {
            return last;
        };
        let frames = (last.len() / self.channels) as u64;
        let excess = (self.done + frames).saturating_sub(end).min(frames);
        last.truncate((frames - excess) as usize * self.channels);
        last
    }

    fn take(&mut self, packet: &[i16]) {
        let frames = (packet.len() / self.channels) as u64;
        let from = self.discard_to.saturating_sub(self.done).min(frames);
        let to = match self.looping {
            Some((_, end)) => end.saturating_sub(self.done).min(frames),
            None => frames,
        };
        if to > from {
            let slice = &packet[from as usize * self.channels..to as usize * self.channels];
            let floats: Vec<f32> = slice.iter().copied().map(unit).collect();
            let mut out = Vec::new();
            self.resampler.push(&floats, &mut out);
            self.queue.push(&out);
            self.emitted_this_pass = true;
        }
        self.done += frames;
    }

    fn conclude(&mut self) {
        let mut out = Vec::new();
        self.resampler.finish(&mut out);
        self.queue.push(&out);
        self.queue.end();
    }

    fn restart(&mut self) -> bool {
        if !self.emitted_this_pass {
            self.conclude();
            return false;
        }
        let Ok(reader) = self.source.open() else {
            self.conclude();
            return false;
        };
        self.reader = reader;
        self.held = None;
        self.done = 0;
        self.discard_to = self.looping.map_or(0, |(start, _)| start);
        self.emitted_this_pass = false;
        true
    }
}

pub(crate) fn open_stream(
    source: &VorbisSource,
    rate: u32,
    looping: Option<(u64, u64)>,
) -> Result<(Arc<PcmQueue>, u16), String> {
    let reader = source.open()?;
    let src_rate = reader.ident_hdr.audio_sample_rate;
    let channels = u16::from(reader.ident_hdr.audio_channels);
    if rate == 0 || src_rate == 0 || channels == 0 {
        return Err("vorbis: no audio".into());
    }
    let looping = looping.filter(|(start, end)| start < end);
    let queue = PcmQueue::new();
    let pump = Pump {
        source: source.clone(),
        reader,
        looping,
        channels: usize::from(channels),
        ahead: ahead_limit(rate, channels),
        queue: Arc::clone(&queue),
        resampler: StreamResampler::new(src_rate, rate, usize::from(channels)),
        held: None,
        done: 0,
        discard_to: 0,
        emitted_this_pass: false,
    };
    std::thread::spawn(move || pump.run());
    Ok((queue, channels))
}

impl Mixer {
    pub fn stream_vorbis(
        &mut self,
        channel: ChannelId,
        source: &VorbisSource,
        looping: Option<(u64, u64)>,
        level: Level,
    ) -> Result<VoiceId, String> {
        let (queue, channels) = open_stream(source, self.rate(), looping)?;
        Ok(self.stream_on(channel, queue, channels, level))
    }

    pub fn play_music_vorbis(
        &mut self,
        source: &VorbisSource,
        looping: Option<(u64, u64)>,
        fade_seconds: f32,
        level: Level,
    ) -> Result<VoiceId, String> {
        let (queue, channels) = open_stream(source, self.rate(), looping)?;
        Ok(self.stream_music(queue, channels, fade_seconds, level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const OGG: &[u8] = include_bytes!("../tests/fixtures/sine.ogg");

    fn bytes() -> VorbisSource {
        VorbisSource::Bytes(Arc::from(OGG))
    }

    fn fill(queue: &PcmQueue, samples: usize) {
        let until = Instant::now() + Duration::from_secs(20);
        while queue.len() < samples && !queue.reader_ended() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(queue.len() >= samples || queue.reader_ended());
    }

    fn finish(queue: &PcmQueue) {
        let until = Instant::now() + Duration::from_secs(20);
        while !queue.reader_ended() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(queue.reader_ended());
    }

    fn drain(queue: &PcmQueue, frames: usize, channels: usize) -> Vec<f32> {
        queue.pop_frames(frames, channels).samples
    }

    #[test]
    fn a_stream_yields_the_same_samples_as_the_clip() {
        let clip = Clip::from_vorbis(OGG).unwrap();
        let (queue, channels) = open_stream(&bytes(), 44_100, None).unwrap();
        assert_eq!(channels, 2);
        finish(&queue);
        assert_eq!(queue.len(), clip.samples.len());
        let streamed = drain(&queue, clip.frames() as usize, 2);
        assert_eq!(streamed, clip.samples);
    }

    #[test]
    fn a_file_source_streams_too() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sine.ogg");
        let (queue, _) = open_stream(&VorbisSource::File(path), 44_100, None).unwrap();
        finish(&queue);
        assert_eq!(queue.len(), 88_200);
    }

    #[test]
    fn a_looping_stream_repeats_between_its_loop_points() {
        let clip = Clip::from_vorbis(OGG).unwrap();
        let (queue, _) = open_stream(&bytes(), 44_100, Some((10_000, 30_000))).unwrap();
        fill(&queue, 2 * 70_000);
        assert!(!queue.reader_ended());
        let out = drain(&queue, 70_000, 2);
        assert_eq!(out[..60_000], clip.samples[..60_000]);
        assert_eq!(out[60_000..100_000], clip.samples[20_000..60_000]);
        assert_eq!(out[100_000..140_000], clip.samples[20_000..60_000]);
    }

    #[test]
    fn a_stream_resamples_to_the_output_rate() {
        let clip = Clip::from_vorbis(OGG).unwrap();
        let (queue, _) = open_stream(&bytes(), 48_000, None).unwrap();
        finish(&queue);
        let frames = queue.len() / 2;
        assert!((47_998..=48_002).contains(&frames), "{frames}");
        let streamed = drain(&queue, frames, 2);
        let power =
            |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
        let drift = 10.0 * (power(&streamed) / power(&clip.samples)).log10();
        assert!(drift.abs() < 0.2, "{drift} dB");
    }

    #[test]
    fn a_dropped_voice_stops_the_decoder_thread() {
        let (queue, _) = open_stream(&bytes(), 44_100, Some((0, 10_000))).unwrap();
        fill(&queue, 1000);
        let weak = Arc::downgrade(&queue);
        drop(queue);
        let until = Instant::now() + Duration::from_secs(20);
        while weak.strong_count() > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(weak.strong_count(), 0);
    }
}
