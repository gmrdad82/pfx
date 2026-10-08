use std::fs;
use std::path::Path;

use block_compression::{BC7Settings, CompressionVariant, decode, encode};
use pfx_materials::encode_channel;
use serde_json::{Value, json};

use super::{Crop, Region, digest};

pub const ALIGN: u32 = 16;
pub const LEVELS: u32 = 5;
pub const PAGE: u32 = 2048;
pub const PAGE_LIMIT: u32 = pfx_gpu::floor::TEXTURE_SIDE;
pub const SCHEMA: u64 = 2;
pub const SCHEMA_UV1: u64 = 3;
pub const KIND: &str = "detail-atlas";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgba8,
    Rgba8Srgb,
    Bc4,
    Bc5,
    Bc7Srgb,
}

impl Format {
    pub fn block(self) -> u32 {
        match self {
            Self::Rgba8 | Self::Rgba8Srgb => 1,
            Self::Bc4 | Self::Bc5 | Self::Bc7Srgb => 4,
        }
    }

    pub fn block_bytes(self) -> u32 {
        match self {
            Self::Rgba8 | Self::Rgba8Srgb => 4,
            Self::Bc4 => 8,
            Self::Bc5 | Self::Bc7Srgb => 16,
        }
    }

    pub fn level_bytes(self, width: u32, height: u32) -> usize {
        width.div_ceil(self.block()) as usize
            * height.div_ceil(self.block()) as usize
            * self.block_bytes() as usize
    }

    pub fn compressed(self) -> bool {
        self.block() == 4
    }

    pub fn vk(self) -> u32 {
        match self {
            Self::Rgba8 => 37,
            Self::Rgba8Srgb => 43,
            Self::Bc4 => 139,
            Self::Bc5 => 141,
            Self::Bc7Srgb => 146,
        }
    }

    pub fn from_vk(value: u32) -> Option<Self> {
        [
            Self::Rgba8,
            Self::Rgba8Srgb,
            Self::Bc4,
            Self::Bc5,
            Self::Bc7Srgb,
        ]
        .into_iter()
        .find(|format| format.vk() == value)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rgba8 => "rgba8",
            Self::Rgba8Srgb => "rgba8-srgb",
            Self::Bc4 => "bc4",
            Self::Bc5 => "bc5",
            Self::Bc7Srgb => "bc7-srgb",
        }
    }

    fn variant(self) -> Option<CompressionVariant> {
        match self {
            Self::Rgba8 | Self::Rgba8Srgb => None,
            Self::Bc4 => Some(CompressionVariant::BC4),
            Self::Bc5 => Some(CompressionVariant::BC5),
            Self::Bc7Srgb => Some(CompressionVariant::BC7(BC7Settings::opaque_slow())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Base,
    Normal,
    Roughness,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Base, Self::Normal, Self::Roughness];

    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Normal => "normal",
            Self::Roughness => "roughness",
        }
    }

    pub fn plain(self) -> Format {
        match self {
            Self::Base => Format::Rgba8Srgb,
            Self::Normal | Self::Roughness => Format::Rgba8,
        }
    }

    pub fn packed(self) -> Format {
        match self {
            Self::Base => Format::Bc7Srgb,
            Self::Normal => Format::Bc5,
            Self::Roughness => Format::Bc4,
        }
    }

    fn channels(self) -> &'static [usize] {
        match self {
            Self::Base => &[0, 1, 2],
            Self::Normal => &[0, 1],
            Self::Roughness => &[0],
        }
    }
}

