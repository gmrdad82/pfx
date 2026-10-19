use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock};

use pfx_materials::Material;

use crate::surface::Surface;

use super::{
    ALPHA_CUTOFF, DEFAULT_MATERIAL, Geometry, IDENTITY, Matrix, Object, Part, Scene, Shadow,
    multiply, pose,
};

pub const EMITTER_RINGS: u32 = 12;
pub const EMITTER_SEGMENTS: u32 = 24;

#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub object: String,
    pub node: String,
    pub joint: Option<usize>,
    pub geometry: Arc<Geometry>,
    pub model: Matrix,
    pub material: u32,
    pub id: u32,
    pub shadow: Shadow,
    pub two_sided: bool,
    pub clip: [[f32; 4]; 2],
    pub content: Option<String>,
    pub alpha_cutoff: f32,
    pub face_camera: bool,
    pub owner: Option<usize>,
    pub part: Matrix,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draws {
    pub names: Vec<String>,
    pub materials: Vec<Material>,
    pub items: Vec<Draw>,
    pub textures: super::textures::Textures,
}

impl Draws {
    pub fn material(&self, name: &str) -> Option<u32> {
        self.names
            .iter()
            .position(|candidate| candidate == name)
            .map(|index| index as u32)
    }

    pub fn models_posed(
        &self,
        objects: &[Matrix],
        palettes: &std::collections::BTreeMap<usize, Vec<Matrix>>,
    ) -> Vec<Matrix> {
        self.items
            .iter()
            .map(|draw| {
                let Some(world) = draw.owner.and_then(|owner| objects.get(owner)) else {
                    return draw.model;
                };
                let joint = draw.joint.and_then(|joint| {
                    palettes
                        .get(&draw.owner?)
                        .and_then(|palette| palette.get(joint))
                });
                match joint {
                    Some(joint) => multiply(*world, *joint),
                    None => multiply(*world, draw.part),
                }
            })
            .collect()
    }

    pub fn models(&self, posed: &[Matrix]) -> Vec<Matrix> {
        self.items
            .iter()
            .map(|draw| match draw.owner.and_then(|owner| posed.get(owner)) {
                Some(world) => multiply(*world, draw.part),
                None => draw.model,
            })
            .collect()
    }
}

pub fn emitter_material(name: &str) -> String {
    format!("emitter {name}")
}

static SPHERE: LazyLock<Arc<Geometry>> = LazyLock::new(|| Arc::new(sphere()));

pub fn emitter_sphere() -> Arc<Geometry> {
    SPHERE.clone()
}

fn sphere() -> Geometry {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    for ring in 0..=EMITTER_RINGS {
        let theta = std::f32::consts::PI * ring as f32 / EMITTER_RINGS as f32;
        for segment in 0..=EMITTER_SEGMENTS {
            let phi = std::f32::consts::TAU * segment as f32 / EMITTER_SEGMENTS as f32;
            positions.push([
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ]);
            uvs.push([
                segment as f32 / EMITTER_SEGMENTS as f32,
                ring as f32 / EMITTER_RINGS as f32,
            ]);
        }
    }
    let row = EMITTER_SEGMENTS + 1;
    let mut indices = Vec::new();
    for ring in 0..EMITTER_RINGS {
        for segment in 0..EMITTER_SEGMENTS {
            let a = ring * row + segment;
            let b = a + row;
            if ring != 0 {
                indices.extend([a, a + 1, b]);
            }
            if ring != EMITTER_RINGS - 1 {
                indices.extend([a + 1, b + 1, b]);
            }
        }
    }
    let count = positions.len();
    Geometry::new(
        positions.clone(),
        positions,
        vec![[1.0, 0.0, 0.0, 1.0]; count],
        uvs,
        None,
        indices,
    )
}

pub(super) fn pick<'a>(scene: &'a Scene, object: &'a Object, part: &'a Part) -> Cow<'a, str> {
    if let Some(material) = object.materials.get(&part.node) {
        return material.into();
    }
    if let Some(material) = &object.material {
        return material.into();
    }
    if let Some(material) = &part.overrides.material {
        return material.into();
    }
    if let Some(name) = &part.material {
        if scene.library.contains(name) {
            return name.into();
        }
        if let Some((head, _)) = name.split_once('.')
            && scene.library.contains(head)
        {
            return head.into();
        }
    }
    if let Some(fallback) = &scene.fallback {
        return fallback.into();
    }
    match &part.surface {
        Some(surface) => super::textures::surface_name(&object.mesh, surface).into(),
        None => DEFAULT_MATERIAL.into(),
    }
}

