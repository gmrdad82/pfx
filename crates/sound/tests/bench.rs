use std::time::Instant;

use pfx_sound::{
    AdaptiveTrack, ChannelId, Clip, Envelope, Level, Mixer, Note, NoteRender, Quantize, Section,
    Shape, SoundSpec, Stem, StemGain, Wave,
};

const RATE: u32 = 48_000;
const TICK: usize = 800;
const TICKS: usize = 600;
const TRIGGERS_PER_SECOND: usize = 2_000;

fn shot() -> Clip {
    let frames = 4_800;
    Clip {
        rate: RATE,
        channels: 1,
        samples: (0..frames)
            .map(|i| {
                let decay = 1.0 - i as f32 / frames as f32;
                let t = i as f32 / RATE as f32;
                0.4 * decay * decay * (std::f32::consts::TAU * 900.0 * t).sin()
            })
            .collect(),
    }
}

fn clips(spec: SoundSpec, spread: bool) -> (f64, usize) {
    let mut mixer = Mixer::new(RATE, 2);
    mixer.set_seed(7);
    let sound = mixer.sound(&shot(), spec);
    let mut out = vec![0.0f32; TICK * 2];
    let mut triggered = 0usize;
    let mut voices = 0usize;
    let started = Instant::now();
    for tick in 0..TICKS {
        let due = (tick + 1) * TRIGGERS_PER_SECOND * TICK / RATE as usize;
        let count = due - triggered;
        for k in 0..count {
            if spread {
                let _ = mixer.trigger_in((k * TICK / count) as u64, sound);
            } else {
                let _ = mixer.trigger(sound);
            }
        }
        triggered = due;
        mixer.render_into(&mut out);
        voices += mixer.voice_count();
    }
    let per_tick = started.elapsed().as_secs_f64() * 1000.0 / TICKS as f64;
    (per_tick, voices / TICKS)
}

