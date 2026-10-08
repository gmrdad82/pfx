use std::cell::{Cell, Ref, RefCell};
use std::ops::Range;

use wgpu::util::DeviceExt;

use crate::frame::Matrix;
use crate::ranges::FreeRanges;

pub const NO_SKIN: u32 = u32::MAX;
pub const SKIN_BINDING: u32 = 4;

pub const SKIN_WGSL: &str = r#"
@group(0) @binding(4) var<storage, read> skin_data: array<vec4u>;
fn skin_joint(palette: u32, joint: u32) -> mat4x4f {
    let at = palette + joint * 4u;
    return mat4x4f(
        bitcast<vec4f>(skin_data[at]),
        bitcast<vec4f>(skin_data[at + 1u]),
        bitcast<vec4f>(skin_data[at + 2u]),
        bitcast<vec4f>(skin_data[at + 3u]),
    );
}
fn skin_blend(skin: vec4u, vertex: u32, palette: u32) -> mat4x4f {
    let record = skin.x + vertex * 2u;
    let packed = skin_data[record];
    let weights = bitcast<vec4f>(skin_data[record + 1u]);
    var blended = skin_joint(palette, packed.x & 0xffffu) * weights.x;
    blended = blended + skin_joint(palette, packed.x >> 16u) * weights.y;
    blended = blended + skin_joint(palette, packed.y & 0xffffu) * weights.z;
    blended = blended + skin_joint(palette, packed.y >> 16u) * weights.w;
    return blended;
}
fn skin_unit(v: vec3f) -> vec3f {
    let length = sqrt(dot(v, v));
    return select(v, v / length, length > 0.0);
}
fn skin_position(position: vec3f, vertex: u32, skin: vec4u) -> vec3f {
    return (skin_blend(skin, vertex, skin.y) * vec4f(position, 1.0)).xyz;
}
fn skin_shape(position: vec3f, normal: vec3f, tangent: vec4f, vertex: u32, skin: vec4u) -> DeformedVertex {
    let current = skin_blend(skin, vertex, skin.y);
    let previous = skin_blend(skin, vertex, skin.z);
    var out: DeformedVertex;
    out.position = (current * vec4f(position, 1.0)).xyz;
    out.normal = skin_unit((current * vec4f(normal, 0.0)).xyz);
    out.tangent = vec4f(skin_unit((current * vec4f(tangent.xyz, 0.0)).xyz), tangent.w);
    out.previous_position = (previous * vec4f(position, 1.0)).xyz;
    return out;
}
"#;

#[derive(Clone, Copy, Debug)]
pub struct SkinData<'a> {
    pub joints: &'a [[u16; 4]],
    pub weights: &'a [[f32; 4]],
}

impl SkinData<'_> {
    pub fn palette(&self) -> usize {
        self.joints
            .iter()
            .zip(self.weights)
            .flat_map(|(joints, weights)| {
                joints
                    .iter()
                    .zip(weights)
                    .filter(|(_, weight)| **weight != 0.0)
                    .map(|(joint, _)| *joint as usize + 1)
            })
            .max()
            .unwrap_or(1)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InstancePose<'a> {
    pub instance: usize,
    pub joints: &'a [Matrix],
    pub previous: &'a [Matrix],
}

#[derive(Clone, Debug, PartialEq)]
struct StoredPose {
    instance: usize,
    joints: Vec<Matrix>,
    previous: Vec<Matrix>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    pub current: u32,
    pub previous: u32,
    pub joints: u32,
}

pub(crate) struct PoseLayout {
    pub placed: Vec<(usize, Placed)>,
    pub words: Vec<[u32; 4]>,
    pub base: u32,
}

impl PoseLayout {
    pub fn get(&self, instance: usize) -> Option<Placed> {
        self.placed
            .binary_search_by_key(&instance, |(index, _)| *index)
            .ok()
            .map(|at| self.placed[at].1)
    }
}

pub(crate) struct SkinStore {
    records: Vec<[u32; 4]>,
    free: FreeRanges,
    palette_capacity: usize,
    buffer: wgpu::Buffer,
    poses: RefCell<Vec<StoredPose>>,
    layout: RefCell<PoseLayout>,
    pending: Cell<bool>,
}

fn words(matrices: &[Matrix]) -> impl Iterator<Item = [u32; 4]> + '_ {
    matrices
        .iter()
        .flat_map(|matrix| matrix.iter().map(|column| column.map(f32::to_bits)))
}

fn layout(base: u32, poses: &[StoredPose]) -> PoseLayout {
    let mut words_out = Vec::new();
    let mut placed = Vec::with_capacity(poses.len());
    for pose in poses {
        let current = base + words_out.len() as u32;
        words_out.extend(words(&pose.joints));
        let previous = base + words_out.len() as u32;
        words_out.extend(words(&pose.previous));
        placed.push((
            pose.instance,
            Placed {
                current,
                previous,
                joints: pose.joints.len() as u32,
            },
        ));
    }
    PoseLayout {
        placed,
        words: words_out,
        base,
    }
}

