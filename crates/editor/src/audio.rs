use pfx_play::PlaySession;
use pfx_sound::{ChannelId, Level, Output, PcmSender};

use crate::log::Log;

pub const AHEAD_SECONDS: f32 = 0.1;
pub const PRIME_SECONDS: f32 = 0.05;
pub const DEFAULT_RATE: u32 = 48_000;
pub const DEFAULT_CHANNELS: u16 = 2;

pub trait Stream {
    fn push(&mut self, samples: &[f32]) -> usize;
    fn queued_frames(&self) -> usize;
    fn finish(&mut self);
}

pub trait Sink {
    fn rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn stream(&mut self, seconds: f32) -> Box<dyn Stream>;
    fn set_muted(&mut self, muted: bool);
}

pub type Opener = Box<dyn FnMut() -> Result<Box<dyn Sink>, String>>;

struct Device {
    output: Output,
}

struct Pcm {
    sender: PcmSender,
}

impl Stream for Pcm {
    fn push(&mut self, samples: &[f32]) -> usize {
        self.sender.push(samples)
    }

    fn queued_frames(&self) -> usize {
        self.sender.queued_frames()
    }

    fn finish(&mut self) {
        self.sender.finish();
    }
}

impl Sink for Device {
    fn rate(&self) -> u32 {
        self.output.rate()
    }

    fn channels(&self) -> u16 {
        self.output.channels()
    }

    fn stream(&mut self, seconds: f32) -> Box<dyn Stream> {
        let (_, sender) = self.output.pcm_stream(
            ChannelId::EFFECTS,
            self.output.channels(),
            seconds,
            Level::default(),
        );
        Box::new(Pcm { sender })
    }

    fn set_muted(&mut self, muted: bool) {
        self.output.set_master_muted(muted);
    }
}

pub fn open_device() -> Result<Box<dyn Sink>, String> {
    Ok(Box::new(Device {
        output: Output::open()?,
    }))
}

enum Source {
    Off,
    Unopened(Opener),
    Ready(Box<dyn Sink>),
    Missing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub dropped_frames: u64,
    pub underruns: u32,
}

pub struct Audio {
    source: Source,
    stream: Option<Box<dyn Stream>>,
    muted: bool,
    pending: Vec<f32>,
    primed: bool,
    pub stats: Stats,
}

impl Audio {
    pub fn off() -> Audio {
        Audio::from(Source::Off)
    }

    pub fn device() -> Audio {
        Audio::with_opener(Box::new(open_device))
    }

    pub fn with_opener(opener: Opener) -> Audio {
        Audio::from(Source::Unopened(opener))
    }

    fn from(source: Source) -> Audio {
        Audio {
            source,
            stream: None,
            muted: false,
            pending: Vec::new(),
            primed: false,
            stats: Stats::default(),
        }
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Source::Ready(sink) = &mut self.source {
            sink.set_muted(muted);
        }
    }

    pub fn available(&self) -> bool {
        matches!(self.source, Source::Unopened(_) | Source::Ready(_))
    }

    pub fn has_device(&self) -> bool {
        matches!(self.source, Source::Ready(_))
    }

    pub fn streaming(&self) -> bool {
        self.stream.is_some()
    }

    fn format(&self) -> Option<(u32, u16)> {
        match &self.source {
            Source::Ready(sink) => Some((sink.rate().max(1), sink.channels().max(1))),
            _ => None,
        }
    }

    pub fn begin(&mut self, log: &mut Log) -> (u32, u16) {
        if let Source::Unopened(opener) = &mut self.source {
            self.source = match opener() {
                Ok(mut sink) => {
                    sink.set_muted(self.muted);
                    Source::Ready(sink)
                }
                Err(error) => {
                    log.warn(format!("sound: {error}; play runs silent"));
                    Source::Missing
                }
            };
        }
        self.stream = None;
        self.pending.clear();
        self.primed = false;
        self.stats = Stats::default();
        let Some(format) = self.format() else {
            return (DEFAULT_RATE, DEFAULT_CHANNELS);
        };
        if let Source::Ready(sink) = &mut self.source {
            self.stream = Some(sink.stream(AHEAD_SECONDS));
        }
        format
    }

