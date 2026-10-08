use crate::protocol::{AUDIO_CHANNELS, AUDIO_RATE, AudioFormat};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Pcm {
    pub format: &'static str,
    pub rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub sha256: String,
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn half(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

pub fn read(bytes: &[u8], announced: AudioFormat) -> Result<Pcm, String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("the adapter's audio is not a WAV file".into());
    }
    let mut at = 12;
    let mut fmt = None;
    let mut data = None;
    while let (Some(id), Some(size)) = (bytes.get(at..at + 4), word(bytes, at + 4)) {
        let start = at + 8;
        let end = (start + size as usize).min(bytes.len());
        match id {
            b"fmt " => fmt = Some(&bytes[start..end]),
            b"data" => {
                data = Some(&bytes[start..end]);
                break;
            }
            _ => {}
        }
        at = start + size as usize + (size as usize & 1);
    }
    let fmt = fmt.ok_or("the adapter's WAV has no fmt chunk")?;
    let data = data.ok_or("the adapter's WAV has no data chunk")?;
    let tag = half(fmt, 0).ok_or("the adapter's WAV fmt chunk is short")?;
    let channels = half(fmt, 2).ok_or("the adapter's WAV fmt chunk is short")?;
    let rate = word(fmt, 4).ok_or("the adapter's WAV fmt chunk is short")?;
    let bits = half(fmt, 14).ok_or("the adapter's WAV fmt chunk is short")?;
    let tag = if tag == 0xFFFE {
        half(fmt, 24).ok_or("the adapter's WAV fmt chunk is short")?
    } else {
        tag
    };
    let (want_tag, want_bits) = match announced {
        AudioFormat::S16 => (1, 16),
        AudioFormat::F32 => (3, 32),
    };
    if tag != want_tag || bits != want_bits {
        return Err(format!(
            "the adapter announced {} audio, and its WAV holds format {tag} at {bits} bits",
            announced.name()
        ));
    }
    if channels != AUDIO_CHANNELS || rate != AUDIO_RATE {
        return Err(format!(
            "the adapter's audio is {channels} channels at {rate} Hz, expected {AUDIO_CHANNELS} at {AUDIO_RATE}"
        ));
    }
    let block = usize::from(channels) * usize::from(bits / 8);
    if data.is_empty() || data.len() % block != 0 {
        return Err("the adapter's audio holds no whole frames".into());
    }
    Ok(Pcm {
        format: announced.name(),
        rate,
        channels,
        frames: (data.len() / block) as u64,
        sha256: hex::encode(Sha256::digest(data)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::wav_bytes;

    #[test]
    fn a_wav_the_library_writes_reads_back() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.0];
        for format in [AudioFormat::S16, AudioFormat::F32] {
            let pcm = read(&wav_bytes(format, &samples), format).unwrap();
            assert_eq!(pcm.frames, 2);
            assert_eq!(pcm.rate, 48_000);
            assert_eq!(pcm.channels, 2);
            assert_eq!(pcm.sha256.len(), 64);
        }
    }

    #[test]
    fn the_hash_covers_the_samples_and_not_the_header() {
        let a = read(&wav_bytes(AudioFormat::F32, &[0.0, 0.25]), AudioFormat::F32).unwrap();
        let b = read(&wav_bytes(AudioFormat::F32, &[0.0, 0.25]), AudioFormat::F32).unwrap();
        let c = read(&wav_bytes(AudioFormat::F32, &[0.0, 0.5]), AudioFormat::F32).unwrap();
        assert_eq!(a.sha256, b.sha256);
        assert_ne!(a.sha256, c.sha256);
    }

    #[test]
    fn a_wav_in_another_format_than_announced_is_refused() {
        let wav = wav_bytes(AudioFormat::S16, &[0.0, 0.0]);
        assert!(read(&wav, AudioFormat::F32).unwrap_err().contains("f32"));
        assert!(read(b"nope", AudioFormat::S16).is_err());
    }

    #[test]
    fn a_wav_at_another_rate_is_refused() {
        let mut wav = wav_bytes(AudioFormat::S16, &[0.0, 0.0]);
        wav[24..28].copy_from_slice(&44_100u32.to_le_bytes());
        assert!(read(&wav, AudioFormat::S16).unwrap_err().contains("44100"));
    }
}
