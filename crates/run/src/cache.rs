use std::path::PathBuf;

pub fn dir() -> Result<PathBuf, String> {
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
