pub const NOISE_SIZE: u32 = 64;
pub static NOISE: &[u8] = include_bytes!("dither_noise.bin");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dither {
    pub offset: [u32; 2],
}

impl Dither {
    pub fn seeded(seed: u64) -> Dither {
        let mut h = seed ^ 0x9e37_79b9_7f4a_7c15;
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        h ^= h >> 31;
        Dither {
            offset: [(h as u32) % NOISE_SIZE, ((h >> 32) as u32) % NOISE_SIZE],
        }
    }

    pub fn at(&self, x: u32, y: u32) -> f32 {
        let n = NOISE_SIZE;
        let i = ((y.wrapping_add(self.offset[1]) % n) * n + x.wrapping_add(self.offset[0]) % n)
            as usize;
        let n = (f32::from(NOISE[i]) + 0.5) / 128.0 - 1.0;
        n.signum() * (1.0 - (1.0 - n.abs()).sqrt())
    }

    pub fn tile(&self) -> Tile {
        Tile {
            offsets: (0..NOISE_SIZE)
                .flat_map(|y| (0..NOISE_SIZE).map(move |x| (x, y)))
                .map(|(x, y)| self.at(x, y))
                .collect(),
        }
    }
}

pub struct Tile {
    offsets: Vec<f32>,
}

impl Tile {
    pub fn at(&self, x: u32, y: u32) -> f32 {
        let n = NOISE_SIZE;
        self.offsets[((y % n) * n + x % n) as usize]
    }
}

pub fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mant = f32::from(bits & 0x3ff);
    match (exp, mant == 0.0) {
        (0, _) => sign * mant * 2f32.powi(-24),
        (31, true) => sign * f32::INFINITY,
        (31, false) => f32::NAN,
        _ => sign * (1.0 + mant / 1024.0) * 2f32.powi(exp - 15),
    }
}

pub fn code(encoded: f32) -> u8 {
    if encoded.is_nan() {
        return 0;
    }
    (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn dithered(encoded: f32, offset: f32) -> u8 {
    if encoded.is_nan() || encoded <= 0.0 {
        return 0;
    }
    if encoded >= 1.0 {
        return 255;
    }
    (encoded.clamp(0.0, 1.0) * 255.0 + offset)
        .round()
        .clamp(0.0, 255.0) as u8
}

pub fn wide(encoded: f32) -> u16 {
    if encoded.is_nan() {
        return 0;
    }
    (encoded.clamp(0.0, 1.0) * 65535.0).round() as u16
}

pub fn eight(
    size: [u32; 2],
    origin: [u32; 2],
    dither: Option<Dither>,
    channels: usize,
    pixel: impl Fn(usize) -> [f32; 4],
) -> Vec<u8> {
    let [width, height] = size;
    let tile = dither.map(|d| d.tile());
    let mut out = Vec::with_capacity(width as usize * height as usize * channels);
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) as usize;
            let p = pixel(at);
            let rgb = match &tile {
                Some(tile) => {
                    let offset = tile.at(origin[0] + x, origin[1] + y);
                    [0, 1, 2].map(|k| dithered(p[k], offset))
                }
                None => [0, 1, 2].map(|k| code(p[k])),
            };
            out.extend_from_slice(&rgb);
            if channels == 4 {
                out.push(code(p[3]));
            }
        }
    }
    out
}

pub fn sixteen(pixels: usize, channels: usize, pixel: impl Fn(usize) -> [f32; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels * channels * 2);
    for at in 0..pixels {
        let p = pixel(at);
        for value in &p[..channels] {
            out.extend_from_slice(&wide(*value).to_be_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_holds_every_level_equally_so_the_dither_has_no_bias() {
        assert_eq!(NOISE.len(), (NOISE_SIZE * NOISE_SIZE) as usize);
        let mut counts = [0u32; 256];
        NOISE.iter().for_each(|&v| counts[v as usize] += 1);
        assert!(counts.iter().all(|&c| c == 16));
        let mean: f64 = (0..NOISE_SIZE)
            .flat_map(|y| (0..NOISE_SIZE).map(move |x| (x, y)))
            .map(|(x, y)| f64::from(Dither::default().at(x, y)))
            .sum::<f64>()
            / f64::from(NOISE_SIZE * NOISE_SIZE);
        assert!(mean.abs() < 1e-6, "{mean}");
    }

    #[test]
    fn black_and_white_stay_exact_and_a_gradient_loses_its_bands() {
        let d = Dither::seeded(7);
        for x in 0..NOISE_SIZE {
            assert_eq!(dithered(0.0, d.at(x, 3)), 0);
            assert_eq!(dithered(1.0, d.at(x, 3)), 255);
            assert!((99..=101).contains(&dithered(100.0 / 255.0, d.at(x, 3))));
        }
        let width = 1024u32;
        let ramp = |x: u32| 0.30 + 0.10 * x as f32 / (width - 1) as f32;
        let runs = |row: Vec<u8>| {
            let (mut best, mut run, mut count) = (1, 1, 1);
            for pair in row.windows(2) {
                run = if pair[0] == pair[1] { run + 1 } else { 1 };
                count += usize::from(run == 1);
                best = best.max(run);
            }
            (best, row.len() as f64 / count as f64)
        };
        let (plain, _) = runs((0..width).map(|x| code(ramp(x))).collect());
        let (longest, typical) = runs((0..width).map(|x| dithered(ramp(x), d.at(x, 0))).collect());
        assert!(plain >= 30, "{plain}");
        assert!(longest * 2 <= plain, "{longest}");
        assert!(typical <= 3.0, "{typical}");
        let mean = (0..width)
            .map(|x| f64::from(dithered(ramp(x), d.at(x, 0))) - f64::from(ramp(x)) * 255.0)
            .sum::<f64>()
            / f64::from(width);
        assert!(mean.abs() < 0.05, "{mean}");
    }

    #[test]
    fn halves_decode_exactly() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(0x0001), 2f32.powi(-24));
        assert!(half(0x7e00).is_nan());
        assert_eq!(wide(half(0x3c00)), 65535);
    }

    #[test]
    fn the_same_seed_gives_the_same_pattern_and_seeds_move_it() {
        assert_eq!(Dither::seeded(3), Dither::seeded(3));
        assert_ne!(Dither::seeded(3), Dither::seeded(4));
        assert!(Dither::seeded(9).offset.iter().all(|&o| o < NOISE_SIZE));
    }
}
