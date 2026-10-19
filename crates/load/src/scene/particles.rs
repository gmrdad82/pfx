use std::collections::BTreeMap;

use pfx_core::sim::mix64;
use pfx_core::sim::particles::{Blend, Gradient, Lerp, Recipe, RecipeKeys};
use pfx_scene::Entry;
use pfx_scene::types::{self as file, KeyValue};

use super::build::{Places, Step, inner};
use super::{Matrix, Object, Scene, SceneError, SceneMesh, multiply};

#[derive(Clone, Debug, PartialEq)]
pub struct Particles {
    pub attach: Option<Attach>,
    pub at: [f32; 3],
    pub recipe: Recipe,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attach {
    pub object: String,
    pub node: Option<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub joint: Option<usize>,
    pub transform: Matrix,
}

fn seed_of(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        mix64(hash ^ u64::from(byte))
    })
}

fn colour(color: &file::Color) -> [f32; 3] {
    color.rgb().unwrap_or([1.0; 3])
}

fn track<T: Lerp>(
    value: &file::Particles,
    name: &str,
    pick: impl Fn(&KeyValue) -> Option<T>,
    fallback: T,
) -> Option<Gradient<T>> {
    let keys: Vec<(f32, T)> = value
        .track(name)?
        .iter()
        .filter_map(|(t, key)| Some((*t, pick(key)?)))
        .collect();
    (!keys.is_empty()).then(|| Gradient::new(keys, fallback))
}

fn keys(value: &file::Particles) -> RecipeKeys {
    RecipeKeys {
        rate: track(value, "rate", KeyValue::number, 0.0),
        speed: track(value, "speed", KeyValue::pair, [0.0; 2]),
        size: track(value, "size", KeyValue::pair, [0.0; 2]),
        colour: track(value, "color", KeyValue::triple, [1.0; 3]),
        alpha: track(value, "alpha", KeyValue::number, 1.0),
    }
}

pub fn recipe(name: &str, value: &file::Particles) -> Recipe {
    let base = Recipe::default();
    let colour = match (&value.colors, &value.color) {
        (Some(keys), _) if !keys.is_empty() => Gradient::new(
            keys.iter().map(|(t, c)| (*t, colour(c))).collect(),
            [1.0; 3],
        ),
        (_, Some(color)) => Gradient::constant(colour(color)),
        _ => base.colour.clone(),
    };
    Recipe {
        rate: value.rate.unwrap_or(base.rate),
        keys: keys(value),
        burst: value.burst.unwrap_or(base.burst),
        max: value.max.unwrap_or(base.max),
        lifetime: value.lifetime.unwrap_or(base.lifetime),
        speed: value.speed.unwrap_or(base.speed),
        direction: value.direction.unwrap_or(base.direction),
        cone: value.cone.unwrap_or(base.cone),
        inherit: value.inherit.unwrap_or(base.inherit),
        gravity: value.gravity.unwrap_or(base.gravity),
        drag: value.drag.unwrap_or(base.drag),
        size: value.size.unwrap_or(base.size),
        alpha: match &value.alpha {
            Some(keys) if !keys.is_empty() => {
                Gradient::new(keys.iter().map(|[t, a]| (*t, *a)).collect(), 1.0)
            }
            _ => base.alpha.clone(),
        },
        colour,
        spin: value.spin.unwrap_or(base.spin),
        stretch: value.stretch.unwrap_or(base.stretch),
        blend: match value.blend.unwrap_or_default() {
            file::ParticleBlend::Alpha => Blend::Alpha,
            file::ParticleBlend::Add => Blend::Add,
        },
        lit: value.lit.unwrap_or(base.lit),
        seed: value.seed.unwrap_or_else(|| seed_of(name)),
        prewarm: value.prewarm.unwrap_or(base.prewarm),
    }
}

pub(super) fn build(
    places: &Places,
    entries: &[Entry<file::Particles>],
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<BTreeMap<String, Particles>, SceneError> {
    let mut out = BTreeMap::new();
    for entry in entries {
        let name = entry.key.as_str();
        let value = &entry.value;
        let at = |key| [Step::Named("particles", inner(name)), Step::Key(key)];
        let attach = match &value.parent {
            None if value.node.is_some() => {
                return places.error(
                    &entry.file,
                    &at("node"),
                    format!("particles {name}: node needs a parent object"),
                );
            }
            None => None,
            Some(parent) => {
                let Some(object) = objects.iter().find(|object| &object.name == parent) else {
                    return places.error(
                        &entry.file,
                        &at("parent"),
                        format!("particles {name}: parent {parent} is no object of this scene"),
                    );
                };
                let node = match &value.node {
                    None => None,
                    Some(node) => {
                        let part = meshes
                            .get(&object.mesh)
                            .and_then(|mesh| mesh.parts.iter().find(|part| &part.node == node));
                        let Some(part) = part else {
                            return places.error(
                                &entry.file,
                                &at("node"),
                                format!(
                                    "particles {name}: object {parent} has no node {node} in its mesh"
                                ),
                            );
                        };
                        Some(Node {
                            name: node.clone(),
                            joint: part.joint,
                            transform: part.transform,
                        })
                    }
                };
                Some(Attach {
                    object: object.name.clone(),
                    node,
                })
            }
        };
        out.insert(
            entry.key.clone(),
            Particles {
                attach,
                at: value.at.unwrap_or([0.0; 3]),
                recipe: recipe(name, value),
            },
        );
    }
    Ok(out)
}

impl Attach {
    pub fn index(&self, scene: &Scene) -> Option<usize> {
        scene
            .objects
            .iter()
            .position(|object| object.name == self.object)
    }

    pub fn world(&self, object: Matrix, palette: Option<&[Matrix]>) -> Matrix {
        match &self.node {
            None => object,
            Some(node) => {
                let joint = node
                    .joint
                    .and_then(|joint| palette.and_then(|palette| palette.get(joint)));
                multiply(object, *joint.unwrap_or(&node.transform))
            }
        }
    }
}
