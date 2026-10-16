use serde_json::json;

use super::{Writer, glb};

pub const TIMELINE_CLIP: &str = "timeline";
pub const TIMELINE_NODE: &str = "box.body";
pub const TIMELINE_KEYS: usize = 9;
pub const TIMELINE_STEP: f32 = 0.125;
pub const TIMELINE_TRAVEL: [f32; 2] = [-0.8, 0.8];

pub fn timeline_x(time: f32) -> f32 {
    let [from, to] = TIMELINE_TRAVEL;
    let span = TIMELINE_STEP * (TIMELINE_KEYS - 1) as f32;
    from + (to - from) * (time / span).clamp(0.0, 1.0)
}

pub fn timeline_glb() -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let corners: [[f32; 3]; 8] = [
        [-0.25, -0.25, -0.25],
        [0.25, -0.25, -0.25],
        [0.25, 0.25, -0.25],
        [-0.25, 0.25, -0.25],
        [-0.25, -0.25, 0.25],
        [0.25, -0.25, 0.25],
        [0.25, 0.25, 0.25],
        [-0.25, 0.25, 0.25],
    ];
    let faces: [([usize; 4], [f32; 3]); 6] = [
        ([4, 5, 6, 7], [0.0, 0.0, 1.0]),
        ([1, 0, 3, 2], [0.0, 0.0, -1.0]),
        ([5, 1, 2, 6], [1.0, 0.0, 0.0]),
        ([0, 4, 7, 3], [-1.0, 0.0, 0.0]),
        ([7, 6, 2, 3], [0.0, 1.0, 0.0]),
        ([0, 1, 5, 4], [0.0, -1.0, 0.0]),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (corner, normal) in faces {
        let base = (positions.len() / 3) as u32;
        for index in corner {
            positions.extend_from_slice(&corners[index]);
            normals.extend_from_slice(&normal);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let position = writer.floats(&positions, "VEC3", 3, true);
    let normal = writer.floats(&normals, "VEC3", 3, false);
    let indices = writer.indices(&indices);
    let times: Vec<f32> = (0..TIMELINE_KEYS)
        .map(|key| key as f32 * TIMELINE_STEP)
        .collect();
    let input = writer.floats(&times, "SCALAR", 1, true);
    let travel: Vec<f32> = times
        .iter()
        .flat_map(|&time| [timeline_x(time), 0.0, 0.0])
        .collect();
    let travel = writer.floats(&travel, "VEC3", 3, false);
    let spin: Vec<f32> = (0..TIMELINE_KEYS)
        .flat_map(|key| {
            let half = key as f32 * 0.1;
            [0.0, half.sin(), 0.0, half.cos()]
        })
        .collect();
    let spin = writer.floats(&spin, "VEC4", 4, false);
    let document = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": TIMELINE_NODE, "mesh": 0 }],
        "meshes": [{ "name": "box", "primitives": [{
            "attributes": { "POSITION": position, "NORMAL": normal },
            "indices": indices,
        }] }],
        "animations": [{
            "name": TIMELINE_CLIP,
            "samplers": [
                { "input": input, "output": travel, "interpolation": "LINEAR" },
                { "input": input, "output": spin, "interpolation": "LINEAR" },
            ],
            "channels": [
                { "sampler": 0, "target": { "node": 0, "path": "translation" } },
                { "sampler": 1, "target": { "node": 0, "path": "rotation" } },
            ],
        }],
        "buffers": [{ "byteLength": writer.bin.len() }],
        "bufferViews": writer.views,
        "accessors": writer.accessors,
    });
    glb(&document.to_string(), &writer.bin)
}
