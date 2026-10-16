#[cfg(feature = "video")]
mod clip;
pub mod footage;
pub mod paint;
pub mod wav;

use crate::colour::hex;
use crate::cut::{Cast, Posed};
use crate::marks::{Check, Marks};
use crate::overlay::Overlay;
use crate::score::Score;
use crate::script::{Drawn, Kinds, Known, Scene, Scheme, Script};
use crate::shoot::{Crew, RATE, Reeled, Rig, shoot};
use crate::show::{Cue, FilmFrame, Show, Take};
use crate::write::{AUDIO, Encoder, FRAMES, Files};
use footage::{Card, Folder, Sequence, Source, sample};
use paint::{Backdrop, Font, Painter};
use pfx_gpu::Gpu;
use pfx_input::InputEvent;
use pfx_live::flat::{Icons, Srgba};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const FPS: u32 = 60;
pub const SIZE: [u32; 2] = [1920, 1080];
pub const MOST_FPS: u32 = 240;
pub const SIDES: [u32; 2] = [16, 16_384];
pub const LOUDEST: f32 = 4.0;
const BLACK: Srgba = Srgba([0.0, 0.0, 0.0, 1.0]);

#[derive(Clone, Debug, PartialEq)]
pub struct Reel;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bare {}

#[derive(Clone, Debug, PartialEq)]
pub struct Blank;

impl Drawn for Blank {
    fn anchor(&self) -> Option<[f32; 2]> {
        None
    }

    fn flat(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Clip,
    Frames,
    Card,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Clip => "clip",
            Kind::Frames => "frames",
            Kind::Card => "card",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    #[default]
    Fit,
    Fill,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footage {
    pub kind: Kind,
    pub file: Option<String>,
    pub dir: Option<String>,
    #[serde(default)]
    pub fit: Fit,
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default)]
    pub track: usize,
    pub colour: Option<String>,
    pub gradient: Option<[String; 2]>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Setup {
    pub size: [u32; 2],
    pub fps: u32,
    pub score: Option<String>,
    pub font: Option<String>,
    pub background: Option<String>,
    pub check: Option<String>,
    pub check_colour: Option<String>,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup {
            size: SIZE,
            fps: FPS,
            score: None,
            font: None,
            background: None,
            check: None,
            check_colour: None,
        }
    }
}

fn colour_ok(text: &Option<String>, key: &str) -> Result<(), String> {
    match text {
        Some(t) => hex(t).map(|_| ()).map_err(|e| format!("{key}: {e}")),
        None => Ok(()),
    }
}

impl Kinds for Reel {
    type Scene = Footage;
    type Film = Setup;
    type Art = Bare;
    type Drawn = Blank;
    const ART: &'static [&'static str] = &[];

    fn check_film(film: &Setup) -> Result<(), String> {
        let [low, high] = SIDES;
        if !film
            .size
            .iter()
            .all(|s| (low..=high).contains(s) && s % 2 == 0)
        {
            return Err(format!(
                "[film] size is an even width and height in [{low}, {high}]"
            ));
        }
        if !(1..=MOST_FPS).contains(&film.fps) {
            return Err(format!("[film] fps is in [1, {MOST_FPS}]"));
        }
        colour_ok(&film.background, "[film] background")?;
        colour_ok(&film.check_colour, "[film] check_colour")
    }

    fn check_scene(scene: &Scene<Reel>) -> Result<(), String> {
        let f = &scene.app;
        match (f.kind, &f.file, &f.dir) {
            (Kind::Clip, Some(_), None)
            | (Kind::Frames, None, Some(_))
            | (Kind::Card, None, None) => {}
            (Kind::Clip, ..) => return Err("a clip names its video in file, and no dir".into()),
            (Kind::Frames, ..) => {
                return Err("a frames scene names its folder in dir, and no file".into());
            }
            (Kind::Card, ..) => return Err("a card takes no file or dir".into()),
        }
        if !(f.volume.is_finite() && (0.0..=LOUDEST).contains(&f.volume)) {
            return Err(format!("volume is in [0, {LOUDEST}]"));
        }
        if f.colour.is_some() && f.gradient.is_some() {
            return Err("a scene's backdrop is a colour or a gradient, not both".into());
        }
        colour_ok(&f.colour, "colour")?;
        for g in f.gradient.iter().flatten() {
            hex(g).map_err(|e| format!("gradient: {e}"))?;
        }
        Ok(())
    }
}

pub struct Words;

impl Cast<Reel> for Words {
    fn fill(&self, text: &str) -> String {
        text.to_string()
    }

