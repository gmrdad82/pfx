use pfx_materials::{Blend, ContentLayer, Material};
pub use pfx_text::MSDF_SPREAD;
use pfx_text::{Atlas, AtlasPage, RichGlyphQuad, RichParagraph, msdf_coverage};

use crate::detail::{ContentFace, ContentText, Transform};
use crate::stage::{ContentKind, Image, Mesh, PlacementContent};

pub const MAX_TEXELS: u64 = 1 << 26;
pub const MAX_RECORDS: usize = 1 << 24;
pub const MAX_CELLS: usize = 1 << 18;
pub const NOMINAL_LIMIT: f32 = 16384.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    pub turn: [f32; 3],
}

impl From<&RichGlyphQuad> for Glyph {
    fn from(quad: &RichGlyphQuad) -> Self {
        Self {
            rect: quad.rect,
            uv: quad.uv,
            color: quad.color,
            clip: quad.clip,
            turn: quad.turn,
        }
    }
}

pub fn glyphs(paragraph: &RichParagraph) -> Vec<Glyph> {
    paragraph
        .quads
        .iter()
        .filter(|quad| quad.atlas_page == AtlasPage::Monochrome)
        .map(Glyph::from)
        .collect()
}

pub fn material(lit: bool) -> Material {
    let layer = if lit {
        ContentLayer::default()
    } else {
        ContentLayer {
            blend: Blend::Emit,
            strength: 1.0,
            ..ContentLayer::default()
        }
    };
    Material {
        base: [0.0; 3],
        roughness: 1.0,
        specular: 0.0,
        content_layer: layer,
        ..Material::default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Print {
    pub width: u32,
    pub height: u32,
    pub density: f32,
    pub rect: [f32; 4],
    pub texels: Vec<[u8; 4]>,
}

pub trait Laid {
    fn rect(&self) -> [f32; 4];
    fn kind(&self) -> ContentKind<'_>;
}

impl Laid for Print {
    fn rect(&self) -> [f32; 4] {
        self.rect
    }

    fn kind(&self) -> ContentKind<'_> {
        ContentKind::Image(self.image())
    }
}

impl Print {
    pub fn image(&self) -> Image<'_> {
        Image {
            width: self.width,
            height: self.height,
            texels: &self.texels,
            srgb: true,
        }
    }

    pub fn content(&self) -> PlacementContent<'_> {
        PlacementContent {
            cutout: true,
            ..PlacementContent::new(self.image())
        }
    }

    pub fn carrier(&self) -> Carrier {
        Carrier::over(self.rect)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Carrier {
    pub positions: [[f32; 3]; 4],
    pub normals: [[f32; 3]; 4],
    pub tangents: [[f32; 4]; 4],
    pub uvs: [[f32; 2]; 4],
    pub indices: [u32; 6],
}

impl Carrier {
    pub fn over(rect: [f32; 4]) -> Self {
        let [x, y, w, h] = rect;
        Self {
            positions: [
                [x, y, 0.0],
                [x + w, y, 0.0],
                [x + w, y + h, 0.0],
                [x, y + h, 0.0],
            ],
            normals: [[0.0, 0.0, 1.0]; 4],
            tangents: [[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: [0, 1, 2, 0, 2, 3],
        }
    }

    pub fn mesh(&self) -> Mesh<'_> {
        Mesh {
            positions: &self.positions,
            normals: &self.normals,
            tangents: &self.tangents,
            uvs: &self.uvs,
            alpha: None,
            indices: &self.indices,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Grid {
    scale: [f32; 2],
    offset: [f32; 2],
}

impl Grid {
    fn texel(&self, point: [f32; 2]) -> [f32; 2] {
        std::array::from_fn(|k| point[k] * self.scale[k] + self.offset[k])
    }

    fn layout(&self, texel: [f32; 2]) -> [f32; 2] {
        std::array::from_fn(|k| (texel[k] - self.offset[k]) / self.scale[k])
    }

    fn valid(&self) -> bool {
        self.scale
            .iter()
            .chain(&self.offset)
            .all(|value| value.is_finite())
            && self.scale.iter().all(|&value| value != 0.0)
    }
}

fn turned(turn: [f32; 3], point: [f32; 2], angle: f32) -> [f32; 2] {
    let (sin, cos) = angle.sin_cos();
    let dx = point[0] - turn[0];
    let dy = point[1] - turn[1];
    [turn[0] + cos * dx - sin * dy, turn[1] + sin * dx + cos * dy]
}

fn corners(glyph: &Glyph) -> [[f32; 2]; 4] {
    let [x, y, w, h] = glyph.rect;
    [[x, y], [x + w, y], [x, y + h], [x + w, y + h]].map(|corner| {
        if glyph.turn[2] == 0.0 {
            corner
        } else {
            turned(glyph.turn, corner, glyph.turn[2])
        }
    })
}

fn checked(atlas: &Atlas, glyphs: &[Glyph]) -> Result<(), String> {
    if atlas.channels != 1 && atlas.channels != 3 {
        return Err("text atlas must be a coverage or an MSDF atlas".into());
    }
    let level = atlas.levels.first().ok_or("text atlas has no levels")?;
    if level.width == 0
        || level.height == 0
        || level.bytes.len()
            != level.width as usize * level.height as usize * atlas.channels as usize
    {
        return Err("text atlas level has the wrong size".into());
    }
    let finite = glyphs.iter().all(|glyph| {
        glyph
            .rect
            .iter()
            .chain(&glyph.uv)
            .chain(&glyph.color)
            .chain(&glyph.turn)
            .all(|value| value.is_finite())
            && !glyph.clip.iter().any(|value| value.is_nan())
    });
    if !finite {
        return Err("text glyph is not finite".into());
    }
    Ok(())
}

fn sample(atlas: &Atlas, uv: [f32; 2]) -> [f32; 3] {
    let level = &atlas.levels[0];
    let channels = atlas.channels as usize;
    let size = [level.width as f32, level.height as f32];
    let x = uv[0] * size[0] - 0.5;
    let y = uv[1] * size[1] - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let column = |value: f32| (value.max(0.0) as usize).min(level.width as usize - 1);
    let row = |value: f32| (value.max(0.0) as usize).min(level.height as usize - 1);
    let at = |px: usize, py: usize| -> [f32; 3] {
        let base = (py * level.width as usize + px) * channels;
        std::array::from_fn(|k| f32::from(level.bytes[base + k.min(channels - 1)]) / 255.0)
    };
    let corners = [
        (at(column(x0), row(y0)), (1.0 - tx) * (1.0 - ty)),
        (at(column(x0 + 1.0), row(y0)), tx * (1.0 - ty)),
        (at(column(x0), row(y0 + 1.0)), (1.0 - tx) * ty),
        (at(column(x0 + 1.0), row(y0 + 1.0)), tx * ty),
    ];
    let mut out = [0.0; 3];
    for (texel, weight) in corners {
        for k in 0..3 {
            out[k] += texel[k] * weight;
        }
    }
    out
}

fn paint(
    atlas: &Atlas,
    glyphs: &[Glyph],
    grid: Grid,
    width: u32,
    height: u32,
    target: &mut [[f32; 4]],
) -> Result<(), String> {
    checked(atlas, glyphs)?;
    if !grid.valid() {
        return Err("text grid is not finite".into());
    }
    if target.len() != width as usize * height as usize {
        return Err("text target has the wrong size".into());
    }
    let level = &atlas.levels[0];
    let size = [level.width as f32, level.height as f32];
    let msdf = atlas.channels == 3;
    for glyph in glyphs {
        let [x, y, w, h] = glyph.rect;
        if w <= 0.0 || h <= 0.0 || glyph.color[3] <= 0.0 {
            continue;
        }
        let cell = [
            (glyph.uv[2] - glyph.uv[0]) * size[0],
            (glyph.uv[3] - glyph.uv[1]) * size[1],
        ];
        if cell[0] <= 0.0 || cell[1] <= 0.0 {
            continue;
        }
        let range = MSDF_SPREAD
            * 0.5
            * (grid.scale[0].abs() * w / cell[0] + grid.scale[1].abs() * h / cell[1]);
        let half = [0.5 / size[0], 0.5 / size[1]];
        let low = [
            (glyph.uv[0] + half[0]).min(glyph.uv[2]),
            (glyph.uv[1] + half[1]).min(glyph.uv[3]),
        ];
        let high = [
            (glyph.uv[2] - half[0]).max(low[0]),
            (glyph.uv[3] - half[1]).max(low[1]),
        ];
        let mut least = [f32::MAX; 2];
        let mut most = [f32::MIN; 2];
        for corner in corners(glyph) {
            let texel = grid.texel(corner);
            for k in 0..2 {
                least[k] = least[k].min(texel[k]);
                most[k] = most[k].max(texel[k]);
            }
        }
        let from = [least[0].floor().max(0.0), least[1].floor().max(0.0)];
        let to = [
            most[0].ceil().min(width as f32),
            most[1].ceil().min(height as f32),
        ];
        if from[0] >= to[0] || from[1] >= to[1] {
            continue;
        }
        let color = glyph.color;
        for row in from[1] as u32..to[1] as u32 {
            for column in from[0] as u32..to[0] as u32 {
                let centre = grid.layout([column as f32 + 0.5, row as f32 + 0.5]);
                let point = if glyph.turn[2] == 0.0 {
                    centre
                } else {
                    turned(glyph.turn, centre, -glyph.turn[2])
                };
                let fraction = [(point[0] - x) / w, (point[1] - y) / h];
                if !(0.0..=1.0).contains(&fraction[0]) || !(0.0..=1.0).contains(&fraction[1]) {
                    continue;
                }
                let inside = point[0] >= glyph.clip[0]
                    && point[0] <= glyph.clip[2]
                    && point[1] >= glyph.clip[1]
                    && point[1] <= glyph.clip[3];
                if !inside {
                    continue;
                }
                let uv = [
                    (glyph.uv[0] + (glyph.uv[2] - glyph.uv[0]) * fraction[0])
                        .clamp(low[0], high[0]),
                    (glyph.uv[1] + (glyph.uv[3] - glyph.uv[1]) * fraction[1])
                        .clamp(low[1], high[1]),
                ];
                let sampled = sample(atlas, uv);
                let coverage = if msdf {
                    msdf_coverage(sampled, range)
                } else {
                    sampled[0]
                };
                let opacity = coverage * color[3];
                if opacity <= 0.0 {
                    continue;
                }
                let pixel = &mut target[(row * width + column) as usize];
                let keep = 1.0 - opacity;
                for k in 0..3 {
                    pixel[k] = color[k] * opacity + pixel[k] * keep;
                }
                pixel[3] = opacity + pixel[3] * keep;
            }
        }
    }
    Ok(())
}

fn encode(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = pfx_materials::encode_channel(value);
    (encoded * 255.0).round() as u8
}

fn decode(byte: u8) -> f32 {
    pfx_materials::linear_channel(f32::from(byte) / 255.0)
}

fn bytes(pixel: [f32; 4], srgb: bool) -> [u8; 4] {
    let linear = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let colour = |value: f32| if srgb { encode(value) } else { linear(value) };
    [
        colour(pixel[0]),
        colour(pixel[1]),
        colour(pixel[2]),
        linear(pixel[3]),
    ]
}

pub fn print(atlas: &Atlas, glyphs: &[Glyph], density: f32) -> Result<Print, String> {
    if !density.is_finite() || density <= 0.0 {
        return Err("text density must be a positive number of texels per unit".into());
    }
    checked(atlas, glyphs)?;
    let mut least = [f32::MAX; 2];
    let mut most = [f32::MIN; 2];
    for glyph in glyphs
        .iter()
        .filter(|glyph| glyph.rect[2] > 0.0 && glyph.rect[3] > 0.0)
    {
        for corner in corners(glyph) {
            for k in 0..2 {
                least[k] = least[k].min(corner[k]);
                most[k] = most[k].max(corner[k]);
            }
        }
    }
    if least[0] > most[0] {
        return Err("text has no glyphs to print".into());
    }
    let first = least.map(|value| (value * density).floor());
    let last = most.map(|value| (value * density).ceil());
    let columns = (last[0] - first[0]).max(1.0);
    let rows = (last[1] - first[1]).max(1.0);
    if (columns as u64).saturating_mul(rows as u64) > MAX_TEXELS {
        return Err("text print is too large at this density".into());
    }
    let (width, height) = (columns as u32, rows as u32);
    let grid = Grid {
        scale: [density; 2],
        offset: [-first[0], -first[1]],
    };
    let mut pixels = vec![[0.0; 4]; width as usize * height as usize];
    paint(atlas, glyphs, grid, width, height, &mut pixels)?;
    Ok(Print {
        width,
        height,
        density,
        rect: [
            first[0] / density,
            first[1] / density,
            columns / density,
            rows / density,
        ],
        texels: pixels.into_iter().map(|pixel| bytes(pixel, true)).collect(),
    })
}

#[derive(Clone, Debug)]
pub struct Lettering {
    pub rect: [f32; 4],
    pub density: f32,
    pub glyphs: Vec<Glyph>,
    pub atlas: Atlas,
}

fn drawn(glyph: &Glyph, size: [f32; 2]) -> bool {
    glyph.rect[2] > 0.0
        && glyph.rect[3] > 0.0
        && glyph.color[3] > 0.0
        && glyph.uv[2] > glyph.uv[0]
        && glyph.uv[3] > glyph.uv[1]
        && size[0] > 0.0
        && size[1] > 0.0
}

pub fn lettering(atlas: &Atlas, glyphs: &[Glyph]) -> Result<Lettering, String> {
    checked(atlas, glyphs)?;
    let level = &atlas.levels[0];
    let size = [level.width as f32, level.height as f32];
    let mut least = [f32::MAX; 2];
    let mut most = [f32::MIN; 2];
    let mut density = 0.0f64;
    let mut counted = 0usize;
    for glyph in glyphs.iter().filter(|glyph| drawn(glyph, size)) {
        for corner in corners(glyph) {
            for k in 0..2 {
                least[k] = least[k].min(corner[k]);
                most[k] = most[k].max(corner[k]);
            }
        }
        let cell = [
            (glyph.uv[2] - glyph.uv[0]) * size[0],
            (glyph.uv[3] - glyph.uv[1]) * size[1],
        ];
        density += 0.5 * f64::from(cell[0] / glyph.rect[2] + cell[1] / glyph.rect[3]);
        counted += 1;
    }
    if counted == 0 || least[0] >= most[0] || least[1] >= most[1] {
        return Err("text has no glyphs to letter".into());
    }
    let density = (density / counted as f64) as f32;
    if !density.is_finite() || density <= 0.0 {
        return Err("text glyphs have no atlas density".into());
    }
    Ok(Lettering {
        rect: [least[0], least[1], most[0] - least[0], most[1] - least[1]],
        density,
        glyphs: glyphs.to_vec(),
        atlas: atlas.clone(),
    })
}

impl Laid for Lettering {
    fn rect(&self) -> [f32; 4] {
        self.rect
    }

    fn kind(&self) -> ContentKind<'_> {
        ContentKind::Text(self.shown())
    }
}

impl Lettering {
    pub fn shown(&self) -> TextContent<'_> {
        let [x, y, w, h] = self.rect;
        TextContent {
            lettering: self,
            scale: [1.0 / w, 1.0 / h],
            offset: [-x / w, -y / h],
            picture: None,
        }
    }

    pub fn content(&self) -> PlacementContent<'_> {
        PlacementContent {
            cutout: true,
            ..PlacementContent::text(self.shown())
        }
    }

    pub fn carrier(&self) -> Carrier {
        Carrier::over(self.rect)
    }

    pub fn sampler(&self) -> impl Fn([f32; 2]) -> [f32; 4] + '_ {
        let shown = self.shown();
        let sample = shown.sampler();
        move |layout| {
            sample(std::array::from_fn(|k| {
                layout[k] * shown.scale[k] + shown.offset[k]
            }))
        }
    }

    pub fn at(&self, layout: [f32; 2]) -> [f32; 4] {
        self.sampler()(layout)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TextContent<'a> {
    pub lettering: &'a Lettering,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub picture: Option<Image<'a>>,
}

impl PartialEq for TextContent<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    map: [f32; 4],
    shift: [f32; 2],
    clip: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    reach: [f32; 4],
}

impl Placed {
    fn fraction(&self, uv: [f32; 2]) -> Option<[f32; 2]> {
        let m = self.map;
        let f = [
            m[0] * uv[0] + m[1] * uv[1] + self.shift[0],
            m[2] * uv[0] + m[3] * uv[1] + self.shift[1],
        ];
        let inside =
            (0..2).all(|k| f[k] >= self.clip[k].max(0.0) && f[k] <= self.clip[k + 2].min(1.0));
        inside.then_some(f)
    }
}

fn over(below: [f32; 4], color: [f32; 4], opacity: f32) -> [f32; 4] {
    let keep = 1.0 - opacity;
    [
        color[0] * opacity + below[0] * keep,
        color[1] * opacity + below[1] * keep,
        color[2] * opacity + below[2] * keep,
        opacity + below[3] * keep,
    ]
}

fn texel_at(image: &crate::detail::ContentImage, uv: [f32; 2]) -> [f32; 4] {
    let size = [image.width as f32, image.height as f32];
    let p = [0, 1].map(|k| (uv[k] * size[k] - 0.5).clamp(0.0, size[k] - 1.0));
    let lo = p.map(|v| v.floor() as u32);
    let hi = [
        (lo[0] + 1).min(image.width - 1),
        (lo[1] + 1).min(image.height - 1),
    ];
    let f = [p[0] - p[0].floor(), p[1] - p[1].floor()];
    let at = |column: u32, row: u32| image.texels[(row * image.width + column) as usize];
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    };
    mix(
        mix(at(lo[0], lo[1]), at(hi[0], lo[1]), f[0]),
        mix(at(lo[0], hi[1]), at(hi[0], hi[1]), f[0]),
        f[1],
    )
}

