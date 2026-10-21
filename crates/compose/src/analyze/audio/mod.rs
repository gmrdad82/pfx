pub mod chroma;
pub mod level;
pub mod onset;
pub mod tempo;

use std::collections::BTreeMap;
use std::f64::consts::LN_2;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use pfx_core::sim::{exp, ln};
use pfx_sound::Clip;
use pfx_sound::voicefx::spectrum;

use super::{Chroma, Origin, Part, Piece, Span};
use crate::score::Note;
use crate::style::MAX_SECONDS;

pub const RATE: u32 = 22_050;

const SIZE: usize = 2048;
const BINS: usize = SIZE / 2;
const HOP: usize = 512;
const SPAN_STEP: f32 = 0.25;
const HIT_STEP: f32 = 0.4;
const FLATNESS: f64 = 0.15;
const NOTE_FRAMES: usize = 3;
const MIN_BEATS: f64 = 0.0625;
const BASE_MIDI: [f64; 3] = [36.0, 60.0, 84.0];
const RATES: [u32; 2] = [4_000, 384_000];
const MAX_FILE_BYTES: u64 = 1 << 31;
const MAX_TOTAL_SECONDS: f64 = 4.0 * MAX_SECONDS;
const MAX_CHANNELS: u16 = 32;
const MAX_SAMPLES: u64 = 1 << 30;
const MAX_SAMPLE: f32 = 16.0;
const MAX_CHUNKS: usize = 1024;

#[derive(Clone, Debug)]
pub struct Stem {
    pub name: String,
    pub clip: Clip,
}

pub(crate) struct Power {
    frames: usize,
    values: Vec<f32>,
}

impl Power {
    pub(crate) fn of(mono: &[f32]) -> Power {
        if mono.is_empty() {
            return Power {
                frames: 0,
                values: Vec::new(),
            };
        }
        let pad = SIZE / 2;
        let mut padded = vec![0.0f32; mono.len() + 2 * pad];
        padded[pad..pad + mono.len()].copy_from_slice(mono);
        let mut values = spectrum::powers(&padded, SIZE, HOP);
        let frames = (values.len() / BINS).min(1 + mono.len() / HOP);
        values.truncate(frames * BINS);
        Power { frames, values }
    }

    pub(crate) fn frame(&self, t: usize) -> Option<&[f32]> {
        (t < self.frames).then(|| &self.values[t * BINS..(t + 1) * BINS])
    }

    pub(crate) fn rows(&self) -> std::slice::ChunksExact<'_, f32> {
        self.values.chunks_exact(BINS)
    }
}

pub(crate) struct Banks {
    mel: Vec<(usize, Vec<f64>)>,
    chroma: Vec<[f64; 12]>,
}

impl Banks {
    pub(crate) fn new(rate: u32) -> Banks {
        Banks {
            mel: onset::mel_bank(rate),
            chroma: chroma::bank(rate),
        }
    }
}

fn wav_files(path: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let meta = std::fs::metadata(path)
        .map_err(|_| format!("{}: no such file or folder", path.display()))?;
    if meta.is_dir() {
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("{}: can't list the folder: {e}", path.display()))?;
        let mut files = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|e| format!("{}: can't list the folder: {e}", path.display()))?;
            let file = entry.path();
            let hidden = file
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if !hidden && file.is_file() && is_wav(&file) {
                files.push(file);
            }
        }
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        if files.is_empty() {
            return Err(format!("{}: the folder holds no .wav file", path.display()));
        }
        Ok(files)
    } else if meta.is_file() && is_wav(path) {
        Ok(vec![path.to_path_buf()])
    } else {
        Err(format!(
            "{}: not a .wav file or a folder of .wav files",
            path.display()
        ))
    }
}

struct Header {
    rate: u32,
    channels: u16,
    samples: u64,
}