    fn pose(&self, _art: &Bare, _posed: Posed) -> Option<(Blank, f32)> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct Start(pub u64);

impl Cue for Start {
    fn at(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub name: String,
    pub kind: Kind,
    pub seconds: f64,
    pub from: f64,
    pub frames: u64,
    pub source: Option<PathBuf>,
}

pub struct Roll {
    pub script: Script<Reel>,
    pub path: PathBuf,
    base: PathBuf,
    marks: Vec<(String, String)>,
    score: String,
    font: Option<Vec<u8>>,
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn mark_files(raw: &toml::Table) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let scenes = raw.get("scene").and_then(|s| s.as_array());
    for scene in scenes.into_iter().flatten() {
        let arts = scene.get("art").and_then(|a| a.as_array());
        for art in arts.into_iter().flatten() {
            let mark = art.get("kind").and_then(|k| k.as_str()) == Some("mark");
            if let (true, Some(file)) = (mark, art.get("file").and_then(|f| f.as_str()))
                && !out.iter().any(|f| f == file)
            {
                out.push(file.to_string());
            }
        }
    }
    out
}

fn film_path(raw: &toml::Table, key: &str) -> Option<String> {
    raw.get("film")?.get(key)?.as_str().map(str::to_string)
}

pub fn family(font: &[u8]) -> Result<String, String> {
    let face = swash::FontRef::from_index(font, 0).ok_or("the [film] font is not a font")?;
    face.localized_strings()
        .find_by_id(swash::StringId::Family, None)
        .map(|s| s.chars().collect::<String>())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "the [film] font names no family".to_string())
}

impl Roll {
    pub fn open(path: &Path) -> Result<Roll, String> {
        let text = read_text(path)?;
        let base = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let at = |e: String| format!("{}: {e}", path.display());
        let raw: toml::Table = toml::from_str(&text).map_err(|e| at(e.to_string()))?;
        let marks = mark_files(&raw)
            .into_iter()
            .map(|name| Ok((name.clone(), read_text(&base.join(&name))?)))
            .collect::<Result<Vec<_>, String>>()?;
        let score = match film_path(&raw, "score") {
            Some(p) => read_text(&base.join(p))?,
            None => String::new(),
        };
        let font = match film_path(&raw, "font") {
            Some(p) => {
                let file = base.join(p);
                Some(std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?)
            }
            None => None,
        };
        let known_score = Score::new(&score, &[], 1, RATE).map_err(at)?;
        let sounds = |name: &str| known_score.knows(name);
        let names: Vec<&str> = marks.iter().map(|(n, _)| n.as_str()).collect();
        let known = Known {
            sounds: &sounds,
            marks: &names,
        };
        let script: Script<Reel> = Script::parse(&text, &known).map_err(at)?;
        if let Some(font) = &font {
            family(font).map_err(at)?;
        }
        let roll = Roll {
            script,
            path: path.to_path_buf(),
            base,
            marks,
            score,
            font,
        };
        roll.check_sources().map_err(at)?;
        Ok(roll)
    }

    fn source(&self, scene: &Scene<Reel>) -> Option<PathBuf> {
        let f = &scene.app;
        f.file
            .as_ref()
            .or(f.dir.as_ref())
            .map(|p| self.base.join(p))
    }

