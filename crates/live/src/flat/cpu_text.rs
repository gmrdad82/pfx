use pfx_text::{Atlas, MSDF_SPREAD, Paragraph};

use super::{Draw, Fit, Glyphs, Matrix, Srgba, multiply};
use crate::text::{PackedGlyph, pack_glyphs, pack_quads};

pub const ID_CUTOUT: f32 = 0.5;
pub const ANISOTROPY_FROM: f32 = 1.2;
pub const ANISOTROPY_FULL: f32 = 1.5;
pub const MOST_TAPS: u32 = 8;
const SUBPIXEL: f32 = 256.0;
const EDGE: f32 = 1e-4;
const MOST_LEVEL: f32 = 3.0;
const MOST_LEVELS: usize = 4;

#[derive(Clone, Copy)]
pub struct CpuText<'a> {
    pub atlas: &'a Atlas,
    pub glyphs: Glyphs<'a>,
    pub colour: Srgba,
}

impl<'a> CpuText<'a> {
    pub fn paragraph(paragraph: &'a Paragraph, colour: Srgba) -> Self {
        Self {
            atlas: &paragraph.atlas,
            glyphs: Glyphs::Paragraph(paragraph),
            colour,
        }
    }
}

type V2 = [f32; 2];

fn sub(a: V2, b: V2) -> V2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: V2, b: V2) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn length(a: V2) -> f32 {
    dot(a, a).sqrt()
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn median(rgb: [f32; 3]) -> f32 {
    rgb[0].min(rgb[1]).max(rgb[0].max(rgb[1]).min(rgb[2]))
}

struct Surface<'a> {
    atlas: &'a Atlas,
    msdf: bool,
    size: V2,
    levels: usize,
}

impl<'a> Surface<'a> {
    fn new(atlas: &'a Atlas) -> Option<Self> {
        let first = atlas.levels.first()?;
        if atlas.channels != 1 && atlas.channels != 3 {
            return None;
        }
        Some(Self {
            atlas,
            msdf: atlas.channels == 3,
            size: [first.width as f32, first.height as f32],
            levels: atlas.levels.len().min(MOST_LEVELS),
        })
    }

    fn texel(&self, level: usize, x: i64, y: i64) -> [f32; 3] {
        let mip = &self.atlas.levels[level];
        let x = x.clamp(0, i64::from(mip.width) - 1) as usize;
        let y = y.clamp(0, i64::from(mip.height) - 1) as usize;
        let channels = self.atlas.channels as usize;
        let at = (y * mip.width as usize + x) * channels;
        let byte = |i: usize| f32::from(mip.bytes.get(at + i).copied().unwrap_or(0)) / 255.0;
        if channels == 1 {
            [byte(0), 0.0, 0.0]
        } else {
            [byte(0), byte(1), byte(2)]
        }
    }

    fn bilinear(&self, level: usize, uv: V2) -> [f32; 3] {
        let mip = &self.atlas.levels[level];
        let x = uv[0] * mip.width as f32 - 0.5;
        let y = uv[1] * mip.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = self.texel(level, x0, y0);
        let b = self.texel(level, x0 + 1, y0);
        let c = self.texel(level, x0, y0 + 1);
        let d = self.texel(level, x0 + 1, y0 + 1);
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bottom = c[i] + (d[i] - c[i]) * tx;
            top + (bottom - top) * ty
        })
    }

    fn sample(&self, level: f32, uv: V2) -> [f32; 3] {
        let level = level.clamp(0.0, (self.levels - 1) as f32);
        let low = level.floor();
        let t = level - low;
        let low = low as usize;
        let first = self.bilinear(low, uv);
        if t == 0.0 {
            return first;
        }
        let second = self.bilinear((low + 1).min(self.levels - 1), uv);
        std::array::from_fn(|i| first[i] + (second[i] - first[i]) * t)
    }

    fn distance(&self, uv: V2) -> f32 {
        (median(self.sample(0.0, uv)) - 0.5) * MSDF_SPREAD
    }
}

struct Quad {
    glyph: PackedGlyph,
    origin: V2,
    inverse: [f32; 4],
    point: V2,
    along_a: V2,
    along_b: V2,
    pixels: [u32; 4],
}

