pub mod audio;
pub mod check;
pub mod editor;
mod held;
pub mod input;
pub mod inspect;
pub mod level;
pub mod log;
pub mod outline;
pub mod pgpu;
pub mod play;
pub mod script;
pub mod session;
pub mod snap;
pub mod theme;
pub mod ui;
pub mod ui_level;
pub mod view;

use std::path::{Path, PathBuf};

use pfx_play::Game;

pub const USAGE: &str = "usage: pfx edit <scene.toml>|<project folder> [--play <module.wasm>]";

fn setup() -> session::Setup {
    session::Setup {
        pgpu: std::env::var("PFX_EDIT_PGPU").map_or(true, |value| value != "off")
            && std::env::var_os("PATH").is_some_and(|path| pgpu::on_path(&path)),
        audio: true,
    }
}

fn window(session: session::Session) -> Result<(), String> {
    pfx_editor_shell::window(session, None).map_err(|error| error.to_string())
}

pub fn parse(args: &[String]) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut path = None;
    let mut module = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--play" if module.is_none() => match rest.next() {
                Some(value) if !value.starts_with('-') => module = Some(PathBuf::from(value)),
                _ => return Err(USAGE.to_string()),
            },
            flag if flag.starts_with('-') => return Err(USAGE.to_string()),
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => return Err(USAGE.to_string()),
        }
    }
    path.map(|path| (path, module))
        .ok_or_else(|| USAGE.to_string())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (path, module) = parse(args)?;
    if let Some(module) = &module
        && !module.is_file()
    {
        return Err(format!("{}: no such module file", module.display()));
    }
    let mut session = if path.is_dir() {
        session::Session::open_project(&path, setup(), None)?
    } else if path.is_file() {
        session::Session::open(&path, setup())?
    } else {
        return Err(format!(
            "{}: no such scene file or project folder",
            path.display()
        ));
    };
    if let Some(module) = module {
        let module = std::path::absolute(&module).unwrap_or(module);
        session
            .editor
            .module
            .set_module(Some(module), play::From::Flag);
    }
    window(session)
}

pub fn factory<G: Game + 'static>(make: impl Fn() -> G + 'static) -> session::Factory {
    pfx_game::factory(make)
}

pub fn run_project(
    project: &Path,
    factory: session::Factory,
    config: pfx_game::Config,
) -> Result<(), String> {
    window(session::Session::open_game(
        project,
        setup(),
        factory,
        config,
    )?)
}

#[cfg(test)]
mod pfx_tests;
#[cfg(test)]
mod play_tests;
#[cfg(test)]
mod tests;
