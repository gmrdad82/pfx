use pfx_sound::voicefx::{Stretch, formant_shift, pitch_shift, time_stretch};

const RATE: u32 = 48_000;

fn sine(hz: f32, seconds: f32, channels: u16) -> Vec<f32> {
    let frames = (RATE as f32 * seconds) as usize;
    (0..frames)
        .flat_map(|i| {
            let v = 0.5 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin();
            std::iter::repeat_n(v, channels as usize)
        })
        .collect()
}

fn dominant_hz(mono: &[f32]) -> f32 {
    let middle = &mono[mono.len() / 4..mono.len() * 3 / 4];
    let crossings = middle
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    crossings as f32 / 2.0 / (middle.len() as f32 / RATE as f32)
}

#[test]
fn pitch_shift_keeps_the_length() {
    for channels in [1, 2] {
        let input = sine(440.0, 1.0, channels);
        assert_eq!(pitch_shift(&input, channels, RATE, 5.0).len(), input.len());
        assert_eq!(
            pitch_shift(&input, channels, RATE, -12.0).len(),
            input.len()
        );
    }
    let short = sine(440.0, 0.05, 1);
    assert_eq!(pitch_shift(&short, 1, RATE, 3.0).len(), short.len());
}

#[test]
fn pitch_shift_moves_a_sine_by_the_semitone_ratio() {
    let input = sine(440.0, 1.0, 1);
    for semitones in [7.0f32, -5.0, 12.0] {
        let expected = 440.0 * 2f32.powf(semitones / 12.0);
        let found = dominant_hz(&pitch_shift(&input, 1, RATE, semitones));
        assert!(
            (found / expected - 1.0).abs() < 0.01,
            "{semitones} semitones: {found} Hz, expected {expected}"
        );
    }
}

#[test]
fn formant_shift_keeps_the_pitch() {
    let input = sine(440.0, 1.0, 1);
    let out = formant_shift(&input, 1, RATE, 4.0);
    assert_eq!(out.len(), input.len());
    let found = dominant_hz(&out);
    assert!((found / 440.0 - 1.0).abs() < 0.01, "{found} Hz");
}

#[test]
fn time_stretch_scales_the_length_and_keeps_the_pitch() {
    let input = sine(440.0, 1.0, 2);
    let frames = input.len() / 2;
    for factor in [0.5f32, 1.5, 3.0] {
        let out = time_stretch(&input, 2, RATE, factor);
        let expected = (frames as f32 * factor).round() as usize * 2;
        assert_eq!(out.len(), expected, "{factor}");
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let found = dominant_hz(&left);
        assert!((found / 440.0 - 1.0).abs() < 0.01, "{factor}: {found} Hz");
    }
}

#[test]
fn the_same_input_and_seed_give_the_same_output() {
    let input: Vec<f32> = sine(220.0, 0.8, 1)
        .iter()
        .zip(sine(1330.0, 0.8, 1))
        .map(|(a, b)| a + 0.3 * b)
        .collect();
    let run = |seed: u64| {
        let mut stretch = Stretch::new(1, RATE);
        stretch.set_seed(seed);
        stretch.set_transpose_semitones(-3.0, 0.0);
        stretch.set_formant_semitones(2.0, true);
        stretch.process(&input, input.len() * 3)
    };
    let a = run(7);
    assert_eq!(a, run(7));
    assert_ne!(a, run(8));
    assert_eq!(
        pitch_shift(&input, 1, RATE, 4.0),
        pitch_shift(&input, 1, RATE, 4.0)
    );
}
