use std::sync::Mutex;

use rapier3d::prelude::{
    ColliderHandle, ColliderSet, CollisionEvent, ContactPair, EventHandler, RigidBodySet,
    SoftBodySet, SoftBodyTearEvent,
};

use super::ColliderId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    ContactStarted {
        a: ColliderId,
        b: ColliderId,
        impulse: f32,
    },
    ContactStopped {
        a: ColliderId,
        b: ColliderId,
    },
    ContactImpulse {
        a: ColliderId,
        b: ColliderId,
        impulse: f32,
    },
    SensorEntered {
        sensor: ColliderId,
        other: ColliderId,
    },
    SensorExited {
        sensor: ColliderId,
        other: ColliderId,
    },
}

#[derive(Default)]
pub(crate) struct Collector {
    pub(crate) collisions: Mutex<Vec<CollisionEvent>>,
    pub(crate) impulses: Mutex<Vec<(ColliderHandle, ColliderHandle, f32)>>,
}

impl EventHandler for Collector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _pair: Option<&ContactPair>,
    ) {
        if let Ok(mut events) = self.collisions.lock() {
            events.push(event);
        }
    }

    fn handle_contact_force_event(
        &self,
        dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        pair: &ContactPair,
        total_force_magnitude: f32,
    ) {
        if let Ok(mut impulses) = self.impulses.lock() {
            impulses.push((pair.collider1, pair.collider2, total_force_magnitude * dt));
        }
    }

    fn handle_soft_body_tear_event(&self, _soft_bodies: &SoftBodySet, _event: &SoftBodyTearEvent) {}
}
