use super::Pose;
use super::convert::{Vec3, iso, vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    Constant,
    Inverse,
    InverseSquare,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Region {
    Sphere { center: [f32; 3], radius: f32 },
    Box { pose: Pose, half: [f32; 3] },
}

impl Region {
    pub fn contains(&self, point: [f32; 3]) -> bool {
        match *self {
            Self::Sphere { center, radius } => {
                vec(point).distance_squared(vec(center)) <= radius * radius
            }
            Self::Box { pose, half } => {
                let local = iso(pose).inverse_transform_point(vec(point));
                local.x.abs() <= half[0] && local.y.abs() <= half[1] && local.z.abs() <= half[2]
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Point {
        center: [f32; 3],
        strength: f32,
        falloff: Falloff,
        range: f32,
        softening: f32,
    },
    Zone {
        region: Region,
        gravity: [f32; 3],
        replace: bool,
    },
}

impl Source {
    pub fn attractor(center: [f32; 3], strength: f32) -> Self {
        Self::Point {
            center,
            strength,
            falloff: Falloff::InverseSquare,
            range: f32::INFINITY,
            softening: 0.01,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub(crate) u32);

pub(crate) fn acceleration(sources: &[(SourceId, Source)], world: Vec3, point: Vec3) -> Vec3 {
    let mut base = world;
    let mut extra = Vec3::ZERO;
    for (_, source) in sources {
        match *source {
            Source::Point {
                center,
                strength,
                falloff,
                range,
                softening,
            } => {
                let towards = vec(center) - point;
                let distance = towards.length();
                if distance > range || distance <= f32::EPSILON {
                    continue;
                }
                let d = distance.max(softening);
                let scale = match falloff {
                    Falloff::Constant => 1.0,
                    Falloff::Inverse => 1.0 / d,
                    Falloff::InverseSquare => 1.0 / (d * d),
                };
                extra += towards / distance * (strength * scale);
            }
            Source::Zone {
                region,
                gravity,
                replace,
            } => {
                if !region.contains(point.to_array()) {
                    continue;
                }
                if replace {
                    base = vec(gravity);
                } else {
                    extra += vec(gravity);
                }
            }
        }
    }
    base - world + extra
}
