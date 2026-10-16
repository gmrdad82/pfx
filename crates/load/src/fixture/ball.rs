use serde_json::{Value, json};

use super::{Writer, glb};

pub const BALL_RINGS: u32 = 32;
pub const BALL_SEGMENTS: u32 = 64;
pub const BALL_RADIUS: f32 = 0.5;

pub fn ball_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("the map header encodes");
        writer.write_image_data(rgba).expect("the map encodes");
    }
    bytes
}

pub fn ball(material: Value, images: &[Vec<u8>]) -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut uvs1 = Vec::new();
    for ring in 0..=BALL_RINGS {
        let theta = std::f32::consts::PI * ring as f32 / BALL_RINGS as f32;
        for segment in 0..=BALL_SEGMENTS {
            let phi = std::f32::consts::TAU * segment as f32 / BALL_SEGMENTS as f32;
            let normal = [
                theta.sin() * phi.sin(),
                theta.cos(),
                theta.sin() * phi.cos(),
            ];
            positions.extend(normal.map(|value| value * BALL_RADIUS));
            normals.extend(normal);
            let u = segment as f32 / BALL_SEGMENTS as f32;
            let v = ring as f32 / BALL_RINGS as f32;
            uvs.extend([u, v]);
            uvs1.extend([v, u]);
        }
    }
    let row = BALL_SEGMENTS + 1;
    let mut indices = Vec::new();
    for ring in 0..BALL_RINGS {
        for segment in 0..BALL_SEGMENTS {
            let a = ring * row + segment;
            let b = a + row;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let position = writer.floats(&positions, "VEC3", 3, true);
    let normal = writer.floats(&normals, "VEC3", 3, false);
    let uv = writer.floats(&uvs, "VEC2", 2, false);
    let uv1 = writer.floats(&uvs1, "VEC2", 2, false);
    let index = writer.indices(&indices);
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
    let mut document = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture ball" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "ball", "mesh": 0 }],
        "materials": [material],
        "meshes": [{
            "name": "ball",
            "primitives": [{
                "attributes": {
                    "POSITION": position,
                    "NORMAL": normal,
                    "TEXCOORD_0": uv,
                    "TEXCOORD_1": uv1,
                },
                "indices": index,
                "material": 0,
            }],
        }],
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
