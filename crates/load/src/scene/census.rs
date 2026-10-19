use super::{Environment, Geometry, Scene};
use pfx_core::memory::Census;
use std::collections::BTreeSet;
use std::sync::Arc;

fn bytes(geometry: &Geometry) -> u64 {
    geometry.positions.len() as u64 * 12
        + geometry.normals.len() as u64 * 12
        + geometry.tangents.len() as u64 * 16
        + geometry.uvs.len() as u64 * 8
        + geometry.uvs1.as_ref().map_or(0, |uvs| uvs.len() as u64 * 8)
        + geometry.indices.len() as u64 * 4
}

pub fn census(scene: &Scene) -> Census {
    let draws = scene.draws();
    let mut out = Census::default();
    let mut held = BTreeSet::new();
    let mut meshes = BTreeSet::new();
    for draw in &draws.items {
        let geometry = &draw.geometry;
        let triangles = geometry.indices.len() as u64 / 3;
        let vertices = geometry.positions.len() as u64;
        out.placed += triangles;
        out.vertices += vertices;
        out.pieces += 1;
        if held.insert(Arc::as_ptr(geometry) as usize) {
            out.built += bytes(geometry)
                + geometry
                    .levels
                    .iter()
                    .map(|level| bytes(&level.geometry))
                    .sum::<u64>();
        }
        if meshes.insert(geometry.hash) {
            out.distinct += triangles;
            out.distinct_vertices += vertices;
        }
    }
    let textures = &draws.textures;
    for image in textures
        .colour
        .iter()
        .chain(&textures.linear)
        .chain(&textures.orm)
    {
        out.maps += 1;
        out.texels += u64::from(image.width) * u64::from(image.height);
        out.largest = out.largest.max(image.width).max(image.height);
    }
    if let Environment::Hdr(sky) = scene.environment()
        && sky.width * sky.height > 1
    {
        out.sky = [sky.width, sky.height];
    }
    out
}
