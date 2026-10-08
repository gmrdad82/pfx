use crate::bvh::{Hit, Ray};
use crate::math::{add, dot, length, normalize, scale, sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    RoundedBox {
        center: [f32; 3],
        half: [f32; 3],
        radius: f32,
        material: u32,
    },
    RoundCone {
        a: [f32; 3],
        b: [f32; 3],
        radius_a: f32,
        radius_b: f32,
        material: u32,
    },
    Ellipsoid {
        center: [f32; 3],
        radii: [f32; 3],
        material: u32,
    },
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
        material: u32,
    },
    SmoothUnion {
        left: usize,
        right: usize,
        radius: f32,
        material: u32,
    },
}

impl Shape {
    pub fn material(self) -> u32 {
        match self {
            Self::RoundedBox { material, .. }
            | Self::RoundCone { material, .. }
            | Self::Ellipsoid { material, .. }
            | Self::Capsule { material, .. }
            | Self::SmoothUnion { material, .. } => material,
        }
    }
    pub fn distance(self, p: [f32; 3], shapes: &[Shape]) -> f32 {
        match self {
            Self::RoundedBox {
                center,
                half,
                radius,
                ..
            } => {
                let q = [0, 1, 2].map(|i| (p[i] - center[i]).abs() - (half[i] - radius));
                length(q.map(|v| v.max(0.0))) + q[0].max(q[1]).max(q[2]).min(0.0) - radius
            }
            Self::RoundCone {
                a,
                b,
                radius_a,
                radius_b,
                ..
            } => round_cone(p, a, b, radius_a, radius_b),
            Self::Ellipsoid { center, radii, .. } => {
                let q = sub(p, center);
                let k0 = length([0, 1, 2].map(|i| q[i] / radii[i]));
                let k1 = length([0, 1, 2].map(|i| q[i] / (radii[i] * radii[i])));
                k0 * (k0 - 1.0) / k1.max(1e-8)
            }
            Self::Capsule { a, b, radius, .. } => {
                let ba = sub(b, a);
                let h = (dot(sub(p, a), ba) / dot(ba, ba).max(1e-9)).clamp(0.0, 1.0);
                length(sub(p, add(a, scale(ba, h)))) - radius
            }
            Self::SmoothUnion {
                left,
                right,
                radius,
                ..
            } => {
                if left >= shapes.len() || right >= shapes.len() {
                    return f32::INFINITY;
                }
                let a = shapes[left].distance(p, shapes);
                let b = shapes[right].distance(p, shapes);
                let h = (0.5 + 0.5 * (b - a) / radius.max(1e-6)).clamp(0.0, 1.0);
                b * (1.0 - h) + a * h - radius * h * (1.0 - h)
            }
        }
    }
}

fn round_cone(p: [f32; 3], a: [f32; 3], b: [f32; 3], r1: f32, r2: f32) -> f32 {
    let ba = sub(b, a);
    let axis_length = length(ba);
    if axis_length <= (r1 - r2).abs() {
        return if r1 >= r2 {
            length(sub(p, a)) - r1
        } else {
            length(sub(p, b)) - r2
        };
    }
    let l2 = dot(ba, ba);
    let rr = r1 - r2;
    let a2 = l2 - rr * rr;
    let pa = sub(p, a);
    let y = dot(pa, ba);
    let z = y - l2;
    let xv = sub(scale(pa, l2), scale(ba, y));
    let x2 = dot(xv, xv);
    let y2 = y * y * l2;
    let z2 = z * z * l2;
    let k = rr.signum() * rr * rr * x2;
    if z.signum() * a2 * z2 > k {
        return (x2 + z2).sqrt() / l2 - r2;
    }
    if y.signum() * a2 * y2 < k {
        return (x2 + y2).sqrt() / l2 - r1;
    }
    ((x2 * a2 / l2).max(0.0)).sqrt() / l2 + y * rr / l2 - r1
}

