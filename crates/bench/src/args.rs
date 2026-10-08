use std::path::PathBuf;

use pfx_core::cli;

pub const DEFAULT_SIZE: (u32, u32) = (1920, 1080);

const FLAGS: &[&str] = &[
    "--seconds",
    "--camera",
    "--replay",
    "--size",
    "--stats",
    "--seed",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub scene: PathBuf,
    pub seconds: f64,
    pub camera: Option<String>,
    pub replay: Option<PathBuf>,
    pub size: Option<(u32, u32)>,
    pub stats: Option<PathBuf>,
    pub seed: u64,
}

fn size(text: &str) -> Result<(u32, u32), String> {
    let wrong = || cli::invalid(text, "--size", "expected WxH between 1 and 16384");
    let (width, height) = text.split_once(['x', 'X']).ok_or_else(wrong)?;
    let number = |part: &str| {
        part.parse::<u32>()
            .ok()
            .filter(|value| (1..=16_384).contains(value))
            .ok_or_else(wrong)
    };
    Ok((number(width)?, number(height)?))
}

pub fn parse(args: &[String]) -> Result<Plan, String> {
    let mut scene = None;
    let mut seconds = None;
    let mut camera = None;
    let mut replay = None;
    let mut size_flag = None;
    let mut stats = None;
    let mut seed = 0;
    let mut seen = Vec::new();
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
        if !FLAGS.contains(&flag) {
            return Err(cli::unexpected(flag));
        }
        if seen.contains(&flag) {
            return Err(cli::repeated(&cli::with_value(flag)));
        }
        seen.push(flag);
        let value = match inline {
            Some(value) => value,
            None => words
                .next()
                .cloned()
                .ok_or_else(|| cli::missing_value(flag))?,
        };
        match flag {
            "--seconds" => {
                let parsed = value
                    .parse::<f64>()
                    .ok()
                    .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
                    .ok_or_else(|| cli::invalid(&value, flag, "expected a number above 0"))?;
                seconds = Some(parsed);
            }
            "--camera" => camera = Some(value),
            "--replay" => replay = Some(PathBuf::from(value)),
            "--size" => size_flag = Some(size(&value)?),
            "--stats" => stats = Some(PathBuf::from(value)),
            _ => {
                seed = value
                    .parse::<u64>()
                    .map_err(|error| cli::invalid(&value, flag, error))?;
            }
        }
    }
    let mut missing = Vec::new();
    if seconds.is_none() {
        missing.push(cli::with_value("--seconds"));
    }
    if scene.is_none() {
        missing.push("<SCENE>".to_string());
    }
    if !missing.is_empty() {
        return Err(cli::required(&missing));
    }
    Ok(Plan {
        scene: scene.unwrap_or_default(),
        seconds: seconds.unwrap_or_default(),
        camera,
        replay,
        size: size_flag,
        stats,
        seed,
    })
}
