use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use pfx_gpu::presents::{GUI_BUDGET_NS, Pacing};
use pfx_gpu::screens::{Display, SystemSource, apply_display, detect};
use pfx_gpu::window::{self, Attention, Pace, Size, SystemClock};
use pfx_gpu::{Gpu, WindowTarget, wgpu};
use pfx_input::{Backend, InputEvent, LayoutSource, SteamPrecedence};
use pfx_live::flat::Srgba;
use pfx_live::stats::{Source, Span};
use pfx_load::scene::Scene;
use pfx_play::{Configuring, Cursor, Game, Scheme};
use pfx_sound::{ChannelId, Level, Output, PcmSender};
#[cfg(feature = "splash")]
use pfx_splash::Player;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::clipboard;
use crate::config::{Config, Pads, report};
use crate::cursor::{Pointer, nudge};
use crate::driver::{Driver, Event};
use crate::painter::{Painter, Warmup};
use crate::scheme::{self, FROM_WINDOW, of_theme, system_scheme};
use crate::start;
use crate::surface::{Attempt, Retry};

pub const AUDIO_SECONDS: f32 = 0.25;

struct Sound {
    output: Output,
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
    #[cfg(feature = "splash")]
    warmed: Option<Size>,
    #[cfg(feature = "splash")]
    warm_budget: f64,
    pointer: Pointer,
    locked: bool,
    at: [f32; 2],
    ground: Srgba,
    grounded: bool,
    schemes: Option<Receiver<Option<Scheme>>>,
    resize: Option<Size>,
    stale: bool,
    retry: Retry,
    configuring: Configuring,
    refollow: bool,
    #[cfg(feature = "splash")]
    splash: Option<Player>,
    pacing: Pacing,
    dirty: bool,
    asked: bool,
    rested: bool,
    rest_ns: Option<u64>,
    _clipboard: Option<clipboard::Service>,
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
    Some(Sound { output, sender })
}

pub(crate) fn frame_source(splash: bool) -> Source {
    if splash { Source::Splash } else { Source::Ui }
}

