use std::collections::BTreeMap;

use pfx_core::sim::mix64;
use pfx_core::sim::particles::{
    Blend, Body, Gradient, Ground, IDENTITY, Lerp, Placed, Recipe, RecipeKeys,
};
use pfx_core::sim::wake::Recipe as WakeRecipe;
use pfx_scene::Entry;
use pfx_scene::types::{self as file, KeyValue};

use super::build::{Places, Step, inner};
use super::{Draw, Draws, Matrix, Object, Scene, SceneError, SceneMesh, multiply};

#[derive(Clone, Debug, PartialEq)]
pub struct Particles {
    pub attach: Option<Attach>,
    pub at: [f32; 3],
    pub recipe: Recipe,
    pub mesh: Option<Attach>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wake {
    pub attach: Attach,
    pub recipe: WakeRecipe,
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
        body: body(value),
        placed: value
            .placed
            .iter()
            .flatten()
            .map(|[x, y, z, yaw]| Placed {
                position: [*x, *y, *z],
                yaw: *yaw,
            })
            .collect(),
    }
}

fn attach(
    places: &Places,
    entry: &Entry<file::Particles>,
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
    keys: [&'static str; 2],
    named: (Option<&String>, Option<&String>),
) -> Result<Option<Attach>, SceneError> {
    let name = entry.key.as_str();
    let at = |key| [Step::Named("particles", inner(name)), Step::Key(key)];
    let [object_key, node_key] = keys;
    let object = match named {
        (None, None) => return Ok(None),
        (None, Some(_)) => {
            return places.error(
                &entry.file,
                &at(node_key),
                format!("particles {name}: {node_key} needs {object_key}, the object it is in"),
            );
        }
        (Some(object), _) => object,
    };
    let Some(found) = objects.iter().find(|candidate| &candidate.name == object) else {
        return places.error(
            &entry.file,
            &at(object_key),
            format!("particles {name}: {object_key} {object} is no object of this scene"),
        );
    };
    let node = match named.1 {
        None => None,
        Some(node) => {
            let part = meshes
                .get(&found.mesh)
                .and_then(|mesh| mesh.parts.iter().find(|part| &part.node == node));
            let Some(part) = part else {
                return places.error(
                    &entry.file,
                    &at(node_key),
                    format!("particles {name}: object {object} has no node {node} in its mesh"),
                );
            };
            Some(Node {
                name: node.clone(),
                joint: part.joint,
                transform: part.transform,
            })
        }
    };
    Ok(Some(Attach {
        object: found.name.clone(),
        node,
    }))
}

pub(super) fn build(
    places: &Places,
    entries: &[Entry<file::Particles>],
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<BTreeMap<String, Particles>, SceneError> {
    let mut out = BTreeMap::new();
    for entry in entries {
        let value = &entry.value;
        let parent = attach(
            places,
            entry,
            objects,
            meshes,
            ["parent", "node"],
            (value.parent.as_ref(), value.node.as_ref()),
        )?;
        let mesh = attach(
            places,
            entry,
            objects,
            meshes,
            ["mesh", "mesh_node"],
            (value.mesh.as_ref(), value.mesh_node.as_ref()),
        )?;
        out.insert(
            entry.key.clone(),
            Particles {
                attach: parent,
                at: value.at.unwrap_or([0.0; 3]),
                recipe: recipe(entry.key.as_str(), value),
                mesh,
            },
        );
    }
    Ok(out)
}

pub(super) fn wakes(
    places: &Places,
    entries: &[Entry<file::Object>],
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<BTreeMap<String, Wake>, SceneError> {
    let mut out = BTreeMap::new();
    for entry in entries {
        let Some(wake) = &entry.value.wake else {
            continue;
        };
        let name = entry.key.as_str();
        let Some(object) = objects.iter().find(|object| object.name == name) else {
            continue;
        };
        let node = match &wake.node {
            None => None,
            Some(node) => {
                let part = meshes
                    .get(&object.mesh)
                    .and_then(|mesh| mesh.parts.iter().find(|part| &part.node == node));
                let Some(part) = part else {
                    return places.error(
                        &entry.file,
                        &[
                            Step::Named("object", inner(name)),
                            Step::Key("wake"),
                            Step::Key("node"),
                        ],
                        format!("object {name}: wake.node {node} is not in its mesh"),
                    );
                };
                Some(Node {
                    name: node.clone(),
                    joint: part.joint,
                    transform: part.transform,
                })
            }
        };
        let base = WakeRecipe::default();
        out.insert(
            entry.key.clone(),
            Wake {
                attach: Attach {
                    object: object.name.clone(),
                    node,
                },
                recipe: WakeRecipe {
                    at: wake.at.unwrap_or(base.at),
                    strength: wake.strength.unwrap_or(base.strength),
                    radius: wake.radius.unwrap_or(base.radius),
                    grow: wake.grow.unwrap_or(base.grow),
                    decay: wake.decay.unwrap_or(base.decay),
                    spread: wake.spread.unwrap_or(base.spread),
                    rise: wake.rise.unwrap_or(base.rise),
                    length: wake.length.unwrap_or(base.length),
                },
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

    pub fn draws(&self, draw: &Draw) -> bool {
        draw.object == self.object && self.node.as_ref().is_none_or(|node| draw.node == node.name)
    }

    fn placed(&self, scene: &Scene, draw: &Draw) -> Matrix {
        match &self.node {
            Some(_) => draw.part,
            None => {
                let model = scene
                    .objects
                    .iter()
                    .find(|object| object.name == self.object)
                    .map_or(IDENTITY, |object| object.model);
                multiply(model, draw.part)
            }
        }
    }
}

pub type Shape = Vec<(usize, Matrix)>;

pub fn shapes(scene: &Scene, draws: &Draws) -> Vec<Shape> {
    scene
        .particles
        .values()
        .map(|particles| match &particles.mesh {
            None => Vec::new(),
            Some(mesh) => centred(
                draws
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, draw)| mesh.draws(draw))
                    .map(|(index, draw)| (index, mesh.placed(scene, draw), draw)),
            ),
        })
        .collect()
}

pub fn shaping(scene: &Scene, draws: &Draws) -> Vec<bool> {
    let meshes: Vec<&Attach> = scene
        .particles
        .values()
        .filter_map(|particles| particles.mesh.as_ref())
        .collect();
    draws
        .items
        .iter()
        .map(|draw| meshes.iter().any(|mesh| mesh.draws(draw)))
        .collect()
}

fn centred<'a>(found: impl Iterator<Item = (usize, Matrix, &'a Draw)>) -> Shape {
    let found: Vec<(usize, Matrix, &Draw)> = found.collect();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for (_, model, draw) in &found {
        for p in &draw.geometry.positions {
            let q = pfx_core::sim::particles::point(*model, *p);
            for axis in 0..3 {
                low[axis] = low[axis].min(q[axis]);
                high[axis] = high[axis].max(q[axis]);
            }
        }
    }
    let mut back = IDENTITY;
    for axis in 0..3 {
        if low[axis] <= high[axis] {
            back[3][axis] = -0.5 * (low[axis] + high[axis]);
        }
    }
    found
        .into_iter()
        .map(|(index, model, _)| (index, multiply(back, model)))
        .collect()
}

fn body(value: &file::Particles) -> Option<Body> {
    if !value.shaped() {
        return None;
    }
    let base = Body::default();
    let ground = Ground::default();
    Some(Body {
        mass: value.mass.unwrap_or(base.mass),
        area: value.area.unwrap_or(base.area),
        flat: value.flat.unwrap_or(base.flat),
        tumble: value.tumble.unwrap_or(base.tumble),
        ground: value.ground.map(|height| Ground {
            height,
            bounce: value.bounce.unwrap_or(ground.bounce),
            friction: value.friction.unwrap_or(ground.friction),
            keep: value.keep.unwrap_or(ground.keep),
            lift: value.lift.unwrap_or(ground.lift),
        }),
    })
}
