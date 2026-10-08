use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use cosmic_text::{
    Align, Attrs, Buffer, CacheKeyFlags, Cursor, Family, FeatureTag, FontFeatures, FontSystem,
    Metrics, Shaping, SubpixelBin, Wrap, fontdb,
};
use swash::scale::{Render, ScaleContext, Source, StrikeWith, image::Content};
use swash::zeno::{Angle, Command, PathData, Transform, Vector};

mod atlas;
#[cfg(test)]
mod atlas_tests;
mod fallback;
#[cfg(test)]
mod fallback_tests;
mod field;
#[cfg(test)]
mod field_reference;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
mod number;
mod prefill;
mod print;
#[cfg(test)]
mod quality;
#[cfg(feature = "subset")]
pub mod subset;
#[cfg(test)]
mod warm_tests;

pub use atlas::{
    AtlasChanges, AtlasStats, CellUpload, DEFAULT_ATLAS_BUDGET, DEFAULT_ATLAS_PAGE, PageInfo,
    PageKind,
};
pub use fallback::{FaceMetrics, Fallback, Pick, matched_scale};
pub use number::{Figures, Ligatures, NumberForms};
pub use prefill::Prefill;
pub use print::{Caret, Icon, PrintBlock, PrintImage, Printed, Selection};

use atlas::{CellRef, SharedAtlas};

pub const ATLAS_PADDING: u32 = 8;
pub const ATLAS_CELL_ALIGN: u32 = 8;
pub const MSDF_FIELD: f32 = 64.0;
pub const FINE_MSDF_FIELD: f32 = 128.0;
pub const FINE_MSDF_FROM: f32 = 120.0;
pub const MSDF_SPREAD: f32 = 8.0;

pub const MSDF_WGSL: &str = r#"
fn msdf_median(rgb: vec3<f32>) -> f32 {
    return max(min(rgb.r, rgb.g), min(max(rgb.r, rgb.g), rgb.b));
}
fn msdf_coverage(rgb: vec3<f32>, uv: vec2<f32>, texture_size: vec2<f32>) -> f32 {
    let signed_distance = msdf_median(rgb) - 0.5;
    let screen_range = 4.0 * 0.5 * dot(vec2<f32>(1.0) / fwidth(uv), vec2<f32>(1.0) / texture_size);
    return clamp(0.5 + signed_distance * screen_range, 0.0, 1.0);
}
fn sample_msdf(atlas: texture_2d<f32>, atlas_sampler: sampler, uv: vec2<f32>) -> f32 {
    let rgb = textureSample(atlas, atlas_sampler, uv).rgb;
    return msdf_coverage(rgb, uv, vec2<f32>(textureDimensions(atlas)));
}
"#;

pub fn msdf_coverage(rgb: [f32; 3], screen_range: f32) -> f32 {
    let median = (rgb[0].min(rgb[1])).max(rgb[0].max(rgb[1]).min(rgb[2]));
    (0.5 + (median - 0.5) * screen_range).clamp(0.0, 1.0)
}

#[derive(Clone, Debug)]
pub enum Baseline {
    Straight {
        origin: [f32; 2],
        direction: [f32; 2],
    },
    Arc {
        center: [f32; 2],
        radius: f32,
        start_angle: f32,
        clockwise: bool,
    },
    Polyline(Vec<[f32; 2]>),
}

