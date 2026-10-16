use serde::{Deserialize, Serialize};

use crate::material::Material;
use crate::math::{clamp01, lerp, lerp3, normalize, scale3, smoothstep};
use crate::noise::{Hash, fbm3_filtered};

pub const WATER_ROUGHNESS: f32 = 0.03;
pub const WET_DARKEST: f32 = 0.2;
pub const PORE_START: f32 = 0.5;
pub const PORE_SPAN: f32 = 0.4;
pub const PUDDLE_OCTAVES: i32 = 3;
pub const PUDDLE_EDGE: f32 = 0.04;
pub const PUDDLE_MEDIAN: f32 = 0.4375;
pub const PUDDLE_SPREAD: [f32; 2] = [0.24, 2.1];
pub const PUDDLE_FLAT: [f32; 2] = [0.8, 0.95];
pub const PUDDLE_SCALE_MIN: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Wet {
    pub amount: f32,
    pub puddles: f32,
    pub puddle_scale: f32,
    pub seed: u64,
}

impl Default for Wet {
    fn default() -> Self {
        Self {
            amount: 0.0,
            puddles: 0.0,
            puddle_scale: 2.0,
            seed: 0,
        }
    }
}

impl Wet {
    pub fn inactive(self) -> bool {
        !(self.amount > 0.0 || self.puddles > 0.0)
    }

    pub fn seed_bits(self) -> u32 {
        (self.seed ^ (self.seed >> 32)) as u32
    }

    pub fn packed(self) -> [f32; 4] {
        [
            self.amount,
            self.puddles,
            self.puddle_scale,
            f32::from_bits(self.seed_bits()),
        ]
    }

    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

pub fn puddle(wet: Wet, position: [f32; 3], up: f32, footprint: f32) -> f32 {
    if wet.puddles <= 0.0 {
        return 0.0;
    }
    let scale = wet.puddle_scale.max(PUDDLE_SCALE_MIN);
    let seed = wet.seed_bits();
    let q = [
        position[0] / scale,
        (seed & 0xffff) as f32 * 1.37,
        position[2] / scale,
    ];
    let mask = fbm3_filtered(q, PUDDLE_OCTAVES, footprint / scale, Hash::Xor);
    let x = clamp01(wet.puddles) - 0.5;
    let threshold = PUDDLE_MEDIAN - x * (PUDDLE_SPREAD[0] + PUDDLE_SPREAD[1] * x * x);
    smoothstep(threshold - PUDDLE_EDGE, threshold + PUDDLE_EDGE, mask)
        * smoothstep(PUDDLE_FLAT[0], PUDDLE_FLAT[1], up)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Soaked {
    pub material: Material,
    pub normal: [f32; 3],
    pub water: f32,
}

pub fn soak(
    material: &Material,
    normal: [f32; 3],
    geometric: [f32; 3],
    position: [f32; 3],
    footprint: f32,
) -> Soaked {
    let wet = material.wet;
    let mut out = *material;
    if wet.inactive() {
        return Soaked {
            material: out,
            normal,
            water: 0.0,
        };
    }
    let water = puddle(wet, position, geometric[1], footprint);
    let level = clamp01(wet.amount.max(water));
    if level <= 0.0 {
        return Soaked {
            material: out,
            normal,
            water,
        };
    }
    let porosity = clamp01((out.roughness - PORE_START) / PORE_SPAN);
    let factor = lerp(1.0, WET_DARKEST, (1.0 - clamp01(out.metalness)) * porosity);
    out.base = scale3(out.base, lerp(1.0, factor, level));
    out.roughness *= lerp(1.0, factor, 0.5 * level);
    let film = lerp(out.roughness, WATER_ROUGHNESS, water);
    out.clearcoat_roughness = if out.clearcoat > 0.0 {
        out.clearcoat_roughness.min(film)
    } else {
        film
    };
    out.clearcoat = out.clearcoat.max(lerp(0.5 * level, 1.0, water));
    let flat = normalize(lerp3(normal, geometric, water));
    Soaked {
        material: out,
        normal: flat,
        water,
    }
}
