use pfx_core::rate::Rate;
use pfx_sound::shot::{self, RATE, Script};
use std::path::{Path, PathBuf};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/car-pass.sound.toml")
}

#[test]
fn the_mix_is_exactly_the_clip_long() {
    for (frames, fps) in [(600, 60), (37, 24), (1, 30), (250, 25)] {
        let out = shot::render(&example(), frames, Rate::whole(fps)).unwrap();
        let expected = frames as u64 * u64::from(RATE) / u64::from(fps);
        assert_eq!(out.len() as u64, expected * 2, "{frames} at {fps}");
    }
    let film: Rate = "24000/1001".parse().unwrap();
    let out = shot::render(&example(), 37, film).unwrap();
    assert_eq!(out.len(), 37 * 2002 * 2);
}

#[test]
fn the_same_script_gives_the_same_samples() {
    let a = shot::render(&example(), 240, Rate::whole(60)).unwrap();
    let b = shot::render(&example(), 240, Rate::whole(60)).unwrap();
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
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = path.with_extension(format!("{}-{n}.part", std::process::id()));
    std::fs::write(&part, bytes).unwrap();
    std::fs::rename(&part, path).unwrap();
}

fn engine_recording() -> PathBuf {
    static MADE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    MADE.get_or_init(record_engine).clone()
}

