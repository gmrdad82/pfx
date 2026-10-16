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

pub fn lod_glb(name: &str, screen_coverage: Option<&[f32]>, with_errors: bool) -> Vec<u8> {
    levels_glb(name, &LOD_RINGS, screen_coverage, with_errors)
}

pub fn plain_glb(name: &str, rings: u32) -> Vec<u8> {
    levels_glb(name, &[rings], None, false)
}

fn levels_glb(
    name: &str,
    rings: &[u32],
    screen_coverage: Option<&[f32]>,
    with_errors: bool,
) -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for (level, rings) in rings.iter().copied().enumerate() {
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
        meshes.push(json!({
            "name": if level == 0 { name.to_string() } else { format!("{name}.lod{level}") },
            "primitives": [{
                "attributes": {
                    "POSITION": position,
                    "NORMAL": normal,
                    "TANGENT": tangent,
                    "TEXCOORD_0": uv,
                },
                "indices": index,
                "material": 0,
            }],
        }));
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
                "triangles": indices.len() / 3,
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
