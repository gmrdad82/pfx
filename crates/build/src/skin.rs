use crate::shape::fail;
use crate::{Error, Part};
use glam::{DMat4, Mat4, Vec3};
use pfx_core::anim::{Pose, Rig};
use pfx_mesh::Mesh;
use pfx_recipe::{Kind, Object, Play, Recipe};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub bind: Vec<[f32; 3]>,
    pub frame: DMat4,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub rig: Arc<Rig>,
    pub first: usize,
    pub second: Option<usize>,
    pub play: Play,
    pub looped: bool,
    pub grounded: bool,
    pub rest: Vec<DMat4>,
}

fn wide(m: &pfx_core::anim::Mat4) -> DMat4 {
    Mat4::from_cols_array_2d(m).as_dmat4()
}

fn blend(palette: &[DMat4], joints: [u16; 4], weights: [f32; 4]) -> DMat4 {
    if weights == [0.0; 4] {
        return DMat4::IDENTITY;
    }
    let mut out = DMat4::ZERO;
    for (joint, weight) in joints.into_iter().zip(weights) {
        if weight != 0.0 {
            out += palette[joint as usize] * f64::from(weight);
        }
    }
    out
}

impl Skin {
    pub fn remap(&mut self, sources: &[u32]) {
        let pick = |s: u32| s as usize;
        self.joints = sources
            .iter()
            .map(|&s| self.joints.get(pick(s)).copied().unwrap_or([0; 4]))
            .collect();
        self.weights = sources
            .iter()
            .map(|&s| {
                self.weights
                    .get(pick(s))
                    .copied()
                    .unwrap_or([1.0, 0.0, 0.0, 0.0])
            })
            .collect();
    }

    pub fn bound(&mut self, mesh: &Mesh, rest: &[DMat4]) {
        let back = self.frame.inverse();
        self.bind = mesh
            .positions
            .iter()
            .zip(self.joints.iter().zip(&self.weights))
            .map(|(p, (&joints, &weights))| {
                let at = back.transform_point3(Vec3::from(*p).as_dvec3());
                let undo = blend(rest, joints, weights).inverse();
                undo.transform_point3(at).as_vec3().to_array()
            })
            .collect();
    }

    pub fn pose(&self, mesh: &mut Mesh, now: &[DMat4], rest: &[DMat4]) {
        let back = self.frame.inverse();
        let places: Vec<DMat4> = now.iter().map(|p| self.frame * *p).collect();
        let turns: Vec<DMat4> = now
            .iter()
            .zip(rest)
            .map(|(p, r)| self.frame * *p * r.inverse() * back)
            .collect();
        for (at, ((&joints, &weights), bind)) in self
            .joints
            .iter()
            .zip(&self.weights)
            .zip(&self.bind)
            .enumerate()
        {
            let place = blend(&places, joints, weights);
            if let Some(p) = mesh.positions.get_mut(at) {
                *p = place
                    .transform_point3(Vec3::from(*bind).as_dvec3())
                    .as_vec3()
                    .to_array();
            }
            if let Some(n) = mesh.normals.get_mut(at) {
                let turn = blend(&turns, joints, weights);
                *n = turn
                    .transform_vector3(Vec3::from(*n).as_dvec3())
                    .normalize_or_zero()
                    .as_vec3()
                    .to_array();
            }
        }
    }
}

impl Player {
    pub fn palette(&self, time: f64) -> Vec<DMat4> {
        let skeleton = self.rig.skeleton();
        let clips = self.rig.clips();
        let mut pose = Pose::rest(skeleton);
        let first = &clips[self.first];
        let length = f64::from(first.duration());
        first.sample(self.play.first(time, length, self.looped) as f32, &mut pose);
        if let Some(second) = self.second.map(|at| &clips[at]) {
            let weight = self.play.weight(time);
            if weight > 0.0 {
                let mut other = Pose::rest(skeleton);
                let length = f64::from(second.duration());
                second.sample(
                    self.play.second(time, length, self.looped) as f32,
                    &mut other,
                );
                pose.mix(&other, weight as f32);
            }
        }
        if self.grounded {
            for (joint, local) in skeleton.joints().iter().zip(pose.locals_mut()) {
                if joint.parent.is_none() {
                    local.translation[0] = joint.rest.translation[0];
                    local.translation[2] = joint.rest.translation[2];
                }
            }
        }
        pose.palette(skeleton).iter().map(wide).collect()
    }
}

pub(crate) fn rest(rig: &Rig) -> Vec<DMat4> {
    Pose::rest(rig.skeleton())
        .palette(rig.skeleton())
        .iter()
        .map(wide)
        .collect()
}

