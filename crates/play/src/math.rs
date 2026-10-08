use pfx_core::anim::Trs;
use pfx_load::scene::{Matrix, Value};
use pfx_physics::rigid::Pose;

pub fn number(value: &Value) -> Option<f32> {
    match value {
        Value::Float(value) => Some(*value),
        Value::Int(value) => Some(*value as f32),
        _ => None,
    }
}

pub fn shortest(value: f32) -> f64 {
    format!("{value}")
        .parse::<f64>()
        .unwrap_or(f64::from(value))
}

pub fn compose(at: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> Matrix {
    Trs {
        translation: at,
        rotation,
        scale,
    }
    .matrix()
}

pub fn pose(model: Matrix) -> Pose {
    Pose::new(position(model), rotation(model))
}

pub fn rotate(rotation: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = rotation;
    let u = [x, y, z];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let uv = cross(u, vector);
    let uuv = cross(u, uv);
    std::array::from_fn(|axis| vector[axis] + 2.0 * (w * uv[axis] + uuv[axis]))
}

pub fn position(model: Matrix) -> [f32; 3] {
    [model[3][0], model[3][1], model[3][2]]
}

pub fn rotation(model: Matrix) -> [f32; 4] {
    let axis = |column: usize| {
        let [x, y, z, _] = model[column];
        let length = (x * x + y * y + z * z).sqrt();
        if length > 0.0 {
            [x / length, y / length, z / length]
        } else {
            [0.0; 3]
        }
    };
    let columns = [axis(0), axis(1), axis(2)];
    let r = |row: usize, column: usize| columns[column][row];
    let trace = r(0, 0) + r(1, 1) + r(2, 2);
    let quat = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (r(2, 1) - r(1, 2)) / s,
            (r(0, 2) - r(2, 0)) / s,
            (r(1, 0) - r(0, 1)) / s,
            0.25 * s,
        ]
    } else if r(0, 0) > r(1, 1) && r(0, 0) > r(2, 2) {
        let s = (1.0 + r(0, 0) - r(1, 1) - r(2, 2)).sqrt() * 2.0;
        [
            0.25 * s,
            (r(0, 1) + r(1, 0)) / s,
            (r(0, 2) + r(2, 0)) / s,
            (r(2, 1) - r(1, 2)) / s,
        ]
    } else if r(1, 1) > r(2, 2) {
        let s = (1.0 + r(1, 1) - r(0, 0) - r(2, 2)).sqrt() * 2.0;
        [
            (r(0, 1) + r(1, 0)) / s,
            0.25 * s,
            (r(1, 2) + r(2, 1)) / s,
            (r(0, 2) - r(2, 0)) / s,
        ]
    } else {
        let s = (1.0 + r(2, 2) - r(0, 0) - r(1, 1)).sqrt() * 2.0;
        [
            (r(0, 2) + r(2, 0)) / s,
            (r(1, 2) + r(2, 1)) / s,
            0.25 * s,
            (r(1, 0) - r(0, 1)) / s,
        ]
    };
    let length = quat.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        quat.map(|value| value / length)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_load::scene::model;

    #[test]
    fn a_rotation_survives_the_trip_through_a_quaternion() {
        for rotate in [
            [0.0, 0.0, 0.0],
            [0.0, 25.0, 0.0],
            [30.0, -70.0, 110.0],
            [179.0, 10.0, -45.0],
        ] {
            let rest = model([1.0, 2.0, 3.0], rotate, [0.7, 1.5, 2.0]);
            let again = compose(position(rest), rotation(rest), [0.7, 1.5, 2.0]);
            for column in 0..4 {
                for row in 0..4 {
                    assert!(
                        (rest[column][row] - again[column][row]).abs() < 1e-5,
                        "{rotate:?}: {rest:?} vs {again:?}"
                    );
                }
            }
        }
    }
}