fn tones(spec: Option<SoundSpec>) -> f64 {
    let mut mixer = Mixer::new(RATE, 2);
    mixer.set_seed(7);
    let started = Instant::now();
    match spec {
        Some(spec) => {
            let beep = mixer.tone_sound(Wave::Sine, 660.0, None, spec);
            for _ in 0..20 {
                let _ = mixer.trigger(beep);
            }
        }
        None => {
            for k in 0..20 {
                mixer.tone(
                    ChannelId::EFFECTS,
                    Wave::Sine,
                    220.0 + 20.0 * k as f32,
                    0.05,
                );
            }
        }
    }
    let mut out = vec![0.0f32; TICK * 2];
    for _ in 0..TICKS {
        mixer.render_into(&mut out);
    }
    started.elapsed().as_secs_f64() * 1000.0 / TICKS as f64
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_short_effects_and_tones() {
    let capped = SoundSpec {
        max_voices: 20,
        pitch_jitter: 2.0,
        volume_jitter: 0.2,
        ..SoundSpec::default()
    };
    for (name, spec) in [
        (
            "clips, no jitter, sinc",
            SoundSpec {
                pitch_jitter: 0.0,
                resample: pfx_sound::Resample::Sinc,
                ..capped
            },
        ),
        (
            "clips, jitter, sinc",
            SoundSpec {
                resample: pfx_sound::Resample::Sinc,
                ..capped
            },
        ),
        (
            "clips, jitter, cubic",
            SoundSpec {
                resample: pfx_sound::Resample::Cubic,
                ..capped
            },
        ),
        (
            "clips, jitter, linear",
            SoundSpec {
                resample: pfx_sound::Resample::Linear,
                ..capped
            },
        ),
    ] {
        let (ms, voices) = clips(spec, false);
        println!("{name}: {ms:.3} ms a tick, {voices} voices on average");
    }
    let (ms, voices) = clips(capped, true);
    println!("clips, jitter, cubic, spread in the tick: {ms:.3} ms a tick, {voices} voices");
    println!("20 sine tones: {:.3} ms a tick", tones(None));
    println!(
        "20 sine tones from one tone sound with jitter: {:.3} ms a tick",
        tones(Some(SoundSpec {
            max_voices: 20,
            pitch_jitter: 2.0,
            ..SoundSpec::default()
        }))
    );
}

fn drive_cost(curve: Option<pfx_sound::DriveCurve>) -> f64 {
    let mut mixer = Mixer::new(RATE, 2);
    for k in 0..4 {
        mixer.tone(ChannelId::EFFECTS, Wave::Saw, 55.0 + 11.0 * k as f32, 0.2);
    }
    if let Some(curve) = curve {
        mixer.set_channel_drive(
            ChannelId::EFFECTS,
            Some(pfx_sound::Drive {
                drive_db: 18.0,
                curve,
                mix: 0.8,
                ..pfx_sound::Drive::default()
            }),
        );
    }
    let mut out = vec![0.0f32; TICK * 2];
    mixer.render_into(&mut out);
    let started = Instant::now();
    for _ in 0..TICKS * 10 {
        mixer.render_into(&mut out);
    }
    started.elapsed().as_secs_f64() * 1000.0 / (TICKS * 10) as f64
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_channel_drive() {
    let off = drive_cost(None);
    println!("4 saw tones, drive off: {off:.4} ms a tick");
    for (name, curve) in [
        ("soft", pfx_sound::DriveCurve::Soft),
        ("tanh", pfx_sound::DriveCurve::Tanh),
        ("hard", pfx_sound::DriveCurve::Hard),
    ] {
        let on = drive_cost(Some(curve));
        println!(
            "4 saw tones, drive {name}: {on:.4} ms a tick, drive alone {:.4} ms a tick",
            on - off
        );
    }
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_shot_mix() {
    use pfx_sound::shot::{self, Script};
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/car-pass.sound.toml");
    let full = Script::load(&path).unwrap();
    let frames = shot::audio_frames(600, pfx_core::rate::Rate::whole(60));
    let time = |script: &Script| {
        (0..5)
            .map(|_| {
                let started = Instant::now();
                let out = script.mix(frames).unwrap();
                assert_eq!(out.len(), frames * 2);
                started.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::MAX, f64::min)
    };
    for layer in &full.layers {
        let mut one = full.clone();
        one.layers = vec![layer.clone()];
        let name = format!("{:?}", layer.kind);
        println!(
            "{:<10} {:7.2} ms for 10 s",
            name.split('(').next().unwrap_or(""),
            time(&one)
        );
    }
    println!("{:<10} {:7.2} ms for 10 s", "all", time(&full));
}

fn adaptive_cost(case: &str) -> f64 {
    let frames = RATE as usize * 8;
    let stems = (0..8)
        .map(|n| Stem {
            name: format!("stem{n}"),
            clip: Clip {
                rate: RATE,
                channels: 2,
                samples: (0..frames * 2)
                    .map(|i| ((i * (n + 3)) % 997) as f32 / 9970.0 - 0.05)
                    .collect(),
            },
            gain: StemGain::above(n as f32 / 10.0, 0.1),
        })
        .collect();
    let mut track = AdaptiveTrack::new(stems, 120.0, 4);
    track.sections = vec![Section::new("a", (0, 2)), Section::new("b", (2, 4))];
    track.transition = 0.5;
    track.intensity = 1.0;
    let prepared = track.prepare(RATE).unwrap();
    let mut mixer = Mixer::new(RATE, 2);
    mixer.play_adaptive(&prepared, 0.0, Level::default());
    let mut out = vec![0.0f32; TICK * 2];
    let started = Instant::now();
    for tick in 0..TICKS {
        match case {
            "ramping" if tick % 120 == 0 => {
                mixer.set_music_intensity(if tick % 240 == 0 { 0.0 } else { 1.0 }, 2.0)
            }
            "crossfading" if tick % 30 == 0 => {
                let _ = mixer
                    .request_music_section(if tick % 60 == 0 { "b" } else { "a" }, Quantize::Now);
            }
            _ => {}
        }
        mixer.render_into(&mut out);
    }
    started.elapsed().as_secs_f64() * 1000.0 / (TICKS * TICK) as f64 * RATE as f64
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_adaptive_eight_stems() {
    for case in ["settled", "ramping", "crossfading"] {
        let best = (0..5).map(|_| adaptive_cost(case)).fold(f64::MAX, f64::min);
        println!("8 stereo stems, {case}: {best:.3} ms a second of audio");
    }
}

const PAD_SECONDS: usize = 30;
const PAD_CHORD: u64 = 96_000;
const PAD_ENVELOPE: Envelope = Envelope {
    attack_ms: 40.0,
    decay_ms: 0.0,
    sustain: 0.85,
    release_ms: 300.0,
    shape: Shape::Exponential,
};

fn pad_notes() -> Vec<Note> {
    let chords = [[52, 55, 59], [52, 57, 60], [52, 55, 60], [51, 54, 59]];
    let mut notes = Vec::new();
    for c in 0..(PAD_SECONDS as u64 * u64::from(RATE) / PAD_CHORD) {
        for midi in chords[c as usize % chords.len()] {
            for (cents, pan) in [(-9.0f32, -0.5f32), (9.0, 0.5)] {
                notes.push(Note {
                    at: c * PAD_CHORD,
                    held: PAD_CHORD,
                    wave: Wave::Saw,
                    hz: 440.0 * 2f32.powf((midi as f32 - 69.0 + cents / 100.0) / 12.0),
                    glide: None,
                    gain: 0.02,
                    pan,
                    envelope: PAD_ENVELOPE,
                    fade: None,
                    seed: c,
                });
            }
        }
    }
    notes
}

fn pad_lowpass() -> Vec<(u64, f32, f32)> {
    (0..PAD_SECONDS as u64 / 4)
        .map(|k| (k * 4 * u64::from(RATE), 1500.0 + 170.0 * k as f32, 4.0))
        .collect()
}

fn pad_mixer(notes: &[Note]) -> Vec<f32> {
    let frames = PAD_SECONDS * RATE as usize;
    let mut mixer = Mixer::new(RATE, 2);
    let ch = ChannelId::custom(100);
    mixer.set_channel_highpass(ch, Some(150.0));
    let mut events: Vec<(u64, usize, u8)> = Vec::new();
    for (i, n) in notes.iter().enumerate() {
        events.push((n.at, i, 0));
        events.push((n.at + n.held, i, 1));
    }
    for (i, (at, _, _)) in pad_lowpass().iter().enumerate() {
        events.push((*at, i, 2));
    }
    events.sort();
    let lowpass = pad_lowpass();
    let mut ids = vec![None; notes.len()];
    let mut out = Vec::with_capacity(frames * 2);
    let mut clock = 0u64;
    for (at, i, kind) in events {
        if at > clock {
            out.extend(mixer.render((at - clock) as usize));
            clock = at;
        }
        match kind {
            0 => {
                let n = notes[i];
                let id = mixer.tone(ch, n.wave, n.hz, n.gain);
                mixer.set_envelope(id, Some(n.envelope));
                mixer.set_pan(id, n.pan);
                ids[i] = Some(id);
            }
            1 => {
                if let Some(id) = ids[i] {
                    mixer.stop(id, notes[i].envelope.release_ms / 1000.0);
                }
            }
            _ => {
                let (_, hz, secs) = lowpass[i];
                mixer.glide_channel_lowpass(ch, Some(hz), secs);
            }
        }
    }
    out.extend(mixer.render(frames - clock as usize));
    out
}

fn pad_offline(notes: &[Note]) -> Vec<f32> {
    let mut render = NoteRender::new(RATE, PAD_SECONDS * RATE as usize);
    render.highpass(0, Some(150.0), 0);
    for (at, hz, secs) in pad_lowpass() {
        render.lowpass(at, Some(hz), (secs * RATE as f32) as u64);
    }
    for note in notes {
        render.note(note);
    }
    render.finish()
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_pad_stem() {
    let notes = pad_notes();
    let time = |f: &dyn Fn() -> Vec<f32>| {
        let mut runs: Vec<f64> = (0..5)
            .map(|_| {
                let started = Instant::now();
                std::hint::black_box(f());
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        runs.sort_by(f64::total_cmp);
        runs[2]
    };
    let live = time(&|| pad_mixer(&notes));
    let offline = time(&|| pad_offline(&notes));
    let a = pad_mixer(&notes);
    let b = pad_offline(&notes);
    let diff = a
        .iter()
        .zip(&b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    println!(
        "pad stem, {} notes, {PAD_SECONDS} s: live mixer {live:.1} ms, offline {offline:.1} ms, max difference {diff:.2e}",
        notes.len()
    );
}
