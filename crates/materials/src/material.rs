use serde::{Deserialize, Deserializer, Serialize};

use crate::content::ContentLayer;
use crate::flakes::Flakes;
use crate::lobes::{Anisotropy, Fabric, Scatter, Textures, UvTransform};
use crate::wet::Wet;

pub const LAYER_CAP: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
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

impl Family {
    fn id(self) -> u32 {
        match self {
            Self::Plain => 0,
            Self::Paper => 1,
            Self::Plaster => 2,
            Self::Stone => 3,
            Self::Cloth => 4,
            Self::Metal => 5,
            Self::Occluder => 7,
            Self::Petal => 8,
            Self::Leaf => 9,
            Self::Bark => 10,
            Self::Cord => 11,
            Self::Wood => 12,
            Self::Lacquer => 13,
            Self::Glass => 14,
            Self::Liquid => 15,
            Self::Emissive => 17,
            Self::Chrome => 18,
        }
    }

    fn from_id(id: u32) -> Self {
        match id {
            1 => Self::Paper,
            2 => Self::Plaster,
            3 => Self::Stone,
            4 => Self::Cloth,
            5 => Self::Metal,
            7 => Self::Occluder,
            8 => Self::Petal,
            9 => Self::Leaf,
            10 => Self::Bark,
            11 => Self::Cord,
            12 => Self::Wood,
            13 => Self::Lacquer,
            14 => Self::Glass,
            15 => Self::Liquid,
            17 => Self::Emissive,
            18 => Self::Chrome,
            _ => Self::Plain,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
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

impl Content {
    fn id(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Ink => 1,
            Self::Decal => 2,
            Self::Screen => 3,
            Self::Photo => 4,
            Self::Scroll => 5,
            Self::Print => 6,
            Self::Field => 7,
        }
    }

    fn from_id(id: u32) -> Self {
        match id {
            1 => Self::Ink,
            2 => Self::Decal,
            3 => Self::Screen,
            4 => Self::Photo,
            5 => Self::Scroll,
            6 => Self::Print,
            7 => Self::Field,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

impl AlphaMode {
    pub fn id(self) -> u32 {
        match self {
            Self::Opaque => 0,
            Self::Mask => 1,
            Self::Blend => 2,
        }
    }

    fn from_id(id: u32) -> Self {
        match id {
            1 => Self::Mask,
            2 => Self::Blend,
            _ => Self::Opaque,
        }
    }

    pub fn opaque(&self) -> bool {
        *self == Self::Opaque
    }
}

pub const ALPHA_CUTOFF: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalSource {
    #[default]
    Flat,
    Bump,
    Map,
}

impl NormalSource {
    fn id(self) -> u32 {
        match self {
            Self::Flat => 0,
            Self::Bump => 1,
            Self::Map => 2,
        }
    }

    fn from_id(id: u32) -> Self {
        match id {
            1 => Self::Bump,
            2 => Self::Map,
            _ => Self::Flat,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
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
}

impl NoiseKind {
    pub const ALL: [NoiseKind; 15] = [
        Self::None,
        Self::Fibre,
        Self::Crinkle,
        Self::PlankWood,
        Self::WallMottle,
        Self::Grime,
        Self::Leaf,
        Self::Bark,
        Self::Flow,
        Self::Value,
        Self::Fbm,
        Self::Scratch,
        Self::CoatWobble,
        Self::Crack,
        Self::Track,
    ];

    pub fn id(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Fibre => 1,
            Self::Crinkle => 2,
            Self::PlankWood => 3,
            Self::WallMottle => 4,
            Self::Grime => 5,
            Self::Leaf => 7,
            Self::Bark => 8,
            Self::Flow => 14,
            Self::Value => 15,
            Self::Fbm => 16,
            Self::Scratch => 17,
            Self::CoatWobble => 18,
            Self::Crack => 19,
            Self::Track => 20,
        }
    }

    fn from_id(id: u32) -> Self {
        match id {
            1 => Self::Fibre,
            2 => Self::Crinkle,
            3 => Self::PlankWood,
            4 => Self::WallMottle,
            5 => Self::Grime,
            7 => Self::Leaf,
            8 => Self::Bark,
            14 => Self::Flow,
            15 => Self::Value,
            16 => Self::Fbm,
            17 => Self::Scratch,
            18 => Self::CoatWobble,
            19 => Self::Crack,
            20 => Self::Track,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, from = "LayerFile")]
pub struct NoiseLayer {
    pub kind: NoiseKind,
    pub frequency: f32,
    pub amplitude: f32,
    pub seed: u32,
    #[serde(default, skip_serializing_if = "unset")]
    pub params: [f32; PARAMS],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerFile {
    kind: NoiseKind,
    frequency: f32,
    amplitude: f32,
    seed: u32,
    #[serde(default)]
    params: [f32; PARAMS],
}

impl From<LayerFile> for NoiseLayer {
    fn from(file: LayerFile) -> Self {
        NoiseLayer::new(file.kind, file.frequency, file.amplitude, file.seed)
            .with_params(file.params)
            .filled()
    }
}

fn read_layers<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<[NoiseLayer; LAYER_CAP], D::Error> {
    let given = Vec::<NoiseLayer>::deserialize(deserializer)?;
    if given.len() > LAYER_CAP {
        return Err(serde::de::Error::custom(format!(
            "a material takes at most {LAYER_CAP} noise layers, got {}",
            given.len()
        )));
    }
    let mut layers = [NoiseLayer::default(); LAYER_CAP];
    for (slot, layer) in layers.iter_mut().zip(given) {
        *slot = layer;
    }
    Ok(layers)
}

pub const PARAMS: usize = 12;
pub const GPU_PARAMS: usize = PARAMS + 4;

pub const SCRATCH_STRETCH: f32 = 0.02;

pub fn gpu_params(layer: &NoiseLayer) -> [f32; GPU_PARAMS] {
    let p = layer.params;
    let mut out = [0.0; GPU_PARAMS];
    out[..PARAMS].copy_from_slice(&p);
    match layer.kind {
        NoiseKind::PlankWood if p[0] > 0.0 => {
            out[PARAMS] = 1.0 / p[0];
            out[PARAMS + 1] = 1.0 - p[7];
        }
        NoiseKind::Scratch => out[PARAMS] = p[0] * SCRATCH_STRETCH,
        NoiseKind::Grime => out[PARAMS] = crate::kinds::Grime::from_params(&p).faded_roughness(),
        _ => {}
    }
    out
}

fn unit(value: &f32) -> bool {
    *value == 1.0
}

fn half(value: &f32) -> bool {
    *value == ALPHA_CUTOFF
}

fn unset(params: &[f32; PARAMS]) -> bool {
    params.iter().all(|value| *value == 0.0)
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

impl NoiseLayer {
    pub fn new(kind: NoiseKind, frequency: f32, amplitude: f32, seed: u32) -> Self {
        Self {
            kind,
            frequency,
            amplitude,
            seed,
            params: [0.0; PARAMS],
        }
    }

    pub fn with_params(self, params: [f32; PARAMS]) -> Self {
        Self { params, ..self }
    }

    pub fn filled(self) -> Self {
        match crate::kinds::sample_params(self.kind) {
            Some(params) if unset(&self.params) => Self { params, ..self },
            _ => self,
        }
    }

    pub fn active(self) -> bool {
        self.kind != NoiseKind::None
            && self.amplitude != 0.0
            && (!matches!(
                self.kind,
                NoiseKind::PlankWood | NoiseKind::CoatWobble | NoiseKind::Crack | NoiseKind::Track
            ) || self.params[0] > 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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

impl Default for Ageing {
    fn default() -> Self {
        Self {
            fade: 0.0,
            yellow: 0.0,
            ink: 0.0,
            scratch: 0.0,
            edge: 0.0,
            dust: 0.0,
            patina: 0.0,
            seed: 0,
        }
    }
}

impl Ageing {
    pub fn inactive(self) -> bool {
        self.fade == 0.0
            && self.yellow == 0.0
            && self.ink == 0.0
            && self.scratch == 0.0
            && self.edge == 0.0
            && self.dust == 0.0
            && self.patina == 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct At {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    pub edge: f32,
    pub time: f32,
}

impl Default for At {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            uv: [0.0; 2],
            normal: [0.0, 0.0, 1.0],
            edge: 0.0,
            time: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Material {
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
    pub thin_walled: bool,
    pub alpha: f32,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub normal: Normal,
    pub maps: Maps,
    pub content: Content,
    pub content_layer: ContentLayer,
    #[serde(deserialize_with = "read_layers")]
    pub layers: [NoiseLayer; LAYER_CAP],
    pub ageing: Ageing,
    pub wet: Wet,
    pub fabric: Fabric,
    pub anisotropy: Anisotropy,
    pub scatter: Scatter,
    pub flakes: Flakes,
    #[serde(skip)]
    pub textures: Textures,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            family: Family::Plain,
            base: [0.5, 0.5, 0.5],
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
            subsurface_tint: [1.0, 1.0, 1.0],
            absorption: 0.0,
            thin_film: 0.0,
            thin_film_ior: 1.5,
            thin_film_amount: 0.0,
            emission: [0.0; 3],
            fresnel_power: 5.0,
            thin_walled: false,
            alpha: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: ALPHA_CUTOFF,
            normal: Normal::default(),
            maps: Maps::default(),
            content: Content::None,
            content_layer: ContentLayer::default(),
            layers: [NoiseLayer::default(); LAYER_CAP],
            ageing: Ageing::default(),
            wet: Wet::default(),
            fabric: Fabric::default(),
            anisotropy: Anisotropy::default(),
            scatter: Scatter::default(),
            flakes: Flakes::default(),
            textures: Textures::default(),
        }
    }
}

impl Material {
    pub fn push_layer(&mut self, layer: NoiseLayer) {
        if let Some(slot) = self
            .layers
            .iter_mut()
            .find(|layer| layer.kind == NoiseKind::None)
        {
            *slot = layer;
        }
    }

    pub fn transmitted(&self) -> f32 {
        self.transmission.clamp(0.0, 1.0) * (1.0 - self.metalness.clamp(0.0, 1.0))
    }

    pub fn refracts(&self) -> bool {
        self.transmitted() > 0.0 || (self.family == Family::Glass && self.transmission <= 0.0)
    }

    pub fn looks_metallic_glass(&self) -> bool {
        self.metalness > 0.5 && self.transmission > 0.0
    }

    pub fn to_toml(&self) -> Result<String, String> {
        let recipe = Recipe::from(*self);
        toml::to_string(&recipe).map_err(|err| err.to_string())
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let recipe: Recipe = toml::from_str(text).map_err(|err| err.to_string())?;
        recipe.into_material()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Recipe {
    family: Family,
    base: [f32; 3],
    roughness: f32,
    metalness: f32,
    specular: f32,
    clearcoat: f32,
    clearcoat_roughness: f32,
    sheen: f32,
    transmission: f32,
    ior: f32,
    dispersion: f32,
    thickness: f32,
    subsurface: f32,
    subsurface_tint: [f32; 3],
    absorption: f32,
    thin_film: f32,
    thin_film_ior: f32,
    thin_film_amount: f32,
    emission: [f32; 3],
    fresnel_power: f32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    thin_walled: bool,
    #[serde(skip_serializing_if = "unit")]
    alpha: f32,
    #[serde(skip_serializing_if = "AlphaMode::opaque")]
    alpha_mode: AlphaMode,
    #[serde(skip_serializing_if = "half")]
    alpha_cutoff: f32,
    normal: Normal,
    maps: Maps,
    content: Content,
    content_layer: ContentLayer,
    layers: Vec<NoiseLayer>,
    ageing: Ageing,
    #[serde(skip_serializing_if = "Wet::is_default")]
    wet: Wet,
    #[serde(skip_serializing_if = "Fabric::off")]
    fabric: Fabric,
    #[serde(skip_serializing_if = "Anisotropy::off")]
    anisotropy: Anisotropy,
    #[serde(skip_serializing_if = "Scatter::off")]
    scatter: Scatter,
    #[serde(skip_serializing_if = "Flakes::off")]
    flakes: Flakes,
}

impl Default for Recipe {
    fn default() -> Self {
        Self::from(Material::default())
    }
}

impl From<Material> for Recipe {
    fn from(m: Material) -> Self {
        Self {
            family: m.family,
            base: m.base,
            roughness: m.roughness,
            metalness: m.metalness,
            specular: m.specular,
            clearcoat: m.clearcoat,
            clearcoat_roughness: m.clearcoat_roughness,
            sheen: m.sheen,
            transmission: m.transmission,
            ior: m.ior,
            dispersion: m.dispersion,
            thickness: m.thickness,
            subsurface: m.subsurface,
            subsurface_tint: m.subsurface_tint,
            absorption: m.absorption,
            thin_film: m.thin_film,
            thin_film_ior: m.thin_film_ior,
            thin_film_amount: m.thin_film_amount,
            emission: m.emission,
            fresnel_power: m.fresnel_power,
            thin_walled: m.thin_walled,
            alpha: m.alpha,
            alpha_mode: m.alpha_mode,
            alpha_cutoff: m.alpha_cutoff,
            normal: m.normal,
            maps: m.maps,
            content: m.content,
            content_layer: m.content_layer,
            layers: m
                .layers
                .into_iter()
                .filter(|layer| layer.kind != NoiseKind::None)
                .collect(),
            ageing: m.ageing,
            wet: m.wet,
            fabric: m.fabric,
            anisotropy: m.anisotropy,
            scatter: m.scatter,
            flakes: m.flakes,
        }
    }
}

impl Recipe {
    pub(crate) fn into_material(self) -> Result<Material, String> {
        if self.layers.len() > LAYER_CAP {
            return Err(format!(
                "a material takes at most {LAYER_CAP} noise layers, got {}",
                self.layers.len()
            ));
        }
        let mut layers = [NoiseLayer::default(); LAYER_CAP];
        for (slot, layer) in layers.iter_mut().zip(self.layers) {
            *slot = layer;
        }
        Ok(Material {
            family: self.family,
            base: self.base,
            roughness: self.roughness,
            metalness: self.metalness,
            specular: self.specular,
            clearcoat: self.clearcoat,
            clearcoat_roughness: self.clearcoat_roughness,
            sheen: self.sheen,
            transmission: self.transmission,
            ior: self.ior,
            dispersion: self.dispersion,
            thickness: self.thickness,
            subsurface: self.subsurface,
            subsurface_tint: self.subsurface_tint,
            absorption: self.absorption,
            thin_film: self.thin_film,
            thin_film_ior: self.thin_film_ior,
            thin_film_amount: self.thin_film_amount,
            emission: self.emission,
            fresnel_power: self.fresnel_power,
            thin_walled: self.thin_walled,
            alpha: self.alpha,
            alpha_mode: self.alpha_mode,
            alpha_cutoff: self.alpha_cutoff,
            normal: self.normal,
            maps: self.maps,
            content: self.content,
            content_layer: self.content_layer,
            layers,
            ageing: self.ageing,
            wet: self.wet,
            fabric: self.fabric,
            anisotropy: self.anisotropy,
            scatter: self.scatter,
            flakes: self.flakes,
            textures: Textures::default(),
        })
    }
}

fn pack_id(id: u32) -> f32 {
    id as f32
}

fn unpack_id(v: f32) -> u32 {
    v.round().max(0.0) as u32
}

#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PackedMaterial {
    pub base_roughness: [f32; 4],
    pub metal_spec_coat: [f32; 4],
    pub sheen_trans_ior_disp: [f32; 4],
    pub thick_sub_film: [f32; 4],
    pub tint_absorption: [f32; 4],
    pub emission_film: [f32; 4],
    pub normal_maps: [f32; 4],
    pub maps_content: [f32; 4],
    pub age_a: [f32; 4],
    pub age_b: [f32; 4],
    pub layer0: [f32; 4],
    pub layer1: [f32; 4],
    pub layer2: [f32; 4],
    pub layer3: [f32; 4],
    pub tail: [f32; 4],
    pub params: [[f32; 4]; PARAM_ROWS],
    pub wet: [f32; 4],
    pub sheen: [f32; 4],
    pub lobes: [f32; 4],
    pub scatter: [f32; 4],
    pub textures: [f32; 4],
    pub texture_scales: [f32; 4],
    pub texture_extra: [f32; 4],
    pub texture_transform: [f32; 4],
    pub detail_transform: [f32; 4],
    pub normal_transform: [f32; 4],
    pub orm_transform: [f32; 4],
    pub map_turns: [f32; 4],
    pub coverage: [f32; 4],
    pub flakes: [f32; 4],
    pub flake_color: [f32; 4],
}

pub const PARAM_ROWS: usize = LAYER_CAP * GPU_PARAMS / 4;
pub const LOBE_ROWS: usize = 14;

impl PackedMaterial {
    pub const SIZE: usize = 256 + PARAM_ROWS * 16 + LOBE_ROWS * 16;
    pub const LOBES_OFFSET: usize = 256 + PARAM_ROWS * 16;
    pub const ALIGN: usize = 16;

    pub fn pack(m: &Material) -> Self {
        let layer = |i: usize| {
            let layer = m.layers[i];
            [
                pack_id(layer.kind.id()),
                layer.frequency,
                layer.amplitude,
                f32::from_bits(layer.seed),
            ]
        };
        Self {
            base_roughness: [m.base[0], m.base[1], m.base[2], m.roughness],
            metal_spec_coat: [m.metalness, m.specular, m.clearcoat, m.clearcoat_roughness],
            sheen_trans_ior_disp: [m.sheen, m.transmission, m.ior, m.dispersion],
            thick_sub_film: [m.thickness, m.subsurface, m.thin_film, m.thin_film_ior],
            tint_absorption: [
                m.subsurface_tint[0],
                m.subsurface_tint[1],
                m.subsurface_tint[2],
                m.absorption,
            ],
            emission_film: [
                m.emission[0],
                m.emission[1],
                m.emission[2],
                m.thin_film_amount,
            ],
            normal_maps: [
                pack_id(m.normal.source.id()),
                m.normal.strength,
                m.maps.layer,
                m.maps.tile,
            ],
            maps_content: [
                m.maps.normal,
                m.maps.albedo,
                pack_id(m.content.id()),
                pack_id(m.family.id()),
            ],
            age_a: [
                m.ageing.fade,
                m.ageing.yellow,
                m.ageing.ink,
                m.ageing.scratch,
            ],
            age_b: [
                m.ageing.edge,
                m.ageing.dust,
                m.ageing.patina,
                m.fresnel_power,
            ],
            layer0: layer(0),
            layer1: layer(1),
            layer2: layer(2),
            layer3: layer(3),
            tail: [
                f32::from_bits(m.ageing.seed),
                f32::from(u8::from(m.thin_walled)),
                0.0,
                0.0,
            ],
            params: std::array::from_fn(|row| {
                let params = gpu_params(&m.layers[row / (GPU_PARAMS / 4)]);
                let at = row % (GPU_PARAMS / 4) * 4;
                [params[at], params[at + 1], params[at + 2], params[at + 3]]
            }),
            wet: m.wet.packed(),
            sheen: [
                m.fabric.color[0],
                m.fabric.color[1],
                m.fabric.color[2],
                m.fabric.roughness,
            ],
            lobes: [
                m.anisotropy.strength,
                m.anisotropy.rotation,
                m.scatter.amount,
                m.scatter.radius,
            ],
            scatter: [
                m.scatter.color[0],
                m.scatter.color[1],
                m.scatter.color[2],
                0.0,
            ],
            textures: [
                m.textures.base as f32,
                m.textures.normal as f32,
                m.textures.orm as f32,
                m.textures.detail as f32,
            ],
            texture_scales: [
                m.textures.normal_scale,
                m.textures.occlusion,
                m.textures.detail_scale,
                (m.textures.main.set + 2 * m.textures.detail_uv.set) as f32,
            ],
            texture_extra: [
                m.textures.anisotropy as f32,
                0.0,
                m.textures.main.rotation,
                m.textures.detail_uv.rotation,
            ],
            texture_transform: [
                m.textures.main.offset[0],
                m.textures.main.offset[1],
                m.textures.main.scale[0],
                m.textures.main.scale[1],
            ],
            detail_transform: [
                m.textures.detail_uv.offset[0],
                m.textures.detail_uv.offset[1],
                m.textures.detail_uv.scale[0],
                m.textures.detail_uv.scale[1],
            ],
            normal_transform: placed(m.textures.normal_transform()),
            orm_transform: placed(m.textures.orm_transform()),
            map_turns: [
                m.textures.normal_transform().rotation,
                m.textures.orm_transform().rotation,
                (m.textures.normal_transform().set + 2 * m.textures.orm_transform().set) as f32,
                0.0,
            ],
            coverage: [m.alpha, m.alpha_cutoff, pack_id(m.alpha_mode.id()), 0.0],
            flakes: [
                m.flakes.amount,
                m.flakes.size,
                m.flakes.spread,
                m.flakes.roughness,
            ],
            flake_color: [m.flakes.color[0], m.flakes.color[1], m.flakes.color[2], 0.0],
        }
    }

    pub fn unpack(self) -> Material {
        let layer = |row: [f32; 4], index: usize| {
            let kind = NoiseKind::from_id(unpack_id(row[0]));
            let params: [f32; PARAMS] =
                std::array::from_fn(|k| self.params[index * GPU_PARAMS / 4 + k / 4][k % 4]);
            NoiseLayer {
                kind,
                frequency: row[1],
                amplitude: row[2],
                seed: row[3].to_bits(),
                params,
            }
        };
        let mut material = Material {
            family: Family::from_id(unpack_id(self.maps_content[3])),
            base: [
                self.base_roughness[0],
                self.base_roughness[1],
                self.base_roughness[2],
            ],
            roughness: self.base_roughness[3],
            metalness: self.metal_spec_coat[0],
            specular: self.metal_spec_coat[1],
            clearcoat: self.metal_spec_coat[2],
            clearcoat_roughness: self.metal_spec_coat[3],
            sheen: self.sheen_trans_ior_disp[0],
            transmission: self.sheen_trans_ior_disp[1],
            ior: self.sheen_trans_ior_disp[2],
            dispersion: self.sheen_trans_ior_disp[3],
            thickness: self.thick_sub_film[0],
            subsurface: self.thick_sub_film[1],
            thin_film: self.thick_sub_film[2],
            thin_film_ior: self.thick_sub_film[3],
            subsurface_tint: [
                self.tint_absorption[0],
                self.tint_absorption[1],
                self.tint_absorption[2],
            ],
            absorption: self.tint_absorption[3],
            emission: [
                self.emission_film[0],
                self.emission_film[1],
                self.emission_film[2],
            ],
            thin_film_amount: self.emission_film[3],
            fresnel_power: self.age_b[3],
            thin_walled: self.tail[1] != 0.0,
            alpha: self.coverage[0],
            alpha_mode: AlphaMode::from_id(unpack_id(self.coverage[2])),
            alpha_cutoff: self.coverage[1],
            normal: Normal {
                source: NormalSource::from_id(unpack_id(self.normal_maps[0])),
                strength: self.normal_maps[1],
            },
            maps: Maps {
                layer: self.normal_maps[2],
                tile: self.normal_maps[3],
                normal: self.maps_content[0],
                albedo: self.maps_content[1],
            },
            content: Content::from_id(unpack_id(self.maps_content[2])),
            content_layer: ContentLayer::default(),
            layers: [
                layer(self.layer0, 0),
                layer(self.layer1, 1),
                layer(self.layer2, 2),
                layer(self.layer3, 3),
            ],
            ageing: Ageing {
                fade: self.age_a[0],
                yellow: self.age_a[1],
                ink: self.age_a[2],
                scratch: self.age_a[3],
                edge: self.age_b[0],
                dust: self.age_b[1],
                patina: self.age_b[2],
                seed: self.tail[0].to_bits(),
            },
            wet: Wet {
                amount: self.wet[0],
                puddles: self.wet[1],
                puddle_scale: self.wet[2],
                seed: u64::from(self.wet[3].to_bits()),
            },
            fabric: Fabric {
                color: [self.sheen[0], self.sheen[1], self.sheen[2]],
                roughness: self.sheen[3],
            },
            anisotropy: Anisotropy {
                strength: self.lobes[0],
                rotation: self.lobes[1],
            },
            scatter: Scatter {
                color: [self.scatter[0], self.scatter[1], self.scatter[2]],
                radius: self.lobes[3],
                amount: self.lobes[2],
            },
            flakes: Flakes {
                amount: self.flakes[0],
                size: self.flakes[1],
                spread: self.flakes[2],
                color: [
                    self.flake_color[0],
                    self.flake_color[1],
                    self.flake_color[2],
                ],
                roughness: self.flakes[3],
            },
            textures: Textures {
                base: self.textures[0] as i32,
                normal: self.textures[1] as i32,
                orm: self.textures[2] as i32,
                detail: self.textures[3] as i32,
                anisotropy: self.texture_extra[0] as i32,
                normal_scale: self.texture_scales[0],
                occlusion: self.texture_scales[1],
                detail_scale: self.texture_scales[2],
                main: UvTransform {
                    set: self.texture_scales[3] as u32 % 2,
                    offset: [self.texture_transform[0], self.texture_transform[1]],
                    rotation: self.texture_extra[2],
                    scale: [self.texture_transform[2], self.texture_transform[3]],
                },
                detail_uv: UvTransform {
                    set: self.texture_scales[3] as u32 / 2,
                    offset: [self.detail_transform[0], self.detail_transform[1]],
                    rotation: self.texture_extra[3],
                    scale: [self.detail_transform[2], self.detail_transform[3]],
                },
                normal_uv: None,
                orm_uv: None,
            },
        };
        let main = material.textures.main;
        let own = |row: [f32; 4], rotation: f32, set: u32| {
            let found = UvTransform {
                set,
                offset: [row[0], row[1]],
                rotation,
                scale: [row[2], row[3]],
            };
            (found != main).then_some(found)
        };
        let sets = self.map_turns[2] as u32;
        material.textures.normal_uv = own(self.normal_transform, self.map_turns[0], sets % 2);
        material.textures.orm_uv = own(self.orm_transform, self.map_turns[1], sets / 2);
        material
    }
}

fn placed(transform: UvTransform) -> [f32; 4] {
    [
        transform.offset[0],
        transform.offset[1],
        transform.scale[0],
        transform.scale[1],
    ]
}

#[cfg(test)]
mod kind_table {
    use super::NoiseKind;

    #[test]
    fn every_layer_kind_the_packer_reads_is_listed_once_and_keeps_its_id() {
        for (at, kind) in NoiseKind::ALL.iter().enumerate() {
            assert_eq!(NoiseKind::from_id(kind.id()), *kind);
            assert!(!NoiseKind::ALL[..at].contains(kind), "{kind:?} twice");
        }
        for id in 0..256 {
            let kind = NoiseKind::from_id(id);
            assert!(NoiseKind::ALL.contains(&kind), "{kind:?} is missing");
        }
    }
}
