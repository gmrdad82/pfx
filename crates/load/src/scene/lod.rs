use std::collections::BTreeSet;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::Geometry;
use crate::{Mesh, Node};

pub const COVERAGE_HEIGHT: f32 = 2160.0;

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryLevel {
    pub geometry: Arc<Geometry>,
    pub error: f32,
}

pub(super) fn level_nodes(mesh: &Mesh) -> BTreeSet<u32> {
    mesh.nodes
        .iter()
        .flat_map(|node| node.lod.iter().copied())
        .collect()
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
    count: usize,
    base: &Geometry,
) -> Result<Vec<GeometryLevel>, String> {
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
        if primitive.joints.is_some() || !primitive.targets.is_empty() {
            return Err(format!(
                "MSFT_lod level {} is skinned or morphed; levels take a static mesh",
                level.name
            ));
        }
        let geometry = super::build::geometry(primitive)
            .map_err(|message| format!("MSFT_lod level {}: {message}", level.name))?;
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
}
