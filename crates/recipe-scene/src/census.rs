use pfx_build::{Built, Part};
use pfx_core::memory::{self, Census, Plan, Run};
use pfx_mesh::rigid::{self, Anchors, Shape};
use pfx_recipe::Recipe;
use std::collections::{BTreeMap, BTreeSet};

use crate::Error;

pub fn census(recipe: &Recipe, built: &Built) -> Census {
    census_of(&built.parts, crate::light::sky_size(recipe))
}

pub fn census_of(parts: &[Part], sky: [u32; 2]) -> Census {
    let mut out = Census {
        sky,
        ..Census::default()
    };
    let mut leads: BTreeMap<u64, Vec<(usize, Option<Anchors>)>> = BTreeMap::new();
    let mut images = BTreeSet::new();
    for (at, part) in parts.iter().enumerate() {
        let mesh = &part.node.mesh;
        let uvs = mesh.uvs.as_deref().unwrap_or_default();
        let vertices = mesh.positions.len() as u64;
        out.built += vertices * 24
            + uvs.len() as u64 * 8
            + mesh.uvs1.as_ref().map_or(0, |uvs| uvs.len() as u64 * 8)
            + mesh.indices.len() as u64 * 4;
        if let Some(targets) = &part.node.targets {
            let morph = &targets.morph;
            out.built += morph
                .positions
                .iter()
                .chain(&morph.normals)
                .map(|target| target.len() as u64 * 12)
                .sum::<u64>();
        }
        if part.hidden {
            continue;
        }
        let triangles = mesh.triangle_count() as u64;
        out.placed += triangles;
        out.vertices += vertices;
        out.pieces += mesh.slots.len().max(1) as u64;
        if !copied(parts, at, &mut leads) {
            out.distinct += triangles;
            out.distinct_vertices += vertices;
        }
        for slot in &mesh.slots {
            let Some(look) = part.looks.get(&slot.material) else {
                continue;
            };
            for map in look.maps().into_iter().flatten() {
                let image = &map.image;
                if images.insert(std::sync::Arc::as_ptr(&image.bytes) as *const u8 as usize) {
                    out.built += image.bytes.len() as u64;
                    out.maps += 1;
                    out.texels += u64::from(image.size[0]) * u64::from(image.size[1]);
                    out.largest = out.largest.max(image.size[0]).max(image.size[1]);
                }
            }
        }
    }
    out
}

fn shape(mesh: &pfx_mesh::Mesh) -> Shape<'_> {
    Shape {
        positions: &mesh.positions,
        normals: &mesh.normals,
    }
}

fn copied(
    parts: &[Part],
    at: usize,
    leads: &mut BTreeMap<u64, Vec<(usize, Option<Anchors>)>>,
) -> bool {
    let mesh = &parts[at].node.mesh;
    let uvs = mesh.uvs.as_deref().unwrap_or_default();
    let key = rigid::key(mesh.positions.len(), &mesh.indices, uvs);
    let known = leads.entry(key).or_default();
    for (lead, anchors) in known.iter_mut() {
        let first = &parts[*lead].node.mesh;
        if first.indices != mesh.indices || first.uvs != mesh.uvs || first.slots != mesh.slots {
            continue;
        }
        if anchors.is_none() {
            *anchors = rigid::anchors(&first.positions);
        }
        if anchors
            .as_ref()
            .is_some_and(|anchors| rigid::register(shape(first), anchors, shape(mesh)).is_some())
        {
            return true;
        }
    }
    known.push((at, None));
    false
}

pub fn admit(recipe: &Recipe, built: &Built, run: Run) -> Result<Plan, Error> {
    let plan = memory::plan(&census(recipe, built), run);
    plan.admit().map_err(Error)?;
    Ok(plan)
}
