#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub busy: Option<u32>,
    pub jobs: usize,
    pub running: Option<String>,
    pub play: Option<String>,
    pub paused: bool,
}

impl Status {
    pub fn parse(json: &str) -> Result<Status, String> {
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        Ok(Status::of(&value))
    }

    pub fn of(value: &serde_json::Value) -> Status {
        let jobs = value
            .get("jobs")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let running = jobs
            .iter()
            .find(|job| job.get("state").and_then(serde_json::Value::as_str) == Some("running"))
            .map(|job| {
                let name = job
                    .get("as")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let label = job
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                format!("{name}: {label}")
            });
        let play = match value.get("play") {
            Some(serde_json::Value::Object(play)) => Some(
                play.get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("a game")
                    .to_string(),
            ),
            _ => None,
        };
        Status {
            busy: value
                .get("gpu_busy")
                .and_then(serde_json::Value::as_u64)
                .map(|busy| busy as u32),
            jobs: jobs.len(),
            running,
            play,
            paused: value
                .get("paused")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        }
    }

    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(busy) = self.busy {
            parts.push(format!("gpu {busy}%"));
        }
        parts.push(format!("{} queued", self.jobs));
        if let Some(play) = &self.play {
            parts.push(format!("playing {play}"));
        }
        if self.paused {
            parts.push("queue paused".to_string());
        }
        if let Some(running) = &self.running {
            parts.push(running.clone());
        }
        parts.join(" · ")
    }
}

pub fn status() -> Result<Status, String> {
    pfx_editor_shell::pgpu::fetch()
        .map(|value| Status::of(&value))
        .ok_or_else(|| "GPU queue status: no answer".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_names_the_load_the_queue_and_a_game() {
        let json = r#"{"v":1,"paused":false,"gpu_busy":42,"play":{"pid":7,"name":"pfx gui: room.pitofx"},
            "jobs":[{"pid":1,"state":"running","as":"pfx","label":"cargo test"},{"pid":2,"state":"yielded","as":"manf","label":"x"}]}"#;
        let status = Status::parse(json).unwrap();
        assert_eq!(status.busy, Some(42));
        assert_eq!(status.jobs, 2);
        assert_eq!(
            status.line(),
            "gpu 42% · 2 queued · playing pfx gui: room.pitofx · pfx: cargo test"
        );
        let idle = Status::parse(r#"{"v":1,"paused":true,"play":null,"jobs":[]}"#).unwrap();
        assert_eq!(idle.line(), "0 queued · queue paused");
        assert!(Status::parse("not json").is_err());
    }

    #[cfg(unix)]
    type Tools<'a> = &'a [(&'a str, &'a str)];

    #[cfg(unix)]
    fn tool(dir: &std::path::Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_hook_is_on_only_behind_a_pci_whose_gpu_status_answers() {
        if std::env::var_os("PFX_EDIT_CHILD").is_some() {
            return;
        }
        let answers = "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\n[ \"$*\" = \"gpu status --json\" ] || exit 9\necho '{\"v\":1,\"play\":null,\"jobs\":[]}'";
        let fails = "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho '{\"v\":1}'\nexit 1";
        let speaks = "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho 'pci: no such command'";
        let stale = "printf 'stale %s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho '{\"v\":1}'";
        let cases: [(&str, Tools, Option<&str>, &str, usize); 6] = [
            ("answers", &[("pci", answers)], None, "on", 1),
            ("fails", &[("pci", fails)], None, "off", 1),
            ("speaks", &[("pci", speaks)], None, "off", 1),
            ("stale", &[(concat!("p", "gpu"), stale)], None, "off", 0),
            ("switched", &[("pci", answers)], Some("off"), "off", 0),
            ("empty", &[], None, "off", 0),
        ];
        for (name, tools, switch, expected, asked) in cases {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tmp")
                .join(format!("edit-queue-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            for (tool_name, body) in tools {
                tool(&dir, tool_name, body);
            }
            let log = dir.join("log");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", "pgpu::tests::setup_child", "--test-threads", "1"])
                .env("PATH", &dir)
                .env("PFX_EDIT_CHILD", expected)
                .env("QUEUE_LOG", &log)
                .env_remove("PFX_EDIT_PGPU");
            if let Some(switch) = switch {
                child.env("PFX_EDIT_PGPU", switch);
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
            assert_eq!(logged.lines().count(), asked, "{name}: {logged}");
            assert!(
                logged.lines().all(|line| line == "gpu status --json"),
                "{name}: {logged}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn setup_child() {
        let Some(expected) = std::env::var_os("PFX_EDIT_CHILD") else {
            return;
        };
        for _ in 0..3 {
            assert_eq!(crate::setup().pgpu, expected == "on");
        }
    }
}
