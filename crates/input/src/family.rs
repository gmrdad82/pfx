use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum GlyphStyle {
    PlayStation,
    Standard,
}

impl GlyphStyle {
    pub const ALL: [GlyphStyle; 2] = [GlyphStyle::PlayStation, GlyphStyle::Standard];

    pub fn family(self) -> Family {
        match self {
            GlyphStyle::PlayStation => Family::DualSense,
            GlyphStyle::Standard => Family::Xbox,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            GlyphStyle::PlayStation => "PlayStation",
            GlyphStyle::Standard => "Standard",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Family {
    DualSense,
    DualShock4,
    Xbox,
    SteamController,
    SteamDeck,
    SwitchPro,
    Generic,
}

impl Family {
    pub const ALL: [Family; 7] = [
        Family::DualSense,
        Family::DualShock4,
        Family::Xbox,
        Family::SteamController,
        Family::SteamDeck,
        Family::SwitchPro,
        Family::Generic,
    ];

    pub fn from_ids(vendor: u16, product: u16) -> Family {
        if let Some(entry) = PADS
            .iter()
            .find(|entry| entry.vendor == vendor && entry.product == product)
        {
            return entry.family;
        }
        match vendor {
            SONY => Family::DualShock4,
            MICROSOFT => Family::Xbox,
            VALVE => Family::SteamController,
            NINTENDO => Family::SwitchPro,
            _ => Family::Generic,
        }
    }

    pub fn from_steam_input_type(kind: i32) -> Family {
        match kind {
            STEAM_INPUT_STEAM_CONTROLLER => Family::SteamController,
            STEAM_INPUT_XBOX_360 | STEAM_INPUT_XBOX_ONE => Family::Xbox,
            STEAM_INPUT_PS4 | STEAM_INPUT_PS3 => Family::DualShock4,
            STEAM_INPUT_PS5 => Family::DualSense,
            STEAM_INPUT_SWITCH_JOYCON_PAIR
            | STEAM_INPUT_SWITCH_JOYCON_SINGLE
            | STEAM_INPUT_SWITCH_PRO => Family::SwitchPro,
            STEAM_INPUT_STEAM_DECK => Family::SteamDeck,
            _ => Family::Generic,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Family::DualSense => "DualSense",
            Family::DualShock4 => "DualShock 4",
            Family::Xbox => "Xbox",
            Family::SteamController => "Steam Controller",
            Family::SteamDeck => "Steam Deck",
            Family::SwitchPro => "Switch Pro",
            Family::Generic => "Generic",
        }
    }

    pub fn style(self) -> GlyphStyle {
        match self {
            Family::DualSense | Family::DualShock4 => GlyphStyle::PlayStation,
            _ => GlyphStyle::Standard,
        }
    }

    pub fn has_grips(self) -> bool {
        matches!(self, Family::SteamController | Family::SteamDeck)
    }
}

pub const SONY: u16 = 0x054c;
pub const MICROSOFT: u16 = 0x045e;
pub const VALVE: u16 = 0x28de;
pub const NINTENDO: u16 = 0x057e;

pub const STEAM_INPUT_UNKNOWN: i32 = 0;
pub const STEAM_INPUT_STEAM_CONTROLLER: i32 = 1;
pub const STEAM_INPUT_XBOX_360: i32 = 2;
pub const STEAM_INPUT_XBOX_ONE: i32 = 3;
pub const STEAM_INPUT_GENERIC: i32 = 4;
pub const STEAM_INPUT_PS4: i32 = 5;
pub const STEAM_INPUT_APPLE_MFI: i32 = 6;
pub const STEAM_INPUT_ANDROID: i32 = 7;
pub const STEAM_INPUT_SWITCH_JOYCON_PAIR: i32 = 8;
pub const STEAM_INPUT_SWITCH_JOYCON_SINGLE: i32 = 9;
pub const STEAM_INPUT_SWITCH_PRO: i32 = 10;
pub const STEAM_INPUT_MOBILE_TOUCH: i32 = 11;
pub const STEAM_INPUT_PS3: i32 = 12;
pub const STEAM_INPUT_PS5: i32 = 13;
pub const STEAM_INPUT_STEAM_DECK: i32 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadEntry {
    pub vendor: u16,
    pub product: u16,
    pub family: Family,
    pub model: &'static str,
}

const fn pad(vendor: u16, product: u16, family: Family, model: &'static str) -> PadEntry {
    PadEntry {
        vendor,
        product,
        family,
        model,
    }
}

pub const PADS: &[PadEntry] = &[
    pad(SONY, 0x0ce6, Family::DualSense, "DualSense"),
    pad(SONY, 0x0df2, Family::DualSense, "DualSense Edge"),
    pad(SONY, 0x05c4, Family::DualShock4, "DualShock 4"),
    pad(
        SONY,
        0x09cc,
        Family::DualShock4,
        "DualShock 4 (second revision)",
    ),
    pad(
        SONY,
        0x0ba0,
        Family::DualShock4,
        "DualShock 4 wireless adapter",
    ),
    pad(SONY, 0x0268, Family::DualShock4, "DualShock 3"),
    pad(MICROSOFT, 0x028e, Family::Xbox, "Xbox 360"),
    pad(MICROSOFT, 0x028f, Family::Xbox, "Xbox 360 wireless"),
    pad(
        MICROSOFT,
        0x0719,
        Family::Xbox,
        "Xbox 360 wireless receiver",
    ),
    pad(MICROSOFT, 0x02d1, Family::Xbox, "Xbox One"),
    pad(MICROSOFT, 0x02dd, Family::Xbox, "Xbox One (2015)"),
    pad(MICROSOFT, 0x02e0, Family::Xbox, "Xbox One S wireless"),
    pad(MICROSOFT, 0x02e3, Family::Xbox, "Xbox One Elite"),
    pad(MICROSOFT, 0x02ea, Family::Xbox, "Xbox One S"),
    pad(MICROSOFT, 0x02fd, Family::Xbox, "Xbox One S Bluetooth"),
    pad(MICROSOFT, 0x0b00, Family::Xbox, "Xbox Elite Series 2"),
    pad(
        MICROSOFT,
        0x0b05,
        Family::Xbox,
        "Xbox Elite Series 2 Bluetooth",
    ),
    pad(MICROSOFT, 0x0b12, Family::Xbox, "Xbox Series X|S"),
    pad(MICROSOFT, 0x0b13, Family::Xbox, "Xbox Series X|S Bluetooth"),
    pad(
        MICROSOFT,
        0x0b20,
        Family::Xbox,
        "Xbox One S Bluetooth (2021)",
    ),
    pad(
        MICROSOFT,
        0x0b22,
        Family::Xbox,
        "Xbox Elite Series 2 Bluetooth (2021)",
    ),
    pad(VALVE, 0x1102, Family::SteamController, "Steam Controller"),
    pad(
        VALVE,
        0x1142,
        Family::SteamController,
        "Steam Controller dongle",
    ),
    pad(VALVE, 0x1205, Family::SteamDeck, "Steam Deck"),
    pad(
        VALVE,
        0x11ff,
        Family::Generic,
        "Steam Input virtual gamepad",
    ),
    pad(NINTENDO, 0x2009, Family::SwitchPro, "Switch Pro"),
    pad(NINTENDO, 0x2006, Family::SwitchPro, "Joy-Con (L)"),
    pad(NINTENDO, 0x2007, Family::SwitchPro, "Joy-Con (R)"),
    pad(NINTENDO, 0x200e, Family::SwitchPro, "Joy-Con charging grip"),
];

pub fn model(vendor: u16, product: u16) -> Option<&'static str> {
    PADS.iter()
        .find(|entry| entry.vendor == vendor && entry.product == product)
        .map(|entry| entry.model)
}