fn coverage(atlas: &Atlas, glyph: &Placed, fraction: [f32; 2]) -> f32 {
    let level = &atlas.levels[0];
    let half = [0.5 / level.width as f32, 0.5 / level.height as f32];
    let uv = std::array::from_fn(|k| {
        let low = (glyph.uv[k] + half[k]).min(glyph.uv[k + 2]);
        let high = (glyph.uv[k + 2] - half[k]).max(low);
        (glyph.uv[k] + (glyph.uv[k + 2] - glyph.uv[k]) * fraction[k]).clamp(low, high)
    });
    let sampled = sample(atlas, uv);
    if atlas.channels == 3 {
        let median = sampled[0]
            .min(sampled[1])
            .max(sampled[0].max(sampled[1]).min(sampled[2]));
        if median >= 0.5 { 1.0 } else { 0.0 }
    } else {
        sampled[0]
    }
}

fn whole(value: usize) -> f32 {
    value as f32
}

impl<'a> TextContent<'a> {
    pub fn same(&self, other: &TextContent<'_>) -> bool {
        std::ptr::eq(self.lettering, other.lettering)
            && self.scale == other.scale
            && self.offset == other.offset
            && match (self.picture, other.picture) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same(&b),
                _ => false,
            }
    }

    pub fn valid(&self) -> bool {
        self.scale
            .iter()
            .chain(&self.offset)
            .all(|value| value.is_finite())
            && self.scale.iter().all(|&value| value != 0.0)
            && self.picture.is_none_or(|picture| picture.valid())
            && checked(&self.lettering.atlas, &self.lettering.glyphs).is_ok()
    }

    fn placed(&self) -> Vec<Placed> {
        let level = &self.lettering.atlas.levels[0];
        let size = [level.width as f32, level.height as f32];
        let scale = self.scale.map(f64::from);
        let offset = self.offset.map(f64::from);
        self.lettering
            .glyphs
            .iter()
            .filter(|glyph| drawn(glyph, size))
            .map(|glyph| {
                let [x, y, w, h] = glyph.rect.map(f64::from);
                let angle = f64::from(glyph.turn[2]);
                let (sin, cos) = (-angle).sin_cos();
                let rotation = [[cos, -sin], [sin, cos]];
                let pivot = [f64::from(glyph.turn[0]), f64::from(glyph.turn[1])];
                let base = [-offset[0] / scale[0], -offset[1] / scale[1]];
                let moved: [f64; 2] = std::array::from_fn(|row| {
                    rotation[row][0] * (base[0] - pivot[0])
                        + rotation[row][1] * (base[1] - pivot[1])
                        + pivot[row]
                });
                let extent = [w, h];
                let start = [x, y];
                let map = [
                    rotation[0][0] / scale[0] / w,
                    rotation[0][1] / scale[1] / w,
                    rotation[1][0] / scale[0] / h,
                    rotation[1][1] / scale[1] / h,
                ];
                let shift: [f64; 2] = std::array::from_fn(|k| (moved[k] - start[k]) / extent[k]);
                let clip = std::array::from_fn(|k| {
                    let value = (f64::from(glyph.clip[k]) - start[k % 2]) / extent[k % 2];
                    value.clamp(-1.0, 2.0) as f32
                });
                let mut least = [f32::MAX; 2];
                let mut most = [f32::MIN; 2];
                for corner in corners(glyph) {
                    for k in 0..2 {
                        let uv = corner[k] * self.scale[k] + self.offset[k];
                        least[k] = least[k].min(uv);
                        most[k] = most[k].max(uv);
                    }
                }
                Placed {
                    map: map.map(|value| value as f32),
                    shift: shift.map(|value| value as f32),
                    clip,
                    uv: glyph.uv,
                    color: glyph.color,
                    reach: [least[0], least[1], most[0], most[1]],
                }
            })
            .collect()
    }

    pub fn sampler(self) -> impl Fn([f32; 2]) -> [f32; 4] + 'a {
        let placed = self.placed();
        let picture = self.picture.map(|picture| picture.linear());
        move |uv| {
            let mut color = picture
                .as_ref()
                .map_or([0.0; 4], |image| texel_at(image, uv));
            for glyph in &placed {
                if let Some(fraction) = glyph.fraction(uv) {
                    let opacity = coverage(&self.lettering.atlas, glyph, fraction) * glyph.color[3];
                    color = over(color, glyph.color, opacity);
                }
            }
            color
        }
    }

    pub fn at(&self, uv: [f32; 2]) -> [f32; 4] {
        self.sampler()(uv)
    }

    pub fn records(&self) -> Result<ContentText, String> {
        if !self.valid() {
            return Err("text content is not finite or has a broken atlas".into());
        }
        let placed = self.placed();
        if placed.is_empty() {
            return Err("text content has no glyphs to draw".into());
        }
        let mut least = [f32::MAX; 2];
        let mut most = [f32::MIN; 2];
        let mut mean = [0.0f64; 2];
        for glyph in &placed {
            for k in 0..2 {
                least[k] = least[k].min(glyph.reach[k]);
                most[k] = most[k].max(glyph.reach[k + 2]);
                mean[k] += f64::from(glyph.reach[k + 2] - glyph.reach[k]);
            }
        }
        let extent = [most[0] - least[0], most[1] - least[1]];
        if !extent.iter().all(|value| value.is_finite() && *value > 0.0) {
            return Err("text content has no extent".into());
        }
        let mut counts = [0, 1].map(|k| {
            let mean = mean[k] / placed.len() as f64;
            (f64::from(extent[k]) / mean.max(1e-30))
                .ceil()
                .clamp(1.0, 1024.0) as usize
        });
        let budget = MAX_CELLS.min(64.max(16 * placed.len()));
        while counts[0] * counts[1] > budget {
            counts = counts.map(|count| count.div_ceil(2));
        }
        let [columns, rows] = counts;
        let inverse = [columns as f32 / extent[0], rows as f32 / extent[1]];
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); columns * rows];
        for (index, glyph) in placed.iter().enumerate() {
            let range = |k: usize, value: f32, nudge: f32| {
                ((value - least[k]) * inverse[k] + nudge)
                    .floor()
                    .clamp(0.0, (counts[k] - 1) as f32) as usize
            };
            for row in range(1, glyph.reach[1], -1e-3)..=range(1, glyph.reach[3], 1e-3) {
                for column in range(0, glyph.reach[0], -1e-3)..=range(0, glyph.reach[2], 1e-3) {
                    cells[row * columns + column].push(index as u32);
                }
            }
        }
        let atlas = &self.lettering.atlas;
        let level = &atlas.levels[0];
        let channels = atlas.channels as usize;
        let atlas_texels = level.width as usize * level.height as usize;
        let picture = self.picture.map(|picture| picture.linear());
        let total: usize = cells.iter().map(Vec::len).sum();
        let index_base = 4 + cells.len().div_ceil(2);
        let glyph_base = index_base + total.div_ceil(4);
        let atlas_base = glyph_base + 5 * placed.len();
        let picture_base = atlas_base + atlas_texels.div_ceil(4);
        let length = picture_base + picture.as_ref().map_or(0, |image| image.texels.len());
        if length > MAX_RECORDS {
            return Err("text content is too large for the tracer".into());
        }
        let mut records = Vec::with_capacity(length);
        records.push([least[0], least[1], inverse[0], inverse[1]]);
        records.push([
            whole(columns),
            whole(rows),
            whole(glyph_base),
            whole(index_base),
        ]);
        records.push([
            whole(level.width as usize),
            whole(level.height as usize),
            whole(atlas_base),
            whole(channels),
        ]);
        records.push(match &picture {
            Some(image) => [
                whole(image.width as usize),
                whole(image.height as usize),
                whole(picture_base),
                0.0,
            ],
            None => [0.0; 4],
        });
        let mut start = 0usize;
        let mut entries = Vec::with_capacity(cells.len() * 2);
        for cell in &cells {
            entries.push(whole(start));
            entries.push(whole(cell.len()));
            start += cell.len();
        }
        let quads = |values: &[f32], records: &mut Vec<[f32; 4]>| {
            for chunk in values.chunks(4) {
                records.push(std::array::from_fn(|k| {
                    chunk.get(k).copied().unwrap_or(0.0)
                }));
            }
        };
        quads(&entries, &mut records);
        let indices: Vec<f32> = cells.iter().flatten().map(|&index| index as f32).collect();
        quads(&indices, &mut records);
        for glyph in &placed {
            records.push(glyph.map);
            records.push([glyph.shift[0], glyph.shift[1], 0.0, 0.0]);
            records.push(glyph.clip);
            records.push(glyph.uv);
            records.push(glyph.color);
        }
        let packed: Vec<f32> = level
            .bytes
            .chunks_exact(channels)
            .map(|texel| {
                let at = |k: usize| u32::from(texel[k.min(channels - 1)]);
                (at(0) | at(1) << 8 | at(2) << 16) as f32
            })
            .collect();
        quads(&packed, &mut records);
        if let Some(image) = &picture {
            records.extend_from_slice(&image.texels);
        }
        debug_assert_eq!(records.len(), length);
        let (width, height) = match &picture {
            Some(image) => (image.width, image.height),
            None => {
                let nominal = |k: usize| {
                    (self.lettering.density / self.scale[k].abs())
                        .round()
                        .clamp(1.0, NOMINAL_LIMIT) as u32
                };
                (nominal(0), nominal(1))
            }
        };
        let text = ContentText {
            width,
            height,
            records,
        };
        if !text.valid() {
            return Err("text content records are not finite".into());
        }
        Ok(text)
    }
}

