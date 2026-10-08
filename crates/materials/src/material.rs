use serde::{Deserialize, Serialize};

use crate::content::ContentLayer;

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
}

impl NoiseKind {
    fn id(self) -> u32 {
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
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseLayer {
    pub kind: NoiseKind,
    pub frequency: f32,
    pub amplitude: f32,
    pub seed: u32,
    #[serde(default, skip_serializing_if = "unset")]
    pub params: [f32; PARAMS],
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
        _ => {}
    }
    out
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

    pub fn active(self) -> bool {
        self.kind != NoiseKind::None
            && self.amplitude != 0.0
            && (!matches!(self.kind, NoiseKind::PlankWood | NoiseKind::CoatWobble)
                || self.params[0] > 0.0)
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
    pub normal: Normal,
    pub maps: Maps,
    pub content: Content,
    pub content_layer: ContentLayer,
    pub layers: [NoiseLayer; LAYER_CAP],
    pub ageing: Ageing,
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
            normal: Normal::default(),
            maps: Maps::default(),
            content: Content::None,
            content_layer: ContentLayer::default(),
            layers: [NoiseLayer::default(); LAYER_CAP],
            ageing: Ageing::default(),
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
    normal: Normal,
    maps: Maps,
    content: Content,
    content_layer: ContentLayer,
    layers: Vec<NoiseLayer>,
    ageing: Ageing,
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
            normal: self.normal,
            maps: self.maps,
            content: self.content,
            content_layer: self.content_layer,
            layers,
            ageing: self.ageing,
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
}

pub const PARAM_ROWS: usize = LAYER_CAP * GPU_PARAMS / 4;

impl PackedMaterial {
    pub const SIZE: usize = 240 + PARAM_ROWS * 16;
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
            tail: [f32::from_bits(m.ageing.seed), 0.0, 0.0, 0.0],
            params: std::array::from_fn(|row| {
                let params = gpu_params(&m.layers[row / (GPU_PARAMS / 4)]);
                let at = row % (GPU_PARAMS / 4) * 4;
                [params[at], params[at + 1], params[at + 2], params[at + 3]]
            }),
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
        Material {
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
        }
    }
}
