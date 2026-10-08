use pfx_gpu::screens::Device;
use pfx_gpu::window::Size;
use pfx_input::Backend;
use pfx_load::scene::Scene;
use pfx_play::{Game, Options, PlaySession, Rigs, Settings, Tunables};

use crate::config::Config;
use crate::driver::Driver;
use crate::painter::Painter;

pub(crate) fn scene(config: &Config) -> Result<Option<Scene>, String> {
    config.bind_text();
    let Some(path) = &config.scene else {
        return Ok(None);
    };
    if !path.is_file() {
        return Err(format!("{}: no such scene file", path.display()));
    }
    Scene::open(path)
        .map(Some)
        .map_err(|error| format!("{}: {error:?}", path.display()))
}

pub(crate) fn settings(game: &dyn Game, config: &Config) -> Settings {
    game.settings()
        .cloned()
        .unwrap_or_else(|| config.settings.clone())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn driver(
    scene: Option<&Scene>,
    painter: Option<&Painter>,
    game: Box<dyn Game>,
    config: &Config,
    device: Device,
    window: Size,
    audio: (u32, u16),
    pads: Option<Box<dyn Backend>>,
) -> Result<Driver, String> {
    let first = settings(game.as_ref(), config);
    let empty = Scene::empty();
    let flat = scene.is_none();
    let mut session = PlaySession::play_with(
        scene.unwrap_or(&empty),
        painter.and_then(Painter::staged),
        game,
        Options {
            seed: config.seed,
            audio_rate: audio.0,
            audio_channels: audio.1,
            tunables: flat.then(Tunables::default),
            rigs: flat.then(Rigs::default),
            ..Options::default()
        },
    )
    .map_err(|error| error.to_string())?;
    if let Some(painter) = painter {
        session.set_frame_stats(painter.renderer().frame_stats());
    }
    let settings = session.game().settings().cloned().unwrap_or(first);
    Ok(Driver::new(
        session,
        settings,
        config.screen.clone(),
        device,
        window,
        pads,
    ))
}
