use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub fn state_dir() -> PathBuf {
    own_dir("XDG_STATE_HOME", state_dir_from)
}

pub fn own_dir(xdg: &str, linux: fn(Option<&OsStr>, Option<&OsStr>) -> PathBuf) -> PathBuf {
    let home = home();
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|dir| !dir.is_empty()) {
            return PathBuf::from(local).join("pfx");
        }
    } else if cfg!(target_os = "macos")
        && let Some(home) = &home
    {
        return PathBuf::from(home).join("Library/Application Support/pfx");
    }
    linux(std::env::var_os(xdg).as_deref(), home.as_deref())
}

pub fn home() -> Option<OsString> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()))
}

pub fn state_dir_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    match xdg {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("pfx"),
        _ => home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/state/pfx"),
    }
}

pub fn data_dir_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    match xdg {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("pfx"),
        _ => home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local/share/pfx"),
    }
}
