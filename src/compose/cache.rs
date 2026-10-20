use std::path::{Path, PathBuf};

pub const KIND: &str = "compose";
pub const FORMAT: u32 = 3;
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

fn entry(root: &Path, key: &str) -> PathBuf {
    root.join(KIND).join(key)
}

pub fn get(root: &Path, key: &str, files: &[&str]) -> Option<PathBuf> {
    let entry = entry(root, key);
    files
        .iter()
        .all(|file| entry.join(file).is_file())
        .then_some(entry)
}

pub fn touch(entry: &Path) {
    let _ = std::fs::write(entry.join(USED), b"");
}

pub fn put(
    root: &Path,
    key: &str,
    files: &[(&str, &[u8])],
    keep: &dyn Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let entry = entry(root, key);
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
    if std::fs::rename(&part, &entry).is_ok() {
        return Ok(entry);
    }
    if keep(&entry) {
        let _ = std::fs::remove_dir_all(&part);
        return Ok(entry);
    }
    let old = folder.join(format!(".{key}.old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::rename(&entry, &old);
    let moved = std::fs::rename(&part, &entry);
    let _ = std::fs::remove_dir_all(&old);
    if let Err(e) = moved {
        let _ = std::fs::remove_dir_all(&part);
        if !keep(&entry) {
            return Err(format!("{}: {e}", entry.display()));
        }
    }
    Ok(entry)
}
