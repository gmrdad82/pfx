use pfx_sound::{ChannelId, Clip, Envelope, Level, Mixer, Shape, SoundSpec, Wave};

const RATE: u32 = 48_000;

fn constant(frames: usize) -> Clip {
    Clip {
        rate: RATE,
        channels: 1,
        samples: vec![1.0; frames],
    }
}

fn largest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0, f32::max)
}

fn power_at(samples: &[f32], frequency: f64) -> f64 {
    let n = samples.len();
    let mut re = 0.0f64;
    let mut im = 0.0f64;
    let mut window_sum = 0.0f64;
    for (i, sample) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let phase = std::f64::consts::TAU * frequency * i as f64 / f64::from(RATE);
        re += f64::from(*sample) * window * phase.cos();
        im += f64::from(*sample) * window * phase.sin();
        window_sum += window;
    }
    let level = 2.0 * (re * re + im * im).sqrt() / window_sum;
    level * level
}

fn octave_power(samples: &[f32], low: f64, count: u32) -> f64 {
    let mut sum = 0.0;
    for k in 0..count {
        let frequency = low * (1.0 + (f64::from(k) + 0.5) / f64::from(count));
        sum += power_at(samples, frequency);
    }
    sum / f64::from(count)
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.log10()
}

fn noise(wave: Wave, seed: u64, frames: usize) -> Vec<f32> {
    let mut mixer = Mixer::new(RATE, 1);
    mixer.set_seed(seed);
    mixer.tone(ChannelId::EFFECTS, wave, 440.0, 0.5);
    mixer.render(frames)
}

#[test]
fn noise_is_seeded_deterministic_and_bounded() {
    for wave in [Wave::White, Wave::Pink] {
        let a = noise(wave, 7, 20_000);
        let b = noise(wave, 7, 20_000);
        let c = noise(wave, 8, 20_000);
        assert_eq!(a, b, "{wave:?}");
        assert_ne!(a, c, "{wave:?}");
        assert!(a.iter().all(|sample| sample.abs() <= 1.0), "{wave:?}");
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        assert!(mean.abs() < 0.05, "{wave:?} mean {mean}");
    }
}

#[test]
fn two_noise_voices_from_one_seed_are_not_the_same_noise() {
    let heard = |first: f32, second: f32| {
        let mut mixer = Mixer::new(RATE, 1);
        mixer.set_seed(3);
        mixer.tone(ChannelId::EFFECTS, Wave::White, 0.0, first);
        mixer.tone(ChannelId::EFFECTS, Wave::White, 0.0, second);
        mixer.render(4000)
    };
    assert_ne!(heard(0.5, 0.0), heard(0.0, 0.5));
    assert_eq!(heard(0.5, 0.0), heard(0.5, 0.0));
}

#[test]
fn white_noise_has_the_level_of_a_uniform_draw_and_a_flat_spectrum() {
    let out = noise(Wave::White, 11, 65_536);
    let rms = (out.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / out.len() as f64).sqrt();
    let expected = 0.5 / 3f64.sqrt();
    assert!((rms - expected).abs() < 0.01, "{rms} vs {expected}");
    let bands: Vec<f64> = [200.0, 400.0, 800.0, 1600.0, 3200.0]
        .iter()
        .map(|low| octave_power(&out, *low, 48))
        .collect();
    for pair in bands.windows(2) {
        let step = db(pair[1] / pair[0]);
        assert!(step.abs() < 1.5, "white octave step {step} dB");
    }
}

#[test]
fn pink_noise_falls_three_decibels_an_octave() {
    let out = noise(Wave::Pink, 11, 131_072);
    let lows = [200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0];
    let levels: Vec<f64> = lows
        .iter()
        .map(|low| db(octave_power(&out, *low, 96)))
        .collect();
    let mean_x = (lows.len() as f64 - 1.0) / 2.0;
    let mean_y = levels.iter().sum::<f64>() / levels.len() as f64;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, y) in levels.iter().enumerate() {
        num += (i as f64 - mean_x) * (y - mean_y);
        den += (i as f64 - mean_x).powi(2);
    }
    let slope = num / den;
    assert!((slope + 3.0).abs() < 0.5, "slope {slope} dB per octave");
    for (i, y) in levels.iter().enumerate() {
        let fitted = mean_y + slope * (i as f64 - mean_x);
        assert!((y - fitted).abs() < 1.5, "band {i}: {y} vs {fitted}");
    }
}

