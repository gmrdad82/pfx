use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::channel::{
    BUILT_IN, Channel, ChannelId, DuckState, Ducking, RAMP_SECONDS, clamp_volume, ramp_frames,
};
use crate::clip::Clip;
use crate::envelope::{EnvState, Envelope};
use crate::filter::Kind;
use crate::interp::{Kernel, MAX_RATIO, MIN_RATIO};
use crate::music::{Curve, Direction, MusicTrack};
use crate::pcm::{PcmSender, pcm_pair};
use crate::sound::{Rng, SoundDef, SoundId, SoundSpec, Steal};
use crate::synth::{Synth, Wave};
use crate::voice::{Block, History, Source, Voice};

pub const FILTER_RAMP_SECONDS: f32 = 0.03;

const DEFAULT_SEED: u64 = 0x5EED_A0D1_0000_0001;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub volume: f32,
    pub pan: f32,
    pub fade_in: u64,
    pub fade_out: u64,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pan: 0.0,
            fade_in: 0,
            fade_out: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceId(u64);

#[derive(Clone, Debug)]
pub struct Placement {
    pub at: u64,
    pub clip: Clip,
    pub level: Level,
}

#[derive(Clone, Debug)]
pub struct Timeline {
    pub rate: u32,
    pub channels: u16,
    pub placements: Vec<Placement>,
}

impl Timeline {
    pub fn new(rate: u32, channels: u16) -> Self {
        Self {
            rate,
            channels,
            placements: Vec::new(),
        }
    }

    pub fn place(&mut self, at: u64, clip: Clip, level: Level) {
        self.placements.push(Placement { at, clip, level });
    }
}

pub fn mixdown(timeline: &Timeline) -> Vec<f32> {
    let mut mixer = Mixer::new(timeline.rate, timeline.channels);
    let mut end = 0u64;
    for placement in &timeline.placements {
        let matched = placement.clip.for_rate(timeline.rate);
        end = end.max(placement.at.saturating_add(matched.frames()));
        mixer.play_prepared(
            ChannelId::EFFECTS,
            placement.at,
            matched,
            None,
            placement.level,
        );
    }
    mixer.render(usize::try_from(end).unwrap_or(usize::MAX))
}

pub(crate) struct PcmQueue {
    inner: Mutex<QueueInner>,
}

struct QueueInner {
    samples: VecDeque<f32>,
    ended: bool,
}

impl PcmQueue {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(QueueInner {
                samples: VecDeque::new(),
                ended: false,
            }),
        })
    }

    pub(crate) fn push(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        lock(&self.inner).samples.extend(samples);
    }

    pub(crate) fn end(&self) {
        lock(&self.inner).ended = true;
    }

    pub(crate) fn len(&self) -> usize {
        lock(&self.inner).samples.len()
    }

    pub(crate) fn reader_ended(&self) -> bool {
        lock(&self.inner).ended
    }

    pub(crate) fn pop_frames(&self, frames: usize, channels: usize) -> Pop {
        if channels == 0 || frames == 0 {
            return Pop {
                samples: Vec::new(),
                real: 0,
                available: 0,
                ended: lock(&self.inner).ended,
            };
        }
        let mut guard = lock(&self.inner);
        let available = guard.samples.len().checked_div(channels).unwrap_or(0);
        let real = available.min(frames);
        let take = real * channels;
        let mut samples: Vec<f32> = guard.samples.drain(..take).collect();
        let ended = guard.ended;
        drop(guard);
        if let Some(len) = frames.checked_mul(channels) {
            samples.resize(len, 0.0);
        }
        Pop {
            samples,
            real,
            available,
            ended,
        }
    }

    pub(crate) fn is_finished(&self, channels: u16) -> bool {
        let guard = lock(&self.inner);
        guard.ended && guard.samples.len() < usize::from(channels.max(1))
    }
}

pub(crate) struct Pop {
    pub(crate) samples: Vec<f32>,
    pub(crate) real: usize,
    pub(crate) available: usize,
    pub(crate) ended: bool,
}

pub struct Mixer {
    rate: u32,
    channels: u16,
    clock: u64,
    next_id: u64,
    voices: Vec<Voice>,
    buses: BTreeMap<ChannelId, Channel>,
    master: Channel,
    limiter: Option<f32>,
    duck: Option<DuckState>,
    music: Option<VoiceId>,
    sounds: Vec<SoundDef>,
    rng: Rng,
    kernel: Kernel,
    scratch: Vec<f32>,
    duck_gains: Vec<f32>,
    activity: Vec<bool>,
}

