use glam::{DMat4, DVec3};

pub const GRAVITY: f64 = 9.81;

const PITCH_GYRATION: f64 = 0.47;
const ROLL_GYRATION: f64 = 0.36;
const SUBSTEP_S: f64 = 0.001;
const SLIP_MAX: f64 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chassis {
    pub mass: f64,
    pub wheelbase: f64,
    pub track: f64,
    pub cg_height: f64,
    pub stiffness: f64,
    pub damping: f64,
    pub grip: f64,
}

impl Chassis {
    pub const MASS: f64 = 1400.0;
    pub const WHEELBASE: f64 = 2.7;
    pub const TRACK: f64 = 1.6;
    pub const CG_HEIGHT: f64 = 0.5;
    pub const STIFFNESS: f64 = 30_000.0;
    pub const DAMPING: f64 = 3_000.0;
    pub const GRIP: f64 = 1.0;

    fn corners(&self) -> [(f64, f64); 4] {
        let (x, z) = (0.5 * self.track, 0.5 * self.wheelbase);
        [(x, z), (-x, z), (x, -z), (-x, -z)]
    }

    fn inertia(&self) -> (f64, f64) {
        let pitch = PITCH_GYRATION * self.wheelbase;
        let roll = ROLL_GYRATION * self.track;
        (self.mass * pitch * pitch, self.mass * roll * roll)
    }

    pub fn settled(&self, force: DVec3) -> Lean {
        let k = self.stiffness;
        let corners = self.corners();
        let along: f64 = corners.iter().map(|(_, z)| z * z).sum();
        let across: f64 = corners.iter().map(|(x, _)| x * x).sum();
        Lean {
            heave: -self.mass * (force.y - GRAVITY) / (4.0 * k),
            pitch: -self.mass * force.z * self.cg_height / (k * along),
            roll: self.mass * force.x * self.cg_height / (k * across),
        }
    }

    pub fn slip(&self, force: DVec3) -> f64 {
        let limit = self.grip.max(0.0) * GRAVITY;
        if limit <= 0.0 {
            return 0.0;
        }
        let drive = force.z.max(0.0);
        let left = (limit * limit - drive * drive).max(0.0).sqrt();
        let excess = (force.x.abs() - left).max(0.0);
        libm::atan(excess / limit).min(SLIP_MAX) * force.x.signum()
    }

    pub fn tracking(&self, curvature: f64) -> f64 {
        -libm::atan(0.5 * self.wheelbase * curvature)
    }
}