pub fn overlay(
    pixels: &mut [[f32; 4]],
    width: u32,
    height: u32,
    atlas: &Atlas,
    glyphs: &[Glyph],
) -> Result<(), String> {
    let grid = Grid {
        scale: [1.0; 2],
        offset: [0.0; 2],
    };
    paint(atlas, glyphs, grid, width, height, pixels)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub face: Vec<usize>,
}

fn affine_inverse(m: Transform) -> Option<[[f32; 4]; 3]> {
    let a = |row: usize, column: usize| f64::from(m[column][row]);
    if m.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let cofactor = |row: usize, column: usize| {
        let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
        let (c0, c1) = ((column + 1) % 3, (column + 2) % 3);
        a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0)
    };
    let determinant: f64 = (0..3)
        .map(|column| a(0, column) * cofactor(0, column))
        .sum();
    if determinant.abs() < 1e-18 {
        return None;
    }
    let inverse: [[f64; 3]; 3] = std::array::from_fn(|row| {
        std::array::from_fn(|column| cofactor(column, row) / determinant)
    });
    let shift = [a(0, 3), a(1, 3), a(2, 3)];
    Some(std::array::from_fn(|row| {
        let moved: f64 = (0..3).map(|k| inverse[row][k] * shift[k]).sum();
        [
            inverse[row][0] as f32,
            inverse[row][1] as f32,
            inverse[row][2] as f32,
            -moved as f32,
        ]
    }))
}

