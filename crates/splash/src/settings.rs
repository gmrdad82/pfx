use pfx_live::flat::Srgba;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour(pub [u8; 3]);

impl Colour {
    pub const GROUND: Colour = Colour([0x0c, 0x0a, 0x12]);

    pub fn parse(text: &str) -> Result<Colour, String> {
        let hex = text
            .strip_prefix('#')
            .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| format!("{text:?} is not a #rrggbb colour"))?;
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string());
        Ok(Colour([byte(0)?, byte(2)?, byte(4)?]))
    }

    pub fn srgba(self) -> Srgba {
        let [r, g, b] = self.0;
        Srgba::hex(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }
}

impl std::fmt::Display for Colour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Colour, D::Error> {
        let text = String::deserialize(deserializer)?;
        Colour::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Show {
    #[default]
    Auto,
    Always,
}

impl Show {
    fn is_auto(&self) -> bool {
        *self == Show::Auto
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub background: Colour,
    #[serde(skip_serializing_if = "Show::is_auto")]
    pub show: Show,
    pub hide: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            background: Colour::GROUND,
            show: Show::Auto,
            hide: false,
            key: None,
        }
    }
}
