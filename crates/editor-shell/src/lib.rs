pub mod focus;
pub mod mean;
mod offscreen;
pub mod paint;
pub mod pgpu;
pub mod profile;
mod script;
pub mod shutter;
mod window;
pub mod worker;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use egui::{Context, Ui};
use egui_wgpu::Renderer;
use pfx_editor_style::theme;
use pfx_gpu::Gpu;

pub use mean::Mean;
pub use offscreen::{MAX_SIDE, check, drive, encode_png, pixels, pixels_shuttered};
pub use shutter::Shutter;
pub use pfx_editor_style::Theme;
pub use pfx_gpu::stats::{FrameStatsSnapshot, Percentiles};
pub use pfx_gpu::window::{FrameRate, Present};
pub use pgpu::Pgpu;
pub use window::{Pulse, window};
pub use worker::Wake;

pub const WATCH_EVERY: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub files: Vec<PathBuf>,
    pub every: Duration,
}

impl Default for Watch {
    fn default() -> Watch {
        Watch {
            files: Vec::new(),
            every: WATCH_EVERY,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frames {
    pub snapshot: FrameStatsSnapshot,
    pub shown: Option<(f64, Percentiles)>,
    pub render_scale: Option<f32>,
    pub present_mode: String,
    pub vrr: bool,
    pub vrr_in_effect: Option<bool>,
    pub frame_rate: &'static str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timing {
    pub gpu_ms: Option<f64>,
    pub drops: u64,
    pub drawn: bool,
}

pub trait App {
    fn ui(&mut self, ui: &mut Ui);

    fn theme(&self) -> Theme;

    fn viewport(&self) -> bool {
        false
    }

    fn frame(&mut self, _gpu: &Gpu, _renderer: &mut Renderer, _pixels_per_point: f32) -> bool {
        false
    }

    fn forget(&mut self) {}

    fn settling(&self) -> bool {
        false
    }

    fn name(&self) -> &str {
        "editor"
    }

    fn label(&self) -> Option<String> {
        None
    }

    fn watched(&self) -> Watch {
        Watch::default()
    }

    fn changed(&mut self, _files: &[PathBuf]) {}

    fn waker(&mut self, _wake: Wake) {}

    fn opened(&mut self) -> bool {
        false
    }

    fn pgpu(&mut self, _state: Pgpu) {}

    fn timing(&self) -> Timing {
        Timing::default()
    }

    fn present(&self) -> Present {
        Present::default()
    }

    fn raised_priority(&self) -> bool {
        false
    }

    fn commands(&mut self) -> Vec<pfx_gpu::wgpu::CommandBuffer> {
        Vec::new()
    }

    fn gpu_timings(&self) -> Option<&pfx_gpu::FrameTimings> {
        None
    }

    fn render_scale(&self) -> Option<f32> {
        None
    }

    fn stats(&self) -> bool {
        false
    }

    fn frames(&mut self, _frames: &Frames) {}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Key(String, Mods),
    Text(String),
    Click([f32; 2]),
    Move([f32; 2]),
    Press {
        at: [f32; 2],
        mods: Mods,
    },
    Release([f32; 2]),
    Drag {
        from: [f32; 2],
        to: [f32; 2],
        middle: bool,
        mods: Mods,
    },
    Scroll {
        at: [f32; 2],
        lines: f32,
    },
    Frame,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub steps: Vec<Step>,
}

impl Script {
    pub fn parse(text: &str) -> Result<Script, Error> {
        script::parse(text)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub fn context(theme: &Theme) -> Context {
    let ctx = Context::default();
    theme::apply_theme(&ctx, theme);
    ctx
}

pub fn shot(app: &mut impl App, size: [u32; 2], script: &Script) -> Result<Vec<u8>, Error> {
    let rgba = pixels(app, size, script)?;
    encode_png(size, &rgba)
}

pub fn shot_shuttered(
    app: &mut impl App,
    size: [u32; 2],
    script: &Script,
    shutter: &Shutter,
) -> Result<Vec<u8>, Error> {
    let rgba = pixels_shuttered(app, size, script, shutter)?;
    encode_png(size, &rgba)
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::yield_now();
    }
}

pub fn gpu_turn(since: std::time::Instant) {
    let ms = (since.elapsed().as_secs_f64() * 1000.0).ceil().to_string();
    let _ = std::process::Command::new("pgpu")
        .args(["turn", "--work-ms", &ms, "--longest-ms", &ms])
        .status();
}
