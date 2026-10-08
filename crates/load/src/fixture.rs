use pfx_core::anim::fixture::{column, rig};
use pfx_core::anim::{Interpolation, Property};
use serde_json::{Value, json};

struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        self.views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": bytes.len(),
        }));
        self.views.len() - 1
    }

    fn floats(&mut self, values: &[f32], kind: &str, width: usize, bounds: bool) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let view = self.view(&bytes);
        let mut accessor = json!({
            "bufferView": view,
            "componentType": 5126,
            "count": values.len() / width,
            "type": kind,
        });
        if bounds {
            let column = |k: usize| values.iter().skip(k).step_by(width).copied();
            let min: Vec<f32> = (0..width)
                .map(|k| column(k).fold(f32::INFINITY, f32::min))
                .collect();
            let max: Vec<f32> = (0..width)
                .map(|k| column(k).fold(f32::NEG_INFINITY, f32::max))
                .collect();
            accessor["min"] = json!(min);
            accessor["max"] = json!(max);
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn shorts(&mut self, values: &[[u16; 4]]) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let view = self.view(&bytes);
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": 5123,
            "count": values.len(),
            "type": "VEC4",
        }));
        self.accessors.len() - 1
    }

    fn indices(&mut self, values: &[u32]) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let view = self.view(&bytes);
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": 5125,
            "count": values.len(),
            "type": "SCALAR",
        }));
        self.accessors.len() - 1
    }
}

pub const SKY_HDR: &[u8] = include_bytes!("../tests/data/sky.hdr");

pub fn skinned_column(bones: usize) -> Vec<u8> {
    let bones = bones.max(1);
    let column = column(bones);
    let rig = rig(bones);
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let position = writer.floats(column.positions.as_flattened(), "VEC3", 3, true);
    let normal = writer.floats(column.normals.as_flattened(), "VEC3", 3, false);
    let tangent = writer.floats(column.tangents.as_flattened(), "VEC4", 4, false);
    let uv = writer.floats(column.uvs.as_flattened(), "VEC2", 2, false);
    let joints = writer.shorts(&column.joints);
    let weights = writer.floats(column.weights.as_flattened(), "VEC4", 4, false);
    let indices = writer.indices(&column.indices);
    let binds: Vec<f32> = rig
        .skeleton()
        .inverse_binds()
        .iter()
        .flat_map(|matrix| matrix.as_flattened().to_vec())
        .collect();
    let binds = writer.floats(&binds, "MAT4", 16, false);
    let mut nodes: Vec<Value> = rig
        .skeleton()
        .joints()
        .iter()
        .enumerate()
        .map(|(index, joint)| {
            let mut node = json!({
                "name": joint.name,
                "translation": joint.rest.translation,
                "rotation": joint.rest.rotation,
                "scale": joint.rest.scale,
            });
            if index + 1 < bones {
                node["children"] = json!([index + 1]);
            }
            node
        })
        .collect();
    nodes.push(json!({ "name": "column", "mesh": 0, "skin": 0 }));
    let mut animations = Vec::new();
    for clip in rig.clips() {
        let mut samplers = Vec::new();
        let mut channels = Vec::new();
        for channel in clip.channels() {
            let input = writer.floats(&channel.times, "SCALAR", 1, true);
            let (kind, width, path) = match channel.property {
                Property::Translation => ("VEC3", 3, "translation"),
                Property::Rotation => ("VEC4", 4, "rotation"),
                Property::Scale => ("VEC3", 3, "scale"),
            };
            let output = writer.floats(&channel.values, kind, width, false);
            let interpolation = match channel.interpolation {
                Interpolation::Linear => "LINEAR",
                Interpolation::Step => "STEP",
                Interpolation::CubicSpline => "CUBICSPLINE",
            };
            channels.push(json!({
                "sampler": samplers.len(),
                "target": { "node": channel.joint, "path": path },
            }));
            samplers.push(json!({
                "input": input,
                "output": output,
                "interpolation": interpolation,
            }));
        }
        animations.push(json!({
            "name": clip.name(),
            "samplers": samplers,
            "channels": channels,
        }));
    }
    while !writer.bin.len().is_multiple_of(4) {
        writer.bin.push(0);
    }
    let document = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture" },
        "scene": 0,
        "scenes": [{ "nodes": [0, bones] }],
        "nodes": nodes,
        "meshes": [{
            "name": "column",
            "primitives": [{
                "attributes": {
                    "POSITION": position,
                    "NORMAL": normal,
                    "TANGENT": tangent,
                    "TEXCOORD_0": uv,
                    "JOINTS_0": joints,
                    "WEIGHTS_0": weights,
                },
                "indices": indices,
            }],
        }],
        "skins": [{ "joints": (0..bones).collect::<Vec<_>>(), "inverseBindMatrices": binds }],
        "animations": animations,
        "buffers": [{ "byteLength": writer.bin.len() }],
        "bufferViews": writer.views,
        "accessors": writer.accessors,
    });
    glb(&document.to_string(), &writer.bin)
}

pub fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

pub fn atlas_rgba(columns: u32, rows: u32, cell: u32) -> Vec<u8> {
    let width = columns * cell;
    let height = rows * cell;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.extend(atlas_colour(y / cell * columns + x / cell));
        }
    }
    rgba
}

pub fn atlas_colour(index: u32) -> [u8; 4] {
    [
        (40 + index * 53 % 200) as u8,
        (60 + index * 97 % 180) as u8,
        (30 + index * 31 % 210) as u8,
        255,
    ]
}

pub fn atlas_png(columns: u32, rows: u32, cell: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, columns * cell, rows * cell);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().expect("the atlas header encodes");
        writer
            .write_image_data(&atlas_rgba(columns, rows, cell))
            .expect("the atlas encodes");
    }
    bytes
}
