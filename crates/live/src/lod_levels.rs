use std::collections::HashMap;

use pfx_geom::mesh::Bounds;
use pfx_load::scene::{Draws, Geometry};

use crate::frame::MeshHandle;
use crate::renderer::Renderer;

pub(crate) fn with_levels(geometry: &Geometry) -> impl Iterator<Item = &Geometry> {
    std::iter::once(geometry).chain(geometry.levels.iter().map(|level| &*level.geometry))
}

fn bounds(geometry: &Geometry) -> Bounds {
    let mut bounds = Bounds::empty();
    for position in &geometry.positions {
        bounds.include(*position);
    }
    bounds
}

pub(crate) fn register(
    renderer: &mut Renderer,
    draws: &Draws,
    meshes: &HashMap<[u8; 32], MeshHandle>,
    skip: impl Fn(usize) -> bool,
    old: &[MeshHandle],
) -> Result<Vec<MeshHandle>, String> {
    let mut leveled: Vec<MeshHandle> = Vec::new();
    for (index, draw) in draws.items.iter().enumerate() {
        if draw.geometry.levels.is_empty() || skip(index) {
            continue;
        }
        let Some(&base) = meshes.get(&draw.geometry.hash) else {
            continue;
        };
        if leveled.contains(&base) {
            continue;
        }
        let mut chain = vec![(base, 0.0)];
        for level in &draw.geometry.levels {
            let handle = meshes
                .get(&level.geometry.hash)
                .ok_or("a staged LOD level was not uploaded")?;
            chain.push((*handle, level.error));
        }
        renderer
            .lod_mut()
            .set_levels(bounds(&draw.geometry), &chain)?;
        leveled.push(base);
    }
    for stale in old.iter().filter(|handle| !leveled.contains(handle)) {
        renderer.lod_mut().clear_levels(*stale);
    }
    Ok(leveled)
}

pub(crate) fn groups(
    draws: &Draws,
    instance: impl Fn(usize) -> Option<usize>,
) -> Vec<(usize, usize)> {
    if !draws.items.iter().any(|draw| draw.geometry.group.is_some()) {
        return Vec::new();
    }
    let mut roots: HashMap<(Option<usize>, &str), usize> = HashMap::new();
    for (index, draw) in draws.items.iter().enumerate() {
        if !draw.geometry.levels.is_empty() {
            roots
                .entry((draw.owner, draw.node.as_str()))
                .or_insert(index);
        }
    }
    let mut links = Vec::new();
    for (index, draw) in draws.items.iter().enumerate() {
        let Some(group) = draw.geometry.group.as_deref() else {
            continue;
        };
        let Some(&root) = roots.get(&(draw.owner, group)) else {
            continue;
        };
        if root == index {
            continue;
        }
        if let (Some(member), Some(root)) = (instance(index), instance(root)) {
            links.push((member, root));
        }
    }
    links
}
