use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx::harness::tools;
use pfx_core::cli::{self, Refusal};
use pfx_film::reel::{Planned, Roll};
use pfx_film::write::{AUDIO, Encoder, FRAMES, Files, Resume, STILLS};
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
    pub memory_limit: Option<u64>,
}

pub fn plan(args: &[String]) -> Result<Command, Refusal> {
    let refuse = |message: String| Refusal::new(message).under(USAGE);
    let mut script = None;
    let mut out = None;
    let mut json = false;
    let mut plan = false;
    let mut memory_limit = None;
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
            "--memory-limit" if memory_limit.is_some() => {
                return Err(refuse(cli::repeated(&cli::with_value(flag))));
            }
            "--memory-limit" => {
                let value = inline
                    .or_else(|| words.next().cloned())
                    .filter(|v| !v.is_empty() && !v.starts_with('-'))
                    .ok_or_else(|| Refusal::new(cli::missing_value(flag)))?;
                memory_limit = Some(
                    pfx_core::memory::gigabytes(&value)
                        .map_err(|why| refuse(cli::invalid(&value, flag, why)))?,
                );
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
        memory_limit,
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
        "fps": crate::shot::fps_value(roll.fps()),
        "frame_rate": roll.fps().to_string(),
        "size": roll.size(),
        "seconds": roll.fps().time(roll.frames()),
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
    json: bool,
    of: u64,
    began: Instant,
    last: Option<Instant>,
}

impl Shown<'_> {
    fn told(&mut self, index: u64) {
        if !self.json {
            return;
        }
        let now = Instant::now();
        let last = index + 1 == self.of;
        if !last
            && self
                .last
                .is_some_and(|at| now < at + std::time::Duration::from_secs(1))
        {
            return;
        }
        self.last = Some(now);
        let elapsed = self.began.elapsed().as_secs_f64();
        let eta = elapsed / (index + 1) as f64 * self.of.saturating_sub(index + 1) as f64;
        eprintln!(
            "{}",
            json!({
                "stage": "frame",
                "frame": index,
                "of": self.of,
                "elapsed_s": (elapsed * 10.0).round() / 10.0,
                "eta_s": (eta * 10.0).round() / 10.0,
            })
        );
    }
}

impl Encoder for Shown<'_> {
    fn frame(&mut self, index: u64, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.files.frame(index, rgba, size)?;
        self.progress.frame(index + 1);
        self.told(index);
        pfx_core::memory::look_gpu();
        Ok(())
    }

    fn kept(&self, index: u64) -> bool {
        self.files.kept(index)
    }

    fn still(&mut self, name: &str, rgba: Vec<u8>, size: [u32; 2]) -> Result<(), String> {
        self.files.still(name, rgba, size)
    }

    fn finish(&mut self, audio: Option<&[f32]>, rate: u32, channels: u16) -> Result<(), String> {
        self.files.finish(audio, rate, channels)
    }
}

fn print(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    if !meta.is_dir() {
        return Ok(format!("{} {stamp}", meta.len()));
    }
    let mut names: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| entry.is_file())
        .collect();
    names.sort();
    let mut hash = Sha256::new();
    for name in &names {
        hash.update(name.file_name().unwrap_or_default().as_encoded_bytes());
        hash.update(print(name)?.as_bytes());
    }
    Ok(format!(
        "{} files {}",
        names.len(),
        hex::encode(hash.finalize())
    ))
}

fn claim(roll: &Roll, script: &str) -> Result<String, String> {
    let [w, h] = roll.size();
    let mut text = format!(
        "pfx film\npfx {}\nscript sha256 {script}\nsize {w}x{h}\nfps {}\nframes {}\n",
        env!("CARGO_PKG_VERSION"),
        roll.fps(),
        roll.frames()
    );
    for planned in roll.plan() {
        if let Some(source) = planned.source {
            text.push_str(&format!(
                "source {} {}\n",
                source.display(),
                print(&source)?
            ));
        }
    }
    Ok(text)
}