    fn frames(&self, seconds: f32) -> usize {
        let rate = self.format().map_or(DEFAULT_RATE, |format| format.0);
        ((rate as f32 * seconds).round() as usize).max(1)
    }

    fn channels(&self) -> usize {
        usize::from(self.format().map_or(DEFAULT_CHANNELS, |format| format.1))
    }

    pub fn push(&mut self, samples: &[f32]) {
        self.feed(samples, false);
    }

    fn feed(&mut self, samples: &[f32], flush: bool) {
        let limit = self.frames(AHEAD_SECONDS);
        let prime = self.frames(PRIME_SECONDS);
        let channels = self.channels();
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / channels * channels;
        self.pending.truncate(whole);
        if self.primed && stream.queued_frames() == 0 && !self.pending.is_empty() {
            self.stats.underruns += 1;
            self.primed = false;
        }
        let waiting = self.pending.len() / channels;
        if !flush && !self.primed {
            if waiting < prime {
                self.trim(limit, channels);
                return;
            }
            self.primed = true;
        }
        let room = if flush {
            usize::MAX
        } else {
            limit.saturating_sub(stream.queued_frames())
        };
        let offer = waiting.min(room);
        let taken = stream.push(&self.pending[..offer * channels]);
        self.pending.drain(..taken * channels);
        self.trim(limit, channels);
    }

    fn trim(&mut self, limit: usize, channels: usize) {
        let frames = self.pending.len() / channels;
        if frames > limit {
            let excess = frames - limit;
            self.pending.drain(..excess * channels);
            self.stats.dropped_frames += excess as u64;
        }
    }

    pub fn advance(&mut self, session: &mut PlaySession) {
        let samples = session.take_audio();
        self.push(&samples);
    }

    pub fn pause(&mut self, session: &mut PlaySession) {
        let samples = session.take_audio();
        self.feed(&samples, true);
        self.primed = false;
    }

