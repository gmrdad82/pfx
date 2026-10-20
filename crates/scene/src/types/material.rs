use serde::{Deserialize, Serialize};

use crate::Id;

pub const LAYER_CAP: usize = 4;
pub const PARAMS: usize = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Family {
    #[default]
    Plain,
    Paper,
    Plaster,
    Stone,
    Cloth,
    Metal,
    Occluder,
    Petal,
    Leaf,
    Bark,
    Cord,
    Wood,
    Lacquer,
    Glass,
    Liquid,
    Emissive,
    Chrome,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Content {
    #[default]
    None,
    Ink,
    Decal,
    Screen,
    Photo,
    Scroll,
    Print,
    Field,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NormalSource {
    #[default]
    Flat,
    Bump,
    Map,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NoiseKind {
    #[default]
    None,
    Fibre,
    Crinkle,
    PlankWood,
    WallMottle,
    Grime,
    Leaf,
    Bark,
    Flow,
    Value,
    Fbm,
    Scratch,
    CoatWobble,
    Crack,
    Track,
    FilmRamp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    #[default]
    Over,
    Multiply,
    Emit,
}

fn unset(params: &[f32; PARAMS]) -> bool {
    params.iter().all(|value| *value == 0.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NoiseLayer {
    pub kind: NoiseKind,
    pub frequency: f32,
    pub amplitude: f32,
    pub seed: u32,
    #[serde(default, skip_serializing_if = "unset")]
    pub params: [f32; PARAMS],
}

impl Default for NoiseLayer {
    fn default() -> Self {
        Self {
            kind: NoiseKind::None,
            frequency: 1.0,
            amplitude: 0.0,
            seed: 0,
            params: [0.0; PARAMS],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Normal {
    pub source: NormalSource,
    pub strength: f32,
}

impl Default for Normal {
    fn default() -> Self {
        Self {
            source: NormalSource::Flat,
            strength: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Maps {
    pub layer: f32,
    pub tile: f32,
    pub normal: f32,
    pub albedo: f32,
}

impl Default for Maps {
    fn default() -> Self {
        Self {
            layer: -1.0,
            tile: 1.0,
            normal: 1.0,
            albedo: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Ageing {
    pub fade: f32,
    pub yellow: f32,
    pub ink: f32,
    pub scratch: f32,
    pub edge: f32,
    pub dust: f32,
    pub patina: f32,
    pub seed: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Wet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub puddles: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub puddle_scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Fabric {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Anisotropy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Scatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Flakes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct ContentLayer {
    pub slot: i32,
    pub blend: Blend,
    pub ink_roughness: f32,
    pub emboss: f32,
    pub strength: f32,
}

impl Default for ContentLayer {
    fn default() -> Self {
        Self {
            slot: -1,
            blend: Blend::Over,
            ink_roughness: -1.0,
            emboss: 0.0,
            strength: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Material {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub family: Family,
    pub base: [f32; 3],
    pub roughness: f32,
    pub metalness: f32,
    pub specular: f32,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    pub sheen: f32,
    pub transmission: f32,
    pub ior: f32,
    pub dispersion: f32,
    pub thickness: f32,
    pub subsurface: f32,
    pub subsurface_tint: [f32; 3],
    pub absorption: f32,
    pub thin_film: f32,
    pub thin_film_ior: f32,
    pub thin_film_amount: f32,
    pub emission: [f32; 3],
    pub fresnel_power: f32,
    pub normal: Normal,
    pub maps: Maps,
    pub content: Content,
    pub content_layer: ContentLayer,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<NoiseLayer>,
    pub ageing: Ageing,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wet: Option<Wet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fabric: Option<Fabric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anisotropy: Option<Anisotropy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scatter: Option<Scatter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thin_walled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha_mode: Option<AlphaMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha_cutoff: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flakes: Option<Flakes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<serde_json::Value>"))]
    pub authoring: Option<toml::Value>,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            id: None,
            family: Family::Plain,
            base: [0.5; 3],
            roughness: 0.5,
            metalness: 0.0,
            specular: 0.04,
            clearcoat: 0.0,
            clearcoat_roughness: 0.05,
            sheen: 0.0,
            transmission: 0.0,
            ior: 1.5,
            dispersion: 0.0,
            thickness: 0.0,
            subsurface: 0.0,
            subsurface_tint: [1.0; 3],
            absorption: 0.0,
            thin_film: 0.0,
            thin_film_ior: 1.5,
            thin_film_amount: 0.0,
            emission: [0.0; 3],
            fresnel_power: 5.0,
            normal: Normal::default(),
            maps: Maps::default(),
            content: Content::None,
            content_layer: ContentLayer::default(),
            layers: Vec::new(),
            ageing: Ageing::default(),
            wet: None,
            fabric: None,
            anisotropy: None,
            scatter: None,
            thin_walled: None,
            alpha: None,
            alpha_mode: None,
            alpha_cutoff: None,
            flakes: None,
            authoring: None,
        }
    }
}

impl Material {
    pub fn to_table(&self) -> toml::Table {
        let base = Material {
            id: self.id,
            authoring: self.authoring.clone(),
            ..Material::default()
        };
        let full = toml::Table::try_from(self).unwrap_or_default();
        let plain = toml::Table::try_from(&base).unwrap_or_default();
        full.into_iter()
            .filter(|(key, value)| {
                key == "id" || key == "authoring" || plain.get(key) != Some(value)
            })
            .collect()
    }
}
