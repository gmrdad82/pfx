use crate::bvh::Bounds;
use crate::detail::{Transform, TriangleDetail};
use crate::gpu::Camera;

pub const SHUTTER_TABLE_SIZE: usize = 256;
pub const BOUND_SAMPLES: u32 = 128;
pub const MAX_STEPS: usize = 64;

pub type Rows = [[f32; 4]; 3];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Decomposed {
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub z: [f32; 4],
    pub w: [f32; 4],
}

impl Decomposed {
    pub fn lanes(&self) -> [[f32; 4]; 4] {
        [self.x, self.y, self.z, self.w]
    }
}

#[derive(Clone, Debug, Default)]
pub struct Motion {
    pub steps: usize,
    pub cameras: Vec<Camera>,
    pub curve: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceMotion {
    pub transforms: Vec<Transform>,
    pub shapes: Vec<Vec<TriangleDetail>>,
}

impl Motion {
    pub fn valid(&self) -> bool {
        (2..=MAX_STEPS).contains(&self.steps)
            && (self.cameras.is_empty() || (2..=MAX_STEPS).contains(&self.cameras.len()))
            && self.cameras.iter().all(|camera| {
                [camera.origin, camera.forward, camera.right, camera.up]
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            })
            && self.curve.iter().all(|value| value.is_finite())
            && (self.curve.is_empty() || self.curve.iter().any(|value| *value != 0.0))
    }

