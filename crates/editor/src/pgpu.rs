use std::ffi::{OsStr, OsString};
use std::process::{Command, Stdio};

use pfx_load::scene::Scene;
use pfx_play::PlayHooks;

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

#[derive(Debug)]
pub struct PgpuPlay {
    program: OsString,
    pid: u32,
    name: String,
    on: bool,
    error: Option<String>,
}

impl PgpuPlay {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            program: OsString::from("pgpu"),
            pid: std::process::id(),
            name: name.into(),
            on: false,
            error: None,
        }
    }

    pub fn editor(scene: &Scene) -> Self {
        let file = scene
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| scene.path.display().to_string());
        Self::new(format!("pfx edit: {file}"))
    }

    pub fn with_pid(mut self, pid: u32) -> Self {
        self.pid = pid;
        self
    }

    pub fn with_program(mut self, program: impl Into<OsString>) -> Self {
        self.program = program.into();
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn on_args(&self) -> Vec<String> {
        vec![
            "play".into(),
            "on".into(),
            "--pid".into(),
            self.pid.to_string(),
            "--name".into(),
            self.name.clone(),
        ]
    }

    pub fn off_args(&self) -> Vec<String> {
        vec![
            "play".into(),
            "off".into(),
            "--pid".into(),
            self.pid.to_string(),
        ]
    }

    fn run(&mut self, args: Vec<String>) -> bool {
        let status = Command::new(&self.program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => {
                self.error = None;
                true
            }
            Ok(status) => {
                self.error = Some(format!("pgpu {} exited with {status}", args.join(" ")));
                false
            }
            Err(error) => {
                self.error = Some(format!("pgpu {}: {error}", args.join(" ")));
                false
            }
        }
    }

    fn off(&mut self) {
        if self.on {
            self.on = false;
            let args = self.off_args();
            self.run(args);
        }
    }
}

impl PlayHooks for PgpuPlay {
    fn on_play(&mut self, _scene: &Scene) {
        let args = self.on_args();
        self.on = self.run(args);
    }

    fn on_stop(&mut self) {
        self.off();
    }
}

impl Drop for PgpuPlay {
    fn drop(&mut self) {
        self.off();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_status_line_names_the_load_the_queue_and_a_game() {
        let json = r#"{"v":1,"paused":false,"gpu_busy":42,"play":{"pid":7,"name":"pfx edit: room.scene.toml"},
            "jobs":[{"pid":1,"state":"running","as":"pfx","label":"cargo test"},{"pid":2,"state":"yielded","as":"manf","label":"x"}]}"#;
        let status = Status::parse(json).unwrap();
        assert_eq!(status.busy, Some(42));
        assert_eq!(status.jobs, 2);
        assert_eq!(
            status.line(),
            "gpu 42% · 2 queued · playing pfx edit: room.scene.toml · pfx: cargo test"
        );
        let idle = Status::parse(r#"{"v":1,"paused":true,"play":null,"jobs":[]}"#).unwrap();
        assert_eq!(idle.line(), "0 queued · queue paused");
        assert!(Status::parse("not json").is_err());
    }

    #[cfg(unix)]
    fn stand_in(name: &str) -> (Scene, std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let path = crate::tests::scene_copy(name);
        let folder = path.parent().unwrap().to_path_buf();
        let log = folder.join("pgpu.log");
        let program = folder.join("pgpu");
        std::fs::write(
            &program,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        (Scene::open(&path).unwrap(), program, log)
    }

    #[cfg(unix)]
    fn read(log: &Path) -> String {
        std::fs::read_to_string(log).unwrap_or_default()
    }

    #[cfg(unix)]
    #[test]
    fn pgpu_play_turns_play_on_at_play_and_off_at_stop_with_the_editor_pid() {
        let (scene, program, log) = stand_in("pgpu play");
        let hook = PgpuPlay::editor(&scene)
            .with_pid(4242)
            .with_program(&program);
        assert_eq!(hook.name(), "pfx edit: room.scene.toml");
        let mut session = pfx_play::PlaySession::play_hooked(
            &scene,
            None,
            Box::new(pfx_play::SceneGame),
            pfx_play::Options::default(),
            Box::new(hook),
        )
        .unwrap();
        assert_eq!(
            read(&log),
            "play on --pid 4242 --name pfx edit: room.scene.toml\n"
        );
        session.pause();
        session.resume();
        assert_eq!(read(&log).lines().count(), 1, "pause keeps play on");
        let stopped = session.stop();
        assert_eq!(read(&log).lines().nth(1), Some("play off --pid 4242"));
        drop(stopped);
        assert_eq!(read(&log).lines().count(), 2, "off is sent once");
    }

    #[cfg(unix)]
    #[test]
    fn pgpu_play_turns_play_off_when_the_session_is_dropped() {
        let (scene, program, log) = stand_in("pgpu play dropped");
        let dropped = PgpuPlay::new("a game").with_pid(7).with_program(&program);
        let session = pfx_play::PlaySession::play_hooked(
            &scene,
            None,
            Box::new(pfx_play::SceneGame),
            pfx_play::Options::default(),
            Box::new(dropped),
        )
        .unwrap();
        drop(session);
        let lines: Vec<String> = read(&log).lines().map(String::from).collect();
        assert_eq!(lines, ["play on --pid 7 --name a game", "play off --pid 7"]);
    }

    #[test]
    fn pgpu_play_reports_a_missing_pgpu_and_stays_off() {
        let path = crate::tests::scene_copy("pgpu play missing");
        let scene = Scene::open(&path).unwrap();
        let mut missing = PgpuPlay::new("x").with_program(path.with_file_name("no-such-pgpu"));
        missing.on_play(&scene);
        assert!(!missing.is_on());
        assert!(missing.error().is_some());
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
