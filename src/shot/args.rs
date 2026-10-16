use std::path::PathBuf;

use pfx_core::cli;

pub const SWITCHES: &[&str] = &["--draft", "--final", "--json"];
pub const FORMATS: &[&str] = &["png", "exr"];
pub const DEFAULT_SIZE: (u32, u32) = (1920, 1080);
pub const DEFAULT_FPS: u32 = 60;
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_FPS: u32 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Exr,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Exr => "exr",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Draft,
    Final,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub scene: PathBuf,
    pub size: (u32, u32),
    pub fps: u32,
    pub frames: u64,
    pub from: u64,
    pub camera: Option<String>,
    pub quality: Quality,
    pub hour: Option<f64>,
    pub sets: Vec<String>,
    pub out: Option<PathBuf>,
    pub format: Format,
    pub audio: Option<PathBuf>,
    pub sound: Option<PathBuf>,
    pub json: bool,
    pub seed: u64,
    pub shutter: Option<f32>,
}

fn size(text: &str) -> Result<(u32, u32), String> {
    let wrong = || {
        cli::invalid(
            text,
            "--size",
            format!("expected an even WxH between 2 and {MAX_SIDE}"),
        )
    };
    let (width, height) = text.split_once(['x', 'X']).ok_or_else(wrong)?;
    let side = |part: &str| {
        part.parse::<u32>()
            .ok()
            .filter(|value| (2..=MAX_SIDE).contains(value) && value % 2 == 0)
            .ok_or_else(wrong)
    };
    Ok((side(width)?, side(height)?))
}

fn whole<T>(value: &str, flag: &str, low: T, high: T) -> Result<T, String>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display + Copy,
{
    value
        .parse::<T>()
        .ok()
        .filter(|parsed| *parsed >= low && *parsed <= high)
        .ok_or_else(|| {
            cli::invalid(
                value,
                flag,
                format!("expected a whole number from {low} to {high}"),
            )
        })
}

fn real(value: &str, flag: &str, low: f64, high: f64, open_low: bool) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|parsed| {
            parsed.is_finite()
                && *parsed <= high
                && if open_low {
                    *parsed > low
                } else {
                    *parsed >= low
                }
        })
        .ok_or_else(|| {
            let bound = if open_low { "above" } else { "from" };
            cli::invalid(
                value,
                flag,
                format!("expected a number {bound} {low} to {high}"),
            )
        })
}

fn set(value: &str) -> Result<String, String> {
    match value.split_once('=') {
        Some((key, _))
            if !key.is_empty() && key.split('.').all(|segment| !segment.trim().is_empty()) =>
        {
            Ok(value.to_string())
        }
        _ => Err(cli::invalid(value, "--set", "expected dotted.key=value")),
    }
}

pub fn parse(args: &[String], flags: &[&str]) -> Result<Plan, String> {
    let mut scene = None;
    let mut values: Vec<(&str, String)> = Vec::new();
    let mut switches: Vec<&str> = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (word.as_str(), None),
        };
        if !flag.starts_with('-') {
            if scene.is_some() {
                return Err(cli::unexpected(word));
            }
            scene = Some(PathBuf::from(word));
            continue;
        }
        let Some(flag) = flags.iter().copied().find(|known| *known == flag) else {
            return Err(cli::unexpected(flag));
        };
        if SWITCHES.contains(&flag) {
            if let Some(value) = inline {
                return Err(cli::unexpected_value(&value, flag));
            }
            if switches.contains(&flag) {
                return Err(cli::repeated(flag));
            }
            switches.push(flag);
            continue;
        }
        if flag != "--set" && values.iter().any(|(seen, _)| *seen == flag) {
            return Err(cli::repeated(&cli::with_value(flag)));
        }
        let value = match inline {
            Some(value) => value,
            None => words
                .next()
                .cloned()
                .ok_or_else(|| cli::missing_value(flag))?,
        };
        values.push((flag, value));
    }
    let value = |flag: &str| {
        values
            .iter()
            .find(|(seen, _)| *seen == flag)
            .map(|(_, value)| value.as_str())
    };
    if switches.contains(&"--draft") && switches.contains(&"--final") {
        return Err(cli::conflict("--final", "--draft"));
    }
    match (value("--seconds"), value("--frames")) {
        (Some(_), Some(_)) => {
            return Err(cli::conflict(
                &cli::with_value("--frames"),
                &cli::with_value("--seconds"),
            ));
        }
        (None, None) => {
            let mut missing = vec![format!(
                "<{}|{}>",
                cli::with_value("--seconds"),
                cli::with_value("--frames")
            )];
            if scene.is_none() {
                missing.push("<SCENE>".to_string());
            }
            return Err(cli::required(&missing));
        }
        _ => {}
    }
    let Some(scene) = scene else {
        return Err(cli::required(&["<SCENE>"]));
    };
    let fps = match value("--fps") {
        Some(text) => whole(text, "--fps", 1, MAX_FPS)?,
        None => DEFAULT_FPS,
    };
    let frames = match (value("--seconds"), value("--frames")) {
        (Some(text), _) => {
            let seconds = real(text, "--seconds", 0.0, 86_400.0, true)?;
            (seconds * f64::from(fps)).round().max(1.0) as u64
        }
        (_, Some(text)) => whole(text, "--frames", 1, 10_000_000)?,
        _ => 0,
    };
    let format = match value("--format") {
        Some("png") | None => Format::Png,
        Some("exr") => Format::Exr,
        Some(other) => return Err(cli::invalid_choice(other, "--format", FORMATS)),
    };
    Ok(Plan {
        scene,
        size: value("--size")
            .map(size)
            .transpose()?
            .unwrap_or(DEFAULT_SIZE),
        fps,
        frames,
        from: value("--from")
            .map(|text| whole(text, "--from", 0, 10_000_000))
            .transpose()?
            .unwrap_or(0),
        camera: value("--camera").map(str::to_string),
        quality: if switches.contains(&"--final") {
            Quality::Final
        } else {
            Quality::Draft
        },
        hour: value("--hour")
            .map(|text| real(text, "--hour", 0.0, 24.0, false))
            .transpose()?,
        sets: values
            .iter()
            .filter(|(flag, _)| *flag == "--set")
            .map(|(_, text)| set(text))
            .collect::<Result<_, _>>()?,
        out: value("--out").map(PathBuf::from),
        format,
        audio: value("--audio").map(PathBuf::from),
        sound: value("--sound").map(PathBuf::from),
        json: switches.contains(&"--json"),
        seed: value("--seed")
            .map(|text| {
                text.parse::<u64>()
                    .map_err(|error| cli::invalid(text, "--seed", error))
            })
            .transpose()?
            .unwrap_or(0),
        shutter: value("--shutter")
            .map(|text| real(text, "--shutter", 0.0, 1.0, true).map(|open| open as f32))
            .transpose()?,
    })
}
