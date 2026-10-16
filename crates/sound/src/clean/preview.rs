use super::Masks;
use super::spectrum::{self, Grid};

const HEIGHT: usize = 400;
const MAX_WIDTH: usize = 2000;
const LOW_HZ: f32 = 20.0;
const RANGE_DB: f32 = 80.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

pub(crate) fn draw(input: &[f32], masks: &Masks, size: usize) -> Preview {
    let grid = Grid::new(size, masks.rate, input.len());
    let mags = spectrum::analyse(input, grid).magnitudes();
    let db: Vec<f32> = mags.iter().map(|m| 20.0 * (m + 1e-12).log10()).collect();
    let top = db.iter().copied().fold(f32::MIN, f32::max);
    let width = grid.frames.clamp(1, MAX_WIDTH);
    let nyquist = grid.rate as f32 / 2.0;
    let mut rgb = vec![0u8; width * HEIGHT * 3];
    for y in 0..HEIGHT {
        let up = (HEIGHT - 1 - y) as f32 / (HEIGHT - 1) as f32;
        let hz = LOW_HZ * (nyquist / LOW_HZ).powf(up);
        let k = (grid.bin(hz).round().max(0.0) as usize).min(grid.bins - 1);
        for x in 0..width {
            let t = (x * grid.frames / width).min(grid.frames - 1);
            let i = t * grid.bins + k;
            let v = ((db[i] - (top - RANGE_DB)) / RANGE_DB).clamp(0.0, 1.0);
            let seconds = grid.seconds(t as f32);
            let g: f32 = masks
                .passes
                .iter()
                .map(|pass| pass.gain_at(seconds, hz))
                .product();
            let g = g.clamp(0.0, 1.0);
            let kept = (v * g * 255.0).round() as u8;
            let o = (y * width + x) * 3;
            rgb[o] = (v * 255.0).round() as u8;
            rgb[o + 1] = kept;
            rgb[o + 2] = kept;
        }
    }
    Preview {
        width: width as u32,
        height: HEIGHT as u32,
        rgb,
    }
}
