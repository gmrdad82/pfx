use crate::math::{fade, fract, lerp};

pub fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Hash {
    #[default]
    Xor,
    Nested,
}

pub fn hash3(p: [i32; 3], hash: Hash) -> f32 {
    match hash {
        Hash::Xor => hash3_xor(p),
        Hash::Nested => hash3_nested(p),
    }
}

fn hash3_xor(p: [i32; 3]) -> f32 {
    let h = (p[0] as u32).wrapping_mul(73856093)
        ^ (p[1] as u32).wrapping_mul(19349663)
        ^ (p[2] as u32).wrapping_mul(83492791);
    (pcg(h) >> 8) as f32 / 16777216.0
}

fn hash3_nested(p: [i32; 3]) -> f32 {
    let h = pcg((p[0] as u32).wrapping_mul(73856093)
        ^ pcg((p[1] as u32).wrapping_mul(19349663) ^ pcg((p[2] as u32).wrapping_mul(83492791))));
    (h >> 8) as f32 / 16777216.0
}

pub fn hash2(p: [i32; 2], seed: u32) -> f32 {
    let mut h = (p[0] as u32).wrapping_mul(0x8da6_b343) ^ (p[1] as u32).wrapping_mul(0xd816_3841);
    h ^= seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16777216.0
}

pub fn value_noise3(p: [f32; 3], hash: Hash) -> f32 {
    match hash {
        Hash::Xor => lattice_noise(p, hash3_xor),
        Hash::Nested => lattice_noise(p, hash3_nested),
    }
}

fn lattice_noise(p: [f32; 3], hash: fn([i32; 3]) -> f32) -> f32 {
    let i = [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ];
    let f = [p[0] - i[0] as f32, p[1] - i[1] as f32, p[2] - i[2] as f32];
    let u = [fade(f[0]), fade(f[1]), fade(f[2])];
    let h = |x, y, z| hash([i[0] + x, i[1] + y, i[2] + z]);
    let a = lerp(
        lerp(h(0, 0, 0), h(1, 0, 0), u[0]),
        lerp(h(0, 1, 0), h(1, 1, 0), u[0]),
        u[1],
    );
    let b = lerp(
        lerp(h(0, 0, 1), h(1, 0, 1), u[0]),
        lerp(h(0, 1, 1), h(1, 1, 1), u[0]),
        u[1],
    );
    lerp(a, b, u[2])
}

pub fn fbm3(p: [f32; 3], octaves: i32, hash: Hash) -> f32 {
    fbm3_filtered(p, octaves, 0.0, hash)
}

pub fn octave_weight(footprint: f32) -> f32 {
    let t = ((footprint - 0.5) * 2.0).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

pub fn fbm3_filtered(p: [f32; 3], octaves: i32, footprint: f32, hash: Hash) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut q = p;
    let mut width = footprint;
    for _ in 0..octaves.max(0) {
        let weight = octave_weight(width);
        if weight == 1.0 {
            sum += amp * value_noise3(q, hash);
        } else if weight == 0.0 {
            sum += amp * 0.5;
        } else {
            sum += amp * (weight * value_noise3(q, hash) + (1.0 - weight) * 0.5);
        }
        q = [q[0] * 2.03 + 1.7, q[1] * 2.03 + 9.2, q[2] * 2.03 + 3.1];
        amp *= 0.5;
        width *= 2.03;
    }
    sum
}

pub fn noise2(p: [f32; 2], seed: u32) -> f32 {
    let i = [p[0].floor() as i32, p[1].floor() as i32];
    let f = [p[0] - i[0] as f32, p[1] - i[1] as f32];
    let u = [fade(f[0]), fade(f[1])];
    let a = lerp(hash2(i, seed), hash2([i[0] + 1, i[1]], seed), u[0]);
    let b = lerp(
        hash2([i[0], i[1] + 1], seed),
        hash2([i[0] + 1, i[1] + 1], seed),
        u[0],
    );
    lerp(a, b, u[1])
}

