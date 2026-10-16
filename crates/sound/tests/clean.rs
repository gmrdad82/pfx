use std::f64::consts::TAU;

use pfx_core::sim::Rng;
use pfx_sound::Clip;
use pfx_sound::clean::{self, Gate, Harmonic, Options};

const RATE: u32 = 48_000;
const SECONDS: f64 = 10.0;
const BARKS: [f64; 4] = [1.5, 3.2, 5.8, 8.1];
const BARK_SECONDS: f64 = 0.25;

fn len() -> usize {
    (SECONDS * f64::from(RATE)) as usize
}

fn f0(t: f64) -> f64 {
    let x = t / SECONDS;
    45.0 + 30.0 * x * x * (3.0 - 2.0 * x) + (TAU * 0.3 * t).sin()
}

fn engine(from: f64) -> Vec<f32> {
    engine_band(from, 1, 1000)
}

fn engine_band(from: f64, first: usize, last: usize) -> Vec<f32> {
    let mut phase = 0.0f64;
    let mut out = vec![0.0f32; len()];
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f64 / f64::from(RATE);
        let f = f0(t);
        phase += TAU * f / f64::from(RATE);
        if t < from {
            continue;
        }
        let fade = ((t - from) / 0.05).min(1.0);
        let count = (4000.0 / f) as usize;
        let mut s = 0.0;
        for h in first..=count.min(last) {
            let h = h as f64;
            let top = ((4000.0 - h * f) / 1000.0).clamp(0.0, 1.0);
            s += h.powf(-0.7) * top * (h * phase + h * h * 0.37).sin();
        }
        *o = (0.06 * s * fade) as f32;
    }
    out
}

fn formant(hz: f64) -> f64 {
    let peak =
        |centre: f64, width: f64, gain: f64| gain * (-0.5 * ((hz - centre) / width).powi(2)).exp();
    peak(1000.0, 200.0, 1.0) + peak(2200.0, 300.0, 0.6) + peak(3300.0, 400.0, 0.35)
}

fn barks() -> Vec<f32> {
    let mut out = vec![0.0f32; len()];
    for (n, &start) in BARKS.iter().enumerate() {
        let first = (start * f64::from(RATE)) as usize;
        let count = (BARK_SECONDS * f64::from(RATE)) as usize;
        let mut phase = 0.0f64;
        for i in 0..count {
            let t = i as f64 / f64::from(RATE);
            let pitch = 650.0 * (480.0f64 / 650.0).powf(t / BARK_SECONDS) * (1.0 + 0.03 * n as f64);
            phase += TAU * pitch / f64::from(RATE);
            let attack = (t / 0.008).min(1.0);
            let decay = (-(t - 0.008).max(0.0) / 0.07).exp();
            let tail = ((BARK_SECONDS - t) / 0.02).clamp(0.0, 1.0);
            let env = attack * decay * (0.5 - 0.5 * (std::f64::consts::PI * tail).cos());
            let mut s = 0.0;
            let mut h = 1.0;
            while h * pitch < 6000.0 {
                s += formant(h * pitch) * (h * phase + h * 1.1).sin();
                h += 1.0;
            }
            out[first + i] += (0.25 * env * s) as f32;
        }
    }
    out
}

fn wind(seed: u64, gusts: bool) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let a = 1.0 - (-TAU * 250.0 / f64::from(RATE)).exp();
    let (mut y1, mut y2) = (0.0f64, 0.0f64);
    (0..len())
        .map(|i| {
            let t = i as f64 / f64::from(RATE);
            let white = rng.next_f64() * 2.0 - 1.0;
            y1 += a * (white - y1);
            y2 += a * (y1 - y2);
            let gust = if gusts {
                1.0 + 0.3 * (TAU * 0.2 * t).sin()
            } else {
                1.0
            };
            (y2 * 0.9 * gust) as f32
        })
        .collect()
}

fn hiss(seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let a = 1.0 - (-TAU * 6000.0 / f64::from(RATE)).exp();
    let mut y = 0.0f64;
    (0..len())
        .map(|_| {
            y += a * (rng.next_f64() * 2.0 - 1.0 - y);
            (y * 0.02) as f32
        })
        .collect()
}

fn mono(samples: Vec<f32>) -> Clip {
    Clip {
        rate: RATE,
        channels: 1,
        samples,
    }
}