pub fn intersect(shapes: &[Shape], ray: Ray, limit: f32) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    let mut consumed = vec![false; shapes.len()];
    for shape in shapes {
        if let Shape::SmoothUnion { left, right, .. } = shape {
            if *left < shapes.len() {
                consumed[*left] = true;
            }
            if *right < shapes.len() {
                consumed[*right] = true;
            }
        }
    }
    for (index, shape) in shapes.iter().copied().enumerate() {
        if consumed[index] {
            continue;
        }
        let max = best.map_or(limit, |hit| hit.distance);
        if let Some(distance) = intersect_one(shape, shapes, ray, max) {
            let p = add(ray.origin, scale(ray.direction, distance));
            let e = 0.0002;
            let normal = normalize([0, 1, 2].map(|axis| {
                let mut a = p;
                let mut b = p;
                a[axis] += e;
                b[axis] -= e;
                shape.distance(a, shapes) - shape.distance(b, shapes)
            }));
            best = Some(Hit {
                distance,
                normal,
                material: shape.material(),
                index: index as u32,
            });
        }
    }
    best
}

fn intersect_one(shape: Shape, shapes: &[Shape], ray: Ray, limit: f32) -> Option<f32> {
    if let Shape::Ellipsoid { center, radii, .. } = shape {
        let o = [0, 1, 2].map(|i| (ray.origin[i] - center[i]) / radii[i]);
        let d = [0, 1, 2].map(|i| ray.direction[i] / radii[i]);
        let a = dot(d, d);
        let b = dot(o, d);
        let c = dot(o, o) - 1.0;
        let discr = b * b - a * c;
        if discr < 0.0 {
            return None;
        }
        let root = discr.sqrt();
        let t = (-b - root) / a;
        let t = if t > 1e-5 { t } else { (-b + root) / a };
        return (t > 1e-5 && t < limit).then_some(t);
    }
    if !matches!(shape, Shape::SmoothUnion { .. }) {
        return analytic_hit(shape, ray, limit);
    }
    let mut t = 0.0f32;
    let mut last = shape.distance(ray.origin, shapes);
    if !last.is_finite() {
        return None;
    }
    for _ in 0..512 {
        let previous = t;
        t += last.abs().max(0.00005);
        if t >= limit {
            return None;
        }
        let distance = shape.distance(add(ray.origin, scale(ray.direction, t)), shapes);
        if !distance.is_finite() {
            return None;
        }
        if distance.abs() < 1e-5 {
            return Some(t);
        }
        if (last > 0.0 && distance < 0.0) || (last < 0.0 && distance > 0.0) {
            let mut lo = previous;
            let mut hi = t;
            for _ in 0..24 {
                let mid = (lo + hi) * 0.5;
                let value = shape.distance(add(ray.origin, scale(ray.direction, mid)), shapes);
                if value.signum() == last.signum() {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return Some((lo + hi) * 0.5);
        }
        last = distance;
    }
    None
}

fn roots(a: f32, b: f32, c: f32) -> [f32; 2] {
    if a.abs() < 1e-10 {
        return if b.abs() < 1e-10 {
            [f32::INFINITY; 2]
        } else {
            [-c / (2.0 * b), f32::INFINITY]
        };
    }
    let discr = b * b - a * c;
    if discr < 0.0 {
        return [f32::INFINITY; 2];
    }
    let root = discr.sqrt();
    [(-b - root) / a, (-b + root) / a]
}

fn analytic_hit(shape: Shape, ray: Ray, limit: f32) -> Option<f32> {
    let mut best = limit;
    let mut accept = |t: f32| {
        if t > 1e-5 && t < best {
            let p = add(ray.origin, scale(ray.direction, t));
            if shape.distance(p, &[]).abs() < 0.002 {
                best = t;
            }
        }
    };
    match shape {
        Shape::RoundedBox {
            center,
            half,
            radius,
            ..
        } => {
            let core = half.map(|v| (v - radius).max(0.0));
            for axis in 0..3 {
                if ray.direction[axis].abs() > 1e-9 {
                    for side in [-1.0, 1.0] {
                        let t = (center[axis] + side * (core[axis] + radius) - ray.origin[axis])
                            / ray.direction[axis];
                        accept(t);
                    }
                }
            }
            for axis in 0..3 {
                let j = (axis + 1) % 3;
                let k = (axis + 2) % 3;
                for sj in [-1.0, 1.0] {
                    for sk in [-1.0, 1.0] {
                        let oj = ray.origin[j] - center[j] - sj * core[j];
                        let ok = ray.origin[k] - center[k] - sk * core[k];
                        let a = ray.direction[j] * ray.direction[j]
                            + ray.direction[k] * ray.direction[k];
                        let b = oj * ray.direction[j] + ok * ray.direction[k];
                        let c = oj * oj + ok * ok - radius * radius;
                        for t in roots(a, b, c) {
                            if (ray.origin[axis] + ray.direction[axis] * t - center[axis]).abs()
                                <= core[axis] + 0.002
                            {
                                accept(t);
                            }
                        }
                    }
                }
            }
            for sx in [-1.0, 1.0] {
                for sy in [-1.0, 1.0] {
                    for sz in [-1.0, 1.0] {
                        let corner = add(center, [sx * core[0], sy * core[1], sz * core[2]]);
                        let o = sub(ray.origin, corner);
                        for t in roots(
                            dot(ray.direction, ray.direction),
                            dot(o, ray.direction),
                            dot(o, o) - radius * radius,
                        ) {
                            accept(t);
                        }
                    }
                }
            }
        }
        Shape::Capsule { a, b, radius, .. } => {
            let ba = sub(b, a);
            let oa = sub(ray.origin, a);
            let baba = dot(ba, ba);
            let bard = dot(ba, ray.direction);
            let baoa = dot(ba, oa);
            let rdoa = dot(ray.direction, oa);
            let oaoa = dot(oa, oa);
            let qa = baba * dot(ray.direction, ray.direction) - bard * bard;
            let qb = baba * rdoa - baoa * bard;
            let qc = baba * oaoa - baoa * baoa - radius * radius * baba;
            for t in roots(qa, qb, qc) {
                let y = baoa + t * bard;
                if y >= 0.0 && y <= baba {
                    accept(t);
                }
            }
            for center in [a, b] {
                let o = sub(ray.origin, center);
                for t in roots(
                    dot(ray.direction, ray.direction),
                    dot(o, ray.direction),
                    dot(o, o) - radius * radius,
                ) {
                    accept(t);
                }
            }
        }
        Shape::RoundCone {
            a,
            b,
            radius_a,
            radius_b,
            ..
        } => {
            let ba = sub(b, a);
            let length = length(ba);
            if length > 1e-8 {
                let axis = scale(ba, 1.0 / length);
                let slope = (radius_a - radius_b) / length;
                if slope.abs() < 1.0 {
                    let side = (1.0 - slope * slope).sqrt();
                    let oa = sub(ray.origin, a);
                    let y0 = dot(oa, axis);
                    let yd = dot(ray.direction, axis);
                    let radial0 = sub(oa, scale(axis, y0));
                    let radiald = sub(ray.direction, scale(axis, yd));
                    let e = radius_a - slope * y0;
                    let qa = side * side * dot(radiald, radiald) - slope * slope * yd * yd;
                    let qb = side * side * dot(radial0, radiald) + e * slope * yd;
                    let qc = side * side * dot(radial0, radial0) - e * e;
                    for t in roots(qa, qb, qc) {
                        let y = y0 + t * yd;
                        if y >= slope * radius_a - 0.002 && y <= length + slope * radius_b + 0.002 {
                            accept(t);
                        }
                    }
                }
            }
            for (center, radius) in [(a, radius_a), (b, radius_b)] {
                let o = sub(ray.origin, center);
                for t in roots(
                    dot(ray.direction, ray.direction),
                    dot(o, ray.direction),
                    dot(o, o) - radius * radius,
                ) {
                    accept(t);
                }
            }
        }
        _ => {}
    }
    (best < limit).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hit(shape: Shape, origin: [f32; 3], direction: [f32; 3]) -> f32 {
        intersect(&[shape], Ray { origin, direction }, 100.0)
            .unwrap()
            .distance
    }
    #[test]
    fn ellipsoid_matches_quadratic() {
        let shape = Shape::Ellipsoid {
            center: [0.0; 3],
            radii: [2.0, 1.0, 1.0],
            material: 0,
        };
        assert!((hit(shape, [4.0, 0.0, 0.0], [-1.0, 0.0, 0.0]) - 2.0).abs() < 1e-5);
        assert!((hit(shape, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]) - 2.0).abs() < 1e-5);
    }
    #[test]
    fn sdf_shapes_match_axis_intersections() {
        let shapes = [
            (
                Shape::RoundedBox {
                    center: [0.0; 3],
                    half: [1.0; 3],
                    radius: 0.2,
                    material: 0,
                },
                1.0,
            ),
            (
                Shape::RoundCone {
                    a: [0.0, -1.0, 0.0],
                    b: [0.0, 1.0, 0.0],
                    radius_a: 0.5,
                    radius_b: 0.5,
                    material: 0,
                },
                0.5,
            ),
            (
                Shape::Capsule {
                    a: [0.0, -1.0, 0.0],
                    b: [0.0, 1.0, 0.0],
                    radius: 0.5,
                    material: 0,
                },
                0.5,
            ),
        ];
        for (shape, radius) in shapes {
            assert!((hit(shape, [3.0, 0.0, 0.0], [-1.0, 0.0, 0.0]) - (3.0 - radius)).abs() < 0.001);
        }
        let shapes = [
            Shape::Ellipsoid {
                center: [-1.0, 0.0, 0.0],
                radii: [1.0; 3],
                material: 0,
            },
            Shape::Ellipsoid {
                center: [1.0, 0.0, 0.0],
                radii: [1.0; 3],
                material: 0,
            },
            Shape::SmoothUnion {
                left: 0,
                right: 1,
                radius: 0.5,
                material: 0,
            },
        ];
        assert!(shapes[2].distance([0.0; 3], &shapes) < 0.0);
        let exit = intersect(
            &shapes,
            Ray {
                origin: [0.0; 3],
                direction: [1.0, 0.0, 0.0],
            },
            10.0,
        )
        .unwrap();
        assert!((exit.distance - 2.0).abs() < 0.001);
    }
    #[test]
    fn curved_patches_have_analytic_roots() {
        let box_shape = Shape::RoundedBox {
            center: [0.0; 3],
            half: [1.0; 3],
            radius: 0.2,
            material: 0,
        };
        let direction = normalize([-1.0, -1.0, -1.0]);
        let expected = 1.2 * 3.0f32.sqrt() - 0.2;
        assert!((hit(box_shape, [2.0; 3], direction) - expected).abs() < 0.0001);
        let capsule = Shape::Capsule {
            a: [0.0, -1.0, 0.0],
            b: [0.0, 1.0, 0.0],
            radius: 0.5,
            material: 0,
        };
        assert!((hit(capsule, [0.0, 3.0, 0.0], [0.0, -1.0, 0.0]) - 1.5).abs() < 0.0001);
        let cone = Shape::RoundCone {
            a: [0.0, -1.0, 0.0],
            b: [0.0, 1.0, 0.0],
            radius_a: 0.5,
            radius_b: 0.2,
            material: 0,
        };
        assert!((hit(cone, [3.0, -1.0, 0.0], [-1.0, 0.0, 0.0]) - 2.5).abs() < 0.001);
        assert!(
            (hit(cone, [3.0, 1.0, 0.0], [-1.0, 0.0, 0.0])
                - (3.0 - 0.2 / (1.0f32 - 0.15 * 0.15).sqrt()))
            .abs()
                < 0.001
        );
    }
}