pub fn hash21(p: [f32; 2]) -> f32 {
    let mut q = [fract(p[0] * 123.34), fract(p[1] * 456.21)];
    let d = q[0] * (q[0] + 45.32) + q[1] * (q[1] + 45.32);
    q[0] += d;
    q[1] += d;
    fract(q[0] * q[1])
}

pub fn value_noise2_signed(p: [f32; 2]) -> f32 {
    let i = [p[0].floor(), p[1].floor()];
    let f = [p[0] - i[0], p[1] - i[1]];
    let u = [fade(f[0]), fade(f[1])];
    let h = |x: f32, y: f32| hash21([i[0] + x, i[1] + y]);
    let a = lerp(h(0.0, 0.0), h(1.0, 0.0), u[0]);
    let b = lerp(h(0.0, 1.0), h(1.0, 1.0), u[0]);
    lerp(a, b, u[1]) * 2.0 - 1.0
}

pub fn flow(p: [f32; 2], time: f32) -> f32 {
    let warp = [
        value_noise2_signed([p[0] * 0.006 + time * 0.010, p[1] * 0.006 + 3.1]),
        value_noise2_signed([p[0] * 0.006 + 7.7, p[1] * 0.006 - time * 0.008]),
    ];
    value_noise2_signed([
        p[0] * 0.009 + warp[0] * 1.6 + time * 0.006,
        p[1] * 0.009 + warp[1] * 1.6,
    ]) * 0.65
        + value_noise2_signed([
            p[0] * 0.021 - warp[0],
            p[1] * 0.021 - warp[1] + time * 0.009,
        ]) * 0.35
}

fn lattice_plane_x(i: [i32; 3]) -> [f32; 4] {
    [
        hash3_xor(i),
        hash3_xor([i[0], i[1] + 1, i[2]]),
        hash3_xor([i[0], i[1], i[2] + 1]),
        hash3_xor([i[0], i[1] + 1, i[2] + 1]),
    ]
}

fn lattice_plane_y(i: [i32; 3]) -> [f32; 4] {
    [
        hash3_xor(i),
        hash3_xor([i[0] + 1, i[1], i[2]]),
        hash3_xor([i[0], i[1], i[2] + 1]),
        hash3_xor([i[0] + 1, i[1], i[2] + 1]),
    ]
}

fn lattice_plane_z(i: [i32; 3]) -> [f32; 4] {
    [
        hash3_xor(i),
        hash3_xor([i[0] + 1, i[1], i[2]]),
        hash3_xor([i[0], i[1] + 1, i[2]]),
        hash3_xor([i[0] + 1, i[1] + 1, i[2]]),
    ]
}

fn bilerp4(p: [f32; 4], u: f32, v: f32) -> f32 {
    lerp(lerp(p[0], p[1], u), lerp(p[2], p[3], u), v)
}

