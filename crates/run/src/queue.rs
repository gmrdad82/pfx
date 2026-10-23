use std::ffi::OsStr;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const HEADLESS_VAR: &str = "PFX_HEADLESS";

pub const DISPLAY_VARS: [&str; 4] = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY", "XAUTHORITY"];

pub const QUEUE: &str = "pci";

pub const QUEUE_ARG: &str = "gpu";

pub const NO_QUEUE: &str = "no GPU queue on the PATH";

pub const TURN_VAR: &str = "GPU_QUEUE_TURN";

pub const TIMEOUT: Duration = Duration::from_millis(1500);

pub const LARGEST_STATUS: u64 = 1 << 20;

const SETTLE: Duration = Duration::from_millis(2);

pub fn headless(command: &mut Command) -> &mut Command {
    for name in DISPLAY_VARS {
        command.env_remove(name);
    }
    command.env(HEADLESS_VAR, "1")
}

pub fn queue_command() -> Command {
    let mut command = Command::new(QUEUE);
    command.arg(QUEUE_ARG);
    command
}

pub fn turning() -> bool {
    std::env::var_os(TURN_VAR).is_some()
}

pub fn present() -> bool {
    static PRESENT: OnceLock<bool> = OnceLock::new();
    *PRESENT.get_or_init(|| probe(QUEUE))
}

pub fn probe(program: impl AsRef<OsStr>) -> bool {
    status_from(program).is_some_and(|status| answers(&status))
}

pub fn answers(status: &Value) -> bool {
    status.is_object() && status.get("v").is_some()
}

pub fn status() -> Option<Value> {
    status_from(QUEUE)
}

pub fn status_from(program: impl AsRef<OsStr>) -> Option<Value> {
    status_within(program, TIMEOUT)
}

fn status_within(program: impl AsRef<OsStr>, timeout: Duration) -> Option<Value> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(program)
        .args([QUEUE_ARG, "status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(out) = child.stdout.take() else {
        end(&mut child);
        return None;
    };
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = Vec::new();
        let read = out
            .take(LARGEST_STATUS + 1)
            .read_to_end(&mut text)
            .map(|_| text);
        let _ = send.send(read);
    });
    let text = match receive.recv_timeout(timeout) {
        Ok(Ok(text)) if text.len() as u64 <= LARGEST_STATUS => text,
        _ => {
            end(&mut child);
            return None;
        }
    };
    let exit = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break exit,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(SETTLE),
            _ => {
                end(&mut child);
                return None;
            }
        }
    };
    if exit.success() {
        serde_json::from_slice(&text).ok()
    } else {
        None
    }
}

fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub struct Launch {
    pub command: Command,
    pub queued: bool,
}

pub fn launch(class: &str, as_name: &str, program: impl AsRef<OsStr>) -> Launch {
    let direct = direct_set(std::env::var("PFX_DIRECT").ok().as_deref());
    let cpus = on_path("taskset").then(crate::pin::build_cpus).flatten();
    launch_with(
        class,
        as_name,
        program,
        direct,
        !direct && present(),
        on_path("systemd-run"),
        cpus.as_deref(),
    )
}

fn direct_set(value: Option<&str>) -> bool {
    value == Some("1")
}

fn launch_with(
    class: &str,
    as_name: &str,
    program: impl AsRef<OsStr>,
    direct: bool,
    pgpu: bool,
    systemd: bool,
    cpus: Option<&str>,
) -> Launch {
    let picked = choice(direct, pgpu, systemd);
    if matches!(picked, Choice::Missing) {
        eprintln!(
            "{}: {NO_QUEUE}, so the adapter runs directly, paced the same",
            crate::job::TOOL
        );
    }
    let mut launch = prepare(picked, class, as_name, program, cpus);
    headless(&mut launch.command);
    launch
}

enum Choice {
    Direct,
    Missing,
    Systemd,
    Pgpu,
}

fn choice(direct: bool, pgpu: bool, systemd: bool) -> Choice {
    if direct {
        Choice::Direct
    } else if !pgpu {
        Choice::Missing
    } else if systemd {
        Choice::Systemd
    } else {
        Choice::Pgpu
    }
}

fn prepare(
    picked: Choice,
    class: &str,
    as_name: &str,
    program: impl AsRef<OsStr>,
    cpus: Option<&str>,
) -> Launch {
    match picked {
        Choice::Direct | Choice::Missing => Launch {
            command: Command::new(program),
            queued: false,
        },
        Choice::Systemd => {
            let mut command = Command::new("systemd-run");
            command.args([
                "--user",
                "--quiet",
                "--scope",
                "--collect",
                "--slice=build.slice",
                "-p",
                "MemoryMax=12G",
                "-p",
                "MemorySwapMax=0",
                "--",
            ]);
            if let Some(cpus) = cpus {
                command.args(crate::pin::taskset_args(cpus));
            }
            command.args([
                "nice", "-n", "10", QUEUE, QUEUE_ARG, "run", "--class", class, "--as", as_name,
                "--",
            ]);
            command.arg(program);
            Launch {
                command,
                queued: true,
            }
        }
        Choice::Pgpu => {
            let mut command = queue_command();
            command
                .args(["run", "--class", class, "--as", as_name, "--"])
                .arg(program);
            Launch {
                command,
                queued: true,
            }
        }
    }
}

pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|dir| dir.join(name).is_file() || dir.join(format!("{name}.exe")).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn pfx_direct_runs_the_child_and_the_wrap_names_the_tool() {
        assert!(direct_set(Some("1")));
        assert!(!direct_set(Some("off")));
        assert!(!direct_set(Some("0")));
        assert!(!direct_set(None));

        let direct = launch_with("clip", "pfx", "adapter", true, true, true, None);
        assert!(!direct.queued);
        let envs: Vec<_> = direct.command.get_envs().collect();
        for name in DISPLAY_VARS {
            assert!(envs.contains(&(OsStr::new(name), None)), "{name} kept");
        }
        assert!(envs.contains(&(OsStr::new(HEADLESS_VAR), Some(OsStr::new("1")))));
        assert_eq!(direct.command.get_program(), "adapter");
        assert_eq!(direct.command.get_args().count(), 0);
        assert!(
            launch_with("clip", "pfx", "adapter", false, true, true, None)
                .command
                .get_envs()
                .any(|pair| pair == (OsStr::new(HEADLESS_VAR), Some(OsStr::new("1"))))
        );

        let missing = launch_with("clip", "pfx", "render", false, false, true, None);
        assert!(!missing.queued);
        assert_eq!(missing.command.get_program(), "render");

        let wrapped = launch_with("clip", "pfx", "adapter", false, true, true, Some("2-3,6-7"));
        assert!(wrapped.queued);
        assert_eq!(wrapped.command.get_program(), "systemd-run");
        let args: Vec<_> = wrapped
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--user",
                "--quiet",
                "--scope",
                "--collect",
                "--slice=build.slice",
                "-p",
                "MemoryMax=12G",
                "-p",
                "MemorySwapMax=0",
                "--",
                "taskset",
                "-c",
                "2-3,6-7",
                "nice",
                "-n",
                "10",
                "pci",
                "gpu",
                "run",
                "--class",
                "clip",
                "--as",
                "pfx",
                "--",
                "adapter",
            ]
        );

        let bare = launch_with("truth", "pfx", "render", false, true, false, None);
        assert!(bare.queued);
        assert_eq!(bare.command.get_program(), "pci");
        let args: Vec<_> = bare
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "gpu", "run", "--class", "truth", "--as", "pfx", "--", "render",
            ]
        );
    }

    #[test]
    fn the_job_runs_as_the_name_the_caller_gives() {
        for name in ["alpha", "beta", "delta", "render", "pfx"] {
            let wrapped = launch_with("clip", name, "adapter", false, true, true, Some("2-3"));
            let args: Vec<_> = wrapped
                .command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let at = args.iter().position(|a| a == "--as").unwrap();
            assert_eq!(args[at + 1], name);
            let bare = launch_with("truth", name, "render", false, true, false, None);
            let args: Vec<_> = bare
                .command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                args,
                [
                    "gpu", "run", "--class", "truth", "--as", name, "--", "render"
                ]
            );
        }
    }

    #[test]
    fn only_a_versioned_json_object_is_a_queue() {
        let answer = |text: &[u8]| serde_json::from_slice::<Value>(text).is_ok_and(|v| answers(&v));
        assert!(answer(br#"{"v":1,"paused":false,"play":null,"jobs":[]}"#));
        assert!(answer(b"{\"v\":2}\n"));
        for text in [
            &b"[1]"[..],
            b"[{\"v\":1}]",
            b"{}",
            br#"{"jobs":[]}"#,
            br#""v""#,
            b"1",
            b"null",
        ] {
            assert!(!answer(text), "{}", String::from_utf8_lossy(text));
        }
    }

    #[test]
    fn a_missing_program_is_no_queue() {
        assert!(!probe("pfx-no-such-queue-anywhere"));
        assert!(!probe(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/no-such-dir/pci")
        ));
    }

    #[cfg(unix)]
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp")
            .join(format!("queue-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn tool(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    type Tools<'a> = &'a [(&'a str, &'a str)];

    #[cfg(unix)]
    const ANSWERS: &str = "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\n[ \"$1 $2 $3\" = \"gpu status --json\" ] && echo '{\"v\":1,\"paused\":false,\"play\":null,\"jobs\":[]}'\nexit 0";

    #[cfg(unix)]
    const FAILS: &str =
        "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho '{\"v\":1,\"jobs\":[]}'\nexit 3";

    #[cfg(unix)]
    const SPEAKS: &str = "printf '%s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho \"error: unrecognized subcommand '$1'\"\nexit 0";

    #[cfg(unix)]
    const STALE: &str =
        "printf 'stale %s\\n' \"$*\" >> \"$QUEUE_LOG\"\necho '{\"v\":1,\"jobs\":[]}'\nexit 0";

    #[cfg(unix)]
    #[test]
    fn the_probe_asks_pci_gpu_status_and_reads_its_answer() {
        let dir = scratch("probe");
        let cases = [
            (
                "answers",
                "[ \"$*\" = \"gpu status --json\" ] || exit 9\necho '{\"v\":1,\"jobs\":[]}'",
                true,
            ),
            (
                "elsewhere",
                "[ \"$*\" = \"status --json\" ] || exit 9\necho '{\"v\":1}'",
                false,
            ),
            ("fails", "echo '{\"v\":1,\"jobs\":[]}'\nexit 3", false),
            (
                "speaks",
                "echo \"error: unrecognized subcommand '$1'\"",
                false,
            ),
            ("array", "echo '[{\"v\":1}]'", false),
            ("unversioned", "echo '{\"jobs\":[]}'", false),
            ("crashes", "kill -TERM $$", false),
            ("silent", "exit 0", false),
            (
                "padded",
                "printf '{\"v\":1,\"pad\":\"'\nhead -c 4096 /dev/zero | tr '\\0' a\nprintf '\"}'",
                true,
            ),
            (
                "oversized",
                "printf '{\"v\":1,\"pad\":\"'\nhead -c 1100000 /dev/zero | tr '\\0' a\nprintf '\"}'",
                false,
            ),
        ];
        for (name, body, expected) in cases {
            let path = tool(&dir, name, body);
            assert_eq!(probe(&path), expected, "{name}");
        }
        let answers = tool(
            &dir,
            "reads",
            "echo '{\"v\":1,\"play\":null,\"jobs\":[{\"as\":\"editor\"}]}'",
        );
        let status = status_from(&answers).unwrap();
        assert_eq!(status["jobs"][0]["as"], "editor");
        assert!(status["play"].is_null());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_pci_that_never_finishes_is_no_queue_and_is_ended() {
        let dir = scratch("stalls");
        let cases = [
            ("hangs", "exec sleep 30"),
            ("lingers", "echo '{\"v\":1}'\nexec 1>&-\nexec sleep 30"),
            ("streams", "exec yes '{\"v\":1}'"),
        ];
        for (name, body) in cases {
            let path = tool(&dir, name, body);
            let started = Instant::now();
            assert!(
                status_within(&path, Duration::from_millis(300)).is_none(),
                "{name}"
            );
            assert!(started.elapsed() < Duration::from_secs(20), "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_job_queues_only_behind_a_pci_that_answers_and_asks_once() {
        if std::env::var_os("PFX_QUEUE_CHILD").is_some() {
            return;
        }
        let cases: [(&str, Tools, &str, &[&str]); 5] = [
            (
                "answers",
                &[("pci", ANSWERS)],
                "queued",
                &["gpu status --json"],
            ),
            ("fails", &[("pci", FAILS)], "direct", &["gpu status --json"]),
            (
                "speaks",
                &[("pci", SPEAKS)],
                "direct",
                &["gpu status --json"],
            ),
            ("stale", &[(concat!("p", "gpu"), STALE)], "direct", &[]),
            (
                "both",
                &[(concat!("p", "gpu"), STALE), ("pci", ANSWERS)],
                "queued",
                &["gpu status --json"],
            ),
        ];
        for (name, tools, expected, asked) in cases {
            let dir = scratch(&format!("launch-{name}"));
            for (tool_name, body) in tools {
                tool(&dir, tool_name, body);
            }
            let log = dir.join("log");
            let out = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "queue::tests::launch_child",
                    "--test-threads",
                    "1",
                    "--nocapture",
                ])
                .env("PATH", &dir)
                .env("PFX_QUEUE_CHILD", expected)
                .env("QUEUE_LOG", &log)
                .env_remove("PFX_DIRECT")
                .output()
                .unwrap();
            let logged = std::fs::read_to_string(&log).unwrap_or_default();
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(
                out.status.success(),
                "{name}: the child failed:\n{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(logged.lines().collect::<Vec<_>>(), asked, "{name}");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(
                stderr.contains("no GPU queue on the PATH, so the adapter runs directly"),
                expected == "direct",
                "{name}: {stderr}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn launch_child() {
        let Some(expected) = std::env::var_os("PFX_QUEUE_CHILD") else {
            return;
        };
        for _ in 0..3 {
            let launched = launch("clip", "pfx", "adapter");
            let args: Vec<_> = launched
                .command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if expected == "queued" {
                assert!(launched.queued);
                assert_eq!(launched.command.get_program(), "pci");
                assert_eq!(
                    args,
                    [
                        "gpu", "run", "--class", "clip", "--as", "pfx", "--", "adapter"
                    ]
                );
            } else {
                assert!(!launched.queued);
                assert_eq!(launched.command.get_program(), "adapter");
                assert!(args.is_empty());
            }
        }
    }
}