impl Baseline {
    pub fn sample(&self, distance: f32, normal_offset: f32) -> Option<([f32; 2], f32)> {
        let (point, tangent) = match self {
            Self::Straight { origin, direction } => {
                let len = direction[0].hypot(direction[1]);
                if len <= 0.0 {
                    return None;
                }
                let t = [direction[0] / len, direction[1] / len];
                (
                    [origin[0] + distance * t[0], origin[1] + distance * t[1]],
                    t,
                )
            }
            Self::Arc {
                center,
                radius,
                start_angle,
                clockwise,
            } => {
                if *radius <= 0.0 {
                    return None;
                }
                let sign = if *clockwise { -1.0 } else { 1.0 };
                let a = start_angle + sign * distance / radius;
                (
                    [center[0] + radius * a.cos(), center[1] + radius * a.sin()],
                    [-sign * a.sin(), sign * a.cos()],
                )
            }
            Self::Polyline(points) => {
                let segments: Vec<_> = points
                    .windows(2)
                    .filter_map(|pair| {
                        let dx = pair[1][0] - pair[0][0];
                        let dy = pair[1][1] - pair[0][1];
                        let len = dx.hypot(dy);
                        (len > 0.0).then_some((pair[0], [dx / len, dy / len], len))
                    })
                    .collect();
                let &(mut origin, mut tangent, _) = segments.first()?;
                let mut remain = distance;
                for &(start, dir, len) in &segments {
                    origin = start;
                    tangent = dir;
                    if remain <= len {
                        break;
                    }
                    remain -= len;
                }
                (
                    [
                        origin[0] + remain * tangent[0],
                        origin[1] + remain * tangent[1],
                    ],
                    tangent,
                )
            }
        };
        Some((
            [
                point[0] - tangent[1] * normal_offset,
                point[1] + tangent[0] * normal_offset,
            ],
            tangent[1].atan2(tangent[0]),
        ))
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Representation {
    Coverage,
    Msdf,
}

#[derive(Clone, Debug)]
pub struct GlyphInstance {
    pub glyph_id: u16,
    pub cluster: std::ops::Range<usize>,
    pub position: [f32; 2],
    pub rotation: f32,
    pub scale: f32,
    pub local_rect: [f32; 4],
    pub atlas_rect: [u32; 4],
    pub uv: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Atlas {
    pub channels: u32,
    pub levels: Vec<MipLevel>,
}

#[derive(Clone, Debug)]
pub struct Paragraph {
    pub glyphs: Vec<GlyphInstance>,
    pub atlas: Atlas,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Style<'a> {
    pub family: &'a str,
    pub size: f32,
    pub line_height: f32,
    pub wrap_width: Option<f32>,
    pub pixels_per_unit: f32,
    pub representation: Representation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub family: String,
    pub size: f32,
    pub line: f32,
    pub weight: u16,
    pub italic: bool,
    pub spacing: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub face: Face,
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Center,
    End,
}

pub struct Block {
    buffer: Buffer,
    scale: f32,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    stamp: u64,
    bytes: u64,
    spans: Vec<usize>,
    lines: Vec<f32>,
    shifts: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct RichGlyphQuad {
    pub key: u64,
    pub line_index: usize,
    pub line_baseline: f32,
    pub pen: [f32; 2],
    pub atlas_page: AtlasPage,
    pub is_color: bool,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub origin: [f32; 2],
    pub alpha: f32,
    pub clip: [f32; 4],
    pub turn: [f32; 3],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasPage {
    Monochrome,
    Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasCell {
    pub page: AtlasPage,
    pub rect: [u32; 4],
}

pub struct RichParagraph {
    pub atlas: Atlas,
    pub color_atlas: Option<Atlas>,
    pub cells: BTreeMap<u64, AtlasCell>,
    pub quads: Vec<RichGlyphQuad>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub scale: f32,
    pub representation: Representation,
    pub origin: [f32; 2],
    pub alpha: f32,
    pub clip: [f32; 4],
    pub turn: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct PageQuad {
    pub page: u32,
    pub quad: RichGlyphQuad,
}

#[derive(Clone, Debug, Default)]
pub struct Placed {
    pub quads: Vec<PageQuad>,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidFont,
    InvalidStyle,
    InvalidPath,
    UnsupportedGlyph,
    AtlasTooLarge,
}

pub struct TextEngine {
    fonts: FontSystem,
    context: ScaleContext,
    blocks: HashMap<u64, Block>,
    rasters: BTreeMap<RasterKey, CachedRaster>,
    atlases: BTreeMap<Vec<RasterKey>, CachedAtlas>,
    shared: SharedAtlas,
    numbers: HashMap<u64, number::NumberFace>,
    fallbacks: fallback::Fallbacks,
    tick: u64,
    frame_start: u64,
    shaping_budget: u64,
    shaping_evicted: u64,
}

pub const DEFAULT_SHAPING_BUDGET: u64 = 8 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapingStats {
    pub blocks: usize,
    pub number_faces: usize,
    pub bytes: u64,
    pub budget: u64,
    pub evicted: u64,
}

struct CachedRaster {
    mono: Option<GlyphImage>,
    color: Option<GlyphImage>,
    used: bool,
}

struct CachedAtlas {
    used: bool,
    mono: Atlas,
    color: Option<Atlas>,
    rects: BTreeMap<RasterKey, AtlasCell>,
    cells: BTreeMap<u64, AtlasCell>,
}

struct PendingGlyph {
    key: RasterKey,
    font_id: fontdb::ID,
    font_weight: fontdb::Weight,
    span_index: usize,
    font_size: f32,
    pixel: [i32; 2],
    pen: [f32; 2],
    line_index: usize,
    line_baseline: f32,
}

impl TextEngine {
    pub fn new(font_bytes: &[u8]) -> Result<Self, Error> {
        let mut db = fontdb::Database::new();
        db.load_font_data(font_bytes.to_vec());
        if db.faces().next().is_none() {
            return Err(Error::InvalidFont);
        }
        Ok(Self {
            fonts: FontSystem::new_with_locale_and_db_and_fallback(
                "en-US".into(),
                db,
                fallback::Explicit,
            ),
            context: ScaleContext::new(),
            blocks: HashMap::new(),
            rasters: BTreeMap::new(),
            atlases: BTreeMap::new(),
            shared: SharedAtlas::new(),
            numbers: HashMap::new(),
            fallbacks: fallback::Fallbacks::default(),
            tick: 0,
            frame_start: 0,
            shaping_budget: DEFAULT_SHAPING_BUDGET,
            shaping_evicted: 0,
        })
    }

    pub fn register_font(&mut self, font_bytes: &[u8]) -> Result<(), Error> {
        let before = self.fonts.db().faces().count();
        self.fonts.db_mut().load_font_data(font_bytes.to_vec());
        if self.fonts.db().faces().count() == before {
            return Err(Error::InvalidFont);
        }
        self.blocks.clear();
        self.rasters.clear();
        self.atlases.clear();
        self.shared.clear();
        self.numbers.clear();
        Ok(())
    }

    pub fn begin_frame(&mut self) {
        self.frame_start = self.tick;
        for raster in self.rasters.values_mut() {
            raster.used = false;
        }
        for atlas in self.atlases.values_mut() {
            atlas.used = false;
        }
        self.shared.begin_frame();
    }

    pub fn end_frame(&mut self) {
        self.rasters.retain(|_, raster| raster.used);
        self.atlases.retain(|_, atlas| atlas.used);
        let mut bytes = self.shaping_bytes();
        if bytes <= self.shaping_budget {
            return;
        }
        let start = self.frame_start;
        let mut idle = self
            .blocks
            .iter()
            .filter(|(_, block)| block.stamp <= start)
            .map(|(&key, block)| (block.stamp, false, key))
            .chain(
                self.numbers
                    .iter()
                    .filter(|(_, face)| face.stamp <= start)
                    .map(|(&key, face)| (face.stamp, true, key)),
            )
            .collect::<Vec<_>>();
        idle.sort_unstable();
        for (_, number, key) in idle {
            if bytes <= self.shaping_budget {
                break;
            }
            let freed = if number {
                self.numbers.remove(&key).map_or(0, |face| face.bytes)
            } else {
                self.blocks.remove(&key).map_or(0, |block| block.bytes)
            };
            bytes -= freed;
            self.shaping_evicted += 1;
        }
    }

    pub fn set_shaping_budget(&mut self, bytes: u64) {
        self.shaping_budget = bytes;
    }

    pub fn shaping_stats(&self) -> ShapingStats {
        ShapingStats {
            blocks: self.blocks.len(),
            number_faces: self.numbers.len(),
            bytes: self.shaping_bytes(),
            budget: self.shaping_budget,
            evicted: self.shaping_evicted,
        }
    }

    fn shaping_bytes(&self) -> u64 {
        self.blocks.values().map(|block| block.bytes).sum::<u64>()
            + self.numbers.values().map(|face| face.bytes).sum::<u64>()
    }

    fn stamp(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    pub fn cached_atlases(&self) -> usize {
        self.atlases.len()
    }

    pub fn set_atlas_budget(&mut self, bytes: u64) {
        self.shared.set_budget(bytes);
    }

    pub fn set_atlas_page(&mut self, size: u32) -> Result<(), Error> {
        self.shared.set_page_size(size)
    }

    pub fn atlas_stats(&self) -> AtlasStats {
        self.shared.stats()
    }

    pub fn take_atlas_changes(&mut self) -> AtlasChanges {
        self.shared.take_changes()
    }

    pub fn atlas_page(&self, id: u32) -> Option<&Atlas> {
        self.shared.page(id)
    }

    pub fn cached_blocks(&self) -> usize {
        self.blocks.len()
    }

    pub fn cached_rasters(&self) -> usize {
        self.rasters.len()
    }

    pub fn layout_spans(
        &mut self,
        spans: &[Span],
        width: Option<f32>,
        scale: f32,
        anchor: Anchor,
    ) -> Result<&Block, Error> {
        let key = self.layout_block(spans, width, scale, anchor, None, None)?;
        Ok(&self.blocks[&key])
    }

    fn layout_block(
        &mut self,
        spans: &[Span],
        width: Option<f32>,
        scale: f32,
        anchor: Anchor,
        forms: Option<NumberForms>,
        columns: Option<number::Columns>,
    ) -> Result<u64, Error> {
        if !scale.is_finite()
            || scale <= 0.0
            || width.is_some_and(|w| !w.is_finite() || w <= 0.0)
            || spans.iter().any(|span| {
                let face = &span.face;
                !face.size.is_finite()
                    || face.size <= 0.0
                    || !face.line.is_finite()
                    || face.line <= 0.0
                    || !face.spacing.is_finite()
                    || span.color.iter().any(|c| !c.is_finite())
            })
        {
            return Err(Error::InvalidStyle);
        }
        let key = span_key(spans, width, scale, anchor, forms);
        let stamp = self.stamp();
        if let Some(block) = self.blocks.get_mut(&key) {
            block.stamp = stamp;
            return Ok(key);
        }
        self.shared.stats.shaped += 1;
        let default_face = Face {
            family: "serif".into(),
            size: 16.0,
            line: 22.0,
            weight: 400,
            italic: false,
            spacing: 0.0,
        };
        let first = spans.first().map_or(&default_face, |span| &span.face);
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(first.size * scale, first.line * scale),
        );
        buffer.set_wrap(
            &mut self.fonts,
            if width.is_some() {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            },
        );
        buffer.set_size(&mut self.fonts, width.map(|w| w * scale), None);
        let default = face_attrs(first, scale, 0, forms);
        let align = match anchor {
            Anchor::Start => None,
            Anchor::Center => Some(Align::Center),
            Anchor::End => Some(Align::End),
        };
        let laid = self.split(spans);
        let attrs = laid.attrs(scale, forms);
        buffer.set_rich_text(
            &mut self.fonts,
            laid.texts
                .iter()
                .zip(attrs)
                .map(|((span, range), attrs)| (&spans[*span].text[range.clone()], attrs)),
            &default,
            Shaping::Advanced,
            align,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let lines = self.line_baselines(buffer.layout_runs(), &laid.pieces);
        let mut w = 0.0f32;
        let mut h = 0.0f32;
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        let baseline = lines.first().copied();
        let (shifts, w) = match (columns, spans.first()) {
            (Some(columns), Some(span)) => self.column_shifts(
                &buffer,
                columns,
                &span.face,
                scale,
                forms.unwrap_or_default(),
            ),
            _ => (Vec::new(), w),
        };
        let bytes = block_bytes(&buffer, spans.len(), lines.len(), shifts.len());
        self.blocks.insert(
            key,
            Block {
                buffer,
                scale,
                width: w / scale,
                height: h / scale,
                baseline: baseline.unwrap_or(h) / scale,
                stamp,
                bytes,
                spans: laid.pieces.iter().map(|piece| piece.span).collect(),
                lines,
                shifts,
            },
        );
        Ok(key)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_spans(
        &mut self,
        spans: &[Span],
        width: Option<f32>,
        scale: f32,
        anchor: Anchor,
        representation: Representation,
        origin: [f32; 2],
        alpha: f32,
        clip: [f32; 4],
        turn: [f32; 3],
    ) -> Result<RichParagraph, Error> {
        let key = self.layout_block(spans, width, scale, anchor, None, None)?;
        let block = &self.blocks[&key];
        let mut pending = Vec::new();
        for (run, &line_y) in block.buffer.layout_runs().zip(&block.lines) {
            for glyph in run.glyphs {
                let span_index = block.span_of(glyph.metadata);
                let ppem = match representation {
                    Representation::Coverage => glyph.font_size,
                    Representation::Msdf => 64.0,
                };
                if !(1.0..=4096.0).contains(&ppem) {
                    return Err(Error::InvalidStyle);
                }
                let physical = glyph.physical((origin[0] * scale, origin[1] * scale), 1.0);
                let pen_x =
                    (glyph.x + glyph.x_offset * glyph.font_size).mul_add(1.0, origin[0] * scale);
                let pen_y = glyph.y - glyph.y_offset * glyph.font_size + origin[1] * scale + line_y;
                let font = self
                    .fonts
                    .get_font(glyph.font_id, glyph.font_weight)
                    .ok_or(Error::UnsupportedGlyph)?;
                let flags = raster_flags(
                    glyph.cache_key_flags,
                    spans.get(span_index).is_some_and(|span| span.face.italic),
                    self.fonts.db().face(glyph.font_id).map(|face| face.style),
                );
                let key = raster_key(
                    glyph.font_id,
                    glyph.glyph_id,
                    ppem,
                    glyph.font_weight,
                    glyph.font_size / scale,
                    flags,
                    font.as_swash(),
                );
                pending.push(PendingGlyph {
                    key: RasterKey {
                        representation,
                        x_bin: physical.cache_key.x_bin,
                        display_size: glyph.font_size.to_bits(),
                        ..key
                    },
                    font_id: glyph.font_id,
                    font_weight: glyph.font_weight,
                    span_index,
                    font_size: glyph.font_size,
                    pixel: [physical.x, pen_y.trunc() as i32],
                    pen: [pen_x / scale, pen_y / scale],
                    line_index: run.line_i,
                    line_baseline: line_y / scale + origin[1],
                });
            }
        }
        let atlas_key = pending
            .iter()
            .map(|glyph| glyph.key.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for key in &atlas_key {
            if let Some(raster) = self.rasters.get_mut(key) {
                raster.used = true;
            } else {
                let glyph = pending.iter().find(|glyph| &glyph.key == key).unwrap();
                let font = self
                    .fonts
                    .get_font(glyph.font_id, glyph.font_weight)
                    .ok_or(Error::UnsupportedGlyph)?;
                let ppem = f32::from_bits(key.ppem);
                let mut scaler = self
                    .context
                    .builder(font.as_swash())
                    .size(ppem)
                    .hint(representation == Representation::Coverage && !key.disable_hinting)
                    .variations(key.variations())
                    .build();
                let source_offset = match representation {
                    Representation::Coverage => 1.0,
                    Representation::Msdf => 64.0 / f32::from_bits(key.display_size),
                };
                let offset = [key.x_bin.as_float() * source_offset, 0.0];
                let color = color_image(&mut scaler, key.glyph_id, key.fake_italic, offset);
                let mono = if color.is_none() {
                    match representation {
                        Representation::Coverage => {
                            coverage_image(&mut scaler, key.glyph_id, key.fake_italic, offset)
                        }
                        Representation::Msdf => {
                            msdf_image(&mut scaler, key.glyph_id, key.fake_italic, offset)
                        }
                    }
                } else {
                    None
                };
                self.rasters.insert(
                    key.clone(),
                    CachedRaster {
                        mono,
                        color,
                        used: true,
                    },
                );
            }
        }
        if let Some(cached) = self.atlases.get_mut(&atlas_key) {
            cached.used = true;
        } else {
            let images = atlas_key
                .iter()
                .map(|key| (key.clone(), self.rasters[key].mono.clone()))
                .collect::<BTreeMap<_, _>>();
            let color_images = atlas_key
                .iter()
                .map(|key| (key.clone(), self.rasters[key].color.clone()))
                .collect::<BTreeMap<_, _>>();
            let channels = if representation == Representation::Coverage {
                1
            } else {
                3
            };
            let (mono, mono_rects) = pack(&images, channels, Some(4))?;
            let (color, color_rects) = if color_images.values().any(Option::is_some) {
                let (atlas, rects) = pack(&color_images, 4, Some(4))?;
                (Some(atlas), rects)
            } else {
                (None, BTreeMap::new())
            };
            let mut rects = BTreeMap::new();
            let mut cells = BTreeMap::new();
            for key in &atlas_key {
                let cell = if let Some(&rect) = color_rects.get(key) {
                    AtlasCell {
                        page: AtlasPage::Color,
                        rect,
                    }
                } else if let Some(&rect) = mono_rects.get(key) {
                    AtlasCell {
                        page: AtlasPage::Monochrome,
                        rect,
                    }
                } else {
                    continue;
                };
                rects.insert(key.clone(), cell);
                cells.insert(key.cell_key(), cell);
            }
            self.atlases.insert(
                atlas_key.clone(),
                CachedAtlas {
                    used: true,
                    mono,
                    color,
                    rects,
                    cells,
                },
            );
        }
        let cached = &self.atlases[&atlas_key];
        let mut quads = Vec::with_capacity(pending.len());
        for glyph in pending {
            let Some(&cell) = cached.rects.get(&glyph.key) else {
                continue;
            };
            let raster = &self.rasters[&glyph.key];
            let (image, target_atlas) = match cell.page {
                AtlasPage::Monochrome => (raster.mono.as_ref().unwrap(), &cached.mono),
                AtlasPage::Color => (
                    raster.color.as_ref().unwrap(),
                    cached.color.as_ref().unwrap(),
                ),
            };
            let rect = cell.rect;
            let factor = match representation {
                Representation::Coverage => 1.0 / scale,
                Representation::Msdf => glyph.font_size / (64.0 * scale),
            };
            let color = spans
                .get(glyph.span_index)
                .map_or([0.0, 0.0, 0.0, 1.0], |s| s.color);
            quads.push(RichGlyphQuad {
                key: glyph.key.cell_key(),
                line_index: glyph.line_index,
                line_baseline: glyph.line_baseline,
                pen: glyph.pen,
                atlas_page: cell.page,
                is_color: cell.page == AtlasPage::Color,
                rect: [
                    glyph.pixel[0] as f32 / scale + image.left * factor,
                    glyph.pixel[1] as f32 / scale + image.top * factor,
                    image.width as f32 * factor,
                    image.height as f32 * factor,
                ],
                uv: [
                    rect[0] as f32 / target_atlas.levels[0].width as f32,
                    rect[1] as f32 / target_atlas.levels[0].height as f32,
                    (rect[0] + rect[2]) as f32 / target_atlas.levels[0].width as f32,
                    (rect[1] + rect[3]) as f32 / target_atlas.levels[0].height as f32,
                ],
                origin,
                alpha,
                clip,
                turn,
                color: [color[0], color[1], color[2], color[3] * alpha],
            });
        }
        Ok(RichParagraph {
            atlas: cached.mono.clone(),
            color_atlas: cached.color.clone(),
            cells: cached.cells.clone(),
            quads,
        })
    }

    pub fn place_spans(
        &mut self,
        spans: &[Span],
        width: Option<f32>,
        anchor: Anchor,
        placement: &Placement,
    ) -> Result<Placed, Error> {
        let key = self.layout_block(spans, width, placement.scale, anchor, None, None)?;
        if width.is_none() && anchor != Anchor::Start {
            let placement = Placement {
                origin: number::anchored(placement.origin, self.blocks[&key].width, anchor),
                ..*placement
            };
            return self.place_block(key, spans, &placement);
        }
        self.place_block(key, spans, placement)
    }

    fn place_block(
        &mut self,
        key: u64,
        spans: &[Span],
        placement: &Placement,
    ) -> Result<Placed, Error> {
        let scale = placement.scale;
        let block = &self.blocks[&key];
        let (width, height, baseline) = (block.width, block.height, block.baseline);
        let spots = self.block_spots(key, spans);
        let mut quads = Vec::with_capacity(spots.len());
        for spot in &spots {
            let pen = Pen::new(spot, placement);
            let key = self.spot_key(spot, placement.representation, scale)?;
            let key = match placement.representation {
                Representation::Coverage => RasterKey {
                    x_bin: pen.bin,
                    ..key
                },
                Representation::Msdf => key,
            };
            self.place_glyph(spot, &key, pen, placement, &mut quads)?;
        }
        Ok(Placed {
            quads,
            width,
            height,
            baseline,
        })
    }

    fn block_spots(&self, key: u64, spans: &[Span]) -> Vec<Spot> {
        let block = &self.blocks[&key];
        let mut spots = Vec::new();
        let mut shifts = block.shifts.iter();
        for (run, &line_y) in block.buffer.layout_runs().zip(&block.lines) {
            for glyph in run.glyphs {
                let shift = shifts.next().copied().unwrap_or_default();
                let span_index = block.span_of(glyph.metadata);
                let flags = raster_flags(
                    glyph.cache_key_flags,
                    spans.get(span_index).is_some_and(|span| span.face.italic),
                    self.fonts.db().face(glyph.font_id).map(|face| face.style),
                );
                spots.push(Spot {
                    font_id: glyph.font_id,
                    font_weight: glyph.font_weight,
                    glyph_id: glyph.glyph_id,
                    flags,
                    font_size: glyph.font_size,
                    x: glyph.x + glyph.x_offset * glyph.font_size + shift,
                    y: glyph.y - glyph.y_offset * glyph.font_size,
                    line_y,
                    line_index: run.line_i,
                    color: spans
                        .get(span_index)
                        .map_or([0.0, 0.0, 0.0, 1.0], |span| span.color),
                });
            }
        }
        spots
    }

    fn spot_key(
        &mut self,
        spot: &Spot,
        representation: Representation,
        scale: f32,
    ) -> Result<RasterKey, Error> {
        let ppem = match representation {
            Representation::Coverage => spot.font_size,
            Representation::Msdf => field_ppem(spot.font_size),
        };
        if !(1.0..=4096.0).contains(&ppem) {
            return Err(Error::InvalidStyle);
        }
        let font = self
            .fonts
            .get_font(spot.font_id, spot.font_weight)
            .ok_or(Error::UnsupportedGlyph)?;
        Ok(RasterKey {
            representation,
            ..raster_key(
                spot.font_id,
                spot.glyph_id,
                ppem,
                spot.font_weight,
                spot.font_size / scale,
                spot.flags,
                font.as_swash(),
            )
        })
    }

    fn place_glyph(
        &mut self,
        spot: &Spot,
        key: &RasterKey,
        pen: Pen,
        placement: &Placement,
        quads: &mut Vec<PageQuad>,
    ) -> Result<(), Error> {
        let cell = match self.shared.touch(key) {
            Some(cell) => cell,
            None => {
                let (kind, image) = self.shared_raster(key, spot.font_id, spot.font_weight)?;
                self.shared.insert(key.clone(), kind, image)?
            }
        };
        let Some(cell) = cell else {
            return Ok(());
        };
        quads.push(PageQuad {
            page: cell.page,
            quad: glyph_quad(spot, cell, key.cell_key(), pen, placement),
        });
        Ok(())
    }

    fn shared_raster(
        &mut self,
        key: &RasterKey,
        font_id: fontdb::ID,
        font_weight: fontdb::Weight,
    ) -> Result<(PageKind, Option<GlyphImage>), Error> {
        let font = self
            .fonts
            .get_font(font_id, font_weight)
            .ok_or(Error::UnsupportedGlyph)?;
        Ok(raster(&mut self.context, &font, key))
    }

    pub fn layout(
        &mut self,
        text: &str,
        style: Style<'_>,
        baseline: &Baseline,
    ) -> Result<Paragraph, Error> {
        if !style.size.is_finite()
            || style.size <= 0.0
            || !style.line_height.is_finite()
            || style.line_height <= 0.0
            || !style.pixels_per_unit.is_finite()
            || style.pixels_per_unit <= 0.0
            || style.wrap_width.is_some_and(|w| !w.is_finite() || w <= 0.0)
        {
            return Err(Error::InvalidStyle);
        }
        if baseline.sample(0.0, 0.0).is_none() {
            return Err(Error::InvalidPath);
        }
        let shape_scale = 1024.0;
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(style.size * shape_scale, style.line_height * shape_scale),
        );
        buffer.set_wrap(
            &mut self.fonts,
            if style.wrap_width.is_some() {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            },
        );
        buffer.set_size(
            &mut self.fonts,
            style.wrap_width.map(|w| w * shape_scale),
            None,
        );
        buffer.set_text(
            &mut self.fonts,
            text,
            &Attrs::new().family(Family::Name(style.family)),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let first_line_y = buffer.layout_runs().next().map_or(0.0, |r| r.line_y);
        let mut pending = Vec::new();
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w / shape_scale);
            height = height.max((run.line_top + run.line_height) / shape_scale);
            for glyph in run.glyphs {
                let x = (glyph.x + glyph.x_offset * glyph.font_size) / shape_scale;
                let y = (run.line_y - first_line_y + glyph.y - glyph.y_offset * glyph.font_size)
                    / shape_scale;
                let (position, rotation) = baseline.sample(x, y).ok_or(Error::InvalidPath)?;
                pending.push((
                    glyph.font_id,
                    glyph.glyph_id,
                    glyph.font_weight,
                    glyph.start..glyph.end,
                    position,
                    rotation,
                ));
            }
        }
        drop(buffer);
        let ppem = match style.representation {
            Representation::Coverage => style.size * style.pixels_per_unit,
            Representation::Msdf => 64.0,
        };
        if !(1.0..=4096.0).contains(&ppem) {
            return Err(Error::InvalidStyle);
        }
        let mut images = BTreeMap::new();
        for (font_id, glyph_id, weight, _, _, _) in &pending {
            let key = (format!("{font_id:?}"), *glyph_id);
            if images.contains_key(&key) {
                continue;
            }
            let font = self
                .fonts
                .get_font(*font_id, *weight)
                .ok_or(Error::UnsupportedGlyph)?;
            let mut scaler = self
                .context
                .builder(font.as_swash())
                .size(ppem)
                .hint(false)
                .build();
            let image = match style.representation {
                Representation::Coverage => coverage_image(&mut scaler, *glyph_id, false, [0.0; 2]),
                Representation::Msdf => msdf_image(&mut scaler, *glyph_id, false, [0.0; 2]),
            };
            images.insert(key, image);
        }
        let channels = if style.representation == Representation::Coverage {
            1
        } else {
            3
        };
        let (atlas, rects) = pack(&images, channels, None)?;
        let mut glyphs = Vec::new();
        for (font_id, glyph_id, _, cluster, position, rotation) in pending {
            let key = (format!("{font_id:?}"), glyph_id);
            let Some(image) = images.get(&key).and_then(Option::as_ref) else {
                continue;
            };
            let rect = rects[&key];
            let scale = if style.representation == Representation::Coverage {
                1.0 / style.pixels_per_unit
            } else {
                style.size / 64.0
            };
            glyphs.push(GlyphInstance {
                glyph_id,
                cluster,
                position,
                rotation,
                scale,
                local_rect: [
                    image.left,
                    image.top,
                    image.width as f32,
                    image.height as f32,
                ],
                atlas_rect: rect,
                uv: [
                    rect[0] as f32 / atlas.levels[0].width as f32,
                    rect[1] as f32 / atlas.levels[0].height as f32,
                    (rect[0] + rect[2]) as f32 / atlas.levels[0].width as f32,
                    (rect[1] + rect[3]) as f32 / atlas.levels[0].height as f32,
                ],
            });
        }
        Ok(Paragraph {
            glyphs,
            atlas,
            width,
            height,
        })
    }
}

fn raster(
    context: &mut ScaleContext,
    font: &cosmic_text::Font,
    key: &RasterKey,
) -> (PageKind, Option<GlyphImage>) {
    let representation = key.representation;
    let mut scaler = context
        .builder(font.as_swash())
        .size(f32::from_bits(key.ppem))
        .hint(representation == Representation::Coverage && !key.disable_hinting)
        .variations(key.variations())
        .build();
    let offset = [key.x_bin.as_float(), 0.0];
    if let Some(color) = color_image(&mut scaler, key.glyph_id, key.fake_italic, offset) {
        return (PageKind::Color, Some(color));
    }
    match representation {
        Representation::Coverage => (
            PageKind::Coverage,
            coverage_image(&mut scaler, key.glyph_id, key.fake_italic, offset),
        ),
        Representation::Msdf => (
            PageKind::Msdf,
            msdf_field(&mut scaler, key.glyph_id, key.fake_italic, offset, true),
        ),
    }
}

pub fn field_ppem(display_size: f32) -> f32 {
    if display_size > FINE_MSDF_FROM {
        FINE_MSDF_FIELD
    } else {
        MSDF_FIELD
    }
}

#[derive(Clone, Copy)]
struct Pen {
    x: f32,
    y: f32,
    pixel_x: i32,
    bin: SubpixelBin,
}

impl Pen {
    fn new(spot: &Spot, placement: &Placement) -> Self {
        let x = spot.x.mul_add(1.0, placement.origin[0] * placement.scale);
        let y = spot.y + placement.origin[1] * placement.scale + spot.line_y;
        let (pixel_x, bin) = SubpixelBin::new(x);
        Self { x, y, pixel_x, bin }
    }
}

fn glyph_quad(
    spot: &Spot,
    cell: CellRef,
    key: u64,
    pen: Pen,
    placement: &Placement,
) -> RichGlyphQuad {
    let scale = placement.scale;
    let (x, factor) = match placement.representation {
        Representation::Coverage => (pen.pixel_x as f32 / scale, 1.0 / scale),
        Representation::Msdf => (
            pen.x / scale,
            spot.font_size / (field_ppem(spot.font_size) * scale),
        ),
    };
    let size = cell.size as f32;
    let rect = cell.rect;
    let page = if cell.kind == PageKind::Color {
        AtlasPage::Color
    } else {
        AtlasPage::Monochrome
    };
    let color = spot.color;
    RichGlyphQuad {
        key,
        line_index: spot.line_index,
        line_baseline: spot.line_y / scale + placement.origin[1],
        pen: [pen.x / scale, pen.y / scale],
        atlas_page: page,
        is_color: page == AtlasPage::Color,
        rect: [
            x + cell.left * factor,
            pen.y.trunc() / scale + cell.top * factor,
            rect[2] as f32 * factor,
            rect[3] as f32 * factor,
        ],
        uv: [
            rect[0] as f32 / size,
            rect[1] as f32 / size,
            (rect[0] + rect[2]) as f32 / size,
            (rect[1] + rect[3]) as f32 / size,
        ],
        origin: placement.origin,
        alpha: placement.alpha,
        clip: placement.clip,
        turn: placement.turn,
        color: [color[0], color[1], color[2], color[3] * placement.alpha],
    }
}

#[derive(Clone)]
struct Spot {
    font_id: fontdb::ID,
    font_weight: fontdb::Weight,
    glyph_id: u16,
    flags: CacheKeyFlags,
    font_size: f32,
    x: f32,
    y: f32,
    line_y: f32,
    line_index: usize,
    color: [f32; 4],
}

fn face_attrs(face: &Face, scale: f32, metadata: usize, forms: Option<NumberForms>) -> Attrs<'_> {
    let mut attrs = Attrs::new()
        .family(Family::Name(&face.family))
        .weight(fontdb::Weight(face.weight))
        .metrics(Metrics::new(face.size * scale, face.line * scale))
        .metadata(metadata)
        .letter_spacing(face.spacing);
    if let Some(forms) = forms {
        let mut features = FontFeatures::new();
        if forms.figures == Figures::Tabular {
            features.enable(FeatureTag::new(b"tnum"));
        }
        if forms.ligatures == Ligatures::Off {
            features.disable(FeatureTag::STANDARD_LIGATURES);
            features.disable(FeatureTag::CONTEXTUAL_LIGATURES);
        }
        attrs = attrs.font_features(features);
    }
    if face.italic {
        attrs.style(cosmic_text::Style::Italic)
    } else {
        attrs
    }
}

fn span_key(
    spans: &[Span],
    width: Option<f32>,
    scale: f32,
    anchor: Anchor,
    forms: Option<NumberForms>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for span in spans {
        span.text.hash(&mut hasher);
        span.face.family.hash(&mut hasher);
        span.face.size.to_bits().hash(&mut hasher);
        span.face.line.to_bits().hash(&mut hasher);
        span.face.weight.hash(&mut hasher);
        span.face.italic.hash(&mut hasher);
        span.face.spacing.to_bits().hash(&mut hasher);
        for channel in span.color {
            channel.to_bits().hash(&mut hasher);
        }
    }
    width.map(f32::to_bits).hash(&mut hasher);
    scale.to_bits().hash(&mut hasher);
    anchor.hash(&mut hasher);
    forms.hash(&mut hasher);
    hasher.finish()
}

fn block_bytes(buffer: &Buffer, spans: usize, lines: usize, shifts: usize) -> u64 {
    let text = buffer
        .lines
        .iter()
        .map(|line| line.text().len() + std::mem::size_of::<cosmic_text::BufferLine>())
        .sum::<usize>();
    let glyphs = buffer
        .layout_runs()
        .map(|run| run.glyphs.len())
        .sum::<usize>();
    (std::mem::size_of::<Block>()
        + text
        + glyphs
            * (std::mem::size_of::<cosmic_text::ShapeGlyph>()
                + std::mem::size_of::<cosmic_text::LayoutGlyph>())
        + lines
            * (std::mem::size_of::<cosmic_text::LayoutLine>()
                + std::mem::size_of::<cosmic_text::ShapeLine>()
                + std::mem::size_of::<f32>())
        + spans * (std::mem::size_of::<cosmic_text::AttrsOwned>() + std::mem::size_of::<usize>())
        + shifts * std::mem::size_of::<f32>()) as u64
        * 7
        / 4
}

impl Block {
    fn span_of(&self, metadata: usize) -> usize {
        self.spans.get(metadata).copied().unwrap_or(metadata)
    }

    pub fn line_baselines(&self) -> Vec<f32> {
        self.lines.iter().map(|y| y / self.scale).collect()
    }
}

pub fn hit(block: &Block, x: f32, y: f32) -> Option<usize> {
    let cursor = block.buffer.hit(x * block.scale, y * block.scale)?;
    Some(
        block
            .buffer
            .lines
            .iter()
            .take(cursor.line)
            .map(|line| line.text().len() + 1)
            .sum::<usize>()
            + cursor.index,
    )
}

pub fn cursor_at(block: &Block, offset: usize) -> (usize, usize) {
    let mut left = offset;
    for (line, text) in block.buffer.lines.iter().enumerate() {
        let len = text.text().len();
        if left <= len {
            return (line, left);
        }
        left = left.saturating_sub(len + 1);
    }
    let line = block.buffer.lines.len().saturating_sub(1);
    (
        line,
        block
            .buffer
            .lines
            .last()
            .map_or(0, |text| text.text().len()),
    )
}

pub fn marks(block: &Block, from: usize, to: usize) -> Vec<[f32; 4]> {
    let (start_line, start_index) = cursor_at(block, from.min(to));
    let (end_line, end_index) = cursor_at(block, from.max(to));
    let start = Cursor::new(start_line, start_index);
    let end = Cursor::new(end_line, end_index);
    block
        .buffer
        .layout_runs()
        .filter_map(|run| {
            let (x, width) = run.highlight(start, end)?;
            (width > 0.0).then_some([
                x / block.scale,
                run.line_top / block.scale,
                width / block.scale,
                run.line_height / block.scale,
            ])
        })
        .collect()
}

pub fn caret(block: &Block, offset: usize) -> Option<[f32; 4]> {
    let (line, index) = cursor_at(block, offset);
    let mut end = None;
    for run in block.buffer.layout_runs().filter(|run| run.line_i == line) {
        for glyph in run.glyphs {
            if glyph.start <= index && index < glyph.end {
                let x = if run.rtl { glyph.x + glyph.w } else { glyph.x };
                return Some([
                    x / block.scale,
                    run.line_top / block.scale,
                    0.0,
                    run.line_height / block.scale,
                ]);
            }
        }
        let x = run.glyphs.last().map_or(
            0.0,
            |glyph| {
                if run.rtl { glyph.x } else { glyph.x + glyph.w }
            },
        );
        end = Some([
            x / block.scale,
            run.line_top / block.scale,
            0.0,
            run.line_height / block.scale,
        ]);
    }
    end
}

pub fn baseline(block: &Block) -> f32 {
    block.baseline
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RasterKey {
    font_id: String,
    glyph_id: u16,
    representation: Representation,
    ppem: u32,
    display_size: u32,
    weight: u16,
    wght: Option<u32>,
    opsz: Option<u32>,
    fake_italic: bool,
    disable_hinting: bool,
    x_bin: SubpixelBin,
    y_bin: SubpixelBin,
}

impl RasterKey {
    fn cell_key(&self) -> u64 {
        fn write(hash: &mut u64, bytes: &[u8]) {
            for byte in bytes {
                *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
        }

        let mut hash = 0xcbf29ce484222325;
        write(&mut hash, &(self.font_id.len() as u64).to_le_bytes());
        write(&mut hash, self.font_id.as_bytes());
        write(&mut hash, &self.glyph_id.to_le_bytes());
        write(&mut hash, &[self.representation as u8]);
        write(&mut hash, &self.ppem.to_le_bytes());
        write(&mut hash, &self.display_size.to_le_bytes());
        write(&mut hash, &self.weight.to_le_bytes());
        for value in [self.wght, self.opsz] {
            write(&mut hash, &[u8::from(value.is_some())]);
            write(&mut hash, &value.unwrap_or_default().to_le_bytes());
        }
        write(&mut hash, &[u8::from(self.fake_italic)]);
        write(&mut hash, &[u8::from(self.disable_hinting)]);
        write(&mut hash, &[self.x_bin as u8, self.y_bin as u8]);
        hash
    }

    fn variations(&self) -> impl Iterator<Item = swash::Setting<f32>> {
        [
            self.wght.map(|value| swash::Setting {
                tag: swash::Tag::from_be_bytes(*b"wght"),
                value: f32::from_bits(value),
            }),
            self.opsz.map(|value| swash::Setting {
                tag: swash::Tag::from_be_bytes(*b"opsz"),
                value: f32::from_bits(value),
            }),
        ]
        .into_iter()
        .flatten()
    }
}

fn raster_key(
    font_id: fontdb::ID,
    glyph_id: u16,
    ppem: f32,
    weight: fontdb::Weight,
    size_points: f32,
    flags: CacheKeyFlags,
    font: swash::FontRef<'_>,
) -> RasterKey {
    let variations = font.variations();
    let wght = variations
        .find_by_tag(swash::Tag::from_be_bytes(*b"wght"))
        .map(|axis| {
            f32::from(weight.0)
                .clamp(axis.min_value(), axis.max_value())
                .to_bits()
        });
    let opsz = variations
        .find_by_tag(swash::Tag::from_be_bytes(*b"opsz"))
        .map(|axis| {
            size_points
                .clamp(axis.min_value(), axis.max_value())
                .to_bits()
        });
    RasterKey {
        font_id: format!("{font_id:?}"),
        glyph_id,
        representation: Representation::Coverage,
        ppem: ppem.to_bits(),
        display_size: ppem.to_bits(),
        weight: weight.0,
        wght,
        opsz,
        fake_italic: flags.contains(CacheKeyFlags::FAKE_ITALIC),
        disable_hinting: flags.contains(CacheKeyFlags::DISABLE_HINTING),
        x_bin: SubpixelBin::Zero,
        y_bin: SubpixelBin::Zero,
    }
}

fn raster_flags(flags: CacheKeyFlags, italic: bool, style: Option<fontdb::Style>) -> CacheKeyFlags {
    if italic && style == Some(fontdb::Style::Normal) {
        flags | CacheKeyFlags::FAKE_ITALIC
    } else {
        flags
    }
}

fn oblique_transform(fake_italic: bool) -> Option<Transform> {
    fake_italic.then(|| Transform::skew(Angle::from_degrees(14.0), Angle::from_degrees(0.0)))
}

#[derive(Clone)]
struct GlyphImage {
    left: f32,
    top: f32,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

fn coverage_image(
    scaler: &mut swash::scale::Scaler<'_>,
    glyph_id: u16,
    fake_italic: bool,
    offset: [f32; 2],
) -> Option<GlyphImage> {
    let image = Render::new(&[Source::Outline])
        .transform(oblique_transform(fake_italic))
        .offset(Vector::new(offset[0], -offset[1]))
        .render(scaler, glyph_id)?;
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }
    Some(GlyphImage {
        left: image.placement.left as f32,
        top: -image.placement.top as f32,
        width: image.placement.width,
        height: image.placement.height,
        bytes: image.data,
    })
}

fn color_image(
    scaler: &mut swash::scale::Scaler<'_>,
    glyph_id: u16,
    fake_italic: bool,
    offset: [f32; 2],
) -> Option<GlyphImage> {
    let mut image = Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(StrikeWith::BestFit),
    ])
    .transform(oblique_transform(fake_italic))
    .offset(Vector::new(offset[0], -offset[1]))
    .render(scaler, glyph_id)?;
    if image.content != Content::Color || image.placement.width == 0 || image.placement.height == 0
    {
        return None;
    }
    if matches!(image.source, Source::ColorBitmap(_)) {
        let whole_x = offset[0].floor() as i32;
        let whole_y = offset[1].floor() as i32;
        let fraction_x = offset[0] - whole_x as f32;
        let fraction_y = offset[1] - whole_y as f32;
        image.placement.left += whole_x;
        image.placement.top -= whole_y;
        if fraction_x > 0.0 || fraction_y > 0.0 {
            let width = image.placement.width;
            let height = image.placement.height;
            let shifted_width = width + u32::from(fraction_x > 0.0);
            let shifted_height = height + u32::from(fraction_y > 0.0);
            let mut shifted = vec![0.0f32; (shifted_width * shifted_height * 4) as usize];
            for y in 0..height {
                for x in 0..width {
                    let source = ((y * width + x) * 4) as usize;
                    for dy in 0..=u32::from(fraction_y > 0.0) {
                        for dx in 0..=u32::from(fraction_x > 0.0) {
                            let weight = if dx == 0 {
                                1.0 - fraction_x
                            } else {
                                fraction_x
                            } * if dy == 0 {
                                1.0 - fraction_y
                            } else {
                                fraction_y
                            };
                            let target = (((y + dy) * shifted_width + x + dx) * 4) as usize;
                            for channel in 0..4 {
                                shifted[target + channel] +=
                                    image.data[source + channel] as f32 * weight;
                            }
                        }
                    }
                }
            }
            image.data = shifted
                .into_iter()
                .map(|value| value.round() as u8)
                .collect();
            image.placement.width = shifted_width;
            image.placement.height = shifted_height;
        }
    }
    Some(GlyphImage {
        left: image.placement.left as f32,
        top: -image.placement.top as f32,
        width: image.placement.width,
        height: image.placement.height,
        bytes: image.data,
    })
}

fn msdf_image(
    scaler: &mut swash::scale::Scaler<'_>,
    glyph_id: u16,
    fake_italic: bool,
    offset: [f32; 2],
) -> Option<GlyphImage> {
    msdf_field(scaler, glyph_id, fake_italic, offset, false)
}

type Contours = (Vec<Vec<[f32; 2]>>, [f32; 2], [u32; 2]);

fn msdf_field(
    scaler: &mut swash::scale::Scaler<'_>,
    glyph_id: u16,
    fake_italic: bool,
    offset: [f32; 2],
    sharp: bool,
) -> Option<GlyphImage> {
    let (contours, [left, top], [width, height]) =
        glyph_contours(scaler, glyph_id, fake_italic, offset)?;
    if sharp {
        field::sharp(&contours, [left, top], [width, height])
    } else {
        field::soft(&contours, [left, top], [width, height])
    }
}

fn glyph_contours(
    scaler: &mut swash::scale::Scaler<'_>,
    glyph_id: u16,
    fake_italic: bool,
    offset: [f32; 2],
) -> Option<Contours> {
    let mut outline = scaler.scale_outline(glyph_id)?;
    if let Some(transform) = oblique_transform(fake_italic) {
        outline.transform(&transform);
    }
    if offset != [0.0, 0.0] {
        outline.transform(&Transform::translation(offset[0], -offset[1]));
    }
    let bounds = outline.bounds();
    let left = bounds.min.x.floor() - 5.0;
    let top = -bounds.max.y.ceil() - 5.0;
    let width = (bounds.max.x.ceil() - left + 5.0) as u32;
    let height = (-bounds.min.y.ceil() - top + 5.0) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut points = Vec::new();
    let mut current = [0.0; 2];
    for command in outline.path().commands() {
        match command {
            Command::MoveTo(p) => {
                if points.len() > 1 {
                    contours.push(std::mem::take(&mut points));
                }
                current = [p.x, -p.y];
                points.push(current);
            }
            Command::LineTo(p) => {
                current = [p.x, -p.y];
                points.push(current);
            }
            Command::QuadTo(c, p) => {
                let a = current;
                for i in 1..=16 {
                    let t = i as f32 / 16.0;
                    let u = 1.0 - t;
                    points.push([
                        u * u * a[0] + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * a[1] - 2.0 * u * t * c.y - t * t * p.y,
                    ]);
                }
                current = [p.x, -p.y];
            }
            Command::CurveTo(c, d, p) => {
                let a = current;
                for i in 1..=24 {
                    let t = i as f32 / 24.0;
                    let u = 1.0 - t;
                    points.push([
                        u * u * u * a[0]
                            + 3.0 * u * u * t * c.x
                            + 3.0 * u * t * t * d.x
                            + t * t * t * p.x,
                        u * u * u * a[1]
                            - 3.0 * u * u * t * c.y
                            - 3.0 * u * t * t * d.y
                            - t * t * t * p.y,
                    ]);
                }
                current = [p.x, -p.y];
            }
            Command::Close => {
                if points.len() > 1 {
                    contours.push(std::mem::take(&mut points));
                }
            }
        }
    }
    if points.len() > 1 {
        contours.push(points);
    }
    Some((contours, [left, top], [width, height]))
}

fn pack<K: Ord + Clone>(
    images: &BTreeMap<K, Option<GlyphImage>>,
    channels: u32,
    mip_count: Option<usize>,
) -> Result<(Atlas, BTreeMap<K, [u32; 4]>), Error> {
    let width = 1024u32;
    let padding = ATLAS_PADDING;
    let mut x = 0u32;
    let mut y = 0u32;
    let mut row_height = 0u32;
    let mut rects = BTreeMap::new();
    for (key, image) in images {
        let Some(image) = image else {
            continue;
        };
        let cell_width = (image.width + padding * 2).next_multiple_of(ATLAS_CELL_ALIGN);
        let cell_height = (image.height + padding * 2).next_multiple_of(ATLAS_CELL_ALIGN);
        if cell_width > width {
            return Err(Error::AtlasTooLarge);
        }
        if x + cell_width > width {
            y += row_height;
            x = 0;
            row_height = 0;
        }
        rects.insert(
            key.clone(),
            [x + padding, y + padding, image.width, image.height],
        );
        x += cell_width;
        row_height = row_height.max(cell_height);
    }
    let height = (y + row_height).next_power_of_two().max(ATLAS_CELL_ALIGN);
    if height > 16384 {
        return Err(Error::AtlasTooLarge);
    }
    let mut bytes = vec![0; (width * height * channels) as usize];
    for (key, image) in images {
        let Some(image) = image else {
            continue;
        };
        let rect = rects[key];
        for dy in -(padding as i32)..(image.height + padding) as i32 {
            for dx in -(padding as i32)..(image.width + padding) as i32 {
                let src_x = dx.clamp(0, image.width as i32 - 1) as u32;
                let src_y = dy.clamp(0, image.height as i32 - 1) as u32;
                let dst_x = (rect[0] as i32 + dx) as u32;
                let dst_y = (rect[1] as i32 + dy) as u32;
                let src = ((src_y * image.width + src_x) * channels) as usize;
                let dst = ((dst_y * width + dst_x) * channels) as usize;
                bytes[dst..dst + channels as usize]
                    .copy_from_slice(&image.bytes[src..src + channels as usize]);
            }
        }
    }
    let mut levels = vec![MipLevel {
        width,
        height,
        bytes,
    }];
    while mip_count.map_or_else(
        || {
            levels
                .last()
                .is_some_and(|level| level.width > 1 || level.height > 1)
        },
        |count| levels.len() < count,
    ) {
        let prev = levels.last().unwrap();
        let w = (prev.width / 2).max(1);
        let h = (prev.height / 2).max(1);
        let mut next = vec![0; (w * h * channels) as usize];
        for row in 0..h {
            for col in 0..w {
                for channel in 0..channels {
                    let mut sum = 0u32;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = (2 * col + dx).min(prev.width - 1);
                            let sy = (2 * row + dy).min(prev.height - 1);
                            sum += prev.bytes
                                [((sy * prev.width + sx) * channels + channel) as usize]
                                as u32;
                        }
                    }
                    next[((row * w + col) * channels + channel) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push(MipLevel {
            width: w,
            height: h,
            bytes: next,
        });
    }
    Ok((Atlas { channels, levels }, rects))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../fonts/EBGaramond[wght].ttf");
    const MONO_FONT: &[u8] = include_bytes!("../tests/fonts/IBMPlexMono-Regular.ttf");
    const VARIABLE_FONT: &[u8] = include_bytes!("../tests/fonts/DMSans[opsz,wght].ttf");
    const COLOR_FONT: &[u8] = include_bytes!("../tests/fonts/BungeeColor-Regular.ttf");

    fn rendered_pixels(paragraph: &RichParagraph) -> (u32, u32, Vec<u8>) {
        let quad = &paragraph.quads[0];
        let level = &paragraph.atlas.levels[0];
        let x = (quad.uv[0] * level.width as f32).round() as u32;
        let y = (quad.uv[1] * level.height as f32).round() as u32;
        let width = (quad.uv[2] * level.width as f32).round() as u32 - x;
        let height = (quad.uv[3] * level.height as f32).round() as u32 - y;
        let mut pixels = Vec::new();
        for row in y..y + height {
            let start = (row * level.width + x) as usize;
            pixels.extend_from_slice(&level.bytes[start..start + width as usize]);
        }
        (width, height, pixels)
    }

    fn first_stem_width(width: u32, height: u32, pixels: &[u8]) -> u32 {
        let row = height / 4;
        let ink: Vec<_> = (0..width)
            .filter(|&x| pixels[(row * width + x) as usize] >= 128)
            .collect();
        let first = ink[0];
        ink.into_iter()
            .enumerate()
            .take_while(|(index, x)| *x == first + *index as u32)
            .count() as u32
    }

    #[test]
    fn variable_weight_matches_swash_and_grows_stems() {
        let mut engine = TextEngine::new(VARIABLE_FONT).unwrap();
        let mut areas = Vec::new();
        let mut stems = Vec::new();
        for weight in [400, 700] {
            let span = Span {
                text: "H".into(),
                face: Face {
                    family: "DM Sans".into(),
                    size: 64.0,
                    line: 72.0,
                    weight,
                    italic: false,
                    spacing: 0.0,
                },
                color: [1.0; 4],
            };
            let block = engine
                .layout_spans(std::slice::from_ref(&span), None, 1.0, Anchor::Start)
                .unwrap();
            let glyph = &block.buffer.layout_runs().next().unwrap().glyphs[0];
            let font_id = glyph.font_id;
            let glyph_id = glyph.glyph_id;
            let font_weight = glyph.font_weight;
            let flags = glyph.cache_key_flags;
            let font_size = glyph.font_size;
            let rendered = engine
                .render_spans(
                    &[span],
                    None,
                    1.0,
                    Anchor::Start,
                    Representation::Coverage,
                    [0.0; 2],
                    1.0,
                    [0.0; 4],
                    [0.0; 3],
                )
                .unwrap();
            let (width, height, pixels) = rendered_pixels(&rendered);
            let font = engine.fonts.get_font(font_id, font_weight).unwrap();
            let key = raster_key(
                font_id,
                glyph_id,
                font_size,
                font_weight,
                font_size,
                flags,
                font.as_swash(),
            );
            assert_eq!(key.wght, Some((weight as f32).to_bits()));
            assert!(key.opsz.is_some());
            let mut scaler = ScaleContext::new();
            let mut scaler = scaler
                .builder(font.as_swash())
                .size(font_size)
                .hint(!flags.contains(CacheKeyFlags::DISABLE_HINTING))
                .variations(key.variations())
                .build();
            let reference = Render::new(&[Source::Outline])
                .render(&mut scaler, glyph_id)
                .unwrap();
            assert_eq!(
                (width, height),
                (reference.placement.width, reference.placement.height)
            );
            assert_eq!(pixels, reference.data);
            areas.push(pixels.iter().map(|&value| u64::from(value)).sum::<u64>());
            stems.push(first_stem_width(width, height, &pixels));
        }
        assert!(areas[1] > areas[0]);
        assert!(stems[1] > stems[0], "stem widths: {stems:?}");
    }

    #[test]
    fn optical_size_uses_unscaled_points_at_fixed_raster_size() {
        let mut engine = TextEngine::new(VARIABLE_FONT).unwrap();
        let mut keys = Vec::new();
        let mut images = Vec::new();
        for (size, scale) in [(16.0, 4.0), (32.0, 2.0)] {
            let span = Span {
                text: "a".into(),
                face: Face {
                    family: "DM Sans".into(),
                    size,
                    line: size * 1.2,
                    weight: 500,
                    italic: false,
                    spacing: 0.0,
                },
                color: [1.0; 4],
            };
            let block = engine
                .layout_spans(std::slice::from_ref(&span), None, scale, Anchor::Start)
                .unwrap();
            let glyph = &block.buffer.layout_runs().next().unwrap().glyphs[0];
            let font_id = glyph.font_id;
            let glyph_id = glyph.glyph_id;
            let font_weight = glyph.font_weight;
            let flags = glyph.cache_key_flags;
            let font_size = glyph.font_size;
            let font = engine.fonts.get_font(font_id, font_weight).unwrap();
            let key = raster_key(
                font_id,
                glyph_id,
                font_size,
                font_weight,
                font_size / scale,
                flags,
                font.as_swash(),
            );
            assert_eq!(key.opsz, Some(size.to_bits()));
            let rendered = engine
                .render_spans(
                    &[span],
                    None,
                    scale,
                    Anchor::Start,
                    Representation::Coverage,
                    [0.0; 2],
                    1.0,
                    [0.0; 4],
                    [0.0; 3],
                )
                .unwrap();
            let (width, height, pixels) = rendered_pixels(&rendered);
            let mut context = ScaleContext::new();
            let mut scaler = context
                .builder(font.as_swash())
                .size(font_size)
                .hint(!flags.contains(CacheKeyFlags::DISABLE_HINTING))
                .variations(key.variations())
                .build();
            let reference = Render::new(&[Source::Outline])
                .render(&mut scaler, glyph_id)
                .unwrap();
            assert_eq!(
                (width, height),
                (reference.placement.width, reference.placement.height)
            );
            assert_eq!(pixels, reference.data);
            keys.push(key);
            images.push(pixels);
        }
        assert_eq!(keys[0].ppem, keys[1].ppem);
        assert_ne!(keys[0].opsz, keys[1].opsz);
        assert_ne!(images[0], images[1]);
    }

    #[test]
    fn fake_italic_shears_vertical_stem() {
        let mut engine = mono_engine();
        let mut spans = mono("H");
        spans[0].face.size = 64.0;
        spans[0].face.line = 72.0;
        let upright = engine
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        spans[0].face.italic = true;
        let block = engine
            .layout_spans(&spans, None, 1.0, Anchor::Start)
            .unwrap();
        let glyph = &block.buffer.layout_runs().next().unwrap().glyphs[0];
        let font_id = glyph.font_id;
        let flags = glyph.cache_key_flags;
        assert!(
            raster_flags(
                flags,
                true,
                engine.fonts.db().face(font_id).map(|face| face.style),
            )
            .contains(CacheKeyFlags::FAKE_ITALIC)
        );
        let italic = engine
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        let (width, height, pixels) = rendered_pixels(&italic);
        let left_at = |row: u32| {
            let x = (0..width)
                .find(|&x| pixels[(row * width + x) as usize] >= 128)
                .unwrap();
            italic.quads[0].rect[0] + x as f32
        };
        let top = height / 4;
        let bottom = height * 3 / 4;
        let shift = left_at(top) - left_at(bottom);
        let expected = (bottom - top) as f32 * 14.0f32.to_radians().tan();
        assert!(
            (shift - expected).abs() < 2.0,
            "shift {shift}, expected {expected}"
        );
        assert!(italic.quads[0].rect[2] > upright.quads[0].rect[2]);
        spans[0].face.italic = false;
        let upright_msdf = engine
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Msdf,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        spans[0].face.italic = true;
        let italic_msdf = engine
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Msdf,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        assert!(italic_msdf.quads[0].rect[2] > upright_msdf.quads[0].rect[2]);
    }

    #[test]
    fn raster_keys_distinguish_weight_coordinates_and_italic() {
        let mut engine = TextEngine::new(VARIABLE_FONT).unwrap();
        let font_id = engine.fonts.db().faces().next().unwrap().id;
        let font = engine.fonts.get_font(font_id, fontdb::Weight(400)).unwrap();
        let regular = raster_key(
            font_id,
            42,
            64.0,
            fontdb::Weight(400),
            16.0,
            CacheKeyFlags::empty(),
            font.as_swash(),
        );
        let heavy = raster_key(
            font_id,
            42,
            64.0,
            fontdb::Weight(700),
            16.0,
            CacheKeyFlags::empty(),
            font.as_swash(),
        );
        let italic = raster_key(
            font_id,
            42,
            64.0,
            fontdb::Weight(400),
            16.0,
            CacheKeyFlags::FAKE_ITALIC,
            font.as_swash(),
        );
        assert_ne!(regular, heavy);
        assert_ne!(regular, italic);
        assert_eq!(regular.cell_key(), regular.clone().cell_key());
        assert_ne!(regular.cell_key(), heavy.cell_key());
        assert_ne!(regular.cell_key(), italic.cell_key());
        assert_ne!(
            regular.cell_key(),
            RasterKey {
                ppem: 65.0f32.to_bits(),
                ..regular.clone()
            }
            .cell_key()
        );
        assert_ne!(
            regular.cell_key(),
            RasterKey {
                x_bin: SubpixelBin::One,
                ..regular.clone()
            }
            .cell_key()
        );
        let optical = RasterKey {
            opsz: Some(20.0f32.to_bits()),
            ..regular.clone()
        };
        assert_ne!(regular, optical);
        assert_ne!(regular.cell_key(), optical.cell_key());
        assert_ne!(
            optical,
            RasterKey {
                opsz: Some(30.0f32.to_bits()),
                ..optical.clone()
            }
        );
    }

    fn mono_face() -> Face {
        Face {
            family: "IBM Plex Mono".into(),
            size: 20.0,
            line: 24.0,
            weight: 400,
            italic: false,
            spacing: 0.0,
        }
    }

    fn mono_engine() -> TextEngine {
        TextEngine::new(MONO_FONT).unwrap()
    }

    fn mono(text: &str) -> Vec<Span> {
        vec![Span {
            text: text.into(),
            face: mono_face(),
            color: [0.2, 0.4, 0.6, 1.0],
        }]
    }

    #[test]
    #[ignore = "manual timing"]
    fn measure_rich_translation() {
        let mut engine = mono_engine();
        for (name, text) in [
            ("label", "Cache"),
            (
                "paragraph",
                "A paragraph of text that moves across the page. This measures repeated raster work and placement across multiple lines of text.",
            ),
        ] {
            let spans = mono(text);
            let width = (name == "paragraph").then_some(220.0);
            let render = |engine: &mut TextEngine, x| {
                engine
                    .render_spans(
                        &spans,
                        width,
                        1.0,
                        Anchor::Start,
                        Representation::Coverage,
                        [x, 0.0],
                        1.0,
                        [0.0; 4],
                        [0.0; 3],
                    )
                    .unwrap()
            };
            for x in [0.0, 0.25] {
                let _ = render(&mut engine, x);
            }
            let start = std::time::Instant::now();
            for i in 0..100 {
                let _ = render(&mut engine, (i % 2) as f32 * 0.25);
            }
            println!(
                "{name}: {:.4} ms/call",
                start.elapsed().as_secs_f64() * 10.0
            );
        }
    }

    #[test]
    fn quarter_pixel_bins_shift_coverage() {
        let mut engine = mono_engine();
        let mut spans = mono("H");
        spans[0].face.size = 64.0;
        spans[0].face.line = 72.0;
        let mut keys = Vec::new();
        let mut centroids = Vec::new();
        for offset in [0.0, 0.25, 0.5, 0.75] {
            let rendered = engine
                .render_spans(
                    &spans,
                    None,
                    1.0,
                    Anchor::Start,
                    Representation::Coverage,
                    [offset, 0.0],
                    1.0,
                    [0.0; 4],
                    [0.0; 3],
                )
                .unwrap();
            let quad = &rendered.quads[0];
            assert_eq!(quad.atlas_page, AtlasPage::Monochrome);
            assert!(!quad.is_color);
            assert_eq!(rendered.cells[&quad.key].page, AtlasPage::Monochrome);
            keys.push(quad.key);
            let (width, height, pixels) = rendered_pixels(&rendered);
            let mut sum = 0.0;
            let mut mass = 0.0;
            for y in 0..height {
                for x in 0..width {
                    let ink = pixels[(y * width + x) as usize] as f32;
                    sum += (quad.rect[0] + x as f32 + 0.5) * ink;
                    mass += ink;
                }
            }
            centroids.push(sum / mass);
        }
        assert_eq!(
            keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
            4
        );
        for pair in centroids.windows(2) {
            assert!((pair[1] - pair[0] - 0.25).abs() < 0.08, "{centroids:?}");
        }
        let mut y_keys = Vec::new();
        let mut y_centroids = Vec::new();
        for offset in [0.0, 0.25, 0.5, 0.75] {
            let rendered = engine
                .render_spans(
                    &spans,
                    None,
                    1.0,
                    Anchor::Start,
                    Representation::Coverage,
                    [0.0, offset],
                    1.0,
                    [0.0; 4],
                    [0.0; 3],
                )
                .unwrap();
            let quad = &rendered.quads[0];
            y_keys.push(quad.key);
            let (width, height, pixels) = rendered_pixels(&rendered);
            let mut sum = 0.0;
            let mut mass = 0.0;
            for y in 0..height {
                for x in 0..width {
                    let ink = pixels[(y * width + x) as usize] as f32;
                    sum += (quad.rect[1] + y as f32 + 0.5) * ink;
                    mass += ink;
                }
            }
            y_centroids.push(sum / mass);
        }
        assert_eq!(
            y_keys
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1
        );
        for pair in y_centroids.windows(2) {
            assert!((pair[1] - pair[0]).abs() < 0.01, "{y_centroids:?}");
        }
        let repeated = engine
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        assert_eq!(repeated.quads[0].key, keys[0]);
        let fresh = mono_engine()
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        assert_eq!(fresh.quads[0].key, keys[0]);
    }

    #[test]
    fn y_placement_matches_cosmic_truncation_and_exposes_pens() {
        let mut engine = mono_engine();
        let mut spans = mono("Hi\nHi");
        spans[0].face.line = 24.6;
        let scale = 1.25;
        for origin in [[0.25, 0.37], [2.5, -0.42], [-1.25, 1.15]] {
            let expected = engine
                .layout_spans(&spans, None, scale, Anchor::Start)
                .unwrap()
                .buffer
                .layout_runs()
                .flat_map(|run| {
                    run.glyphs.iter().map(move |glyph| {
                        (
                            run.line_i,
                            run.line_y,
                            glyph.x + glyph.x_offset * glyph.font_size,
                            glyph.y - glyph.y_offset * glyph.font_size,
                        )
                    })
                })
                .collect::<Vec<_>>();
            let rendered = engine
                .render_spans(
                    &spans,
                    None,
                    scale,
                    Anchor::Start,
                    Representation::Coverage,
                    origin,
                    1.0,
                    [0.0; 4],
                    [0.0; 3],
                )
                .unwrap();
            assert_eq!(rendered.quads.len(), expected.len());
            for (quad, (line_index, line_y, glyph_x, glyph_y)) in
                rendered.quads.iter().zip(expected)
            {
                let raster = engine
                    .rasters
                    .iter()
                    .find(|(key, _)| key.cell_key() == quad.key)
                    .unwrap()
                    .1;
                let image = raster.mono.as_ref().unwrap();
                let y = (glyph_y + origin[1] * scale + line_y).trunc();
                assert!((quad.rect[1] - (y + image.top) / scale).abs() < 0.00001);
                assert_eq!(quad.line_index, line_index);
                assert!((quad.line_baseline - (line_y / scale + origin[1])).abs() < 0.001);
                assert!((quad.pen[0] - (glyph_x / scale + origin[0])).abs() < 0.001);
                assert!((quad.pen[1] - ((glyph_y + line_y) / scale + origin[1])).abs() < 0.001);
            }
        }
    }

    #[test]
    fn raster_cache_reuses_translation_and_rasterizes_new_glyphs_and_sizes() {
        let mut engine = mono_engine();
        let spans = mono("HH");
        let render = |engine: &mut TextEngine, spans: &[Span], origin| {
            engine
                .render_spans(
                    spans,
                    None,
                    1.0,
                    Anchor::Start,
                    Representation::Coverage,
                    origin,
                    1.0,
                    [0.0; 4],
                    [0.0; 3],
                )
                .unwrap()
        };
        let first = render(&mut engine, &spans, [0.0, 0.0]);
        let count = engine.cached_rasters();
        let translated = render(&mut engine, &spans, [10.0, 0.75]);
        assert_eq!(engine.cached_rasters(), count);
        assert_eq!(first.quads[0].key, translated.quads[0].key);
        assert_eq!(first.atlas.levels[0], translated.atlas.levels[0]);
        assert_eq!(translated.quads[0].rect[0] - first.quads[0].rect[0], 10.0);
        let _ = render(&mut engine, &mono("HZ"), [0.0, 0.0]);
        assert_eq!(engine.cached_rasters(), count + 1);
        let mut larger = mono("H");
        larger[0].face.size = 24.0;
        let _ = render(&mut engine, &larger, [0.0, 0.0]);
        assert_eq!(engine.cached_rasters(), count + 2);
    }

    #[test]
    fn color_outline_uses_rgba_page() {
        let mut engine = TextEngine::new(COLOR_FONT).unwrap();
        let rendered = engine
            .render_spans(
                &[Span {
                    text: "A".into(),
                    face: Face {
                        family: "Bungee Color".into(),
                        size: 64.0,
                        line: 72.0,
                        weight: 400,
                        italic: false,
                        spacing: 0.0,
                    },
                    color: [1.0; 4],
                }],
                None,
                1.0,
                Anchor::Start,
                Representation::Coverage,
                [0.0; 2],
                1.0,
                [0.0; 4],
                [0.0; 3],
            )
            .unwrap();
        let quad = &rendered.quads[0];
        assert!(quad.is_color);
        assert_eq!(quad.atlas_page, AtlasPage::Color);
        let cell = rendered.cells[&quad.key];
        assert_eq!(cell.page, AtlasPage::Color);
        let atlas = rendered.color_atlas.as_ref().unwrap();
        assert_eq!(atlas.channels, 4);
        let level = &atlas.levels[0];
        let pixels = (cell.rect[1]..cell.rect[1] + cell.rect[3])
            .flat_map(|y| {
                (cell.rect[0]..cell.rect[0] + cell.rect[2]).map(move |x| {
                    let index = ((y * level.width + x) * 4) as usize;
                    &level.bytes[index..index + 4]
                })
            })
            .collect::<Vec<_>>();
        assert!(pixels.iter().any(|pixel| pixel[3] > 0));
        assert!(pixels.iter().any(|pixel| pixel[0] != pixel[1]));
    }

    #[test]
    fn atlas_constants_are_exported() {
        assert_eq!(ATLAS_PADDING, 8);
        assert_eq!(ATLAS_CELL_ALIGN, 8);
    }

    #[test]
    fn rich_cursor_and_marks_use_byte_offsets() {
        let mut engine = mono_engine();
        let block = engine
            .layout_spans(&mono("iiii"), None, 1.0, Anchor::Start)
            .unwrap();
        let cell = block.width / 4.0;
        let mark = marks(block, 1, 3);
        assert_eq!(mark.len(), 1);
        assert!((mark[0][0] - cell).abs() < 0.01);
        assert!((mark[0][2] - cell * 2.0).abs() < 0.01);
        assert_eq!(hit(block, cell * 0.25, block.height * 0.5), Some(0));
        assert_eq!(hit(block, cell * 0.75, block.height * 0.5), Some(1));
        assert_eq!(hit(block, cell * 2.75, block.height * 0.5), Some(3));
        assert_eq!(cursor_at(block, 3), (0, 3));
        assert!((baseline(block) - block.baseline).abs() < 0.001);

        let block = engine
            .layout_spans(&mono("i\ni"), None, 1.0, Anchor::Start)
            .unwrap();
        assert_eq!(cursor_at(block, 0), (0, 0));
        assert_eq!(cursor_at(block, 1), (0, 1));
        assert_eq!(cursor_at(block, 2), (1, 0));
        assert_eq!(cursor_at(block, 3), (1, 1));
        assert_eq!(hit(block, 0.0, block.height * 0.75), Some(2));
        assert_eq!(marks(block, 0, 3).len(), 2);
    }

    #[test]
    fn rich_faces_wrap_and_align_together() {
        let mut engine = mono_engine();
        engine.register_font(FONT).unwrap();
        let spans = vec![
            Span {
                text: "ii".into(),
                face: mono_face(),
                color: [1.0, 0.0, 0.0, 1.0],
            },
            Span {
                text: "AV".into(),
                face: Face {
                    family: "EB Garamond".into(),
                    size: 24.0,
                    line: 28.0,
                    weight: 600,
                    italic: true,
                    spacing: 0.02,
                },
                color: [0.0, 0.0, 1.0, 0.5],
            },
        ];
        let block = engine
            .layout_spans(&spans, None, 1.0, Anchor::Start)
            .unwrap();
        assert_eq!(cursor_at(block, 4), (0, 4));
        let rendered = engine
            .render_spans(
                &spans,
                None,
                1.0,
                Anchor::Start,
                Representation::Msdf,
                [5.0, 7.0],
                0.5,
                [0.0, 0.0, 100.0, 100.0],
                [0.0, 0.0, 0.0],
            )
            .unwrap();
        assert_eq!(rendered.quads.len(), 4);
        assert_eq!(rendered.quads[0].color, [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(rendered.quads[3].color, [0.0, 0.0, 1.0, 0.25]);
        assert_eq!(rendered.quads[0].origin, [5.0, 7.0]);

        let plain = mono("iiii");
        let width = engine
            .layout_spans(&plain, None, 1.0, Anchor::Start)
            .unwrap()
            .width;
        let wrapped = engine
            .layout_spans(&plain, Some(width * 0.55), 1.0, Anchor::Start)
            .unwrap();
        assert!(wrapped.height > wrapped.baseline);
        assert_eq!(wrapped.buffer.layout_runs().count(), 2);
        let container = width + 40.0;
        let mut xs = Vec::new();
        for anchor in [Anchor::Start, Anchor::Center, Anchor::End] {
            let block = engine
                .layout_spans(&plain, Some(container), 1.0, anchor)
                .unwrap();
            xs.push(marks(block, 0, 1)[0][0]);
        }
        assert!((xs[0] - 0.0).abs() < 0.01);
        assert!((xs[1] - 20.0).abs() < 0.01);
        assert!((xs[2] - 40.0).abs() < 0.01);
    }

    #[test]
    fn rich_cache_expires_after_an_unused_frame() {
        let mut engine = mono_engine();
        engine.set_shaping_budget(0);
        let spans = mono("iiii");
        let first = engine
            .layout_spans(&spans, None, 1.0, Anchor::Start)
            .unwrap() as *const Block;
        let second = engine
            .layout_spans(&spans, None, 1.0, Anchor::Start)
            .unwrap() as *const Block;
        assert_eq!(first, second);
        assert_eq!(engine.cached_blocks(), 1);
        engine.end_frame();
        assert_eq!(engine.cached_blocks(), 1);
        engine.begin_frame();
        engine.end_frame();
        assert_eq!(engine.cached_blocks(), 0);
    }

    #[test]
    fn atlas_padding_is_isolated_at_every_mip() {
        let images = BTreeMap::from([
            (
                0,
                Some(GlyphImage {
                    left: 0.0,
                    top: 0.0,
                    width: 16,
                    height: 16,
                    bytes: vec![30; 256],
                }),
            ),
            (
                1,
                Some(GlyphImage {
                    left: 0.0,
                    top: 0.0,
                    width: 16,
                    height: 16,
                    bytes: vec![220; 256],
                }),
            ),
        ]);
        let (atlas, rects) = pack(&images, 1, Some(4)).unwrap();
        assert_eq!(atlas.levels.len(), 4);
        for (level_index, level) in atlas.levels.iter().enumerate() {
            for (key, value) in [(0, 30u8), (1, 220u8)] {
                let rect = rects[&key];
                let shift = level_index as u32;
                let y = (rect[1] >> shift) + 1;
                for x in ((rect[0] - 8) >> shift)..=((rect[0] + rect[2] + 7) >> shift) {
                    assert_eq!(level.bytes[(y * level.width + x) as usize], value);
                }
            }
        }
    }

    fn style(representation: Representation, density: f32) -> Style<'static> {
        Style {
            family: "EB Garamond",
            size: 1.0,
            line_height: 1.2,
            wrap_width: None,
            pixels_per_unit: density,
            representation,
        }
    }

    fn straight() -> Baseline {
        Baseline::Straight {
            origin: [0.0, 0.0],
            direction: [1.0, 0.0],
        }
    }

    #[test]
    fn atlas_bytes_are_deterministic() {
        let mut a = TextEngine::new(FONT).unwrap();
        let mut b = TextEngine::new(FONT).unwrap();
        for representation in [Representation::Coverage, Representation::Msdf] {
            let first = a
                .layout("AVATAR ffi été", style(representation, 96.0), &straight())
                .unwrap();
            let second = b
                .layout("AVATAR ffi été", style(representation, 96.0), &straight())
                .unwrap();
            assert_eq!(first.atlas.channels, second.atlas.channels);
            assert_eq!(first.atlas.levels, second.atlas.levels);
            assert_eq!(first.glyphs.len(), second.glyphs.len());
            assert!(first.atlas.levels.len() > 1);
        }
    }

    #[test]
    fn coverage_matches_reference_glyph() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let paragraph = engine
            .layout("A", style(Representation::Coverage, 64.0), &straight())
            .unwrap();
        let glyph = &paragraph.glyphs[0];
        let rect = glyph.atlas_rect;
        let atlas = &paragraph.atlas.levels[0];
        let mut pixels = Vec::new();
        for y in rect[1]..rect[1] + rect[3] {
            let start = (y * atlas.width + rect[0]) as usize;
            pixels.extend_from_slice(&atlas.bytes[start..start + rect[2] as usize]);
        }
        let checksum = pixels.iter().fold(0xcbf29ce484222325u64, |hash, value| {
            (hash ^ u64::from(*value)).wrapping_mul(0x100000001b3)
        });
        assert_eq!((rect[2], rect[3], checksum), (46, 45, 5879817070010596299));
    }

    #[test]
    fn msdf_reconstructs_coverage_at_multiple_scales() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let msdf = engine
            .layout("A", style(Representation::Msdf, 64.0), &straight())
            .unwrap();
        let m = &msdf.glyphs[0];
        for density in [32.0, 64.0, 128.0] {
            let coverage = engine
                .layout("A", style(Representation::Coverage, density), &straight())
                .unwrap();
            let c = &coverage.glyphs[0];
            let level = &coverage.atlas.levels[0];
            let mut error = 0.0;
            let mut count = 0;
            for y in 0..c.atlas_rect[3] {
                for x in 0..c.atlas_rect[2] {
                    let px = (x as f32 + c.local_rect[0] + 0.5) / density;
                    let py = (y as f32 + c.local_rect[1] + 0.5) / density;
                    let sx = px * 64.0 - m.local_rect[0];
                    let sy = py * 64.0 - m.local_rect[1];
                    let rgb = sample_rgb(&msdf.atlas.levels[0], m.atlas_rect, sx, sy);
                    let predicted = msdf_coverage(rgb, 4.0 * density / 64.0);
                    let reference = level.bytes
                        [((c.atlas_rect[1] + y) * level.width + c.atlas_rect[0] + x) as usize]
                        as f32
                        / 255.0;
                    error += (predicted - reference).abs();
                    count += 1;
                }
            }
            assert!(
                error / (count as f32) < 0.17,
                "density {density}: {}",
                error / count as f32
            );
        }
    }

    fn sample_rgb(level: &MipLevel, rect: [u32; 4], x: f32, y: f32) -> [f32; 3] {
        let x = x - 0.5;
        let y = y - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let mut rgb = [0.0; 3];
        for dy in 0..2 {
            for dx in 0..2 {
                let sx = (x0 as i32 + dx).clamp(0, rect[2] as i32 - 1) as u32;
                let sy = (y0 as i32 + dy).clamp(0, rect[3] as i32 - 1) as u32;
                let wx = if dx == 0 { 1.0 - x.fract() } else { x.fract() };
                let wy = if dy == 0 { 1.0 - y.fract() } else { y.fract() };
                let at = (((rect[1] + sy) * level.width + rect[0] + sx) * 3) as usize;
                for (channel, value) in rgb.iter_mut().enumerate() {
                    *value += level.bytes[at + channel] as f32 / 255.0 * wx * wy;
                }
            }
        }
        rgb
    }

    #[test]
    fn glyphs_follow_curved_baselines() {
        let mut engine = TextEngine::new(FONT).unwrap();
        let style = style(Representation::Msdf, 64.0);
        let flat = engine.layout("CURVE", style, &straight()).unwrap();
        let path = Baseline::Arc {
            center: [2.0, 4.0],
            radius: 8.0,
            start_angle: -std::f32::consts::FRAC_PI_2,
            clockwise: false,
        };
        let curved = engine.layout("CURVE", style, &path).unwrap();
        for (a, b) in flat.glyphs.iter().zip(&curved.glyphs) {
            let (expected, angle) = path.sample(a.position[0], 0.0).unwrap();
            assert!((b.position[0] - expected[0]).abs() < 0.001);
            assert!((b.position[1] - expected[1]).abs() < 0.001);
            assert!((b.rotation - angle).abs() < 0.001);
        }
        let polyline = Baseline::Polyline(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
        let first = polyline.sample(0.5, 0.0).unwrap();
        let second = polyline.sample(1.5, 0.0).unwrap();
        assert_eq!(first.0, [0.5, 0.0]);
        assert_eq!(second.0, [1.0, 0.5]);
        assert!((second.1 - std::f32::consts::FRAC_PI_2).abs() < 0.001);
    }
}
