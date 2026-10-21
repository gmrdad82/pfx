#![cfg_attr(not(feature = "tools"), windows_subsystem = "windows")]

use std::process::ExitCode;
use std::thread::JoinHandle;

use pfx_game::flat::Srgba;
use pfx_game::{
    Anchor, Config, Game, GameError, Input, Label, Renderer, SplashSettings, SplashShow, Tick, Ui,
    UiFrame, Warm, Warming, World,
};

const FONT: &[u8] = pfx_text::fixture::DM_SANS;
const FAMILY: &str = "DM Sans";
const IDENT: f64 = 1.5;
const TITLE: &str = "A GENERATED BOOT";
const CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .";
const SIZES: [f32; 2] = [96.0, 32.0];

struct Boot {
    seconds: f64,
    warmed: Vec<(u32, bool, Option<[u32; 2]>)>,
    first: Option<[u32; 2]>,
    loader: bool,
    loading: Option<JoinHandle<()>>,
}

impl Game for Boot {
    fn start(&mut self, _world: &mut World) {}

    fn tick(&mut self, world: &mut World, _tick: &Tick, _input: &Input) {
        if world.time() >= self.seconds {
            world.quit(0);
        }
    }

    fn warm(&mut self, warm: &mut Warm<'_>) -> Result<Warming, GameError> {
        self.warmed.push((warm.pass(), warm.again(), warm.size()));
        if self.loader
            && self.loading.is_none()
            && let Some(renderer) = warm.renderer()
        {
            let gpu = renderer.gpu().clone();
            let [width, height] = warm.size().unwrap_or([1920, 1080]);
            self.loading = Some(std::thread::spawn(move || {
                if let Err(error) = Renderer::new(gpu, width, height) {
                    eprintln!("boot: the loader's renderer: {error}");
                }
            }));
        }
        warm.glyphs(FAMILY, 400, &SIZES, CHARS)?;
        Ok(Warming::Done)
    }

    fn ui(&mut self, world: &World, frame: &UiFrame) -> Ui {
        let [width, height] = frame.layout;
        let centre = [width * 0.5, height * 0.5];
        let time = world.time() as f32;
        self.first.get_or_insert(frame.window);
        let mut ui = Ui::new();
        ui.clear(Srgba::hex(0x101820));
        if world.time() < IDENT {
            let alpha = 1.0 - (1.0 - (time / 0.5).min(1.0)).powi(3);
            ui.label(
                Label::new("IDENT", FAMILY, SIZES[0], centre)
                    .anchor(Anchor::Center)
                    .colour(Srgba([1.0, 1.0, 1.0, alpha])),
            );
            return ui;
        }
        let step = SIZES[0] * 0.62;
        let start = centre[0] - step * (TITLE.len() as f32 - 1.0) * 0.5;
        for (index, letter) in TITLE.chars().enumerate() {
            let lift = (time * 3.0 + index as f32 * 0.4).sin() * 12.0;
            ui.label(
                Label::new(
                    letter.to_string(),
                    FAMILY,
                    SIZES[0],
                    [start + step * index as f32, centre[1] - SIZES[0] + lift],
                )
                .anchor(Anchor::Center),
            );
        }
        let blink = 0.5 + 0.5 * (time * 4.0).sin();
        ui.label(
            Label::new(
                "PRESS START",
                FAMILY,
                SIZES[1],
                [centre[0], centre[1] + SIZES[0]],
            )
            .anchor(Anchor::Center)
            .colour(Srgba([1.0, 1.0, 1.0, blink])),
        );
        ui
    }

    fn stop(&mut self) {
        if let Some(loading) = self.loading.take() {
            let _ = loading.join();
        }
        for (pass, again, size) in &self.warmed {
            let again = if *again { " again" } else { "" };
            match size {
                Some([width, height]) => eprintln!("warm pass {pass}{again}: {width}x{height}"),
                None => eprintln!("warm pass {pass}{again}: no size"),
            }
        }
        if let Some([width, height]) = self.first {
            eprintln!("first frame: {width}x{height}");
        }
    }
}

fn main() -> ExitCode {
    let seconds = std::env::var("BOOT_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(6.0);
    let splash = SplashSettings {
        show: SplashShow::Always,
        ..SplashSettings::default()
    };
    let config = Config::flat("boot", env!("CARGO_PKG_VERSION"))
        .font(FONT)
        .splash(splash);
    let boot = Boot {
        seconds,
        warmed: Vec::new(),
        first: None,
        loader: std::env::var_os("BOOT_RENDERER").is_some_and(|value| value == "1"),
        loading: None,
    };
    match pfx_game::run(boot, config) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("boot: {error}");
            ExitCode::FAILURE
        }
    }
}
