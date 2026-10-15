use crate::camera::WIDE;
pub use crate::colour::colour;
use pfx_core::ease::Ease;
use serde::Deserialize;

pub const MOST_SPEED: f32 = 8.0;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub at: f64,
    pub ease: Option<String>,
    pub zoom: Option<f32>,
    pub focus: Option<[f32; 2]>,
    pub follow: Option<f32>,
    pub roll: Option<f32>,
    pub speed: Option<f32>,
    pub bars: Option<f32>,
    pub black: Option<f32>,
    pub dim: Option<f32>,
    pub desat: Option<f32>,
    pub contrast: Option<f32>,
    pub tint: Option<String>,
    pub gain: Option<f32>,
    pub glow: Option<f32>,
    pub split: Option<f32>,
    pub vignette: Option<f32>,
    pub grain: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub zoom: f32,
    pub focus: [f32; 2],
    pub follow: f32,
    pub roll: f32,
    pub speed: f32,
    pub bars: f32,
    pub black: f32,
    pub dim: f32,
    pub desat: f32,
    pub contrast: f32,
    pub tint: [f32; 3],
    pub gain: f32,
    pub glow: f32,
    pub split: f32,
    pub vignette: f32,
    pub grain: f32,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            zoom: 1.0,
            focus: [WIDE[0] * 0.5, WIDE[1] * 0.5],
            follow: 0.0,
            roll: 0.0,
            speed: 1.0,
            bars: 0.0,
            black: 0.0,
            dim: 0.0,
            desat: 0.0,
            contrast: 0.0,
            tint: [1.0; 3],
            gain: 1.0,
            glow: 0.0,
            split: 0.0,
            vignette: 0.0,
            grain: 0.0,
        }
    }
}

fn curve(name: Option<&str>) -> Option<Option<Ease>> {
    match name {
        None => Some(Some(Ease::SineInOut)),
        Some("step") => Some(None),
        Some(n) => Ease::from_name(n).map(Some),
    }
}

trait Blend: Copy {
    fn blend(self, to: Self, t: f32) -> Self;
}

impl Blend for f32 {
    fn blend(self, to: f32, t: f32) -> f32 {
        self + (to - self) * t
    }
}

impl<const N: usize> Blend for [f32; N] {
    fn blend(self, to: [f32; N], t: f32) -> [f32; N] {
        std::array::from_fn(|i| self[i] + (to[i] - self[i]) * t)
    }
}

fn value<T: Blend>(keys: &[Key], s: f64, get: impl Fn(&Key) -> Option<T>) -> Option<T> {
    let mut before: Option<(f64, T)> = None;
    for k in keys {
        let Some(v) = get(k) else {
            continue;
        };
        if k.at <= s {
            before = Some((k.at, v));
            continue;
        }
        let Some((from, was)) = before else {
            return Some(v);
        };
        let t = ((s - from) / (k.at - from).max(1e-9)).clamp(0.0, 1.0) as f32;
        let e = match curve(k.ease.as_deref()).flatten() {
            Some(ease) => ease.at(t),
            None => 0.0,
        };
        return Some(was.blend(v, e));
    }
    before.map(|(_, v)| v)
}

pub fn look(keys: &[Key], s: f64, base: Look) -> Look {
    let f = |get: fn(&Key) -> Option<f32>, d: f32| value(keys, s, get).unwrap_or(d);
    let tint = value(keys, s, |k| k.tint.as_deref().and_then(|t| colour(t).ok()));
    Look {
        zoom: f(|k| k.zoom, base.zoom),
        focus: value(keys, s, |k| k.focus).unwrap_or(base.focus),
        follow: f(|k| k.follow, base.follow).clamp(0.0, 1.0),
        roll: f(|k| k.roll, base.roll),
        speed: f(|k| k.speed, base.speed).clamp(0.0, MOST_SPEED),
        bars: f(|k| k.bars, base.bars),
        black: f(|k| k.black, base.black),
        dim: f(|k| k.dim, base.dim),
        desat: f(|k| k.desat, base.desat),
        contrast: f(|k| k.contrast, base.contrast),
        tint: tint.unwrap_or(base.tint),
        gain: f(|k| k.gain, base.gain),
        glow: f(|k| k.glow, base.glow),
        split: f(|k| k.split, base.split),
        vignette: f(|k| k.vignette, base.vignette),
        grain: f(|k| k.grain, base.grain),
    }
}

fn within(name: &str, v: Option<f32>, low: f32, high: f32) -> Result<(), String> {
    match v {
        Some(v) if !(v.is_finite() && (low..=high).contains(&v)) => {
            Err(format!("a key's {name} is in [{low}, {high}]"))
        }
        _ => Ok(()),
    }
}

impl Key {
    pub fn check(&self, seconds: f64) -> Result<(), String> {
        if !(self.at.is_finite() && (0.0..=seconds).contains(&self.at)) {
            return Err(format!("a key's at is in [0, {seconds}]"));
        }
        if curve(self.ease.as_deref()).is_none() {
            return Err(format!(
                "no ease '{}' (step, linear, sine_in_out, cubic_out, expo_in, back_out, …)",
                self.ease.as_deref().unwrap_or_default()
            ));
        }
        within("zoom", self.zoom, 0.25, 8.0)?;
        within("roll", self.roll, -45.0, 45.0)?;
        within("speed", self.speed, 0.0, MOST_SPEED)?;
        within("bars", self.bars, 0.0, 4.0)?;
        for (name, v) in [
            ("black", self.black),
            ("follow", self.follow),
            ("dim", self.dim),
            ("desat", self.desat),
            ("gain", self.gain),
            ("glow", self.glow),
            ("vignette", self.vignette),
            ("grain", self.grain),
        ] {
            within(name, v, 0.0, 2.0)?;
        }
        within("contrast", self.contrast, -1.0, 2.0)?;
        within("split", self.split, 0.0, 40.0)?;
        if let Some(f) = self.focus
            && !f.iter().all(|v| v.is_finite() && v.abs() <= 10_000.0)
        {
            return Err("a key's focus is a point in the film's layout".into());
        }
        if let Some(t) = &self.tint {
            colour(t).map_err(|e| format!("a key's tint: {e}"))?;
        }
        Ok(())
    }
}
