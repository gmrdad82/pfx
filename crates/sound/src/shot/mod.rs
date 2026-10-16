mod ambience;
mod curve;
mod dsp;
mod engine;
mod layers;
mod read;

pub use curve::{Curve, Ease};

use curve::Track;
use read::{Fields, tables};
use std::path::{Path, PathBuf};

pub const RATE: u32 = 48_000;
pub(crate) const CONTROL: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Fixed(f32),
    Curve(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Common {
    pub name: Option<String>,
    pub gain: Value,
    pub pan: Value,
    pub start: f32,
    pub end: Option<f32>,
    pub fade_in: f32,
    pub fade_out: f32,
    pub seed: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Engine {
    pub rpm: Value,
    pub load: Value,
    pub cylinders: u32,
    pub drive: f32,
    pub lowpass: f32,
    pub crackle: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Road {
    pub speed: Value,
    pub roughness: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Squeal {
    pub at: Vec<[f32; 2]>,
    pub curve: Option<String>,
    pub threshold: f32,
    pub range: f32,
    pub hz: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spray {
    pub speed: Value,
    pub wet: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Whoosh {
    pub at: Vec<f32>,
    pub speed: f32,
    pub distance: f32,
    pub hz: f32,
    pub sweep: [f32; 2],
    pub length: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ambience {
    pub bed: f32,
    pub rumble: f32,
    pub horns: f32,
    pub birds: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Engine(Engine),
    Road(Road),
    Squeal(Squeal),
    Spray(Spray),
    Whoosh(Whoosh),
    Ambience(Ambience),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub common: Common,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub path: PathBuf,
    pub at: f32,
    pub gain: f32,
    pub pan: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub seed: u64,
    pub gain: f32,
    pub limiter: Option<f32>,
    pub curves: Vec<Curve>,
    pub layers: Vec<Layer>,
    pub samples: Vec<Sample>,
}

pub fn render(script_path: &Path, frames: usize, fps: u32) -> Result<Vec<f32>, String> {
    if fps == 0 {
        return Err("fps must be above 0".into());
    }
    let script = Script::load(script_path)?;
    script.mix(audio_frames(frames, fps))
}

pub fn audio_frames(frames: usize, fps: u32) -> usize {
    let fps = u64::from(fps.max(1));
    ((frames as u64 * u64::from(RATE) + fps / 2) / fps) as usize
}

const KINDS: [&str; 6] = ["engine", "road", "squeal", "spray", "whoosh", "ambience"];

impl Script {
    pub fn load(path: &Path) -> Result<Script, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("{}: cannot read: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new(""));
        Script::parse(&text, dir).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str, dir: &Path) -> Result<Script, String> {
        let root: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let mut top = Fields::new(&root, "script".into());
        let seed = top.whole("seed")?.unwrap_or(0);
        let mut gain = 1.0;
        let mut limiter = None;
        if let Some(mix) = root.get("mix") {
            let toml::Value::Table(mix) = mix else {
                return Err("`mix` must be a table".into());
            };
            let mut f = Fields::new(mix, "[mix]".into());
            gain = f.number_or("gain", 1.0)?;
            limiter = f.number("limiter")?.map(|v| v.clamp(0.05, 0.999));
            f.finish()?;
            top.mark("mix");
        }
        let mut curves = Vec::new();
        for (i, table) in tables(&root, "curve")?.into_iter().enumerate() {
            curves.push(curve(table, i)?);
        }
        top.mark("curve");
        let mut layers = Vec::new();
        for kind in KINDS {
            for (i, table) in tables(&root, kind)?.into_iter().enumerate() {
                layers.push(layer(kind, table, i)?);
            }
            top.mark(kind);
        }
        let mut samples = Vec::new();
        for (i, table) in tables(&root, "sample")?.into_iter().enumerate() {
            let mut f = Fields::new(table, format!("[[sample]] {}", i + 1));
            let path = f
                .text("path")?
                .ok_or_else(|| format!("[[sample]] {}: `path` is required", i + 1))?;
            samples.push(Sample {
                path: dir.join(path),
                at: f.number_or("at", 0.0)?,
                gain: f.number_or("gain", 1.0)?,
                pan: f.number_or("pan", 0.0)?.clamp(-1.0, 1.0),
            });
            f.finish()?;
        }
        top.mark("sample");
        top.finish()?;
        let script = Script {
            seed,
            gain,
            limiter,
            curves,
            layers,
            samples,
        };
        script.check()?;
        Ok(script)
    }

    fn check(&self) -> Result<(), String> {
        for (i, c) in self.curves.iter().enumerate() {
            if self.curves[..i].iter().any(|o| o.name == c.name) {
                return Err(format!("curve `{}` is named twice", c.name));
            }
        }
        let known = |v: &Value| match v {
            Value::Curve(name) if !self.curves.iter().any(|c| &c.name == name) => {
                Err(format!("no curve named `{name}`"))
            }
            _ => Ok(()),
        };
        for layer in &self.layers {
            known(&layer.common.gain)?;
            known(&layer.common.pan)?;
            match &layer.kind {
                Kind::Engine(e) => {
                    known(&e.rpm)?;
                    known(&e.load)?;
                }
                Kind::Road(r) => known(&r.speed)?,
                Kind::Squeal(s) => {
                    known(&s.hz)?;
                    if let Some(name) = &s.curve {
                        known(&Value::Curve(name.clone()))?;
                    }
                }
                Kind::Spray(s) => {
                    known(&s.speed)?;
                    known(&s.wet)?;
                }
                Kind::Whoosh(_) | Kind::Ambience(_) => {}
            }
        }
        Ok(())
    }

    pub fn mix(&self, frames: usize) -> Result<Vec<f32>, String> {
        let ctx = Ctx {
            tracks: self.curves.iter().map(Track::new).collect(),
            names: self.curves.iter().map(|c| c.name.clone()).collect(),
            seconds: frames as f32 / RATE as f32,
        };
        let mut out = vec![0.0f32; frames * 2];
        for (index, layer) in self.layers.iter().enumerate() {
            let seed = layer.common.seed.unwrap_or_else(|| {
                pfx_core::sim::mix64(self.seed ^ pfx_core::sim::mix64(index as u64 + 1))
            });
            let voice = layers::voice(&layer.kind, seed, &ctx);
            layers::run(voice, &layer.common, &ctx, &mut out);
        }
        for sample in &self.samples {
            place(sample, &mut out)?;
        }
        for s in &mut out {
            let mut v = *s * self.gain;
            if let Some(threshold) = self.limiter {
                v = crate::mix::soft_limit(v, threshold);
            }
            *s = v.clamp(-1.0, 1.0);
        }
        Ok(out)
    }
}

fn place(sample: &Sample, out: &mut [f32]) -> Result<(), String> {
    let bytes = std::fs::read(&sample.path)
        .map_err(|e| format!("{}: cannot read: {e}", sample.path.display()))?;
    let clip = crate::Clip::from_wav(&bytes)
        .ok_or_else(|| format!("{}: not a WAV this mixer reads", sample.path.display()))?;
    let pcm = clip.at_rate(RATE, 2);
    let (l, r) = (
        (1.0 - sample.pan).min(1.0) * sample.gain,
        (1.0 + sample.pan).min(1.0) * sample.gain,
    );
    let at = (sample.at.max(0.0) * RATE as f32).round() as usize * 2;
    for (i, frame) in pcm.chunks_exact(2).enumerate() {
        let Some(dst) = out.get_mut(at + i * 2..at + i * 2 + 2) else {
            break;
        };
        dst[0] += frame[0] * l;
        dst[1] += frame[1] * r;
    }
    Ok(())
}

pub(crate) struct Ctx {
    tracks: Vec<Track>,
    names: Vec<String>,
    pub(crate) seconds: f32,
}

#[derive(Clone, Copy)]
pub(crate) enum Knob {
    Fixed(f32),
    Track(usize),
}

impl Ctx {
    pub(crate) fn knob(&self, value: &Value) -> Knob {
        match value {
            Value::Fixed(v) => Knob::Fixed(*v),
            Value::Curve(name) => self
                .names
                .iter()
                .position(|n| n == name)
                .map_or(Knob::Fixed(0.0), Knob::Track),
        }
    }

    pub(crate) fn get(&self, knob: Knob, t: f32) -> f32 {
        match knob {
            Knob::Fixed(v) => v,
            Knob::Track(i) => self.tracks[i].at(t),
        }
    }
}

fn curve(table: &toml::Table, i: usize) -> Result<Curve, String> {
    let at = format!("[[curve]] {}", i + 1);
    let mut f = Fields::new(table, at.clone());
    let name = f
        .text("name")?
        .ok_or_else(|| format!("{at}: `name` is required"))?;
    let keys = f
        .pairs("keys")?
        .ok_or_else(|| format!("{at}: `keys` is required"))?;
    if keys.is_empty() {
        return Err(format!("{at}: `keys` is empty"));
    }
    if keys.windows(2).any(|w| w[1][0] < w[0][0]) {
        return Err(format!("{at}: key times must not go backwards"));
    }
    let ease = match f.text("ease")?.as_deref() {
        None | Some("linear") => Ease::Linear,
        Some("smooth") => Ease::Smooth,
        Some(other) => {
            return Err(format!(
                "{at}: `ease` must be \"linear\" or \"smooth\", not \"{other}\""
            ));
        }
    };
    f.finish()?;
    Ok(Curve { name, keys, ease })
}

fn layer(kind: &str, table: &toml::Table, i: usize) -> Result<Layer, String> {
    let mut f = Fields::new(table, format!("[[{kind}]] {}", i + 1));
    let common = Common {
        name: f.text("name")?,
        gain: f.value("gain", 1.0)?,
        pan: f.value("pan", 0.0)?,
        start: f.number_or("start", 0.0)?.max(0.0),
        end: f.number("end")?,
        fade_in: f.number_or("fade_in", 0.0)?.max(0.0),
        fade_out: f.number_or("fade_out", 0.0)?.max(0.0),
        seed: f.whole("seed")?,
    };
    let kind = match kind {
        "engine" => Kind::Engine(Engine {
            rpm: f.value("rpm", 900.0)?,
            load: f.value("load", 1.0)?,
            cylinders: f.whole("cylinders")?.unwrap_or(8).clamp(1, 16) as u32,
            drive: f.number_or("drive", 6.0)?.clamp(0.0, 40.0),
            lowpass: f.number_or("lowpass", 6_000.0)?.clamp(50.0, 20_000.0),
            crackle: f.number_or("crackle", 0.0)?.clamp(0.0, 1.0),
        }),
        "road" => Kind::Road(Road {
            speed: f.value("speed", 0.0)?,
            roughness: f.number_or("roughness", 0.4)?.clamp(0.0, 1.0),
        }),
        "squeal" => Kind::Squeal(Squeal {
            at: f.pairs("at")?.unwrap_or_default(),
            curve: f.text("curve")?,
            threshold: f.number_or("threshold", 0.5)?,
            range: f.number_or("range", 0.5)?.max(1e-3),
            hz: f.value("hz", 1_400.0)?,
        }),
        "spray" => Kind::Spray(Spray {
            speed: f.value("speed", 0.0)?,
            wet: f.value("wet", 1.0)?,
        }),
        "whoosh" => Kind::Whoosh(Whoosh {
            at: f.numbers("at")?.unwrap_or_default(),
            speed: f.number_or("speed", 20.0)?.clamp(0.0, 300.0),
            distance: f.number_or("distance", 3.0)?.max(0.2),
            hz: f.number_or("hz", 700.0)?.clamp(40.0, 12_000.0),
            sweep: f.pair("sweep", [-1.0, 1.0])?,
            length: f.number_or("length", 2.0)?.max(0.05),
        }),
        _ => Kind::Ambience(Ambience {
            bed: f.number_or("bed", 1.0)?.max(0.0),
            rumble: f.number_or("rumble", 1.0)?.max(0.0),
            horns: f.number_or("horns", 3.0)?.max(0.0),
            birds: f.number_or("birds", 6.0)?.max(0.0),
        }),
    };
    f.finish()?;
    Ok(Layer { common, kind })
}
