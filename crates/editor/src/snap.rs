use std::collections::BTreeMap;

use egui::Pos2;
use glam::{DMat3, DMat4, DQuat, DVec3};
use pfx_editor_viewport::gizmo::euler;
use pfx_editor_viewport::lens::{slab, triangle};
use pfx_editor_viewport::pick::matrix;
use pfx_editor_viewport::{Lens, Ray};
use pfx_load::scene::Scene;

pub const STEP: f64 = 0.5;
pub const LEAST_STEP: f64 = 0.01;
pub const MOST_STEP: f64 = 100.0;
pub const VERTEX_POINTS: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapping {
    pub grid: bool,
    pub step: f64,
    pub surface: bool,
    pub align: bool,
    pub vertex: bool,
}

impl Default for Snapping {
    fn default() -> Snapping {
        Snapping {
            grid: false,
            step: STEP,
            surface: true,
            align: false,
            vertex: false,
        }
    }
}

impl Snapping {
    pub fn set_step(&mut self, step: f64) {
        if step.is_finite() {
            self.step = step.clamp(LEAST_STEP, MOST_STEP);
        }
    }

    pub fn active(&self) -> bool {
        self.grid || self.vertex || self.align || !self.surface
    }
}

pub fn grid(point: DVec3, step: f64, axes: [bool; 3]) -> DVec3 {
    DVec3::from_array(std::array::from_fn(|k| {
        if axes[k] && step > 0.0 {
            (point[k] / step).round() * step
        } else {
            point[k]
        }
    }))
}

pub fn across(normal: DVec3) -> [bool; 3] {
    std::array::from_fn(|k| normal[k].abs() < 0.9)
}

