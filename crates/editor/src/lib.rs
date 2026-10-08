pub mod audio;
pub mod check;
pub mod editor;
pub mod input;
pub mod inspect;
pub mod level;
pub mod log;
pub mod outline;
pub mod pgpu;
pub mod script;
pub mod session;
pub mod snap;
pub mod theme;
pub mod ui;
pub mod ui_level;
pub mod view;

use std::path::Path;

use pfx_play::Game;

pub const USAGE: &str = "usage: pfx edit <scene.toml>|<project folder>";

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

pub fn run(args: &[String]) -> Result<(), String> {
    match args {
        [path] if !path.starts_with('-') => {
            let path = Path::new(path);
            if path.is_dir() {
                return window(session::Session::open_project(path, setup(), None)?);
            }
            if !path.is_file() {
                return Err(format!(
                    "{}: no such scene file or project folder",
                    path.display()
                ));
            }
            window(session::Session::open(path, setup())?)
        }
        _ => Err(USAGE.to_string()),
    }
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
mod tests;