fn rotate(angle: f32, v: V2) -> V2 {
    let (s, c) = angle.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1]]
}

fn place(glyph: &PackedGlyph, corner: V2) -> (V2, V2) {
    let local = [
        (glyph.rect[0] + corner[0] * glyph.rect[2]) * glyph.pose[3],
        (glyph.rect[1] + corner[1] * glyph.rect[3]) * glyph.pose[3],
    ];
    let spun = rotate(glyph.pose[2], local);
    let point = [glyph.pose[0] + spun[0], glyph.pose[1] + spun[1]];
    let pivot = [glyph.turn[0], glyph.turn[1]];
    let turned = rotate(glyph.turn[2], sub(point, pivot));
    (point, [pivot[0] + turned[0], pivot[1] + turned[1]])
}

fn snapped(p: V2) -> V2 {
    [
        (p[0] * SUBPIXEL).round() / SUBPIXEL,
        (p[1] * SUBPIXEL).round() / SUBPIXEL,
    ]
}

fn screen(matrix: &Matrix, at: V2, size: [u32; 2]) -> V2 {
    let x = matrix[0][0] * at[0] + matrix[1][0] * at[1] + matrix[3][0];
    let y = matrix[0][1] * at[0] + matrix[1][1] * at[1] + matrix[3][1];
    let w = matrix[0][3] * at[0] + matrix[1][3] * at[1] + matrix[3][3];
    let w = if w.abs() < 1e-12 { 1.0 } else { w };
    [
        (x / w * 0.5 + 0.5) * size[0] as f32,
        (0.5 - y / w * 0.5) * size[1] as f32,
    ]
}

impl Quad {
    fn new(glyph: PackedGlyph, matrix: &Matrix, size: [u32; 2]) -> Option<Self> {
        let (point, turned) = place(&glyph, [0.0, 0.0]);
        let (point_a, turned_a) = place(&glyph, [1.0, 0.0]);
        let (point_b, turned_b) = place(&glyph, [0.0, 1.0]);
        let (_, turned_c) = place(&glyph, [1.0, 1.0]);
        let origin = snapped(screen(matrix, turned, size));
        let a = sub(snapped(screen(matrix, turned_a, size)), origin);
        let b = sub(snapped(screen(matrix, turned_b, size)), origin);
        let corner = snapped(screen(matrix, turned_c, size));
        let det = a[0] * b[1] - b[0] * a[1];
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inverse = [b[1] / det, -b[0] / det, -a[1] / det, a[0] / det];
        let xs = [origin[0], origin[0] + a[0], origin[0] + b[0], corner[0]];
        let ys = [origin[1], origin[1] + a[1], origin[1] + b[1], corner[1]];
        let low = |v: [f32; 4]| v.into_iter().fold(f32::MAX, f32::min);
        let high = |v: [f32; 4]| v.into_iter().fold(f32::MIN, f32::max);
        let (x0, y0) = (low(xs).floor().max(0.0), low(ys).floor().max(0.0));
        let x1 = high(xs).ceil().min(size[0] as f32);
        let y1 = high(ys).ceil().min(size[1] as f32);
        if !(x1 > x0 && y1 > y0) {
            return None;
        }
        Some(Self {
            glyph,
            origin,
            inverse,
            point,
            along_a: sub(point_a, point),
            along_b: sub(point_b, point),
            pixels: [x0 as u32, y0 as u32, x1 as u32, y1 as u32],
        })
    }

    fn corner(&self, x: i64, y: i64) -> V2 {
        let q = [
            x as f32 + 0.5 - self.origin[0],
            y as f32 + 0.5 - self.origin[1],
        ];
        [
            self.inverse[0] * q[0] + self.inverse[1] * q[1],
            self.inverse[2] * q[0] + self.inverse[3] * q[1],
        ]
    }

    fn uv(&self, corner: V2) -> V2 {
        let uv = self.glyph.uv_rect;
        [
            uv[0] + (uv[2] - uv[0]) * corner[0],
            uv[1] + (uv[3] - uv[1]) * corner[1],
        ]
    }