fn own(part: &Part, name: &str) -> bool {
    part.material.as_deref().is_some_and(|material| {
        material == name
            || material
                .split_once('.')
                .is_some_and(|(head, _)| head == name)
    })
}

pub(super) fn draws(scene: &Scene) -> Draws {
    let mut used = BTreeSet::new();
    let mut picked = Vec::new();
    let mut surfaces = BTreeMap::new();
    let mut soaked: BTreeMap<String, Arc<Surface>> = BTreeMap::new();
    for (owner, object) in scene.objects.iter().enumerate() {
        let Some(mesh) = scene.meshes.get(&object.mesh) else {
            continue;
        };
        for part in &mesh.parts {
            if object.hidden || part.overrides.hidden {
                continue;
            }
            let name = pick(scene, object, part).into_owned();
            if let Some(surface) = &part.surface
                && !scene.library.contains(&name)
                && name == super::textures::surface_name(&object.mesh, surface)
            {
                surfaces.insert(name.clone(), surface.clone());
            }
            if let Some(surface) = &part.surface
                && surface.maps.any()
                && own(part, &name)
                && scene
                    .library
                    .get(&name)
                    .is_some_and(|material| !material.wet.inactive())
            {
                soaked
                    .entry(name.clone())
                    .or_insert_with(|| surface.clone());
            }
            used.insert(name.clone());
            picked.push((owner, object, part, name));
        }
    }
    for name in scene.emitters.keys() {
        used.insert(emitter_material(name));
    }
    let names: Vec<String> = used.into_iter().collect();
    let mut textures = super::textures::Textures::default();
    let materials = names
        .iter()
        .map(|name| match scene.library.get(name) {
            None if surfaces.contains_key(name) => textures.material(&surfaces[name]),
            Some(material) => match soaked.get(name) {
                Some(surface) => textures.material(&Surface {
                    material: *material,
                    ..(**surface).clone()
                }),
                None => *material,
            },
            None => match name
                .strip_prefix("emitter ")
                .and_then(|key| scene.emitters.get(key))
            {
                Some(emitter) => {
                    let area = std::f32::consts::PI * emitter.radius * emitter.radius;
                    Material {
                        base: [0.0; 3],
                        roughness: 1.0,
                        specular: 0.0,
                        emission: emitter.color.map(|c| c * emitter.intensity / area),
                        ..Material::default()
                    }
                }
                None => Material::default(),
            },
        })
        .collect();
    let index = |name: &str| {
        names
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or(0) as u32
    };
    let mut items = Vec::new();
    for (owner, object, part, name) in picked {
        let mut clip = [[0.0; 4]; 2];
        for (slot, plane) in clip.iter_mut().zip(pose::clips(scene, owner)) {
            *slot = plane;
        }
        items.push(Draw {
            object: object.name.clone(),
            node: part.node.clone(),
            joint: part.joint,
            geometry: part.geometry.clone(),
            model: multiply(object.model, part.transform),
            material: index(&name),
            id: object.id,
            shadow: part.overrides.shadow.unwrap_or(object.shadow),
            two_sided: part.overrides.two_sided.unwrap_or(object.two_sided),
            clip,
            content: object.content.clone(),
            alpha_cutoff: object.alpha_cutoff,
            face_camera: object.face_camera,
            owner: Some(owner),
            part: part.transform,
        });
    }
    let first_id = scene
        .objects
        .iter()
        .map(|object| object.id)
        .max()
        .unwrap_or(0)
        + 1;
    for (place, (name, emitter)) in scene.emitters.iter().enumerate() {
        let mut model = IDENTITY;
        for (axis, column) in model.iter_mut().take(3).enumerate() {
            column[axis] = emitter.radius;
        }
        model[3] = [
            emitter.position[0],
            emitter.position[1],
            emitter.position[2],
            1.0,
        ];
        items.push(Draw {
            object: name.clone(),
            node: String::new(),
            joint: None,
            geometry: emitter_sphere(),
            model,
            material: index(&emitter_material(name)),
            id: first_id + place as u32,
            shadow: Shadow::None,
            two_sided: false,
            clip: [[0.0; 4]; 2],
            content: None,
            alpha_cutoff: ALPHA_CUTOFF,
            face_camera: false,
            owner: None,
            part: IDENTITY,
        });
    }
    Draws {
        names,
        materials,
        items,
        textures,
    }
}
