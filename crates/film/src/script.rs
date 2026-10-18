use crate::art::Art;
use crate::beat::{self, Grid, TempoPart};
use crate::camera::WIDE;
use crate::duck::{self, Duck, DuckPart, Side};
use crate::input::Touch;
use crate::layers::{Flash, Shake, Sound, Style, Text};
use crate::track::Key;
use crate::transition::Transition;
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
    pub duck: Option<Duck>,
    pub sides: Vec<Side>,
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
    pub ceiling: f32,
    pub limit: f32,
    pub true_peak: f32,
    pub beat: Option<Grid>,
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
    pub duck: Option<DuckPart>,
    pub score_db: f32,
    pub transition: Option<Transition>,
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
    ceiling: f32,
    limit: f32,
    true_peak: f32,
    bpm: Option<f64>,
    beats_per_bar: Option<u32>,
    signature: Option<Vec<u32>>,
    tempo: Vec<TempoPart>,
    offset: Option<f64>,
    snap: Option<String>,
    strict: Option<bool>,
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
            ceiling: CEILING,
            limit: LIMIT,
            true_peak: TRUE_PEAK,
            bpm: None,
            beats_per_bar: None,
            signature: None,
            tempo: Vec::new(),
            offset: None,
            snap: None,
            strict: None,
        }
    }
}

pub const CEILING: f32 = 0.89;
pub const LIMIT: f32 = 0.72;
pub const TRUE_PEAK: f32 = -1.0;

const FILM_KEYS: [&str; 20] = [
    "frames",
    "audio",
    "bars",
    "grain",
    "vignette",
    "glow",
    "seed",
    "theme",
    "safe",
    "layout",
    "ceiling",
    "limit",
    "true_peak",
    "bpm",
    "beats_per_bar",
    "signature",
    "tempo",
    "offset",
    "snap",
    "strict",
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
    #[serde(default)]
    duck: Option<DuckPart>,
    #[serde(default)]
    score: Option<ScorePart>,
    #[serde(default)]
    transition: Option<Transition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScorePart {
    gain_db: f32,
}

const SCENE_KEYS: [&str; 14] = [
    "name",
    "seconds",
    "from",
    "still",
    "key",
    "text",
    "art",
    "sound",
    "flash",
    "shake",
    "input",
    "duck",
    "score",
    "transition",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    film: toml::Table,
    #[serde(rename = "scene", default)]
    scenes: Vec<toml::Table>,
    #[serde(default)]
    style: BTreeMap<String, toml::Table>,
    #[serde(default)]
    duck: Option<toml::Table>,
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

pub fn cuts(starts: &[f64], fps: u32) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::with_capacity(starts.len());
    for t in starts {
        let at = (t * f64::from(fps)).round() as u64;
        out.push(match out.last() {
            Some(last) => at.max(last + 1),
            None => at,
        });
    }
    out
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
        let beat = match f.bpm {
            Some(bpm) => {
                let signature = match (&f.signature, f.beats_per_bar) {
                    (Some(_), Some(_)) => {
                        return Err("[film] signature or beats_per_bar, not both".into());
                    }
                    (Some(s), None) => beat::signature(s)?,
                    (None, n) => [n.unwrap_or(4), 4],
                };
                let mut grid = Grid::new(bpm, signature, f.offset.unwrap_or(0.0))?;
                grid.snap = f.snap.as_deref().map_or(Ok(Some(4)), beat::snap)?;
                grid.strict = f.strict.unwrap_or(false);
                for change in &f.tempo {
                    grid.change(change)?;
                }
                Some(grid)
            }
            None if f.beats_per_bar.is_some()
                || f.signature.is_some()
                || !f.tempo.is_empty()
                || f.offset.is_some()
                || f.snap.is_some()
                || f.strict.is_some() =>
            {
                return Err(
                    "[film] beats_per_bar, signature, tempo, offset, snap and strict need bpm"
                        .into(),
                );
            }
            None => None,
        };
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
            ceiling: f.ceiling,
            limit: f.limit,
            true_peak: f.true_peak,
            beat,
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
        if !(self.ceiling.is_finite() && self.ceiling > 0.1 && self.ceiling <= 1.0) {
            return Err("[film] ceiling is the master's peak in (0.1, 1]".into());
        }
        if !(self.limit.is_finite() && (0.1..1.0).contains(&self.limit)) {
            return Err("[film] limit is where the master's soft limit starts, in [0.1, 1)".into());
        }
        if !(self.true_peak.is_finite() && (-20.0..=0.0).contains(&self.true_peak)) {
            return Err("[film] true_peak is the master's ceiling in dBTP, in [-20, 0]".into());
        }
        K::check_film(&self.app)
    }
}

