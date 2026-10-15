use std::sync::Arc;
use std::sync::mpsc::Receiver;

use pfx_gpu::screens::{Display, SystemSource, apply_display, detect};
use pfx_gpu::window::{self, Attention, Pace, Size, SystemClock};
use pfx_gpu::{Gpu, WindowTarget, wgpu};
use pfx_input::{Backend, InputEvent, LayoutSource, SteamPrecedence};
use pfx_live::flat::Srgba;
use pfx_load::scene::Scene;
use pfx_play::{Cursor, Game, Scheme};
use pfx_sound::{ChannelId, Level, Output, PcmSender};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::config::{Config, Pads, report};
use crate::cursor::{Pointer, nudge};
use crate::driver::{Driver, Event, present_preference};
use crate::painter::{Painter, Warmup};
use crate::scheme::{self, FROM_WINDOW, of_theme, system_scheme};
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
    pointer: Pointer,
    locked: bool,
    at: [f32; 2],
    ground: Srgba,
    grounded: bool,
    schemes: Option<Receiver<Option<Scheme>>>,
    resize: Option<Size>,
    stale: bool,
}

struct App {
    pending: Option<(Box<dyn Game>, Config, Option<Scene>)>,
    schemes: Option<Receiver<Option<Scheme>>>,
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
    pfx_gpu::refuse_window("the game window")?;
    let scene = start::scene(&config)?;
    let event_loop = EventLoop::new().map_err(|error| format!("the event loop: {error}"))?;
    let proxy = event_loop.create_proxy();
    let schemes = scheme::watch(move || {
        let _ = proxy.send_event(());
    });
    let mut app = App {
        pending: Some((game, config, scene)),
        schemes,
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

fn decode(c: f32) -> f64 {
    let c = f64::from(c);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn present_ground(gpu: &Gpu, target: &WindowTarget, window: &Window, ground: Srgba) -> bool {
    let texture = match target.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
        _ => return false,
    };
    let view = texture.texture.create_view(&Default::default());
    let [r, g, b, _] = ground.0;
    let channel = |c: f32| {
        if target.config.format.is_srgb() {
            decode(c)
        } else {
            f64::from(c)
        }
    };
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pfx game ground"),
        });
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        multiview_mask: None,
        label: Some("pfx game ground"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: channel(r),
                    g: channel(g),
                    b: channel(b),
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    gpu.queue.submit([encoder.finish()]);
    window.pre_present_notify();
    gpu.queue.present(texture);
    true
}

fn open(
    event_loop: &ActiveEventLoop,
    game: Box<dyn Game>,
    mut config: Config,
    scene: Option<&Scene>,
    schemes: Option<Receiver<Option<Scheme>>>,
) -> Result<Live, String> {
    let settings = start::settings(game.as_ref(), &config);
    let device = config
        .device
        .unwrap_or_else(|| detect(&SystemSource).device);
    let display = Display::new(device, settings.display.clone());
    let windowed = display.settings().size;
    let attributes = Window::default_attributes()
        .with_title(config.window_title())
        .with_inner_size(PhysicalSize::new(windowed.width, windowed.height))
        .with_visible(false);
    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .map_err(|error| format!("the window: {error}"))?,
    );
    apply_display(&window, &display).map_err(|error| format!("the display mode: {error}"))?;
    let preference = present_preference(settings.pacing.vrr, window::host(&window));
    let (gpu, target) = pollster::block_on(Gpu::window_present(window.clone(), preference))?;
    let ground = config.clear.unwrap_or(Srgba([0.0, 0.0, 0.0, 1.0]));
    present_ground(&gpu, &target, &window, ground);
    window.set_visible(true);
    let grounded = present_ground(&gpu, &target, &window, ground);
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
    if FROM_WINDOW {
        let scheme = window.theme().map(of_theme).or_else(system_scheme);
        driver.feed(Event::Scheme(scheme));
    }
    let pointer = Pointer::new(window.has_focus());
    let mut live = Live {
        window,
        target,
        painter,
        driver,
        sound,
        minimized: false,
        warming: Some(Warmup::new(config.warm_budget)),
        pointer,
        locked: false,
        at: [0.0; 2],
        ground,
        grounded,
        schemes,
        resize: None,
        stale: false,
    };
    live.schemes();
    Ok(live)
}

impl Live {
    fn schemes(&mut self) {
        let Some(schemes) = &self.schemes else {
            return;
        };
        while let Ok(scheme) = schemes.try_recv() {
            self.driver.feed(Event::Scheme(scheme));
        }
    }

    fn warm(&mut self, now_ns: u64) -> Result<(), String> {
        let Some(warmup) = &mut self.warming else {
            return Ok(());
        };
        if !self.grounded {
            let gpu = self.painter.renderer().gpu();
            self.grounded = present_ground(gpu, &self.target, &self.window, self.ground);
        }
        let format = self.target.config.format;
        if warmup.pass(&mut self.painter, &mut self.driver, format, now_ns)? {
            self.warming = None;
        }
        Ok(())
    }

