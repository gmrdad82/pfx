use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};

pub const SEED: u64 = 7;
pub const BUILD_DATE: &str = "2026-06-01";
pub const EPOCH: &str = "1780272000";
pub const GIT_DATE: &str = "2026-06-01T00:00:00Z";
pub const EMAIL: &str = "story@example.invalid";

const DOCUMENTS: &[&str] = &["pitofx", "pfx", "toml", "glb", "gltf"];
const HIDDEN: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "CLICOLOR_FORCE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
];

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn sample(name: &str) -> PathBuf {
    root().join("samples").join(name)
}

pub fn fixture(path: &str) -> PathBuf {
    root().join("tests").join("fixtures").join(path)
}

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

pub struct Sandbox {
    pub dir: PathBuf,
    home: PathBuf,
    runtime: PathBuf,
}

impl Sandbox {
    pub fn new(story: &str) -> Sandbox {
        let dir = root().join("tmp").join("stories").join(story);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        }
        let home = dir.join("home");
        let runtime = dir.join("runtime");
        for folder in [&dir.join("work"), &home, &runtime] {
            std::fs::create_dir_all(folder).unwrap_or_else(|e| panic!("{}: {e}", folder.display()));
        }
        private(&runtime);
        std::fs::write(home.join(".gitconfig"), "").unwrap();
        Sandbox { dir, home, runtime }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.dir.join("work").join(relative)
    }

    pub fn copy(&self, from: &Path, relative: &str) -> PathBuf {
        let to = self.path(relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::copy(from, &to)
            .unwrap_or_else(|e| panic!("{} -> {}: {e}", from.display(), to.display()));
        to
    }

    pub fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let to = self.path(relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&to, bytes).unwrap_or_else(|e| panic!("{}: {e}", to.display()));
        to
    }

    pub fn pfx(&self) -> Pfx {
        Pfx::new(self.isolated(Command::new(env!("CARGO_BIN_EXE_pfx")), &root()))
    }

    pub fn pfx_in(&self, dir: &Path) -> Pfx {
        let mut command = self.isolated(Command::new(env!("CARGO_BIN_EXE_pfx")), dir);
        command.env("GIT_CEILING_DIRECTORIES", &self.dir);
        Pfx::new(command)
    }

    pub fn git(&self, dir: &Path, args: &[&str]) -> Run {
        let mut command = self.isolated(Command::new("git"), dir);
        command.env("GIT_CEILING_DIRECTORIES", &self.dir);
        command.args(args);
        Run::of(command, None)
    }

    pub fn git_init(&self, dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        self.git(dir, &["init", "-q", "-b", "main"]).ok();
    }

    pub fn prun_set(&self, set: &str) -> PathBuf {
        let into = self.dir.join("fixtures").join(set);
        let mut command = self.isolated(Command::new("prun"), &root());
        command
            .args(["fixtures", set, "--into"])
            .arg(&into)
            .arg("--json");
        Run::of(command, None).ok();
        into
    }

    fn isolated(&self, mut command: Command, dir: &Path) -> Command {
        for name in HIDDEN {
            command.env_remove(name);
        }
        let config = self.home.join(".config");
        let cache = self.home.join(".cache");
        let data = self.home.join(".local/share");
        let state = self.home.join(".local/state");
        command
            .current_dir(dir)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", &config)
            .env("LOCALAPPDATA", &data)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_CACHE_HOME", &cache)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("NO_COLOR", "1")
            .env("PFX_DIRECT", "1")
            .env("PFX_CACHE_DIR", "off")
            .env("PFX_CACHE", self.dir.join("cache"))
            .env("PITO_ARCHIVE", self.dir.join("archive"))
            .env("SOURCE_DATE_EPOCH", EPOCH)
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Story")
            .env("GIT_AUTHOR_EMAIL", EMAIL)
            .env("GIT_AUTHOR_DATE", GIT_DATE)
            .env("GIT_COMMITTER_NAME", "Story")
            .env("GIT_COMMITTER_EMAIL", EMAIL)
            .env("GIT_COMMITTER_DATE", GIT_DATE)
            .stdin(Stdio::null());
        command
    }
}

#[cfg(unix)]
fn private(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(not(unix))]
fn private(_: &Path) {}

pub struct Pfx {
    command: Command,
    args: Vec<OsString>,
    input: Option<Vec<u8>>,
}

