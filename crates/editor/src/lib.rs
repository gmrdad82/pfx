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

use pfx_play::Game;

pub fn setup() -> session::Setup {
    session::Setup {
        pgpu: pfx_editor_shell::pgpu::wanted() && pfx_editor_shell::pgpu::present(),
        audio: true,
    }
}

pub fn factory<G: Game + 'static>(make: impl Fn() -> G + 'static) -> session::Factory {
    pfx_game::factory(make)
}

#[cfg(test)]
mod pfx_tests;
#[cfg(test)]
mod play_tests;
#[cfg(test)]
mod tests;
