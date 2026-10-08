use egui::{Rect, Vec2};
use glam::{DQuat, DVec3};
use pfx_load::scene::{Camera, Projection};

pub const DEGREES_PER_POINT: f64 = 0.25;
pub const DOLLY_PER_POINT: f64 = 0.002;
pub const NEAREST: f64 = 0.001;
pub const FILL: f64 = 0.8;
pub const STEP_POINTS: f64 = 60.0;
const WORLD_UP: DVec3 = DVec3::Y;

pub type Bounds = [[f64; 3]; 2];

pub const EMPTY: Bounds = [[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Basis {
    pub right: DVec3,
    pub up: DVec3,
    pub forward: DVec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eye {
    pub position: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub fov_y_deg: f64,
    pub shift: [f64; 2],
    pub ortho_height: Option<f64>,
}

pub fn wide(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

pub fn narrow(value: f64) -> f32 {
    value as f32
}

pub fn passepartout(image: Rect, aspect: f32) -> Option<Rect> {
    if !(aspect.is_finite() && aspect > 0.0) || image.width() < 1.0 || image.height() < 1.0 {
        return None;
    }
    let size = if aspect >= image.width() / image.height() {
        Vec2::new(image.width(), image.width() / aspect)
    } else {
        Vec2::new(image.height() * aspect, image.height())
    };
    Some(Rect::from_center_size(image.center(), size))
}

pub fn reach(image: Rect, frame: Rect) -> [f64; 2] {
    [
        f64::from(image.width()) / f64::from(frame.width().max(1e-3)),
        f64::from(image.height()) / f64::from(frame.height().max(1e-3)),
    ]
}

pub fn through(camera: &Camera, reach: [f64; 2]) -> Camera {
    let eye = Eye::of(camera).widened(reach);
    Camera {
        projection: eye.projection(),
        shift: eye.shift.map(narrow),
        ..*camera
    }
}

impl Eye {
    pub fn of(camera: &Camera) -> Eye {
        let (fov_y_deg, ortho_height) = match camera.projection {
            Projection::Perspective { fov } => (wide(fov), None),
            Projection::Orthographic { height } => (40.0, Some(wide(height))),
        };
        Eye {
            position: camera.at.map(wide),
            target: camera.look_at.map(wide),
            up: camera.up.map(wide),
            fov_y_deg,
            shift: camera.shift.map(wide),
            ortho_height,
        }
    }

    pub fn basis(&self) -> Basis {
        let position = DVec3::from_array(self.position);
        let target = DVec3::from_array(self.target);
        let forward = (target - position).try_normalize().unwrap_or(DVec3::NEG_Z);
        let up = DVec3::from_array(self.up);
        let right = forward
            .cross(up)
            .try_normalize()
            .unwrap_or_else(|| forward.any_orthonormal_vector());
        Basis {
            right,
            up: right.cross(forward),
            forward,
        }
    }

    pub fn distance(&self) -> f64 {
        (DVec3::from_array(self.position) - DVec3::from_array(self.target)).length()
    }

    pub fn depth_range(&self, bounds: Bounds) -> (f64, f64) {
        let forward = self.basis().forward;
        let position = DVec3::from_array(self.position);
        let mut nearest = f64::INFINITY;
        let mut farthest = f64::NEG_INFINITY;
        for x in [bounds[0][0], bounds[1][0]] {
            for y in [bounds[0][1], bounds[1][1]] {
                for z in [bounds[0][2], bounds[1][2]] {
                    let along = (DVec3::new(x, y, z) - position).dot(forward);
                    nearest = nearest.min(along);
                    farthest = farthest.max(along);
                }
            }
        }
        if !nearest.is_finite() || !farthest.is_finite() {
            return (0.01, 100.0);
        }
        let span = (farthest - nearest).max(1e-3);
        let far = farthest + span * 0.05 + 1e-3;
        let near = (nearest - span * 0.05).max(far * 1e-4);
        (near, far.max(near * 2.0))
    }

    pub fn projection(&self) -> Projection {
        match self.ortho_height {
            Some(height) => Projection::Orthographic {
                height: narrow(height),
            },
            None => Projection::Perspective {
                fov: narrow(self.fov_y_deg),
            },
        }
    }

    pub fn widened(&self, reach: [f64; 2]) -> Eye {
        let [across, tall] = reach;
        let half = (self.fov_y_deg.to_radians() * 0.5).tan() * tall;
        Eye {
            fov_y_deg: match self.ortho_height {
                Some(_) => self.fov_y_deg,
                None => (half.atan() * 2.0).to_degrees(),
            },
            ortho_height: self.ortho_height.map(|height| height * tall),
            shift: [self.shift[0] / across, self.shift[1] / tall],
            ..*self
        }
    }

    pub fn camera(&self, base: &Camera, bounds: Bounds) -> Camera {
        let (near, far) = self.depth_range(bounds);
        let projection = self.projection();
        let basis = self.basis();
        Camera {
            projection,
            at: self.position.map(narrow),
            look_at: self.target.map(narrow),
            up: basis.up.to_array().map(narrow),
            near: narrow(near),
            far: narrow(far),
            shift: self.shift.map(narrow),
            depth_of_field: None,
            ..*base
        }
    }

    pub fn turn(&self, points: [f64; 2]) -> Eye {
        let position = DVec3::from_array(self.position);
        let target = DVec3::from_array(self.target);
        let yaw = DQuat::from_axis_angle(WORLD_UP, -(points[0] * DEGREES_PER_POINT).to_radians());
        let offset = yaw * (position - target);
        let up = yaw * DVec3::from_array(self.up);
        let turned = Eye {
            position: (target + offset).to_array(),
            up: up.to_array(),
            ..*self
        };
        let frame = turned.basis();
        let pitch =
            DQuat::from_axis_angle(frame.right, -(points[1] * DEGREES_PER_POINT).to_radians());
        Eye {
            position: (target + pitch * offset).to_array(),
            up: (pitch * frame.up).to_array(),
            ..*self
        }
    }

    pub fn span(&self, height_points: f64) -> f64 {
        if height_points <= 0.0 {
            return 0.0;
        }
        let tall = match self.ortho_height {
            Some(height) => height,
            None => 2.0 * self.distance() * (self.fov_y_deg.to_radians() / 2.0).tan(),
        };
        tall / height_points
    }

    pub fn pan(&self, points: [f64; 2], height_points: f64) -> Eye {
        let frame = self.basis();
        let shift = (-points[0] * frame.right + points[1] * frame.up) * self.span(height_points);
        Eye {
            position: (DVec3::from_array(self.position) + shift).to_array(),
            target: (DVec3::from_array(self.target) + shift).to_array(),
            ..*self
        }
    }

    pub fn dolly(&self, points: f64) -> Eye {
        let target = DVec3::from_array(self.target);
        let offset = DVec3::from_array(self.position) - target;
        let factor = (-points * DOLLY_PER_POINT).exp();
        if let Some(height) = self.ortho_height {
            return Eye {
                ortho_height: Some((height * factor).max(NEAREST)),
                ..*self
            };
        }
        let Some(direction) = offset.try_normalize() else {
            return *self;
        };
        let distance = offset.length();
        let nearer = (distance * factor).max(NEAREST.min(distance));
        Eye {
            position: (target + direction * nearer).to_array(),
            ..*self
        }
    }

    pub fn framed(&self, bounds: Bounds, aspect: f64) -> Eye {
        let low = DVec3::from_array(bounds[0]);
        let high = DVec3::from_array(bounds[1]);
        let centre = (low + high) * 0.5;
        let radius = ((high - low).length() * 0.5).max(NEAREST);
        let back = -self.basis().forward;
        if self.ortho_height.is_some() {
            let height = 2.0 * radius / FILL / aspect.clamp(1e-6, 1.0);
            return Eye {
                position: (centre + back * self.distance().max(radius * 2.0)).to_array(),
                target: centre.to_array(),
                ortho_height: Some(height),
                shift: [0.0, 0.0],
                ..*self
            };
        }
        let half_y = (self.fov_y_deg.to_radians() * 0.5).max(1e-6);
        let half_x = (half_y.tan() * aspect.max(1e-6)).atan();
        let half = half_y.min(half_x);
        let distance = radius / half.sin() / FILL;
        Eye {
            position: (centre + back * distance).to_array(),
            target: centre.to_array(),
            shift: [0.0, 0.0],
            ..*self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eye() -> Eye {
        Eye {
            position: [0.0, 0.0, 2.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fov_y_deg: 40.0,
            shift: [0.0, 0.0],
            ortho_height: None,
        }
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    fn offset(eye: &Eye) -> DVec3 {
        DVec3::from_array(eye.position) - DVec3::from_array(eye.target)
    }

    #[test]
    fn a_sideways_drag_turns_around_the_target_by_its_angle() {
        let start = eye();
        let turned = start.turn([120.0, 0.0]);
        let angle = offset(&start).angle_between(offset(&turned)).to_degrees();
        assert!((angle - 120.0 * DEGREES_PER_POINT).abs() < 1e-9, "{angle}");
        assert_eq!(turned.target, start.target);
        assert!((offset(&turned).length() - 2.0).abs() < 1e-12);
        assert!(turned.position[0] < 0.0, "{:?}", turned.position);
        assert!(close(turned.up, start.up));
    }

    #[test]
    fn a_drag_down_lifts_the_eye_by_its_angle() {
        let start = eye();
        let turned = start.turn([0.0, 80.0]);
        let angle = offset(&start).angle_between(offset(&turned)).to_degrees();
        assert!((angle - 80.0 * DEGREES_PER_POINT).abs() < 1e-9, "{angle}");
        assert!(turned.position[1] > 0.0, "{:?}", turned.position);
        let frame = turned.basis();
        assert!(frame.up.dot(frame.forward).abs() < 1e-12);
    }

    #[test]
    fn turning_back_returns_home() {
        let start = Eye {
            position: [0.3, 1.1, 2.5],
            target: [0.1, 0.2, -0.4],
            ..eye()
        };
        let back = start.turn([37.0, 0.0]).turn([-37.0, 0.0]);
        assert!(close(back.position, start.position));
    }

    #[test]
    fn a_straight_down_camera_still_turns() {
        let plan = Eye {
            position: [0.0, 10.0, 0.0],
            up: [0.0, 0.0, -1.0],
            ortho_height: Some(2.2),
            ..eye()
        };
        let turned = plan.turn([0.0, 40.0]);
        assert!(turned.position.iter().all(|value| value.is_finite()));
        let angle = offset(&plan).angle_between(offset(&turned)).to_degrees();
        assert!((angle - 10.0).abs() < 1e-9, "{angle}");
    }

    #[test]
    fn dolly_never_crosses_the_target() {
        let mut moved = eye();
        for _ in 0..2000 {
            moved = moved.dolly(400.0);
            assert!(offset(&moved).dot(offset(&eye())) > 0.0);
        }
        assert!((offset(&moved).length() - NEAREST).abs() < 1e-12);
        assert!(offset(&eye().dolly(-100.0)).length() > 2.0);
        let ortho = Eye {
            ortho_height: Some(2.0),
            ..eye()
        };
        let zoomed = ortho.dolly(100.0);
        assert_eq!(zoomed.position, ortho.position);
        assert!(zoomed.ortho_height.unwrap() < 2.0);
    }

    #[test]
    fn a_pan_moves_eye_and_target_together_by_the_view_span() {
        let start = eye();
        let moved = start.pan([100.0, 0.0], 500.0);
        let tall = 2.0 * 2.0 * 20.0_f64.to_radians().tan();
        let expected = -100.0 * tall / 500.0;
        assert!((moved.position[0] - expected).abs() < 1e-12);
        assert!((moved.target[0] - expected).abs() < 1e-12);
        assert!(close(offset(&moved).to_array(), offset(&start).to_array()));
        assert!(start.pan([0.0, 50.0], 500.0).target[1] > 0.0);
    }

    #[test]
    fn framing_puts_the_box_in_the_middle_and_fits_it() {
        let start = Eye {
            position: [3.0, 2.0, 5.0],
            target: [0.0, 0.0, 0.0],
            ..eye()
        };
        let bounds = [[1.0, 0.0, -1.0], [2.0, 1.0, 0.0]];
        let framed = start.framed(bounds, 1.6);
        assert!(close(framed.target, [1.5, 0.5, -0.5]));
        let radius = 3.0_f64.sqrt() * 0.5;
        let expected = radius / 20.0_f64.to_radians().sin() / FILL;
        assert!((framed.distance() - expected).abs() < 1e-9);
        let before = offset(&start).normalize();
        let after = offset(&framed).normalize();
        assert!((before - after).length() < 1e-9);
    }

    #[test]
    fn the_scene_camera_reads_and_writes_back() {
        let camera = Camera {
            at: [0.0, 1.2, 3.2],
            look_at: [0.0, 0.35, 0.0],
            projection: Projection::Perspective { fov: 40.0 },
            ..Camera::default()
        };
        let eye = Eye::of(&camera);
        assert_eq!(eye.position, [0.0, 1.2, 3.2]);
        assert_eq!(eye.target, [0.0, 0.35, 0.0]);
        assert_eq!(eye.fov_y_deg, 40.0);
        let back = eye.camera(&camera, [[-2.0, -0.1, -2.0], [2.0, 1.0, 2.0]]);
        assert_eq!(back.at, camera.at);
        assert_eq!(back.look_at, camera.look_at);
        assert_eq!(back.projection, camera.projection);
        assert!(back.near > 0.0 && back.far > back.near);
        let (_, right, up) = back.axes();
        assert!((right[1]).abs() < 1e-6 && up[1] > 0.9);
    }

    #[test]
    fn the_depth_range_holds_the_bounds() {
        let (near, far) = eye().depth_range([[-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]]);
        assert!(near > 0.0 && near < 1.0, "{near}");
        assert!(far > 3.0 && far < 3.5, "{far}");
    }

    #[test]
    fn the_passepartout_is_the_largest_centred_frame_of_the_aspect() {
        let image = Rect::from_min_size(egui::Pos2::new(10.0, 20.0), Vec2::new(640.0, 480.0));
        let letterbox = passepartout(image, 16.0 / 9.0).unwrap();
        assert_eq!(letterbox.size(), Vec2::new(640.0, 360.0));
        assert_eq!(letterbox.center(), image.center());
        let pillarbox = passepartout(image, 1.0).unwrap();
        assert_eq!(pillarbox.size(), Vec2::new(480.0, 480.0));
        assert_eq!(pillarbox.center(), image.center());
        assert_eq!(passepartout(image, 4.0 / 3.0), Some(image));
        assert_eq!(passepartout(image, 0.0), None);
        assert_eq!(passepartout(image, f32::NAN), None);
    }

    #[test]
    fn a_widened_eye_over_the_panel_shows_the_frame_where_the_camera_put_it() {
        let image = Rect::from_min_size(egui::Pos2::new(10.0, 20.0), Vec2::new(640.0, 480.0));
        for aspect in [16.0 / 9.0, 0.75] {
            let frame = passepartout(image, aspect).unwrap();
            for ortho in [None, Some(3.0)] {
                let own = Eye {
                    position: [0.4, 1.1, 2.9],
                    target: [0.1, 0.2, -0.3],
                    shift: [0.12, -0.3],
                    ortho_height: ortho,
                    ..eye()
                };
                let wide = own.widened(reach(image, frame));
                let panel = crate::lens::Lens::new(&wide, image);
                let framed = crate::lens::Lens::new(&own, frame);
                for point in [[0.0, 0.0, 0.0], [0.6, -0.2, 0.4], [-0.9, 0.8, -1.0]] {
                    let point = DVec3::from_array(point);
                    let (a, b) = (
                        panel.project(point).unwrap(),
                        framed.project(point).unwrap(),
                    );
                    assert!((a - b).length() < 1e-3, "{aspect} {ortho:?} {a:?} {b:?}");
                }
            }
        }
    }

    #[test]
    fn the_engine_camera_through_the_frame_matches_the_widened_eye() {
        let camera = Camera {
            at: [0.0, 1.2, 3.2],
            look_at: [0.0, 0.35, 0.0],
            projection: Projection::Perspective { fov: 40.0 },
            shift: [0.1, -0.2],
            ..Camera::default()
        };
        let wide = through(&camera, [1.0, 4.0 / 3.0]);
        let eye = Eye::of(&camera).widened([1.0, 4.0 / 3.0]);
        assert_eq!(wide.projection, eye.projection());
        assert_eq!(wide.shift, [0.1, -0.15]);
        assert_eq!(
            (wide.at, wide.near, wide.far),
            (camera.at, camera.near, camera.far)
        );
        assert_eq!(through(&camera, [1.0, 1.0]).projection, camera.projection);
    }
}
