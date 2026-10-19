use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const BUNDLE_ID: &str = "com.pitomd.pfx";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub const HOST: Os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Window {
    Here,
    Spawn {
        program: PathBuf,
        args: Vec<OsString>,
        detached: bool,
    },
}

pub fn args(exe: Option<&Path>, given: Vec<String>) -> Vec<String> {
    let lone_psn = matches!(given.as_slice(), [only] if only.starts_with("-psn_"));
    if Os::HOST == Os::MacOs && lone_psn && exe.is_some_and(in_bundle) {
        return Vec::new();
    }
    given
}

pub fn window(os: Os, exe: &Path, args: &[String]) -> Window {
    match os {
        Os::Linux => Window::Here,
        Os::MacOs if args.is_empty() && !in_bundle(exe) => Window::Spawn {
            program: PathBuf::from("open"),
            args: ["-b", BUNDLE_ID].map(OsString::from).to_vec(),
            detached: false,
        },
        Os::MacOs => Window::Here,
        Os::Windows if console(exe) => Window::Spawn {
            program: exe.with_file_name("pfx.exe"),
            args: args.iter().map(OsString::from).collect(),
            detached: true,
        },
        Os::Windows => Window::Here,
    }
}

pub fn start(window: &Window) -> Result<bool, String> {
    let Window::Spawn {
        program,
        args,
        detached,
    } = window
    else {
        return Ok(false);
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    if *detached {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let shown = program.display();
    if *detached {
        command
            .spawn()
            .map(|_| true)
            .map_err(|error| format!("{shown}: {error}"))
    } else {
        match command.status() {
            Ok(status) if status.success() => Ok(true),
            Ok(status) => Err(format!("{shown} {}: {status}", join(args))),
            Err(error) => Err(format!("{shown}: {error}")),
        }
    }
}

fn join(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn console(exe: &Path) -> bool {
    exe.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("com"))
}

pub fn in_bundle(exe: &Path) -> bool {
    let folder = exe.parent();
    let contents = folder.and_then(Path::parent);
    let bundle = contents.and_then(Path::parent);
    folder
        .and_then(Path::file_name)
        .is_some_and(|name| name == "MacOS")
        && contents
            .and_then(Path::file_name)
            .is_some_and(|name| name == "Contents")
        && bundle
            .and_then(Path::extension)
            .is_some_and(|extension| extension == "app")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn linux_opens_the_window_in_the_process_it_was_asked_in() {
        let exe = Path::new("/usr/local/bin/pfx");
        assert_eq!(window(Os::Linux, exe, &[]), Window::Here);
        assert_eq!(
            window(Os::Linux, exe, &strings(&["room.pitofx"])),
            Window::Here
        );
    }

    #[test]
    fn a_bare_call_outside_the_app_bundle_opens_the_bundle_and_one_inside_draws() {
        let outside = Path::new("/usr/local/bin/pfx");
        assert_eq!(
            window(Os::MacOs, outside, &[]),
            Window::Spawn {
                program: PathBuf::from("open"),
                args: ["-b", "com.pitomd.pfx"].map(OsString::from).to_vec(),
                detached: false,
            }
        );
        assert_eq!(
            window(Os::MacOs, outside, &strings(&["room.pitofx"])),
            Window::Here
        );
        let inside = Path::new("/Applications/pfx.app/Contents/MacOS/pfx");
        assert!(in_bundle(inside));
        assert!(!in_bundle(Path::new("/opt/pfx/Contents/MacOS/pfx")));
        assert_eq!(window(Os::MacOs, inside, &[]), Window::Here);
        assert_eq!(
            window(Os::MacOs, inside, &strings(&["room.pitofx"])),
            Window::Here
        );
    }

    #[test]
    fn the_console_tool_hands_a_window_call_to_the_launcher_beside_it_detached() {
        let console = Path::new("C:/Program Files/pfx/pfx.com");
        assert_eq!(
            window(Os::Windows, console, &strings(&["C:\\scenes\\room.pitofx"])),
            Window::Spawn {
                program: Path::new("C:/Program Files/pfx").join("pfx.exe"),
                args: vec![OsString::from("C:\\scenes\\room.pitofx")],
                detached: true,
            }
        );
        assert_eq!(
            window(Os::Windows, Path::new("C:/pfx/PFX.COM"), &[]),
            Window::Spawn {
                program: Path::new("C:/pfx").join("pfx.exe"),
                args: Vec::new(),
                detached: true,
            }
        );
        assert_eq!(
            window(Os::Windows, Path::new("C:/dev/target/release/pfx.exe"), &[]),
            Window::Here
        );
    }

    #[test]
    fn the_window_plan_is_left_alone_when_it_draws_here() {
        assert_eq!(start(&Window::Here), Ok(false));
        let given = strings(&["room.pitofx"]);
        assert_eq!(args(None, given.clone()), given);
    }
}
