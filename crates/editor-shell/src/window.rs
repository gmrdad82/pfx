use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use egui::{Context, ViewportId};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use egui_winit::winit::application::ApplicationHandler;
use egui_winit::winit::dpi::LogicalSize;
use egui_winit::winit::event::WindowEvent;
use egui_winit::winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use egui_winit::winit::window::{Window, WindowId};
use pfx_gpu::presents::{GUI_BUDGET_NS, Pacing, Setting};
use pfx_gpu::stats::{FrameStats, percentiles};
use pfx_gpu::window::{self as pace, Clock, FramePacer, Pace, SystemClock};
use pfx_gpu::{Gpu, WindowTarget, wgpu};

use crate::paint::Painter;
use crate::pgpu::{self, Pgpu};
use crate::profile::{Phase, Profile};
use crate::{App, Error, Frames, Present, Theme, Watch, block_on, context};
use pfx_gpu::presents::Shown;
use pfx_gpu::{FrameTimings, PassTiming};

const SLOW: Duration = Duration::from_millis(250);
const TALLY: u32 = 240;
const SIZE: [f64; 2] = [1586.0, 992.0];
const SAMPLE_NS: u64 = 250_000_000;
const SHOWN_KEPT: usize = 240;

pub trait Pulse {
    fn frame(&self, work: Duration);
    fn idle(&self);
    fn label(&self, _label: &str) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Signal {
    Files(u64, Vec<PathBuf>),
    App,
    Pgpu(Pgpu),
}

struct Live {
    window: Arc<Window>,
    gpu: Gpu,
    target: WindowTarget,
    state: egui_winit::State,
    painter: Painter,
    wanted: bool,
    wake_at: Option<Instant>,
}

#[derive(Default)]
struct Tally {
    frames: u32,
    work: Duration,
    longest: Duration,
    gpu_ms: f64,
    gpu_frames: u32,
    noted: u64,
}

impl Tally {
    fn add(&mut self, name: &str, work: Duration, gpu_ms: Option<f64>, drops: u64) -> Vec<String> {
        let mut lines = Vec::new();
        if drops > self.noted {
            lines.push(format!(
                "{name}: {drops} frame timings dropped so far; the GPU ran more than two frames behind"
            ));
            self.noted = drops.saturating_mul(2);
        }
        self.frames += 1;
        self.work += work;
        self.longest = self.longest.max(work);
        if let Some(ms) = gpu_ms {
            self.gpu_ms += ms;
            self.gpu_frames += 1;
        }
        if self.frames == TALLY {
            let gpu = if self.gpu_frames > 0 {
                format!("{:.2} ms", self.gpu_ms / f64::from(self.gpu_frames))
            } else {
                "unmeasured".to_string()
            };
            lines.push(format!(
                "{name}: {} frames, the UI thread {:.2} ms a frame (longest {:.2} ms), the viewport on the GPU {gpu}",
                self.frames,
                self.work.as_secs_f64() * 1000.0 / f64::from(self.frames),
                self.longest.as_secs_f64() * 1000.0,
            ));
            *self = Tally {
                noted: self.noted,
                ..Tally::default()
            };
        }
        lines
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

fn moved(files: &[PathBuf], last: &[Option<Stamp>], now: &[Option<Stamp>]) -> Vec<PathBuf> {
    files
        .iter()
        .zip(last.iter().zip(now))
        .filter(|(_, (then, now))| then != now)
        .map(|(file, _)| file.clone())
        .collect()
}

fn watch_files(
    watch: Watch,
    generation: u64,
    stop: Arc<AtomicBool>,
    proxy: EventLoopProxy<Signal>,
) {
    std::thread::spawn(move || {
        let stamps = || {
            watch
                .files
                .iter()
                .map(|file| stamp(file))
                .collect::<Vec<_>>()
        };
        let mut last = stamps();
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(watch.every);
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let now = stamps();
            if now != last {
                let changed = moved(&watch.files, &last, &now);
                last = now;
                if proxy
                    .send_event(Signal::Files(generation, changed))
                    .is_err()
                {
                    return;
                }
            }
        }
    });
}

fn title(name: &str, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("{name} · {label}"),
        None => name.to_string(),
    }
}

fn retitle(window: &Window, pulse: Option<&dyn Pulse>, app: &impl App) {
    let label = app.label();
    window.set_title(&title(app.name(), label.as_deref()));
    if let (Some(pulse), Some(label)) = (pulse, label) {
        pulse.label(&label);
    }
}

struct Watcher {
    stop: Option<Arc<AtomicBool>>,
    watch: Watch,
    generation: u64,
}

impl Watcher {
    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn follow(&mut self, watch: Watch, proxy: &EventLoopProxy<Signal>) {
        if self.stop.is_some() && watch == self.watch {
            return;
        }
        self.stop();
        self.generation += 1;
        self.watch = watch.clone();
        if watch.files.is_empty() {
            return;
        }
        let flag = Arc::new(AtomicBool::new(false));
        self.stop = Some(Arc::clone(&flag));
        watch_files(watch, self.generation, flag, proxy.clone());
    }
}

#[derive(Default)]
struct Meter {
    on: bool,
    stats: FrameStats,
    last_ns: Option<u64>,
    timed: Option<FrameTimings>,
    painted: Option<FrameTimings>,
    shown: VecDeque<f64>,
    last_shown: Option<(u64, u64)>,
    sampled_ns: Option<u64>,
    mode: String,
    vrr_in_effect: Option<bool>,
}

fn fresh(seen: &mut Option<FrameTimings>, timings: Option<&FrameTimings>) -> Vec<PassTiming> {
    match timings.filter(|timings| seen.as_ref() != Some(*timings)) {
        Some(timings) => {
            *seen = Some(timings.clone());
            timings.passes.clone()
        }
        None => Vec::new(),
    }
}

impl Meter {
    fn follow(&mut self, on: bool) {
        if on && !self.on {
            self.stats = FrameStats::default();
            self.timed = None;
            self.painted = None;
            self.shown.clear();
            self.last_shown = None;
            self.sampled_ns = None;
        }
        self.on = on;
    }