    fn coverage(&self, surface: &Surface<'_>, x: u32, y: u32) -> Option<f32> {
        let (xi, yi) = (i64::from(x), i64::from(y));
        let corner = self.corner(xi, yi);
        if !(-EDGE..1.0 - EDGE).contains(&corner[0]) || !(-EDGE..1.0 - EDGE).contains(&corner[1]) {
            return None;
        }
        let uv = self.uv(corner);
        let bounds = self.glyph.uv_rect;
        let delta = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
        let dx = [delta[0] * self.inverse[0], delta[1] * self.inverse[2]];
        let dy = [delta[0] * self.inverse[1], delta[1] * self.inverse[3]];
        let size = surface.size;
        let scaled = |d: V2| [d[0] * size[0], d[1] * size[1]];
        let (jx, jy) = (scaled(dx), scaled(dy));
        let extent = [
            (bounds[2] - bounds[0]) * size[0],
            (bounds[3] - bounds[1]) * size[1],
        ];
        let level = if surface.msdf {
            0.0
        } else {
            let footprint = length(jx).max(length(jy));
            let safe = extent[0].min(extent[1]).max(1.0).log2().floor();
            footprint.max(1.0).log2().clamp(0.0, safe.min(MOST_LEVEL))
        };
        let half = [level.exp2() * 0.5 / size[0], level.exp2() * 0.5 / size[1]];
        let low = [
            (bounds[0] + half[0]).min(bounds[2]),
            (bounds[1] + half[1]).min(bounds[3]),
        ];
        let high = [
            (bounds[2] - half[0]).max(low[0]),
            (bounds[3] - half[1]).max(low[1]),
        ];
        let clamp = |uv: V2| [uv[0].clamp(low[0], high[0]), uv[1].clamp(low[1], high[1])];
        let alpha = if !surface.msdf {
            surface.sample(level, clamp(uv))[0]
        } else {
            let signed = |x: i64, y: i64| {
                let uv = self.uv(self.corner(x, y));
                median(surface.sample(0.0, clamp(uv))) - 0.5
            };
            let here = signed(xi, yi);
            let (qx, qy) = (xi & !1, yi & !1);
            let corner = signed(qx, qy);
            let across = signed(qx + 1, qy) - corner;
            let down = signed(qx, qy + 1) - corner;
            let fwidth = across.abs() + down.abs();
            let mut alpha = (0.5 + here / fwidth.max(1e-5)).clamp(0.0, 1.0);
            let a = dot(jx, jx);
            let b = dot(jx, jy);
            let c = dot(jy, jy);
            let area = (jx[0] * jy[1] - jx[1] * jy[0]).abs();
            let major_squared = 0.5 * (a + c) + (0.25 * (a - c) * (a - c) + b * b).sqrt();
            if major_squared > ANISOTROPY_FROM * area {
                let major = major_squared.sqrt();
                let minor = area / major.max(1e-6);
                let blend = smoothstep(ANISOTROPY_FROM, ANISOTROPY_FULL, major / minor.max(1e-6));
                let taps = (major / minor.max(1.0)).ceil().clamp(1.0, MOST_TAPS as f32) as u32;
                let axis = major_axis(a, b, c, major_squared);
                let filtered = anisotropic(surface, uv, [low, high], [jx, jy], axis, taps);
                alpha += (filtered - alpha) * blend;
            }
            alpha
        };
        let point = [
            self.point[0] + self.along_a[0] * corner[0] + self.along_b[0] * corner[1],
            self.point[1] + self.along_a[1] * corner[0] + self.along_b[1] * corner[1],
        ];
        let clip = self.glyph.clip;
        let inside = point[0] >= clip[0]
            && point[0] <= clip[2]
            && point[1] >= clip[1]
            && point[1] <= clip[3];
        Some(if inside { alpha } else { 0.0 })
    }
}

