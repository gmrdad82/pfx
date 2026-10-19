use crate::color::{self, REC709};
use crate::hash::{self, grain_hash};
use crate::image::{self, Image};
use crate::tone::Tone;

#[derive(Clone, Copy, Debug)]
pub struct Saturation {
    pub amount: f32,
    pub weights: [f32; 3],
}

impl Saturation {
    pub fn rec709(amount: f32) -> Self {
        Self {
            amount,
            weights: REC709,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Contrast {
    Pivot { amount: f32, pivot: f32 },
    Power { power: f32, lift: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarmthKind {
    Linear,
    LumaCurve,
}

#[derive(Clone, Copy, Debug)]
pub struct Warmth {
    pub amount: f32,
    pub kind: WarmthKind,
    pub low: [f32; 2],
    pub high: [f32; 2],
    pub shadow: f32,
    pub highlight: f32,
}

impl Warmth {
    pub fn linear(amount: f32) -> Self {
        Self {
            amount,
            kind: WarmthKind::Linear,
            low: [1.0; 2],
            high: [1.0; 2],
            shadow: 0.0,
            highlight: 1.0,
        }
    }

    pub fn luma_curve(
        amount: f32,
        low: [f32; 2],
        high: [f32; 2],
        shadow: f32,
        highlight: f32,
    ) -> Self {
        Self {
            amount,
            kind: WarmthKind::LumaCurve,
            low,
            high,
            shadow,
            highlight,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutShape {
    Cube,
    Curves,
}

pub const NEUTRAL_DOMAIN: [[f32; 3]; 2] = [[0.0; 3], [1.0; 3]];

#[derive(Clone, Debug)]
pub struct Lut {
    pub size: u32,
    pub values: Vec<[f32; 3]>,
    pub shape: LutShape,
    pub amount: f32,
    pub domain: [[f32; 3]; 2],
}

impl Lut {
    pub fn cube(size: u32, values: Vec<[f32; 3]>) -> Self {
        Self {
            size,
            values,
            shape: LutShape::Cube,
            amount: 1.0,
            domain: NEUTRAL_DOMAIN,
        }
    }

    pub fn needs(&self) -> usize {
        let n = self.size as usize;
        match self.shape {
            LutShape::Cube => n.saturating_mul(n).saturating_mul(n),
            LutShape::Curves => n,
        }
    }

    fn remaps(&self) -> bool {
        self.domain != NEUTRAL_DOMAIN
    }

    pub fn params(&self) -> [f32; 16] {
        let [lo, hi] = self.domain;
        [
            self.size as f32,
            self.amount,
            if self.shape == LutShape::Curves {
                1.0
            } else {
                0.0
            },
            if self.remaps() { 1.0 } else { 0.0 },
            lo[0],
            lo[1],
            lo[2],
            0.0,
            hi[0],
            hi[1],
            hi[2],
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ]
    }

    pub fn sample(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        if n < 2 || self.values.len() < self.needs() {
            return c;
        }
        let t = if self.remaps() {
            let [lo, hi] = self.domain;
            std::array::from_fn(|k| (c[k] - lo[k]) / (hi[k] - lo[k]))
        } else {
            c
        };
        let looked = match self.shape {
            LutShape::Cube => self.cube_at(t),
            LutShape::Curves => self.curves_at(t),
        };
        if self.amount < 1.0 {
            color::lerp3(c, looked, self.amount)
        } else {
            looked
        }
    }

    fn curves_at(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let scale = (n - 1) as f32;
        std::array::from_fn(|k| {
            let x = c[k].clamp(0.0, 1.0) * scale;
            let i = (x.floor() as u32).min(n - 1);
            let j = (i + 1).min(n - 1);
            let a = self.values[i as usize][k];
            let b = self.values[j as usize][k];
            a + (b - a) * (x - i as f32)
        })
    }

    fn cube_at(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let scale = (n - 1) as f32;
        let coord = |v: f32| {
            let x = v.clamp(0.0, 1.0) * scale;
            let i = (x.floor() as u32).min(n - 1);
            let j = (i + 1).min(n - 1);
            (i, j, x - i as f32)
        };
        let (x0, x1, tx) = coord(c[0]);
        let (y0, y1, ty) = coord(c[1]);
        let (z0, z1, tz) = coord(c[2]);
        let at = |x, y, z| self.values[(x + n * (y + n * z)) as usize];
        let mix = |a: [f32; 3], b: [f32; 3], t: f32| color::lerp3(a, b, t);
        let c00 = mix(at(x0, y0, z0), at(x1, y0, z0), tx);
        let c01 = mix(at(x0, y0, z1), at(x1, y0, z1), tx);
        let c10 = mix(at(x0, y1, z0), at(x1, y1, z0), tx);
        let c11 = mix(at(x0, y1, z1), at(x1, y1, z1), tx);
        let c0 = mix(c00, c10, ty);
        let c1 = mix(c01, c11, ty);
        mix(c0, c1, tz)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VignetteKind {
    Power,
    Smoothstep,
}

#[derive(Clone, Copy, Debug)]
pub struct Vignette {
    pub strength: f32,
    pub kind: VignetteKind,
    pub power: f32,
    pub aspect: f32,
    pub scale: f32,
    pub inner: f32,
    pub outer: f32,
}

impl Vignette {
    pub fn power(strength: f32, power: f32, aspect: f32, scale: f32) -> Self {
        Self {
            strength,
            kind: VignetteKind::Power,
            power,
            aspect,
            scale,
            inner: 0.0,
            outer: 1.0,
        }
    }

    pub fn smoothstep(strength: f32, aspect: f32, inner: f32, outer: f32) -> Self {
        Self {
            strength,
            kind: VignetteKind::Smoothstep,
            power: 1.0,
            aspect,
            scale: 1.0,
            inner,
            outer,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Grain {
    pub strength: f32,
    pub response: f32,
    pub clamp: bool,
    pub size: f32,
    pub seed: u32,
}

impl Grain {
    pub const SMOOTH_GAIN: f32 = 35.0 / 26.0;

    pub fn new(strength: f32, response: f32, clamp: bool) -> Self {
        Self {
            strength,
            response,
            clamp,
            size: 1.0,
            seed: 0,
        }
    }

    pub fn clamped(strength: f32, response: f32) -> Self {
        Self::new(strength, response, true)
    }

    pub fn salt(&self) -> u32 {
        self.seed.wrapping_mul(0x9e37_79b9) >> 8
    }

    pub fn noise(&self, x: u32, y: u32, mixed: u32) -> f32 {
        let mixed = mixed ^ self.salt().wrapping_mul(0x632b_e5ab);
        if self.size <= 1.0 {
            return grain_hash(x, y, mixed ^ 7) + grain_hash(x, y, mixed ^ 19) - 1.0;
        }
        let u = (x as f32 + 0.5) / self.size;
        let v = (y as f32 + 0.5) / self.size;
        (hash::value_noise(u, v, mixed ^ 7) + hash::value_noise(u, v, mixed ^ 19) - 1.0)
            * Self::SMOOTH_GAIN
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Dither {
    pub amplitude: f32,
}

pub fn exposure(image: &Image, amount: f32) -> Image {
    if (amount - 1.0).abs() < 1e-8 {
        return image.clone();
    }
    image.map_rgb(|_, _, c| [c[0] * amount, c[1] * amount, c[2] * amount])
}

pub fn black(image: &Image, black: f32) -> Image {
    let denom = 1.0 - black;
    image.map_rgb(|_, _, c| {
        if denom.abs() < 1e-6 {
            [0.0; 3]
        } else {
            [
                ((c[0] - black) / denom).max(0.0),
                ((c[1] - black) / denom).max(0.0),
                ((c[2] - black) / denom).max(0.0),
            ]
        }
    })
}

pub fn saturation(image: &Image, pass: &Saturation) -> Image {
    image.map_rgb(|_, _, c| color::saturate(c, pass.amount, pass.weights))
}

pub fn contrast(image: &Image, pass: &Contrast) -> Image {
    image.map_rgb(|_, _, c| match *pass {
        Contrast::Pivot { amount, pivot } => [
            (c[0] - pivot) * amount + pivot,
            (c[1] - pivot) * amount + pivot,
            (c[2] - pivot) * amount + pivot,
        ],
        Contrast::Power { power, lift } => [
            (color::safe_pow(c[0].max(0.0), power) * lift).clamp(0.0, 1.0),
            (color::safe_pow(c[1].max(0.0), power) * lift).clamp(0.0, 1.0),
            (color::safe_pow(c[2].max(0.0), power) * lift).clamp(0.0, 1.0),
        ],
    })
}

pub fn warmth(image: &Image, pass: &Warmth) -> Image {
    image.map_rgb(|_, _, c| match pass.kind {
        WarmthKind::Linear => {
            let w = pass.amount;
            [c[0] * (1.0 + w), c[1], c[2] * (1.0 - w)]
        }
        WarmthKind::LumaCurve => {
            let l = color::luma(c, REC709).clamp(0.0, 1.0);
            let t = color::smoothstep(pass.shadow, pass.highlight, l);
            let warm = color::lerp3(
                [pass.low[0], 1.0, pass.low[1]],
                [pass.high[0], 1.0, pass.high[1]],
                t,
            );
            let scale = color::lerp3([1.0; 3], warm, pass.amount);
            [
                (c[0] * scale[0]).max(0.0),
                (c[1] * scale[1]).max(0.0),
                (c[2] * scale[2]).max(0.0),
            ]
        }
    })
}

pub fn lut(image: &Image, table: &Lut) -> Image {
    image.map_rgb(|_, _, c| table.sample(c))
}

pub fn tone(image: &Image, curve: &Tone) -> Image {
    image.map_rgb(|_, _, c| curve.map(c))
}

pub fn encode(image: &Image) -> Image {
    image.map_rgb(|_, _, c| {
        [
            color::encode_channel(c[0]),
            color::encode_channel(c[1]),
            color::encode_channel(c[2]),
        ]
    })
}

pub fn vignette(image: &Image, pass: &Vignette) -> Image {
    if pass.strength.abs() < 1e-8 {
        return image.clone();
    }
    let w = image.width.max(1) as f32;
    let h = image.height.max(1) as f32;
    image.map_rgb(|x, y, c| {
        let st_x = (x as f32 + 0.5) / w;
        let st_y = (y as f32 + 0.5) / h;
        let factor = match pass.kind {
            VignetteKind::Power => {
                let uv_x = st_x * 2.0 - 1.0;
                let uv_y = st_y * 2.0 - 1.0;
                let r = (uv_x * uv_x + (uv_y * pass.aspect).powi(2)).sqrt() / pass.scale.max(1e-4);
                1.0 - pass.strength * color::safe_pow(r.max(0.0), pass.power)
            }
            VignetteKind::Smoothstep => {
                let dx = st_x - 0.5;
                let dy = (st_y - 0.5) * pass.aspect;
                let v = (dx * dx + dy * dy).sqrt();
                1.0 - pass.strength * color::smoothstep(pass.inner, pass.outer, v)
            }
        };
        [c[0] * factor, c[1] * factor, c[2] * factor]
    })
}

pub fn grain(image: &Image, pass: &Grain, frame: u32, seed: u32) -> Image {
    grain_at(image, pass, frame, seed, [0, 0])
}

pub fn grain_at(image: &Image, pass: &Grain, frame: u32, seed: u32, origin: [u32; 2]) -> Image {
    if pass.strength.abs() < 1e-8 && !pass.clamp {
        return image.clone();
    }
    let mixed = hash::mixed(seed, frame);
    Image::from_fn_banded(image.width, image.height, |x, y| {
        let p = image.at(x, y);
        let c = [p[0], p[1], p[2]];
        let n = pass.noise(origin[0] + x, origin[1] + y, mixed);
        let l = color::luma(c, REC709);
        let add = n * pass.strength * (1.0 - pass.response * l);
        let out = [c[0] + add, c[1] + add, c[2] + add];
        if pass.clamp {
            [out[0].max(0.0), out[1].max(0.0), out[2].max(0.0), p[3]]
        } else {
            [out[0], out[1], out[2], p[3]]
        }
    })
}

pub fn dither(image: &Image, pass: &Dither, frame: u32, seed: u32) -> Image {
    if pass.amplitude.abs() < 1e-8 {
        return image.clone();
    }
    image.map_rgb(|x, y, c| {
        let h = hash::hash21(
            x as f32 + (seed % 1024) as f32,
            y as f32 + (frame % 1024) as f32,
        );
        let d = (h - 0.5) * pass.amplitude / 255.0;
        [c[0] + d, c[1] + d, c[2] + d]
    })
}

pub fn sample_bilinear(image: &Image, x: f32, y: f32) -> [f32; 4] {
    image.sample(x, y)
}

pub fn separable(image: &Image, radius: u32, spread: f32) -> Image {
    let radius = radius.min(64) as i32;
    let spread = if spread <= 1e-4 { 1.0 } else { spread };
    let horizontal = blur_axis(image, radius, spread, true);
    blur_axis(&horizontal, radius, spread, false)
}

fn kernel(radius: i32, spread: f32) -> Vec<f32> {
    let mut weights = Vec::with_capacity((radius * 2 + 1) as usize);
    let mut total = 0.0;
    for k in -radius..=radius {
        let w = (-(k * k) as f32 / spread).exp();
        weights.push(w);
        total += w;
    }
    if total > 0.0 {
        for w in &mut weights {
            *w /= total;
        }
    }
    weights
}

fn blur_axis(image: &Image, radius: i32, spread: f32, horizontal: bool) -> Image {
    let weights = kernel(radius, spread);
    let mut out = image.clone();
    for y in 0..image.height {
        for x in 0..image.width {
            let mut acc = [0.0; 4];
            for (i, k) in (-radius..=radius).enumerate() {
                let (sx, sy) = if horizontal {
                    (x as i32 + k, y as i32)
                } else {
                    (x as i32, y as i32 + k)
                };
                let p = image.get(sx, sy);
                let w = weights[i];
                for ch in 0..4 {
                    acc[ch] += p[ch] * w;
                }
            }
            out.pixels[image.index(x, y)] = acc;
        }
    }
    out
}

pub fn keep_rgb(rgb: [f32; 3], threshold: f32, knee: f32, knee_width: f32, soft: f32) -> [f32; 3] {
    let l = rgb[0].max(rgb[1]).max(rgb[2]);
    let hard = if l > threshold {
        (l - threshold).max(0.0) / l.max(1e-4)
    } else {
        0.0
    };
    let knee_t = if knee_width > 1e-6 {
        ((l - knee) / knee_width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let keep = hard + knee_t * knee_t * soft;
    [rgb[0] * keep, rgb[1] * keep, rgb[2] * keep]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Downsample {
    Box,
    Tent,
}

pub fn downsample(image: &Image, factor: u32, filter: Downsample) -> Image {
    let factor = factor.max(1);
    let w = (image.width / factor).max(1);
    let h = (image.height / factor).max(1);
    Image::from_fn(w, h, |x, y| {
        let c = if filter == Downsample::Tent {
            let mut acc = [0.0; 3];
            let base_x = x as f32 * factor as f32 + factor as f32 * 0.5 - 0.5;
            let base_y = y as f32 * factor as f32 + factor as f32 * 0.5 - 0.5;
            for dy in [-1.0, 1.0] {
                for dx in [-1.0, 1.0] {
                    let s = image.sample(base_x + dx, base_y + dy);
                    acc[0] += s[0];
                    acc[1] += s[1];
                    acc[2] += s[2];
                }
            }
            [acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25]
        } else {
            let base_x = x as i32 * factor as i32;
            let base_y = y as i32 * factor as i32;
            let mut acc = [0.0; 3];
            let mut n = 0.0;
            let span = factor.min(64) as i32;
            for dy in 0..span {
                for dx in 0..span {
                    let s = image.get(base_x + dx, base_y + dy);
                    acc[0] += s[0];
                    acc[1] += s[1];
                    acc[2] += s[2];
                    n += 1.0;
                }
            }
            [acc[0] / n, acc[1] / n, acc[2] / n]
        };
        image::px(c, 1.0)
    })
}
