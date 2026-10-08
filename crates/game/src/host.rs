use std::sync::Arc;

use pfx_gpu::screens::{Display, SystemSource, apply_display, detect};
use pfx_gpu::window::{self, Attention, Pace, Size, SystemClock};
use pfx_gpu::{Gpu, WindowTarget, wgpu};
use pfx_input::{Backend, LayoutSource, SteamPrecedence};
use pfx_load::scene::Scene;
use pfx_play::Game;
use pfx_sound::{ChannelId, Level, Output, PcmSender};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::config::{Config, Pads, report};
use crate::driver::{Driver, Event, present_preference};
use crate::painter::{Painter, Warmup};
use crate::start;

pub const AUDIO_SECONDS: f32 = 0.25;

struct Sound {
    _output: Output,
    sender: PcmSender,
}

struct Live {
    window: Arc<Window>,
    target: WindowTarget,
    painter: Painter,
    driver: Driver,
    sound: Option<Sound>,
    minimized: bool,
    warming: Option<Warmup>,
}

struct App {
    pending: Option<(Box<dyn Game>, Config, Option<Scene>)>,
    live: Option<Live>,
    clock: SystemClock,
    error: Option<String>,
    outcome: Option<Result<u8, String>>,
}

pub fn run<G: Game + 'static>(game: G, mut config: Config) -> Result<u8, String> {
    let name = config.name.clone();
    let crash = config.crash.take();
    let outcome = play(Box::new(game), config);
    if let Err(error) = &outcome {
        report(&name, crash.as_ref(), error);
    }
    outcome
}

fn play(game: Box<dyn Game>, config: Config) -> Result<u8, String> {
    let scene = start::scene(&config)?;
    let event_loop = EventLoop::new().map_err(|error| format!("the event loop: {error}"))?;
    let mut app = App {
        pending: Some((game, config, scene)),
        live: None,
        clock: SystemClock::new(),
        error: None,
        outcome: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|error| format!("the event loop: {error}"))?;
    if let Some(live) = app.live.take() {
        app.outcome = Some(live.driver.finish());
    }
    match (app.error, app.outcome) {
        (Some(error), _) => Err(error),
        (None, Some(outcome)) => outcome,
        (None, None) => Ok(0),
    }
}

fn pads(config: &mut Config) -> Option<Box<dyn Backend>> {
    match std::mem::replace(&mut config.pads, Pads::None) {
        Pads::None => None,
        Pads::Backend(backend) => Some(backend),
        Pads::Gilrs => pfx_input::pads::Gilrs::new().ok().map(|gilrs| {
            Box::new(gilrs.with_steam(SteamPrecedence::from_env())) as Box<dyn Backend>
        }),
    }
}

#[cfg(windows)]
fn layout(config: &mut Config) -> Option<Box<dyn LayoutSource>> {
    Some(
        config
            .layout
            .take()
            .unwrap_or_else(|| Box::new(pfx_input::WindowsLayout::new())),
    )
}

#[cfg(not(windows))]
fn layout(config: &mut Config) -> Option<Box<dyn LayoutSource>> {
    config.layout.take()
}

fn sound(config: &Config) -> Option<Sound> {
    if !config.sound {
        return None;
    }
    let output = Output::open().ok()?;
    let (_, sender) = output.pcm_stream(
        ChannelId::EFFECTS,
        output.channels(),
        AUDIO_SECONDS,
        Level::default(),
    );
    Some(Sound {
        _output: output,
        sender,
    })
}

fn size_of(window: &Window) -> Size {
    let size = window.inner_size();
    Size {
        width: size.width,
        height: size.height,
    }
}

fn open(
    event_loop: &ActiveEventLoop,
    game: Box<dyn Game>,
    mut config: Config,
    scene: Option<&Scene>,
) -> Result<Live, String> {
    let settings = start::settings(game.as_ref(), &config);
    let device = config
        .device
        .unwrap_or_else(|| detect(&SystemSource).device);
    let display = Display::new(device, settings.display.clone());
    let windowed = display.settings().size;
    let attributes = Window::default_attributes()
        .with_title(config.window_title())
        .with_inner_size(PhysicalSize::new(windowed.width, windowed.height));
    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .map_err(|error| format!("the window: {error}"))?,
    );
    apply_display(&window, &display).map_err(|error| format!("the display mode: {error}"))?;
    let preference = present_preference(settings.pacing.vrr, window::host(&window));
    let (gpu, target) = pollster::block_on(Gpu::window_present(window.clone(), preference))?;
    let size = size_of(&window);
    let painter = Painter::new(gpu, scene, &config, size)?;
    let sound = sound(&config);
    let audio = sound.as_ref().map_or((48_000, 2), |sound| {
        (sound.sender.rate(), sound.sender.channels())
    });
    let pads = pads(&mut config);
    let mut driver = start::driver(
        scene,
        Some(&painter),
        game,
        &config,
        device,
        size,
        audio,
        pads,
    )?;
    driver.set_layout(layout(&mut config));
    driver.feed(Event::Refresh(
        window
            .current_monitor()
            .and_then(|monitor| window::refresh_millihertz(&monitor)),
    ));
    Ok(Live {
        window,
        target,
        painter,
        driver,
        sound,
        minimized: false,
        warming: Some(Warmup::new(config.warm_budget)),
    })
}

