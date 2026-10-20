use crate::channel::{Ramp, ramp_frames};
use crate::drive::{Drive, DriveStage};
use crate::envelope::{EnvState, Envelope, Shape};
use crate::filter::{Filter, Kind};
use crate::mix::FILTER_RAMP_SECONDS;
use crate::synth::{Synth, Wave};
use crate::voice::balance;

const TUNE_EVERY: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    pub after: u64,
    pub frames: u64,
    pub to_hz: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub at: u64,
    pub held: u64,
    pub wave: Wave,
    pub hz: f32,
    pub glide: Option<Glide>,
    pub gain: f32,
    pub pan: f32,
    pub envelope: Envelope,
    pub fade: Option<u64>,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
    LowPass(Option<f32>, u32),
    HighPass(Option<f32>, u32),
    LowSweep(f32, f32, u32),
    HighSweep(f32, f32, u32),
    Drive(Option<Drive>),
}

struct Curves {
    key: [u32; 5],
    attack: Vec<f32>,
    decay: Vec<f32>,
    sustain: f32,
    release: Vec<f32>,
}

impl Curves {
    fn new(envelope: Envelope, rate: u32) -> Curves {
        let mut state = EnvState::new(envelope, rate, false);
        let (attack, decay, release) = state.lengths();
        let attack = (0..attack).map(|_| state.next()).collect();
        let decay = (0..decay).map(|_| state.next()).collect();
        let sustain = state.next();
        let mut state = EnvState::new(
            Envelope {
                sustain: 1.0,
                decay_ms: 0.0,
                ..envelope
            },
            rate,
            true,
        );
        state.next();
        state.release(0);
        let release = (0..release).map(|_| state.next()).collect();
        Curves {
            key: key(envelope),
            attack,
            decay,
            sustain,
            release,
        }
    }

    fn held(&self, n: usize) -> f32 {
        if let Some(level) = self.attack.get(n) {
            return *level;
        }
        self.decay
            .get(n - self.attack.len())
            .copied()
            .unwrap_or(self.sustain)
    }
}

fn every(glide: u32, rate: u32) -> usize {
    if u64::from(glide) * 10 > u64::from(rate) {
        TUNE_EVERY
    } else {
        1
    }
}

fn key(envelope: Envelope) -> [u32; 5] {
    [
        envelope.attack_ms.to_bits(),
        envelope.decay_ms.to_bits(),
        envelope.sustain.to_bits(),
        envelope.release_ms.to_bits(),
        u32::from(envelope.shape == Shape::Exponential),
    ]
}

pub struct NoteRender {
    rate: u32,
    frames: usize,
    buffer: Vec<f32>,
    changes: Vec<(u64, Change)>,
    curves: Vec<Curves>,
    level: Vec<f32>,
    wave: Vec<f32>,
}

