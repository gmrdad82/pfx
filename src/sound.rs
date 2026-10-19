use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_core::cli::{self, Refusal};
use pfx_sound::Clip;
use pfx_sound::clean::{self, Cleaned, Gate, Harmonic, Options, Preview, Report};
use serde_json::{Value, json};

pub const CLEAN: &str = "pfx sound clean [OPTIONS] <IN> <OUT>";
pub const MIX: &str = "pfx sound mix [OPTIONS] --frames <FRAMES> --out <OUT> <SCRIPT>";
const COMMANDS: &[&str] = &["clean", "mix"];

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Clean(Clean),
    #[cfg(feature = "shot-sound")]
    Mix(Mix),
}

#[cfg(feature = "shot-sound")]
#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    pub script: PathBuf,
    pub frames: u64,
    pub fps: pfx_core::rate::Rate,
    pub output: PathBuf,
    pub json: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clean {
    pub input: PathBuf,
    pub output: PathBuf,
    pub options: Options,
    pub mask: Option<PathBuf>,
    pub json: bool,
}

fn pair(value: &str, flag: &str, what: &str) -> Result<(f32, f32), String> {
    let parsed = value.split_once(',').and_then(|(a, b)| {
        let a: f32 = a.trim().parse().ok()?;
        let b: f32 = b.trim().parse().ok()?;
        (a.is_finite() && b.is_finite() && a < b).then_some((a, b))
    });
    parsed.ok_or_else(|| {
        cli::invalid(
            value,
            flag,
            format!("expected {what}, the first below the second"),
        )
    })
}

pub fn plan(args: &[String]) -> Result<Command, Refusal> {
    match args.first().map(String::as_str) {
        None => Err(cli::no_command("pfx sound", COMMANDS).into()),
        Some("clean") => plan_clean(args).map(Command::Clean),
        #[cfg(feature = "shot-sound")]
        Some("mix") => plan_mix(&args[1..]).map(Command::Mix),
        #[cfg(not(feature = "shot-sound"))]
        Some("mix") => Err(Refusal::new(
            "this pfx was built without the offline mix (the shot-sound feature)",
        )),
        Some(flag) if flag.starts_with('-') => Err(cli::unexpected(flag).into()),
        Some(word) => Err(cli::unrecognized(word).into()),
    }
}

#[cfg(feature = "shot-sound")]
fn plan_mix(args: &[String]) -> Result<Mix, Refusal> {
    let refuse = |message: String| Refusal::new(message).under(MIX);
    let mut script = None;
    let mut frames = None;
    let mut fps = None;
    let mut output = None;
    let mut json = false;
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--json" if inline.is_some() => return Err(refuse(cli::unexpected(arg))),
            "--json" if json => return Err(refuse(cli::repeated(flag))),
            "--json" => json = true,
            "--frames" | "--fps" | "--out" => {
                let value = inline
                    .or_else(|| words.next().cloned())
                    .filter(|v| !v.is_empty() && !v.starts_with("--"))
                    .ok_or_else(|| Refusal::new(cli::missing_value(flag)))?;
                let slot = match flag {
                    "--frames" => &mut frames,
                    "--fps" => &mut fps,
                    _ => &mut output,
                };
                if slot.is_some() {
                    return Err(refuse(cli::repeated(&cli::with_value(flag))));
                }
                *slot = Some(value);
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(refuse(cli::unexpected(arg)));
            }
            _ if script.is_some() => return Err(refuse(cli::unexpected(arg))),
            file => script = Some(PathBuf::from(file)),
        }
    }
    let mut missing = Vec::new();
    if frames.is_none() {
        missing.push(cli::with_value("--frames"));
    }
    if output.is_none() {
        missing.push(cli::with_value("--out"));
    }
    if script.is_none() {
        missing.push("<SCRIPT>".to_string());
    }
    if !missing.is_empty() {
        return Err(refuse(cli::required(&missing)));
    }
    let whole = |value: &str, flag: &str, high: u64| {
        value
            .parse::<u64>()
            .ok()
            .filter(|n| (1..=high).contains(n))
            .ok_or_else(|| {
                Refusal::new(cli::invalid(
                    value,
                    flag,
                    format!("expected a whole number from 1 to {high}"),
                ))
            })
    };
    let frames = whole(
        frames.as_deref().unwrap_or_default(),
        "--frames",
        10_000_000,
    )?;
    let fps = match &fps {
        Some(value) => crate::shot::rate(value, "--fps").map_err(Refusal::new)?,
        None => crate::shot::DEFAULT_FPS,
    };
    Ok(Mix {
        script: script.unwrap_or_default(),
        frames,
        fps,
        output: output.map(PathBuf::from).unwrap_or_default(),
        json,
    })
}