fn resuming(done: u64, of: u64, first: u64) -> String {
    format!("resuming: {done} of {of} frames done, starting at frame {first}")
}

fn shoot(
    roll: &Roll,
    out: &Path,
    script: &str,
    json: bool,
) -> Result<(pfx_film::Reeled, pfx_core::memory::Peak), String> {
    let mut progress = Progress::new("pfx film", roll.frames(), !json && progress::live());
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let film = &roll.script.film;
    let claim = claim(roll, script)?;
    let files = Files::resume(
        out,
        film.frames,
        roll.script.stills() > 0,
        Resume {
            claim: &claim,
            frames: roll.frames(),
            size: roll.size(),
        },
    )?;
    let done = files.done();
    if done > 0 {
        let first = files.first_missing();
        eprintln!("pfx film: {}", resuming(done, roll.frames(), first));
        if json {
            eprintln!(
                "{}",
                json!({"stage": "resume", "done": done, "of": roll.frames(), "from": first})
            );
        }
    }
    let mut shown = Shown {
        files,
        progress: &mut progress,
        json,
        of: roll.frames(),
        began: Instant::now(),
        last: None,
    };
    let reeled = roll.roll(Some(&mut shown))?;
    Ok((reeled, pfx_core::memory::peak()))
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn gigabytes(bytes: Option<u64>) -> Value {
    bytes.map_or(Value::Null, |bytes| {
        json!((bytes as f64 / pfx_core::memory::GB as f64 * 100.0).round() / 100.0)
    })
}

pub fn run(command: &Command) -> Result<(), String> {
    pfx_core::memory::from_env(command.memory_limit)?;
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
            println!(
                "{} frames at {w}x{h}, {} fps",
                roll.frames(),
                roll.fps().label()
            );
            for miss in &misses {
                println!("off grid: {miss}");
            }
        }
        return Ok(());
    }
    let out = command.out.as_ref().ok_or("a film needs --out")?;
    let out = inside_tmp(out)?;
    let memory = pfx_core::memory::plan(
        &pfx_core::memory::Census::default(),
        pfx_core::memory::Run::Live { size: roll.size() },
    );
    memory.admit()?;
    let started = Instant::now();
    let script_sha = sha256(&script)?;
    let (reeled, peak) = shoot(&roll, &out, &script_sha, command.json)?;
    let total = started.elapsed().as_secs_f64() * 1000.0;
    let film = &roll.script.film;
    let mut manifest = head(&roll);
    let wav = film.audio.then(|| out.join(AUDIO).display().to_string());
    let extra = json!({
        "script_sha256": script_sha,
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
        "memory": {
            "limit_gb": gigabytes(pfx_core::memory::limit()),
            "planned_cpu_gb": gigabytes(Some(memory.cpu())),
            "planned_gpu_gb": gigabytes(Some(memory.gpu())),
            "cpu_peak_gb": gigabytes(peak.cpu),
            "gpu_peak_gb": gigabytes(peak.gpu),
        },
    });
    if let (Some(m), Value::Object(e)) = (manifest.as_object_mut(), extra) {
        m.extend(e);
    }
    let path = out.join(MANIFEST);
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    pfx_core::atomic::write(&path, format!("{text}\n").as_bytes())?;
    if command.json {
        println!("{manifest}");
    } else {
        eprintln!("pfx film: {}", peak.line(&memory));
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
        assert_eq!(
            plan(&words("a.film.toml --out tmp/f --memory-limit 3"))
                .unwrap()
                .memory_limit,
            Some(3 << 30)
        );
        for bad in [
            "",
            "a.film.toml",
            "--out tmp/f",
            "a.film.toml --out",
            "a.film.toml b.film.toml --out tmp/f",
            "a.film.toml --out tmp/f --json --json",
            "a.film.toml --out tmp/f --fast",
            "a.film.toml --out tmp/f --memory-limit 0",
            "a.film.toml --out tmp/f --memory-limit",
        ] {
            assert!(plan(&words(bad)).is_err(), "{bad}");
        }
    }
}
