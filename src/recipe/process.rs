use std::process::{Command, ExitCode};

pub(super) const CPUS: &str = "PFX_CPUS";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Machine<'a> {
    pub(super) queued: bool,
    pub(super) pgpu: bool,
    pub(super) systemd: bool,
    pub(super) cpus: Option<&'a str>,
}

pub(super) fn line(
    command: &str,
    exe: &str,
    args: &[String],
    machine: Machine<'_>,
) -> Option<Vec<String>> {
    if machine.queued || !machine.pgpu {
        return None;
    }
    let mut line: Vec<String> = Vec::new();
    if machine.systemd {
        line.extend(
            [
                "systemd-run",
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
            ]
            .map(String::from),
        );
        if let Some(cpus) = machine.cpus {
            line.extend(["taskset", "-c", cpus].map(String::from));
        }
        line.extend(["nice", "-n", "10"].map(String::from));
    }
    line.extend(queue_run());
    line.push(exe.to_string());
    line.push(command.to_string());
    line.extend(args.iter().cloned());
    Some(line)
}

pub(super) fn on_path(name: &str) -> bool {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(&file).is_file()))
}

pub(super) fn cpus(value: Option<&std::ffi::OsStr>) -> Option<String> {
    value
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn queue_run() -> Vec<String> {
    [
        pfx_run::queue::QUEUE,
        pfx_run::queue::QUEUE_ARG,
        "run",
        "--class",
        "clip",
        "--as",
        "pfx",
        "--",
    ]
    .map(String::from)
    .to_vec()
}

pub(super) fn unqueued(command: &str, subject: &str, machine: Machine<'_>) -> Option<String> {
    (!machine.queued && !machine.pgpu).then(|| {
        format!(
            "pfx {command}: {}, so {subject}, paced the same",
            pfx_run::queue::NO_QUEUE
        )
    })
}

pub(super) fn found(command: &str, subject: &str) -> (bool, bool) {
    let queued = std::env::var_os("GPU_QUEUE").is_some();
    let pgpu = !queued && pfx_run::queue::present();
    let machine = Machine {
        queued,
        pgpu,
        systemd: false,
        cpus: None,
    };
    if let Some(note) = unqueued(command, subject, machine) {
        eprintln!("{note}");
    }
    (queued, pgpu)
}

pub(super) fn requeue(command: &str, args: &[String], direct: &str) -> Option<ExitCode> {
    let cpus = on_path("taskset")
        .then(|| {
            cpus(std::env::var_os(CPUS).as_deref())
                .or_else(|| cpus(option_env!("PFX_CPUS").map(std::ffi::OsStr::new)))
        })
        .flatten();
    let (queued, pgpu) = found(command, &format!("the {command} {direct}"));
    let machine = Machine {
        queued,
        pgpu,
        systemd: on_path("systemd-run"),
        cpus: cpus.as_deref(),
    };
    let exe = std::env::current_exe().ok()?;
    let line = line(command, &exe.display().to_string(), args, machine)?;
    Some(run(command, &line))
}

#[cfg(unix)]
fn run(command: &str, line: &[String]) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let error = Command::new(&line[0]).args(&line[1..]).exec();
    eprintln!("pfx {command}: cannot start {}: {error}", line[0]);
    ExitCode::FAILURE
}

#[cfg(not(unix))]
fn run(command: &str, line: &[String]) -> ExitCode {
    match Command::new(&line[0]).args(&line[1..]).status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from),
        Err(error) => {
            eprintln!("pfx {command}: cannot start {}: {error}", line[0]);
            ExitCode::FAILURE
        }
    }
}