impl Default for Chassis {
    fn default() -> Self {
        Chassis {
            mass: Self::MASS,
            wheelbase: Self::WHEELBASE,
            track: Self::TRACK,
            cg_height: Self::CG_HEIGHT,
            stiffness: Self::STIFFNESS,
            damping: Self::DAMPING,
            grip: Self::GRIP,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lean {
    pub heave: f64,
    pub pitch: f64,
    pub roll: f64,
}

impl Lean {
    pub fn matrix(&self) -> DMat4 {
        DMat4::from_translation(DVec3::new(0.0, self.heave, 0.0))
            * DMat4::from_rotation_x(self.pitch)
            * DMat4::from_rotation_z(self.roll)
    }

    pub fn lerp(&self, other: &Lean, s: f64) -> Lean {
        Lean {
            heave: self.heave + (other.heave - self.heave) * s,
            pitch: self.pitch + (other.pitch - self.pitch) * s,
            roll: self.roll + (other.roll - self.roll) * s,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ride {
    chassis: Chassis,
    at: Lean,
    rate: Lean,
}

impl Ride {
    pub fn settled(chassis: Chassis, force: DVec3) -> Ride {
        Ride {
            chassis,
            at: chassis.settled(force),
            rate: Lean::default(),
        }
    }

    pub fn lean(&self) -> Lean {
        self.at
    }

    pub fn step(&mut self, force: DVec3, dt: f64) -> Lean {
        if dt <= 0.0 {
            return self.at;
        }
        let steps = (dt / SUBSTEP_S).ceil().max(1.0);
        let h = dt / steps;
        let c = self.chassis;
        let (pitch_inertia, roll_inertia) = c.inertia();
        let preload = 0.25 * c.mass * GRAVITY;
        for _ in 0..steps as usize {
            let (mut lift, mut pitch, mut roll) = (0.0, 0.0, 0.0);
            for (x, z) in c.corners() {
                let y = self.at.heave - z * self.at.pitch + x * self.at.roll;
                let v = self.rate.heave - z * self.rate.pitch + x * self.rate.roll;
                let push = (preload - c.stiffness * y - c.damping * v).max(0.0);
                lift += push;
                pitch -= z * push;
                roll += x * push;
            }
            let heave = (lift - c.mass * force.y) / c.mass;
            let pitch = (pitch - c.mass * force.z * c.cg_height) / pitch_inertia;
            let roll = (roll + c.mass * force.x * c.cg_height) / roll_inertia;
            self.rate.heave += heave * h;
            self.rate.pitch += pitch * h;
            self.rate.roll += roll * h;
            self.at.heave += self.rate.heave * h;
            self.at.pitch += self.rate.pitch * h;
            self.at.roll += self.rate.roll * h;
        }
        self.at
    }
}

pub fn ride(chassis: Chassis, forces: &[DVec3], dt: f64) -> Vec<Lean> {
    let Some(first) = forces.first() else {
        return Vec::new();
    };
    let mut ride = Ride::settled(chassis, *first);
    forces
        .iter()
        .enumerate()
        .map(|(n, force)| {
            if n == 0 {
                ride.lean()
            } else {
                ride.step(*force, dt)
            }
        })
        .collect()
}

pub fn forces(poses: &[DMat4], dt: f64) -> Vec<DVec3> {
    let places: Vec<DVec3> = poses.iter().map(|m| m.w_axis.truncate()).collect();
    let last = places.len().saturating_sub(1);
    (0..places.len())
        .map(|n| {
            let accel = if last < 2 {
                DVec3::ZERO
            } else {
                let at = n.clamp(1, last - 1);
                (places[at + 1] - 2.0 * places[at] + places[at - 1]) / (dt * dt)
            };
            let world = accel + DVec3::new(0.0, GRAVITY, 0.0);
            let m = poses[n];
            DVec3::new(
                world.dot(m.x_axis.truncate().normalize_or_zero()),
                world.dot(m.y_axis.truncate().normalize_or_zero()),
                world.dot(m.z_axis.truncate().normalize_or_zero()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn at_rest_the_body_sits_level() {
        let c = Chassis::default();
        let lean = ride(c, &vec![DVec3::new(0.0, GRAVITY, 0.0); 120], 1.0 / 60.0);
        for l in lean {
            assert!(l.heave.abs() < 1e-12 && l.pitch.abs() < 1e-12 && l.roll.abs() < 1e-12);
        }
    }

    #[test]
    fn braking_dips_the_nose_by_the_weight_transfer_over_the_pitch_stiffness() {
        let c = Chassis::default();
        let braking = DVec3::new(0.0, GRAVITY, -8.0);
        let want = c.mass * 8.0 * c.cg_height / (c.stiffness * c.wheelbase * c.wheelbase);
        assert!(close(c.settled(braking).pitch, want, 1e-12));
        assert!(close(want.to_degrees(), 1.467, 0.01));
        let mut forces = vec![DVec3::new(0.0, GRAVITY, 0.0); 30];
        forces.extend(vec![braking; 150]);
        let lean = ride(c, &forces, 1.0 / 60.0);
        assert!(lean[29].pitch.abs() < 1e-12);
        let peak = lean.iter().map(|l| l.pitch).fold(f64::MIN, f64::max);
        assert!(peak > want && peak < 1.5 * want, "{peak} {want}");
        assert!(close(lean[179].pitch, want, 0.02 * want));
        let nose = lean[179]
            .matrix()
            .transform_point3(DVec3::new(0.0, 0.5, 2.0));
        assert!(nose.y < 0.5);
    }

    #[test]
    fn a_right_hand_bend_rolls_the_body_toward_its_left_the_outside() {
        let c = Chassis::default();
        let right = DVec3::new(-8.0, GRAVITY, 0.0);
        let lean = ride(c, &vec![right; 120], 1.0 / 60.0);
        let want = -c.mass * 8.0 * c.cg_height / (c.stiffness * c.track * c.track);
        assert!(close(lean[119].roll, want, 1e-9));
        assert!(close(want.to_degrees(), -4.18, 0.01));
        let left_side = lean[119]
            .matrix()
            .transform_point3(DVec3::new(0.8, 1.0, 0.0));
        let right_side = lean[119]
            .matrix()
            .transform_point3(DVec3::new(-0.8, 1.0, 0.0));
        assert!(left_side.y < right_side.y);
    }

    #[test]
    fn grip_left_after_throttle_sets_the_slip() {
        let c = Chassis::default();
        assert_eq!(c.slip(DVec3::new(-7.0, GRAVITY, 0.0)), 0.0);
        assert_eq!(c.slip(DVec3::new(-9.0, GRAVITY, -4.0)), 0.0);
        let exit = c.slip(DVec3::new(-9.0, GRAVITY, 5.0));
        let left = (GRAVITY * GRAVITY - 25.0f64).sqrt();
        assert!(close(exit, -((9.0 - left) / GRAVITY).atan(), 1e-12));
        assert!(exit < -0.05 && exit > -0.15, "{exit}");
        assert!(close(c.slip(DVec3::new(9.0, GRAVITY, 5.0)), -exit, 1e-12));
        assert!(c.slip(DVec3::new(-40.0, GRAVITY, 9.0)) >= -SLIP_MAX);
    }

    #[test]
    fn forces_read_the_path_in_the_body_frame() {
        let dt = 0.1;
        let poses: Vec<DMat4> = (0..5)
            .map(|n| {
                let t = f64::from(n) * dt;
                DMat4::from_translation(DVec3::new(0.0, 0.0, 10.0 * t - t * t))
            })
            .collect();
        for f in forces(&poses, dt) {
            assert!(close(f.z, -2.0, 1e-9) && close(f.y, GRAVITY, 1e-9) && f.x.abs() < 1e-9);
        }
    }
}
