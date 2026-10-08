use rapier3d::prelude::SharedShape;
use rapier3d::prelude::{CoefficientCombineRule, Group, InteractionGroups, InteractionTestMode};

use super::Pose;
use super::convert::vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Sphere { radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
    Disc { radius: f32, thickness: f32 },
    Capsule { half_height: f32, radius: f32 },
    Box { half: [f32; 3] },
    RoundBox { half: [f32; 3], radius: f32 },
    Hull { points: Vec<[f32; 3]> },
}

impl Shape {
    pub(crate) fn shared(&self) -> Option<SharedShape> {
        match self {
            Self::Sphere { radius } => Some(SharedShape::ball(*radius)),
            Self::Cylinder {
                half_height,
                radius,
            } => Some(SharedShape::cylinder(*half_height, *radius)),
            Self::Disc { radius, thickness } => {
                Some(SharedShape::cylinder(thickness * 0.5, *radius))
            }
            Self::Capsule {
                half_height,
                radius,
            } => Some(SharedShape::capsule_y(*half_height, *radius)),
            Self::Box { half } => Some(SharedShape::cuboid(half[0], half[1], half[2])),
            Self::RoundBox { half, radius } => {
                let radius = radius.min(half[0]).min(half[1]).min(half[2]).max(0.0);
                Some(SharedShape::round_cuboid(
                    half[0] - radius,
                    half[1] - radius,
                    half[2] - radius,
                    radius,
                ))
            }
            Self::Hull { points } => {
                let points: Vec<_> = points.iter().map(|&p| vec(p)).collect();
                SharedShape::convex_hull(&points)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Combine {
    #[default]
    Average,
    Min,
    Multiply,
    Max,
}

impl Combine {
    pub fn apply(a: f32, b: f32, rule_a: Self, rule_b: Self) -> f32 {
        match rule_a.max(rule_b) {
            Self::Average => (a + b) * 0.5,
            Self::Min => a.min(b).abs(),
            Self::Multiply => a * b,
            Self::Max => a.max(b),
        }
    }

    pub(crate) fn rule(self) -> CoefficientCombineRule {
        match self {
            Self::Average => CoefficientCombineRule::Average,
            Self::Min => CoefficientCombineRule::Min,
            Self::Multiply => CoefficientCombineRule::Multiply,
            Self::Max => CoefficientCombineRule::Max,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Friction {
    pub still: f32,
    pub sliding: f32,
    pub combine: Combine,
}

impl Friction {
    pub const SLIP: f32 = 0.01;

    pub fn new(still: f32, sliding: f32) -> Self {
        Self {
            still,
            sliding,
            combine: Combine::Average,
        }
    }

    pub fn uniform(coefficient: f32) -> Self {
        Self::new(coefficient, coefficient)
    }

    pub fn with_combine(mut self, combine: Combine) -> Self {
        self.combine = combine;
        self
    }
}

impl Default for Friction {
    fn default() -> Self {
        Self::uniform(0.5)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layers {
    pub member: u32,
    pub mask: u32,
}

impl Layers {
    pub const ALL: Self = Self {
        member: u32::MAX,
        mask: u32::MAX,
    };

    pub fn new(member: u32, mask: u32) -> Self {
        Self { member, mask }
    }

    pub(crate) fn groups(self) -> InteractionGroups {
        InteractionGroups::new(
            Group::from_bits_retain(self.member),
            Group::from_bits_retain(self.mask),
            InteractionTestMode::And,
        )
    }
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            member: 1,
            mask: u32::MAX,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColliderDesc {
    pub shape: Shape,
    pub offset: Pose,
    pub density: f32,
    pub friction: Friction,
    pub restitution: f32,
    pub restitution_combine: Combine,
    pub sensor: bool,
    pub layers: Layers,
    pub impulse_threshold: Option<f32>,
}

impl ColliderDesc {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            offset: Pose::IDENTITY,
            density: 1.0,
            friction: Friction::default(),
            restitution: 0.0,
            restitution_combine: Combine::Average,
            sensor: false,
            layers: Layers::default(),
            impulse_threshold: None,
        }
    }

    pub fn sphere(radius: f32) -> Self {
        Self::new(Shape::Sphere { radius })
    }

    pub fn cuboid(half: [f32; 3]) -> Self {
        Self::new(Shape::Box { half })
    }

    pub fn round_box(half: [f32; 3], radius: f32) -> Self {
        Self::new(Shape::RoundBox { half, radius })
    }

    pub fn with_offset(mut self, offset: Pose) -> Self {
        self.offset = offset;
        self
    }

    pub fn with_density(mut self, density: f32) -> Self {
        self.density = density;
        self
    }

    pub fn with_friction(mut self, friction: Friction) -> Self {
        self.friction = friction;
        self
    }

    pub fn with_restitution(mut self, restitution: f32, combine: Combine) -> Self {
        self.restitution = restitution;
        self.restitution_combine = combine;
        self
    }

    pub fn with_layers(mut self, layers: Layers) -> Self {
        self.layers = layers;
        self
    }

    pub fn with_impulse_threshold(mut self, impulse: f32) -> Self {
        self.impulse_threshold = Some(impulse);
        self
    }

    pub fn as_sensor(mut self) -> Self {
        self.sensor = true;
        self
    }
}
