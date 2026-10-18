use std::ffi::OsStr;

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
        .ok_or_else(|| "pgpu status: no answer".to_string())
}

pub fn on_path(path: &OsStr) -> bool {
    let program = format!("pgpu{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path).any(|folder| folder.join(&program).is_file())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_status_line_names_the_load_the_queue_and_a_game() {
        let json = r#"{"v":1,"paused":false,"gpu_busy":42,"play":{"pid":7,"name":"pfx gui: room.scene.toml"},
            "jobs":[{"pid":1,"state":"running","as":"pfx","label":"cargo test"},{"pid":2,"state":"yielded","as":"manf","label":"x"}]}"#;
        let status = Status::parse(json).unwrap();
        assert_eq!(status.busy, Some(42));
        assert_eq!(status.jobs, 2);
        assert_eq!(
            status.line(),
            "gpu 42% · 2 queued · playing pfx gui: room.scene.toml · pfx: cargo test"
        );
        let idle = Status::parse(r#"{"v":1,"paused":true,"play":null,"jobs":[]}"#).unwrap();
        assert_eq!(idle.line(), "0 queued · queue paused");
        assert!(Status::parse("not json").is_err());
    }

    #[test]
    fn the_hook_is_on_only_when_pgpu_is_found_on_the_path() {
        let path = crate::tests::scene_copy("pgpu on path");
        let folder = path.parent().unwrap().to_path_buf();
        let empty = folder.join("empty");
        let tools = folder.join("tools");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(
            tools.join(format!("pgpu{}", std::env::consts::EXE_SUFFIX)),
            "",
        )
        .unwrap();
        let joined = |folders: &[&Path]| std::env::join_paths(folders).unwrap();
        assert!(!on_path(OsStr::new("")));
        assert!(!on_path(&joined(&[&empty])));
        assert!(on_path(&joined(&[&empty, &tools])));
        assert!(!on_path(&joined(&[&tools.join("pgpu")])));
    }
}
