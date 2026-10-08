use std::fmt;

use serde::Deserialize;

use super::device::Device;
use crate::window::{
    Letterbox, LetterboxRounding, Rect, RenderScale, Size, Viewport, letterbox_nearest,
    viewport_with,
};

pub const TABLE: &str = "screen";
pub const DEFAULT_TOLERANCE: f64 = 0.03;
pub const DEFAULT_LAYOUT_HEIGHT: u32 = 1080;
pub const LAYOUT_WIDTHS: std::ops::RangeInclusive<u32> = 240..=15360;
pub const DECK_SAFE: f32 = 0.03;
pub const DECK_UI_SCALE: f32 = 1.4;
pub const BACKDROP: &str = "backdrop";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Aspect {
    pub width: u32,
    pub height: u32,
}

impl Aspect {
    pub const DECK: Aspect = Aspect {
        width: 16,
        height: 10,
    };
    pub const WIDE: Aspect = Aspect {
        width: 16,
        height: 9,
    };

    pub fn new(width: u32, height: u32) -> Option<Self> {
        (width > 0 && height > 0).then_some(Self { width, height })
    }

    pub fn of(size: Size) -> Option<Self> {
        let divisor = gcd(size.width, size.height);
        if divisor == 0 {
            return None;
        }
        Self::new(size.width / divisor, size.height / divisor)
    }

