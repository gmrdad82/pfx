use std::path::Path;
use std::process::Command;

const MARK: &str = "PFX_SLICE";

const MACHINE_ENV: Option<&str> = option_env!("PFX_MACHINE_ENV");

pub fn build_cpus() -> Option<String> {
    let path = MACHINE_ENV?;
    let cpus = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| setting(&text, "BUILD_CPUS"));
    if cpus.is_none() {
        eprintln!(
            "{}: {path} names no BUILD_CPUS; running unpinned",
            crate::job::TOOL
        );
    }
    cpus
}

pub fn setting(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .filter(|(name, _)| name.trim().trim_start_matches("export ").trim() == key)
        .map(|(_, value)| {
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        })
        .rfind(|value| !value.is_empty())
}

pub fn taskset_args(cpus: &str) -> [String; 3] {
    ["taskset".into(), "-c".into(), cpus.to_string()]
}

pub fn should_reenter(direct: bool, inside: bool, systemd: bool, taskset: bool) -> bool {
    !direct && !inside && systemd && taskset
}

pub fn reentry(exe: &Path, args: &[String], cpus: Option<&str>) -> Command {
    let mut command = Command::new("systemd-run");
    command
        .args([
            "--user",
            "--quiet",
            "--scope",
            "--collect",
            "--slice=build.slice",
            "--",
        ])
        .args(cpus.map(taskset_args).into_iter().flatten())
        .args(["nice", "-n", "10"])
        .arg(exe)
        .args(args)
        .env(MARK, "1");
    command
}

pub fn reenter_if_needed(args: &[String]) -> Option<i32> {
    MACHINE_ENV?;
    let direct = std::env::var("PFX_DIRECT").ok().as_deref() == Some("1");
    let inside = std::env::var_os(MARK).is_some();
    if !should_reenter(
        direct,
        inside,
        crate::queue::on_path("systemd-run"),
        crate::queue::on_path("taskset"),
    ) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let status = reentry(&exe, args, build_cpus().as_deref()).status().ok()?;
    Some(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_cpus_come_from_the_machine_file_and_the_last_value_wins() {
        let text = "# tuned for this machine\nQUEUE_SLOTS=2\nBUILD_CPUS=2-3,6-7\nexport BUILD_CPUS=\"4-5\"\n";
        assert_eq!(setting(text, "BUILD_CPUS").as_deref(), Some("4-5"));
        assert_eq!(setting(text, "QUEUE_SLOTS").as_deref(), Some("2"));
        assert_eq!(setting(text, "MISSING"), None);
        assert_eq!(setting("BUILD_CPUS=\n", "BUILD_CPUS"), None);
    }

    #[test]
    fn without_a_cpu_set_the_reentry_runs_unpinned() {
        let command = reentry(Path::new("/opt/pfx"), &["list".into()], None);
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args.iter().any(|a| a == "taskset"));
        assert!(args.windows(3).any(|w| w == ["nice", "-n", "10"]));
    }

    #[test]
    fn pfx_reenters_the_build_slice_once_and_never_in_tests() {
        assert!(should_reenter(false, false, true, true));
        assert!(!should_reenter(true, false, true, true));
        assert!(!should_reenter(false, true, true, true));
        assert!(!should_reenter(false, false, false, true));
        assert!(!should_reenter(false, false, true, false));
    }

    #[test]
    fn the_reentry_runs_pinned_and_niced_in_the_build_slice() {
        let command = reentry(
            Path::new("/opt/pfx"),
            &["render".into(), "--app".into(), "alpha".into()],
            Some("2-3,6-7"),
        );
        assert_eq!(command.get_program(), "systemd-run");
        let args: Vec<String> = command
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
                "--",
                "taskset",
                "-c",
                "2-3,6-7",
                "nice",
                "-n",
                "10",
                "/opt/pfx",
                "render",
                "--app",
                "alpha"
            ]
        );
        assert!(
            command
                .get_envs()
                .any(|(k, v)| k == MARK && v.is_some_and(|v| v == "1"))
        );
    }
}
