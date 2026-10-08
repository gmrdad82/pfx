use pfx_core::clock::Tick;
use pfx_gpu::pace::{Pace, WallClock};
use pfx_gpu::screens::{Device, Display};
use pfx_gpu::window::{
    FakeClock, FullscreenPlan, Host, Monitor, Monitors, Pace as Due, PlatformCaps, Point,
    PresentPreference, Size, WindowOps,
};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_play::{Game, NoWarmer, Warming};

use crate::config::{Config, Crash, Pads, report};
use crate::driver::{Driver, Event, present_preference};
use crate::painter::{Drawn, Painter};
use crate::start;

#[derive(Clone, Debug, PartialEq)]
pub enum WindowCall {
    Fullscreen(FullscreenPlan),
    Decorations(bool),
    Size(Size),
    Position(Point),
    Resizable(bool),
    Present(PresentPreference),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StandIn {
    pub calls: Vec<WindowCall>,
}

impl WindowOps for StandIn {
    fn set_fullscreen(&mut self, fullscreen: &FullscreenPlan) {
        self.calls.push(WindowCall::Fullscreen(*fullscreen));
    }

    fn set_decorations(&mut self, decorations: bool) {
        self.calls.push(WindowCall::Decorations(decorations));
    }

    fn request_inner_size(&mut self, size: Size) {
        self.calls.push(WindowCall::Size(size));
    }

    fn set_outer_position(&mut self, position: Point) {
        self.calls.push(WindowCall::Position(position));
    }

    fn set_resizable(&mut self, resizable: bool) {
        self.calls.push(WindowCall::Resizable(resizable));
    }
}

pub fn stand_in_monitors(size: Size) -> Monitors {
    Monitors {
        list: vec![Monitor {
            name: Some("stand-in".into()),
            position: Point { x: 0, y: 0 },
            size,
            refresh_millihertz: Some(60_000),
            video_modes: Vec::new(),
        }],
        primary: Some(0),
        current: Some(0),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub frames: u64,
    pub ticks: u64,
}

struct Gpued {
    painter: Painter,
    target: OffscreenTarget,
    pace: Pace<WallClock>,
}

pub struct Headless {
    pub driver: Driver,
    pub clock: FakeClock,
    pub window: StandIn,
    pub monitors: Monitors,
    pub host: Host,
    pub audio: Vec<f32>,
    pub frame_cost_ns: u64,
    pub drawn: Option<Drawn>,
    pub warm_passes: u32,
    pub warm_budget: f64,
    warmed: bool,
    gpu: Option<Gpued>,
    name: String,
    crash: Option<Crash>,
}

pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn backend(config: &mut Config) -> Option<Box<dyn pfx_input::Backend>> {
    match std::mem::replace(&mut config.pads, Pads::None) {
        Pads::Backend(backend) => Some(backend),
        Pads::None | Pads::Gilrs => None,
    }
}

impl Headless {
    pub fn new<G: Game + 'static>(game: G, config: Config, window: Size) -> Result<Self, String> {
        Self::build(Box::new(game), config, None, window)
    }

    pub fn with_gpu<G: Game + 'static>(
        game: G,
        config: Config,
        gpu: Gpu,
        window: Size,
    ) -> Result<Self, String> {
        Self::build(Box::new(game), config, Some(gpu), window)
    }

    fn build(
        game: Box<dyn Game>,
        mut config: Config,
        gpu: Option<Gpu>,
        window: Size,
    ) -> Result<Self, String> {
        let scene = start::scene(&config)?;
        let device = config.device.unwrap_or(Device::Desktop);
        let gpu = gpu
            .map(|gpu| -> Result<Gpued, String> {
                let target = gpu.offscreen(window.width, window.height, TARGET_FORMAT)?;
                let painter = Painter::new(gpu, scene.as_ref(), &config, window)?;
                Ok(Gpued {
                    painter,
                    target,
                    pace: Pace::new(),
                })
            })
            .transpose()?;
        let pads = backend(&mut config);
        let mut driver = start::driver(
            scene.as_ref(),
            gpu.as_ref().map(|gpu| &gpu.painter),
            game,
            &config,
            device,
            window,
            (48_000, 2),
            pads,
        )?;
        driver.set_layout(config.layout.take());
        let mut headless = Self {
            driver,
            clock: FakeClock::default(),
            window: StandIn::default(),
            monitors: stand_in_monitors(window),
            host: Host::Wayland,
            audio: Vec::new(),
            frame_cost_ns: 1_000_000,
            drawn: None,
            warm_passes: 0,
            warm_budget: config.warm_budget,
            warmed: false,
            gpu,
            crash: config.crash.take(),
            name: config.name,
        };
        let display = headless.driver.display().clone();
        apply(&display, &mut headless.window, &headless.monitors);
        Ok(headless)
    }

