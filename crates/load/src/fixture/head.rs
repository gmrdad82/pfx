use serde_json::{Value, json};

use super::{Writer, glb};

pub const HEAD_RINGS: u32 = 16;
pub const HEAD_SEGMENTS: u32 = 32;
pub const HEAD_VERTICES: usize = ((HEAD_RINGS + 1) * (HEAD_SEGMENTS + 1)) as usize;
pub const HEAD_TARGETS: [&str; 3] = ["jaw open", "smile", "brow raise"];
pub const HEAD_WEIGHTS: [f32; 3] = [0.0, 0.3, 0.0];
pub const HEAD_TIMES: [f32; 4] = [0.0, 0.25, 0.5, 0.75];
pub const HEAD_KEYS: [[f32; 3]; 4] = [
    [0.0, 0.3, 0.0],
    [1.0, 0.3, 0.5],
    [0.2, 0.3, 1.0],
    [0.0, 0.3, 0.0],
];

type Deltas = Vec<[f32; 3]>;

fn ring(ring: u32, segment: u32) -> u32 {
    ring * (HEAD_SEGMENTS + 1) + segment
}

fn head_positions() -> Vec<[f32; 3]> {
    let mut positions = Vec::with_capacity(HEAD_VERTICES);
    for at in 0..=HEAD_RINGS {
        let theta = std::f32::consts::PI * at as f32 / HEAD_RINGS as f32;
        for segment in 0..=HEAD_SEGMENTS {
            let phi = std::f32::consts::TAU * segment as f32 / HEAD_SEGMENTS as f32;
            positions.push([
                0.5 * theta.sin() * phi.cos(),
                0.6 * theta.cos(),
                0.5 * theta.sin() * phi.sin(),
            ]);
        }
    }
    positions
}

fn head_indices() -> Vec<u32> {
    let mut indices = Vec::new();
    for at in 0..HEAD_RINGS {
        for segment in 0..HEAD_SEGMENTS {
            let a = ring(at, segment);
            let b = ring(at + 1, segment);
            if at != 0 {
                indices.extend([a, a + 1, b]);
            }
            if at != HEAD_RINGS - 1 {
                indices.extend([a + 1, b + 1, b]);
            }
        }
    }
    indices
}

fn welded(index: usize) -> usize {
    let row = (HEAD_SEGMENTS + 1) as usize;
    let at = index / row;
    let segment = index % row;
    if at == 0 {
        0
    } else if at == HEAD_RINGS as usize || segment == HEAD_SEGMENTS as usize {
        at * row
    } else {
        index
    }
}

fn normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut sums = vec![[0.0f32; 3]; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| positions[triangle[k] as usize]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for &corner in triangle {
            let sum = &mut sums[welded(corner as usize)];
            for k in 0..3 {
                sum[k] += face[k];
            }
        }
    }
    (0..positions.len())
        .map(|index| {
            let sum = sums[welded(index)];
            let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
            if length > 0.0 {
                sum.map(|value| value / length)
            } else {
                [0.0, 1.0, 0.0]
            }
        })
        .collect()
}

fn falloff(position: [f32; 3], center: [f32; 3], radius: f32) -> f32 {
    let distance = (0..3)
        .map(|k| (position[k] - center[k]).powi(2))
        .sum::<f32>()
        / (radius * radius);
    (1.0 - distance).max(0.0).powi(2)
}

fn delta(position: [f32; 3], target: usize) -> [f32; 3] {
    match target {
        0 => {
            let w = falloff(position, [0.0, -0.4, 0.35], 0.4);
            [0.0, -0.12 * w, 0.03 * w]
        }
        1 => {
            let side = if position[0] < 0.0 { -1.0 } else { 1.0 };
            let w = falloff(position, [0.18 * side, -0.22, 0.42], 0.16);
            [0.03 * side * w, 0.05 * w, -0.01 * w]
        }
        _ => {
            let side = if position[0] < 0.0 { -1.0 } else { 1.0 };
            let w = falloff(position, [0.15 * side, 0.24, 0.42], 0.16);
            [0.0, 0.06 * w, 0.01 * w]
        }
    }
}

