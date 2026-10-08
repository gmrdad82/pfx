use pfx_load::{ColorSpace, Image, Pixels};
use pfx_materials::{CoatWobble, Hash, hash3};

pub const SIZE: u32 = 512;
pub const BROAD_RANGE: f32 = 2.5;
pub const FINE_RANGE: f32 = 1.5;
pub const FINE_CELLS: u32 = 64;
pub const SLICE: f32 = 0.861_891_6;
const BROAD_SALT: i32 = 0x55;
const COARSE_SALT: i32 = 0x16;
const FINE_SALT: i32 = 0x95;
pub const BLEND: f32 = 0.5;
const STEP: f32 = 0.35;

struct Lattice {
    cells: u32,
    values: Vec<f32>,
}

impl Lattice {
    fn new(cells: u32, salt: i32) -> Self {
        let values = (0..cells * cells)
            .map(|index| {
                let (i, j) = ((index % cells) as i32, (index / cells) as i32);
                hash3([i, j, salt], Hash::Xor)
            })
            .collect();
        Self { cells, values }
    }

    fn at(&self, i: i64, j: i64) -> f32 {
        let n = i64::from(self.cells);
        self.values[(j.rem_euclid(n) * n + i.rem_euclid(n)) as usize]
    }

    fn value(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (x.floor(), y.floor());
        let (i, j) = (fx as i64, fy as i64);
        let (u, v) = (fade(x - fx), fade(y - fy));
        let a = lerp(self.at(i, j), self.at(i + 1, j), u);
        let b = lerp(self.at(i, j + 1), self.at(i + 1, j + 1), u);
        lerp(a, b, v)
    }

    fn gradient(&self, x: f32, y: f32) -> [f32; 2] {
        [
            (self.value(x + STEP, y) - self.value(x - STEP, y)) / (2.0 * STEP),
            (self.value(x, y + STEP) - self.value(x, y - STEP)) / (2.0 * STEP),
        ]
    }
}

fn fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub struct Octaves {
    broad: Lattice,
    coarse: Lattice,
    fine: Lattice,
    weights: [f32; 2],
}

pub fn cells(frequency: f32) -> u32 {
    frequency.round().clamp(1.0, SIZE as f32) as u32
}

impl Octaves {
    pub fn new(coat: &CoatWobble) -> Self {
        Self {
            broad: Lattice::new(cells(coat.frequencies[1]), BROAD_SALT),
            coarse: Lattice::new(cells(coat.frequencies[2]), COARSE_SALT),
            fine: Lattice::new(FINE_CELLS, FINE_SALT),
            weights: coat.weights,
        }
    }

    pub fn broad(&self, at: [f32; 2]) -> [f32; 2] {
        let (broad, coarse) = (self.broad.cells as f32, self.coarse.cells as f32);
        let a = self.broad.gradient(at[0] * broad, at[1] * broad);
        let b = self.coarse.gradient(at[0] * coarse, at[1] * coarse);
        let [wa, wb] = self.weights;
        [
            (a[0] * wa + b[0] * wb) * SLICE,
            (a[1] * wa + b[1] * wb) * SLICE,
        ]
    }

    pub fn fine(&self, at: [f32; 2]) -> [f32; 2] {
        let cells = FINE_CELLS as f32;
        let g = self.fine.gradient(at[0] * cells, at[1] * cells);
        [g[0] * SLICE, g[1] * SLICE]
    }

    pub fn texel(&self, x: u32, y: u32) -> [f32; 4] {
        let at = [
            (x as f32 + 0.5) / SIZE as f32,
            (y as f32 + 0.5) / SIZE as f32,
        ];
        let broad = self.broad(at);
        let fine = self.fine(at);
        [
            encode(broad[0], BROAD_RANGE),
            encode(broad[1], BROAD_RANGE),
            encode(fine[0], FINE_RANGE),
            encode(fine[1], FINE_RANGE),
        ]
    }
}

fn encode(value: f32, range: f32) -> f32 {
    (value / range * 0.5 + 0.5).clamp(0.0, 1.0)
}

pub fn decode(byte: u8, range: f32) -> f32 {
    (f32::from(byte) / 255.0 * 2.0 - 1.0) * range
}

pub fn fine_scale(coat: &CoatWobble) -> f32 {
    coat.frequencies[0] / FINE_CELLS as f32
}

pub fn bytes(coat: &CoatWobble) -> Vec<u8> {
    let octaves = Octaves::new(coat);
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            out.extend(
                octaves
                    .texel(x, y)
                    .map(|value| (value * 255.0).round() as u8),
            );
        }
    }
    out
}

