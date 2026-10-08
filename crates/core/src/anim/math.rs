use crate::sim::{atan2f, cosf, sinf, sqrtf};

pub type Mat4 = [[f32; 4]; 4];
pub type Quat = [f32; 4];

pub const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub const QUAT_IDENTITY: Quat = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub translation: [f32; 3],
    pub rotation: Quat,
    pub scale: [f32; 3],
}

impl Trs {
    pub const IDENTITY: Self = Self {
        translation: [0.0; 3],
        rotation: QUAT_IDENTITY,
        scale: [1.0; 3],
    };

    pub fn matrix(&self) -> Mat4 {
        let [x, y, z, w] = self.rotation;
        let [sx, sy, sz] = self.scale;
        let [tx, ty, tz] = self.translation;
        [
            [
                (1.0 - 2.0 * (y * y + z * z)) * sx,
                2.0 * (x * y + z * w) * sx,
                2.0 * (x * z - y * w) * sx,
                0.0,
            ],
            [
                2.0 * (x * y - z * w) * sy,
                (1.0 - 2.0 * (x * x + z * z)) * sy,
                2.0 * (y * z + x * w) * sy,
                0.0,
            ],
            [
                2.0 * (x * z + y * w) * sz,
                2.0 * (y * z - x * w) * sz,
                (1.0 - 2.0 * (x * x + y * y)) * sz,
                0.0,
            ],
            [tx, ty, tz, 1.0],
        ]
    }

    pub fn finite(&self) -> bool {
        self.translation
            .iter()
            .chain(&self.rotation)
            .chain(&self.scale)
            .all(|value| value.is_finite())
    }
}

impl Default for Trs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (column, target) in out.iter_mut().enumerate() {
        for (row, value) in target.iter_mut().enumerate() {
            *value = a[0][row] * b[column][0]
                + a[1][row] * b[column][1]
                + a[2][row] * b[column][2]
                + a[3][row] * b[column][3];
        }
    }
    out
}

pub fn transform_point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

pub fn transform_vector(m: &Mat4, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

pub fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let length = sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    if length > 0.0 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        v
    }
}

pub fn invert(m: &Mat4) -> Option<Mat4> {
    let a = |c: usize, r: usize| m[c][r] as f64;
    let s0 = a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0);
    let s1 = a(0, 0) * a(1, 2) - a(0, 2) * a(1, 0);
    let s2 = a(0, 0) * a(1, 3) - a(0, 3) * a(1, 0);
    let s3 = a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1);
    let s4 = a(0, 1) * a(1, 3) - a(0, 3) * a(1, 1);
    let s5 = a(0, 2) * a(1, 3) - a(0, 3) * a(1, 2);
    let c5 = a(2, 2) * a(3, 3) - a(2, 3) * a(3, 2);
    let c4 = a(2, 1) * a(3, 3) - a(2, 3) * a(3, 1);
    let c3 = a(2, 1) * a(3, 2) - a(2, 2) * a(3, 1);
    let c2 = a(2, 0) * a(3, 3) - a(2, 3) * a(3, 0);
    let c1 = a(2, 0) * a(3, 2) - a(2, 2) * a(3, 0);
    let c0 = a(2, 0) * a(3, 1) - a(2, 1) * a(3, 0);
    let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    let out = [
        [
            (a(1, 1) * c5 - a(1, 2) * c4 + a(1, 3) * c3) * inv,
            (-a(0, 1) * c5 + a(0, 2) * c4 - a(0, 3) * c3) * inv,
            (a(3, 1) * s5 - a(3, 2) * s4 + a(3, 3) * s3) * inv,
            (-a(2, 1) * s5 + a(2, 2) * s4 - a(2, 3) * s3) * inv,
        ],
        [
            (-a(1, 0) * c5 + a(1, 2) * c2 - a(1, 3) * c1) * inv,
            (a(0, 0) * c5 - a(0, 2) * c2 + a(0, 3) * c1) * inv,
            (-a(3, 0) * s5 + a(3, 2) * s2 - a(3, 3) * s1) * inv,
            (a(2, 0) * s5 - a(2, 2) * s2 + a(2, 3) * s1) * inv,
        ],
        [
            (a(1, 0) * c4 - a(1, 1) * c2 + a(1, 3) * c0) * inv,
            (-a(0, 0) * c4 + a(0, 1) * c2 - a(0, 3) * c0) * inv,
            (a(3, 0) * s4 - a(3, 1) * s2 + a(3, 3) * s0) * inv,
            (-a(2, 0) * s4 + a(2, 1) * s2 - a(2, 3) * s0) * inv,
        ],
        [
            (-a(1, 0) * c3 + a(1, 1) * c1 - a(1, 2) * c0) * inv,
            (a(0, 0) * c3 - a(0, 1) * c1 + a(0, 2) * c0) * inv,
            (-a(3, 0) * s3 + a(3, 1) * s1 - a(3, 2) * s0) * inv,
            (a(2, 0) * s3 - a(2, 1) * s1 + a(2, 2) * s0) * inv,
        ],
    ];
    let out = out.map(|column| column.map(|value| value as f32));
    out.iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(out)
}

pub fn quat_normalize(q: Quat) -> Quat {
    let length = sqrtf(q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]);
    if length > 0.0 {
        [q[0] / length, q[1] / length, q[2] / length, q[3] / length]
    } else {
        QUAT_IDENTITY
    }
}

pub fn quat_dot(a: Quat, b: Quat) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

pub fn quat_multiply(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn quat_axis_angle(axis: [f32; 3], angle: f32) -> Quat {
    let axis = normalize3(axis);
    let half = angle * 0.5;
    let s = sinf(half);
    let c = cosf(half);
    quat_normalize([axis[0] * s, axis[1] * s, axis[2] * s, c])
}

pub fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if quat_dot(a, b) < 0.0 {
        [-b[0], -b[1], -b[2], -b[3]]
    } else {
        b
    };
    quat_normalize(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t))
}

pub fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let mut d = quat_dot(a, b);
    let b = if d < 0.0 {
        d = -d;
        [-b[0], -b[1], -b[2], -b[3]]
    } else {
        b
    };
    if d > 0.9995 {
        return quat_normalize(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t));
    }
    let theta = atan2f(sqrtf((1.0 - d * d).max(0.0)), d);
    let s = sinf(theta);
    let wa = sinf((1.0 - t) * theta) / s;
    let wb = sinf(t * theta) / s;
    quat_normalize(std::array::from_fn(|k| a[k] * wa + b[k] * wb))
}

pub fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}
