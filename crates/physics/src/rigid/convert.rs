use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};

use super::{BodyId, ColliderId, Pose};

pub(crate) type Vec3 = rapier3d::math::Vector;
pub(crate) type Quat = rapier3d::math::Rotation;
pub(crate) type Iso = rapier3d::math::Pose;

pub(crate) fn vec(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

pub(crate) fn quat(q: [f32; 4]) -> Quat {
    Quat::from_xyzw(q[0], q[1], q[2], q[3])
}

pub(crate) fn quat_axis_angle(axis: [f32; 3], angle: f32) -> Quat {
    let axis = vec(axis);
    let length = axis.length();
    if length <= f32::EPSILON {
        return Quat::IDENTITY;
    }
    Quat::from_axis_angle(axis / length, angle)
}

pub(crate) fn iso(pose: Pose) -> Iso {
    Iso::from_parts(vec(pose.position), quat(pose.rotation).normalize())
}

pub(crate) fn pose(iso: &Iso) -> Pose {
    Pose {
        position: iso.translation.to_array(),
        rotation: iso.rotation.to_array(),
    }
}

pub(crate) fn body_handle(id: BodyId) -> RigidBodyHandle {
    RigidBodyHandle::from_raw_parts(id.index, id.generation)
}

pub(crate) fn body_id(handle: RigidBodyHandle) -> BodyId {
    let (index, generation) = handle.into_raw_parts();
    BodyId { index, generation }
}

pub(crate) fn collider_handle(id: ColliderId) -> ColliderHandle {
    ColliderHandle::from_raw_parts(id.index, id.generation)
}

pub(crate) fn collider_id(handle: ColliderHandle) -> ColliderId {
    let (index, generation) = handle.into_raw_parts();
    ColliderId { index, generation }
}

pub(crate) fn lerp_iso(from: &Iso, to: &Iso, t: f32) -> Iso {
    Iso::from_parts(
        from.translation.lerp(to.translation, t),
        from.rotation.slerp(to.rotation, t).normalize(),
    )
}
