use pfx_sound::{DRIVE_MAX_DB, DRIVE_MIN_DB, Drive, DriveCurve, Wave};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Sine,
    Square,
    Saw,
    White,
    Pink,
}

impl Kind {
    pub fn wave(self) -> Wave {
        match self {
            Kind::Sine => Wave::Sine,
            Kind::Square => Wave::Square,
            Kind::Saw => Wave::Saw,
            Kind::White => Wave::White,
            Kind::Pink => Wave::Pink,
        }
    }

    fn noise(self) -> bool {
        matches!(self, Kind::White | Kind::Pink)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub wave: Kind,
    pub note: Option<f32>,
    pub hz: Option<f32>,
    #[serde(default)]
    pub cents: f32,
    pub volume: f32,
    #[serde(default)]
    pub attack: f32,
    #[serde(default)]
    pub decay: f32,
    #[serde(default)]
    pub sustain: f32,
    #[serde(default)]
    pub hold: f32,
    #[serde(default)]
    pub release: f32,
    #[serde(default)]
    pub at: f32,
    #[serde(default)]
    pub bend: f32,
    #[serde(default)]
    pub glide: f32,
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub linear: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Curve {
    Soft,
    Tanh,
    Hard,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Saturate {
    pub curve: Curve,
    pub mix: f32,
    #[serde(default)]
    pub output: f32,
    pub sweep: Vec<[f32; 2]>,
}

impl Saturate {
    pub fn stage(&self) -> Drive {
        Drive {
            drive_db: self.sweep.first().map_or(0.0, |p| p[1]),
            curve: match self.curve {
                Curve::Soft => DriveCurve::Soft,
                Curve::Tanh => DriveCurve::Tanh,
                Curve::Hard => DriveCurve::Hard,
            },
            mix: self.mix,
            compensate: false,
            output_db: self.output,
        }
    }

    fn check(&self) -> Result<(), String> {
        if !(0.0..=1.0).contains(&self.mix) || !(-60.0..=24.0).contains(&self.output) {
            return Err("a drive's mix is in [0, 1] and its output in [-60, 24] dB".into());
        }
        if self.sweep.is_empty() {
            return Err("a drive needs a sweep of [ms, dB]".into());
        }
        let mut last = -1.0;
        for [ms, db] in &self.sweep {
            if !(ms.is_finite() && *ms >= 0.0 && *ms >= last) {
                return Err("a drive sweep's times rise from 0".into());
            }
            if !(DRIVE_MIN_DB..=DRIVE_MAX_DB).contains(db) {
                return Err(format!("a drive is in {DRIVE_MIN_DB}..={DRIVE_MAX_DB} dB"));
            }
            last = *ms;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    #[serde(default = "second")]
    pub length: f32,
    #[serde(default)]
    pub lowpass: Vec<[f32; 2]>,
    #[serde(default)]
    pub highpass: Vec<[f32; 2]>,
    pub drive: Option<Saturate>,
    #[serde(rename = "layer", default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub limiter: f32,
    pub master: f32,
    #[serde(rename = "recipe")]
    pub recipes: BTreeMap<String, Recipe>,
}

fn second() -> f32 {
    1000.0
}

pub fn hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

impl Layer {
    pub fn pitch(&self) -> f32 {
        let base = self.hz.or(self.note.map(hz)).unwrap_or(0.0);
        base * 2f32.powf(self.cents / 1200.0)
    }

    pub fn span(&self) -> f32 {
        if self.sustain > 0.0 {
            self.attack + self.hold + self.release
        } else {
            self.attack + self.decay
        }
    }

    fn check(&self) -> Result<(), String> {
        let pitched = self.hz.is_some() || self.note.is_some();
        if self.hz.is_some() && self.note.is_some() {
            return Err("a layer takes hz or note, not both".into());
        }
        if !self.wave.noise() && !pitched {
            return Err("a tone layer needs hz or note".into());
        }
        let times = [
            self.attack,
            self.decay,
            self.hold,
            self.release,
            self.at,
            self.glide,
        ];
        if times.iter().any(|t| !t.is_finite() || *t < 0.0) {
            return Err("times are milliseconds, never negative".into());
        }
        if !(0.0..=1.0).contains(&self.volume) || !(0.0..=1.0).contains(&self.sustain) {
            return Err("volume and sustain are in [0, 1]".into());
        }
        if !(-1.0..=1.0).contains(&self.pan) || !self.bend.is_finite() || self.bend.abs() > 48.0 {
            return Err("pan is in [-1, 1] and bend within four octaves".into());
        }
        if self.span() <= 0.0 {
            return Err("a layer needs an attack, decay or hold".into());
        }
        Ok(())
    }
}

fn sweep(points: &[[f32; 2]]) -> Result<(), String> {
    let mut last = -1.0;
    for [ms, hz] in points {
        if !(ms.is_finite() && *ms >= 0.0 && *ms >= last) {
            return Err("a sweep's times rise from 0".into());
        }
        if !(hz.is_finite() && (5.0..=24_000.0).contains(hz)) {
            return Err("a sweep's cutoffs are in 5..=24000 Hz".into());
        }
        last = *ms;
    }
    Ok(())
}

impl Table {
    pub fn parse(text: &str, bare: &[&str]) -> Result<Table, String> {
        let table: Table = toml::from_str(text).map_err(|e| format!("score: {e}"))?;
        for (name, r) in &table.recipes {
            let at = |e: String| format!("score: {name}: {e}");
            if r.layers.is_empty() && !bare.contains(&name.as_str()) {
                return Err(at("a recipe needs a layer".into()));
            }
            if !(r.length.is_finite() && r.length > 0.0) {
                return Err(at("length is positive milliseconds".into()));
            }
            if r.kind
                .as_deref()
                .is_some_and(|k| !crate::script::plain(k) || k == "score")
            {
                return Err(at(
                    "a kind is letters, digits, '-' and '_', and not 'score'".into(),
                ));
            }
            sweep(&r.lowpass).map_err(at)?;
            sweep(&r.highpass).map_err(at)?;
            if let Some(d) = &r.drive {
                d.check().map_err(at)?;
            }
            for l in &r.layers {
                l.check().map_err(at)?;
            }
        }
        Ok(table)
    }
}