#[cfg(feature = "splash")]
pub(crate) fn rewarm(warming: bool, warmed: Option<Size>, window: Size) -> bool {
    !warming && warmed.is_some_and(|size| size != window)
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
    if !target.configured() {
        return false;
    }
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
    #[cfg(feature = "splash")]
    let mut splash = {
        let size = size_of(&window);
        Player::start(
            &config.splash,
            config.build_date.as_deref(),
            [size.width, size.height],
            config.sound,
        )
    };
    let preference = window::present_preference(&window, settings.pacing.present());
    let (gpu, mut target) = pollster::block_on(Gpu::window_present(window.clone(), preference))?;
    target.follow(&gpu, &window, settings.pacing.present());
    if pfx_gpu::priority::asked() {
        match pfx_gpu::priority::raise_current_thread() {
            Ok(raised) => eprintln!("the window's thread: {raised}"),
            Err(error) => eprintln!("the window's thread keeps its priority: {error}"),
        }
    }
    let ground = config.clear.unwrap_or(Srgba([0.0, 0.0, 0.0, 1.0]));
    #[cfg(feature = "splash")]
    let ground = if splash.is_some() {
        config.splash.background.srgba()
    } else {
        ground
    };
    present_ground(&gpu, &target, &window, ground);
    window.set_visible(true);
    let grounded = present_ground(&gpu, &target, &window, ground);
    let size = size_of(&window);
    let painter = Painter::live(gpu, scene, &config, size)?;
    #[cfg(feature = "splash")]
    {
        let surface = &target.config;
        let gpu = painter.renderer().gpu();
        let warmed = splash
            .as_mut()
            .map(|player| player.warm(gpu, surface.format, [surface.width, surface.height]));
        if let Some(Err(error)) = warmed {
            eprintln!("the splash is left out: {error}");
            splash = None;
        }
    }
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
    let browser = config
        .browser
        .take()
        .unwrap_or_else(|| Box::new(crate::SystemBrowser::default()));
    driver.session_mut().world_mut().attach_browser(browser);
    let configuring = driver.session().world().configuring();
    let clipboard = clipboard::system(&window);
    if let Some(service) = &clipboard {
        driver
            .session_mut()
            .world_mut()
            .attach_clipboard(Box::new(service.link()));
    }
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
        #[cfg(feature = "splash")]
        warmed: None,
        #[cfg(feature = "splash")]
        warm_budget: config.warm_budget,
        pointer,
        locked: false,
        at: [0.0; 2],
        ground,
        grounded,
        schemes,
        resize: None,
        stale: false,
        retry: Retry::default(),
        configuring,
        refollow: false,
        #[cfg(feature = "splash")]
        splash,
        pacing: Pacing::from_env(GUI_BUDGET_NS),
        dirty: false,
        asked: false,
        rested: false,
        rest_ns: None,
        _clipboard: clipboard,
    };
    live.pacing
        .set(live.target.setting(&live.window, live.driver.present()));
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
        #[cfg(feature = "splash")]
        {
            self.warmed = Some(self.driver.window());
        }
        if warmup.pass(&mut self.painter, &mut self.driver, format, now_ns)? {
            self.warming = None;
        }
        Ok(())
    }

    fn frame(&mut self, now_ns: u64, exposed: bool) -> Result<bool, String> {
        #[cfg(feature = "splash")]
        let splash = self.splash.is_some();
        #[cfg(not(feature = "splash"))]
        let splash = false;
        let stats = self.painter.renderer().frame_stats();
        stats.present_begin(frame_source(splash));
        let presented = self.draw(now_ns, exposed);
        if matches!(presented, Ok(true)) {
            let config = &self.target.config;
            stats.presented([config.width, config.height]);
        } else {
            stats.present_abandon();
        }
        presented
    }

    fn draw(&mut self, now_ns: u64, exposed: bool) -> Result<bool, String> {
        self.rested = false;
        if let Some(size) = self.resize.take() {
            let max = self
                .painter
                .renderer()
                .gpu()
                .device
                .limits()
                .max_texture_dimension_2d;
            let (width, height) = pfx_gpu::surface_size(size.width, size.height, max);
            let config = &mut self.target.config;
            if (config.width, config.height) != (width, height) {
                config.width = width;
                config.height = height;
                self.stale = true;
            }
        }
        let ready = if std::mem::take(&mut self.stale) || !self.target.configured() {
            self.dirty = true;
            self.configure(now_ns)?
        } else {
            true
        };
        #[cfg(feature = "splash")]
        if self.splash.is_some() {
            return if ready {
                self.splash_frame(now_ns)
            } else {
                Ok(false)
            };
        }
        if self.warming.is_some() {
            return if ready {
                self.warm(now_ns).map(|()| false)
            } else {
                Ok(false)
            };
        }
        self.driver.update(now_ns);
        self.cursor();
        if let Some(window) = self.driver.take_warp() {
            self.warp(window);
        }
        for notice in self.driver.take_notices() {
            eprintln!("{notice}");
        }
        let mut switched = false;
        if let Some(display) = self.driver.take_display() {
            apply_display(&self.window, display)
                .map_err(|error| format!("the display mode: {error}"))?;
            switched = true;
        }
        switched |= self.driver.take_present().is_some();
        let settled = self.target.settled();
        if settled {
            switched = true;
        }
        switched |= std::mem::take(&mut self.refollow);
        if switched && !self.idle() {
            self.refollow = true;
            self.configuring.set_pending(true);
        } else if switched {
            let present = self.driver.present();
            let gpu = self.painter.renderer().gpu();
            self.target.follow(gpu, &self.window, present);
            self.driver.set_vrr_in_effect(self.target.vrr_in_effect());
            self.pacing.set(self.target.setting(&self.window, present));
            self.dirty = true;
            if self.target.configured() && !self.stale {
                self.configuring.set_pending(false);
            }
        }
        if !self.target.configured() || self.stale {
            self.audio();
            return Ok(false);
        }
        if self.driver.exit().is_some() {
            return Ok(false);
        }
        let forced = std::mem::take(&mut self.dirty) || exposed;
        if !self.painter.prepare(&mut self.driver, forced)? {
            self.audio();
            self.rested = true;
            return Ok(false);
        }
        self.dirty = true;
        let asking = Instant::now();
        let acquired = self.target.surface.get_current_texture();
        self.pacing
            .acquired(u64::try_from(asking.elapsed().as_nanos()).unwrap_or(u64::MAX));
        self.painter
            .renderer()
            .frame_stats()
            .span(Span::Wait, Some(asking));
        let texture = match acquired {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.stale = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.stale = true;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("the window's surface: validation failed".to_string());
            }
        };
        let view = texture.texture.create_view(&Default::default());
        self.painter
            .compose(&mut self.driver, &view, self.target.config.format)?;
        self.window.pre_present_notify();
        self.target.present(
            &self.painter.renderer().gpu().queue,
            texture,
            self.pacing.tag(),
        );
        for shown in self.target.shown() {
            self.pacing.shown(shown);
        }
        self.dirty = false;
        self.audio();
        Ok(true)
    }

    fn idle(&self) -> bool {
        let gpu = self.painter.renderer().gpu();
        matches!(
            gpu.device.poll(wgpu::PollType::Poll),
            Ok(wgpu::PollStatus::QueueEmpty)
        )
    }

    fn configure(&mut self, now_ns: u64) -> Result<bool, String> {
        self.configuring.set_pending(true);
        let attempt = if !self.idle() {
            Attempt::Busy
        } else if self.target.reconfigure(self.painter.renderer().gpu()) {
            Attempt::Configured
        } else {
            Attempt::Refused
        };
        self.retry.record(attempt, now_ns)?;
        let configured = attempt == Attempt::Configured;
        if configured {
            self.configuring.set_pending(false);
        } else {
            self.stale = true;
        }
        Ok(configured)
    }

    fn rest_until(&self, at: u64) -> u64 {
        self.driver.next_tick_ns().unwrap_or_else(|| {
            at.saturating_add((self.driver.session().world().clock().tick_dt() * 1e9) as u64)
        })
    }

    fn audio(&mut self) {
        let sound = &self.sound;
        self.driver.drain_audio(|audio| {
            if let Some(sound) = sound {
                sound.sender.push(audio);
            }
        });
    }

    #[cfg(feature = "splash")]
    fn splash_frame(&mut self, now_ns: u64) -> Result<bool, String> {
        if rewarm(self.warming.is_some(), self.warmed, self.driver.window()) {
            self.warming = Some(Warmup::new(self.warm_budget));
        }
        if self.warming.is_some() {
            self.grounded = true;
            self.warm(now_ns)?;
        }
        let loaded = self.warming.is_none();
        let Some(splash) = &mut self.splash else {
            return Ok(false);
        };
        let phase = splash.phase(now_ns, loaded);
        if let (pfx_splash::Phase::Showing { at, .. }, Some(sound)) = (phase, &self.sound)
            && let Some(clip) = splash.chime(at)
        {
            let audio = &self.driver.settings().audio;
            if !audio.muted {
                let level = Level {
                    volume: audio.master,
                    ..Level::default()
                };
                sound.output.play_on(ChannelId::EFFECTS, &clip, level);
            }
        }
        if phase == pfx_splash::Phase::Done {
            self.splash = None;
            return Ok(false);
        }
        let gpu = self.painter.renderer().gpu();
        let config = &self.target.config;
        let size = [config.width, config.height];
        if let Err(error) = splash.settle(gpu, size) {
            eprintln!("the splash is left out: {error}");
            self.splash = None;
            return Ok(false);
        }
        let asking = Instant::now();
        let acquired = self.target.surface.get_current_texture();
        self.painter
            .renderer()
            .frame_stats()
            .span(Span::Wait, Some(asking));
        let texture = match acquired {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.stale = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.stale = true;
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("the window's surface: validation failed".to_string());
            }
        };
        let view = texture.texture.create_view(&Default::default());
        if let Err(error) = splash.draw(gpu, &view, self.target.config.format, size, phase) {
            eprintln!("the splash is left out: {error}");
            self.splash = None;
        }
        self.window.pre_present_notify();
        gpu.queue.present(texture);
        Ok(true)
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
            self.dirty = true;
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
        if !matches!(event, WindowEvent::RedrawRequested) {
            live.rest_ns = None;
        }
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
            WindowEvent::Occluded(false) => live.dirty = true,
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
                let exposed = !std::mem::take(&mut live.asked);
                let now = window::Clock::now_ns(&self.clock);
                match live.frame(now, exposed) {
                    Err(error) => {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                    Ok(presented) => {
                        let at = window::Clock::now_ns(&self.clock);
                        if presented {
                            live.pacing.presented(at);
                            live.rest_ns = None;
                        } else if live.rested {
                            live.pacing.idle();
                            live.rest_ns = Some(live.rest_until(at));
                        }
                        if live.driver.exit().is_some() {
                            event_loop.exit();
                        }
                    }
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
            live.rest_ns = None;
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
            live.rest_ns = None;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = &mut self.live else {
            return;
        };
        #[cfg(feature = "splash")]
        let starting = live.warming.is_some() || live.splash.is_some();
        #[cfg(not(feature = "splash"))]
        let starting = live.warming.is_some();
        if starting {
            live.window.request_redraw();
            live.asked = true;
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }
        let flow = match live.driver.poll(&self.clock) {
            Pace::Start { next_ns } => {
                let now = window::Clock::now_ns(&self.clock);
                match live.rest_ns.filter(|&until| now < until) {
                    Some(until) => ControlFlow::WaitUntil(self.clock.instant(until)),
                    None => {
                        live.window.request_redraw();
                        live.asked = true;
                        next_ns.map_or(ControlFlow::Wait, |ns| {
                            ControlFlow::WaitUntil(self.clock.instant(ns))
                        })
                    }
                }
            }
            Pace::Wait { until_ns } => ControlFlow::WaitUntil(self.clock.instant(until_ns)),
            Pace::Paused => {
                live.pacing.idle();
                ControlFlow::Wait
            }
        };
        event_loop.set_control_flow(flow);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(live) = self.live.take() {
            self.outcome = Some(live.driver.finish());
        }
    }
}