fn header(file: &Path, size: u64) -> std::io::Result<Option<Header>> {
    let mut reader = std::fs::File::open(file)?;
    let mut riff = [0u8; 12];
    if size < 12 || reader.read_exact(&mut riff).is_err() {
        return Ok(None);
    }
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Ok(None);
    }
    let mut at = 12u64;
    let mut rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data = None;
    let mut chunks = 0usize;
    while at + 8 <= size {
        chunks += 1;
        if chunks > MAX_CHUNKS {
            return Ok(None);
        }
        let mut head = [0u8; 8];
        reader.seek(SeekFrom::Start(at))?;
        reader.read_exact(&mut head)?;
        let len = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
        let start = at + 8;
        let body = len.min(size - start);
        match &head[0..4] {
            b"fmt " if body >= 16 => {
                let mut fmt = [0u8; 16];
                reader.read_exact(&mut fmt)?;
                channels = u16::from_le_bytes([fmt[2], fmt[3]]);
                rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
                bits = u16::from_le_bytes([fmt[14], fmt[15]]);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at = start + len + (len & 1);
    }
    let width = match bits {
        8 => 1,
        16 => 2,
        32 => 4,
        _ => return Ok(None),
    };
    Ok(data
        .filter(|_| rate > 0 && channels > 0)
        .map(|data| Header {
            rate,
            channels,
            samples: data / width,
        }))
}

pub fn read(path: &Path) -> Result<Vec<Stem>, String> {
    let files = wav_files(path)?;
    let mut total = 0.0;
    let mut total_samples = 0u64;
    let mut stems = Vec::with_capacity(files.len());
    for file in &files {
        let shown = file.display();
        let size = std::fs::metadata(file)
            .map_err(|e| format!("{shown}: can't read it: {e}"))?
            .len();
        if size > MAX_FILE_BYTES {
            return Err(format!(
                "{shown}: {size} bytes; analyze reads WAV files up to {MAX_FILE_BYTES} bytes"
            ));
        }
        let head = header(file, size)
            .map_err(|e| format!("{shown}: can't read it: {e}"))?
            .ok_or_else(|| format!("{shown}: not a WAV file pfx can decode"))?;
        if head.channels > MAX_CHANNELS {
            return Err(format!(
                "{shown}: {} channels; analyze reads up to {MAX_CHANNELS}",
                head.channels
            ));
        }
        if !(RATES[0]..=RATES[1]).contains(&head.rate) {
            return Err(format!(
                "{shown}: a sample rate of {} Hz; analyze reads {} to {} Hz",
                head.rate, RATES[0], RATES[1]
            ));
        }
        let frames = head.samples / u64::from(head.channels);
        let seconds = frames as f64 / f64::from(head.rate);
        if seconds > MAX_SECONDS {
            return Err(format!(
                "{shown}: {seconds:.0} seconds long; analyze reads stems up to {MAX_SECONDS} seconds"
            ));
        }
        total += seconds;
        total_samples += head.samples;
        if total > MAX_TOTAL_SECONDS || total_samples > MAX_SAMPLES {
            return Err(format!(
                "{}: the stems run past {MAX_TOTAL_SECONDS} seconds or {MAX_SAMPLES} samples in all; analyze fewer or shorter ones",
                path.display()
            ));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(file)
            .and_then(|f| f.take(size).read_to_end(&mut bytes))
            .map_err(|e| format!("{shown}: can't read it: {e}"))?;
        let clip = Clip::from_wav(&bytes)
            .filter(|clip| clip.channels == head.channels && clip.rate == head.rate)
            .ok_or_else(|| format!("{shown}: not a WAV file pfx can decode"))?;
        drop(bytes);
        if clip.samples.len() as u64 > head.samples {
            return Err(format!("{shown}: changed while analyze read it"));
        }
        if clip.samples.iter().any(|s| !s.is_finite()) {
            return Err(format!(
                "{shown}: holds samples that are not numbers (NaN or infinity)"
            ));
        }
        if clip.samples.iter().any(|s| s.abs() > MAX_SAMPLE) {
            return Err(format!(
                "{shown}: holds samples past {MAX_SAMPLE} times full scale; analyze reads audio at sane levels"
            ));
        }
        let name = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        stems.push(Stem { name, clip });
    }
    Ok(stems)
}

fn is_wav(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("wav"))
}

pub fn mono(clip: &Clip, rate: u32) -> Vec<f32> {
    let channels = clip.channels as usize;
    if channels == 0 || clip.rate == 0 || rate == 0 {
        return Vec::new();
    }
    let scale = 1.0 / channels as f32;
    let samples: Vec<f32> = clip
        .samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * scale)
        .collect();
    if clip.rate == rate {
        return samples;
    }
    Clip {
        rate: clip.rate,
        channels: 1,
        samples,
    }
    .for_rate(rate)
    .samples
}

