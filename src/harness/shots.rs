use crate::protocol::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const LOGICAL: (u32, u32) = (1920, 1080);

#[derive(Deserialize)]
struct Book {
    shots: BTreeMap<String, Shot>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Shot {
    pub line: String,
    pub fixtures: String,
    pub seconds: f64,
    #[serde(default = "sixty")]
    pub fps: Vec<u32>,
    #[serde(default = "full_hd")]
    pub size: Vec<String>,
    #[serde(default)]
    pub closure: Option<toml::Value>,
    #[serde(default = "warm")]
    pub warmup: u32,
    #[serde(default = "sixteen")]
    pub per_frame: u32,
    #[serde(default)]
    pub moving: u32,
    #[serde(default)]
    pub cutouts: bool,
    #[serde(default)]
    pub youtube: bool,
    #[serde(default)]
    pub app: toml::Table,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daylight: Option<toml::Table>,
    #[serde(default)]
    pub inputs: Vec<Timed>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Timed {
    pub at: f64,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub cue: Option<String>,
    #[serde(default)]
    pub pointer: Option<[f32; 2]>,
    #[serde(default)]
    pub button: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Closure {
    Open,
    Crossfade(f64),
    Palindrome,
}

fn sixty() -> Vec<u32> {
    vec![60]
}

fn full_hd() -> Vec<String> {
    vec!["1080p".into()]
}

fn warm() -> u32 {
    256
}

fn sixteen() -> u32 {
    16
}

pub fn book(text: &str) -> Result<BTreeMap<String, Shot>, String> {
    toml::from_str::<Book>(text)
        .map(|b| b.shots)
        .map_err(|e| format!("shots.toml: {e}"))
}

const SIZES: [(&str, u32, u32, f32); 4] = [
    ("720p", 1280, 720, 2.0 / 3.0),
    ("1080p", 1920, 1080, 1.0),
    ("1440p", 2560, 1440, 4.0 / 3.0),
    ("2160p", 3840, 2160, 2.0),
];

pub fn size(name: &str) -> Result<(u32, u32, f32), String> {
    SIZES
        .iter()
        .find(|s| s.0 == name || format!("{}x{}", s.1, s.2) == name)
        .map(|s| (s.1, s.2, s.3))
        .ok_or_else(|| {
            format!(
                "'{name}' is not a target size; use 720p, 1080p, 1440p or 2160p (or 1280x720 … 3840x2160)"
            )
        })
}

pub fn named(text: &str) -> Result<String, String> {
    let (w, h, _) = size(text)?;
    Ok(SIZES
        .iter()
        .find(|s| (s.1, s.2) == (w, h))
        .map(|s| s.0.to_string())
        .unwrap_or_default())
}

const KEYS: [&str; 14] = [
    "line",
    "fixtures",
    "seconds",
    "fps",
    "size",
    "closure",
    "warmup",
    "per_frame",
    "moving",
    "cutouts",
    "youtube",
    "app",
    "inputs",
    "daylight",
];

const DAYLIGHT: [(&str, f64, f64, Option<f64>); 4] = [
    ("hour", 0.0, 24.0, None),
    ("day", 1.0, 365.0, Some(172.0)),
    ("latitude", -90.0, 90.0, Some(45.0)),
    ("heading", 0.0, 360.0, Some(180.0)),
];

pub fn daylight(shot: &mut Shot) -> Result<(), String> {
    let Some(table) = shot.daylight.as_mut() else {
        return Ok(());
    };
    for key in table.keys() {
        if !DAYLIGHT.iter().any(|d| d.0 == key) {
            return Err(format!(
                "daylight knows no '{key}'; it takes hour, day, latitude and heading"
            ));
        }
    }
    for (key, low, high, fallback) in DAYLIGHT {
        let value = match table.get(key) {
            Some(toml::Value::Float(v)) => *v,
            Some(toml::Value::Integer(v)) => *v as f64,
            Some(other) => return Err(format!("daylight.{key} is {other}, not a number")),
            None => fallback.ok_or("daylight needs an hour, 0 to 24")?,
        };
        if !(low..=high).contains(&value) {
            return Err(format!(
                "daylight.{key} = {value} is outside {low} to {high}"
            ));
        }
        table.insert(key.to_string(), toml::Value::Float(value));
    }
    Ok(())
}

pub fn set(shot: &Shot, pairs: &[String]) -> Result<Shot, String> {
    let mut value = toml::Value::try_from(shot).map_err(|e| format!("--set: {e}"))?;
    for pair in pairs {
        let (key, raw) = pair
            .split_once('=')
            .ok_or_else(|| format!("--set takes key=value, got '{pair}'"))?;
        let parts: Vec<&str> = key.trim().split('.').collect();
        if !KEYS.contains(&parts[0])
            || (parts.len() > 1 && parts[0] != "app" && parts[0] != "daylight")
        {
            return Err(format!(
                "--set knows no key '{key}'; it takes {}, app.<key> or daylight.<key>",
                KEYS.join(", ")
            ));
        }
        let parsed = toml::from_str::<toml::Table>(&format!("v = {raw}"))
            .ok()
            .and_then(|mut t| t.remove("v"))
            .unwrap_or_else(|| toml::Value::String(raw.to_string()));
        let mut node = &mut value;
        for part in &parts[..parts.len() - 1] {
            node = node
                .as_table_mut()
                .ok_or_else(|| format!("--set {key}: not a table"))?
                .entry(part.to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        }
        node.as_table_mut()
            .ok_or_else(|| format!("--set {key}: not a table"))?
            .insert(parts[parts.len() - 1].to_string(), parsed);
    }
    value.try_into().map_err(|e| format!("--set: {e}"))
}

pub fn rate(fps: u32) -> Result<u32, String> {
    match fps {
        60 => Ok(fps),
        other => Err(format!("{other} fps is not a target; pfx renders at 60")),
    }
}

impl Shot {
    pub fn closure(&self) -> Result<Closure, String> {
        match &self.closure {
            None => Ok(Closure::Open),
            Some(toml::Value::String(s)) if s == "palindrome" => Ok(Closure::Palindrome),
            Some(toml::Value::Table(t)) => match t.get("crossfade") {
                Some(toml::Value::Float(s)) if *s > 0.0 => Ok(Closure::Crossfade(*s)),
                Some(toml::Value::Integer(s)) if *s > 0 => Ok(Closure::Crossfade(*s as f64)),
                _ => Err("closure table needs crossfade = <seconds>".into()),
            },
            Some(other) => Err(format!(
                "closure {other} is neither \"palindrome\" nor {{ crossfade = <seconds> }}"
            )),
        }
    }

    pub fn targets(&self) -> Result<Vec<(u32, String)>, String> {
        let mut out = Vec::new();
        for &fps in &self.fps {
            for name in &self.size {
                size(name)?;
                out.push((rate(fps)?, name.clone()));
            }
        }
        if out.is_empty() {
            return Err("a shot needs at least one fps and one size".into());
        }
        Ok(out)
    }

    pub fn master(&self) -> Result<(u32, String), String> {
        let targets = self.targets()?;
        let fps = targets.iter().map(|t| t.0).max().unwrap_or(30);
        let name = targets
            .iter()
            .map(|t| t.1.clone())
            .max_by_key(|n| size(n).map(|s| s.0).unwrap_or(0))
            .unwrap_or_else(|| "1080p".into());
        if targets.iter().any(|t| fps % t.0 != 0) {
            return Err("every fps must divide the master fps".into());
        }
        Ok((fps, name))
    }

    pub fn app_json(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(&self.app)
            .unwrap_or(serde_json::Value::Object(Default::default()));
        if let (Some(d), serde_json::Value::Object(map)) = (&self.daylight, &mut value) {
            map.insert(
                "daylight".into(),
                serde_json::to_value(d).unwrap_or(serde_json::Value::Null),
            );
        }
        value
    }
}

impl Timed {
    pub fn event(&self) -> Result<Event, String> {
        let set = [
            self.key.is_some(),
            self.text.is_some(),
            self.cue.is_some(),
            self.pointer.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if set != 1 {
            return Err(format!(
                "the input at {} s needs exactly one of key, text, cue or pointer",
                self.at
            ));
        }
        if let Some(key) = &self.key {
            return Ok(Event::Key {
                key: key.clone(),
                state: self.state.clone(),
            });
        }
        if let Some(text) = &self.text {
            return Ok(Event::Text { text: text.clone() });
        }
        if let Some(cue) = &self.cue {
            return Ok(Event::Cue { cue: cue.clone() });
        }
        let at = self.pointer.unwrap_or([0.0, 0.0]);
        Ok(Event::Pointer {
            at,
            button: self.button.clone(),
            state: self.state.clone(),
        })
    }
}

pub fn recipe(app: &str, name: &str, shot: &Shot) -> String {
    let body = serde_json::to_string(&(app, name, shot)).unwrap_or_default();
    hex::encode(Sha256::digest(body.as_bytes()))
}

pub fn seed(recipe: &str, base: u64, frame: u64, pass: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(recipe.as_bytes());
    h.update(base.to_le_bytes());
    h.update(frame.to_le_bytes());
    h.update(pass.to_le_bytes());
    let d = h.finalize();
    u64::from_le_bytes(d[..8].try_into().unwrap_or([0; 8]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOK: &str = r#"
[shots.at-rest]
line = "The desk at rest."
fixtures = "coast"
seconds = 11.0
fps = [60]
size = ["1080p", "2160p"]
closure = { crossfade = 0.5 }
app = { camera = "director" }

[[shots.at-rest.inputs]]
at = 1.5
cue = "deck:keep"
"#;

    #[test]
    fn a_shot_reads_with_its_matrix_and_master() {
        let book = book(BOOK).unwrap();
        let shot = &book["at-rest"];
        assert_eq!(shot.closure().unwrap(), Closure::Crossfade(0.5));
        assert_eq!(shot.targets().unwrap().len(), 2);
        assert_eq!(shot.master().unwrap(), (60, "2160p".to_string()));
        assert_eq!(shot.warmup, 256);
        assert_eq!(
            shot.inputs[0].event().unwrap(),
            Event::Cue {
                cue: "deck:keep".into()
            }
        );
        assert_eq!(shot.app_json()["camera"], "director");
    }

    #[test]
    fn seeds_follow_the_recipe_and_never_the_clock() {
        let r = recipe("alpha", "at-rest", &book(BOOK).unwrap()["at-rest"]);
        assert_eq!(seed(&r, 7, 3, 1), seed(&r, 7, 3, 1));
        assert_ne!(seed(&r, 7, 3, 1), seed(&r, 7, 4, 1));
        assert_ne!(seed(&r, 7, 3, 1), seed(&r, 7, 3, 2));
        assert_ne!(seed(&r, 7, 3, 1), seed(&r, 8, 3, 1));
    }

    #[test]
    fn daylight_fills_its_defaults_reaches_the_adapter_and_refuses_nonsense() {
        let shot = &book(BOOK).unwrap()["at-rest"];
        let mut lit = set(shot, &["daylight.hour=16.5".into()]).unwrap();
        daylight(&mut lit).unwrap();
        let sent = lit.app_json();
        assert_eq!(sent["camera"], "director");
        assert_eq!(sent["daylight"]["hour"], 16.5);
        assert_eq!(sent["daylight"]["day"], 172.0);
        assert_eq!(sent["daylight"]["latitude"], 45.0);
        assert_eq!(sent["daylight"]["heading"], 180.0);
        let mut plain = shot.clone();
        daylight(&mut plain).unwrap();
        assert!(plain.app_json().get("daylight").is_none());
        for bad in ["daylight.hour=25", "daylight.day=0", "daylight.moon=1"] {
            let mut s = set(shot, &[bad.to_string()]).unwrap();
            assert!(daylight(&mut s).is_err(), "{bad}");
        }
        let mut hourless = set(shot, &["daylight.day=100".into()]).unwrap();
        assert!(daylight(&mut hourless).is_err());
    }

    #[test]
    fn set_overrides_a_known_key_or_an_app_key_and_nothing_else() {
        let shot = &book(BOOK).unwrap()["at-rest"];
        let changed = set(
            shot,
            &[
                "per_frame=32".into(),
                "app.camera=still".into(),
                "line=A quiet desk.".into(),
            ],
        )
        .unwrap();
        assert_eq!(changed.per_frame, 32);
        assert_eq!(changed.app_json()["camera"], "still");
        assert_eq!(changed.line, "A quiet desk.");
        assert_eq!(changed.inputs, shot.inputs);
        assert!(set(shot, &["nope=1".into()]).is_err());
        assert!(set(shot, &["fps.x=1".into()]).is_err());
        assert!(set(shot, &["per_frame".into()]).is_err());
    }

    #[test]
    fn sizes_are_a_scale_of_one_logical_frame() {
        for name in ["720p", "1080p", "1440p", "2160p"] {
            let (w, h, s) = size(name).unwrap();
            assert_eq!(
                (
                    (LOGICAL.0 as f32 * s).round() as u32,
                    (LOGICAL.1 as f32 * s).round() as u32
                ),
                (w, h)
            );
        }
        assert_eq!(size("1920x1080").unwrap(), size("1080p").unwrap());
        assert_eq!(named("3840x2160").unwrap(), "2160p");
        assert!(size("4k").is_err());
        assert!(size("1920x1200").is_err());
        assert_eq!(rate(60).unwrap(), 60);
        assert!(rate(30).is_err());
        assert!(rate(120).is_err());
    }
}