    fn set(&mut self, setting: &Setting) {
        self.mode.clone_from(&setting.present_mode);
        self.vrr_in_effect = setting.vrr_in_effect;
    }

    fn tag(&self) -> Option<u64> {
        self.on.then_some(0)
    }

    fn presented(
        &mut self,
        now_ns: u64,
        work: Duration,
        viewport: Option<&FrameTimings>,
        paint: Option<&FrameTimings>,
    ) {
        let interval = self
            .last_ns
            .replace(now_ns)
            .map_or(f64::NAN, |last| now_ns.saturating_sub(last) as f64 / 1e6);
        if !self.on {
            return;
        }
        let mut passes = fresh(&mut self.timed, viewport);
        passes.extend(fresh(&mut self.painted, paint));
        let gpu_ms =
            (!passes.is_empty()).then(|| passes.iter().map(|pass| pass.milliseconds).sum::<f64>());
        self.stats
            .record_frame(work.as_secs_f64() * 1000.0, interval, gpu_ms, &passes);
    }

    fn shown(&mut self, shown: &Shown) {
        let Some(at) = shown.at_ns.filter(|_| self.on) else {
            return;
        };
        if let Some((tag, last)) = self.last_shown
            && tag == shown.tag
            && at > last
        {
            if self.shown.len() == SHOWN_KEPT {
                self.shown.pop_front();
            }
            self.shown.push_back((at - last) as f64 / 1e6);
        }
        self.last_shown = Some((shown.tag, at));
    }

    fn idle(&mut self) {
        self.last_ns = None;
        self.last_shown = None;
    }