pub(crate) fn player(
    recipe: &Recipe,
    object: &Object,
    rig: Arc<Rig>,
    strict: bool,
) -> Result<Option<Player>, Error> {
    let (clip, looped, play) = match &object.kind {
        Kind::Library {
            clip, looped, play, ..
        }
        | Kind::Mesh {
            clip, looped, play, ..
        } => (clip, *looped, play),
        _ => return Ok(None),
    };
    if recipe.timeline.is_none() || rig.clips().is_empty() {
        return Ok(None);
    }
    let find = |key: &str, want: &str| -> Result<Option<usize>, Error> {
        match rig.clips().iter().position(|c| c.name() == want) {
            Some(at) => Ok(Some(at)),
            None if !strict => Ok(None),
            None => {
                let names: Vec<&str> = rig.clips().iter().map(|c| c.name()).collect();
                Err(fail(
                    object,
                    key,
                    format!(
                        "the model has no animation named `{want}`, expected one of: {}",
                        names.join(", ")
                    ),
                ))
            }
        }
    };
    let first = match clip {
        Some(want) => find("clip", want)?,
        None => Some(0),
    };
    let Some(first) = first else {
        return Ok(None);
    };
    let second = match &play.next {
        Some(want) => find("next_clip", want)?,
        None => None,
    };
    let rest = rest(&rig);
    Ok(Some(Player {
        rig,
        first,
        second,
        play: play.clone(),
        looped,
        grounded: crate::library::grounded(recipe, object),
        rest,
    }))
}

pub(crate) fn influences(
    m: &gltf::Mesh,
    buffers: &[gltf::buffer::Data],
) -> (Vec<[u16; 4]>, Vec<[f32; 4]>) {
    let mut joints = Vec::new();
    let mut weights = Vec::new();
    for primitive in m.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            continue;
        }
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
        let Some(positions) = reader.read_positions() else {
            continue;
        };
        let count = positions.len();
        let kept = match reader.read_indices() {
            Some(i) => i
                .into_u32()
                .collect::<Vec<u32>>()
                .chunks_exact(3)
                .any(|t| t.iter().all(|&v| (v as usize) < count)),
            None => count > 0,
        };
        if !kept {
            continue;
        }
        let found: Vec<[u16; 4]> = reader
            .read_joints(0)
            .map(|j| j.into_u16().collect())
            .filter(|j: &Vec<[u16; 4]>| j.len() == count)
            .unwrap_or_else(|| vec![[0; 4]; count]);
        let mut found_weights: Vec<[f32; 4]> = reader
            .read_weights(0)
            .map(|w| w.into_f32().collect())
            .filter(|w: &Vec<[f32; 4]>| w.len() == count)
            .unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 0.0]; count]);
        pfx_core::anim::normalize_weights(&mut found_weights);
        joints.extend(found);
        weights.extend(found_weights);
    }
    (joints, weights)
}

pub(crate) fn rigged(
    bytes: &[u8],
    folder: &std::path::Path,
    skin: usize,
) -> Result<pfx_load::SkinRig, String> {
    let read = |uri: &str| std::fs::read(folder.join(uri)).map_err(|e| format!("{uri}: {e}"));
    pfx_load::Mesh::rigging_with(bytes, read)?.rig(skin as u32)
}

pub(crate) fn skinned(
    mesh: &mut Mesh,
    from: usize,
    raw: &[[u16; 4]],
    weights: &[[f32; 4]],
    found: &pfx_load::SkinRig,
) -> Result<Skin, String> {
    if raw.len() != mesh.positions.len() - from || weights.len() != raw.len() {
        return Err(format!(
            "a skinned mesh has {} joint sets for {} vertices",
            raw.len(),
            mesh.positions.len() - from
        ));
    }
    let joints = found.joints(raw, weights)?;
    let rest = rest(&found.rig);
    for (p, (&j, &w)) in mesh.positions[from..]
        .iter_mut()
        .zip(joints.iter().zip(weights))
    {
        *p = blend(&rest, j, w)
            .transform_point3(Vec3::from(*p).as_dvec3())
            .as_vec3()
            .to_array();
    }
    let mut out = Skin {
        joints: vec![[0; 4]; from],
        weights: vec![[0.0; 4]; from],
        bind: Vec::new(),
        frame: DMat4::IDENTITY,
    };
    out.joints.extend(joints);
    out.weights.extend_from_slice(weights);
    Ok(out)
}

impl Skin {
    pub(crate) fn extend_static(&mut self, to: usize) {
        self.joints.resize(to, [0; 4]);
        self.weights.resize(to, [0.0; 4]);
    }

    pub(crate) fn join(&mut self, other: Skin) {
        let from = self.joints.len();
        self.joints.extend(other.joints.into_iter().skip(from));
        self.weights.extend(other.weights.into_iter().skip(from));
    }
}

pub(crate) fn pose(
    parts: &mut [Part],
    skins: &[(String, String, Skin)],
    players: &std::collections::BTreeMap<String, Player>,
    time: f64,
) {
    let mut palettes = std::collections::BTreeMap::new();
    for (piece, object, skin) in skins {
        let Some(player) = players.get(object) else {
            continue;
        };
        let palette = palettes
            .entry(object.clone())
            .or_insert_with(|| player.palette(time));
        for part in parts.iter_mut().filter(|p| p.node.name == *piece) {
            skin.pose(&mut part.node.mesh, palette, &player.rest);
        }
    }
}
