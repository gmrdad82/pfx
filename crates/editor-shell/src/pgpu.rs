use std::time::Duration;

use serde_json::Value;

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

pub use pfx_run::queue::{present, queue_command, turning};

pub const SWITCH: &str = "PFX_EDIT_PGPU";

pub fn wanted() -> bool {
    switched_on(std::env::var(SWITCH).ok().as_deref())
}

fn switched_on(value: Option<&str>) -> bool {
    value != Some("off")
}

pub fn fetch() -> Option<Value> {
    pfx_run::queue::status()
}

pub fn status(name: &str) -> Pgpu {
    fetch().map_or(Pgpu::Free, |status| classify(&status, name))
}

pub fn watch(name: String, mut tell: impl FnMut(Pgpu) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        if !wanted() || !present() {
            return;
        }
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
    fn only_the_word_off_switches_the_watch_off() {
        assert!(switched_on(None));
        assert!(switched_on(Some("")));
        assert!(switched_on(Some("on")));
        assert!(switched_on(Some("OFF")));
        assert!(!switched_on(Some("off")));
    }

    #[cfg(unix)]
    type Tools<'a> = &'a [(&'a str, &'a str)];

    #[cfg(unix)]
    #[test]
    fn the_watch_asks_only_an_answering_queue_from_its_own_thread() {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("PFX_WATCH_CHILD").is_some() {
            return;
        }
        let log = "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\n";
        let free = format!("{log}echo '{{\"v\":1,\"play\":null,\"jobs\":[]}}'");
        let play = format!("{log}echo '{{\"v\":1,\"play\":{{\"in_front\":true}},\"jobs\":[]}}'");
        let ours =
            format!("{log}echo '{{\"v\":1,\"play\":null,\"jobs\":[{{\"as\":\"editor\"}}]}}'");
        let fails = format!("{log}echo '{{\"v\":1}}'\nexit 1");
        let speaks = format!("{log}echo 'pci: no such command'");
        let stale = "printf 'stale %s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho '{\"v\":1}'".to_string();
        let old = concat!("p", "gpu");
        let cases: [(&str, Tools, Option<&str>, &str, &[&str]); 8] = [
            (
                "free",
                &[("pci", free.as_str())],
                None,
                "free",
                &["gpu status --json"; 2],
            ),
            (
                "play",
                &[("pci", play.as_str())],
                None,
                "play",
                &["gpu status --json"; 2],
            ),
            (
                "ours",
                &[("pci", ours.as_str())],
                None,
                "queued",
                &["gpu status --json"; 2],
            ),
            (
                "fails",
                &[("pci", fails.as_str())],
                None,
                "none",
                &["gpu status --json"],
            ),
            (
                "speaks",
                &[("pci", speaks.as_str())],
                None,
                "none",
                &["gpu status --json"],
            ),
            ("stale", &[(old, stale.as_str())], None, "none", &[]),
            (
                "switched",
                &[("pci", free.as_str())],
                Some("off"),
                "none",
                &[],
            ),
            ("empty", &[], None, "none", &[]),
        ];
        for (name, tools, switch, expected, asked) in cases {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp")
                .join(format!("shell-watch-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            for (tool, body) in tools {
                let path = dir.join(tool);
                std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let log = dir.join("log");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "pgpu::tests::watch_child", "--test-threads", "1"])
                .env("PATH", &dir)
                .env("PFX_WATCH_CHILD", expected)
                .env("QUEUE_LOG", &log)
                .env_remove(SWITCH);
            if let Some(switch) = switch {
                child.env(SWITCH, switch);
            }
            let out = child.output().unwrap();
            let logged = std::fs::read_to_string(&log).unwrap_or_default();
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(
                out.status.success(),
                "{name}: the child failed:\n{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(logged.lines().collect::<Vec<_>>(), asked, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn watch_child() {
        let Some(expected) = std::env::var_os("PFX_WATCH_CHILD") else {
            return;
        };
        let (send, receive) = std::sync::mpsc::channel();
        watch("editor".to_string(), move |state| {
            let _ = send.send(state);
            false
        });
        let told = receive.recv().ok().map(Pgpu::word).unwrap_or("none");
        assert_eq!(told, expected);
        assert!(receive.recv().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_turn_reaches_the_queue_only_inside_a_queued_job() {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("PFX_TURN_CHILD").is_some() {
            return;
        }
        for inside in [true, false] {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp")
                .join(format!("shell-turn-{inside}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("pci");
            std::fs::write(
                &path,
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho noise\n",
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let log = dir.join("log");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "pgpu::tests::turn_child",
                    "--test-threads",
                    "1",
                    "--nocapture",
                ])
                .env("PATH", &dir)
                .env("PFX_TURN_CHILD", "1")
                .env("QUEUE_LOG", &log)
                .env_remove(pfx_run::queue::TURN_VAR);
            if inside {
                child.env(pfx_run::queue::TURN_VAR, "1");
            }
            let out = child.output().unwrap();
            let logged = std::fs::read_to_string(&log).unwrap_or_default();
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(out.status.success(), "{inside}");
            assert!(!String::from_utf8_lossy(&out.stdout).contains("noise"));
            let lines: Vec<_> = logged.lines().collect();
            if inside {
                assert_eq!(lines.len(), 2, "{logged}");
                for line in lines {
                    let words: Vec<_> = line.split(' ').collect();
                    assert_eq!(words[..3], ["gpu", "turn", "--work-ms"], "{line}");
                    assert_eq!(words[4], "--longest-ms", "{line}");
                    assert_eq!(words[3], words[5], "{line}");
                    assert!(words[3].parse::<u64>().is_ok(), "{line}");
                }
            } else {
                assert!(lines.is_empty(), "{logged}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn turn_child() {
        if std::env::var_os("PFX_TURN_CHILD").is_none() {
            return;
        }
        let since = std::time::Instant::now();
        crate::gpu_turn(since);
        crate::gpu_turn(since);
    }
}
