use glam::DMat4;
use pfx_glb::{MORPH_EPSILON, Node, Order, Targets, write};
use pfx_mesh::{Mesh, Morph};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Instant;

const TARGETS: usize = 20;

fn hashed(i: usize, t: usize) -> u32 {
    let mut x = (i as u32).wrapping_mul(0x9e37_79b9) ^ (t as u32).wrapping_mul(0x85eb_ca6b);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x
}

fn moved(i: usize, t: usize) -> bool {
    hashed(i, t) % 10 < 4
}

fn head(cells: u32) -> Node {
    let side = cells + 1;
    let mut mesh = Mesh::default();
    for y in 0..side {
        for x in 0..side {
            let (u, v) = (x as f32 / cells as f32, y as f32 / cells as f32);
            mesh.positions.push([u, v, 0.0]);
            mesh.normals.push([0.0, 0.0, 1.0]);
        }
    }
    for y in 0..cells {
        for x in 0..cells {
            let a = y * side + x;
            mesh.indices
                .extend([a, a + side, a + 1, a + 1, a + side, a + side + 1]);
        }
    }
    let count = mesh.positions.len();
    let delta = |t: usize, scale: f32| -> Vec<[f32; 3]> {
        (0..count)
            .map(|i| {
                let h = hashed(i, t + 100) as f32 / u32::MAX as f32;
                if moved(i, t) {
                    [scale * (h - 0.5), scale * 0.5 * h, scale * (0.25 + h)]
                } else {
                    [1e-7 * h, 0.0, -1e-7 * h]
                }
            })
            .collect()
    };
    let morph = Morph {
        names: (0..TARGETS).map(|t| format!("shape{t}")).collect(),
        weights: vec![0.0; TARGETS],
        positions: (0..TARGETS).map(|t| delta(t, 0.004)).collect(),
        normals: (0..TARGETS).map(|t| delta(t, 0.05)).collect(),
    };
    Node {
        name: "head".into(),
        mesh: mesh.clone(),
        transform: DMat4::IDENTITY,
        keep_transform: false,
        targets: Some(Box::new(Targets { base: mesh, morph })),
    }
}

fn split(glb: &[u8]) -> (Value, Vec<u8>) {
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = serde_json::from_slice(&glb[20..20 + length]).unwrap();
    let bin = glb[20 + length + 8..].to_vec();
    (json, bin)
}

fn join(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut text = serde_json::to_vec(json).unwrap();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out
}

