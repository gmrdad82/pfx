use rapier3d::parry::mass_properties::MassProperties;
use rapier3d::prelude::{
    ActiveCollisionTypes, ActiveEvents, ActiveHooks, BroadPhaseBvh, ColliderBuilder,
    ColliderHandle, CollisionEvent, CollisionEventFlags, ContactModificationContext, LockedAxes,
    PhysicsHooks, PhysicsWorld, RigidBodyBuilder, RigidBodyHandle,
};

use super::convert::{
    Iso, Quat, Vec3, body_handle, body_id, collider_handle, collider_id, iso, lerp_iso, pose, vec,
};
use super::event::Collector;
use super::gravity::{SourceId, acceleration};
use super::{
    BodyDesc, BodyId, BodyState, ColliderDesc, ColliderId, Combine, Drive, Event, Friction, Kind,
    Locks, Pose, Source,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldDesc {
    pub rate: f32,
    pub substeps: u32,
    pub iterations: u32,
    pub gravity: [f32; 3],
    pub ccd: bool,
}

impl Default for WorldDesc {
    fn default() -> Self {
        Self {
            rate: 60.0,
            substeps: 1,
            iterations: 4,
            gravity: [0.0, -9.81, 0.0],
            ccd: true,
        }
    }
}

struct Extra {
    generation: u32,
    kind: Kind,
    locks: Locks,
    anchor: Iso,
    force: Vec3,
    torque: Vec3,
    drive: Option<Drive>,
    target: Option<Iso>,
    mass_given: bool,
}

#[derive(Clone, Copy)]
struct Surface {
    generation: u32,
    friction: Friction,
}

pub struct World {
    pub(super) physics: PhysicsWorld,
    pub(super) queries: BroadPhaseBvh,
    rate: f32,
    substeps: u32,
    extras: Vec<Option<Extra>>,
    surfaces: Vec<Option<Surface>>,
    sources: Vec<(SourceId, Source)>,
    next_source: u32,
    forced: Vec<RigidBodyHandle>,
    collector: Collector,
    events: Vec<Event>,
    steps: u64,
}

impl Default for World {
    fn default() -> Self {
        Self::new(WorldDesc::default())
    }
}

impl World {
    pub fn new(desc: WorldDesc) -> Self {
        let rate = if desc.rate.is_finite() && desc.rate > 0.0 {
            desc.rate
        } else {
            60.0
        };
        let substeps = desc.substeps.max(1);
        let mut physics = PhysicsWorld::new();
        physics.gravity = vec(desc.gravity);
        physics.integration_parameters.dt = 1.0 / rate / substeps as f32;
        physics.integration_parameters.num_solver_iterations = desc.iterations.max(1) as usize;
        physics.integration_parameters.max_ccd_substeps = usize::from(desc.ccd);
        Self {
            physics,
            queries: BroadPhaseBvh::new(),
            rate,
            substeps,
            extras: Vec::new(),
            surfaces: Vec::new(),
            sources: Vec::new(),
            next_source: 0,
            forced: Vec::new(),
            collector: Collector::default(),
            events: Vec::new(),
            steps: 0,
        }
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    pub fn substeps(&self) -> u32 {
        self.substeps
    }

    pub fn dt(&self) -> f32 {
        1.0 / self.rate
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn time(&self) -> f64 {
        self.steps as f64 / f64::from(self.rate)
    }

    pub fn gravity(&self) -> [f32; 3] {
        self.physics.gravity.to_array()
    }

    pub fn set_gravity(&mut self, gravity: [f32; 3]) {
        self.physics.gravity = vec(gravity);
        self.wake_all();
    }

    pub fn gravity_at(&self, point: [f32; 3]) -> [f32; 3] {
        let world = self.physics.gravity;
        if self.sources.is_empty() {
            return world.to_array();
        }
        (world + acceleration(&self.sources, world, vec(point))).to_array()
    }

    pub(super) fn planned(&self, body: BodyId) -> Option<Iso> {
        let extra = self.extra(body)?;
        if extra.kind != Kind::Kinematic {
            return None;
        }
        let from = *self.physics.bodies.get(body_handle(body))?.position();
        let to = match (extra.drive, extra.target) {
            (_, Some(target)) => target,
            (Some(mut drive), None) => drive.advance(1.0 / self.rate, &from),
            (None, None) => return Some(from),
        };
        Some(locked(extra.locks, &extra.anchor, &to))
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn add_source(&mut self, source: Source) -> SourceId {
        let id = SourceId(self.next_source);
        self.next_source += 1;
        self.sources.push((id, source));
        self.wake_all();
        id
    }

    pub fn remove_source(&mut self, id: SourceId) -> bool {
        let before = self.sources.len();
        self.sources.retain(|(s, _)| *s != id);
        self.sources.len() != before
    }

    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut Source> {
        self.sources
            .iter_mut()
            .find(|(s, _)| *s == id)
            .map(|(_, source)| source)
    }

    pub fn add_body(&mut self, desc: BodyDesc) -> BodyId {
        let start = iso(desc.pose);
        let builder = match desc.kind {
            Kind::Dynamic => RigidBodyBuilder::dynamic(),
            Kind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
            Kind::Fixed => RigidBodyBuilder::fixed(),
        };
        let mut builder = builder
            .pose(start)
            .linvel(masked(vec(desc.linear_velocity), desc.locks.translation))
            .angvel(masked(vec(desc.angular_velocity), desc.locks.rotation))
            .gravity_scale(desc.gravity_scale)
            .linear_damping(desc.linear_damping)
            .angular_damping(desc.angular_damping)
            .locked_axes(locked_axes(desc.locks))
            .ccd_enabled(desc.ccd)
            .can_sleep(desc.can_sleep)
            .sleeping(desc.asleep);
        if let Some(mass) = desc.mass {
            builder = builder.additional_mass_properties(MassProperties::new(
                vec(mass.center),
                mass.mass,
                vec(mass.inertia),
            ));
        }
        let handle = self.physics.bodies.insert(builder);
        let (index, generation) = handle.into_raw_parts();
        let slot = index as usize;
        if self.extras.len() <= slot {
            self.extras.resize_with(slot + 1, || None);
        }
        self.extras[slot] = Some(Extra {
            generation,
            kind: desc.kind,
            locks: desc.locks,
            anchor: start,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            drive: None,
            target: None,
            mass_given: desc.mass.is_some(),
        });
        body_id(handle)
    }

    pub fn add_collider(&mut self, body: BodyId, desc: ColliderDesc) -> Option<ColliderId> {
        let mass_given = self.extra(body)?.mass_given;
        let shape = desc.shape.shared()?;
        let handle = body_handle(body);
        let mut events = ActiveEvents::COLLISION_EVENTS;
        let mut builder = ColliderBuilder::new(shape)
            .position(iso(desc.offset))
            .density(if mass_given { 0.0 } else { desc.density })
            .friction(desc.friction.sliding)
            .friction_combine_rule(desc.friction.combine.rule())
            .restitution(desc.restitution)
            .restitution_combine_rule(desc.restitution_combine.rule())
            .sensor(desc.sensor)
            .collision_groups(desc.layers.groups());
        if desc.sensor {
            builder = builder.active_collision_types(ActiveCollisionTypes::all());
        }
        if desc.friction.still != desc.friction.sliding {
            builder = builder.active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
        }
        if let Some(impulse) = desc.impulse_threshold {
            events |= ActiveEvents::CONTACT_FORCE_EVENTS;
            builder = builder
                .contact_force_event_threshold(impulse / self.physics.integration_parameters.dt);
        }
        let builder = builder.active_events(events);
        let physics = &mut self.physics;
        let collider = physics
            .colliders
            .insert_with_parent(builder, handle, &mut physics.bodies);
        if let Some(rb) = physics.bodies.get_mut(handle) {
            rb.recompute_mass_properties_from_colliders(&physics.colliders);
        }
        let (index, generation) = collider.into_raw_parts();
        let slot = index as usize;
        if self.surfaces.len() <= slot {
            self.surfaces.resize_with(slot + 1, || None);
        }
        self.surfaces[slot] = Some(Surface {
            generation,
            friction: desc.friction,
        });
        Some(collider_id(collider))
    }

    pub fn remove_body(&mut self, body: BodyId) -> bool {
        let handle = body_handle(body);
        let Some(rb) = self.physics.bodies.get(handle) else {
            return false;
        };
        for collider in rb.colliders().to_vec() {
            self.forget_surface(collider);
        }
        self.physics.remove_body(handle);
        self.extras[body.index as usize] = None;
        self.forced.retain(|h| *h != handle);
        true
    }

    pub fn remove_collider(&mut self, collider: ColliderId) -> bool {
        let handle = collider_handle(collider);
        let removed = self.physics.remove_collider(handle).is_some();
        if removed {
            self.forget_surface(handle);
        }
        removed
    }

    pub fn bodies(&self) -> Vec<BodyId> {
        self.extras
            .iter()
            .enumerate()
            .filter_map(|(index, extra)| {
                extra.as_ref().map(|e| BodyId {
                    index: index as u32,
                    generation: e.generation,
                })
            })
            .collect()
    }

    pub fn body(&self, body: BodyId) -> Option<BodyState> {
        let rb = self.physics.bodies.get(body_handle(body))?;
        Some(BodyState {
            pose: pose(rb.position()),
            linear_velocity: rb.linvel().to_array(),
            angular_velocity: rb.angvel().to_array(),
            asleep: rb.is_sleeping(),
        })
    }

    pub fn pose(&self, body: BodyId) -> Option<Pose> {
        self.physics
            .bodies
            .get(body_handle(body))
            .map(|rb| pose(rb.position()))
    }

    pub fn mass(&self, body: BodyId) -> Option<f32> {
        self.physics
            .bodies
            .get(body_handle(body))
            .map(|rb| rb.mass())
    }

    pub fn body_of(&self, collider: ColliderId) -> Option<BodyId> {
        self.physics
            .colliders
            .get(collider_handle(collider))?
            .parent()
            .map(body_id)
    }

    pub fn is_asleep(&self, body: BodyId) -> bool {
        self.physics
            .bodies
            .get(body_handle(body))
            .is_some_and(|rb| rb.is_sleeping())
    }

    pub fn set_pose(&mut self, body: BodyId, to: Pose) -> bool {
        let to = iso(to);
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        extra.anchor = to;
        extra.target = None;
        let Some(rb) = self.physics.bodies.get_mut(body_handle(body)) else {
            return false;
        };
        rb.set_position(to, true);
        true
    }

    pub fn set_velocity(&mut self, body: BodyId, linear: [f32; 3], angular: [f32; 3]) -> bool {
        let Some(locks) = self.extra(body).map(|e| e.locks) else {
            return false;
        };
        let Some(rb) = self.physics.bodies.get_mut(body_handle(body)) else {
            return false;
        };
        rb.set_linvel(masked(vec(linear), locks.translation), true);
        rb.set_angvel(masked(vec(angular), locks.rotation), true);
        true
    }

    pub fn apply_impulse(&mut self, body: BodyId, impulse: [f32; 3]) -> bool {
        self.with_body(body, |rb| rb.apply_impulse(vec(impulse), true))
    }

    pub fn apply_impulse_at(&mut self, body: BodyId, impulse: [f32; 3], point: [f32; 3]) -> bool {
        self.with_body(body, |rb| {
            rb.apply_impulse_at_point(vec(impulse), vec(point), true)
        })
    }

    pub fn apply_torque_impulse(&mut self, body: BodyId, impulse: [f32; 3]) -> bool {
        self.with_body(body, |rb| rb.apply_torque_impulse(vec(impulse), true))
    }

    pub fn add_force(&mut self, body: BodyId, force: [f32; 3]) -> bool {
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        extra.force += vec(force);
        true
    }

    pub fn add_force_at(&mut self, body: BodyId, force: [f32; 3], point: [f32; 3]) -> bool {
        let Some(center) = self
            .physics
            .bodies
            .get(body_handle(body))
            .map(|rb| rb.center_of_mass())
        else {
            return false;
        };
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        let force = vec(force);
        extra.force += force;
        extra.torque += (vec(point) - center).cross(force);
        true
    }

    pub fn add_torque(&mut self, body: BodyId, torque: [f32; 3]) -> bool {
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        extra.torque += vec(torque);
        true
    }

    pub fn set_gravity_scale(&mut self, body: BodyId, scale: f32) -> bool {
        self.with_body(body, |rb| rb.set_gravity_scale(scale, true))
    }

    pub fn set_damping(&mut self, body: BodyId, linear: f32, angular: f32) -> bool {
        self.with_body(body, |rb| {
            rb.set_linear_damping(linear);
            rb.set_angular_damping(angular);
        })
    }

    pub fn set_ccd(&mut self, body: BodyId, ccd: bool) -> bool {
        self.with_body(body, |rb| rb.enable_ccd(ccd))
    }

    pub fn set_locks(&mut self, body: BodyId, locks: Locks) -> bool {
        let Some(current) = self
            .physics
            .bodies
            .get(body_handle(body))
            .map(|rb| *rb.position())
        else {
            return false;
        };
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        extra.locks = locks;
        extra.anchor = current;
        self.with_body(body, |rb| rb.set_locked_axes(locked_axes(locks), true))
    }

    pub fn locks(&self, body: BodyId) -> Option<Locks> {
        self.extra(body).map(|e| e.locks)
    }

    pub fn sleep(&mut self, body: BodyId) -> bool {
        self.with_body(body, |rb| rb.sleep())
    }

    pub fn wake(&mut self, body: BodyId) -> bool {
        let handle = body_handle(body);
        if self.physics.bodies.get(handle).is_none() {
            return false;
        }
        self.physics.wake_up(handle, true);
        true
    }

    pub fn set_drive(&mut self, body: BodyId, drive: Option<Drive>) -> bool {
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        extra.drive = drive;
        true
    }

    pub fn drive(&self, body: BodyId) -> Option<&Drive> {
        self.extra(body)?.drive.as_ref()
    }

    pub fn drive_mut(&mut self, body: BodyId) -> Option<&mut Drive> {
        self.extra_mut(body)?.drive.as_mut()
    }

    pub fn move_to(&mut self, body: BodyId, to: Pose) -> bool {
        let Some(extra) = self.extra_mut(body) else {
            return false;
        };
        extra.target = Some(iso(to));
        true
    }

    pub fn step(&mut self) {
        self.events.clear();
        let dt = 1.0 / self.rate;
        let n = self.substeps;
        let motions = self.motions(dt);
        let mut started: Vec<(ColliderHandle, ColliderHandle, usize)> = Vec::new();
        for i in 0..n {
            self.apply_forces(dt / n as f32);
            for (handle, from, to) in &motions {
                let at = if i + 1 == n {
                    *to
                } else {
                    lerp_iso(from, to, (i + 1) as f32 / n as f32)
                };
                if let Some(rb) = self.physics.bodies.get_mut(*handle) {
                    rb.set_next_kinematic_position(at);
                }
            }
            let hooks = Hooks {
                surfaces: &self.surfaces,
            };
            self.physics.step_with_events(&hooks, &self.collector);
            self.collect(&mut started);
            self.enforce_locks();
        }
        for extra in self.extras.iter_mut().flatten() {
            extra.force = Vec3::ZERO;
            extra.torque = Vec3::ZERO;
        }
        self.steps += 1;
        self.refresh_queries();
    }

    pub fn refresh_queries(&mut self) {
        let physics = &mut self.physics;
        let moved: Vec<(ColliderHandle, Iso)> = physics
            .colliders
            .iter()
            .filter_map(|(handle, collider)| {
                let parent = physics.bodies.get(collider.parent()?)?;
                let at = *parent.position() * *collider.position_wrt_parent()?;
                (at != *collider.position()).then_some((handle, at))
            })
            .collect();
        for (handle, at) in moved {
            if let Some(collider) = physics.colliders.get_mut(handle) {
                collider.set_position(at);
            }
        }
        let mut queries = BroadPhaseBvh::new();
        for (handle, collider) in physics.colliders.iter() {
            queries.set_aabb(
                &physics.integration_parameters,
                handle,
                collider.compute_aabb(),
            );
        }
        self.queries = queries;
    }

    pub fn run(&mut self, steps: u32) {
        for _ in 0..steps {
            self.step();
        }
    }

    fn wake_all(&mut self) {
        for (index, slot) in self.extras.iter().enumerate() {
            if let Some(extra) = slot
                && extra.kind == Kind::Dynamic
            {
                let handle = RigidBodyHandle::from_raw_parts(index as u32, extra.generation);
                self.physics
                    .islands
                    .wake_up(&mut self.physics.bodies, handle, true);
            }
        }
    }

    fn extra(&self, body: BodyId) -> Option<&Extra> {
        self.extras
            .get(body.index as usize)?
            .as_ref()
            .filter(|e| e.generation == body.generation)
    }

    fn extra_mut(&mut self, body: BodyId) -> Option<&mut Extra> {
        self.extras
            .get_mut(body.index as usize)?
            .as_mut()
            .filter(|e| e.generation == body.generation)
    }

    fn with_body(
        &mut self,
        body: BodyId,
        f: impl FnOnce(&mut rapier3d::prelude::RigidBody),
    ) -> bool {
        match self.physics.bodies.get_mut(body_handle(body)) {
            Some(rb) => {
                f(rb);
                true
            }
            None => false,
        }
    }

    fn forget_surface(&mut self, handle: ColliderHandle) {
        let (index, generation) = handle.into_raw_parts();
        if let Some(slot) = self.surfaces.get_mut(index as usize)
            && slot.is_some_and(|s| s.generation == generation)
        {
            *slot = None;
        }
    }

    fn motions(&mut self, dt: f32) -> Vec<(RigidBodyHandle, Iso, Iso)> {
        let mut motions = Vec::new();
        for (index, slot) in self.extras.iter_mut().enumerate() {
            let Some(extra) = slot else {
                continue;
            };
            if extra.kind != Kind::Kinematic {
                continue;
            }
            let handle = RigidBodyHandle::from_raw_parts(index as u32, extra.generation);
            let Some(rb) = self.physics.bodies.get(handle) else {
                continue;
            };
            let from = *rb.position();
            let to = match (&mut extra.drive, extra.target.take()) {
                (_, Some(target)) => target,
                (Some(drive), None) => drive.advance(dt, &from),
                (None, None) => continue,
            };
            motions.push((handle, from, locked(extra.locks, &extra.anchor, &to)));
        }
        motions
    }

    fn apply_forces(&mut self, h: f32) {
        for handle in self.forced.drain(..) {
            if let Some(rb) = self.physics.bodies.get_mut(handle) {
                rb.reset_forces(false);
                rb.reset_torques(false);
            }
        }
        let world = self.physics.gravity;
        for (index, slot) in self.extras.iter().enumerate() {
            let Some(extra) = slot else {
                continue;
            };
            if extra.kind != Kind::Dynamic {
                continue;
            }
            let handle = RigidBodyHandle::from_raw_parts(index as u32, extra.generation);
            let Some(rb) = self.physics.bodies.get(handle) else {
                continue;
            };
            let kick = if !self.sources.is_empty() && !rb.is_sleeping() {
                acceleration(&self.sources, world, rb.center_of_mass())
                    * (rb.mass() * rb.gravity_scale() * h)
            } else {
                Vec3::ZERO
            };
            let user = extra.force != Vec3::ZERO || extra.torque != Vec3::ZERO;
            if kick == Vec3::ZERO && !user {
                continue;
            }
            if let Some(rb) = self.physics.bodies.get_mut(handle) {
                if kick != Vec3::ZERO {
                    rb.apply_impulse(kick, false);
                }
                if user {
                    rb.add_force(extra.force, true);
                    rb.add_torque(extra.torque, true);
                    self.forced.push(handle);
                }
            }
        }
    }

    fn collect(&mut self, started: &mut Vec<(ColliderHandle, ColliderHandle, usize)>) {
        let collisions = self
            .collector
            .collisions
            .lock()
            .map(|mut events| std::mem::take(&mut *events))
            .unwrap_or_default();
        for event in collisions {
            let (c1, c2, flags, entered) = match event {
                CollisionEvent::Started(c1, c2, flags) => (c1, c2, flags, true),
                CollisionEvent::Stopped(c1, c2, flags) => (c1, c2, flags, false),
            };
            let (a, b) = (collider_id(c1), collider_id(c2));
            if flags.contains(CollisionEventFlags::SENSOR) {
                let second = self
                    .physics
                    .colliders
                    .get(c2)
                    .is_some_and(|c| c.is_sensor());
                let first = self
                    .physics
                    .colliders
                    .get(c1)
                    .is_some_and(|c| c.is_sensor());
                let (sensor, other) = if second && !first { (b, a) } else { (a, b) };
                self.events.push(if entered {
                    Event::SensorEntered { sensor, other }
                } else {
                    Event::SensorExited { sensor, other }
                });
            } else if entered {
                started.push((c1, c2, self.events.len()));
                self.events
                    .push(Event::ContactStarted { a, b, impulse: 0.0 });
            } else {
                self.events.push(Event::ContactStopped { a, b });
            }
        }
        for (c1, c2, at) in started.iter() {
            let Some(pair) = self.physics.narrow_phase.contact_pair(*c1, *c2) else {
                continue;
            };
            let add = pair.total_impulse_magnitude();
            if let Some(Event::ContactStarted { impulse, .. }) = self.events.get_mut(*at) {
                *impulse += add;
            }
        }
        let impulses = self
            .collector
            .impulses
            .lock()
            .map(|mut impulses| std::mem::take(&mut *impulses))
            .unwrap_or_default();
        for (c1, c2, add) in impulses {
            let (a, b) = (collider_id(c1), collider_id(c2));
            let existing = self.events.iter_mut().find_map(|event| match event {
                Event::ContactImpulse {
                    a: ea,
                    b: eb,
                    impulse,
                } if *ea == a && *eb == b => Some(impulse),
                _ => None,
            });
            match existing {
                Some(impulse) => *impulse += add,
                None => self
                    .events
                    .push(Event::ContactImpulse { a, b, impulse: add }),
            }
        }
    }

    fn enforce_locks(&mut self) {
        for (index, slot) in self.extras.iter().enumerate() {
            let Some(extra) = slot else {
                continue;
            };
            if extra.kind != Kind::Dynamic || !extra.locks.any() {
                continue;
            }
            let handle = RigidBodyHandle::from_raw_parts(index as u32, extra.generation);
            let Some(rb) = self.physics.bodies.get(handle) else {
                continue;
            };
            let at = *rb.position();
            let fixed = locked(extra.locks, &extra.anchor, &at);
            let linear = rb.linvel();
            let angular = rb.angvel();
            let held_linear = masked(linear, extra.locks.translation);
            let held_angular = masked(angular, extra.locks.rotation);
            let moved = !same_iso(&fixed, &at);
            let spun = held_linear != linear || held_angular != angular;
            if !moved && !spun {
                continue;
            }
            if let Some(rb) = self.physics.bodies.get_mut(handle) {
                if moved {
                    rb.set_position(fixed, false);
                }
                if spun {
                    rb.set_linvel(held_linear, false);
                    rb.set_angvel(held_angular, false);
                }
            }
        }
    }
}

struct Hooks<'a> {
    surfaces: &'a [Option<Surface>],
}

impl Hooks<'_> {
    fn friction(&self, handle: ColliderHandle) -> Option<Friction> {
        let (index, generation) = handle.into_raw_parts();
        self.surfaces
            .get(index as usize)?
            .filter(|s| s.generation == generation)
            .map(|s| s.friction)
    }
}

impl PhysicsHooks for Hooks<'_> {
    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        let (Some(f1), Some(f2)) = (
            self.friction(context.collider1),
            self.friction(context.collider2),
        ) else {
            return;
        };
        let bodies = context.bodies;
        let colliders = context.colliders;
        let (b1, b2, c1) = (context.rigid_body1, context.rigid_body2, context.collider1);
        let Some(rigid) = context.rigid_mut() else {
            return;
        };
        let Some(point) = rigid.manifold.points.first() else {
            return;
        };
        let Some(position) = colliders.get(c1).map(|c| *c.position()) else {
            return;
        };
        let p = position.transform_point(point.local_p1);
        let velocity = |handle: Option<RigidBodyHandle>| {
            handle
                .and_then(|h| bodies.get(h))
                .map(|rb| rb.velocity_at_point(p))
                .unwrap_or(Vec3::ZERO)
        };
        let relative = velocity(b1) - velocity(b2);
        let normal = *rigid.normal;
        let tangent = relative - normal * relative.dot(normal);
        let (a, b) = if tangent.length() > Friction::SLIP {
            (f1.sliding, f2.sliding)
        } else {
            (f1.still, f2.still)
        };
        *rigid.friction = Combine::apply(a, b, f1.combine, f2.combine);
    }
}