    pub fn camera_moves(&self) -> bool {
        self.cameras
            .windows(2)
            .any(|pair| camera_rows(&pair[0]) != camera_rows(&pair[1]))
    }
}

impl InstanceMotion {
    pub fn moves(&self, reference: &Transform) -> bool {
        self.transforms
            .iter()
            .any(|transform| transform != reference)
            || self.shapes.windows(2).any(|pair| pair[0] != pair[1])
    }
}

pub fn rows(m: &Transform) -> Rows {
    std::array::from_fn(|row| [m[0][row], m[1][row], m[2][row], m[3][row]])
}

pub fn transform(r: &Rows) -> Transform {
    [
        [r[0][0], r[1][0], r[2][0], 0.0],
        [r[0][1], r[1][1], r[2][1], 0.0],
        [r[0][2], r[1][2], r[2][2], 0.0],
        [r[0][3], r[1][3], r[2][3], 1.0],
    ]
}

pub fn camera_rows(camera: &Camera) -> Rows {
    let c = [camera.right, camera.up, camera.forward, camera.origin];
    std::array::from_fn(|row| [c[0][row], c[1][row], c[2][row], c[3][row]])
}

pub fn camera_of(rows: &Rows) -> Camera {
    let column = |k: usize| [rows[0][k], rows[1][k], rows[2][k]];
    Camera {
        right: column(0),
        up: column(1),
        forward: column(2),
        origin: column(3),
    }
}

fn column(t: &Rows, k: usize) -> [f32; 3] {
    [t[0][k], t[1][k], t[2][k]]
}

fn set_column(t: &mut Rows, k: usize, v: [f32; 3]) {
    for (row, value) in v.into_iter().enumerate() {
        t[row][k] = value;
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn safe_divide(a: [f32; 3], b: f32) -> [f32; 3] {
    if b != 0.0 { a.map(|v| v / b) } else { [0.0; 3] }
}

fn axpy(a: [f32; 3], s: f32, x: [f32; 3]) -> [f32; 3] {
    [a[0] - s * x[0], a[1] - s * x[1], a[2] - s * x[2]]
}

fn negative_scale(t: &Rows) -> bool {
    dot(cross(column(t, 0), column(t, 1)), column(t, 2)) < 0.0
}

fn to_quat(t: &Rows) -> [f32; 4] {
    let trace = f64::from(t[0][0] + t[1][1] + t[2][2]);
    if trace > 0.0 {
        let mut s = (trace + 1.0).sqrt();
        let w = (s / 2.0) as f32;
        s = 0.5 / s;
        [
            (f64::from(t[2][1] - t[1][2]) * s) as f32,
            (f64::from(t[0][2] - t[2][0]) * s) as f32,
            (f64::from(t[1][0] - t[0][1]) * s) as f32,
            w,
        ]
    } else {
        let mut i = 0;
        if t[1][1] > t[i][i] {
            i = 1;
        }
        if t[2][2] > t[i][i] {
            i = 2;
        }
        let j = (i + 1) % 3;
        let k = (j + 1) % 3;
        let mut s = (f64::from(t[i][i] - (t[j][j] + t[k][k])) + 1.0).sqrt();
        let mut q = [0.0_f64; 3];
        q[i] = s * 0.5;
        if s != 0.0 {
            s = 0.5 / s;
        }
        let w = f64::from(t[k][j] - t[j][k]) * s;
        q[j] = f64::from(t[j][i] + t[i][j]) * s;
        q[k] = f64::from(t[k][i] + t[i][k]) * s;
        [q[0] as f32, q[1] as f32, q[2] as f32, w as f32]
    }
}

pub fn decompose(t: &Rows) -> Decomposed {
    let mut m = *t;
    for row in &mut m {
        row[3] = 0.0;
    }
    let mut colx = column(&m, 0);
    let mut coly = column(&m, 1);
    let mut colz = column(&m, 2);
    let mut scale = [0.0_f32; 3];
    let mut shear = [0.0_f32; 3];
    scale[0] = len(colx);
    colx = safe_divide(colx, scale[0]);
    shear[2] = dot(colx, coly);
    coly = axpy(coly, shear[2], colx);
    scale[1] = len(coly);
    coly = safe_divide(coly, scale[1]);
    shear[1] = dot(colx, colz);
    colz = axpy(colz, shear[1], colx);
    shear[0] = dot(coly, colz);
    colz = axpy(colz, shear[0], coly);
    scale[2] = len(colz);
    colz = safe_divide(colz, scale[2]);
    set_column(&mut m, 0, colx);
    set_column(&mut m, 1, coly);
    set_column(&mut m, 2, colz);
    if negative_scale(&m) {
        scale = scale.map(|v| -v);
        for row in &mut m {
            for value in &mut row[..3] {
                *value = -*value;
            }
        }
    }
    Decomposed {
        x: to_quat(&m),
        y: [t[0][3], t[1][3], t[2][3], scale[0]],
        z: [shear[2], shear[1], 0.0, scale[1]],
        w: [shear[0], 0.0, 0.0, scale[2]],
    }
}

pub fn decompose_steps(steps: &[Rows]) -> Vec<Decomposed> {
    let mut out: Vec<Decomposed> = Vec::with_capacity(steps.len());
    for step in steps {
        let mut next = decompose(step);
        if let Some(last) = out.last()
            && dot4(last.x, next.x) < 0.0
        {
            next.x = next.x.map(|v| -v);
        }
        out.push(next);
    }
    for i in 0..out.len() {
        if [out[i].y[3], out[i].z[3], out[i].w[3]] != [0.0; 3] {
            continue;
        }
        if i > 0 {
            out[i].x = out[i - 1].x;
        } else if i + 1 < out.len() {
            out[i].x = out[i + 1].x;
        }
    }
    out
}

fn dot4(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn normalize4(a: [f32; 4]) -> [f32; 4] {
    let l = dot4(a, a).sqrt();
    a.map(|v| v / l)
}

fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|k| (1.0 - t) * a[k] + t * b[k])
}

pub fn quat_interpolate(q1: [f32; 4], q2: [f32; 4], t: f32) -> [f32; 4] {
    let costheta = dot4(q1, q2);
    if costheta > 0.9995 {
        return normalize4(mix4(q1, q2, t));
    }
    let theta = costheta.clamp(-1.0, 1.0).acos();
    let qperp = normalize4(std::array::from_fn(|k| q2[k] - q1[k] * costheta));
    let thetap = theta * t;
    std::array::from_fn(|k| q1[k] * thetap.cos() + qperp[k] * thetap.sin())
}

pub fn compose(d: &Decomposed) -> Rows {
    let sqrt2 = std::f32::consts::SQRT_2;
    let q0 = sqrt2 * d.x[3];
    let q1 = sqrt2 * d.x[0];
    let q2 = sqrt2 * d.x[1];
    let q3 = sqrt2 * d.x[2];
    let qda = q0 * q1;
    let qdb = q0 * q2;
    let qdc = q0 * q3;
    let qaa = q1 * q1;
    let qab = q1 * q2;
    let qac = q1 * q3;
    let qbb = q2 * q2;
    let qbc = q2 * q3;
    let qcc = q3 * q3;
    let rotation = [
        [1.0 - qbb - qcc, -qdc + qab, qdb + qac],
        [qdc + qab, 1.0 - qaa - qcc, -qda + qbc],
        [-qdb + qac, qda + qbc, 1.0 - qaa - qbb],
    ];
    let scale = [
        [d.y[3], d.z[2], d.w[1]],
        [d.z[0], d.z[3], d.w[2]],
        [d.z[1], d.w[0], d.w[3]],
    ];
    std::array::from_fn(|row| {
        [
            dot(rotation[row], scale[0]),
            dot(rotation[row], scale[1]),
            dot(rotation[row], scale[2]),
            d.y[row],
        ]
    })
}

pub fn interpolate(motion: &[Decomposed], time: f32) -> Rows {
    let maxstep = motion.len() as i32 - 1;
    let step = ((time * maxstep as f32) as i32).min(maxstep - 1);
    let t = time * maxstep as f32 - step as f32;
    let a = &motion[step as usize];
    let b = &motion[step as usize + 1];
    compose(&Decomposed {
        x: quat_interpolate(a.x, b.x, t),
        y: mix4(a.y, b.y, t),
        z: mix4(a.z, b.z, t),
        w: mix4(a.w, b.w, t),
    })
}

pub fn inverse(t: &Rows) -> Rows {
    let mut x = [t[0][0], t[1][0], t[2][0]];
    let mut y = [t[0][1], t[1][1], t[2][1]];
    let mut z = [t[0][2], t[1][2], t[2][2]];
    let w = [t[0][3], t[1][3], t[2][3]];
    let mut det = dot(x, cross(y, z));
    if det == 0.0 {
        x[0] += 1e-8;
        y[1] += 1e-8;
        z[2] += 1e-8;
        det = dot(x, cross(y, z));
        if det == 0.0 {
            det = f32::MAX;
        }
    }
    let inverse_x = cross(y, z).map(|v| v / det);
    let inverse_y = cross(z, x).map(|v| v / det);
    let inverse_z = cross(x, y).map(|v| v / det);
    [
        [inverse_x[0], inverse_x[1], inverse_x[2], -dot(inverse_x, w)],
        [inverse_y[0], inverse_y[1], inverse_y[2], -dot(inverse_y, w)],
        [inverse_z[0], inverse_z[1], inverse_z[2], -dot(inverse_z, w)],
    ]
}

pub fn point(t: &Rows, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| t[row][0] * p[0] + t[row][1] * p[1] + t[row][2] * p[2] + t[row][3])
}

pub fn transformed(bounds: Bounds, t: &Rows) -> Bounds {
    let mut out = Bounds::empty();
    for corner in 0..8 {
        let p = std::array::from_fn(|axis| {
            if corner >> axis & 1 == 0 {
                bounds.lo[axis]
            } else {
                bounds.hi[axis]
            }
        });
        out.point(point(t, p));
    }
    out
}

pub fn swept(bounds: Bounds, motion: &[Decomposed]) -> Bounds {
    let mut out = Bounds::empty();
    for k in 0..=BOUND_SAMPLES {
        let t = k as f32 / BOUND_SAMPLES as f32;
        out.grow(transformed(bounds, &interpolate(motion, t)));
    }
    out
}

fn shutter_curve_eval(x: f32, curve: &[f32]) -> f32 {
    if curve.is_empty() {
        return 1.0;
    }
    let x = (x.clamp(0.0, 1.0) * curve.len() as f32 - 1.0).max(0.0);
    let index = x as usize;
    let frac = x - index as f32;
    if index + 1 < curve.len() {
        curve[index] * (1.0 - frac) + curve[index + 1] * frac
    } else {
        curve[curve.len() - 1]
    }
}

pub fn shutter_table(curve: &[f32]) -> Vec<f32> {
    let resolution = SHUTTER_TABLE_SIZE;
    let evaluated = resolution - 1;
    let mut cdf = vec![0.0_f32; evaluated + 1];
    for i in 0..evaluated {
        let x = i as f32 / (evaluated - 1) as f32;
        cdf[i + 1] = cdf[i] + shutter_curve_eval(x, curve).abs();
    }
    let fac = if cdf[evaluated] == 0.0 {
        0.0
    } else {
        1.0 / cdf[evaluated]
    };
    for value in &mut cdf {
        *value *= fac;
    }
    cdf[evaluated] = 1.0;
    let inv_resolution = 1.0 / resolution as f32;
    (0..resolution)
        .map(|i| {
            let x = (i as f32 + 0.5) * inv_resolution;
            let above = cdf.partition_point(|&value| value <= x);
            let mut index = above as i64 - 1;
            let t = if (index as usize) + 1 < cdf.len() {
                let i = index as usize;
                (x - cdf[i]) / (cdf[i + 1] - cdf[i])
            } else {
                index = resolution as i64;
                0.0
            };
            (index as f32 + t) * inv_resolution
        })
        .collect()
}

pub fn lookup(table: &[f32], x: f32) -> f32 {
    let size = table.len();
    let x = x.clamp(0.0, 1.0) * (size - 1) as f32;
    let index = (x as usize).min(size - 1);
    let next = (index + 1).min(size - 1);
    let t = x - index as f32;
    if t == 0.0 {
        return table[index];
    }
    (1.0 - t) * table[index] + t * table[next]
}

pub fn step_times(steps: usize) -> Vec<f32> {
    (0..steps)
        .map(|k| k as f32 / (steps - 1).max(1) as f32)
        .collect()
}

pub use pfx_post::ShutterPosition as Position;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shutter {
    pub open: f64,
    pub close: f64,
}

impl Shutter {
    pub fn around(time: f64, frame_seconds: f64, shutter: f64, position: Position) -> Self {
        let span = frame_seconds * shutter;
        let open = match position {
            Position::Start => time,
            Position::Centre => time - span * 0.5,
            Position::End => time - span,
        };
        Self {
            open,
            close: open + span,
        }
    }