    pub fn exited(&self) -> bool {
        self.driver.exit().is_some()
    }

    pub fn finish(self) -> Result<u8, String> {
        let outcome = self.driver.finish();
        if let Err(error) = &outcome {
            report(&self.name, self.crash.as_ref(), error);
        }
        outcome
    }

    pub fn feed(&mut self, event: Event) {
        self.driver.feed(event);
    }

    pub fn frame(&mut self) -> Result<Option<Tick>, String> {
        self.frame_before(u64::MAX)
    }

    pub fn warm(&mut self) -> Result<u32, String> {
        while !self.warmed {
            let elapsed = self.warm_passes as f64 * self.frame_cost_ns as f64 / 1e9;
            if self.warm_passes > 0 && elapsed >= self.warm_budget {
                break;
            }
            let pass = self.warm_passes;
            let budget = self.warm_budget;
            let warming = match &mut self.gpu {
                Some(gpu) => {
                    let driver = &mut self.driver;
                    let painter = &mut gpu.painter;
                    let mut result = Ok(Warming::Done);
                    gpu.pace.frame(|| {
                        result = painter.warm(driver, TARGET_FORMAT, pass, elapsed, budget);
                    });
                    result?
                }
                None => self.driver.warm(&mut NoWarmer, pass, elapsed, budget)?,
            };
            self.warm_passes += 1;
            if warming == Warming::Done {
                break;
            }
        }
        self.warmed = true;
        Ok(self.warm_passes)
    }

    fn frame_before(&mut self, end_ns: u64) -> Result<Option<Tick>, String> {
        if self.exited() {
            return Ok(None);
        }
        self.warm()?;
        loop {
            match self.driver.poll(&self.clock) {
                Due::Start { .. } => break,
                Due::Wait { until_ns } if until_ns < end_ns => self.clock.now_ns = until_ns,
                Due::Wait { .. } | Due::Paused => return Ok(None),
            }
        }
        let tick = self.driver.update(self.clock.now_ns);
        if let Some(display) = self.driver.take_display().cloned() {
            apply(&display, &mut self.window, &self.monitors);
        }
        if let Some(vrr) = self.driver.take_present() {
            self.window
                .calls
                .push(WindowCall::Present(present_preference(vrr, self.host)));
        }
        if self.exited() {
            self.audio.extend(self.driver.take_audio());
            return Ok(Some(tick));
        }
        if let Some(gpu) = &mut self.gpu {
            let driver = &mut self.driver;
            let painter = &mut gpu.painter;
            let view = &gpu.target.view;
            let mut result = Ok(None);
            gpu.pace.frame(|| {
                result = painter.draw(driver, view, TARGET_FORMAT);
            });
            self.drawn = result?;
        }
        self.audio.extend(self.driver.take_audio());
        self.clock.now_ns = self.clock.now_ns.saturating_add(self.frame_cost_ns);
        Ok(Some(tick))
    }

    pub fn run_for(&mut self, seconds: f64) -> Result<Count, String> {
        let end = self
            .clock
            .now_ns
            .saturating_add((seconds * 1e9).round() as u64);
        let frames = self.driver.frames();
        let ticks = self.driver.ticks();
        while self.clock.now_ns < end {
            if self.frame_before(end)?.is_none() {
                break;
            }
        }
        self.clock.now_ns = self.clock.now_ns.max(end);
        Ok(Count {
            frames: self.driver.frames() - frames,
            ticks: self.driver.ticks() - ticks,
        })
    }

    pub fn target(&self) -> Option<&OffscreenTarget> {
        self.gpu.as_ref().map(|gpu| &gpu.target)
    }

    pub fn painter(&self) -> Option<&Painter> {
        self.gpu.as_ref().map(|gpu| &gpu.painter)
    }

    pub fn painter_mut(&mut self) -> Option<&mut Painter> {
        self.gpu.as_mut().map(|gpu| &mut gpu.painter)
    }

    pub fn readback(&self) -> Result<Vec<u8>, String> {
        let gpu = self.gpu.as_ref().ok_or("this run has no GPU")?;
        gpu.painter.renderer().gpu().readback_rgba8(&gpu.target)
    }
}

fn apply(display: &Display, window: &mut StandIn, monitors: &Monitors) {
    let _ = display.apply(PlatformCaps { exclusive: true }, monitors, window);
}