impl Live {
    fn warm(&mut self, now_ns: u64) -> Result<(), String> {
        let Some(warmup) = &mut self.warming else {
            return Ok(());
        };
        let format = self.target.config.format;
        if warmup.pass(&mut self.painter, &mut self.driver, format, now_ns)? {
            self.warming = None;
        }
        Ok(())
    }

    fn frame(&mut self, now_ns: u64) -> Result<(), String> {
        if self.warming.is_some() {
            return self.warm(now_ns);
        }
        self.driver.update(now_ns);
        for notice in self.driver.take_notices() {
            eprintln!("{notice}");
        }
        if let Some(display) = self.driver.take_display() {
            apply_display(&self.window, display)
                .map_err(|error| format!("the display mode: {error}"))?;
        }
        let gpu = self.painter.renderer().gpu();
        if let Some(vrr) = self.driver.take_present() {
            let preference = present_preference(vrr, window::host(&self.window));
            self.target.set_present(gpu, &preference);
        }
        if self.driver.exit().is_some() {
            return Ok(());
        }
        let texture = match self.target.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.target
                    .surface
                    .configure(&gpu.device, &self.target.config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(format!("the window's surface: {error}")),
        };
        let view = texture.texture.create_view(&Default::default());
        self.painter
            .draw(&mut self.driver, &view, self.target.config.format)?;
        self.window.pre_present_notify();
        texture.present();
        let audio = self.driver.take_audio();
        if let Some(sound) = &self.sound {
            sound.sender.push(&audio);
        }
        Ok(())
    }

    fn resized(&mut self, size: Size) {
        if size.width > 0 && size.height > 0 {
            self.target
                .resize(self.painter.renderer().gpu(), size.width, size.height);
            self.driver.feed(Event::Resized(size));
        }
    }

    fn moved(&mut self) {
        if let Some((monitor, position)) = window::placement(&self.window) {
            self.driver.feed(Event::Moved { monitor, position });
        }
        let monitor = self.window.current_monitor();
        self.driver.feed(Event::Monitor(
            monitor.as_ref().and_then(|monitor| monitor.name()),
        ));
        self.driver.feed(Event::Refresh(
            monitor.as_ref().and_then(window::refresh_millihertz),
        ));
    }

    fn check_minimized(&mut self) {
        let minimized = window::minimized(&self.window);
        if minimized != self.minimized {
            self.minimized = minimized;
            self.driver
                .feed(Event::Attention(Attention::Minimized(minimized)));
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some((game, config, scene)) = self.pending.take() else {
            return;
        };
        match open(event_loop, game, config, scene.as_ref()) {
            Ok(live) => self.live = Some(live),
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(live) = &mut self.live else {
            return;
        };
        if let Some(attention) = window::attention(&event) {
            live.driver.feed(Event::Attention(attention));
        }
        pfx_input::winit::learn(live.driver.labels_mut(), &event);
        if let Some(input) = pfx_input::winit::event(&event) {
            live.driver.feed(Event::Input(input));
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => live.resized(Size {
                width: size.width,
                height: size.height,
            }),
            WindowEvent::Moved(_) => live.moved(),
            WindowEvent::RedrawRequested => {
                let now = window::Clock::now_ns(&self.clock);
                if let Err(error) = live.frame(now) {
                    self.error = Some(error);
                    event_loop.exit();
                } else if live.driver.exit().is_some() {
                    event_loop.exit();
                }
            }
            _ => {}
        }
        live.check_minimized();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = &mut self.live else {
            return;
        };
        if live.warming.is_some() {
            live.window.request_redraw();
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }
        let flow = match live.driver.poll(&self.clock) {
            Pace::Start { next_ns } => {
                live.window.request_redraw();
                next_ns.map_or(ControlFlow::Poll, |ns| {
                    ControlFlow::WaitUntil(self.clock.instant(ns))
                })
            }
            Pace::Wait { until_ns } => ControlFlow::WaitUntil(self.clock.instant(until_ns)),
            Pace::Paused => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(live) = self.live.take() {
            self.outcome = Some(live.driver.finish());
        }
    }
}
