use crate::frame::{Frame, InstanceSurface};
use crate::maps::{ContentFormat, MipRegion, check_write, mip_count, mip_regions};
use pfx_gpu::floor::TEXTURE_SIDE;
use pfx_gpu::pace::{Pacer, Slice, Stats, Turned, Work};
use pfx_gpu::wgpu;
use std::fmt;

pub const DEFAULT_BLEED_LEVEL: u32 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtlasError {
    BadSide {
        width: u32,
        height: u32,
        limit: u32,
    },
    BadLevel {
        level: u32,
        largest: u32,
    },
    EmptyImage,
    BadTexels {
        width: u32,
        height: u32,
        expected: usize,
        got: usize,
    },
    TooLarge {
        cell: [u32; 2],
        atlas: [u32; 2],
    },
    Full {
        cell: [u32; 2],
        atlas: [u32; 2],
        images: usize,
        used: u64,
    },
}

impl fmt::Display for AtlasError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSide {
                width,
                height,
                limit,
            } => write!(
                out,
                "a {width}x{height} atlas must have sides from 1 to {limit}"
            ),
            Self::BadLevel { level, largest } => write!(
                out,
                "gutters for mip level {level} do not fit this atlas, whose largest level is {largest}"
            ),
            Self::EmptyImage => write!(out, "an atlas image has no texels"),
            Self::BadTexels {
                width,
                height,
                expected,
                got,
            } => write!(
                out,
                "a {width}x{height} atlas image needs {expected} bytes, got {got}"
            ),
            Self::TooLarge { cell, atlas } => write!(
                out,
                "an image with its gutters takes {}x{} texels, more than the {}x{} atlas",
                cell[0], cell[1], atlas[0], atlas[1]
            ),
            Self::Full {
                cell,
                atlas,
                images,
                used,
            } => write!(
                out,
                "the {}x{} atlas is full: {images} images hold {used} texels and no shelf takes another {}x{} cell",
                atlas[0], atlas[1], cell[0], cell[1]
            ),
        }
    }
}

impl std::error::Error for AtlasError {}

impl From<AtlasError> for String {
    fn from(error: AtlasError) -> Self {
        error.to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Shelf {
    y: u32,
    height: u32,
    cursor: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packer {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
}

impl Packer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            shelves: Vec::new(),
        }
    }

    pub fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub fn place(&mut self, width: u32, height: u32) -> Option<[u32; 2]> {
        if width == 0 || height == 0 || width > self.width || height > self.height {
            return None;
        }
        let best = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, shelf)| shelf.height >= height && shelf.cursor + width <= self.width)
            .min_by_key(|(index, shelf)| (shelf.height, *index))
            .map(|(index, _)| index);
        if let Some(index) = best {
            let shelf = &mut self.shelves[index];
            let at = [shelf.cursor, shelf.y];
            shelf.cursor += width;
            return Some(at);
        }
        let y = self
            .shelves
            .last()
            .map_or(0, |shelf| shelf.y + shelf.height);
        if y.checked_add(height).is_none_or(|end| end > self.height) {
            return None;
        }
        self.shelves.push(Shelf {
            y,
            height,
            cursor: width,
        });
        Some([0, y])
    }
}

pub fn texture_bytes(width: u32, height: u32, format: ContentFormat) -> u64 {
    (0..mip_count(width, height))
        .map(|level| {
            u64::from((width >> level).max(1))
                * u64::from((height >> level).max(1))
                * u64::from(format.texel_bytes())
        })
        .sum()
}

pub fn gutter(level: u32) -> u32 {
    1 << level
}

pub fn cell_size(image: [u32; 2], level: u32) -> [u32; 2] {
    let block = gutter(level);
    image.map(|side| side.div_ceil(block) * block + 2 * block)
}

pub fn cards_that_fit(atlas: [u32; 2], image: [u32; 2], level: u32) -> u32 {
    let cell = cell_size(image, level);
    (atlas[0] / cell[0]) * (atlas[1] / cell[1])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub rect: [u32; 4],
    pub cell: [u32; 4],
}

impl Placement {
    pub fn uv_offset(&self, atlas: [u32; 2]) -> [f32; 2] {
        [
            self.rect[0] as f32 / atlas[0] as f32,
            self.rect[1] as f32 / atlas[1] as f32,
        ]
    }

    pub fn uv_scale(&self, atlas: [u32; 2]) -> [f32; 2] {
        [
            self.rect[2] as f32 / atlas[0] as f32,
            self.rect[3] as f32 / atlas[1] as f32,
        ]
    }

    pub fn crop(&self, atlas: [u32; 2]) -> [f32; 4] {
        let [x, y, width, height] = self.rect;
        [
            x as f32 / atlas[0] as f32,
            y as f32 / atlas[1] as f32,
            (x + width) as f32 / atlas[0] as f32,
            (y + height) as f32 / atlas[1] as f32,
        ]
    }

