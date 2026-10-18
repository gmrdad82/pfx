use std::time::Instant;

use pfx_sound::{
    AdaptiveTrack, ChannelId, Clip, Level, Mixer, Quantize, Section, SoundSpec, Stem, StemGain,
    Wave,
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
    let frames = shot::audio_frames(600, 60);
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
