use crate::mesh::Mesh;
use crate::simplify::{Input, Options, simplify};

pub const NORMAL_WEIGHT: f32 = 0.5;
pub const UV_WEIGHT: f32 = 1.0;
pub const MIN_TRIANGLES: usize = 32;
pub const MAX_LEVELS: usize = 8;

#[derive(Clone, Debug)]
pub struct Level {
    pub mesh: Mesh,
    pub error: f32,
}

pub fn chain(mesh: &Mesh) -> Vec<Level> {
    let mut levels = vec![Level {
        mesh: mesh.clone(),
        error: 0.0,
    }];
    if mesh.indices.is_empty() {
        return levels;
    }
    let attributes: Vec<f32> = mesh
        .normals
        .iter()
        .zip(mesh.uvs.iter())
        .flat_map(|(n, uv)| [n[0], n[1], n[2], uv[0], uv[1]])
        .collect();
    let weights = [
        NORMAL_WEIGHT,
        NORMAL_WEIGHT,
        NORMAL_WEIGHT,
        UV_WEIGHT,
        UV_WEIGHT,
    ];
    let input = Input {
        indices: &mesh.indices,
        positions: &mesh.positions,
        attributes: &attributes,
        weights: &weights,
        locks: &[],
    };
    let options = Options {
        lock_border: true,
        error_absolute: true,
    };
    let mut previous = mesh.triangle_count();
    let mut target = previous / 2;
    while levels.len() < MAX_LEVELS && target >= MIN_TRIANGLES {
        let simplified = simplify(&input, target * 3, f32::MAX, options);
        let (indices, error) = (simplified.indices, simplified.error);
        let triangles = indices.len() / 3;
        if triangles == 0 || triangles * 10 > previous * 9 {
            break;
        }
        let floor = levels.last().map_or(0.0, |level| level.error);
        levels.push(Level {
            mesh: compact(mesh, &indices),
            error: error.max(floor),
        });
        previous = triangles;
        target = triangles / 2;
    }
    levels
}

pub fn errors(levels: &[Level]) -> Vec<f32> {
    levels.iter().map(|level| level.error).collect()
}

fn compact(mesh: &Mesh, indices: &[u32]) -> Mesh {
    let mut remap = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tangents = Vec::new();
    let mut uvs = Vec::new();
    let mut out = Vec::with_capacity(indices.len());
    for &index in indices {
        let slot = &mut remap[index as usize];
        if *slot == u32::MAX {
            *slot = positions.len() as u32;
            positions.push(mesh.positions[index as usize]);
            normals.push(mesh.normals[index as usize]);
            tangents.push(mesh.tangents[index as usize]);
            uvs.push(mesh.uvs[index as usize]);
        }
        out.push(*slot);
    }
    Mesh::new(positions, normals, tangents, uvs, out)
}
