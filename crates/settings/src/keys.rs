use crate::{Effect, Preferences, Quality, Rate};

pub const LICENCE: &str = "licence";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Vrr,
    FrameRate,
    Stats,
    Quality,
    Effect(Effect),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Text(&'static str),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(true) => write!(f, "on"),
            Value::Bool(false) => write!(f, "off"),
            Value::Text(text) => write!(f, "{text}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Display,
    Quality,
    Effects,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Display, Section::Quality, Section::Effects];

    pub fn label(self) -> &'static str {
        match self {
            Section::Display => "Display",
            Section::Quality => "Quality",
            Section::Effects => "Effects",
        }
    }

    pub fn keys(self) -> Vec<Key> {
        match self {
            Section::Display => vec![Key::Vrr, Key::FrameRate, Key::Stats],
            Section::Quality => vec![Key::Quality],
            Section::Effects => Effect::ALL.into_iter().map(Key::Effect).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    pub name: &'static str,
    pub label: &'static str,
    pub about: &'static str,
}

impl Key {
    pub fn all() -> Vec<Key> {
        Section::ALL.into_iter().flat_map(Section::keys).collect()
    }

    pub fn name(self) -> String {
        match self {
            Key::Vrr => "vrr".into(),
            Key::FrameRate => "frame_rate".into(),
            Key::Stats => "stats".into(),
            Key::Quality => "quality".into(),
            Key::Effect(effect) => format!("effects.{}", effect.name()),
        }
    }

    pub fn parse(text: &str) -> Result<Key, String> {
        let found = match text {
            "vrr" => Some(Key::Vrr),
            "frame_rate" => Some(Key::FrameRate),
            "stats" => Some(Key::Stats),
            "quality" => Some(Key::Quality),
            _ => text
                .strip_prefix("effects.")
                .and_then(Effect::parse)
                .map(Key::Effect),
        };
        found.ok_or_else(|| {
            let names: Vec<String> = Key::all().into_iter().map(Key::name).collect();
            format!(
                "no setting is named '{text}'; the settings are {} and {LICENCE}",
                names.join(", ")
            )
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Key::Vrr => "VRR",
            Key::FrameRate => "Frame rate",
            Key::Stats => "Frame stats",
            Key::Quality => "Quality",
            Key::Effect(effect) => effect.label(),
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Key::Vrr => "adaptive sync: the display follows the frame rate; off is steady vsync",
            Key::FrameRate => "how fast the window presents frames",
            Key::Stats => "the frame stats overlay over the viewport",
            Key::Quality => "native resolution, or a render scale that holds 8.33 ms",
            Key::Effect(effect) => effect.about(),
        }
    }

    pub fn section(self) -> Section {
        match self {
            Key::Vrr | Key::FrameRate | Key::Stats => Section::Display,
            Key::Quality => Section::Quality,
            Key::Effect(_) => Section::Effects,
        }
    }

    pub fn choices(self) -> Vec<Choice> {
        match self {
            Key::Vrr | Key::Stats | Key::Effect(_) => vec![
                Choice {
                    name: "on",
                    label: "On",
                    about: "",
                },
                Choice {
                    name: "off",
                    label: "Off",
                    about: "",
                },
            ],
            Key::FrameRate => Rate::ALL
                .into_iter()
                .map(|rate| Choice {
                    name: rate.name(),
                    label: rate.label(),
                    about: rate.about(),
                })
                .collect(),
            Key::Quality => Quality::ALL
                .into_iter()
                .map(|quality| Choice {
                    name: quality.name(),
                    label: quality.label(),
                    about: quality.about(),
                })
                .collect(),
        }
    }

    pub fn get(self, preferences: &Preferences) -> Value {
        match self {
            Key::Vrr => Value::Bool(preferences.vrr),
            Key::FrameRate => Value::Text(preferences.rate().name()),
            Key::Stats => Value::Bool(preferences.stats),
            Key::Quality => Value::Text(preferences.graphics.quality().name()),
            Key::Effect(effect) => Value::Bool(preferences.graphics.effects.get(effect)),
        }
    }

    pub fn chosen(self, preferences: &Preferences) -> &'static str {
        match self.get(preferences) {
            Value::Bool(true) => "on",
            Value::Bool(false) => "off",
            Value::Text(text) => text,
        }
    }

    pub fn set(self, preferences: &mut Preferences, text: &str) -> Result<(), String> {
        let refused = || {
            let names: Vec<&str> = self.choices().iter().map(|choice| choice.name).collect();
            format!(
                "'{text}' is not a value of {}; it takes {}",
                self.name(),
                names.join(", ")
            )
        };
        match self {
            Key::Vrr => preferences.vrr = switch(text).ok_or_else(refused)?,
            Key::FrameRate => preferences.frame_rate = Some(Rate::parse(text).ok_or_else(refused)?),
            Key::Stats => preferences.stats = switch(text).ok_or_else(refused)?,
            Key::Quality => {
                preferences.graphics.quality = Some(Quality::parse(text).ok_or_else(refused)?)
            }
            Key::Effect(effect) => preferences
                .graphics
                .effects
                .set(effect, switch(text).ok_or_else(refused)?),
        }
        Ok(())
    }
}

fn switch(text: &str) -> Option<bool> {
    match text {
        "on" | "true" => Some(true),
        "off" | "false" => Some(false),
        _ => None,
    }
}

pub fn effect_names() -> String {
    let names: Vec<&str> = Effect::ALL.into_iter().map(Effect::name).collect();
    names.join(", ")
}