fn sine_with(envelope: Envelope) -> (Mixer, pfx_sound::VoiceId) {
    let mut mixer = Mixer::new(RATE, 1);
    let id = mixer.tone(ChannelId::EFFECTS, Wave::Sine, 1000.0, 1.0);
    mixer.set_envelope(id, Some(envelope));
    (mixer, id)
}

#[test]
fn a_linear_attack_decay_and_sustain_shape_a_clip_exactly() {
    let mut mixer = Mixer::new(RATE, 1);
    let id = mixer.play(&constant(48_000), Level::default());
    mixer.set_envelope(
        id,
        Some(Envelope {
            attack_ms: 10.0,
            decay_ms: 20.0,
            sustain: 0.5,
            release_ms: 10.0,
            shape: Shape::Linear,
        }),
    );
    let out = mixer.render(4800);
    assert_eq!(out[0], 0.0);
    assert!((out[240] - 0.5).abs() < 1e-5, "{}", out[240]);
    assert!(out[..480].windows(2).all(|pair| pair[1] > pair[0]));
    assert!((out[480] - 1.0).abs() < 1e-5, "{}", out[480]);
    assert!((out[480 + 480] - 0.75).abs() < 1e-5, "{}", out[960]);
    assert!(out[480..1440].windows(2).all(|pair| pair[1] < pair[0]));
    assert!(out[1440..].iter().all(|sample| (sample - 0.5).abs() < 1e-6));
}

