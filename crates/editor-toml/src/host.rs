use std::fmt;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::process::{Command, Stdio};

pub const NO_EDITOR: &str = "set $VISUAL or $EDITOR to open the file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoHead {
    NoGit,
    NotRepo,
    Untracked,
    Failed(String),
}

impl fmt::Display for NoHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoHead::NoGit => f.write_str("git is not on PATH"),
            NoHead::NotRepo => f.write_str("the file is not in a git repository"),
            NoHead::Untracked => f.write_str("git HEAD has no such file"),
            NoHead::Failed(why) => write!(f, "git HEAD cannot be read: {why}"),
        }
    }
}

pub trait Head {
    fn show(&self, file: &Path) -> Result<String, NoHead>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Git;

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dir).stdin(Stdio::null());
    command
}

fn spawned(error: io::Error) -> NoHead {
    if error.kind() == ErrorKind::NotFound {
        NoHead::NoGit
    } else {
        NoHead::Failed(error.to_string())
    }
}

impl Head for Git {
    fn show(&self, file: &Path) -> Result<String, NoHead> {
        let dir = file
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = file.file_name().ok_or(NoHead::Untracked)?;
        if !dir.is_dir() {
            return Err(NoHead::NotRepo);
        }
        let shown = git(dir)
            .arg("show")
            .arg(format!("HEAD:./{}", name.to_string_lossy()))
            .output()
            .map_err(spawned)?;
        if shown.status.success() {
            return String::from_utf8(shown.stdout)
                .map_err(|_| NoHead::Failed("the file is not UTF-8".into()));
        }
        let inside = git(dir)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map_err(spawned)?;
        Err(if inside.status.success() {
            NoHead::Untracked
        } else {
            NoHead::NotRepo
        })
    }
}

pub trait Opener {
    fn command(&self) -> Option<String>;
    fn open(&self, command: &str, file: &Path) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Process;

pub fn pick(visual: Option<String>, editor: Option<String>) -> Option<String> {
    [visual, editor]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

impl Opener for Process {
    fn command(&self) -> Option<String> {
        pick(std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok())
    }

    fn open(&self, command: &str, file: &Path) -> io::Result<()> {
        #[cfg(unix)]
        let mut run = {
            let mut run = Command::new("sh");
            run.arg("-c")
                .arg(format!("{command} \"$1\""))
                .arg("sh")
                .arg(file);
            run
        };
        #[cfg(windows)]
        let mut run = {
            let mut run = Command::new("cmd");
            run.arg("/C").arg(command).arg(file);
            run
        };
        let mut child = run
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_wins_over_editor_and_blanks_are_skipped() {
        let some = |text: &str| Some(text.to_string());
        assert_eq!(pick(some("code -w"), some("vi")), some("code -w"));
        assert_eq!(pick(None, some("vi")), some("vi"));
        assert_eq!(pick(some("  "), some("vi")), some("vi"));
        assert_eq!(pick(some(""), None), None);
        assert_eq!(pick(None, None), None);
    }

    #[test]
    fn a_missing_folder_is_not_a_repository() {
        let gone = Path::new("tmp/no-such-folder/scene.toml");
        assert_eq!(Git.show(gone), Err(NoHead::NotRepo));
    }

    #[test]
    fn the_reasons_read_plainly() {
        assert_eq!(NoHead::NoGit.to_string(), "git is not on PATH");
        assert!(NoHead::NotRepo.to_string().contains("repository"));
    }
}