pub fn value_noise3_gradient(p: [f32; 3], scale: f32) -> [f32; 3] {
    let q = p.map(|v| v * scale);
    let i = q.map(|v| v.floor() as i32);
    let f = q.map(|v| v - v.floor());
    let [u, v, w] = f.map(fade);
    let left = q.map(|v| (v - 0.35).floor() != v.floor());
    let right = q.map(|v| (v + 0.35).floor() != v.floor());
    let x0 = lattice_plane_x(i);
    let x1 = lattice_plane_x([i[0] + 1, i[1], i[2]]);
    let xlo = if left[0] {
        lattice_plane_x([i[0] - 1, i[1], i[2]])
    } else {
        x0
    };
    let xhi = if right[0] {
        lattice_plane_x([i[0] + 2, i[1], i[2]])
    } else {
        x1
    };
    let xminus = lerp(
        bilerp4(xlo, v, w),
        bilerp4(if left[0] { x0 } else { x1 }, v, w),
        fade(fract(q[0] - 0.35)),
    );
    let xplus = lerp(
        bilerp4(if right[0] { x1 } else { x0 }, v, w),
        bilerp4(xhi, v, w),
        fade(fract(q[0] + 0.35)),
    );
    let y0 = [x0[0], x1[0], x0[2], x1[2]];
    let y1 = [x0[1], x1[1], x0[3], x1[3]];
    let ylo = if left[1] {
        lattice_plane_y([i[0], i[1] - 1, i[2]])
    } else {
        y0
    };
    let yhi = if right[1] {
        lattice_plane_y([i[0], i[1] + 2, i[2]])
    } else {
        y1
    };
    let yminus = lerp(
        bilerp4(ylo, u, w),
        bilerp4(if left[1] { y0 } else { y1 }, u, w),
        fade(fract(q[1] - 0.35)),
    );
    let yplus = lerp(
        bilerp4(if right[1] { y1 } else { y0 }, u, w),
        bilerp4(yhi, u, w),
        fade(fract(q[1] + 0.35)),
    );
    let z0 = [x0[0], x1[0], x0[1], x1[1]];
    let z1 = [x0[2], x1[2], x0[3], x1[3]];
    let zlo = if left[2] {
        lattice_plane_z([i[0], i[1], i[2] - 1])
    } else {
        z0
    };
    let zhi = if right[2] {
        lattice_plane_z([i[0], i[1], i[2] + 2])
    } else {
        z1
    };
    let zminus = lerp(
        bilerp4(zlo, u, v),
        bilerp4(if left[2] { z0 } else { z1 }, u, v),
        fade(fract(q[2] - 0.35)),
    );
    let zplus = lerp(
        bilerp4(if right[2] { z1 } else { z0 }, u, v),
        bilerp4(zhi, u, v),
        fade(fract(q[2] + 0.35)),
    );
    [
        (xplus - xminus) / 0.7,
        (yplus - yminus) / 0.7,
        (zplus - zminus) / 0.7,
    ]
}

#[cfg(test)]
mod tests {
    use super::{Hash, fbm3, fbm3_filtered, octave_weight, value_noise3, value_noise3_gradient};

    #[test]
    fn wobble_reuses_hashes_without_changing_the_noise() {
        for sample in 0..1000 {
            let p = [
                sample as f32 * 0.013 - 6.0,
                sample as f32 * 0.007 - 2.0,
                sample as f32 * 0.031 - 9.0,
            ];
            let q = p.map(|v| v * 640.0);
            let n =
                |x: f32, y: f32, z: f32| value_noise3([q[0] + x, q[1] + y, q[2] + z], Hash::Xor);
            let expected = [
                (n(0.35, 0.0, 0.0) - n(-0.35, 0.0, 0.0)) / 0.7,
                (n(0.0, 0.35, 0.0) - n(0.0, -0.35, 0.0)) / 0.7,
                (n(0.0, 0.0, 0.35) - n(0.0, 0.0, -0.35)) / 0.7,
            ];
            let actual = value_noise3_gradient(p, 640.0);
            for axis in 0..3 {
                assert!(
                    (actual[axis] - expected[axis]).abs() < 0.0001,
                    "sample {sample} axis {axis}: actual {} expected {} q {q:?}",
                    actual[axis],
                    expected[axis]
                );
            }
        }
    }

    #[test]
    fn filtered_octaves_are_continuous_and_deterministic_across_zoom() {
        let p = [0.212, -0.354, 0.477];
        assert_eq!(
            fbm3_filtered(p, 4, 0.0, Hash::Nested),
            fbm3(p, 4, Hash::Nested)
        );
        let mut previous = 1.0;
        for step in 0..=1000 {
            let footprint = step as f32 * 0.002;
            let weight = octave_weight(footprint);
            assert!(weight <= previous);
            previous = weight;
            let left = fbm3_filtered(p, 4, footprint, Hash::Nested);
            let right = fbm3_filtered(p, 4, footprint + 0.00001, Hash::Nested);
            assert!((left - right).abs() < 0.0001);
            assert_eq!(left, fbm3_filtered(p, 4, footprint, Hash::Nested));
        }
        assert_eq!(octave_weight(0.5), 1.0);
        assert_eq!(octave_weight(1.0), 0.0);
    }
}
