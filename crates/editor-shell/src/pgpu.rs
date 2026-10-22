use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::time::Duration;

use serde_json::Value;

pub const TIMEOUT: Duration = Duration::from_millis(1500);
pub const EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pgpu {
    #[default]
    Free,
    Play,
    Queued,
}

impl Pgpu {
    pub fn word(self) -> &'static str {
        match self {
            Pgpu::Free => "free",
            Pgpu::Play => "play",
            Pgpu::Queued => "queued",
        }
    }

    pub fn paused(self) -> bool {
        self == Pgpu::Play
    }
}

pub fn classify(status: &Value, name: &str) -> Pgpu {
    let play = status.get("play").filter(|play| !play.is_null());
    if play.is_some_and(|play| play.get("in_front").and_then(Value::as_bool) != Some(false)) {
        return Pgpu::Play;
    }
    let ours = status
        .get("jobs")
        .and_then(Value::as_array)
        .is_some_and(|jobs| {
            jobs.iter()
                .any(|job| job.get("as").and_then(Value::as_str) == Some(name))
        });
    if ours { Pgpu::Queued } else { Pgpu::Free }
}

#[cfg(test)]
fn read(text: &str) -> Pgpu {
    serde_json::from_str::<Value>(text).map_or(Pgpu::Free, |status| classify(&status, "editor"))
}

pub const QUEUE: &str = "pci";

pub fn fetch() -> Option<Value> {
    fetch_from(QUEUE)
}

pub fn fetch_from(program: impl AsRef<OsStr>) -> Option<Value> {
    let mut child = Command::new(program)
        .args(["gpu", "status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let read = out.read_to_string(&mut text).map(|_| text);
        let _ = send.send(read);
    });
    match receive.recv_timeout(TIMEOUT) {
        Ok(Ok(text)) => {
            let ok = child.wait().is_ok_and(|exit| exit.success());
            if ok {
                serde_json::from_str(&text).ok()
            } else {
                None
            }
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

pub fn answers(status: &Value) -> bool {
    status.is_object() && status.get("v").is_some()
}

pub fn found(program: impl AsRef<OsStr>) -> bool {
    fetch_from(program).is_some_and(|status| answers(&status))
}

pub fn present() -> bool {
    static PRESENT: OnceLock<bool> = OnceLock::new();
    *PRESENT.get_or_init(|| found(QUEUE))
}

pub fn status(name: &str) -> Pgpu {
    fetch().map_or(Pgpu::Free, |status| classify(&status, name))
}

pub fn watch(name: String, mut tell: impl FnMut(Pgpu) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        loop {
            if !tell(status(&name)) {
                return;
            }
            std::thread::sleep(EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_game_is_free() {
        let text = r#"{"v":1,"paused":false,"gpu_busy":5,"play":null,"away":false,"jobs":[{"pid":7,"state":"running","as":"other"}]}"#;
        assert_eq!(read(text), Pgpu::Free);
    }

    #[test]
    fn a_game_in_front_pauses() {
        let text = r#"{"v":1,"play":{"game":"a game","in_front":true},"jobs":[]}"#;
        assert_eq!(read(text), Pgpu::Play);
        assert!(read(text).paused());
    }

    #[test]
    fn a_game_without_in_front_pauses() {
        assert_eq!(read(r#"{"play":{"game":"a game"}}"#), Pgpu::Play);
        assert_eq!(read(r#"{"play":{"in_front":null}}"#), Pgpu::Play);
    }

    #[test]
    fn a_game_behind_does_not_pause() {
        let text = r#"{"play":{"game":"a game","in_front":false},"jobs":[]}"#;
        assert_eq!(read(text), Pgpu::Free);
    }

    #[test]
    fn our_own_job_reads_queued() {
        let text = r#"{"play":null,"jobs":[{"as":"other"},{"as":"editor","state":"waiting","waits_for":"desk"}]}"#;
        assert_eq!(read(text), Pgpu::Queued);
        let behind = r#"{"play":{"in_front":false},"jobs":[{"as":"editor"}]}"#;
        assert_eq!(read(behind), Pgpu::Queued);
        let front = r#"{"play":{"in_front":true},"jobs":[{"as":"editor"}]}"#;
        assert_eq!(read(front), Pgpu::Play);
    }

    #[test]
    fn a_failure_is_no_game() {
        for text in ["", "not json", "[]", "{}", r#"{"play":"#] {
            assert_eq!(read(text), Pgpu::Free, "{text:?}");
        }
    }

    #[test]
    fn only_a_versioned_object_is_the_queue() {
        let answer = |text: &str| serde_json::from_str::<Value>(text).is_ok_and(|v| answers(&v));
        assert!(answer(r#"{"v":1,"paused":false,"play":null,"jobs":[]}"#));
        for text in ["{}", "[1]", r#"{"jobs":[]}"#, r#""v""#, "1"] {
            assert!(!answer(text), "{text}");
        }
    }

    #[test]
    fn the_queue_is_pci_and_a_missing_one_is_not_found() {
        assert_eq!(QUEUE, "pci");
        assert!(!found("pfx-no-such-queue-anywhere"));
        assert!(fetch_from("pfx-no-such-queue-anywhere").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn only_a_pci_whose_gpu_status_answers_is_found() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("shell-queue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            (
                "answers",
                "[ \"$*\" = \"gpu status --json\" ] || exit 9\necho '{\"v\":1,\"play\":null,\"jobs\":[]}'",
                true,
            ),
            (
                "elsewhere",
                "[ \"$*\" = \"status --json\" ] || exit 9\necho '{\"v\":1}'",
                false,
            ),
            ("fails", "echo '{\"v\":1,\"jobs\":[]}'\nexit 2", false),
            (
                "speaks",
                "echo \"error: unrecognized subcommand '$1'\"",
                false,
            ),
            ("unversioned", "echo '{\"jobs\":[]}'", false),
            ("silent", "exit 0", false),
        ];
        for (name, body, expected) in cases {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(found(&path), expected, "{name}");
        }
        let stale = dir.join(concat!("p", "gpu"));
        std::fs::write(&stale, "#!/bin/sh\necho '{\"v\":1}'\n").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!found(dir.join("pci")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
