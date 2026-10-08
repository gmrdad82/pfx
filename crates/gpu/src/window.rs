use std::fmt;

use serde::{Deserialize, Serialize};

const NS_PER_SEC: u64 = 1_000_000_000;
const FIFO_LATENCY: u32 = 2;
const VRR_LATENCY: u32 = 1;
const VRR_HEADROOM_MHZ: u32 = 3_000;

const GAME_MODES: [wgpu::PresentMode; 3] = [
    wgpu::PresentMode::Mailbox,
    wgpu::PresentMode::FifoRelaxed,
    wgpu::PresentMode::Fifo,
];
const MACOS_VRR: [wgpu::PresentMode; 2] = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo];
const X11_VRR: [wgpu::PresentMode; 3] = [
    wgpu::PresentMode::Mailbox,
    wgpu::PresentMode::Fifo,
    wgpu::PresentMode::FifoRelaxed,
];
const WAYLAND_VRR: [wgpu::PresentMode; 2] = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo];
const FIFO_ONLY: [wgpu::PresentMode; 1] = [wgpu::PresentMode::Fifo];

pub const LAYOUT_UNITS: Size = Size {
    width: 1920,
    height: 1080,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoMode {
    pub size: Size,
    pub bit_depth: u16,
    pub refresh_millihertz: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub name: Option<String>,
    pub position: Point,
    pub size: Size,
    pub refresh_millihertz: Option<u32>,
    pub video_modes: Vec<VideoMode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitors {
    pub list: Vec<Monitor>,
    pub primary: Option<usize>,
    pub current: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonitorChoice {
    Current,
    Primary,
    Index(usize),
    Name(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoModeChoice {
    Exact(VideoMode),
    SizeRefresh { size: Size, refresh_millihertz: u32 },
    Native,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Windowed {
        size: Size,
        position: Option<Point>,
    },
    BorderlessWindowed {
        size: Size,
        position: Option<Point>,
    },
    BorderlessFullscreen {
        monitor: MonitorChoice,
    },
    ExclusiveFullscreen {
        monitor: MonitorChoice,
        video_mode: VideoModeChoice,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformCaps {
    pub exclusive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Windows,
    Macos,
    X11,
    Wayland,
    Other,
}

impl Host {
    pub fn supports_exclusive(self) -> bool {
        matches!(self, Host::Windows | Host::Macos | Host::X11)
    }

    pub fn vrr_present_modes(self) -> &'static [wgpu::PresentMode] {
        match self {
            Host::Windows => &GAME_MODES,
            Host::Macos => &MACOS_VRR,
            Host::X11 => &X11_VRR,
            Host::Wayland => &WAYLAND_VRR,
            Host::Other => &FIFO_ONLY,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    ExclusiveUnsupported,
    VideoModeMissing,
    MonitorMissing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenPlan {
    Windowed,
    Borderless { monitor: usize },
    Exclusive { monitor: usize, mode: VideoMode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModePlan {
    pub fullscreen: FullscreenPlan,
    pub decorations: bool,
    pub resizable: bool,
    pub inner_size: Option<Size>,
    pub outer_position: Option<Point>,
    pub fallback: Option<Fallback>,
    pub monitor: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    EmptySize,
    NoMonitor,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::EmptySize => write!(f, "window size must be nonzero"),
            PlanError::NoMonitor => write!(f, "no monitor to fullscreen on"),
        }
    }
}

impl std::error::Error for PlanError {}

pub trait WindowOps {
    fn set_fullscreen(&mut self, fullscreen: &FullscreenPlan);
    fn set_decorations(&mut self, decorations: bool);
    fn request_inner_size(&mut self, size: Size);
    fn set_outer_position(&mut self, position: Point);
    fn set_resizable(&mut self, _resizable: bool) {}
}

pub fn plan_display_mode(
    mode: &DisplayMode,
    platform: PlatformCaps,
    monitors: &Monitors,
) -> Result<ModePlan, PlanError> {
    match mode {
        DisplayMode::Windowed { size, position } => placed(*size, *position, true, monitors),
        DisplayMode::BorderlessWindowed { size, position } => {
            placed(*size, *position, false, monitors)
        }
        DisplayMode::BorderlessFullscreen { monitor } => borderless(monitor, monitors, None),
        DisplayMode::ExclusiveFullscreen {
            monitor,
            video_mode,
        } => exclusive(monitor, video_mode, platform, monitors),
    }
}

pub fn apply_plan(plan: &ModePlan, window: &mut impl WindowOps) {
    match plan.fullscreen {
        FullscreenPlan::Windowed => {
            window.set_fullscreen(&plan.fullscreen);
            window.set_decorations(plan.decorations);
            window.set_resizable(plan.resizable);
            if let Some(position) = plan.outer_position {
                window.set_outer_position(position);
            }
            if let Some(size) = plan.inner_size {
                window.request_inner_size(size);
            }
        }
        FullscreenPlan::Borderless { .. } | FullscreenPlan::Exclusive { .. } => {
            window.set_decorations(plan.decorations);
            window.set_fullscreen(&plan.fullscreen);
        }
    }
}

fn placed(
    size: Size,
    position: Option<Point>,
    decorations: bool,
    monitors: &Monitors,
) -> Result<ModePlan, PlanError> {
    if size.width == 0 || size.height == 0 {
        return Err(PlanError::EmptySize);
    }
    Ok(ModePlan {
        fullscreen: FullscreenPlan::Windowed,
        decorations,
        resizable: decorations,
        inner_size: Some(size),
        outer_position: position,
        fallback: None,
        monitor: monitors.current,
    })
}

fn resolve(
    choice: &MonitorChoice,
    monitors: &Monitors,
) -> Result<(usize, Option<Fallback>), PlanError> {
    if monitors.list.is_empty() {
        return Err(PlanError::NoMonitor);
    }
    let found = match choice {
        MonitorChoice::Current => monitors.current,
        MonitorChoice::Primary => monitors.primary,
        MonitorChoice::Index(index) => (*index < monitors.list.len()).then_some(*index),
        MonitorChoice::Name(name) => monitors
            .list
            .iter()
            .position(|monitor| monitor.name.as_deref() == Some(name.as_str())),
    };
    if let Some(index) = found {
        return Ok((index, None));
    }
    let index = monitors.current.or(monitors.primary).unwrap_or(0);
    Ok((index, Some(Fallback::MonitorMissing)))
}

fn borderless_plan(monitor: usize, fallback: Option<Fallback>) -> ModePlan {
    ModePlan {
        fullscreen: FullscreenPlan::Borderless { monitor },
        decorations: false,
        resizable: false,
        inner_size: None,
        outer_position: None,
        fallback,
        monitor: Some(monitor),
    }
}

fn borderless(
    choice: &MonitorChoice,
    monitors: &Monitors,
    fallback: Option<Fallback>,
) -> Result<ModePlan, PlanError> {
    let (monitor, missed) = resolve(choice, monitors)?;
    Ok(borderless_plan(monitor, fallback.or(missed)))
}

fn exclusive(
    choice: &MonitorChoice,
    video_mode: &VideoModeChoice,
    platform: PlatformCaps,
    monitors: &Monitors,
) -> Result<ModePlan, PlanError> {
    let (monitor, missed) = resolve(choice, monitors)?;
    if !platform.exclusive {
        return Ok(borderless_plan(
            monitor,
            missed.or(Some(Fallback::ExclusiveUnsupported)),
        ));
    }
    if let Some(mode) = find_video_mode(&monitors.list[monitor], video_mode) {
        return Ok(ModePlan {
            fullscreen: FullscreenPlan::Exclusive { monitor, mode },
            decorations: false,
            resizable: false,
            inner_size: None,
            outer_position: None,
            fallback: missed,
            monitor: Some(monitor),
        });
    }
    Ok(borderless_plan(
        monitor,
        missed.or(Some(Fallback::VideoModeMissing)),
    ))
}

fn find_video_mode(monitor: &Monitor, choice: &VideoModeChoice) -> Option<VideoMode> {
    match choice {
        VideoModeChoice::Exact(wanted) => monitor
            .video_modes
            .iter()
            .copied()
            .find(|mode| *mode == *wanted),
        VideoModeChoice::SizeRefresh {
            size,
            refresh_millihertz,
        } => monitor
            .video_modes
            .iter()
            .copied()
            .filter(|mode| mode.size == *size && mode.refresh_millihertz == *refresh_millihertz)
            .max_by_key(|mode| mode.bit_depth),
        VideoModeChoice::Native => monitor
            .video_modes
            .iter()
            .copied()
            .filter(|mode| mode.size == monitor.size)
            .max_by_key(|mode| {
                let current = monitor
                    .refresh_millihertz
                    .map(|refresh| mode.refresh_millihertz.abs_diff(refresh) <= 1_000);
                (current, mode.refresh_millihertz, mode.bit_depth)
            }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentPreference {
    pub modes: Vec<wgpu::PresentMode>,
    pub desired_maximum_frame_latency: u32,
}

impl Default for PresentPreference {
    fn default() -> Self {
        Self::fifo()
    }
}

impl PresentPreference {
    pub fn fifo() -> Self {
        Self {
            modes: vec![wgpu::PresentMode::Fifo],
            desired_maximum_frame_latency: FIFO_LATENCY,
        }
    }

    pub fn game() -> Self {
        Self {
            modes: GAME_MODES.to_vec(),
            desired_maximum_frame_latency: FIFO_LATENCY,
        }
    }

    pub fn vrr(host: Host) -> Self {
        Self {
            modes: host.vrr_present_modes().to_vec(),
            desired_maximum_frame_latency: VRR_LATENCY,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentChoice {
    pub requested: Vec<wgpu::PresentMode>,
    pub selected: wgpu::PresentMode,
    pub fell_back: bool,
    pub desired_maximum_frame_latency: u32,
}

pub fn choose_present(
    preference: &PresentPreference,
    available: &[wgpu::PresentMode],
) -> PresentChoice {
    let requested = if preference.modes.is_empty() {
        vec![wgpu::PresentMode::Fifo]
    } else {
        preference.modes.clone()
    };
    let matched = requested
        .iter()
        .copied()
        .find(|mode| available.contains(mode));
    let (selected, fell_back) = match matched {
        Some(mode) => (mode, false),
        None => (wgpu::PresentMode::Fifo, true),
    };
    PresentChoice {
        requested,
        selected,
        fell_back,
        desired_maximum_frame_latency: preference.desired_maximum_frame_latency.max(1),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RenderScale {
    Native,
    Factor(f32),
    Size(Size),
}

pub fn render_size(window: Size, scale: RenderScale) -> Option<Size> {
    match scale {
        RenderScale::Native => nonzero(window),
        RenderScale::Size(size) => nonzero(size),
        RenderScale::Factor(factor) => scaled(window, factor),
    }
}

fn nonzero(size: Size) -> Option<Size> {
    (size.width > 0 && size.height > 0).then_some(size)
}

fn scaled(window: Size, factor: f32) -> Option<Size> {
    if !factor.is_finite() || factor <= 0.0 || window.width == 0 || window.height == 0 {
        return None;
    }
    let width = (f64::from(window.width) * f64::from(factor)).round();
    let height = (f64::from(window.height) * f64::from(factor)).round();
    if !(1.0..f64::from(u32::MAX)).contains(&width) || !(1.0..f64::from(u32::MAX)).contains(&height)
    {
        return None;
    }
    Some(Size {
        width: width as u32,
        height: height as u32,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    pub matrix: [f32; 16],
    pub inverse: [f32; 16],
    pub visible: Rect,
}

impl Letterbox {
    pub fn map(&self, layout_x: f32, layout_y: f32) -> [f32; 2] {
        transform(&self.matrix, layout_x, layout_y)
    }

    pub fn unmap(&self, target_x: f32, target_y: f32) -> [f32; 2] {
        transform(&self.inverse, target_x, target_y)
    }

    pub fn contains(&self, target_x: f32, target_y: f32) -> bool {
        let visible = self.visible;
        target_x >= visible.x
            && target_y >= visible.y
            && target_x < visible.x + visible.width
            && target_y < visible.y + visible.height
    }
}

pub fn letterbox(layout: Size, target: Size) -> Option<Letterbox> {
    if layout.width == 0 || layout.height == 0 || target.width == 0 || target.height == 0 {
        return None;
    }
    let layout_w = u64::from(layout.width);
    let layout_h = u64::from(layout.height);
    let target_w = u64::from(target.width);
    let target_h = u64::from(target.height);
    let (visible_w, visible_h, x, y) = if target_w * layout_h >= layout_w * target_h {
        let visible_w = u32::try_from(target_h * layout_w / layout_h).ok()?;
        let gap = target.width.checked_sub(visible_w)?;
        (visible_w, target.height, gap / 2, 0)
    } else {
        let visible_h = u32::try_from(target_w * layout_h / layout_w).ok()?;
        let gap = target.height.checked_sub(visible_h)?;
        (target.width, visible_h, 0, gap / 2)
    };
    if visible_w == 0 || visible_h == 0 {
        return None;
    }
    let sx = visible_w as f32 / layout.width as f32;
    let sy = visible_h as f32 / layout.height as f32;
    let tx = x as f32;
    let ty = y as f32;
    Some(Letterbox {
        matrix: affine(sx, sy, tx, ty),
        inverse: affine(1.0 / sx, 1.0 / sy, -tx / sx, -ty / sy),
        visible: Rect {
            x: tx,
            y: ty,
            width: visible_w as f32,
            height: visible_h as f32,
        },
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LetterboxRounding {
    #[default]
    Floor,
    Nearest,
}

pub fn letterbox_with(
    layout: Size,
    target: Size,
    rounding: LetterboxRounding,
) -> Option<Letterbox> {
    match rounding {
        LetterboxRounding::Floor => letterbox(layout, target),
        LetterboxRounding::Nearest => {
            letterbox_nearest(layout.width as f32, layout.height as f32, target)
        }
    }
}

pub fn letterbox_nearest(layout_width: f32, layout_height: f32, target: Size) -> Option<Letterbox> {
    let finite = layout_width.is_finite() && layout_height.is_finite();
    if !finite
        || layout_width <= 0.0
        || layout_height <= 0.0
        || target.width == 0
        || target.height == 0
    {
        return None;
    }
    let target_width = target.width as f32;
    let target_height = target.height as f32;
    let scale = (target_width / layout_width).min(target_height / layout_height);
    let x = ((target_width - layout_width * scale) * 0.5).round();
    let y = ((target_height - layout_height * scale) * 0.5).round();
    Some(Letterbox {
        matrix: affine(scale, scale, x, y),
        inverse: affine(1.0 / scale, 1.0 / scale, -x / scale, -y / scale),
        visible: Rect {
            x,
            y,
            width: layout_width * scale,
            height: layout_height * scale,
        },
    })
}

fn affine(sx: f32, sy: f32, tx: f32, ty: f32) -> [f32; 16] {
    [
        sx, 0.0, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, 0.0, 1.0,
    ]
}

fn transform(matrix: &[f32; 16], x: f32, y: f32) -> [f32; 2] {
    [
        matrix[0] * x + matrix[4] * y + matrix[12],
        matrix[1] * x + matrix[5] * y + matrix[13],
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub window: Size,
    pub render: Size,
    pub letterbox: Letterbox,
}

impl Viewport {
    pub fn render_point(&self, window_x: f32, window_y: f32) -> [f32; 2] {
        [
            window_x * self.render.width as f32 / self.window.width as f32,
            window_y * self.render.height as f32 / self.window.height as f32,
        ]
    }

    pub fn pointer_to_layout(&self, window_x: f32, window_y: f32) -> [f32; 2] {
        let [x, y] = self.render_point(window_x, window_y);
        self.letterbox.unmap(x, y)
    }

    pub fn pointer_inside(&self, window_x: f32, window_y: f32) -> bool {
        let [x, y] = self.render_point(window_x, window_y);
        self.letterbox.contains(x, y)
    }
}

pub fn viewport(window: Size, scale: RenderScale, layout: Size) -> Option<Viewport> {
    viewport_with(window, scale, layout, LetterboxRounding::Floor)
}

pub fn viewport_with(
    window: Size,
    scale: RenderScale,
    layout: Size,
    rounding: LetterboxRounding,
) -> Option<Viewport> {
    let render = render_size(window, scale)?;
    Some(Viewport {
        window,
        render,
        letterbox: letterbox_with(layout, render, rounding)?,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameCap {
    Fps30,
    Fps60,
    Fps90,
    Fps120,
    Uncapped,
}

impl FrameCap {
    pub fn fps(self) -> Option<u32> {
        match self {
            FrameCap::Fps30 => Some(30),
            FrameCap::Fps60 => Some(60),
            FrameCap::Fps90 => Some(90),
            FrameCap::Fps120 => Some(120),
            FrameCap::Uncapped => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameCaps {
    pub focused: FrameCap,
    pub background: FrameCap,
    pub pause_when_minimized: bool,
}

impl FrameCaps {
    pub fn new(focused: FrameCap, background: FrameCap) -> Self {
        Self {
            focused,
            background,
            pause_when_minimized: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    Focused(bool),
    Occluded(bool),
    Minimized(bool),
}

pub trait Clock {
    fn now_ns(&self) -> u64;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FakeClock {
    pub now_ns: u64,
}

impl Clock for FakeClock {
    fn now_ns(&self) -> u64 {
        self.now_ns
    }
}

pub struct SystemClock {
    start: std::time::Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn instant(&self, ns: u64) -> std::time::Instant {
        self.start + std::time::Duration::from_nanos(ns)
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Start { next_ns: Option<u64> },
    Wait { until_ns: u64 },
    Paused,
}

#[derive(Clone, Copy, Debug)]
enum Schedule {
    Fresh,
    Hold,
    Open,
    Grid {
        origin_ns: u64,
        index: u64,
        fps: u32,
    },
}

#[derive(Clone, Debug)]
pub struct FramePacer {
    caps: FrameCaps,
    focused: bool,
    occluded: bool,
    minimized: bool,
    vrr_millihertz: Option<u32>,
    schedule: Schedule,
}

impl FramePacer {
    pub fn new(caps: FrameCaps) -> Self {
        Self {
            caps,
            focused: true,
            occluded: false,
            minimized: false,
            vrr_millihertz: None,
            schedule: Schedule::Fresh,
        }
    }

    pub fn set_caps(&mut self, caps: FrameCaps) {
        self.caps = caps;
    }

    pub fn set_vrr(&mut self, refresh_millihertz: Option<u32>) {
        self.vrr_millihertz = refresh_millihertz;
    }

    pub fn on(&mut self, event: Attention) {
        match event {
            Attention::Focused(focused) => self.focused = focused,
            Attention::Occluded(occluded) => self.occluded = occluded,
            Attention::Minimized(minimized) => self.minimized = minimized,
        }
    }

    pub fn effective_fps(&self) -> Option<u32> {
        let cap = if self.focused && !self.occluded {
            self.caps.focused
        } else {
            self.caps.background
        };
        paced_fps(cap, self.vrr_millihertz)
    }

    pub fn poll(&mut self, clock: &impl Clock) -> Pace {
        let now = clock.now_ns();
        if self.caps.pause_when_minimized && self.minimized {
            self.schedule = Schedule::Hold;
            return Pace::Paused;
        }
        match self.effective_fps() {
            None => {
                self.schedule = Schedule::Open;
                Pace::Start { next_ns: None }
            }
            Some(0) => {
                self.schedule = Schedule::Hold;
                Pace::Paused
            }
            Some(fps) => self.poll_grid(now, fps),
        }
    }

    fn poll_grid(&mut self, now: u64, fps: u32) -> Pace {
        let (origin_ns, index) = match self.schedule {
            Schedule::Grid {
                origin_ns,
                index,
                fps: scheduled,
            } if scheduled == fps => (origin_ns, index),
            _ => (now, 0),
        };
        let due = slot(origin_ns, index, fps);
        if now < due {
            return Pace::Wait { until_ns: due };
        }
        let next = first_slot_after(origin_ns, now, fps).max(index.saturating_add(1));
        self.schedule = Schedule::Grid {
            origin_ns,
            index: next,
            fps,
        };
        Pace::Start {
            next_ns: Some(slot(origin_ns, next, fps)),
        }
    }
}

pub fn vrr_limit_fps(refresh_millihertz: u32) -> Option<u32> {
    let fps = refresh_millihertz.saturating_sub(VRR_HEADROOM_MHZ) / 1_000;
    (fps > 0).then_some(fps)
}

pub fn paced_fps(cap: FrameCap, vrr_millihertz: Option<u32>) -> Option<u32> {
    let Some(limit) = vrr_millihertz.and_then(vrr_limit_fps) else {
        return cap.fps();
    };
    Some(cap.fps().map_or(limit, |fps| fps.min(limit)))
}

fn slot(origin_ns: u64, index: u64, fps: u32) -> u64 {
    debug_assert!(fps > 0);
    let step = u128::from(index) * u128::from(NS_PER_SEC) / u128::from(fps);
    origin_ns.saturating_add(u64::try_from(step).unwrap_or(u64::MAX))
}

fn first_slot_after(origin_ns: u64, now_ns: u64, fps: u32) -> u64 {
    debug_assert!(fps > 0);
    if now_ns <= origin_ns {
        return 1;
    }
    let need = u128::from(now_ns - origin_ns) + 1;
    let ns = u128::from(NS_PER_SEC);
    let prod = need.saturating_mul(u128::from(fps));
    let index = prod.div_ceil(ns);
    u64::try_from(index).unwrap_or(u64::MAX).max(1)
}

#[cfg(any(feature = "window", test))]
mod glue {
    use super::{
        Attention, DisplayMode, Fallback, FullscreenPlan, Host, ModePlan, Monitor, Monitors,
        PlanError, PlatformCaps, Point, Size, VideoMode, WindowOps, apply_plan, plan_display_mode,
    };
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::monitor::MonitorHandle;
    use winit::window::{Fullscreen, Window};

    pub fn refresh_millihertz(monitor: &MonitorHandle) -> Option<u32> {
        monitor.refresh_rate_millihertz()
    }

    pub fn minimized(window: &Window) -> bool {
        window.is_minimized().unwrap_or(false)
    }

    pub fn attention(event: &winit::event::WindowEvent) -> Option<Attention> {
        match event {
            winit::event::WindowEvent::Focused(focused) => Some(Attention::Focused(*focused)),
            winit::event::WindowEvent::Occluded(occluded) => Some(Attention::Occluded(*occluded)),
            _ => None,
        }
    }

    pub fn host(window: &Window) -> Host {
        #[cfg(target_os = "windows")]
        {
            let _ = window;
            Host::Windows
        }
        #[cfg(target_os = "macos")]
        {
            let _ = window;
            Host::Macos
        }
        #[cfg(any(target_os = "android", target_os = "ios", target_arch = "wasm32"))]
        {
            let _ = window;
            Host::Other
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd",
        ))]
        {
            if wayland(window) {
                Host::Wayland
            } else {
                Host::X11
            }
        }
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "android",
            target_os = "ios",
            target_arch = "wasm32",
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd",
        )))]
        {
            let _ = window;
            Host::Other
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
    ))]
    fn wayland(window: &Window) -> bool {
        use winit::platform::wayland::WindowExtWayland;
        window.xdg_toplevel().is_some()
    }

    pub fn monitors(window: &Window) -> Monitors {
        let handles: Vec<MonitorHandle> = window.available_monitors().collect();
        listed(window, &handles)
    }

    fn listed(window: &Window, handles: &[MonitorHandle]) -> Monitors {
        Monitors {
            current: index_of(handles, window.current_monitor().as_ref()),
            primary: index_of(handles, window.primary_monitor().as_ref()),
            list: handles.iter().map(describe).collect(),
        }
    }

    pub fn placement(window: &Window) -> Option<(Option<String>, Point)> {
        let monitor = window.current_monitor()?;
        let outer = window.outer_position().ok()?;
        let origin = monitor.position();
        Some((
            monitor.name(),
            Point {
                x: outer.x - origin.x,
                y: outer.y - origin.y,
            },
        ))
    }

    pub fn apply_display_mode(window: &Window, mode: &DisplayMode) -> Result<ModePlan, PlanError> {
        let handles: Vec<MonitorHandle> = window.available_monitors().collect();
        let monitors = listed(window, &handles);
        let platform = PlatformCaps {
            exclusive: host(window).supports_exclusive(),
        };
        let plan = plan_display_mode(mode, platform, &monitors)?;
        let mut ops = WinitOps {
            window,
            handles: &handles,
            missed: false,
        };
        apply_plan(&plan, &mut ops);
        Ok(ops.finish(plan))
    }

    fn index_of(handles: &[MonitorHandle], monitor: Option<&MonitorHandle>) -> Option<usize> {
        monitor.and_then(|monitor| handles.iter().position(|handle| handle == monitor))
    }

    fn describe(handle: &MonitorHandle) -> Monitor {
        let position = handle.position();
        let size = handle.size();
        Monitor {
            name: handle.name(),
            position: Point {
                x: position.x,
                y: position.y,
            },
            size: Size {
                width: size.width,
                height: size.height,
            },
            refresh_millihertz: refresh_millihertz(handle),
            video_modes: handle
                .video_modes()
                .map(|mode| {
                    let size = mode.size();
                    VideoMode {
                        size: Size {
                            width: size.width,
                            height: size.height,
                        },
                        bit_depth: mode.bit_depth(),
                        refresh_millihertz: mode.refresh_rate_millihertz(),
                    }
                })
                .collect(),
        }
    }

    struct WinitOps<'a> {
        window: &'a Window,
        handles: &'a [MonitorHandle],
        missed: bool,
    }

    impl WindowOps for WinitOps<'_> {
        fn set_fullscreen(&mut self, fullscreen: &FullscreenPlan) {
            match fullscreen {
                FullscreenPlan::Windowed => self.window.set_fullscreen(None),
                FullscreenPlan::Borderless { monitor } => {
                    let handle = self.handles.get(*monitor).cloned();
                    self.window
                        .set_fullscreen(Some(Fullscreen::Borderless(handle)));
                }
                FullscreenPlan::Exclusive { monitor, mode } => {
                    let video = self
                        .handles
                        .get(*monitor)
                        .and_then(|handle| find_handle(handle, mode));
                    if let Some(video) = video {
                        self.window
                            .set_fullscreen(Some(Fullscreen::Exclusive(video)));
                    } else {
                        self.missed = true;
                        let handle = self.handles.get(*monitor).cloned();
                        self.window
                            .set_fullscreen(Some(Fullscreen::Borderless(handle)));
                    }
                }
            }
        }

        fn set_decorations(&mut self, decorations: bool) {
            self.window.set_decorations(decorations);
        }

        fn set_resizable(&mut self, resizable: bool) {
            self.window.set_resizable(resizable);
        }

        fn request_inner_size(&mut self, size: Size) {
            let _ = self
                .window
                .request_inner_size(PhysicalSize::new(size.width, size.height));
        }

        fn set_outer_position(&mut self, position: Point) {
            self.window
                .set_outer_position(PhysicalPosition::new(position.x, position.y));
        }
    }

    impl WinitOps<'_> {
        fn finish(self, mut plan: ModePlan) -> ModePlan {
            if self.missed
                && let FullscreenPlan::Exclusive { monitor, .. } = plan.fullscreen
            {
                plan.fullscreen = FullscreenPlan::Borderless { monitor };
                if plan.fallback.is_none() {
                    plan.fallback = Some(Fallback::VideoModeMissing);
                }
            }
            plan
        }
    }

    fn find_handle(
        monitor: &MonitorHandle,
        mode: &VideoMode,
    ) -> Option<winit::monitor::VideoModeHandle> {
        monitor.video_modes().find(|candidate| {
            let size = candidate.size();
            size.width == mode.size.width
                && size.height == mode.size.height
                && candidate.bit_depth() == mode.bit_depth
                && candidate.refresh_rate_millihertz() == mode.refresh_millihertz
        })
    }
}

#[cfg(any(feature = "window", test))]
pub use glue::{
    apply_display_mode, attention, host, minimized, monitors, placement, refresh_millihertz,
};

#[cfg(test)]
mod tests {
    use super::*;

    const WAYLAND: PlatformCaps = PlatformCaps { exclusive: false };
    const X11: PlatformCaps = PlatformCaps { exclusive: true };

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    fn video(width: u32, height: u32, refresh: u32, depth: u16) -> VideoMode {
        VideoMode {
            size: size(width, height),
            bit_depth: depth,
            refresh_millihertz: refresh,
        }
    }

    fn desk() -> Monitors {
        Monitors {
            list: vec![
                Monitor {
                    name: Some("primary".into()),
                    position: Point { x: 0, y: 0 },
                    size: size(1920, 1080),
                    refresh_millihertz: Some(60_000),
                    video_modes: vec![video(1920, 1080, 60_000, 32)],
                },
                Monitor {
                    name: Some("side".into()),
                    position: Point { x: 1920, y: 0 },
                    size: size(2560, 1440),
                    refresh_millihertz: Some(144_000),
                    video_modes: vec![
                        video(2560, 1440, 144_000, 24),
                        video(2560, 1440, 144_000, 32),
                        video(1920, 1080, 60_000, 32),
                    ],
                },
            ],
            primary: Some(0),
            current: Some(1),
        }
    }

    #[derive(Default)]
    struct Record {
        calls: Vec<Call>,
    }

    #[derive(Debug, PartialEq)]
    enum Call {
        Fullscreen(FullscreenPlan),
        Decorations(bool),
        Size(Size),
        Position(Point),
    }

    impl WindowOps for Record {
        fn set_fullscreen(&mut self, fullscreen: &FullscreenPlan) {
            self.calls.push(Call::Fullscreen(*fullscreen));
        }

        fn set_decorations(&mut self, decorations: bool) {
            self.calls.push(Call::Decorations(decorations));
        }

        fn request_inner_size(&mut self, size: Size) {
            self.calls.push(Call::Size(size));
        }

        fn set_outer_position(&mut self, position: Point) {
            self.calls.push(Call::Position(position));
        }
    }

    fn applied(plan: &ModePlan) -> Vec<Call> {
        let mut record = Record::default();
        apply_plan(plan, &mut record);
        record.calls
    }

    #[test]
    fn windowed_and_borderless_windowed_keep_their_size() {
        let monitors = desk();
        let place = Some(Point { x: 40, y: 20 });
        let windowed = plan_display_mode(
            &DisplayMode::Windowed {
                size: size(1280, 720),
                position: place,
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(windowed.fullscreen, FullscreenPlan::Windowed);
        assert!(windowed.decorations);
        assert_eq!(windowed.fallback, None);
        assert_eq!(windowed.monitor, Some(1));
        assert_eq!(
            applied(&windowed),
            vec![
                Call::Fullscreen(FullscreenPlan::Windowed),
                Call::Decorations(true),
                Call::Position(Point { x: 40, y: 20 }),
                Call::Size(size(1280, 720)),
            ]
        );
        let borderless = plan_display_mode(
            &DisplayMode::BorderlessWindowed {
                size: size(1600, 900),
                position: None,
            },
            WAYLAND,
            &monitors,
        )
        .unwrap();
        assert!(!borderless.decorations);
        assert_eq!(borderless.fallback, None);
        assert_eq!(
            applied(&borderless),
            vec![
                Call::Fullscreen(FullscreenPlan::Windowed),
                Call::Decorations(false),
                Call::Size(size(1600, 900)),
            ]
        );
        assert!(matches!(
            plan_display_mode(
                &DisplayMode::Windowed {
                    size: size(0, 10),
                    position: None,
                },
                X11,
                &monitors,
            ),
            Err(PlanError::EmptySize)
        ));
    }

    #[test]
    fn fullscreen_chooses_a_monitor_and_exclusive_falls_back() {
        let monitors = desk();
        let borderless = plan_display_mode(
            &DisplayMode::BorderlessFullscreen {
                monitor: MonitorChoice::Name("primary".into()),
            },
            WAYLAND,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            borderless.fullscreen,
            FullscreenPlan::Borderless { monitor: 0 }
        );
        assert_eq!(borderless.fallback, None);
        assert_eq!(
            applied(&borderless),
            vec![
                Call::Decorations(false),
                Call::Fullscreen(FullscreenPlan::Borderless { monitor: 0 }),
            ]
        );

        let mode = VideoModeChoice::SizeRefresh {
            size: size(2560, 1440),
            refresh_millihertz: 144_000,
        };
        let exclusive = plan_display_mode(
            &DisplayMode::ExclusiveFullscreen {
                monitor: MonitorChoice::Current,
                video_mode: mode,
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            exclusive.fullscreen,
            FullscreenPlan::Exclusive {
                monitor: 1,
                mode: video(2560, 1440, 144_000, 32),
            }
        );
        assert_eq!(exclusive.fallback, None);

        let exact = plan_display_mode(
            &DisplayMode::ExclusiveFullscreen {
                monitor: MonitorChoice::Index(1),
                video_mode: VideoModeChoice::Exact(video(1920, 1080, 60_000, 32)),
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            exact.fullscreen,
            FullscreenPlan::Exclusive {
                monitor: 1,
                mode: video(1920, 1080, 60_000, 32),
            }
        );
        let depth = plan_display_mode(
            &DisplayMode::ExclusiveFullscreen {
                monitor: MonitorChoice::Index(1),
                video_mode: VideoModeChoice::Exact(video(2560, 1440, 144_000, 24)),
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            depth.fullscreen,
            FullscreenPlan::Exclusive {
                monitor: 1,
                mode: video(2560, 1440, 144_000, 24),
            }
        );

        let wayland = plan_display_mode(
            &DisplayMode::ExclusiveFullscreen {
                monitor: MonitorChoice::Primary,
                video_mode: VideoModeChoice::Exact(video(1920, 1080, 60_000, 32)),
            },
            WAYLAND,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            wayland.fullscreen,
            FullscreenPlan::Borderless { monitor: 0 }
        );
        assert_eq!(wayland.fallback, Some(Fallback::ExclusiveUnsupported));
        assert!(
            applied(&wayland)
                .contains(&Call::Fullscreen(FullscreenPlan::Borderless { monitor: 0 }))
        );

        let missing_mode = plan_display_mode(
            &DisplayMode::ExclusiveFullscreen {
                monitor: MonitorChoice::Primary,
                video_mode: VideoModeChoice::Exact(video(1280, 720, 60_000, 32)),
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            missing_mode.fullscreen,
            FullscreenPlan::Borderless { monitor: 0 }
        );
        assert_eq!(missing_mode.fallback, Some(Fallback::VideoModeMissing));

        let missing_monitor = plan_display_mode(
            &DisplayMode::BorderlessFullscreen {
                monitor: MonitorChoice::Name("gone".into()),
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(
            missing_monitor.fullscreen,
            FullscreenPlan::Borderless { monitor: 1 }
        );
        assert_eq!(missing_monitor.fallback, Some(Fallback::MonitorMissing));

        let missing_index = plan_display_mode(
            &DisplayMode::ExclusiveFullscreen {
                monitor: MonitorChoice::Index(9),
                video_mode: VideoModeChoice::Exact(video(1920, 1080, 60_000, 32)),
            },
            X11,
            &monitors,
        )
        .unwrap();
        assert_eq!(missing_index.monitor, Some(1));
        assert_eq!(missing_index.fallback, Some(Fallback::MonitorMissing));
        assert!(matches!(
            missing_index.fullscreen,
            FullscreenPlan::Exclusive { monitor: 1, .. }
        ));

        assert!(matches!(
            plan_display_mode(
                &DisplayMode::BorderlessFullscreen {
                    monitor: MonitorChoice::Current,
                },
                X11,
                &Monitors {
                    list: Vec::new(),
                    primary: None,
                    current: None,
                },
            ),
            Err(PlanError::NoMonitor)
        ));
    }

    #[test]
    fn present_mode_follows_the_preference_and_falls_back_to_fifo() {
        let fifo = PresentPreference::default();
        assert_eq!(fifo.modes, vec![wgpu::PresentMode::Fifo]);
        assert_eq!(fifo.desired_maximum_frame_latency, 2);
        let game = PresentPreference::game();
        assert_eq!(
            game.modes,
            vec![
                wgpu::PresentMode::Mailbox,
                wgpu::PresentMode::FifoRelaxed,
                wgpu::PresentMode::Fifo,
            ]
        );
        let available = [
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Immediate,
            wgpu::PresentMode::FifoRelaxed,
        ];
        let choice = choose_present(&game, &available);
        assert_eq!(choice.selected, wgpu::PresentMode::FifoRelaxed);
        assert!(!choice.fell_back);
        assert_eq!(choice.desired_maximum_frame_latency, 2);

        let mailbox_only = PresentPreference {
            modes: vec![wgpu::PresentMode::Mailbox],
            desired_maximum_frame_latency: 0,
        };
        let fell = choose_present(&mailbox_only, &[wgpu::PresentMode::Fifo]);
        assert_eq!(fell.selected, wgpu::PresentMode::Fifo);
        assert!(fell.fell_back);
        assert_eq!(fell.desired_maximum_frame_latency, 1);

        let empty = choose_present(&mailbox_only, &[]);
        assert_eq!(empty.selected, wgpu::PresentMode::Fifo);
        assert!(empty.fell_back);

        let asked = choose_present(
            &PresentPreference::fifo(),
            &[wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo],
        );
        assert_eq!(asked.selected, wgpu::PresentMode::Fifo);
        assert!(!asked.fell_back);

        assert_eq!(
            Host::Windows.vrr_present_modes(),
            PresentPreference::game().modes.as_slice()
        );
        assert_eq!(
            Host::Wayland.vrr_present_modes(),
            &[wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo]
        );
        assert_eq!(
            Host::X11.vrr_present_modes(),
            &[
                wgpu::PresentMode::Mailbox,
                wgpu::PresentMode::Fifo,
                wgpu::PresentMode::FifoRelaxed,
            ]
        );
        assert_eq!(
            Host::Macos.vrr_present_modes(),
            &[wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo]
        );
        assert_eq!(Host::Other.vrr_present_modes(), &[wgpu::PresentMode::Fifo]);
        assert!(Host::X11.supports_exclusive());
        assert!(Host::Windows.supports_exclusive());
        assert!(Host::Macos.supports_exclusive());
        assert!(!Host::Wayland.supports_exclusive());
        assert!(!Host::Other.supports_exclusive());
        assert_eq!(
            PresentPreference::vrr(Host::Wayland).desired_maximum_frame_latency,
            1
        );
    }

    fn affine(sx: f32, sy: f32, tx: f32, ty: f32) -> [f32; 16] {
        [
            sx, 0.0, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, tx, ty, 0.0, 1.0,
        ]
    }

    fn assert_round_trip(letter: &Letterbox, x: f32, y: f32) {
        let [px, py] = letter.map(x, y);
        let [lx, ly] = letter.unmap(px, py);
        assert!((lx - x).abs() < 1e-3 && (ly - y).abs() < 1e-3);
    }

    #[test]
    fn nearest_rounding_keeps_one_scale_and_rounds_the_offset() {
        let layout = LAYOUT_UNITS;
        for target in [
            size(3840, 2160),
            size(1920, 1080),
            size(2520, 1080),
            size(1440, 1080),
        ] {
            assert_eq!(
                letterbox_with(layout, target, LetterboxRounding::Nearest),
                letterbox(layout, target)
            );
        }
        let deck = letterbox_with(layout, size(1280, 800), LetterboxRounding::Nearest).unwrap();
        assert_eq!(deck.map(0.0, 0.0), [0.0, 40.0]);
        assert_eq!(deck.matrix[0], deck.matrix[5]);
        assert_round_trip(&deck, 1919.0, 1079.0);
        let odd =
            letterbox_with(size(100, 100), size(201, 100), LetterboxRounding::Nearest).unwrap();
        assert_eq!(odd.visible.x, 51.0);
        assert_eq!(
            letterbox(size(100, 100), size(201, 100)).unwrap().visible.x,
            50.0
        );
        let laptop = letterbox_with(layout, size(1366, 768), LetterboxRounding::Nearest).unwrap();
        assert_eq!(laptop.matrix[0], laptop.matrix[5]);
        assert_eq!(laptop.map(0.0, 0.0), [0.0, 0.0]);
        let fractional = letterbox_nearest(1920.5, 1080.0, size(1280, 800)).unwrap();
        assert!(fractional.map(1920.5, 1080.0)[0] <= 1280.0);
        assert!(letterbox_nearest(0.0, 1.0, size(1, 1)).is_none());
        assert!(letterbox_nearest(f32::NAN, 1.0, size(1, 1)).is_none());
        let window = size(2560, 1600);
        let view = viewport_with(
            window,
            RenderScale::Native,
            layout,
            LetterboxRounding::Nearest,
        )
        .unwrap();
        let [x, y] = view.letterbox.map(960.0, 540.0);
        let back = view.pointer_to_layout(
            x * 2560.0 / view.render.width as f32,
            y * 1600.0 / view.render.height as f32,
        );
        assert!((back[0] - 960.0).abs() < 1e-3 && (back[1] - 540.0).abs() < 1e-3);
    }

    #[test]
    fn letterbox_keeps_16_by_9_inside_16_by_9_21_by_9_and_4_by_3() {
        let layout = LAYOUT_UNITS;
        let same = letterbox(layout, size(1920, 1080)).unwrap();
        assert_eq!(same.matrix, affine(1.0, 1.0, 0.0, 0.0));
        assert_eq!(same.inverse, affine(1.0, 1.0, 0.0, 0.0));
        assert_eq!(
            same.visible,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            }
        );
        assert_eq!(same.map(960.0, 540.0), [960.0, 540.0]);
        assert!(same.contains(0.0, 0.0));
        assert!(!same.contains(1920.0, 0.0));

        let wide = letterbox(layout, size(2520, 1080)).unwrap();
        assert_eq!(wide.matrix, affine(1.0, 1.0, 300.0, 0.0));
        assert_eq!(wide.inverse, affine(1.0, 1.0, -300.0, 0.0));
        assert_eq!(wide.map(0.0, 0.0), [300.0, 0.0]);
        assert_eq!(wide.map(960.0, 540.0), [1260.0, 540.0]);
        assert!(!wide.contains(10.0, 10.0));
        assert!(wide.contains(300.0, 0.0));
        assert!(wide.contains(2219.0, 1079.0));
        assert!(!wide.contains(2220.0, 0.0));
        assert_round_trip(&wide, 1.0, 1.0);
        assert_round_trip(&wide, 1919.0, 1079.0);

        let tall = letterbox(layout, size(1440, 1080)).unwrap();
        assert_eq!(tall.matrix, affine(0.75, 0.75, 0.0, 135.0));
        assert_eq!(tall.inverse, affine(1.0 / 0.75, 1.0 / 0.75, 0.0, -180.0));
        assert_eq!(tall.map(960.0, 540.0), [720.0, 540.0]);
        assert_eq!(tall.unmap(720.0, 540.0), [960.0, 540.0]);
        assert!(!tall.contains(10.0, 10.0));
        assert!(tall.contains(0.0, 135.0));
        assert!(!tall.contains(0.0, 135.0 + 810.0));
        assert_round_trip(&tall, 0.0, 0.0);
        assert_round_trip(&tall, 1920.0, 1080.0);

        let odd = letterbox(size(100, 100), size(201, 100)).unwrap();
        assert_eq!(odd.visible.x, 50.0);
        assert_eq!(odd.visible.width, 100.0);
        assert_eq!(odd.visible.y, 0.0);
        assert!(letterbox(size(0, 10), size(10, 10)).is_none());
    }

    #[test]
    fn render_scale_is_independent_of_the_window_and_the_pointer_maps_back() {
        assert_eq!(
            render_size(size(1920, 1080), RenderScale::Native),
            Some(size(1920, 1080))
        );
        assert_eq!(
            render_size(size(1920, 1080), RenderScale::Factor(0.5)),
            Some(size(960, 540))
        );
        assert_eq!(
            render_size(size(1919, 1079), RenderScale::Factor(0.5)),
            Some(size(960, 540))
        );
        assert_eq!(
            render_size(size(800, 600), RenderScale::Size(size(320, 180))),
            Some(size(320, 180))
        );
        assert!(render_size(size(1920, 1080), RenderScale::Factor(0.0)).is_none());
        assert!(render_size(size(1920, 1080), RenderScale::Factor(-1.0)).is_none());
        assert!(render_size(size(1920, 1080), RenderScale::Factor(f32::NAN)).is_none());
        assert!(render_size(size(0, 10), RenderScale::Native).is_none());

        let view = viewport(size(2520, 1080), RenderScale::Factor(0.5), LAYOUT_UNITS).unwrap();
        assert_eq!(view.render, size(1260, 540));
        assert_eq!(view.pointer_to_layout(1260.0, 540.0), [960.0, 540.0]);
        assert!(view.pointer_inside(300.0, 0.0));
        assert!(!view.pointer_inside(299.0, 0.0));
        let [x, y] = view.pointer_to_layout(300.0, 0.0);
        assert!((x - 0.0).abs() < 1e-3 && (y - 0.0).abs() < 1e-3);
    }

    fn take(pacer: &mut FramePacer, clock: &mut FakeClock) -> u64 {
        for _ in 0..4 {
            match pacer.poll(clock) {
                Pace::Start { .. } => return clock.now_ns,
                Pace::Wait { until_ns } => clock.now_ns = until_ns,
                Pace::Paused => panic!("paused"),
            }
        }
        panic!("no start");
    }

    #[test]
    fn the_absolute_grid_does_not_drift() {
        assert_eq!(slot(0, 0, 60), 0);
        assert_eq!(slot(0, 1, 60), NS_PER_SEC / 60);
        assert_eq!(slot(0, 60, 60), NS_PER_SEC);
        assert_eq!(first_slot_after(0, 0, 60), 1);
        assert_eq!(first_slot_after(0, NS_PER_SEC / 60, 60), 2);
        assert_eq!(first_slot_after(0, 10 * NS_PER_SEC / 60, 60), 11);
        assert_eq!(first_slot_after(0, 10 * NS_PER_SEC / 60 + 10, 60), 11);
        assert_eq!(first_slot_after(0, NS_PER_SEC, 30), 31);
    }

    #[test]
    fn a_thousand_frames_at_each_cap_stay_within_one_tick() {
        for cap in [
            FrameCap::Fps30,
            FrameCap::Fps60,
            FrameCap::Fps90,
            FrameCap::Fps120,
        ] {
            let fps = u64::from(cap.fps().unwrap());
            let mut pacer = FramePacer::new(FrameCaps::new(cap, cap));
            let mut clock = FakeClock::default();
            for frame in 0..1_000 {
                let start = take(&mut pacer, &mut clock);
                let expected = frame * NS_PER_SEC / fps;
                assert!(
                    start.abs_diff(expected) <= 1,
                    "{cap:?} frame {frame} at {start}, grid {expected}"
                );
            }
        }
    }

    #[test]
    fn uncapped_never_waits_and_a_late_frame_returns_to_the_grid() {
        let mut pacer = FramePacer::new(FrameCaps::new(FrameCap::Uncapped, FrameCap::Fps30));
        let mut clock = FakeClock::default();
        for frame in 0..1_000 {
            clock.now_ns = frame * 123_456 + (frame % 7) * 89;
            assert!(matches!(pacer.poll(&clock), Pace::Start { next_ns: None }));
        }

        let mut pacer = FramePacer::new(FrameCaps::new(FrameCap::Fps60, FrameCap::Fps60));
        let mut clock = FakeClock::default();
        assert!(matches!(pacer.poll(&clock), Pace::Start { .. }));
        clock.now_ns = 10 * NS_PER_SEC / 60 + 10;
        match pacer.poll(&clock) {
            Pace::Start {
                next_ns: Some(next),
            } => {
                assert_eq!(next, 11 * NS_PER_SEC / 60);
            }
            other => panic!("late frame should start once, got {other:?}"),
        }
        assert!(matches!(
            pacer.poll(&clock),
            Pace::Wait { until_ns } if until_ns == 11 * NS_PER_SEC / 60
        ));
        for frame in 11..40 {
            let start = take(&mut pacer, &mut clock);
            assert_eq!(start, frame * NS_PER_SEC / 60);
        }
    }

    #[test]
    fn focus_occlusion_and_minimize_swap_the_schedule() {
        let mut pacer = FramePacer::new(FrameCaps::new(FrameCap::Fps60, FrameCap::Fps30));
        let mut clock = FakeClock::default();
        assert_eq!(take(&mut pacer, &mut clock), 0);
        let second = take(&mut pacer, &mut clock);
        assert_eq!(second, NS_PER_SEC / 60);

        pacer.on(Attention::Occluded(true));
        assert_eq!(pacer.effective_fps(), Some(30));
        let origin = second;
        assert_eq!(take(&mut pacer, &mut clock), origin);
        let background = take(&mut pacer, &mut clock);
        assert_eq!(background, origin + NS_PER_SEC / 30);
        assert!(matches!(pacer.poll(&clock), Pace::Wait { .. }));

        pacer.on(Attention::Focused(false));
        let held = take(&mut pacer, &mut clock);
        assert_eq!(held, origin + 2 * NS_PER_SEC / 30);

        pacer.on(Attention::Focused(true));
        let still = take(&mut pacer, &mut clock);
        assert_eq!(still, origin + 3 * NS_PER_SEC / 30);

        pacer.on(Attention::Occluded(false));
        assert_eq!(pacer.effective_fps(), Some(60));
        assert_eq!(take(&mut pacer, &mut clock), still);
        assert_eq!(take(&mut pacer, &mut clock), still + NS_PER_SEC / 60);

        pacer.on(Attention::Minimized(true));
        assert!(matches!(pacer.poll(&clock), Pace::Paused));
        clock.now_ns += 5 * NS_PER_SEC;
        assert!(matches!(pacer.poll(&clock), Pace::Paused));
        pacer.on(Attention::Minimized(false));
        assert!(matches!(pacer.poll(&clock), Pace::Start { .. }));
        assert!(matches!(pacer.poll(&clock), Pace::Wait { .. }));

        let mut caps = FrameCaps::new(FrameCap::Fps60, FrameCap::Fps30);
        caps.pause_when_minimized = false;
        let mut running = FramePacer::new(caps);
        running.on(Attention::Minimized(true));
        assert_eq!(running.effective_fps(), Some(60));
        assert!(matches!(running.poll(&clock), Pace::Start { .. }));
    }

    #[test]
    fn vrr_caps_at_refresh_minus_three_and_the_pacer_uses_it() {
        let cases = [
            (60_000, FrameCap::Fps60, Some(57)),
            (60_000, FrameCap::Fps30, Some(30)),
            (120_000, FrameCap::Fps120, Some(117)),
            (144_000, FrameCap::Fps120, Some(120)),
            (144_000, FrameCap::Uncapped, Some(141)),
            (165_000, FrameCap::Fps90, Some(90)),
            (165_000, FrameCap::Fps120, Some(120)),
            (165_000, FrameCap::Uncapped, Some(162)),
        ];
        for (refresh, cap, expected) in cases {
            assert_eq!(paced_fps(cap, Some(refresh)), expected, "{refresh} {cap:?}");
        }
        assert_eq!(vrr_limit_fps(60_000), Some(57));
        assert_eq!(vrr_limit_fps(120_000), Some(117));
        assert_eq!(vrr_limit_fps(144_000), Some(141));
        assert_eq!(vrr_limit_fps(165_000), Some(162));
        assert_eq!(paced_fps(FrameCap::Fps60, None), Some(60));
        assert_eq!(paced_fps(FrameCap::Uncapped, None), None);
        assert_eq!(vrr_limit_fps(2_000), None);

        let mut pacer = FramePacer::new(FrameCaps::new(FrameCap::Fps60, FrameCap::Fps30));
        pacer.set_vrr(Some(60_000));
        assert_eq!(pacer.effective_fps(), Some(57));
        let mut clock = FakeClock::default();
        for frame in 0..1_000 {
            let start = take(&mut pacer, &mut clock);
            assert!(start.abs_diff(frame * NS_PER_SEC / 57) <= 1);
        }
        pacer.set_vrr(None);
        assert_eq!(pacer.effective_fps(), Some(60));
        let switched = clock.now_ns;
        assert_eq!(take(&mut pacer, &mut clock), switched);
        assert_eq!(take(&mut pacer, &mut clock), switched + NS_PER_SEC / 60);
    }

    #[test]
    fn winit_focus_and_occlusion_events_drive_the_pacer() {
        let mut pacer = FramePacer::new(FrameCaps::new(FrameCap::Fps120, FrameCap::Fps30));
        let blurred = winit::event::WindowEvent::Focused(false);
        pacer.on(attention(&blurred).unwrap());
        assert_eq!(pacer.effective_fps(), Some(30));
        pacer.on(Attention::Focused(true));
        let covered = winit::event::WindowEvent::Occluded(true);
        pacer.on(attention(&covered).unwrap());
        assert_eq!(pacer.effective_fps(), Some(30));
        assert!(attention(&winit::event::WindowEvent::RedrawRequested).is_none());
        assert!(attention(&winit::event::WindowEvent::CloseRequested).is_none());
    }
}
