use pfx_load::scene::BodyShape;
use pfx_physics::rigid::{self, BodyId, ColliderId, Filter, Pose};

use super::{ObjectId, World, shape};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    layers: Option<Vec<String>>,
    mask: Option<u32>,
    triggers: bool,
    exclude: Option<String>,
}

impl Query {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn layers<S: AsRef<str>>(names: &[S]) -> Self {
        Self::all().on(names)
    }

    pub fn on<S: AsRef<str>>(mut self, names: &[S]) -> Self {
        self.layers = Some(names.iter().map(|name| name.as_ref().to_string()).collect());
        self.mask = None;
        self
    }

    pub fn mask(mut self, mask: u32) -> Self {
        self.mask = Some(mask);
        self.layers = None;
        self
    }

    pub fn with_triggers(mut self) -> Self {
        self.triggers = true;
        self
    }

    pub fn excluding(mut self, object: &str) -> Self {
        self.exclude = Some(object.to_string());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    Layer(String),
    Object(String),
    Shape,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layer(message) => f.write_str(message),
            Self::Object(name) => write!(f, "query: {name} is not an object of the scene"),
            Self::Shape => f.write_str("query: the shape needs sizes above 0"),
        }
    }
}

impl std::error::Error for QueryError {}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub object: Option<ObjectId>,
    pub name: Option<String>,
    pub trigger: bool,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlap {
    pub object: ObjectId,
    pub name: String,
    pub trigger: bool,
}

fn usable(body: BodyShape) -> Result<rigid::Shape, QueryError> {
    let ok = |size: f32| size.is_finite() && size > 0.0;
    let fits = match body {
        BodyShape::Box { half } => half.into_iter().all(ok),
        BodyShape::Sphere { radius } => ok(radius),
        BodyShape::Capsule {
            half_height,
            radius,
        }
        | BodyShape::Cylinder {
            half_height,
            radius,
        } => ok(half_height) && ok(radius),
    };
    if fits {
        Ok(shape(body))
    } else {
        Err(QueryError::Shape)
    }
}

impl World {
    fn filter(&self, query: &Query) -> Result<Filter, QueryError> {
        let mask = match (&query.layers, query.mask) {
            (Some(names), _) => self.layers.mask(names).map_err(QueryError::Layer)?,
            (None, Some(mask)) => mask,
            (None, None) => u32::MAX,
        };
        let mut filter = Filter::layers(mask);
        if query.triggers {
            filter = filter.with_sensors();
        }
        if let Some(name) = &query.exclude {
            let place = *self
                .index
                .get(name)
                .ok_or_else(|| QueryError::Object(name.clone()))?;
            if let Some(body) = self.body_of_place(place) {
                filter = filter.excluding(body);
            }
        }
        Ok(filter)
    }

    fn body_of_place(&self, place: usize) -> Option<BodyId> {
        self.bodies
            .get(&place)
            .map(|physical| physical.id)
            .or_else(|| self.sensors.get(&place).map(|sensor| sensor.body))
    }

    fn owner(&self, collider: ColliderId) -> Option<(usize, bool)> {
        self.owners
            .get(&collider)
            .map(|place| (*place, false))
            .or_else(|| {
                self.sensor_owners
                    .get(&collider)
                    .map(|place| (*place, true))
            })
    }

    fn hit(&self, hit: rigid::Hit) -> Hit {
        let owner = self.owner(hit.collider);
        Hit {
            object: owner.map(|(place, _)| ObjectId(place)),
            name: owner.map(|(place, _)| self.objects[place].name.clone()),
            trigger: owner.is_some_and(|(_, trigger)| trigger),
            distance: hit.distance,
            point: hit.point,
            normal: hit.normal,
        }
    }

    pub fn raycast(
        &self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        query: &Query,
    ) -> Result<Option<Hit>, QueryError> {
        let filter = self.filter(query)?;
        Ok(self
            .physics
            .raycast(origin, direction, max_distance, filter)
            .map(|hit| self.hit(hit)))
    }

    pub fn shape_cast(
        &self,
        shape_of: BodyShape,
        at: [f32; 3],
        rotation: [f32; 4],
        direction: [f32; 3],
        max_distance: f32,
        query: &Query,
    ) -> Result<Option<Hit>, QueryError> {
        let filter = self.filter(query)?;
        let shape = usable(shape_of)?;
        Ok(self
            .physics
            .shape_cast(
                &shape,
                Pose::new(at, rotation),
                direction,
                max_distance,
                filter,
            )
            .map(|hit| self.hit(hit)))
    }

    pub fn overlaps(
        &self,
        shape_of: BodyShape,
        at: [f32; 3],
        rotation: [f32; 4],
        query: &Query,
    ) -> Result<Vec<Overlap>, QueryError> {
        let filter = self.filter(query)?;
        let shape = usable(shape_of)?;
        Ok(self.overlaps_of(
            self.physics
                .shape_overlaps(&shape, Pose::new(at, rotation), filter),
        ))
    }

    pub fn overlaps_point(
        &self,
        point: [f32; 3],
        query: &Query,
    ) -> Result<Vec<Overlap>, QueryError> {
        let filter = self.filter(query)?;
        Ok(self.overlaps_of(self.physics.point_overlaps(point, filter)))
    }

    fn overlaps_of(&self, colliders: Vec<ColliderId>) -> Vec<Overlap> {
        let mut found: Vec<(usize, bool)> = colliders
            .into_iter()
            .filter_map(|collider| self.owner(collider))
            .collect();
        found.sort();
        found.dedup();
        found
            .into_iter()
            .map(|(place, trigger)| Overlap {
                object: ObjectId(place),
                name: self.objects[place].name.clone(),
                trigger,
            })
            .collect()
    }
}
