#![cfg_attr(not(feature = "tools"), windows_subsystem = "windows")]

use std::process::ExitCode;

use pfx_game::{Config, Game, Input, Label, Tick, Ui, UiFrame, World};

const FONT: &[u8] = pfx_text::fixture::DM_SANS;

struct Empty;

impl Game for Empty {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, _world: &mut World, _tick: &Tick, _input: &Input) {}

    fn ui(&mut self, _world: &World, frame: &UiFrame) -> Ui {
        let [x, y, _, _] = frame.safe;
        let mut ui = Ui::new();
        ui.label(Label::new(
            "empty world",
            "DM Sans",
            40.0 * frame.ui_scale,
            [x + 48.0, y + 48.0],
        ));
        ui
    }
}

fn main() -> ExitCode {
    let world = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/empty/world/empty.scene.toml"
    );
    let config = Config::new("empty", env!("CARGO_PKG_VERSION"), world).font(FONT);
    pfx_game::launch(
        || Empty,
        config,
        |commands| {
            commands.add("hello", "prints a greeting", |_| {
                println!("hello from the empty world");
            });
        },
    )
}