    pub fn ratio(self) -> f64 {
        f64::from(self.width) / f64::from(self.height)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (width, height) = text.trim().split_once(':')?;
        Self::new(width.trim().parse().ok()?, height.trim().parse().ok()?)
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.width, self.height)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inset {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Inset {
    pub const NONE: Inset = Inset::all(0.0);

    pub const fn all(fraction: f32) -> Self {
        Self {
            left: fraction,
            right: fraction,
            top: fraction,
            bottom: fraction,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerDevice<T> {
    pub deck: T,
    pub desktop: T,
}

impl<T: Copy> PerDevice<T> {
    pub fn get(&self, device: Device) -> T {
        if device.is_deck() {
            self.deck
        } else {
            self.desktop
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScreenPolicy {
    pub deck: Vec<Aspect>,
    pub desktop: Vec<Aspect>,
    pub tolerance: f64,
    pub bars: [f32; 4],
    pub backdrop: bool,
    pub layout_height: u32,
    pub layout_width: Option<u32>,
    pub safe: PerDevice<Inset>,
    pub ui_scale: PerDevice<f32>,
}

impl Default for ScreenPolicy {
    fn default() -> Self {
        Self {
            deck: vec![Aspect::DECK],
            desktop: vec![Aspect::WIDE],
            tolerance: DEFAULT_TOLERANCE,
            bars: [0.0, 0.0, 0.0, 1.0],
            backdrop: false,
            layout_height: DEFAULT_LAYOUT_HEIGHT,
            layout_width: None,
            safe: PerDevice {
                deck: Inset::all(DECK_SAFE),
                desktop: Inset::NONE,
            },
            ui_scale: PerDevice {
                deck: DECK_UI_SCALE,
                desktop: 1.0,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyError {
    pub key: String,
    pub reason: String,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{TABLE}] {}: {}", self.key, self.reason)
    }
}

impl std::error::Error for PolicyError {}

fn refuse(key: &str, reason: impl Into<String>) -> PolicyError {
    PolicyError {
        key: key.to_owned(),
        reason: reason.into(),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    deck: Option<Vec<String>>,
    desktop: Option<Vec<String>>,
    tolerance: Option<f64>,
    bars: Option<String>,
    layout_height: Option<u32>,
    layout_width: Option<u32>,
    #[serde(default)]
    safe: RawPerDevice<RawInset>,
    #[serde(default)]
    ui_scale: RawPerDevice<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPerDevice<T> {
    deck: Option<T>,
    desktop: Option<T>,
}

impl<T> Default for RawPerDevice<T> {
    fn default() -> Self {
        Self {
            deck: None,
            desktop: None,
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum RawInset {
    All(f32),
    Sides {
        left: f32,
        right: f32,
        top: f32,
        bottom: f32,
    },
}

impl RawInset {
    fn inset(self) -> Inset {
        match self {
            RawInset::All(fraction) => Inset::all(fraction),
            RawInset::Sides {
                left,
                right,
                top,
                bottom,
            } => Inset {
                left,
                right,
                top,
                bottom,
            },
        }
    }
}

impl ScreenPolicy {
    pub fn from_toml(text: &str) -> Result<Self, PolicyError> {
        let document: toml::Table =
            toml::from_str(text).map_err(|error| refuse(TABLE, error.message().to_owned()))?;
        match document.get(TABLE) {
            Some(toml::Value::Table(table)) => Self::from_table(table),
            Some(_) => Err(refuse(TABLE, "must be a table")),
            None => Ok(Self::default()),
        }
    }

    pub fn from_table(table: &toml::Table) -> Result<Self, PolicyError> {
        let raw: RawPolicy = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|error: toml::de::Error| refuse(TABLE, error.message().to_owned()))?;
        let defaults = Self::default();
        if raw.layout_height.is_some() && raw.layout_width.is_some() {
            return Err(refuse(
                "layout_width",
                "set layout_width or layout_height, not both",
            ));
        }
        let policy = Self {
            deck: aspects("deck", raw.deck, defaults.deck)?,
            desktop: aspects("desktop", raw.desktop, defaults.desktop)?,
            tolerance: raw.tolerance.unwrap_or(defaults.tolerance),
            bars: match raw.bars.as_deref() {
                Some(BACKDROP) | None => defaults.bars,
                Some(text) => colour(text).ok_or_else(|| {
                    refuse(
                        "bars",
                        format!("{text:?} is not \"{BACKDROP}\" or a #rrggbb or #rrggbbaa colour"),
                    )
                })?,
            },
            backdrop: raw.bars.as_deref() == Some(BACKDROP),
            layout_height: raw.layout_height.unwrap_or(defaults.layout_height),
            layout_width: raw.layout_width,
            safe: PerDevice {
                deck: raw.safe.deck.map_or(defaults.safe.deck, RawInset::inset),
                desktop: raw
                    .safe
                    .desktop
                    .map_or(defaults.safe.desktop, RawInset::inset),
            },
            ui_scale: PerDevice {
                deck: raw.ui_scale.deck.unwrap_or(defaults.ui_scale.deck),
                desktop: raw.ui_scale.desktop.unwrap_or(defaults.ui_scale.desktop),
            },
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        if !(0.0..=0.25).contains(&self.tolerance) {
            return Err(refuse("tolerance", "must be from 0 to 0.25"));
        }
        if !(240..=8640).contains(&self.layout_height) {
            return Err(refuse("layout_height", "must be from 240 to 8640"));
        }
        if self
            .layout_width
            .is_some_and(|width| !LAYOUT_WIDTHS.contains(&width))
        {
            return Err(refuse("layout_width", "must be from 240 to 15360"));
        }
        if self
            .bars
            .iter()
            .any(|channel| !(0.0..=1.0).contains(channel))
        {
            return Err(refuse("bars", "channels must be from 0 to 1"));
        }
        for (key, inset) in [
            ("safe.deck", self.safe.deck),
            ("safe.desktop", self.safe.desktop),
        ] {
            let sides = [inset.left, inset.right, inset.top, inset.bottom];
            if sides.iter().any(|side| !(0.0..=0.25).contains(side)) {
                return Err(refuse(key, "each side must be from 0 to 0.25"));
            }
        }
        for (key, scale) in [
            ("ui_scale.deck", self.ui_scale.deck),
            ("ui_scale.desktop", self.ui_scale.desktop),
        ] {
            if !(0.25..=4.0).contains(&scale) {
                return Err(refuse(key, "must be from 0.25 to 4"));
            }
        }
        for (key, list) in [("deck", &self.deck), ("desktop", &self.desktop)] {
            if list.iter().any(|aspect| {
                let ratio = aspect.ratio();
                !(0.25..=8.0).contains(&ratio)
            }) {
                return Err(refuse(key, "aspects must be from 1:4 to 8:1"));
            }
        }
        Ok(())
    }

    pub fn aspects(&self, device: Device) -> &[Aspect] {
        if device.is_deck() {
            &self.deck
        } else {
            &self.desktop
        }
    }

    pub fn fit(&self, device: Device, window: Size) -> Option<ScreenReport> {
        let actual = Aspect::of(window)?;
        let ratio = f64::from(window.width) / f64::from(window.height);
        let supported = self.aspects(device);
        let nearest = supported.iter().copied().min_by(|a, b| {
            distance(ratio, a.ratio())
                .total_cmp(&distance(ratio, b.ratio()))
                .then(a.width.cmp(&b.width))
        });
        let (aspect, native, layout_ratio) = match nearest {
            None => (actual, true, ratio),
            Some(aspect) if (ratio / aspect.ratio() - 1.0).abs() <= self.tolerance => {
                (aspect, true, ratio)
            }
            Some(aspect) => (aspect, false, aspect.ratio()),
        };
        let layout = self.layout_size(layout_ratio)?;
        let letterbox = letterbox_nearest(layout.width as f32, layout.height as f32, window)?;
        let content = letterbox.visible;
        let bars = if native {
            Bars::None
        } else if aspect.ratio() < ratio {
            Bars::Pillarbox
        } else {
            Bars::Letterbox
        };
        let inset = self.safe.get(device);
        let short = content.width.min(content.height);
        let safe = Rect {
            x: content.x + inset.left * short,
            y: content.y + inset.top * short,
            width: content.width - (inset.left + inset.right) * short,
            height: content.height - (inset.top + inset.bottom) * short,
        };
        let [lx, ly] = letterbox.unmap(safe.x, safe.y);
        let [rx, ry] = letterbox.unmap(safe.x + safe.width, safe.y + safe.height);
        Some(ScreenReport {
            device,
            window,
            aspect,
            native,
            bars,
            bar_colour: self.bars,
            backdrop: self.backdrop,
            layout,
            letterbox,
            content,
            safe,
            safe_layout: Rect {
                x: lx,
                y: ly,
                width: rx - lx,
                height: ry - ly,
            },
            ui_scale: self.ui_scale.get(device),
        })
    }
}

impl ScreenPolicy {
    pub fn layout_size(&self, ratio: f64) -> Option<Size> {
        if !(ratio.is_finite() && ratio > 0.0) {
            return None;
        }
        let units = |value: f64| u32::try_from(value.round().max(1.0) as u64).ok();
        match self.layout_width {
            Some(width) => Some(Size {
                width,
                height: units(f64::from(width) / ratio)?,
            }),
            None => Some(Size {
                width: units(f64::from(self.layout_height) * ratio)?,
                height: self.layout_height,
            }),
        }
    }
}

fn distance(a: f64, b: f64) -> f64 {
    (a / b).ln().abs()
}

fn aspects(
    key: &str,
    raw: Option<Vec<String>>,
    defaults: Vec<Aspect>,
) -> Result<Vec<Aspect>, PolicyError> {
    let Some(raw) = raw else {
        return Ok(defaults);
    };
    raw.iter()
        .map(|text| {
            Aspect::parse(text)
                .ok_or_else(|| refuse(key, format!("{text:?} is not an aspect such as \"16:9\"")))
        })
        .collect()
}

fn colour(text: &str) -> Option<[f32; 4]> {
    let hex = text.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |index: usize| {
        u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .ok()
            .map(|value| f32::from(value) / 255.0)
    };
    Some([
        channel(0)?,
        channel(1)?,
        channel(2)?,
        if hex.len() == 8 { channel(3)? } else { 1.0 },
    ])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bars {
    None,
    Pillarbox,
    Letterbox,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenReport {
    pub device: Device,
    pub window: Size,
    pub aspect: Aspect,
    pub native: bool,
    pub bars: Bars,
    pub bar_colour: [f32; 4],
    pub backdrop: bool,
    pub layout: Size,
    pub letterbox: Letterbox,
    pub content: Rect,
    pub safe: Rect,
    pub safe_layout: Rect,
    pub ui_scale: f32,
}

impl ScreenReport {
    pub fn layout_units(&self) -> [f32; 2] {
        [self.layout.width as f32, self.layout.height as f32]
    }

    pub fn pixels_per_unit(&self) -> f32 {
        self.letterbox.matrix[0]
    }

    pub fn window_units(&self) -> [f32; 2] {
        let scale = self.pixels_per_unit();
        [
            self.window.width as f32 / scale,
            self.window.height as f32 / scale,
        ]
    }

    pub fn content_units(&self) -> Rect {
        let scale = self.pixels_per_unit();
        Rect {
            x: self.content.x / scale,
            y: self.content.y / scale,
            width: self.layout.width as f32,
            height: self.layout.height as f32,
        }
    }

    pub fn units_for_pixels(&self, pixels: f32) -> f32 {
        pixels / self.pixels_per_unit()
    }

    pub fn viewport(&self) -> Option<Viewport> {
        viewport_with(
            self.window,
            RenderScale::Native,
            self.layout,
            LetterboxRounding::Nearest,
        )
    }

    pub fn render_size(&self, scale: f32) -> Option<Size> {
        let content = Size {
            width: self.content.width.round() as u32,
            height: self.content.height.round() as u32,
        };
        crate::window::render_size(content, RenderScale::Factor(scale))
    }

    pub fn in_bars(&self, window_x: f32, window_y: f32) -> bool {
        !self.letterbox.contains(window_x, window_y)
    }
}
