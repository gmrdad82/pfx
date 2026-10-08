use super::Pose;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dynamic,
    Kinematic,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    pub translation: [bool; 3],
    pub rotation: [bool; 3],
}

impl Locks {
    pub const NONE: Self = Self {
        translation: [false; 3],
        rotation: [false; 3],
    };
    pub const ROTATION: Self = Self {
        translation: [false; 3],
        rotation: [true; 3],
    };
    pub const PLANE_XY: Self = Self {
        translation: [false, false, true],
        rotation: [true, true, false],
    };
    pub const PLANE_YZ: Self = Self {
        translation: [true, false, false],
        rotation: [false, true, true],
    };

    pub fn any(&self) -> bool {
        self.translation.iter().chain(&self.rotation).any(|&l| l)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mass {
    pub mass: f32,
    pub center: [f32; 3],
    pub inertia: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyDesc {
    pub kind: Kind,
    pub pose: Pose,
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub locks: Locks,
    pub ccd: bool,
    pub can_sleep: bool,
    pub asleep: bool,
    pub mass: Option<Mass>,
}

impl BodyDesc {
    pub fn dynamic(pose: Pose) -> Self {
        Self {
            kind: Kind::Dynamic,
            pose,
            linear_velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            locks: Locks::NONE,
            ccd: false,
            can_sleep: true,
            asleep: false,
            mass: None,
        }
    }

    pub fn kinematic(pose: Pose) -> Self {
        Self {
            kind: Kind::Kinematic,
            ..Self::dynamic(pose)
        }
    }

    pub fn fixed(pose: Pose) -> Self {
        Self {
            kind: Kind::Fixed,
            ..Self::dynamic(pose)
        }
    }

    pub fn with_velocity(mut self, linear: [f32; 3]) -> Self {
        self.linear_velocity = linear;
        self
    }

    pub fn with_spin(mut self, angular: [f32; 3]) -> Self {
        self.angular_velocity = angular;
        self
    }

    pub fn with_locks(mut self, locks: Locks) -> Self {
        self.locks = locks;
        self
    }

    pub fn with_ccd(mut self) -> Self {
        self.ccd = true;
        self
    }

    pub fn with_mass(mut self, mass: Mass) -> Self {
        self.mass = Some(mass);
        self
    }

    pub fn with_damping(mut self, linear: f32, angular: f32) -> Self {
        self.linear_damping = linear;
        self.angular_damping = angular;
        self
    }

    pub fn with_gravity_scale(mut self, scale: f32) -> Self {
        self.gravity_scale = scale;
        self
    }
}

impl Default for BodyDesc {
    fn default() -> Self {
        Self::dynamic(Pose::IDENTITY)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    pub pose: Pose,
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub asleep: bool,
}