fn masked(v: Vec3, locks: [bool; 3]) -> Vec3 {
    Vec3::new(
        if locks[0] { 0.0 } else { v.x },
        if locks[1] { 0.0 } else { v.y },
        if locks[2] { 0.0 } else { v.z },
    )
}

fn locked_axes(locks: Locks) -> LockedAxes {
    let flags = [
        LockedAxes::TRANSLATION_LOCKED_X,
        LockedAxes::TRANSLATION_LOCKED_Y,
        LockedAxes::TRANSLATION_LOCKED_Z,
        LockedAxes::ROTATION_LOCKED_X,
        LockedAxes::ROTATION_LOCKED_Y,
        LockedAxes::ROTATION_LOCKED_Z,
    ];
    locks
        .translation
        .iter()
        .chain(&locks.rotation)
        .zip(flags)
        .filter(|(on, _)| **on)
        .fold(LockedAxes::empty(), |all, (_, flag)| all | flag)
}

fn locked(locks: Locks, anchor: &Iso, at: &Iso) -> Iso {
    let mut translation = at.translation;
    for axis in 0..3 {
        if locks.translation[axis] {
            translation[axis] = anchor.translation[axis];
        }
    }
    let mut free = [0usize; 3];
    let mut count = 0;
    for axis in (0..3).filter(|&axis| !locks.rotation[axis]) {
        free[count] = axis;
        count += 1;
    }
    let rotation = match &free[..count] {
        [] => anchor.rotation,
        [axis] => {
            let relative = at.rotation * anchor.rotation.inverse();
            let mut parts = [0.0, 0.0, 0.0, relative.w];
            parts[*axis] = relative.to_array()[*axis];
            let twist = Quat::from_array(parts);
            let length = twist.length();
            let twist = if length > f32::EPSILON {
                twist / length
            } else {
                Quat::IDENTITY
            };
            twist * anchor.rotation
        }
        _ => at.rotation,
    };
    Iso::from_parts(translation, rotation)
}

fn same_iso(a: &Iso, b: &Iso) -> bool {
    a.translation == b.translation && a.rotation == b.rotation
}
