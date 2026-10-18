mod either;
mod finish;
mod keys;
mod material;
mod scene;
mod sky;
mod tunable;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use either::Color;
pub use finish::{
    Bloom, BloomKind, BloomTable, Dither, DitherKind, Finish, FinishKeys, Grain, GrainTable, Ink,
    Lut, LutKey, Outline, OutlineTable, STYLES, Scratches, ScratchesTable, Style, Tape,
    TapeDirection, TapeTable, Tone, ToneKind, ToneTable, Vignette, VignetteKind, VignetteTable,
    Warmth, WarmthKind, WarmthTable,
};
pub use keys::{Key, KeyShape, KeyValue, Keys, sample};
pub use material::{
    Ageing, Anisotropy, Blend, Content as ContentKind, ContentLayer, Fabric, Family, LAYER_CAP,
    Maps, Material, NoiseKind, NoiseLayer, Normal, NormalSource, PARAMS, Scatter, Wet,
};
pub use scene::{
    Animation, BlendClip, Body, BodyKind, BodyShape, Bounds, Camera, Casters, CellSize, Character,
    CharacterKind, ClipEvent, Content, Cue, Depth, Emitter, Flare, Haze, Hush, Light, LightKind,
    Look, Mesh, Motion, MotionBlur, Mover, MoverKind, Node, Nudge, Object, Orbit, ParticleBlend,
    Particles, Physics, Plane, Plates, Play, Preset, Projection, Proxy, ProxyKind, Repeat, Rig,
    RigKind, Scale, Shadow, ShadowQuality, Shadows, Sound, Sounds, SpriteClip, Sun, SunModel, Text,
    TileCell, TilePlane, Tiles, Trace, Trigger, Wander, Zoom,
};
pub use sky::{Prepare, Room, RoomLamp, Sky, SkyKind};
pub use tunable::{Tunable, TunableKind, TunableValue};

pub const FORMAT: u32 = 1;

pub const LAYER_COUNT: usize = 32;

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn is_zero3(value: &[f32; 3]) -> bool {
    *value == [0.0; 3]
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    pub project: ProjectTable,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tunables: BTreeMap<String, Tunable>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")
    )]
    pub authoring: Option<toml::Table>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProjectTable {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SceneFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mesh: BTreeMap<String, Mesh>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object: Vec<Object>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub light: Vec<Light>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emitter: Vec<Emitter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mover: Vec<Mover>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub content: BTreeMap<String, Content>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text: BTreeMap<String, Text>,
    #[serde(default, skip_serializing_if = "Sounds::is_empty")]
    pub sound: Sounds,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rig: Vec<Rig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<Tiles>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub particles: Vec<Particles>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sun: Option<Sun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sky: Option<Sky>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haze: Option<Haze>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Camera>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<Finish>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flare: Option<Flare>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_blur: Option<MotionBlur>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Trace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<Shadows>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plates: Option<Plates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<Physics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play: Option<Play>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")
    )]
    pub authoring: Option<toml::Table>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PrefabFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mesh: BTreeMap<String, Mesh>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object: Vec<Object>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub light: Vec<Light>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emitter: Vec<Emitter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mover: Vec<Mover>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub content: BTreeMap<String, Content>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub text: BTreeMap<String, Text>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sound: BTreeMap<String, Sound>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rig: Vec<Rig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub particles: Vec<Particles>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")
    )]
    pub authoring: Option<toml::Table>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MaterialsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub materials: BTreeMap<String, Material>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")
    )]
    pub authoring: Option<toml::Table>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProxiesFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proxy: Vec<Proxy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")
    )]
    pub authoring: Option<toml::Table>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum File {
    Project(ProjectFile),
    Scene(SceneFile),
    Prefab(PrefabFile),
    Materials(MaterialsFile),
    Proxies(ProxiesFile),
    Finish(FinishKeys),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    Project,
    Scene,
    Prefab,
    Materials,
    Proxies,
    Finish,
}

impl FileKind {
    pub fn of(path: &std::path::Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        if name == "project.toml" {
            Some(Self::Project)
        } else if name.ends_with(".scene.toml") || name == "scene.toml" {
            Some(Self::Scene)
        } else if name.ends_with(".prefab.toml") {
            Some(Self::Prefab)
        } else if name.ends_with(".materials.toml") {
            Some(Self::Materials)
        } else if name.ends_with(".proxies.toml") {
            Some(Self::Proxies)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Project => "project file",
            Self::Scene => "scene",
            Self::Prefab => "prefab",
            Self::Materials => "material library",
            Self::Proxies => "proxies file",
            Self::Finish => "finish file",
        }
    }
}
