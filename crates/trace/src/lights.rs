use bytemuck::{Pod, Zeroable};

use crate::math::{add, cross, dot, length, scale, sub};
use crate::shapes::Shape;

pub const MAX_EMITTERS: usize = 1 << 24;
pub const MAX_LIGHTS: usize = 1 << 16;
const SHAPE_MARGIN: f32 = 0.003;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightShape {
    Point,
    Spot {
        direction: [f32; 3],
        inner_deg: f32,
        outer_deg: f32,
    },
    Rect {
        half_u: [f32; 3],
        half_v: [f32; 3],
        two_sided: bool,
    },
    Disc {
        normal: [f32; 3],
        radius: f32,
        two_sided: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalLight {
    pub position: [f32; 3],
    pub colour: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
    pub range: f32,
    pub shadow: bool,
    pub shape: LightShape,
}

impl LocalLight {
    pub fn point(position: [f32; 3], colour: [f32; 3], intensity: f32, range: f32) -> Self {
        Self {
            position,
            colour,
            intensity,
            radius: 0.0,
            range,
            shadow: true,
            shape: LightShape::Point,
        }
    }

    pub fn rect(
        centre: [f32; 3],
        half_u: [f32; 3],
        half_v: [f32; 3],
        colour: [f32; 3],
        intensity: f32,
    ) -> Self {
        Self {
            position: centre,
            colour,
            intensity,
            radius: 0.0,
            range: f32::MAX,
            shadow: true,
            shape: LightShape::Rect {
                half_u,
                half_v,
                two_sided: false,
            },
        }
    }

    pub fn disc(
        centre: [f32; 3],
        normal: [f32; 3],
        radius: f32,
        colour: [f32; 3],
        intensity: f32,
    ) -> Self {
        Self {
            position: centre,
            colour,
            intensity,
            radius: 0.0,
            range: f32::MAX,
            shadow: true,
            shape: LightShape::Disc {
                normal,
                radius,
                two_sided: false,
            },
        }
    }

    pub fn lit(&self) -> bool {
        self.intensity > 0.0 && self.range > 0.0 && self.colour.iter().any(|&value| value > 0.0)
    }

    pub fn valid(&self) -> bool {
        let scalars = self
            .position
            .iter()
            .chain(&self.colour)
            .chain([&self.intensity, &self.radius, &self.range])
            .all(|value| value.is_finite())
            && self.colour.iter().all(|&value| value >= 0.0)
            && self.intensity >= 0.0
            && self.radius >= 0.0
            && self.range >= 0.0;
        scalars
            && match self.shape {
                LightShape::Point => true,
                LightShape::Spot {
                    direction,
                    inner_deg,
                    outer_deg,
                } => {
                    direction.iter().all(|value| value.is_finite())
                        && length(direction) > 1e-6
                        && inner_deg.is_finite()
                        && outer_deg.is_finite()
                        && 0.0 <= inner_deg
                        && inner_deg <= outer_deg
                        && outer_deg > 0.0
                        && outer_deg <= 90.0
                }
                LightShape::Rect { half_u, half_v, .. } => {
                    let lu = length(half_u);
                    let lv = length(half_v);
                    half_u.iter().chain(&half_v).all(|value| value.is_finite())
                        && lu > 1e-6
                        && lv > 1e-6
                        && dot(half_u, half_v).abs() <= 1e-4 * lu * lv
                }
                LightShape::Disc { normal, radius, .. } => {
                    normal.iter().all(|value| value.is_finite())
                        && length(normal) > 1e-6
                        && radius.is_finite()
                        && radius > 1e-6
                }
            }
    }

    pub fn area(&self) -> f32 {
        match self.shape {
            LightShape::Rect { half_u, half_v, .. } => 4.0 * length(cross(half_u, half_v)),
            LightShape::Disc { radius, .. } => std::f32::consts::PI * radius * radius,
            _ => 0.0,
        }
    }

    pub fn falloff(&self, distance: f32) -> f32 {
        if distance >= self.range || distance <= 1e-6 {
            return 0.0;
        }
        let ratio = distance / self.range;
        let window = (1.0 - ratio.powi(4)).clamp(0.0, 1.0);
        let reach = distance.max(self.radius);
        self.intensity * window * window / (reach * reach)
    }

    pub fn cone(&self, toward_surface: [f32; 3]) -> f32 {
        match self.shape {
            LightShape::Spot {
                direction,
                inner_deg,
                outer_deg,
            } => {
                let (scale, offset) = spot_terms(inner_deg, outer_deg);
                let axis = crate::math::normalize(direction);
                let cd = dot(crate::math::normalize(toward_surface), axis);
                let t = (cd * scale + offset).clamp(0.0, 1.0);
                t * t
            }
            _ => 1.0,
        }
    }

    pub(crate) fn pack(&self) -> GpuLight {
        let colour = self.colour.map(|channel| channel * self.intensity);
        let shadow = f32::from(u8::from(self.shadow));
        match self.shape {
            LightShape::Point => GpuLight {
                a: vec4(self.position, 0.0),
                b: vec4(colour, self.range),
                c: [0.0, 0.0, 0.0, self.radius],
                d: [0.0, 0.0, 0.0, shadow],
            },
            LightShape::Spot {
                direction,
                inner_deg,
                outer_deg,
            } => {
                let (scale, offset) = spot_terms(inner_deg, outer_deg);
                GpuLight {
                    a: vec4(self.position, 1.0),
                    b: vec4(colour, self.range),
                    c: vec4(crate::math::normalize(direction), self.radius),
                    d: [scale, offset, 0.0, shadow],
                }
            }
            LightShape::Rect {
                half_u,
                half_v,
                two_sided,
            } => {
                let area = self.area();
                GpuLight {
                    a: vec4(self.position, 2.0),
                    b: vec4(colour.map(|channel| channel / area), f32::MAX),
                    c: vec4(half_u, f32::from(u8::from(two_sided))),
                    d: vec4(half_v, shadow),
                }
            }
            LightShape::Disc {
                normal,
                radius,
                two_sided,
            } => {
                let area = self.area();
                GpuLight {
                    a: vec4(self.position, 3.0),
                    b: vec4(colour.map(|channel| channel / area), f32::MAX),
                    c: vec4(
                        crate::math::normalize(normal),
                        f32::from(u8::from(two_sided)),
                    ),
                    d: [radius, 0.0, 0.0, shadow],
                }
            }
        }
    }
}

fn spot_terms(inner_deg: f32, outer_deg: f32) -> (f32, f32) {
    let inner = inner_deg.to_radians().cos();
    let outer = outer_deg.to_radians().cos();
    let scale = 1.0 / (inner - outer).max(0.001);
    (scale, -outer * scale)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuLight {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub d: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub(crate) struct GpuEmitter {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub emission: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Source {
    Triangle {
        vertices: [[f32; 3]; 3],
        slot: u32,
    },
    Shape {
        center: [f32; 3],
        radius: f32,
        index: u32,
        area: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub source: Source,
    pub emission: [f32; 3],
}

pub(crate) struct EmitterList {
    pub entries: Vec<GpuEmitter>,
    pub cdf: Vec<f32>,
}

pub(crate) fn luminance(v: [f32; 3]) -> f32 {
    0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]
}

pub(crate) fn emits(emission: [f32; 3]) -> bool {
    emission
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
        && luminance(emission) > 0.0
}

pub(crate) fn triangle_area(vertices: [[f32; 3]; 3]) -> f32 {
    0.5 * length(cross(
        sub(vertices[1], vertices[0]),
        sub(vertices[2], vertices[0]),
    ))
}

impl Candidate {
    fn area(&self) -> f32 {
        match self.source {
            Source::Triangle { vertices, .. } => triangle_area(vertices),
            Source::Shape { area, .. } => area,
        }
    }
}

pub(crate) fn shape_area(shape: Shape, shapes: &[Shape]) -> f32 {
    use std::f32::consts::PI;
    match shape {
        Shape::RoundedBox { half, radius, .. } => {
            let core = half.map(|value| (value - radius).max(0.0));
            8.0 * (core[0] * core[1] + core[1] * core[2] + core[0] * core[2])
                + 2.0 * PI * radius * 2.0 * (core[0] + core[1] + core[2])
                + 4.0 * PI * radius * radius
        }
        Shape::RoundCone {
            a,
            b,
            radius_a,
            radius_b,
            ..
        } => {
            let axis = length(sub(b, a));
            let slant = (axis * axis + (radius_a - radius_b).powi(2)).sqrt();
            PI * (radius_a + radius_b) * slant
                + 2.0 * PI * (radius_a * radius_a + radius_b * radius_b)
        }
        Shape::Ellipsoid { radii, .. } => {
            let p = 1.6075;
            let [a, b, c] = radii.map(|value| value.powf(p));
            4.0 * PI * ((a * b + a * c + b * c) / 3.0).powf(1.0 / p)
        }
        Shape::Capsule { a, b, radius, .. } => {
            2.0 * PI * radius * length(sub(b, a)) + 4.0 * PI * radius * radius
        }
        Shape::SmoothUnion { left, right, .. } => {
            shape_area(shapes[left], shapes) + shape_area(shapes[right], shapes)
        }
    }
}

pub(crate) fn bounding_sphere(shape: Shape, shapes: &[Shape]) -> ([f32; 3], f32) {
    let (center, radius) = match shape {
        Shape::RoundedBox { center, half, .. } => (center, length(half)),
        Shape::RoundCone {
            a,
            b,
            radius_a,
            radius_b,
            ..
        } => (
            scale(add(a, b), 0.5),
            0.5 * length(sub(b, a)) + radius_a.max(radius_b),
        ),
        Shape::Ellipsoid { center, radii, .. } => (center, radii[0].max(radii[1]).max(radii[2])),
        Shape::Capsule { a, b, radius, .. } => {
            (scale(add(a, b), 0.5), 0.5 * length(sub(b, a)) + radius)
        }
        Shape::SmoothUnion {
            left,
            right,
            radius,
            ..
        } => {
            let (c0, r0) = bounding_sphere(shapes[left], shapes);
            let (c1, r1) = bounding_sphere(shapes[right], shapes);
            let gap = length(sub(c1, c0));
            if gap + r1 <= r0 {
                (c0, r0 + radius)
            } else if gap + r0 <= r1 {
                (c1, r1 + radius)
            } else {
                let r = 0.5 * (gap + r0 + r1);
                let center = add(c0, scale(sub(c1, c0), (r - r0) / gap.max(1e-12)));
                (center, r + radius)
            }
        }
    };
    (center, radius + SHAPE_MARGIN)
}

pub(crate) fn build_emitters(candidates: &[Candidate], lights: usize) -> EmitterList {
    let powers: Vec<f64> = candidates
        .iter()
        .map(|candidate| f64::from(candidate.area()) * f64::from(luminance(candidate.emission)))
        .collect();
    let total: f64 = powers.iter().sum();
    let mut entries = vec![GpuEmitter {
        a: [candidates.len() as f32, lights as f32, total as f32, 0.0],
        b: [0.0; 4],
        c: [0.0; 4],
        emission: [0.0; 4],
    }];
    let mut cdf = Vec::with_capacity(candidates.len());
    let mut running = 0.0_f64;
    for (candidate, power) in candidates.iter().zip(&powers) {
        running += power;
        cdf.push((running / total) as f32);
        let probability = (power / total) as f32;
        entries.push(match candidate.source {
            Source::Triangle { vertices, slot } => GpuEmitter {
                a: vec4(vertices[0], 0.0),
                b: vec4(vertices[1], slot as f32),
                c: vec4(vertices[2], triangle_area(vertices)),
                emission: vec4(candidate.emission, probability),
            },
            Source::Shape {
                center,
                radius,
                index,
                ..
            } => GpuEmitter {
                a: vec4(center, 1.0),
                b: vec4([0.0; 3], index as f32),
                c: [radius, 0.0, 0.0, 0.0],
                emission: vec4(candidate.emission, probability),
            },
        });
    }
    if let Some(last) = cdf.last_mut() {
        *last = 1.0;
    }
    EmitterList { entries, cdf }
}

fn vec4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(size: f32, height: f32) -> [[[f32; 3]; 3]; 2] {
        [
            [
                [0.0, height, 0.0],
                [size, height, 0.0],
                [size, height, size],
            ],
            [
                [0.0, height, 0.0],
                [size, height, size],
                [0.0, height, size],
            ],
        ]
    }

    #[test]
    fn emitters_are_weighted_by_power() {
        let mut candidates = Vec::new();
        for (slot, vertices) in quad(1.0, 2.0).into_iter().enumerate() {
            candidates.push(Candidate {
                source: Source::Triangle {
                    vertices,
                    slot: slot as u32,
                },
                emission: [1.0; 3],
            });
        }
        for (slot, vertices) in quad(2.0, 2.0).into_iter().enumerate() {
            candidates.push(Candidate {
                source: Source::Triangle {
                    vertices,
                    slot: 2 + slot as u32,
                },
                emission: [0.5; 3],
            });
        }
        let list = build_emitters(&candidates, 1);
        assert_eq!(list.entries.len(), 5);
        assert_eq!(list.entries[0].a[0], 4.0);
        assert_eq!(list.entries[0].a[1], 1.0);
        assert!((list.entries[0].a[2] - 3.0).abs() < 1e-5);
        let probabilities: Vec<f32> = list.entries[1..].iter().map(|e| e.emission[3]).collect();
        let expected = [1.0 / 6.0, 1.0 / 6.0, 1.0 / 3.0, 1.0 / 3.0];
        for (got, want) in probabilities.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{got} against {want}");
        }
        let mut running = 0.0;
        for (value, want) in list.cdf.iter().zip(expected) {
            running += want;
            assert!((value - running).abs() < 1e-6);
        }
        assert_eq!(list.cdf.len(), 4);
        assert_eq!(list.cdf[3], 1.0);
        assert_eq!(list.entries[3].b[3], 2.0);
        assert!((list.entries[3].c[3] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn empty_list_keeps_its_header() {
        let list = build_emitters(&[], 2);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].a[..2], [0.0, 2.0]);
        assert!(list.cdf.is_empty());
    }

    #[test]
    fn only_positive_emission_emits() {
        assert!(emits([0.0, 0.0, 1.0]));
        assert!(!emits([0.0; 3]));
        assert!(!emits([1.0, -0.5, 1.0]));
        assert!(!emits([f32::NAN, 1.0, 1.0]));
    }

    #[test]
    fn bounding_spheres_hold_their_shapes() {
        let shapes = [
            Shape::RoundedBox {
                center: [1.0, 2.0, 3.0],
                half: [0.5, 0.2, 0.1],
                radius: 0.05,
                material: 0,
            },
            Shape::Capsule {
                a: [0.0, 0.0, 0.0],
                b: [0.0, 2.0, 0.0],
                radius: 0.25,
                material: 0,
            },
            Shape::RoundCone {
                a: [0.0, 0.0, 0.0],
                b: [1.0, 0.0, 0.0],
                radius_a: 0.4,
                radius_b: 0.1,
                material: 0,
            },
            Shape::Ellipsoid {
                center: [0.5, 0.0, 0.0],
                radii: [0.3, 0.6, 0.2],
                material: 0,
            },
            Shape::SmoothUnion {
                left: 1,
                right: 3,
                radius: 0.1,
                material: 0,
            },
        ];
        for shape in shapes {
            let (center, radius) = bounding_sphere(shape, &shapes);
            for i in 0..4096 {
                let z = 1.0 - 2.0 * (i as f32 + 0.5) / 4096.0;
                let phi = i as f32 * 2.399_963;
                let r = (1.0 - z * z).sqrt();
                let p = add(center, scale([r * phi.cos(), z, r * phi.sin()], radius));
                assert!(shape.distance(p, &shapes) > 0.0, "{shape:?} at {p:?}");
            }
        }
        let sphere = Shape::Ellipsoid {
            center: [0.0; 3],
            radii: [0.5; 3],
            material: 0,
        };
        let area = shape_area(sphere, &[]);
        assert!((area - std::f32::consts::PI).abs() < 1e-4);
    }

    #[test]
    fn lights_validate_and_pack() {
        let point = LocalLight::point([0.0, 2.0, 0.0], [1.0, 0.5, 0.25], 4.0, 10.0);
        assert!(point.valid() && point.lit());
        let packed = point.pack();
        assert_eq!(packed.b, [4.0, 2.0, 1.0, 10.0]);
        assert_eq!(packed.d[3], 1.0);
        assert!((point.falloff(2.0) - 4.0 * (1.0 - 0.2_f32.powi(4)).powi(2) / 4.0).abs() < 1e-6);
        assert_eq!(point.falloff(10.0), 0.0);
        let rect = LocalLight::rect(
            [0.0, 2.0, 0.0],
            [0.5, 0.0, 0.0],
            [0.0, 0.0, 0.25],
            [1.0; 3],
            2.0,
        );
        assert!(rect.valid());
        assert!((rect.area() - 0.5).abs() < 1e-6);
        assert_eq!(rect.pack().b[..3], [4.0; 3]);
        let skew = LocalLight {
            shape: LightShape::Rect {
                half_u: [0.5, 0.0, 0.0],
                half_v: [0.5, 0.0, 0.25],
                two_sided: false,
            },
            ..rect
        };
        assert!(!skew.valid());
        let spot = LocalLight {
            shape: LightShape::Spot {
                direction: [0.0, -1.0, 0.0],
                inner_deg: 20.0,
                outer_deg: 30.0,
            },
            ..point
        };
        assert!(spot.valid());
        assert_eq!(spot.cone([0.0, -1.0, 0.0]), 1.0);
        assert_eq!(spot.cone([1.0, -1.0, 0.0]), 0.0);
        let edge = spot.cone([25.0_f32.to_radians().tan(), -1.0, 0.0]);
        assert!(edge > 0.0 && edge < 1.0);
        let wide = LocalLight {
            shape: LightShape::Spot {
                direction: [0.0, -1.0, 0.0],
                inner_deg: 40.0,
                outer_deg: 30.0,
            },
            ..point
        };
        assert!(!wide.valid());
    }
}
