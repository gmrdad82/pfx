use serde::{Deserialize, Serialize};

use super::device::Device;
use crate::window::{
    DisplayMode, ModePlan, MonitorChoice, Monitors, PlanError, PlatformCaps, Point, Size,
    VideoModeChoice, WindowOps, apply_plan, plan_display_mode,
};

pub const DEFAULT_WINDOWED: Size = Size {
    width: 1600,
    height: 900,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    Windowed,
    #[default]
    Borderless,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySettings {
    #[serde(default)]
    pub mode: WindowMode,
    #[serde(default = "default_windowed")]
    pub size: Size,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
}

fn default_windowed() -> Size {
    DEFAULT_WINDOWED
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            mode: WindowMode::Borderless,
            size: DEFAULT_WINDOWED,
            monitor: None,
            position: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Switched { from: WindowMode, to: WindowMode },
    Same,
    FixedOnDeck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    device: Device,
    settings: DisplaySettings,
}

impl Display {
    pub fn new(device: Device, settings: DisplaySettings) -> Self {
        let mut settings = settings;
        if settings.size.width == 0 || settings.size.height == 0 {
            settings.size = DEFAULT_WINDOWED;
        }
        Self { device, settings }
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn settings(&self) -> &DisplaySettings {
        &self.settings
    }

    pub fn mode(&self) -> WindowMode {
        if self.device.is_deck() {
            WindowMode::Fullscreen
        } else {
            self.settings.mode
        }
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.mode_with(None)
    }

    pub fn display_mode_on(&self, monitors: &Monitors) -> DisplayMode {
        self.mode_with(self.windowed_at(monitors))
    }

    pub fn windowed_at(&self, monitors: &Monitors) -> Option<Point> {
        let relative = self.settings.position?;
        let index = match &self.settings.monitor {
            Some(name) => monitors
                .list
                .iter()
                .position(|monitor| monitor.name.as_deref() == Some(name.as_str()))?,
            None => monitors.current.or(monitors.primary)?,
        };
        let monitor = monitors.list.get(index)?;
        let room = |span: u32, window: u32| i32::try_from(span.saturating_sub(window)).unwrap_or(0);
        Some(Point {
            x: monitor.position.x
                + relative
                    .x
                    .clamp(0, room(monitor.size.width, self.settings.size.width)),
            y: monitor.position.y
                + relative
                    .y
                    .clamp(0, room(monitor.size.height, self.settings.size.height)),
        })
    }

    fn mode_with(&self, position: Option<Point>) -> DisplayMode {
        let monitor = match &self.settings.monitor {
            Some(name) => MonitorChoice::Name(name.clone()),
            None => MonitorChoice::Current,
        };
        if self.device.is_deck() {
            return DisplayMode::BorderlessFullscreen {
                monitor: MonitorChoice::Current,
            };
        }
        match self.settings.mode {
            WindowMode::Windowed => DisplayMode::Windowed {
                size: self.settings.size,
                position,
            },
            WindowMode::Borderless => DisplayMode::BorderlessFullscreen { monitor },
            WindowMode::Fullscreen => DisplayMode::ExclusiveFullscreen {
                monitor,
                video_mode: VideoModeChoice::Native,
            },
        }
    }

    pub fn set_mode(&mut self, mode: WindowMode) -> Change {
        if self.device.is_deck() {
            return Change::FixedOnDeck;
        }
        let from = self.settings.mode;
        if from == mode {
            return Change::Same;
        }
        self.settings.mode = mode;
        Change::Switched { from, to: mode }
    }

    pub fn resized(&mut self, size: Size) {
        if !self.device.is_deck()
            && self.settings.mode == WindowMode::Windowed
            && size.width > 0
            && size.height > 0
        {
            self.settings.size = size;
        }
    }

    pub fn moved_to(&mut self, monitor: Option<String>) {
        if !self.device.is_deck() {
            self.settings.monitor = monitor;
        }
    }

    pub fn placed(&mut self, monitor: Option<String>, position: Point) {
        if !self.device.is_deck() && self.settings.mode == WindowMode::Windowed {
            self.settings.monitor = monitor;
            self.settings.position = Some(position);
        }
    }

    pub fn plan(&self, platform: PlatformCaps, monitors: &Monitors) -> Result<ModePlan, PlanError> {
        plan_display_mode(&self.display_mode_on(monitors), platform, monitors)
    }

    pub fn apply(
        &self,
        platform: PlatformCaps,
        monitors: &Monitors,
        window: &mut impl WindowOps,
    ) -> Result<ModePlan, PlanError> {
        let plan = self.plan(platform, monitors)?;
        apply_plan(&plan, window);
        Ok(plan)
    }

    pub fn switch(
        &mut self,
        mode: WindowMode,
        platform: PlatformCaps,
        monitors: &Monitors,
        window: &mut impl WindowOps,
    ) -> Result<(Change, Option<ModePlan>), PlanError> {
        let change = self.set_mode(mode);
        match change {
            Change::Switched { from, .. } => match self.apply(platform, monitors, window) {
                Ok(plan) => Ok((change, Some(plan))),
                Err(error) => {
                    self.settings.mode = from;
                    Err(error)
                }
            },
            Change::Same | Change::FixedOnDeck => Ok((change, None)),
        }
    }
}

#[cfg(feature = "window")]
pub fn apply_display(
    window: &winit::window::Window,
    display: &Display,
) -> Result<ModePlan, PlanError> {
    let monitors = crate::window::monitors(window);
    crate::window::apply_display_mode(window, &display.display_mode_on(&monitors))
}
