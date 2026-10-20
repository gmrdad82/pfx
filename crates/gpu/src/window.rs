use std::fmt;

use serde::{Deserialize, Serialize};

const NS_PER_SEC: u64 = 1_000_000_000;
const FIFO_LATENCY: u32 = 2;
const QUEUE_LATENCY: u32 = 1;
const REFRESH_SLACK_MHZ: u64 = 600;
const AUTO_TOP_FPS: u32 = 120;
const AUTO_FLOOR_FPS: f64 = 29.5;
const AUTO_LADDER: [u32; 10] = [120, 100, 90, 80, 72, 60, 50, 45, 40, 30];
const AUTO_WINDOW: usize = 30;
const AUTO_RISE_WINDOWS: u32 = 8;
const AUTO_SETTLED_WINDOWS: u32 = 16;
const AUTO_MAX_BACKOFF: u32 = 5;
const AUTO_IDLE_NS: u64 = 250_000_000;

pub const PRESENT_VAR: &str = "PFX_PRESENT";
pub const LATENCY_VAR: &str = "PFX_FRAME_LATENCY";

const FIFO_ONLY: [wgpu::PresentMode; 1] = [wgpu::PresentMode::Fifo];
const FIXED_REFRESH: [wgpu::PresentMode; 3] = [
    wgpu::PresentMode::Mailbox,
    wgpu::PresentMode::FifoRelaxed,
    wgpu::PresentMode::Fifo,
];
const LINUX_VRR: [wgpu::PresentMode; 2] = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo];
const WINDOWS_FULLSCREEN_VRR: [wgpu::PresentMode; 2] =
    [wgpu::PresentMode::Immediate, wgpu::PresentMode::Fifo];

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

    pub fn vrr_present_modes(self, fullscreen: bool) -> &'static [wgpu::PresentMode] {
        match self {
            Host::Windows if fullscreen => &WINDOWS_FULLSCREEN_VRR,
            Host::X11 | Host::Wayland => &LINUX_VRR,
            Host::Windows | Host::Macos | Host::Other => &FIFO_ONLY,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Host::Windows => "windows",
            Host::Macos => "macos",
            Host::X11 => "x11",
            Host::Wayland => "wayland",
            Host::Other => "other",
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
    Borderless { monitor: Option<usize> },
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
) -> Result<(Option<usize>, Option<Fallback>), PlanError> {
    if monitors.list.is_empty() {
        return Err(PlanError::NoMonitor);
    }
    let found = match choice {
        MonitorChoice::Current => return Ok((None, None)),
        MonitorChoice::Primary => monitors.primary,
        MonitorChoice::Index(index) => (*index < monitors.list.len()).then_some(*index),
        MonitorChoice::Name(name) => monitors
            .list
            .iter()
            .position(|monitor| monitor.name.as_deref() == Some(name.as_str())),
    };
    Ok(match found {
        Some(index) => (Some(index), None),
        None => (None, Some(Fallback::MonitorMissing)),
    })
}

fn borderless_plan(
    target: Option<usize>,
    fallback: Option<Fallback>,
    monitors: &Monitors,
) -> ModePlan {
    ModePlan {
        fullscreen: FullscreenPlan::Borderless { monitor: target },
        decorations: false,
        resizable: false,
        inner_size: None,
        outer_position: None,
        fallback,
        monitor: target.or(monitors.current),
    }
}

fn borderless(
    choice: &MonitorChoice,
    monitors: &Monitors,
    fallback: Option<Fallback>,
) -> Result<ModePlan, PlanError> {
    let (target, missed) = resolve(choice, monitors)?;
    Ok(borderless_plan(target, fallback.or(missed), monitors))
}