    pub fn times(self, steps: usize) -> Vec<f64> {
        step_times(steps)
            .into_iter()
            .map(|t| self.open + (self.close - self.open) * f64::from(t))
            .collect()
    }

    pub fn times_held(self, steps: usize, start: f64, end: f64) -> Vec<f64> {
        self.times(steps)
            .into_iter()
            .map(|time| time.clamp(start, end))
            .collect()
    }
}

pub fn steps_of(setting: u32) -> usize {
    if setting == 0 {
        0
    } else {
        (1_usize << setting.min(5)) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turned(angle: f32, at: [f32; 3], scale: [f32; 3]) -> Rows {
        let (s, c) = angle.sin_cos();
        [
            [c * scale[0], -s * scale[1], 0.0, at[0]],
            [s * scale[0], c * scale[1], 0.0, at[1]],
            [0.0, 0.0, scale[2], at[2]],
        ]
    }

    fn close(a: &Rows, b: &Rows, eps: f32) -> bool {
        a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(x, y)| (x - y).abs() <= eps)
    }

    #[test]
    fn a_decomposed_transform_composes_back() {
        for (angle, scale) in [
            (0.3, [1.0, 1.0, 1.0]),
            (2.5, [2.0, 0.5, 1.5]),
            (-1.7, [-1.0, 1.0, 1.0]),
            (3.1, [1.0, 1.0, -0.25]),
        ] {
            let t = turned(angle, [1.0, -2.0, 3.0], scale);
            assert!(
                close(&compose(&decompose(&t)), &t, 1e-5),
                "{angle} {scale:?}"
            );
        }
    }

    #[test]
    fn a_wheel_turns_through_its_steps_at_constant_speed() {
        let steps: Vec<Rows> = (0..3)
            .map(|k| turned(k as f32 * 1.2, [k as f32 * 2.0, 0.0, 0.0], [1.0; 3]))
            .collect();
        let motion = decompose_steps(&steps);
        for k in 0..=20 {
            let time = k as f32 / 20.0;
            let want = turned(time * 2.4, [time * 4.0, 0.0, 0.0], [1.0; 3]);
            assert!(close(&interpolate(&motion, time), &want, 1e-5), "{time}");
        }
    }

    #[test]
    fn the_shortest_turn_is_taken_between_steps() {
        let steps = [
            turned(0.0, [0.0; 3], [1.0; 3]),
            turned(3.0, [0.0; 3], [1.0; 3]),
        ];
        let middle = interpolate(&decompose_steps(&steps), 0.5);
        assert!(close(&middle, &turned(1.5, [0.0; 3], [1.0; 3]), 1e-5));
    }

    #[test]
    fn the_inverse_undoes_the_transform() {
        let t = turned(0.7, [3.0, 1.0, -2.0], [2.0, 0.5, 1.0]);
        let p = [0.25, -1.5, 4.0];
        let back = point(&inverse(&t), point(&t, p));
        assert!(back.iter().zip(p).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn the_swept_bounds_hold_every_pose_over_the_shutter() {
        let steps: Vec<Rows> = (0..3)
            .map(|k| turned(k as f32 * 1.4, [k as f32 * 3.0, 1.0, 0.0], [1.0; 3]))
            .collect();
        let motion = decompose_steps(&steps);
        let unit = Bounds {
            lo: [-1.0; 3],
            hi: [1.0; 3],
        };
        let all = swept(unit, &motion);
        for k in 0..=97 {
            let t = k as f32 / 97.0;
            let posed = transformed(unit, &interpolate(&motion, t));
            for axis in 0..3 {
                assert!(posed.lo[axis] >= all.lo[axis] - 1e-2);
                assert!(posed.hi[axis] <= all.hi[axis] + 1e-2);
            }
        }
    }

    #[test]
    fn a_flat_shutter_curve_maps_time_almost_to_itself_and_a_ramp_leans_late() {
        let flat = shutter_table(&[1.0; 16]);
        assert_eq!(flat.len(), SHUTTER_TABLE_SIZE);
        for k in 0..=10 {
            let x = k as f32 / 10.0;
            assert!((lookup(&flat, x) - x).abs() < 0.01, "{x}");
        }
        let ramp: Vec<f32> = (0..16).map(|k| k as f32).collect();
        let late = shutter_table(&ramp);
        assert!(lookup(&late, 0.5) > 0.6);
        assert!(late.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn the_shutter_opens_around_its_frame() {
        let shutter = Shutter::around(1.0, 0.04, 0.5, Position::Centre);
        let times = shutter.times(3);
        assert!((times[0] - 0.99).abs() < 1e-12);
        assert!((times[1] - 1.0).abs() < 1e-12);
        assert!((times[2] - 1.01).abs() < 1e-12);
        assert_eq!((steps_of(0), steps_of(1), steps_of(3)), (0, 3, 9));
    }

    #[test]
    fn the_shutter_opens_at_or_closes_at_its_frame_and_holds_the_timeline_ends() {
        let opens = Shutter::around(1.0, 0.04, 0.5, Position::Start).times(3);
        let closes = Shutter::around(1.0, 0.04, 0.5, Position::End).times(3);
        for (got, want) in opens.iter().zip([1.0, 1.01, 1.02]) {
            assert!((got - want).abs() < 1e-12, "{opens:?}");
        }
        for (got, want) in closes.iter().zip([0.98, 0.99, 1.0]) {
            assert!((got - want).abs() < 1e-12, "{closes:?}");
        }
        let fps = 60.0;
        let end = 3.0 / fps;
        let first = Shutter::around(0.0, 1.0 / fps, 0.5, Position::Centre).times_held(5, 0.0, end);
        assert_eq!(first[..3], [0.0, 0.0, 0.0]);
        assert!(first[3] > 0.0 && first[4] <= end);
        let last = Shutter::around(end, 1.0 / fps, 0.5, Position::Centre).times_held(5, 0.0, end);
        assert_eq!(last[2..], [end, end, end]);
        assert!(last[0] < end && last[0] >= 0.0);
        let opened = Shutter::around(end, 1.0 / fps, 0.5, Position::Start).times_held(3, 0.0, end);
        assert_eq!(opened, [end, end, end]);
        let closed = Shutter::around(0.0, 1.0 / fps, 0.5, Position::End).times_held(3, 0.0, end);
        assert_eq!(closed, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_camera_keeps_its_axes_and_origin_through_rows() {
        let camera = Camera {
            origin: [1.0, 2.0, 3.0],
            forward: [0.0, 0.0, -1.0],
            right: [0.6, 0.0, 0.0],
            up: [0.0, 0.4, 0.0],
        };
        let back = camera_of(&compose(&decompose(&camera_rows(&camera))));
        for (a, b) in [
            (back.origin, camera.origin),
            (back.forward, camera.forward),
            (back.right, camera.right),
            (back.up, camera.up),
        ] {
            assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5));
        }
    }
}
