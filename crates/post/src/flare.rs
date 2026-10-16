use crate::grade;
use crate::hash::grain_hash;
use crate::image::{self, Image};

pub const ROWS: u32 = 192;
pub const BLOOM_THRESHOLD: f32 = 1.0;
pub const HIGHLIGHT: f32 = 1000.0;
pub const ABERRATION: f32 = 0.005;
pub const BLUR_RADIUS: u32 = 4;
pub const BLUR_SPREAD: f32 = 2.0 * (10.0 / 6.0) * (10.0 / 6.0);
pub const STARBURST_TEXELS: u32 = 256;
pub const STARBURST_SEED: u32 = 0x2545_f491;
pub const MAX_GHOSTS: u32 = 16;
pub const KNEE: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flare {
    pub amount: f32,
    pub ghosts: u32,
    pub ghost_spacing: f32,
    pub halo: f32,
    pub halo_radius: f32,
    pub starburst: f32,
    pub threshold: f32,
}

impl Default for Flare {
    fn default() -> Self {
        Self {
            amount: 0.1,
            ghosts: 4,
            ghost_spacing: 0.6,
            halo: 0.1,
            halo_radius: 0.4,
            starburst: 1.0,
            threshold: 10.0,
        }
    }
}

impl Flare {
    pub fn is_off(&self) -> bool {
        self.amount.abs() < 1e-8 || (self.ghosts == 0 && self.halo <= 0.0)
    }
}

pub fn factor(height: u32) -> u32 {
    (height / ROWS).max(1)
}

fn max3(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2])
}

pub fn prepare(image: &Image, factor: u32) -> Image {
    let factor = factor.max(1);
    let w = (image.width / factor).max(1);
    let h = (image.height / factor).max(1);
    let span = factor.min(64) as i32;
    Image::from_fn(w, h, |x, y| {
        let base_x = x as i32 * factor as i32;
        let base_y = y as i32 * factor as i32;
        let mut acc = [0.0f32; 3];
        let mut total = 0.0f32;
        for dy in (0..span).step_by(2) {
            for dx in (0..span).step_by(2) {
                let (x0, y0) = (base_x + dx, base_y + dy);
                let quad = [
                    image.get(x0, y0),
                    image.get(x0 + 1, y0),
                    image.get(x0, y0 + 1),
                    image.get(x0 + 1, y0 + 1),
                ];
                let c: [f32; 3] = std::array::from_fn(|ch| {
                    (quad.iter().map(|texel| texel[ch]).sum::<f32>() * 0.25).max(0.0)
                });
                let weight = 1.0 / (1.0 + max3(c));
                for ch in 0..3 {
                    acc[ch] += c[ch] * weight;
                }
                total += weight;
            }
        }
        let mut c = acc.map(|v| (v / total.max(1e-8) - BLOOM_THRESHOLD).max(0.0));
        let crush = 1.0 / (1.0 + max3(c) * (1.0 / HIGHLIGHT));
        for v in &mut c {
            *v *= crush;
        }
        image::px(c, 1.0)
    })
}

fn sample_uv(image: &Image, u: f32, v: f32) -> [f32; 4] {
    image.sample(u * image.width as f32 - 0.5, v * image.height as f32 - 0.5)
}

fn normalize(v: [f32; 2]) -> [f32; 2] {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if len < 1e-12 {
        return [0.0; 2];
    }
    [v[0] / len, v[1] / len]
}

fn sample_color(image: &Image, uv: [f32; 2]) -> [f32; 3] {
    let dir = normalize([0.5001 - uv[0], 0.5001 - uv[1]]);
    let offset = [dir[0] * ABERRATION, dir[1] * ABERRATION];
    [
        sample_uv(image, uv[0] + offset[0], uv[1] + offset[1])[0],
        sample_uv(image, uv[0], uv[1])[1],
        sample_uv(image, uv[0] - offset[0], uv[1] - offset[1])[2],
    ]
}