#[test]
fn a_stop_releases_from_where_the_level_is_and_ends_the_voice_at_silence() {
    let (mut mixer, id) = sine_with(Envelope {
        attack_ms: 10.0,
        decay_ms: 0.0,
        sustain: 0.8,
        release_ms: 20.0,
        shape: Shape::Linear,
    });
    let held = mixer.render(2000);
    assert!(
        held[1500..]
            .iter()
            .any(|sample| (sample.abs() - 0.8).abs() < 0.02)
    );
    mixer.stop(id, 0.0);
    assert_eq!(mixer.voice_count(), 1);
    let tail = mixer.render(960);
    assert!(largest_step(&tail) < 0.14, "{}", largest_step(&tail));
    assert!(tail.last().unwrap().abs() < 0.001);
    let more = mixer.render(10);
    assert!(more.iter().all(|sample| *sample == 0.0));
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn a_stop_in_the_middle_of_the_attack_releases_from_the_level_reached() {
    let (mut mixer, id) = sine_with(Envelope {
        attack_ms: 50.0,
        release_ms: 10.0,
        ..Envelope::default()
    });
    mixer.render(1000);
    let before = mixer.envelope_level(id).unwrap();
    assert!(before > 0.3 && before < 0.5, "{before}");
    mixer.stop(id, 0.0);
    let tail = mixer.render(600);
    assert!(largest_step(&tail) < 0.14);
    assert!(tail[0].abs() <= before + 0.01);
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn a_stop_with_a_longer_fade_than_the_release_takes_the_fade() {
    let (mut mixer, id) = sine_with(Envelope {
        release_ms: 10.0,
        ..Envelope::default()
    });
    mixer.render(2000);
    mixer.stop(id, 0.1);
    mixer.render(4000);
    assert_eq!(mixer.voice_count(), 1);
    mixer.render(1000);
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn an_envelope_starts_and_stops_without_a_click_even_on_a_square_wave() {
    let mut mixer = Mixer::new(RATE, 1);
    let id = mixer.tone(ChannelId::EFFECTS, Wave::Square, 100.0, 1.0);
    mixer.set_envelope(id, Some(Envelope::default()));
    let start = mixer.render(96);
    assert_eq!(start[0], 0.0);
    assert!(start.windows(2).all(|pair| pair[1].abs() >= pair[0].abs()));
    mixer.render(5000);
    mixer.stop(id, 0.0);
    let end = mixer.render(200);
    assert!(end.windows(2).all(|pair| pair[1].abs() <= pair[0].abs()));
    assert!(end.last().unwrap().abs() < 0.01);
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn an_exponential_shape_rises_and_falls_faster_at_first_and_lands_exactly() {
    let shaped = |shape| {
        let mut mixer = Mixer::new(RATE, 1);
        let id = mixer.play(&constant(48_000), Level::default());
        mixer.set_envelope(
            id,
            Some(Envelope {
                attack_ms: 10.0,
                decay_ms: 20.0,
                sustain: 0.2,
                release_ms: 10.0,
                shape,
            }),
        );
        mixer.render(4800)
    };
    let linear = shaped(Shape::Linear);
    let curved = shaped(Shape::Exponential);
    assert!(curved[240] > linear[240] + 0.3, "{}", curved[240]);
    assert!(curved[480 + 240] < linear[480 + 240] - 0.1);
    assert!((curved[480] - 1.0).abs() < 1e-5);
    assert!((curved[4000] - 0.2).abs() < 1e-6);
    assert!(curved[..480].windows(2).all(|pair| pair[1] > pair[0]));
    assert!(curved[480..1440].windows(2).all(|pair| pair[1] < pair[0]));
}

#[test]
fn an_exponential_release_has_no_step() {
    let (mut mixer, id) = sine_with(Envelope {
        release_ms: 10.0,
        shape: Shape::Exponential,
        ..Envelope::default()
    });
    mixer.render(3000);
    mixer.stop(id, 0.0);
    let tail = mixer.render(600);
    assert!(largest_step(&tail) < 0.15, "{}", largest_step(&tail));
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn a_voice_that_never_played_is_dropped_by_a_stop() {
    let mut mixer = Mixer::new(RATE, 1);
    let id = mixer.tone(ChannelId::EFFECTS, Wave::Sine, 440.0, 1.0);
    mixer.set_envelope(id, Some(Envelope::default()));
    mixer.stop(id, 0.0);
    assert_eq!(mixer.voice_count(), 0);
}

#[test]
fn a_delayed_clip_starts_its_envelope_when_it_starts() {
    let mut mixer = Mixer::new(RATE, 1);
    let id = mixer.play_at(1000, &constant(8000), Level::default());
    mixer.set_envelope(
        id,
        Some(Envelope {
            attack_ms: 10.0,
            ..Envelope::default()
        }),
    );
    let out = mixer.render(2000);
    assert!(out[..1000].iter().all(|sample| *sample == 0.0));
    assert_eq!(out[1000], 0.0);
    assert!((out[1240] - 0.5).abs() < 1e-5);
    assert!((out[1500] - 1.0).abs() < 1e-5);
}

#[test]
fn an_envelope_attached_to_a_playing_voice_does_not_jump() {
    let mut mixer = Mixer::new(RATE, 1);
    let id = mixer.play(&constant(48_000), Level::default());
    mixer.render(100);
    mixer.set_envelope(
        id,
        Some(Envelope {
            attack_ms: 10.0,
            decay_ms: 10.0,
            sustain: 0.5,
            ..Envelope::default()
        }),
    );
    let out = mixer.render(1000);
    assert_eq!(out[0], 1.0);
    assert!(largest_step(&out) < 0.002);
    assert!((out[999] - 0.5).abs() < 1e-6);
}

#[test]
fn a_sound_spec_envelope_shapes_every_trigger() {
    let mut mixer = Mixer::new(RATE, 1);
    let hit = mixer.sound(
        &constant(48_000),
        SoundSpec {
            envelope: Some(Envelope {
                attack_ms: 10.0,
                ..Envelope::default()
            }),
            ..SoundSpec::default()
        },
    );
    mixer.trigger_at(0, hit);
    mixer.trigger_at(2000, hit);
    let out = mixer.render(4000);
    assert_eq!(out[0], 0.0);
    assert!((out[240] - 0.5).abs() < 1e-5);
    assert_eq!(out[1999], 1.0);
    assert!((out[2000] - 1.0).abs() < 1e-5);
    assert!(out[2001] > 1.0 - 1e-5);
}

#[test]
fn removing_an_envelope_leaves_the_voice_as_it_was() {
    let (mut mixer, id) = sine_with(Envelope {
        attack_ms: 10.0,
        ..Envelope::default()
    });
    mixer.render(100);
    mixer.set_envelope(id, None);
    assert_eq!(mixer.envelope_level(id), None);
    mixer.stop(id, 0.0);
    assert_eq!(mixer.voice_count(), 0);
}
