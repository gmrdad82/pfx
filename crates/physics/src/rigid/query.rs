use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::{Collider, ColliderHandle, Group, QueryFilter, QueryPipeline, Ray};

use super::convert::{body_handle, body_id, collider_id, iso, vec};
use super::{BodyId, ColliderId, Pose, Shape, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub mask: u32,
    pub sensors: bool,
    pub exclude: Option<BodyId>,
}

impl Filter {
    pub const ALL: Self = Self {
        mask: u32::MAX,
        sensors: false,
        exclude: None,
    };

    pub fn layers(mask: u32) -> Self {
        Self { mask, ..Self::ALL }
    }

    pub fn with_sensors(mut self) -> Self {
        self.sensors = true;
        self
    }

    pub fn excluding(mut self, body: BodyId) -> Self {
        self.exclude = Some(body);
        self
    }

    fn with_pipeline<R>(self, world: &World, run: impl FnOnce(&QueryPipeline<'_>) -> R) -> R {
        let mask = Group::from_bits_retain(self.mask);
        let by_layer = move |_: ColliderHandle, collider: &Collider| {
            collider.collision_groups().memberships.intersects(mask)
        };
        let mut filter = QueryFilter::new().predicate(&by_layer);
        if !self.sensors {
            filter = filter.exclude_sensors();
        }
        if let Some(body) = self.exclude {
            filter = filter.exclude_rigid_body(body_handle(body));
        }
        let physics = &world.physics;
        let pipeline = world.queries.as_query_pipeline(
            physics.narrow_phase.query_dispatcher(),
            &physics.bodies,
            &physics.colliders,
            filter,
        );
        run(&pipeline)
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::ALL
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub collider: ColliderId,
    pub body: Option<BodyId>,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

impl World {
    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        filter: Filter,
    ) -> Option<Hit> {
        let direction = vec(direction).normalize_or_zero();
        if direction == super::convert::Vec3::ZERO {
            return None;
        }
        let ray = Ray::new(vec(origin), direction);
        let physics = &self.physics;
        let (handle, hit) = filter.with_pipeline(self, |pipeline| {
            pipeline.cast_ray_and_get_normal(&ray, max_distance, true)
        })?;
        Some(Hit {
            collider: collider_id(handle),
            body: physics
                .colliders
                .get(handle)
                .and_then(|c| c.parent())
                .map(body_id),
            distance: hit.time_of_impact,
            point: ray.point_at(hit.time_of_impact).to_array(),
            normal: hit.normal.to_array(),
        })
    }

    pub fn shape_cast(
        &self,
        shape: &Shape,
        pose: Pose,
        direction: [f32; 3],
        max_distance: f32,
        filter: Filter,
    ) -> Option<Hit> {
        let shared = shape.shared()?;
        let direction = vec(direction).normalize_or_zero();
        if direction == super::convert::Vec3::ZERO {
            return None;
        }
        let start = iso(pose);
        let physics = &self.physics;
        let (handle, hit) = filter.with_pipeline(self, |pipeline| {
            pipeline.cast_shape(
                &start,
                direction,
                shared.as_ref(),
                ShapeCastOptions::with_max_time_of_impact(max_distance),
            )
        })?;
        let mut at = start;
        at.translation += direction * hit.time_of_impact;
        Some(Hit {
            collider: collider_id(handle),
            body: physics
                .colliders
                .get(handle)
                .and_then(|c| c.parent())
                .map(body_id),
            distance: hit.time_of_impact,
            point: at.transform_point(hit.witness2).to_array(),
            normal: hit.normal1.to_array(),
        })
    }

    pub fn point_overlaps(&self, point: [f32; 3], filter: Filter) -> Vec<ColliderId> {
        let mut found: Vec<ColliderId> = filter.with_pipeline(self, |pipeline| {
            pipeline
                .intersect_point(vec(point))
                .map(|(handle, _)| collider_id(handle))
                .collect()
        });
        found.sort();
        found
    }

    pub fn shape_overlaps(&self, shape: &Shape, pose: Pose, filter: Filter) -> Vec<ColliderId> {
        let Some(shared) = shape.shared() else {
            return Vec::new();
        };
        let mut found: Vec<ColliderId> = filter.with_pipeline(self, |pipeline| {
            pipeline
                .intersect_shape(iso(pose), shared.as_ref())
                .map(|(handle, _)| collider_id(handle))
                .collect()
        });
        found.sort();
        found
    }
}
