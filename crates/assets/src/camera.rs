use pfx_trace::{Camera, Projection};
use serde_json::Value;

use crate::build::{AXES, Box3, apply};

pub const SENSOR: f64 = 36.0;
const FIT_STEPS: usize = 24;
const FIT_CLOSE: f64 = 0.004;

#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub turn: f64,
    pub tilt: f64,
    pub roll: f64,
    pub zoom: f64,
    pub shift: [f64; 2],
    pub target: Option<[f64; 3]>,
    pub lens: f64,
    pub ortho: bool,
    pub margin: f64,
}

impl Pose {
    pub fn read(value: &Value) -> Result<Pose, String> {
        let get = |k: &str, d: f64| value.get(k).and_then(Value::as_f64).unwrap_or(d);
        let list = |k: &str| -> Vec<f64> {
            value
                .get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default()
        };
        let shift = match list("move")[..] {
            [x, y] => [x, y],
            _ => [0.0, 0.0],
        };
        let target = match list("target")[..] {
            [x, y, z] => Some([x, y, z]),
            _ => None,
        };
        let pose = Pose {
            turn: get("turn", 0.0),
            tilt: get("tilt", 12.0),
            roll: get("roll", 0.0),
            zoom: get("zoom", 1.0),
            shift,
            target,
            lens: get("lens", 85.0),
            ortho: value.get("ortho").and_then(Value::as_bool).unwrap_or(false),
            margin: get("margin", 1.12),
        };
        if pose.zoom <= 0.0 || pose.lens <= 0.0 || pose.margin <= 0.0 {
            return Err("a pose's zoom, lens and margin are above 0".into());
        }
        Ok(pose)
    }
}

#[derive(Clone, Copy, Debug)]
struct Fit {
    distance: f64,
    scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub origin: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub back: [f64; 3],
    pub half: [f64; 2],
    pub ortho: bool,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn axpy(a: [f64; 3], k: f64, b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + k * b[i])
}

fn halves(lens: f64, w: f64, h: f64) -> [f64; 2] {
    let wide = SENSOR * 0.5 / lens;
    if w >= h {
        [wide, wide * h / w]
    } else {
        [wide * w / h, wide]
    }
}

fn place(p: &Pose, centre: [f64; 3], fit: Fit, size: (u32, u32)) -> View {
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let target = p.target.unwrap_or(centre);
    let (yaw, pitch, roll) = (
        p.turn.to_radians(),
        p.tilt.to_radians(),
        p.roll.to_radians(),
    );
    let back = [
        yaw.sin() * pitch.sin(),
        -yaw.cos() * pitch.sin(),
        pitch.cos(),
    ];
    let right = [yaw.cos(), yaw.sin(), 0.0];
    let up = cross(back, right);
    let x: [f64; 3] = std::array::from_fn(|i| right[i] * roll.cos() + up[i] * roll.sin());
    let y: [f64; 3] = std::array::from_fn(|i| up[i] * roll.cos() - right[i] * roll.sin());
    let tall = (h / w).min(1.0);
    let (dist, view, half) = if p.ortho {
        let scale = fit.scale / p.zoom;
        let half = if w >= h {
            [scale * 0.5, scale * 0.5 * h / w]
        } else {
            [scale * 0.5 * w / h, scale * 0.5]
        };
        (fit.distance, scale * tall, half)
    } else {
        let dist = fit.distance / p.zoom;
        (dist, dist * SENSOR / p.lens * tall, halves(p.lens, w, h))
    };
    let spot = axpy(axpy(target, p.shift[0] * view, x), p.shift[1] * view, y);
    View {
        origin: axpy(spot, dist, back),
        x,
        y,
        back,
        half,
        ortho: p.ortho,
    }
}

impl View {
    pub fn project(&self, p: [f64; 3]) -> [f64; 2] {
        let q: [f64; 3] = std::array::from_fn(|i| p[i] - self.origin[i]);
        let (a, b) = (dot(q, self.x), dot(q, self.y));
        if self.ortho {
            [0.5 + 0.5 * a / self.half[0], 0.5 + 0.5 * b / self.half[1]]
        } else {
            let d = (-dot(q, self.back)).max(1e-9);
            [
                0.5 + 0.5 * a / (d * self.half[0]),
                0.5 + 0.5 * b / (d * self.half[1]),
            ]
        }
    }

    pub fn engine(&self) -> (Camera, Projection) {
        let axes = |v: [f64; 3]| apply(&AXES, v).map(|c| c as f32);
        let forward = axes(self.back.map(|v| -v));
        if self.ortho {
            (
                Camera {
                    origin: axes(self.origin),
                    forward,
                    right: axes(self.x),
                    up: axes(self.y),
                },
                Projection::Orthographic {
                    width: (2.0 * self.half[0]) as f32,
                    height: (2.0 * self.half[1]) as f32,
                },
            )
        } else {
            (
                Camera {
                    origin: axes(self.origin),
                    forward,
                    right: axes(self.x.map(|v| v * self.half[0])),
                    up: axes(self.y.map(|v| v * self.half[1])),
                },
                Projection::Perspective,
            )
        }
    }

