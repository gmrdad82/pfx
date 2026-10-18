use serde_json::{Value, json};

use super::{Writer, glb};

pub const LOD_RADIUS: f32 = 0.5;
pub const LOD_RINGS: [u32; 3] = [32, 12, 4];

pub fn lod_error(rings: u32) -> f32 {
    LOD_RADIUS * (1.0 - (std::f32::consts::PI / rings as f32).cos())
}

pub type SphereArrays = (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<[f32; 2]>, Vec<u32>);

pub fn lod_sphere(rings: u32) -> SphereArrays {
    let segments = rings * 2;
    let mut positions = Vec::new();
    let mut tangents = Vec::new();
    let mut uvs = Vec::new();
    for at in 0..=rings {
        let theta = std::f32::consts::PI * at as f32 / rings as f32;
        for segment in 0..=segments {
            let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
            positions.push([
                LOD_RADIUS * theta.sin() * phi.cos(),
                LOD_RADIUS * theta.cos(),
                LOD_RADIUS * theta.sin() * phi.sin(),
            ]);
            tangents.push([-phi.sin(), 0.0, phi.cos(), 1.0]);
            uvs.push([segment as f32 / segments as f32, at as f32 / rings as f32]);
        }
    }
    let row = segments + 1;
    let mut indices = Vec::new();
    for at in 0..rings {
        for segment in 0..segments {
            let a = at * row + segment;
            let b = a + row;
            if at != 0 {
                indices.extend([a, a + 1, b]);
            }
            if at != rings - 1 {
                indices.extend([a + 1, b + 1, b]);
            }
        }
    }
    (positions, tangents, uvs, indices)
}

pub const LOD_TARGETS: [&str; 2] = ["bulge", "slide"];
pub const LOD_SLIDE: f32 = 0.05;

pub type TargetArrays = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 3]>);

pub fn lod_targets(positions: &[[f32; 3]]) -> TargetArrays {
    let mut bulge = Vec::with_capacity(positions.len());
    let mut bulge_normals = Vec::with_capacity(positions.len());
    for position in positions {
        let n = position.map(|value| value / LOD_RADIUS);
        let lift = n[1].max(0.0) * 0.3;
        bulge.push(n.map(|value| value * lift * LOD_RADIUS));
        bulge_normals.push([-0.4 * n[0] * lift, 0.5 * lift, -0.4 * n[2] * lift]);
    }
    let slide = vec![[LOD_SLIDE, 0.0, 0.0]; positions.len()];
    (bulge, bulge_normals, slide)
}

pub fn lod_glb(name: &str, screen_coverage: Option<&[f32]>, with_errors: bool) -> Vec<u8> {
    levels_glb(name, &LOD_RINGS, screen_coverage, with_errors, None)
}

pub fn lod_morph_glb(name: &str, level_targets: &[&str]) -> Vec<u8> {
    levels_glb(name, &LOD_RINGS, None, true, Some(level_targets))
}

pub fn plain_glb(name: &str, rings: u32) -> Vec<u8> {
    levels_glb(name, &[rings], None, false, None)
}

fn sphere_mesh(writer: &mut Writer, name: String, rings: u32, names: Option<&[&str]>) -> Value {
    let (positions, tangents, uvs, indices) = lod_sphere(rings);
    let normals: Vec<[f32; 3]> = positions
        .iter()
        .map(|p| p.map(|value| value / LOD_RADIUS))
        .collect();
    let position = writer.floats(positions.as_flattened(), "VEC3", 3, true);
    let normal = writer.floats(normals.as_flattened(), "VEC3", 3, false);
    let tangent = writer.floats(tangents.as_flattened(), "VEC4", 4, false);
    let uv = writer.floats(uvs.as_flattened(), "VEC2", 2, false);
    let index = writer.indices(&indices);
    let mut targets = Vec::new();
    if let Some(names) = names {
        let (bulge, bulge_normals, slide) = lod_targets(&positions);
        for at in 0..names.len() {
            targets.push(if at % 2 == 0 {
                json!({
                    "POSITION": writer.floats(bulge.as_flattened(), "VEC3", 3, true),
                    "NORMAL": writer.floats(bulge_normals.as_flattened(), "VEC3", 3, false),
                })
            } else {
                json!({ "POSITION": writer.floats(slide.as_flattened(), "VEC3", 3, true) })
            });
        }
    }
    let mut primitive = json!({
        "attributes": {
            "POSITION": position,
            "NORMAL": normal,
            "TANGENT": tangent,
            "TEXCOORD_0": uv,
        },
        "indices": index,
        "material": 0,
    });
    if !targets.is_empty() {
        primitive["targets"] = json!(targets);
    }
    let mut entry = json!({ "name": name, "primitives": [primitive] });
    if let Some(names) = names {
        entry["weights"] = json!(vec![0.0; names.len()]);
        entry["extras"] = json!({ "targetNames": names });
    }
    entry
}