fn plan_clean(args: &[String]) -> Result<Clean, Refusal> {
    let refuse = |message: String| Refusal::new(message).under(CLEAN);
    let mut files: Vec<PathBuf> = Vec::new();
    let mut keep = None;
    let mut f0 = None;
    let mut remove = None;
    let mut gate = false;
    let mut noise = None;
    let mut reduce = None;
    let mut mask = None;
    let mut json = false;
    let mut words = args[1..].iter();
    while let Some(arg) = words.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--json" | "--gate" if inline.is_some() => return Err(refuse(cli::unexpected(arg))),
            "--json" if json => return Err(refuse(cli::repeated(flag))),
            "--gate" if gate => return Err(refuse(cli::repeated(flag))),
            "--json" => json = true,
            "--gate" => gate = true,
            "--keep" | "--f0" | "--remove" | "--noise" | "--reduce" | "--mask" => {
                let value = inline
                    .or_else(|| words.next().cloned())
                    .filter(|v| !v.is_empty() && !v.starts_with("--"))
                    .ok_or_else(|| Refusal::new(cli::missing_value(flag)))?;
                let slot = match flag {
                    "--keep" => &mut keep,
                    "--f0" => &mut f0,
                    "--remove" => &mut remove,
                    "--noise" => &mut noise,
                    "--reduce" => &mut reduce,
                    _ => &mut mask,
                };
                if slot.is_some() {
                    return Err(refuse(cli::repeated(&cli::with_value(flag))));
                }
                *slot = Some(value);
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(refuse(cli::unexpected(arg)));
            }
            _ if files.len() == 2 => return Err(refuse(cli::unexpected(arg))),
            file => files.push(PathBuf::from(file)),
        }
    }
    if let Some(value) = &keep
        && value != "harmonic"
    {
        return Err(Refusal::new(cli::invalid_choice(
            value,
            "--keep",
            &["harmonic"],
        )));
    }
    if let Some(value) = &remove
        && value != "transients"
    {
        return Err(Refusal::new(cli::invalid_choice(
            value,
            "--remove",
            &["transients"],
        )));
    }
    if f0.is_some() && keep.is_none() {
        return Err(refuse(cli::required(&["--keep <KEEP>"])));
    }
    if (noise.is_some() || reduce.is_some()) && !gate {
        return Err(refuse(cli::required(&["--gate"])));
    }
    let harmonic = match (&keep, &f0) {
        (None, _) => None,
        (Some(_), None) => Some(Harmonic::default()),
        (Some(_), Some(value)) => {
            let (low_hz, high_hz) =
                pair(value, "--f0", "LOW,HIGH in Hz above 0").map_err(Refusal::new)?;
            if low_hz <= 0.0 {
                return Err(Refusal::new(cli::invalid(
                    value,
                    "--f0",
                    "the range must start above 0 Hz",
                )));
            }
            Some(Harmonic { low_hz, high_hz })
        }
    };
    let noise = match &noise {
        None => None,
        Some(value) => {
            let range = pair(value, "--noise", "START,END in seconds").map_err(Refusal::new)?;
            if range.0 < 0.0 {
                return Err(Refusal::new(cli::invalid(
                    value,
                    "--noise",
                    "the print starts at 0 s or later",
                )));
            }
            Some(range)
        }
    };
    let reduce_db = match &reduce {
        None => None,
        Some(value) => Some(
            value
                .parse::<f32>()
                .ok()
                .filter(|db| db.is_finite() && *db > 0.0)
                .ok_or_else(|| {
                    Refusal::new(cli::invalid(value, "--reduce", "expected decibels above 0"))
                })?,
        ),
    };
    let options = Options {
        harmonic,
        transients: remove.is_some(),
        gate: gate.then_some(Gate { noise, reduce_db }),
    };
    if files.len() < 2 {
        let missing: Vec<&str> = ["<IN>", "<OUT>"][files.len()..].to_vec();
        return Err(refuse(cli::required(&missing)));
    }
    if options == Options::default() {
        return Err(refuse(cli::required(&[
            "<--keep <KEEP>|--remove <REMOVE>|--gate>",
        ])));
    }
    Ok(Clean {
        output: files.pop().unwrap_or_default(),
        input: files.pop().unwrap_or_default(),
        options,
        mask: mask.map(PathBuf::from),
        json,
    })
}

