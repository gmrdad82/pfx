const MANTISSA: i32 = 9;
const BIAS: i32 = 15;
const MAX_EXPONENT: i32 = 31;
pub const RGB9E5_MAX: f32 = 65408.0;
pub const DEPTH_STEPS: f32 = 65535.0;
pub const SNORM16: f32 = 32767.0;

fn floor_log2(value: f32) -> i32 {
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32;
    if exponent == 0 { -127 } else { exponent - 127 }
}

pub fn rgb9e5(color: [f32; 3]) -> u32 {
    let clamped = color.map(|value| {
        if value.is_finite() {
            value.clamp(0.0, RGB9E5_MAX)
        } else if value == f32::INFINITY {
            RGB9E5_MAX
        } else {
            0.0
        }
    });
    let largest = clamped[0].max(clamped[1]).max(clamped[2]);
    if largest <= 0.0 {
        return 0;
    }
    let mut exponent = floor_log2(largest).max(-BIAS - 1) + 1 + BIAS;
    let mut scale = 2f32.powi(exponent - BIAS - MANTISSA);
    if (largest / scale + 0.5).floor() as i32 == 1 << MANTISSA {
        scale *= 2.0;
        exponent += 1;
    }
    let exponent = exponent.min(MAX_EXPONENT) as u32;
    let [r, g, b] = clamped.map(|value| ((value / scale + 0.5).floor() as u32).min(511));
    r | (g << 9) | (b << 18) | (exponent << 27)
}

pub fn from_rgb9e5(packed: u32) -> [f32; 3] {
    let exponent = (packed >> 27) as i32;
    let scale = 2f32.powi(exponent - BIAS - MANTISSA);
    [packed & 511, (packed >> 9) & 511, (packed >> 18) & 511].map(|m| m as f32 * scale)
}

fn sign(value: f32) -> f32 {
    if value >= 0.0 { 1.0 } else { -1.0 }
}

pub fn oct16(normal: [f32; 3]) -> u32 {
    let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length <= 1e-12 || !length.is_finite() {
        return 0;
    }
    let n = normal.map(|v| v / length);
    let l1 = n[0].abs() + n[1].abs() + n[2].abs();
    let mut x = n[0] / l1;
    let mut y = n[1] / l1;
    if n[2] < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * sign(ox);
        y = (1.0 - ox.abs()) * sign(oy);
    }
    let pack = |v: f32| ((v.clamp(-1.0, 1.0) * SNORM16).round() as i16) as u16 as u32;
    pack(x) | (pack(y) << 16)
}

pub fn from_oct16(packed: u32) -> [f32; 3] {
    let unpack = |bits: u32| (bits as u16 as i16) as f32 / SNORM16;
    let x = unpack(packed & 0xffff).max(-1.0);
    let y = unpack(packed >> 16).max(-1.0);
    let z = 1.0 - x.abs() - y.abs();
    let (x, y) = if z < 0.0 {
        ((1.0 - y.abs()) * sign(x), (1.0 - x.abs()) * sign(y))
    } else {
        (x, y)
    };
    let length = (x * x + y * y + z * z).sqrt();
    [x / length, y / length, z / length]
}

pub fn inverse_depth(depth: f32, near: f32) -> u16 {
    if depth <= 0.0 || !depth.is_finite() {
        return 0;
    }
    (near / depth * DEPTH_STEPS).round().clamp(1.0, DEPTH_STEPS) as u16
}

pub fn from_inverse_depth(code: u16, near: f32) -> Option<f32> {
    (code != 0).then(|| near * DEPTH_STEPS / f32::from(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative(a: f32, b: f32) -> f32 {
        (a - b).abs() / b.abs().max(1e-30)
    }

    #[test]
    fn rgb9e5_round_trips_within_its_mantissa_and_clamps_the_rest() {
        assert_eq!(rgb9e5([0.0; 3]), 0);
        assert_eq!(from_rgb9e5(0), [0.0; 3]);
        assert_eq!(from_rgb9e5(rgb9e5([1.0, 0.5, 0.25])), [1.0, 0.5, 0.25]);
        assert_eq!(from_rgb9e5(rgb9e5([-1.0, f32::NAN, 2.0]))[..2], [0.0, 0.0]);
        assert_eq!(from_rgb9e5(rgb9e5([1e9, 0.0, 0.0]))[0], RGB9E5_MAX);
        let mut state = 0x9e3779b9u32;
        for _ in 0..20000 {
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32) * 2f32.powi((state % 24) as i32 - 12)
            };
            let color = [next(), next(), next()];
            let back = from_rgb9e5(rgb9e5(color));
            let largest = color[0].max(color[1]).max(color[2]);
            for k in 0..3 {
                assert!(
                    (back[k] - color[k]).abs() <= largest / 256.0 + 1e-9,
                    "{color:?} came back as {back:?}"
                );
            }
            assert!(relative(back[0].max(back[1]).max(back[2]), largest) <= 1.0 / 512.0 + 1e-6);
        }
    }

    #[test]
    fn rgb9e5_matches_the_gpu_layout() {
        let one = rgb9e5([1.0, 0.0, 0.0]);
        assert_eq!(one & 511, 256);
        assert_eq!(one >> 27, 16);
        let blue = rgb9e5([0.0, 0.0, 1.0]);
        assert_eq!((blue >> 18) & 511, 256);
    }

    #[test]
    fn oct16_round_trips_every_direction_within_a_hundredth_of_a_degree() {
        let mut worst: f32 = 0.0;
        for i in 0..64 {
            for j in 0..128 {
                let theta = std::f32::consts::PI * (i as f32 + 0.5) / 64.0;
                let phi = std::f32::consts::TAU * j as f32 / 128.0;
                let n = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let back = from_oct16(oct16(n));
                let a = n.map(f64::from);
                let b = back.map(f64::from);
                let cross = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                let sin = cross.iter().map(|v| v * v).sum::<f64>().sqrt();
                worst = worst.max(sin.asin().to_degrees() as f32);
            }
        }
        assert!(worst < 0.01, "{worst}");
        assert_eq!(from_oct16(oct16([0.0, 0.0, -1.0])), [0.0, 0.0, -1.0]);
        assert_eq!(oct16([0.0; 3]), 0);
    }

    #[test]
    fn inverse_depth_keeps_its_step_and_marks_the_sky() {
        assert_eq!(inverse_depth(0.0, 0.5), 0);
        assert_eq!(inverse_depth(f32::INFINITY, 0.5), 0);
        assert_eq!(from_inverse_depth(0, 0.5), None);
        assert_eq!(inverse_depth(0.5, 0.5), 65535);
        assert_eq!(inverse_depth(0.4, 0.5), 65535);
        for depth in [0.5f32, 1.0, 3.0, 10.0] {
            let back = from_inverse_depth(inverse_depth(depth, 0.5), 0.5).unwrap();
            let step = depth * depth / (0.5 * DEPTH_STEPS);
            assert!((back - depth).abs() <= step * 0.5 + 1e-6, "{depth} {back}");
        }
    }
}
