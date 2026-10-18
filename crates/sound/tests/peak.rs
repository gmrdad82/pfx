use ebur128::{EbuR128, Mode};
use pfx_sound::{Limiter, decibels, true_peak};
use std::f64::consts::PI;

const RATE: u32 = 48_000;

fn tone(hz: f64, amplitude: f64, phase_deg: f64, seconds: f64) -> Vec<f32> {
    let frames = (seconds * f64::from(RATE)) as usize;
    let phase = phase_deg.to_radians();
    let mut out = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let v = (amplitude * (2.0 * PI * hz * n as f64 / f64::from(RATE) + phase).sin()) as f32;
        out.extend([v, v]);
    }
    out
}

fn reference(samples: &[f32]) -> f32 {
    let mut meter = EbuR128::new(2, RATE, Mode::TRUE_PEAK).unwrap();
    meter.add_frames_f32(samples).unwrap();
    let peak = (0..2)
        .map(|c| meter.true_peak(c).unwrap())
        .fold(0.0, f64::max);
    decibels(peak as f32)
}

fn signals() -> Vec<(&'static str, Vec<f32>, f32)> {
    let half = decibels(0.5);
    let mut out = vec![
        (
            "quarter rate at 0 degrees",
            tone(12_000.0, 0.5, 0.0, 1.0),
            half,
        ),
        (
            "quarter rate at 45 degrees",
            tone(12_000.0, 0.5, 45.0, 1.0),
            half,
        ),
        (
            "quarter rate at 60 degrees",
            tone(12_000.0, 0.5, 60.0, 1.0),
            half,
        ),
        (
            "quarter rate at 67.5 degrees",
            tone(12_000.0, 0.5, 67.5, 1.0),
            half,
        ),
        (
            "quarter rate at 45 degrees, samples at full scale",
            tone(12_000.0, 2f64.sqrt(), 45.0, 1.0),
            decibels(2f32.sqrt()),
        ),
        ("1 kHz", tone(1_000.0, 1.0, 30.0, 1.0), 0.0),
        ("17 kHz", tone(17_000.0, 1.0, 0.0, 1.0), 0.0),
        ("9.6 kHz", tone(9_600.0, 1.0, 18.0, 1.0), 0.0),
    ];
    let mut burst = vec![0.0f32; 9_600];
    burst[4_800] = 1.0;
    burst[4_801] = 1.0;
    out.push(("a full-scale click", burst, 0.0));
    out
}

#[test]
fn the_meter_reads_the_known_true_peaks_and_matches_the_reference() {
    for (name, signal, truth) in signals() {
        let ours = decibels(true_peak(&signal, 2));
        let theirs = reference(&signal);
        assert!(
            (truth - 0.4..=truth + 0.4).contains(&ours),
            "{name}: {ours} dBTP for {truth}"
        );
        assert!((ours - theirs).abs() < 0.3, "{name}: {ours} vs {theirs}");
    }
}

fn hot(seconds: f64) -> Vec<f32> {
    let frames = (seconds * f64::from(RATE)) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    let mut state = 0x2545_f491_4f6c_dd1du64;
    for n in 0..frames {
        let t = n as f64 / f64::from(RATE);
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let noise = (state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
        let kick = (-((t * 2.0).fract() * 18.0)).exp() * (2.0 * PI * 55.0 * t).sin();
        let lead = (2.0 * PI * 11_997.0 * t).sin() * 0.6 + (2.0 * PI * 7_001.0 * t).sin() * 0.5;
        let l = 1.3 * kick + lead + 0.2 * noise;
        let r = 1.1 * kick + lead - 0.2 * noise;
        out.push(l.clamp(-1.0, 1.0) as f32);
        out.push(r.clamp(-1.0, 1.0) as f32);
    }
    out
}

#[test]
fn a_hot_mix_masters_under_its_true_peak_ceiling() {
    let mut mix = hot(3.0);
    let before = decibels(true_peak(&mix, 2));
    assert!(before > 1.0, "the test mix overshoots: {before} dBTP");
    let limiter = Limiter::new(-1.0);
    assert!(limiter.master(&mut mix, 2, RATE));
    let after = true_peak(&mix, 2);
    assert!(after <= limiter.ceiling, "{} dBTP", decibels(after));
    assert!(reference(&mix) <= -1.0 + 0.2, "{}", reference(&mix));
    let loud = mix.iter().map(|s| s * s).sum::<f32>() / mix.len() as f32;
    assert!(decibels(loud.sqrt()) > -12.0, "{}", decibels(loud.sqrt()));
    let mut quiet = tone(1_000.0, 0.5, 0.0, 0.5);
    let kept = quiet.clone();
    assert!(!limiter.master(&mut quiet, 2, RATE));
    assert_eq!(quiet, kept);
}