pub fn level_size(width: u32, height: u32, level: u32) -> [u32; 2] {
    [(width >> level).max(1), (height >> level).max(1)]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

impl Texture {
    pub fn check(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.levels.is_empty() {
            return Err("detail page is empty".into());
        }
        if self.format.compressed()
            && (!self.width.is_multiple_of(4) || !self.height.is_multiple_of(4))
        {
            return Err("a block-compressed detail page needs sides in fours".into());
        }
        for (level, bytes) in self.levels.iter().enumerate() {
            let [width, height] = level_size(self.width, self.height, level as u32);
            if bytes.len() != self.format.level_bytes(width, height) {
                return Err(format!("detail page level {level} has the wrong size"));
            }
        }
        Ok(())
    }

    pub fn bytes(&self) -> u64 {
        self.levels.iter().map(|level| level.len() as u64).sum()
    }

    pub fn compress(&self, kind: Kind) -> Result<Self, String> {
        if self.format != kind.plain() {
            return Err(format!(
                "{} pages compress from {}",
                kind.name(),
                kind.plain().name()
            ));
        }
        let format = kind.packed();
        let levels = self
            .levels
            .iter()
            .enumerate()
            .map(|(level, bytes)| {
                let [width, height] = level_size(self.width, self.height, level as u32);
                encode_level(format, width, height, bytes)
            })
            .collect();
        Ok(Self {
            format,
            width: self.width,
            height: self.height,
            levels,
        })
    }

    pub fn decode(&self) -> Self {
        let format = match self.format {
            Format::Bc7Srgb => Format::Rgba8Srgb,
            Format::Bc4 | Format::Bc5 => Format::Rgba8,
            plain => return self.clone().with_format(plain),
        };
        let levels = self
            .levels
            .iter()
            .enumerate()
            .map(|(level, bytes)| {
                let [width, height] = level_size(self.width, self.height, level as u32);
                decode_level(self.format, width, height, bytes)
            })
            .collect();
        Self {
            format,
            width: self.width,
            height: self.height,
            levels,
        }
    }

    fn with_format(mut self, format: Format) -> Self {
        self.format = format;
        self
    }
}

fn padded(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let wide = width.div_ceil(4) * 4;
    let tall = height.div_ceil(4) * 4;
    let mut out = vec![0; (wide * tall * 4) as usize];
    for y in 0..tall {
        let sy = y.min(height - 1);
        for x in 0..wide {
            let sx = x.min(width - 1);
            let from = ((sy * width + sx) * 4) as usize;
            let to = ((y * wide + x) * 4) as usize;
            out[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    (wide, tall, out)
}

pub fn encode_level(format: Format, width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let Some(variant) = format.variant() else {
        return rgba.to_vec();
    };
    let (wide, tall, source) = padded(width, height, rgba);
    let mut out = vec![0; format.level_bytes(width, height)];
    let rows = (tall / 4) as usize;
    let row_bytes = format.level_bytes(wide, 4);
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, rows);
    let per = rows.div_ceil(threads);
    std::thread::scope(|scope| {
        for (chunk, blocks) in out.chunks_mut(per * row_bytes).enumerate() {
            let first = chunk * per;
            let count = blocks.len() / row_bytes;
            let start = first * 4 * wide as usize * 4;
            let end = start + count * 4 * wide as usize * 4;
            let source = &source[start..end];
            scope.spawn(move || {
                encode::compress_rgba8(variant, source, blocks, wide, count as u32 * 4, wide * 4);
            });
        }
    });
    out
}

pub fn decode_level(format: Format, width: u32, height: u32, blocks: &[u8]) -> Vec<u8> {
    let Some(variant) = format.variant() else {
        return blocks.to_vec();
    };
    let wide = width.div_ceil(4) * 4;
    let tall = height.div_ceil(4) * 4;
    let mut full = vec![0; (wide * tall * 4) as usize];
    decode::decompress_blocks_as_rgba8(variant, wide, tall, blocks, &mut full);
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let at = ((y * wide + x) * 4) as usize;
            let texel = &full[at..at + 4];
            out.extend_from_slice(&match format {
                Format::Bc4 => [texel[0], 0, 0, 255],
                Format::Bc5 => [texel[0], texel[1], normal_z(texel[0], texel[1]), 255],
                _ => [texel[0], texel[1], texel[2], texel[3]],
            });
        }
    }
    out
}

pub fn normal_z(x: u8, y: u8) -> u8 {
    let x = x as f32 / 255.0 * 2.0 - 1.0;
    let y = y as f32 / 255.0 * 2.0 - 1.0;
    let z = (1.0 - x * x - y * y).max(0.0).sqrt();
    ((z * 0.5 + 0.5) * 255.0).round() as u8
}

fn quantize(level: &[[f32; 4]], kind: Kind) -> Vec<u8> {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    level
        .iter()
        .flat_map(|&texel| {
            let rgba = match kind {
                Kind::Base => {
                    let a = texel[3];
                    let rgb: [f32; 3] = std::array::from_fn(|c| {
                        if a > 0.0 {
                            encode_channel((texel[c] / a).clamp(0.0, 1.0))
                        } else {
                            0.0
                        }
                    });
                    [rgb[0], rgb[1], rgb[2], a]
                }
                Kind::Normal | Kind::Roughness => texel,
            };
            rgba.map(byte)
        })
        .collect()
}

pub fn chain(kind: Kind, width: u32, height: u32, linear: &[u8], levels: u32) -> Vec<Vec<u8>> {
    let mut level: Vec<[f32; 4]> = linear
        .chunks_exact(4)
        .map(|p| {
            let rgba: [f32; 4] = std::array::from_fn(|c| p[c] as f32 / 255.0);
            match kind {
                Kind::Base => {
                    let a = rgba[3];
                    [rgba[0] * a, rgba[1] * a, rgba[2] * a, a]
                }
                Kind::Normal | Kind::Roughness => rgba,
            }
        })
        .collect();
    let (mut width, mut height) = (width, height);
    let mut out = vec![quantize(&level, kind)];
    while (out.len() as u32) < levels && (width > 1 || height > 1) {
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let mut next = Vec::with_capacity(next_width as usize * next_height as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut total = [0.0; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (2 * x + dx).min(width - 1);
                        let sy = (2 * y + dy).min(height - 1);
                        let texel = level[(sy * width + sx) as usize];
                        for c in 0..4 {
                            total[c] += texel[c] * 0.25;
                        }
                    }
                }
                if kind == Kind::Normal {
                    let normal: [f32; 3] = std::array::from_fn(|c| total[c] * 2.0 - 1.0);
                    let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
                    for c in 0..3 {
                        total[c] = normal[c] / length * 0.5 + 0.5;
                    }
                }
                next.push(total);
            }
        }
        out.push(quantize(&next, kind));
        level = next;
        width = next_width;
        height = next_height;
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packing {
    pub width: u32,
    pub height: u32,
    pub pages: u32,
    pub places: Places,
}

type Places = Vec<(u32, [u32; 2])>;

fn shelves(sizes: &[[u32; 2]], width: u32, height: u32) -> Option<(u32, Places)> {
    if sizes.iter().any(|size| size[0] > width || size[1] > height) {
        return None;
    }
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&index| {
        (
            std::cmp::Reverse(sizes[index][1]),
            std::cmp::Reverse(sizes[index][0]),
            index,
        )
    });
    struct Shelf {
        page: u32,
        y: u32,
        height: u32,
        x: u32,
    }
    let mut shelves: Vec<Shelf> = Vec::new();
    let mut pages = 0u32;
    let mut floor = 0u32;
    let mut places = vec![(0, [0, 0]); sizes.len()];
    for index in order {
        let [w, h] = sizes[index];
        if let Some(shelf) = shelves
            .iter_mut()
            .find(|shelf| shelf.height >= h && shelf.x + w <= width)
        {
            places[index] = (shelf.page, [shelf.x, shelf.y]);
            shelf.x += w;
            continue;
        }
        if pages == 0 || floor + h > height {
            pages += 1;
            floor = 0;
        }
        places[index] = (pages - 1, [0, floor]);
        shelves.push(Shelf {
            page: pages - 1,
            y: floor,
            height: h,
            x: w,
        });
        floor += h;
    }
    Some((pages.max(1), places))
}

