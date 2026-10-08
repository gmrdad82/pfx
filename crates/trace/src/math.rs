pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

pub(crate) fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = length(a);
    if len <= 1e-12 {
        [0.0, 0.0, 1.0]
    } else {
        scale(a, 1.0 / len)
    }
}

#[cfg(test)]
pub(crate) const RAY_ERROR_SCALE: f32 = 4e-7;

#[cfg(test)]
pub(crate) fn offset_ray(p: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    const ORIGIN: f32 = 1.0 / 32.0;
    const FLOAT_SCALE: f32 = 1.0 / 65536.0;
    const INT_SCALE: f32 = 256.0;
    std::array::from_fn(|axis| {
        let position = p[axis];
        let normal = n[axis];
        if position.abs() < ORIGIN {
            position + FLOAT_SCALE * normal
        } else {
            let bits = position.to_bits() as i32;
            let shift = (INT_SCALE * normal) as i32;
            let moved = bits.wrapping_add(if position < 0.0 {
                shift.wrapping_neg()
            } else {
                shift
            });
            f32::from_bits(moved as u32)
        }
    })
}

#[cfg(test)]
pub(crate) fn ray_error(origin: [f32; 3], t: f32) -> f32 {
    RAY_ERROR_SCALE * (origin.iter().map(|value| value.abs()).fold(0.0, f32::max) + t)
}

#[cfg(test)]
pub(crate) fn spawn_point(p: [f32; 3], dir: [f32; 3], origin: [f32; 3], t: f32) -> [f32; 3] {
    let nudged = offset_ray(p, dir);
    let err = ray_error(origin, t);
    std::array::from_fn(|axis| nudged[axis] + dir[axis] * err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_ray_matches_the_integer_cases() {
        let near = offset_ray([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(near, [0.0, 1.0 / 65536.0, 0.0]);
        assert_eq!(
            offset_ray([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])[0],
            1.0 + 1.0 / 32768.0
        );
        assert_eq!(
            offset_ray([-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0])[0],
            -1.0 - 1.0 / 32768.0
        );
    }

    #[test]
    fn a_half_millimetre_gap_stays_in_front_of_the_spawn_point() {
        let near = spawn_point([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0], 2.0);
        assert!(near[1] > 0.0 && near[1] < 0.0005 * 0.5, "{}", near[1]);
        let far = spawn_point(
            [0.0, -0.05, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 2600.0, 0.0],
            2600.0,
        );
        assert!(far[1] + 0.05 > 0.001, "{}", far[1] + 0.05);
    }
}
