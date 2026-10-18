use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::Diagnostic;
use crate::types::{
    Camera, Content, Emitter, Finish, FinishKeys, Flare, Haze, Light, Material, Mesh, MotionBlur,
    Mover, Object, Particles, Physics, Plates, Proxy, Rig, Shadows, Sky, Sound, Sun, Text, Tiles,
    Trace,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry<T> {
    pub key: String,
    pub id: Option<String>,
    pub file: PathBuf,
    pub value: T,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub path: PathBuf,
    pub files: Vec<PathBuf>,
    pub materials: Vec<Entry<Material>>,
    pub fallback: Option<String>,
    pub meshes: Vec<Entry<Mesh>>,
    pub objects: Vec<Entry<Object>>,
    pub lights: Vec<Entry<Light>>,
    pub emitters: Vec<Entry<Emitter>>,
    pub movers: Vec<Entry<Mover>>,
    pub contents: Vec<Entry<Content>>,
    pub texts: Vec<Entry<Text>>,
    pub sounds: Vec<Entry<Sound>>,
    pub sound_file: Option<PathBuf>,
    pub rigs: Vec<Entry<Rig>>,
    pub tiles: Vec<Entry<Tiles>>,
    pub particles: Vec<Entry<Particles>>,
    pub sun: Option<Sun>,
    pub sky: Option<Sky>,
    pub haze: Option<Haze>,
    pub camera: Option<Camera>,
    pub finish: Option<Finish>,
    pub finish_file: Option<FinishKeys>,
    pub flare: Option<Flare>,
    pub motion_blur: Option<MotionBlur>,
    pub trace: Option<Trace>,
    pub shadows: Option<Shadows>,
    pub plates: Option<Plates>,
    pub proxies: Vec<Entry<Proxy>>,
    pub physics: Option<Physics>,
    pub play_module: Option<PathBuf>,
    pub layers: BTreeMap<String, Vec<String>>,
    pub warnings: Vec<Diagnostic>,
}

fn find<'a, T>(entries: &'a [Entry<T>], key: &str) -> Option<&'a Entry<T>> {
    entries.iter().find(|entry| entry.key == key)
}

impl Scene {
    pub fn material(&self, key: &str) -> Option<&Entry<Material>> {
        find(&self.materials, key)
    }

    pub fn mesh(&self, key: &str) -> Option<&Entry<Mesh>> {
        find(&self.meshes, key)
    }

    pub fn object(&self, key: &str) -> Option<&Entry<Object>> {
        find(&self.objects, key)
    }

    pub fn light(&self, key: &str) -> Option<&Entry<Light>> {
        find(&self.lights, key)
    }

    pub fn emitter(&self, key: &str) -> Option<&Entry<Emitter>> {
        find(&self.emitters, key)
    }

    pub fn mover(&self, key: &str) -> Option<&Entry<Mover>> {
        find(&self.movers, key)
    }

    pub fn content(&self, key: &str) -> Option<&Entry<Content>> {
        find(&self.contents, key)
    }

    pub fn text(&self, key: &str) -> Option<&Entry<Text>> {
        find(&self.texts, key)
    }

    pub fn sound(&self, key: &str) -> Option<&Entry<Sound>> {
        find(&self.sounds, key)
    }

    pub fn rig(&self, key: &str) -> Option<&Entry<Rig>> {
        find(&self.rigs, key)
    }

    pub fn particles(&self, key: &str) -> Option<&Entry<Particles>> {
        find(&self.particles, key)
    }

    pub fn tile_layer(&self, key: &str) -> Option<&Entry<Tiles>> {
        find(&self.tiles, key)
    }
}
