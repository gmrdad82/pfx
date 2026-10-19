#[cfg(feature = "video")]
mod clip;
pub mod footage;
pub mod pack;
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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const FPS: u32 = 60;
pub const SIZE: [u32; 2] = [1920, 1080];
pub const MOST_FPS: u32 = 240;
pub const SIDES: [u32; 2] = [16, 16_384];
pub const LOUDEST: f32 = 4.0;
pub const MOST_BLUR: f32 = 400.0;
const SILENT_SCORE: &str = "limiter = 1.0\nmaster = 1.0\nrecipe = {}\n";
const BLACK: Srgba = Srgba([0.0, 0.0, 0.0, 1.0]);

#[derive(Clone, Debug, PartialEq)]
pub struct Reel;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub kind: String,
    pub file: String,
    pub width: Option<f32>,
    pub height: Option<f32>,
    #[serde(default)]
    pub fit: Fit,
    pub from: Option<f64>,
    pub radius: Option<f32>,
    pub shadow: Option<DropShadow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropShadow {
    #[serde(default)]
    pub offset: [f32; 2],
    #[serde(default)]
    pub blur: f32,
    #[serde(default = "half")]
    pub alpha: f32,
}

fn half() -> f32 {
    0.5
}

#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
    pub path: PathBuf,
    pub from: f64,
    pub at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub centre: [f32; 2],
    pub size: [f32; 2],
    pub uv: [f32; 4],
    pub radius: f32,
    pub shadow: Option<DropShadow>,
    pub clip: Option<Playing>,
    pub page: usize,
}

impl Drawn for Shown {
    fn anchor(&self) -> Option<[f32; 2]> {
        Some(self.centre)
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
    pub crop: Option<[f32; 4]>,
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
    pub dither: bool,
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
            dither: true,
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
    type Art = Image;
    type Drawn = Shown;
    const ART: &'static [&'static str] = &["image", "clip"];

    fn check_art(art: &crate::art::Art<Image>) -> Result<(), String> {
        let crate::art::What::App(image) = &art.what else {
            return Ok(());
        };
        let what = image.kind.as_str();
        let side =
            |v: Option<f32>| v.is_none_or(|v| v.is_finite() && v > 0.0 && v <= SIDES[1] as f32);
        if !(side(image.width) && side(image.height)) {
            return Err(format!(
                "an {what}'s width and height are in (0, {}]",
                SIDES[1]
            ));
        }
        match image.from {
            Some(_) if what != "clip" => {
                return Err("from is a clip art's start in its video; an image has none".into());
            }
            Some(f) if !(f.is_finite() && f >= 0.0) => {
                return Err("a clip art's from is 0 or more seconds into its video".into());
            }
            _ => {}
        }
        if image
            .radius
            .is_some_and(|r| !(r.is_finite() && (0.0..=SIDES[1] as f32).contains(&r)))
        {
            return Err(format!("an {what}'s radius is in [0, {}]", SIDES[1]));
        }
        if let Some(d) = image.shadow {
            let finite = d
                .offset
                .iter()
                .all(|v| v.is_finite() && v.abs() <= SIDES[1] as f32);
            if !(finite
                && d.blur.is_finite()
                && (0.0..=MOST_BLUR).contains(&d.blur)
                && (0.0..=1.0).contains(&d.alpha))
            {
                return Err(format!(
                    "an {what}'s shadow is an offset in layout units, a blur in [0, {MOST_BLUR}] and an alpha in [0, 1]"
                ));
            }
        }
        Ok(())
    }

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
        if let Some([x, y, w, h]) = f.crop {
            if f.kind == Kind::Card {
                return Err("a card has no footage to crop".into());
            }
            let finite = [x, y, w, h].iter().all(|v| v.is_finite());
            if !(finite && x >= 0.0 && y >= 0.0 && w >= 1.0 && h >= 1.0) {
                return Err("crop is [x, y, width, height] in source pixels, x and y 0 or more, the sides 1 or more".into());
            }
        }
        colour_ok(&f.colour, "colour")?;
        for g in f.gradient.iter().flatten() {
            hex(g).map_err(|e| format!("gradient: {e}"))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub uv: [f32; 4],
    pub size: [u32; 2],
    pub page: usize,
}

#[derive(Default)]
pub struct Words {
    pub images: HashMap<String, Slot>,
    pub clips: HashMap<String, (PathBuf, [u32; 2])>,
}

pub fn posed(image: &Image, slot: Slot, centre: [f32; 2]) -> Shown {
    let natural = [slot.size[0].max(1) as f32, slot.size[1].max(1) as f32];
    let aspect = natural[0] / natural[1];
    let boxed = match (image.width, image.height) {
        (Some(w), Some(h)) => [w, h],
        (Some(w), None) => [w, w / aspect],
        (None, Some(h)) => [h * aspect, h],
        (None, None) => natural,
    };
    let [u0, v0, u1, v1] = slot.uv;
    let (size, uv) = match (image.fit, image.width.zip(image.height)) {
        (Fit::Fit, Some(_)) => {
            let s = (boxed[0] / natural[0]).min(boxed[1] / natural[1]);
            ([natural[0] * s, natural[1] * s], slot.uv)
        }
        (Fit::Fill, Some(_)) => {
            let s = (boxed[0] / natural[0]).max(boxed[1] / natural[1]);
            let keep = [boxed[0] / (natural[0] * s), boxed[1] / (natural[1] * s)];
            let mid = [(u0 + u1) * 0.5, (v0 + v1) * 0.5];
            let half = [(u1 - u0) * keep[0] * 0.5, (v1 - v0) * keep[1] * 0.5];
            (
                boxed,
                [
                    mid[0] - half[0],
                    mid[1] - half[1],
                    mid[0] + half[0],
                    mid[1] + half[1],
                ],
            )
        }
        (_, None) => (boxed, slot.uv),
    };
    Shown {
        centre,
        size,
        uv,
        radius: image.radius.unwrap_or(0.0),
        shadow: image.shadow,
        clip: None,
        page: slot.page,
    }
}

impl Cast<Reel> for Words {
    fn fill(&self, text: &str) -> String {
        text.to_string()
    }

