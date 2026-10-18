use glam::DVec3;

use crate::scatter::{Ground, triangles};
use crate::{Built, Error, Part, world};
use pfx_recipe::Recipe;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub low: DVec3,
    pub high: DVec3,
}

impl Bounds {
    pub fn center(&self) -> DVec3 {
        (self.low + self.high) / 2.0
    }

    fn edge(&self, axis: usize, edge: Edge) -> f64 {
        match edge {
            Edge::Min => self.low[axis],
            Edge::Center => self.center()[axis],
            Edge::Max => self.high[axis],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Min,
    Center,
    Max,
}

pub fn bounds(parts: &[Part], object: usize) -> Option<Bounds> {
    let mut low = DVec3::INFINITY;
    let mut high = DVec3::NEG_INFINITY;
    for tri in triangles(parts, object) {
        for p in tri {
            low = low.min(p);
            high = high.max(p);
        }
    }
    (low.is_finite() && high.is_finite()).then_some(Bounds { low, high })
}

fn along(axis: usize, amount: f64) -> DVec3 {
    let mut delta = DVec3::ZERO;
    delta[axis] = amount;
    delta
}

pub fn align(boxes: &[Bounds], axis: usize, edge: Edge, to: Option<Bounds>) -> Vec<DVec3> {
    let goal = match (to, edge) {
        (Some(target), _) => target.edge(axis, edge),
        (None, Edge::Min) => boxes
            .iter()
            .map(|b| b.low[axis])
            .fold(f64::INFINITY, f64::min),
        (None, Edge::Max) => boxes
            .iter()
            .map(|b| b.high[axis])
            .fold(f64::NEG_INFINITY, f64::max),
        (None, Edge::Center) => {
            boxes.iter().map(|b| b.center()[axis]).sum::<f64>() / boxes.len().max(1) as f64
        }
    };
    boxes
        .iter()
        .map(|b| along(axis, goal - b.edge(axis, edge)))
        .collect()
}

pub fn distribute(boxes: &[Bounds], axis: usize) -> Vec<DVec3> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| {
        boxes[a].center()[axis]
            .total_cmp(&boxes[b].center()[axis])
            .then(a.cmp(&b))
    });
    let mut deltas = vec![DVec3::ZERO; boxes.len()];
    if boxes.len() < 3 {
        return deltas;
    }
    let first = boxes[order[0]].center()[axis];
    let last = boxes[order[order.len() - 1]].center()[axis];
    let step = (last - first) / (order.len() - 1) as f64;
    for (rank, &at) in order.iter().enumerate() {
        deltas[at] = along(axis, first + step * rank as f64 - boxes[at].center()[axis]);
    }
    deltas
}

pub fn snap(boxes: &[Bounds], target: Bounds, axis: usize, edge: Edge) -> Vec<DVec3> {
    boxes
        .iter()
        .map(|b| {
            let amount = match edge {
                Edge::Max => target.high[axis] - b.low[axis],
                Edge::Min => target.low[axis] - b.high[axis],
                Edge::Center => target.center()[axis] - b.center()[axis],
            };
            along(axis, amount)
        })
        .collect()
}

pub fn ground(boxes: &[Bounds], surface: Option<&Ground>) -> Vec<DVec3> {
    boxes
        .iter()
        .map(|b| {
            let bottom = b.low.y;
            let rest = surface
                .and_then(|surface| {
                    let xs = [b.low.x, b.center().x, b.high.x];
                    let zs = [b.low.z, b.center().z, b.high.z];
                    xs.iter()
                        .flat_map(|&x| zs.iter().map(move |&z| (x, z)))
                        .filter_map(|(x, z)| surface.under(x, z, bottom + 1e-9))
                        .map(|hit| hit.at.y)
                        .reduce(f64::max)
                })
                .unwrap_or(0.0);
            DVec3::new(0.0, rest - bottom, 0.0)
        })
        .collect()
}

pub fn surface(built: &Built, objects: &[usize]) -> Option<Ground> {
    let mut tris = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for part in built.parts.iter().filter(|p| !p.hidden) {
        if objects.contains(&part.object) && seen.insert(part.object) {
            tris.extend(triangles(&built.parts, part.object));
        }
    }
    Ground::new(tris)
}