fn extent(sizes: &[[u32; 2]], places: &[(u32, [u32; 2])]) -> [u32; 2] {
    let used = |axis: usize| {
        places
            .iter()
            .zip(sizes)
            .map(|((_, at), size)| at[axis] + size[axis])
            .max()
            .unwrap_or(ALIGN)
            .next_power_of_two()
    };
    [used(0), used(1)]
}

pub fn pack(sizes: &[[u32; 2]], page: u32) -> Result<Packing, String> {
    if !page.is_power_of_two() || page < ALIGN {
        return Err(format!(
            "a detail page side must be a power of two of at least {ALIGN}"
        ));
    }
    if let Some(size) = sizes
        .iter()
        .find(|size| size[0] == 0 || size[1] == 0 || size[0] > page || size[1] > page)
    {
        return Err(format!(
            "a {}×{} detail crop does not fit a {page}² page",
            size[0], size[1]
        ));
    }
    let (pages, places) = shelves(sizes, page, page).ok_or("detail crops do not fit")?;
    if pages > 1 {
        return Ok(Packing {
            width: page,
            height: page,
            pages,
            places,
        });
    }
    let mut best = (extent(sizes, &places), places);
    let mut width = page / 2;
    while width >= ALIGN {
        match shelves(sizes, width, page) {
            Some((1, places)) => {
                let size = extent(sizes, &places);
                if u64::from(size[0]) * u64::from(size[1])
                    < u64::from(best.0[0]) * u64::from(best.0[1])
                {
                    best = (size, places);
                }
            }
            _ => break,
        }
        width /= 2;
    }
    Ok(Packing {
        width: best.0[0],
        height: best.0[1],
        pages: 1,
        places: best.1,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub index: u32,
    pub node: String,
    pub input_hash: String,
    pub region: Region,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub index: u32,
    pub node: String,
    pub density: u32,
    pub crop: Crop,
    pub page: u32,
    pub at: [u32; 2],
    pub input_hash: String,
    pub uv_set: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub parts: Vec<Placed>,
    pub base: Vec<Texture>,
    pub normal: Vec<Texture>,
    pub roughness: Vec<Texture>,
}

pub fn page_side(pieces: &[Piece], page: u32) -> u32 {
    let largest = pieces
        .iter()
        .map(|piece| piece.region.crop.width.max(piece.region.crop.height))
        .max()
        .unwrap_or(0);
    page.max(largest.next_power_of_two())
}

pub fn assemble(pieces: Vec<Piece>, page: u32) -> Result<Atlas, String> {
    if pieces.is_empty() {
        return Err("a detail atlas needs at least one part".into());
    }
    for piece in &pieces {
        let crop = piece.region.crop;
        if !crop.x.is_multiple_of(ALIGN)
            || !crop.y.is_multiple_of(ALIGN)
            || !crop.width.is_multiple_of(ALIGN)
            || !crop.height.is_multiple_of(ALIGN)
        {
            return Err(format!("detail crops must be aligned to {ALIGN} texels"));
        }
        let count = (crop.width * crop.height * 4) as usize;
        if [
            &piece.region.base,
            &piece.region.normal,
            &piece.region.roughness,
        ]
        .iter()
        .any(|map| map.len() != count)
        {
            return Err("a detail crop has the wrong number of texels".into());
        }
    }
    let side = page_side(&pieces, page);
    if side > PAGE_LIMIT {
        return Err(format!("detail pages cannot exceed {PAGE_LIMIT}²"));
    }
    let sizes: Vec<[u32; 2]> = pieces
        .iter()
        .map(|piece| [piece.region.crop.width, piece.region.crop.height])
        .collect();
    let packing = pack(&sizes, side)?;
    let (width, height) = (packing.width, packing.height);
    let mut atlas = Atlas {
        width,
        height,
        parts: Vec::with_capacity(pieces.len()),
        base: Vec::new(),
        normal: Vec::new(),
        roughness: Vec::new(),
    };
    let page_bytes = (width * height * 4) as usize;
    let mut linear = vec![
        [
            vec![0u8; page_bytes],
            vec![0u8; page_bytes],
            vec![0u8; page_bytes]
        ];
        packing.pages as usize
    ];
    for (piece, &(page, at)) in pieces.into_iter().zip(&packing.places) {
        let crop = piece.region.crop;
        let row = (crop.width * 4) as usize;
        for (kind, source) in [
            &piece.region.base,
            &piece.region.normal,
            &piece.region.roughness,
        ]
        .into_iter()
        .enumerate()
        {
            let target = &mut linear[page as usize][kind];
            for y in 0..crop.height {
                let from = (y * crop.width * 4) as usize;
                let to = (((at[1] + y) * width + at[0]) * 4) as usize;
                target[to..to + row].copy_from_slice(&source[from..from + row]);
            }
        }
        atlas.parts.push(Placed {
            index: piece.index,
            node: piece.node,
            density: piece.region.size,
            crop,
            page,
            at,
            input_hash: piece.input_hash,
            uv_set: piece.region.uv_set,
        });
    }
    for maps in linear {
        for (kind, bytes) in Kind::ALL.into_iter().zip(maps) {
            let texture = Texture {
                format: kind.plain(),
                width,
                height,
                levels: chain(kind, width, height, &bytes, LEVELS),
            };
            atlas.pages_mut(kind).push(texture);
        }
    }
    Ok(atlas)
}

impl Atlas {
    pub fn pages(&self) -> u32 {
        self.base.len() as u32
    }

    pub fn pages_of(&self, kind: Kind) -> &[Texture] {
        match kind {
            Kind::Base => &self.base,
            Kind::Normal => &self.normal,
            Kind::Roughness => &self.roughness,
        }
    }

    fn pages_mut(&mut self, kind: Kind) -> &mut Vec<Texture> {
        match kind {
            Kind::Base => &mut self.base,
            Kind::Normal => &mut self.normal,
            Kind::Roughness => &mut self.roughness,
        }
    }

    pub fn schema(&self) -> u64 {
        if self.parts.iter().any(|part| part.uv_set != 0) {
            SCHEMA_UV1
        } else {
            SCHEMA
        }
    }

    pub fn transform(&self, part: &Placed) -> [f32; 4] {
        let width = f64::from(self.width);
        let height = f64::from(self.height);
        let density = f64::from(part.density);
        [
            (density / width) as f32,
            (density / height) as f32,
            ((f64::from(part.at[0]) - f64::from(part.crop.x)) / width) as f32,
            ((f64::from(part.at[1]) - f64::from(part.crop.y)) / height) as f32,
        ]
    }

    pub fn compressed(&self) -> bool {
        Kind::ALL.iter().all(|&kind| {
            self.pages_of(kind)
                .iter()
                .all(|page| page.format == kind.packed())
        })
    }

    pub fn compress(&self) -> Result<Self, String> {
        let mut out = Self {
            width: self.width,
            height: self.height,
            parts: self.parts.clone(),
            base: Vec::new(),
            normal: Vec::new(),
            roughness: Vec::new(),
        };
        for kind in Kind::ALL {
            for page in self.pages_of(kind) {
                let packed = page.compress(kind)?;
                out.pages_mut(kind).push(packed);
            }
        }
        Ok(out)
    }

    pub fn decode(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            parts: self.parts.clone(),
            base: self.base.iter().map(Texture::decode).collect(),
            normal: self.normal.iter().map(Texture::decode).collect(),
            roughness: self.roughness.iter().map(Texture::decode).collect(),
        }
    }

    pub fn vram_bytes(&self) -> u64 {
        Kind::ALL
            .iter()
            .flat_map(|&kind| self.pages_of(kind))
            .map(Texture::bytes)
            .sum()
    }

    pub fn decoded_vram_bytes(&self) -> u64 {
        Kind::ALL
            .iter()
            .flat_map(|&kind| self.pages_of(kind))
            .map(|page| {
                (0..page.levels.len() as u32)
                    .map(|level| {
                        let [width, height] = level_size(page.width, page.height, level);
                        u64::from(width) * u64::from(height) * 4
                    })
                    .sum::<u64>()
            })
            .sum()
    }

    pub fn part_bytes(&self, part: &Placed) -> u64 {
        Kind::ALL
            .iter()
            .filter_map(|&kind| self.pages_of(kind).first())
            .map(|page| {
                (0..page.levels.len() as u32)
                    .map(|level| {
                        let width = (part.crop.width >> level).max(1);
                        let height = (part.crop.height >> level).max(1);
                        page.format.level_bytes(width, height) as u64
                    })
                    .sum::<u64>()
            })
            .sum()
    }

    fn check(&self) -> Result<(), String> {
        let pages = self.base.len();
        if pages == 0 || self.normal.len() != pages || self.roughness.len() != pages {
            return Err("detail atlas needs the same pages for every map".into());
        }
        for kind in Kind::ALL {
            for page in self.pages_of(kind) {
                page.check()?;
                if page.width != self.width
                    || page.height != self.height
                    || ![kind.plain(), kind.packed()].contains(&page.format)
                    || page.levels.len() != self.base[0].levels.len()
                {
                    return Err(format!(
                        "detail {} page differs from the atlas",
                        kind.name()
                    ));
                }
            }
        }
        for part in &self.parts {
            if part.page as usize >= pages
                || part.at[0] + part.crop.width > self.width
                || part.at[1] + part.crop.height > self.height
                || part.crop.x + part.crop.width > part.density
                || part.crop.y + part.crop.height > part.density
            {
                return Err(format!("detail part {} lies outside its page", part.node));
            }
            if part.uv_set > 1 {
                return Err(format!(
                    "detail part {} names UV set {}; only TEXCOORD_0 and TEXCOORD_1 bake",
                    part.node, part.uv_set
                ));
            }
        }
        Ok(())
    }
}

pub fn psnr(plain: &Atlas, packed: &Atlas, kind: Kind) -> Option<f64> {
    let decoded = packed.decode();
    let mut error = 0.0f64;
    let mut count = 0u64;
    for part in &plain.parts {
        let a = &plain.pages_of(kind)[part.page as usize].levels[0];
        let b = &decoded.pages_of(kind)[part.page as usize].levels[0];
        for y in part.at[1]..part.at[1] + part.crop.height {
            for x in part.at[0]..part.at[0] + part.crop.width {
                let at = ((y * plain.width + x) * 4) as usize;
                for &channel in kind.channels() {
                    let difference = f64::from(a[at + channel]) - f64::from(b[at + channel]);
                    error += difference * difference;
                    count += 1;
                }
            }
        }
    }
    if count == 0 || error == 0.0 {
        return None;
    }
    let mean = error / count as f64;
    Some(10.0 * (255.0f64 * 255.0 / mean).log10())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub input_hash: String,
    pub padding: u32,
    pub psnr: [Option<f64>; 3],
    pub total_bytes: u64,
    pub vram_bytes: u64,
}

fn file_name(page: usize, kind: Kind) -> String {
    format!("page-{page:03}-{}.ktx2", kind.name())
}

pub fn file_names(pages: usize) -> Vec<String> {
    (0..pages)
        .flat_map(|page| Kind::ALL.map(|kind| file_name(page, kind)))
        .collect()
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub fn write(
    path: &Path,
    atlas: &Atlas,
    input_hash: &str,
    padding: u32,
    psnr: [Option<f64>; 3],
) -> Result<Summary, String> {
    atlas.check()?;
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut total = 0u64;
    for page in 0..atlas.pages() as usize {
        for kind in Kind::ALL {
            let name = file_name(page, kind);
            let bytes = super::ktx2::encode(&atlas.pages_of(kind)[page])?;
            fs::write(path.join(&name), &bytes).map_err(|e| e.to_string())?;
            total += bytes.len() as u64;
            files.push(json!({"path": name, "bytes": bytes.len(), "sha256": digest(&bytes)}));
        }
    }
    let parts: Vec<Value> = atlas
        .parts
        .iter()
        .map(|part| {
            let transform = atlas.transform(part);
            json!({
                "part": part.index,
                "node": part.node,
                "size": part.density,
                "crop": [part.crop.x, part.crop.y, part.crop.width, part.crop.height],
                "page": part.page,
                "at": part.at,
                "scale": [transform[0], transform[1]],
                "offset": [transform[2], transform[3]],
                "bytes": atlas.part_bytes(part),
                "input_hash": part.input_hash,
                "uv_set": part.uv_set,
            })
        })
        .collect();
    let formats: Vec<&str> = Kind::ALL
        .iter()
        .map(|&kind| atlas.pages_of(kind)[0].format.name())
        .collect();
    let psnr = psnr.map(|value| value.map(round));
    let summary = Summary {
        input_hash: input_hash.to_owned(),
        padding,
        psnr,
        total_bytes: total,
        vram_bytes: atlas.vram_bytes(),
    };
    let manifest = serde_json::to_vec_pretty(&json!({
        "schema_version": atlas.schema(),
        "format": KIND,
        "input_hash": input_hash,
        "padding": padding,
        "align": ALIGN,
        "page": {
            "width": atlas.width,
            "height": atlas.height,
            "count": atlas.pages(),
            "levels": atlas.base[0].levels.len(),
        },
        "maps": {"base": formats[0], "normal": formats[1], "roughness": formats[2]},
        "total_bytes": total,
        "vram_bytes": summary.vram_bytes,
        "decoded_vram_bytes": atlas.decoded_vram_bytes(),
        "psnr": {
            "base": psnr[0],
            "normal": psnr[1],
            "roughness": psnr[2],
        },
        "files": files,
        "parts": parts,
    }))
    .map_err(|e| e.to_string())?;
    fs::write(path.join("manifest.json"), &manifest).map_err(|e| e.to_string())?;
    fs::write(path.join("manifest.sha256"), digest(&manifest)).map_err(|e| e.to_string())?;
    Ok(summary)
}

fn manifest_bytes(files: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Value, String> {
    let fetch = |name: &str| files(name).ok_or_else(|| format!("missing detail file {name}"));
    let manifest = fetch("manifest.json")?;
    let expected = String::from_utf8(fetch("manifest.sha256")?).map_err(|e| e.to_string())?;
    if expected != digest(&manifest) {
        return Err("detail manifest checksum differs".into());
    }
    serde_json::from_slice(&manifest).map_err(|e| e.to_string())
}

fn manifest(path: &Path) -> Result<Value, String> {
    manifest_bytes(&|name| fs::read(path.join(name)).ok())
}

fn number(value: &Value, name: &str) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("detail {name} is missing or invalid"))
}

fn pair(value: &Value, name: &str) -> Result<[u32; 2], String> {
    Ok([number(&value[0], name)?, number(&value[1], name)?])
}

pub fn read(path: &Path) -> Result<(Summary, Atlas), String> {
    read_from(&|name| fs::read(path.join(name)).ok())
}

pub fn read_from(files: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<(Summary, Atlas), String> {
    let value = manifest_bytes(files)?;
    let fetch = |name: &str| files(name).ok_or_else(|| format!("missing detail file {name}"));
    let schema = value["schema_version"].as_u64();
    if !matches!(schema, Some(SCHEMA | SCHEMA_UV1)) || value["format"] != KIND {
        return Err("unsupported detail atlas".into());
    }
    let page = &value["page"];
    let width = number(&page["width"], "page width")?;
    let height = number(&page["height"], "page height")?;
    let count = number(&page["count"], "page count")? as usize;
    let entries = value["files"]
        .as_array()
        .ok_or("detail files are missing")?;
    let names = file_names(count);
    if entries.len() != names.len() {
        return Err("detail atlas lists the wrong files".into());
    }
    let mut atlas = Atlas {
        width,
        height,
        parts: Vec::new(),
        base: Vec::new(),
        normal: Vec::new(),
        roughness: Vec::new(),
    };
    let mut total = 0u64;
    for (index, (entry, name)) in entries.iter().zip(&names).enumerate() {
        if entry["path"] != name.as_str() {
            return Err("detail page name differs".into());
        }
        let bytes = fetch(name)?;
        if entry["bytes"].as_u64() != Some(bytes.len() as u64)
            || entry["sha256"].as_str() != Some(digest(&bytes).as_str())
        {
            return Err(format!("detail {name} checksum differs"));
        }
        total += bytes.len() as u64;
        let texture = super::ktx2::decode(&bytes)?;
        atlas.pages_mut(Kind::ALL[index % 3]).push(texture);
    }
    for part in value["parts"]
        .as_array()
        .ok_or("detail parts are missing")?
    {
        let crop = &part["crop"];
        atlas.parts.push(Placed {
            index: number(&part["part"], "part")?,
            node: part["node"]
                .as_str()
                .ok_or("detail node is missing")?
                .to_owned(),
            density: number(&part["size"], "size")?,
            crop: Crop {
                x: number(&crop[0], "crop")?,
                y: number(&crop[1], "crop")?,
                width: number(&crop[2], "crop")?,
                height: number(&crop[3], "crop")?,
            },
            page: number(&part["page"], "page")?,
            at: pair(&part["at"], "placement")?,
            input_hash: part["input_hash"]
                .as_str()
                .ok_or("detail input hash is missing")?
                .to_owned(),
            uv_set: match &part["uv_set"] {
                Value::Null => 0,
                set => number(set, "UV set")?,
            },
        });
    }
    atlas.check()?;
    if schema != Some(atlas.schema()) {
        return Err("detail atlas schema differs from its parts' UV sets".into());
    }
    if total != value["total_bytes"].as_u64().unwrap_or(u64::MAX) {
        return Err("detail total bytes differ".into());
    }
    let psnr = Kind::ALL.map(|kind| value["psnr"][kind.name()].as_f64());
    Ok((
        Summary {
            input_hash: value["input_hash"]
                .as_str()
                .ok_or("detail input hash is missing")?
                .to_owned(),
            padding: number(&value["padding"], "padding")?,
            psnr,
            total_bytes: total,
            vram_bytes: atlas.vram_bytes(),
        },
        atlas,
    ))
}

pub fn covered_crop(size: u32, base: &[u8]) -> Option<Crop> {
    let mut low = [u32::MAX; 2];
    let mut high = [0u32; 2];
    for (index, texel) in base.chunks_exact(4).enumerate() {
        if texel[3] == 0 {
            continue;
        }
        let at = [index as u32 % size, index as u32 / size];
        for axis in 0..2 {
            low[axis] = low[axis].min(at[axis]);
            high[axis] = high[axis].max(at[axis]);
        }
    }
    if low[0] > high[0] {
        return None;
    }
    let start = low.map(|value| value / ALIGN * ALIGN);
    let end = high.map(|value| ((value + 1).div_ceil(ALIGN) * ALIGN).min(size));
    Some(Crop {
        x: start[0],
        y: start[1],
        width: end[0] - start[0],
        height: end[1] - start[1],
    })
}

pub fn cut(maps: &super::Maps, crop: Crop) -> Region {
    let take = |map: &[u8]| {
        let mut out = Vec::with_capacity((crop.width * crop.height * 4) as usize);
        for y in crop.y..crop.y + crop.height {
            let from = ((y * maps.size + crop.x) * 4) as usize;
            out.extend_from_slice(&map[from..from + (crop.width * 4) as usize]);
        }
        out
    };
    Region {
        size: maps.size,
        crop,
        uv_set: 0,
        base: take(&maps.base),
        roughness: take(&maps.roughness),
        normal: take(&maps.normal),
    }
}

pub fn load(path: &Path) -> Result<Atlas, String> {
    let value = manifest(path)?;
    match value["schema_version"].as_u64() {
        Some(SCHEMA | SCHEMA_UV1) => read(path).map(|(_, atlas)| atlas),
        Some(1) => {
            let parts = value["parts"]
                .as_array()
                .ok_or("detail parts are missing")?;
            let mut pieces = Vec::with_capacity(parts.len());
            for (position, part) in parts.iter().enumerate() {
                let folder = part["path"].as_str().ok_or("detail part path is missing")?;
                if folder.is_empty() || folder.contains(['/', '\\']) || folder.starts_with('.') {
                    return Err(format!("detail part path {folder:?} is not a part folder"));
                }
                let (hash, maps) = super::read(&path.join(folder))?;
                if part["input_hash"].as_str() != Some(hash.as_str())
                    || part["size"].as_u64() != Some(u64::from(maps.size))
                {
                    return Err(format!("detail {folder} differs from the manifest"));
                }
                let crop = covered_crop(maps.size, &maps.base)
                    .ok_or_else(|| format!("detail {folder} covers nothing"))?;
                pieces.push(Piece {
                    index: folder
                        .strip_prefix("part-")
                        .and_then(|number| number.parse().ok())
                        .unwrap_or(position as u32),
                    node: part["node"].as_str().unwrap_or_default().to_owned(),
                    input_hash: hash,
                    region: cut(&maps, crop),
                });
            }
            assemble(pieces, PAGE)
        }
        _ => Err("unsupported detail folder".into()),
    }
}

pub fn hash(pieces: &[(u32, &str, &str)], page: u32, padding: u32) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"pito-engine-detail-atlas-v2");
    h.update(env!("CARGO_PKG_VERSION"));
    h.update(include_str!("atlas.rs"));
    h.update(include_str!("ktx2.rs"));
    h.update(page.to_le_bytes());
    h.update(padding.to_le_bytes());
    for (index, node, input) in pieces {
        h.update(index.to_le_bytes());
        h.update((node.len() as u64).to_le_bytes());
        h.update(node.as_bytes());
        h.update(input.as_bytes());
    }
    h.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(count: usize, seed: u32) -> Vec<[u32; 2]> {
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 16) % 12 + 1
        };
        (0..count)
            .map(|_| [next() * ALIGN, next() * ALIGN])
            .collect()
    }

    fn overlap(a: ([u32; 2], [u32; 2]), b: ([u32; 2], [u32; 2])) -> bool {
        a.0[0] < b.0[0] + b.1[0]
            && b.0[0] < a.0[0] + a.1[0]
            && a.0[1] < b.0[1] + b.1[1]
            && b.0[1] < a.0[1] + a.1[1]
    }

    #[test]
    fn packing_places_every_crop_once_without_overlap_and_the_same_way_each_time() {
        let sizes = sizes(60, 7);
        let packing = pack(&sizes, 256).unwrap();
        assert_eq!(packing, pack(&sizes, 256).unwrap());
        assert!(packing.pages > 1);
        assert_eq!((packing.width, packing.height), (256, 256));
        for (index, (&(page, at), size)) in packing.places.iter().zip(&sizes).enumerate() {
            assert!(page < packing.pages);
            assert!(at[0] + size[0] <= 256 && at[1] + size[1] <= 256);
            assert!(at[0].is_multiple_of(ALIGN) && at[1].is_multiple_of(ALIGN));
            for (other, (&(other_page, other_at), other_size)) in
                packing.places.iter().zip(&sizes).enumerate()
            {
                if other != index && other_page == page {
                    assert!(
                        !overlap((at, *size), (other_at, *other_size)),
                        "{index} and {other} overlap"
                    );
                }
            }
        }
        let area: u32 = sizes.iter().map(|size| size[0] * size[1]).sum();
        assert!(
            area * 10 > packing.pages * 256 * 256 * 6,
            "packing wastes too much"
        );
        let one = pack(&[[64, 32], [32, 32], [16, 16]], 2048).unwrap();
        assert_eq!((one.pages, one.width, one.height), (1, 128, 32));
        assert_eq!(one.places, vec![(0, [0, 0]), (0, [64, 0]), (0, [96, 0])]);
        let tall = pack(&[[64, 64], [64, 64], [64, 64], [64, 64], [64, 64]], 2048).unwrap();
        assert_eq!((tall.width, tall.height), (512, 64));
        assert!(pack(&[[512, 16]], 256).is_err());
        assert!(pack(&[[0, 16]], 256).is_err());
        assert!(pack(&[[16, 16]], 96).is_err());
    }

    fn region(width: u32, height: u32, seed: u32) -> Region {
        let mut state = seed;
        let mut map = |alpha: bool| {
            (0..width * height)
                .flat_map(|index| {
                    let x = index % width;
                    let y = index / width;
                    state = state.wrapping_mul(22_695_477).wrapping_add(1);
                    let noise = (state >> 24) as u8 / 16;
                    let value = |c: u32| ((x * (3 + c) + y * (5 + c)) % 200) as u8 + noise;
                    [
                        value(0),
                        value(1),
                        value(2),
                        if alpha { 255 } else { value(3) },
                    ]
                })
                .collect::<Vec<u8>>()
        };
        Region {
            size: 256,
            crop: Crop {
                x: 32,
                y: 64,
                width,
                height,
            },
            uv_set: 0,
            base: map(true),
            roughness: map(true),
            normal: map(true),
        }
    }

    fn atlas() -> Atlas {
        assemble(
            vec![
                Piece {
                    index: 4,
                    node: "planks".into(),
                    input_hash: "a".repeat(64),
                    region: region(48, 32, 1),
                },
                Piece {
                    index: 9,
                    node: "grime".into(),
                    input_hash: "b".repeat(64),
                    region: region(32, 64, 2),
                },
            ],
            128,
        )
        .unwrap()
    }

    #[test]
    fn the_encoder_gives_the_same_blocks_each_time_and_for_any_split() {
        let plain = atlas();
        let first = plain.compress().unwrap();
        assert_eq!(first, plain.compress().unwrap());
        for kind in Kind::ALL {
            let page = &plain.pages_of(kind)[0];
            let [width, height] = [page.width, page.height];
            let mut whole = vec![0; kind.packed().level_bytes(width, height)];
            encode::compress_rgba8(
                kind.packed().variant().unwrap(),
                &page.levels[0],
                &mut whole,
                width,
                height,
                width * 4,
            );
            assert_eq!(first.pages_of(kind)[0].levels[0], whole);
            assert_eq!(first.pages_of(kind)[0].format, kind.packed());
        }
        let levels = &first.base[0].levels;
        assert_eq!(levels.len() as u32, LEVELS);
        assert_eq!(levels[4].len(), Format::Bc7Srgb.level_bytes(8, 4));
    }

    #[test]
    fn block_compression_keeps_the_maps_close() {
        let plain = atlas();
        let packed = plain.compress().unwrap();
        for kind in Kind::ALL {
            let quality = psnr(&plain, &packed, kind).unwrap();
            assert!(quality > 30.0, "{} psnr {quality}", kind.name());
        }
        let decoded = packed.decode();
        assert_eq!(decoded.normal[0].format, Format::Rgba8);
        assert_eq!(decoded.base[0].format, Format::Rgba8Srgb);
        assert!(
            decoded.roughness[0].levels[0]
                .chunks_exact(4)
                .all(|t| t[1] == 0 && t[2] == 0 && t[3] == 255)
        );
        assert_eq!(decoded.decoded_vram_bytes(), plain.vram_bytes());
    }

    #[test]
    fn the_manifest_and_pages_round_trip_and_refuse_a_changed_byte() {
        let plain = atlas();
        assert_eq!((plain.width, plain.height, plain.pages()), (128, 64, 1));
        let packed = plain.compress().unwrap();
        let psnr = Kind::ALL.map(|kind| psnr(&plain, &packed, kind));
        let root = Path::new("tmp").join(format!("detail-atlas-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let written = write(&root, &packed, "c0ffee", 8, psnr).unwrap();
        assert!(write(&root, &packed, "c0ffee", 8, psnr).is_err());
        let (summary, read_back) = read(&root).unwrap();
        assert_eq!(read_back, packed);
        let mut stored = std::collections::BTreeMap::new();
        for name in ["manifest.json", "manifest.sha256"]
            .into_iter()
            .map(str::to_owned)
            .chain(file_names(read_back.pages() as usize))
        {
            stored.insert(name.clone(), fs::read(root.join(&name)).unwrap());
        }
        let from_bytes = read_from(&|name| stored.get(name).cloned()).unwrap();
        assert_eq!(from_bytes.0, summary);
        assert_eq!(from_bytes.1, read_back);
        let mut changed = stored.clone();
        let page_name = file_names(1)[0].clone();
        let page_bytes = changed.get_mut(&page_name).unwrap();
        let last = page_bytes.len() - 1;
        page_bytes[last] ^= 1;
        assert!(read_from(&|name| changed.get(name).cloned()).is_err());
        assert_eq!(summary.input_hash, "c0ffee");
        assert_eq!(summary.total_bytes, written.total_bytes);
        assert_eq!(summary.vram_bytes, packed.vram_bytes());
        assert_eq!(summary.psnr, written.psnr);
        assert!(summary.psnr.iter().all(Option::is_some));
        assert_eq!(load(&root).unwrap(), packed);
        for (name, kind) in file_names(1).iter().zip(Kind::ALL) {
            let bytes = fs::read(root.join(name)).unwrap();
            let reader = ::ktx2::Reader::new(&bytes[..]).unwrap();
            let header = reader.header();
            assert_eq!(header.pixel_width, 128);
            assert_eq!(header.pixel_height, 64);
            assert_eq!(header.level_count, LEVELS);
            assert_eq!(header.format.map(|f| f.value()), Some(kind.packed().vk()));
            let levels: Vec<&[u8]> = reader.levels().map(|level| level.data).collect();
            assert_eq!(levels.len(), packed.pages_of(kind)[0].levels.len());
            for (level, data) in levels.iter().enumerate() {
                assert_eq!(*data, &packed.pages_of(kind)[0].levels[level][..]);
            }
            assert!(!reader.dfd_blocks().is_empty());
        }
        let page = root.join(&file_names(1)[1]);
        let mut bytes = fs::read(&page).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&page, bytes).unwrap();
        assert!(read(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transforms_map_a_tile_uv_onto_its_place_in_the_page() {
        let plain = atlas();
        for part in &plain.parts {
            let [su, sv, ou, ov] = plain.transform(part);
            let corner = [
                part.crop.x as f32 / part.density as f32,
                part.crop.y as f32 / part.density as f32,
            ];
            let placed = [corner[0] * su + ou, corner[1] * sv + ov];
            assert!((placed[0] * plain.width as f32 - part.at[0] as f32).abs() < 1e-3);
            assert!((placed[1] * plain.height as f32 - part.at[1] as f32).abs() < 1e-3);
        }
    }

    #[test]
    fn the_manifest_records_each_parts_uv_set_and_old_readers_refuse_a_second_set() {
        let plain = atlas();
        assert_eq!(plain.schema(), SCHEMA);
        assert!(plain.parts.iter().all(|part| part.uv_set == 0));
        let mut second = region(48, 32, 1);
        second.uv_set = 1;
        let mixed = assemble(
            vec![
                Piece {
                    index: 4,
                    node: "planks".into(),
                    input_hash: "a".repeat(64),
                    region: second,
                },
                Piece {
                    index: 9,
                    node: "grime".into(),
                    input_hash: "b".repeat(64),
                    region: region(32, 64, 2),
                },
            ],
            128,
        )
        .unwrap();
        assert_eq!(mixed.schema(), SCHEMA_UV1);
        let sets: Vec<(&str, u32)> = mixed
            .parts
            .iter()
            .map(|part| (part.node.as_str(), part.uv_set))
            .collect();
        assert!(sets.contains(&("planks", 1)) && sets.contains(&("grime", 0)));
        assert_eq!(plain.base, mixed.base);
        let root = Path::new("tmp").join(format!("detail-atlas-uv-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let first = root.join("first");
        let second = root.join("second");
        write(&first, &plain, "c0ffee", 8, [None; 3]).unwrap();
        write(&second, &mixed, "c0ffee", 8, [None; 3]).unwrap();
        let text = |path: &Path| -> Value {
            serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap()
        };
        let one = text(&first);
        assert_eq!(one["schema_version"], SCHEMA);
        assert!(
            one["parts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|part| part["uv_set"] == 0)
        );
        let two = text(&second);
        assert_eq!(two["schema_version"], SCHEMA_UV1);
        assert_eq!(read(&second).unwrap().1, mixed);
        let mut second_files = std::collections::BTreeMap::new();
        for entry in fs::read_dir(&second).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            second_files.insert(name, fs::read(entry.path()).unwrap());
        }
        assert_eq!(
            read_from(&|name| second_files.get(name).cloned())
                .unwrap()
                .1,
            mixed
        );
        assert_eq!(load(&second).unwrap(), mixed);
        assert_eq!(read(&first).unwrap().1, plain);

        let rewrite = |path: &Path, value: &Value| {
            let bytes = serde_json::to_vec_pretty(value).unwrap();
            fs::write(path.join("manifest.json"), &bytes).unwrap();
            fs::write(path.join("manifest.sha256"), digest(&bytes)).unwrap();
        };
        let mut unmarked = one.clone();
        for part in unmarked["parts"].as_array_mut().unwrap() {
            part.as_object_mut().unwrap().remove("uv_set");
        }
        rewrite(&first, &unmarked);
        assert_eq!(read(&first).unwrap().1, plain);
        let mut downgraded = two.clone();
        downgraded["schema_version"] = json!(SCHEMA);
        rewrite(&second, &downgraded);
        assert!(read(&second).unwrap_err().contains("schema"));
        let mut third = two.clone();
        third["parts"][0]["uv_set"] = json!(2);
        rewrite(&second, &third);
        assert!(read(&second).unwrap_err().contains("UV set 2"));
        fs::remove_dir_all(root).unwrap();
    }
}