impl Mixer {
    pub fn new(rate: u32, channels: u16) -> Self {
        let mut buses = BTreeMap::new();
        for id in BUILT_IN {
            buses.insert(id, Channel::new());
        }
        Self {
            rate,
            channels,
            clock: 0,
            next_id: 0,
            voices: Vec::new(),
            buses,
            master: Channel::new(),
            limiter: None,
            duck: None,
            music: None,
            sounds: Vec::new(),
            rng: Rng::new(DEFAULT_SEED),
            kernel: Kernel::new(),
            scratch: Vec::new(),
            duck_gains: Vec::new(),
            activity: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }

    pub fn set_master_volume(&mut self, volume: f32) {
        self.master.volume = clamp_volume(volume);
        retarget(&mut self.master, self.rate);
    }

    pub fn set_master_muted(&mut self, muted: bool) {
        self.master.muted = muted;
        retarget(&mut self.master, self.rate);
    }

    pub fn master_volume(&self) -> f32 {
        self.master.volume
    }

    pub fn master_muted(&self) -> bool {
        self.master.muted
    }

    pub fn set_limiter(&mut self, threshold: Option<f32>) {
        self.limiter = threshold.map(|value| {
            if value.is_nan() {
                1.0
            } else {
                value.clamp(0.05, 0.999)
            }
        });
    }

    pub fn limiter(&self) -> Option<f32> {
        self.limiter
    }

    pub fn set_channel_volume(&mut self, channel: ChannelId, volume: f32) {
        let rate = self.rate;
        let volume = clamp_volume(volume);
        match self.buses.get_mut(&channel) {
            Some(bus) => {
                bus.volume = volume;
                retarget(bus, rate);
            }
            None => {
                let mut bus = Channel::new();
                bus.volume = volume;
                bus.gain.snap(volume);
                self.buses.insert(channel, bus);
            }
        }
    }

    pub fn set_channel_muted(&mut self, channel: ChannelId, muted: bool) {
        let rate = self.rate;
        match self.buses.get_mut(&channel) {
            Some(bus) => {
                bus.muted = muted;
                retarget(bus, rate);
            }
            None => {
                let mut bus = Channel::new();
                bus.muted = muted;
                bus.gain.snap(bus.effective());
                self.buses.insert(channel, bus);
            }
        }
    }

    pub fn channel_volume(&self, channel: ChannelId) -> f32 {
        self.buses.get(&channel).map_or(1.0, |bus| bus.volume)
    }

    pub fn channel_gain(&self, channel: ChannelId) -> f32 {
        self.buses
            .get(&channel)
            .map_or(1.0, |bus| bus.gain.current())
    }

    pub fn master_gain(&self) -> f32 {
        self.master.gain.current()
    }

    pub fn channel_muted(&self, channel: ChannelId) -> bool {
        self.buses.get(&channel).is_some_and(|bus| bus.muted)
    }

    pub fn set_channel_pitch(&mut self, channel: ChannelId, ratio: f32) {
        self.glide_channel_pitch(channel, ratio, RAMP_SECONDS);
    }

    pub fn glide_channel_pitch(&mut self, channel: ChannelId, ratio: f32, seconds: f32) {
        let frames = glide_frames(seconds, self.rate);
        let ratio = clamp_ratio(ratio);
        match self.buses.get_mut(&channel) {
            Some(bus) => bus.pitch.to(ratio, frames),
            None => {
                let mut bus = Channel::new();
                bus.pitch.snap(ratio);
                self.buses.insert(channel, bus);
            }
        }
    }

    pub fn channel_pitch(&self, channel: ChannelId) -> f32 {
        self.buses
            .get(&channel)
            .map_or(1.0, |bus| bus.pitch.current())
    }

    pub fn set_channel_lowpass(&mut self, channel: ChannelId, hz: Option<f32>) {
        self.glide_channel_filter(channel, Kind::LowPass, hz, FILTER_RAMP_SECONDS);
    }

    pub fn set_channel_highpass(&mut self, channel: ChannelId, hz: Option<f32>) {
        self.glide_channel_filter(channel, Kind::HighPass, hz, FILTER_RAMP_SECONDS);
    }

    pub fn glide_channel_lowpass(&mut self, channel: ChannelId, hz: Option<f32>, seconds: f32) {
        self.glide_channel_filter(channel, Kind::LowPass, hz, seconds);
    }

    pub fn glide_channel_highpass(&mut self, channel: ChannelId, hz: Option<f32>, seconds: f32) {
        self.glide_channel_filter(channel, Kind::HighPass, hz, seconds);
    }

    pub fn channel_lowpass(&self, channel: ChannelId) -> Option<f32> {
        self.buses
            .get(&channel)
            .and_then(|bus| bus.lowpass.target())
    }

    pub fn channel_highpass(&self, channel: ChannelId) -> Option<f32> {
        self.buses
            .get(&channel)
            .and_then(|bus| bus.highpass.target())
    }

    fn glide_channel_filter(
        &mut self,
        channel: ChannelId,
        kind: Kind,
        hz: Option<f32>,
        seconds: f32,
    ) {
        let frames = glide_frames(seconds, self.rate);
        let rate = self.rate;
        let bus = self.buses.entry(channel).or_insert_with(Channel::new);
        match kind {
            Kind::LowPass => bus.lowpass.set(hz, frames, rate),
            Kind::HighPass => bus.highpass.set(hz, frames, rate),
        }
    }

    pub fn set_ducking(&mut self, ducking: Option<Ducking>) {
        self.duck = ducking.map(DuckState::new);
    }

    pub fn ducking(&self) -> Option<Ducking> {
        self.duck.as_ref().map(|state| state.config)
    }

    pub fn ducked_db(&self) -> f32 {
        self.duck.as_ref().map_or(0.0, |state| state.reduction_db)
    }

    pub fn play(&mut self, clip: &Clip, level: Level) -> VoiceId {
        self.play_on(ChannelId::EFFECTS, clip, level)
    }

    pub fn play_at(&mut self, at: u64, clip: &Clip, level: Level) -> VoiceId {
        self.play_at_on(ChannelId::EFFECTS, at, clip, level)
    }

    pub fn play_on(&mut self, channel: ChannelId, clip: &Clip, level: Level) -> VoiceId {
        self.play_at_on(channel, self.clock, clip, level)
    }

    pub fn play_at_on(
        &mut self,
        channel: ChannelId,
        at: u64,
        clip: &Clip,
        level: Level,
    ) -> VoiceId {
        self.play_prepared(channel, at, clip.for_rate(self.rate), None, level)
    }

    pub fn play_music(&mut self, track: &MusicTrack, fade_seconds: f32, level: Level) -> VoiceId {
        let looping = track.loop_frames(self.rate);
        let clip = track.clip.for_rate(self.rate);
        let id = self.play_prepared(ChannelId::MUSIC, self.clock, clip, looping, level);
        self.begin_music(id, fade_seconds);
        id
    }

    pub fn stop_music(&mut self, fade_seconds: f32) {
        if let Some(old) = self.music.take() {
            self.stop(old, fade_seconds);
        }
    }

    pub fn music(&self) -> Option<VoiceId> {
        self.music
            .filter(|id| self.voices.iter().any(|voice| voice.id == *id))
    }

    pub fn stop(&mut self, id: VoiceId, fade_seconds: f32) {
        let len = seconds_to_frames(fade_seconds, self.rate);
        let clock = self.clock;
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id)
            && let Some(state) = voice.envelope.as_mut()
        {
            if voice.rendered {
                state.release(len);
            } else {
                self.voices.retain(|voice| voice.id != id);
            }
            return;
        }
        if len == 0 {
            self.voices.retain(|voice| voice.id != id);
            return;
        }
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.curve = Some(Curve {
                start: clock,
                len,
                direction: Direction::Out,
            });
        }
    }

    pub fn set_volume(&mut self, id: VoiceId, volume: f32) {
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.level.volume = volume;
        }
    }

    pub fn set_pan(&mut self, id: VoiceId, pan: f32) {
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.level.pan = pan;
        }
    }

    pub fn set_pitch(&mut self, id: VoiceId, ratio: f32) {
        self.glide_pitch(id, ratio, RAMP_SECONDS);
    }

    pub fn glide_pitch(&mut self, id: VoiceId, ratio: f32, seconds: f32) {
        let frames = glide_frames(seconds, self.rate);
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.pitch.to(clamp_ratio(ratio), frames);
        }
    }

    pub fn pitch(&self, id: VoiceId) -> Option<f32> {
        self.voices
            .iter()
            .find(|voice| voice.id == id)
            .map(|voice| voice.pitch.current())
    }

    pub fn set_gain(&mut self, id: VoiceId, gain: f32) {
        self.glide_gain(id, gain, RAMP_SECONDS);
    }

    pub fn glide_gain(&mut self, id: VoiceId, gain: f32, seconds: f32) {
        let frames = glide_frames(seconds, self.rate);
        let gain = if gain.is_nan() { 0.0 } else { gain.max(0.0) };
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.gain.to(gain, frames);
        }
    }

    pub fn tone(&mut self, channel: ChannelId, wave: Wave, hz: f32, volume: f32) -> VoiceId {
        let level = Level {
            volume,
            ..Level::default()
        };
        let seed = self.rng.seed();
        self.push_voice(
            channel,
            self.clock,
            Source::Tone(Synth::new(wave, hz, seed)),
            level,
        )
    }

    pub fn set_envelope(&mut self, id: VoiceId, envelope: Option<Envelope>) {
        let rate = self.rate;
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.envelope = envelope.map(|envelope| EnvState::new(envelope, rate, voice.rendered));
        }
    }

    pub fn envelope_level(&self, id: VoiceId) -> Option<f32> {
        self.voices
            .iter()
            .find(|voice| voice.id == id)
            .and_then(|voice| voice.envelope.as_ref().map(EnvState::level))
    }

    pub fn pcm_stream(
        &mut self,
        channel: ChannelId,
        channels: u16,
        capacity_seconds: f32,
        level: Level,
    ) -> (VoiceId, PcmSender) {
        let capacity = seconds_to_frames(capacity_seconds, self.rate) as usize;
        let (voice, sender) = pcm_pair(channels.max(1), capacity, self.rate);
        let id = self.push_voice(channel, self.clock, Source::Pcm(voice), level);
        (id, sender)
    }

    pub fn set_tone_frequency(&mut self, id: VoiceId, hz: f32) {
        self.glide_tone_frequency(id, hz, RAMP_SECONDS);
    }

    pub fn glide_tone_frequency(&mut self, id: VoiceId, hz: f32, seconds: f32) {
        let frames = glide_frames(seconds, self.rate);
        let hz = if hz.is_nan() { 0.0 } else { hz.max(0.0) };
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id)
            && let Source::Tone(synth) = &mut voice.source
        {
            synth.frequency.to(hz, frames);
        }
    }

    pub fn sound(&mut self, clip: &Clip, spec: SoundSpec) -> SoundId {
        let prepared = clip.for_rate(self.rate);
        let frames = prepared.frames();
        let channels = prepared.channels;
        self.sounds.push(SoundDef {
            samples: Arc::new(prepared.samples),
            channels,
            frames,
            spec,
        });
        SoundId(self.sounds.len() - 1)
    }

    pub fn trigger(&mut self, sound: SoundId) -> Option<VoiceId> {
        self.trigger_at(self.clock, sound)
    }

    pub fn trigger_at(&mut self, at: u64, sound: SoundId) -> Option<VoiceId> {
        let def = self.sounds.get(sound.0)?;
        let spec = def.spec;
        let rate = self.rate;
        let source = Source::Buffer {
            samples: Arc::clone(&def.samples),
            channels: def.channels,
            frames: def.frames,
            looping: None,
        };
        if spec.max_voices == 0 {
            return None;
        }
        self.make_room(sound, &spec);
        let pitch = 2f32.powf(self.rng.signed() * spec.pitch_jitter / 12.0);
        let volume = (1.0 + self.rng.signed() * spec.volume_jitter).max(0.0);
        let mut level = spec.level;
        level.volume *= volume;
        let id = self.push_voice(spec.channel, at, source, level);
        if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id) {
            voice.sound = Some(sound);
            voice.pitch.snap(clamp_ratio(pitch));
            voice.envelope = spec
                .envelope
                .map(|envelope| EnvState::new(envelope, rate, false));
        }
        Some(id)
    }

    pub fn voices_of(&self, sound: SoundId) -> usize {
        self.voices
            .iter()
            .filter(|voice| voice.sound == Some(sound) && !voice.fading())
            .count()
    }

    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    fn make_room(&mut self, sound: SoundId, spec: &SoundSpec) {
        let clock = self.clock;
        let mut live: Vec<usize> = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, voice)| voice.sound == Some(sound) && !voice.fading())
            .map(|(index, _)| index)
            .collect();
        let mut gone = Vec::new();
        let fade = seconds_to_frames(spec.steal_fade, self.rate);
        while live.len() >= spec.max_voices {
            let pick = match spec.steal {
                Steal::Oldest => live.iter().enumerate().min_by_key(|(_, index)| {
                    let voice = &self.voices[**index];
                    (voice.start, voice.id)
                }),
                Steal::Quietest => live.iter().enumerate().min_by(|(_, a), (_, b)| {
                    let (a, b) = (&self.voices[**a], &self.voices[**b]);
                    a.loudness(clock)
                        .total_cmp(&b.loudness(clock))
                        .then(a.id.cmp(&b.id))
                }),
            };
            let Some((position, _)) = pick else {
                break;
            };
            let index = live.swap_remove(position);
            let voice = &mut self.voices[index];
            if voice.rendered && fade > 0 {
                voice.curve = Some(Curve {
                    start: clock,
                    len: fade,
                    direction: Direction::Out,
                });
            } else {
                gone.push(voice.id);
            }
        }
        if !gone.is_empty() {
            self.voices.retain(|voice| !gone.contains(&voice.id));
        }
    }

    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let len = frames.saturating_mul(self.channels as usize);
        let mut out = vec![0.0; len];
        self.render_into(&mut out);
        out
    }

    pub fn render_into(&mut self, out: &mut [f32]) {
        let out_channels = self.channels as usize;
        let frames = out.len().checked_div(out_channels).unwrap_or(0);
        let body = frames * out_channels;
        out[..body].fill(0.0);
        out[body..].fill(0.0);
        if frames == 0 {
            return;
        }
        let clock = self.clock;
        for bus in self.buses.values_mut() {
            bus.touched = false;
            bus.prepare_pitch(frames);
        }
        self.fill_duck_gains(clock, frames);
        for voice in &mut self.voices {
            let Some(bus) = self.buses.get_mut(&voice.channel) else {
                continue;
            };
            if !bus.touched {
                bus.bus.clear();
                bus.bus.resize(body, 0.0);
                bus.touched = true;
            }
            let block = Block {
                clock,
                frames,
                out_channels,
                rate: self.rate,
                kernel: &self.kernel,
                pitch: bus.pitch_on.then_some(bus.pitch_curve.as_slice()),
            };
            voice.mix_into(&mut bus.bus, &block);
        }
        let duck_target = self.duck.as_ref().map(|state| state.config.target);
        for (id, bus) in self.buses.iter_mut() {
            if !bus.touched {
                bus.gain.skip(frames);
                bus.lowpass.idle(frames);
                bus.highpass.idle(frames);
                continue;
            }
            bus.lowpass
                .process(&mut bus.bus, frames, out_channels, self.rate);
            bus.highpass
                .process(&mut bus.bus, frames, out_channels, self.rate);
            let ducked = duck_target == Some(*id);
            for frame in 0..frames {
                let mut gain = bus.gain.next();
                if ducked {
                    gain *= self.duck_gains[frame];
                }
                let base = frame * out_channels;
                for channel in 0..out_channels {
                    out[base + channel] += bus.bus[base + channel] * gain;
                }
            }
        }
        for frame in 0..frames {
            let gain = self.master.gain.next();
            let base = frame * out_channels;
            for channel in 0..out_channels {
                out[base + channel] *= gain;
            }
        }
        if let Some(threshold) = self.limiter {
            for sample in &mut out[..body] {
                *sample = soft_limit(*sample, threshold);
            }
        }
        for sample in &mut out[..body] {
            *sample = sample.clamp(-1.0, 1.0);
        }
        self.clock = self.clock.saturating_add(frames as u64);
        let clock = self.clock;
        self.voices.retain(|voice| !voice.finished(clock));
    }

    pub(crate) fn render_i16(&mut self, out: &mut [i16]) {
        let channels = self.channels as usize;
        let frames = out.len().checked_div(channels).unwrap_or(0);
        let len = frames.saturating_mul(channels);
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.resize(len, 0.0);
        self.render_into(&mut scratch);
        for (slot, sample) in out.iter_mut().zip(scratch.iter()) {
            *slot = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        }
        for slot in out.iter_mut().skip(scratch.len()) {
            *slot = 0;
        }
        self.scratch = scratch;
    }

    pub(crate) fn stream(&mut self, queue: Arc<PcmQueue>, channels: u16, level: Level) -> VoiceId {
        self.stream_on(ChannelId::EFFECTS, queue, channels, level)
    }

    pub(crate) fn stream_on(
        &mut self,
        channel: ChannelId,
        queue: Arc<PcmQueue>,
        channels: u16,
        level: Level,
    ) -> VoiceId {
        self.push_voice(
            channel,
            self.clock,
            Source::Stream {
                queue,
                channels,
                history: History::new(),
            },
            level,
        )
    }

    #[cfg_attr(not(feature = "vorbis"), allow(dead_code))]
    pub(crate) fn stream_music(
        &mut self,
        queue: Arc<PcmQueue>,
        channels: u16,
        fade_seconds: f32,
        level: Level,
    ) -> VoiceId {
        let id = self.stream_on(ChannelId::MUSIC, queue, channels, level);
        self.begin_music(id, fade_seconds);
        id
    }

    fn begin_music(&mut self, id: VoiceId, fade_seconds: f32) {
        let len = seconds_to_frames(fade_seconds, self.rate);
        let clock = self.clock;
        if let Some(old) = self.music.replace(id) {
            self.stop(old, fade_seconds);
        }
        if len > 0
            && let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == id)
        {
            voice.curve = Some(Curve {
                start: clock,
                len,
                direction: Direction::In,
            });
        }
    }

    fn fill_duck_gains(&mut self, clock: u64, frames: usize) {
        let Some(state) = self.duck.as_mut() else {
            return;
        };
        self.activity.clear();
        self.activity.resize(frames, false);
        let end = clock + frames as u64;
        for voice in self
            .voices
            .iter()
            .filter(|voice| voice.channel == state.config.trigger)
        {
            let (from, to) = voice.audible_span(clock, end);
            let from = from.max(clock);
            let to = to.min(end);
            for at in from..to {
                self.activity[(at - clock) as usize] = true;
            }
        }
        self.duck_gains.clear();
        for frame in 0..frames {
            self.duck_gains
                .push(state.step(self.activity[frame], self.rate));
        }
    }

    fn play_prepared(
        &mut self,
        channel: ChannelId,
        at: u64,
        clip: Clip,
        looping: Option<(u64, u64)>,
        level: Level,
    ) -> VoiceId {
        let frames = clip.frames();
        self.push_voice(
            channel,
            at,
            Source::Buffer {
                samples: Arc::new(clip.samples),
                channels: clip.channels,
                frames,
                looping,
            },
            level,
        )
    }

    fn push_voice(&mut self, channel: ChannelId, at: u64, source: Source, level: Level) -> VoiceId {
        let id = self.alloc();
        self.buses.entry(channel).or_insert_with(Channel::new);
        self.voices.push(Voice::new(id, channel, at, level, source));
        id
    }

    fn alloc(&mut self) -> VoiceId {
        self.next_id = self.next_id.saturating_add(1);
        VoiceId(self.next_id)
    }
}