    fn pose(&self, art: &Image, posed_at: Posed) -> Option<(Shown, f32)> {
        if art.kind == "clip" {
            let (path, size) = self.clips.get(&art.file)?;
            let slot = Slot {
                uv: [0.0, 0.0, 1.0, 1.0],
                size: *size,
                page: 0,
            };
            let mut shown = posed(art, slot, posed_at.centre);
            let from = art.from.unwrap_or(0.0);
            shown.clip = Some(Playing {
                path: path.clone(),
                from,
                at: from + posed_at.age.max(0.0),
            });
            return Some((shown, 1.0));
        }
        let slot = *self.images.get(&art.file)?;
        Some((posed(art, slot, posed_at.centre), 1.0))
    }
}

const GUTTER: u32 = 2;
const ATLAS_SIDE: u32 = 8192;
const MOST_PAGES: usize = 4;

pub struct Page {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}

pub struct Atlas {
    pub pages: Vec<Page>,
    pub slots: HashMap<String, Slot>,
}

pub fn atlas(images: &[(String, [u32; 2], Vec<u8>)]) -> Result<Option<Atlas>, String> {
    atlas_in(images, ATLAS_SIDE, MOST_PAGES)
}

pub fn atlas_in(
    images: &[(String, [u32; 2], Vec<u8>)],
    side: u32,
    most_pages: usize,
) -> Result<Option<Atlas>, String> {
    if images.is_empty() {
        return Ok(None);
    }
    let boxes: Vec<[u32; 2]> = images
        .iter()
        .map(|(_, [w, h], _)| [w + GUTTER * 2, h + GUTTER * 2])
        .collect();
    let placed = pack::pack(&boxes, side, most_pages).map_err(|i| {
        let (name, [w, h], _) = &images[i];
        let most = side - GUTTER * 2;
        if *w > most || *h > most {
            format!("the image '{name}' is {w}x{h}; a film's image is at most {most}x{most}")
        } else {
            format!(
                "the image '{name}' ({w}x{h}) does not fit: the film's {} images fill {most_pages} atlas pages of {side}x{side}",
                images.len()
            )
        }
    })?;
    let count = placed.iter().map(|p| p.page + 1).max().unwrap_or(1);
    let mut sizes = vec![[1u32; 2]; count];
    for (p, b) in placed.iter().zip(&boxes) {
        let size = &mut sizes[p.page];
        size[0] = size[0].max(p.at[0] + b[0]);
        size[1] = size[1].max(p.at[1] + b[1]);
    }
    let mut pages: Vec<Page> = sizes
        .iter()
        .map(|size| Page {
            size: *size,
            rgba: vec![0u8; (size[0] * size[1] * 4) as usize],
        })
        .collect();
    let mut slots = HashMap::new();
    for ((name, [w, h], pixels), p) in images.iter().zip(&placed) {
        let (w, h) = (*w, *h);
        let page = &mut pages[p.page];
        let width = page.size[0];
        let [left, top] = p.at;
        for y in 0..h + GUTTER * 2 {
            let sy = y.saturating_sub(GUTTER).min(h - 1);
            for x in 0..w + GUTTER * 2 {
                let sx = x.saturating_sub(GUTTER).min(w - 1);
                let from = ((sy * w + sx) * 4) as usize;
                let to = (((top + y) * width + left + x) * 4) as usize;
                page.rgba[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
            }
        }
        let [pw, ph] = [page.size[0] as f32, page.size[1] as f32];
        let uv = [
            (left + GUTTER) as f32 / pw,
            (top + GUTTER) as f32 / ph,
            (left + GUTTER + w) as f32 / pw,
            (top + GUTTER + h) as f32 / ph,
        ];
        slots.insert(
            name.clone(),
            Slot {
                uv,
                size: [w, h],
                page: p.page,
            },
        );
    }
    Ok(Some(Atlas { pages, slots }))
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
    images: Vec<(String, [u32; 2], Vec<u8>)>,
    clips: Vec<(String, [u32; 2])>,
    score: String,
    score_dir: PathBuf,
    font: Option<Vec<u8>>,
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn art_files(raw: &toml::Table, kind: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let scenes = raw.get("scene").and_then(|s| s.as_array());
    for scene in scenes.into_iter().flatten() {
        let arts = scene.get("art").and_then(|a| a.as_array());
        for art in arts.into_iter().flatten() {
            let mark = art.get("kind").and_then(|k| k.as_str()) == Some(kind);
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
        let images = art_files(&raw, "image")
            .into_iter()
            .map(|name| {
                let (size, rgba) = footage::decode_png(&base.join(&name))?;
                Ok((name, size, rgba))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let clips = art_files(&raw, "clip")
            .into_iter()
            .map(|name| Ok((name.clone(), clip_size(&base.join(&name))?)))
            .collect::<Result<Vec<_>, String>>()
            .map_err(at)?;
        let marks = art_files(&raw, "mark")
            .into_iter()
            .map(|name| Ok((name.clone(), read_text(&base.join(&name))?)))
            .collect::<Result<Vec<_>, String>>()?;
        let (score, score_dir) = match film_path(&raw, "score") {
            Some(p) => {
                let file = base.join(p);
                let dir = file
                    .parent()
                    .map_or_else(|| base.clone(), Path::to_path_buf);
                (read_text(&file)?, dir)
            }
            None => (SILENT_SCORE.to_string(), base.clone()),
        };
        let font = match film_path(&raw, "font") {
            Some(p) => {
                let file = base.join(p);
                Some(std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?)
            }
            None => None,
        };
        let known_score = Score::in_dir(&score, &[], 1, RATE, &score_dir).map_err(at)?;
        let sounds = |name: &str| known_score.knows(name);
        let names: Vec<&str> = marks.iter().map(|(n, _)| n.as_str()).collect();
        let known = Known {
            sounds: &sounds,
            marks: &names,
        };
        let mut script: Script<Reel> = Script::parse(&text, &known).map_err(at)?;
        script.read_bed(&base).map_err(at)?;
        if let Some(font) = &font {
            family(font).map_err(at)?;
        }
        let roll = Roll {
            script,
            path: path.to_path_buf(),
            base,
            marks,
            images,
            clips,
            score,
            score_dir,
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
        let cuts = self.script.cuts(self.fps());
        self.script
            .scenes
            .iter()
            .zip(cuts.windows(2))
            .map(|(scene, cut)| Planned {
                name: scene.name.clone(),
                kind: scene.app.kind,
                seconds: scene.seconds,
                from: scene.from,
                frames: cut[1] - cut[0],
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
        let atlas = atlas(&self.images)?;
        let words = Words {
            images: atlas.as_ref().map(|a| a.slots.clone()).unwrap_or_default(),
            clips: self
                .clips
                .iter()
                .map(|(name, size)| (name.clone(), (self.base.join(name), *size)))
                .collect(),
        };
        let mut painter = Painter::new(gpu, self.size(), icons, font, tick, seed, atlas)?;
        if !self.script.film.app.dither {
            painter.set_dither(None);
        }
        let mut player = Player {
            roll: self,
            painter,
            overlay: Overlay::new(marks),
            playing: None,
            serial: 0,
            crop: None,
            fit: Fit::Fit,
            backdrop: Backdrop::Solid(BLACK),
            volume: 1.0,
            from: 0.0,
            clock: 0.0,
            heard: Vec::new(),
            frame: Vec::new(),
        };
        let mut score = Score::in_dir(
            &self.score,
            &[],
            self.script.film.seed,
            RATE,
            &self.score_dir,
        )?;
        let crew = Crew {
            show: &mut player,
            cast: &words,
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
    serial: u64,
    crop: Option<[f32; 4]>,
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

#[cfg(feature = "video")]
fn clip_size(path: &Path) -> Result<[u32; 2], String> {
    let probe = pito_codecs::media::probe(path, &pito_codecs::media::Prefs::default())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(if probe.rotation % 180 == 90 {
        [probe.height, probe.width]
    } else {
        [probe.width, probe.height]
    })
}

#[cfg(not(feature = "video"))]
fn clip_size(_path: &Path) -> Result<[u32; 2], String> {
    Err("a clip art needs pfx-film's video feature, which this build lacks".into())
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
        self.serial += 1;
        self.crop = f.crop;
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

    fn draw(&mut self, frame: &FilmFrame<'_, Shown>) -> Result<(), String> {
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
        self.frame = self.painter.paint(
            frame,
            &mut self.overlay,
            picture.map(|p| (p, self.serial, self.crop)),
            self.fit,
            self.backdrop,
        )?;
        Ok(())
    }

    fn audio(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.heard);
    }

    fn readback(&mut self) -> Result<Vec<u8>, String> {
        Ok(std::mem::take(&mut self.frame))
    }
}