fn triangles(rings: u32) -> usize {
    (rings as usize * 2 - 2) * rings as usize * 2
}

fn levels_glb(
    name: &str,
    rings: &[u32],
    screen_coverage: Option<&[f32]>,
    with_errors: bool,
    level_targets: Option<&[&str]>,
) -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for (level, rings) in rings.iter().copied().enumerate() {
        let names: Option<&[&str]> = match level_targets {
            Some(_) if level == 0 => Some(&LOD_TARGETS),
            other => other,
        };
        let mesh_name = if level == 0 {
            name.to_string()
        } else {
            format!("{name}.lod{level}")
        };
        let entry = sphere_mesh(&mut writer, mesh_name, rings, names);
        meshes.push(entry);
        let mut node = if level == 0 {
            json!({ "name": name, "mesh": 0 })
        } else {
            json!({ "name": format!("{name}.lod{level}"), "mesh": level })
        };
        if level == 0
            && let Some(coverage) = screen_coverage
        {
            node["extras"] = json!({ "MSFT_screencoverage": coverage });
        }
        if level > 0 && with_errors {
            node["extras"] = json!({ "lod": {
                "of": name,
                "level": level,
                "triangles": triangles(rings),
                "target": 0.5_f32.powi(level as i32 * 2),
                "error": lod_error(rings),
            } });
        }
        nodes.push(node);
    }
    let ids: Vec<usize> = (1..nodes.len()).collect();
    if !ids.is_empty() {
        nodes[0]["extensions"] = json!({ "MSFT_lod": { "ids": ids } });
    }
    while !writer.bin.len().is_multiple_of(4) {
        writer.bin.push(0);
    }
    let document: Value = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture" },
        "extensionsUsed": if ids.is_empty() { json!([]) } else { json!(["MSFT_lod"]) },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": [{ "name": "skin" }],
        "buffers": [{ "byteLength": writer.bin.len() }],
        "bufferViews": writer.views,
        "accessors": writer.accessors,
    });
    glb(&document.to_string(), &writer.bin)
}

pub const GROUP_HEAD_RINGS: [u32; 3] = [32, 12, 4];
pub const GROUP_BEARD_RINGS: [u32; 2] = [24, 6];
pub const GROUP_BEARD_ERROR: f32 = 1.0;

pub fn lod_group_glb(group: Option<&str>) -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut meshes = Vec::new();
    for (level, rings) in GROUP_HEAD_RINGS.iter().copied().enumerate() {
        meshes.push(sphere_mesh(
            &mut writer,
            format!("head{level}"),
            rings,
            None,
        ));
    }
    for (level, rings) in GROUP_BEARD_RINGS.iter().copied().enumerate() {
        meshes.push(sphere_mesh(
            &mut writer,
            format!("beard{level}"),
            rings,
            None,
        ));
    }
    meshes.push(sphere_mesh(&mut writer, "brow".into(), 8, None));
    let head_level = |at: usize| {
        json!({ "name": format!("head.lod{at}"), "mesh": at, "extras": { "lod": {
            "of": "head",
            "level": at,
            "triangles": triangles(GROUP_HEAD_RINGS[at]),
            "error": lod_error(GROUP_HEAD_RINGS[at]),
        } } })
    };
    let mut beard_lod = json!({
        "of": "beard",
        "level": 1,
        "triangles": triangles(GROUP_BEARD_RINGS[1]),
        "error": GROUP_BEARD_ERROR,
    });
    if let Some(group) = group {
        beard_lod["group"] = json!(group);
    }
    let nodes = json!([
        { "name": "head", "mesh": 0, "extensions": { "MSFT_lod": { "ids": [1, 2] } } },
        head_level(1),
        head_level(2),
        { "name": "beard", "mesh": 3, "translation": [0.0, -0.35, 0.3], "scale": [0.4, 0.4, 0.4],
          "extensions": { "MSFT_lod": { "ids": [4] } } },
        { "name": "beard.lod1", "mesh": 4, "extras": { "lod": beard_lod } },
        { "name": "brow", "mesh": 5, "translation": [0.0, 0.3, 0.4], "scale": [0.2, 0.2, 0.2] },
    ]);
    while !writer.bin.len().is_multiple_of(4) {
        writer.bin.push(0);
    }
    let document: Value = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture" },
        "extensionsUsed": ["MSFT_lod"],
        "scene": 0,
        "scenes": [{ "nodes": [0, 3, 5] }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": [{ "name": "skin" }],
        "buffers": [{ "byteLength": writer.bin.len() }],
        "bufferViews": writer.views,
        "accessors": writer.accessors,
    });
    glb(&document.to_string(), &writer.bin)
}
