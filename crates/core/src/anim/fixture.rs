use super::math::{Mat4, Trs, invert, quat_axis_angle};
use super::{
    Channel, Clip, ClipEvent, Interpolation, Joint, Property, Rig, Skeleton, normalize_weights,
};

pub const SEGMENT: f32 = 1.0;
pub const HALF_WIDTH: f32 = 0.15;
pub const ROWS_PER_BONE: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

fn translate(y: f32) -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, y, 0.0, 1.0],
    ]
}

fn turn(axis: [f32; 3], angles: &[f32]) -> Vec<f32> {
    angles
        .iter()
        .flat_map(|&angle| quat_axis_angle(axis, angle))
        .collect()
}

pub fn skeleton(bones: usize) -> Skeleton {
    let bones = bones.max(1);
    let joints = (0..bones)
        .map(|index| Joint {
            name: format!("bone{index}"),
            parent: index.checked_sub(1),
            rest: Trs {
                translation: [0.0, if index == 0 { 0.0 } else { SEGMENT }, 0.0],
                ..Trs::IDENTITY
            },
        })
        .collect();
    let binds = (0..bones)
        .map(|index| invert(&translate(index as f32 * SEGMENT)).unwrap_or(super::math::IDENTITY))
        .collect();
    Skeleton::new(joints, binds).expect("the fixture skeleton is valid")
}

pub fn clips(bones: usize) -> Vec<Clip> {
    let bones = bones.max(1);
    let bend = Clip::new(
        "bend",
        (1..bones)
            .map(|joint| Channel {
                joint,
                property: Property::Rotation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 0.5, 1.0],
                values: turn([0.0, 0.0, 1.0], &[0.0, 0.6, 0.0]),
            })
            .collect(),
    )
    .and_then(|clip| {
        clip.with_events(vec![
            ClipEvent::new(0.0, "start"),
            ClipEvent::new(0.5, "half"),
        ])
    })
    .expect("the fixture bend clip is valid");
    let mut wave_values = Vec::new();
    for angle in [0.0f32, -0.5, 0.0] {
        wave_values.extend([0.0; 4]);
        wave_values.extend(quat_axis_angle([0.0, 0.0, 1.0], angle));
        wave_values.extend([0.0; 4]);
    }
    let wave = Clip::new(
        "wave",
        (1..bones)
            .map(|joint| Channel {
                joint,
                property: Property::Rotation,
                interpolation: Interpolation::CubicSpline,
                times: vec![0.0, 0.6, 1.2],
                values: wave_values.clone(),
            })
            .collect(),
    )
    .expect("the fixture wave clip is valid");
    let lift = Clip::new(
        "lift",
        vec![
            Channel {
                joint: 0,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 0.4, 0.8],
                values: vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0],
            },
            Channel {
                joint: 0,
                property: Property::Scale,
                interpolation: Interpolation::Step,
                times: vec![0.0, 0.4],
                values: vec![1.0, 1.0, 1.0, 1.2, 1.0, 1.2],
            },
        ],
    )
    .expect("the fixture lift clip is valid");
    let twist = Clip::new(
        "twist",
        vec![Channel {
            joint: bones - 1,
            property: Property::Rotation,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: turn([1.0, 0.0, 0.0], &[0.0, 1.2]),
        }],
    )
    .expect("the fixture twist clip is valid");
    vec![bend, wave, lift, twist]
}

pub fn rig(bones: usize) -> Rig {
    Rig::new(skeleton(bones), clips(bones)).expect("the fixture rig is valid")
}

pub fn column(bones: usize) -> Column {
    let bones = bones.max(1);
    let height = bones as f32 * SEGMENT;
    let rows = bones * ROWS_PER_BONE;
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 4] = [
        ([0.0, 0.0, 1.0], [-1.0, 0.0, 1.0], [1.0, 0.0, 1.0]),
        ([1.0, 0.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, -1.0]),
        ([0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [-1.0, 0.0, -1.0]),
        ([-1.0, 0.0, 0.0], [-1.0, 0.0, -1.0], [-1.0, 0.0, 1.0]),
    ];
    let mut out = Column {
        positions: Vec::new(),
        normals: Vec::new(),
        tangents: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        joints: Vec::new(),
        weights: Vec::new(),
    };
    for (face, (normal, left, right)) in faces.iter().enumerate() {
        let base = out.positions.len() as u32;
        let along = [right[0] - left[0], 0.0, right[2] - left[2]];
        let length = (along[0] * along[0] + along[2] * along[2]).sqrt();
        for row in 0..=rows {
            let y = height * row as f32 / rows as f32;
            let (joints, weights) = influence(y, bones);
            for (side, corner) in [left, right].into_iter().enumerate() {
                out.positions
                    .push([corner[0] * HALF_WIDTH, y, corner[2] * HALF_WIDTH]);
                out.normals.push(*normal);
                out.tangents
                    .push([along[0] / length, 0.0, along[2] / length, 1.0]);
                out.uvs
                    .push([(face as f32 + side as f32) / 4.0, 1.0 - y / height]);
                out.joints.push(joints);
                out.weights.push(weights);
            }
        }
        for row in 0..rows as u32 {
            let a = base + row * 2;
            out.indices.extend([a, a + 1, a + 3, a, a + 3, a + 2]);
        }
    }
    normalize_weights(&mut out.weights);
    out
}

fn influence(y: f32, bones: usize) -> ([u16; 4], [f32; 4]) {
    let mut found: Vec<(f32, usize)> = (0..bones)
        .map(|joint| {
            let centre = (joint as f32 + 0.5) * SEGMENT;
            ((1.0 - (y - centre).abs() / SEGMENT).max(0.0), joint)
        })
        .filter(|(weight, _)| *weight > 0.0)
        .collect();
    found.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    found.truncate(2);
    if found.is_empty() {
        let joint = if y <= 0.0 { 0 } else { bones - 1 };
        return ([joint as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
    }
    let mut joints = [0u16; 4];
    let mut weights = [0.0f32; 4];
    for (slot, (weight, joint)) in found.into_iter().enumerate() {
        joints[slot] = joint as u16;
        weights[slot] = weight;
    }
    (joints, weights)
}
