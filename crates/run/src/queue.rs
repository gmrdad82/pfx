use std::ffi::OsStr;
use std::process::Command;

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
        on_path("pgpu"),
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
            "{}: pgpu is not installed here; running the adapter directly",
            crate::job::TOOL
        );
    }
    prepare(picked, class, as_name, program, cpus)
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
                "nice", "-n", "10", "pgpu", "run", "--class", class, "--as", as_name, "--",
            ]);
            command.arg(program);
            Launch {
                command,
                queued: true,
            }
        }
        Choice::Pgpu => {
            let mut command = Command::new("pgpu");
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

    #[test]
    fn pfx_direct_runs_the_child_and_the_wrap_names_the_tool() {
        assert!(direct_set(Some("1")));
        assert!(!direct_set(Some("off")));
        assert!(!direct_set(Some("0")));
        assert!(!direct_set(None));

        let direct = launch_with("clip", "pfx", "adapter", true, true, true, None);
        assert!(!direct.queued);
        assert_eq!(direct.command.get_program(), "adapter");
        assert_eq!(direct.command.get_args().count(), 0);

        let missing = launch_with("clip", "pfx", "blender", false, false, true, None);
        assert!(!missing.queued);
        assert_eq!(missing.command.get_program(), "blender");

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
                "pgpu",
                "run",
                "--class",
                "clip",
                "--as",
                "pfx",
                "--",
                "adapter",
            ]
        );

        let bare = launch_with("truth", "pfx", "blender", false, true, false, None);
        assert!(bare.queued);
        assert_eq!(bare.command.get_program(), "pgpu");
        let args: Vec<_> = bare
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            ["run", "--class", "truth", "--as", "pfx", "--", "blender",]
        );
    }

    #[test]
    fn the_job_runs_as_the_name_the_caller_gives() {
        for name in ["alpha", "beta", "delta", "blender", "pfx"] {
            let wrapped = launch_with("clip", name, "adapter", false, true, true, Some("2-3"));
            let args: Vec<_> = wrapped
                .command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            let at = args.iter().position(|a| a == "--as").unwrap();
            assert_eq!(args[at + 1], name);
            let bare = launch_with("truth", name, "blender", false, true, false, None);
            let args: Vec<_> = bare
                .command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                args,
                ["run", "--class", "truth", "--as", name, "--", "blender"]
            );
        }
    }
}
