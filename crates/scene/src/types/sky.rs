use serde::{Deserialize, Serialize};

use super::keys::{Key, KeyShape, Keys};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SkyKind {
    Analytic,
    Hdr,
    Room,
    Mix,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Sky {
    pub kind: SkyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_deg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon_color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon_amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbidity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_albedo: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare: Option<Prepare>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<Room>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer: Vec<Sky>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
}

impl Sky {
    pub const KEYED: &[(&str, KeyShape)] = &[
        ("intensity", KeyShape::Number),
        ("backdrop", KeyShape::Number),
        ("horizon_color", KeyShape::Triple),
        ("horizon_width", KeyShape::Number),
        ("horizon_amount", KeyShape::Number),
    ];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        self.keys.as_ref()?.get(name).map(<[Key]>::to_vec)
    }

    pub const TURBIDITY: f32 = 3.0;
    pub const GROUND_ALBEDO: [f32; 3] = [0.2; 3];
    pub const AMBIENT: f32 = 1.0;
    pub const INTENSITY: f32 = 1.0;
    pub const WEIGHT: f32 = 1.0;
    pub const BACKDROP: f32 = 1.0;
    pub const HORIZON_WIDTH: f32 = 10.0;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Room {
    #[serde(default = "Room::width")]
    pub width: u32,
    #[serde(default = "Room::grey")]
    pub floor: [f32; 3],
    #[serde(default = "Room::grey")]
    pub wall: [f32; 3],
    #[serde(default = "Room::grey")]
    pub ceiling: [f32; 3],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<RoomLamp>,
}

impl Room {
    pub const MAX_WIDTH: u32 = 8192;

    fn width() -> u32 {
        64
    }

    fn grey() -> [f32; 3] {
        [0.5; 3]
    }
}

impl Default for Room {
    fn default() -> Self {
        Self {
            width: Self::width(),
            floor: Self::grey(),
            wall: Self::grey(),
            ceiling: Self::grey(),
            lights: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RoomLamp {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default)]
    pub az: f32,
    #[serde(default)]
    pub el: f32,
    #[serde(default = "RoomLamp::one")]
    pub power: f32,
    #[serde(default = "RoomLamp::white")]
    pub color: [f32; 3],
    #[serde(default = "RoomLamp::spread")]
    pub width: f32,
    #[serde(default = "RoomLamp::spread")]
    pub height: f32,
    #[serde(default = "RoomLamp::soft")]
    pub soft: f32,
    #[serde(default)]
    pub slats: f32,
    #[serde(default = "RoomLamp::one")]
    pub open: f32,
}

impl RoomLamp {
    fn one() -> f32 {
        1.0
    }

    fn white() -> [f32; 3] {
        [1.0; 3]
    }

    fn spread() -> f32 {
        45.0
    }

    fn soft() -> f32 {
        0.1
    }
}

impl Default for RoomLamp {
    fn default() -> Self {
        Self {
            name: String::new(),
            az: 0.0,
            el: 0.0,
            power: Self::one(),
            color: Self::white(),
            width: Self::spread(),
            height: Self::spread(),
            soft: Self::soft(),
            slats: 0.0,
            open: Self::one(),
        }
    }
}
