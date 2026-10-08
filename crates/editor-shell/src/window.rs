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
use pfx_gpu::{Gpu, WindowTarget, wgpu};

use crate::paint::Painter;
use crate::pgpu::{self, Pgpu};
use crate::{App, Error, Theme, Watch, block_on, context};

const SLOW: Duration = Duration::from_millis(250);
const TALLY: u32 = 240;
const SIZE: [f64; 2] = [1586.0, 992.0];

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
        let (gpu, target) = block_on(Gpu::window(window.clone()))?;
        let side = gpu.device.limits().max_texture_dimension_2d as usize;
        let state = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(side),
        );
        let painter = Painter::new(&gpu, target.config.format, RendererOptions::default());
        window.request_redraw();
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
            ..
        } = self;
        let Some(live) = live.as_mut() else {
            return Ok(());
        };
        let started = Instant::now();
        let native = live.window.scale_factor() as f32;
        ctx.set_zoom_factor(native.round().max(1.0) / native);
        let input = live.state.take_egui_input(&live.window);
        let output = ctx.run(input, |ctx| app.ui(ctx));
        if app.opened() {
            watcher.follow(app.watched(), proxy);
            retitle(&live.window, pulse.as_deref(), app);
        }
        let mut more = false;
        if app.viewport() {
            more = app.frame(&live.gpu, live.painter.renderer(), output.pixels_per_point);
        }
        live.state
            .handle_platform_output(&live.window, output.platform_output);
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let frame = match live.target.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                let size = live.window.inner_size();
                live.target.resize(&live.gpu, size.width, size.height);
                live.window.request_redraw();
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => {
                live.window.request_redraw();
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let screen = ScreenDescriptor {
            size_in_pixels: [live.target.config.width, live.target.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        live.painter.paint(
            &live.gpu,
            &view,
            &screen,
            &output.textures_delta,
            &primitives,
            Theme::of(ctx).bg,
        );
        live.window.pre_present_notify();
        frame.present();
        let work = started.elapsed();
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
        let again = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|viewport| viewport.repaint_delay.is_zero());
        if again || more {
            live.window.request_redraw();
        }
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
        if redraw && let Some(live) = self.live.as_ref() {
            live.window.request_redraw();
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
            _ => {}
        }
        let response = live.state.on_window_event(&live.window, &event);
        if response.repaint {
            live.window.request_redraw();
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.watcher.stop();
        self.live = None;
    }
}

pub fn window(mut app: impl App, pulse: Option<Box<dyn Pulse>>) -> Result<(), Error> {
    let event_loop = EventLoop::<Signal>::with_user_event()
        .build()
        .map_err(|error| Error(error.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let ctx = context(&app.theme());
    let proxy = event_loop.create_proxy();
    let started = Instant::now();
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