pub fn image(coat: &CoatWobble) -> Image {
    Image {
        width: SIZE,
        height: SIZE,
        space: ColorSpace::Linear,
        pixels: Pixels::Eight(bytes(coat)),
    }
}

pub fn sample(bytes: &[u8], at: [f32; 2]) -> [f32; 4] {
    let x = at[0] * SIZE as f32 - 0.5;
    let y = at[1] * SIZE as f32 - 0.5;
    let (fx, fy) = (x.floor(), y.floor());
    let (u, v) = (x - fx, y - fy);
    let n = i64::from(SIZE);
    let texel = |i: i64, j: i64| -> [f32; 4] {
        let index = ((j.rem_euclid(n) * n + i.rem_euclid(n)) * 4) as usize;
        std::array::from_fn(|c| f32::from(bytes[index + c]))
    };
    let (i, j) = (fx as i64, fy as i64);
    let (a, b, c, d) = (
        texel(i, j),
        texel(i + 1, j),
        texel(i, j + 1),
        texel(i + 1, j + 1),
    );
    let mixed: [f32; 4] =
        std::array::from_fn(|k| lerp(lerp(a[k], b[k], u), lerp(c[k], d[k], u), v) / 255.0);
    [
        (mixed[0] * 2.0 - 1.0) * BROAD_RANGE,
        (mixed[1] * 2.0 - 1.0) * BROAD_RANGE,
        (mixed[2] * 2.0 - 1.0) * FINE_RANGE,
        (mixed[3] * 2.0 - 1.0) * FINE_RANGE,
    ]
}

