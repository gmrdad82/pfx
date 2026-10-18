use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx::harness::tools;
use pfx_core::cli::{self, Refusal};
use pfx_film::reel::{Planned, Roll};
use pfx_film::write::{AUDIO, Encoder, FRAMES, Files, STILLS};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::progress::{self, Progress};

pub const USAGE: &str = "pfx film [OPTIONS] --out <OUT> <SCRIPT>";
pub const MANIFEST: &str = "manifest.json";
const MP4: &str = "film.mp4";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub script: PathBuf,
    pub out: Option<PathBuf>,
    pub json: bool,
    pub plan: bool,
}

pub fn plan(args: &[String]) -> Result<Command, Refusal> {
    let refuse = |message: String| Refusal::new(message).under(USAGE);
    let mut script = None;
    let mut out = None;
    let mut json = false;
    let mut plan = false;
    let mut words = args.iter();
    while let Some(arg) = words.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        match flag {
            "--json" | "--plan" if inline.is_some() => {
                return Err(refuse(cli::unexpected(arg)));
            }
            "--json" if json => return Err(refuse(cli::repeated(flag))),
            "--plan" if plan => return Err(refuse(cli::repeated(flag))),
            "--json" => json = true,
            "--plan" => plan = true,
            "--out" if out.is_some() => return Err(refuse(cli::repeated(&cli::with_value(flag)))),
            "--out" => {
                let value = inline
                    .or_else(|| words.next().cloned())
                    .filter(|v| !v.is_empty() && !v.starts_with('-'))
                    .ok_or_else(|| Refusal::new(cli::missing_value(flag)))?;
                out = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') => return Err(refuse(cli::unexpected(arg))),
            _ if script.is_some() => return Err(refuse(cli::unexpected(arg))),
            file => script = Some(PathBuf::from(file)),
        }
    }
    let script = match (script, out.is_some() || plan) {
        (Some(script), true) => script,
        (None, true) => return Err(refuse(cli::required(&["<SCRIPT>"]))),
        (Some(_), false) => return Err(refuse(cli::required(&["--out <OUT>"]))),
        (None, false) => return Err(refuse(cli::required(&["--out <OUT>", "<SCRIPT>"]))),
    };
    Ok(Command {
        script,
        out,
        json,
        plan,
    })
}

