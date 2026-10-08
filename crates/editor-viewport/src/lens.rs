use egui::{Pos2, Rect};
use glam::DVec3;

use crate::camera::Eye;

pub const NEAR: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: DVec3,
    pub direction: DVec3,
}

impl Ray {
    pub fn at(&self, distance: f64) -> DVec3 {
        self.origin + self.direction * distance
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    image: Rect,
    position: DVec3,
    right: DVec3,
    up: DVec3,
    forward: DVec3,
    shift: [f64; 2],
    half: [f64; 2],
    ortho: bool,
}

impl Lens {
    pub fn new(eye: &Eye, image: Rect) -> Lens {
        let basis = eye.basis();
        let aspect = f64::from(image.width().max(1.0)) / f64::from(image.height().max(1.0));
        let (half_y, ortho) = match eye.ortho_height {
            Some(height) => ((height * 0.5).max(1e-9), true),
            None => ((eye.fov_y_deg.to_radians() * 0.5).tan().max(1e-9), false),
        };
        Lens {
            image,
            position: DVec3::from_array(eye.position),
            right: basis.right,
            up: basis.up,
            forward: basis.forward,
            shift: eye.shift,
            half: [half_y * aspect, half_y],
            ortho,
        }
    }

    pub fn image(&self) -> Rect {
        self.image
    }

    pub fn ortho(&self) -> bool {
        self.ortho
    }

    pub fn depth(&self, point: DVec3) -> f64 {
        (point - self.position).dot(self.forward)
    }

    pub fn ndc(&self, point: DVec3) -> Option<[f64; 2]> {
        let offset = point - self.position;
        let depth = offset.dot(self.forward);
        let scale = if self.ortho {
            1.0
        } else if depth > NEAR {
            depth
        } else {
            return None;
        };
        Some([
            offset.dot(self.right) / (scale * self.half[0]) - self.shift[0],
            offset.dot(self.up) / (scale * self.half[1]) - self.shift[1],
        ])
    }

    pub fn project(&self, point: DVec3) -> Option<Pos2> {
        let [x, y] = self.ndc(point)?;
        let (left, top) = (f64::from(self.image.left()), f64::from(self.image.top()));
        let (width, height) = (
            f64::from(self.image.width()),
            f64::from(self.image.height()),
        );
        Some(Pos2::new(
            (left + (x + 1.0) * 0.5 * width) as f32,
            (top + (1.0 - y) * 0.5 * height) as f32,
        ))
    }

    pub fn segment(&self, a: DVec3, b: DVec3) -> Option<[Pos2; 2]> {
        if self.ortho {
            return Some([self.project(a)?, self.project(b)?]);
        }
        let near = NEAR * 1e3;
        let (da, db) = (self.depth(a), self.depth(b));
        if da < near && db < near {
            return None;
        }
        let clip = |inside: DVec3, outside: DVec3, din: f64, dout: f64| {
            inside + (outside - inside) * ((din - near) / (din - dout))
        };
        let (a, b) = match (da < near, db < near) {
            (true, false) => (clip(b, a, db, da), b),
            (false, true) => (a, clip(a, b, da, db)),
            _ => (a, b),
        };
        Some([self.project(a)?, self.project(b)?])
    }

    pub fn ray(&self, at: Pos2) -> Ray {
        let x = 2.0 * f64::from(at.x - self.image.left()) / f64::from(self.image.width().max(1.0))
            - 1.0;
        let y = 1.0
            - 2.0 * f64::from(at.y - self.image.top()) / f64::from(self.image.height().max(1.0));
        let across = (x + self.shift[0]) * self.half[0];
        let rise = (y + self.shift[1]) * self.half[1];
        if self.ortho {
            Ray {
                origin: self.position + self.right * across + self.up * rise,
                direction: self.forward,
            }
        } else {
            Ray {
                origin: self.position,
                direction: (self.forward + self.right * across + self.up * rise).normalize(),
            }
        }
    }

    pub fn metres_per_point(&self, point: DVec3) -> f64 {
        let tall = if self.ortho {
            2.0 * self.half[1]
        } else {
            2.0 * self.depth(point).max(NEAR) * self.half[1]
        };
        tall / f64::from(self.image.height().max(1.0))
    }