pub fn weights(face: [f32; 3]) -> [f32; 3] {
    let size = (face[0] * face[0] + face[1] * face[1] + face[2] * face[2]).sqrt();
    if size <= 0.0 {
        return [0.0, 0.0, 1.0];
    }
    let w = face.map(|v| (v.abs() / size - BLEND).max(0.0));
    let total = w[0] + w[1] + w[2];
    w.map(|v| v / total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_materials::value_noise3_gradient;

    const COAT: CoatWobble = CoatWobble::SAMPLE;

    fn tangential(g: [f32; 3]) -> [f32; 2] {
        [g[0], g[1]]
    }

    fn rms(values: &[[f32; 2]]) -> f64 {
        let total: f64 = values
            .iter()
            .map(|g| f64::from(g[0] * g[0] + g[1] * g[1]))
            .sum();
        (total / values.len() as f64).sqrt()
    }

    fn points(count: usize, seed: u32) -> Vec<[f32; 2]> {
        (0..count as u32)
            .map(|i| {
                [
                    pfx_materials::hash2([i as i32, 1], seed),
                    pfx_materials::hash2([i as i32, 2], seed),
                ]
            })
            .collect()
    }

    #[test]
    fn the_slice_factor_is_the_root_of_a_smoothstep_blend_of_two_layers() {
        let steps = 200_000;
        let mean = (0..steps)
            .map(|i| {
                let w = fade((i as f32 + 0.5) / steps as f32);
                f64::from((1.0 - w) * (1.0 - w) + w * w)
            })
            .sum::<f64>()
            / steps as f64;
        assert!((mean - 26.0 / 35.0).abs() < 1e-6, "{mean}");
        assert!((f64::from(SLICE) - (26.0f64 / 35.0).sqrt()).abs() < 1e-6);
    }

    #[test]
    fn the_tile_is_the_same_bytes_every_time() {
        let first = bytes(&COAT);
        assert_eq!(first.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(first, bytes(&COAT));
        use sha2::Digest;
        let digest = sha2::Sha256::digest(&first);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, TILE_SHA256);
    }

    const TILE_SHA256: &str = "7e79b641b9311c427aee4de99958d8d1ecb4efb10a0edadcf6aa98fb1a247eac";

    #[test]
    fn no_coat_leaves_the_broad_channels_flat_and_keeps_the_fine_one() {
        let flat = bytes(&CoatWobble::default());
        assert!(flat.chunks(4).all(|texel| texel[..2] == [128, 128]));
        assert!(flat.chunks(4).any(|texel| texel[2..] != [128, 128]));
    }

    #[test]
    fn the_tile_tiles() {
        let octaves = Octaves::new(&COAT);
        for at in points(64, 7) {
            let a = octaves.broad(at);
            let b = octaves.broad([at[0] + 1.0, at[1] - 1.0]);
            let c = octaves.fine(at);
            let d = octaves.fine([at[0] - 1.0, at[1] + 2.0]);
            for k in 0..2 {
                assert!((a[k] - b[k]).abs() < 1e-3);
                assert!((c[k] - d[k]).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn the_codes_cover_the_gradients() {
        let octaves = Octaves::new(&COAT);
        let mut broad = 0.0f32;
        let mut fine = 0.0f32;
        for y in (0..SIZE).step_by(3) {
            for x in (0..SIZE).step_by(3) {
                let at = [
                    (x as f32 + 0.5) / SIZE as f32,
                    (y as f32 + 0.5) / SIZE as f32,
                ];
                let a = octaves.broad(at);
                let b = octaves.fine(at);
                broad = broad.max(a[0].abs()).max(a[1].abs());
                fine = fine.max(b[0].abs()).max(b[1].abs());
            }
        }
        assert!(broad < BROAD_RANGE, "{broad}");
        assert!(fine < FINE_RANGE, "{fine}");
    }

    #[test]
    fn the_filtered_tile_follows_the_exact_gradients() {
        let tile = bytes(&COAT);
        let octaves = Octaves::new(&COAT);
        let at = points(20_000, 11);
        let mut broad_error = Vec::new();
        let mut fine_error = Vec::new();
        let mut broad_exact = Vec::new();
        let mut fine_exact = Vec::new();
        for p in &at {
            let sampled = sample(&tile, *p);
            let a = octaves.broad(*p);
            let b = octaves.fine(*p);
            broad_error.push([sampled[0] - a[0], sampled[1] - a[1]]);
            fine_error.push([sampled[2] - b[0], sampled[3] - b[1]]);
            broad_exact.push(a);
            fine_exact.push(b);
        }
        let broad = rms(&broad_error) / rms(&broad_exact);
        let fine = rms(&fine_error) / rms(&fine_exact);
        println!("filtered tile against exact gradients: broad {broad:.4}, fine {fine:.4}");
        assert!(broad < 0.03, "broad relative error {broad}");
        assert!(fine < 0.03, "fine relative error {fine}");
    }

    #[test]
    fn the_tile_matches_the_procedural_bump_on_planes() {
        let tile = bytes(&COAT);
        let mut procedural = Vec::new();
        let mut sampled = Vec::new();
        for plane in 0..64u32 {
            let z = pfx_materials::hash2([plane as i32, 9], 3) * 0.4 - 0.2;
            for p in points(400, plane + 100) {
                let q = [p[0] * 0.3 - 0.15, p[1] * 0.3 - 0.15];
                let g = COAT
                    .frequencies
                    .iter()
                    .zip([1.0, COAT.weights[0], COAT.weights[1]])
                    .map(|(&scale, weight)| {
                        tangential(value_noise3_gradient([q[0], q[1], z], scale))
                            .map(|v| v * weight)
                    })
                    .fold([0.0f32; 2], |a, b| [a[0] + b[0], a[1] + b[1]]);
                procedural.push(g);
                let t = sample(&tile, [q[0], q[1]]);
                let f = sample(&tile, [q[0] * fine_scale(&COAT), q[1] * fine_scale(&COAT)]);
                sampled.push([t[0] + f[2], t[1] + f[3]]);
            }
        }
        let ratio = rms(&sampled) / rms(&procedural);
        println!("tile against procedural bump on planes: rms ratio {ratio:.4}");
        assert!((ratio - 1.0).abs() < 0.05, "rms ratio {ratio}");
    }

    #[test]
    fn the_shader_reads_the_tile_with_its_constants() {
        for (name, value) in [
            ("broad_range", BROAD_RANGE),
            ("fine_range", FINE_RANGE),
            ("blend", BLEND),
        ] {
            let line = format!("const lacquer_{name}: f32 = {value};");
            assert!(crate::maps::MAPS_WGSL.contains(&line), "{line}");
        }
    }

    #[test]
    fn faces_pick_one_plane_near_an_axis_and_blend_between() {
        assert_eq!(weights([0.0, 0.0, -3.0]), [0.0, 0.0, 1.0]);
        assert_eq!(weights([0.0, 2.0, 0.0]), [0.0, 1.0, 0.0]);
        assert_eq!(weights([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        let tilted = weights([0.3f32.sin(), 0.0, 0.3f32.cos()]);
        assert_eq!(tilted, [0.0, 0.0, 1.0]);
        let half = weights([1.0, 0.0, 1.0]);
        assert!((half[0] - 0.5).abs() < 1e-6 && (half[2] - 0.5).abs() < 1e-6);
        let corner = weights([1.0, 1.0, 1.0]);
        assert!(corner.iter().all(|w| (w - 1.0 / 3.0).abs() < 1e-6));
    }
}
