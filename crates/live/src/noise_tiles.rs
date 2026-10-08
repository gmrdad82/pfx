use pfx_load::{ColorSpace, Image, Pixels};
use pfx_materials::{Hash, hash3};

pub const SIZE: u32 = 512;
pub const CELLS: u32 = 64;
pub const LAYERS: u32 = 2;
pub const OCTAVES: u32 = 2 * LAYERS;
const SALT: i32 = 0x0d17;

struct Lattice {
    values: Vec<f32>,
}

impl Lattice {
    fn new(salt: i32) -> Self {
        let count = (CELLS * CELLS) as usize;
        let hashes: Vec<f32> = (0..CELLS * CELLS)
            .map(|index| {
                let (i, j) = ((index % CELLS) as i32, (index / CELLS) as i32);
                hash3([i, j, salt], Hash::Xor)
            })
            .collect();
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|&a, &b| hashes[a].total_cmp(&hashes[b]).then(a.cmp(&b)));
        let mut values = vec![0.0; count];
        for (rank, &index) in order.iter().enumerate() {
            values[index] = (rank as f32 + 0.5) / count as f32;
        }
        Self { values }
    }

    fn at(&self, i: i64, j: i64) -> f32 {
        let n = i64::from(CELLS);
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
}

fn fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn slice(pair: [f32; 2], depth: f32) -> f32 {
    let k = depth.floor();
    let w = fade(depth - k);
    if k.rem_euclid(2.0) == 0.0 {
        lerp(pair[0], pair[1], w)
    } else {
        lerp(pair[1], pair[0], w)
    }
}

pub struct Octaves {
    layers: Vec<[Lattice; 2]>,
}

impl Default for Octaves {
    fn default() -> Self {
        Self::new()
    }
}

impl Octaves {
    pub fn new() -> Self {
        let layers = (0..OCTAVES as i32)
            .map(|octave| {
                [
                    Lattice::new(SALT + octave * 2),
                    Lattice::new(SALT + octave * 2 + 1),
                ]
            })
            .collect();
        Self { layers }
    }

    pub fn pair(&self, octave: u32, at: [f32; 2]) -> [f32; 2] {
        let [even, odd] = &self.layers[octave as usize];
        [even.value(at[0], at[1]), odd.value(at[0], at[1])]
    }

    pub fn value(&self, octave: u32, at: [f32; 3]) -> f32 {
        slice(self.pair(octave, [at[0], at[1]]), at[2])
    }

    pub fn texel(&self, layer: u32, x: u32, y: u32) -> [f32; 4] {
        let at = [
            (x as f32 + 0.5) / SIZE as f32 * CELLS as f32,
            (y as f32 + 0.5) / SIZE as f32 * CELLS as f32,
        ];
        let low = self.pair(layer * 2, at);
        let high = self.pair(layer * 2 + 1, at);
        [low[0], low[1], high[0], high[1]]
    }
}

pub fn bytes(layer: u32) -> Vec<u8> {
    let octaves = Octaves::new();
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            out.extend(
                octaves
                    .texel(layer, x, y)
                    .map(|value| (value * 255.0).round() as u8),
            );
        }
    }
    out
}

pub fn images() -> Vec<Image> {
    (0..LAYERS)
        .map(|layer| Image {
            width: SIZE,
            height: SIZE,
            space: ColorSpace::Linear,
            pixels: Pixels::Eight(bytes(layer)),
        })
        .collect()
}

