use crate::{Error, fail, normal_map};
use pfx_glb::{Lod, Node, Targets};
use pfx_mesh::{Mesh, Morph, SimplifySettings, simplify_with, tangents};
use pfx_recipe::{LodTarget, Object};
use serde_json::json;

pub const DISTANCE_SHARE: f64 = 0.01;
pub const DISTANCE_ERRORS: f64 = 2.0;
pub const RISE: f64 = 1e-3;

pub(crate) struct Authored<'a> {
    pub(crate) file: &'a str,
    pub(crate) nodes: &'a [Node],
}

struct Level {
    mesh: Mesh,
    morph: Option<Morph>,
    error: f64,
    target: usize,
}

fn names(mesh: &Mesh) -> Vec<&str> {
    mesh.slots
        .iter()
        .map(|slot| slot.material.as_str())
        .collect()
}

fn read(
    object: &Object,
    k: usize,
    master: &Node,
    base: &Mesh,
    authored: &Authored,
) -> Result<Level, Error> {
    let file = authored.file;
    let refuse = |text: String| fail(object, &format!("lod[{k}].path"), format!("{file}: {text}"));
    let Some(node) = authored.nodes.iter().find(|n| n.name == master.name) else {
        return Err(refuse(format!(
            "has no node `{}`, expected the object's nodes and slots",
            master.name
        )));
    };
    let mesh = node.targets.as_ref().map_or(&node.mesh, |t| &t.base);
    if names(mesh) != names(base) {
        return Err(refuse(format!(
            "the node `{}` has the slots [{}], expected the object's [{}]",
            master.name,
            names(mesh).join(", "),
            names(base).join(", ")
        )));
    }
    let wanted = master.targets.as_ref().map(|t| &t.morph.names);
    let found = node.targets.as_ref().map(|t| &t.morph.names);
    if wanted != found {
        let list = |n: Option<&Vec<String>>| n.map_or(String::new(), |n| n.join(", "));
        return Err(refuse(format!(
            "the node `{}` has the morph targets [{}], expected the object's [{}]",
            master.name,
            list(found),
            list(wanted)
        )));
    }
    if base.uvs.is_some() && mesh.uvs.is_none() {
        return Err(refuse(format!(
            "the node `{}` has no TEXCOORD_0, expected one as the object's mesh has",
            master.name
        )));
    }
    Ok(Level {
        mesh: mesh.clone(),
        morph: node.targets.as_ref().map(|t| Morph {
            weights: master
                .targets
                .as_ref()
                .map_or_else(|| t.morph.weights.clone(), |m| m.morph.weights.clone()),
            ..t.morph.clone()
        }),
        error: normal_map::deviation(base, mesh, reach(base)),
        target: mesh.triangle_count(),
    })
}

fn reach(mesh: &Mesh) -> f32 {
    (diagonal(mesh) * DISTANCE_SHARE * 10.0) as f32
}