    fn check_sources(&self) -> Result<(), String> {
        for scene in &self.script.scenes {
            let at = |e: &str| format!("scene '{}': {e}", scene.name);
            let source = self.source(scene);
            match scene.app.kind {
                Kind::Clip if !cfg!(feature = "video") => {
                    return Err(at(
                        "this build plays no clips (pfx-film's video feature is off)",
                    ));
                }
                Kind::Clip if !source.as_ref().is_some_and(|p| p.is_file()) => {
                    return Err(at(&format!(
                        "no video at {}",
                        source.unwrap_or_default().display()
                    )));
                }
                Kind::Frames => {
                    Sequence::find(&source.unwrap_or_default(), self.fps()).map_err(|e| at(&e))?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn size(&self) -> [u32; 2] {
        self.script.film.app.size
    }

    pub fn fps(&self) -> u32 {
        self.script.film.app.fps
    }

    pub fn frames(&self) -> u64 {
        self.script.frames(self.fps())
    }

    pub fn plan(&self) -> Vec<Planned> {
        self.script
            .scenes
            .iter()
            .map(|scene| Planned {
                name: scene.name.clone(),
                kind: scene.app.kind,
                seconds: scene.seconds,
                from: scene.from,
                frames: crate::script::frames(scene.seconds, self.fps()),
                source: self.source(scene),
            })
            .collect()
    }

    fn marks(&self) -> Result<(Marks, u32), String> {
        let files: Vec<(&str, &str)> = self
            .marks
            .iter()
            .map(|(n, s)| (n.as_str(), s.as_str()))
            .collect();
        let film = &self.script.film.app;
        let mut check = Check::default();
        if let Some(word) = &film.check {
            check.word = word.clone();
        }
        if let Some(c) = &film.check_colour {
            check.colour = hex(c)?;
        }
        let mut marks = Marks::load(0, &files)?.with_check(check);
        let tick = marks.shapes.len() as u32;
        marks.shapes.extend(paint::ticks());
        Ok((marks, tick))
    }

    pub fn roll(&self, encoder: Option<&mut dyn Encoder>) -> Result<Reeled, String> {
        let gpu = pollster::block_on(Gpu::headless())?;
        let (marks, tick) = self.marks()?;
        let icons = Icons::bake(&marks.shapes)?;
        let font = match &self.font {
            Some(bytes) => Some(Font {
                engine: pfx_text::TextEngine::new(bytes)
                    .map_err(|e| format!("the [film] font: {e:?}"))?,
                family: family(bytes)?,
            }),
            None => None,
        };
        let seed = self.script.film.seed as u32;
        let painter = Painter::new(gpu, self.size(), icons, font, tick, seed)?;
        let mut player = Player {
            roll: self,
            painter,
            overlay: Overlay::new(marks),
            playing: None,
            fit: Fit::Fit,
            backdrop: Backdrop::Solid(BLACK),
            volume: 1.0,
            from: 0.0,
            clock: 0.0,
            heard: Vec::new(),
            frame: Vec::new(),
        };
        let mut score = Score::new(&self.score, &[], self.script.film.seed, RATE)?;
        let crew = Crew {
            show: &mut player,
            cast: &Words,
            score: &mut score,
            encoder: encoder.map(|e| e as &mut dyn Encoder),
        };
        let rig = Rig {
            size: self.size(),
            fps: self.fps(),
        };
        shoot(&self.script, rig, crew)
    }

    pub fn shoot(&self, out: &Path) -> Result<Reeled, String> {
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let film = &self.script.film;
        let mut files = Files::open(out, film.frames, self.script.stills() > 0)?;
        self.roll(Some(&mut files))
    }

    pub fn pcod(&self, out: &Path, mp4: &Path) -> Vec<String> {
        let mut args = vec![
            "pcod".to_string(),
            "frames".to_string(),
            out.join(FRAMES).display().to_string(),
        ];
        if self.script.film.audio {
            args.extend(["--audio".to_string(), out.join(AUDIO).display().to_string()]);
        }
        args.extend([
            "--fps".to_string(),
            self.fps().to_string(),
            mp4.display().to_string(),
        ]);
        args
    }
}

struct Player<'a> {
    roll: &'a Roll,
    painter: Painter,
    overlay: Overlay,
    playing: Option<Box<dyn Source>>,
    fit: Fit,
    backdrop: Backdrop,
    volume: f32,
    from: f64,
    clock: f64,
    heard: Vec<f32>,
    frame: Vec<u8>,
}

fn backdrop(footage: &Footage, film: &Setup) -> Backdrop {
    let pick = |t: &String| hex(t).unwrap_or(BLACK);
    match (&footage.gradient, &footage.colour, &film.background) {
        (Some([top, bottom]), ..) => Backdrop::Fall(pick(top), pick(bottom)),
        (None, Some(c), _) | (None, None, Some(c)) => Backdrop::Solid(pick(c)),
        (None, None, None) => Backdrop::Solid(BLACK),
    }
}

#[cfg(feature = "video")]
fn clip(path: &Path, from: f64, track: usize) -> Result<Box<dyn Source>, String> {
    Ok(Box::new(clip::Clip::open(path, from, track)?))
}

#[cfg(not(feature = "video"))]
fn clip(_path: &Path, _from: f64, _track: usize) -> Result<Box<dyn Source>, String> {
    Err("this build plays no clips (pfx-film's video feature is off)".into())
}

impl Show for Player<'_> {
    type Kinds = Reel;
    type Cue = Start;

    fn plan(&mut self, script: &Script<Reel>) -> Result<Vec<Start>, String> {
        Ok(script
            .scenes
            .iter()
            .map(|s| Start(sample(s.from)))
            .collect())
    }

    fn set_theme(&mut self, _scheme: Option<Scheme>) {}

    fn seek(&mut self, scene: &Scene<Reel>, _take: &Take<Start>) -> Result<(), String> {
        let f = &scene.app;
        let path = self.roll.source(scene).unwrap_or_default();
        self.playing = Some(match f.kind {
            Kind::Clip => clip(&path, scene.from, f.track)?,
            Kind::Frames => Box::new(Folder::open(Sequence::find(&path, self.roll.fps())?)?),
            Kind::Card => Box::new(Card),
        });
        self.fit = f.fit;
        self.backdrop = backdrop(f, &self.roll.script.film.app);
        self.volume = f.volume;
        self.from = scene.from;
        self.clock = 0.0;
        Ok(())
    }

    fn aim(&self, _scene: &Scene<Reel>) -> Option<[f32; 2]> {
        None
    }

    fn tick(&mut self, _frame: u64, _input: &[InputEvent]) {}

    fn draw(&mut self, frame: &FilmFrame<'_, Blank>) -> Result<(), String> {
        let source = self.playing.as_mut().ok_or("no scene was sought")?;
        let at = self.from + self.clock;
        let next = at + frame.seconds();
        let (start, end) = (sample(at), sample(next));
        let from = self.heard.len();
        source.audio(start, (end - start) as usize, &mut self.heard)?;
        if self.volume != 1.0 {
            let volume = self.volume;
            self.heard[from..].iter_mut().for_each(|s| *s *= volume);
        }
        self.clock += frame.seconds();
        let picture = source.picture(at)?;
        self.frame =
            self.painter
                .paint(frame, &mut self.overlay, picture, self.fit, self.backdrop)?;
        Ok(())
    }

    fn audio(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.heard);
    }

    fn readback(&mut self) -> Result<Vec<u8>, String> {
        Ok(std::mem::take(&mut self.frame))
    }
}
