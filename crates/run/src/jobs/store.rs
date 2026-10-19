use std::io::Write;
use std::path::{Path, PathBuf};

use super::state::{Asked, Job};

pub const STATE_VAR: &str = "PFX_STATE";
pub const JOB: &str = "job.json";
pub const ASK: &str = "ask";
pub const LOG: &str = "log";
pub const OLD_LOG: &str = "log.1";
pub const LOCK: &str = "lock";
pub const LOG_BYTES: u64 = 4 << 20;

pub fn state_dir() -> Result<PathBuf, String> {
    let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(dir) = env(STATE_VAR)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        return Ok(dir);
    }
    let home = env("HOME")
        .or_else(|| env("USERPROFILE"))
        .map(PathBuf::from);
    if cfg!(windows) {
        if let Some(local) = env("LOCALAPPDATA") {
            return Ok(PathBuf::from(local).join("pfx"));
        }
    } else if cfg!(target_os = "macos")
        && let Some(home) = &home
    {
        return Ok(home.join("Library/Application Support/pfx"));
    }
    if let Some(xdg) = env("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        return Ok(xdg.join("pfx"));
    }
    home.map(|home| home.join(".local/state/pfx"))
        .ok_or_else(|| format!("no state folder: set {STATE_VAR}"))
}

pub fn root() -> Result<PathBuf, String> {
    state_dir().map(|dir| dir.join("jobs"))
}

pub fn check_name(name: &str) -> Result<(), String> {
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if name.is_empty() || name.len() > 64 || !plain || name.starts_with('.') {
        return Err(format!(
            "{name:?}: a job's name is 1 to 64 letters, digits, '-', '_' or '.', not starting with '.'"
        ));
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "{name}: a job's name is not all digits, so a pid never reads as one"
        ));
    }
    Ok(())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("{}: {e}", path.display());
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let temp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = std::fs::File::create(&temp).map_err(failed)?;
    file.write_all(bytes).map_err(failed)?;
    file.sync_all().map_err(failed)?;
    drop(file);
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        failed(e)
    })?;
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(dir) = std::fs::File::open(dir)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

pub fn write(dir: &Path, job: &Job) -> Result<(), String> {
    let text = serde_json::to_string_pretty(job).map_err(|e| e.to_string())?;
    write_atomic(&dir.join(JOB), format!("{text}\n").as_bytes())
}

pub fn read(dir: &Path) -> Result<Job, String> {
    let path = dir.join(JOB);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn all(root: &Path) -> Vec<(PathBuf, Result<Job, String>)> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join(JOB).is_file())
        .collect();
    dirs.sort();
    dirs.into_iter()
        .map(|dir| {
            let job = read(&dir);
            (dir, job)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Run,
    Pause,
    Stop,
    StopNow,
}

impl Ask {
    fn word(self) -> &'static str {
        match self {
            Ask::Run => "run",
            Ask::Pause => "pause",
            Ask::Stop => "stop",
            Ask::StopNow => "stop-now",
        }
    }

    pub fn asked(self) -> Asked {
        match self {
            Ask::Run => Asked::Run,
            Ask::Pause => Asked::Pause,
            Ask::Stop | Ask::StopNow => Asked::Stop,
        }
    }
}

pub fn ask(dir: &Path, ask: Ask) -> Result<(), String> {
    write_atomic(&dir.join(ASK), ask.word().as_bytes())
}

pub fn take_ask(dir: &Path) -> Option<Ask> {
    let path = dir.join(ASK);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    match text.trim() {
        "run" => Some(Ask::Run),
        "pause" => Some(Ask::Pause),
        "stop" => Some(Ask::Stop),
        "stop-now" => Some(Ask::StopNow),
        _ => None,
    }
}

pub struct Log {
    dir: PathBuf,
    file: Option<std::fs::File>,
}

impl Log {
    pub fn open(dir: &Path) -> Log {
        let mut log = Log {
            dir: dir.to_path_buf(),
            file: None,
        };
        log.reopen();
        log
    }

    fn reopen(&mut self) {
        self.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(LOG))
            .ok();
    }

    pub fn line(&mut self, line: &str) {
        let full = self
            .file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .is_some_and(|meta| meta.len() >= LOG_BYTES);
        if full {
            let _ = std::fs::rename(self.dir.join(LOG), self.dir.join(OLD_LOG));
            self.reopen();
        }
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{}", line.trim_end());
        }
    }
}

pub fn log_lines(dir: &Path) -> Vec<String> {
    [OLD_LOG, LOG]
        .iter()
        .filter_map(|name| std::fs::read_to_string(dir.join(name)).ok())
        .flat_map(|text| text.lines().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/jobs-store")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn job_names_are_plain_and_never_a_pid() {
        for good in ["film", "night-4k.v2", "a_b"] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            ".hidden",
            "a/b",
            "../x",
            "12345",
            "a b",
            &"x".repeat(65),
        ] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_atomic_write_leaves_the_old_file_or_the_new_one_and_no_temp() {
        let dir = scratch("atomic");
        let path = dir.join(JOB);
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [JOB]);
        ask(&dir, Ask::Pause).unwrap();
        assert_eq!(take_ask(&dir), Some(Ask::Pause));
        assert_eq!(take_ask(&dir), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_log_rotates_into_one_old_file_and_reads_back_in_order() {
        let dir = scratch("log");
        std::fs::write(dir.join(LOG), vec![b'x'; LOG_BYTES as usize]).unwrap();
        let mut log = Log::open(&dir);
        log.line("after the turn");
        assert_eq!(
            std::fs::metadata(dir.join(OLD_LOG)).unwrap().len(),
            LOG_BYTES
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(LOG)).unwrap(),
            "after the turn\n"
        );
        assert_eq!(
            log_lines(&dir).last().map(String::as_str),
            Some("after the turn")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