fn major_axis(a: f32, b: f32, c: f32, major: f32) -> V2 {
    let first = [b, major - a];
    let second = [major - c, b];
    let longer = if dot(first, first) >= dot(second, second) {
        first
    } else {
        second
    };
    let squared = dot(longer, longer);
    if squared <= 1e-12 {
        return [1.0, 0.0];
    }
    let n = squared.sqrt();
    [longer[0] / n, longer[1] / n]
}

fn anisotropic(
    surface: &Surface<'_>,
    uv: V2,
    clamp: [V2; 2],
    jacobian: [V2; 2],
    axis: V2,
    taps: u32,
) -> f32 {
    let [jx, jy] = jacobian;
    let along = [
        jx[0] * axis[0] + jy[0] * axis[1],
        jx[1] * axis[0] + jy[1] * axis[1],
    ];
    let across = [
        jx[0] * -axis[1] + jy[0] * axis[0],
        jx[1] * -axis[1] + jy[1] * axis[0],
    ];
    let texel = [1.0 / surface.size[0], 1.0 / surface.size[1]];
    let count = taps as f32;
    let mut total = 0.0;
    for tap in 0..taps {
        let offset = (tap as f32 + 0.5) / count - 0.5;
        let at = [
            (uv[0] + along[0] * offset * texel[0]).clamp(clamp[0][0], clamp[1][0]),
            (uv[1] + along[1] * offset * texel[1]).clamp(clamp[0][1], clamp[1][1]),
        ];
        let distance = surface.distance(at);
        let gradient = [
            surface.distance([at[0] + texel[0], at[1]]) - distance,
            surface.distance([at[0], at[1] + texel[1]]) - distance,
        ];
        let width = dot(gradient, along).abs() / count + dot(gradient, across).abs();
        total += (0.5 + distance / width.max(1e-4)).clamp(0.0, 1.0);
    }
    total / count
}

pub(crate) struct Label<'a> {
    surface: Surface<'a>,
    quads: Vec<Quad>,
}

impl<'a> Label<'a> {
    pub(crate) fn new(text: &CpuText<'a>, draw: &Draw, fit: &Fit, size: [u32; 2]) -> Option<Self> {
        let surface = Surface::new(text.atlas)?;
        let mut glyphs = match text.glyphs {
            Glyphs::Paragraph(paragraph) => pack_glyphs(&paragraph.glyphs),
            Glyphs::Quads(quads) => pack_quads(quads),
        };
        let tint = text.colour.0;
        for glyph in &mut glyphs {
            for (channel, value) in glyph.color.iter_mut().zip(tint).take(3) {
                *channel *= value;
            }
            glyph.color[3] *= tint[3] * draw.opacity.min(1.0);
        }
        let matrix = multiply(fit.clip_from_layout(), draw.transform);
        let quads = glyphs
            .into_iter()
            .filter_map(|glyph| Quad::new(glyph, &matrix, size))
            .collect::<Vec<_>>();
        Some(Self { surface, quads })
    }

    pub(crate) fn blend(&self, image: &mut [[f32; 4]], size: [u32; 2]) {
        for quad in &self.quads {
            let [x0, y0, x1, y1] = quad.pixels;
            for y in y0..y1 {
                for x in x0..x1 {
                    let Some(alpha) = quad.coverage(&self.surface, x, y) else {
                        continue;
                    };
                    let colour = quad.glyph.color;
                    let opacity = alpha * colour[3];
                    let src = [
                        colour[0] * opacity,
                        colour[1] * opacity,
                        colour[2] * opacity,
                        opacity,
                    ];
                    let dst = &mut image[(y * size[0] + x) as usize];
                    for i in 0..4 {
                        dst[i] = src[i] + dst[i] * (1.0 - src[3]);
                    }
                }
            }
        }
    }

    pub(crate) fn picked(&self, pixel: [u32; 2]) -> bool {
        self.quads.iter().any(|quad| {
            let [x0, y0, x1, y1] = quad.pixels;
            pixel[0] >= x0
                && pixel[0] < x1
                && pixel[1] >= y0
                && pixel[1] < y1
                && quad.glyph.color[3] > 0.0
                && quad
                    .coverage(&self.surface, pixel[0], pixel[1])
                    .is_some_and(|alpha| alpha >= ID_CUTOUT)
        })
    }
}