    pub fn ray(&self, ndc: [f64; 2]) -> [f64; 3] {
        if self.ortho {
            return self.back.map(|v| -v);
        }
        let d: [f64; 3] = std::array::from_fn(|i| {
            -self.back[i] + self.x[i] * self.half[0] * ndc[0] - self.y[i] * self.half[1] * ndc[1]
        });
        let l = dot(d, d).sqrt();
        d.map(|v| v / l)
    }
}

pub fn aim(poses: &[Pose], corners: &[[f64; 3]], size: (u32, u32)) -> Result<Vec<View>, String> {
    let first = poses.first().ok_or("a render needs a pose")?;
    let b = Box3::of(corners);
    let centre = [(b.lo[0] + b.hi[0]) * 0.5, (b.lo[1] + b.hi[1]) * 0.5, 0.1];
    let span = (b.hi[0] - b.lo[0]).max(b.hi[1] - b.lo[1]).max(1e-3);
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let fov = 2.0 * (SENSOR * 0.5 / first.lens).atan();
    let mut fit = Fit {
        distance: (span * first.margin / 2.0) / (fov / 2.0).tan() / (w / h).min(1.0),
        scale: span * 1.2,
    };
    let rest = Pose {
        zoom: 1.0,
        shift: [0.0, 0.0],
        ..first.clone()
    };
    let want = 0.5 - (first.margin - 1.0) / 2.0;
    for _ in 0..FIT_STEPS {
        let view = place(&rest, centre, fit, size);
        let pts: Vec<[f64; 2]> = corners.iter().map(|c| view.project(*c)).collect();
        let low = pts
            .iter()
            .map(|p| p[0].min(p[1]))
            .fold(f64::INFINITY, f64::min);
        let high = pts
            .iter()
            .map(|p| p[0].max(p[1]))
            .fold(f64::NEG_INFINITY, f64::max);
        let spread = (0.5 - low).max(high - 0.5);
        if (spread - want).abs() < FIT_CLOSE || want <= 0.0 {
            break;
        }
        if first.ortho {
            fit.scale *= spread / want;
        } else {
            fit.distance *= spread / want;
        }
    }
    Ok(poses.iter().map(|p| place(p, centre, fit, size)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cube() -> Vec<[f64; 3]> {
        Box3 {
            lo: [-1.0, -1.0, 0.0],
            hi: [1.0, 1.0, 0.2],
        }
        .corners()
    }

    #[test]
    fn the_fit_leaves_the_margin_around_the_subject() {
        for (ortho, size) in [
            (false, (512, 512)),
            (true, (512, 512)),
            (false, (1200, 630)),
        ] {
            let pose =
                Pose::read(&json!({"tilt": 24.0, "turn": 22.0, "ortho": ortho, "margin": 1.2}))
                    .unwrap();
            let view = &aim(&[pose], &cube(), size).unwrap()[0];
            let pts: Vec<[f64; 2]> = cube().iter().map(|c| view.project(*c)).collect();
            let low = pts.iter().map(|p| p[0].min(p[1])).fold(1.0, f64::min);
            let high = pts.iter().map(|p| p[0].max(p[1])).fold(0.0, f64::max);
            let spread = (0.5 - low).max(high - 0.5);
            assert!((spread - 0.4).abs() < 0.005, "{ortho} {size:?}: {spread}");
        }
    }

    #[test]
    fn the_top_view_looks_straight_down_and_maps_to_engine_axes() {
        let pose = Pose::read(&json!({"tilt": 0.0, "turn": 0.0})).unwrap();
        let view = &aim(&[pose], &cube(), (512, 512)).unwrap()[0];
        let (camera, projection) = view.engine();
        assert_eq!(projection, Projection::Perspective);
        assert!(
            (camera.forward[1] + 1.0).abs() < 1e-6,
            "{:?}",
            camera.forward
        );
        assert!(camera.origin[1] > 1.0);
        let half = (SENSOR * 0.5 / 85.0) as f32;
        assert!((camera.right[0] - half).abs() < 1e-6);
        assert!(
            (camera.up[2] + half).abs() < 1e-6,
            "image up is the far side, -z"
        );
        let centre = view.project([0.0, 0.0, 0.1]);
        assert!((centre[0] - 0.5).abs() < 1e-9 && (centre[1] - 0.5).abs() < 1e-9);
        let ray = view.ray([0.0, 0.0]);
        assert!((ray[2] + 1.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_and_move_shift_the_framed_view() {
        let base = Pose::read(&json!({"tilt": 0.0})).unwrap();
        let zoomed = Pose {
            zoom: 2.0,
            shift: [0.25, 0.0],
            ..base.clone()
        };
        let views = aim(&[base, zoomed], &cube(), (512, 512)).unwrap();
        let far = views[0].origin[2];
        let near = views[1].origin[2];
        assert!((far - 0.1 - 2.0 * (near - 0.1)).abs() < 1e-9);
        let c = views[1].project([0.0, 0.0, 0.1]);
        assert!((c[0] - 0.25).abs() < 1e-6, "{c:?}");
        let ortho = Pose::read(&json!({"tilt": 0.0, "ortho": true})).unwrap();
        let (_, projection) = aim(&[ortho], &cube(), (800, 400)).unwrap()[0].engine();
        let Projection::Orthographic { width, height } = projection else {
            panic!("{projection:?}");
        };
        assert!((width / height - 2.0).abs() < 1e-5);
    }
}
