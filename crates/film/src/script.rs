use crate::art::Art;
use crate::camera::WIDE;
use crate::input::Touch;
use crate::layers::{Flash, Shake, Sound, Style, Text};
use crate::track::Key;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::fmt::Debug;

pub const MAX_SECONDS: f64 = 600.0;

pub trait Drawn: Clone + Debug + PartialEq {
    fn anchor(&self) -> Option<[f32; 2]>;
    fn flat(&self) -> bool;
}

pub trait Kinds: Sized + Clone + Debug + PartialEq + 'static {
    type Scene: DeserializeOwned + Clone + Debug + PartialEq;
    type Film: DeserializeOwned + Clone + Debug + PartialEq + Default;
    type Art: DeserializeOwned + Clone + Debug + PartialEq;
    type Drawn: Drawn;
    const ART: &'static [&'static str];

    fn check_film(_film: &Self::Film) -> Result<(), String> {
        Ok(())
    }

    fn check_scene(_scene: &Scene<Self>) -> Result<(), String> {
        Ok(())
    }

    fn check_art(_art: &Art<Self::Art>) -> Result<(), String> {
        Ok(())
    }

    fn check_script(_script: &Script<Self>) -> Result<(), String> {
        Ok(())
    }

    fn shown(_scene: &Scene<Self>) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Light,
    Dark,
}

