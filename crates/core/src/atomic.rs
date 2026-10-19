use std::io;
use std::path::{Path, PathBuf};

const MARK: &str = ".partial";

fn shown(path: &Path, error: io::Error) -> String {
    format!("{}: {error}", path.display())
}

pub fn partial(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let hidden = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            format!(".{stem}{MARK}.{ext}")
        }
        _ => format!(".{name}{MARK}"),
    };
    path.with_file_name(hidden)
}

pub fn is_partial(name: &str) -> bool {
    name.starts_with('.') && (name.ends_with(MARK) || name.contains(".partial."))
}

pub fn write_with<E: From<String>>(
    path: &Path,
    make: impl FnOnce(&Path) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| E::from(shown(parent, e)))?;
    }
    let temp = partial(path);
    if let Err(error) = make(&temp) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    commit(&temp, path).map_err(E::from)
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_with(path, |temp| {
        std::fs::write(temp, bytes).map_err(|e| shown(temp, e))
    })
}

pub fn commit(temp: &Path, path: &Path) -> Result<(), String> {
    let synced = sync_file(temp);
    if let Err(error) = synced.and_then(|()| std::fs::rename(temp, path)) {
        let _ = std::fs::remove_file(temp);
        return Err(shown(path, error));
    }
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => sync_dir(parent),
        None => sync_dir(Path::new(".")),
    }
}

fn sync_file(path: &Path) -> io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
}

#[cfg(unix)]
pub fn sync_dir(dir: &Path) -> Result<(), String> {
    std::fs::File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|e| shown(dir, e))
}

#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}

pub fn sync_tree(dir: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| shown(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| shown(dir, e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| shown(&path, e))?;
        if kind.is_dir() {
            sync_tree(&path)?;
        } else if kind.is_file() {
            sync_file(&path).map_err(|e| shown(&path, e))?;
        }
    }
    sync_dir(dir)
}

pub fn sweep(dir: &Path) -> Result<Vec<PathBuf>, String> {
    sweep_matching(dir, |_| true)
}

pub fn sweep_matching(dir: &Path, pick: impl Fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(shown(dir, e)),
    };
    let mut gone = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| shown(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if file && is_partial(&name) && pick(&name) {
            let path = entry.path();
            std::fs::remove_file(&path).map_err(|e| shown(&path, e))?;
            gone.push(path);
        }
    }
    gone.sort();
    Ok(gone)
}

#[cfg(test)]
mod tests;