pub(super) fn real(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let real = path.canonicalize()?;
    Ok(
        match real.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
            Some(rest) if cfg!(windows) && rest.as_bytes().get(1) == Some(&b':') => rest.into(),
            _ => real,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn machine(queued: bool, pgpu: bool) -> Machine<'static> {
        Machine {
            queued,
            pgpu,
            systemd: false,
            cpus: None,
        }
    }

    #[test]
    fn without_the_queue_the_render_runs_directly_with_one_plain_note() {
        let args = vec!["a.toml".to_string()];
        let absent = machine(false, false);
        assert_eq!(line("render", "pfx", &args, absent), None);
        assert_eq!(
            unqueued("render", "the render runs directly", absent).as_deref(),
            Some(
                "pfx render: no GPU queue on the PATH, so the render runs directly, paced the same"
            )
        );
        assert_eq!(
            unqueued("render", "the render runs directly", machine(false, true)),
            None
        );
        assert_eq!(
            unqueued("render", "the render runs directly", machine(true, false)),
            None
        );
    }

    #[test]
    fn the_cpu_set_comes_only_from_its_variable() {
        assert_eq!(cpus(None), None);
        assert_eq!(cpus(Some(OsStr::new(""))), None);
        assert_eq!(cpus(Some(OsStr::new("  "))), None);
        assert_eq!(cpus(Some(OsStr::new(" 2-5 "))).as_deref(), Some("2-5"));
        assert_eq!(CPUS, "PFX_CPUS");
    }

    #[test]
    fn a_tool_that_is_not_on_the_path_is_absent() {
        assert!(!on_path("pfx-no-such-tool-anywhere"));
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
    fn render_clip_gui_and_shoot_queue_only_behind_a_pci_that_answers() {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("PFX_REQUEUE_CHILD").is_some() {
            return;
        }
        let exe = std::env::current_exe().unwrap().display().to_string();
        let ran = |command: &str| {
            format!("gpu run --class clip --as pfx -- {exe} {command} a.toml --camera main")
        };
        let asked = "gpu status --json".to_string();
        let old = concat!("p", "gpu");
        let cases: [(&str, Tools, &str, bool, Vec<String>, Option<&str>); 14] = [
            (
                "render-answers",
                &[("pci", ANSWERS)],
                "render",
                false,
                vec![asked.clone(), ran("render")],
                None,
            ),
            (
                "clip-answers",
                &[("pci", ANSWERS)],
                "clip",
                false,
                vec![asked.clone(), ran("clip")],
                None,
            ),
            (
                "gui-answers",
                &[("pci", ANSWERS)],
                "gui",
                false,
                vec![asked.clone(), ran("gui")],
                None,
            ),
            (
                "render-fails",
                &[("pci", FAILS)],
                "render",
                false,
                vec![asked.clone(), "direct".into()],
                Some(
                    "pfx render: no GPU queue on the PATH, so the render runs directly, paced the same",
                ),
            ),
            (
                "clip-speaks",
                &[("pci", SPEAKS)],
                "clip",
                false,
                vec![asked.clone(), "direct".into()],
                Some(
                    "pfx clip: no GPU queue on the PATH, so the clip traces directly, paced the same",
                ),
            ),
            (
                "gui-fails",
                &[("pci", FAILS)],
                "gui",
                false,
                vec![asked.clone(), "direct".into()],
                Some(
                    "pfx gui: no GPU queue on the PATH, so the gui draws directly, paced the same",
                ),
            ),
            (
                "render-stale",
                &[(old, STALE)],
                "render",
                false,
                vec!["direct".into()],
                Some("pfx render: no GPU queue on the PATH"),
            ),
            (
                "clip-both",
                &[(old, STALE), ("pci", ANSWERS)],
                "clip",
                false,
                vec![asked.clone(), ran("clip")],
                None,
            ),
            (
                "render-queued",
                &[("pci", ANSWERS)],
                "render",
                true,
                vec!["direct".into()],
                None,
            ),
            (
                "clip-queued",
                &[("pci", ANSWERS)],
                "clip",
                true,
                vec!["direct".into()],
                None,
            ),
            (
                "shoot-answers",
                &[("pci", ANSWERS)],
                "shoot",
                false,
                vec![
                    asked.clone(),
                    "queue true".into(),
                    "queue true".into(),
                    "queue true".into(),
                ],
                None,
            ),
            (
                "shoot-fails",
                &[("pci", FAILS)],
                "shoot",
                false,
                vec![
                    asked.clone(),
                    "queue false".into(),
                    "queue false".into(),
                    "queue false".into(),
                ],
                Some("pfx shoot: no GPU queue on the PATH, so pfx runs directly, paced the same"),
            ),
            (
                "shoot-stale",
                &[(old, STALE)],
                "shoot",
                false,
                vec![
                    "queue false".into(),
                    "queue false".into(),
                    "queue false".into(),
                ],
                Some("pfx shoot: no GPU queue on the PATH"),
            ),
            (
                "shoot-queued",
                &[("pci", ANSWERS)],
                "shoot",
                true,
                vec![
                    "queue false".into(),
                    "queue false".into(),
                    "queue false".into(),
                ],
                None,
            ),
        ];
        for (name, tools, command, queued, expected, note) in cases {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tmp")
                .join(format!("requeue-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            for (tool, body) in tools {
                let path = dir.join(tool);
                std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let log = dir.join("log");
            let mut child = Command::new(&exe);
            child
                .args([
                    "--exact",
                    "recipe::process::tests::requeue_child",
                    "--test-threads",
                    "1",
                    "--nocapture",
                ])
                .env("PATH", &dir)
                .env("PFX_REQUEUE_CHILD", command)
                .env("QUEUE_LOG", &log)
                .env_remove("GPU_QUEUE")
                .env_remove("GPU_QUEUE_PID")
                .env_remove(CPUS);
            if queued {
                child.env("GPU_QUEUE", "1");
            }
            let out = child.output().unwrap();
            let logged = std::fs::read_to_string(&log).unwrap_or_default();
            std::fs::remove_dir_all(&dir).unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{name}: the child failed:\n{stderr}");
            assert_eq!(logged.lines().collect::<Vec<_>>(), expected, "{name}");
            match note {
                Some(note) => assert!(stderr.contains(note), "{name}: {stderr}"),
                None => assert!(!stderr.contains("no GPU queue"), "{name}: {stderr}"),
            }
            assert!(!logged.contains("stale"), "{name}: {logged}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn requeue_child() {
        use std::io::Write;
        let Some(command) = std::env::var("PFX_REQUEUE_CHILD").ok() else {
            return;
        };
        let note = |line: &str| {
            let log = std::env::var_os("QUEUE_LOG").unwrap();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .unwrap();
            writeln!(file, "{line}").unwrap();
        };
        let args: Vec<String> = ["a.toml", "--camera", "main"].map(String::from).to_vec();
        let requeued = match command.as_str() {
            "render" => super::super::render::requeue(&args),
            "clip" => super::super::clip::requeue(&args),
            "gui" => requeue("gui", &args, "draws directly"),
            _ => {
                for _ in 0..3 {
                    let (_, queue) = found("shoot", "pfx runs directly");
                    note(&format!("queue {queue}"));
                }
                return;
            }
        };
        assert!(requeued.is_none(), "{command}: the queue did not start");
        note("direct");
    }
}
