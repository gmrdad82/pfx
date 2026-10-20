use std::path::{Path, PathBuf};

pub const KIND: &str = "compose";
pub const FORMAT: u32 = 1;
const USED: &str = ".used";

pub fn root() -> Result<PathBuf, String> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    absolute("PFX_CACHE_DIR")
        .or_else(|| absolute("XDG_CACHE_HOME").map(|p| p.join("pfx")))
        .or_else(|| absolute("HOME").map(|p| p.join(".cache").join("pfx")))
        .or_else(|| absolute("LOCALAPPDATA").map(|p| p.join("pfx")))
        .ok_or_else(|| "no cache folder: set PFX_CACHE_DIR".to_string())
}

pub fn key(recipe: &str) -> String {
    let text = format!("{KIND} {FORMAT} pfx {} {recipe}", env!("CARGO_PKG_VERSION"));
    pfx_file::sha256::hex(&pfx_file::sha256::sha256(text.as_bytes()))
}

fn entry(key: &str) -> Result<PathBuf, String> {
    Ok(root()?.join(KIND).join(key))
}

pub fn get(key: &str, files: &[&str]) -> Option<PathBuf> {
    let entry = entry(key).ok()?;
    if !files.iter().all(|file| entry.join(file).is_file()) {
        return None;
    }
    let _ = std::fs::write(entry.join(USED), b"");
    Some(entry)
}

pub fn put(key: &str, files: &[(&str, &[u8])]) -> Result<PathBuf, String> {
    let entry = entry(key)?;
    let folder = entry.parent().unwrap_or(Path::new(""));
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let part = folder.join(format!(".{key}.part-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&part);
    let written = std::fs::create_dir_all(&part).and_then(|()| {
        files
            .iter()
            .try_for_each(|(name, bytes)| std::fs::write(part.join(name), bytes))
    });
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&part);
        return Err(format!("{}: {e}", part.display()));
    }
    if std::fs::rename(&part, &entry).is_err() {
        let _ = std::fs::remove_dir_all(&part);
        let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
        if get(key, &names).is_none() {
            return Err(format!("{}: could not be written", entry.display()));
        }
    }
    Ok(entry)
}