fn bits(bytes: &[u8]) -> Option<u16> {
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        if &bytes[at..at + 4] == b"fmt " && at + 24 <= bytes.len() {
            return Some(u16::from_le_bytes([bytes[at + 22], bytes[at + 23]]));
        }
        at = at.checked_add(8 + len + (len & 1))?;
    }
    None
}

fn write_png(path: &Path, preview: &Preview) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), preview.width, preview.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    writer
        .write_image_data(&preview.rgb)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn round(v: f32, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (f64::from(v) * scale).round() / scale
}

fn report_json(command: &Clean, report: &Report, ms: f64) -> Value {
    json!({
        "clean": 1,
        "input": command.input.display().to_string(),
        "output": command.output.display().to_string(),
        "mask": command.mask.as_ref().map(|m| m.display().to_string()),
        "seconds": round(report.seconds, 3),
        "rate": report.rate,
        "channels": report.channels,
        "input_db": round(report.input_db, 2),
        "output_db": round(report.output_db, 2),
        "ms": (ms * 10.0).round() / 10.0,
        "harmonic": report.harmonic.as_ref().map(|h| json!({
            "low_hz": round(h.low_hz, 2),
            "median_hz": round(h.median_hz, 2),
            "high_hz": round(h.high_hz, 2),
            "voiced": round(h.voiced, 3),
            "removed_db": round(h.removed_db, 2),
        })),
        "transients": report.events.as_ref().map(|events| events.iter().map(|e| json!({
            "start": round(e.start, 3),
            "end": round(e.end, 3),
            "removed_db": round(e.removed_db, 2),
        })).collect::<Vec<_>>()),
        "gate": report.gate.as_ref().map(|g| json!({
            "noise": [round(g.noise.0, 3), round(g.noise.1, 3)],
            "auto": g.auto,
            "reduce_db": g.reduce_db,
            "removed_db": round(g.removed_db, 2),
            "noise_removed_db": round(g.noise_removed_db, 2),
        })),
    })
}

fn report_text(command: &Clean, report: &Report, ms: f64) -> String {
    let mut lines = vec![format!(
        "cleaned {} -> {}: {:.3} s, {} Hz, {} channel{}, {:.1} dBFS -> {:.1} dBFS, {ms:.0} ms",
        command.input.display(),
        command.output.display(),
        report.seconds,
        report.rate,
        report.channels,
        if report.channels == 1 { "" } else { "s" },
        report.input_db,
        report.output_db,
    )];
    if let Some(h) = &report.harmonic {
        lines.push(format!(
            "harmonic: fundamental {:.1}..{:.1} Hz (median {:.1}), voiced {:.1} %, removed {:.1} dB",
            h.low_hz,
            h.high_hz,
            h.median_hz,
            h.voiced * 100.0,
            h.removed_db
        ));
    }
    if let Some(events) = &report.events {
        lines.push(format!("transients: {} removed", events.len()));
        for e in events {
            lines.push(format!(
                "  {:.3}..{:.3} s  {:.1} dB",
                e.start, e.end, e.removed_db
            ));
        }
    }
    if let Some(g) = &report.gate {
        let amount = g
            .reduce_db
            .map_or("fully".to_string(), |db| format!("by {db} dB"));
        lines.push(format!(
            "gate: print {:.3}..{:.3} s ({}), gating {amount}, removed {:.1} dB ({:.1} dB in the print)",
            g.noise.0,
            g.noise.1,
            if g.auto { "the quietest second" } else { "--noise" },
            g.removed_db,
            g.noise_removed_db
        ));
    }
    if let Some(mask) = &command.mask {
        lines.push(format!("mask: {}", mask.display()));
    }
    lines.join("\n")
}

pub fn run(command: &Command) -> Result<(), String> {
    match command {
        Command::Clean(clean) => run_clean(clean),
        #[cfg(feature = "shot-sound")]
        Command::Mix(mix) => run_mix(mix),
    }
}

