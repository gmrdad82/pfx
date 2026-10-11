use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct Ffmpeg {
    pub path: PathBuf,
    pub version: String,
    pub sha256: String,
}

impl Ffmpeg {
    pub fn record(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path.display().to_string(),
            "version": self.version,
            "sha256": self.sha256,
        })
    }
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

fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

fn version_of(text: &str) -> Option<String> {
    text.lines()
        .next()?
        .strip_prefix("ffmpeg version ")?
        .split_whitespace()
        .next()
        .map(String::from)
}

pub fn system() -> Result<Ffmpeg, String> {
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let path = on_path(exe).ok_or_else(|| {
        format!("no {exe} on PATH: install the system's ffmpeg, with libx264 and zscale")
    })?;
    let path = path.canonicalize().unwrap_or(path);
    let out = Command::new(&path)
        .arg("-version")
        .output()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !out.status.success() {
        return Err(format!("{} -version failed", path.display()));
    }
    let version = version_of(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| format!("{} -version printed no version", path.display()))?;
    let sha256 = hash_file(&path)?;
    Ok(Ffmpeg {
        path,
        version,
        sha256,
    })
}
