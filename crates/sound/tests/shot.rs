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

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shot-recording");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn save(path: &Path, bytes: &[u8]) {
    let part = path.with_extension(format!("{:?}.part", std::thread::current().id()));
    std::fs::write(&part, bytes).unwrap();
    std::fs::rename(&part, path).unwrap();
}

fn engine_recording() -> PathBuf {
    let path = scratch("engine.wav");
    let rpm_at = |t: f32| match t {
        t if t < 2.0 => 900.0,
        t if t < 2.5 => 900.0 + (t - 2.0) / 0.5 * 2100.0,
        t if t < 3.5 => 3000.0,
        t if t < 4.0 => 3000.0 - (t - 3.5) / 0.5 * 2100.0,
        _ => 900.0,
    };
    let accents = [1.0f32, 0.72, 0.9, 0.58];
    let mut crank = 0.0f64;
    let mut state = 0x1234_5678u32;
    let samples: Vec<f32> = (0..(4.5 * RATE as f32) as usize)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            crank += f64::from(rpm_at(t)) / 120.0 / f64::from(RATE);
            let fire = crank.fract() * 4.0;
            let cylinder = fire as usize % 4;
            let theta = fire.fract() as f32;
            let pulse = (-theta * 7.0).exp()
                * ((std::f32::consts::TAU * theta * 3.0).sin()
                    + 0.4 * (std::f32::consts::TAU * theta * 7.0).sin());
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            0.4 * accents[cylinder] * pulse + 0.01 * noise
        })
        .collect();
    let clip = pfx_sound::Clip {
        rate: RATE,
        channels: 1,
        samples,
    };
    save(&path, &clip.to_wav());
    path
}

fn engine_script(extra: &str, curve: &str) -> Script {
    let path = engine_recording();
    Script::parse(
        &format!(
            r#"
            seed = 3
            [[curve]]
            name = "rpm"
            keys = {curve}
            [[engine]]
            rpm = "rpm"
            sample = "{}"
            idle = [0.1, 1.9]
            idle_rpm = 900
            revs = [[3000, 2.6, 3.4]]
            {extra}
            "#,
            path.file_name().unwrap().to_str().unwrap()
        ),
        path.parent().unwrap(),
    )
    .unwrap()
}

fn mono(out: &[f32]) -> Vec<f32> {
    out.chunks_exact(2).map(|f| f[0] + f[1]).collect()
}

#[test]
fn a_recorded_engine_mixes_to_the_exact_length_and_repeats_by_seed() {
    let curve = "[[0.0, 900], [1.0, 2500], [2.0, 4000]]";
    let script = engine_script("cylinders = 4\nblend = 0.3", curve);
    for frames in [1, 37, 120] {
        let n = shot::audio_frames(frames, 60);
        assert_eq!(script.mix(n).unwrap().len(), n * 2);
    }
    let a = script.mix(RATE as usize * 2).unwrap();
    assert_eq!(a, script.mix(RATE as usize * 2).unwrap());
    assert!(a.iter().all(|s| s.is_finite()) && a.iter().any(|s| s.abs() > 0.01));
    let mut other = script.clone();
    other.seed = 4;
    assert_ne!(other.mix(RATE as usize * 2).unwrap(), a);
    let shots = engine_script("crank = [4.0, 4.4]\nshutdown = [1.5, 1.9]", curve);
    let b = mono(&shots.mix(RATE as usize * 2).unwrap());
    assert_eq!(b.len(), RATE as usize * 2);
    assert!(b.iter().all(|s| s.is_finite()));
    assert!(rms(&b[..RATE as usize / 5]) > 0.01 && rms(&b[RATE as usize * 17 / 10..]) > 0.01);
}

#[test]
fn a_recorded_engine_follows_the_rpm_curve_past_its_recording() {
    let curve = "[[0.0, 900], [1.0, 900], [1.5, 2000], [2.5, 2000], [3.0, 3000], [4.0, 3000], [4.5, 5500], [6.0, 5500]]";
    for extra in ["cylinders = 4\nblend = 0.0", ""] {
        let out = engine_script(extra, curve).mix(6 * RATE as usize).unwrap();
        let mono = mono(&out);
        for (from, to, rpm) in [
            (0.3, 0.9, 900.0),
            (1.8, 2.4, 2000.0),
            (3.3, 3.9, 3000.0),
            (5.0, 5.9, 5500.0),
        ] {
            let window = &mono[(from * RATE as f32) as usize..(to * RATE as f32) as usize];
            let expected = (30.0 / rpm * RATE as f32) as usize;
            let found = cycle(window, expected);
            let error = (found as f32 - expected as f32).abs() / expected as f32;
            assert!(
                error < 0.03,
                "`{extra}` at {rpm} rpm: firing period {found} frames, expected {expected}"
            );
        }
    }
}

