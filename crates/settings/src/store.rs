use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

use crate::{Effect, Effects, Graphics, Quality, Rate};

pub const FILE: &str = "settings.pitofx";
pub const ENTRY: &str = "settings.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub vrr: bool,
    pub frame_rate: Option<Rate>,
    pub stats: bool,
    pub graphics: Graphics,
}

impl Default for Preferences {
    fn default() -> Preferences {
        Preferences {
            vrr: true,
            frame_rate: None,
            stats: false,
            graphics: Graphics::default(),
        }
    }
}

impl Preferences {
    pub fn rate(&self) -> Rate {
        self.frame_rate.unwrap_or_default()
    }

    pub fn measured(&self) -> bool {
        self.frame_rate.is_some() && self.graphics.measured()
    }

    pub fn measure(&mut self, native_ms: f32) -> bool {
        let fresh = !self.measured();
        self.frame_rate.get_or_insert(Rate::pick(native_ms));
        self.graphics
            .quality
            .get_or_insert(Quality::pick(native_ms));
        fresh
    }

    #[cfg(feature = "present")]
    pub fn present(&self) -> pfx_gpu::window::Present {
        pfx_gpu::window::Present {
            vrr: self.vrr,
            frame_rate: self.rate().frame_rate(),
        }
    }

    pub fn read(document: &DocumentMut) -> Preferences {
        let mut found = Preferences::default();
        let table = |name: &str| document.get(name).and_then(Item::as_table_like);
        if let Some(display) = table("display") {
            if let Some(vrr) = display.get("vrr").and_then(Item::as_bool) {
                found.vrr = vrr;
            }
            if let Some(stats) = display.get("stats").and_then(Item::as_bool) {
                found.stats = stats;
            }
            found.frame_rate = display
                .get("frame_rate")
                .and_then(Item::as_str)
                .and_then(Rate::parse);
        }
        if let Some(graphics) = table("graphics") {
            found.graphics.quality = graphics
                .get("quality")
                .and_then(Item::as_str)
                .and_then(Quality::parse);
            if let Some(effects) = graphics.get("effects").and_then(Item::as_table_like) {
                for effect in Effect::ALL {
                    if let Some(on) = effects.get(effect.name()).and_then(Item::as_bool) {
                        found.graphics.effects.set(effect, on);
                    }
                }
            }
        }
        found
    }

    pub fn write(&self, document: &mut DocumentMut) {
        let display = table(document, "display");
        display["vrr"] = value(self.vrr);
        display["stats"] = value(self.stats);
        match self.frame_rate {
            Some(rate) => display["frame_rate"] = value(rate.stored()),
            None => {
                display.remove("frame_rate");
            }
        }
        let graphics = table(document, "graphics");
        match self.graphics.quality {
            Some(quality) => graphics["quality"] = value(quality.name()),
            None => {
                graphics.remove("quality");
            }
        }
        let mut effects = Table::new();
        for effect in Effect::ALL {
            effects[effect.name()] = value(self.graphics.effects.get(effect));
        }
        graphics["effects"] = Item::Table(effects);
    }
}

fn table<'a>(document: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    if !document.get(name).is_some_and(Item::is_table) {
        document[name] = Item::Table(Table::new());
    }
    document[name]
        .as_table_mut()
        .expect("a table was just put there")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    path: Option<PathBuf>,
}

impl Store {
    pub fn machine() -> Store {
        Store {
            path: Some(crate::dirs::state_dir().join(FILE)),
        }
    }

    pub fn at(path: impl Into<PathBuf>) -> Store {
        Store {
            path: Some(path.into()),
        }
    }

    pub fn in_dir(dir: &Path) -> Store {
        Store::at(dir.join(FILE))
    }

    pub fn none() -> Store {
        Store { path: None }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn document(&self) -> DocumentMut {
        self.path
            .as_ref()
            .and_then(|path| pfx_file::Reader::open(path).ok())
            .and_then(|reader| {
                reader
                    .named(pfx_file::FILE, ENTRY)
                    .ok()
                    .flatten()
                    .map(|bytes| bytes.into_owned())
            })
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| text.parse().ok())
            .unwrap_or_default()
    }

    pub fn keep(&self, document: &DocumentMut) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        pfx_file::Writer::new()
            .named(
                pfx_file::FILE,
                ENTRY,
                document.to_string().into_bytes(),
                pfx_file::Codec::Auto,
            )
            .write(path)
            .map_err(|error| std::io::Error::other(error.to_string()))
    }

    pub fn preferences(&self) -> Preferences {
        Preferences::read(&self.document())
    }

    pub fn keep_preferences(&self, preferences: &Preferences) -> std::io::Result<()> {
        if self.path.is_none() {
            return Ok(());
        }
        let mut document = self.document();
        preferences.write(&mut document);
        self.keep(&document)
    }

    pub fn licence_key(&self) -> Option<String> {
        self.document()
            .get("licence")
            .and_then(Item::as_table_like)
            .and_then(|licence| licence.get("key"))
            .and_then(Item::as_str)
            .map(str::to_string)
            .filter(|key| !key.trim().is_empty())
    }

    pub fn splash_hidden(&self) -> bool {
        self.document()
            .get("licence")
            .and_then(Item::as_table_like)
            .and_then(|licence| licence.get("hide_splash"))
            .and_then(Item::as_bool)
            .unwrap_or(true)
    }

    pub fn keep_splash_hidden(&self, hidden: bool) -> std::io::Result<()> {
        if self.path.is_none() {
            return Ok(());
        }
        let mut document = self.document();
        table(&mut document, "licence")["hide_splash"] = value(hidden);
        self.keep(&document)
    }

    pub fn keep_licence_key(&self, key: Option<&str>) -> std::io::Result<()> {
        if self.path.is_none() {
            return Ok(());
        }
        let mut document = self.document();
        match key {
            Some(key) => table(&mut document, "licence")["key"] = value(key.trim()),
            None => {
                document.remove("licence");
            }
        }
        self.keep(&document)
    }
}

pub fn effects_from(table: &dyn toml_edit::TableLike) -> Result<Effects, String> {
    let mut effects = Effects::default();
    for (key, item) in table.iter() {
        let effect = Effect::parse(key).ok_or_else(|| {
            format!(
                "unknown effect '{key}'; the effects are {}",
                crate::keys::effect_names()
            )
        })?;
        let on = item
            .as_bool()
            .ok_or_else(|| format!("{key} takes true or false"))?;
        effects.set(effect, on);
    }
    Ok(effects)
}