    pub fn abandon(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            stream.finish();
        }
        self.pending.clear();
        self.primed = false;
    }

    pub fn resume(&mut self) {
        self.primed = false;
    }

    pub fn step(&mut self, session: &mut PlaySession) {
        let samples = session.take_audio();
        self.feed(&samples, true);
        self.primed = false;
    }

    pub fn stop(&mut self, session: &mut PlaySession) {
        let samples = session.take_audio();
        self.feed(&samples, true);
        if let Some(mut stream) = self.stream.take() {
            stream.finish();
        }
        self.stats.dropped_frames += (self.pending.len() / self.channels()) as u64;
        self.pending.clear();
        self.primed = false;
    }

    pub fn line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.muted {
            parts.push("muted".to_string());
        }
        if self.stats.underruns > 0 {
            parts.push(format!("{} underruns", self.stats.underruns));
        }
        if self.stats.dropped_frames > 0 {
            parts.push(format!("{} frames dropped", self.stats.dropped_frames));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;

    use pfx_play::{Options, PlaySession, SceneGame};
    use pfx_sound::Clip;

    use super::*;
    use crate::editor::Editor;
    use crate::tests::scene_copy;

    const RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;
    const TICK_FRAMES: usize = 800;
    const FRAME: f32 = 1.0 / 60.0;

    #[derive(Default)]
    struct Shared {
        opened: usize,
        streams: usize,
        pushed: Vec<f32>,
        finished: usize,
        queued: usize,
        capacity: usize,
        muted: Vec<bool>,
    }

    type Handle = Rc<RefCell<Shared>>;

    struct Stand(Handle);

    struct StandStream(Handle);

    impl Sink for Stand {
        fn rate(&self) -> u32 {
            RATE
        }

        fn channels(&self) -> u16 {
            CHANNELS
        }

        fn stream(&mut self, seconds: f32) -> Box<dyn Stream> {
            let mut shared = self.0.borrow_mut();
            shared.streams += 1;
            shared.queued = 0;
            shared.capacity = (RATE as f32 * seconds) as usize;
            Box::new(StandStream(Rc::clone(&self.0)))
        }

        fn set_muted(&mut self, muted: bool) {
            self.0.borrow_mut().muted.push(muted);
        }
    }

    impl Stream for StandStream {
        fn push(&mut self, samples: &[f32]) -> usize {
            let mut shared = self.0.borrow_mut();
            let room = shared.capacity.saturating_sub(shared.queued);
            let frames = (samples.len() / usize::from(CHANNELS)).min(room);
            shared
                .pushed
                .extend_from_slice(&samples[..frames * usize::from(CHANNELS)]);
            shared.queued += frames;
            frames
        }

        fn queued_frames(&self) -> usize {
            self.0.borrow().queued
        }

        fn finish(&mut self) {
            self.0.borrow_mut().finished += 1;
        }
    }

    fn stand() -> (Audio, Handle) {
        let handle = Handle::default();
        let opener = Rc::clone(&handle);
        let audio = Audio::with_opener(Box::new(move || {
            opener.borrow_mut().opened += 1;
            Ok(Box::new(Stand(Rc::clone(&opener))) as Box<dyn Sink>)
        }));
        (audio, handle)
    }

    fn drain(handle: &Handle, frames: usize) {
        let mut shared = handle.borrow_mut();
        shared.queued = shared.queued.saturating_sub(frames);
    }

    fn scene(name: &str) -> pfx_load::scene::Scene {
        let path = scene_copy(name);
        let tone = Clip {
            rate: RATE,
            channels: 1,
            samples: (0..4_800)
                .map(|i| ((i as f32) * 0.07).sin() * 0.5)
                .collect(),
        };
        fs::write(path.with_file_name("tone.wav"), tone.to_wav()).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("\n[sound.hum]\nfile = \"tone.wav\"\nloop = true\nvolume = 0.5\n");
        fs::write(&path, text).unwrap();
        Editor::open(&path).unwrap().scene().clone()
    }

    fn play(scene: &pfx_load::scene::Scene) -> PlaySession {
        PlaySession::play_with(
            scene,
            None,
            Box::new(SceneGame),
            Options {
                audio_rate: RATE,
                audio_channels: CHANNELS,
                ..Options::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn the_frames_pushed_match_the_sessions_audio_in_order_across_play_pause_step_resume_and_stop()
    {
        let scene = scene("audio order");
        let (mut audio, handle) = stand();
        let mut log = Log::default();
        assert_eq!(audio.begin(&mut log), (RATE, CHANNELS));
        assert!(log.entries.is_empty());
        let mut session = play(&scene);
        let mut twin = play(&scene);
        let mut heard = Vec::new();

        let run = |session: &mut PlaySession,
                   twin: &mut PlaySession,
                   audio: &mut Audio,
                   heard: &mut Vec<f32>,
                   frames: usize| {
            for _ in 0..frames {
                session.advance(FRAME);
                twin.advance(FRAME);
                audio.advance(session);
                heard.extend(twin.take_audio());
                drain(&handle, TICK_FRAMES);
            }
        };

        run(&mut session, &mut twin, &mut audio, &mut heard, 12);
        session.pause();
        twin.pause();
        audio.pause(&mut session);
        heard.extend(twin.take_audio());
        let paused_at = handle.borrow().pushed.len();
        assert_eq!(paused_at, heard.len());
        assert!(paused_at > 0);

        run(&mut session, &mut twin, &mut audio, &mut heard, 5);
        assert_eq!(handle.borrow().pushed.len(), paused_at, "pause is silent");

        for step in 1..=2 {
            assert!(session.step() && twin.step());
            audio.step(&mut session);
            heard.extend(twin.take_audio());
            assert_eq!(
                handle.borrow().pushed.len(),
                paused_at + step * TICK_FRAMES * usize::from(CHANNELS),
                "a step pushes its tick's audio"
            );
        }

        session.resume();
        twin.resume();
        audio.resume();
        run(&mut session, &mut twin, &mut audio, &mut heard, 9);

        session.advance(FRAME);
        twin.advance(FRAME);
        audio.stop(&mut session);
        heard.extend(twin.take_audio());

        let shared = handle.borrow();
        assert_eq!(shared.pushed, heard);
        assert!(heard.iter().any(|sample| sample.abs() > 0.01));
        assert_eq!(shared.finished, 1);
        assert_eq!(audio.stats, Stats::default());
        assert!(!audio.streaming());
    }

    #[test]
    fn play_again_opens_a_fresh_stream_on_the_same_device() {
        let (mut audio, handle) = stand();
        let mut log = Log::default();
        audio.begin(&mut log);
        audio.abandon();
        audio.begin(&mut log);
        let shared = handle.borrow();
        assert_eq!(shared.opened, 1);
        assert_eq!(shared.streams, 2);
        assert_eq!(shared.finished, 1);
        assert!(audio.has_device() && audio.streaming());
    }

    #[test]
    fn no_device_logs_once_and_plays_silent() {
        let scene = scene("audio silent");
        let mut audio = Audio::with_opener(Box::new(|| Err("no output device".to_string())));
        let mut log = Log::default();
        for _ in 0..2 {
            let (rate, channels) = audio.begin(&mut log);
            assert_eq!((rate, channels), (DEFAULT_RATE, DEFAULT_CHANNELS));
            let mut session = PlaySession::play_with(
                &scene,
                None,
                Box::new(SceneGame),
                Options {
                    audio_rate: rate,
                    audio_channels: channels,
                    ..Options::default()
                },
            )
            .unwrap();
            for _ in 0..5 {
                session.advance(FRAME);
                audio.advance(&mut session);
                assert!(session.take_audio().is_empty());
            }
            audio.stop(&mut session);
        }
        assert_eq!(log.entries.len(), 1);
        assert!(log.last().unwrap().text.contains("no output device"));
        assert!(!audio.has_device() && !audio.streaming());
        assert_eq!(audio.stats, Stats::default());
    }

    #[test]
    fn an_off_audio_logs_nothing() {
        let mut audio = Audio::off();
        let mut log = Log::default();
        assert_eq!(audio.begin(&mut log), (DEFAULT_RATE, DEFAULT_CHANNELS));
        audio.push(&[0.5; 64]);
        assert!(log.entries.is_empty());
        assert_eq!(audio.line(), None);
    }

    #[test]
    fn a_full_stream_drops_the_oldest_frames_and_counts_them() {
        let (mut audio, handle) = stand();
        audio.begin(&mut Log::default());
        let chunk: Vec<f32> = (0..TICK_FRAMES * 2).map(|i| i as f32).collect();
        for _ in 0..60 {
            audio.push(&chunk);
        }
        let ahead = (RATE as f32 * AHEAD_SECONDS) as usize;
        assert_eq!(handle.borrow().queued, ahead);
        assert_eq!(
            audio.stats.dropped_frames as usize,
            60 * TICK_FRAMES - 2 * ahead
        );
        assert_eq!(audio.stats.underruns, 0);
        assert_eq!(
            audio.line().unwrap(),
            format!("{} frames dropped", audio.stats.dropped_frames)
        );
    }

    #[test]
    fn a_dry_stream_counts_an_underrun_and_primes_again() {
        let (mut audio, handle) = stand();
        audio.begin(&mut Log::default());
        let chunk = vec![0.25; TICK_FRAMES * 2 * usize::from(CHANNELS)];
        for _ in 0..2 {
            audio.push(&chunk);
        }
        assert_eq!(handle.borrow().queued, TICK_FRAMES * 4);
        drain(&handle, usize::MAX);
        audio.push(&chunk);
        assert_eq!(audio.stats.underruns, 1);
        assert_eq!(handle.borrow().queued, 0);
        audio.push(&chunk);
        assert!(handle.borrow().queued > 0);
        assert_eq!(audio.line().unwrap(), "1 underruns");
    }

    #[test]
    fn mute_reaches_the_device_before_and_after_it_opens() {
        let (mut audio, handle) = stand();
        audio.set_muted(true);
        assert!(handle.borrow().muted.is_empty());
        audio.begin(&mut Log::default());
        audio.set_muted(false);
        assert_eq!(handle.borrow().muted, [true, false]);
        assert!(!audio.muted());
        audio.set_muted(true);
        assert_eq!(audio.line().unwrap(), "muted");
    }

    struct Singer {
        stream: Option<pfx_play::PcmStream>,
        phase: u64,
    }

    impl pfx_play::Game for Singer {
        fn start(&mut self, world: &mut pfx_play::World) {
            self.stream = Some(world.sounds.stream(
                pfx_sound::ChannelId::MUSIC,
                1,
                0.1,
                pfx_sound::Level::default(),
            ));
        }

        fn tick(
            &mut self,
            world: &mut pfx_play::World,
            _tick: &pfx_core::clock::Tick,
            _input: &pfx_input::Input,
        ) {
            let due = world.audio_due();
            let rate = world.sounds.rate() as f32;
            let samples: Vec<f32> = (0..due)
                .map(|i| {
                    let t = (self.phase + i as u64) as f32 / rate;
                    (t * std::f32::consts::TAU * 330.0).sin() * 0.5
                })
                .collect();
            self.phase += due as u64;
            if let Some(stream) = &self.stream {
                stream.push(&samples);
            }
        }
    }

    fn singing(audio_rate: u32, audio_channels: u16) -> pfx_game::Driver {
        let session = PlaySession::play_with(
            &pfx_load::scene::Scene::empty(),
            None,
            Box::new(Singer {
                stream: None,
                phase: 0,
            }),
            Options {
                audio_rate,
                audio_channels,
                tunables: Some(pfx_play::Tunables::default()),
                rigs: Some(pfx_play::Rigs::default()),
                ..Options::default()
            },
        )
        .unwrap();
        pfx_game::Driver::new(
            session,
            pfx_play::Settings::default(),
            crate::session::free_policy(),
            pfx_gpu::screens::Device::Desktop,
            pfx_gpu::window::Size {
                width: 640,
                height: 360,
            },
            None,
        )
    }

    #[test]
    fn a_games_own_pcm_stream_plays_through_the_world_mixer_in_the_editors_play() {
        let (mut audio, handle) = stand();
        let (rate, channels) = audio.begin(&mut Log::default());
        let mut played = singing(rate, channels);
        let mut twin = singing(rate, channels);
        let mut heard = Vec::new();
        let frame_ns = 16_666_667;
        for frame in 0..30u64 {
            played.update(frame * frame_ns);
            twin.update(frame * frame_ns);
            audio.advance(played.session_mut());
            heard.extend(twin.take_audio());
            drain(&handle, TICK_FRAMES);
        }
        audio.stop(played.session_mut());
        heard.extend(twin.take_audio());
        let shared = handle.borrow();
        assert_eq!(shared.pushed, heard, "the same mix as a run of its own");
        assert!(played.ticks() >= 28, "{} ticks", played.ticks());
        let loud = shared
            .pushed
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(loud > 0.3, "the game's sine is heard: peak {loud}");
        assert_eq!(audio.stats, Stats::default());
    }
}
