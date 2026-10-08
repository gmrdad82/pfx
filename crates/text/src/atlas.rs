use std::collections::BTreeMap;

use crate::{ATLAS_CELL_ALIGN, ATLAS_PADDING, Atlas, Error, GlyphImage, MipLevel, RasterKey};

pub const DEFAULT_ATLAS_BUDGET: u64 = 32 << 20;
pub const DEFAULT_ATLAS_PAGE: u32 = 1024;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum PageKind {
    Coverage,
    Msdf,
    Color,
}

impl PageKind {
    pub fn channels(self) -> u32 {
        match self {
            Self::Coverage => 1,
            Self::Msdf => 3,
            Self::Color => 4,
        }
    }

    pub fn levels(self) -> u32 {
        match self {
            Self::Msdf => 1,
            Self::Coverage | Self::Color => 4,
        }
    }

    pub fn texel_bytes(self) -> u64 {
        match self {
            Self::Coverage => 1,
            Self::Msdf | Self::Color => 4,
        }
    }

    pub fn page_bytes(self, size: u32) -> u64 {
        (0..self.levels())
            .map(|level| u64::from((size >> level).max(1)).pow(2) * self.texel_bytes())
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageInfo {
    pub id: u32,
    pub kind: PageKind,
    pub channels: u32,
    pub size: u32,
    pub levels: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellUpload {
    pub page: u32,
    pub level: u32,
    pub rect: [u32; 4],
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AtlasChanges {
    pub created: Vec<PageInfo>,
    pub dropped: Vec<u32>,
    pub cells: Vec<CellUpload>,
}

impl AtlasChanges {
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.dropped.is_empty() && self.cells.is_empty()
    }

    pub fn bytes(&self) -> u64 {
        self.cells.iter().map(|cell| cell.bytes.len() as u64).sum()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    pub pages: usize,
    pub cells: usize,
    pub bytes: u64,
    pub budget: u64,
    pub shaped: u64,
    pub fast: u64,
    pub rasterized: u64,
    pub evicted: u64,
    pub uploaded: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellRef {
    pub page: u32,
    pub kind: PageKind,
    pub rect: [u32; 4],
    pub size: u32,
    pub left: f32,
    pub top: f32,
}

struct Shelf {
    y: u32,
    height: u32,
    free: Vec<[u32; 2]>,
}

struct Shelves {
    units: u32,
    shelves: Vec<Shelf>,
    rows: Vec<[u32; 2]>,
}

fn insert_span(spans: &mut Vec<[u32; 2]>, start: u32, length: u32) {
    let at = spans.partition_point(|span| span[0] < start);
    spans.insert(at, [start, length]);
    if at + 1 < spans.len() && spans[at][0] + spans[at][1] == spans[at + 1][0] {
        spans[at][1] += spans[at + 1][1];
        spans.remove(at + 1);
    }
    if at > 0 && spans[at - 1][0] + spans[at - 1][1] == spans[at][0] {
        spans[at - 1][1] += spans[at][1];
        spans.remove(at);
    }
}

fn shelf_height(height: u32) -> u32 {
    if height <= 4 {
        height
    } else {
        height.next_multiple_of(2)
    }
}

impl Shelves {
    fn new(units: u32) -> Self {
        Self {
            units,
            shelves: Vec::new(),
            rows: vec![[0, units]],
        }
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<[u32; 2]> {
        if width > self.units || height > self.units {
            return None;
        }
        let wanted = shelf_height(height);
        let mut best: Option<(u32, usize, usize)> = None;
        for (index, shelf) in self.shelves.iter().enumerate() {
            if shelf.height < height || shelf.height > wanted + wanted / 2 {
                continue;
            }
            if best.is_some_and(|(h, _, _)| h <= shelf.height) {
                continue;
            }
            if let Some(span) = shelf.free.iter().position(|span| span[1] >= width) {
                best = Some((shelf.height, index, span));
            }
        }
        if best.is_none() {
            let row = self.rows.iter().position(|row| row[1] >= wanted)?;
            let y = self.rows[row][0];
            self.rows[row][0] += wanted;
            self.rows[row][1] -= wanted;
            if self.rows[row][1] == 0 {
                self.rows.remove(row);
            }
            let at = self.shelves.partition_point(|shelf| shelf.y < y);
            self.shelves.insert(
                at,
                Shelf {
                    y,
                    height: wanted,
                    free: vec![[0, self.units]],
                },
            );
            best = Some((wanted, at, 0));
        }
        let (_, index, span) = best?;
        let shelf = &mut self.shelves[index];
        let x = shelf.free[span][0];
        shelf.free[span][0] += width;
        shelf.free[span][1] -= width;
        if shelf.free[span][1] == 0 {
            shelf.free.remove(span);
        }
        Some([x, shelf.y])
    }

    fn release(&mut self, x: u32, y: u32, width: u32) {
        let Some(index) = self.shelves.iter().position(|shelf| shelf.y == y) else {
            return;
        };
        let shelf = &mut self.shelves[index];
        insert_span(&mut shelf.free, x, width);
        if shelf.free == [[0, self.units]] {
            let shelf = self.shelves.remove(index);
            insert_span(&mut self.rows, shelf.y, shelf.height);
        }
    }

    fn is_empty(&self) -> bool {
        self.shelves.is_empty()
    }
}

struct Page {
    kind: PageKind,
    face: String,
    atlas: Atlas,
    shelves: Shelves,
}

struct Cell {
    page: u32,
    slot: [u32; 4],
    rect: [u32; 4],
    left: f32,
    top: f32,
}

struct Entry {
    cell: Option<Cell>,
    stamp: u64,
}

pub(crate) struct SharedAtlas {
    budget: u64,
    size: u32,
    pages: BTreeMap<u32, Page>,
    next_page: u32,
    entries: BTreeMap<RasterKey, Entry>,
    tick: u64,
    frame_start: u64,
    changes: AtlasChanges,
    pub(crate) stats: AtlasStats,
}

impl SharedAtlas {
    pub fn new() -> Self {
        Self {
            budget: DEFAULT_ATLAS_BUDGET,
            size: DEFAULT_ATLAS_PAGE,
            pages: BTreeMap::new(),
            next_page: 0,
            entries: BTreeMap::new(),
            tick: 0,
            frame_start: 0,
            changes: AtlasChanges::default(),
            stats: AtlasStats {
                budget: DEFAULT_ATLAS_BUDGET,
                ..AtlasStats::default()
            },
        }
    }

    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.stats.budget = bytes;
    }

    pub fn set_page_size(&mut self, size: u32) -> Result<(), Error> {
        if !(64..=8192).contains(&size) || !size.is_multiple_of(ATLAS_CELL_ALIGN) {
            return Err(Error::InvalidStyle);
        }
        if size != self.size {
            self.clear();
            self.size = size;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        let ids = self.pages.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.drop_page(id);
        }
        self.entries.clear();
    }

    pub fn begin_frame(&mut self) {
        self.frame_start = self.tick;
        if self.entries.len() > 1 << 16 {
            let start = self.frame_start;
            self.entries
                .retain(|_, entry| entry.cell.is_some() || entry.stamp >= start);
        }
    }

    pub fn take_changes(&mut self) -> AtlasChanges {
        std::mem::take(&mut self.changes)
    }

    pub fn page(&self, id: u32) -> Option<&Atlas> {
        self.pages.get(&id).map(|page| &page.atlas)
    }

    pub fn stats(&self) -> AtlasStats {
        AtlasStats {
            pages: self.pages.len(),
            cells: self
                .entries
                .values()
                .filter(|entry| entry.cell.is_some())
                .count(),
            bytes: self.bytes(),
            ..self.stats
        }
    }

    fn bytes(&self) -> u64 {
        self.pages
            .values()
            .map(|page| page.kind.page_bytes(self.size))
            .sum()
    }

    pub fn touch(&mut self, key: &RasterKey) -> Option<Option<CellRef>> {
        let entry = self.entries.get_mut(key)?;
        self.tick += 1;
        entry.stamp = self.tick;
        Some(entry.cell.as_ref().map(|cell| {
            let page = &self.pages[&cell.page];
            CellRef {
                page: cell.page,
                kind: page.kind,
                rect: cell.rect,
                size: self.size,
                left: cell.left,
                top: cell.top,
            }
        }))
    }

    pub fn insert(
        &mut self,
        key: RasterKey,
        kind: PageKind,
        image: Option<GlyphImage>,
    ) -> Result<Option<CellRef>, Error> {
        self.stats.rasterized += 1;
        self.tick += 1;
        let stamp = self.tick;
        let Some(image) = image else {
            self.entries.insert(key, Entry { cell: None, stamp });
            return Ok(None);
        };
        let width = (image.width + ATLAS_PADDING * 2).next_multiple_of(ATLAS_CELL_ALIGN);
        let height = (image.height + ATLAS_PADDING * 2).next_multiple_of(ATLAS_CELL_ALIGN);
        if width > self.size || height > self.size {
            return Err(Error::AtlasTooLarge);
        }
        let units = [width / ATLAS_CELL_ALIGN, height / ATLAS_CELL_ALIGN];
        let face = key.font_id.clone();
        let (page, origin) = self.place(&face, kind, units)?;
        let slot = [
            origin[0] * ATLAS_CELL_ALIGN,
            origin[1] * ATLAS_CELL_ALIGN,
            width,
            height,
        ];
        let rect = [
            slot[0] + ATLAS_PADDING,
            slot[1] + ATLAS_PADDING,
            image.width,
            image.height,
        ];
        self.write(page, slot, &image);
        self.entries.insert(
            key,
            Entry {
                cell: Some(Cell {
                    page,
                    slot,
                    rect,
                    left: image.left,
                    top: image.top,
                }),
                stamp,
            },
        );
        Ok(Some(CellRef {
            page,
            kind,
            rect,
            size: self.size,
            left: image.left,
            top: image.top,
        }))
    }

    fn place(
        &mut self,
        face: &str,
        kind: PageKind,
        units: [u32; 2],
    ) -> Result<(u32, [u32; 2]), Error> {
        let mut victims: Option<Vec<(u64, RasterKey)>> = None;
        loop {
            for (&id, page) in &mut self.pages {
                if page.kind == kind
                    && page.face == face
                    && let Some(origin) = page.shelves.allocate(units[0], units[1])
                {
                    return Ok((id, origin));
                }
            }
            if self.bytes() + kind.page_bytes(self.size) <= self.budget {
                let id = self.create_page(face, kind);
                let origin = self
                    .pages
                    .get_mut(&id)
                    .and_then(|page| page.shelves.allocate(units[0], units[1]))
                    .ok_or(Error::AtlasTooLarge)?;
                return Ok((id, origin));
            }
            let list = victims.get_or_insert_with(|| {
                let start = self.frame_start;
                let mut list = self
                    .entries
                    .iter()
                    .filter(|(_, entry)| entry.cell.is_some() && entry.stamp <= start)
                    .map(|(key, entry)| (entry.stamp, key.clone()))
                    .collect::<Vec<_>>();
                list.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                list
            });
            let (_, key) = list.pop().ok_or(Error::AtlasTooLarge)?;
            self.evict(&key);
        }
    }

    fn evict(&mut self, key: &RasterKey) {
        let Some(Entry {
            cell: Some(cell), ..
        }) = self.entries.remove(key)
        else {
            return;
        };
        self.stats.evicted += 1;
        let Some(page) = self.pages.get_mut(&cell.page) else {
            return;
        };
        page.shelves.release(
            cell.slot[0] / ATLAS_CELL_ALIGN,
            cell.slot[1] / ATLAS_CELL_ALIGN,
            cell.slot[2] / ATLAS_CELL_ALIGN,
        );
        if page.shelves.is_empty() {
            self.drop_page(cell.page);
        }
    }

    fn create_page(&mut self, face: &str, kind: PageKind) -> u32 {
        let id = self.next_page;
        self.next_page += 1;
        let channels = kind.channels();
        let levels = (0..kind.levels())
            .map(|level| {
                let side = (self.size >> level).max(1);
                MipLevel {
                    width: side,
                    height: side,
                    bytes: vec![0; (side * side * channels) as usize],
                }
            })
            .collect();
        self.pages.insert(
            id,
            Page {
                kind,
                face: face.to_owned(),
                atlas: Atlas { channels, levels },
                shelves: Shelves::new(self.size / ATLAS_CELL_ALIGN),
            },
        );
        self.changes.created.push(PageInfo {
            id,
            kind,
            channels,
            size: self.size,
            levels: kind.levels(),
        });
        id
    }

    fn drop_page(&mut self, id: u32) {
        if self.pages.remove(&id).is_none() {
            return;
        }
        self.entries
            .retain(|_, entry| entry.cell.as_ref().is_none_or(|cell| cell.page != id));
        self.changes.cells.retain(|cell| cell.page != id);
        if let Some(at) = self.changes.created.iter().position(|page| page.id == id) {
            self.changes.created.remove(at);
        } else {
            self.changes.dropped.push(id);
        }
    }

    fn write(&mut self, id: u32, slot: [u32; 4], image: &GlyphImage) {
        let page = self.pages.get_mut(&id).unwrap();
        let channels = page.atlas.channels as usize;
        let levels = &mut page.atlas.levels;
        let width = levels[0].width;
        for row in slot[1]..slot[1] + slot[3] {
            let start = ((row * width + slot[0]) as usize) * channels;
            levels[0].bytes[start..start + slot[2] as usize * channels].fill(0);
        }
        let padding = ATLAS_PADDING as i32;
        let origin = [slot[0] as i32 + padding, slot[1] as i32 + padding];
        for dy in -padding..image.height as i32 + padding {
            for dx in -padding..image.width as i32 + padding {
                let sx = dx.clamp(0, image.width as i32 - 1) as usize;
                let sy = dy.clamp(0, image.height as i32 - 1) as usize;
                let src = (sy * image.width as usize + sx) * channels;
                let dst = (((origin[1] + dy) as u32 * width + (origin[0] + dx) as u32) as usize)
                    * channels;
                levels[0].bytes[dst..dst + channels]
                    .copy_from_slice(&image.bytes[src..src + channels]);
            }
        }
        for level in 1..levels.len() {
            let (done, rest) = levels.split_at_mut(level);
            let prev = &done[level - 1];
            let next = &mut rest[0];
            let shift = level as u32;
            let [x0, y0, w, h] = [
                slot[0] >> shift,
                slot[1] >> shift,
                slot[2] >> shift,
                slot[3] >> shift,
            ];
            for row in y0..y0 + h {
                for col in x0..x0 + w {
                    for channel in 0..channels {
                        let mut sum = 0u32;
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let sx = (2 * col + dx).min(prev.width - 1);
                                let sy = (2 * row + dy).min(prev.height - 1);
                                sum += u32::from(
                                    prev.bytes
                                        [(sy * prev.width + sx) as usize * channels + channel],
                                );
                            }
                        }
                        next.bytes[(row * next.width + col) as usize * channels + channel] =
                            ((sum + 2) / 4) as u8;
                    }
                }
            }
        }
        for (index, level) in levels.iter().enumerate() {
            let shift = index as u32;
            let rect = [
                slot[0] >> shift,
                slot[1] >> shift,
                (slot[2] >> shift).max(1),
                (slot[3] >> shift).max(1),
            ];
            let mut bytes = Vec::with_capacity((rect[2] * rect[3]) as usize * channels);
            for row in rect[1]..rect[1] + rect[3] {
                let start = ((row * level.width + rect[0]) as usize) * channels;
                bytes.extend_from_slice(&level.bytes[start..start + rect[2] as usize * channels]);
            }
            self.stats.uploaded += (rect[2] * rect[3]) as u64 * page.kind.texel_bytes();
            self.changes.cells.push(CellUpload {
                page: id,
                level: index as u32,
                rect,
                bytes,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelves_reuse_freed_space_and_return_empty_rows() {
        let mut shelves = Shelves::new(16);
        let a = shelves.allocate(8, 4).unwrap();
        let b = shelves.allocate(8, 4).unwrap();
        assert_eq!(a, [0, 0]);
        assert_eq!(b, [8, 0]);
        assert!(shelves.allocate(1, 4).is_some_and(|at| at[1] == 4));
        shelves.release(a[0], a[1], 8);
        assert_eq!(shelves.allocate(8, 3), Some([0, 0]));
        shelves.release(0, 0, 8);
        shelves.release(8, 0, 8);
        assert_eq!(shelves.shelves.len(), 1);
        shelves.release(0, 4, 1);
        assert!(shelves.is_empty());
        assert_eq!(shelves.rows, vec![[0, 16]]);
        assert_eq!(shelves.allocate(16, 16), Some([0, 0]));
        assert_eq!(shelves.allocate(1, 1), None);
    }

    #[test]
    fn page_bytes_count_every_level_at_its_gpu_format() {
        assert_eq!(PageKind::Msdf.page_bytes(1024), 4 << 20);
        assert_eq!(
            PageKind::Coverage.page_bytes(1024),
            (1 << 20) + (1 << 18) + (1 << 16) + (1 << 14)
        );
    }
}