fn apply(m: Transform, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| m[0][row] * p[0] + m[1][row] * p[1] + m[2][row] * p[2] + m[3][row])
}

fn solve(rows: [[f64; 4]; 3]) -> Option<[f64; 3]> {
    let mut m = rows;
    for pivot in 0..3 {
        let best = (pivot..3).max_by(|&a, &b| m[a][pivot].abs().total_cmp(&m[b][pivot].abs()))?;
        if m[best][pivot].abs() < 1e-18 {
            return None;
        }
        m.swap(pivot, best);
        for row in 0..3 {
            if row != pivot {
                let factor = m[row][pivot] / m[pivot][pivot];
                let lead = m[pivot];
                for (value, above) in m[row].iter_mut().zip(lead).skip(pivot) {
                    *value -= factor * above;
                }
            }
        }
    }
    Some(std::array::from_fn(|k| m[k][3] / m[k][k]))
}

pub fn fit(part: &Mesh<'_>, part_model: Transform, text_model: Transform) -> Result<Fit, String> {
    let back = affine_inverse(text_model).ok_or("text model is not an invertible affine map")?;
    if affine_inverse(part_model).is_none() {
        return Err("part model is not an invertible affine map".into());
    }
    if part.uvs.len() != part.positions.len()
        || !part.indices.len().is_multiple_of(3)
        || part
            .indices
            .iter()
            .any(|&index| index as usize >= part.positions.len())
    {
        return Err("part mesh has mismatched UVs or indices".into());
    }
    let layout = |index: u32| {
        let world = apply(part_model, part.positions[index as usize]);
        std::array::from_fn::<f32, 3, _>(|row| {
            back[row][0] * world[0]
                + back[row][1] * world[1]
                + back[row][2] * world[2]
                + back[row][3]
        })
    };
    let mut parallel = Vec::new();
    let mut reach = 0.0f32;
    for (triangle, face) in part.indices.chunks_exact(3).enumerate() {
        let [a, b, c] = [layout(face[0]), layout(face[1]), layout(face[2])];
        let e1: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
        let e2: [f32; 3] = std::array::from_fn(|k| c[k] - a[k]);
        let normal = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length <= 0.0 || normal[2].abs() < 0.999 * length {
            continue;
        }
        for p in [a, b, c] {
            reach = reach.max(p[0].abs()).max(p[1].abs());
        }
        parallel.push((triangle, (a[2] + b[2] + c[2]).abs() / 3.0));
    }
    let nearest = parallel
        .iter()
        .map(|&(_, distance)| distance)
        .fold(f32::INFINITY, f32::min);
    if !nearest.is_finite() {
        return Err("part has no face parallel to the text".into());
    }
    let tolerance = nearest + 1e-4 * reach.max(1e-6);
    let face: Vec<usize> = parallel
        .into_iter()
        .filter(|&(_, distance)| distance <= tolerance)
        .map(|(triangle, _)| triangle)
        .collect();
    let mut sums = [[0.0f64; 4]; 3];
    let mut targets = [[0.0f64; 3]; 2];
    let mut points = Vec::new();
    for &triangle in &face {
        for &index in &part.indices[triangle * 3..triangle * 3 + 3] {
            let p = layout(index);
            let row = [f64::from(p[0]), f64::from(p[1]), 1.0];
            let uv = part.uvs[index as usize].map(f64::from);
            for i in 0..3 {
                for j in 0..3 {
                    sums[i][j] += row[i] * row[j];
                }
                for k in 0..2 {
                    targets[k][i] += row[i] * uv[k];
                }
            }
            points.push((row, uv));
        }
    }
    let mut maps = [[0.0f64; 3]; 2];
    for k in 0..2 {
        let rows: [[f64; 4]; 3] =
            std::array::from_fn(|i| [sums[i][0], sums[i][1], sums[i][2], targets[k][i]]);
        maps[k] = solve(rows).ok_or("part face's UVs do not span it")?;
    }
    let spread = points
        .iter()
        .flat_map(|(_, uv)| uv.iter().map(|value| value.abs()))
        .fold(1e-6, f64::max);
    for (row, uv) in &points {
        for k in 0..2 {
            let predicted: f64 = (0..3).map(|i| maps[k][i] * row[i]).sum();
            if (predicted - uv[k]).abs() > 1e-3 * spread {
                return Err("part face's UVs are not a flat map of it".into());
            }
        }
    }
    let [u, v] = maps;
    if u[0] == 0.0
        || v[1] == 0.0
        || u[1].abs() > 1e-3 * u[0].abs()
        || v[0].abs() > 1e-3 * v[1].abs()
    {
        return Err(
            "part face's UVs turn against the text; only UVs along its axes are fitted".into(),
        );
    }
    Ok(Fit {
        scale: [u[0] as f32, v[1] as f32],
        offset: [u[2] as f32, v[2] as f32],
        face,
    })
}