fn dense(glb: &[u8]) -> Vec<u8> {
    let (mut json, mut bin) = split(glb);
    let views = json["bufferViews"].clone();
    let span = |view: &Value| -> (usize, usize) {
        let at = &views[view.as_u64().unwrap() as usize];
        (
            at["byteOffset"].as_u64().unwrap_or(0) as usize,
            at["byteLength"].as_u64().unwrap() as usize,
        )
    };
    let count = json["accessors"].as_array().unwrap().len();
    for a in 0..count {
        let accessor = json["accessors"][a].clone();
        let Some(sparse) = accessor.get("sparse") else {
            continue;
        };
        let vertices = accessor["count"].as_u64().unwrap() as usize;
        let entries = sparse["count"].as_u64().unwrap() as usize;
        let (index_at, _) = span(&sparse["indices"]["bufferView"]);
        let (value_at, _) = span(&sparse["values"]["bufferView"]);
        let width = match sparse["indices"]["componentType"].as_u64().unwrap() {
            5121 => 1,
            5123 => 2,
            _ => 4,
        };
        let mut floats = vec![0u8; vertices * 12];
        for e in 0..entries {
            let mut raw = [0u8; 4];
            raw[..width].copy_from_slice(&bin[index_at + e * width..index_at + (e + 1) * width]);
            let i = u32::from_le_bytes(raw) as usize;
            floats[i * 12..i * 12 + 12]
                .copy_from_slice(&bin[value_at + e * 12..value_at + e * 12 + 12]);
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(&floats);
        let views = json["bufferViews"].as_array_mut().unwrap();
        views.push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": floats.len(), "target": 34962 }));
        let view = views.len() - 1;
        let accessor = json["accessors"][a].as_object_mut().unwrap();
        accessor.remove("sparse");
        accessor.insert("bufferView".into(), json!(view));
        accessor.insert("byteOffset".into(), json!(0));
    }
    json["buffers"][0]["byteLength"] = json!(bin.len());
    join(&json, &bin)
}

fn sparse_accessors(glb: &[u8]) -> usize {
    let (json, _) = split(glb);
    json["accessors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a.get("sparse").is_some())
        .count()
}

#[test]
fn morph_targets_are_written_sparse_and_every_reader_gets_the_same_deltas() {
    let node = head(40);
    let glb = write(std::slice::from_ref(&node), Order::File, "t");
    assert_eq!(sparse_accessors(&glb), TARGETS * 2);
    let full = dense(&glb);
    assert!(
        glb.len() * 10 < full.len() * 7,
        "{} bytes sparse against {} dense",
        glb.len(),
        full.len()
    );
    let morph = &node.targets.as_ref().unwrap().morph;
    let shown = |list: &[[f32; 3]]| -> Vec<[f32; 3]> {
        list.iter()
            .map(|d| {
                if d.iter().any(|c| c.abs() > MORPH_EPSILON) {
                    *d
                } else {
                    [0.0; 3]
                }
            })
            .collect()
    };
    let source: HashMap<[u32; 3], usize> = node
        .mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| (p.map(f32::to_bits), i))
        .collect();
    let order = |positions: &[[f32; 3]]| -> Vec<usize> {
        positions
            .iter()
            .map(|p| source[&p.map(f32::to_bits)])
            .collect()
    };
    let expect = |list: &[[f32; 3]], order: &[usize]| -> Vec<[f32; 3]> {
        let shown = shown(list);
        order.iter().map(|&i| shown[i]).collect()
    };
    for bytes in [&glb, &full] {
        let loaded = pfx_load::Mesh::parse(bytes).unwrap();
        let primitive = &loaded.groups[0].primitives[0];
        let order = order(&primitive.positions);
        assert_eq!(primitive.targets.len(), TARGETS);
        for (t, target) in primitive.targets.iter().enumerate() {
            assert_eq!(target.positions, expect(&morph.positions[t], &order));
            assert_eq!(
                target.normals.as_deref(),
                Some(expect(&morph.normals[t], &order).as_slice())
            );
        }
    }
    let (document, buffers, _) = gltf::import_slice(&glb).unwrap();
    let primitive = document
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
    let order = order(&reader.read_positions().unwrap().collect::<Vec<_>>());
    for (t, (positions, normals, _)) in reader.read_morph_targets().enumerate() {
        let positions: Vec<[f32; 3]> = positions.unwrap().collect();
        let normals: Vec<[f32; 3]> = normals.unwrap().collect();
        assert_eq!(positions, expect(&morph.positions[t], &order));
        assert_eq!(normals, expect(&morph.normals[t], &order));
    }
}

#[test]
fn a_target_mostly_moved_stays_dense() {
    let mut node = head(8);
    let morph = &mut node.targets.as_mut().unwrap().morph;
    for list in morph.positions.iter_mut().chain(morph.normals.iter_mut()) {
        for d in list.iter_mut() {
            *d = [0.01, 0.0, 0.0];
        }
    }
    let glb = write(&[node], Order::File, "t");
    assert_eq!(sparse_accessors(&glb), 0);
}

fn median(mut list: Vec<f64>) -> f64 {
    list.sort_by(f64::total_cmp);
    list[list.len() / 2]
}

#[test]
#[ignore = "a measurement; run it in release with --nocapture"]
fn measure_sparse_morph_targets_against_dense() {
    let node = head(273);
    let vertices = node.mesh.positions.len();
    let started = Instant::now();
    let glb = write(&[node], Order::File, "t");
    let written = started.elapsed().as_secs_f64() * 1e3;
    let full = dense(&glb);
    let mut times = [Vec::new(), Vec::new()];
    for _ in 0..5 {
        for (k, bytes) in [&full, &glb].into_iter().enumerate() {
            let started = Instant::now();
            pfx_load::Mesh::parse(bytes).unwrap();
            times[k].push(started.elapsed().as_secs_f64() * 1e3);
        }
    }
    let [before, after] = times.map(median);
    println!(
        "{vertices} vertices, {TARGETS} targets: dense {} bytes, sparse {} bytes ({:.1}%); load median dense {before:.2} ms, sparse {after:.2} ms; write {written:.1} ms",
        full.len(),
        glb.len(),
        glb.len() as f64 * 100.0 / full.len() as f64
    );
}
