use crate::subdiv::{
    self, Boundary, Cage, Corner, Crease, Creasing, INFINITE, MATERIAL, MAX_LEVELS, Options,
    UvInterpolation,
};
use crate::{Call, Generated, Piece};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub const NAME: &str = "pfx/cage";

const PARAMS: [&str; 9] = [
    "points",
    "faces",
    "creases",
    "corners",
    "uvs",
    "subdivide",
    "boundary",
    "creasing",
    "uv_interpolation",
];

pub struct Parsed {
    pub cage: Cage,
    pub levels: u32,
    pub options: Options,
}

pub fn generate(call: &Call) -> Result<Generated, String> {
    let Parsed {
        cage,
        levels,
        options,
    } = parse(call.params)?;
    let mesh = subdiv::subdivide(&cage, levels, &options).map_err(|e| format!("params.{e}"))?;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k] as f64);
            hi[k] = hi[k].max(p[k] as f64);
        }
    }
    let Value::Object(facts) = json!({
        "cage": {
            "points": cage.points.len(),
            "faces": cage.faces.len(),
            "triangles": mesh.indices.len() / 3,
            "size": [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        }
    }) else {
        unreachable!()
    };
    Ok(Generated {
        pieces: vec![Piece {
            name: call.object.name.clone(),
            mesh,
        }],
        facts,
        materials: materials(),
        ..Generated::default()
    })
}

pub fn materials() -> BTreeMap<String, Value> {
    BTreeMap::from([(
        MATERIAL.to_string(),
        json!({ "family": "plain", "base": [0.62, 0.6, 0.57], "roughness": 0.55 }),
    )])
}

pub fn parse(params: &BTreeMap<String, Value>) -> Result<Parsed, String> {
    if let Some(key) = params.keys().find(|k| !PARAMS.contains(&k.as_str())) {
        return Err(format!(
            "params.{key}: unknown param, expected one of: {}",
            PARAMS.join(", ")
        ));
    }
    let points = list(params.get("points"), "points", "[[x, y, z], …]")?
        .iter()
        .enumerate()
        .map(|(i, v)| {
            numbers::<3>(v).ok_or_else(|| format!("params.points[{i}]: expected [x, y, z]"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let faces = list(params.get("faces"), "faces", "[[i, j, k, …], …]")?
        .iter()
        .enumerate()
        .map(|(f, v)| {
            v.as_array()
                .and_then(|a| a.iter().map(index).collect::<Option<Vec<u32>>>())
                .ok_or_else(|| format!("params.faces[{f}]: expected a list of point indices"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let creases = match params.get("creases") {
        None => Vec::new(),
        Some(v) => list(Some(v), "creases", "[{ edge = [i, j], sharpness }, …]")?
            .iter()
            .enumerate()
            .map(|(c, v)| {
                let key = format!("creases[{c}]");
                let t = table(v, &key, &["edge", "sharpness"])?;
                let edge = t
                    .get("edge")
                    .and_then(|e| e.as_array())
                    .filter(|e| e.len() == 2)
                    .and_then(|e| Some([index(&e[0])?, index(&e[1])?]))
                    .ok_or_else(|| {
                        format!("params.{key}.edge: expected [i, j], two point indices")
                    })?;
                Ok(Crease {
                    edge,
                    sharpness: sharpness(t, &key)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let corners = match params.get("corners") {
        None => Vec::new(),
        Some(v) => list(Some(v), "corners", "[{ point = i, sharpness }, …]")?
            .iter()
            .enumerate()
            .map(|(c, v)| {
                let key = format!("corners[{c}]");
                let t = table(v, &key, &["point", "sharpness"])?;
                let point = t
                    .get("point")
                    .and_then(index)
                    .ok_or_else(|| format!("params.{key}.point: expected a point index"))?;
                Ok(Corner {
                    point,
                    sharpness: sharpness(t, &key)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let uvs = match params.get("uvs") {
        None => None,
        Some(v) => Some(
            list(Some(v), "uvs", "[[[u, v], …], …], one list per face")?
                .iter()
                .enumerate()
                .map(|(f, v)| {
                    v.as_array()
                        .and_then(|a| a.iter().map(numbers::<2>).collect::<Option<Vec<_>>>())
                        .ok_or_else(|| format!("params.uvs[{f}]: expected a list of [u, v] pairs"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let levels = match params.get("subdivide") {
        None => 2,
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= u64::from(MAX_LEVELS))
            .ok_or_else(|| {
                format!("params.subdivide: expected a whole number from 0 to {MAX_LEVELS}")
            })? as u32,
    };
    let boundary = match params.get("boundary").map(|v| v.as_str()) {
        None | Some(Some("edges")) => Boundary::Edges,
        Some(Some("corners")) => Boundary::EdgesAndCorners,
        Some(_) => return Err("params.boundary: expected \"edges\" or \"corners\"".into()),
    };
    let creasing = match params.get("creasing").map(|v| v.as_str()) {
        None | Some(Some("chaikin")) => Creasing::Chaikin,
        Some(Some("uniform")) => Creasing::Uniform,
        Some(_) => return Err("params.creasing: expected \"chaikin\" or \"uniform\"".into()),
    };
    let uv_interpolation = match params.get("uv_interpolation").map(|v| v.as_str()) {
        None | Some(Some("smooth")) => UvInterpolation::Smooth,
        Some(Some("linear")) => UvInterpolation::Linear,
        Some(_) => {
            return Err("params.uv_interpolation: expected \"smooth\" or \"linear\"".into());
        }
    };
    Ok(Parsed {
        cage: Cage {
            points,
            faces,
            creases,
            corners,
            uvs,
        },
        levels,
        options: Options {
            boundary,
            creasing,
            uvs: uv_interpolation,
        },
    })
}

fn list<'a>(value: Option<&'a Value>, key: &str, shape: &str) -> Result<&'a Vec<Value>, String> {
    value
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("params.{key}: expected {shape}"))
}

fn table<'a>(value: &'a Value, key: &str, keys: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let map = value
        .as_object()
        .ok_or_else(|| format!("params.{key}: expected {{ {} }}", keys.join(", ")))?;
    if let Some(unknown) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!(
            "params.{key}.{unknown}: unknown key, expected one of: {}",
            keys.join(", ")
        ));
    }
    Ok(map)
}

fn sharpness(t: &Map<String, Value>, key: &str) -> Result<f64, String> {
    match t.get("sharpness") {
        None => Ok(INFINITE),
        Some(v) => v
            .as_f64()
            .filter(|s| s.is_finite() && (0.0..=INFINITE).contains(s))
            .ok_or_else(|| {
                format!("params.{key}.sharpness: expected a number from 0 to {INFINITE}")
            }),
    }
}

fn index(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|i| u32::try_from(i).ok())
}

fn numbers<const N: usize>(v: &Value) -> Option<[f64; N]> {
    let a = v.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(a) {
        *slot = item.as_f64().filter(|n| n.is_finite())?;
    }
    Some(out)
}