pub fn moved(recipe: &Recipe, index: usize, delta: DVec3) -> Result<[f64; 3], Error> {
    let object = &recipe.objects[index];
    let local = match &object.parent {
        None => delta,
        Some(parent) => {
            let at = recipe
                .objects
                .iter()
                .rposition(|o| o.name == *parent)
                .ok_or_else(|| {
                    Error::new(
                        format!("object[{}].parent", object.name),
                        format!("no object is named `{parent}`"),
                    )
                })?;
            world(recipe, at)?.inverse().transform_vector3(delta)
        }
    };
    Ok((DVec3::from(object.at) + local).to_array())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(x: f64, y: f64, z: f64, half: f64) -> Bounds {
        Bounds {
            low: DVec3::new(x - half, y - half, z - half),
            high: DVec3::new(x + half, y + half, z + half),
        }
    }

    #[test]
    fn align_moves_each_edge_to_the_selection_s_or_a_target_s() {
        let boxes = [cube(0.0, 0.0, 0.0, 1.0), cube(5.0, 3.0, 0.0, 0.5)];
        assert_eq!(
            align(&boxes, 1, Edge::Min, None),
            [DVec3::ZERO, DVec3::new(0.0, -3.0 + 0.5 - 1.0, 0.0)]
        );
        assert_eq!(
            align(&boxes, 0, Edge::Center, None),
            [DVec3::new(2.5, 0.0, 0.0), DVec3::new(-2.5, 0.0, 0.0)]
        );
        let target = cube(0.0, 10.0, 0.0, 2.0);
        assert_eq!(
            align(&boxes, 1, Edge::Max, Some(target)),
            [DVec3::new(0.0, 11.0, 0.0), DVec3::new(0.0, 8.5, 0.0)]
        );
    }

    #[test]
    fn distribute_spaces_the_centres_evenly_between_the_outermost() {
        let boxes = [
            cube(0.0, 0.0, 0.0, 1.0),
            cube(9.0, 0.0, 0.0, 1.0),
            cube(1.0, 0.0, 0.0, 0.2),
            cube(2.0, 0.0, 0.0, 0.2),
        ];
        let deltas = distribute(&boxes, 0);
        let centres: Vec<f64> = boxes
            .iter()
            .zip(&deltas)
            .map(|(b, d)| b.center().x + d.x)
            .collect();
        assert_eq!(centres, [0.0, 9.0, 3.0, 6.0]);
        assert_eq!(distribute(&boxes[..2], 0), [DVec3::ZERO; 2]);
    }

    #[test]
    fn snap_sets_one_box_against_another_s_side() {
        let table = Bounds {
            low: DVec3::new(-1.0, 0.0, -1.0),
            high: DVec3::new(1.0, 0.75, 1.0),
        };
        let cup = cube(0.25, 2.0, 0.0, 0.125);
        assert_eq!(
            snap(&[cup], table, 1, Edge::Max),
            [DVec3::new(0.0, -1.125, 0.0)]
        );
        assert_eq!(
            snap(&[cup], table, 0, Edge::Min),
            [DVec3::new(-1.375, 0.0, 0.0)]
        );
        assert_eq!(snap(&[cup], table, 2, Edge::Center), [DVec3::ZERO]);
    }

    #[test]
    fn ground_rests_each_box_on_the_highest_surface_below_it_or_the_floor() {
        let step = Ground::new(vec![
            [
                DVec3::new(0.0, 1.0, -1.0),
                DVec3::new(2.0, 1.0, -1.0),
                DVec3::new(2.0, 1.0, 1.0),
            ],
            [
                DVec3::new(0.0, 1.0, -1.0),
                DVec3::new(2.0, 1.0, 1.0),
                DVec3::new(0.0, 1.0, 1.0),
            ],
        ])
        .unwrap();
        let above = cube(1.0, 4.0, 0.0, 0.5);
        let half_over = cube(-0.2, 4.0, 0.0, 0.5);
        let beside = cube(-5.0, 4.0, 0.0, 0.5);
        let under = cube(1.0, 0.5, 0.0, 0.25);
        assert_eq!(
            ground(&[above, half_over, beside, under], Some(&step)),
            [
                DVec3::new(0.0, -2.5, 0.0),
                DVec3::new(0.0, -2.5, 0.0),
                DVec3::new(0.0, -3.5, 0.0),
                DVec3::new(0.0, -0.25, 0.0),
            ]
        );
        assert_eq!(ground(&[above], None), [DVec3::new(0.0, -3.5, 0.0)]);
    }
}