    fn due(&mut self, now_ns: u64, present: Present, render_scale: Option<f32>) -> Option<Frames> {
        if !self.on
            || self
                .sampled_ns
                .is_some_and(|at| now_ns.saturating_sub(at) < SAMPLE_NS)
        {
            return None;
        }
        self.sampled_ns = Some(now_ns);
        let total: f64 = self.shown.iter().sum();
        let shown = (total > 0.0).then(|| {
            (
                self.shown.len() as f64 * 1000.0 / total,
                percentiles(self.shown.iter().copied()),
            )
        });
        Some(Frames {
            snapshot: self.stats.snapshot(),
            shown,
            render_scale,
            present_mode: self.mode.clone(),
            vrr: present.vrr,
            vrr_in_effect: self.vrr_in_effect,
            frame_rate: present.frame_rate.name(),
        })
    }
}

struct Host<A: App> {
    app: A,
    ctx: Context,
    live: Option<Live>,
    failure: Option<String>,
    pulse: Option<Box<dyn Pulse>>,
    proxy: EventLoopProxy<Signal>,
    watching: bool,
    watcher: Watcher,
    pgpu: Pgpu,
    drew: bool,
    started: Instant,
    ui_shown: bool,
    view_shown: bool,
    tally: Tally,
    clock: SystemClock,
    pacer: FramePacer,
    present: Present,
    pacing: Pacing,
    profile: Profile,
    meter: Meter,
}

impl<A: App> Host<A> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, failure: String) {
        self.failure = Some(failure);
        event_loop.exit();
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<Live, String> {
        let attributes = Window::default_attributes()
            .with_title(title(self.app.name(), self.app.label().as_deref()))
            .with_inner_size(LogicalSize::new(SIZE[0], SIZE[1]));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?,
        );
        retitle(&window, self.pulse.as_deref(), &self.app);
        let (gpu, mut target) = block_on(Gpu::window(window.clone()))?;
        self.present = self.app.present();
        target.follow(&gpu, &window, self.present);
        self.pacer.set_present(self.present);
        self.pacer.set_refresh(
            window
                .current_monitor()
                .and_then(|monitor| pace::refresh_millihertz(&monitor)),
        );
        let setting = target.setting(&window, self.present);
        self.meter.set(&setting);
        self.pacing.set(setting);
        if self.app.raised_priority() || pfx_gpu::priority::asked() {
            match pfx_gpu::priority::raise_current_thread() {
                Ok(raised) => eprintln!("{}: the window's thread: {raised}", self.app.name()),
                Err(error) => eprintln!(
                    "{}: the window's thread keeps its priority: {error}",
                    self.app.name()
                ),
            }
        }
        let side = gpu.device.limits().max_texture_dimension_2d as usize;
        let state = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(side),
        );
        let mut painter = Painter::new(&gpu, target.config.format, RendererOptions::default());
        if self.profile.on() {
            painter.time(&gpu);
        }
        if !self.watching {
            self.watching = true;
            let proxy = self.proxy.clone();
            pgpu::watch(self.app.name().to_string(), move |state| {
                proxy.send_event(Signal::Pgpu(state)).is_ok()
            });
        }
        if self.watcher.stop.is_none() {
            self.watcher.follow(self.app.watched(), &self.proxy);
        }
        Ok(Live {
            window,
            gpu,
            target,
            state,
            painter,
            wanted: true,
            wake_at: None,
        })
    }

    fn redraw(&mut self) -> Result<(), String> {
        let Host {
            app,
            ctx,
            live,
            pulse,
            proxy,
            watcher,
            drew,
            started: opened,
            ui_shown,
            view_shown,
            tally,
            clock,
            pacer,
            present,
            pacing,
            profile,
            meter,
            ..
        } = self;
        let Some(live) = live.as_mut() else {
            return Ok(());
        };
        let started = Instant::now();
        profile.start();
        let native = live.window.scale_factor() as f32;
        ctx.set_zoom_factor(native.round().max(1.0) / native);
        meter.follow(app.stats());
        if let Some(frames) = meter.due(clock.now_ns(), *present, app.render_scale()) {
            app.frames(&frames);
        }
        let input = live.state.take_egui_input(&live.window);
        let mut output = ctx.run_ui(input, |root| app.ui(root));
        profile.span(Phase::Ui);
        if app.opened() {
            watcher.follow(app.watched(), proxy);
            retitle(&live.window, pulse.as_deref(), app);
        }
        let asked = app.present();
        let settled = live.target.settled();
        if asked != *present || settled {
            *present = asked;
            pacer.set_present(asked);
            live.target.follow(&live.gpu, &live.window, asked);
            pacer.set_vrr_in_effect(live.target.vrr_in_effect());
            let setting = live.target.setting(&live.window, asked);
            meter.set(&setting);
            pacing.set(setting);
            if settled {
                eprintln!("{}: {}", app.name(), live.target.describe());
            }
        }
        let mut more = false;
        if app.viewport() {
            let framing = clock.now_ns();
            more = app.frame(&live.gpu, live.painter.renderer(), output.pixels_per_point);
            pacing.viewport(clock.now_ns().saturating_sub(framing));
        }
        live.painter.queue(app.commands());
        profile.span(Phase::Viewport);
        live.state
            .handle_platform_output(&live.window, output.platform_output);
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        profile.span(Phase::Tessellate);
        if !live.target.retry(&live.gpu) {
            live.wanted = true;
        }
        let asking = clock.now_ns();
        let acquired = live.target.surface.get_current_texture();
        pacing.acquired(clock.now_ns().saturating_sub(asking));
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                let size = live.window.inner_size();
                live.target.resize(&live.gpu, size.width, size.height);
                live.wanted = true;
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                live.wanted = true;
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface validation failed".to_string());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        profile.span(Phase::Acquire);
        let screen = ScreenDescriptor {
            size_in_pixels: [live.target.config.width, live.target.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        let painting = clock.now_ns();
        live.painter.paint(
            &live.gpu,
            &view,
            &screen,
            &output.textures_delta,
            &primitives,
            Theme::of(ctx).bg,
        );
        pacing.paint(clock.now_ns().saturating_sub(painting));
        output.textures_delta.clear();
        profile.span(Phase::Paint);
        live.window.pre_present_notify();
        let presenting = clock.now_ns();
        live.target
            .present(&live.gpu.queue, frame, pacing.tag().or(meter.tag()));
        pacing.present_call(clock.now_ns().saturating_sub(presenting));
        let now = clock.now_ns();
        pacing.presented(now);
        for shown in live.target.shown() {
            meter.shown(&shown);
            pacing.shown(shown);
        }
        profile.span(Phase::Present);
        if profile.on() {
            let setting = live.target.setting(&live.window, *present);
            let label = format!(
                "{} {} {}",
                setting.present_mode,
                if setting.vrr { "vrr" } else { "fixed" },
                setting.frame_rate
            );
            profile.frame(
                &label,
                app.render_scale(),
                app.gpu_timings(),
                live.painter.timed(),
            );
        }
        let work = started.elapsed();
        meter.presented(now, work, app.gpu_timings(), live.painter.timed());
        let name = app.name().to_string();
        if work >= SLOW {
            eprintln!("{name}: a frame took {} ms", work.as_millis());
        }
        let timing = app.timing();
        for line in tally.add(&name, work, timing.gpu_ms, timing.drops) {
            eprintln!("{line}");
        }
        if let Some(pulse) = pulse.as_ref() {
            pulse.frame(work);
        }
        *drew = true;
        if !*ui_shown {
            *ui_shown = true;
            eprintln!(
                "{name}: the window's first frame at {} ms",
                opened.elapsed().as_millis()
            );
        }
        if !*view_shown && timing.drawn {
            *view_shown = true;
            eprintln!(
                "{name}: the viewport's first frame at {} ms",
                opened.elapsed().as_millis()
            );
        }
        let delay = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map(|viewport| viewport.repaint_delay);
        if delay.is_some_and(|delay| delay.is_zero()) || more {
            live.wanted = true;
        } else {
            pacing.idle();
            meter.idle();
        }
        live.wake_at = delay
            .filter(|delay| !delay.is_zero())
            .and_then(|delay| Instant::now().checked_add(delay));
        Ok(())
    }
}

impl<A: App> ApplicationHandler<Signal> for Host<A> {
    fn user_event(&mut self, _: &ActiveEventLoop, signal: Signal) {
        let mut redraw = false;
        match signal {
            Signal::Files(generation, files) => {
                if generation == self.watcher.generation {
                    self.app.changed(&files);
                    self.watcher.follow(self.app.watched(), &self.proxy);
                    redraw = true;
                }
            }
            Signal::App => redraw = true,
            Signal::Pgpu(state) => {
                if state != self.pgpu {
                    self.pgpu = state;
                    self.app.pgpu(state);
                    redraw = true;
                }
                if !self.drew
                    && let Some(pulse) = self.pulse.as_ref()
                {
                    pulse.idle();
                }
                self.drew = false;
            }
        }
        if redraw && let Some(live) = self.live.as_mut() {
            live.wanted = true;
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(live) => self.live = Some(live),
            Err(failure) => self.fail(event_loop, failure),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                if let Err(failure) = self.redraw() {
                    self.fail(event_loop, failure);
                }
                return;
            }
            WindowEvent::Resized(size) => {
                live.target.resize(&live.gpu, size.width, size.height);
                live.window.request_redraw();
            }
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.pacer.set_refresh(
                    live.window
                        .current_monitor()
                        .and_then(|monitor| pace::refresh_millihertz(&monitor)),
                );
            }
            _ => {}
        }
        let response = live.state.on_window_event(&live.window, &event);
        if response.repaint {
            live.wanted = true;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let now = Instant::now();
        if live.wake_at.is_some_and(|at| at <= now) {
            live.wake_at = None;
            live.wanted = true;
        }
        let waiting = live.wake_at;
        let until = |at: Instant| waiting.map_or(at, |wake| wake.min(at));
        let flow = if !live.wanted {
            waiting.map_or(ControlFlow::Wait, ControlFlow::WaitUntil)
        } else {
            match self.pacer.poll(&self.clock) {
                Pace::Start { .. } => {
                    live.wanted = false;
                    live.window.request_redraw();
                    waiting.map_or(ControlFlow::Wait, ControlFlow::WaitUntil)
                }
                Pace::Wait { until_ns } => {
                    ControlFlow::WaitUntil(until(self.clock.instant(until_ns)))
                }
                Pace::Paused => waiting.map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
            }
        };
        event_loop.set_control_flow(flow);
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.watcher.stop();
        self.pacing.finish();
        self.profile.finish();
        self.live = None;
    }
}