#[test]
fn an_unmarked_recording_finds_its_idle_and_its_rpm() {
    let path = engine_recording();
    let text = format!(
        "[[engine]]\nrpm = 900\nsample = \"{}\"\n",
        path.file_name().unwrap().to_str().unwrap()
    );
    let script = Script::parse(&text, path.parent().unwrap()).unwrap();
    let out = mono(&script.mix(RATE as usize).unwrap());
    let found = cycle(&out[RATE as usize / 10..], 1600);
    assert!(
        (found as f32 - 1600.0).abs() < 1600.0 * 0.03,
        "firing period {found}"
    );
}

fn steepest(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|x| x * x).sum::<f32>() / signal.len() as f32).sqrt()
}

#[test]
fn grain_seams_of_a_recorded_idle_do_not_click() {
    let script = engine_script("drive = 0\nlowpass = 20000", "[[0.0, 900], [4.0, 900]]");
    let out = mono(&script.mix(4 * RATE as usize).unwrap());
    let body = &out[RATE as usize / 10..];
    let bytes = std::fs::read(engine_recording()).unwrap();
    let source = pfx_sound::Clip::from_wav(&bytes).unwrap().samples;
    let idle = &source[RATE as usize / 10..(1.9 * RATE as f32) as usize];
    let ours = steepest(body) / rms(body);
    let theirs = steepest(idle) / rms(idle);
    assert!(
        ours < theirs * 1.5,
        "steepest step {ours} against the recording's {theirs}"
    );
}

fn tone(name: &str, seconds: f32) -> PathBuf {
    let path = scratch(name);
    let frames = (seconds * RATE as f32) as usize;
    let samples = (0..frames)
        .flat_map(|i| {
            let v = 0.5 * (std::f32::consts::TAU * 220.0 * i as f32 / RATE as f32).sin();
            [v, v]
        })
        .collect();
    let clip = pfx_sound::Clip {
        rate: RATE,
        channels: 2,
        samples,
    };
    save(&path, &clip.to_wav());
    path
}

#[test]
fn a_looped_sample_crossfades_its_seam_and_trims_hold() {
    let path = tone("tone.wav", 0.3013);
    let dir = path.parent().unwrap();
    let mix = |keys: &str| {
        let text = format!("[[sample]]\npath = \"tone.wav\"\n{keys}\n");
        let out = Script::parse(&text, dir)
            .unwrap()
            .mix(2 * RATE as usize)
            .unwrap();
        out.chunks_exact(2).map(|f| f[0]).collect::<Vec<f32>>()
    };
    let own = 0.5 * std::f32::consts::TAU * 220.0 / RATE as f32;
    let smooth = mix("loop = true\nloop_crossfade = 0.02");
    assert!(rms(&smooth[(1.5 * RATE as f32) as usize..]) > 0.2);
    assert!(
        steepest(&smooth) < own * 1.5,
        "{} against {own}",
        steepest(&smooth)
    );
    let hard = mix("loop = true\nloop_crossfade = 0");
    assert!(steepest(&hard) > own * 2.0);
    let trimmed = mix("start = 0.1\nend = 0.2\nat = 0.5\nfade_in = 0.01\nfade_out = 0.01");
    let first = trimmed.iter().position(|s| *s != 0.0).unwrap();
    let last = trimmed.iter().rposition(|s| *s != 0.0).unwrap();
    assert!(first >= RATE as usize / 2 && last < RATE as usize * 6 / 10);
    assert!(steepest(&trimmed) < own * 1.2);
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_recorded_engine() {
    let curve = "[[0.0, 900], [2.0, 900], [6.0, 6000], [8.0, 2500], [10.0, 900]]";
    for extra in ["", "keep_formants = false", "cylinders = 4\nblend = 0.3"] {
        let script = engine_script(extra, curve);
        let best = (0..5)
            .map(|_| {
                let started = std::time::Instant::now();
                assert_eq!(
                    script.mix(10 * RATE as usize).unwrap().len(),
                    20 * RATE as usize
                );
                started.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::MAX, f64::min);
        println!("{:<28} {best:7.2} ms for 10 s", extra.replace('\n', " "));
    }
}