    pub fn toward_eye(&self, point: DVec3) -> DVec3 {
        if self.ortho {
            -self.forward
        } else {
            (self.position - point)
                .try_normalize()
                .unwrap_or(-self.forward)
        }
    }
}

pub fn pixel(image: Rect, at: Pos2, size: [u32; 2]) -> Option<[u32; 2]> {
    if !image.contains(at) {
        return None;
    }
    let side = |fraction: f32, length: u32| {
        ((fraction * length as f32).floor().max(0.0) as u32).min(length.saturating_sub(1))
    };
    Some([
        side((at.x - image.left()) / image.width().max(1.0), size[0]),
        side((at.y - image.top()) / image.height().max(1.0), size[1]),
    ])
}

pub fn triangle(ray: &Ray, corners: [DVec3; 3]) -> Option<f64> {
    let [a, b, c] = corners;
    let ab = b - a;
    let ac = c - a;
    let p = ray.direction.cross(ac);
    let det = ab.dot(p);
    if det.abs() < 1e-18 {
        return None;
    }
    let inverse = 1.0 / det;
    let t = ray.origin - a;
    let u = t.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(ab);
    let v = ray.direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = ac.dot(q) * inverse;
    (distance > 0.0).then_some(distance)
}

pub fn slab(ray: &Ray, low: DVec3, high: DVec3) -> Option<f64> {
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    for axis in 0..3 {
        let (origin, direction) = (ray.origin[axis], ray.direction[axis]);
        if direction.abs() < 1e-300 {
            if origin < low[axis] || origin > high[axis] {
                return None;
            }
            continue;
        }
        let a = (low[axis] - origin) / direction;
        let b = (high[axis] - origin) / direction;
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far >= 0.0).then_some(near.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Vec2;

    fn eye(shift: [f64; 2]) -> Eye {
        Eye {
            position: [0.0, 0.0, 2.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fov_y_deg: 90.0,
            shift,
            ortho_height: None,
        }
    }

    fn image() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(400.0, 200.0))
    }

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn projection_lands_on_hand_computed_points() {
        let lens = Lens::new(&eye([0.0, 0.0]), image());
        assert!(near(
            lens.project(DVec3::ZERO).unwrap(),
            Pos2::new(300.0, 150.0)
        ));
        assert!(near(
            lens.project(DVec3::new(0.0, 2.0, 0.0)).unwrap(),
            Pos2::new(300.0, 50.0)
        ));
        assert!(near(
            lens.project(DVec3::new(2.0, 0.0, 0.0)).unwrap(),
            Pos2::new(400.0, 150.0)
        ));
        assert!(near(
            lens.project(DVec3::new(1.0, -1.0, 1.0)).unwrap(),
            Pos2::new(400.0, 250.0)
        ));
        assert_eq!(lens.project(DVec3::new(0.0, 0.0, 3.0)), None);
    }

    #[test]
    fn lens_shift_moves_the_picture_as_the_engine_does() {
        let lens = Lens::new(&eye([0.1, -0.5]), image());
        let at = lens.project(DVec3::ZERO).unwrap();
        assert!(
            near(at, Pos2::new(300.0 - 0.1 * 200.0, 150.0 - 0.5 * 100.0)),
            "{at:?}"
        );
    }

    #[test]
    fn an_orthographic_lens_projects_flat() {
        let mut flat = eye([0.25, 0.0]);
        flat.ortho_height = Some(4.0);
        let lens = Lens::new(&flat, image());
        let centre = lens.project(DVec3::new(0.0, 0.0, -5.0)).unwrap();
        assert!(near(centre, Pos2::new(300.0 - 0.25 * 200.0, 150.0)));
        let corner = lens.project(DVec3::new(4.0, 2.0, 1.0)).unwrap();
        assert!(near(corner, Pos2::new(450.0, 50.0)), "{corner:?}");
        assert!((lens.metres_per_point(DVec3::ZERO) - 0.02).abs() < 1e-12);
    }

    #[test]
    fn a_ray_through_a_projected_point_passes_through_it() {
        for shift in [[0.0, 0.0], [0.1, -0.6395]] {
            for ortho in [None, Some(3.0)] {
                let mut view = eye(shift);
                view.position = [0.4, 0.7, 1.9];
                view.target = [0.1, 0.05, -0.2];
                view.ortho_height = ortho;
                let lens = Lens::new(&view, image());
                let point = DVec3::new(0.2, -0.1, 0.3);
                let ray = lens.ray(lens.project(point).unwrap());
                let along = (point - ray.origin).dot(ray.direction);
                assert!(
                    (ray.at(along) - point).length() < 1e-4,
                    "{shift:?} {ortho:?}"
                );
            }
        }
    }

    #[test]
    fn a_segment_behind_the_eye_is_cut_at_the_near_plane() {
        let lens = Lens::new(&eye([0.0, 0.0]), image());
        assert!(
            lens.segment(DVec3::new(0.0, 0.0, 3.0), DVec3::new(1.0, 0.0, 4.0))
                .is_none()
        );
        let [a, b] = lens
            .segment(DVec3::new(-1.0, 0.0, 0.0), DVec3::new(-1.0, 0.0, 5.0))
            .unwrap();
        assert!(near(a, Pos2::new(250.0, 150.0)), "{a:?}");
        assert!(b.x < a.x && b.x.is_finite(), "{b:?}");
    }

    #[test]
    fn metres_per_point_is_the_view_height_over_the_image() {
        let lens = Lens::new(&eye([0.0, 0.0]), image());
        assert!((lens.metres_per_point(DVec3::ZERO) - 4.0 / 200.0).abs() < 1e-12);
    }

    #[test]
    fn a_point_maps_to_the_frame_pixel() {
        assert_eq!(
            pixel(image(), Pos2::new(300.0, 150.0), [800, 400]),
            Some([400, 200])
        );
        assert_eq!(pixel(image(), Pos2::new(90.0, 150.0), [800, 400]), None);
        assert_eq!(
            pixel(image(), Pos2::new(500.0, 250.0), [800, 400]),
            Some([799, 399])
        );
    }

    #[test]
    fn a_slab_meets_the_box_in_front_only() {
        let ray = Ray {
            origin: DVec3::new(0.0, 0.0, 5.0),
            direction: DVec3::NEG_Z,
        };
        let low = DVec3::splat(-1.0);
        let high = DVec3::splat(1.0);
        assert_eq!(slab(&ray, low, high), Some(4.0));
        let away = Ray {
            direction: DVec3::Z,
            ..ray
        };
        assert_eq!(slab(&away, low, high), None);
        let inside = Ray {
            origin: DVec3::ZERO,
            ..ray
        };
        assert_eq!(slab(&inside, low, high), Some(0.0));
    }
}