fn smoothstep01(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn ring(x: f32, c: f32, r: f32) -> f32 {
    1.0 - smoothstep01(((x - c).abs() / r).min(1.0))
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

pub fn knee(s: [f32; 3], threshold: f32) -> [f32; 3] {
    let peak = max3(s);
    if peak <= 0.0 {
        return [0.0; 3];
    }
    let over = ((peak - threshold) / (threshold.max(1e-4) * KNEE)).clamp(0.0, 1.0);
    let gain = over * over / peak;
    s.map(|c| c.max(0.0) * gain)
}

fn ghosts(image: &Image, flare: &Flare, vertex: [f32; 2]) -> [f32; 3] {
    let uv = [1.0 - vertex[0], 1.0 - vertex[1]];
    let v = [
        (0.5 - uv[0]) * flare.ghost_spacing,
        (0.5 - uv[1]) * flare.ghost_spacing,
    ];
    let mut color = [0.0f32; 3];
    for i in 0..flare.ghosts.min(MAX_GHOSTS) {
        let i = i as f32;
        let suv = [fract(uv[0] + v[0] * i), fract(uv[1] + v[1] * i)];
        let s = knee(sample_color(image, suv), flare.threshold);
        for ch in 0..3 {
            color[ch] += s[ch];
        }
    }
    color
}

fn halo(image: &Image, flare: &Flare, uv: [f32; 2], aspect: [f32; 2]) -> [f32; 3] {
    let mut v = [0.5001 - uv[0], 0.5001 - uv[1]];
    v[0] *= aspect[0];
    v = normalize(v);
    v[0] *= aspect[1];
    let s = sample_color(
        image,
        [
            uv[0] + v[0] * flare.halo_radius,
            uv[1] + v[1] * flare.halo_radius,
        ],
    );
    let wx = (uv[0] - 0.5) * aspect[0] + 0.5;
    let wy = uv[1];
    let d = ((wx - 0.5) * (wx - 0.5) + (wy - 0.5) * (wy - 0.5)).sqrt();
    let weight = ring(d, flare.halo_radius, flare.halo);
    knee(s, flare.threshold).map(|c| c * weight)
}

pub fn features(prepared: &Image, flare: &Flare) -> Image {
    let w = prepared.width.max(1) as f32;
    let h = prepared.height.max(1) as f32;
    let ratio = w / h;
    let aspect = [ratio, 1.0 / ratio];
    Image::from_fn(prepared.width, prepared.height, |x, y| {
        let uv = [(x as f32 + 0.5) / w, (y as f32 + 0.5) / h];
        let mut color = [0.0f32; 3];
        if flare.ghosts > 0 {
            let g = ghosts(prepared, flare, uv);
            for ch in 0..3 {
                color[ch] += g[ch];
            }
        }
        if flare.halo > 0.0 {
            let s = halo(prepared, flare, uv, aspect);
            for ch in 0..3 {
                color[ch] += s[ch];
            }
        }
        image::px(color, 1.0)
    })
}

pub fn starburst_texel(index: u32) -> f32 {
    let r = 0.5 + 0.5 * grain_hash(index % STARBURST_TEXELS, 0, STARBURST_SEED);
    (r * 255.0).floor() / 255.0
}

fn starburst_lookup(u: f32) -> f32 {
    let t = fract(u) * STARBURST_TEXELS as f32 - 0.5;
    let i0 = t.floor();
    let f = t - i0;
    let n = STARBURST_TEXELS as i32;
    let a = (i0 as i32).rem_euclid(n) as u32;
    let b = (i0 as i32 + 1).rem_euclid(n) as u32;
    let ta = starburst_texel(a);
    ta + (starburst_texel(b) - ta) * f
}

fn acos_fast_positive(x: f32) -> f32 {
    let p = -0.156_582_7 * x + std::f32::consts::FRAC_PI_2;
    p * (1.0 - x).max(0.0).sqrt()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn starburst(u: f32, v: f32) -> f32 {
    let cx = u - 0.5001;
    let cy = v - 0.5001;
    let d = (cx * cx + cy * cy).sqrt();
    let radial = acos_fast_positive(cx / d.max(1e-12));
    let mask = starburst_lookup(radial) * starburst_lookup(radial);
    (mask + (1.0 - smoothstep(0.0, 0.3, d))).clamp(0.0, 1.0)
}

pub fn apply(image: &Image, flare: &Flare) -> Image {
    if flare.is_off() {
        return image.clone();
    }
    let prepared = prepare(image, factor(image.height));
    let found = features(&prepared, flare);
    let blurred = grade::separable(&found, BLUR_RADIUS, BLUR_SPREAD);
    let bw = blurred.width as f32;
    let bh = blurred.height as f32;
    let w = image.width.max(1) as f32;
    let h = image.height.max(1) as f32;
    let mut out = image.clone();
    for y in 0..image.height {
        for x in 0..image.width {
            let u = (x as f32 + 0.5) / w;
            let v = (y as f32 + 0.5) / h;
            let s = blurred.sample(u * bw - 0.5, v * bh - 0.5);
            let mask = 1.0 + (starburst(u, v) - 1.0) * flare.starburst;
            let gain = flare.amount * mask;
            let i = image.index(x, y);
            let p = out.pixels[i];
            out.pixels[i] = [
                p[0] + s[0] * gain,
                p[1] + s[1] * gain,
                p[2] + s[2] * gain,
                p[3],
            ];
        }
    }
    out
}
