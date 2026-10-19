use crate::bvh::{Bounds, Node};
use crate::detail::{Detail, Pose};
use crate::levels::Levels;
use crate::motion::{
    Decomposed, InstanceMotion, Motion, camera_rows, decompose_steps, shutter_table,
};

pub(crate) const MOVERS: u32 = 0x00ff_ffff;
const SLOT_HEAD: usize = 2;
const RIGID: u32 = 1;

#[derive(Clone, Debug)]
pub(crate) struct Moving {
    pub steps: usize,
    pub cameras: Vec<Decomposed>,
    pub table: Vec<f32>,
    pub firsts: Vec<u32>,
}

pub(crate) fn moves(detail: &Detail) -> bool {
    let Some(motion) = &detail.motion else {
        return false;
    };
    motion.camera_moves()
        || detail.instances.iter().any(|instance| {
            instance
                .motion
                .as_ref()
                .is_some_and(|moving| moving.moves(&instance.transform))
        })
}

pub(crate) fn check(detail: &Detail) -> Result<(), String> {
    let Some(motion) = &detail.motion else {
        return Ok(());
    };
    if !motion.valid() {
        return Err("invalid trace motion: 2 to 64 steps, finite cameras and curve".into());
    }
    for instance in &detail.instances {
        if let Some(moving) = &instance.motion {
            check_instance(moving, instance.triangles.len(), motion.steps)?;
        }
    }
    Ok(())
}

pub(crate) fn check_instance(
    moving: &InstanceMotion,
    triangles: usize,
    steps: usize,
) -> Result<(), String> {
    if !moving.transforms.is_empty() && moving.transforms.len() != steps {
        return Err(format!(
            "a moving instance has {} transforms for {steps} motion steps",
            moving.transforms.len()
        ));
    }
    for transform in &moving.transforms {
        Pose::new(*transform)?;
    }
    if !moving.shapes.is_empty() && moving.shapes.len() != steps {
        return Err(format!(
            "a deforming instance has {} shapes for {steps} motion steps",
            moving.shapes.len()
        ));
    }
    if moving.shapes.iter().any(|shape| {
        shape.len() != triangles
            || shape.iter().any(|triangle| {
                triangle
                    .vertices
                    .iter()
                    .chain(&triangle.normals)
                    .flatten()
                    .any(|value| !value.is_finite())
            })
    }) {
        return Err("a deforming instance's shapes must match its triangles and be finite".into());
    }
    Ok(())
}

impl Moving {
    pub(crate) fn new(motion: &Motion, slots: usize) -> Self {
        let cameras = if motion.camera_moves() {
            decompose_steps(&motion.cameras.iter().map(camera_rows).collect::<Vec<_>>())
        } else {
            Vec::new()
        };
        let table = if motion.curve.is_empty() {
            Vec::new()
        } else {
            shutter_table(&motion.curve)
        };
        Self {
            steps: motion.steps,
            cameras,
            table,
            firsts: vec![0; slots],
        }
    }

    pub(crate) fn len(&self, slots: usize) -> usize {
        1 + 4 * self.cameras.len()
            + self.table.len() / 4
            + slots * (SLOT_HEAD + 4 * self.steps)
            + slots.div_ceil(4)
    }

    pub(crate) fn block(&self, levels: Option<&Levels>) -> (Vec<[f32; 4]>, Vec<[f32; 6]>) {
        let placed: Vec<_> = levels
            .map(|levels| levels.placed.iter().flatten().collect())
            .unwrap_or_default();
        let word = |values: [u32; 4]| values.map(f32::from_bits);
        let mut out = Vec::with_capacity(self.len(placed.len()));
        out.push(word([
            self.steps as u32,
            self.cameras.len() as u32,
            u32::from(!self.table.is_empty()),
            placed.len() as u32,
        ]));
        for camera in &self.cameras {
            out.extend(camera.lanes());
        }
        out.extend(
            self.table
                .chunks(4)
                .map(|chunk| [chunk[0], chunk[1], chunk[2], chunk[3]]),
        );
        let mut movers = Vec::new();
        let mut boxes = Vec::new();
        for (slot, placed) in placed.iter().enumerate() {
            let Some(mesh) = levels.map(|levels| &levels.meshes[placed.mesh]) else {
                continue;
            };
            let rigid = !placed.motion.is_empty();
            out.push(word([
                ((placed.deform as u32) << 1) | if rigid { RIGID } else { 0 },
                mesh.triangles.len() as u32,
                self.firsts[slot],
                placed.base,
            ]));
            out.push(word([mesh.root, 0, 0, 0]));
            if rigid {
                for step in &placed.motion {
                    out.extend(step.lanes());
                }
            } else {
                out.extend(std::iter::repeat_n([0.0; 4], 4 * self.steps));
            }
            if placed.moving() {
                movers.push(slot as u32);
                boxes.push(bounds_box(placed.bounds));
            }
        }
        movers.resize(placed.len().div_ceil(4) * 4, 0);
        out.extend(
            movers
                .chunks(4)
                .map(|chunk| word([chunk[0], chunk[1], chunk[2], chunk[3]])),
        );
        (out, boxes)
    }
}

fn bounds_box(bounds: Bounds) -> [f32; 6] {
    [
        bounds.lo[0],
        bounds.lo[1],
        bounds.lo[2],
        bounds.hi[0],
        bounds.hi[1],
        bounds.hi[2],
    ]
}

pub(crate) fn as_nodes(block: &[[f32; 4]]) -> Vec<Node> {
    block
        .chunks(3)
        .map(|chunk| {
            let lane = |k: usize| chunk.get(k).copied().unwrap_or([0.0; 4]);
            let last = lane(2).map(f32::to_bits);
            Node {
                lo: lane(0),
                hi: lane(1),
                first: last[0],
                count: last[1],
                _pad: [last[2], last[3]],
            }
        })
        .collect()
}

pub(crate) fn as_words(block: &[[f32; 4]]) -> Vec<u32> {
    block
        .iter()
        .flatten()
        .map(|value| value.to_bits())
        .collect()
}

pub(crate) fn node_words(nodes: &[Node]) -> Vec<u32> {
    bytemuck::cast_slice(nodes).to_vec()
}
