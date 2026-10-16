use std::path::Path;

use pfx_bench::shot::{Lens, candidates, splits};
use pfx_load::scene::Scene;
use serde_json::Value;

pub struct Found {
    pub lens: Lens,
    pub aspect: Option<f32>,
}

fn document(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(b"glTF") {
        let length = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
        if bytes.get(16..20)? != b"JSON" {
            return None;
        }
        return serde_json::from_slice(bytes.get(20..20 + length)?).ok();
    }
    serde_json::from_slice(&bytes).ok()
}

pub fn of_node(json: &Value, name: &str) -> Option<Found> {
    let wanted = candidates(name);
    let nodes = json.get("nodes")?.as_array()?;
    let node = wanted.iter().find_map(|wanted| {
        nodes
            .iter()
            .find(|node| node.get("name").and_then(Value::as_str) == Some(wanted.as_str()))
    })?;
    let camera = node.get("camera")?.as_u64()? as usize;
    let perspective = json.get("cameras")?.get(camera)?.get("perspective")?;
    let number = |key: &str| {
        perspective
            .get(key)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
    };
    Some(Found {
        lens: Lens {
            fov: number("yfov").map(f32::to_degrees),
            near: number("znear"),
        },
        aspect: number("aspectRatio"),
    })
}

pub fn find(scene: &Scene, name: &str) -> Option<Found> {
    if !scene.objects.iter().any(|object| object.name == name) {
        for (object, node) in splits(name) {
            if let Some(mesh) = scene
                .objects
                .iter()
                .find(|found| found.name == object)
                .and_then(|found| scene.meshes.get(&found.mesh))
            {
                return document(&mesh.file).and_then(|json| of_node(&json, node));
            }
        }
    }
    let mut files: Vec<&Path> = scene
        .meshes
        .values()
        .map(|mesh| mesh.file.as_path())
        .collect();
    files.sort_unstable();
    files.dedup();
    files
        .into_iter()
        .filter_map(document)
        .find_map(|json| of_node(&json, name))
}
