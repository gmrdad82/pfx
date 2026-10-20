use serde::{Deserialize, Serialize};

use crate::brdf::PI;

pub const SHEEN_ROUGHNESS_FLOOR: f32 = 0.045;
pub const SHEEN_TABLE_SIDE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fabric {
    pub color: [f32; 3],
    pub roughness: f32,
}

impl Default for Fabric {
    fn default() -> Self {
        Self {
            color: [0.0; 3],
            roughness: 0.0,
        }
    }
}

impl Fabric {
    pub fn off(&self) -> bool {
        self.color.iter().all(|value| *value <= 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Anisotropy {
    pub strength: f32,
    pub rotation: f32,
}

impl Default for Anisotropy {
    fn default() -> Self {
        Self {
            strength: 0.0,
            rotation: 0.0,
        }
    }
}

impl Anisotropy {
    pub fn off(&self) -> bool {
        self.strength == 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scatter {
    pub color: [f32; 3],
    pub radius: f32,
    pub amount: f32,
}

impl Default for Scatter {
    fn default() -> Self {
        Self {
            color: [1.0, 0.35, 0.2],
            radius: 0.01,
            amount: 0.0,
        }
    }
}

impl Scatter {
    pub fn off(&self) -> bool {
        self.amount <= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    pub set: u32,
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
}

impl Default for UvTransform {
    fn default() -> Self {
        Self {
            set: 0,
            offset: [0.0; 2],
            rotation: 0.0,
            scale: [1.0; 2],
        }
    }
}

impl UvTransform {
    pub fn apply(&self, uv: [f32; 2]) -> [f32; 2] {
        let (s, c) = self.rotation.sin_cos();
        let x = uv[0] * self.scale[0];
        let y = uv[1] * self.scale[1];
        [
            self.offset[0] + c * x - s * y,
            self.offset[1] + s * x + c * y,
        ]
    }
}

pub const THIN_FILM_MIN: f32 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Textures {
    pub base: i32,
    pub normal: i32,
    pub orm: i32,
    pub detail: i32,
    pub anisotropy: i32,
    pub thin_film: i32,
    pub thin_film_min: f32,
    pub normal_scale: f32,
    pub occlusion: f32,
    pub detail_scale: f32,
    pub main: UvTransform,
    pub detail_uv: UvTransform,
    pub normal_uv: Option<UvTransform>,
    pub orm_uv: Option<UvTransform>,
}

impl Default for Textures {
    fn default() -> Self {
        Self {
            base: -1,
            normal: -1,
            orm: -1,
            detail: -1,
            anisotropy: -1,
            thin_film: -1,
            thin_film_min: THIN_FILM_MIN,
            normal_scale: 1.0,
            occlusion: 1.0,
            detail_scale: 1.0,
            main: UvTransform::default(),
            detail_uv: UvTransform {
                set: 1,
                ..UvTransform::default()
            },
            normal_uv: None,
            orm_uv: None,
        }
    }
}

impl Textures {
    pub fn normal_transform(&self) -> UvTransform {
        self.normal_uv.unwrap_or(self.main)
    }

    pub fn orm_transform(&self) -> UvTransform {
        self.orm_uv.unwrap_or(self.main)
    }

    pub fn any(&self) -> bool {
        self.base >= 0
            || self.normal >= 0
            || self.orm >= 0
            || self.detail >= 0
            || self.anisotropy >= 0
    }
}

pub fn d_charlie(alpha: f32, ndh: f32) -> f32 {
    let inv = 1.0 / alpha;
    let cos2 = ndh * ndh;
    let sin2 = (1.0 - cos2).max(0.0078125);
    (2.0 + inv) * sin2.powf(inv * 0.5) / (2.0 * PI)
}

pub fn v_neubelt(ndv: f32, ndl: f32) -> f32 {
    (1.0 / (4.0 * (ndl + ndv - ndl * ndv))).clamp(0.0, 65504.0)
}

pub fn sheen_alpha(roughness: f32) -> f32 {
    let r = roughness.clamp(SHEEN_ROUGHNESS_FLOOR, 1.0);
    r * r
}

pub fn sheen_albedo_exact(ndv: f32, roughness: f32) -> f32 {
    let ndv = ndv.clamp(1e-3, 1.0);
    let alpha = sheen_alpha(roughness);
    let view = [(1.0 - ndv * ndv).max(0.0).sqrt(), 0.0, ndv];
    let rings = 512;
    let spokes = 64;
    let mut total = 0.0f64;
    for i in 0..rings {
        let theta = (i as f64 + 0.5) / rings as f64 * std::f64::consts::FRAC_PI_2;
        let (sin_t, cos_t) = theta.sin_cos();
        for j in 0..spokes {
            let phi = (j as f64 + 0.5) / spokes as f64 * std::f64::consts::PI;
            let h = [sin_t * phi.cos(), sin_t * phi.sin(), cos_t];
            let vdh = view[0] as f64 * h[0] + view[2] as f64 * h[2];
            if vdh <= 0.0 {
                continue;
            }
            let ndl = 2.0 * vdh * h[2] - view[2] as f64;
            if ndl <= 0.0 {
                continue;
            }
            let d = d_charlie(alpha, cos_t as f32) as f64;
            let v = v_neubelt(ndv, ndl as f32) as f64;
            let area = sin_t
                * (std::f64::consts::FRAC_PI_2 / rings as f64)
                * (std::f64::consts::PI / spokes as f64);
            total += 2.0 * d * v * ndl * 4.0 * vdh * area;
        }
    }
    total as f32
}

pub fn sheen_albedo(ndv: f32, roughness: f32) -> f32 {
    let last = (SHEEN_TABLE_SIDE - 1) as f32;
    let x = ndv.clamp(0.0, 1.0) * last;
    let y = roughness.clamp(0.0, 1.0) * last;
    let x0 = (x.floor() as usize).min(SHEEN_TABLE_SIDE - 2);
    let y0 = (y.floor() as usize).min(SHEEN_TABLE_SIDE - 2);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let at = |r: usize, c: usize| SHEEN_TABLE[r * SHEEN_TABLE_SIDE + c];
    let top = at(y0, x0) + (at(y0, x0 + 1) - at(y0, x0)) * fx;
    let bottom = at(y0 + 1, x0) + (at(y0 + 1, x0 + 1) - at(y0 + 1, x0)) * fx;
    top + (bottom - top) * fy
}

pub const SHEEN_TABLE_TEXT: &str = include_str!("wgsl/sheen_table.txt");

pub static SHEEN_TABLE: std::sync::LazyLock<[f32; SHEEN_TABLE_SIDE * SHEEN_TABLE_SIDE]> =
    std::sync::LazyLock::new(|| {
        let mut table = [0.0; SHEEN_TABLE_SIDE * SHEEN_TABLE_SIDE];
        for (slot, value) in table.iter_mut().zip(
            SHEEN_TABLE_TEXT
                .split(',')
                .map(str::trim)
                .filter(|text| !text.is_empty()),
        ) {
            *slot = value.parse().unwrap_or(0.0);
        }
        table
    });
