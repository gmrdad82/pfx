#[derive(Clone, Debug)]
pub struct Clip {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Clip {
    pub fn from_wav(bytes: &[u8]) -> Option<Clip> {
        crate::wav::parse(bytes)
    }

    pub fn to_wav(&self) -> Vec<u8> {
        crate::wav::write_f32(self)
    }

    pub fn to_wav_i16(&self) -> Vec<u8> {
        crate::wav::write_i16(self)
    }

    pub fn reverse(&mut self) {
        let channels = self.channels as usize;
        if channels == 0 {
            return;
        }
        let frames: Vec<&[f32]> = self.samples.chunks_exact(channels).collect();
        let mut reversed = Vec::with_capacity(frames.len().saturating_mul(channels));
        for frame in frames.into_iter().rev() {
            reversed.extend_from_slice(frame);
        }
        self.samples = reversed;
    }

    pub fn seconds(&self) -> f32 {
        if self.rate == 0 || self.channels == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / (self.rate as f32 * self.channels as f32)
    }

    pub fn frames(&self) -> u64 {
        if self.channels == 0 {
            return 0;
        }
        (self.samples.len() / self.channels as usize) as u64
    }

    pub fn for_rate(&self, rate: u32) -> Clip {
        if self.rate == rate {
            return self.clone();
        }
        Clip {
            rate,
            channels: self.channels,
            samples: self.at_rate(rate, self.channels),
        }
    }

    pub fn at_rate(&self, rate: u32, channels: u16) -> Vec<f32> {
        if self.rate == rate && self.channels == channels {
            return self.samples.clone();
        }
        if self.rate == 0 || self.channels == 0 || rate == 0 || channels == 0 {
            return Vec::new();
        }
        if self.rate == rate {
            return remap(&self.samples, self.channels, channels);
        }
        resample(self, rate, channels)
    }
}

fn remap(samples: &[f32], src_channels: u16, dst_channels: u16) -> Vec<f32> {
    let src_channels = src_channels as usize;
    let dst_channels = dst_channels as usize;
    if src_channels == 0 || dst_channels == 0 {
        return Vec::new();
    }
    let frames = samples.len() / src_channels;
    let mut out = Vec::with_capacity(frames.saturating_mul(dst_channels));
    for frame in 0..frames {
        for channel in 0..dst_channels {
            let src = channel.min(src_channels - 1);
            out.push(samples[frame * src_channels + src]);
        }
    }
    out
}

fn resample(clip: &Clip, rate: u32, channels: u16) -> Vec<f32> {
    let src_channels = clip.channels as usize;
    let frames_in = clip.samples.len() / src_channels;
    let frames_out = scaled_frames(frames_in, clip.rate, rate);
    if frames_out == 0 || clip.samples.is_empty() {
        return Vec::new();
    }
    let dst_channels = channels as usize;
    let last = clip.samples.len() - 1;
    let mut out = Vec::with_capacity(frames_out.saturating_mul(dst_channels));
    for i in 0..frames_out {
        let pos = i as f64 * f64::from(clip.rate) / f64::from(rate);
        let base = pos.floor().max(0.0) as usize;
        let frac = (pos - base as f64) as f32;
        for channel in 0..dst_channels {
            let src = channel.min(src_channels - 1);
            let a = clip.samples[(base.saturating_mul(src_channels).saturating_add(src)).min(last)];
            let b = clip.samples[(base
                .saturating_add(1)
                .saturating_mul(src_channels)
                .saturating_add(src))
            .min(last)];
            out.push(a + (b - a) * frac);
        }
    }
    out
}

fn scaled_frames(frames_in: usize, from: u32, to: u32) -> usize {
    if from == 0 {
        return 0;
    }
    let frames = (frames_in as u128).saturating_mul(u128::from(to)) / u128::from(from);
    usize::try_from(frames).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|sample| sample.to_bits()).collect()
    }

    fn pcm16(rate: u32, channels: u16, frames: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = frames
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"fact");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&(frames.len() as u32 / u32::from(channels.max(1))).to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn a_pcm16_wav_parses_to_unit_samples() {
        let clip = Clip::from_wav(&pcm16(48_000, 1, &[0, 16384, -32768])).unwrap();
        assert_eq!(clip.rate, 48_000);
        assert_eq!(clip.channels, 1);
        assert_eq!(bits(&clip.samples), bits(&[0.0, 0.5, -1.0]));
    }

    #[test]
    fn garbage_is_not_a_clip() {
        assert!(Clip::from_wav(b"not a wav").is_none());
        assert!(Clip::from_wav(&[]).is_none());
        assert!(Clip::from_wav(&pcm16(0, 1, &[0])).is_none());
    }

    #[test]
    fn eight_bit_samples_are_centred_on_128() {
        let data = [0u8, 128, 255];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        let clip = Clip::from_wav(&bytes).unwrap();
        assert_eq!(bits(&clip.samples), bits(&[-1.0, 0.0, 127.0 / 128.0]));
    }

    #[test]
    fn wav_f32_and_pcm16_round_trip() {
        let clip = Clip {
            rate: 48_000,
            channels: 2,
            samples: vec![0.0, 1.0, -1.0, 0.1, -0.3],
        };
        let back = Clip::from_wav(&clip.to_wav()).unwrap();
        assert_eq!(back.rate, 48_000);
        assert_eq!(back.channels, 2);
        assert_eq!(bits(&back.samples), bits(&clip.samples));
        assert_eq!(&clip.to_wav()[20..22], &3u16.to_le_bytes());
        assert_eq!(&clip.to_wav()[34..36], &32u16.to_le_bytes());

        let pcm = Clip {
            rate: 44_100,
            channels: 1,
            samples: vec![0.0, 0.5, -1.0],
        };
        let back = Clip::from_wav(&pcm.to_wav_i16()).unwrap();
        assert_eq!(bits(&back.samples), bits(&[0.0, 0.5, -1.0]));
        assert_eq!(&pcm.to_wav_i16()[20..22], &1u16.to_le_bytes());
        assert_eq!(&pcm.to_wav_i16()[34..36], &16u16.to_le_bytes());
    }

    #[test]
    fn reversing_keeps_stereo_frames_paired() {
        let mut clip = Clip::from_wav(&pcm16(48_000, 2, &[1, 2, 3, 4, 5, 6])).unwrap();
        clip.reverse();
        let back: Vec<i32> = clip
            .samples
            .iter()
            .map(|sample| (sample * 32768.0).round() as i32)
            .collect();
        assert_eq!(back, vec![5, 6, 3, 4, 1, 2]);
    }

    #[test]
    fn matching_rates_copy_samples_and_a_rate_change_interpolates() {
        let clip = Clip {
            rate: 24_000,
            channels: 1,
            samples: vec![0.0, 1.0, 0.0, -1.0],
        };
        assert_eq!(bits(&clip.at_rate(24_000, 1)), bits(&clip.samples));
        let spread = clip.at_rate(24_000, 2);
        assert_eq!(
            bits(&spread),
            bits(&[0.0, 0.0, 1.0, 1.0, 0.0, 0.0, -1.0, -1.0])
        );
        let up = clip.at_rate(48_000, 1);
        assert_eq!(
            bits(&up),
            bits(&[0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -1.0])
        );
        let tone = Clip::from_wav(&pcm16(24_000, 1, &[16384; 240])).unwrap();
        let stereo = tone.at_rate(48_000, 2);
        assert_eq!(stereo.len(), 960);
        assert_eq!(bits(&stereo[..2]), bits(&[0.5, 0.5]));
        assert!((tone.seconds() - 0.01).abs() < 0.0001);
    }
}
