use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use pfx_scene::Entry;
use pfx_scene::types as file;

use super::build::{Places, Reader, Step, inner};
use super::{Object, SceneError, SceneMesh, multiply};
use crate::sha256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Physics {
    pub gravity: [f32; 3],
    pub rate: f32,
    pub substeps: u32,
    pub iterations: u32,
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            gravity: [0.0, -9.81, 0.0],
            rate: 60.0,
            substeps: 1,
            iterations: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Dynamic,
    Kinematic,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyShape {
    Box { half: [f32; 3] },
    Sphere { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub kind: BodyKind,
    pub shape: BodyShape,
    pub offset: [f32; 3],
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub velocity: [f32; 3],
    pub spin: [f32; 3],
    pub gravity_scale: f32,
    pub damping: [f32; 2],
    pub ccd: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub shape: BodyShape,
    pub offset: [f32; 3],
    pub layer: Option<String>,
    pub mask: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterPlane {
    Xy,
    Yz,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub plane: Option<CharacterPlane>,
    pub radius: f32,
    pub height: f32,
    pub max_climb: f32,
    pub max_step: f32,
    pub snap: f32,
    pub coyote: u32,
    pub jump_buffer: u32,
    pub jump_speed: f32,
    pub variable_jump: f32,
    pub wall_slide: Option<f32>,
    pub wall_jump: Option<[f32; 2]>,
    pub layer: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Start,
    Hit,
    Game,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub file: PathBuf,
    pub bytes: Arc<Vec<u8>>,
    pub hash: [u8; 32],
    pub volume: f32,
    pub pan: f32,
    pub looping: bool,
    pub cue: Cue,
    pub object: Option<String>,
}

pub(super) fn build_physics(file: &file::Physics) -> Physics {
    let base = Physics::default();
    Physics {
        gravity: file.gravity.unwrap_or(base.gravity),
        rate: file.rate.unwrap_or(base.rate),
        substeps: file.substeps.unwrap_or(base.substeps),
        iterations: file.iterations.unwrap_or(base.iterations),
    }
}

pub(super) fn build_sound(
    places: &Places,
    entry: &Entry<file::Sound>,
    reader: &mut Reader,
) -> Result<Sound, SceneError> {
    let name = entry.key.as_str();
    let steps = [Step::Key("sound"), Step::Key(inner(name))];
    let sound = &entry.value;
    let path = places.root.join(&sound.file);
    let bytes = reader
        .read(&path)
        .map_err(|message| places.refusal(&entry.file, &steps, message))?;
    if bytes.starts_with(b"OggS") {
        if let Err(message) = check_vorbis(&bytes) {
            return places.error(
                &entry.file,
                &steps,
                format!("sound {name}: {} {message}", sound.file),
            );
        }
    } else if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return places.error(
            &entry.file,
            &steps,
            format!(
                "sound {name}: {} is not a WAV or an Ogg Vorbis file",
                sound.file
            ),
        );
    }
    Ok(Sound {
        file: path,
        hash: sha256(&bytes),
        bytes: Arc::new(bytes),
        volume: sound.volume.unwrap_or(1.0),
        pan: sound.pan.unwrap_or(0.0),
        looping: sound.looping,
        cue: match sound.play.unwrap_or_default() {
            file::Cue::Start => Cue::Start,
            file::Cue::Hit => Cue::Hit,
            file::Cue::Game => Cue::Game,
        },
        object: sound.object.clone(),
    })
}

fn check_vorbis(bytes: &[u8]) -> Result<(), String> {
    let mut at = 0usize;
    let mut first = true;
    let mut frames = 0i64;
    let mut rate = 0u32;
    while at < bytes.len() {
        let head = bytes
            .get(at..at + 27)
            .filter(|head| &head[0..4] == b"OggS" && head[4] == 0)
            .ok_or("is a damaged Ogg file")?;
        let segments = usize::from(head[26]);
        let table = bytes
            .get(at + 27..at + 27 + segments)
            .ok_or("is a truncated Ogg file")?;
        let body_len: usize = table.iter().map(|len| usize::from(*len)).sum();
        let body_at = at + 27 + segments;
        let body = bytes
            .get(body_at..body_at + body_len)
            .ok_or("is a truncated Ogg file")?;
        if first {
            if body.len() < 30 || &body[0..7] != b"\x01vorbis" {
                return Err("is an Ogg file with no Vorbis stream".into());
            }
            let version = u32::from_le_bytes([body[7], body[8], body[9], body[10]]);
            let channels = body[11];
            rate = u32::from_le_bytes([body[12], body[13], body[14], body[15]]);
            if version != 0 || channels == 0 || rate == 0 {
                return Err("has a Vorbis header with no audio".into());
            }
            first = false;
        }
        let granule = i64::from_le_bytes([
            head[6], head[7], head[8], head[9], head[10], head[11], head[12], head[13],
        ]);
        if granule > 0 {
            frames = granule;
        }
        at = body_at + body_len;
    }
    if first {
        return Err("is not a WAV or an Ogg Vorbis file".into());
    }
    if frames == 0 || rate == 0 {
        return Err("is an Ogg Vorbis file with no duration".into());
    }
    Ok(())
}

pub(super) fn build_bodies(
    places: &Places,
    entries: &[Entry<file::Object>],
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<BTreeMap<String, Body>, SceneError> {
    let mut out = BTreeMap::new();
    for (entry, object) in entries.iter().zip(objects) {
        let Some(file) = &entry.value.body else {
            continue;
        };
        let name = entry.key.as_str();
        let steps = [Step::Named("object", inner(name)), Step::Key("body")];
        let Some(mesh) = meshes.get(&object.mesh) else {
            return places.error(
                &entry.file,
                &steps,
                format!("object {name} has a body and no mesh"),
            );
        };
        let body = build_body(name, file, object, mesh)
            .map_err(|message| places.refusal(&entry.file, &steps, message))?;
        out.insert(name.to_string(), body);
    }
    Ok(out)
}

pub(super) fn build_triggers(
    places: &Places,
    entries: &[Entry<file::Object>],
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<BTreeMap<String, Trigger>, SceneError> {
    let mut out = BTreeMap::new();
    for (entry, object) in entries.iter().zip(objects) {
        let Some(file) = &entry.value.trigger else {
            continue;
        };
        let name = entry.key.as_str();
        let steps = [Step::Named("object", inner(name)), Step::Key("trigger")];
        let Some(mesh) = meshes.get(&object.mesh) else {
            return places.error(
                &entry.file,
                &steps,
                format!("object {name} has a trigger and no mesh"),
            );
        };
        let (shape, offset) = volume(
            name,
            "trigger",
            Volume {
                shape: file.shape.unwrap_or_default(),
                half: file.half,
                radius: file.radius,
                half_height: file.half_height,
                offset: file.offset,
            },
            object,
            mesh,
        )
        .map_err(|message| places.refusal(&entry.file, &steps, message))?;
        out.insert(
            name.to_string(),
            Trigger {
                shape,
                offset,
                layer: file.layer.clone(),
                mask: file.mask.clone(),
            },
        );
    }
    Ok(out)
}

pub(super) fn build_characters(
    places: &Places,
    entries: &[Entry<file::Object>],
    objects: &[Object],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<BTreeMap<String, Character>, SceneError> {
    let mut out = BTreeMap::new();
    for (entry, object) in entries.iter().zip(objects) {
        let Some(file) = &entry.value.character else {
            continue;
        };
        let name = entry.key.as_str();
        let steps = [Step::Named("object", inner(name)), Step::Key("character")];
        let Some(mesh) = meshes.get(&object.mesh) else {
            return places.error(
                &entry.file,
                &steps,
                format!("object {name} has a character and no mesh"),
            );
        };
        let character = build_character(name, file, object, mesh)
            .map_err(|message| places.refusal(&entry.file, &steps, message))?;
        out.insert(name.to_string(), character);
    }
    Ok(out)
}

fn build_character(
    name: &str,
    file: &file::Character,
    object: &Object,
    mesh: &SceneMesh,
) -> Result<Character, String> {
    let (low, high) = bounds(mesh);
    let scale = object.scale.map(f32::abs);
    let half: [f32; 3] = std::array::from_fn(|axis| (high[axis] - low[axis]) * 0.5 * scale[axis]);
    let radius = file.radius.unwrap_or(half[0].max(half[2]));
    let height = file.height.unwrap_or(half[1] * 2.0);
    if !(radius.is_finite() && radius > 0.0 && height.is_finite() && height > 0.0) {
        return Err(format!(
            "object {name}: its character has no size; give it a positive radius and height"
        ));
    }
    if height < radius * 2.0 {
        return Err(format!(
            "object {name}: its character's height {height} is less than twice its radius {radius}"
        ));
    }
    let plane = match file.kind.unwrap_or_default() {
        file::CharacterKind::ThreeD => None,
        file::CharacterKind::TwoD => Some(match file.plane.unwrap_or_default() {
            file::Plane::Xy => CharacterPlane::Xy,
            file::Plane::Yz => CharacterPlane::Yz,
        }),
    };
    Ok(Character {
        plane,
        radius,
        height,
        max_climb: file.max_climb.unwrap_or(file::Character::MAX_CLIMB),
        max_step: file.max_step.unwrap_or(file::Character::MAX_STEP),
        snap: file.snap.unwrap_or(file::Character::SNAP),
        coyote: file.coyote.unwrap_or(file::Character::COYOTE),
        jump_buffer: file.jump_buffer.unwrap_or(file::Character::JUMP_BUFFER),
        jump_speed: file.jump_speed.unwrap_or(file::Character::JUMP_SPEED),
        variable_jump: file.variable_jump.unwrap_or(file::Character::VARIABLE_JUMP),
        wall_slide: file.wall_slide,
        wall_jump: file.wall_jump,
        layer: file.layer.clone(),
    })
}

pub(super) fn body_layers(entries: &[Entry<file::Object>]) -> BTreeMap<String, String> {
    entries
        .iter()
        .filter_map(|entry| {
            let layer = entry.value.body.as_ref()?.layer.clone()?;
            Some((entry.key.clone(), layer))
        })
        .collect()
}

fn build_body(
    name: &str,
    file: &file::Body,
    object: &Object,
    mesh: &SceneMesh,
) -> Result<Body, String> {
    let kind = match file.kind.unwrap_or_default() {
        file::BodyKind::Dynamic => BodyKind::Dynamic,
        file::BodyKind::Kinematic => BodyKind::Kinematic,
        file::BodyKind::Fixed => BodyKind::Fixed,
    };
    let (shape, offset) = volume(
        name,
        "body",
        Volume {
            shape: file.shape.unwrap_or_default(),
            half: file.half,
            radius: file.radius,
            half_height: file.half_height,
            offset: file.offset,
        },
        object,
        mesh,
    )?;
    Ok(Body {
        kind,
        shape,
        offset,
        density: file.density.unwrap_or(file::Body::DENSITY),
        friction: file.friction.unwrap_or(file::Body::FRICTION),
        restitution: file.restitution.unwrap_or(file::Body::RESTITUTION),
        velocity: file.velocity.unwrap_or([0.0; 3]),
        spin: file.spin.unwrap_or([0.0; 3]),
        gravity_scale: file.gravity_scale.unwrap_or(file::Body::GRAVITY_SCALE),
        damping: file.damping.unwrap_or([0.0; 2]),
        ccd: file.ccd,
    })
}

struct Volume {
    shape: file::BodyShape,
    half: Option<[f32; 3]>,
    radius: Option<f32>,
    half_height: Option<f32>,
    offset: Option<[f32; 3]>,
}

fn volume(
    name: &str,
    what: &str,
    file: Volume,
    object: &Object,
    mesh: &SceneMesh,
) -> Result<(BodyShape, [f32; 3]), String> {
    let (low, high) = bounds(mesh);
    let scale = object.scale.map(f32::abs);
    let half: [f32; 3] = std::array::from_fn(|axis| (high[axis] - low[axis]) * 0.5 * scale[axis]);
    let centre: [f32; 3] =
        std::array::from_fn(|axis| (high[axis] + low[axis]) * 0.5 * object.scale[axis]);
    let round = half[0].max(half[2]);
    let (shape, shape_name) = match file.shape {
        file::BodyShape::Box => (
            BodyShape::Box {
                half: file.half.unwrap_or(half),
            },
            "box",
        ),
        file::BodyShape::Sphere => (
            BodyShape::Sphere {
                radius: file
                    .radius
                    .unwrap_or_else(|| half[0].max(half[1]).max(half[2])),
            },
            "sphere",
        ),
        file::BodyShape::Capsule => {
            let radius = file.radius.unwrap_or(round);
            (
                BodyShape::Capsule {
                    half_height: file
                        .half_height
                        .unwrap_or_else(|| (half[1] - radius).max(0.0)),
                    radius,
                },
                "capsule",
            )
        }
        file::BodyShape::Cylinder => (
            BodyShape::Cylinder {
                half_height: file.half_height.unwrap_or(half[1]),
                radius: file.radius.unwrap_or(round),
            },
            "cylinder",
        ),
    };
    let sizes = match shape {
        BodyShape::Box { half } => half.to_vec(),
        BodyShape::Sphere { radius } => vec![radius],
        BodyShape::Capsule {
            half_height,
            radius,
        } => vec![half_height + radius, radius],
        BodyShape::Cylinder {
            half_height,
            radius,
        } => vec![half_height, radius],
    };
    if sizes.iter().any(|size| !(size.is_finite() && *size > 0.0)) {
        return Err(format!(
            "object {name}: its {what} has a {shape_name} with no size; give it a positive size"
        ));
    }
    Ok((shape, file.offset.unwrap_or(centre)))
}

fn bounds(mesh: &SceneMesh) -> ([f32; 3], [f32; 3]) {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for part in &mesh.parts {
        for position in &part.geometry.positions {
            let point = multiply(
                part.transform,
                [
                    [position[0], position[1], position[2], 1.0],
                    [0.0; 4],
                    [0.0; 4],
                    [0.0; 4],
                ],
            )[0];
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
    }
    if low[0] > high[0] {
        return ([0.0; 3], [0.0; 3]);
    }
    (low, high)
}