pub fn sample(layers: &[Vec<u8>], octave: u32, at: [f32; 3]) -> f32 {
    let bytes = &layers[(octave / 2) as usize];
    let channel = (octave % 2) as usize * 2;
    let x = at[0] / CELLS as f32 * SIZE as f32 - 0.5;
    let y = at[1] / CELLS as f32 * SIZE as f32 - 0.5;
    let (fx, fy) = (x.floor(), y.floor());
    let (u, v) = (x - fx, y - fy);
    let n = i64::from(SIZE);
    let texel = |i: i64, j: i64| -> [f32; 2] {
        let index = ((j.rem_euclid(n) * n + i.rem_euclid(n)) * 4) as usize + channel;
        [f32::from(bytes[index]), f32::from(bytes[index + 1])]
    };
    let (i, j) = (fx as i64, fy as i64);
    let (a, b, c, d) = (
        texel(i, j),
        texel(i + 1, j),
        texel(i, j + 1),
        texel(i + 1, j + 1),
    );
    let pair: [f32; 2] =
        std::array::from_fn(|k| lerp(lerp(a[k], b[k], u), lerp(c[k], d[k], u), v) / 255.0);
    slice(pair, at[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_materials::{Hash, hash2, value_noise3};

    fn noise(p: [f32; 3]) -> f32 {
        value_noise3(p, Hash::Xor)
    }

    fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn unit(seed: u32, i: u32, k: i32) -> f32 {
        hash2([i as i32, k], seed)
    }

    struct Moments {
        count: f64,
        sum: f64,
        squares: f64,
    }

    impl Moments {
        fn of(values: impl Iterator<Item = f32>) -> Self {
            let mut out = Self {
                count: 0.0,
                sum: 0.0,
                squares: 0.0,
            };
            for value in values {
                out.count += 1.0;
                out.sum += f64::from(value);
                out.squares += f64::from(value) * f64::from(value);
            }
            out
        }

        fn mean(&self) -> f64 {
            self.sum / self.count
        }

        fn deviation(&self) -> f64 {
            (self.squares / self.count - self.mean() * self.mean())
                .max(0.0)
                .sqrt()
        }
    }

    fn planes(count: u32, points: u32, extent: f32) -> Vec<[f32; 3]> {
        let mut out = Vec::with_capacity((count * points) as usize);
        for plane in 0..count {
            let depth = unit(41, plane, 0) * 3.0 - 1.5;
            for i in 0..points {
                let a = (unit(plane + 100, i, 1) - 0.5) * extent;
                let b = (unit(plane + 100, i, 2) - 0.5) * extent;
                out.push([a, b, depth]);
            }
        }
        out
    }

    fn tiled(layers: &[Vec<u8>], q: [f32; 3], octave: u32) -> f32 {
        sample(layers, octave, q)
    }

    fn layers() -> Vec<Vec<u8>> {
        (0..LAYERS).map(bytes).collect()
    }

    #[test]
    fn the_tiles_are_the_same_bytes_every_time() {
        use sha2::Digest;
        for (layer, pinned) in TILE_SHA256.iter().enumerate() {
            let first = bytes(layer as u32);
            assert_eq!(first.len(), (SIZE * SIZE * 4) as usize);
            assert_eq!(first, bytes(layer as u32));
            let digest = sha2::Sha256::digest(&first);
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, *pinned, "layer {layer}");
        }
    }

    const TILE_SHA256: [&str; LAYERS as usize] = [
        "a972ac328a5c786fb084df8b3bf8cc5a4f6145bcf8a6b70e2a51deb2d5878177",
        "56ef7600ad51ae1973895711f7f9633789ac8669fad7c0ef0c650aee3d0a9ee3",
    ];

    #[test]
    fn the_tiles_tile_and_share_the_lacquer_tiles_size_and_cells() {
        assert_eq!(SIZE, crate::lacquer::SIZE);
        assert_eq!(CELLS, crate::lacquer::FINE_CELLS);
        let octaves = Octaves::new();
        for i in 0..64 {
            let at = [
                unit(7, i, 0) * 64.0,
                unit(7, i, 1) * 64.0,
                unit(7, i, 2) * 9.0,
            ];
            for octave in 0..OCTAVES {
                let a = octaves.value(octave, at);
                let b = octaves.value(octave, [at[0] + 64.0, at[1] - 128.0, at[2] + 2.0]);
                assert!((a - b).abs() < 1e-4, "{a} {b}");
            }
        }
    }

    #[test]
    fn a_slice_crosses_the_depth_lattice_continuously() {
        let octaves = Octaves::new();
        for i in 0..64 {
            let at = [unit(9, i, 0) * 64.0, unit(9, i, 1) * 64.0];
            for k in -3..4 {
                let edge = k as f32;
                let below = octaves.value(i % OCTAVES, [at[0], at[1], edge - 1e-4]);
                let above = octaves.value(i % OCTAVES, [at[0], at[1], edge + 1e-4]);
                assert!((below - above).abs() < 1e-3, "{below} {above} at {edge}");
            }
        }
    }

    #[test]
    fn the_filtered_tiles_follow_the_exact_octaves() {
        let tiles = layers();
        let octaves = Octaves::new();
        for octave in 0..OCTAVES {
            let mut error = 0.0f64;
            let mut spread = 0.0f64;
            for i in 0..20_000 {
                let at = [
                    unit(octave + 11, i, 0) * 64.0,
                    unit(octave + 11, i, 1) * 64.0,
                    unit(octave + 11, i, 2) * 8.0,
                ];
                let exact = octaves.value(octave, at);
                let read = tiled(&tiles, at, octave);
                error += f64::from(read - exact).powi(2);
                spread += f64::from(exact - 0.5).powi(2);
            }
            let relative = (error / spread).sqrt();
            println!(
                "octave {octave}: filtered tile against exact, relative rms error {relative:.4}"
            );
            assert!(relative < 0.03, "octave {octave}: {relative}");
        }
    }

    #[test]
    fn a_tiled_octave_has_the_hash_noises_statistics_on_planes() {
        let tiles = layers();
        let points = planes(400, 200, 0.4);
        for octave in 0..OCTAVES {
            let hashed = Moments::of(
                points
                    .iter()
                    .map(|p| noise([p[0] * 160.0, p[1] * 160.0, p[2] * 7.3])),
            );
            let read = Moments::of(
                points
                    .iter()
                    .map(|p| tiled(&tiles, [p[0] * 160.0, p[1] * 160.0, p[2] * 7.3], octave)),
            );
            println!(
                "octave {octave}: mean {:.4} hashed, {:.4} tiled; deviation {:.4} hashed, {:.4} tiled",
                hashed.mean(),
                read.mean(),
                hashed.deviation(),
                read.deviation()
            );
            assert!((read.mean() - hashed.mean()).abs() < 0.01);
            assert!((read.deviation() / hashed.deviation() - 1.0).abs() < 0.04);
        }
    }

    fn line(n: f32) -> f32 {
        1.0 - smoothstep(0.0, 0.045, (n - 0.5).abs())
    }

    #[test]
    fn scratch_lines_and_smudges_keep_their_cover_on_planes() {
        let tiles = layers();
        let points = planes(1000, 400, 8.0);
        for (face, side) in [("side", true), ("top", false)] {
            let at = |p: &[f32; 3]| -> [f32; 3] {
                if side {
                    [p[0] * 14.0 + 3.0, p[1] * 700.0, p[2] * 14.0]
                } else {
                    [p[0] * 14.0 + 3.0, p[2] * 700.0, p[1] * 14.0]
                }
            };
            let hashed = Moments::of(points.iter().map(|p| line(noise(at(p)))));
            let read = Moments::of(points.iter().map(|p| {
                let q = at(p);
                let plane = if side {
                    q
                } else {
                    [q[2] + 29.0, q[0] + 7.0, q[1]]
                };
                line(tiled(&tiles, plane, 2))
            }));
            println!(
                "scratch line on a {face} face: cover {:.4} hashed, {:.4} tiled",
                hashed.mean(),
                read.mean()
            );
            assert!((read.mean() / hashed.mean() - 1.0).abs() < 0.06);
        }
        let smudge = |a: f32, b: f32| smoothstep(0.52, 0.78, a * 0.65 + b * 0.35);
        let hashed = Moments::of(
            points
                .iter()
                .map(|p| smudge(noise(p.map(|v| v * 20.0)), noise(p.map(|v| v * 50.0)))),
        );
        let read = Moments::of(points.iter().map(|p| {
            smudge(
                tiled(&tiles, p.map(|v| v * 20.0), 3),
                tiled(&tiles, p.map(|v| v * 50.0), 1),
            )
        }));
        println!(
            "scratch smudge: cover {:.4} hashed, {:.4} tiled; deviation {:.4} hashed, {:.4} tiled",
            hashed.mean(),
            read.mean(),
            hashed.deviation(),
            read.deviation()
        );
        assert!((read.mean() - hashed.mean()).abs() < 0.01);
        assert!((read.deviation() / hashed.deviation() - 1.0).abs() < 0.08);
    }

    #[test]
    fn the_shader_reads_the_tiles_with_their_constants() {
        let line = format!("const tile_cells: f32 = {}.0;", CELLS);
        assert!(crate::maps::MAPS_WGSL.contains(&line), "{line}");
    }
}
