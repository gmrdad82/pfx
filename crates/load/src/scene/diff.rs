use std::collections::BTreeMap;

use super::Scene;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    pub fn by<T>(
        older: &BTreeMap<String, T>,
        newer: &BTreeMap<String, T>,
        same: impl Fn(&T, &T) -> bool,
    ) -> Self {
        let mut changes = Self::default();
        for (name, value) in newer {
            match older.get(name) {
                None => changes.added.push(name.clone()),
                Some(old) if !same(old, value) => changes.changed.push(name.clone()),
                Some(_) => {}
            }
        }
        for name in older.keys() {
            if !newer.contains_key(name) {
                changes.removed.push(name.clone());
            }
        }
        changes
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneDiff {
    pub meshes: Changes,
    pub materials: Changes,
    pub objects: Changes,
    pub lights: Changes,
    pub emitters: Changes,
    pub particles: Changes,
    pub contents: Changes,
    pub texts: Changes,
    pub movers: Changes,
    pub bodies: Changes,
    pub characters: Changes,
    pub animations: Changes,
    pub triggers: Changes,
    pub layers: bool,
    pub sounds: Changes,
    pub order: bool,
    pub fallback: bool,
    pub sun: bool,
    pub sky: bool,
    pub haze: bool,
    pub camera: bool,
    pub finish: bool,
    pub trace: bool,
    pub plates: bool,
    pub physics: bool,
    pub motion_blur: bool,
}

impl SceneDiff {
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
            && self.materials.is_empty()
            && self.objects.is_empty()
            && self.lights.is_empty()
            && self.emitters.is_empty()
            && self.particles.is_empty()
            && self.contents.is_empty()
            && self.texts.is_empty()
            && self.movers.is_empty()
            && self.bodies.is_empty()
            && self.characters.is_empty()
            && self.animations.is_empty()
            && self.triggers.is_empty()
            && !self.layers
            && self.sounds.is_empty()
            && !self.order
            && !self.fallback
            && !self.sun
            && !self.sky
            && !self.haze
            && !self.camera
            && !self.finish
            && !self.trace
            && !self.plates
            && !self.physics
            && !self.motion_blur
    }

    pub fn draws(&self) -> bool {
        !self.meshes.is_empty()
            || !self.materials.is_empty()
            || !self.objects.is_empty()
            || !self.emitters.is_empty()
            || !self.contents.is_empty()
            || !self.movers.is_empty()
            || !self.animations.is_empty()
            || self.order
            || self.fallback
    }
}

fn keyed_lights(
    scene: &Scene,
) -> BTreeMap<String, (super::Light, Option<&pfx_scene::types::Keys>)> {
    scene
        .lights
        .iter()
        .map(|(name, light)| (name.clone(), (*light, scene.keys.lights.get(name))))
        .collect()
}

pub(super) fn diff(older: &Scene, newer: &Scene) -> SceneDiff {
    let objects = |scene: &Scene| -> BTreeMap<String, super::Object> {
        scene
            .objects
            .iter()
            .map(|object| (object.name.clone(), object.clone()))
            .collect()
    };
    let materials = |scene: &Scene| -> BTreeMap<String, pfx_materials::Material> {
        scene
            .library
            .iter()
            .map(|(name, material)| (name.to_string(), *material))
            .collect()
    };
    let names = |scene: &Scene| -> Vec<String> {
        scene
            .objects
            .iter()
            .map(|object| object.name.clone())
            .collect()
    };
    let objects_changes = Changes::by(&objects(older), &objects(newer), |a, b| a == b);
    SceneDiff {
        meshes: Changes::by(&older.meshes, &newer.meshes, |a, b| a.hash == b.hash),
        materials: Changes::by(&materials(older), &materials(newer), |a, b| a == b),
        order: objects_changes.added.is_empty()
            && objects_changes.removed.is_empty()
            && names(older) != names(newer),
        objects: objects_changes,
        lights: Changes::by(&keyed_lights(older), &keyed_lights(newer), |a, b| a == b),
        emitters: Changes::by(&older.emitters, &newer.emitters, |a, b| a == b),
        particles: Changes::by(&older.particles, &newer.particles, |a, b| a == b),
        contents: Changes::by(&older.contents, &newer.contents, |a, b| a.hash == b.hash),
        texts: Changes::by(&older.texts, &newer.texts, |a, b| a == b),
        movers: Changes::by(&older.movers, &newer.movers, |a, b| a == b),
        bodies: Changes::by(&older.bodies, &newer.bodies, |a, b| a == b),
        characters: Changes::by(&older.characters, &newer.characters, |a, b| a == b),
        animations: Changes::by(&older.animations, &newer.animations, |a, b| a == b),
        triggers: Changes::by(&older.triggers, &newer.triggers, |a, b| a == b),
        layers: older.layers != newer.layers || older.body_layers != newer.body_layers,
        sounds: Changes::by(&older.sounds, &newer.sounds, |a, b| {
            a.hash == b.hash
                && a.volume == b.volume
                && a.pan == b.pan
                && a.looping == b.looping
                && a.cue == b.cue
                && a.object == b.object
        }),
        fallback: older.fallback != newer.fallback,
        sun: older.sun != newer.sun || older.keys.sun != newer.keys.sun,
        sky: older.sky != newer.sky
            || older.keys.sky != newer.keys.sky
            || (older.keys.sun != newer.keys.sun && newer.keys.sky.is_some()),
        haze: older.haze != newer.haze || older.keys.haze != newer.keys.haze,
        camera: older.camera != newer.camera || older.keys.camera != newer.keys.camera,
        finish: older.finish != newer.finish || older.keys.finish != newer.keys.finish,
        trace: older.trace != newer.trace,
        plates: older.plates != newer.plates,
        physics: older.physics != newer.physics,
        motion_blur: older.motion_blur != newer.motion_blur,
    }
}