fn each<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(items.len());
    if workers <= 1 {
        return items.iter().map(work).collect();
    }
    let per = items.len().div_ceil(workers);
    let work = &work;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(per)
            .map(|chunk| scope.spawn(move || chunk.iter().map(work).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}

pub fn piece(name: &str, stems: &[Stem]) -> Result<Piece, String> {
    if stems.is_empty() {
        return Err(format!("{name}: no stems to analyze"));
    }
    let banks = Banks::new(RATE);
    let monos: Vec<Vec<f32>> = each(stems, |stem| mono(&stem.clip, RATE));
    let length = monos.iter().map(Vec::len).max().unwrap_or(0);
    let mut mix = vec![0.0f32; length];
    for samples in &monos {
        for (sum, s) in mix.iter_mut().zip(samples) {
            *sum += s;
        }
    }
    let power = Power::of(&mix);
    let strength = onset::strength_of(&power, &banks, RATE);
    let beat = tempo::estimate(&strength)
        .ok_or_else(|| format!("{name}: no beat found; analyze needs audio with a pulse"))?;
    let beats_per_bar = tempo::beats_per_bar(&strength, &beat);
    let per_beat = chroma::per_beat(&chroma::frames_of(&power, &banks, RATE), &beat, 1.0);
    drop(power);
    let levels = level::per_bar(&mix, RATE, &beat, beats_per_bar);
    drop(mix);
    let bar = f64::from(beats_per_bar);
    let beats = levels.len() as f64 * bar;
    let loudest = monos.iter().map(|m| rms(m)).fold(0.0f64, f64::max);
    let named: Vec<(&str, &[f32])> = stems
        .iter()
        .zip(&monos)
        .map(|(stem, samples)| (stem.name.as_str(), samples.as_slice()))
        .collect();
    let heard = each(&named, |(name, samples)| {
        part(name, samples, &banks, &beat, bar, beats, loudest)
    });
    let mut harmonic: Option<Vec<[f64; 12]>> = None;
    let mut parts = Vec::with_capacity(heard.len());
    for (part, chroma) in heard {
        if let Some(chroma) = chroma {
            let sum = harmonic.get_or_insert_with(Vec::new);
            if sum.len() < chroma.len() {
                sum.resize(chroma.len(), [0.0; 12]);
            }
            for (s, c) in sum.iter_mut().zip(&chroma) {
                for (a, b) in s.iter_mut().zip(c) {
                    *a += b;
                }
            }
        }
        parts.push(part);
    }
    let per_beat = harmonic.unwrap_or(per_beat);
    let (spans, hits) = spans(&levels, bar);
    Ok(Piece {
        origin: Origin::Audio,
        name: name.to_string(),
        bpm: round(beat.bpm, 1e2),
        beats_per_bar,
        beats,
        key: None,
        chords: None,
        chroma: Some(Chroma {
            beats: 1.0,
            frames: per_beat,
        }),
        parts,
        spans,
        hits,
        patches: BTreeMap::new(),
        cues: Vec::new(),
        master: None,
    })
}

fn part(
    name: &str,
    samples: &[f32],
    banks: &Banks,
    beat: &tempo::Beat,
    bar: f64,
    end: f64,
    loudest: f64,
) -> (Part, Option<Vec<[f64; 12]>>) {
    let power = Power::of(samples);
    let strength = onset::strength_of(&power, banks, RATE);
    let peaks = onset::peaks(&strength);
    let top = strength.values.iter().copied().fold(0.0f32, f32::max);
    let bands = level::bands_of(&power, RATE);
    let band = (0..3).fold(0, |best, i| if bands[i] > bands[best] { i } else { best });
    let base = BASE_MIDI[band];
    let seconds_per_beat = 60.0 / beat.bpm;
    let mut starts: Vec<(f64, usize)> = Vec::new();
    for &peak in &peaks {
        let at = round(
            ((strength.seconds(peak) - beat.offset) / seconds_per_beat).max(0.0),
            1e4,
        );
        if at >= end {
            break;
        }
        if starts.last().is_some_and(|&(last, _)| at <= last) {
            continue;
        }
        starts.push((at, peak));
    }
    let notes = starts
        .iter()
        .enumerate()
        .map(|(i, &(at, peak))| {
            let next = starts.get(i + 1).map_or(end, |&(next, _)| next);
            let beats = round((next - at).min(bar).max(MIN_BEATS), 1e4);
            let vel = if top > 0.0 {
                f64::from(strength.values[peak] / top)
            } else {
                0.0
            };
            let class = strongest_class(&power, &banks.chroma, peak);
            Note::new(
                at,
                base + class as f64,
                beats,
                round(vel.clamp(0.0, 1.0), 1e4),
            )
        })
        .collect();
    let peak_frames: Vec<usize> = starts.iter().map(|&(_, peak)| peak).collect();
    let gain = if loudest > 0.0 {
        round(rms(samples) / loudest, 1e4) as f32
    } else {
        0.0
    };
    let pitched = !peak_frames.is_empty() && flatness(&power, &peak_frames) <= FLATNESS;
    let chroma =
        pitched.then(|| chroma::per_beat(&chroma::frames_of(&power, banks, RATE), beat, 1.0));
    let part = Part {
        name: name.to_string(),
        patch: None,
        plays: None,
        pitched,
        gain,
        notes,
    };
    (part, chroma)
}

fn strongest_class(power: &Power, bank: &[[f64; 12]], at: usize) -> usize {
    let mut sum = [0.0f64; 12];
    for t in at..at + NOTE_FRAMES {
        let Some(frame) = power.frame(t) else {
            break;
        };
        for (s, v) in sum.iter_mut().zip(chroma::frame(frame, bank)) {
            *s += v;
        }
    }
    (0..12).fold(0, |best, i| if sum[i] > sum[best] { i } else { best })
}

fn log_sum(values: &[f32]) -> f64 {
    const EXPONENT: u64 = 0x7ff << 52;
    const ONE: u64 = 1023 << 52;
    let mut mantissa = 1.0f64;
    let mut exponent = 0i64;
    for &p in values {
        mantissa *= f64::from(p).max(1e-10);
        let bits = mantissa.to_bits();
        exponent += ((bits & EXPONENT) >> 52) as i64 - 1023;
        mantissa = f64::from_bits((bits & !EXPONENT) | ONE);
    }
    ln(mantissa) + exponent as f64 * LN_2
}

fn flatness(power: &Power, frames: &[usize]) -> f64 {
    let mut total = 0.0;
    let mut count = 0usize;
    let mut seen = Vec::new();
    for &frame in frames {
        for t in frame..frame + NOTE_FRAMES {
            if seen.last().is_some_and(|&last| t <= last) {
                continue;
            }
            seen.push(t);
            let Some(row) = power.frame(t) else {
                continue;
            };
            let sum: f64 = row.iter().map(|&p| f64::from(p).max(1e-10)).sum();
            let n = row.len() as f64;
            total += exp(log_sum(row) / n) / (sum / n);
            count += 1;
        }
    }
    if count == 0 {
        return 1.0;
    }
    total / count as f64
}

fn spans(levels: &[f32], bar: f64) -> (Vec<Span>, Vec<f64>) {
    let mut starts = vec![0usize];
    let mut hits = Vec::new();
    let mut hit_at = BTreeMap::new();
    for i in 1..levels.len() {
        let jump = levels[i] - levels[i - 1];
        if jump.abs() > SPAN_STEP {
            starts.push(i);
            if jump > HIT_STEP {
                hit_at.insert(i, hits.len());
                hits.push(i as f64 * bar);
            }
        }
    }
    let spans = starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let stop = starts.get(n + 1).copied().unwrap_or(levels.len().max(1));
            let first = levels.get(start).copied().unwrap_or(0.0);
            let last = levels.get(stop.saturating_sub(1)).copied().unwrap_or(first);
            Span {
                name: format!("part{}", n + 1),
                from: start as f64 * bar,
                beats: (stop - start) as f64 * bar,
                tempo: 1.0,
                intensity: Some([first, last]),
                gain_db: 0.0,
                hit: hit_at.get(&start).copied(),
            }
        })
        .collect();
    (spans, hits)
}

fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt()
}

fn round(value: f64, scale: f64) -> f64 {
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use pfx_core::sim::sin;

    use super::*;

    fn stem(name: &str, bpm: f64, hz: f64, seconds: f64) -> Stem {
        let frames = (seconds * f64::from(RATE)) as usize;
        let period = 60.0 / bpm * f64::from(RATE);
        let samples = (0..frames)
            .flat_map(|i| {
                let since = (i as f64) % period;
                let decay = exp(-since / (0.05 * f64::from(RATE)));
                let s = (0.5 * decay * sin(2.0 * PI * hz * i as f64 / f64::from(RATE))) as f32;
                [s, s * 0.5]
            })
            .collect();
        Stem {
            name: name.to_string(),
            clip: Clip {
                rate: RATE,
                channels: 2,
                samples,
            },
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/compose-analyze-tests")
            .join(format!("audio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pcm8(channels: u16, rate: u32, samples: usize) -> Vec<u8> {
        let data = vec![128u8; samples];
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(channels)).to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    fn refused(dir: &Path, file: &str, bytes: &[u8], says: &str) {
        let path = dir.join(file);
        std::fs::write(&path, bytes).unwrap();
        let refusal = read(&path).unwrap_err();
        assert!(refusal.contains(file), "{refusal}");
        assert!(refusal.contains(says), "{refusal}");
    }

    #[test]
    fn stems_past_the_bounds_are_refused_naming_the_file() {
        let dir = scratch("bounds");
        refused(&dir, "slow.wav", &pcm8(1, 1, 16), "Hz");
        refused(&dir, "fast.wav", &pcm8(1, 400_000, 16), "Hz");
        refused(
            &dir,
            "wide.wav",
            &pcm8(MAX_CHANNELS + 1, 4_000, 64),
            "channels",
        );
        let long = (MAX_SECONDS as usize + 1) * 4_000;
        refused(&dir, "long.wav", &pcm8(1, 4_000, long), "seconds");
        let float = |s: f32| {
            Clip {
                rate: RATE,
                channels: 1,
                samples: vec![0.0, s, 0.0],
            }
            .to_wav()
        };
        refused(&dir, "nan.wav", &float(f32::NAN), "not numbers");
        refused(&dir, "endless.wav", &float(f32::INFINITY), "not numbers");
        refused(&dir, "loud.wav", &float(1e30), "full scale");
        refused(&dir, "empty.wav", b"RIFF\0\0\0\0WAVE", "decode");
        #[cfg(unix)]
        {
            let zero = dir.join("zero.wav");
            std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
            let refusal = read(&zero).unwrap_err();
            assert!(refusal.contains("zero.wav"), "{refusal}");
        }
        let fine = dir.join("fine.wav");
        std::fs::write(&fine, pcm8(2, 8_000, 1_600)).unwrap();
        let stems = read(&fine).unwrap();
        assert_eq!(stems.len(), 1);
        assert_eq!(stems[0].name, "fine");
        assert_eq!((stems[0].clip.rate, stems[0].clip.channels), (8_000, 2));
        assert_eq!(stems[0].clip.samples.len(), 1_600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_stems_give_the_same_piece() {
        let stems = [
            stem("bass", 120.0, 110.0, 6.0),
            stem("lead", 120.0, 660.0, 6.0),
        ];
        let first = piece("loop", &stems).unwrap();
        let second = piece("loop", &stems).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.origin, Origin::Audio);
        assert_eq!(
            first
                .parts
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["bass", "lead"]
        );
    }
}