#[cfg(feature = "shot-sound")]
fn run_mix(mix: &Mix) -> Result<(), String> {
    let started = Instant::now();
    crate::shot::script_wav(&mix.script, mix.frames, mix.fps, &mix.output)?;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let frames = crate::shot::audio_frames(mix.frames, mix.fps);
    let seconds = frames as f64 / f64::from(crate::shot::RATE);
    if mix.json {
        println!(
            "{}",
            json!({
                "mix": 1,
                "script": mix.script.display().to_string(),
                "output": mix.output.display().to_string(),
                "frames": mix.frames,
                "fps": crate::shot::fps_value(mix.fps),
                "frame_rate": mix.fps.to_string(),
                "samples": frames,
                "seconds": (seconds * 1000.0).round() / 1000.0,
                "rate": crate::shot::RATE,
                "channels": crate::shot::CHANNELS,
                "ms": (ms * 10.0).round() / 10.0,
            })
        );
    } else {
        println!(
            "mixed {} -> {}: {} frames at {} fps, {seconds:.3} s, {} Hz stereo, {ms:.0} ms",
            mix.script.display(),
            mix.output.display(),
            mix.frames,
            mix.fps.label(),
            crate::shot::RATE,
        );
    }
    Ok(())
}

fn run_clean(command: &Clean) -> Result<(), String> {
    let path = &command.input;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let clip = Clip::from_wav(&bytes).ok_or_else(|| {
        format!(
            "{}: not a WAV of 8-bit, 16-bit or float samples",
            path.display()
        )
    })?;
    let started = Instant::now();
    let cleaned =
        clean::clean(&clip, &command.options).map_err(|e| format!("{}: {e}", path.display()))?;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let Cleaned {
        clip: out, report, ..
    } = &cleaned;
    let bytes_out = if bits(&bytes) == Some(16) {
        out.to_wav_i16()
    } else {
        out.to_wav()
    };
    std::fs::write(&command.output, bytes_out)
        .map_err(|e| format!("{}: {e}", command.output.display()))?;
    if let Some(mask) = &command.mask {
        write_png(mask, &cleaned.preview())?;
    }
    if command.json {
        println!("{}", report_json(command, report, ms));
    } else {
        println!("{}", report_text(command, report, ms));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[cfg(feature = "shot-sound")]
    #[test]
    fn mix_takes_its_flags_and_refuses_like_clap() {
        let Command::Mix(mix) = plan(&words(
            "mix a.sound.toml --frames 90 --fps 30 --out b.wav --json",
        ))
        .unwrap() else {
            panic!("not a mix");
        };
        assert_eq!(mix.script, PathBuf::from("a.sound.toml"));
        assert_eq!(
            (mix.frames, mix.fps, mix.json),
            (90, pfx_core::rate::Rate::whole(30), true)
        );
        assert_eq!(mix.output, PathBuf::from("b.wav"));
        let Command::Mix(mix) = plan(&words("mix a.sound.toml --frames=12 --out=b.wav")).unwrap()
        else {
            panic!("not a mix");
        };
        assert_eq!(mix.fps, crate::shot::DEFAULT_FPS);
        let refusal = |line: &str| plan(&words(line)).unwrap_err();
        assert_eq!(refusal("").message, cli::no_command("pfx sound", COMMANDS));
        let missing = refusal("mix");
        assert_eq!(
            missing.message,
            cli::required(&["--frames <FRAMES>", "--out <OUT>", "<SCRIPT>"])
        );
        assert_eq!(missing.usage.as_deref(), Some(MIX));
        assert_eq!(
            refusal("mix a.sound.toml --out b.wav").message,
            cli::required(&["--frames <FRAMES>"])
        );
        assert_eq!(
            refusal("mix a.sound.toml --frames 1 --out b.wav c").message,
            cli::unexpected("c")
        );
        assert_eq!(
            refusal("mix a.sound.toml --frames 1 --frames 2 --out b.wav").message,
            cli::repeated("--frames <FRAMES>")
        );
        assert_eq!(
            refusal("mix a.sound.toml --frames 1 --out b.wav --seconds 2").message,
            cli::unexpected("--seconds")
        );
        assert_eq!(
            refusal("mix a.sound.toml --out b.wav --frames").message,
            cli::missing_value("--frames")
        );
        assert!(
            refusal("mix a.sound.toml --frames 0 --out b.wav")
                .message
                .starts_with("invalid value '0' for '--frames <FRAMES>'")
        );
        assert!(
            refusal("mix a.sound.toml --frames 1 --fps 241 --out b.wav")
                .message
                .starts_with("invalid value '241' for '--fps <FPS>'")
        );
    }

    #[cfg(feature = "shot-sound")]
    #[test]
    fn mix_writes_the_wav_pfx_shot_writes_for_the_script() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let script = root.join("crates/sound/examples/car-pass.sound.toml");
        let dir = root.join("tmp/sound-mix-test");
        let _ = std::fs::remove_dir_all(&dir);
        for (frames, fps) in [(90u64, 30u32), (37, 24)] {
            let mixed = dir.join(format!("mix-{frames}-{fps}.wav"));
            let line = format!(
                "mix {} --frames {frames} --fps {fps} --out {}",
                script.display(),
                mixed.display()
            );
            run(&plan(&words(&line)).unwrap()).unwrap();
            let shot_out = dir.join(format!("shot-{frames}-{fps}"));
            let line = format!(
                "scene.pitofx --frames {frames} --fps {fps} --sound {}",
                script.display()
            );
            let shot = crate::shot::parse(&words(&line), crate::SHOT_FLAGS).unwrap();
            let (wav, _) = crate::shot::audio(&shot, Some(&script), &shot_out).unwrap();
            let shot_bytes = std::fs::read(wav.unwrap()).unwrap();
            let mix_bytes = std::fs::read(&mixed).unwrap();
            assert_eq!(mix_bytes, shot_bytes, "{frames} frames at {fps} fps");
            let clip = Clip::from_wav(&mix_bytes).unwrap();
            assert_eq!((clip.rate, clip.channels), (48_000, 2));
            assert_eq!(
                clip.frames() as usize,
                crate::shot::audio_frames(frames, pfx_core::rate::Rate::whole(fps))
            );
            assert!(clip.samples.iter().any(|s| s.abs() > 0.01));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clean_takes_its_modes_and_refuses_like_clap() {
        let Command::Clean(command) = plan(&words("clean a.wav b.wav --keep harmonic --f0 30,200 --remove transients --gate --noise 0,1.5 --reduce 12 --mask m.png --json")).unwrap() else {
            panic!("not a clean");
        };
        assert_eq!(command.input, PathBuf::from("a.wav"));
        assert_eq!(command.output, PathBuf::from("b.wav"));
        assert_eq!(
            command.options.harmonic,
            Some(Harmonic {
                low_hz: 30.0,
                high_hz: 200.0
            })
        );
        assert!(command.options.transients);
        assert_eq!(
            command.options.gate,
            Some(Gate {
                noise: Some((0.0, 1.5)),
                reduce_db: Some(12.0)
            })
        );
        assert!(command.json);
        let refusal = |line: &str| plan(&words(line)).unwrap_err().message;
        assert_eq!(refusal(""), cli::no_command("pfx sound", COMMANDS));
        assert_eq!(refusal("tidy"), cli::unrecognized("tidy"));
        assert_eq!(
            refusal("clean a.wav b.wav"),
            cli::required(&["<--keep <KEEP>|--remove <REMOVE>|--gate>"])
        );
        assert_eq!(refusal("clean a.wav --gate"), cli::required(&["<OUT>"]));
        assert_eq!(
            refusal("clean a.wav b.wav --keep wind"),
            cli::invalid_choice("wind", "--keep", &["harmonic"])
        );
        assert_eq!(
            refusal("clean a.wav b.wav --remove transients --noise 0,1"),
            cli::required(&["--gate"])
        );
        assert_eq!(
            refusal("clean a.wav b.wav --gate --gate"),
            cli::repeated("--gate")
        );
        assert_eq!(
            refusal("clean a.wav b.wav c.wav --gate"),
            cli::unexpected("c.wav")
        );
        assert_eq!(
            refusal("clean a.wav b.wav --gate --reduce"),
            cli::missing_value("--reduce")
        );
        assert!(
            refusal("clean a.wav b.wav --gate --noise 2,1")
                .starts_with("invalid value '2,1' for '--noise <NOISE>'")
        );
    }
}
