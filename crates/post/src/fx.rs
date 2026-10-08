use crate::color::{self, REC709};
use crate::grade;
use crate::hash::{self, grain_hash};
use crate::image::{self, Image};

const PALETTE: [[f32; 3]; 16] = [
    [0.0, 0.0, 0.0],
    [0.012_286_5, 0.024_157_6, 0.086_500_5],
    [0.208_637, 0.018_500_2, 0.086_500_5],
    [0.0, 0.242_281, 0.082_282_7],
    [0.407_24, 0.084_376_2, 0.036_889_5],
    [0.114_435, 0.095_307_5, 0.078_187_4],
    [0.539_479, 0.545_724, 0.571_125],
    [1.0, 0.879_622, 0.806_952],
    [1.0, 0.0, 0.074_213_6],
    [1.0, 0.366_253, 0.0],
    [1.0, 0.838_799, 0.020_288_6],
    [0.0, 0.775_822, 0.036_889_5],
    [0.022_173_9, 0.417_885, 1.0],
    [0.226_966, 0.181_164, 0.332_452],
    [1.0, 0.184_475, 0.391_572],
    [1.0, 0.603_827, 0.401_978],
];

#[derive(Clone, Copy, Debug)]
pub struct Cel {
    pub bands: u32,
    pub shadow: f32,
    pub threshold: f32,
    pub softness: f32,
    pub spec: f32,
    pub spec_roughness: f32,
    pub spec_threshold: f32,
    pub light: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Outline {
    pub enabled: bool,
    pub thickness: f32,
    pub color: [f32; 3],
    pub local_color: bool,
    pub alpha: f32,
    pub crease_angle: f32,
    pub depth_gap: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Cavity {
    pub strength: f32,
    pub distance: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Rim {
    pub strength: f32,
    pub width: f32,
    pub color: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Bw {
    pub weights: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Noir {
    pub contrast: f32,
    pub crush: f32,
    pub keep: Option<[f32; 3]>,
    pub keep_range: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    Ordered,
    Blue,
}

#[derive(Clone, Copy, Debug)]
pub struct OneBit {
    pub kind: NoiseKind,
}

#[derive(Clone, Copy, Debug)]
pub struct Halftone {
    pub cell: f32,
    pub angle: f32,
    pub ink: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Comic {
    pub levels: u32,
    pub ink: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Watercolor {
    pub darkening: f32,
    pub grain: f32,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PaperGrain {
    pub strength: f32,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Pixelate {
    pub size: u32,
    pub levels: u32,
    pub palette: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Duotone {
    pub shadow: [f32; 3],
    pub highlight: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct GradientMap {
    pub stops: [[f32; 4]; 4],
    pub count: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Neon {
    pub saturation: f32,
    pub strength: f32,
    pub radius: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Scratches {
    pub count: u32,
    pub strength: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Halation {
    pub strength: f32,
    pub threshold: f32,
    pub radius: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Scanlines {
    pub strength: f32,
    pub period: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct TiltShift {
    pub focus: f32,
    pub range: f32,
    pub radius: f32,
}

pub fn cel(image: &Image, pass: &Cel, normal: Option<&[[f32; 3]]>) -> Image {
    let light_dir = color::normalize(pass.light);
    let bands = pass.bands.max(1);
    image.map_rgb(|x, y, c| {
        let l = color::luma(c, REC709).max(0.0);
        let (light, from_normal) = if let Some(normals) = normal {
            let n = color::normalize(normals[image.index(x, y)]);
            (color::dot3(n, light_dir).clamp(0.0, 1.0), true)
        } else {
            (l.clamp(0.0, 1.0), false)
        };
        let shade = band_shade(light, bands, pass.softness, pass.threshold, pass.shadow)
            .max(if from_normal { 0.55 } else { 0.18 });
        let gray = c[0] == c[1] && c[1] == c[2];
        let mut out = if from_normal {
            [c[0] * shade, c[1] * shade, c[2] * shade]
        } else if gray || l <= 1e-5 {
            [shade, shade, shade]
        } else {
            let s = shade / l;
            [c[0] * s, c[1] * s, c[2] * s]
        };
        if pass.spec > 0.0 {
            let rough = pass.spec_roughness.max(0.04);
            let denom = (1.0 - pass.spec_threshold).max(1e-3);
            let t = ((light - pass.spec_threshold) / denom).clamp(0.0, 1.0);
            let s = color::safe_pow(t, 1.0 / rough) * pass.spec;
            out[0] += s;
            out[1] += s;
            out[2] += s;
        }
        if from_normal && light < 0.55 {
            let cool = (0.55 - light) * 0.11;
            out[0] *= 1.0 - cool;
            out[1] += cool * 0.35;
            out[2] += cool;
        }
        out = color::saturate(out, 1.12, REC709);
        out
    })
}

fn band_shade(light: f32, bands: u32, softness: f32, threshold: f32, shadow: f32) -> f32 {
    let bands_f = bands.max(1) as f32;
    let light = light.clamp(0.0, 1.0);
    let x = (light * bands_f).min(bands_f - 1e-4);
    let frac = x.fract();
    let w = if softness <= 0.0 {
        0.0
    } else {
        color::smoothstep(0.5 - softness * 0.5, 0.5 + softness * 0.5, frac)
    };
    let idx = x.floor() + w;
    let q = if bands_f <= 1.0 {
        light
    } else {
        (idx / (bands_f - 1.0)).clamp(0.0, 1.0)
    };
    let gate = if softness <= 1e-5 {
        if light < threshold { 0.0 } else { 1.0 }
    } else {
        color::smoothstep(threshold - softness, threshold + softness, light)
    };
    q * (shadow + (1.0 - shadow) * gate)
}

pub fn outline(
    image: &Image,
    pass: &Outline,
    depth: Option<&[f32]>,
    normal: Option<&[[f32; 3]]>,
) -> Image {
    if !pass.enabled {
        return image.clone();
    }
    let radius = pass.thickness.clamp(1.0, 8.0);
    let rad = radius.ceil() as i32;
    let min_dot = pass.crease_angle.to_radians().cos();
    image.map_rgb(|x, y, c| {
        let mut edge: f32 = 0.0;
        let i = image.index(x, y);
        for dy in -rad..=rad {
            for dx in -rad..=rad {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let dist = ((dx * dx + dy * dy) as f32).sqrt();
                if dist > radius {
                    continue;
                }
                if pass.local_color {
                    let neighbor = image.get(x as i32 + dx, y as i32 + dy);
                    let gap = (c[0] - neighbor[0])
                        .abs()
                        .max((c[1] - neighbor[1]).abs())
                        .max((c[2] - neighbor[2]).abs());
                    edge = edge.max(color::smoothstep(0.08, 0.22, gap));
                }
                if let Some(depth) = depth {
                    let gap =
                        (depth[i] - depth_at(image, depth, x as i32 + dx, y as i32 + dy)).abs();
                    if gap > pass.depth_gap {
                        let t = ((gap - pass.depth_gap) / pass.depth_gap.max(1e-4)).clamp(0.0, 1.0);
                        edge = edge.max(t);
                    }
                }
                if let Some(normals) = normal {
                    let n = color::normalize(normals[i]);
                    let nn =
                        color::normalize(normal_at(image, normals, x as i32 + dx, y as i32 + dy));
                    let dots = color::dot3(n, nn).clamp(-1.0, 1.0);
                    if dots < min_dot {
                        let t = ((min_dot - dots) / (1.0 + min_dot).max(0.1)).clamp(0.0, 1.0);
                        edge = edge.max(t);
                    }
                }
            }
        }
        if depth.is_none() && normal.is_none() {
            edge = edge.max(color::sobel_luma(image, x as i32, y as i32).clamp(0.0, 1.0));
        }
        let a = (edge * pass.alpha).clamp(0.0, 1.0);
        let ink = if pass.local_color {
            let base = image.get(x as i32, y as i32);
            [
                (base[0] * 0.15).max(0.035),
                (base[1] * 0.15).max(0.035),
                (base[2] * 0.15).max(0.035),
            ]
        } else {
            pass.color
        };
        color::lerp3(c, ink, a)
    })
}

fn depth_at(image: &Image, depth: &[f32], x: i32, y: i32) -> f32 {
    if image.width == 0 || image.height == 0 {
        return 0.0;
    }
    let x = x.clamp(0, image.width as i32 - 1) as u32;
    let y = y.clamp(0, image.height as i32 - 1) as u32;
    depth[image.index(x, y)]
}

fn normal_at(image: &Image, normal: &[[f32; 3]], x: i32, y: i32) -> [f32; 3] {
    if image.width == 0 || image.height == 0 {
        return [0.0, 0.0, 1.0];
    }
    let x = x.clamp(0, image.width as i32 - 1) as u32;
    let y = y.clamp(0, image.height as i32 - 1) as u32;
    normal[image.index(x, y)]
}

pub fn cavity(image: &Image, pass: &Cavity, normal: Option<&[[f32; 3]]>) -> Image {
    let Some(normals) = normal else {
        return image.clone();
    };
    if pass.strength.abs() < 1e-8 {
        return image.clone();
    }
    let d = pass.distance.max(1.0).round() as i32;
    image.map_rgb(|x, y, c| {
        let n = color::normalize(normals[image.index(x, y)]);
        let mut bend = 0.0;
        for (dx, dy) in [(d, 0), (-d, 0), (0, d), (0, -d)] {
            let nn = color::normalize(normal_at(image, normals, x as i32 + dx, y as i32 + dy));
            bend += (1.0 - color::dot3(n, nn)).max(0.0);
        }
        let occ = (bend * 0.25).clamp(0.0, 1.0);
        let k = (1.0 - pass.strength * occ).max(0.0);
        [c[0] * k, c[1] * k, c[2] * k]
    })
}

pub fn rim(image: &Image, pass: &Rim, normal: Option<&[[f32; 3]]>) -> Image {
    let Some(normals) = normal else {
        return image.clone();
    };
    if pass.strength.abs() < 1e-8 {
        return image.clone();
    }
    let power = (1.0 / pass.width.max(0.05)).clamp(0.05, 32.0);
    image.map_rgb(|x, y, c| {
        let n = color::normalize(normals[image.index(x, y)]);
        let ndotv = n[2].abs().clamp(0.0, 1.0);
        let fres = color::safe_pow((1.0 - ndotv).clamp(0.0, 1.0), power);
        [
            c[0] + pass.color[0] * fres * pass.strength,
            c[1] + pass.color[1] * fres * pass.strength,
            c[2] + pass.color[2] * fres * pass.strength,
        ]
    })
}

pub fn bw(image: &Image, pass: &Bw) -> Image {
    image.map_rgb(|_, _, c| {
        let l = color::luma(c, pass.weights);
        [l, l, l]
    })
}

pub fn noir(image: &Image, pass: &Noir) -> Image {
    image.map_rgb(|_, _, c| {
        let l = color::luma(c, REC709);
        let gray = [l, l, l];
        let mut out = gray;
        if let Some(keep) = pass.keep {
            let kl = color::luma(keep, REC709);
            let kdir = [keep[0] - kl, keep[1] - kl, keep[2] - kl];
            let chroma = [c[0] - l, c[1] - l, c[2] - l];
            let kn = color::length3(kdir);
            let cn = color::length3(chroma);
            if kn > 1e-4 && cn > 1e-4 {
                let d = color::dot3(chroma, kdir) / (kn * cn);
                let gate = color::smoothstep(pass.keep_range, 1.0, d);
                out = color::lerp3(gray, c, gate);
            }
        }
        out = [
            (out[0] - 0.5) * pass.contrast + 0.5,
            (out[1] - 0.5) * pass.contrast + 0.5,
            (out[2] - 0.5) * pass.contrast + 0.5,
        ];
        if pass.crush < 0.999 {
            let denom = (1.0 - pass.crush).max(1e-4);
            out = [
                ((out[0] - pass.crush) / denom).max(0.0),
                ((out[1] - pass.crush) / denom).max(0.0),
                ((out[2] - pass.crush) / denom).max(0.0),
            ];
        }
        [out[0].max(0.0), out[1].max(0.0), out[2].max(0.0)]
    })
}

pub fn sepia(image: &Image, strength: f32) -> Image {
    image.map_rgb(|_, _, c| {
        let tint = [
            color::dot3(c, [0.393, 0.769, 0.189]),
            color::dot3(c, [0.349, 0.686, 0.168]),
            color::dot3(c, [0.272, 0.534, 0.131]),
        ];
        color::lerp3(c, tint, strength.clamp(0.0, 1.0))
    })
}

pub fn posterize(image: &Image, levels: f32) -> Image {
    let levels = levels.max(2.0);
    image.map_rgb(|_, _, c| c.map(|v| quantize_channel(v, levels)))
}

fn quantize_channel(v: f32, levels: f32) -> f32 {
    let t = v.clamp(0.0, 1.0);
    let q = (t * levels).floor().min(levels - 1.0);
    q / (levels - 1.0)
}

pub fn one_bit(image: &Image, pass: &OneBit) -> Image {
    image.map_rgb(|x, y, c| {
        let l = color::luma(c, REC709).clamp(0.0, 1.0);
        let t = match pass.kind {
            NoiseKind::Ordered => hash::bayer(x, y),
            NoiseKind::Blue => hash::blue_noise(x, y),
        };
        let v = if l >= t { 1.0 } else { 0.0 };
        [v, v, v]
    })
}

pub fn halftone(image: &Image, pass: &Halftone) -> Image {
    let rad = pass.angle.to_radians();
    let (s, c) = rad.sin_cos();
    let cell = pass.cell.max(1.0);
    image.map_rgb(|x, y, color| {
        let fx = x as f32 * c + y as f32 * s;
        let fy = -(x as f32) * s + y as f32 * c;
        let lx = fract(fx / cell) - 0.5;
        let ly = fract(fy / cell) - 0.5;
        let dist = (lx * lx + ly * ly).sqrt();
        let l = color::luma(color, REC709).clamp(0.0, 1.0);
        let radius = (1.0 - l).sqrt() * 0.5;
        if dist < radius { pass.ink } else { color }
    })
}

fn fract(v: f32) -> f32 {
    v - v.floor()
}

pub fn comic(
    image: &Image,
    pass: &Comic,
    depth: Option<&[f32]>,
    normal: Option<&[[f32; 3]]>,
) -> Image {
    let angle = 15.0_f32.to_radians();
    let (s, cs) = angle.sin_cos();
    let cell = pass.levels.max(2) as f32 + 2.0;
    image.map_rgb(|x, y, c| {
        let light = color::luma(c, REC709).clamp(0.0, 1.0);
        let peak = c[0].max(c[1]).max(c[2]);
        let base = if peak - c[0].min(c[1]).min(c[2]) < 0.12 {
            [0.97, 0.93, 0.78]
        } else if c[0] >= c[1] && c[0] >= c[2] {
            if c[1] > c[0] * 0.75 {
                [0.98, 0.77, 0.12]
            } else {
                [0.91, 0.12, 0.16]
            }
        } else if c[1] >= c[2] {
            [0.11, 0.68, 0.39]
        } else {
            [0.12, 0.38, 0.82]
        };
        let fx = x as f32 * cs + y as f32 * s;
        let fy = -(x as f32) * s + y as f32 * cs;
        let dx = fract(fx / cell) - 0.5;
        let dy = fract(fy / cell) - 0.5;
        let radius = (1.0 - light).sqrt() * 0.34;
        let dist = (dx * dx + dy * dy).sqrt();
        let dot = 1.0 - color::smoothstep(radius - 0.5 / cell, radius + 0.5 / cell, dist);
        let mut ink =
            color::smoothstep(0.12, 0.30, color::sobel_luma(image, x as i32, y as i32)) * 0.65;
        for (ox, oy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
            let n = image.get(x as i32 + ox, y as i32 + oy);
            let gap = (c[0] - n[0])
                .abs()
                .max((c[1] - n[1]).abs())
                .max((c[2] - n[2]).abs());
            ink = ink.max(color::smoothstep(0.08, 0.22, gap));
        }
        if let Some(d) = depth {
            let z = d[image.index(x, y)];
            for (ox, oy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
                ink = ink.max(color::smoothstep(
                    0.002,
                    0.012,
                    (z - depth_at(image, d, x as i32 + ox, y as i32 + oy)).abs(),
                ));
            }
        }
        if let Some(n) = normal {
            let a = color::normalize(n[image.index(x, y)]);
            for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let b = color::normalize(normal_at(image, n, x as i32 + ox, y as i32 + oy));
                ink = ink.max(color::smoothstep(0.3, 0.6, 1.0 - color::dot3(a, b)) * 0.55);
            }
        }
        let covered = ink.max(dot * 0.55) * pass.ink.clamp(0.0, 1.0);
        color::lerp3(base, [0.004, 0.003, 0.008], covered)
    })
}

pub fn rubber_hose(
    image: &Image,
    depth: Option<&[f32]>,
    normal: Option<&[[f32; 3]]>,
    frame: u32,
    seed: u32,
) -> Image {
    let mixed = hash::mixed(seed, frame);
    image.map_rgb(|x, y, c| {
        let xi = x as i32;
        let yi = y as i32;
        let wobble = hash::grain_hash(x / 32, y / 32, mixed);
        let shift = if wobble < 0.33 {
            -1
        } else if wobble > 0.66 {
            1
        } else {
            0
        };
        let radius = 5 + (hash::grain_hash(x / 48, y / 48, mixed ^ 0x12ab_9921) > 0.5) as i32;
        let mut edge = 0.0_f32;
        if let Some(d) = depth {
            let z = depth_at(image, d, xi + shift, yi);
            for (dx, dy) in [(radius, 0), (-radius, 0), (0, radius), (0, -radius)] {
                edge = edge.max(color::smoothstep(
                    0.002,
                    0.012,
                    (z - depth_at(image, d, xi + shift + dx, yi + dy)).abs(),
                ));
            }
        }
        if let Some(n) = normal {
            let a = color::normalize(normal_at(image, n, xi + shift, yi));
            for (dx, dy) in [(2, 0), (-2, 0), (0, 2), (0, -2)] {
                let b = color::normalize(normal_at(image, n, xi + shift + dx, yi + dy));
                edge = edge.max(color::smoothstep(0.3, 0.65, 1.0 - color::dot3(a, b)));
            }
        }
        edge =
            edge.max(color::smoothstep(0.12, 0.32, color::sobel_luma(image, xi + shift, yi)) * 0.7);
        let center = image.get(xi + shift, yi);
        for (dx, dy) in [(radius, 0), (-radius, 0), (0, radius), (0, -radius)] {
            let neighbor = image.get(xi + shift + dx, yi + dy);
            let gap = (center[0] - neighbor[0])
                .abs()
                .max((center[1] - neighbor[1]).abs())
                .max((center[2] - neighbor[2]).abs());
            edge = edge.max(color::smoothstep(0.08, 0.22, gap));
        }
        let l = color::luma(c, REC709).clamp(0.0, 1.0);
        let dark = depth.is_none_or(|values| values[image.index(x, y)] < 0.999)
            && (normal.is_some_and(|normals| {
                let n = color::normalize(normals[image.index(x, y)]);
                color::dot3(n, [0.0, 0.0, 1.0]) > 0.1
                    && color::dot3(n, color::normalize([-0.45, 0.65, 0.61])) < 0.1
            }) || l < 0.18);
        let fill = if l > 0.56 { 0.91 } else { 0.77 };
        let grain = (hash::grain_hash(x, y, mixed) - 0.5) * 0.07;
        let flicker = (hash::grain_hash(0, 0, mixed ^ 0x55aa_10ef) - 0.5) * 0.08;
        let u = (x as f32 / image.width.max(1) as f32 - 0.5) * 2.0;
        let v = (y as f32 / image.height.max(1) as f32 - 0.5) * 2.0;
        let vignette = 1.0 - 0.25 * (u * u + v * v).min(1.0);
        let dust = hash::grain_hash(x / 3, y / 3, mixed ^ 0x2983_7781) < 0.0008;
        let light = ((fill + grain + flicker) * vignette).clamp(0.0, 1.0);
        let ink = if edge > 0.5 || dust || dark {
            0.003
        } else {
            light
        };
        [ink * 1.0, ink * 0.91, ink * 0.73]
    })
}

pub fn watercolor(image: &Image, pass: &Watercolor, frame: u32, seed: u32) -> Image {
    let scale = pass.scale.max(1.0);
    let mixed = hash::mixed(seed, frame) ^ 0xA9E1_0006;
    image.map_rgb(|x, y, c| {
        let e = color::sobel_luma(image, x as i32, y as i32).clamp(0.0, 1.0);
        let dark = (1.0 - pass.darkening * e).max(0.0);
        let n = hash::value_noise(x as f32 / scale, y as f32 / scale, mixed);
        let grain = 1.0 + (n - 0.5) * pass.grain;
        [
            (c[0] * dark * grain).max(0.0),
            (c[1] * dark * grain).max(0.0),
            (c[2] * dark * grain).max(0.0),
        ]
    })
}

pub fn paper_grain(image: &Image, pass: &PaperGrain, frame: u32, seed: u32) -> Image {
    let scale = pass.scale.max(1.0);
    let mixed = hash::mixed(seed, frame) ^ 0xA9E1_0006;
    image.map_rgb(|x, y, c| {
        let n = hash::value_noise(x as f32 / scale, y as f32 / scale, mixed);
        let grain = 1.0 + (n - 0.5) * pass.strength;
        [
            (c[0] * grain).max(0.0),
            (c[1] * grain).max(0.0),
            (c[2] * grain).max(0.0),
        ]
    })
}

pub fn pixelate(image: &Image, pass: &Pixelate) -> Image {
    let size = pass.size.max(1);
    let mut out = image.clone();
    let mut y = 0;
    while y < image.height {
        let y1 = (y + size).min(image.height);
        let mut x = 0;
        while x < image.width {
            let x1 = (x + size).min(image.width);
            let mut acc = [0.0; 3];
            let mut n = 0.0;
            for py in y..y1 {
                for px in x..x1 {
                    let p = image.at(px, py);
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                    n += 1.0;
                }
            }
            let mut mean = [acc[0] / n, acc[1] / n, acc[2] / n];
            if pass.palette {
                mean = nearest(mean);
            } else if pass.levels >= 2 {
                let levels = pass.levels as f32;
                mean = mean.map(|v| quantize_channel(v, levels));
            }
            for py in y..y1 {
                for px in x..x1 {
                    let p = image.at(px, py);
                    out.pixels[image.index(px, py)] = image::px(mean, p[3]);
                }
            }
            x = x1;
        }
        y = y1;
    }
    out
}

fn nearest(c: [f32; 3]) -> [f32; 3] {
    let mut best = PALETTE[0];
    let mut best_d = f32::MAX;
    for p in PALETTE {
        let d = (c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2) + (c[2] - p[2]).powi(2);
        if d < best_d {
            best_d = d;
            best = p;
        }
    }
    best
}

pub fn duotone(image: &Image, pass: &Duotone) -> Image {
    image.map_rgb(|_, _, c| {
        let l = color::luma(c, REC709).clamp(0.0, 1.0);
        color::lerp3(pass.shadow, pass.highlight, l)
    })
}

pub fn gradient_map(image: &Image, pass: &GradientMap) -> Image {
    let count = pass.count.clamp(2, 4) as usize;
    image.map_rgb(|_, _, c| {
        let l = color::luma(c, REC709).clamp(0.0, 1.0);
        let mut lower = pass.stops[0];
        let mut upper = pass.stops[count - 1];
        for stop in pass.stops.iter().take(count) {
            if stop[3] <= l {
                lower = *stop;
            }
        }
        for stop in pass.stops.iter().take(count) {
            if stop[3] >= l {
                upper = *stop;
                break;
            }
        }
        let span = upper[3] - lower[3];
        let t = if span.abs() < 1e-6 {
            0.0
        } else {
            ((l - lower[3]) / span).clamp(0.0, 1.0)
        };
        color::lerp3(
            [lower[0], lower[1], lower[2]],
            [upper[0], upper[1], upper[2]],
            t,
        )
    })
}

pub fn kuwahara(image: &Image, radius: u32) -> Image {
    let r = radius.min(8) as i32;
    if r <= 0 {
        return image.clone();
    }
    let quads = [(-r, 0, -r, 0), (0, r, -r, 0), (-r, 0, 0, r), (0, r, 0, r)];
    image.map_rgb(|x, y, _| {
        let mut best_mean = [0.0; 3];
        let mut best_var = f32::MAX;
        for (x0, x1, y0, y1) in quads {
            let mut sum = [0.0; 3];
            let mut sq = 0.0;
            let mut n = 0.0;
            for dy in y0..=y1 {
                for dx in x0..=x1 {
                    let p = image.get(x as i32 + dx, y as i32 + dy);
                    sum[0] += p[0];
                    sum[1] += p[1];
                    sum[2] += p[2];
                    let l = color::luma(image::rgb(p), REC709);
                    sq += l * l;
                    n += 1.0;
                }
            }
            let inv = 1.0 / n;
            let mean = [sum[0] * inv, sum[1] * inv, sum[2] * inv];
            let ml = color::luma(mean, REC709);
            let var = (sq * inv - ml * ml).max(0.0);
            if var < best_var {
                best_var = var;
                best_mean = mean;
            }
        }
        best_mean
    })
}

pub fn neon(image: &Image, pass: &Neon) -> Image {
    if pass.radius == 0 || pass.strength.abs() < 1e-8 {
        return image.map_rgb(|_, _, c| color::saturate(c, pass.saturation, REC709));
    }
    let width = (image.width / 2).max(1);
    let height = (image.height / 2).max(1);
    let edge = Image::from_fn(width, height, |x, y| {
        let sx = (x * 2 + 1) as i32;
        let sy = (y * 2 + 1) as i32;
        let e = color::sobel_luma(image, sx, sy).clamp(0.0, 4.0);
        let c = color::saturate(image::rgb(image.get(sx, sy)), pass.saturation, REC709);
        [c[0] * e, c[1] * e, c[2] * e, 1.0]
    });
    let blurred = grade::separable(
        &edge,
        pass.radius.min(8).div_ceil(2),
        pass.radius.max(1) as f32 * 1.5,
    );
    let saturated = image.map_rgb(|_, _, c| color::saturate(c, pass.saturation, REC709));
    crate::bloom::add(&saturated, &blurred, pass.strength)
}

pub fn flicker(image: &Image, amount: f32, frame: u32, seed: u32) -> Image {
    if amount.abs() < 1e-8 {
        return image.clone();
    }
    let h = grain_hash(frame, 0, seed ^ 0xF11C_E001);
    let scale = 1.0 + (h - 0.5) * amount;
    image.map_rgb(|_, _, c| [c[0] * scale, c[1] * scale, c[2] * scale])
}

pub fn weave(image: &Image, amount: f32, frame: u32, seed: u32) -> Image {
    if amount.abs() < 1e-8 {
        return image.clone();
    }
    let hx = grain_hash(frame, 1, seed ^ 0x11A0_0001);
    let hy = grain_hash(frame, 2, seed ^ 0x22B0_0002);
    let dx = (hx - 0.5) * 2.0 * amount;
    let dy = (hy - 0.5) * 4.0 * amount;
    Image::from_fn(image.width, image.height, |x, y| {
        image.sample(x as f32 - dx, y as f32 - dy)
    })
}

pub fn dust(image: &Image, amount: f32, frame: u32, seed: u32) -> Image {
    if amount <= 0.0 {
        return image.clone();
    }
    let mixed = hash::mixed(seed, frame) ^ 0xD057_0003;
    image.map_rgb(|x, y, c| {
        let h = grain_hash(x, y, mixed);
        if h < amount {
            let a = 0.55 + 0.45 * grain_hash(x, y, mixed ^ 0x51);
            [c[0] + a, c[1] + a, c[2] + a]
        } else {
            c
        }
    })
}

pub fn scratches(image: &Image, pass: &Scratches, frame: u32, seed: u32) -> Image {
    if pass.strength.abs() < 1e-8 || pass.count == 0 || image.width == 0 {
        return image.clone();
    }
    let width = image.width as f32;
    image.map_rgb(|x, y, c| {
        let mut out = c;
        for i in 0..pass.count {
            let x0 = grain_hash(i, frame, seed ^ 0x5C8A_0005) * width;
            let wobble = (grain_hash(i, y / 8, seed ^ 0x5C8A_0004) - 0.5) * 3.0;
            let dist = (x as f32 - x0 - wobble).abs();
            if dist < 0.8 {
                let gap = grain_hash(i.wrapping_mul(97).wrapping_add(x), y, seed ^ frame);
                if gap > 0.4 {
                    let s = pass.strength * (1.0 - dist / 0.8);
                    out[0] += s;
                    out[1] += s;
                    out[2] += s;
                }
            }
        }
        out
    })
}

pub fn halation(image: &Image, pass: &Halation) -> Image {
    if pass.strength.abs() < 1e-8 {
        return image.clone();
    }
    let small = grade::downsample(image, 4, grade::Downsample::Tent);
    let hot = small.map_rgb(|_, _, c| {
        let l = color::luma(c, REC709);
        let h = (l - pass.threshold).max(0.0);
        [h, h * 0.25, h * 0.05]
    });
    let blurred = grade::separable(
        &hot,
        pass.radius.div_ceil(4),
        (pass.radius.max(1) as f32) * 0.25,
    );
    crate::bloom::add(image, &blurred, pass.strength)
}

pub fn aberration(image: &Image, amount: f32) -> Image {
    if amount.abs() < 1e-6 {
        return image.clone();
    }
    let cx = (image.width.saturating_sub(1)) as f32 * 0.5;
    let cy = (image.height.saturating_sub(1)) as f32 * 0.5;
    Image::from_fn(image.width, image.height, |x, y| {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = if len < 1e-4 {
            (0.0, 0.0)
        } else {
            (dx / len, dy / len)
        };
        let r = image.sample(x as f32 + ux * amount, y as f32 + uy * amount);
        let g = image.at(x, y);
        let b = image.sample(x as f32 - ux * amount, y as f32 - uy * amount);
        [r[0], g[1], b[2], g[3]]
    })
}

pub fn distort(image: &Image, amount: f32) -> Image {
    if amount.abs() < 1e-8 {
        return image.clone();
    }
    remap(image, amount)
}

pub fn curvature(image: &Image, amount: f32) -> Image {
    distort(image, amount)
}

fn remap(image: &Image, amount: f32) -> Image {
    let w = image.width.max(1) as f32;
    let h = image.height.max(1) as f32;
    Image::from_fn(image.width, image.height, |x, y| {
        let nx = (x as f32 + 0.5) / w * 2.0 - 1.0;
        let ny = (y as f32 + 0.5) / h * 2.0 - 1.0;
        let r2 = nx * nx + ny * ny;
        let f = 1.0 + amount * r2;
        let sx = (nx * f * 0.5 + 0.5) * w - 0.5;
        let sy = (ny * f * 0.5 + 0.5) * h - 0.5;
        image.sample(sx, sy)
    })
}

pub fn scanlines(image: &Image, pass: &Scanlines) -> Image {
    if pass.strength.abs() < 1e-8 {
        return image.clone();
    }
    let period = pass.period.max(2);
    image.map_rgb(|_, y, c| {
        if y % period == period - 1 {
            let k = (1.0 - pass.strength).max(0.0);
            [c[0] * k, c[1] * k, c[2] * k]
        } else {
            c
        }
    })
}

pub fn aperture(image: &Image, strength: f32) -> Image {
    if strength.abs() < 1e-8 {
        return image.clone();
    }
    let s = strength.clamp(0.0, 1.0);
    image.map_rgb(|x, _, c| match x % 3 {
        0 => [c[0], c[1] * (1.0 - s), c[2] * (1.0 - s)],
        1 => [c[0] * (1.0 - s), c[1], c[2] * (1.0 - s)],
        _ => [c[0] * (1.0 - s), c[1] * (1.0 - s), c[2]],
    })
}

pub fn tilt_shift(image: &Image, pass: &TiltShift, depth: Option<&[f32]>) -> Image {
    let Some(depth) = depth else {
        return image.clone();
    };
    if pass.radius < 0.35 || pass.range.abs() < 1e-6 {
        return image.clone();
    }
    let taps = 8u32;
    Image::from_fn(image.width, image.height, |x, y| {
        let z = depth[image.index(x, y)];
        let coc = ((z - pass.focus).abs() / pass.range).clamp(0.0, 1.0) * pass.radius;
        if coc < 0.35 {
            return image.at(x, y);
        }
        let center = image.at(x, y);
        let mut acc = center;
        let mut weight = 1.0;
        for k in 0..taps {
            let a = k as f32 * std::f32::consts::TAU / taps as f32;
            let d = coc * ((k as f32 + 0.5) / taps as f32).sqrt();
            let s = image.sample(x as f32 + a.cos() * d, y as f32 + a.sin() * d);
            for ch in 0..4 {
                acc[ch] += s[ch];
            }
            weight += 1.0;
        }
        [
            acc[0] / weight,
            acc[1] / weight,
            acc[2] / weight,
            acc[3] / weight,
        ]
    })
}
