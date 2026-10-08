use serde::{Deserialize, Serialize};

pub const DMI_PRODUCT_NAME: &str = "/sys/class/dmi/id/product_name";
pub const STEAM_DECK_VAR: &str = "SteamDeck";
pub const DESKTOP_VAR: &str = "XDG_CURRENT_DESKTOP";
pub const GAMESCOPE_VAR: &str = "GAMESCOPE_WAYLAND_DISPLAY";
pub const OVERRIDE_VAR: &str = "PFX_DEVICE";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeckModel {
    Lcd,
    Oled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    SteamDeck { model: DeckModel },
    Desktop,
}

impl Device {
    pub fn is_deck(self) -> bool {
        matches!(self, Device::SteamDeck { .. })
    }

    pub fn label(self) -> &'static str {
        match self {
            Device::SteamDeck {
                model: DeckModel::Lcd,
            } => "steam-deck-lcd",
            Device::SteamDeck {
                model: DeckModel::Oled,
            } => "steam-deck-oled",
            Device::Desktop => "desktop",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        [
            Device::SteamDeck {
                model: DeckModel::Lcd,
            },
            Device::SteamDeck {
                model: DeckModel::Oled,
            },
            Device::Desktop,
        ]
        .into_iter()
        .find(|device| {
            device.label() == label || device.label().strip_prefix("steam-") == Some(label)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Evidence {
    Override,
    SteamDeckVar,
    ProductName(String),
    Gamescope,
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    pub device: Device,
    pub evidence: Evidence,
}

pub trait DeviceSource {
    fn var(&self, name: &str) -> Option<String>;
    fn product_name(&self) -> Option<String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemSource;

impl DeviceSource for SystemSource {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn product_name(&self) -> Option<String> {
        std::fs::read_to_string(DMI_PRODUCT_NAME).ok()
    }
}

pub fn deck_model(product_name: &str) -> Option<DeckModel> {
    match product_name.trim() {
        "Jupiter" => Some(DeckModel::Lcd),
        "Galileo" => Some(DeckModel::Oled),
        _ => None,
    }
}

pub fn detect(source: &impl DeviceSource) -> Detected {
    if let Some(device) = source
        .var(OVERRIDE_VAR)
        .and_then(|value| Device::from_label(value.trim()))
    {
        return Detected {
            device,
            evidence: Evidence::Override,
        };
    }
    let product = source.product_name();
    let model = product.as_deref().and_then(deck_model);
    if source.var(STEAM_DECK_VAR).as_deref().map(str::trim) == Some("1") {
        return Detected {
            device: Device::SteamDeck {
                model: model.unwrap_or(DeckModel::Lcd),
            },
            evidence: Evidence::SteamDeckVar,
        };
    }
    if let Some(model) = model {
        return Detected {
            device: Device::SteamDeck { model },
            evidence: Evidence::ProductName(product.unwrap_or_default().trim().to_owned()),
        };
    }
    let desktop = source
        .var(DESKTOP_VAR)
        .is_some_and(|desktop| desktop.trim().eq_ignore_ascii_case("gamescope"));
    let nested = source
        .var(GAMESCOPE_VAR)
        .is_some_and(|display| !display.trim().is_empty());
    if desktop || nested {
        return Detected {
            device: Device::SteamDeck {
                model: DeckModel::Lcd,
            },
            evidence: Evidence::Gamescope,
        };
    }
    Detected {
        device: Device::Desktop,
        evidence: Evidence::Nothing,
    }
}