impl NoteRender {
    pub fn new(rate: u32, frames: usize) -> NoteRender {
        NoteRender {
            rate: rate.max(1),
            frames,
            buffer: vec![0.0; frames * 2],
            changes: Vec::new(),
            curves: Vec::new(),
            level: Vec::new(),
            wave: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn buffer_mut(&mut self) -> &mut [f32] {
        &mut self.buffer
    }

    pub fn lowpass(&mut self, at: u64, hz: Option<f32>, frames: u64) {
        let frames = self.glide_frames(frames);
        self.changes.push((at, Change::LowPass(hz, frames)));
    }

    pub fn highpass(&mut self, at: u64, hz: Option<f32>, frames: u64) {
        let frames = self.glide_frames(frames);
        self.changes.push((at, Change::HighPass(hz, frames)));
    }

    pub fn lowpass_sweep(&mut self, at: u64, from: f32, to: f32, frames: u64) {
        let frames = self.glide_frames(frames);
        self.changes.push((at, Change::LowSweep(from, to, frames)));
    }

    pub fn highpass_sweep(&mut self, at: u64, from: f32, to: f32, frames: u64) {
        let frames = self.glide_frames(frames);
        self.changes.push((at, Change::HighSweep(from, to, frames)));
    }

    pub fn drive(&mut self, at: u64, drive: Option<Drive>) {
        self.changes.push((at, Change::Drive(drive)));
    }

    fn glide_frames(&self, frames: u64) -> u32 {
        if frames == 0 {
            let ramp = (f64::from(FILTER_RAMP_SECONDS) * f64::from(self.rate)).round();
            (ramp as u32).max(1)
        } else {
            u32::try_from(frames).unwrap_or(u32::MAX)
        }
    }

    fn curves(&mut self, envelope: Envelope) -> usize {
        let key = key(envelope);
        if let Some(found) = self.curves.iter().position(|c| c.key == key) {
            return found;
        }
        self.curves.push(Curves::new(envelope, self.rate));
        self.curves.len() - 1
    }

    pub fn note(&mut self, note: &Note) {
        let Ok(start) = usize::try_from(note.at) else {
            return;
        };
        if start >= self.frames {
            return;
        }
        let curves = self.curves(note.envelope);
        let held = usize::try_from(note.held).unwrap_or(usize::MAX).max(1);
        let curves = &self.curves[curves];
        let from = curves.held(held - 1);
        let length = held.saturating_add(curves.release.len());
        let end = start.saturating_add(length).min(self.frames);
        let span = end - start;
        self.level.clear();
        self.level.reserve(span);
        for n in 0..span.min(held) {
            self.level.push(curves.held(n));
        }
        for r in curves.release.iter().take(span.saturating_sub(held)) {
            self.level.push(from * r);
        }
        let mut gain = Ramp::new(1.0);
        if let Some(fade) = note.fade {
            gain.to(0.0, u32::try_from(fade).unwrap_or(u32::MAX).max(1));
        }
        let (pan_l, pan_r) = balance(note.pan);
        let mut synth = Synth::new(note.wave, note.hz, note.seed);
        self.wave.clear();
        self.wave.resize(span, 0.0);
        match note.glide {
            Some(glide) => {
                let at = usize::try_from(glide.after).unwrap_or(usize::MAX).min(span);
                synth.fill(&mut self.wave[..at], self.rate);
                let frames = u32::try_from(glide.frames).unwrap_or(u32::MAX).max(1);
                synth.frequency.to(glide.to_hz.max(0.0), frames);
                synth.fill(&mut self.wave[at..], self.rate);
            }
            None => synth.fill(&mut self.wave, self.rate),
        }
        let volume = note.gain;
        let out = &mut self.buffer[start * 2..end * 2];
        let voice = self.wave.iter().zip(&self.level);
        if note.fade.is_some() {
            for (frame, (sample, level)) in out.chunks_exact_mut(2).zip(voice) {
                let g = volume * level * gain.next();
                frame[0] += sample * g * pan_l;
                frame[1] += sample * g * pan_r;
            }
        } else {
            for (frame, (sample, level)) in out.chunks_exact_mut(2).zip(voice) {
                let g = volume * level;
                frame[0] += sample * g * pan_l;
                frame[1] += sample * g * pan_r;
            }
        }
    }

    pub fn finish(self) -> Vec<f32> {
        let NoteRender {
            rate,
            frames,
            mut buffer,
            mut changes,
            ..
        } = self;
        changes.sort_by_key(|(at, _)| *at);
        let mut lowpass = Filter::new(Kind::LowPass);
        let mut highpass = Filter::new(Kind::HighPass);
        let mut drive = DriveStage::new();
        let mut low_every = 1;
        let mut high_every = 1;
        let mut done = 0usize;
        let mut pending = changes.into_iter().peekable();
        while done < frames {
            while let Some((_, change)) = pending.next_if(|(at, _)| *at <= done as u64) {
                match change {
                    Change::LowPass(hz, glide) => {
                        low_every = every(glide, rate);
                        lowpass.set(hz, glide, rate);
                    }
                    Change::HighPass(hz, glide) => {
                        high_every = every(glide, rate);
                        highpass.set(hz, glide, rate);
                    }
                    Change::LowSweep(from, to, glide) => {
                        low_every = every(glide, rate);
                        lowpass.jump(from, rate);
                        lowpass.set(Some(to), glide, rate);
                    }
                    Change::HighSweep(from, to, glide) => {
                        high_every = every(glide, rate);
                        highpass.jump(from, rate);
                        highpass.set(Some(to), glide, rate);
                    }
                    Change::Drive(config) => drive.set(config, ramp_frames(rate)),
                }
            }
            let next = pending.peek().map_or(frames, |(at, _)| {
                usize::try_from(*at).map_or(frames, |at| at.min(frames))
            });
            let block = &mut buffer[done * 2..next * 2];
            let len = next - done;
            lowpass.process_every(block, len, 2, rate, low_every);
            highpass.process_every(block, len, 2, rate, high_every);
            drive.process(block, len, 2);
            done = next;
        }
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelId, DriveCurve, Mixer};

    const RATE: u32 = 48_000;
    const FRAMES: usize = 24_000;

    type Sweep = (u64, f32, u64);

    fn envelope(sustain: f32, shape: Shape) -> Envelope {
        Envelope {
            attack_ms: 12.0,
            decay_ms: 30.0,
            sustain,
            release_ms: 60.0,
            shape,
        }
    }

    fn notes() -> Vec<Note> {
        let base = Note {
            at: 0,
            held: FRAMES as u64,
            wave: Wave::Saw,
            hz: 110.0,
            glide: None,
            gain: 0.2,
            pan: -0.4,
            envelope: envelope(0.7, Shape::Exponential),
            fade: None,
            seed: 3,
        };
        vec![
            base,
            Note {
                at: 2_000,
                held: 5_000,
                wave: Wave::Triangle,
                hz: 330.0,
                glide: Some(Glide {
                    after: 1_000,
                    frames: 3_000,
                    to_hz: 660.0,
                }),
                pan: 0.6,
                envelope: envelope(0.0, Shape::Linear),
                ..base
            },
            Note {
                at: 6_500,
                held: 9_000,
                wave: Wave::Pulse(0.25),
                hz: 220.0,
                fade: Some(8_000),
                pan: 0.0,
                ..base
            },
            Note {
                at: 9_000,
                held: 4_000,
                wave: Wave::Pink,
                hz: 0.0,
                gain: 0.1,
                seed: 77,
                ..base
            },
        ]
    }

    fn drive() -> Drive {
        Drive {
            drive_db: 6.0,
            curve: DriveCurve::Tanh,
            mix: 0.5,
            compensate: false,
            output_db: 0.0,
        }
    }

    fn through_mixer(notes: &[Note], highpass: Option<f32>, lowpass: Option<Sweep>) -> Vec<f32> {
        let mut mixer = Mixer::new(RATE, 2);
        let channel = ChannelId::custom(9);
        if highpass.is_some() {
            mixer.set_channel_highpass(channel, highpass);
        }
        if let Some((at, hz, frames)) = lowpass {
            mixer.glide_channel_lowpass_at(at, channel, Some(hz), frames as f32 / RATE as f32);
        }
        mixer.set_channel_drive(channel, Some(drive()));
        let mut events: Vec<(u64, usize, bool)> = Vec::new();
        for (i, n) in notes.iter().enumerate() {
            events.push((n.at, i, true));
            events.push((n.at + n.held, i, false));
        }
        events.sort();
        let mut ids = vec![None; notes.len()];
        let mut out = Vec::new();
        let mut clock = 0u64;
        for (at, i, start) in events {
            let at = at.min(FRAMES as u64);
            if at > clock {
                out.extend(mixer.render((at - clock) as usize));
                clock = at;
            }
            let n = notes[i];
            if start {
                mixer.set_seed(n.seed);
                let id = mixer.tone(channel, n.wave, n.hz, n.gain);
                mixer.set_envelope(id, Some(n.envelope));
                mixer.set_pan(id, n.pan);
                if let Some(g) = n.glide {
                    mixer.glide_tone_frequency_in(
                        g.after,
                        id,
                        g.to_hz,
                        g.frames as f32 / RATE as f32,
                    );
                }
                if let Some(fade) = n.fade {
                    mixer.glide_gain(id, 0.0, fade as f32 / RATE as f32);
                }
                ids[i] = Some(id);
            } else if let Some(id) = ids[i] {
                mixer.stop(id, n.envelope.release_ms / 1000.0);
            }
        }
        out.extend(mixer.render(FRAMES - clock as usize));
        out
    }

    fn offline(notes: &[Note], highpass: Option<f32>, lowpass: Option<Sweep>) -> Vec<f32> {
        let mut render = NoteRender::new(RATE, FRAMES);
        if highpass.is_some() {
            render.highpass(0, highpass, 0);
        }
        if let Some((at, hz, frames)) = lowpass {
            render.lowpass(at, Some(hz), frames);
        }
        render.drive(0, Some(drive()));
        for note in notes {
            let seeded = Note {
                seed: {
                    let mut rng = crate::sound::Rng::new(note.seed);
                    rng.seed()
                },
                ..*note
            };
            render.note(&seeded);
        }
        render.finish()
    }

    #[test]
    fn a_stem_renders_exactly_as_the_live_mixer_renders_it() {
        let notes = notes();
        let live = through_mixer(&notes, None, None);
        let rendered = offline(&notes, None, None);
        assert_eq!(live.len(), rendered.len());
        assert!(rendered.iter().any(|s| s.abs() > 0.05));
        let first = live.iter().zip(&rendered).position(|(a, b)| a != b);
        assert_eq!(
            first,
            None,
            "{:?}",
            first.map(|i| (i, live[i], rendered[i]))
        );
        let live = through_mixer(&notes, Some(90.0), None);
        let rendered = offline(&notes, Some(90.0), None);
        assert_eq!(live, rendered);
    }

    #[test]
    fn a_long_glide_and_a_note_released_in_its_attack_stay_close_to_the_live_mixer() {
        let mut notes = notes();
        notes.push(Note {
            at: 15_000,
            held: 300,
            wave: Wave::Square,
            hz: 440.0,
            ..notes[0]
        });
        let sweep = Some((1_234, 400.0, 6_000));
        let live = through_mixer(&notes, None, sweep);
        let rendered = offline(&notes, None, sweep);
        assert_eq!(live.len(), rendered.len());
        let worst = live
            .iter()
            .zip(&rendered)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "{worst}");
    }

    #[test]
    fn every_note_sweeps_its_filter_from_the_same_cutoff() {
        let note = Note {
            at: 0,
            held: 4_000,
            wave: Wave::Saw,
            hz: 55.0,
            glide: None,
            gain: 0.3,
            pan: 0.0,
            envelope: envelope(0.7, Shape::Exponential),
            fade: None,
            seed: 5,
        };
        let mut render = NoteRender::new(RATE, FRAMES);
        for at in [0, 12_000] {
            render.note(&Note { at, ..note });
            render.lowpass_sweep(at, 2_000.0, 320.0, 9_600);
        }
        let out = render.finish();
        let (first, second) = (&out[..12_000], &out[24_000..36_000]);
        assert!(first.iter().any(|s| s.abs() > 0.05));
        let worst = first
            .iter()
            .zip(second)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "{worst}");
    }
}
