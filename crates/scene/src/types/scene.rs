use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::either::{Color, either};
use super::keys::{Key, KeyShape, Keys, numbers};
use super::{is_false, is_zero3};
use crate::Id;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, Node>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Node {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_sided: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Recolor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metalness: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Shadow {
    #[default]
    Cast,
    Only,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Scale {
    Uniform(f32),
    Axes([f32; 3]),
}

either!(Scale, "a number or three numbers", number => Uniform, seq => Axes([f32; 3]),);

impl Scale {
    pub fn axes(self) -> [f32; 3] {
        match self {
            Self::Uniform(value) => [value; 3],
            Self::Axes(axes) => axes,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Object {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefab: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub at: [f32; 3],
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub rotate: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Scale>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub materials: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<Color>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recolor: BTreeMap<String, Recolor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub two_sided: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip: Vec<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_cutoff: Option<f32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub face_camera: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<Character>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<Animation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "BTreeMap<String, serde_json::Value>")
    )]
    pub set: BTreeMap<String, toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Object {
    pub const ALPHA_CUTOFF: f32 = 0.5;

    pub fn is_placement(&self) -> bool {
        self.prefab.is_some()
    }

    pub fn scale_axes(&self) -> [f32; 3] {
        self.scale.map_or([1.0; 3], Scale::axes)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    #[default]
    Point,
    Spot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Light {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<LightKind>,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cone: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    pub intensity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Light {
    pub const KEYED: &[(&str, KeyShape)] = &[
        ("intensity", KeyShape::Number),
        ("color", KeyShape::Triple),
        ("range", KeyShape::Number),
        ("cone", KeyShape::Pair),
    ];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        self.keys.as_ref()?.get(name).map(<[Key]>::to_vec)
    }

    pub const COLOR: [f32; 3] = [1.0; 3];
    pub const RADIUS: f32 = 0.0;
    pub const RANGE: f32 = 10.0;
    pub const SHADOW: bool = true;
    pub const DIRECTION: [f32; 3] = [0.0, -1.0, 0.0];
    pub const CONE: [f32; 2] = [30.0, 45.0];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ParticleBlend {
    #[default]
    Alpha,
    Add,
}

pub const PREWARM_MAX: f32 = 30.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Particles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_keys: Option<Vec<[f32; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burst: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cone: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<Vec<[f32; 2]>>,
    #[serde(default, alias = "colour", skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, alias = "colours", skip_serializing_if = "Option::is_none")]
    pub colors: Option<Vec<(f32, Color)>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spin: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stretch: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<ParticleBlend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prewarm: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Particles {
    pub const KEYED: &[(&str, KeyShape)] = &[
        ("rate", KeyShape::Number),
        ("speed", KeyShape::Pair),
        ("size", KeyShape::Pair),
        ("color", KeyShape::Triple),
        ("alpha", KeyShape::Number),
    ];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        match self.keys.as_ref().and_then(|keys| keys.get(name)) {
            Some(track) => Some(track.to_vec()),
            None if name == "rate" => self.rate_keys.as_deref().map(numbers),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Flare {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ghosts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ghost_spacing: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halo: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub halo_radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starburst: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MotionBlur {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(range(min = 0.0, max = 4.0)))]
    pub shutter: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_px: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<ShutterPosition>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ShutterPosition {
    Start,
    #[default]
    Centre,
    End,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Emitter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    pub position: [f32; 3],
    pub radius: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    pub intensity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MoverKind {
    Turn,
    Slide,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    #[default]
    Swing,
    Loop,
    Once,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Mover {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    pub objects: Vec<String>,
    pub kind: MoverKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<[f32; 3]>,
    pub axis: [f32; 3],
    pub travel: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clip: Vec<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Mover {
    pub const PERIOD: f32 = 4.0;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Content {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Text {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub text: String,
    pub font: String,
    pub size: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub at: [f32; 3],
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub rotate: [f32; 3],
    #[serde(default, skip_serializing_if = "is_false")]
    pub lit: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dynamic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Text {
    pub const COLOR: [f32; 4] = [1.0; 4];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Cue {
    #[default]
    Start,
    Hit,
    Game,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Sound {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<f32>,
    #[serde(default, rename = "loop", skip_serializing_if = "is_false")]
    pub looping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play: Option<Cue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SunModel {
    Daylight,
    Authored,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Sun {
    pub model: SunModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_hour: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irradiance: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
}

impl Sun {
    pub const KEYED: &[(&str, KeyShape)] = &[("hour", KeyShape::Number)];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        self.keys.as_ref()?.get(name).map(<[Key]>::to_vec)
    }

    pub const REFERENCE_HOUR: f64 = 12.0;
    pub const DAY: f64 = 172.0;
    pub const LATITUDE: f64 = 45.0;
    pub const HEADING: f64 = 180.0;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Haze {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lo: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hi: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fog: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoke: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mist: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unmapped: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub falloff: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shafts: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(range(max = 8)))]
    pub bounces: Option<u32>,
    #[serde(default, alias = "colour", skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
}

impl Haze {
    pub const KEYED: &[(&str, KeyShape)] =
        &[("amount", KeyShape::Number), ("color", KeyShape::Triple)];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        self.keys.as_ref()?.get(name).map(<[Key]>::to_vec)
    }

    pub fn outdoor(&self) -> bool {
        self.height.is_some() || self.falloff.is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Camera {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<Projection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look_at: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fov: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub far: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fstop: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_keys: Option<Vec<[f32; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<Preset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Keys>,
}

impl Camera {
    pub const KEYED: &[(&str, KeyShape)] = &[
        ("fov", KeyShape::Number),
        ("fstop", KeyShape::Number),
        ("focus", KeyShape::Number),
    ];

    pub fn track(&self, name: &str) -> Option<Vec<Key>> {
        match self.keys.as_ref().and_then(|keys| keys.get(name)) {
            Some(track) => Some(track.to_vec()),
            None if name == "focus" => self.focus_keys.as_deref().map(numbers),
            None => None,
        }
    }

    pub const AT: [f32; 3] = [0.0, 0.0, 5.0];
    pub const LOOK_AT: [f32; 3] = [0.0; 3];
    pub const UP: [f32; 3] = [0.0, 1.0, 0.0];
    pub const FOV: f32 = 40.0;
    pub const SENSOR: f32 = 24.0;
    pub const NEAR: f32 = 0.05;
    pub const FAR: f32 = 100.0;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit: Option<Orbit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<Zoom>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wander: Option<Wander>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nudge: Option<Nudge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hush: Option<Hush>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Depth>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Orbit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stiffness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Look {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stiffness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lean: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Zoom {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stiffness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Wander {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Nudge {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stiffness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Hush {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stiffness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks_cursor: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Depth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Trace {
    #[serde(default, skip_serializing_if = "is_false")]
    pub transmissive_shadows: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clamp_indirect: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_glossy: Option<f32>,
}

impl Trace {
    pub const CLAMP_INDIRECT: f32 = 0.0;
    pub const FILTER_GLOSSY: f32 = 0.0;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ShadowQuality {
    Low,
    #[default]
    Medium,
    High,
    Ultra,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Shadows {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<ShadowQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascades: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub softness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bias: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion_radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_length: Option<f32>,
}

impl Shadows {
    pub const CASCADES: [u32; 2] = [1, 3];
    pub const RESOLUTION: [u32; 2] = [256, 8192];
    pub const SOFTNESS: [f32; 2] = [0.0, 8.0];
    pub const BIAS: [f32; 2] = [0.0, 16.0];
    pub const OCCLUSION_RADIUS: [f32; 2] = [0.0, 100.0];
    pub const CONTACT_LENGTH: [f32; 2] = [0.0, 10.0];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LiveSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stochastic_reflections: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_reflections: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dof_spread: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversed_depth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coat_reflections: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taa: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssr: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Casters {
    #[default]
    Meshes,
    Proxies,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Plates {
    pub dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxies: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub casters: Option<Casters>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Physics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substeps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Play {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl Physics {
    pub const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];
    pub const RATE: f32 = 60.0;
    pub const SUBSTEPS: u32 = 1;
    pub const ITERATIONS: u32 = 4;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    #[default]
    Dynamic,
    Kinematic,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BodyShape {
    #[default]
    Box,
    Sphere,
    Capsule,
    Cylinder,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Body {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BodyKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<BodyShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restitution: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spin: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity_scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damping: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ccd: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
}

impl Body {
    pub const DENSITY: f32 = 1.0;
    pub const FRICTION: f32 = 0.5;
    pub const RESTITUTION: f32 = 0.0;
    pub const GRAVITY_SCALE: f32 = 1.0;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum CharacterKind {
    #[default]
    #[serde(rename = "3d")]
    ThreeD,
    #[serde(rename = "2d")]
    TwoD,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Plane {
    #[default]
    Xy,
    Yz,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Character {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<CharacterKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_climb: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_step: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coyote: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_buffer: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<Plane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_jump: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_slide: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_jump: Option<[f32; 2]>,
}

impl Character {
    pub const MAX_CLIMB: f32 = 45.0;
    pub const MAX_STEP: f32 = 0.3;
    pub const SNAP: f32 = 0.2;
    pub const COYOTE: u32 = 6;
    pub const JUMP_BUFFER: u32 = 6;
    pub const JUMP_SPEED: f32 = 5.0;
    pub const VARIABLE_JUMP: f32 = 0.0;
    pub const TICKS: u32 = 1000;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<BodyShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum RigKind {
    Follow,
    FirstPerson,
    ThirdPerson,
}

impl RigKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::FirstPerson => "first-person",
            Self::ThirdPerson => "third-person",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Rig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    pub kind: RigKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead_zone: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look_ahead: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damping: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orthographic: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothing: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_bob: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collide: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Rig {
    pub const DEAD_ZONE: [f32; 2] = [0.0; 2];
    pub const LOOK_AHEAD: f32 = 0.0;
    pub const DAMPING: [f32; 3] = [0.0; 3];
    pub const SENSITIVITY: f32 = 1.0;
    pub const INVERT: bool = false;
    pub const SMOOTHING: f32 = 0.0;
    pub const PITCH: [f32; 2] = [-85.0, 85.0];
    pub const HEAD_BOB: f32 = 0.0;
    pub const DISTANCE: f32 = 4.0;
    pub const COLLIDE: bool = true;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Repeat {
    #[default]
    Loop,
    Once,
    PingPong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct BlendClip {
    pub clip: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

impl BlendClip {
    pub const WEIGHT: f32 = 1.0;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SpriteClip {
    pub from: u32,
    pub to: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub looping: Option<Repeat>,
}

impl SpriteClip {
    pub fn count(&self) -> u32 {
        self.to.saturating_sub(self.from).saturating_add(1)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ClipEvent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Animation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub looping: Option<Repeat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<Vec<BlendClip>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ClipEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atlas: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<[u32; 4]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub clips: BTreeMap<String, SpriteClip>,
}

impl Animation {
    pub const SPEED: f32 = 1.0;
    pub const START: f32 = 0.0;
    pub const FPS: f32 = 12.0;

    pub fn is_sprite(&self) -> bool {
        self.atlas.is_some()
    }

    pub fn frame_count(&self) -> Option<u32> {
        if let Some([columns, rows]) = self.grid {
            return Some(columns.saturating_mul(rows));
        }
        self.frames
            .as_ref()
            .map(|frames| u32::try_from(frames.len()).unwrap_or(u32::MAX))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TilePlane {
    #[default]
    Xy,
    Xz,
    Yz,
}

impl TilePlane {
    pub fn axes(self) -> ([f32; 3], [f32; 3]) {
        match self {
            Self::Xy => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Self::Xz => ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            Self::Yz => ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CellSize {
    Square(f32),
    Sides([f32; 2]),
}

either!(CellSize, "a number or two numbers", number => Square, seq => Sides([f32; 2]),);

impl CellSize {
    pub fn sides(self) -> [f32; 2] {
        match self {
            Self::Square(side) => [side; 2],
            Self::Sides(sides) => sides,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TileCell {
    pub at: [i32; 2],
    pub tile: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Tiles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<CellSize>,
    #[serde(default, skip_serializing_if = "is_zero3")]
    pub origin: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<TilePlane>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub palette: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<Vec<TileCell>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Tiles {
    pub const CELL: [f32; 2] = [1.0; 2];
    pub const EMPTY: char = '.';

    pub fn cell_sides(&self) -> [f32; 2] {
        self.cell.map_or(Self::CELL, CellSize::sides)
    }

    pub fn cells(&self) -> Vec<([i32; 2], &str)> {
        let mut out = Vec::new();
        if let Some(rows) = &self.rows {
            for (place, row) in rows.iter().enumerate() {
                let j = (rows.len() - 1 - place) as i32;
                for (i, (at, character)) in row.char_indices().enumerate() {
                    if character != Self::EMPTY {
                        out.push(([i as i32, j], &row[at..at + character.len_utf8()]));
                    }
                }
            }
        }
        for cell in self.cells.iter().flatten() {
            out.push((cell.at, cell.tile.as_str()));
        }
        out
    }

    pub fn position(&self, at: [i32; 2]) -> [f32; 3] {
        let (across, up) = self.plane.unwrap_or_default().axes();
        let [width, height] = self.cell_sides();
        let (i, j) = (at[0] as f32 * width, at[1] as f32 * height);
        [
            self.origin[0] + i * across[0] + j * up[0],
            self.origin[1] + i * across[1] + j * up[1],
            self.origin[2] + i * across[2] + j * up[2],
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    Box,
    Hull,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Proxy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub object: String,
    pub kind: ProxyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f32; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triangles: Option<Vec<[u32; 3]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sounds {
    pub file: Option<String>,
    pub table: BTreeMap<String, Sound>,
}

impl Sounds {
    pub fn is_empty(&self) -> bool {
        self.file.is_none() && self.table.is_empty()
    }
}

impl std::ops::Deref for Sounds {
    type Target = BTreeMap<String, Sound>;

    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl std::ops::DerefMut for Sounds {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.table
    }
}

impl From<BTreeMap<String, Sound>> for Sounds {
    fn from(table: BTreeMap<String, Sound>) -> Self {
        Self { file: None, table }
    }
}

impl FromIterator<(String, Sound)> for Sounds {
    fn from_iter<I: IntoIterator<Item = (String, Sound)>>(items: I) -> Self {
        Self::from(items.into_iter().collect::<BTreeMap<_, _>>())
    }
}

impl IntoIterator for Sounds {
    type Item = (String, Sound);
    type IntoIter = std::collections::btree_map::IntoIter<String, Sound>;

    fn into_iter(self) -> Self::IntoIter {
        self.table.into_iter()
    }
}

impl<'a> IntoIterator for &'a Sounds {
    type Item = (&'a String, &'a Sound);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Sound>;

    fn into_iter(self) -> Self::IntoIter {
        self.table.iter()
    }
}

impl Serialize for Sounds {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.file {
            Some(file) => serializer.serialize_str(file),
            None => self.table.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Sounds {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Pick;

        impl<'de> serde::de::Visitor<'de> for Pick {
            type Value = Sounds;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a path to a sound file or a table of sounds")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Sounds, E> {
                Ok(Sounds {
                    file: Some(value.to_string()),
                    table: BTreeMap::new(),
                })
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Sounds, A::Error> {
                BTreeMap::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(Sounds::from)
            }
        }

        deserializer.deserialize_any(Pick)
    }
}