impl Pfx {
    fn new(command: Command) -> Pfx {
        Pfx {
            command,
            args: Vec::new(),
            input: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Pfx {
        self.args.push(arg.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: Into<OsString>>(mut self, args: I) -> Pfx {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, name: &str, value: impl AsRef<std::ffi::OsStr>) -> Pfx {
        self.command.env(name, value);
        self
    }

    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Pfx {
        self.input = Some(bytes.into());
        self
    }

    pub fn run(mut self) -> Run {
        opens_no_window(&self.args);
        self.command.args(&self.args);
        Run::of(self.command, self.input)
    }
}

fn opens_no_window(args: &[OsString]) {
    let words: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert!(
        !words.is_empty(),
        "a bare pfx opens the window; a story always names a verb or a flag"
    );
    let lone_document = words.len() == 1
        && !words[0].starts_with('-')
        && (Path::new(&words[0])
            .extension()
            .is_some_and(|ext| DOCUMENTS.iter().any(|known| ext == *known))
            || Path::new(&words[0]).is_dir());
    assert!(
        !lone_document,
        "pfx {} opens the window; a story always names a verb or a flag",
        words[0]
    );
    let window = words
        .first()
        .is_some_and(|verb| verb == "gui" || verb == "edit")
        && !words
            .iter()
            .any(|word| matches!(word.as_str(), "--shot" | "--clip" | "--help" | "-h"));
    assert!(
        !window,
        "pfx {} without --shot or --clip opens the window; stories draw offscreen",
        words.join(" ")
    );
}

#[derive(Debug)]
pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub shown: String,
}

impl Run {
    fn of(mut command: Command, input: Option<Vec<u8>>) -> Run {
        let shown = format!("{command:?}");
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .spawn()
            .unwrap_or_else(|error| panic!("{shown}: {error}"));
        if let Some(bytes) = input {
            let mut pipe = child.stdin.take().unwrap();
            pipe.write_all(&bytes).unwrap();
        }
        let output = child
            .wait_with_output()
            .unwrap_or_else(|error| panic!("{shown}: {error}"));
        Run {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            shown,
        }
    }

    pub fn ok(&self) -> &Run {
        assert_eq!(self.code, Some(0), "{self}");
        self
    }

    pub fn json(&self) -> serde_json::Value {
        self.ok();
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|error| panic!("stdout is not one JSON value: {error}\n{self}"))
    }

    pub fn json_lines(&self) -> Vec<serde_json::Value> {
        self.ok();
        self.stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|error| panic!("{line}: {error}\n{self}"))
            })
            .collect()
    }

    pub fn refused(&self) -> &str {
        assert_eq!(self.code, Some(1), "a refusal exits 1\n{self}");
        let lines: Vec<&str> = self.stderr.lines().collect();
        assert_eq!(lines.len(), 1, "a refusal is one line\n{self}");
        let line = lines[0];
        let named = line.starts_with("pfx: ") || (line.starts_with("pfx ") && line.contains(": "));
        assert!(named, "a refusal starts with pfx: or pfx <verb>: \n{self}");
        assert!(!line.contains("panicked"), "{self}");
        line
    }

    pub fn usage_error(&self) -> &str {
        assert_eq!(self.code, Some(2), "a usage error exits 2\n{self}");
        assert!(
            self.stdout.is_empty(),
            "a usage error prints nothing on stdout\n{self}"
        );
        assert!(!self.stderr.contains('\u{1b}'), "plain when piped\n{self}");
        let lines: Vec<&str> = self.stderr.lines().collect();
        assert!(
            lines
                .first()
                .is_some_and(|line| line.starts_with("error: ")),
            "{self}"
        );
        assert_eq!(
            lines.last().copied(),
            Some("For more information, try '--help'."),
            "{self}"
        );
        assert_eq!(lines[lines.len() - 2], "", "{self}");
        if let Some(usage) = lines.iter().position(|line| line.starts_with("Usage: ")) {
            assert!(lines[usage].starts_with("Usage: pfx"), "{self}");
            assert_eq!(lines[usage - 1], "", "{self}");
            assert_eq!(usage + 3, lines.len(), "{self}");
        }
        lines[0]
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\nexit {:?}\n--- stdout\n{}\n--- stderr\n{}",
            self.shown, self.code, self.stdout, self.stderr
        )
    }
}

pub struct Keys {
    signing: SigningKey,
    pub public: String,
}

impl Keys {
    pub fn new(seed: u8) -> Keys {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let public = STANDARD.encode(signing.verifying_key().to_bytes());
        Keys { signing, public }
    }

    pub fn mint(&self, payload: &str) -> String {
        let signed = format!("PITO1.{}", URL_SAFE_NO_PAD.encode(payload.as_bytes()));
        let signature = self.signing.sign(signed.as_bytes());
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }
}

pub fn claims(app: &str, tier: &str, group: Option<&str>, timed: Option<(&str, u32)>) -> String {
    let group = group.map_or(String::new(), |group| format!(r#","group":"{group}""#));
    let timed = timed.map_or(String::new(), |(through, grace)| {
        format!(r#","type":"timed","paid_through":"{through}","grace_days":{grace}"#)
    });
    format!(
        r#"{{"v":2,"id":"story","email":"{EMAIL}","app":"{app}","tier":"{tier}","major":3,"issued":"2026-01-01"{group}{timed}}}"#
    )
}