fn head_targets(
    positions: &[[f32; 3]],
    indices: &[u32],
    base: &[[f32; 3]],
) -> Vec<(Deltas, Deltas)> {
    (0..HEAD_TARGETS.len())
        .map(|target| {
            let deltas: Vec<[f32; 3]> = positions.iter().map(|p| delta(*p, target)).collect();
            let moved: Vec<[f32; 3]> = positions
                .iter()
                .zip(&deltas)
                .map(|(p, d)| [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
                .collect();
            let turned: Vec<[f32; 3]> = normals(&moved, indices)
                .iter()
                .zip(base)
                .map(|(n, b)| {
                    let d = [n[0] - b[0], n[1] - b[1], n[2] - b[2]];
                    if d.iter().all(|value| value.abs() < 1e-6) {
                        [0.0; 3]
                    } else {
                        d
                    }
                })
                .collect();
            (deltas, turned)
        })
        .collect()
}

fn bounds(values: &[[f32; 3]]) -> (Vec<f32>, Vec<f32>) {
    let min = (0..3)
        .map(|k| values.iter().map(|v| v[k]).fold(f32::INFINITY, f32::min))
        .collect();
    let max = (0..3)
        .map(|k| {
            values
                .iter()
                .map(|v| v[k])
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect();
    (min, max)
}

fn sparse(writer: &mut Writer, values: &[[f32; 3]], keep: &[u32]) -> usize {
    let index_bytes: Vec<u8> = keep.iter().flat_map(|index| index.to_le_bytes()).collect();
    let value_bytes: Vec<u8> = keep
        .iter()
        .flat_map(|&index| values[index as usize])
        .flat_map(f32::to_le_bytes)
        .collect();
    let indices = writer.view(&index_bytes);
    let kept = writer.view(&value_bytes);
    let (min, max) = bounds(values);
    writer.accessors.push(json!({
        "componentType": 5126,
        "count": values.len(),
        "type": "VEC3",
        "min": min,
        "max": max,
        "sparse": {
            "count": keep.len(),
            "indices": { "bufferView": indices, "componentType": 5125 },
            "values": { "bufferView": kept },
        },
    }));
    writer.accessors.len() - 1
}

fn hat() -> (Vec<[f32; 3]>, Vec<u32>) {
    let sides = 12u32;
    let mut positions = vec![[0.0, 0.35, 0.0], [0.0, 0.0, 0.0]];
    for side in 0..sides {
        let angle = std::f32::consts::TAU * side as f32 / sides as f32;
        positions.push([0.3 * angle.cos(), 0.0, 0.3 * angle.sin()]);
    }
    let mut indices = Vec::new();
    for side in 0..sides {
        let a = 2 + side;
        let b = 2 + (side + 1) % sides;
        indices.extend([0, b, a, 1, a, b]);
    }
    (positions, indices)
}

pub fn head() -> Vec<u8> {
    let mut writer = Writer {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let positions = head_positions();
    let indices = head_indices();
    let base = normals(&positions, &indices);
    let targets = head_targets(&positions, &indices, &base);
    let position = writer.floats(positions.as_flattened(), "VEC3", 3, true);
    let normal = writer.floats(base.as_flattened(), "VEC3", 3, false);
    let index = writer.indices(&indices);
    let mut morphs: Vec<Value> = Vec::new();
    for (target, (deltas, turned)) in targets.iter().enumerate() {
        if target == HEAD_TARGETS.len() - 1 {
            let keep: Vec<u32> = (0..deltas.len() as u32)
                .filter(|&at| deltas[at as usize] != [0.0; 3] || turned[at as usize] != [0.0; 3])
                .collect();
            let moved = sparse(&mut writer, deltas, &keep);
            let normals = sparse(&mut writer, turned, &keep);
            morphs.push(json!({ "POSITION": moved, "NORMAL": normals }));
        } else {
            let moved = writer.floats(deltas.as_flattened(), "VEC3", 3, true);
            let normals = writer.floats(turned.as_flattened(), "VEC3", 3, false);
            morphs.push(json!({ "POSITION": moved, "NORMAL": normals }));
        }
    }
    let (hat_positions, hat_indices) = hat();
    let hat_position = writer.floats(hat_positions.as_flattened(), "VEC3", 3, true);
    let hat_index = writer.indices(&hat_indices);
    let input = writer.floats(&HEAD_TIMES, "SCALAR", 1, true);
    let output = writer.floats(HEAD_KEYS.as_flattened(), "SCALAR", 1, false);
    while !writer.bin.len().is_multiple_of(4) {
        writer.bin.push(0);
    }
    let document = json!({
        "asset": { "version": "2.0", "generator": "pfx fixture head" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [
            { "name": "head", "mesh": 0, "children": [1] },
            { "name": "hat", "mesh": 1, "translation": [0.0, 0.55, 0.0] },
        ],
        "materials": [{ "name": "face" }, { "name": "felt" }],
        "meshes": [
            {
                "name": "head",
                "primitives": [{
                    "attributes": { "POSITION": position, "NORMAL": normal },
                    "indices": index,
                    "material": 0,
                    "targets": morphs,
                }],
                "weights": HEAD_WEIGHTS,
                "extras": { "targetNames": HEAD_TARGETS },
            },
            {
                "name": "hat",
                "primitives": [{
                    "attributes": { "POSITION": hat_position },
                    "indices": hat_index,
                    "material": 1,
                }],
            },
        ],
        "animations": [{
            "name": "talk",
            "samplers": [{ "input": input, "output": output, "interpolation": "LINEAR" }],
            "channels": [{ "sampler": 0, "target": { "node": 0, "path": "weights" } }],
        }],
        "buffers": [{ "byteLength": writer.bin.len() }],
        "bufferViews": writer.views,
        "accessors": writer.accessors,
    });
    glb(&document.to_string(), &writer.bin)
}