fn record_engine() -> PathBuf {
    let path = scratch(&format!("engine-{}.wav", std::process::id()));
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
        let n = shot::audio_frames(frames, Rate::whole(60));
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

fn road(keys: &str, seconds: usize) -> Vec<f32> {
    let text = format!("seed = 9\n[[road]]\nspeed = 10\n{keys}\n");
    let script = Script::parse(&text, Path::new("")).unwrap();
    mono(&script.mix(seconds * RATE as usize).unwrap())
}

#[test]
fn asphalt_is_the_road_it_always_was() {
    let plain = road("", 1);
    assert_eq!(road("surface = \"asphalt\"", 1), plain);
    assert_eq!(
        road("surface = [[0.0, \"asphalt\"], [0.5, \"asphalt\"]]", 1),
        plain
    );
    assert_ne!(road("surface = \"wet\"", 1), plain);
}

#[test]
fn cobbles_thump_at_the_stone_spacing_over_the_speed() {
    for (spacing, expected) in [(0.5, 2400), (0.8, 3840)] {
        let out = road(
            &format!("roughness = 0\nsurface = \"cobbles\"\nspacing = {spacing}"),
            2,
        );
        let found = cycle(&out[RATE as usize / 4..], expected);
        let error = (found as f32 - expected as f32).abs() / expected as f32;
        assert!(
            error < 0.03,
            "spacing {spacing}: {found} frames, expected {expected}"
        );
    }
}

#[test]
fn the_same_surfaces_give_the_same_samples() {
    let keys = "load = 0.6\nsurface = [[0.0, \"gravel\"], [0.4, \"cobbles\"], [0.8, \"wet\"]]";
    let a = road(keys, 1);
    assert_eq!(a, road(keys, 1));
    assert!(a.iter().all(|s| s.is_finite()) && rms(&a) > 0.01);
    let text = format!("seed = 10\n[[road]]\nspeed = 10\n{keys}\n");
    let other = Script::parse(&text, Path::new("")).unwrap();
    assert_ne!(mono(&other.mix(RATE as usize).unwrap()), a);
}

fn tyre_script(seed: u64, keys: &str, slip: &str) -> Result<Script, String> {
    let text = format!(
        "seed = {seed}\n[[curve]]\nname = \"slip\"\nkeys = {slip}\n[[tyre]]\nslip = \"slip\"\n{keys}\n"
    );
    Script::parse(&text, Path::new(""))
}

fn tyre(keys: &str, slip: &str, seconds: f32) -> Vec<f32> {
    let frames = (seconds * RATE as f32) as usize;
    mono(&tyre_script(5, keys, slip).unwrap().mix(frames).unwrap())
}

fn span(signal: &[f32], from: f32, to: f32) -> &[f32] {
    let at = |t: f32| ((t * RATE as f32) as usize).min(signal.len());
    &signal[at(from)..at(to)]
}

const SLIDE: &str = "[[0.0, 0.0], [0.5, 0.0], [0.6, 6.0], [1.4, 9.0], [1.6, 0.0], [2.0, 0.0]]";

#[test]
fn a_tyre_repeats_by_seed_and_curve() {
    let a = tyre("", SLIDE, 2.0);
    assert_eq!(a, tyre("", SLIDE, 2.0));
    assert!(a.iter().all(|s| s.is_finite()) && rms(&a) > 0.01);
    let other = tyre_script(6, "", SLIDE).unwrap();
    assert_ne!(mono(&other.mix(2 * RATE as usize).unwrap()), a);
    let skid = Script::parse(
        &format!(
            "seed = 5\n[[curve]]\nname = \"slip\"\nkeys = {SLIDE}\n[[skid]]\nslip = \"slip\"\n"
        ),
        Path::new(""),
    )
    .unwrap();
    assert_eq!(mono(&skid.mix(2 * RATE as usize).unwrap()), a);
}

#[test]
fn a_tyre_squeals_only_above_its_threshold() {
    let only = "scrub = 0\nchirp = 0\nthreshold = 2\nrange = 2";
    let under = tyre(only, "[[0.0, 1.9], [1.0, 1.9]]", 1.0);
    assert!(under.iter().all(|&s| s == 0.0));
    let over = tyre(only, "[[0.0, 4.0], [1.0, 4.0]]", 1.0);
    assert!(rms(span(&over, 0.2, 1.0)) > 0.05);
    let back = tyre(
        only,
        "[[0.0, 4.0], [0.5, 4.0], [0.55, 0.5], [2.5, 0.5]]",
        2.5,
    );
    assert!(rms(span(&back, 0.2, 0.5)) > 0.05);
    assert!(span(&back, 2.0, 2.5).iter().all(|&s| s == 0.0));
}

#[test]
fn a_tyre_chirps_at_the_onset_of_each_slide_only() {
    let only = "scrub = 0\nsqueal = 0\ngrip = 0.5\nthreshold = 2";
    let slip = "[[0.0, 0.0], [1.0, 0.0], [1.05, 5.0], [2.0, 5.0], [2.05, 0.0], [3.0, 0.0], [3.05, 5.0], [4.5, 5.0]]";
    let out = tyre(only, slip, 4.5);
    assert!(span(&out, 0.0, 1.0).iter().all(|&s| s == 0.0));
    assert!(rms(span(&out, 1.0, 1.15)) > 0.02);
    assert!(span(&out, 1.9, 3.0).iter().all(|&s| s == 0.0));
    assert!(rms(span(&out, 3.0, 3.15)) > 0.02);
    assert!(span(&out, 3.9, 4.5).iter().all(|&s| s == 0.0));
    let started = tyre(only, "[[0.0, 5.0], [1.0, 5.0]]", 1.0);
    assert!(started.iter().all(|&s| s == 0.0));
}

#[test]
fn a_tyre_scrubs_louder_as_it_slips_faster() {
    let only = "chirp = 0\nsqueal = 0";
    let level = |slip: f32| {
        let out = tyre(only, &format!("[[0.0, {slip}], [1.0, {slip}]]"), 1.0);
        rms(span(&out, 0.2, 1.0))
    };
    let still = tyre(only, "[[0.0, 0.0], [1.0, 0.0]]", 1.0);
    assert!(still.iter().all(|&s| s == 0.0));
    let levels = [level(1.0), level(4.0), level(12.0)];
    assert!(levels[0] > 0.001, "{levels:?}");
    assert!(levels[1] > levels[0] * 1.5, "{levels:?}");
    assert!(levels[2] > levels[1] * 1.5, "{levels:?}");
    let heavy = tyre(
        &format!("{only}\nload = 2"),
        "[[0.0, 4.0], [1.0, 4.0]]",
        1.0,
    );
    assert!(rms(span(&heavy, 0.2, 1.0)) > levels[1]);
}

#[test]
fn a_tyre_refuses_values_out_of_bounds() {
    let refused = |keys: &str, words: &str| {
        let err = tyre_script(1, keys, "[[0.0, 1.0]]").unwrap_err();
        assert!(err.contains("[[tyre]] 1") && err.contains(words), "{err}");
    };
    refused("hz = 9000", "`hz` must be 200 to 4000 Hz, not 9000");
    refused("threshold = 0", "`threshold` must be 0.05 to 40 m/s");
    refused(
        "grip = 3\nthreshold = 2",
        "`grip` (3 m/s) must not be above `threshold` (2 m/s)",
    );
    refused("load = -1", "`load` must be 0 to 4");
    refused("squeal = 5", "`squeal` must be 0 to 4");
    refused("whine = 1", "whine");
    let text = "[[tyre]]\nslip = -2\n";
    let err = Script::parse(text, Path::new("")).unwrap_err();
    assert!(err.contains("`slip` must be 0 to 200 m/s"), "{err}");
    let err = Script::parse("[[tyre]]\nslip = \"nope\"\n", Path::new("")).unwrap_err();
    assert!(err.contains("no curve named `nope`"), "{err}");
}

fn band_power(signal: &[f32], lo: f32, hi: f32, step: f32) -> f64 {
    let mut total = 0.0;
    let mut hz = lo;
    while hz <= hi {
        let w = std::f64::consts::TAU * f64::from(hz) / f64::from(RATE);
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in signal {
            let s = f64::from(x) + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        total += s1 * s1 + s2 * s2 - coeff * s1 * s2;
        hz += step;
    }
    total
}

fn filtered_road(keys: &str) -> Vec<f32> {
    let text = format!(
        "seed = 4\n[[curve]]\nname = \"cut\"\nkeys = [[0.0, 130.0], [0.5, 400.0], [1.0, 130.0]]\n[[road]]\nspeed = 20\n{keys}\n"
    );
    let script = Script::parse(&text, Path::new("")).unwrap();
    mono(&script.mix(2 * RATE as usize).unwrap())
}

#[test]
fn a_layer_highpass_removes_the_band_below_it() {
    let plain = filtered_road("");
    let cut = filtered_road("highpass = 130");
    let body = |s: &[f32]| span(s, 0.25, 2.0).to_vec();
    let (plain, cut) = (body(&plain), body(&cut));
    let low = band_power(&cut, 20.0, 65.0, 5.0) / band_power(&plain, 20.0, 65.0, 5.0);
    assert!(low < 0.1, "below 65 Hz kept {low}");
    let high = band_power(&cut, 600.0, 1_500.0, 50.0) / band_power(&plain, 600.0, 1_500.0, 50.0);
    assert!((0.8..1.25).contains(&high), "600 to 1500 Hz kept {high}");
    let dull = body(&filtered_road("lowpass = 1000\nlowpass_q = 0.5"));
    let top =
        band_power(&dull, 4_000.0, 6_000.0, 100.0) / band_power(&plain, 4_000.0, 6_000.0, 100.0);
    assert!(top < 0.1, "above 4 kHz kept {top}");
}

#[test]
fn a_keyed_layer_filter_repeats_and_moves() {
    let keys = "highpass = \"cut\"\nhighpass_q = 1.2\nlowpass = 8000";
    let a = filtered_road(keys);
    assert_eq!(a, filtered_road(keys));
    assert!(a.iter().all(|s| s.is_finite()) && rms(&a) > 0.01);
    assert_ne!(
        a,
        filtered_road("highpass = 130\nhighpass_q = 1.2\nlowpass = 8000")
    );
    assert_ne!(a, filtered_road(""));
}

#[test]
fn layer_filters_refuse_out_of_bounds() {
    let refused = |layer: &str, keys: &str, words: &str| {
        let err = Script::parse(&format!("[[{layer}]]\n{keys}\n"), Path::new("")).unwrap_err();
        assert!(err.contains(words), "{err}");
    };
    refused("road", "highpass = 5", "`highpass` must be 10 to 24000 Hz");
    refused(
        "road",
        "lowpass = 30000",
        "`lowpass` must be 10 to 24000 Hz",
    );
    refused(
        "road",
        "highpass = 100\nhighpass_q = 50",
        "`highpass_q` must be 0.1 to 10",
    );
    refused("road", "lowpass_q = 1", "`lowpass_q` needs `lowpass`");
    refused("road", "highpass = \"none\"", "no curve named `none`");
    refused("engine", "lowpass_q = 1", "unknown key `lowpass_q`");
    let engine =
        Script::parse("[[engine]]\nlowpass = 3000\nhighpass = 40\n", Path::new("")).unwrap();
    assert!(engine.layers[0].common.lowpass.is_none());
    assert!(engine.layers[0].common.highpass.is_some());
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