fn sum(parts: &[&[f32]]) -> Clip {
    mono(
        (0..len())
            .map(|i| parts.iter().map(|p| p[i]).sum())
            .collect(),
    )
}

fn energy(samples: &[f32], from: f64, to: f64) -> f64 {
    let a = ((from * f64::from(RATE)) as usize).min(samples.len());
    let b = ((to * f64::from(RATE)) as usize).min(samples.len());
    samples[a..b].iter().map(|s| f64::from(*s).powi(2)).sum()
}

fn db(before: f64, after: f64) -> f64 {
    10.0 * (before / after.max(1e-30)).log10()
}

fn through(masks: &clean::Masks, samples: &[f32]) -> Vec<f32> {
    masks.apply(&mono(samples.to_vec())).unwrap().samples
}

fn bark_drops(masks: &clean::Masks, bark: &[f32]) -> Vec<f64> {
    let out = through(masks, bark);
    BARKS
        .iter()
        .map(|&s| {
            let (a, b) = (s - 0.05, s + BARK_SECONDS + 0.1);
            db(energy(bark, a, b), energy(&out, a, b))
        })
        .collect()
}

fn settled(out: &[f32], rest: &[f32], bark: &[f32]) -> (f64, Vec<f64>, Vec<f64>) {
    let whole = db(energy(rest, 0.0, SECONDS), energy(out, 0.0, SECONDS));
    let error: Vec<f32> = out.iter().zip(rest).map(|(o, r)| o - r).collect();
    let mut inside = Vec::new();
    let mut residual = Vec::new();
    for &s in &BARKS {
        let (a, b) = (s - 0.05, s + BARK_SECONDS + 0.1);
        inside.push(db(energy(rest, a, b), energy(out, a, b)));
        residual.push(db(energy(bark, a, b), energy(&error, a, b)));
    }
    (whole, inside, residual)
}

fn engine_change(masks: &clean::Masks, engine: &[f32]) -> f64 {
    let out = through(masks, engine);
    db(energy(engine, 0.0, SECONDS), energy(&out, 0.0, SECONDS))
}

#[test]
fn transients_drop_the_barks_and_keep_the_engine() {
    let (e, b, w) = (engine(0.0), barks(), wind(7, true));
    let mix = sum(&[&e, &b, &w]);
    let options = Options {
        transients: true,
        ..Options::default()
    };
    let cleaned = clean::clean(&mix, &options).unwrap();
    assert_eq!(cleaned.clip.samples.len(), mix.samples.len());
    let events = cleaned.report.events.clone().unwrap();
    let drops = bark_drops(&cleaned.masks, &b);
    let rest: Vec<f32> = e.iter().zip(&w).map(|(a, b)| a + b).collect();
    let (engine, inside, residual) = settled(&cleaned.clip.samples, &rest, &b);
    println!("transients: events {events:?}");
    println!(
        "transients: barks down {drops:.1?} dB, engine and wind {engine:.2} dB ({inside:.2?} dB in the barks), what is left against them {residual:.1?} dB"
    );
    assert!(drops.iter().all(|d| *d >= 30.0), "{drops:?}");
    assert!(engine.abs() <= 1.0, "{engine}");
    assert_eq!(events.len(), BARKS.len());
    for (event, &start) in events.iter().zip(&BARKS) {
        assert!(
            event.start <= start as f32 + 0.03 && event.end >= (start + BARK_SECONDS) as f32 - 0.08,
            "{event:?}"
        );
    }
}

#[test]
fn keep_harmonic_follows_the_engine_and_drops_the_wind() {
    let (e, w) = (engine(0.0), wind(11, true));
    let mix = sum(&[&e, &w]);
    let options = Options {
        harmonic: Some(Harmonic::default()),
        ..Options::default()
    };
    let cleaned = clean::clean(&mix, &options).unwrap();
    let report = cleaned.report.harmonic.clone().unwrap();
    let engine = engine_change(&cleaned.masks, &e);
    let wind = engine_change(&cleaned.masks, &w);
    println!("harmonic: {report:?}, engine {engine:.2} dB, wind down {wind:.1} dB");
    assert!(report.low_hz > 43.0 && report.low_hz < 47.0, "{report:?}");
    assert!(report.high_hz > 73.0 && report.high_hz < 78.0, "{report:?}");
    assert!(engine.abs() <= 1.0, "{engine}");
    assert!(wind >= 5.0, "{wind}");
}