fn quoted(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:@%,".contains(c))
    {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn pcod(roll: &Roll, out: &Path) -> String {
    roll.pcod(out, &out.join(MP4))
        .iter()
        .map(|w| quoted(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn scenes(plan: &[Planned]) -> Value {
    plan.iter()
        .map(|p| {
            json!({
                "name": p.name,
                "kind": p.kind.name(),
                "seconds": p.seconds,
                "from": p.from,
                "frames": p.frames,
                "source": p.source.as_ref().map(|s| s.display().to_string()),
            })
        })
        .collect()
}

fn head(roll: &Roll) -> Value {
    json!({
        "film": 1,
        "script": roll.path.display().to_string(),
        "frames": roll.frames(),
        "fps": roll.fps(),
        "size": roll.size(),
        "seconds": roll.frames() as f64 / f64::from(roll.fps()),
        "scenes": scenes(&roll.plan()),
    })
}

fn inside_tmp(out: &Path) -> Result<PathBuf, String> {
    let tmp = tools::caller_tmp()?;
    let out = std::path::absolute(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut probe = out.as_path();
    while !probe.exists() {
        probe = probe
            .parent()
            .ok_or_else(|| format!("{} has no existing parent", out.display()))?;
    }
    let real = probe
        .canonicalize()
        .map_err(|e| format!("{}: {e}", probe.display()))?;
    let rest = out.strip_prefix(probe).unwrap_or(Path::new(""));
    let real = real.join(rest);
    if !real.starts_with(&tmp) || real == tmp {
        return Err(format!(
            "{} is outside the caller's tmp/ ({}); a film writes only under it",
            out.display(),
            tmp.display()
        ));
    }
    Ok(real)
}

struct Shown<'a> {
    files: Files,
    progress: &'a mut Progress,
}

impl Encoder for Shown<'_> {
    fn frame(&mut self, index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.files.frame(index, rgba, size)?;
        self.progress.frame(index + 1);
        Ok(())
    }

    fn still(&mut self, name: &str, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.files.still(name, rgba, size)
    }

    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String> {
        self.files.finish(audio, rate, channels)
    }
}

fn shoot(roll: &Roll, out: &Path, json: bool) -> Result<pfx_film::Reeled, String> {
    let mut progress = Progress::new("pfx film", roll.frames(), !json && progress::live());
    if !progress.shows() {
        return roll.shoot(out);
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let film = &roll.script.film;
    let files = Files::open(out, film.frames, roll.script.stills() > 0)?;
    let mut shown = Shown {
        files,
        progress: &mut progress,
    };
    roll.roll(Some(&mut shown))
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

pub fn run(command: &Command) -> Result<(), String> {
    let script = std::path::absolute(&command.script)
        .map_err(|e| format!("{}: {e}", command.script.display()))?;
    let roll = Roll::open(&script)?;
    if command.plan {
        let misses = roll.script.off_grid(roll.fps())?;
        let mut plan = head(&roll);
        if !misses.is_empty() {
            plan["off_grid"] = json!(misses);
        }
        if let Some(out) = &command.out {
            let out = std::path::absolute(out).map_err(|e| format!("{}: {e}", out.display()))?;
            plan["pcod"] = json!(pcod(&roll, &out));
        }
        if command.json {
            println!("{plan}");
        } else {
            for p in roll.plan() {
                let source = p
                    .source
                    .map(|s| format!(" {}", s.display()))
                    .unwrap_or_default();
                println!(
                    "{} {} {:.3}s from {:.3}s, {} frames{source}",
                    p.name,
                    p.kind.name(),
                    p.seconds,
                    p.from,
                    p.frames
                );
            }
            let [w, h] = roll.size();
            println!("{} frames at {w}x{h}, {} fps", roll.frames(), roll.fps());
            for miss in &misses {
                println!("off grid: {miss}");
            }
        }
        return Ok(());
    }
    let out = command.out.as_ref().ok_or("a film needs --out")?;
    let out = inside_tmp(out)?;
    let started = Instant::now();
    let reeled = shoot(&roll, &out, command.json)?;
    let total = started.elapsed().as_secs_f64() * 1000.0;
    let film = &roll.script.film;
    let mut manifest = head(&roll);
    let wav = film.audio.then(|| out.join(AUDIO).display().to_string());
    let extra = json!({
        "script_sha256": sha256(&script)?,
        "pfx": env!("CARGO_PKG_VERSION"),
        "format": "png",
        "out": out.display().to_string(),
        "frames_dir": film.frames.then(|| out.join(FRAMES).display().to_string()),
        "stills_dir": (reeled.stills > 0).then(|| out.join(STILLS).display().to_string()),
        "stills": reeled.stills,
        "wav": wav,
        "samples": reeled.samples,
        "rate": pfx_film::RATE,
        "pcod": pcod(&roll, &out),
        "wall_ms": {
            "total": total,
            "per_frame": total / reeled.frames.max(1) as f64,
        },
    });
    if let (Some(m), Value::Object(e)) = (manifest.as_object_mut(), extra) {
        m.extend(e);
    }
    let path = out.join(MANIFEST);
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{text}\n")).map_err(|e| format!("{}: {e}", path.display()))?;
    if command.json {
        println!("{manifest}");
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn the_film_command_takes_a_script_an_out_and_its_switches() {
        let c = plan(&words("a.film.toml --out tmp/f --json")).unwrap();
        assert_eq!(c.script, PathBuf::from("a.film.toml"));
        assert_eq!(c.out, Some(PathBuf::from("tmp/f")));
        assert!(c.json && !c.plan);
        assert!(plan(&words("a.film.toml --plan")).unwrap().plan);
        assert!(plan(&words("--out=tmp/f a.film.toml")).is_ok());
        for bad in [
            "",
            "a.film.toml",
            "--out tmp/f",
            "a.film.toml --out",
            "a.film.toml b.film.toml --out tmp/f",
            "a.film.toml --out tmp/f --json --json",
            "a.film.toml --out tmp/f --fast",
        ] {
            assert!(plan(&words(bad)).is_err(), "{bad}");
        }
    }
}