pub(crate) fn levels(
    object: &Object,
    master: &Node,
    authored: &[Option<Authored>],
) -> Result<Vec<Lod>, Error> {
    if object.lod.is_empty() {
        return Ok(Vec::new());
    }
    let base = master.targets.as_ref().map_or(&master.mesh, |t| &t.base);
    let morph = master.targets.as_ref().map(|t| &t.morph);
    let triangles = base.triangle_count();
    let mut out = Vec::with_capacity(object.lod.len());
    let mut last = 0.0f64;
    for (k, level) in object.lod.iter().enumerate() {
        let at = |key: &str| format!("lod[{k}].{key}");
        let file = match &level.target {
            LodTarget::File(file) => Some(file.as_str()),
            _ => None,
        };
        let simplified = |target: usize, key: &str| -> Result<Level, Error> {
            if target >= triangles {
                return Err(fail(
                    object,
                    &at(key),
                    format!(
                        "{target} triangles is not below the object's {triangles}, expected fewer"
                    ),
                ));
            }
            let reduced = simplify_with(
                base,
                target,
                SimplifySettings {
                    lock_borders: level.lock_borders,
                },
            );
            let mut morph = morph.cloned();
            if let Some(m) = morph.as_mut() {
                m.remap(&reduced.sources);
            }
            Ok(Level {
                mesh: reduced.mesh,
                morph,
                error: f64::from(reduced.error),
                target,
            })
        };
        let made = match &level.target {
            LodTarget::Triangles(n) => simplified(*n as usize, "triangles")?,
            LodTarget::Ratio(r) => {
                simplified(((triangles as f64 * r).round() as usize).max(1), "ratio")?
            }
            LodTarget::File(_) => match authored.get(k).and_then(Option::as_ref) {
                Some(authored) => read(object, k, master, base, authored)?,
                None => {
                    return Err(fail(
                        object,
                        &at("path"),
                        "the level's file was not read, expected a glTF file",
                    ));
                }
            },
        };
        let Level {
            mut mesh,
            mut morph,
            mut error,
            target,
        } = made;
        if file.is_some() && error <= last {
            error = last * (1.0 + RISE) + f64::from(f32::EPSILON);
        }
        last = last.max(error);
        let mut tangent_list = None;
        let mut png = None;
        if level.bake_normals || level.bake_material {
            let key = if level.bake_material {
                "bake_material"
            } else {
                "bake_normals"
            };
            let Some(spaced) = tangents(&mesh) else {
                return Err(fail(
                    object,
                    &at(key),
                    "the level has no TEXCOORD_0 or no normals to bake a normal map on, expected uv other than \"none\" and smooth normals",
                ));
            };
            if let Some(m) = morph.as_mut() {
                m.remap(&spaced.sources);
            }
            if !level.bake_material {
                let distance = level
                    .bake_distance
                    .unwrap_or_else(|| default_distance(base, error as f32));
                let reach = level.bake_reach.unwrap_or(distance);
                let map = normal_map::bake(
                    base,
                    &spaced.mesh,
                    &spaced.tangents,
                    level.normal_size,
                    (distance as f32, reach as f32),
                )
                .map_err(|text| fail(object, &at("bake_normals"), text))?;
                png = Some(map);
            }
            mesh = spaced.mesh;
            tangent_list = Some(spaced.tangents);
        }
        let mut extras = json!({
            "of": master.name,
            "level": k + 1,
            "triangles": mesh.triangle_count(),
            "target": target,
            "error": error,
        });
        if png.is_some() {
            extras["normal_size"] = json!(level.normal_size);
        }
        if level.bake_material {
            extras["material_size"] = json!(level.material_size);
        }
        if let Some(file) = file {
            extras["path"] = json!(file);
        }
        let targets = morph.map(|morph| {
            let unposed = mesh.clone();
            morph.pose(&mut mesh);
            Box::new(Targets {
                base: unposed,
                morph,
            })
        });
        out.push(Lod {
            of: 0,
            node: Node {
                name: format!("{}.lod{}", master.name, k + 1),
                mesh,
                transform: master.transform,
                keep_transform: master.keep_transform,
                targets,
            },
            tangents: tangent_list,
            normal_map: png,
            extras: json!({ "lod": extras }),
        });
    }
    Ok(out)
}

pub(crate) fn authored(lod: &Lod) -> bool {
    lod.extras
        .get("lod")
        .and_then(|lod| lod.get("path"))
        .is_some()
}

fn diagonal(mesh: &Mesh) -> f64 {
    pfx_mesh::bounds(mesh).map_or(0.0, |[lo, hi]| {
        (glam::DVec3::from(hi) - glam::DVec3::from(lo)).length()
    })
}

pub(crate) fn default_distance(mesh: &Mesh, error: f32) -> f64 {
    (diagonal(mesh) * DISTANCE_SHARE).max(f64::from(error) * DISTANCE_ERRORS)
}
