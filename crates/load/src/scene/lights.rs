use std::collections::BTreeMap;

use pfx_core::anim::Pose;
use pfx_scene::Entry;
use pfx_scene::types as file;

use super::build::{Places, Step, inner};
use super::{Light, Matrix, Object, SceneError, SceneMesh, multiply};

pub const SPOT_DIRECTION: [f32; 3] = file::Light::DIRECTION;
pub const SPOT_CONE: [f32; 2] = file::Light::CONE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub direction: [f32; 3],
    pub inner: f32,
    pub outer: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attach {
    pub object: usize,
    pub joint: Option<usize>,
    pub rest: Matrix,
    pub at: [f32; 3],
    pub direction: [f32; 3],
}

impl Spot {
    pub fn scale_offset(&self) -> (f32, f32) {
        let inner = self.inner.to_radians().cos();
        let outer = self.outer.to_radians().cos();
        let scale = 1.0 / (inner - outer).max(1e-3);
        (scale, -outer * scale)
    }

    pub fn falloff(&self, toward_point: [f32; 3]) -> f32 {
        let (scale, offset) = self.scale_offset();
        let cd = dot(self.direction, toward_point);
        let attenuation = (cd * scale + offset).clamp(0.0, 1.0);
        attenuation * attenuation
    }
}

impl Light {
    pub fn placed(&self, posed: &[Matrix]) -> Light {
        self.placed_with(posed, &BTreeMap::new())
    }

    pub fn placed_with(&self, posed: &[Matrix], palettes: &BTreeMap<usize, Vec<Matrix>>) -> Light {
        let Some(attach) = self.attach else {
            return *self;
        };
        let Some(model) = posed.get(attach.object) else {
            return *self;
        };
        let node = attach
            .joint
            .and_then(|joint| palettes.get(&attach.object)?.get(joint))
            .copied()
            .unwrap_or(attach.rest);
        self.with_model(attach, multiply(*model, node))
    }

    fn with_model(&self, attach: Attach, model: Matrix) -> Light {
        let position = point(model, attach.at);
        let spot = self.spot.map(|spot| Spot {
            direction: unit(vector(model, attach.direction)).unwrap_or(spot.direction),
            ..spot
        });
        Light {
            position,
            spot,
            ..*self
        }
    }
}

