use super::AnimError;
use super::math::{Mat4, normalize3, transform_point, transform_vector};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skinned {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
}

pub fn normalize_weights(weights: &mut [[f32; 4]]) {
    for weight in weights {
        let sum = weight[0] + weight[1] + weight[2] + weight[3];
        if sum > 0.0 && sum.is_finite() {
            for value in weight.iter_mut() {
                *value /= sum;
            }
        } else {
            *weight = [1.0, 0.0, 0.0, 0.0];
        }
    }
}

pub fn blend_matrix(palette: &[Mat4], joints: [u16; 4], weights: [f32; 4]) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (joint, weight) in joints.into_iter().zip(weights) {
        let matrix = &palette[joint as usize];
        for (column, target) in out.iter_mut().enumerate() {
            for (row, value) in target.iter_mut().enumerate() {
                *value += matrix[column][row] * weight;
            }
        }
    }
    out
}

pub fn check_skin(
    vertices: usize,
    joints: &[[u16; 4]],
    weights: &[[f32; 4]],
    palette: usize,
) -> Result<(), AnimError> {
    if joints.len() != vertices || weights.len() != vertices {
        return Err(AnimError::Skin(format!(
            "{} joint sets and {} weight sets for {vertices} vertices",
            joints.len(),
            weights.len()
        )));
    }
    if let Some(joint) = joints
        .iter()
        .flatten()
        .find(|joint| **joint as usize >= palette)
    {
        return Err(AnimError::Skin(format!(
            "a vertex names joint {joint} of a palette of {palette}"
        )));
    }
    if weights.iter().flatten().any(|weight| !weight.is_finite()) {
        return Err(AnimError::Skin("a weight is not finite".into()));
    }
    Ok(())
}

pub fn skin(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    tangents: &[[f32; 4]],
    joints: &[[u16; 4]],
    weights: &[[f32; 4]],
    palette: &[Mat4],
) -> Result<Skinned, AnimError> {
    check_skin(positions.len(), joints, weights, palette.len())?;
    if normals.len() != positions.len()
        || (!tangents.is_empty() && tangents.len() != positions.len())
    {
        return Err(AnimError::Skin(
            "normals and tangents need one entry per vertex".into(),
        ));
    }
    let mut out = Skinned {
        positions: Vec::with_capacity(positions.len()),
        normals: Vec::with_capacity(positions.len()),
        tangents: Vec::with_capacity(tangents.len()),
    };
    for index in 0..positions.len() {
        let matrix = blend_matrix(palette, joints[index], weights[index]);
        out.positions
            .push(transform_point(&matrix, positions[index]));
        out.normals
            .push(normalize3(transform_vector(&matrix, normals[index])));
        if let Some(tangent) = tangents.get(index) {
            let along = normalize3(transform_vector(
                &matrix,
                [tangent[0], tangent[1], tangent[2]],
            ));
            out.tangents
                .push([along[0], along[1], along[2], tangent[3]]);
        }
    }
    Ok(out)
}