impl Fit {
    pub fn content<'a, L: Laid>(&self, laid: &'a L) -> PlacementContent<'a> {
        let [x, y, w, h] = laid.rect();
        PlacementContent {
            uv_offset: [
                -(self.offset[0] / self.scale[0] + x) / w,
                -(self.offset[1] / self.scale[1] + y) / h,
            ],
            uv_scale: [1.0 / (self.scale[0] * w), 1.0 / (self.scale[1] * h)],
            crop: [0.0, 0.0, 1.0, 1.0],
            face: ContentFace::Both,
            cutout: false,
            kind: laid.kind(),
        }
    }

    pub fn over<'a>(
        &self,
        picture: &PlacementContent<'a>,
        lettering: &'a Lettering,
    ) -> Result<PlacementContent<'a>, String> {
        let ContentKind::Image(image) = picture.kind else {
            return Err("text goes over a picture, not over other text".into());
        };
        if !image.valid() {
            return Err("picture has the wrong texel count".into());
        }
        let text = TextContent {
            lettering,
            scale: std::array::from_fn(|k| self.scale[k] * picture.uv_scale[k]),
            offset: std::array::from_fn(|k| {
                self.offset[k] * picture.uv_scale[k] + picture.uv_offset[k]
            }),
            picture: Some(image),
        };
        if !text.valid() {
            return Err("text over the picture is not finite".into());
        }
        Ok(PlacementContent {
            kind: ContentKind::Text(text),
            ..*picture
        })
    }

    pub fn elsewhere(&self, part: &Mesh<'_>, content: &PlacementContent<'_>) -> Vec<usize> {
        let shown = |uv: [f32; 2]| -> [f32; 2] {
            std::array::from_fn(|k| uv[k] * content.uv_scale[k] + content.uv_offset[k])
        };
        part.indices
            .chunks_exact(3)
            .enumerate()
            .filter(|(triangle, _)| !self.face.contains(triangle))
            .filter(|(_, face)| {
                let at: Vec<[f32; 2]> = face
                    .iter()
                    .map(|&index| {
                        shown(
                            part.uvs
                                .get(index as usize)
                                .copied()
                                .unwrap_or([f32::NAN; 2]),
                        )
                    })
                    .collect();
                let low = [0, 1].map(|k| at.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min));
                let high =
                    [0, 1].map(|k| at.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max));
                low[0] < content.crop[2]
                    && high[0] > content.crop[0]
                    && low[1] < content.crop[3]
                    && high[1] > content.crop[1]
            })
            .map(|(triangle, _)| triangle)
            .collect()
    }

    pub fn onto(
        &self,
        picture: &PlacementContent<'_>,
        atlas: &Atlas,
        glyphs: &[Glyph],
    ) -> Result<Vec<[u8; 4]>, String> {
        let ContentKind::Image(image) = &picture.kind else {
            return Err("text prints onto a picture, not onto other text".into());
        };
        if !image.valid() {
            return Err("picture has the wrong texel count".into());
        }
        let size = [image.width as f32, image.height as f32];
        let grid = Grid {
            scale: std::array::from_fn(|k| self.scale[k] * picture.uv_scale[k] * size[k]),
            offset: std::array::from_fn(|k| {
                (self.offset[k] * picture.uv_scale[k] + picture.uv_offset[k]) * size[k]
            }),
        };
        let unpack = |byte: u8| {
            if image.srgb {
                decode(byte)
            } else {
                f32::from(byte) / 255.0
            }
        };
        let mut pixels: Vec<[f32; 4]> = image
            .texels
            .iter()
            .map(|&[r, g, b, a]| [unpack(r), unpack(g), unpack(b), f32::from(a) / 255.0])
            .collect();
        paint(atlas, glyphs, grid, image.width, image.height, &mut pixels)?;
        Ok(pixels
            .into_iter()
            .map(|pixel| bytes(pixel, image.srgb))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_text::{Anchor, Face, Representation, Span, TextEngine};

    const FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;
    const UNITS_PER_METRE: f32 = 8192.0;
    const OPEN: [f32; 4] = [-1e9, -1e9, 1e9, 1e9];
    const COLOR: [f32; 4] = [1.0, 0.5, 0.25, 1.0];

    fn laid(text: &str, size: f32, scale: f32, at: [f32; 2]) -> (Atlas, Vec<Glyph>) {
        let representation = if scale == 1.0 {
            Representation::Coverage
        } else {
            Representation::Msdf
        };
        let mut engine = TextEngine::new(FONT).unwrap();
        let spans = [Span {
            text: text.into(),
            face: Face {
                family: "EB Garamond".into(),
                size,
                line: size * 1.25,
                weight: 400,
                italic: false,
                spacing: 0.0,
            },
            color: COLOR,
        }];
        let rich = engine
            .render_spans(
                &spans,
                None,
                scale,
                Anchor::Start,
                representation,
                at,
                1.0,
                OPEN,
                [0.0; 3],
            )
            .unwrap();
        let glyphs = glyphs(&rich);
        (rich.atlas, glyphs)
    }

    struct Cube {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        tangents: Vec<[f32; 4]>,
        uvs: Vec<[f32; 2]>,
        indices: Vec<u32>,
    }

    impl Cube {
        fn new() -> Self {
            let mut cube = Self {
                positions: Vec::new(),
                normals: Vec::new(),
                tangents: Vec::new(),
                uvs: Vec::new(),
                indices: Vec::new(),
            };
            for axis in 0..3 {
                for sign in [-1.0_f32, 1.0] {
                    let mut normal = [0.0; 3];
                    normal[axis] = sign;
                    let start = cube.positions.len() as u32;
                    for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                        let mut p = normal;
                        p[(axis + 1) % 3] = a;
                        p[(axis + 2) % 3] = b;
                        cube.positions.push(p.map(|v| v * 0.5));
                        cube.normals.push(normal);
                        cube.tangents.push([1.0, 0.0, 0.0, 1.0]);
                        cube.uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
                    }
                    let quad = if sign > 0.0 {
                        [0, 1, 2, 0, 2, 3]
                    } else {
                        [0, 2, 1, 0, 3, 2]
                    };
                    cube.indices.extend(quad.map(|k| start + k));
                }
            }
            cube
        }

        fn mesh(&self) -> Mesh<'_> {
            Mesh {
                positions: &self.positions,
                normals: &self.normals,
                tangents: &self.tangents,
                uvs: &self.uvs,
                alpha: None,
                indices: &self.indices,
            }
        }
    }

    const IDENTITY: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    const ON_FRONT: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.5005, 1.0],
    ];

    #[test]
    fn a_print_covers_every_glyph_on_whole_texels_with_premultiplied_srgb_colour() {
        let (atlas, glyphs) = laid("Hi", 0.1, UNITS_PER_METRE, [0.25, -0.5]);
        let density = 2000.0;
        let print = print(&atlas, &glyphs, density).unwrap();
        assert!(print.image().valid());
        let [x, y, w, h] = print.rect;
        assert_eq!(print.width as f32, (w * density).round());
        assert_eq!(print.height as f32, (h * density).round());
        assert_eq!((x * density).fract(), 0.0);
        for glyph in &glyphs {
            assert!(glyph.rect[0] >= x && glyph.rect[1] >= y);
            assert!(glyph.rect[0] + glyph.rect[2] <= x + w + 1e-6);
            assert!(glyph.rect[1] + glyph.rect[3] <= y + h + 1e-6);
        }
        let at = |column: u32, row: u32| print.texels[(row * print.width + column) as usize];
        for column in 0..print.width {
            assert_eq!(at(column, 0)[3], 0);
            assert_eq!(at(column, print.height - 1)[3], 0);
        }
        let solid: Vec<[u8; 4]> = print
            .texels
            .iter()
            .copied()
            .filter(|texel| texel[3] == 255)
            .collect();
        assert!(solid.len() > 100, "{} solid texels", solid.len());
        let want = [encode(1.0), encode(0.5), encode(0.25), 255];
        assert!(
            solid
                .iter()
                .all(|texel| texel.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 1)),
            "{:?}",
            solid[0]
        );
        assert!(solid.contains(&want));
        let partial = print
            .texels
            .iter()
            .find(|t| (64..192).contains(&t[3]))
            .unwrap();
        let alpha = f32::from(partial[3]) / 255.0;
        assert!(
            (decode(partial[1]) - 0.5 * alpha).abs() < 0.01,
            "{partial:?}"
        );

        let content = print.content();
        assert!(content.cutout);
        assert_eq!(content.kind, ContentKind::Image(print.image()));
        assert_eq!(content.crop, [0.0, 0.0, 1.0, 1.0]);
        let carrier = print.carrier();
        assert_eq!(carrier.positions[0], [x, y, 0.0]);
        assert_eq!(carrier.positions[2], [x + w, y + h, 0.0]);
        assert_eq!(carrier.uvs[2], [1.0, 1.0]);
        let mesh = carrier.mesh();
        assert_eq!(mesh.indices.len(), 6);
    }

    #[test]
    fn msdf_edges_stay_about_a_texel_wide_at_any_density() {
        let (atlas, glyphs) = laid("l", 0.2, UNITS_PER_METRE, [0.0; 2]);
        for density in [400.0, 1500.0, 6000.0] {
            let print = print(&atlas, &glyphs, density).unwrap();
            let row = print.height / 2;
            let line =
                &print.texels[(row * print.width) as usize..((row + 1) * print.width) as usize];
            let partial = line.iter().filter(|t| t[3] > 8 && t[3] < 247).count();
            let solid = line.iter().filter(|t| t[3] >= 247).count();
            assert!(solid > 0, "{density}: no solid texel");
            assert!(
                partial <= 4,
                "{density}: {partial} partial texels across a stem"
            );
        }
    }

    #[test]
    fn a_fit_maps_a_box_face_and_names_the_faces_sharing_its_uvs() {
        let cube = Cube::new();
        let part = cube.mesh();
        let fit = fit(&part, IDENTITY, ON_FRONT).unwrap();
        assert_eq!(fit.face, [6 * 2 - 2, 6 * 2 - 1]);
        assert!((fit.scale[0] - 1.0).abs() < 1e-5 && (fit.scale[1] + 1.0).abs() < 1e-5);
        assert!((fit.offset[0] - 0.5).abs() < 1e-5 && (fit.offset[1] - 0.5).abs() < 1e-5);

        let (atlas, glyphs) = laid("on", 0.08, UNITS_PER_METRE, [-0.1, -0.05]);
        let print = print(&atlas, &glyphs, 1000.0).unwrap();
        let content = fit.content(&print);
        assert!(!content.cutout);
        let [x, y, w, h] = print.rect;
        for layout in [[x, y], [x + w, y + h], [x + 0.3 * w, y + 0.6 * h]] {
            let world = [layout[0], -layout[1]];
            let uv = [world[0] + 0.5, world[1] + 0.5];
            let shown: [f32; 2] =
                std::array::from_fn(|k| uv[k] * content.uv_scale[k] + content.uv_offset[k]);
            let want = [(layout[0] - x) / w, (layout[1] - y) / h];
            for k in 0..2 {
                assert!(
                    (shown[k] - want[k]).abs() < 1e-4,
                    "{shown:?} against {want:?}"
                );
            }
        }
        let shared = fit.elsewhere(&part, &content);
        assert_eq!(shared.len(), 10);
        assert!(shared.contains(&4) && shared.contains(&5));
    }

    #[test]
    fn a_fit_refuses_uvs_turned_against_the_text_and_parts_without_a_face() {
        let cube = Cube::new();
        let part = cube.mesh();
        let quarter = [
            [0.0, 1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.5005, 1.0],
        ];
        assert!(fit(&part, IDENTITY, quarter).is_err());
        let slanted = [
            [0.8, 0.0, 0.6, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [-0.6, 0.0, 0.8, 0.0],
            [0.0, 0.0, 0.5005, 1.0],
        ];
        assert!(fit(&part, IDENTITY, slanted).is_err());
        let flat = [
            [0.0; 4],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert!(fit(&part, IDENTITY, flat).is_err());
    }

    #[test]
    fn text_composes_onto_a_picture_where_the_fit_puts_it() {
        let cube = Cube::new();
        let part = cube.mesh();
        let fit = fit(&part, IDENTITY, ON_FRONT).unwrap();
        let (atlas, glyphs) = laid("ab", 0.1, UNITS_PER_METRE, [-0.1, -0.05]);
        let grey = vec![[128, 128, 128, 255]; 256 * 256];
        let picture = PlacementContent::new(Image {
            width: 256,
            height: 256,
            texels: &grey,
            srgb: true,
        });
        let composed = fit.onto(&picture, &atlas, &glyphs).unwrap();
        assert_eq!(composed.len(), grey.len());
        let print = print(&atlas, &glyphs, 256.0).unwrap();
        let [x, y, w, h] = print.rect;
        let box_texels = |lx: f32, ly: f32| [(lx + 0.5) * 256.0, (-ly + 0.5) * 256.0];
        let [left, bottom] = box_texels(x, y);
        let [right, top] = box_texels(x + w, y + h);
        let mut changed = 0;
        for (index, texel) in composed.iter().enumerate() {
            if *texel == grey[index] {
                continue;
            }
            changed += 1;
            let column = (index % 256) as f32 + 0.5;
            let row = (index / 256) as f32 + 0.5;
            assert!(
                column >= left && column <= right && row >= top && row <= bottom,
                "{column}, {row} outside {left}..{right}, {top}..{bottom}"
            );
            assert_eq!(texel[3], 255);
        }
        assert!(changed > 50, "{changed} texels changed");
        let inked = composed
            .iter()
            .filter(|t| t[0] == encode(1.0) && t[1] == encode(0.5) && t[2] == encode(0.25))
            .count();
        assert!(inked > 10, "{inked} texels carry the ink");
    }

    #[test]
    fn overlay_text_blends_premultiplied_over_the_frame_in_pixels() {
        let (atlas, glyphs) = laid("Ink", 32.0, 1.0, [10.0, 20.0]);
        assert_eq!(atlas.channels, 1);
        let (width, height) = (96, 64);
        let background = [0.2, 0.3, 0.4, 1.0];
        let mut pixels = vec![background; (width * height) as usize];
        overlay(&mut pixels, width, height, &atlas, &glyphs).unwrap();
        let solid = pixels.iter().filter(|p| **p == COLOR).count();
        let mixed = pixels
            .iter()
            .filter(|p| **p != COLOR && **p != background)
            .count();
        assert!(solid > 40 && mixed > 40, "{solid} solid, {mixed} mixed");
        for pixel in &pixels {
            let t = (pixel[0] - background[0]) / (COLOR[0] - background[0]);
            for k in 1..3 {
                let expected = background[k] + (COLOR[k] - background[k]) * t;
                assert!((pixel[k] - expected).abs() < 1e-5, "{pixel:?}");
            }
        }
        let rows_touched = (0..height)
            .filter(|row| {
                pixels[(row * width) as usize..((row + 1) * width) as usize]
                    .iter()
                    .any(|p| *p != background)
            })
            .collect::<Vec<_>>();
        assert!(rows_touched[0] >= 20 && *rows_touched.last().unwrap() < 20 + 40);
    }

    #[test]
    fn broken_input_is_refused() {
        let (atlas, glyphs) = laid("x", 0.1, UNITS_PER_METRE, [0.0; 2]);
        assert!(print(&atlas, &glyphs, 0.0).is_err());
        assert!(print(&atlas, &glyphs, f32::NAN).is_err());
        assert!(print(&atlas, &[], 100.0).is_err());
        let four = Atlas {
            channels: 4,
            ..atlas.clone()
        };
        assert!(print(&four, &glyphs, 100.0).is_err());
        let bad = Glyph {
            rect: [f32::NAN, 0.0, 1.0, 1.0],
            ..glyphs[0]
        };
        assert!(print(&atlas, &[bad], 100.0).is_err());
        assert!(print(&atlas, &glyphs, 1e9).is_err());
        let mut short = vec![[0.0; 4]; 3];
        assert!(overlay(&mut short, 2, 2, &atlas, &glyphs).is_err());
    }

    fn evaluate(records: &[[f32; 4]], uv: [f32; 2]) -> [f32; 4] {
        let grid = records[0];
        let shape = records[1].map(|value| value as usize);
        let atlas = records[2].map(|value| value as usize);
        let picture = records[3].map(|value| value as usize);
        let mut color = [0.0; 4];
        if picture[0] > 0 {
            let image = crate::detail::ContentImage {
                width: picture[0] as u32,
                height: picture[1] as u32,
                texels: records[picture[2]..picture[2] + picture[0] * picture[1]].to_vec(),
            };
            color = texel_at(&image, uv);
        }
        let cell = [
            ((uv[0] - grid[0]) * grid[2]).floor(),
            ((uv[1] - grid[1]) * grid[3]).floor(),
        ];
        if cell[0] < 0.0
            || cell[1] < 0.0
            || cell[0] >= shape[0] as f32
            || cell[1] >= shape[1] as f32
        {
            return color;
        }
        let index = cell[1] as usize * shape[0] + cell[0] as usize;
        let pair = records[4 + index / 2];
        let (start, count) = if index % 2 == 1 {
            (pair[2] as usize, pair[3] as usize)
        } else {
            (pair[0] as usize, pair[1] as usize)
        };
        let texel = |x: usize, y: usize| -> [f32; 3] {
            let at = y * atlas[0] + x;
            let packed = records[atlas[2] + at / 4][at % 4] as u32;
            [packed & 255, (packed >> 8) & 255, packed >> 16].map(|v| v as f32 / 255.0)
        };
        for at in start..start + count {
            let glyph = shape[2] + records[shape[3] + at / 4][at % 4] as usize * 5;
            let placed = Placed {
                map: records[glyph],
                shift: [records[glyph + 1][0], records[glyph + 1][1]],
                clip: records[glyph + 2],
                uv: records[glyph + 3],
                color: records[glyph + 4],
                reach: [0.0; 4],
            };
            let Some(f) = placed.fraction(uv) else {
                continue;
            };
            let size = [atlas[0] as f32, atlas[1] as f32];
            let a: [f32; 2] = std::array::from_fn(|k| {
                let half = 0.5 / size[k];
                let low = (placed.uv[k] + half).min(placed.uv[k + 2]);
                let high = (placed.uv[k + 2] - half).max(low);
                (placed.uv[k] + (placed.uv[k + 2] - placed.uv[k]) * f[k]).clamp(low, high)
            });
            let p = [a[0] * size[0] - 0.5, a[1] * size[1] - 0.5];
            let start = p.map(f32::floor);
            let t = [p[0] - start[0], p[1] - start[1]];
            let clamp = |value: f32, k: usize| value.clamp(0.0, size[k] - 1.0) as usize;
            let (x0, y0) = (clamp(start[0], 0), clamp(start[1], 1));
            let (x1, y1) = (clamp(start[0] + 1.0, 0), clamp(start[1] + 1.0, 1));
            let mix = |a: [f32; 3], b: [f32; 3], t: f32| -> [f32; 3] {
                std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
            };
            let sampled = mix(
                mix(texel(x0, y0), texel(x1, y0), t[0]),
                mix(texel(x0, y1), texel(x1, y1), t[0]),
                t[1],
            );
            let coverage = if atlas[3] == 3 {
                let median = sampled[0]
                    .min(sampled[1])
                    .max(sampled[0].max(sampled[1]).min(sampled[2]));
                if median >= 0.5 { 1.0 } else { 0.0 }
            } else {
                sampled[0]
            };
            color = over(color, placed.color, coverage * placed.color[3]);
        }
        color
    }

    fn agree(text: &TextContent<'_>, from: [f32; 2], to: [f32; 2], steps: usize) -> usize {
        let records = text.records().unwrap().records;
        let reference = text.sampler();
        let mut inked = 0;
        for row in 0..steps {
            for column in 0..steps {
                let uv = [
                    from[0] + (to[0] - from[0]) * (column as f32 + 0.37) / steps as f32,
                    from[1] + (to[1] - from[1]) * (row as f32 + 0.61) / steps as f32,
                ];
                let ours = evaluate(&records, uv);
                let theirs = reference(uv);
                for k in 0..4 {
                    assert!(
                        (ours[k] - theirs[k]).abs() < 1e-5,
                        "{uv:?}: {ours:?} against {theirs:?}"
                    );
                }
                if ours[3] > 0.0 {
                    inked += 1;
                }
            }
        }
        inked
    }

    #[test]
    fn lettering_records_find_every_glyph_through_the_grid() {
        let (atlas, mut glyphs) = laid("Lettering at any zoom", 0.05, UNITS_PER_METRE, [0.1, 0.2]);
        let pivot = glyphs[3].rect;
        glyphs[3].turn = [pivot[0] + pivot[2] * 0.5, pivot[1] + pivot[3] * 0.5, 0.4];
        glyphs[5].clip = [-1e9, -1e9, glyphs[5].rect[0] + glyphs[5].rect[2] * 0.5, 1e9];
        glyphs[7].color = [0.2, 0.9, 0.4, 0.5];
        let lettering = lettering(&atlas, &glyphs).unwrap();
        let shown = lettering.shown();
        let records = shown.records().unwrap();
        let sample = lettering.sampler();
        assert_eq!(records.records[3], [0.0; 4]);
        assert_eq!(
            records.width as f32,
            (lettering.rect[2] * lettering.density).round()
        );
        let inked = agree(&shown, [-0.02, -0.03], [1.02, 1.03], 300);
        assert!(inked > 3000, "{inked} inked samples");
        let values: Vec<[f32; 4]> = (0..2000)
            .map(|k| {
                let t = k as f32 / 2000.0;
                sample([
                    lettering.rect[0] + lettering.rect[2] * t,
                    lettering.rect[1] + lettering.rect[3] * 0.6,
                ])
            })
            .collect();
        assert!(
            values.iter().all(|v| [0.0, 0.5, 0.75, 1.0]
                .iter()
                .any(|a| (v[3] - a).abs() < 1e-6)),
            "binary coverage"
        );
    }

    #[test]
    fn lettering_matches_a_fine_print_inside_the_glyphs() {
        let (atlas, glyphs) = laid("Bold", 0.2, UNITS_PER_METRE, [0.0; 2]);
        let lettering = lettering(&atlas, &glyphs).unwrap();
        let print = print(&atlas, &glyphs, 8000.0).unwrap();
        let sample = lettering.sampler();
        let [x, y, w, h] = print.rect;
        let (mut both, mut either) = (0, 0);
        for row in 0..print.height {
            for column in 0..print.width {
                let printed = print.texels[(row * print.width + column) as usize][3] >= 128;
                let point = [
                    x + w * (column as f32 + 0.5) / print.width as f32,
                    y + h * (row as f32 + 0.5) / print.height as f32,
                ];
                let lettered = sample(point)[3] >= 0.5;
                both += usize::from(printed && lettered);
                either += usize::from(printed || lettered);
            }
        }
        let overlap = both as f64 / either as f64;
        assert!(both > 10_000, "{both} texels inside");
        assert!(overlap > 0.995, "IoU {overlap:.4}");
        let content = lettering.content();
        assert!(content.cutout);
        assert!(matches!(content.kind, ContentKind::Text(_)));
        assert_eq!(
            lettering.carrier().positions[0],
            [lettering.rect[0], lettering.rect[1], 0.0]
        );
    }

    #[test]
    fn text_over_a_picture_lands_where_the_print_onto_it_does() {
        let cube = Cube::new();
        let part = cube.mesh();
        let fit = fit(&part, IDENTITY, ON_FRONT).unwrap();
        let (atlas, glyphs) = laid("ab", 0.3, UNITS_PER_METRE, [-0.3, -0.15]);
        let grey = vec![[128, 128, 128, 255]; 128 * 128];
        let picture = PlacementContent {
            uv_offset: [0.1, -0.05],
            uv_scale: [0.9, 1.1],
            ..PlacementContent::new(Image {
                width: 128,
                height: 128,
                texels: &grey,
                srgb: true,
            })
        };
        let lettering = lettering(&atlas, &glyphs).unwrap();
        let content = fit.over(&picture, &lettering).unwrap();
        assert_eq!(content.uv_offset, picture.uv_offset);
        assert_eq!(content.uv_scale, picture.uv_scale);
        let ContentKind::Text(text) = content.kind else {
            panic!("text over a picture is text content");
        };
        let inked = agree(&text, [0.0; 2], [1.0; 2], 200);
        assert_eq!(inked, 40_000, "the picture shows everywhere");
        let composed = fit.onto(&picture, &atlas, &glyphs).unwrap();
        let reference = text.sampler();
        let mut sum = [[0.0f64; 3]; 2];
        for row in 0..128u32 {
            for column in 0..128u32 {
                let uv = [(column as f32 + 0.5) / 128.0, (row as f32 + 0.5) / 128.0];
                let printed = composed[(row * 128 + column) as usize][0] > 200;
                let ours = reference(uv);
                for (k, weight) in [
                    (0, f64::from(u8::from(printed))),
                    (1, f64::from(ours[0] > 0.5)),
                ] {
                    sum[k][0] += weight * f64::from(column);
                    sum[k][1] += weight * f64::from(row);
                    sum[k][2] += weight;
                }
            }
        }
        assert!(sum[0][2] > 200.0 && sum[1][2] > 200.0, "{sum:?}");
        for axis in 0..2 {
            let apart = sum[0][axis] / sum[0][2] - sum[1][axis] / sum[1][2];
            assert!(apart.abs() < 0.5, "{apart} texels apart on axis {axis}");
        }
        let words = lettering.content();
        assert!(fit.over(&words, &lettering).is_err());
        assert!(fit.onto(&words, &atlas, &glyphs).is_err());
    }

    #[test]
    fn a_fit_puts_lettering_where_it_puts_the_print() {
        let cube = Cube::new();
        let part = cube.mesh();
        let fit = fit(&part, IDENTITY, ON_FRONT).unwrap();
        let (atlas, glyphs) = laid("on", 0.08, UNITS_PER_METRE, [-0.1, -0.05]);
        let lettering = lettering(&atlas, &glyphs).unwrap();
        let content = fit.content(&lettering);
        assert!(!content.cutout);
        let [x, y, w, h] = lettering.rect;
        for layout in [[x, y], [x + w, y + h], [x + 0.3 * w, y + 0.6 * h]] {
            let uv = [layout[0] + 0.5, -layout[1] + 0.5];
            let shown: [f32; 2] =
                std::array::from_fn(|k| uv[k] * content.uv_scale[k] + content.uv_offset[k]);
            let want = [(layout[0] - x) / w, (layout[1] - y) / h];
            for k in 0..2 {
                assert!(
                    (shown[k] - want[k]).abs() < 1e-4,
                    "{shown:?} against {want:?}"
                );
            }
        }
        assert_eq!(fit.elsewhere(&part, &content).len(), 10);
    }

    #[test]
    fn broken_lettering_is_refused() {
        let (atlas, glyphs) = laid("x", 0.1, UNITS_PER_METRE, [0.0; 2]);
        assert!(lettering(&atlas, &[]).is_err());
        let four = Atlas {
            channels: 4,
            ..atlas.clone()
        };
        assert!(lettering(&four, &glyphs).is_err());
        let hidden = Glyph {
            color: [1.0, 1.0, 1.0, 0.0],
            ..glyphs[0]
        };
        assert!(lettering(&atlas, &[hidden]).is_err());
        let lettering = lettering(&atlas, &glyphs).unwrap();
        let flat = TextContent {
            scale: [0.0, 1.0],
            ..lettering.shown()
        };
        assert!(!flat.valid());
        assert!(flat.records().is_err());
    }

    #[test]
    fn staged_lettering_rides_its_own_slot_beside_images() {
        use crate::gpu::{Camera, Sun};
        use crate::stage::{Placement, Stage};
        let (atlas, glyphs) = laid("slot", 0.1, UNITS_PER_METRE, [0.0; 2]);
        let lettering = lettering(&atlas, &glyphs).unwrap();
        let carrier = lettering.carrier();
        let red = [[255, 0, 0, 255]; 4];
        let image = Image {
            width: 2,
            height: 2,
            texels: &red,
            srgb: true,
        };
        let meshes = [carrier.mesh()];
        let placements = [
            Placement {
                content: Some(lettering.content()),
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                content: Some(PlacementContent::new(image)),
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                content: Some(lettering.content()),
                ..Placement::new(0, IDENTITY, 0)
            },
        ];
        let materials = [material(false)];
        let staged = Stage {
            meshes: &meshes,
            instances: &placements,
            materials: &materials,
            sky: pfx_load::Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 0.0,
            },
            camera: Camera {
                origin: [0.0, 0.0, 1.0],
                forward: [0.0, 0.0, -1.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
            },
            projection: crate::gpu::Projection::Perspective,
            lens: crate::detail::Lens::default(),
        }
        .build()
        .unwrap();
        let detail = &staged.detail;
        let slots: Vec<i32> = detail
            .instances
            .iter()
            .map(|instance| instance.surface.layer.slot)
            .collect();
        assert_eq!(slots, [0, 1, 0]);
        assert!(detail.instances[0].surface.cutout);
        assert_eq!(detail.content_slots.len(), 2);
        assert_eq!(detail.text_slots.len(), 2);
        assert!(detail.content_slots[0].is_none() && detail.content_slots[1].is_some());
        assert!(detail.text_slots[1].is_none());
        let text = detail.text_slots[0].as_ref().unwrap();
        assert_eq!(*text, lettering.shown().records().unwrap());
        let buffers = crate::gpu::SceneBuffers::build(&[], &[], &materials, detail).unwrap();
        let info: &[u32] = bytemuck::cast_slice(&buffers.content_info);
        assert_eq!(&info[..4], [4, text.width, text.height, 1]);
        assert_eq!(&info[4..], [0, 2, 2, 0]);
        assert_eq!(buffers.content.len(), (4 + text.records.len()) * 16);
    }
}