impl<K: Kinds> Scene<K> {
    fn read(mut table: toml::Table, grid: Option<&Grid>, start: f64) -> Result<Scene<K>, String> {
        let name = table
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let at = |e: String| format!("scene '{name}': {e}");
        beat::scene(&mut table, grid, start).map_err(at)?;
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
            duck: s.duck,
            score_db: s.score.map_or(0.0, |p| p.gain_db),
            transition: s.transition,
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
        if let Some(d) = &self.duck {
            d.check().map_err(at)?;
        }
        if !(self.score_db.is_finite() && (-60.0..=12.0).contains(&self.score_db)) {
            return Err(at("[scene.score] gain_db is in [-60, 12]".into()));
        }
        if let Some(t) = &self.transition {
            t.check(self.seconds).map_err(at)?;
        }
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
        let film = Film::read(raw.film)?;
        film.check()?;
        let grid = film.beat.as_ref();
        let mut styles = BTreeMap::new();
        for (name, mut table) in raw.style {
            let at = |e: String| format!("style '{name}': {e}");
            if !plain(&name) {
                return Err(at("a name is letters, digits, '-' and '_'".into()));
            }
            beat::convert(&mut table, &beat::STYLE, grid, 0.0).map_err(at)?;
            let style: Style = read(table).map_err(at)?;
            style.check().map_err(at)?;
            styles.insert(name, style);
        }
        let (duck, sides) = match raw.duck {
            Some(table) => duck::read_all(table, known.sounds)?,
            None => (None, Vec::new()),
        };
        let mut scenes: Vec<Scene<K>> = Vec::with_capacity(raw.scenes.len());
        let mut start = 0.0;
        for table in raw.scenes {
            let scene = Scene::read(table, grid, start)?;
            start += scene.seconds;
            scenes.push(scene);
        }
        if scenes[0].transition.is_some() {
            return Err(format!(
                "scene '{}': the first scene has nothing to come from, so no [scene.transition]",
                scenes[0].name
            ));
        }
        for (i, scene) in scenes.iter().enumerate() {
            scene.check(&styles, known)?;
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
            style: styles,
            duck,
            sides,
        };
        K::check_script(&script)?;
        Ok(script)
    }

    pub fn frames(&self, fps: u32) -> u64 {
        self.cuts(fps).last().copied().unwrap_or(0)
    }

    pub fn starts(&self) -> Vec<f64> {
        let mut out = Vec::with_capacity(self.scenes.len() + 1);
        let mut start = 0.0;
        out.push(start);
        for s in &self.scenes {
            start += s.seconds;
            out.push(start);
        }
        out
    }

    pub fn cuts(&self, fps: u32) -> Vec<u64> {
        cuts(&self.starts(), fps)
    }

    pub fn seconds(&self) -> f64 {
        self.scenes.iter().map(|s| s.seconds).sum()
    }

    pub fn off_grid(&self, fps: u32) -> Result<Vec<String>, String> {
        let Some(grid) = &self.film.beat else {
            return Ok(Vec::new());
        };
        let misses = beat::misses(grid, &self.scenes, fps);
        match misses.first() {
            Some(first) if grid.strict => Err(first.clone()),
            _ => Ok(misses),
        }
    }

    pub fn stills(&self) -> usize {
        self.scenes.iter().filter(|s| s.still.is_some()).count()
    }
}
