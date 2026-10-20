pub const MIN_TILE_ALIGNMENT: u32 = 16;
pub const DEFAULT_MAX_TILE_PIXELS: u64 = 2160 * 2160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub src: [u32; 2],
    pub dst: [u32; 2],
    pub size: [u32; 2],
    pub out_src: [u32; 2],
    pub out_dst: [u32; 2],
    pub out_size: [u32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub tile_h: u32,
    pub tile_w: u32,
    pub overlap: u32,
    pub tiles: Vec<Tile>,
}

fn round_up(a: u32, b: u32) -> u32 {
    a.div_ceil(b) * b
}

fn round_up_rem(a: u32, b: u32, c: u32) -> u32 {
    (a + b - c - 1) / b * b + c
}

pub fn plan(height: u32, width: u32, max_tile_pixels: u64) -> Plan {
    let alignment = MIN_TILE_ALIGNMENT;
    let overlap = round_up(crate::net::RECEPTIVE_FIELD / 2, MIN_TILE_ALIGNMENT);
    let (h, w) = (height, width);
    let mut tile_h = round_up(h, MIN_TILE_ALIGNMENT);
    let mut tile_w = round_up(w, MIN_TILE_ALIGNMENT);
    let pad_h = tile_h % alignment;
    let pad_w = tile_w % alignment;
    let mut count_h = 1u32;
    let mut count_w = 1u32;
    let min_dim = (4 * overlap).max(768);
    let min_tile_h = round_up_rem(min_dim, alignment, pad_h);
    let min_tile_w = round_up_rem(min_dim, alignment, pad_w);
    let span_h = 2 * overlap + pad_h;
    let span_w = 2 * overlap + pad_w;
    while u64::from(tile_h) * u64::from(tile_w) > max_tile_pixels {
        if tile_h > min_tile_h && tile_h > tile_w {
            let next = (h + span_h * count_h).div_ceil(count_h + 1);
            tile_h = round_up_rem(next, alignment, pad_h).clamp(min_tile_h, tile_h - alignment);
            count_h = h.saturating_sub(span_h).div_ceil(tile_h - span_h).max(1);
        } else if tile_w > min_tile_w {
            let next = (w + span_w * count_w).div_ceil(count_w + 1);
            tile_w = round_up_rem(next, alignment, pad_w).clamp(min_tile_w, tile_w - alignment);
            count_w = w.saturating_sub(span_w).div_ceil(tile_w - span_w).max(1);
        } else {
            break;
        }
    }
    let mut tiles = Vec::new();
    for i in 0..count_h {
        let y = i * tile_h.saturating_sub(span_h);
        let begin_h = if i > 0 { overlap } else { 0 };
        let end_h = if i < count_h - 1 { overlap + pad_h } else { 0 };
        let tile_h1 = (h - y).min(tile_h);
        let tile_h2 = tile_h1 - begin_h - end_h;
        let align_h = tile_h - round_up(tile_h1, MIN_TILE_ALIGNMENT);
        for j in 0..count_w {
            let x = j * tile_w.saturating_sub(span_w);
            let begin_w = if j > 0 { overlap } else { 0 };
            let end_w = if j < count_w - 1 { overlap + pad_w } else { 0 };
            let tile_w1 = (w - x).min(tile_w);
            let tile_w2 = tile_w1 - begin_w - end_w;
            let align_w = tile_w - round_up(tile_w1, MIN_TILE_ALIGNMENT);
            tiles.push(Tile {
                src: [y, x],
                dst: [align_h, align_w],
                size: [tile_h1, tile_w1],
                out_src: [align_h + begin_h, align_w + begin_w],
                out_dst: [y + begin_h, x + begin_w],
                out_size: [tile_h2, tile_w2],
            });
        }
    }
    Plan {
        tile_h,
        tile_w,
        overlap,
        tiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(plan: &Plan, height: u32, width: u32) -> bool {
        let mut hits = vec![0u8; (height * width) as usize];
        for tile in &plan.tiles {
            for y in 0..tile.out_size[0] {
                for x in 0..tile.out_size[1] {
                    let at = (tile.out_dst[0] + y) * width + tile.out_dst[1] + x;
                    hits[at as usize] += 1;
                }
            }
        }
        hits.iter().all(|&hit| hit == 1)
    }

    #[test]
    fn tiles_cover_every_pixel_once_as_oidn_splits_them() {
        let one = plan(1080, 1920, DEFAULT_MAX_TILE_PIXELS);
        assert_eq!((one.tile_h, one.tile_w, one.tiles.len()), (1088, 1920, 1));
        assert!(covered(&one, 1080, 1920));
        assert!(covered(&plan(24, 32, DEFAULT_MAX_TILE_PIXELS), 24, 32));
        let four_k = plan(2160, 3840, DEFAULT_MAX_TILE_PIXELS);
        assert_eq!(
            (four_k.tile_h, four_k.tile_w, four_k.tiles.len()),
            (2160, 2016, 2)
        );
        assert!(covered(&four_k, 2160, 3840));
        for (h, w, max) in [
            (1000, 3000, 1 << 20),
            (777, 1333, 600_000),
            (5000, 4000, 2160 * 2160),
        ] {
            let split = plan(h, w, max);
            assert!(covered(&split, h, w), "{h}x{w}");
            assert!(split.tiles.len() > 1);
        }
        assert_eq!(plan(1200, 1200, 768 * 768).tiles.len(), 4);
    }
}