impl SkinStore {
    pub fn new(device: &wgpu::Device) -> Self {
        let palette_capacity = 4;
        Self {
            records: Vec::new(),
            free: FreeRanges::default(),
            palette_capacity,
            buffer: skin_buffer(device, &[], palette_capacity),
            poses: RefCell::new(Vec::new()),
            layout: RefCell::new(layout(0, &[])),
            pending: Cell::new(true),
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        self.buffer = skin_buffer(device, &self.records, self.palette_capacity);
        self.pending.set(true);
    }

    pub fn allocate(
        &mut self,
        device: &wgpu::Device,
        skin: SkinData<'_>,
    ) -> Result<Range<u32>, String> {
        let words: Vec<[u32; 4]> = skin
            .joints
            .iter()
            .zip(skin.weights)
            .flat_map(|(joints, weights)| {
                [
                    [
                        u32::from(joints[0]) | u32::from(joints[1]) << 16,
                        u32::from(joints[2]) | u32::from(joints[3]) << 16,
                        0,
                        0,
                    ],
                    weights.map(f32::to_bits),
                ]
            })
            .collect();
        let base = self
            .free
            .take(&mut self.records, words.len(), [0; 4], "skinned vertices")?;
        self.records[base as usize..base as usize + words.len()].copy_from_slice(&words);
        *self.layout.get_mut() = layout(self.records.len() as u32, &self.poses.borrow());
        self.rebuild(device);
        Ok(base..base + words.len() as u32)
    }

    pub fn release(&mut self, range: Range<u32>) {
        self.free.give(range);
    }

    pub fn set_poses(&self, poses: &[InstancePose<'_>]) -> Result<(), String> {
        let mut stored: Vec<StoredPose> = Vec::with_capacity(poses.len());
        for pose in poses {
            if pose.joints.is_empty() || pose.joints.len() != pose.previous.len() {
                return Err(format!(
                    "the pose of instance {} needs joints and as many previous joints",
                    pose.instance
                ));
            }
            if pose.joints.len() > pfx_core::anim::MAX_JOINTS {
                return Err(format!(
                    "the pose of instance {} has {} joints, over {}",
                    pose.instance,
                    pose.joints.len(),
                    pfx_core::anim::MAX_JOINTS
                ));
            }
            if !pose
                .joints
                .iter()
                .chain(pose.previous)
                .flatten()
                .flatten()
                .all(|value| value.is_finite())
            {
                return Err(format!(
                    "the pose of instance {} is not finite",
                    pose.instance
                ));
            }
            stored.push(StoredPose {
                instance: pose.instance,
                joints: pose.joints.to_vec(),
                previous: pose.previous.to_vec(),
            });
        }
        stored.sort_by_key(|pose| pose.instance);
        if stored
            .windows(2)
            .any(|pair| pair[0].instance == pair[1].instance)
        {
            return Err("two poses name the same instance".into());
        }
        *self.layout.borrow_mut() = layout(self.records.len() as u32, &stored);
        *self.poses.borrow_mut() = stored;
        self.pending.set(true);
        Ok(())
    }

    pub fn same(&self, poses: &[InstancePose<'_>]) -> bool {
        let stored = self.poses.borrow();
        stored.len() == poses.len()
            && poses.iter().all(|pose| {
                stored
                    .binary_search_by_key(&pose.instance, |stored| stored.instance)
                    .is_ok_and(|at| {
                        stored[at].joints == pose.joints && stored[at].previous == pose.previous
                    })
            })
    }

    pub fn layout(&self) -> Ref<'_, PoseLayout> {
        self.layout.borrow()
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if !self.pending.replace(false) {
            return false;
        }
        let needed = self.layout.get_mut().words.len();
        let mut rebuilt = false;
        if needed > self.palette_capacity {
            self.palette_capacity = needed.next_power_of_two();
            self.rebuild(device);
            self.pending.set(false);
            rebuilt = true;
        }
        let layout = self.layout.get_mut();
        if !layout.words.is_empty() {
            queue.write_buffer(
                &self.buffer,
                u64::from(layout.base) * 16,
                bytemuck::cast_slice(&layout.words),
            );
        }
        rebuilt
    }
}

fn skin_buffer(device: &wgpu::Device, records: &[[u32; 4]], palette: usize) -> wgpu::Buffer {
    let mut contents = records.to_vec();
    contents.resize(records.len() + palette.max(1), [0; 4]);
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("live skins"),
        contents: bytemuck::cast_slice(&contents),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}