pub struct Known<'a> {
    pub sounds: &'a dyn Fn(&str) -> bool,
    pub marks: &'a [&'a str],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Script<K: Kinds> {
    pub film: Film<K>,
    pub scenes: Vec<Scene<K>>,
    pub style: BTreeMap<String, Style>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Film<K: Kinds> {
    pub frames: bool,
    pub audio: bool,
    pub bars: f32,
    pub grain: f32,
    pub vignette: f32,
    pub glow: f32,
    pub seed: u64,
    pub theme: Option<Scheme>,
    pub safe: [f32; 2],
    pub layout: [f32; 2],
    pub app: K::Film,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene<K: Kinds> {
    pub name: String,
    pub seconds: f64,
    pub from: f64,
    pub still: Option<String>,
    pub key: Vec<Key>,
    pub text: Vec<Text>,
    pub art: Vec<Art<K::Art>>,
    pub sound: Vec<Sound>,
    pub flash: Vec<Flash>,
    pub shake: Vec<Shake>,
    pub input: Vec<Touch>,
    pub app: K::Scene,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FilmPart {
    frames: bool,
    audio: bool,
    bars: f32,
    grain: f32,
    vignette: f32,
    glow: f32,
    seed: u64,
    theme: Option<Scheme>,
    safe: [f32; 2],
    layout: [f32; 2],
}

impl Default for FilmPart {
    fn default() -> FilmPart {
        FilmPart {
            frames: true,
            audio: true,
            bars: 0.0,
            grain: 0.0,
            vignette: 0.0,
            glow: 0.0,
            seed: 1,
            theme: None,
            safe: [0.0; 2],
            layout: WIDE,
        }
    }
}

const FILM_KEYS: [&str; 10] = [
    "frames", "audio", "bars", "grain", "vignette", "glow", "seed", "theme", "safe", "layout",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenePart {
    name: String,
    seconds: f64,
    #[serde(default)]
    from: f64,
    #[serde(default)]
    still: Option<String>,
    #[serde(default)]
    key: Vec<Key>,
    #[serde(default)]
    text: Vec<Text>,
    #[serde(default)]
    art: Vec<toml::Table>,
    #[serde(default)]
    sound: Vec<Sound>,
    #[serde(default)]
    flash: Vec<Flash>,
    #[serde(default)]
    shake: Vec<Shake>,
    #[serde(default)]
    input: Vec<Touch>,
}

const SCENE_KEYS: [&str; 11] = [
    "name", "seconds", "from", "still", "key", "text", "art", "sound", "flash", "shake", "input",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    film: toml::Table,
    #[serde(rename = "scene", default)]
    scenes: Vec<toml::Table>,
    #[serde(default)]
    style: BTreeMap<String, Style>,
}

pub fn split(mut table: toml::Table, keys: &[&str]) -> (toml::Table, toml::Table) {
    let mut ours = toml::Table::new();
    for k in keys {
        if let Some(v) = table.remove(*k) {
            ours.insert((*k).to_string(), v);
        }
    }
    (ours, table)
}

pub fn read<T: DeserializeOwned>(table: toml::Table) -> Result<T, String> {
    T::deserialize(table).map_err(|e| e.to_string().trim().to_string())
}

pub fn frames(seconds: f64, fps: u32) -> u64 {
    ((seconds * f64::from(fps)).round() as u64).max(1)
}

pub fn plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl<K: Kinds> Film<K> {
    fn read(table: toml::Table) -> Result<Film<K>, String> {
        let (ours, rest) = split(table, &FILM_KEYS);
        let f: FilmPart = read(ours).map_err(|e| format!("[film] {e}"))?;
        let app: K::Film = read(rest).map_err(|e| format!("[film] {e}"))?;
        Ok(Film {
            frames: f.frames,
            audio: f.audio,
            bars: f.bars,
            grain: f.grain,
            vignette: f.vignette,
            glow: f.glow,
            seed: f.seed,
            theme: f.theme,
            safe: f.safe,
            layout: f.layout,
            app,
        })
    }

    fn check(&self) -> Result<(), String> {
        let ok = |v: f32, high: f32| v.is_finite() && (0.0..=high).contains(&v);
        if !ok(self.bars, 4.0) {
            return Err("[film] bars is an aspect in [0, 4], 0 for none".into());
        }
        if !(ok(self.grain, 2.0) && ok(self.vignette, 2.0) && ok(self.glow, 2.0)) {
            return Err("[film] grain, vignette and glow are in [0, 2]".into());
        }
        if !(self.safe.iter().all(|v| ok(*v, 0.4)) && self.safe[0] + self.safe[1] < 0.6) {
            return Err("[film] safe is the top and bottom in [0, 0.4], under 0.6 together".into());
        }
        if !self
            .layout
            .iter()
            .all(|v| v.is_finite() && (16.0..=16_384.0).contains(v))
        {
            return Err("[film] layout is a width and height in [16, 16384]".into());
        }
        K::check_film(&self.app)
    }
}

impl<K: Kinds> Scene<K> {
    fn read(table: toml::Table) -> Result<Scene<K>, String> {
        let name = table
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let at = |e: String| format!("scene '{name}': {e}");
        let (ours, rest) = split(table, &SCENE_KEYS);
        let s: ScenePart = read(ours).map_err(at)?;
        let app: K::Scene = read(rest).map_err(at)?;
        let art = s
            .art
            .into_iter()
            .map(Art::read::<K>)
            .collect::<Result<Vec<_>, String>>()
            .map_err(at)?;
        Ok(Scene {
            name: s.name,
            seconds: s.seconds,
            from: s.from,
            still: s.still,
            key: s.key,
            text: s.text,
            art,
            sound: s.sound,
            flash: s.flash,
            shake: s.shake,
            input: s.input,
            app,
        })
    }

    fn check(&self, styles: &BTreeMap<String, Style>, known: &Known<'_>) -> Result<(), String> {
        let at = |why: String| format!("scene '{}': {why}", self.name);
        if !plain(&self.name) {
            return Err(at("a name is letters, digits, '-' and '_'".into()));
        }
        if !(self.seconds.is_finite() && self.seconds > 0.0 && self.seconds <= MAX_SECONDS) {
            return Err(at(format!("seconds is in (0, {MAX_SECONDS}]")));
        }
        if !self.from.is_finite() || self.from.abs() > MAX_SECONDS * 10.0 {
            return Err(at("from is a number of seconds".into()));
        }
        if let Some(still) = &self.still
            && !plain(still)
        {
            return Err(at("a still's name is letters, digits, '-' and '_'".into()));
        }
        crate::input::check(&self.input).map_err(at)?;
        K::check_scene(self).map_err(at)?;
        self.check_layers(styles, known).map_err(at)
    }

    fn check_layers(
        &self,
        styles: &BTreeMap<String, Style>,
        known: &Known<'_>,
    ) -> Result<(), String> {
        let s = self.seconds;
        for k in &self.key {
            k.check(s)?;
        }
        for t in &self.text {
            t.check(s)?;
            if let Some(name) = &t.style
                && !styles.contains_key(name)
            {
                return Err(format!("no [style.{name}]"));
            }
        }
        for a in &self.art {
            a.check(s, known.marks)?;
            K::check_art(a)?;
        }
        for sound in &self.sound {
            sound.check(s, known.sounds)?;
        }
        for f in &self.flash {
            f.check(s)?;
        }
        for k in &self.shake {
            k.check(s)?;
        }
        Ok(())
    }
}

impl<K: Kinds> Script<K> {
    pub fn parse(text: &str, known: &Known<'_>) -> Result<Script<K>, String> {
        let raw: Raw = toml::from_str(text).map_err(|e| e.to_string())?;
        if raw.scenes.is_empty() {
            return Err("a film has at least one [[scene]]".into());
        }
        for (name, style) in &raw.style {
            if !plain(name) {
                return Err(format!(
                    "style '{name}': a name is letters, digits, '-' and '_'"
                ));
            }
            style.check().map_err(|e| format!("style '{name}': {e}"))?;
        }
        let film = Film::read(raw.film)?;
        film.check()?;
        let scenes = raw
            .scenes
            .into_iter()
            .map(Scene::read)
            .collect::<Result<Vec<_>, String>>()?;
        for (i, scene) in scenes.iter().enumerate() {
            scene.check(&raw.style, known)?;
            if scenes[..i].iter().any(|s| s.name == scene.name) {
                return Err(format!("scene '{}' is named twice", scene.name));
            }
            let still = scene.still.as_ref();
            if still.is_some() && scenes[..i].iter().any(|s| s.still.as_ref() == still) {
                return Err(format!("scene '{}': its still is named twice", scene.name));
            }
        }
        let script = Script {
            film,
            scenes,
            style: raw.style,
        };
        K::check_script(&script)?;
        Ok(script)
    }

    pub fn frames(&self, fps: u32) -> u64 {
        self.scenes.iter().map(|s| frames(s.seconds, fps)).sum()
    }

    pub fn seconds(&self) -> f64 {
        self.scenes.iter().map(|s| s.seconds).sum()
    }

    pub fn stills(&self) -> usize {
        self.scenes.iter().filter(|s| s.still.is_some()).count()
    }
}