    pub fn surface(&self, atlas: [u32; 2]) -> InstanceSurface {
        InstanceSurface {
            uv_offset: self.uv_offset(atlas),
            uv_scale: self.uv_scale(atlas),
            crop: self.crop(atlas),
            ..InstanceSurface::default()
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AtlasImage<'a> {
    pub width: u32,
    pub height: u32,
    pub texels: &'a [u8],
}

pub fn padded_cell(
    image: &AtlasImage<'_>,
    format: ContentFormat,
    level: u32,
) -> Result<Vec<u8>, AtlasError> {
    if image.width == 0 || image.height == 0 {
        return Err(AtlasError::EmptyImage);
    }
    let texel = format.texel_bytes() as usize;
    let expected = image.width as usize * image.height as usize * texel;
    if image.texels.len() != expected {
        return Err(AtlasError::BadTexels {
            width: image.width,
            height: image.height,
            expected,
            got: image.texels.len(),
        });
    }
    let block = gutter(level);
    let [cell_width, cell_height] = cell_size([image.width, image.height], level);
    let mut cell = Vec::with_capacity(cell_width as usize * cell_height as usize * texel);
    for row in 0..cell_height {
        let y = row.saturating_sub(block).min(image.height - 1) as usize;
        let source =
            &image.texels[y * image.width as usize * texel..][..image.width as usize * texel];
        for column in 0..cell_width {
            let x = column.saturating_sub(block).min(image.width - 1) as usize;
            cell.extend_from_slice(&source[x * texel..][..texel]);
        }
    }
    Ok(cell)
}

pub struct ContentAtlas {
    packer: Packer,
    format: ContentFormat,
    level: u32,
    placements: Vec<Placement>,
    pending: Vec<(usize, Vec<u8>)>,
    used: u64,
}

impl ContentAtlas {
    pub fn new(
        width: u32,
        height: u32,
        format: ContentFormat,
        level: u32,
    ) -> Result<Self, AtlasError> {
        if width == 0 || height == 0 || width > TEXTURE_SIDE || height > TEXTURE_SIDE {
            return Err(AtlasError::BadSide {
                width,
                height,
                limit: TEXTURE_SIDE,
            });
        }
        let largest = mip_count(width, height) - 1;
        if level > largest || 4 * gutter(level) > width.min(height) {
            return Err(AtlasError::BadLevel { level, largest });
        }
        Ok(Self {
            packer: Packer::new(width, height),
            format,
            level,
            placements: Vec::new(),
            pending: Vec::new(),
            used: 0,
        })
    }

    pub fn size(&self) -> [u32; 2] {
        self.packer.size()
    }

    pub fn format(&self) -> ContentFormat {
        self.format
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    pub fn len(&self) -> usize {
        self.placements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn vram_bytes(&self) -> u64 {
        let [width, height] = self.size();
        texture_bytes(width, height, self.format)
    }

    pub fn placement(&self, id: usize) -> Option<&Placement> {
        self.placements.get(id)
    }

    pub fn crop(&self, id: usize) -> Option<[f32; 4]> {
        self.placement(id).map(|place| place.crop(self.size()))
    }

    pub fn surface(&self, id: usize) -> Option<InstanceSurface> {
        self.placement(id).map(|place| place.surface(self.size()))
    }

    pub fn add(&mut self, image: &AtlasImage<'_>) -> Result<usize, AtlasError> {
        let cell = padded_cell(image, self.format, self.level)?;
        let [cell_width, cell_height] = cell_size([image.width, image.height], self.level);
        let atlas = self.size();
        if cell_width > atlas[0] || cell_height > atlas[1] {
            return Err(AtlasError::TooLarge {
                cell: [cell_width, cell_height],
                atlas,
            });
        }
        let Some([x, y]) = self.packer.place(cell_width, cell_height) else {
            return Err(AtlasError::Full {
                cell: [cell_width, cell_height],
                atlas,
                images: self.placements.len(),
                used: self.used,
            });
        };
        let block = gutter(self.level);
        let id = self.placements.len();
        self.placements.push(Placement {
            rect: [x + block, y + block, image.width, image.height],
            cell: [x, y, cell_width, cell_height],
        });
        self.pending.push((id, cell));
        self.used += u64::from(cell_width) * u64::from(cell_height);
        Ok(id)
    }

    pub fn add_all(&mut self, images: &[AtlasImage<'_>]) -> Result<Vec<usize>, AtlasError> {
        let mut order: Vec<usize> = (0..images.len()).collect();
        order.sort_by_key(|&index| (std::cmp::Reverse(images[index].height), index));
        let saved = (
            self.packer.clone(),
            self.placements.len(),
            self.pending.len(),
            self.used,
        );
        let first = self.placements.len();
        let mut ids = vec![0; images.len()];
        for index in order {
            match self.add(&images[index]) {
                Ok(id) => ids[index] = id,
                Err(error) => {
                    self.packer = saved.0;
                    self.placements.truncate(saved.1);
                    self.pending.truncate(saved.2);
                    self.used = saved.3;
                    return Err(error);
                }
            }
        }
        debug_assert!(ids.iter().all(|&id| id >= first));
        Ok(ids)
    }

    pub fn allocate(&self, frame: &Frame, slot: usize) -> Result<(), String> {
        let [width, height] = self.size();
        frame.set_content(slot, width, height, self.format)
    }

    fn check_target(&self, frame: &Frame, slot: usize) -> Result<(), String> {
        let atlas = self.size();
        let content = frame.content();
        let target = content
            .get(slot)
            .ok_or_else(|| format!("content slot {slot} is not allocated"))?;
        if [target.width, target.height] != atlas || target.format != self.format {
            return Err(format!(
                "content slot {slot} holds {}x{} {:?}, not this {}x{} {:?} atlas",
                target.width, target.height, target.format, atlas[0], atlas[1], self.format
            ));
        }
        for (id, cell) in &self.pending {
            check_write(atlas, self.format, self.placements[*id].cell, cell.len())?;
        }
        Ok(())
    }

    fn write_cell(&self, frame: &Frame, slot: usize, index: usize) {
        let content = frame.content();
        let Some(target) = content.get(slot) else {
            return;
        };
        let (id, cell) = &self.pending[index];
        let rect = self.placements[*id].cell;
        frame.gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect[0],
                    y: rect[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            cell,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(rect[2] * self.format.texel_bytes()),
                rows_per_image: Some(rect[3]),
            },
            wgpu::Extent3d {
                width: rect[2],
                height: rect[3],
                depth_or_array_layers: 1,
            },
        );
    }

    fn pending_regions(&self) -> Vec<MipRegion> {
        let size = self.size();
        let mut regions: Vec<MipRegion> = self
            .pending
            .iter()
            .flat_map(|(id, _)| mip_regions(size, self.placements[*id].cell))
            .collect();
        regions.sort_by_key(|region| region.level);
        regions
    }

    pub fn upload(&mut self, frame: &Frame, slot: usize) -> Result<usize, String> {
        if self.pending.is_empty() {
            return Ok(0);
        }
        self.check_target(frame, slot)?;
        for index in 0..self.pending.len() {
            self.write_cell(frame, slot, index);
        }
        let regions = self.pending_regions();
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("content atlas mips"),
                });
        frame.regenerate_content_regions(&mut encoder, slot, &regions)?;
        frame.gpu.queue.submit(Some(encoder.finish()));
        let written = self.pending.len();
        self.pending.clear();
        Ok(written)
    }

    pub fn upload_paced<T: Turned>(
        &mut self,
        frame: &Frame,
        slot: usize,
        pacer: &mut Pacer,
        between: impl FnMut(f64) -> T,
    ) -> Result<(usize, Stats), String> {
        if self.pending.is_empty() {
            return Ok((0, Stats::default()));
        }
        self.check_target(frame, slot)?;
        let regions = self.pending_regions();
        let writes = self.pending.len();
        let mut failure: Option<String> = None;
        let stats = pacer.run(
            "content atlas mips",
            &frame.gpu.device,
            &frame.gpu.queue,
            Work::Dispatches {
                count: (writes + regions.len()) as u32,
            },
            |encoder, slice| {
                let Slice::Dispatches { start, count } = slice else {
                    return;
                };
                let (start, end) = (start as usize, (start + count) as usize);
                for index in start..end.min(writes) {
                    self.write_cell(frame, slot, index);
                }
                let first = start.saturating_sub(writes);
                let last = end.saturating_sub(writes);
                if first < last
                    && failure.is_none()
                    && let Err(error) =
                        frame.regenerate_content_regions(encoder, slot, &regions[first..last])
                {
                    failure = Some(error);
                }
            },
            between,
        )?;
        if let Some(error) = failure {
            return Err(error);
        }
        self.pending.clear();
        Ok((writes, stats))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{
        Camera, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply, transform,
    };
    use pfx_gpu::Gpu;
    use pfx_materials::{Content, ContentLayer, Material};

    fn flat(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat(width as usize * height as usize)
    }

    fn marked(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|texel| {
                let (x, y) = (texel % width, texel / width);
                [x as u8, y as u8, (x ^ y) as u8, 255]
            })
            .collect()
    }

    fn overlap(a: [u32; 4], b: [u32; 4]) -> bool {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    }

    fn sizes() -> Vec<[u32; 2]> {
        (0..40)
            .map(|i| [20 + (i * 37) % 90, 12 + (i * 53) % 110])
            .collect()
    }

    fn pack(level: u32) -> (ContentAtlas, Vec<usize>) {
        let data: Vec<Vec<u8>> = sizes().iter().map(|s| marked(s[0], s[1])).collect();
        let images: Vec<AtlasImage> = sizes()
            .iter()
            .zip(&data)
            .map(|(s, t)| AtlasImage {
                width: s[0],
                height: s[1],
                texels: t,
            })
            .collect();
        let mut atlas = ContentAtlas::new(1024, 1024, ContentFormat::Srgb8, level).unwrap();
        let ids = atlas.add_all(&images).unwrap();
        (atlas, ids)
    }

    #[test]
    fn cells_never_overlap_and_stay_inside_the_atlas() {
        for level in [0, 2, 4] {
            let (atlas, ids) = pack(level);
            assert_eq!(ids.len(), 40);
            let cells: Vec<[u32; 4]> = ids
                .iter()
                .map(|&id| atlas.placement(id).unwrap().cell)
                .collect();
            for (i, a) in cells.iter().enumerate() {
                assert!(a[0] + a[2] <= 1024 && a[1] + a[3] <= 1024, "{a:?}");
                for b in &cells[i + 1..] {
                    assert!(!overlap(*a, *b), "{a:?} overlaps {b:?}");
                }
            }
            for (&id, size) in ids.iter().zip(sizes()) {
                let place = atlas.placement(id).unwrap();
                let block = gutter(level);
                assert_eq!([place.rect[2], place.rect[3]], size);
                assert_eq!(place.rect[0] % block, 0);
                assert_eq!(place.rect[1] % block, 0);
                assert_eq!(place.cell[0] % block, 0);
                assert_eq!(place.cell[2], cell_size(size, level)[0]);
                assert!(place.rect[0] >= place.cell[0] + block);
                assert!(place.rect[0] + place.rect[2] + block <= place.cell[0] + place.cell[2]);
                assert!(place.rect[1] + place.rect[3] + block <= place.cell[1] + place.cell[3]);
            }
        }
    }

    #[test]
    fn the_same_images_in_the_same_order_land_in_the_same_places() {
        let (a, ids_a) = pack(3);
        let (b, ids_b) = pack(3);
        assert_eq!(ids_a, ids_b);
        for id in ids_a {
            assert_eq!(a.placement(id), b.placement(id));
        }
        let (c, _) = pack(4);
        assert_ne!(a.placement(0), c.placement(0));
    }

    #[test]
    fn a_batch_returns_ids_in_input_order_whatever_the_packing_order() {
        let (atlas, ids) = pack(2);
        let mut distinct = ids.clone();
        distinct.sort();
        assert_eq!(distinct, (0..40).collect::<Vec<_>>());
        for (&id, size) in ids.iter().zip(sizes().iter()) {
            let place = atlas.placement(id).unwrap();
            assert_eq!([place.rect[2], place.rect[3]], *size);
        }
    }

    #[test]
    fn gutters_hold_the_images_own_edge_texels() {
        for level in [0, 1, 3] {
            let (width, height) = (13, 9);
            let texels = marked(width, height);
            let cell = padded_cell(
                &AtlasImage {
                    width,
                    height,
                    texels: &texels,
                },
                ContentFormat::Srgb8,
                level,
            )
            .unwrap();
            let block = gutter(level);
            let [cell_width, cell_height] = cell_size([width, height], level);
            assert_eq!(cell.len(), (cell_width * cell_height * 4) as usize);
            for row in 0..cell_height {
                for column in 0..cell_width {
                    let x = column.saturating_sub(block).min(width - 1);
                    let y = row.saturating_sub(block).min(height - 1);
                    let want = &texels[((y * width + x) * 4) as usize..][..4];
                    let got = &cell[((row * cell_width + column) * 4) as usize..][..4];
                    assert_eq!(got, want, "level {level} at {column},{row}");
                }
            }
            let inside =
                |column: u32, row: u32| &cell[((row * cell_width + column) * 4) as usize..][..4];
            assert_eq!(inside(block, block), &texels[0..4]);
            assert_eq!(inside(0, 0), &texels[0..4]);
            assert_eq!(
                inside(cell_width - 1, cell_height - 1),
                &texels[(((height - 1) * width + width - 1) * 4) as usize..][..4]
            );
        }
    }

    #[test]
    fn sixteen_bit_gutters_copy_whole_texels() {
        let texels: Vec<u8> = (0..3 * 2 * 8).map(|i| i as u8).collect();
        let cell = padded_cell(
            &AtlasImage {
                width: 3,
                height: 2,
                texels: &texels,
            },
            ContentFormat::Linear16,
            0,
        )
        .unwrap();
        let [width, height] = cell_size([3, 2], 0);
        assert_eq!(cell.len(), (width * height * 8) as usize);
        assert_eq!(&cell[..8], &texels[..8]);
        assert_eq!(&cell[8..16], &texels[..8]);
        let last = ((height - 1) * width + width - 1) as usize * 8;
        assert_eq!(&cell[last..last + 8], &texels[5 * 8..6 * 8]);
    }

    #[test]
    fn crop_maths_matches_the_instance_surface() {
        let (atlas, ids) = pack(3);
        let size = atlas.size();
        for id in ids {
            let place = *atlas.placement(id).unwrap();
            let surface = atlas.surface(id).unwrap();
            assert_eq!(surface.crop, atlas.crop(id).unwrap());
            assert_eq!(surface.uv_offset, place.uv_offset(size));
            assert_eq!(surface.uv_scale, place.uv_scale(size));
            let [x, y, width, height] = place.rect;
            let texel = |u: f32, v: f32| {
                let at = surface.content_uv([u, v]).unwrap();
                [at[0] * size[0] as f32, at[1] * size[1] as f32]
            };
            let corner = texel(0.0, 0.0);
            assert!((corner[0] - x as f32).abs() < 1e-3 && (corner[1] - y as f32).abs() < 1e-3);
            let far = texel(1.0, 1.0);
            assert!((far[0] - (x + width) as f32).abs() < 1e-2);
            assert!((far[1] - (y + height) as f32).abs() < 1e-2);
            let middle = texel(0.5, 0.5);
            assert!((middle[0] - (x as f32 + width as f32 / 2.0)).abs() < 1e-2);
            assert!(surface.content_uv([-0.05, 0.5]).is_none());
            assert!(surface.content_uv([0.5, 1.05]).is_none());
        }
    }

    #[test]
    fn adding_to_a_live_atlas_moves_nothing_until_it_is_full() {
        let mut atlas = ContentAtlas::new(256, 256, ContentFormat::Srgb8, 2).unwrap();
        let tile = flat(40, 40, [9, 9, 9, 255]);
        let image = AtlasImage {
            width: 40,
            height: 40,
            texels: &tile,
        };
        let mut seen = Vec::new();
        let error = loop {
            match atlas.add(&image) {
                Ok(id) => {
                    seen.push(*atlas.placement(id).unwrap());
                    for (before, now) in seen.iter().zip((0..=id).map(|i| atlas.placement(i))) {
                        assert_eq!(Some(before), now);
                    }
                }
                Err(error) => break error,
            }
        };
        assert_eq!(seen.len(), 25);
        assert_eq!(
            error,
            AtlasError::Full {
                cell: [48, 48],
                atlas: [256, 256],
                images: 25,
                used: 25 * 48 * 48,
            }
        );
        assert!(error.to_string().contains("full"));
        assert_eq!(atlas.len(), 25);
        assert_eq!(atlas.add(&image).unwrap_err(), error);
    }

    #[test]
    fn a_later_small_image_fills_a_gap_without_moving_the_first() {
        let mut atlas = ContentAtlas::new(128, 128, ContentFormat::Srgb8, 0).unwrap();
        let wide = flat(100, 20, [1, 1, 1, 255]);
        let first = atlas
            .add(&AtlasImage {
                width: 100,
                height: 20,
                texels: &wide,
            })
            .unwrap();
        let before = *atlas.placement(first).unwrap();
        let small = flat(10, 10, [2, 2, 2, 255]);
        let second = atlas
            .add(&AtlasImage {
                width: 10,
                height: 10,
                texels: &small,
            })
            .unwrap();
        assert_eq!(*atlas.placement(first).unwrap(), before);
        assert!(!overlap(
            atlas.placement(first).unwrap().cell,
            atlas.placement(second).unwrap().cell
        ));
    }

    #[test]
    fn a_failed_batch_leaves_the_atlas_as_it_was() {
        let mut atlas = ContentAtlas::new(128, 128, ContentFormat::Srgb8, 1).unwrap();
        let tile = flat(50, 50, [5, 5, 5, 255]);
        let image = AtlasImage {
            width: 50,
            height: 50,
            texels: &tile,
        };
        atlas.add(&image).unwrap();
        let before = *atlas.placement(0).unwrap();
        let used = atlas.pending();
        assert!(atlas.add_all(&[image, image, image, image, image]).is_err());
        assert_eq!(atlas.len(), 1);
        assert_eq!(atlas.pending(), used);
        assert_eq!(*atlas.placement(0).unwrap(), before);
        let ids = atlas.add_all(&[image, image]).unwrap();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn bad_images_and_sides_are_refused_with_a_reason() {
        assert!(ContentAtlas::new(0, 64, ContentFormat::Srgb8, 0).is_err());
        assert!(ContentAtlas::new(16385, 64, ContentFormat::Srgb8, 0).is_err());
        assert!(ContentAtlas::new(64, 64, ContentFormat::Srgb8, 5).is_err());
        assert!(ContentAtlas::new(16384, 16384, ContentFormat::Srgb8, 14).is_err());
        assert!(ContentAtlas::new(16384, 16384, ContentFormat::Srgb8, 4).is_ok());
        let mut atlas = ContentAtlas::new(64, 64, ContentFormat::Srgb8, 2).unwrap();
        let short = AtlasImage {
            width: 4,
            height: 4,
            texels: &[0; 10],
        };
        assert!(matches!(
            atlas.add(&short),
            Err(AtlasError::BadTexels {
                expected: 64,
                got: 10,
                ..
            })
        ));
        let empty = AtlasImage {
            width: 0,
            height: 4,
            texels: &[],
        };
        assert_eq!(atlas.add(&empty), Err(AtlasError::EmptyImage));
        let big = flat(60, 8, [0; 4]);
        let big = AtlasImage {
            width: 60,
            height: 8,
            texels: &big,
        };
        assert!(matches!(atlas.add(&big), Err(AtlasError::TooLarge { .. })));
        assert!(atlas.is_empty());
    }

    #[test]
    fn five_hundred_twelve_cards_fit_in_the_stated_numbers() {
        let card = [512, 512];
        assert_eq!(cell_size(card, DEFAULT_BLEED_LEVEL), [544, 544]);
        assert_eq!(cards_that_fit([8192, 8192], card, DEFAULT_BLEED_LEVEL), 225);
        assert_eq!(
            cards_that_fit([16384, 16384], card, DEFAULT_BLEED_LEVEL),
            900
        );
        assert_eq!(cards_that_fit([8192, 8192], card, 0), 15 * 15);
        assert_eq!(cards_that_fit([16384, 16384], card, 0), 31 * 31);
        let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
        assert!((mib(texture_bytes(8192, 8192, ContentFormat::Srgb8)) - 341.33).abs() < 0.01);
        assert!((mib(texture_bytes(16384, 16384, ContentFormat::Srgb8)) - 1365.33).abs() < 0.01);
        assert!((mib(texture_bytes(8192, 8192, ContentFormat::Linear16)) - 682.67).abs() < 0.01);
        for (side, want) in [(8192u32, 225usize), (16384, 900)] {
            let mut packer = Packer::new(side, side);
            let mut placed = Vec::new();
            while let Some(at) = packer.place(544, 544) {
                placed.push(at);
            }
            assert_eq!(placed.len(), want);
            assert_eq!(placed[0], [0, 0]);
            assert_eq!(placed[1], [544, 0]);
        }
    }

    #[test]
    fn the_packer_is_deterministic_and_best_fits_shelves() {
        let mut a = Packer::new(100, 100);
        let mut b = Packer::new(100, 100);
        let asks = [[40, 30], [40, 50], [20, 28], [60, 50], [10, 10], [50, 20]];
        let placed_a: Vec<_> = asks.iter().map(|s| a.place(s[0], s[1])).collect();
        let placed_b: Vec<_> = asks.iter().map(|s| b.place(s[0], s[1])).collect();
        assert_eq!(placed_a, placed_b);
        assert_eq!(
            placed_a,
            [
                Some([0, 0]),
                Some([0, 30]),
                Some([40, 0]),
                Some([40, 30]),
                Some([60, 0]),
                Some([0, 80]),
            ]
        );
        let placed: Vec<[u32; 4]> = asks
            .iter()
            .zip(&placed_a)
            .filter_map(|(s, at)| at.map(|at| [at[0], at[1], s[0], s[1]]))
            .collect();
        for (i, p) in placed.iter().enumerate() {
            for q in &placed[i + 1..] {
                assert!(!overlap(*p, *q));
            }
        }
        assert_eq!(Packer::new(10, 10).place(11, 1), None);
        assert_eq!(Packer::new(10, 10).place(0, 1), None);
    }

    fn card_texels(card: usize, side: u32) -> Vec<u8> {
        let mut texels = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                texels.extend(quadrant_colour(
                    card,
                    usize::from(x >= side / 2) + 2 * usize::from(y >= side / 2),
                ));
            }
        }
        texels
    }

    fn quadrant_colour(card: usize, quadrant: usize) -> [u8; 4] {
        let k = card * 4 + quadrant;
        [
            ((k * 53 + 17) % 256) as u8,
            ((k * 97 + 101) % 256) as u8,
            ((k * 193 + 7) % 256) as u8,
            255,
        ]
    }

    fn linear(value: u8) -> f32 {
        pfx_materials::linear_channel(f32::from(value) / 255.0)
    }

    fn half(value: u16) -> f32 {
        half::f16::from_bits(value).to_f32()
    }

    fn identity() -> Matrix {
        let mut m = [[0.0; 4]; 4];
        for (i, column) in m.iter_mut().enumerate() {
            column[i] = 1.0;
        }
        m
    }

    fn perspective() -> Matrix {
        let (near, far) = (0.1, 100.0);
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        [
            [f, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ]
    }

    fn look_down(height: f32) -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, -height, 1.0],
        ]
    }

    struct Sheet {
        frame_px: u32,
        card_px: u32,
        image: u32,
        atlas: u32,
        level: u32,
    }

    #[derive(Debug)]
    struct Worst {
        error: f32,
        card: usize,
        quadrant: usize,
        at: [u32; 2],
        pixels: usize,
        seen: [f32; 3],
    }

    impl Worst {
        fn describe(&self) -> String {
            format!(
                "worst {:.5}/255 on card {} quadrant {} at pixel {:?} of {} checked, got/want {:?}",
                self.error * 255.0,
                self.card,
                self.quadrant,
                self.at,
                self.pixels,
                self.seen
            )
        }
    }

    fn render_sheet(sheet: &Sheet) -> Worst {
        let Sheet {
            frame_px,
            card_px,
            image,
            atlas: atlas_side,
            level,
        } = *sheet;
        let per_side = (frame_px / card_px) as usize;
        let cards = per_side * per_side;
        assert_eq!(cards, 64);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, frame_px, frame_px).unwrap();
        let data: Vec<Vec<u8>> = (0..cards).map(|card| card_texels(card, image)).collect();
        let images: Vec<AtlasImage> = data
            .iter()
            .map(|texels| AtlasImage {
                width: image,
                height: image,
                texels,
            })
            .collect();
        let mut atlas =
            ContentAtlas::new(atlas_side, atlas_side, ContentFormat::Srgb8, level).unwrap();
        atlas.allocate(&frame, 0).unwrap();
        let mut ids = atlas.add_all(&images[..cards / 2]).unwrap();
        assert_eq!(atlas.upload(&frame, 0).unwrap(), cards / 2);
        let before: Vec<Placement> = ids
            .iter()
            .map(|&id| *atlas.placement(id).unwrap())
            .collect();
        ids.extend(atlas.add_all(&images[cards / 2..]).unwrap());
        assert_eq!(atlas.upload(&frame, 0).unwrap(), cards / 2);
        assert_eq!(atlas.upload(&frame, 0).unwrap(), 0);
        for (id, place) in ids.iter().zip(&before) {
            assert_eq!(atlas.placement(*id).unwrap(), place, "adding moved a card");
        }

        let quad = frame
            .upload_mesh(MeshData {
                positions: &[
                    [-0.5, 0.0, -0.5],
                    [0.5, 0.0, -0.5],
                    [-0.5, 0.0, 0.5],
                    [0.5, 0.0, 0.5],
                ],
                normals: &[[0.0, 1.0, 0.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &[0, 2, 1, 1, 2, 3],
            })
            .unwrap();
        let height = 6.0;
        let view = look_down(height);
        let projection = perspective();
        let view_projection = multiply(projection, view);
        let pitch = 2.0 * height * 25.0_f32.to_radians().tan() / frame_px as f32;
        let span = pitch * card_px as f32;
        let centre = |index: usize| {
            [
                ((index % per_side) as f32 - (per_side as f32 - 1.0) / 2.0) * span,
                ((index / per_side) as f32 - (per_side as f32 - 1.0) / 2.0) * span,
            ]
        };
        let instances: Vec<Instance> = (0..cards)
            .map(|index| {
                let [x, z] = centre(index);
                let mut model = identity();
                model[0][0] = span;
                model[2][2] = span;
                model[3] = [x, 0.0, z, 1.0];
                Instance::new(quad, model, 1, index as u32 + 1)
            })
            .collect();
        let surfaces: Vec<InstanceSurface> = (0..cards)
            .map(|index| atlas.surface(ids[index]).unwrap())
            .collect();
        let dark = Material {
            base: [0.0; 3],
            roughness: 0.3,
            ..Default::default()
        };
        let lit = Material {
            content_layer: ContentLayer::from_kind(
                Content::Screen,
                1.0,
                0,
                &pfx_materials::ContentLook {
                    screen_gain: 1.5,
                    ..Default::default()
                },
            ),
            ..dark
        };
        let camera = Camera {
            view,
            projection,
            previous_view_projection: view_projection,
            position: [0.0, height, 0.0],
        };
        let sun = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let shoot =
            |frame: &mut Frame, materials: &[Material], surfaces: &[InstanceSurface], used: u32| {
                let scene_instances: Vec<Instance> = instances
                    .iter()
                    .map(|instance| Instance {
                        material: used,
                        ..*instance
                    })
                    .collect();
                frame.set_surfaces(surfaces).unwrap();
                frame
                    .render(&Scene {
                        camera,
                        time: 0.0,
                        seed: 7,
                        sun,
                        instances: &scene_instances,
                        materials,
                        deformers: &[],
                        wind: SceneWind::default(),
                    })
                    .unwrap();
                frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap()
            };
        let materials = [dark, lit];
        let on = shoot(&mut frame, &materials, &surfaces, 1);
        let black = [0u8, 0, 0, 255].repeat((image * image) as usize);
        let blanks: Vec<AtlasImage> = (0..cards)
            .map(|_| AtlasImage {
                width: image,
                height: image,
                texels: &black,
            })
            .collect();
        let mut blank_atlas =
            ContentAtlas::new(atlas_side, atlas_side, ContentFormat::Srgb8, level).unwrap();
        blank_atlas.allocate(&frame, 0).unwrap();
        let blank_ids = blank_atlas.add_all(&blanks).unwrap();
        assert_eq!(blank_ids, ids);
        blank_atlas.upload(&frame, 0).unwrap();
        let zero = shoot(&mut frame, &materials, &surfaces, 1);

        let pixel = |point: [f32; 3]| {
            let clip = transform(view_projection, [point[0], point[1], point[2], 1.0]);
            [
                (clip[0] / clip[3] * 0.5 + 0.5) * frame_px as f32,
                (0.5 - clip[1] / clip[3] * 0.5) * frame_px as f32,
            ]
        };
        let cross = 2.0 * card_px as f32 / 32.0;
        let mut worst = Worst {
            error: 0.0,
            card: 0,
            quadrant: 0,
            at: [0, 0],
            pixels: 0,
            seen: [0.0; 3],
        };
        for index in 0..cards {
            let [x, z] = centre(index);
            let a = pixel([x - span / 2.0, 0.0, z - span / 2.0]);
            let bu = pixel([x + span / 2.0, 0.0, z - span / 2.0]);
            let bv = pixel([x - span / 2.0, 0.0, z + span / 2.0]);
            let du = [bu[0] - a[0], bu[1] - a[1]];
            let dv = [bv[0] - a[0], bv[1] - a[1]];
            for py in 0..frame_px {
                for px in 0..frame_px {
                    let c = [px as f32 + 0.5 - a[0], py as f32 + 0.5 - a[1]];
                    let u = (c[0] * du[0] + c[1] * du[1]) / (du[0] * du[0] + du[1] * du[1]);
                    let v = (c[0] * dv[0] + c[1] * dv[1]) / (dv[0] * dv[0] + dv[1] * dv[1]);
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                        continue;
                    }
                    if (u - 0.5).abs() * (card_px as f32) < cross
                        || (v - 0.5).abs() * (card_px as f32) < cross
                    {
                        continue;
                    }
                    let at = ((py * frame_px + px) * 4) as usize;
                    if zero[at..at + 3].iter().any(|v| half(*v) > 0.05) {
                        continue;
                    }
                    let quadrant = usize::from(u >= 0.5) + 2 * usize::from(v >= 0.5);
                    let want = quadrant_colour(index, quadrant);
                    worst.pixels += 1;
                    for channel in 0..3 {
                        let got = (half(on[at + channel]) - half(zero[at + channel])) / 1.5;
                        let error = (got - linear(want[channel])).abs();
                        if error > worst.error {
                            worst = Worst {
                                error,
                                card: index,
                                quadrant,
                                at: [px, py],
                                pixels: worst.pixels,
                                seen: [channel as f32, got, linear(want[channel])],
                            };
                        }
                    }
                }
            }
        }
        worst
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn sixty_four_cards_show_their_own_image_at_mip_zero() {
        let worst = render_sheet(&Sheet {
            frame_px: 512,
            card_px: 64,
            image: 64,
            atlas: 1024,
            level: DEFAULT_BLEED_LEVEL,
        });
        println!("1:1, gutter level 4: {}", worst.describe());
        assert!(worst.pixels > 64 * 3000, "{}", worst.describe());
        assert!(worst.error <= 1.0 / 255.0, "{}", worst.describe());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn minified_sixteen_times_the_gutters_keep_neighbours_out() {
        let sheet = |level| Sheet {
            frame_px: 256,
            card_px: 32,
            image: 512,
            atlas: 8192,
            level,
        };
        let held = render_sheet(&sheet(DEFAULT_BLEED_LEVEL));
        println!("16x minified, gutter level 4: {}", held.describe());
        assert!(held.pixels > 64 * 600, "{}", held.describe());
        assert!(held.error <= 1.0 / 255.0, "{}", held.describe());
        let bare = render_sheet(&sheet(0));
        println!("16x minified, gutter level 0: {}", bare.describe());
        assert!(
            bare.error > 4.0 / 255.0,
            "a bare gutter should bleed: {}",
            bare.describe()
        );
    }

    fn reads(from: u32, to: u32, texel: u32) -> [u32; 2] {
        let at = (f64::from(texel) + 0.5) * f64::from(from) / f64::from(to) - 0.5;
        let first = at.floor() as i64;
        let last = i64::from(from) - 1;
        [
            first.clamp(0, last) as u32,
            (first + 1).clamp(0, last) as u32,
        ]
    }

    fn side(size: u32, level: u32) -> u32 {
        (size >> level).max(1)
    }

    #[test]
    fn every_mip_texel_that_reads_a_changed_texel_lies_inside_its_region() {
        let sizes = [
            [1000u32, 616],
            [1024, 1024],
            [257, 9],
            [1, 64],
            [16384, 16384],
        ];
        for size in sizes {
            let rects = [
                [0u32, 0, 24, 24],
                [8, 4, 40, 36],
                [size[0] / 2, size[1] / 2, 1, 1],
                [size[0].saturating_sub(5), size[1].saturating_sub(3), 5, 3],
                [0, 0, size[0], size[1]],
            ];
            for rect in rects {
                let (x, y) = (rect[0].min(size[0] - 1), rect[1].min(size[1] - 1));
                let rect = [x, y, rect[2].min(size[0] - x), rect[3].min(size[1] - y)];
                let regions = mip_regions(size, rect);
                assert_eq!(regions.len() as u32 + 1, mip_count(size[0], size[1]));
                let mut changed = [[rect[0], rect[0] + rect[2]], [rect[1], rect[1] + rect[3]]];
                for region in &regions {
                    let covered = [
                        [region.rect[0], region.rect[0] + region.rect[2]],
                        [region.rect[1], region.rect[1] + region.rect[3]],
                    ];
                    for axis in 0..2 {
                        let from = side(size[axis], region.level - 1);
                        let to = side(size[axis], region.level);
                        assert!(covered[axis][0] < covered[axis][1] && covered[axis][1] <= to);
                        for texel in 0..to {
                            let [a, b] = reads(from, to, texel);
                            let touches = a < changed[axis][1] && changed[axis][0] <= b;
                            let inside = covered[axis][0] <= texel && texel < covered[axis][1];
                            assert!(
                                inside || !touches,
                                "{size:?} {rect:?} level {} axis {axis} texel {texel}",
                                region.level
                            );
                        }
                    }
                    changed = covered;
                }
            }
        }
    }

    #[test]
    fn a_cell_region_stays_close_to_its_footprint() {
        let size = [16384, 16384];
        let cell = [544u32, 1088, 544, 544];
        for region in mip_regions(size, cell) {
            let shrink = 1u32 << region.level;
            assert!(
                region.rect[2] <= cell[2] / shrink + 8 && region.rect[3] <= cell[3] / shrink + 8,
                "{region:?}"
            );
        }
    }

    fn marked16(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|texel| {
                let (x, y) = (texel % width, texel / width);
                [
                    x as f32 / width as f32,
                    y as f32 / height as f32,
                    ((x ^ y) % 16) as f32 / 16.0,
                    1.0,
                ]
            })
            .flat_map(|value| half::f16::from_f32(value).to_le_bytes())
            .collect()
    }

    fn texels_for(format: ContentFormat, width: u32, height: u32) -> Vec<u8> {
        match format {
            ContentFormat::Srgb8 => marked(width, height),
            ContentFormat::Linear16 => marked16(width, height),
        }
    }

    fn read_levels(frame: &Frame, slot: usize) -> Vec<Vec<u8>> {
        let gpu = &frame.gpu;
        let content = frame.content();
        let target = content.get(slot).unwrap();
        let texel = target.format.texel_bytes();
        (0..target.mips)
            .map(|level| {
                let width = (target.width >> level).max(1);
                let height = (target.height >> level).max(1);
                let raw = width * texel;
                let row = raw.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("atlas level readback"),
                    size: u64::from(row) * u64::from(height),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &target.texture,
                        mip_level: level,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(row),
                            rows_per_image: Some(height),
                        },
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
                gpu.queue.submit(Some(encoder.finish()));
                let (sender, receiver) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        let _ = sender.send(result);
                    });
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                receiver.recv().unwrap().unwrap();
                let mapped = buffer.slice(..).get_mapped_range();
                let mut bytes = Vec::with_capacity((raw * height) as usize);
                for padded in mapped.chunks_exact(row as usize) {
                    bytes.extend_from_slice(&padded[..raw as usize]);
                }
                drop(mapped);
                buffer.unmap();
                bytes
            })
            .collect()
    }

    fn assert_same_levels(a: &[Vec<u8>], b: &[Vec<u8>], what: &str) {
        assert_eq!(a.len(), b.len());
        for (level, (left, right)) in a.iter().zip(b).enumerate() {
            let differ = left.iter().zip(right).filter(|(x, y)| x != y).count();
            assert_eq!(differ, 0, "{what}: level {level} differs in {differ} bytes");
        }
    }

    const ATLAS: [u32; 2] = [1001, 777];

    fn batches(format: ContentFormat) -> [Vec<Vec<u8>>; 2] {
        let all: Vec<Vec<u8>> = sizes()
            .iter()
            .map(|size| texels_for(format, size[0], size[1]))
            .collect();
        [all[..20].to_vec(), all[20..].to_vec()]
    }

    fn add_batch(atlas: &mut ContentAtlas, data: &[Vec<u8>], first: usize) {
        let all = sizes();
        let images: Vec<AtlasImage> = data
            .iter()
            .enumerate()
            .map(|(index, texels)| AtlasImage {
                width: all[first + index][0],
                height: all[first + index][1],
                texels,
            })
            .collect();
        atlas.add_all(&images).unwrap();
    }

    fn headless_frame() -> Frame {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        Frame::new(gpu, 64, 64).unwrap()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn regional_mips_equal_a_full_rebuild_byte_for_byte() {
        for format in [ContentFormat::Srgb8, ContentFormat::Linear16] {
            let frame = headless_frame();
            let [first, second] = batches(format);
            let mut atlas = ContentAtlas::new(ATLAS[0], ATLAS[1], format, 2).unwrap();
            atlas.allocate(&frame, 0).unwrap();
            add_batch(&mut atlas, &first, 0);
            assert_eq!(atlas.upload(&frame, 0).unwrap(), 20);
            add_batch(&mut atlas, &second, 20);
            assert_eq!(atlas.upload(&frame, 0).unwrap(), 20);
            let regional = read_levels(&frame, 0);
            assert!(regional.len() > 5);
            assert!(regional.last().unwrap().iter().any(|byte| *byte != 0));
            let mut encoder = frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            frame.regenerate_content(&mut encoder, 0).unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            let full = read_levels(&frame, 0);
            assert_same_levels(
                &regional,
                &full,
                &format!("{format:?} regional against full"),
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn paced_mips_equal_unpaced_byte_for_byte() {
        for format in [ContentFormat::Srgb8, ContentFormat::Linear16] {
            let frame = headless_frame();
            let [first, second] = batches(format);
            let mut plain = ContentAtlas::new(ATLAS[0], ATLAS[1], format, 2).unwrap();
            let mut paced = ContentAtlas::new(ATLAS[0], ATLAS[1], format, 2).unwrap();
            plain.allocate(&frame, 0).unwrap();
            paced.allocate(&frame, 1).unwrap();
            let mut pacer = Pacer::default();
            let mut turns = pfx_gpu::pace::Turns::default();
            let mut slices = 0;
            for (data, start) in [(&first, 0), (&second, 20)] {
                add_batch(&mut plain, data, start);
                add_batch(&mut paced, data, start);
                plain.upload(&frame, 0).unwrap();
                let (written, stats) = paced
                    .upload_paced(&frame, 1, &mut pacer, |ms| turns.add(ms))
                    .unwrap();
                assert_eq!(written, 20);
                assert_eq!(paced.pending(), 0);
                slices += stats.count();
            }
            assert!(slices >= 2, "{format:?} ran in {slices} submissions");
            assert_same_levels(
                &read_levels(&frame, 0),
                &read_levels(&frame, 1),
                &format!("{format:?} paced against unpaced"),
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn sixty_four_new_cards_at_sixteen_thousand_pixels_upload_in_short_submissions() {
        let frame = headless_frame();
        let side = 512;
        let mut atlas =
            ContentAtlas::new(16384, 16384, ContentFormat::Srgb8, DEFAULT_BLEED_LEVEL).unwrap();
        atlas.allocate(&frame, 0).unwrap();
        let data: Vec<Vec<u8>> = (0..72).map(|card| card_texels(card, side)).collect();
        let images: Vec<AtlasImage> = data
            .iter()
            .map(|texels| AtlasImage {
                width: side,
                height: side,
                texels,
            })
            .collect();
        atlas.add_all(&images[..8]).unwrap();
        assert_eq!(atlas.upload(&frame, 0).unwrap(), 8);
        atlas.add_all(&images[8..]).unwrap();
        let mut pacer = Pacer::default();
        let mut turns = pfx_gpu::pace::Turns::default();
        let (written, stats) = atlas
            .upload_paced(&frame, 0, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        assert_eq!(written, 64);
        println!(
            "16384x16384 Srgb8, 64 new 512x512 cards: {} submissions, longest {:.3} ms, median {:.3} ms, {:?} timing, wall {:.1} ms",
            stats.count(),
            stats.longest_ms(),
            stats.median_ms(),
            stats.source(),
            stats.wall_ms
        );
        assert!(stats.count() > 1);
        assert!(
            stats.longest_ms() < 50.0,
            "longest {:.3} ms",
            stats.longest_ms()
        );
        let began = std::time::Instant::now();
        let mut encoder = frame
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        frame.regenerate_content(&mut encoder, 0).unwrap();
        frame.gpu.queue.submit(Some(encoder.finish()));
        frame
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        println!(
            "one full rebuild of the same slot: {:.3} ms",
            began.elapsed().as_secs_f64() * 1000.0
        );
    }
}
