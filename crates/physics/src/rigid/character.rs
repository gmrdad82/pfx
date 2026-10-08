use pfx_core::sim::cosf;
use rapier3d::control::{
    CharacterAutostep, CharacterCollision, CharacterLength, KinematicCharacterController,
};
use rapier3d::parry::query::{ShapeCastHit, ShapeCastOptions};
use rapier3d::prelude::{Collider, ColliderHandle, QueryFilter, Ray, RigidBodySet, SharedShape};

use super::convert::{Iso, Quat, Vec3, body_handle, body_id, vec};
use super::{BodyDesc, BodyId, ColliderDesc, ColliderId, Layers, Locks, Pose, Shape, World};

const WALL: f32 = 0.3;
const REACH: f32 = 0.05;
pub const TICKS: u32 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane {
    Xy,
    Yz,
}

impl Plane {
    fn axis(self) -> usize {
        match self {
            Self::Xy => 2,
            Self::Yz => 0,
        }
    }

    fn locks(self) -> Locks {
        match self {
            Self::Xy => Locks::PLANE_XY,
            Self::Yz => Locks::PLANE_YZ,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterDesc {
    pub radius: f32,
    pub height: f32,
    pub speed: f32,
    pub acceleration: f32,
    pub air_acceleration: f32,
    pub max_climb: f32,
    pub max_step: f32,
    pub step_width: f32,
    pub snap: f32,
    pub skin: f32,
    pub jump_speed: f32,
    pub coyote: u32,
    pub jump_buffer: u32,
    pub gravity_scale: f32,
    pub max_fall: f32,
    pub mass: f32,
    pub push: bool,
    pub ride: bool,
    pub variable_jump: f32,
    pub wall_slide: Option<f32>,
    pub wall_jump: Option<[f32; 2]>,
    pub plane: Option<Plane>,
    pub layers: Layers,
}

impl Default for CharacterDesc {
    fn default() -> Self {
        Self {
            radius: 0.3,
            height: 1.8,
            speed: 5.0,
            acceleration: 60.0,
            air_acceleration: 20.0,
            max_climb: 45.0,
            max_step: 0.3,
            step_width: 0.1,
            snap: 0.2,
            skin: 0.02,
            jump_speed: 5.0,
            coyote: 6,
            jump_buffer: 6,
            gravity_scale: 1.0,
            max_fall: 50.0,
            mass: 80.0,
            push: true,
            ride: true,
            variable_jump: 0.0,
            wall_slide: None,
            wall_jump: None,
            plane: None,
            layers: Layers::default(),
        }
    }
}

impl CharacterDesc {
    pub fn flat(plane: Plane) -> Self {
        Self {
            plane: Some(plane),
            ..Self::default()
        }
    }

    pub fn check(&self) -> Result<(), String> {
        let positive = [
            ("radius", self.radius),
            ("height", self.height),
            ("skin", self.skin),
        ];
        for (name, value) in positive {
            if !(value.is_finite() && value > 0.0) {
                return Err(format!("{name} needs a number above 0"));
            }
        }
        let at_least_zero = [
            ("speed", self.speed),
            ("acceleration", self.acceleration),
            ("air_acceleration", self.air_acceleration),
            ("max_step", self.max_step),
            ("step_width", self.step_width),
            ("snap", self.snap),
            ("jump_speed", self.jump_speed),
            ("gravity_scale", self.gravity_scale),
            ("max_fall", self.max_fall),
            ("mass", self.mass),
        ];
        for (name, value) in at_least_zero {
            if !(value.is_finite() && value >= 0.0) {
                return Err(format!("{name} needs a number of 0 or more"));
            }
        }
        if self.height < 2.0 * self.radius {
            return Err(format!(
                "height {} is less than twice the radius {}",
                self.height, self.radius
            ));
        }
        if !(self.max_climb.is_finite() && (0.0..90.0).contains(&self.max_climb)) {
            return Err("max_climb needs degrees from 0 up to 90".to_string());
        }
        if self.max_step >= self.height {
            return Err(format!(
                "max_step {} is not below the height {}",
                self.max_step, self.height
            ));
        }
        for (name, ticks) in [("coyote", self.coyote), ("jump_buffer", self.jump_buffer)] {
            if ticks > TICKS {
                return Err(format!("{name} needs 0 to {TICKS} ticks"));
            }
        }
        if !(self.variable_jump.is_finite() && (0.0..=1.0).contains(&self.variable_jump)) {
            return Err("variable_jump needs a number from 0 to 1".to_string());
        }
        if let Some(slide) = self.wall_slide
            && !(slide.is_finite() && slide >= 0.0)
        {
            return Err("wall_slide needs a number of 0 or more".to_string());
        }
        if let Some(kick) = self.wall_jump
            && !kick.iter().all(|part| part.is_finite() && *part >= 0.0)
        {
            return Err("wall_jump needs two numbers of 0 or more".to_string());
        }
        Ok(())
    }

    fn half_height(&self) -> f32 {
        (self.height * 0.5 - self.radius).max(0.0)
    }

    fn collider(&self) -> ColliderDesc {
        ColliderDesc::new(self.shape())
            .with_layers(self.layers)
            .with_density(0.0)
    }

    fn shape(&self) -> Shape {
        Shape::Capsule {
            half_height: self.half_height(),
            radius: self.radius,
        }
    }

    fn controller(&self, up: Vec3, rising: bool) -> KinematicCharacterController {
        let radians = std::f32::consts::PI / 180.0;
        KinematicCharacterController {
            up,
            offset: CharacterLength::Absolute(self.skin),
            slide: true,
            autostep: (self.max_step > 0.0).then_some(CharacterAutostep {
                max_height: CharacterLength::Absolute(self.max_step),
                min_width: CharacterLength::Absolute(self.step_width),
                include_dynamic_bodies: false,
            }),
            max_slope_climb_angle: self.max_climb * radians,
            min_slope_slide_angle: self.max_climb * radians,
            snap_to_ground: (!rising && self.snap > 0.0)
                .then_some(CharacterLength::Absolute(self.snap)),
            normal_nudge_factor: 1.0e-4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    pub direction: [f32; 3],
    pub jump: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    desc: CharacterDesc,
    body: BodyId,
    collider: ColliderId,
    center: Vec3,
    velocity: Vec3,
    up: Vec3,
    intent: Intent,
    held: bool,
    buffered: Option<u32>,
    grounded: bool,
    sliding: bool,
    since_ground: u32,
    jumped: bool,
    rising: bool,
    wall: Option<Vec3>,
    since_wall: u32,
    ground: Option<BodyId>,
    anchor: f32,
    ticks: u64,
}

impl Character {
    pub fn spawn(world: &mut World, desc: CharacterDesc, feet: [f32; 3]) -> Result<Self, String> {
        desc.check()?;
        let feet = vec(feet);
        let up = up_of(world, feet, desc.plane);
        let center = feet + up * (desc.height * 0.5);
        let locks = desc.plane.map_or(Locks::NONE, Plane::locks);
        let body = world.add_body(
            BodyDesc::kinematic(Pose::new(center.to_array(), turn(up).to_array()))
                .with_locks(locks),
        );
        let Some(collider) = world.add_collider(body, desc.collider()) else {
            world.remove_body(body);
            return Err("the character's capsule did not build".to_string());
        };
        let anchor = desc.plane.map_or(0.0, |plane| center[plane.axis()]);
        Ok(Self {
            desc,
            body,
            collider,
            center,
            velocity: Vec3::ZERO,
            up,
            intent: Intent::default(),
            held: false,
            buffered: None,
            grounded: false,
            sliding: false,
            since_ground: u32::MAX,
            jumped: false,
            rising: false,
            wall: None,
            since_wall: u32::MAX,
            ground: None,
            anchor,
            ticks: 0,
        })
    }

    pub fn drive(&mut self, direction: [f32; 3], jump: bool) {
        self.intent = Intent { direction, jump };
    }

    pub fn intent(&self) -> Intent {
        self.intent
    }

    pub fn desc(&self) -> &CharacterDesc {
        &self.desc
    }

    pub fn set_desc(&mut self, world: &mut World, desc: CharacterDesc) -> Result<(), String> {
        desc.check()?;
        if desc.plane != self.desc.plane {
            return Err("plane cannot change while the character lives".to_string());
        }
        if desc.radius != self.desc.radius
            || desc.height != self.desc.height
            || desc.layers != self.desc.layers
        {
            let feet = self.feet_vec();
            let collider = world
                .add_collider(self.body, desc.collider())
                .ok_or_else(|| "the character's capsule did not build".to_string())?;
            world.remove_collider(self.collider);
            self.collider = collider;
            self.center = feet + self.up * (desc.height * 0.5);
            world.set_pose(self.body, self.pose());
        }
        self.desc = desc;
        Ok(())
    }

    pub fn body(&self) -> BodyId {
        self.body
    }

    pub fn collider(&self) -> ColliderId {
        self.collider
    }

    pub fn feet(&self) -> [f32; 3] {
        self.feet_vec().to_array()
    }

    pub fn center(&self) -> [f32; 3] {
        self.center.to_array()
    }

    pub fn velocity(&self) -> [f32; 3] {
        self.velocity.to_array()
    }

    pub fn up(&self) -> [f32; 3] {
        self.up.to_array()
    }

    pub fn grounded(&self) -> bool {
        self.grounded
    }

    pub fn sliding(&self) -> bool {
        self.sliding
    }

    pub fn on_wall(&self) -> Option<[f32; 3]> {
        self.wall
            .filter(|_| self.since_wall == 0)
            .map(|normal| normal.to_array())
    }

    pub fn ground(&self) -> Option<BodyId> {
        self.ground
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn teleport(&mut self, world: &mut World, feet: [f32; 3]) {
        let mut center = vec(feet) + self.up * (self.desc.height * 0.5);
        if let Some(plane) = self.desc.plane {
            center[plane.axis()] = self.anchor;
        }
        self.center = center;
        self.velocity = Vec3::ZERO;
        self.ground = None;
        self.grounded = false;
        world.set_pose(self.body, self.pose());
    }

    pub fn remove(self, world: &mut World) -> bool {
        world.remove_body(self.body)
    }

    pub fn step(&mut self, world: &mut World) {
        let dt = world.dt();
        let desc = self.desc;
        let gravity = vec(world.gravity_at(self.center.to_array())) * desc.gravity_scale;
        self.up = level_up(up_from(gravity, self.up), desc.plane, self.up);
        let up = self.up;
        let pull = gravity.length();

        let pressed = self.intent.jump && !self.held;
        self.held = self.intent.jump;
        self.buffered = if pressed {
            Some(0)
        } else {
            self.buffered
                .map(|age| age.saturating_add(1))
                .filter(|age| *age <= desc.jump_buffer)
        };

        let wish = {
            let raw = flatten(vec(self.intent.direction), desc.plane);
            let level = raw - up * raw.dot(up);
            let length = level.length();
            if length > 1.0 { level / length } else { level }
        } * desc.speed;

        let mut rise = self.velocity.dot(up);
        let mut level = self.velocity - up * rise;
        let rate = if self.grounded {
            desc.acceleration
        } else if self.sliding {
            0.0
        } else {
            desc.air_acceleration
        };
        level = toward(level, wish, rate * dt);

        if self.grounded && rise < 0.0 {
            rise = 0.0;
        }
        let can_jump = self.grounded || (self.since_ground <= desc.coyote && !self.jumped);
        let mut jumped_now = false;
        if self.buffered.is_some() {
            if can_jump {
                rise = desc.jump_speed;
                jumped_now = true;
            } else if let (Some([away, upward]), Some(normal)) = (desc.wall_jump, self.wall)
                && self.since_wall <= desc.coyote
            {
                let off = flatten(normal - up * normal.dot(up), desc.plane).normalize_or_zero();
                level = off * away;
                rise = upward;
                jumped_now = true;
            }
        }
        if jumped_now {
            self.buffered = None;
            self.jumped = true;
            self.rising = true;
            self.grounded = false;
        }
        if self.rising && !self.intent.jump && rise > 0.0 {
            rise *= 1.0 - desc.variable_jump;
            self.rising = false;
        }
        rise -= pull * dt;
        if rise <= 0.0 {
            self.rising = false;
        }
        if let Some(slide) = desc.wall_slide
            && !self.grounded
            && self.wall.is_some()
            && self.since_wall == 0
            && rise < -slide
        {
            rise = -slide;
        }
        rise = rise.max(-desc.max_fall);

        let carry = if desc.ride {
            self.carry(world, dt)
        } else {
            Vec3::ZERO
        };
        let desired = flatten((level + up * rise) * dt, desc.plane);

        let controller = desc.controller(up, rise > 0.0);
        let shape = SharedShape::capsule_y(desc.half_height(), desc.radius);
        let start = self.iso();
        let mut collisions: Vec<CharacterCollision> = Vec::new();
        let unseen = RigidBodySet::new();
        let physics = &world.physics;
        let (freed, moved) = {
            let pipeline = physics.broad_phase.as_query_pipeline(
                physics.narrow_phase.query_dispatcher(),
                &unseen,
                &physics.colliders,
                self.filter(),
            );
            let freeing = KinematicCharacterController {
                snap_to_ground: None,
                autostep: None,
                ..controller
            };
            let freed = flatten(
                freeing
                    .move_shape(dt, &pipeline, shape.as_ref(), &start, Vec3::ZERO, |_| {})
                    .translation,
                desc.plane,
            );
            let start = Iso::from_parts(start.translation + freed, start.rotation);
            let moved = controller.move_shape(
                dt,
                &pipeline,
                shape.as_ref(),
                &start,
                desired,
                |collision| collisions.push(collision),
            );
            (freed, moved)
        };
        let translation = freed + flatten(moved.translation, desc.plane);
        let mut center = self.center + translation;
        if carry != Vec3::ZERO {
            let along = carry.normalize_or_zero();
            let at = Iso::from_parts(center, turn(up));
            let reach = self
                .cast(world, &shape, &at, along, carry.length(), self.ground)
                .map_or(carry.length(), |(_, hit)| hit.time_of_impact);
            center += flatten(along * reach, desc.plane);
        }
        if let Some(plane) = desc.plane {
            center[plane.axis()] = self.anchor;
        }

        if desc.push && desc.mass > 0.0 && !collisions.is_empty() {
            let physics = &mut world.physics;
            let mut pipeline = physics.broad_phase.as_query_pipeline_mut(
                physics.narrow_phase.query_dispatcher(),
                &mut physics.bodies,
                &mut physics.colliders,
                self.filter(),
            );
            controller.solve_character_collision_impulses(
                dt,
                &mut pipeline,
                shape.as_ref(),
                desc.mass,
                &collisions,
            );
        }

        let below = self.cast(
            world,
            &shape,
            &Iso::from_parts(center, turn(up)),
            -up,
            desc.skin + desc.snap.max(REACH),
            None,
        );
        let level_ground = cosf(desc.max_climb * std::f32::consts::PI / 180.0);
        let floor = self
            .floor_normal(world, center, level_ground)
            .or(below.map(|(_, hit)| hit.normal1));
        let steep = floor.is_some_and(|normal| normal.dot(up) < level_ground - 1.0e-4);
        self.grounded = moved.grounded && !steep && !jumped_now && rise <= 0.0;
        self.sliding = moved.grounded && steep && !jumped_now;
        if self.sliding {
            let along = translation / dt;
            rise = along.dot(up).min(rise.max(0.0));
            level = along - up * along.dot(up);
        } else if self.grounded {
            rise = 0.0;
        } else if rise > 0.0
            && translation.dot(up) < rise * dt * 0.5
            && collisions.iter().any(|c| c.hit.normal1.dot(up) < -WALL)
        {
            rise = (translation.dot(up) / dt).max(0.0);
            self.rising = false;
        }
        for collision in &collisions {
            let normal = collision.hit.normal1;
            if normal.dot(up).abs() >= WALL {
                continue;
            }
            let side = flatten(normal - up * normal.dot(up), desc.plane).normalize_or_zero();
            let into = level.dot(side);
            if into < 0.0 {
                level -= side * into;
            }
        }
        self.velocity = level + up * rise;

        let wall = collisions
            .iter()
            .map(|collision| collision.hit.normal1)
            .find(|normal| normal.dot(up).abs() < WALL)
            .or_else(|| {
                if self.grounded || wish == Vec3::ZERO {
                    return None;
                }
                let at = Iso::from_parts(center, turn(up));
                self.cast(
                    world,
                    &shape,
                    &at,
                    wish.normalize_or_zero(),
                    desc.skin + REACH,
                    None,
                )
                .map(|(_, hit)| hit.normal1)
                .filter(|normal| normal.dot(up).abs() < WALL)
            });
        match wall {
            Some(normal) if !self.grounded => {
                self.wall = Some(normal);
                self.since_wall = 0;
            }
            _ => self.since_wall = self.since_wall.saturating_add(1),
        }
        if self.grounded {
            self.since_ground = 0;
            self.jumped = false;
            self.wall = None;
        } else {
            self.since_ground = self.since_ground.saturating_add(1);
        }

        self.center = center;
        self.ground = match below {
            Some((handle, _)) if self.grounded || self.sliding => {
                let physics = &world.physics;
                physics
                    .colliders
                    .get(handle)
                    .and_then(|collider| collider.parent())
                    .filter(|parent| physics.bodies.get(*parent).is_some_and(|rb| !rb.is_fixed()))
                    .map(body_id)
            }
            _ => None,
        };
        world.move_to(self.body, self.pose());
        self.ticks += 1;
    }

    fn feet_vec(&self) -> Vec3 {
        self.center - self.up * (self.desc.height * 0.5)
    }

    fn pose(&self) -> Pose {
        Pose::new(self.center.to_array(), turn(self.up).to_array())
    }

    fn iso(&self) -> Iso {
        Iso::from_parts(self.center, turn(self.up))
    }

    fn filter(&self) -> QueryFilter<'static> {
        QueryFilter::new()
            .exclude_sensors()
            .exclude_rigid_body(body_handle(self.body))
            .groups(self.desc.layers.groups())
    }

    fn carry(&self, world: &World, dt: f32) -> Vec3 {
        let Some(ground) = self.ground else {
            return Vec3::ZERO;
        };
        let feet = self.feet_vec();
        if let Some(to) = world.planned(ground) {
            let Some(from) = world.physics.bodies.get(body_handle(ground)) else {
                return Vec3::ZERO;
            };
            let from = *from.position();
            return to.transform_point(from.inverse_transform_point(feet)) - feet;
        }
        match world.physics.bodies.get(body_handle(ground)) {
            Some(rb) if rb.is_dynamic() => rb.velocity_at_point(feet) * dt,
            _ => Vec3::ZERO,
        }
    }

    fn floor_normal(&self, world: &World, center: Vec3, level: f32) -> Option<Vec3> {
        let physics = &world.physics;
        let pipeline = physics.broad_phase.as_query_pipeline(
            physics.narrow_phase.query_dispatcher(),
            &physics.bodies,
            &physics.colliders,
            self.filter(),
        );
        let reach = self.desc.half_height()
            + self.desc.radius / level.max(0.05)
            + self.desc.skin
            + self.desc.snap.max(REACH);
        pipeline
            .cast_ray_and_get_normal(&Ray::new(center, -self.up), reach, true)
            .map(|(_, hit)| hit.normal)
    }

    fn cast(
        &self,
        world: &World,
        shape: &SharedShape,
        at: &Iso,
        along: Vec3,
        reach: f32,
        ignore: Option<BodyId>,
    ) -> Option<(ColliderHandle, ShapeCastHit)> {
        let physics = &world.physics;
        let skip = ignore.map(body_handle);
        let other =
            |_: ColliderHandle, collider: &Collider| skip.is_none() || collider.parent() != skip;
        let pipeline = physics.broad_phase.as_query_pipeline(
            physics.narrow_phase.query_dispatcher(),
            &physics.bodies,
            &physics.colliders,
            self.filter().predicate(&other),
        );
        pipeline.cast_shape(
            at,
            along,
            shape.as_ref(),
            ShapeCastOptions {
                target_distance: self.desc.skin,
                stop_at_penetration: false,
                max_time_of_impact: reach,
                compute_impact_geometry_on_penetration: true,
            },
        )
    }
}

fn up_from(gravity: Vec3, last: Vec3) -> Vec3 {
    let length = gravity.length();
    if length > 1.0e-6 {
        -gravity / length
    } else if last == Vec3::ZERO {
        Vec3::Y
    } else {
        last
    }
}

fn up_of(world: &World, at: Vec3, plane: Option<Plane>) -> Vec3 {
    level_up(
        up_from(vec(world.gravity_at(at.to_array())), Vec3::Y),
        plane,
        Vec3::Y,
    )
}

fn level_up(up: Vec3, plane: Option<Plane>, last: Vec3) -> Vec3 {
    let flat = flatten(up, plane).normalize_or_zero();
    if flat == Vec3::ZERO { last } else { flat }
}

fn flatten(v: Vec3, plane: Option<Plane>) -> Vec3 {
    let Some(plane) = plane else {
        return v;
    };
    let mut v = v;
    v[plane.axis()] = 0.0;
    v
}

fn turn(up: Vec3) -> Quat {
    let up = up.normalize_or_zero();
    if up == Vec3::ZERO || up == Vec3::Y {
        return Quat::IDENTITY;
    }
    Quat::from_rotation_arc(Vec3::Y, up)
}

fn toward(from: Vec3, to: Vec3, step: f32) -> Vec3 {
    let gap = to - from;
    let length = gap.length();
    if length <= step || length <= f32::EPSILON {
        to
    } else {
        from + gap / length * step
    }
}