#[test]
fn the_gate_lowers_steady_noise_by_its_amount() {
    let (e, w) = (engine(4.0), hiss(3));
    let mix = sum(&[&e, &w]);
    for reduce in [6.0f32, 12.0] {
        let options = Options {
            gate: Some(Gate {
                noise: None,
                reduce_db: Some(reduce),
            }),
            ..Options::default()
        };
        let cleaned = clean::clean(&mix, &options).unwrap();
        let report = cleaned.report.gate.clone().unwrap();
        assert!(report.auto && report.noise.1 <= 4.0, "{report:?}");
        let noise_out = through(&cleaned.masks, &w);
        let noise = db(energy(&w, 0.5, 3.5), energy(&noise_out, 0.5, 3.5));
        let engine_out = through(&cleaned.masks, &e);
        let engine = db(energy(&e, 4.5, 9.5), energy(&engine_out, 4.5, 9.5));
        println!("gate {reduce} dB: noise down {noise:.2} dB, engine {engine:.2} dB, {report:?}");
        assert!((noise - f64::from(reduce)).abs() <= 1.5, "{noise}");
        assert!(engine.abs() <= 1.0, "{engine}");
    }
}

#[test]
fn every_mode_together_cleans_the_field_recording() {
    let (e, b) = (engine(1.0), barks());
    let w: Vec<f32> = wind(5, true)
        .iter()
        .zip(hiss(9))
        .map(|(a, h)| a * 0.25 + h)
        .collect();
    let mix = sum(&[&e, &b, &w]);
    let options = Options {
        harmonic: Some(Harmonic::default()),
        transients: true,
        gate: Some(Gate {
            noise: Some((0.1, 0.9)),
            reduce_db: Some(10.0),
        }),
    };
    let started = std::time::Instant::now();
    let cleaned = clean::clean(&mix, &options).unwrap();
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cleaned.clip.samples.len(), mix.samples.len());
    let drops = bark_drops(&cleaned.masks, &b);
    let engine = engine_change(&cleaned.masks, &e);
    let wind = engine_change(&cleaned.masks, &w);
    let out = &cleaned.clip.samples;
    let level = db(energy(&e, 1.0, SECONDS), energy(out, 1.0, SECONDS));
    println!(
        "all: barks down {drops:.1?} dB, engine {engine:.2} dB through the gains, output against the engine {level:.2} dB, wind down {wind:.1} dB, {ms:.0} ms"
    );
    assert!(drops.iter().all(|d| *d >= 30.0), "{drops:?}");
    assert!(engine.abs() <= 1.0, "{engine}");
    let again = clean::clean(&mix, &options).unwrap();
    assert_eq!(again.clip.samples, cleaned.clip.samples);
    let preview = cleaned.preview();
    assert_eq!(
        preview.rgb.len(),
        (preview.width * preview.height * 3) as usize
    );
}

#[test]
#[ignore]
fn cost_of_ten_seconds() {
    let (e, b) = (engine(1.0), barks());
    let w: Vec<f32> = wind(5, true)
        .iter()
        .zip(hiss(9))
        .map(|(a, h)| a * 0.25 + h)
        .collect();
    let mix = sum(&[&e, &b, &w]);
    let gate = Some(Gate::default());
    for (name, options) in [
        (
            "keep harmonic",
            Options {
                harmonic: Some(Harmonic::default()),
                ..Options::default()
            },
        ),
        (
            "remove transients",
            Options {
                transients: true,
                ..Options::default()
            },
        ),
        (
            "gate",
            Options {
                gate,
                ..Options::default()
            },
        ),
        (
            "all three",
            Options {
                harmonic: Some(Harmonic::default()),
                transients: true,
                gate,
            },
        ),
    ] {
        let started = std::time::Instant::now();
        let cleaned = clean::clean(&mix, &options).unwrap();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = std::time::Instant::now();
        let preview = cleaned.preview();
        let preview_ms = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "cost {name}: {ms:.0} ms, preview {preview_ms:.0} ms ({}x{})",
            preview.width, preview.height
        );
    }
}
