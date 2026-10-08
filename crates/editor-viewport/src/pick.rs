use std::collections::BTreeMap;

use glam::{DMat4, DVec3};
use pfx_load::scene::{Draws, Matrix, Scene};

use crate::camera::{Bounds, EMPTY};
use crate::lens::{Ray, slab, triangle};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub index: usize,
    pub id: u32,
    pub name: String,
}

impl Selection {
    pub fn of(scene: &Scene, index: usize) -> Option<Selection> {
        let object = scene.objects.get(index)?;
        Some(Selection {
            index,
            id: object.id,
            name: object.name.clone(),
        })
    }

    pub fn named(scene: &Scene, name: &str) -> Option<Selection> {
        let index = scene
            .objects
            .iter()
            .position(|object| object.name == name)?;
        Selection::of(scene, index)
    }

    pub fn by_id(scene: &Scene, id: u32) -> Option<Selection> {
        if id == 0 {
            return None;
        }
        let index = scene.objects.iter().position(|object| object.id == id)?;
        Selection::of(scene, index)
    }
}

pub fn matrix(model: Matrix) -> DMat4 {
    DMat4::from_cols_array_2d(&model.map(|column| column.map(f64::from)))
}

fn corners(low: DVec3, high: DVec3) -> [DVec3; 8] {
    std::array::from_fn(|k| {
        DVec3::new(
            if k & 1 == 0 { low.x } else { high.x },
            if k & 2 == 0 { low.y } else { high.y },
            if k & 4 == 0 { low.z } else { high.z },
        )
    })
}

pub const EDGES: [[usize; 2]; 12] = [
    [0, 1],
    [2, 3],
    [4, 5],
    [6, 7],
    [0, 2],
    [1, 3],
    [4, 6],
    [5, 7],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

#[derive(Clone, Debug)]
struct Part {
    object: Option<usize>,
    model: DMat4,
    inverse: DMat4,
    low: DVec3,
    high: DVec3,
    drawn: bool,
}

pub type Extents = BTreeMap<[u8; 32], (DVec3, DVec3)>;

#[derive(Clone, Debug)]
pub struct Picker {
    draws: Draws,
    parts: Vec<Part>,
    parents: Vec<Option<usize>>,
    bounds: Bounds,
}

fn local_bounds(positions: &[[f32; 3]]) -> (DVec3, DVec3) {
    let mut low = DVec3::INFINITY;
    let mut high = DVec3::NEG_INFINITY;
    for p in positions {
        let p = DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
        low = low.min(p);
        high = high.max(p);
    }
    if low.is_finite() && high.is_finite() {
        (low, high)
    } else {
        (DVec3::ZERO, DVec3::ZERO)
    }
}

impl Picker {
    pub fn of(scene: &Scene, extents: &mut Extents) -> Picker {
        let names: BTreeMap<&str, usize> = scene
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| (object.name.as_str(), index))
            .collect();
        let draws = scene.draws();
        let parts: Vec<Part> = draws
            .items
            .iter()
            .map(|draw| {
                let (low, high) = *extents
                    .entry(draw.geometry.hash)
                    .or_insert_with(|| local_bounds(&draw.geometry.positions));
                let model = matrix(draw.model);
                Part {
                    object: names.get(draw.object.as_str()).copied(),
                    inverse: model.inverse(),
                    model,
                    low,
                    high,
                    drawn: draw.shadow.drawn(),
                }
            })
            .collect();
        let mut low = DVec3::INFINITY;
        let mut high = DVec3::NEG_INFINITY;
        for part in parts.iter().filter(|part| part.drawn) {
            for corner in corners(part.low, part.high) {
                let at = part.model.transform_point3(corner);
                low = low.min(at);
                high = high.max(at);
            }
        }
        let bounds = if low.is_finite() && high.is_finite() {
            [low.to_array(), high.to_array()]
        } else {
            EMPTY
        };
        let parents = scene
            .objects
            .iter()
            .map(|object| {
                object
                    .parent
                    .as_deref()
                    .and_then(|parent| names.get(parent).copied())
            })
            .collect();
        Picker {
            draws,
            parts,
            parents,
            bounds,
        }
    }

    pub fn draws(&self) -> &Draws {
        &self.draws
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    pub fn family(&self, object: usize) -> Vec<usize> {
        let mut found = vec![object];
        let mut index = 0;
        while index < found.len() {
            let parent = found[index];
            for (child, above) in self.parents.iter().enumerate() {
                if *above == Some(parent) && !found.contains(&child) {
                    found.push(child);
                }
            }
            index += 1;
        }
        found
    }

    fn holds(&self, object: usize) -> impl Fn(&&Part) -> bool {
        let family = self.family(object);
        move |part| part.object.is_some_and(|owner| family.contains(&owner))
    }

    pub fn bounds_of(&self, object: usize) -> Option<Bounds> {
        let mut low = DVec3::INFINITY;
        let mut high = DVec3::NEG_INFINITY;
        for part in self.parts.iter().filter(self.holds(object)) {
            for corner in corners(part.low, part.high) {
                let at = part.model.transform_point3(corner);
                low = low.min(at);
                high = high.max(at);
            }
        }
        (low.is_finite() && high.is_finite()).then(|| [low.to_array(), high.to_array()])
    }

    pub fn outline(&self, object: usize) -> Vec<[DVec3; 8]> {
        self.parts
            .iter()
            .filter(self.holds(object))
            .filter(|part| part.drawn)
            .map(|part| corners(part.low, part.high).map(|at| part.model.transform_point3(at)))
            .collect()
    }

    pub fn cast(&self, ray: &Ray) -> Option<usize> {
        let mut nearest: Option<(f64, usize)> = None;
        for (part, draw) in self.parts.iter().zip(&self.draws.items) {
            let (true, Some(object)) = (part.drawn, part.object) else {
                continue;
            };
            let local = Ray {
                origin: part.inverse.transform_point3(ray.origin),
                direction: part.inverse.transform_vector3(ray.direction),
            };
            let Some(entry) = slab(&local, part.low, part.high) else {
                continue;
            };
            if nearest.is_some_and(|(best, _)| entry > best) {
                continue;
            }
            let geometry = &draw.geometry;
            let corner = |index: u32| {
                let p = geometry.positions[index as usize];
                DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
            };
            for face in geometry.indices.chunks_exact(3) {
                if let Some(distance) =
                    triangle(&local, [corner(face[0]), corner(face[1]), corner(face[2])])
                    && nearest.is_none_or(|(best, _)| distance < best)
                {
                    nearest = Some((distance, object));
                }
            }
        }
        nearest.map(|(_, object)| object)
    }
}
