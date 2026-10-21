use serde_json::json;

pub struct KitMaps<'a> {
    pub base: Option<&'a [u8]>,
    pub normal: Option<&'a [u8]>,
    pub roughness: Option<&'a [u8]>,
    pub occlusion: Option<&'a [u8]>,
}

impl KitMaps<'_> {
    pub const NONE: KitMaps<'static> = KitMaps {
        base: None,
        normal: None,
        roughness: None,
        occlusion: None,
    };
}

pub fn kit_quad(material: &str, half: [f32; 2], base: [f32; 4], maps: KitMaps<'_>) -> Vec<u8> {
    let [hx, hy] = half;
    let positions: [[f32; 3]; 4] = [
        [-hx, -hy, 0.0],
        [hx, -hy, 0.0],
        [hx, hy, 0.0],
        [-hx, hy, 0.0],
    ];
    let normals = [[0.0f32, 0.0, 1.0]; 4];
    let uvs: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let mut bin = Vec::new();
    bin.extend(positions.iter().flatten().flat_map(|v| v.to_le_bytes()));
    bin.extend(normals.iter().flatten().flat_map(|v| v.to_le_bytes()));
    bin.extend(uvs.iter().flatten().flat_map(|v| v.to_le_bytes()));
    bin.extend(indices.iter().flat_map(|v| v.to_le_bytes()));
    let mut views = vec![
        json!({ "buffer": 0, "byteOffset": 0, "byteLength": 48 }),
        json!({ "buffer": 0, "byteOffset": 48, "byteLength": 48 }),
        json!({ "buffer": 0, "byteOffset": 96, "byteLength": 32 }),
        json!({ "buffer": 0, "byteOffset": 128, "byteLength": 24 }),
    ];
    let mut images = Vec::new();
    let mut textures = Vec::new();
    let mut pbr = json!({ "baseColorFactor": base, "metallicFactor": 0.0, "roughnessFactor": 1.0 });
    let mut extra = serde_json::Map::new();
    for (slot, png) in [
        ("base", maps.base),
        ("normal", maps.normal),
        ("roughness", maps.roughness),
        ("occlusion", maps.occlusion),
    ] {
        let Some(png) = png else {
            continue;
        };
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": png.len() }));
        bin.extend_from_slice(png);
        images.push(json!({ "bufferView": views.len() - 1, "mimeType": "image/png" }));
        textures.push(json!({ "source": images.len() - 1 }));
        let texture = json!({ "index": textures.len() - 1 });
        match slot {
            "base" => pbr["baseColorTexture"] = texture,
            "roughness" => pbr["metallicRoughnessTexture"] = texture,
            "normal" => {
                extra.insert("normalTexture".into(), texture);
            }
            _ => {
                extra.insert("occlusionTexture".into(), texture);
            }
        }
    }
    let mut made = json!({ "name": material, "pbrMetallicRoughness": pbr });
    for (key, value) in extra {
        made[key] = value;
    }
    let mut document = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": "panel" }],
        "meshes": [{ "primitives": [{
            "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
            "indices": 3,
            "material": 0,
        }]}],
        "materials": [made],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-hx, -hy, 0.0], "max": [hx, hy, 0.0] },
            { "bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5125, "count": 6, "type": "SCALAR" },
        ],
    });
    if !images.is_empty() {
        document["images"] = json!(images);
        document["textures"] = json!(textures);
    }
    super::glb(&document.to_string(), &bin)
}