pub(super) fn build_lights(
    places: &Places,
    entries: &[Entry<file::Light>],
    objects: &[Object],
    owners: &[Entry<file::Object>],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<Vec<(String, Light)>, SceneError> {
    entries
        .iter()
        .map(|entry| {
            build_light(places, entry, objects, owners, meshes)
                .map(|light| (entry.key.clone(), light))
        })
        .collect()
}

struct Holder {
    object: usize,
    joint: Option<usize>,
    rest: Matrix,
}

fn holder(
    parent: &str,
    node: Option<&str>,
    objects: &[Object],
    owners: &[Entry<file::Object>],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<Holder, String> {
    let object = objects
        .iter()
        .zip(owners)
        .position(|(object, owner)| {
            object.name == parent
                || inner(&object.name) == parent
                || owner.id.as_deref() == Some(parent)
        })
        .ok_or_else(|| format!("parent {parent} names no object"))?;
    let Some(node) = node else {
        return Ok(Holder {
            object,
            joint: None,
            rest: super::IDENTITY,
        });
    };
    let held = &objects[object];
    let missing = || format!("node {node} is not in object {parent}'s mesh");
    let mesh = meshes.get(&held.mesh).ok_or_else(missing)?;
    let rig = mesh
        .rig
        .as_ref()
        .and_then(|rig| Some((rig, rig.skeleton().joint(node)?)));
    if let Some((rig, joint)) = rig {
        return Ok(Holder {
            object,
            joint: Some(joint),
            rest: Pose::rest(rig.skeleton()).palette(rig.skeleton())[joint],
        });
    }
    let part = mesh
        .parts
        .iter()
        .find(|part| part.node == node)
        .ok_or_else(missing)?;
    Ok(Holder {
        object,
        joint: None,
        rest: part.transform,
    })
}

fn build_light(
    places: &Places,
    entry: &Entry<file::Light>,
    objects: &[Object],
    owners: &[Entry<file::Object>],
    meshes: &BTreeMap<String, SceneMesh>,
) -> Result<Light, SceneError> {
    let light = &entry.value;
    let name = entry.key.as_str();
    let steps = [Step::Named("light", inner(name))];
    let fail = |message: String| places.refusal(&entry.file, &steps, message);
    let spot = match light.kind.unwrap_or_default() {
        file::LightKind::Point => None,
        file::LightKind::Spot => {
            let [inner_deg, outer_deg] = light.cone.unwrap_or(SPOT_CONE);
            if !(inner_deg.is_finite()
                && outer_deg.is_finite()
                && 0.0 <= inner_deg
                && inner_deg <= outer_deg
                && outer_deg <= 90.0)
            {
                return Err(fail(format!(
                    "light {name}: cone needs 0 <= inner <= outer <= 90 degrees"
                )));
            }
            let direction = unit(light.direction.unwrap_or(SPOT_DIRECTION)).ok_or_else(|| {
                fail(format!(
                    "light {name}: direction needs a finite, nonzero length"
                ))
            })?;
            Some(Spot {
                direction,
                inner: inner_deg,
                outer: outer_deg,
            })
        }
    };
    let attach = match &light.parent {
        None if light.node.is_some() => {
            return Err(fail(format!("light {name}: node needs a parent object")));
        }
        None => None,
        Some(parent) => {
            let held = holder(parent, light.node.as_deref(), objects, owners, meshes)
                .map_err(|message| fail(format!("light {name}: {message}")))?;
            Some(Attach {
                object: held.object,
                joint: held.joint,
                rest: held.rest,
                at: light.at.unwrap_or([0.0; 3]),
                direction: spot.map_or(SPOT_DIRECTION, |spot| spot.direction),
            })
        }
    };
    let built = Light {
        position: light.position,
        color: light.color.unwrap_or(file::Light::COLOR),
        intensity: light.intensity,
        radius: light.radius.unwrap_or(file::Light::RADIUS),
        range: light.range.unwrap_or(file::Light::RANGE),
        shadow: light.shadow.unwrap_or(file::Light::SHADOW),
        spot,
        attach,
    };
    Ok(match attach {
        Some(attach) => {
            built.with_model(attach, multiply(objects[attach.object].model, attach.rest))
        }
        None => built,
    })
}

fn point(model: Matrix, at: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|row| {
        model[0][row] * at[0] + model[1][row] * at[1] + model[2][row] * at[2] + model[3][row]
    })
}

fn vector(model: Matrix, direction: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|row| {
        model[0][row] * direction[0] + model[1][row] * direction[1] + model[2][row] * direction[2]
    })
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(v, v).sqrt();
    (length.is_finite() && length > 1e-12).then(|| v.map(|x| x / length))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::{Scene, SceneError};
    use super::*;

    fn open(name: &str, lights: &str) -> Result<Scene, SceneError> {
        let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/scene-lights")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scenes");
        for entry in std::fs::read_dir(fixtures).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        let path = root.join("test.scene.toml");
        std::fs::write(
            &path,
            format!("format = 1\n\ninclude = [\"room.scene.toml\"]\n\n{lights}"),
        )
        .unwrap();
        let scene = Scene::open(&path);
        let _ = std::fs::remove_dir_all(&root);
        scene
    }

    #[test]
    fn a_light_on_a_mesh_node_sits_where_the_node_draws() {
        let scene = open(
            "node",
            "[[light]]\nname = \"cap lamp\"\nkind = \"spot\"\nparent = \"right pillar\"\nnode = \"Cap\"\nat = [0.0, 0.5, 0.0]\ndirection = [1.0, 0.0, 0.0]\ncone = [10.0, 20.0]\nintensity = 1.0\nshadow = false\n",
        )
        .unwrap();
        let light = scene.lights["cap lamp"];
        let cap = scene
            .draws()
            .items
            .into_iter()
            .find(|draw| draw.object == "right pillar" && draw.node == "Cap")
            .unwrap();
        let expected = point(cap.model, [0.0, 0.5, 0.0]);
        for k in 0..3 {
            assert!(
                (light.position[k] - expected[k]).abs() < 1e-5,
                "{light:?} {expected:?}"
            );
        }
        let spot = light.spot.unwrap();
        assert!((spot.direction[0] - 1.0).abs() < 1e-6);
        assert_eq!((spot.inner, spot.outer), (10.0, 20.0));
        let posed = scene.posed(&scene.camera_or_default(), 0.0);
        assert_eq!(light.placed(&posed).position, light.position);
    }

    #[test]
    fn an_unknown_parent_or_node_is_refused_by_name() {
        let message = |text: &str| open("refused", text).unwrap_err().message;
        let lamp = "[[light]]\nname = \"lamp\"\nintensity = 1.0\n";
        assert!(message(&format!("{lamp}parent = \"nowhere\"\n")).contains("nowhere"));
        assert!(
            message(&format!(
                "{lamp}parent = \"right pillar\"\nnode = \"Wheel\"\n"
            ))
            .contains("Wheel")
        );
        assert!(message(&format!("{lamp}node = \"Cap\"\n")).contains("parent"));
    }
}
