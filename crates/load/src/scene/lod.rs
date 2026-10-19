use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::{Geometry, GeometryMorph};
use crate::surface::Surface;
use crate::{Group, Mesh, Node};

pub const COVERAGE_HEIGHT: f32 = 2160.0;

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryLevel {
    pub geometry: Arc<Geometry>,
    pub error: f32,
    pub surface: Option<Arc<Surface>>,
}

fn own_surface(mesh: &Mesh, base: Option<u32>, level: Option<u32>) -> Option<Arc<Surface>> {
    let name = |at: Option<u32>| {
        at.and_then(|at| mesh.materials.get(at as usize))
            .map(|m| &m.name)
    };
    if name(level).is_none() || name(level) == name(base) {
        return None;
    }
    mesh.surfaces
        .get(level? as usize)
        .filter(|surface| surface.declared)
        .map(|surface| Arc::new(surface.clone()))
}

pub(super) fn level_nodes(mesh: &Mesh) -> BTreeSet<u32> {
    mesh.nodes
        .iter()
        .flat_map(|node| node.lod.iter().copied())
        .collect()
}

fn declared_group(mesh: &Mesh, node: &Node) -> Result<Option<String>, String> {
    let mut found: Option<&str> = None;
    for &id in &node.lod {
        let level = &mesh.nodes[id as usize];
        let Some(value) = level.extra_object("lod").and_then(|lod| lod.get("group")) else {
            continue;
        };
        let group = value.as_str().ok_or_else(|| {
            format!(
                "MSFT_lod level {}: extras.lod.group must be a node name",
                level.name
            )
        })?;
        if found.is_some_and(|known| known != group) {
            return Err(format!(
                "MSFT_lod level {}: extras.lod.group is {group}, another level of {} names {}",
                level.name,
                node.name,
                found.unwrap_or_default()
            ));
        }
        found = Some(group);
    }
    Ok(found.map(str::to_string))
}

pub(super) fn groups(mesh: &Mesh) -> Result<BTreeMap<u32, String>, String> {
    let mut groups = BTreeMap::new();
    for (index, node) in mesh.nodes.iter().enumerate() {
        if node.lod.is_empty() {
            continue;
        }
        let Some(group) = declared_group(mesh, node)? else {
            continue;
        };
        let root = mesh
            .nodes
            .iter()
            .find(|candidate| candidate.name == group)
            .ok_or_else(|| {
                format!(
                    "node {}: extras.lod.group names node {group}, which is not in the file",
                    node.name
                )
            })?;
        if root.lod.is_empty() {
            return Err(format!(
                "node {}: extras.lod.group names node {group}, which has no MSFT_lod levels",
                node.name
            ));
        }
        if let Some(further) = declared_group(mesh, root)?.filter(|further| *further != group) {
            return Err(format!(
                "node {}: extras.lod.group names node {group}, which follows group {further}; a group's root follows no other",
                node.name
            ));
        }
        groups.insert(index as u32, group);
    }
    Ok(groups)
}

fn radius(geometry: &Geometry) -> f32 {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for position in &geometry.positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    (0..3)
        .map(|axis| (max[axis] - min[axis]).powi(2))
        .sum::<f32>()
        .sqrt()
        * 0.5
}

fn error(base: &Node, level: &Node, rank: usize, geometry: &Geometry) -> Result<f32, String> {
    if let Some(error) = level
        .extra_object("lod")
        .and_then(|lod| lod.get("error"))
        .and_then(serde_json::Value::as_f64)
    {
        return Ok(error as f32);
    }
    let coverage = base
        .extra_f64s("MSFT_screencoverage")
        .and_then(|coverage| coverage.get(rank).copied())
        .ok_or_else(|| {
            format!(
                "MSFT_lod level {} has no extras.lod.error and no MSFT_screencoverage",
                level.name
            )
        })?;
    if coverage.is_nan() || coverage <= 0.0 {
        return Err(format!(
            "MSFT_lod level {}: a screen coverage must be above zero",
            level.name
        ));
    }
    Ok(radius(geometry) / (coverage as f32 * COVERAGE_HEIGHT))
}

pub(super) fn levels(
    mesh: &Mesh,
    node: &Node,
    slot: usize,
    owner: &Group,
    morph: Option<usize>,
    base: &Geometry,
) -> Result<Vec<GeometryLevel>, String> {
    let count = owner.primitives.len();
    let names = |group: &Group| group.extra("targetNames").cloned();
    let mut levels: Vec<GeometryLevel> = Vec::with_capacity(node.lod.len());
    for (rank, &id) in node.lod.iter().enumerate() {
        let level = &mesh.nodes[id as usize];
        let group = level
            .group
            .map(|group| &mesh.groups[group as usize])
            .ok_or_else(|| format!("MSFT_lod level {} holds no mesh", level.name))?;
        if group.primitives.len() != count {
            return Err(format!(
                "MSFT_lod level {} has {} primitives, the node has {count}",
                level.name,
                group.primitives.len()
            ));
        }
        let primitive = &group.primitives[slot];
        if primitive.joints.is_some() || level.skin.is_some() {
            return Err(format!(
                "MSFT_lod level {} is skinned; levels take a static or morphed mesh",
                level.name
            ));
        }
        let targets = owner.primitives[slot].targets.len();
        if primitive.targets.len() != targets || names(group) != names(owner) {
            return Err(format!(
                "MSFT_lod level {} has {} morph targets {}, the node has {targets} {}",
                level.name,
                primitive.targets.len(),
                names(group).unwrap_or_default(),
                names(owner).unwrap_or_default()
            ));
        }
        let mut geometry = super::build::geometry(primitive)
            .map_err(|message| format!("MSFT_lod level {}: {message}", level.name))?;
        if let Some(morph) = morph {
            geometry = geometry.with_morph(GeometryMorph {
                morph,
                targets: primitive.targets.clone(),
            });
        }
        let error = error(node, level, rank, base)?;
        if !error.is_finite() || error < 0.0 {
            return Err(format!(
                "MSFT_lod level {}: the error must be a finite length in metres",
                level.name
            ));
        }
        if levels.last().is_some_and(|last| error < last.error) {
            return Err(format!(
                "MSFT_lod level {}: errors must grow from level to level",
                level.name
            ));
        }
        levels.push(GeometryLevel {
            geometry: Arc::new(geometry),
            error,
            surface: own_surface(mesh, owner.primitives[slot].material, primitive.material),
        });
    }
    Ok(levels)
}

impl Geometry {
    pub fn with_levels(mut self, levels: Vec<GeometryLevel>) -> Self {
        if levels.is_empty() {
            return self;
        }
        let mut hasher = Sha256::new();
        hasher.update(self.hash);
        hasher.update([4]);
        for level in &levels {
            hasher.update(level.geometry.hash);
            hasher.update(level.error.to_le_bytes());
        }
        self.hash = hasher.finalize().into();
        self.levels = levels;
        self
    }

    pub fn with_group(mut self, group: String) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(self.hash);
        hasher.update([5]);
        hasher.update(group.as_bytes());
        self.hash = hasher.finalize().into();
        self.group = Some(group);
        self
    }
}