fn retarget(channel: &mut Channel, rate: u32) {
    let target = channel.effective();
    channel.gain.to(target, ramp_frames(rate));
}

fn seconds_to_frames(seconds: f32, rate: u32) -> u64 {
    if seconds.is_nan() || seconds <= 0.0 {
        return 0;
    }
    (f64::from(seconds) * f64::from(rate)).round() as u64
}

fn glide_frames(seconds: f32, rate: u32) -> u32 {
    u32::try_from(seconds_to_frames(seconds, rate))
        .unwrap_or(u32::MAX)
        .max(1)
}

fn clamp_ratio(ratio: f32) -> f32 {
    if ratio.is_nan() {
        1.0
    } else {
        ratio.clamp(MIN_RATIO, MAX_RATIO)
    }
}

fn soft_limit(sample: f32, threshold: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= threshold {
        return sample;
    }
    let over = (magnitude - threshold) / (1.0 - threshold);
    let shaped = threshold + (1.0 - threshold) * over / (1.0 + over);
    shaped.copysign(sample)
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::tone_level;

    fn bits(samples: &[f32]) -> Vec<u32> {
        samples.iter().map(|sample| sample.to_bits()).collect()
    }

    fn mono(samples: &[f32]) -> Clip {
        Clip {
            rate: 48_000,
            channels: 1,
            samples: samples.to_vec(),
        }
    }

    #[test]
    fn mixer_sums_and_clips() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(&mono(&[0.25]), Level::default());
        mixer.play(&mono(&[0.25]), Level::default());
        assert_eq!(bits(&mixer.render(1)), bits(&[0.5]));

        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(&mono(&[0.75]), Level::default());
        mixer.play(&mono(&[0.5]), Level::default());
        assert_eq!(bits(&mixer.render(1)), bits(&[1.0]));

        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(&mono(&[-0.75]), Level::default());
        mixer.play(&mono(&[-0.5]), Level::default());
        assert_eq!(bits(&mixer.render(1)), bits(&[-1.0]));

        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(
            &mono(&[1.0]),
            Level {
                volume: 2.0,
                ..Level::default()
            },
        );
        assert_eq!(bits(&mixer.render(1)), bits(&[1.0]));

        let mut mixer = Mixer::new(48_000, 2);
        mixer.play(
            &mono(&[1.0]),
            Level {
                pan: -1.0,
                ..Level::default()
            },
        );
        mixer.play(
            &mono(&[1.0]),
            Level {
                pan: 1.0,
                ..Level::default()
            },
        );
        assert_eq!(bits(&mixer.render(1)), bits(&[1.0, 1.0]));

        let mut mixer = Mixer::new(48_000, 2);
        mixer.play(&mono(&[1.0]), Level::default());
        mixer.play(&mono(&[1.0]), Level::default());
        assert_eq!(bits(&mixer.render(1)), bits(&[1.0, 1.0]));

        let mut mixer = Mixer::new(48_000, 1);
        let id = mixer.play(&mono(&[0.5, 0.5]), Level::default());
        assert_eq!(bits(&mixer.render(1)), bits(&[0.5]));
        mixer.set_volume(id, 0.0);
        mixer.set_pan(id, -1.0);
        assert_eq!(bits(&mixer.render(1)), bits(&[0.0]));
        assert_eq!(bits(&mixer.render(1)), bits(&[0.0]));
    }

    #[test]
    fn fades_ramp_at_the_start_and_the_end() {
        let clip = mono(&[1.0; 8]);
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(
            &clip,
            Level {
                fade_in: 4,
                ..Level::default()
            },
        );
        assert_eq!(
            bits(&mixer.render(8)),
            bits(&[0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0])
        );

        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(
            &clip,
            Level {
                fade_out: 4,
                ..Level::default()
            },
        );
        assert_eq!(
            bits(&mixer.render(8)),
            bits(&[1.0, 1.0, 1.0, 1.0, 1.0, 0.75, 0.5, 0.25])
        );

        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(
            &mono(&[1.0; 4]),
            Level {
                fade_in: 4,
                fade_out: 4,
                ..Level::default()
            },
        );
        assert_eq!(bits(&mixer.render(4)), bits(&[0.0, 0.1875, 0.25, 0.1875]));
    }

    #[test]
    fn matching_rates_play_the_samples_unchanged() {
        let samples = vec![0.0, 1.0, 0.0, -1.0];
        let clip = Clip {
            rate: 16_000,
            channels: 1,
            samples: samples.clone(),
        };
        let mut mixer = Mixer::new(16_000, 1);
        mixer.play(&clip, Level::default());
        assert_eq!(bits(&mixer.render(4)), bits(&samples));

        let stereo = vec![0.1, -0.2, 0.3, -0.4, 0.5, -0.6];
        let clip = Clip {
            rate: 16_000,
            channels: 2,
            samples: stereo.clone(),
        };
        let mut mixer = Mixer::new(16_000, 2);
        mixer.play(&clip, Level::default());
        assert_eq!(bits(&mixer.render(3)), bits(&stereo));
    }

    #[test]
    fn a_stream_plays_queued_samples_and_fades_once_it_ends() {
        let queue = PcmQueue::new();
        queue.push(&[0.25, -0.25, 0.5, -0.5]);
        let mut mixer = Mixer::new(48_000, 2);
        mixer.stream(Arc::clone(&queue), 2, Level::default());
        assert_eq!(bits(&mixer.render(2)), bits(&[0.25, -0.25, 0.5, -0.5]));
        assert_eq!(bits(&mixer.render(1)), bits(&[0.0, 0.0]));

        let queue = PcmQueue::new();
        queue.push(&[1.0, 1.0, 1.0, 1.0]);
        let mut mixer = Mixer::new(48_000, 1);
        mixer.stream(
            Arc::clone(&queue),
            1,
            Level {
                fade_out: 2,
                ..Level::default()
            },
        );
        assert_eq!(bits(&mixer.render(4)), bits(&[1.0, 1.0, 1.0, 1.0]));

        let queue = PcmQueue::new();
        queue.push(&[1.0, 1.0, 1.0, 1.0]);
        queue.end();
        let mut mixer = Mixer::new(48_000, 1);
        mixer.stream(
            queue,
            1,
            Level {
                fade_out: 2,
                ..Level::default()
            },
        );
        assert_eq!(bits(&mixer.render(2)), bits(&[1.0, 1.0]));
        assert_eq!(bits(&mixer.render(2)), bits(&[1.0, 0.5]));
    }

    #[test]
    fn mixdown_bytes_are_the_sample_exact_sum() {
        let mut timeline = Timeline::new(48_000, 1);
        let clip = mono(&[0.5, 0.5, 0.5, 0.5]);
        timeline.place(0, clip.clone(), Level::default());
        timeline.place(2, clip, Level::default());
        let once = mixdown(&timeline);
        let twice = mixdown(&timeline);
        let expect = [0.5, 0.5, 1.0, 1.0, 0.5, 0.5];
        assert_eq!(bits(&once), bits(&expect));
        assert_eq!(bits(&once), bits(&twice));
        let wav_once = Clip {
            rate: 48_000,
            channels: 1,
            samples: once,
        }
        .to_wav();
        let wav_twice = Clip {
            rate: 48_000,
            channels: 1,
            samples: twice,
        }
        .to_wav();
        assert_eq!(wav_once, wav_twice);
    }

    fn constant(value: f32, frames: usize) -> Clip {
        mono(&vec![value; frames])
    }

    fn slow(value: f32, frames: usize) -> Clip {
        Clip {
            rate: 1000,
            channels: 1,
            samples: vec![value; frames],
        }
    }

    fn sine(period: f32, amplitude: f32, frames: usize) -> Clip {
        mono(
            &(0..frames)
                .map(|i| amplitude * (std::f32::consts::TAU * i as f32 / period).sin())
                .collect::<Vec<_>>(),
        )
    }

    fn power(samples: &[f32]) -> f32 {
        samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32
    }

    fn decibels(ratio: f32) -> f32 {
        10.0 * ratio.log10()
    }

    #[test]
    fn today_s_calls_play_on_the_effects_channel() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(&constant(0.5, 2000), Level::default());
        mixer.set_channel_volume(ChannelId::EFFECTS, 0.0);
        mixer.render(600);
        assert_eq!(mixer.render(1)[0], 0.0);

        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(&constant(0.5, 2000), Level::default());
        mixer.set_channel_volume(ChannelId::MUSIC, 0.0);
        mixer.set_channel_volume(ChannelId::DIALOG, 0.0);
        mixer.render(600);
        assert_eq!(mixer.render(1)[0], 0.5);
    }

    #[test]
    fn channel_volumes_master_and_voice_level_multiply() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.set_master_volume(0.5);
        mixer.set_channel_volume(ChannelId::MUSIC, 0.5);
        mixer.set_channel_volume(ChannelId::EFFECTS, 0.25);
        mixer.set_channel_volume(ChannelId::DIALOG, 1.0);
        let half = Level {
            volume: 0.5,
            ..Level::default()
        };
        mixer.play_on(ChannelId::MUSIC, &constant(0.8, 2000), half);
        mixer.play_on(ChannelId::EFFECTS, &constant(0.8, 2000), Level::default());
        mixer.play_on(ChannelId::DIALOG, &constant(0.2, 2000), Level::default());
        mixer.render(700);
        let settled = mixer.render(1)[0];
        let expect = 0.5 * (0.8 * 0.5 * 0.5 + 0.8 * 0.25 + 0.2);
        assert!((settled - expect).abs() < 1e-6, "{settled} vs {expect}");
        assert_eq!(mixer.master_gain(), 0.5);
        assert_eq!(mixer.channel_gain(ChannelId::MUSIC), 0.5);
    }

    #[test]
    fn a_custom_channel_has_its_own_volume() {
        let ui = ChannelId::custom(0);
        let mut mixer = Mixer::new(48_000, 1);
        mixer.set_channel_volume(ui, 0.25);
        mixer.play_on(ui, &constant(1.0, 10), Level::default());
        mixer.play(&constant(0.5, 10), Level::default());
        assert_eq!(mixer.render(1)[0], 0.75);
        assert_eq!(mixer.channel_volume(ui), 0.25);
        assert_eq!(mixer.channel_volume(ChannelId::custom(1)), 1.0);
    }

    #[test]
    fn muting_a_channel_silences_only_that_channel() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play_on(ChannelId::MUSIC, &constant(0.25, 4000), Level::default());
        mixer.play_on(ChannelId::EFFECTS, &constant(0.5, 4000), Level::default());
        mixer.set_channel_muted(ChannelId::MUSIC, true);
        assert!(mixer.channel_muted(ChannelId::MUSIC));
        assert_eq!(mixer.channel_volume(ChannelId::MUSIC), 1.0);
        mixer.render(600);
        assert_eq!(mixer.render(1)[0], 0.5);
        mixer.set_channel_muted(ChannelId::MUSIC, false);
        mixer.render(600);
        assert_eq!(mixer.render(1)[0], 0.75);
        mixer.set_master_muted(true);
        mixer.render(600);
        assert_eq!(mixer.render(1)[0], 0.0);
        assert!(mixer.master_muted());
    }

    #[test]
    fn a_volume_change_ramps_in_steps_of_a_few_thousandths() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play(&constant(1.0, 4000), Level::default());
        mixer.render(10);
        mixer.set_channel_volume(ChannelId::EFFECTS, 0.0);
        let down = mixer.render(600);
        mixer.set_channel_volume(ChannelId::EFFECTS, 1.0);
        let up = mixer.render(600);
        mixer.set_master_volume(0.0);
        let master = mixer.render(600);
        for run in [&down, &up, &master] {
            let largest = run
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0, f32::max);
            assert!(largest < 0.003, "step {largest}");
        }
        assert_eq!(*down.last().unwrap(), 0.0);
        assert_eq!(*up.last().unwrap(), 1.0);
        assert!(down[0] > 0.0 && down[0] < 1.0);
        assert_eq!(*master.last().unwrap(), 0.0);
    }

    #[test]
    fn a_loop_seam_is_as_smooth_as_the_tone_itself() {
        let tone = sine(100.0, 0.5, 1000);
        let track = MusicTrack::with_loop(tone.clone(), 200, 800);
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play_music(&track, 0.0, Level::default());
        let out = mixer.render(3000);
        assert_eq!(bits(&out[..800]), bits(&tone.samples[..800]));
        assert_eq!(out[800].to_bits(), tone.samples[200].to_bits());
        assert_eq!(out[1400].to_bits(), tone.samples[200].to_bits());
        let intrinsic = 0.5 * std::f32::consts::TAU / 100.0;
        let largest = out
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max);
        assert!(largest <= intrinsic * 1.001, "{largest} vs {intrinsic}");
        assert!(mixer.music().is_some());
    }

    #[test]
    fn a_track_without_a_loop_ends() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.play_music(&MusicTrack::once(constant(0.5, 4)), 0.0, Level::default());
        assert_eq!(
            bits(&mixer.render(6)),
            bits(&[0.5, 0.5, 0.5, 0.5, 0.0, 0.0])
        );
        assert!(mixer.music().is_none());
    }

    #[test]
    fn a_crossfade_keeps_the_music_within_half_a_decibel() {
        let first = MusicTrack::looping(sine(100.0, 0.5, 4800));
        let second = MusicTrack::looping(sine(80.0, 0.5, 4800));
        let mut mixer = Mixer::new(48_000, 1);
        let old = mixer.play_music(&first, 0.0, Level::default());
        let before = mixer.render(4800);
        let reference = power(&before);
        let new = mixer.play_music(&second, 0.1, Level::default());
        assert_ne!(old, new);
        assert_eq!(mixer.music(), Some(new));
        let fade = mixer.render(4800);
        for window in fade.chunks(1200) {
            let drift = decibels(power(window) / reference);
            assert!(drift.abs() < 0.5, "{drift} dB");
        }
        assert_eq!(mixer.voices.len(), 1);
        let after = mixer.render(4800);
        assert!((decibels(power(&after) / reference)).abs() < 0.5);
    }

    #[test]
    fn stopping_with_a_fade_ends_the_voice_at_silence() {
        let mut mixer = Mixer::new(1000, 1);
        let id = mixer.play(&slow(1.0, 5000), Level::default());
        mixer.render(10);
        mixer.stop(id, 0.1);
        let out = mixer.render(100);
        assert!(out[0] > 0.99 && out[99] < 0.05);
        assert!(mixer.voices.is_empty());
        let id = mixer.play(&slow(1.0, 5000), Level::default());
        mixer.stop(id, 0.0);
        assert_eq!(mixer.render(1)[0], 0.0);
    }

    fn ducked_mixer() -> Mixer {
        let mut mixer = Mixer::new(1000, 1);
        mixer.play_music(&MusicTrack::looping(slow(0.5, 100)), 0.0, Level::default());
        mixer.play_at_on(ChannelId::DIALOG, 200, &slow(0.0, 500), Level::default());
        mixer
    }

    #[test]
    fn ducking_is_off_until_a_game_sets_it() {
        let mut mixer = ducked_mixer();
        assert_eq!(mixer.ducking(), None);
        let out = mixer.render(900);
        assert!(out.iter().all(|sample| *sample == 0.5));
        assert_eq!(mixer.ducked_db(), 0.0);
    }

    #[test]
    fn dialog_ducks_the_music_with_attack_and_release_timing() {
        let ducking = Ducking::dialog_over_music(12.0, 0.1, 0.2);
        let mut mixer = ducked_mixer();
        mixer.set_ducking(Some(ducking));
        assert_eq!(mixer.ducking(), Some(ducking));
        let out = mixer.render(1000);
        let full = 0.5 * 10f32.powf(-12.0 / 20.0);
        let half = 0.5 * 10f32.powf(-6.0 / 20.0);
        assert_eq!(out[199], 0.5);
        assert!(out[200] < 0.5 && out[200] > half);
        assert!((out[249] - half).abs() < 1e-4, "{}", out[249]);
        assert!((out[299] - full).abs() < 1e-4, "{}", out[299]);
        assert!((out[500] - full).abs() < 1e-4);
        assert!((out[699] - full).abs() < 1e-4);
        assert!((out[799] - half).abs() < 1e-4, "{}", out[799]);
        assert!((out[899] - 0.5).abs() < 1e-4, "{}", out[899]);
        assert_eq!(out[999], 0.5);
        let monotone = out[200..300].windows(2).all(|pair| pair[1] < pair[0]);
        assert!(monotone);

        let mut blocks = ducked_mixer();
        blocks.set_ducking(Some(ducking));
        let mut pieces = Vec::new();
        for size in [37, 1, 150, 12, 800] {
            pieces.extend(blocks.render(size));
        }
        assert_eq!(bits(&pieces), bits(&out));
    }

    #[test]
    fn ducking_only_dips_its_target_channel() {
        let mut mixer = ducked_mixer();
        mixer.set_ducking(Some(Ducking::dialog_over_music(12.0, 0.1, 0.2)));
        mixer.play(&slow(0.25, 1000), Level::default());
        let out = mixer.render(500);
        let full = 0.5 * 10f32.powf(-12.0 / 20.0);
        assert!((out[400] - (full + 0.25)).abs() < 1e-4);
    }

    fn looped_sine(mixer: &mut Mixer, hz: u32, amplitude: f32) -> VoiceId {
        let rate = mixer.rate();
        let clip = Clip {
            rate,
            channels: 1,
            samples: (0..rate)
                .map(|i| {
                    amplitude
                        * (std::f64::consts::TAU * f64::from(hz) * f64::from(i) / f64::from(rate))
                            .sin() as f32
                })
                .collect(),
        };
        mixer.play_music(&MusicTrack::looping(clip), 0.0, Level::default())
    }

    fn largest_step(samples: &[f32]) -> f32 {
        samples
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max)
    }

    fn up_crossings(samples: &[f32], rate: f32) -> f32 {
        let mut times = Vec::new();
        for (i, pair) in samples.windows(2).enumerate() {
            if pair[0] < 0.0 && pair[1] >= 0.0 {
                times.push(i as f32 + pair[0].abs() / (pair[1] - pair[0]));
            }
        }
        let span = times[times.len() - 1] - times[0];
        (times.len() - 1) as f32 * rate / span
    }

    #[test]
    fn a_voice_pitch_scales_its_frequency_and_the_channel_pitch_scales_all() {
        let mut mixer = Mixer::new(48_000, 1);
        let id = looped_sine(&mut mixer, 1000, 0.5);
        mixer.set_pitch(id, 1.5);
        mixer.render(2000);
        let out = mixer.render(8192);
        assert!(tone_level(&out, 48_000.0, 1500.0) > 0.49);
        assert!(tone_level(&out, 48_000.0, 1000.0) < 0.005);
        assert_eq!(mixer.pitch(id), Some(1.5));

        let mut mixer = Mixer::new(48_000, 1);
        looped_sine(&mut mixer, 1000, 0.5);
        mixer.set_channel_pitch(ChannelId::MUSIC, 0.5);
        mixer.render(2000);
        let out = mixer.render(8192);
        assert!(tone_level(&out, 48_000.0, 500.0) > 0.49);
        assert_eq!(mixer.channel_pitch(ChannelId::MUSIC), 0.5);
        assert_eq!(mixer.channel_pitch(ChannelId::EFFECTS), 1.0);
    }

    #[test]
    fn pitch_stays_inside_its_limits() {
        let mut mixer = Mixer::new(48_000, 1);
        let id = looped_sine(&mut mixer, 100, 0.5);
        mixer.set_pitch(id, 1000.0);
        mixer.render(1000);
        assert_eq!(mixer.pitch(id), Some(8.0));
        mixer.set_pitch(id, f32::NAN);
        mixer.render(1000);
        assert_eq!(mixer.pitch(id), Some(1.0));
    }

    #[test]
    fn interpolated_playback_matches_the_tone_at_a_fractional_rate() {
        let mut mixer = Mixer::new(48_000, 1);
        let id = looped_sine(&mut mixer, 480, 0.5);
        mixer.set_pitch(id, 1.25);
        mixer.render(2000);
        let out = mixer.render(4800);
        let worst = out
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                let t = (2000 + i) as f64 + 1.0;
                let _ = t;
                sample.abs()
            })
            .fold(0.0, f32::max);
        assert!((worst - 0.5).abs() < 0.01, "{worst}");
        assert!(tone_level(&out, 48_000.0, 600.0) > 0.495);
    }

    #[test]
    fn pitch_rate_cutoff_and_volume_ramps_never_click() {
        let intrinsic = |hz: f32| 0.5 * std::f32::consts::TAU * hz / 48_000.0;

        let mut mixer = Mixer::new(48_000, 1);
        let id = looped_sine(&mut mixer, 440, 0.5);
        mixer.render(500);
        mixer.glide_pitch(id, 4.0, 0.3);
        let up = mixer.render(20_000);
        mixer.glide_pitch(id, 0.25, 0.3);
        let down = mixer.render(20_000);
        assert!(
            largest_step(&up) <= intrinsic(1760.0) * 1.05,
            "{}",
            largest_step(&up)
        );
        assert!(
            largest_step(&down) <= intrinsic(1760.0) * 1.05,
            "{}",
            largest_step(&down)
        );

        let mut mixer = Mixer::new(48_000, 1);
        looped_sine(&mut mixer, 440, 0.5);
        mixer.render(500);
        mixer.glide_channel_pitch(ChannelId::MUSIC, 3.0, 0.2);
        let swept = mixer.render(20_000);
        assert!(largest_step(&swept) <= intrinsic(1320.0) * 1.05);
        mixer.set_channel_pitch(ChannelId::MUSIC, 1.0);
        let jumped = mixer.render(2000);
        assert!(largest_step(&jumped) <= intrinsic(1320.0) * 1.05);

        let mut mixer = Mixer::new(48_000, 1);
        looped_sine(&mut mixer, 440, 0.5);
        mixer.render(500);
        mixer.set_channel_lowpass(ChannelId::MUSIC, Some(300.0));
        let closing = mixer.render(4000);
        mixer.set_channel_highpass(ChannelId::MUSIC, Some(2000.0));
        let high = mixer.render(4000);
        mixer.set_channel_lowpass(ChannelId::MUSIC, None);
        mixer.set_channel_highpass(ChannelId::MUSIC, None);
        let opening = mixer.render(4000);
        for run in [&closing, &high, &opening] {
            eprintln!(
                "filter run step {} vs {}",
                largest_step(run),
                intrinsic(440.0)
            );
            assert!(
                largest_step(run) <= intrinsic(440.0) * 1.5,
                "{}",
                largest_step(run)
            );
        }

        let mut mixer = Mixer::new(48_000, 1);
        let id = looped_sine(&mut mixer, 440, 0.5);
        mixer.render(500);
        mixer.glide_gain(id, 0.0, 0.05);
        let fade = mixer.render(4000);
        mixer.glide_gain(id, 1.0, 0.05);
        let back = mixer.render(4000);
        assert!(largest_step(&fade) <= intrinsic(440.0) * 1.1);
        assert!(largest_step(&back) <= intrinsic(440.0) * 1.1);
        assert_eq!(*fade.last().unwrap(), 0.0);
    }

    #[test]
    fn aliasing_after_a_speed_up_is_far_below_the_tone() {
        let rate = 48_000.0;
        let cases = [
            (2.0, 17_000u32, 14_000.0f32),
            (2.0, 21_000, 6_000.0),
            (4.0, 11_000, 4_000.0),
            (4.0, 21_000, 12_000.0),
        ];
        for (ratio, source, alias) in cases {
            let mut mixer = Mixer::new(48_000, 1);
            let id = looped_sine(&mut mixer, source, 0.5);
            mixer.set_pitch(id, ratio);
            mixer.render(3000);
            let out = mixer.render(16_384);
            let level = tone_level(&out, rate, alias) / 0.5;
            let db = 20.0 * level.max(1e-9).log10();
            eprintln!("alias {ratio}x {source} Hz -> {alias} Hz: {db:.1} dB");
            assert!(db < -60.0, "{ratio}x {source} Hz: {db} dB");
        }
        for ratio in [2.0f32, 4.0] {
            let source = (3000.0 / ratio) as u32;
            let mut mixer = Mixer::new(48_000, 1);
            let id = looped_sine(&mut mixer, source, 0.5);
            mixer.set_pitch(id, ratio);
            mixer.render(3000);
            let out = mixer.render(16_384);
            let gain = tone_level(&out, rate, source as f32 * ratio) / 0.5;
            let db = 20.0 * gain.log10();
            eprintln!("passband {ratio}x: {db:.3} dB");
            assert!(db.abs() < 0.1, "{db}");
        }
    }

    fn hits() -> Clip {
        Clip {
            rate: 48_000,
            channels: 1,
            samples: (0..2400)
                .map(|i| {
                    let decay = 1.0 - i as f32 / 2400.0;
                    0.5 * decay * (std::f32::consts::TAU * 600.0 * i as f32 / 48_000.0).sin()
                })
                .collect(),
        }
    }

    fn burst(seed: u64, steal: Steal, limiter: Option<f32>) -> (Vec<f32>, usize) {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.set_seed(seed);
        mixer.set_limiter(limiter);
        let sound = mixer.sound(
            &hits(),
            SoundSpec {
                max_voices: 12,
                steal,
                pitch_jitter: 3.0,
                volume_jitter: 0.3,
                ..SoundSpec::default()
            },
        );
        let mut out = Vec::new();
        let mut worst = 0;
        for _ in 0..48 {
            for _ in 0..209 {
                mixer.trigger(sound);
                assert!(mixer.voices_of(sound) <= 12);
            }
            worst = worst.max(mixer.voice_count());
            out.extend(mixer.render(1000));
        }
        (out, worst)
    }

    #[test]
    fn ten_thousand_hits_in_a_second_play_a_capped_varied_set() {
        let (hard, worst) = burst(1, Steal::Oldest, None);
        assert!(worst <= 24, "{worst} voices");
        assert!(hard.iter().any(|sample| sample.abs() >= 1.0));
        let (soft, _) = burst(1, Steal::Oldest, Some(0.8));
        assert!(soft.iter().all(|sample| sample.abs() < 1.0));
        assert!(soft.iter().any(|sample| sample.abs() > 0.8));
        let (quiet, _) = burst(1, Steal::Quietest, Some(0.8));
        assert!(quiet.iter().all(|sample| sample.abs() < 1.0));
        assert_ne!(bits(&soft), bits(&quiet));
    }

    #[test]
    fn the_cap_and_the_jitter_repeat_from_the_same_seed() {
        let (a, _) = burst(42, Steal::Oldest, Some(0.8));
        let (b, _) = burst(42, Steal::Oldest, Some(0.8));
        let (c, _) = burst(43, Steal::Oldest, Some(0.8));
        assert_eq!(bits(&a), bits(&b));
        assert_ne!(bits(&a), bits(&c));

        let mut mixer = Mixer::new(48_000, 1);
        let sound = mixer.sound(
            &hits(),
            SoundSpec {
                pitch_jitter: 2.0,
                volume_jitter: 0.5,
                ..SoundSpec::default()
            },
        );
        let ids: Vec<VoiceId> = (0..50).filter_map(|_| mixer.trigger(sound)).collect();
        let pitches: Vec<f32> = ids.iter().filter_map(|id| mixer.pitch(*id)).collect();
        let low = 2f32.powf(-2.0 / 12.0);
        let high = 2f32.powf(2.0 / 12.0);
        assert!(pitches.iter().all(|p| *p >= low && *p <= high));
        let spread = pitches.iter().cloned().fold(0.0, f32::max)
            - pitches.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.05, "{spread}");
        assert_eq!(mixer.voices_of(sound), 50);
    }

    #[test]
    fn a_full_sound_steals_its_oldest_or_its_quietest_voice() {
        for (steal, loud_first) in [(Steal::Oldest, true), (Steal::Quietest, true)] {
            let mut mixer = Mixer::new(48_000, 1);
            let sound = mixer.sound(
                &hits(),
                SoundSpec {
                    max_voices: 2,
                    steal,
                    ..SoundSpec::default()
                },
            );
            let first = mixer.trigger(sound).unwrap();
            let second = mixer.trigger(sound).unwrap();
            let (loud, soft) = if loud_first {
                (first, second)
            } else {
                (second, first)
            };
            mixer.set_volume(loud, 0.9);
            mixer.set_volume(soft, 0.2);
            mixer.render(10);
            let third = mixer.trigger(sound).unwrap();
            assert_eq!(mixer.voices_of(sound), 2);
            let victim = if steal == Steal::Oldest { first } else { soft };
            let fading: Vec<VoiceId> = mixer
                .voices
                .iter()
                .filter(|voice| voice.fading())
                .map(|voice| voice.id)
                .collect();
            assert_eq!(fading, vec![victim], "{steal:?}");
            assert!(mixer.voices.iter().any(|voice| voice.id == third));
            mixer.render(2000);
            assert!(mixer.voices.iter().all(|voice| voice.id != victim));
        }
        let mut mixer = Mixer::new(48_000, 1);
        let silent = mixer.sound(
            &hits(),
            SoundSpec {
                max_voices: 0,
                ..SoundSpec::default()
            },
        );
        assert!(mixer.trigger(silent).is_none());
    }

    #[test]
    fn the_soft_limiter_is_off_by_default_and_never_reaches_full_scale() {
        let mut mixer = Mixer::new(48_000, 1);
        assert_eq!(mixer.limiter(), None);
        mixer.play(&constant(0.9, 4), Level::default());
        mixer.play(&constant(0.9, 4), Level::default());
        assert_eq!(bits(&mixer.render(4)), bits(&[1.0; 4]));
        mixer.set_limiter(Some(0.8));
        mixer.play(&constant(0.5, 4), Level::default());
        assert_eq!(mixer.render(1)[0], 0.5);
        mixer.render(3);
        mixer.play(&constant(0.9, 4), Level::default());
        mixer.play(&constant(0.9, 4), Level::default());
        mixer.play(&constant(-3.0, 4), Level::default());
        let out = mixer.render(1)[0];
        assert!(out < -0.8 && out > -1.0, "{out}");
        assert!(soft_limit(5.0, 0.8) < 1.0 && soft_limit(5.0, 0.8) > 0.95);
        assert_eq!(soft_limit(-0.3, 0.8), -0.3);
        let knee = soft_limit(0.8001, 0.8) - soft_limit(0.7999, 0.8);
        assert!((knee - 0.0002).abs() < 1e-4);
    }

    fn tone_of(wave: Wave, hz: f32) -> f32 {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.tone(ChannelId::EFFECTS, wave, hz, 0.5);
        let out = mixer.render(48_000);
        up_crossings(&out, 48_000.0)
    }

    #[test]
    fn a_tone_voice_holds_its_frequency_within_a_tenth_of_a_percent() {
        for wave in [Wave::Sine, Wave::Square, Wave::Saw] {
            for hz in [110.0f32, 440.0, 1234.5, 4000.0] {
                let measured = tone_of(wave, hz);
                let error = (measured - hz).abs() / hz;
                assert!(error < 0.001, "{wave:?} {hz} Hz measured {measured}");
            }
        }
        let mut mixer = Mixer::new(48_000, 1);
        let id = mixer.tone(ChannelId::EFFECTS, Wave::Saw, 300.0, 0.5);
        mixer.set_pitch(id, 2.0);
        mixer.render(2000);
        let out = mixer.render(48_000);
        let measured = up_crossings(&out, 48_000.0);
        assert!((measured - 600.0).abs() / 600.0 < 0.001, "{measured}");
    }

    #[test]
    fn a_tone_is_band_limited_and_bounded() {
        let mut mixer = Mixer::new(48_000, 1);
        mixer.tone(ChannelId::EFFECTS, Wave::Saw, 2000.0, 1.0);
        let out = mixer.render(16_384);
        assert!(out.iter().all(|sample| sample.abs() <= 1.0));
        let alias = tone_level(&out, 48_000.0, 20_000.0);
        let partial = tone_level(&out, 48_000.0, 2000.0);
        assert!(alias < 0.1 * partial, "{alias} vs {partial}");
        assert!(
            (partial - 2.0 / std::f32::consts::PI).abs() < 0.05,
            "{partial}"
        );

        let mut mixer = Mixer::new(48_000, 1);
        mixer.tone(ChannelId::EFFECTS, Wave::Sine, 1000.0, 0.5);
        let out = mixer.render(8192);
        assert!((tone_level(&out, 48_000.0, 1000.0) - 0.5).abs() < 0.005);
    }

    #[test]
    fn a_driven_tone_glides_and_fades_without_clicks() {
        let mut mixer = Mixer::new(48_000, 1);
        let id = mixer.tone(ChannelId::EFFECTS, Wave::Sine, 200.0, 1.0);
        mixer.render(1000);
        mixer.glide_tone_frequency(id, 2000.0, 0.5);
        let rising = mixer.render(30_000);
        assert!(largest_step(&rising) <= std::f32::consts::TAU * 2000.0 / 48_000.0 * 1.01);
        let settled = mixer.render(8192);
        assert!(tone_level(&settled, 48_000.0, 2000.0) > 0.99);
        mixer.glide_gain(id, 0.0, 0.05);
        let fade = mixer.render(4000);
        assert!(largest_step(&fade) <= std::f32::consts::TAU * 2000.0 / 48_000.0 * 1.01);
        assert_eq!(*fade.last().unwrap(), 0.0);
        mixer.stop(id, 0.01);
        mixer.render(1000);
        assert_eq!(mixer.voice_count(), 0);
    }

    fn filtered_gain(low: bool, cutoff: f32) -> f32 {
        let mut mixer = Mixer::new(48_000, 1);
        looped_sine(&mut mixer, cutoff as u32, 0.5);
        if low {
            mixer.set_channel_lowpass(ChannelId::MUSIC, Some(cutoff));
        } else {
            mixer.set_channel_highpass(ChannelId::MUSIC, Some(cutoff));
        }
        mixer.render(8000);
        let out = mixer.render(9600);
        tone_level(&out, 48_000.0, cutoff) / 0.5
    }

    #[test]
    fn the_filters_are_three_decibels_down_at_their_cutoff() {
        for cutoff in [200.0f32, 1000.0, 5000.0] {
            for low in [true, false] {
                let gain = filtered_gain(low, cutoff);
                let db = 20.0 * gain.log10();
                assert!((db + 3.01).abs() < 0.5, "low {low} {cutoff} Hz: {db} dB");
            }
        }
    }

    #[test]
    fn the_filters_pass_and_stop_either_side_of_the_cutoff() {
        let response = |low: bool, cutoff: f32, tone: u32| {
            let mut mixer = Mixer::new(48_000, 1);
            looped_sine(&mut mixer, tone, 0.5);
            if low {
                mixer.set_channel_lowpass(ChannelId::MUSIC, Some(cutoff));
            } else {
                mixer.set_channel_highpass(ChannelId::MUSIC, Some(cutoff));
            }
            mixer.render(8000);
            let out = mixer.render(9600);
            tone_level(&out, 48_000.0, tone as f32) / 0.5
        };
        assert!(response(true, 2000.0, 200) > 0.99);
        assert!(response(true, 2000.0, 8000) < 0.1);
        assert!(response(false, 2000.0, 8000) > 0.99);
        assert!(response(false, 2000.0, 200) < 0.1);

        let mut mixer = Mixer::new(48_000, 1);
        assert_eq!(mixer.channel_lowpass(ChannelId::MUSIC), None);
        mixer.set_channel_lowpass(ChannelId::MUSIC, Some(800.0));
        mixer.set_channel_highpass(ChannelId::MUSIC, Some(100.0));
        assert_eq!(mixer.channel_lowpass(ChannelId::MUSIC), Some(800.0));
        assert_eq!(mixer.channel_highpass(ChannelId::MUSIC), Some(100.0));
        mixer.set_channel_lowpass(ChannelId::MUSIC, None);
        assert_eq!(mixer.channel_lowpass(ChannelId::MUSIC), None);
    }

    #[test]
    fn a_closed_filter_leaves_the_audio_bit_for_bit_alone() {
        let plain = {
            let mut mixer = Mixer::new(48_000, 1);
            looped_sine(&mut mixer, 440, 0.5);
            mixer.render(3000)
        };
        let mut mixer = Mixer::new(48_000, 1);
        looped_sine(&mut mixer, 440, 0.5);
        mixer.set_channel_lowpass(ChannelId::MUSIC, Some(300.0));
        mixer.render(10);
        mixer.set_channel_lowpass(ChannelId::MUSIC, None);
        mixer.render(2000);
        let after = mixer.render(1000);
        let clean = {
            let mut mixer = Mixer::new(48_000, 1);
            looped_sine(&mut mixer, 440, 0.5);
            mixer.render(2010);
            mixer.render(1000)
        };
        assert_eq!(bits(&after), bits(&clean));
        assert_eq!(plain.len(), 3000);
    }

    #[test]
    fn a_stream_plays_faster_when_its_pitch_rises() {
        let queue = PcmQueue::new();
        queue.push(&vec![0.5; 400]);
        queue.end();
        let mut mixer = Mixer::new(48_000, 1);
        let id = mixer.stream(Arc::clone(&queue), 1, Level::default());
        mixer.set_pitch(id, 2.0);
        mixer.render(480);
        let out = mixer.render(400);
        let heard = out.iter().filter(|sample| sample.abs() > 0.01).count();
        assert!(heard < 40, "{heard}");
        assert_eq!(mixer.voice_count(), 0);

        let queue = PcmQueue::new();
        queue.push(&vec![0.5; 4000]);
        queue.end();
        let mut mixer = Mixer::new(48_000, 1);
        let id = mixer.stream(queue, 1, Level::default());
        mixer.set_pitch(id, 0.5);
        mixer.render(480);
        let out = mixer.render(1000);
        assert!(out.iter().all(|sample| (sample - 0.5).abs() < 0.01));
    }
}