    fn frame(&mut self, now_ns: u64) -> Result<(), String> {
        if let Some(size) = self.resize.take() {
            let config = &mut self.target.config;
            if (config.width, config.height) != (size.width, size.height) {
                config.width = size.width;
                config.height = size.height;
                self.stale = true;
            }
        }
        if std::mem::take(&mut self.stale) {
            let gpu = self.painter.renderer().gpu();
            self.target
                .surface
                .configure(&gpu.device, &self.target.config);
        }
        if self.warming.is_some() {
            return self.warm(now_ns);
        }
        self.driver.update(now_ns);
        self.cursor();
        if let Some(window) = self.driver.take_warp() {
            self.warp(window);
        }
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
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.stale = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.target
                    .surface
                    .configure(&gpu.device, &self.target.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("the window's surface: validation failed".to_string());
            }
        };
        let view = texture.texture.create_view(&Default::default());
        self.painter
            .draw(&mut self.driver, &view, self.target.config.format)?;
        self.window.pre_present_notify();
        self.painter.renderer().gpu().queue.present(texture);
        let sound = &self.sound;
        self.driver.drain_audio(|audio| {
            if let Some(sound) = sound {
                sound.sender.push(audio);
            }
        });
        Ok(())
    }

    fn resized(&mut self, size: Size) {
        if size.width > 0 && size.height > 0 {
            self.resize = Some(size);
            self.driver.feed(Event::Resized(size));
            self.grounded = false;
        }
    }

    fn moved(&mut self) {
        let monitor = self.window.current_monitor();
        let name = monitor.as_ref().and_then(|monitor| monitor.name());
        if let Some(handle) = &monitor
            && let Ok(outer) = self.window.outer_position()
        {
            let origin = handle.position();
            self.driver.feed(Event::Moved {
                monitor: name.clone(),
                position: window::Point {
                    x: outer.x - origin.x,
                    y: outer.y - origin.y,
                },
            });
        }
        self.driver.feed(Event::Monitor(name));
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
            self.pointer.minimize(minimized);
            self.cursor();
        }
    }

    fn cursor(&mut self) {
        let Some(cursor) = self.pointer.change(self.driver.cursor()) else {
            return;
        };
        self.locked = false;
        let grab = match cursor {
            Cursor::Shown | Cursor::Hidden => self.window.set_cursor_grab(CursorGrabMode::None),
            Cursor::Captured => self
                .window
                .set_cursor_grab(CursorGrabMode::Confined)
                .or_else(|_| {
                    self.window
                        .set_cursor_grab(CursorGrabMode::Locked)
                        .map(|()| self.locked = true)
                }),
        };
        self.window.set_cursor_visible(cursor == Cursor::Shown);
        if let Err(error) = grab {
            eprintln!("the pointer stays free: {error}");
        }
    }

    fn warp(&mut self, window: [f32; 2]) {
        self.at = window;
        if self.locked {
            return;
        }
        let position = PhysicalPosition::new(f64::from(window[0]), f64::from(window[1]));
        if let Err(error) = self.window.set_cursor_position(position) {
            eprintln!("the pointer stays where it is: {error}");
        }
    }

    fn moved_locked(&mut self, delta: (f64, f64)) {
        let scale = if cfg!(target_os = "macos") {
            self.window.scale_factor()
        } else {
            1.0
        };
        self.at = nudge(
            self.at,
            [delta.0 * scale, delta.1 * scale],
            size_of(&self.window),
        );
        self.driver
            .feed(Event::Input(InputEvent::PointerMoved { window: self.at }));
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some((game, config, scene)) = self.pending.take() else {
            return;
        };
        match open(
            event_loop,
            game,
            config,
            scene.as_ref(),
            self.schemes.take(),
        ) {
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
        let minimizing = matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::Moved(_)
                | WindowEvent::Occluded(_)
                | WindowEvent::Focused(_)
                | WindowEvent::RedrawRequested
        );
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => live.resized(Size {
                width: size.width,
                height: size.height,
            }),
            WindowEvent::Moved(_) => live.moved(),
            WindowEvent::ThemeChanged(theme) if FROM_WINDOW => {
                live.driver.feed(Event::Scheme(Some(of_theme(theme))));
            }
            WindowEvent::Focused(focused) => {
                live.pointer.focus(focused);
                live.cursor();
            }
            WindowEvent::CursorMoved { position, .. } => {
                live.at = [position.x as f32, position.y as f32];
            }
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
        if minimizing {
            live.check_minimized();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(live) = &mut self.live {
            live.schemes();
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(live) = &mut self.live else {
            return;
        };
        if let DeviceEvent::MouseMotion { delta } = event
            && live.locked
        {
            live.moved_locked(delta);
        }
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