pub fn window(mut app: impl App, pulse: Option<Box<dyn Pulse>>) -> Result<(), Error> {
    pfx_gpu::refuse_window("the editor window").map_err(Error)?;
    let event_loop = EventLoop::<Signal>::with_user_event()
        .build()
        .map_err(|error| Error(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let ctx = context(&app.theme());
    let proxy = event_loop.create_proxy();
    let started = Instant::now();
    let present = app.present();
    let wake = proxy.clone();
    app.waker(Arc::new(move || {
        let _ = wake.send_event(Signal::App);
    }));
    let mut host = Host {
        app,
        ctx,
        live: None,
        failure: None,
        pulse,
        proxy,
        watching: false,
        watcher: Watcher {
            stop: None,
            watch: Watch::default(),
            generation: 0,
        },
        pgpu: Pgpu::default(),
        drew: false,
        started,
        ui_shown: false,
        view_shown: false,
        tally: Tally::default(),
        clock: SystemClock::new(),
        pacer: FramePacer::new(present),
        present,
        pacing: Pacing::from_env(GUI_BUDGET_NS),
        profile: Profile::from_env(),
        meter: Meter::default(),
    };
    event_loop
        .run_app(&mut host)
        .map_err(|error| Error(error.to_string()))?;
    match host.failure {
        Some(failure) => Err(Error(failure)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;

    impl Pulse for Quiet {
        fn frame(&self, _: Duration) {}
        fn idle(&self) {}
    }

    #[test]
    fn the_title_names_the_label_and_a_pulse_may_ignore_it() {
        assert_eq!(title("editor", None), "editor");
        assert_eq!(title("editor", Some("desk.toml")), "editor · desk.toml");
        Quiet.label("desk.toml");
    }

    #[test]
    fn dropped_timings_are_noted_as_they_climb_and_not_every_frame() {
        let mut tally = Tally::default();
        let ms = Duration::from_millis(2);
        assert!(tally.add("editor", ms, Some(4.0), 0).is_empty());
        let noted: Vec<usize> = [1, 1, 2, 3, 3, 9, 10, 30]
            .into_iter()
            .map(|drops| tally.add("editor", ms, None, drops).len())
            .collect();
        assert_eq!(noted, [1, 0, 0, 1, 0, 1, 0, 1]);
        assert!(
            tally.add("editor", ms, None, 61)[0]
                .starts_with("editor: 61 frame timings dropped so far")
        );
    }

    #[test]
    fn every_240_frames_log_the_ui_threads_time_and_the_gpus() {
        let mut tally = Tally::default();
        let mut lines = Vec::new();
        for frame in 0..TALLY * 2 {
            let work = Duration::from_micros(if frame == 7 { 9_000 } else { 1_000 });
            lines.extend(tally.add("editor", work, (frame % 2 == 0).then_some(5.0), 0));
        }
        assert_eq!(
            lines,
            [
                "editor: 240 frames, the UI thread 1.03 ms a frame (longest 9.00 ms), the viewport on the GPU 5.00 ms",
                "editor: 240 frames, the UI thread 1.00 ms a frame (longest 1.00 ms), the viewport on the GPU 5.00 ms",
            ]
        );
    }

    #[test]
    fn the_meter_skips_idle_gaps_counts_a_timing_once_and_samples_four_times_a_second() {
        let mut meter = Meter::default();
        let ms = 1_000_000;
        let timings = FrameTimings {
            frame: 3,
            passes: vec![pfx_gpu::PassTiming {
                label: "opaque".into(),
                milliseconds: 2.0,
            }],
        };
        let paint = FrameTimings {
            frame: 9,
            passes: vec![pfx_gpu::PassTiming {
                label: "ui".into(),
                milliseconds: 0.5,
            }],
        };
        let at = |ms: f64| Shown {
            tag: 0,
            at_ns: Some((ms * 1e6) as u64),
            refresh_ns: None,
        };
        meter.presented(0, Duration::from_millis(1), None, None);
        meter.shown(&at(1.0));
        assert_eq!(meter.tag(), None);
        meter.follow(true);
        assert_eq!(meter.tag(), Some(0));
        meter.presented(
            8 * ms,
            Duration::from_millis(1),
            Some(&timings),
            Some(&paint),
        );
        meter.presented(
            16 * ms,
            Duration::from_millis(1),
            Some(&timings),
            Some(&paint),
        );
        for shown in [at(10.0), at(18.0), at(26.5)] {
            meter.shown(&shown);
        }
        meter.shown(&Shown {
            at_ns: None,
            ..at(30.0)
        });
        meter.idle();
        meter.shown(&at(5_000.0));
        meter.presented(5_000 * ms, Duration::from_millis(1), None, None);
        meter.presented(5_010 * ms, Duration::from_millis(3), None, None);
        let present = Present::default();
        let frames = meter.due(5_010 * ms, present, Some(0.75)).unwrap();
        let (shown_fps, shown) = frames.shown.unwrap();
        assert!((shown_fps - 2000.0 / 16.5).abs() < 1e-9);
        assert_eq!((shown.p50, shown.max), (8.0, 8.5));
        assert_eq!(frames.render_scale, Some(0.75));
        let snapshot = &frames.snapshot;
        assert_eq!(snapshot.interval_ms.max, 10.0);
        assert_eq!(snapshot.interval_ms.p50, 8.0);
        assert!((snapshot.fps - 3000.0 / 26.0).abs() < 1e-9);
        assert_eq!(snapshot.gpu_ms.unwrap().max, 2.5);
        assert_eq!(
            snapshot.gpu_pass_ms,
            vec![("opaque".to_string(), 2.0), ("ui".to_string(), 0.5)]
        );
        assert_eq!(frames.frame_rate, present.frame_rate.name());
        assert!(meter.due(5_100 * ms, present, None).is_none());
        assert!(meter.due(5_260 * ms, present, None).is_some());
        meter.follow(false);
        assert!(meter.due(9_000 * ms, present, None).is_none());
    }

    #[test]
    fn only_the_files_whose_stamps_moved_are_named() {
        let files = [
            PathBuf::from("a.toml"),
            PathBuf::from("b.toml"),
            PathBuf::from("c.toml"),
        ];
        let at = |len| {
            Some(Stamp {
                modified: None,
                len,
            })
        };
        let last = [at(1), None, at(3)];
        let now = [at(1), at(2), at(4)];
        assert_eq!(moved(&files, &last, &now), files[1..].to_vec());
        assert!(moved(&files, &last, &last).is_empty());
    }

    #[test]
    fn a_stamp_reads_a_file_and_none_for_a_missing_one() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/shell-tests/stamp");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("watched.toml");
        std::fs::write(&file, "a = 1\n").unwrap();
        assert_eq!(stamp(&file).map(|stamp| stamp.len), Some(6));
        assert_eq!(stamp(&dir.join("missing.toml")), None);
    }
}
