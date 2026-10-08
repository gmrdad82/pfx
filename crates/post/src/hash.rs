pub const BLUE_NOISE_SIZE: u32 = 64;
pub static BLUE_NOISE: &[u8] = include_bytes!("blue_noise.bin");

const BAYER8: [u8; 64] = [
    0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60,
    28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15,
    47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
];

pub fn grain_hash(x: u32, y: u32, seed: u32) -> f32 {
    let mut h =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h >> 8) as f32 / 16_777_216.0
}

pub fn mixed(seed: u32, frame: u32) -> u32 {
    seed ^ frame.wrapping_mul(0x9e37_79b9)
}

pub fn hash21(x: f32, y: f32) -> f32 {
    let mut qx = (x * 123.34).fract();
    let mut qy = (y * 456.21).fract();
    let d = qx * (qx + 45.32) + qy * (qy + 45.32);
    qx += d;
    qy += d;
    (qx * qy).fract()
}

pub fn bayer(x: u32, y: u32) -> f32 {
    let v = BAYER8[((y & 7) * 8 + (x & 7)) as usize];
    (v as f32 + 0.5) / 64.0
}

pub fn blue_noise(x: u32, y: u32) -> f32 {
    let n = BLUE_NOISE_SIZE;
    let i = ((y % n) * n + (x % n)) as usize;
    BLUE_NOISE[i] as f32 / 255.0
}

pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = {
        let t = x - x0;
        t * t * (3.0 - 2.0 * t)
    };
    let ty = {
        let t = y - y0;
        t * t * (3.0 - 2.0 * t)
    };
    let xi = x0 as i32;
    let yi = y0 as i32;
    let h = |i: i32, j: i32| grain_hash(i as u32, j as u32, seed);
    let a = h(xi, yi);
    let b = h(xi + 1, yi);
    let c = h(xi, yi + 1);
    let d = h(xi + 1, yi + 1);
    let ab = a + (b - a) * tx;
    let cd = c + (d - c) * tx;
    ab + (cd - ab) * ty
}
