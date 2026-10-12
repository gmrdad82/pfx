pub const PATTERNS: u32 = 256;
pub const DIMENSIONS: u32 = 4;
pub const MIN_SAMPLES: u32 = 256;
pub const MAX_SAMPLES: u32 = 8192;
pub const TABLE_WIDTH: u32 = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampler {
    #[default]
    Pcg,
    Sobol {
        samples: u32,
    },
}

impl Sampler {
    pub fn sequence(self) -> Option<u32> {
        match self {
            Sampler::Pcg => None,
            Sampler::Sobol { samples } => Some(sequence_size(samples)),
        }
    }
}

pub fn sequence_size(samples: u32) -> u32 {
    samples
        .max(1)
        .checked_next_power_of_two()
        .unwrap_or(MAX_SAMPLES)
        .clamp(MIN_SAMPLES, MAX_SAMPLES)
}

const XORS: [[u32; 32]; 4] = [
    [0; 32],
    [
        0x00000000, 0x00000001, 0x00000001, 0x00000007, 0x00000001, 0x00000013, 0x00000015,
        0x0000007f, 0x00000001, 0x00000103, 0x00000105, 0x0000070f, 0x00000111, 0x00001333,
        0x00001555, 0x00007fff, 0x00000001, 0x00010003, 0x00010005, 0x0007000f, 0x00010011,
        0x00130033, 0x00150055, 0x007f00ff, 0x00010101, 0x01030303, 0x01050505, 0x070f0f0f,
        0x01111111, 0x13333333, 0x15555555, 0x7fffffff,
    ],
    [
        0x00000000, 0x00000001, 0x00000003, 0x00000001, 0x00000005, 0x0000001f, 0x0000002b,
        0x0000003d, 0x00000011, 0x00000133, 0x00000377, 0x00000199, 0x00000445, 0x00001ccf,
        0x00002ddb, 0x0000366d, 0x00000101, 0x00010303, 0x00030707, 0x00010909, 0x00051515,
        0x001f3f3f, 0x002b6b6b, 0x003dbdbd, 0x00101011, 0x01303033, 0x03707077, 0x01909099,
        0x04515145, 0x1cf3f3cf, 0x2db6b6db, 0x36dbdb6d,
    ],
    [
        0x00000000, 0x00000001, 0x00000000, 0x00000003, 0x0000000d, 0x0000000c, 0x00000005,
        0x0000004f, 0x00000014, 0x000000e7, 0x00000329, 0x0000039c, 0x00000011, 0x00001033,
        0x00000044, 0x000030bb, 0x0000d1cd, 0x0000c2ec, 0x00005415, 0x0004fc3f, 0x00015054,
        0x000e5c97, 0x0032e5b9, 0x0039725c, 0x00000101, 0x01000303, 0x00000404, 0x03000b0b,
        0x0d001d1d, 0x0c002c2c, 0x05004545, 0x4f00cfcf,
    ],
];

fn hash_hp_uint(mut i: u32) -> u32 {
    i ^= i >> 16;
    i = i.wrapping_mul(0x21f0aaad);
    i ^= i >> 15;
    i = i.wrapping_mul(0xd35a2d97);
    i ^= i >> 15;
    i ^ 0xe6fe3beb
}

fn hash_hp_seeded_uint(i: u32, mut seed: u32) -> u32 {
    seed ^= seed << 19;
    hash_hp_uint(i ^ seed)
}

fn hash_hp_float(i: u32) -> f32 {
    hash_hp_uint(i) as f32 * (1.0 / 4_294_967_808.0)
}

pub fn generate_4d(points: &mut [[f32; 4]], seed: u32) {
    let size = points.len();
    if size == 0 {
        return;
    }
    let mut rng = hash_hp_seeded_uint(seed, 0x44605a73);
    let mut next = || {
        let value = hash_hp_float(rng);
        rng = rng.wrapping_add(1);
        value
    };
    points[0] = [next(), next(), next(), next()];
    let mut log_n = 0;
    let mut n = 1;
    while n < size {
        let strata = (n * 2) as f32;
        let mut i = 0;
        while i < n && n + i < size {
            for axis in 0..4 {
                let occupied = (points[i ^ XORS[axis][log_n] as usize][axis] * strata) as u32;
                points[n + i][axis] = ((occupied ^ 1) as f32 + next()) / strata;
            }
            i += 1;
        }
        log_n += 1;
        n *= 2;
    }
}

pub fn table(sequence: u32) -> Vec<[f32; 4]> {
    let length = sequence as usize;
    let mut points = vec![[0.0; 4]; length * PATTERNS as usize];
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let per = (PATTERNS as usize).div_ceil(threads) * length;
    std::thread::scope(|scope| {
        for (chunk, block) in points.chunks_mut(per).enumerate() {
            scope.spawn(move || {
                for (j, pattern) in block.chunks_mut(length).enumerate() {
                    generate_4d(pattern, (chunk * per / length + j) as u32);
                }
            });
        }
    });
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_same_bytes_every_time() {
        let a = table(MIN_SAMPLES);
        let b = table(MIN_SAMPLES);
        assert_eq!(a.len(), (MIN_SAMPLES * PATTERNS) as usize);
        assert!(a.iter().flatten().all(|v| (0.0..1.0).contains(v)));
        assert_eq!(a, b);
        let mut single = vec![[0.0; 4]; MIN_SAMPLES as usize];
        generate_4d(&mut single, 37);
        assert_eq!(
            &a[37 * MIN_SAMPLES as usize..38 * MIN_SAMPLES as usize],
            &single[..]
        );
    }

    #[test]
    fn every_power_of_two_prefix_is_stratified_in_each_axis() {
        let mut points = vec![[0.0; 4]; 1024];
        generate_4d(&mut points, 5);
        for n in [2usize, 16, 128, 1024] {
            for axis in 0..4 {
                let mut seen = vec![false; n];
                for point in &points[..n] {
                    seen[(point[axis] * n as f32) as usize] = true;
                }
                assert!(seen.iter().all(|s| *s), "{n} points, axis {axis}");
            }
        }
    }

    #[test]
    fn the_sequence_follows_the_sample_count() {
        assert_eq!(sequence_size(1), MIN_SAMPLES);
        assert_eq!(sequence_size(300), 512);
        assert_eq!(sequence_size(4096), 4096);
        assert_eq!(sequence_size(u32::MAX), MAX_SAMPLES);
        assert_eq!(Sampler::default().sequence(), None);
    }
}
