use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG: &str = crate::RENDER_TOML;
const AGENT: &str = concat!("pfx/", env!("CARGO_PKG_VERSION"), " (gmrdad82)");
const STAMP: &str = ".used";
const PRUNE_DAYS: u64 = 30;

#[derive(Deserialize, Clone)]
pub struct Pin {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct Config {
    ffmpeg: BTreeMap<String, Pin>,
}

fn config() -> Result<Config, String> {
    toml::from_str(CONFIG).map_err(|e| format!("render.toml: {e}"))
}

pub fn pins() -> Result<BTreeMap<String, Pin>, String> {
    Ok(config()?.ffmpeg)
}

pub fn pin() -> Result<Pin, String> {
    let key = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    pins()?
        .remove(&key)
        .ok_or_else(|| format!("render.toml pins no ffmpeg for {key}"))
}

pub fn cache() -> Result<PathBuf, String> {
    let dir = std::env::var_os("PFX_CACHE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("pito-hd-renderer"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".cache").join("pito-hd-renderer"))
        })
        .ok_or("no cache folder: set PFX_CACHE")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn touch(entry: &Path) {
    if entry.is_dir() {
        let _ = std::fs::write(entry.join(STAMP), b"");
    }
}

fn size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn grew(cache: &Path) {
    eprintln!(
        "pfx: the cache at {} now holds {} (pfx cache --prune drops what went unused for {PRUNE_DAYS} days)",
        cache.display(),
        gigabytes(size(cache))
    );
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn fetch(pin: &Pin, dest: &Path, who: &str) -> Result<(), String> {
    if dest.is_file() && hash_file(dest)? == pin.sha256 {
        return Ok(());
    }
    eprintln!("{who}: fetching {}", pin.url);
    let response = ureq::get(&pin.url)
        .header("User-Agent", AGENT)
        .call()
        .map_err(|e| format!("{}: {e}", pin.url))?;
    let part = dest.with_extension("part");
    let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut reader = response.into_body().into_reader();
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", pin.url))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        total += n as u64;
    }
    let got = hex::encode(hasher.finalize());
    if total != pin.size || got != pin.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "{}: expected {} bytes with sha256 {}, got {total} bytes with {got}",
            pin.url, pin.size, pin.sha256
        ));
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == name)
                && path.parent().is_some_and(|p| p.ends_with("bin"))
            {
                return Some(path);
            }
        }
    }
    None
}

pub fn resolve(tools: &Path, who: &str, pin: &Pin) -> Result<PathBuf, String> {
    let dir = tools.join("ffmpeg").join(&pin.sha256[..16]);
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if dir.join("ready").is_file()
        && let Some(path) = find(&dir, exe)
    {
        touch(&dir);
        return Ok(path);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let archive = dir.join(pin.url.rsplit('/').next().unwrap_or("ffmpeg-archive"));
    fetch(pin, &archive, who)?;
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&dir)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar could not unpack {}", archive.display()));
    }
    let path = find(&dir, exe).ok_or_else(|| format!("no {exe} inside {}", archive.display()))?;
    std::fs::remove_file(&archive).ok();
    std::fs::write(dir.join("ready"), &pin.sha256).map_err(|e| e.to_string())?;
    touch(&dir);
    grew(tools);
    Ok(path)
}

pub fn ffmpeg(tools: &Path, who: &str) -> Result<(PathBuf, Pin), String> {
    let pin = pin()?;
    let path = resolve(tools, who, &pin)?;
    Ok((path, pin))
}
