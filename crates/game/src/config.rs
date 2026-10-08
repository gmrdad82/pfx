use std::path::PathBuf;

use pfx_gpu::screens::{Budget, Device, ScreenPolicy};
use pfx_input::{Backend, LayoutSource};
use pfx_live::flat::{Icons, SpriteImage};
use pfx_live::renderer::Exposure;
use pfx_live::upscale::Filter;
use pfx_play::Settings;

pub const WARM_BUDGET: f64 = 2.0;

pub enum Pads {
    None,
    Gilrs,
    Backend(Box<dyn Backend>),
}

pub type Crash = Box<dyn Fn(&str)>;

pub struct Config {
    pub name: String,
    pub title: Option<String>,
    pub crash: Option<Crash>,
    pub version: String,
    pub scene: Option<PathBuf>,
    pub settings: Settings,
    pub screen: ScreenPolicy,
    pub device: Option<Device>,
    pub render_scale: f32,
    pub budget: Option<Budget>,
    pub filter: Filter,
    pub exposure: Exposure,
    pub fonts: Vec<Vec<u8>>,
    pub pads: Pads,
    pub seed: u64,
    pub sound: bool,
    pub layout: Option<Box<dyn LayoutSource>>,
    pub icons: Option<Icons>,
    pub sprites: Option<SpriteImage>,
    pub environment: Option<EnvironmentTexels>,
    pub warm_budget: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentTexels {
    pub width: u32,
    pub height: u32,
    pub linear: Vec<[f32; 4]>,
    pub intensity: f32,
}

impl Config {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        scene: impl Into<PathBuf>,
    ) -> Self {
        Self {
            scene: Some(scene.into()),
            ..Self::flat(name, version)
        }
    }

    pub fn flat(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            title: None,
            crash: None,
            version: version.into(),
            scene: None,
            settings: Settings::default(),
            screen: ScreenPolicy::default(),
            device: None,
            render_scale: 1.0,
            budget: None,
            filter: Filter::Sharp,
            exposure: Exposure::Auto { bias: 0.0 },
            fonts: Vec::new(),
            pads: Pads::Gilrs,
            seed: 0,
            sound: true,
            layout: None,
            icons: None,
            sprites: None,
            environment: None,
            warm_budget: WARM_BUDGET,
        }
    }

    pub fn bind_text(&self) {
        pfx_load::scene::bind("name", &self.name);
        pfx_load::scene::bind("version", &self.version);
    }

    pub fn warm_budget(mut self, seconds: f64) -> Self {
        self.warm_budget = seconds;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn window_title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }

    pub fn crash(mut self, hook: impl Fn(&str) + 'static) -> Self {
        self.crash = Some(Box::new(hook));
        self
    }

    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    pub fn screen(mut self, screen: ScreenPolicy) -> Self {
        self.screen = screen;
        self
    }

    pub fn device(mut self, device: Device) -> Self {
        self.device = Some(device);
        self
    }

    pub fn render_scale(mut self, scale: f32) -> Self {
        self.render_scale = scale;
        self
    }

    pub fn budget(mut self, budget: Budget) -> Self {
        self.budget = Some(budget);
        self
    }

    pub fn exposure(mut self, exposure: Exposure) -> Self {
        self.exposure = exposure;
        self
    }

    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    pub fn font(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.fonts.push(bytes.into());
        self
    }

    pub fn pads(mut self, pads: Pads) -> Self {
        self.pads = pads;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn layout(mut self, layout: Box<dyn LayoutSource>) -> Self {
        self.layout = Some(layout);
        self
    }

    pub fn icons(mut self, icons: Icons) -> Self {
        self.icons = Some(icons);
        self
    }

    pub fn sprites(mut self, sprites: SpriteImage) -> Self {
        self.sprites = Some(sprites);
        self
    }

    pub fn environment(mut self, environment: EnvironmentTexels) -> Self {
        self.environment = Some(environment);
        self
    }

    pub fn sound(mut self, on: bool) -> Self {
        self.sound = on;
        self
    }
}

pub(crate) fn report(name: &str, crash: Option<&Crash>, error: &str) {
    eprintln!("{name}: {error}");
    if let Some(crash) = crash {
        crash(error);
    }
}
