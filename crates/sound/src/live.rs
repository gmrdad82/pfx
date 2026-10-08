use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::channel::{ChannelId, Ducking};
use crate::clip::Clip;
use crate::decode::{Decode, DecodeSpec, spec_at_output};
use crate::mix::{Level, Mixer, VoiceId, lock};
use crate::music::MusicTrack;
use crate::pcm::PcmSender;
use crate::sound::{SoundId, SoundSpec};
use crate::synth::Wave;

pub struct Output {
    stream: Option<cpal::Stream>,
    mixer: Arc<Mutex<Mixer>>,
    decodes: Mutex<Vec<Decode>>,
    rate: u32,
    channels: u16,
}

impl Output {
    pub fn open() -> Result<Output, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no output device".to_string())?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("output device: {e}"))?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate;
        let channels = config.channels;
        if channels == 0 || rate == 0 {
            return Err("output device has no channels".into());
        }
        let mixer = Arc::new(Mutex::new(Mixer::new(rate, channels)));
        let stream = match format {
            cpal::SampleFormat::F32 => {
                let shared = Arc::clone(&mixer);
                device
                    .build_output_stream(
                        config,
                        move |data: &mut [f32], _| fill_f32(data, &shared),
                        |_| {},
                        None,
                    )
                    .map_err(|e| format!("output stream: {e}"))?
            }
            cpal::SampleFormat::I16 => {
                let shared = Arc::clone(&mixer);
                device
                    .build_output_stream(
                        config,
                        move |data: &mut [i16], _| fill_i16(data, &shared),
                        |_| {},
                        None,
                    )
                    .map_err(|e| format!("output stream: {e}"))?
            }
            other => return Err(format!("output format {other} is not supported")),
        };
        stream.play().map_err(|e| format!("output stream: {e}"))?;
        Ok(Output {
            stream: Some(stream),
            mixer,
            decodes: Mutex::new(Vec::new()),
            rate,
            channels,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn play(&self, clip: &Clip, level: Level) -> VoiceId {
        lock(&self.mixer).play(clip, level)
    }

    pub fn set_volume(&self, id: VoiceId, volume: f32) {
        lock(&self.mixer).set_volume(id, volume);
    }

    pub fn set_pan(&self, id: VoiceId, pan: f32) {
        lock(&self.mixer).set_pan(id, pan);
    }

    pub fn play_on(&self, channel: ChannelId, clip: &Clip, level: Level) -> VoiceId {
        lock(&self.mixer).play_on(channel, clip, level)
    }

    pub fn stop(&self, id: VoiceId, fade_seconds: f32) {
        lock(&self.mixer).stop(id, fade_seconds);
    }

    pub fn set_master_volume(&self, volume: f32) {
        lock(&self.mixer).set_master_volume(volume);
    }

    pub fn set_master_muted(&self, muted: bool) {
        lock(&self.mixer).set_master_muted(muted);
    }

    pub fn set_channel_volume(&self, channel: ChannelId, volume: f32) {
        lock(&self.mixer).set_channel_volume(channel, volume);
    }

    pub fn set_channel_muted(&self, channel: ChannelId, muted: bool) {
        lock(&self.mixer).set_channel_muted(channel, muted);
    }

    pub fn set_ducking(&self, ducking: Option<Ducking>) {
        lock(&self.mixer).set_ducking(ducking);
    }

    pub fn set_pitch(&self, id: VoiceId, ratio: f32) {
        lock(&self.mixer).set_pitch(id, ratio);
    }

    pub fn glide_pitch(&self, id: VoiceId, ratio: f32, seconds: f32) {
        lock(&self.mixer).glide_pitch(id, ratio, seconds);
    }

    pub fn set_gain(&self, id: VoiceId, gain: f32) {
        lock(&self.mixer).set_gain(id, gain);
    }

    pub fn glide_gain(&self, id: VoiceId, gain: f32, seconds: f32) {
        lock(&self.mixer).glide_gain(id, gain, seconds);
    }

    pub fn set_channel_pitch(&self, channel: ChannelId, ratio: f32) {
        lock(&self.mixer).set_channel_pitch(channel, ratio);
    }

    pub fn glide_channel_pitch(&self, channel: ChannelId, ratio: f32, seconds: f32) {
        lock(&self.mixer).glide_channel_pitch(channel, ratio, seconds);
    }

    pub fn set_channel_lowpass(&self, channel: ChannelId, hz: Option<f32>) {
        lock(&self.mixer).set_channel_lowpass(channel, hz);
    }

    pub fn set_channel_highpass(&self, channel: ChannelId, hz: Option<f32>) {
        lock(&self.mixer).set_channel_highpass(channel, hz);
    }

    pub fn set_limiter(&self, threshold: Option<f32>) {
        lock(&self.mixer).set_limiter(threshold);
    }

    pub fn set_seed(&self, seed: u64) {
        lock(&self.mixer).set_seed(seed);
    }

    pub fn sound(&self, clip: &Clip, spec: SoundSpec) -> SoundId {
        lock(&self.mixer).sound(clip, spec)
    }

    pub fn trigger(&self, sound: SoundId) -> Option<VoiceId> {
        lock(&self.mixer).trigger(sound)
    }

    pub fn tone(&self, channel: ChannelId, wave: Wave, hz: f32, volume: f32) -> VoiceId {
        lock(&self.mixer).tone(channel, wave, hz, volume)
    }

    pub fn set_tone_frequency(&self, id: VoiceId, hz: f32) {
        lock(&self.mixer).set_tone_frequency(id, hz);
    }

    pub fn glide_tone_frequency(&self, id: VoiceId, hz: f32, seconds: f32) {
        lock(&self.mixer).glide_tone_frequency(id, hz, seconds);
    }

    pub fn pcm_stream(
        &self,
        channel: ChannelId,
        channels: u16,
        capacity_seconds: f32,
        level: Level,
    ) -> (VoiceId, PcmSender) {
        lock(&self.mixer).pcm_stream(channel, channels, capacity_seconds, level)
    }

    pub fn play_music(&self, track: &MusicTrack, fade_seconds: f32, level: Level) -> VoiceId {
        lock(&self.mixer).play_music(track, fade_seconds, level)
    }

    pub fn stop_music(&self, fade_seconds: f32) {
        lock(&self.mixer).stop_music(fade_seconds);
    }

    #[cfg(feature = "vorbis")]
    pub fn stream_vorbis(
        &self,
        channel: ChannelId,
        source: &crate::vorbis::VorbisSource,
        looping: Option<(u64, u64)>,
        level: Level,
    ) -> Result<VoiceId, String> {
        lock(&self.mixer).stream_vorbis(channel, source, looping, level)
    }

    #[cfg(feature = "vorbis")]
    pub fn play_music_vorbis(
        &self,
        source: &crate::vorbis::VorbisSource,
        looping: Option<(u64, u64)>,
        fade_seconds: f32,
        level: Level,
    ) -> Result<VoiceId, String> {
        lock(&self.mixer).play_music_vorbis(source, looping, fade_seconds, level)
    }

    pub fn stream(&self, spec: &DecodeSpec, level: Level) -> Result<VoiceId, String> {
        let spec = spec_at_output(spec, self.rate, self.channels);
        let decode = Decode::start(&spec)?;
        let channels = decode.channels();
        let queue = decode.queue();
        let id = lock(&self.mixer).stream(queue, channels, level);
        lock(&self.decodes).push(decode);
        Ok(id)
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stream.take();
        lock(&self.decodes).clear();
    }
}

pub fn chime(wav: &'static [u8]) {
    spawn_chime(wav, false);
}

pub fn chime_reversed(wav: &'static [u8]) {
    spawn_chime(wav, true);
}

fn spawn_chime(wav: &'static [u8], reversed: bool) {
    std::thread::spawn(move || {
        let Some(mut clip) = Clip::from_wav(wav) else {
            return;
        };
        if reversed {
            clip.reverse();
        }
        let _ = play_chime(clip);
    });
}

fn play_chime(clip: Clip) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no output device".to_string())?;
    let default = device
        .default_output_config()
        .map_err(|e| format!("output device: {e}"))?;
    let rate = default.sample_rate();
    let channels = default.channels();
    let config = cpal::StreamConfig {
        channels,
        sample_rate: rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let buffer = Arc::new(clip.at_rate(rate, channels));
    let head = Arc::new(AtomicUsize::new(0));
    let total = buffer.len();
    let feed_buffer = Arc::clone(&buffer);
    let feed_head = Arc::clone(&head);
    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [f32], _| {
                let start = feed_head.load(Ordering::Relaxed);
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = feed_buffer.get(start + i).copied().unwrap_or(0.0);
                }
                feed_head.store(start.saturating_add(out.len()), Ordering::Relaxed);
            },
            |_| {},
            None,
        )
        .map_err(|e| format!("output stream: {e}"))?;
    stream.play().map_err(|e| format!("output stream: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs_f32(clip.seconds() + 0.35);
    while head.load(Ordering::Relaxed) < total && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(15));
    }
    drop(stream);
    Ok(())
}

fn fill_f32(data: &mut [f32], mixer: &Mutex<Mixer>) {
    lock(mixer).render_into(data);
}

fn fill_i16(data: &mut [i16], mixer: &Mutex<Mixer>) {
    lock(mixer).render_i16(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a sound device"]
    fn the_default_output_plays_a_clip() {
        let clip = Clip {
            rate: 48_000,
            channels: 1,
            samples: vec![0.2; 4_800],
        };
        let output = Output::open().expect("output device");
        output.play(&clip, Level::default());
        std::thread::sleep(Duration::from_millis(150));
        assert!(output.rate() > 0 && output.channels() > 0);
    }

    #[test]
    #[ignore = "needs a sound device"]
    fn a_chime_plays_on_the_default_output() {
        use std::sync::OnceLock;
        static WAV: OnceLock<Vec<u8>> = OnceLock::new();
        let wav = WAV.get_or_init(|| {
            Clip {
                rate: 48_000,
                channels: 1,
                samples: vec![0.2; 2_400],
            }
            .to_wav()
        });
        chime(wav);
        chime_reversed(wav);
        std::thread::sleep(Duration::from_millis(400));
    }
}