pub fn upright(normal: DVec3) -> DVec3 {
    let Some(normal) = normal.try_normalize() else {
        return DVec3::ZERO;
    };
    let turn = DQuat::from_rotation_arc(DVec3::Y, normal);
    let degrees = euler(DMat3::from_quat(turn));
    DVec3::from_array(degrees.to_array().map(|value| {
        let rounded = (value * 1e6).round() / 1e6;
        if rounded == 0.0 { 0.0 } else { rounded }
    }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub object: usize,
    pub point: DVec3,
    pub normal: DVec3,
    pub distance: f64,
}

#[derive(Clone, Debug)]
struct Part {
    object: usize,
    draw: usize,
    model: DMat4,
    inverse: DMat4,
    low: DVec3,
    high: DVec3,
}

#[derive(Clone, Debug)]
pub struct Surfaces {
    draws: pfx_load::scene::Draws,
    parts: Vec<Part>,
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

pub fn corners(low: DVec3, high: DVec3) -> [DVec3; 8] {
    std::array::from_fn(|k| {
        DVec3::new(
            if k & 1 == 0 { low.x } else { high.x },
            if k & 2 == 0 { low.y } else { high.y },
            if k & 4 == 0 { low.z } else { high.z },
        )
    })
}

impl Surfaces {
    pub fn of(scene: &Scene) -> Surfaces {
        let names: BTreeMap<&str, usize> = scene
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| (object.name.as_str(), index))
            .collect();
        let draws = scene.draws();
        let mut extents: BTreeMap<[u8; 32], (DVec3, DVec3)> = BTreeMap::new();
        let parts = draws
            .items
            .iter()
            .enumerate()
            .filter(|(_, draw)| draw.shadow.drawn())
            .filter_map(|(index, draw)| {
                let object = *names.get(draw.object.as_str())?;
                let (low, high) = *extents
                    .entry(draw.geometry.hash)
                    .or_insert_with(|| local_bounds(&draw.geometry.positions));
                let model = matrix(draw.model);
                Some(Part {
                    object,
                    draw: index,
                    inverse: model.inverse(),
                    model,
                    low,
                    high,
                })
            })
            .collect();
        Surfaces { draws, parts }
    }

    pub fn cast(&self, ray: &Ray) -> Option<Hit> {
        self.cast_except(ray, &[])
    }

    pub fn cast_except(&self, ray: &Ray, skip: &[usize]) -> Option<Hit> {
        let mut nearest: Option<Hit> = None;
        for part in &self.parts {
            if skip.contains(&part.object) {
                continue;
            }
            let local = Ray {
                origin: part.inverse.transform_point3(ray.origin),
                direction: part.inverse.transform_vector3(ray.direction),
            };
            let Some(entry) = slab(&local, part.low, part.high) else {
                continue;
            };
            if nearest.is_some_and(|best| entry > best.distance) {
                continue;
            }
            let geometry = &self.draws.items[part.draw].geometry;
            let corner = |index: u32| {
                let p = geometry.positions[index as usize];
                DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
            };
            for face in geometry.indices.chunks_exact(3) {
                let three = [corner(face[0]), corner(face[1]), corner(face[2])];
                let Some(distance) = triangle(&local, three) else {
                    continue;
                };
                if nearest.is_some_and(|best| distance >= best.distance) {
                    continue;
                }
                let face_normal = (three[1] - three[0]).cross(three[2] - three[0]);
                let world = part.inverse.transpose().transform_vector3(face_normal);
                let Some(mut normal) = world.try_normalize() else {
                    continue;
                };
                if normal.dot(ray.direction) > 0.0 {
                    normal = -normal;
                }
                nearest = Some(Hit {
                    object: part.object,
                    point: part.model.transform_point3(local.at(distance)),
                    normal,
                    distance,
                });
            }
        }
        nearest
    }

    pub fn vertex(&self, lens: &Lens, at: Pos2, reach: f32, skip: &[usize]) -> Option<DVec3> {
        let mut best: Option<(f32, DVec3)> = None;
        for part in &self.parts {
            if skip.contains(&part.object) {
                continue;
            }
            let projected: Vec<Pos2> = corners(part.low, part.high)
                .iter()
                .filter_map(|corner| lens.project(part.model.transform_point3(*corner)))
                .collect();
            if projected.len() < 8 {
                continue;
            }
            let rect = egui::Rect::from_points(&projected).expand(reach);
            if !rect.contains(at) {
                continue;
            }
            let geometry = &self.draws.items[part.draw].geometry;
            for p in &geometry.positions {
                let world = part.model.transform_point3(DVec3::new(
                    f64::from(p[0]),
                    f64::from(p[1]),
                    f64::from(p[2]),
                ));
                let Some(seen) = lens.project(world) else {
                    continue;
                };
                let away = seen.distance(at);
                if away <= reach && best.is_none_or(|(near, _)| away < near) {
                    best = Some((away, world));
                }
            }
        }
        best.map(|(_, point)| point)
    }

    pub fn boxes(&self, object: usize) -> Vec<[DVec3; 8]> {
        self.parts
            .iter()
            .filter(|part| part.object == object)
            .map(|part| corners(part.low, part.high).map(|at| part.model.transform_point3(at)))
            .collect()
    }
}

pub fn ground(ray: &Ray) -> Option<DVec3> {
    if ray.direction.y.abs() < 1e-9 {
        return None;
    }
    let distance = -ray.origin.y / ray.direction.y;
    (distance > 0.0).then(|| ray.at(distance))
}

pub fn plane(ray: &Ray, origin: DVec3, normal: DVec3) -> Option<DVec3> {
    let facing = ray.direction.dot(normal);
    if facing.abs() < 1e-9 {
        return None;
    }
    let distance = (origin - ray.origin).dot(normal) / facing;
    (distance > 0.0).then(|| ray.at(distance))
}

pub fn lift(bounds: [[f32; 3]; 2], rotate: DVec3, normal: DVec3) -> DVec3 {
    let turn = pfx_editor_viewport::gizmo::rotation(rotate);
    let low = DVec3::from_array(bounds[0].map(f64::from));
    let high = DVec3::from_array(bounds[1].map(f64::from));
    let below = corners(low, high)
        .iter()
        .map(|corner| -turn.transform_point3(*corner).dot(normal))
        .fold(f64::NEG_INFINITY, f64::max);
    if below.is_finite() {
        normal * below
    } else {
        DVec3::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_snapping_rounds_only_the_axes_asked() {
        let point = DVec3::new(1.26, 0.74, -0.24);
        assert_eq!(grid(point, 0.5, [true; 3]), DVec3::new(1.5, 0.5, -0.0));
        assert_eq!(
            grid(point, 0.5, across(DVec3::Y)),
            DVec3::new(1.5, 0.74, -0.0)
        );
        assert_eq!(grid(point, 0.0, [true; 3]), point);
        let mut snapping = Snapping::default();
        snapping.set_step(0.0);
        assert_eq!(snapping.step, LEAST_STEP);
        snapping.set_step(f64::NAN);
        assert_eq!(snapping.step, LEAST_STEP);
        assert!(!Snapping::default().active());
    }

    #[test]
    fn upright_turns_y_onto_the_normal() {
        assert_eq!(upright(DVec3::Y), DVec3::ZERO);
        let turned = upright(DVec3::X);
        let up = pfx_editor_viewport::gizmo::rotation(turned).transform_vector3(DVec3::Y);
        assert!((up - DVec3::X).length() < 1e-9, "{turned} {up}");
        let slope = DVec3::new(0.0, 1.0, 1.0).normalize();
        let up = pfx_editor_viewport::gizmo::rotation(upright(slope)).transform_vector3(DVec3::Y);
        assert!((up - slope).length() < 1e-6);
    }

    #[test]
    fn the_ground_and_a_plane_meet_a_ray() {
        let ray = Ray {
            origin: DVec3::new(0.0, 2.0, 0.0),
            direction: DVec3::new(0.0, -1.0, 1.0).normalize(),
        };
        let hit = ground(&ray).unwrap();
        assert!((hit - DVec3::new(0.0, 0.0, 2.0)).length() < 1e-9);
        let up = Ray {
            direction: DVec3::Y,
            ..ray
        };
        assert!(ground(&up).is_none());
        let wall = plane(&ray, DVec3::new(0.0, 0.0, 1.0), DVec3::Z).unwrap();
        assert!((wall - DVec3::new(0.0, 1.0, 1.0)).length() < 1e-9);
    }

    #[test]
    fn lift_sets_a_box_on_the_surface() {
        let unit = [[-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]];
        let up = lift(unit, DVec3::ZERO, DVec3::Y);
        assert!((up - DVec3::new(0.0, 0.5, 0.0)).length() < 1e-9);
        let standing = [[-0.5, 0.0, -0.5], [0.5, 1.0, 0.5]];
        assert!(lift(standing, DVec3::ZERO, DVec3::Y).length() < 1e-9);
        let side = lift(standing, upright(DVec3::X), DVec3::X);
        assert!(side.length() < 1e-9, "{side}");
    }
}
