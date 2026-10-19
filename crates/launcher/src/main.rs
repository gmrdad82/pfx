#![cfg_attr(windows, windows_subsystem = "windows")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const CLI: &str = if cfg!(windows) { "pfx.com" } else { "pfx" };

fn cli(launcher: &Path) -> PathBuf {
    launcher.with_file_name(CLI)
}

fn line(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    std::iter::once(OsString::from("gui")).chain(args).collect()
}

fn main() -> ExitCode {
    let Ok(launcher) = std::env::current_exe() else {
        return ExitCode::from(1);
    };
    let mut command = Command::new(cli(&launcher));
    command
        .args(line(std::env::args_os().skip(1)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.spawn() {
        Ok(_) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_starts_the_console_tool_beside_it_with_gui_and_every_argument() {
        let launcher = Path::new("C:/Program Files/pfx/pfx.exe");
        assert_eq!(cli(launcher), Path::new("C:/Program Files/pfx").join(CLI));
        assert_eq!(
            line(["C:/scenes/room.pitofx".into(), "--play".into()]),
            ["gui", "C:/scenes/room.pitofx", "--play"].map(OsString::from)
        );
    }
}
