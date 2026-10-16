use pfx_sound::shot::{self, RATE, Script};
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/car-pass.sound.toml")
}

#[test]
fn the_mix_is_exactly_the_clip_long() {
    for (frames, fps) in [(600, 60), (37, 24), (1, 30), (250, 25)] {
        let out = shot::render(&example(), frames, fps).unwrap();
        let expected = frames as u64 * u64::from(RATE) / u64::from(fps);
        assert_eq!(out.len() as u64, expected * 2, "{frames} at {fps}");
    }
    assert!(shot::render(&example(), 10, 0).is_err());
}

#[test]
fn the_same_script_gives_the_same_samples() {
    let a = shot::render(&example(), 240, 60).unwrap();
    let b = shot::render(&example(), 240, 60).unwrap();
    assert_eq!(a, b);
    assert!(a.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(a.iter().any(|s| s.abs() > 0.01));
    let text = std::fs::read_to_string(example()).unwrap();
    let other = Script::parse(&text.replace("seed = 7", "seed = 8"), Path::new("")).unwrap();
    assert_ne!(other.mix(a.len() / 2).unwrap(), a);
}

fn cycle(signal: &[f32], expected: usize) -> usize {
    let lo = expected * 6 / 10;
    let hi = expected * 16 / 10;
    let span = signal.len() - hi;
    (lo..=hi)
        .max_by(|&a, &b| {
            let score = |lag: usize| {
                let mut dot = 0.0f64;
                let mut energy = 0.0f64;
                for i in 0..span {
                    dot += f64::from(signal[i]) * f64::from(signal[i + lag]);
                    energy += f64::from(signal[i + lag]).powi(2);
                }
                dot / energy.sqrt().max(1e-12)
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap()
}

#[test]
fn the_engine_pitch_follows_the_rpm_curve() {
    let script = Script::parse(
        r#"
        [[curve]]
        name = "rpm"
        keys = [[0.0, 2000], [1.0, 2000], [1.4, 5000], [3.0, 5000]]
        [[engine]]
        rpm = "rpm"
        cylinders = 6
        "#,
        Path::new(""),
    )
    .unwrap();
    let out = script.mix(3 * RATE as usize).unwrap();
    let mono: Vec<f32> = out.chunks_exact(2).map(|f| f[0] + f[1]).collect();
    for (from, to, rpm) in [(0.3, 0.9, 2000.0), (2.0, 2.8, 5000.0)] {
        let window = &mono[(from * RATE as f32) as usize..(to * RATE as f32) as usize];
        let expected = (120.0 / rpm * RATE as f32) as usize;
        let found = cycle(window, expected);
        let error = (found as f32 - expected as f32).abs() / expected as f32;
        assert!(
            error < 0.02,
            "{rpm} rpm: cycle {found} frames, expected {expected}"
        );
    }
}
