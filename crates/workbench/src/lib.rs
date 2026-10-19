mod app;
mod backdrop;
mod cameras;
mod clay;
mod clipboard;
mod curves;
mod document;
mod edit;
mod file;
mod fixture;
mod follow;
mod game;
mod gizmo;
mod handles;
mod mark;
mod open;
mod orbit;
mod overlay;
mod panel;
mod pick;
mod routes;
mod sample;
mod scenes;
mod shell;
mod shots;
mod shown;
mod spans;
mod start;
mod tape;
mod theme;
mod timeline;
mod undo;
mod viewer;
mod viewport;
mod views;
mod wire;

use pfx_editor_style::widgets;

pub use app::Workbench;
pub use clay::Shading;
pub use curves::{Change, CurveEdit, TRACKS as CURVE_TRACKS};
pub use fixture::Fixture;
pub use follow::{Follow, Mode, Shape};
pub use game::{EDIT_USAGE, Offscreen, Out, copy_project, edit_project, offscreen};
pub use pfx_editor_shell::{Error, Pulse, Script, shot, window};
pub use scenes::{is_project, is_scene};
pub use shell::{PLAY_DIR, Shell, recipe_scene};
pub use start::{start_recipe, start_recipe_here};
pub use tape::{
    Act, Layout, MAX_FRAMES, Tape, View, frame_png, play, poster_png, steps as tape_steps,
};
pub use viewer::Viewer;
pub use wire::Look;

use std::path::Path;

use pfx_editor::session::{Factory, Session};

pub fn scene_shell(path: &Path) -> Result<Shell, String> {
    Session::open(path, pfx_editor::setup()).map(Shell::edited)
}

pub fn project_shell(folder: &Path) -> Result<Shell, String> {
    Session::open_project(folder, pfx_editor::setup(), None).map(Shell::edited)
}

pub fn run_project(
    project: &Path,
    factory: Factory,
    config: pfx_game::Config,
) -> Result<(), String> {
    let session = Session::open_game(project, pfx_editor::setup(), factory, config)?;
    let mut shell = Shell::edited(session);
    window(Workbench::new(&mut shell, None), None).map_err(|error| error.to_string())
}

#[cfg(test)]
use pfx_editor_shell::{encode_png, gpu_turn};

#[cfg(test)]
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/ui-tests")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("scene.toml")
}
