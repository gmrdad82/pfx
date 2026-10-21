use serde_json::{Value, json};

use super::{Writer, glb};

pub const GROUND_SIDE: f32 = 4.0;
pub const DECAL_SIDE: f32 = 1.0;
pub const DECAL_LIFT: f32 = 0.001;

fn quad(writer: &mut Writer, side: f32, height: f32, tiles: f32, material: usize) -> Value {
    let half = side * 0.5;
    let positions = [
        -half, height, -half, half, height, -half, half, height, half, -half, height, half,
    ];
    let normals = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let uvs = [0.0, 0.0, tiles, 0.0, tiles, tiles, 0.0, tiles];
    let position = writer.floats(&positions, "VEC3", 3, true);
    let normal = writer.floats(&normals, "VEC3", 3, false);
    let uv = writer.floats(&uvs, "VEC2", 2, false);
    let index = writer.indices(&[0, 2, 1, 0, 3, 2]);
    json!({
        "attributes": { "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv },
        "indices": index,
        "material": material,
    })
}

pub fn ground(ground: Value, decals: &[Value], images: &[Vec<u8>]) -> Vec<u8> {
    ground_lifted(ground, decals, images, DECAL_LIFT)
}

pub fn ground_lifted(ground: Value, decals: &[Value], images: &[Vec<u8>], lift: f32) -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut meshes = vec![json!({
        "name": "ground",
        "primitives": [quad(&mut writer, GROUND_SIDE, 0.0, GROUND_SIDE, 0)],
    })];
    let mut nodes = vec![json!({ "name": "ground", "mesh": 0 })];
    for (at, _) in decals.iter().enumerate() {
        let lift = lift * (at + 1) as f32;
        meshes.push(json!({
            "name": format!("decal{at}"),
            "primitives": [quad(&mut writer, DECAL_SIDE, lift, 1.0, at + 1)],
        }));
        nodes.push(json!({ "name": format!("decal{at}"), "mesh": at + 1 }));
    }
    let pictures: Vec<Value> = images
        .iter()
        .map(|png| json!({ "bufferView": writer.view(png), "mimeType": "image/png" }))
        .collect();
    let textures: Vec<Value> = (0..images.len())
        .map(|at| json!({ "source": at }))
        .collect();
    while !writer.bin.len().is_multiple_of(4) {
        writer.bin.push(0);
    }
    let mut materials = vec![ground];
    materials.extend(decals.iter().cloned());
    let mut document = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture ground" },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
        "materials": materials,
        "meshes": meshes,
        "buffers": [{ "byteLength": writer.bin.len() }],
        "bufferViews": writer.views,
        "accessors": writer.accessors,
    });
    if !images.is_empty() {
        document["images"] = json!(pictures);
        document["textures"] = json!(textures);
    }
    glb(&document.to_string(), &writer.bin)
}