fn exclusive(
    choice: &MonitorChoice,
    video_mode: &VideoModeChoice,
    platform: PlatformCaps,
    monitors: &Monitors,
) -> Result<ModePlan, PlanError> {
    let (target, missed) = resolve(choice, monitors)?;
    if !platform.exclusive {
        return Ok(borderless_plan(
            target,
            missed.or(Some(Fallback::ExclusiveUnsupported)),
            monitors,
        ));
    }
    let Some(monitor) = target.or(monitors.current) else {
        return Ok(borderless_plan(
            None,
            missed.or(Some(Fallback::MonitorMissing)),
            monitors,
        ));
    };
    if let Some(mode) = monitors
        .list
        .get(monitor)
        .and_then(|found| find_video_mode(found, video_mode))
    {
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
        target,
        missed.or(Some(Fallback::VideoModeMissing)),
        monitors,
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
}

impl Default for PresentPreference {
    fn default() -> Self {
        Self::fifo()
    }
}

impl PresentPreference {
    pub fn fifo() -> Self {
        Self {
            modes: FIFO_ONLY.to_vec(),
        }
    }

    pub fn of(present: Present, host: Host, fullscreen: bool) -> Self {
        if !present.vrr {
            return Self::fifo();
        }
        Self {
            modes: host.vrr_present_modes(fullscreen).to_vec(),
        }
    }

    pub fn forced(mode: wgpu::PresentMode) -> Self {
        Self { modes: vec![mode] }
    }

    pub fn fixed_refresh() -> Self {
        Self {
            modes: FIXED_REFRESH.to_vec(),
        }
    }
}

pub fn present_mode_named(name: &str) -> Option<wgpu::PresentMode> {
    match name.trim().to_ascii_lowercase().as_str() {
        "fifo" => Some(wgpu::PresentMode::Fifo),
        "fifo-relaxed" => Some(wgpu::PresentMode::FifoRelaxed),
        "mailbox" => Some(wgpu::PresentMode::Mailbox),
        "immediate" => Some(wgpu::PresentMode::Immediate),
        _ => None,
    }
}

pub fn latency_named(name: &str) -> Option<u32> {
    name.trim()
        .parse()
        .ok()
        .filter(|latency| (1..=3).contains(latency))
}

pub fn frame_latency(mode: wgpu::PresentMode) -> u32 {
    match mode {
        wgpu::PresentMode::Mailbox | wgpu::PresentMode::Immediate => QUEUE_LATENCY,
        _ => FIFO_LATENCY,
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
        FIFO_ONLY.to_vec()
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
        desired_maximum_frame_latency: frame_latency(selected),
    }
}

pub fn setting(
    host: Host,
    choice: &PresentChoice,
    present: Present,
    vrr_in_effect: Option<bool>,
) -> crate::presents::Setting {
    crate::presents::Setting {
        host: host.name(),
        present_mode: format!("{:?}", choice.selected),
        frame_latency: choice.desired_maximum_frame_latency,
        vrr: present.vrr,
        vrr_in_effect,
        frame_rate: present.frame_rate.name(),
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameRate {
    #[serde(rename = "cap60")]
    Cap60,
    #[serde(rename = "cap120")]
    Cap120,
    #[default]
    #[serde(rename = "uncapped")]
    Uncapped,
    #[serde(rename = "auto")]
    Auto,
}

impl FrameRate {
    pub fn name(self) -> &'static str {
        match self {
            FrameRate::Cap60 => "cap60",
            FrameRate::Cap120 => "cap120",
            FrameRate::Uncapped => "uncapped",
            FrameRate::Auto => "auto",
        }
    }

    pub fn fps(self) -> Option<u32> {
        match self {
            FrameRate::Cap60 => Some(60),
            FrameRate::Cap120 => Some(120),
            FrameRate::Auto => Some(AUTO_TOP_FPS),
            FrameRate::Uncapped => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Present {
    pub vrr: bool,
    pub frame_rate: FrameRate,
}

impl Default for Present {
    fn default() -> Self {
        Self {
            vrr: true,
            frame_rate: FrameRate::Uncapped,
        }
    }
}

impl Present {
    pub fn preference(self, host: Host, fullscreen: bool) -> PresentPreference {
        PresentPreference::of(self, host, fullscreen)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub ns: u64,
    pub per: u64,
}

impl Period {
    pub fn fps(self) -> f64 {
        self.per as f64 * NS_PER_SEC as f64 / self.ns as f64
    }

    pub fn ms(self) -> f64 {
        self.ns as f64 / self.per as f64 / 1e6
    }
}

pub fn refresh_vblanks(refresh_millihertz: u32, fps: u32) -> u64 {
    (u64::from(refresh_millihertz) + REFRESH_SLACK_MHZ) / (u64::from(fps).max(1) * 1_000)
}

pub fn frame_period(present: Present, refresh_millihertz: Option<u32>) -> Option<Period> {
    let fps = present.frame_rate.fps()?;
    let locked = refresh_millihertz
        .filter(|refresh| !present.vrr && *refresh > 0)
        .map(|refresh| Period {
            ns: refresh_vblanks(refresh, fps).max(1) * NS_PER_SEC * 1_000,
            per: u64::from(refresh),
        });
    Some(locked.unwrap_or(Period {
        ns: NS_PER_SEC,
        per: u64::from(fps),
    }))
}

pub fn auto_period(vrr: bool, refresh_millihertz: Option<u32>, rung: usize) -> Option<Period> {
    match refresh_millihertz.filter(|refresh| !vrr && *refresh > 0) {
        Some(refresh) => {
            let vblanks = refresh_vblanks(refresh, AUTO_TOP_FPS).max(1)
                + u64::try_from(rung).unwrap_or(u64::MAX);
            let period = Period {
                ns: vblanks.saturating_mul(NS_PER_SEC * 1_000),
                per: u64::from(refresh),
            };
            (rung == 0 || period.fps() >= AUTO_FLOOR_FPS).then_some(period)
        }
        None => AUTO_LADDER.get(rung).map(|fps| Period {
            ns: NS_PER_SEC,
            per: u64::from(*fps),
        }),
    }
}

fn frame_ns(period: Period) -> u64 {
    period.ns / period.per.max(1)
}

#[derive(Clone, Debug, Default)]
struct Auto {
    rung: usize,
    samples: Vec<u64>,
    started_ns: Option<u64>,
    calm: u32,
    held: u32,
    rose: bool,
    failures: u32,
}

impl Auto {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn polled(&mut self, now_ns: u64, vrr: bool, refresh: Option<u32>) {
        let Some(started) = self.started_ns.take() else {
            return;
        };
        let busy = now_ns.saturating_sub(started);
        if busy > AUTO_IDLE_NS {
            return;
        }
        self.samples.push(busy);
        if self.samples.len() >= AUTO_WINDOW {
            self.judge(vrr, refresh);
            self.samples.clear();
        }
    }

    fn judge(&mut self, vrr: bool, refresh: Option<u32>) {
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        let p90 = sorted[(sorted.len() * 9).div_ceil(10) - 1];
        let at = |rung: usize| auto_period(vrr, refresh, rung).map(frame_ns);
        let Some(current) = at(self.rung) else {
            self.reset();
            return;
        };
        self.held = self.held.saturating_add(1);
        if p90 > current {
            let need = p90.saturating_mul(11) / 10;
            let mut rung = self.rung;
            while let Some(next) = at(rung + 1) {
                rung += 1;
                if next >= need {
                    break;
                }
            }
            if rung != self.rung {
                if self.rose && self.held < AUTO_SETTLED_WINDOWS {
                    self.failures = (self.failures + 1).min(AUTO_MAX_BACKOFF);
                }
                self.rung = rung;
                self.held = 0;
                self.rose = false;
            }
            self.calm = 0;
            return;
        }
        if self.rose && self.held >= AUTO_SETTLED_WINDOWS {
            self.failures = 0;
            self.rose = false;
        }
        let above = self.rung.checked_sub(1).and_then(at);
        match above {
            Some(above) if p90.saturating_mul(13) / 10 <= above => {
                self.calm += 1;
                if self.calm >= AUTO_RISE_WINDOWS << self.failures {
                    self.rung -= 1;
                    self.calm = 0;
                    self.held = 0;
                    self.rose = true;
                }
            }
            _ => self.calm = 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Schedule {
    Open,
    Hold,
    Grid {
        origin_ns: u64,
        index: u64,
        period: Period,
    },
}

#[derive(Clone, Debug)]
pub struct FramePacer {
    present: Present,
    refresh_millihertz: Option<u32>,
    pause_when_minimized: bool,
    minimized: bool,
    schedule: Schedule,
    auto: Auto,
    vrr_in_effect: Option<bool>,
}

impl FramePacer {
    pub fn new(present: Present) -> Self {
        Self {
            present,
            refresh_millihertz: None,
            pause_when_minimized: true,
            minimized: false,
            schedule: Schedule::Open,
            auto: Auto::default(),
            vrr_in_effect: None,
        }
    }

    pub fn present(&self) -> Present {
        self.present
    }

    pub fn set_present(&mut self, present: Present) {
        if present != self.present {
            self.auto.reset();
        }
        self.present = present;
    }

    pub fn set_refresh(&mut self, refresh_millihertz: Option<u32>) {
        if refresh_millihertz != self.refresh_millihertz {
            self.auto.reset();
        }
        self.refresh_millihertz = refresh_millihertz;
    }

    pub fn set_vrr_in_effect(&mut self, in_effect: Option<bool>) {
        if in_effect != self.vrr_in_effect {
            self.auto.reset();
        }
        self.vrr_in_effect = in_effect;
    }

    pub fn paced(&self) -> Present {
        Present {
            vrr: self.present.vrr && self.vrr_in_effect != Some(false),
            ..self.present
        }
    }

    pub fn set_pause_when_minimized(&mut self, pause: bool) {
        self.pause_when_minimized = pause;
    }

    pub fn on(&mut self, event: Attention) {
        if let Attention::Minimized(minimized) = event {
            self.minimized = minimized;
        }
    }

    pub fn period(&self) -> Option<Period> {
        if self.present.frame_rate == FrameRate::Auto {
            return auto_period(self.paced().vrr, self.refresh_millihertz, self.auto.rung);
        }
        frame_period(self.paced(), self.refresh_millihertz)
    }

    pub fn poll(&mut self, clock: &impl Clock) -> Pace {
        if self.pause_when_minimized && self.minimized {
            self.schedule = Schedule::Hold;
            self.auto.started_ns = None;
            return Pace::Paused;
        }
        match self.period() {
            None => {
                self.schedule = Schedule::Open;
                Pace::Start { next_ns: None }
            }
            Some(_) => {
                let now = clock.now_ns();
                if self.present.frame_rate == FrameRate::Auto {
                    self.auto
                        .polled(now, self.paced().vrr, self.refresh_millihertz);
                }
                let Some(period) = self.period() else {
                    return Pace::Start { next_ns: None };
                };
                let pace = self.poll_grid(now, period);
                if matches!(pace, Pace::Start { .. }) {
                    self.auto.started_ns = Some(now);
                }
                pace
            }
        }
    }

    pub fn auto_rung(&self) -> usize {
        self.auto.rung
    }

    fn poll_grid(&mut self, now: u64, period: Period) -> Pace {
        let (origin_ns, index) = match self.schedule {
            Schedule::Grid {
                origin_ns,
                index,
                period: scheduled,
            } if scheduled == period => (origin_ns, index),
            _ => (now, 0),
        };
        let due = slot(origin_ns, index, period);
        if now < due {
            return Pace::Wait { until_ns: due };
        }
        let next = first_slot_after(origin_ns, now, period).max(index.saturating_add(1));
        self.schedule = Schedule::Grid {
            origin_ns,
            index: next,
            period,
        };
        Pace::Start {
            next_ns: Some(slot(origin_ns, next, period)),
        }
    }
}

fn slot(origin_ns: u64, index: u64, period: Period) -> u64 {
    debug_assert!(period.per > 0);
    let step = u128::from(index) * u128::from(period.ns) / u128::from(period.per);
    origin_ns.saturating_add(u64::try_from(step).unwrap_or(u64::MAX))
}

fn first_slot_after(origin_ns: u64, now_ns: u64, period: Period) -> u64 {
    debug_assert!(period.ns > 0);
    if now_ns <= origin_ns {
        return 1;
    }
    let need = u128::from(now_ns - origin_ns) + 1;
    let index = need
        .saturating_mul(u128::from(period.per))
        .div_ceil(u128::from(period.ns));
    u64::try_from(index).unwrap_or(u64::MAX).max(1)
}

#[cfg(any(feature = "window", test))]
mod glue {
    use super::{
        Attention, DisplayMode, Fallback, FullscreenPlan, Host, LATENCY_VAR, ModePlan, Monitor,
        Monitors, PRESENT_VAR, PlanError, PlatformCaps, Point, Present, PresentChoice,
        PresentPreference, Size, VideoMode, WindowOps, apply_plan, latency_named,
        plan_display_mode, present_mode_named,
    };
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::monitor::MonitorHandle;
    use winit::window::{Fullscreen, Window};

    pub fn refresh_millihertz(monitor: &MonitorHandle) -> Option<u32> {
        monitor.refresh_rate_millihertz()
    }

    pub fn fullscreen(window: &Window) -> bool {
        window.fullscreen().is_some()
    }

    pub fn present_forced() -> bool {
        std::env::var(PRESENT_VAR)
            .ok()
            .as_deref()
            .and_then(present_mode_named)
            .is_some()
    }

    pub fn present_preference(window: &Window, present: Present) -> PresentPreference {
        match std::env::var(PRESENT_VAR)
            .ok()
            .as_deref()
            .and_then(present_mode_named)
        {
            Some(mode) => PresentPreference::forced(mode),
            None => present.preference(host(window), fullscreen(window)),
        }
    }

    pub fn latency_override() -> Option<u32> {
        std::env::var(LATENCY_VAR)
            .ok()
            .as_deref()
            .and_then(latency_named)
    }

    pub fn overridden(mut choice: PresentChoice) -> PresentChoice {
        if let Some(latency) = latency_override() {
            choice.desired_maximum_frame_latency = latency;
        }
        choice
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
                    let handle = monitor.and_then(|monitor| self.handles.get(monitor).cloned());
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
                plan.fullscreen = FullscreenPlan::Borderless {
                    monitor: Some(monitor),
                };
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
    apply_display_mode, attention, fullscreen, host, latency_override, minimized, monitors,
    overridden, placement, present_forced, present_preference, refresh_millihertz,
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
            FullscreenPlan::Borderless { monitor: Some(0) }
        );
        assert_eq!(borderless.fallback, None);
        assert_eq!(
            applied(&borderless),
            vec![
                Call::Decorations(false),
                Call::Fullscreen(FullscreenPlan::Borderless { monitor: Some(0) }),
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
            FullscreenPlan::Borderless { monitor: Some(0) }
        );
        assert_eq!(wayland.fallback, Some(Fallback::ExclusiveUnsupported));
        assert!(
            applied(&wayland).contains(&Call::Fullscreen(FullscreenPlan::Borderless {
                monitor: Some(0)
            }))
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
            FullscreenPlan::Borderless { monitor: Some(0) }
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
            FullscreenPlan::Borderless { monitor: None }
        );
        assert_eq!(missing_monitor.monitor, Some(1));
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
    fn the_current_monitor_is_left_to_the_compositor() {
        let unmapped = Monitors {
            current: None,
            primary: None,
            ..desk()
        };
        let current = MonitorChoice::Current;
        let borderless = plan_display_mode(
            &DisplayMode::BorderlessFullscreen {
                monitor: current.clone(),
            },
            WAYLAND,
            &unmapped,
        )
        .unwrap();
        assert_eq!(
            borderless.fullscreen,
            FullscreenPlan::Borderless { monitor: None }
        );
        assert_eq!((borderless.fallback, borderless.monitor), (None, None));
        for platform in [WAYLAND, X11] {
            let exclusive = plan_display_mode(
                &DisplayMode::ExclusiveFullscreen {
                    monitor: current.clone(),
                    video_mode: VideoModeChoice::Native,
                },
                platform,
                &unmapped,
            )
            .unwrap();
            assert_eq!(
                exclusive.fullscreen,
                FullscreenPlan::Borderless { monitor: None }
            );
        }
    }

    #[test]
    fn present_mode_follows_the_vrr_switch_and_falls_back_to_fifo() {
        use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
        let fifo = PresentPreference::default();
        assert_eq!(fifo.modes, vec![Fifo]);
        let off = Present {
            vrr: false,
            ..Present::default()
        };
        for host in [Host::Windows, Host::Macos, Host::X11, Host::Wayland] {
            for fullscreen in [false, true] {
                assert_eq!(off.preference(host, fullscreen), fifo);
            }
        }
        let on = Present::default();
        assert!(on.vrr);
        assert_eq!(on.frame_rate, FrameRate::Uncapped);
        assert_eq!(
            on.preference(Host::Wayland, false).modes,
            vec![Mailbox, Fifo]
        );
        assert_eq!(on.preference(Host::X11, true).modes, vec![Mailbox, Fifo]);
        assert_eq!(on.preference(Host::Windows, false).modes, vec![Fifo]);
        assert_eq!(
            on.preference(Host::Windows, true).modes,
            vec![Immediate, Fifo]
        );
        assert_eq!(on.preference(Host::Macos, true).modes, vec![Fifo]);
        assert_eq!(on.preference(Host::Other, false).modes, vec![Fifo]);

        let mailbox = choose_present(&on.preference(Host::Wayland, false), &[Fifo, Mailbox]);
        assert_eq!(mailbox.selected, Mailbox);
        assert_eq!(mailbox.desired_maximum_frame_latency, 1);
        let fell = choose_present(&on.preference(Host::Wayland, false), &[Fifo, FifoRelaxed]);
        assert_eq!(fell.selected, Fifo);
        assert!(!fell.fell_back);
        assert_eq!(fell.desired_maximum_frame_latency, 2);
        let none = choose_present(&PresentPreference::forced(Immediate), &[]);
        assert_eq!(none.selected, Fifo);
        assert!(none.fell_back);

        assert_eq!(present_mode_named(" Mailbox "), Some(Mailbox));
        assert_eq!(present_mode_named("fifo-relaxed"), Some(FifoRelaxed));
        assert_eq!(present_mode_named("vsync"), None);
        assert!(Host::X11.supports_exclusive());
        assert!(!Host::Wayland.supports_exclusive());
    }

    #[test]
    fn the_two_switches_read_and_write_as_settings() {
        let parsed: Present = toml::from_str("vrr = false\nframe_rate = \"cap60\"\n").unwrap();
        assert_eq!(
            parsed,
            Present {
                vrr: false,
                frame_rate: FrameRate::Cap60
            }
        );
        let empty: Present = toml::from_str("").unwrap();
        assert_eq!(empty, Present::default());
        assert!(toml::from_str::<Present>("frame_rate = \"30\"\n").is_err());
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

    fn cap60(vrr: bool) -> Present {
        Present {
            vrr,
            frame_rate: FrameRate::Cap60,
        }
    }

    fn cap120(vrr: bool) -> Present {
        Present {
            vrr,
            frame_rate: FrameRate::Cap120,
        }
    }

    const MS_NS: u64 = 1_000_000;
    fn auto(vrr: bool) -> Present {
        Present {
            vrr,
            frame_rate: FrameRate::Auto,
        }
    }

    fn run(
        pacer: &mut FramePacer,
        clock: &mut FakeClock,
        frames: usize,
        cost: impl Fn(usize) -> u64,
    ) -> Vec<usize> {
        let mut rungs = Vec::with_capacity(frames);
        for _ in 0..frames {
            take(pacer, clock);
            rungs.push(pacer.auto_rung());
            clock.now_ns += cost(pacer.auto_rung());
        }
        rungs
    }

    #[test]
    fn auto_steps_through_whole_refreshes_with_vrr_off_and_a_ladder_with_it_on() {
        let fps = |vrr, refresh| {
            (0..)
                .map_while(|rung| auto_period(vrr, refresh, rung))
                .map(|period| (period.fps() * 10.0).round() / 10.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            fps(false, Some(239_970)),
            [120.0, 80.0, 60.0, 48.0, 40.0, 34.3, 30.0]
        );
        assert_eq!(fps(false, Some(60_000)), [60.0, 30.0]);
        assert_eq!(
            fps(true, Some(239_970)),
            [120.0, 100.0, 90.0, 80.0, 72.0, 60.0, 50.0, 45.0, 40.0, 30.0]
        );
        assert_eq!(fps(false, None), fps(true, None));
        assert_eq!(FrameRate::Auto.name(), "auto");
        assert_eq!(
            toml::from_str::<Present>("frame_rate = \"auto\"").unwrap(),
            auto(true)
        );
    }

    #[test]
    fn auto_settles_to_a_steady_rate_and_rises_when_the_scene_lightens() {
        let mut clock = FakeClock::default();
        let mut pacer = FramePacer::new(auto(false));
        pacer.set_refresh(Some(239_970));
        let heavy = run(&mut pacer, &mut clock, 60, |_| 10 * MS_NS);
        assert_eq!(heavy[59], 1);
        assert!((pacer.period().unwrap().fps() - 79.99).abs() < 0.01);
        let steady = run(&mut pacer, &mut clock, 900, |_| 11_500_000);
        assert!(steady.iter().all(|rung| *rung == 1));
        let light = run(&mut pacer, &mut clock, 30 * 10, |_| 3 * MS_NS);
        assert_eq!(light.last(), Some(&0));
        let mut vrr = FramePacer::new(auto(true));
        vrr.set_refresh(Some(239_970));
        run(&mut vrr, &mut clock, 60, |_| 9_500_000);
        assert!((vrr.period().unwrap().fps() - 90.0).abs() < 1e-9);
        vrr.set_present(auto(false));
        assert_eq!(vrr.auto_rung(), 0);
    }

    #[test]
    fn auto_backs_off_a_rate_it_failed_to_hold_instead_of_flapping() {
        let mut clock = FakeClock::default();
        let mut pacer = FramePacer::new(auto(false));
        pacer.set_refresh(Some(239_970));
        let rungs = run(&mut pacer, &mut clock, 6_000, |rung| {
            if rung == 0 { 10 * MS_NS } else { 3 * MS_NS }
        });
        let rises = rungs.windows(2).filter(|pair| pair[1] < pair[0]).count();
        assert!((2..=5).contains(&rises), "{rises} rises");
        let top = rungs.iter().filter(|rung| **rung == 0).count();
        assert!(top <= (rises + 1) * 31, "{top} frames at the top");
    }

    #[test]
    fn cap60_locks_to_whole_refreshes_with_vrr_off_and_to_60_with_it_on() {
        let sixty = Period {
            ns: NS_PER_SEC,
            per: 60,
        };
        assert_eq!(frame_period(cap60(true), Some(239_970)), Some(sixty));
        assert_eq!(frame_period(cap60(false), None), Some(sixty));
        assert_eq!(frame_period(Present::default(), Some(239_970)), None);
        for (refresh, vblanks) in [
            (239_970, 4),
            (240_000, 4),
            (144_000, 2),
            (165_000, 2),
            (119_880, 2),
            (117_000, 1),
            (60_000, 1),
            (59_940, 1),
            (50_000, 0),
        ] {
            assert_eq!(refresh_vblanks(refresh, 60), vblanks, "{refresh}");
        }
        let locked = frame_period(cap60(false), Some(239_970)).unwrap();
        assert!((locked.ms() - 4.0 * 1e3 / 239.97).abs() < 1e-9);
        assert!((locked.fps() - 59.9925).abs() < 1e-9);
        let slow = frame_period(cap60(false), Some(50_000)).unwrap();
        assert!((slow.fps() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn cap120_locks_to_whole_refreshes_with_vrr_off_and_to_120_with_it_on() {
        let hundred_twenty = Period {
            ns: NS_PER_SEC,
            per: 120,
        };
        assert_eq!(
            frame_period(cap120(true), Some(239_970)),
            Some(hundred_twenty)
        );
        assert_eq!(frame_period(cap120(false), None), Some(hundred_twenty));
        for (refresh, vblanks) in [
            (239_970, 2),
            (240_000, 2),
            (360_000, 3),
            (165_000, 1),
            (144_000, 1),
            (119_880, 1),
            (60_000, 0),
        ] {
            assert_eq!(refresh_vblanks(refresh, 120), vblanks, "{refresh}");
        }
        let locked = frame_period(cap120(false), Some(239_970)).unwrap();
        assert!((locked.ms() - 2.0 * 1e3 / 239.97).abs() < 1e-9);
        assert!((locked.fps() - 119.985).abs() < 1e-9);
        let one = frame_period(cap120(false), Some(144_000)).unwrap();
        assert!((one.fps() - 144.0).abs() < 1e-9);
        let below = frame_period(cap120(false), Some(60_000)).unwrap();
        assert!((below.fps() - 60.0).abs() < 1e-9);
        let parsed: Present = toml::from_str("frame_rate = \"cap120\"\n").unwrap();
        assert_eq!(parsed, cap120(true));
        assert_eq!(FrameRate::Cap120.name(), "cap120");
    }

    #[test]
    fn the_absolute_grid_does_not_drift() {
        let sixty = Period {
            ns: NS_PER_SEC,
            per: 60,
        };
        assert_eq!(slot(0, 0, sixty), 0);
        assert_eq!(slot(0, 1, sixty), NS_PER_SEC / 60);
        assert_eq!(slot(0, 60, sixty), NS_PER_SEC);
        assert_eq!(first_slot_after(0, 0, sixty), 1);
        assert_eq!(first_slot_after(0, NS_PER_SEC / 60, sixty), 2);
        assert_eq!(first_slot_after(0, 10 * NS_PER_SEC / 60, sixty), 11);
        assert_eq!(first_slot_after(0, 10 * NS_PER_SEC / 60 + 10, sixty), 11);
        for present in [cap60(true), cap60(false), cap120(true), cap120(false)] {
            let mut pacer = FramePacer::new(present);
            pacer.set_refresh(Some(239_970));
            let period = pacer.period().unwrap();
            let mut clock = FakeClock::default();
            for frame in 0..100_000u64 {
                let start = take(&mut pacer, &mut clock);
                let expected = u128::from(frame) * u128::from(period.ns) / u128::from(period.per);
                assert!(u128::from(start).abs_diff(expected) <= 1);
            }
        }
    }

    #[test]
    fn uncapped_never_waits_and_a_late_cap60_frame_returns_to_the_grid() {
        let mut pacer = FramePacer::new(Present::default());
        let mut clock = FakeClock::default();
        for frame in 0..1_000 {
            clock.now_ns = frame * 123_456 + (frame % 7) * 89;
            assert!(matches!(pacer.poll(&clock), Pace::Start { next_ns: None }));
        }

        let mut pacer = FramePacer::new(cap60(true));
        let mut clock = FakeClock::default();
        assert!(matches!(pacer.poll(&clock), Pace::Start { .. }));
        clock.now_ns = 10 * NS_PER_SEC / 60 + 10;
        match pacer.poll(&clock) {
            Pace::Start {
                next_ns: Some(next),
            } => assert_eq!(next, 11 * NS_PER_SEC / 60),
            other => panic!("late frame should start once, got {other:?}"),
        }
        assert!(matches!(
            pacer.poll(&clock),
            Pace::Wait { until_ns } if until_ns == 11 * NS_PER_SEC / 60
        ));
        for frame in 11..40 {
            assert_eq!(take(&mut pacer, &mut clock), frame * NS_PER_SEC / 60);
        }
    }

    #[test]
    fn focus_and_occlusion_never_slow_a_frame_and_minimize_pauses() {
        let mut pacer = FramePacer::new(cap60(true));
        let mut clock = FakeClock::default();
        assert_eq!(take(&mut pacer, &mut clock), 0);
        pacer.on(Attention::Focused(false));
        pacer.on(Attention::Occluded(true));
        assert_eq!(take(&mut pacer, &mut clock), NS_PER_SEC / 60);
        pacer.on(Attention::Minimized(true));
        assert!(matches!(pacer.poll(&clock), Pace::Paused));
        clock.now_ns += 5 * NS_PER_SEC;
        assert!(matches!(pacer.poll(&clock), Pace::Paused));
        pacer.on(Attention::Minimized(false));
        let resumed = clock.now_ns;
        assert_eq!(take(&mut pacer, &mut clock), resumed);
        assert_eq!(take(&mut pacer, &mut clock), resumed + NS_PER_SEC / 60);

        pacer.set_pause_when_minimized(false);
        pacer.on(Attention::Minimized(true));
        assert!(!matches!(pacer.poll(&clock), Pace::Paused));
    }

    #[test]
    fn switching_at_run_time_starts_the_new_rate_at_that_poll() {
        let mut pacer = FramePacer::new(Present::default());
        pacer.set_refresh(Some(60_000));
        let mut clock = FakeClock {
            now_ns: 7 * NS_PER_SEC,
        };
        assert!(matches!(pacer.poll(&clock), Pace::Start { next_ns: None }));
        pacer.set_present(cap60(false));
        let start = take(&mut pacer, &mut clock);
        assert_eq!(start, 7 * NS_PER_SEC);
        assert_eq!(take(&mut pacer, &mut clock), start + NS_PER_SEC / 60);
        pacer.set_present(Present::default());
        assert!(matches!(pacer.poll(&clock), Pace::Start { next_ns: None }));
        assert_eq!(pacer.present(), Present::default());
    }

    #[test]
    fn winit_focus_and_occlusion_events_map_to_attention() {
        let blurred = winit::event::WindowEvent::Focused(false);
        assert_eq!(attention(&blurred), Some(Attention::Focused(false)));
        let covered = winit::event::WindowEvent::Occluded(true);
        assert_eq!(attention(&covered), Some(Attention::Occluded(true)));
        assert!(attention(&winit::event::WindowEvent::RedrawRequested).is_none());
        assert!(attention(&winit::event::WindowEvent::CloseRequested).is_none());
    }
}
