use std::cell::{Cell, Ref, RefCell};
use std::ops::Range;

use wgpu::util::DeviceExt;

use crate::ranges::FreeRanges;

pub const NO_MORPH: u32 = u32::MAX;
pub const MORPH_BINDING: u32 = 5;
const NORMALS: u32 = 1 << 31;

pub const MORPH_WGSL: &str = r#"
@group(0) @binding(5) var<storage, read> morph_data: array<vec4u>;
struct Morphed {
    position: vec3f,
    normal: vec3f,
    previous_position: vec3f,
}
fn morph_vertex(position: vec3f, normal: vec3f, vertex: u32, morph: vec4u) -> Morphed {
    var out: Morphed;
    out.position = position;
    out.normal = normal;
    out.previous_position = position;
    let normals = morph.w >> 31u;
    let stride = 1u + normals;
    let count = morph.w & 0x7fffffffu;
    let first = morph.x + vertex * morph.y * stride;
    for (var i = 0u; i < count; i = i + 1u) {
        let weight = morph_data[morph.z + i];
        let at = first + weight.x * stride;
        let delta = bitcast<vec4f>(morph_data[at]).xyz;
        let current = bitcast<f32>(weight.y);
        out.position = out.position + delta * current;
        out.previous_position = out.previous_position + delta * bitcast<f32>(weight.z);
        if (normals != 0u) {
            out.normal = out.normal + bitcast<vec4f>(morph_data[at + 1u]).xyz * current;
        }
    }
    return out;
}
fn morph_position(position: vec3f, vertex: u32, morph: vec4u) -> vec3f {
    let stride = 1u + (morph.w >> 31u);
    let count = morph.w & 0x7fffffffu;
    let first = morph.x + vertex * morph.y * stride;
    var out = position;
    for (var i = 0u; i < count; i = i + 1u) {
        let weight = morph_data[morph.z + i];
        out = out + bitcast<vec4f>(morph_data[first + weight.x * stride]).xyz * bitcast<f32>(weight.y);
    }
    return out;
}
"#;

#[derive(Clone, Copy, Debug)]
pub struct MorphTargetData<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: Option<&'a [[f32; 3]]>,
}

#[derive(Clone, Copy, Debug)]
pub struct MorphData<'a> {
    pub targets: &'a [MorphTargetData<'a>],
}

impl MorphData<'_> {
    pub fn check(&self, vertices: usize) -> Result<(), String> {
        if self.targets.is_empty() {
            return Err("a morphed mesh needs at least one target".into());
        }
        if self.targets.len() > NORMALS as usize - 1 {
            return Err("a morphed mesh has too many targets".into());
        }
        for (index, target) in self.targets.iter().enumerate() {
            if target.positions.len() != vertices
                || target
                    .normals
                    .is_some_and(|normals| normals.len() != vertices)
            {
                return Err(format!(
                    "morph target {index} needs one position delta per vertex, and one normal delta per vertex when it has normals"
                ));
            }
            if !target
                .positions
                .iter()
                .chain(target.normals.unwrap_or_default())
                .flatten()
                .all(|value| value.is_finite())
            {
                return Err(format!("morph target {index} is not finite"));
            }
        }
        Ok(())
    }

    pub fn normals(&self) -> bool {
        self.targets.iter().any(|target| target.normals.is_some())
    }

    pub fn stride(&self) -> u32 {
        1 + u32::from(self.normals())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InstanceWeights<'a> {
    pub instance: usize,
    pub current: &'a [f32],
    pub previous: &'a [f32],
}

#[derive(Clone, Debug, PartialEq)]
struct StoredWeights {
    instance: usize,
    current: Vec<f32>,
    previous: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    pub offset: u32,
    pub active: u32,
    pub weights: u32,
}

pub(crate) struct WeightLayout {
    pub placed: Vec<(usize, Placed)>,
    pub words: Vec<[u32; 4]>,
    pub base: u32,
}

impl WeightLayout {
    pub fn get(&self, instance: usize) -> Option<Placed> {
        self.placed
            .binary_search_by_key(&instance, |(index, _)| *index)
            .ok()
            .map(|at| self.placed[at].1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MorphRange {
    pub base: u32,
    pub len: u32,
    pub targets: u32,
    pub stride: u32,
}

impl MorphRange {
    pub const NONE: Self = Self {
        base: NO_MORPH,
        len: 0,
        targets: 0,
        stride: 0,
    };

    pub fn range(&self) -> Range<u32> {
        self.base..self.base + self.len
    }

    pub fn pack(&self, placed: Option<Placed>, instance: usize) -> Result<[u32; 4], String> {
        match placed.filter(|_| self.base != NO_MORPH) {
            Some(placed) if placed.weights > self.targets => Err(format!(
                "instance {instance} has {} morph weights where its mesh has {} targets",
                placed.weights, self.targets
            )),
            Some(placed) => Ok([
                self.base,
                self.targets,
                placed.offset,
                placed.active | if self.stride > 1 { NORMALS } else { 0 },
            ]),
            None => Ok([NO_MORPH, 0, 0, 0]),
        }
    }
}

pub(crate) struct MorphStore {
    records: Vec<[u32; 4]>,
    free: FreeRanges,
    weight_capacity: usize,
    buffer: wgpu::Buffer,
    weights: RefCell<Vec<StoredWeights>>,
    layout: RefCell<WeightLayout>,
    pending: Cell<bool>,
}

fn layout(base: u32, stored: &[StoredWeights]) -> WeightLayout {
    let mut words = Vec::new();
    let mut placed = Vec::with_capacity(stored.len());
    for weights in stored {
        let offset = base + words.len() as u32;
        words.extend(
            weights
                .current
                .iter()
                .zip(&weights.previous)
                .enumerate()
                .filter(|(_, (current, previous))| **current != 0.0 || **previous != 0.0)
                .map(|(target, (current, previous))| {
                    [target as u32, current.to_bits(), previous.to_bits(), 0]
                }),
        );
        let active = base + words.len() as u32 - offset;
        if active > 0 {
            placed.push((
                weights.instance,
                Placed {
                    offset,
                    active,
                    weights: weights.current.len() as u32,
                },
            ));
        }
    }
    WeightLayout {
        placed,
        words,
        base,
    }
}

impl MorphStore {
    pub fn new(device: &wgpu::Device) -> Self {
        let weight_capacity = 4;
        Self {
            records: Vec::new(),
            free: FreeRanges::default(),
            weight_capacity,
            buffer: morph_buffer(device, &[], weight_capacity),
            weights: RefCell::new(Vec::new()),
            layout: RefCell::new(layout(0, &[])),
            pending: Cell::new(true),
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        self.buffer = morph_buffer(device, &self.records, self.weight_capacity);
        self.pending.set(true);
    }

    pub fn allocate(
        &mut self,
        device: &wgpu::Device,
        vertices: usize,
        morph: MorphData<'_>,
    ) -> Result<MorphRange, String> {
        let stride = morph.stride();
        let none = [0.0f32; 3];
        let mut words: Vec<[u32; 4]> =
            Vec::with_capacity(vertices * morph.targets.len() * stride as usize);
        for vertex in 0..vertices {
            for target in morph.targets {
                let [x, y, z] = target.positions[vertex];
                words.push([x.to_bits(), y.to_bits(), z.to_bits(), 0]);
                if stride > 1 {
                    let [x, y, z] = target.normals.map_or(none, |normals| normals[vertex]);
                    words.push([x.to_bits(), y.to_bits(), z.to_bits(), 0]);
                }
            }
        }
        let limits = device.limits();
        let bytes = (self.records.len() + words.len() + self.weight_capacity) as u64 * 16;
        if bytes
            > limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size)
        {
            return Err(format!(
                "morph targets need {bytes} bytes, over the device's storage buffer limit"
            ));
        }
        let base = self
            .free
            .take(&mut self.records, words.len(), [0; 4], "morphed vertices")?;
        self.records[base as usize..base as usize + words.len()].copy_from_slice(&words);
        *self.layout.get_mut() = layout(self.records.len() as u32, &self.weights.borrow());
        self.rebuild(device);
        Ok(MorphRange {
            base,
            len: words.len() as u32,
            targets: morph.targets.len() as u32,
            stride,
        })
    }

    pub fn release(&mut self, range: Range<u32>) {
        self.free.give(range);
    }

    pub fn set_weights(&self, weights: &[InstanceWeights<'_>]) -> Result<(), String> {
        let mut stored: Vec<StoredWeights> = Vec::with_capacity(weights.len());
        for entry in weights {
            if entry.current.len() != entry.previous.len() {
                return Err(format!(
                    "the morph weights of instance {} need as many previous weights",
                    entry.instance
                ));
            }
            if entry.current.len() >= NORMALS as usize {
                return Err(format!(
                    "the morph weights of instance {} are too many",
                    entry.instance
                ));
            }
            if !entry
                .current
                .iter()
                .chain(entry.previous)
                .all(|value| value.is_finite())
            {
                return Err(format!(
                    "the morph weights of instance {} are not finite",
                    entry.instance
                ));
            }
            stored.push(StoredWeights {
                instance: entry.instance,
                current: entry.current.to_vec(),
                previous: entry.previous.to_vec(),
            });
        }
        stored.sort_by_key(|weights| weights.instance);
        if stored
            .windows(2)
            .any(|pair| pair[0].instance == pair[1].instance)
        {
            return Err("two morph weights name the same instance".into());
        }
        *self.layout.borrow_mut() = layout(self.records.len() as u32, &stored);
        *self.weights.borrow_mut() = stored;
        self.pending.set(true);
        Ok(())
    }

    pub fn same(&self, weights: &[InstanceWeights<'_>]) -> bool {
        let stored = self.weights.borrow();
        stored.len() == weights.len()
            && weights.iter().all(|entry| {
                stored
                    .binary_search_by_key(&entry.instance, |stored| stored.instance)
                    .is_ok_and(|at| {
                        stored[at].current == entry.current && stored[at].previous == entry.previous
                    })
            })
    }

    pub fn layout(&self) -> Ref<'_, WeightLayout> {
        self.layout.borrow()
    }

    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if !self.pending.replace(false) {
            return false;
        }
        let needed = self.layout.get_mut().words.len();
        let mut rebuilt = false;
        if needed > self.weight_capacity {
            self.weight_capacity = needed.next_power_of_two();
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

fn morph_buffer(device: &wgpu::Device, records: &[[u32; 4]], weights: usize) -> wgpu::Buffer {
    let mut contents = records.to_vec();
    contents.resize(records.len() + weights.max(1), [0; 4]);
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("live morphs"),
        contents: bytemuck::cast_slice(&contents),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_targets_with_a_weight_reach_the_gpu() {
        let stored = [
            StoredWeights {
                instance: 2,
                current: vec![0.0, 0.5, 0.0],
                previous: vec![0.0, 0.0, 0.25],
            },
            StoredWeights {
                instance: 5,
                current: vec![0.0, 0.0],
                previous: vec![0.0, 0.0],
            },
        ];
        let layout = layout(10, &stored);
        assert_eq!(
            layout.get(2),
            Some(Placed {
                offset: 10,
                active: 2,
                weights: 3
            })
        );
        assert_eq!(layout.get(5), None);
        assert_eq!(layout.words[0], [1, 0.5f32.to_bits(), 0, 0]);
        assert_eq!(layout.words[1], [2, 0, 0.25f32.to_bits(), 0]);
        let mesh = MorphRange {
            base: 0,
            len: 6,
            targets: 2,
            stride: 1,
        };
        assert!(mesh.pack(layout.get(2), 2).is_err());
        assert_eq!(
            MorphRange::NONE.pack(layout.get(2), 2),
            Ok([NO_MORPH, 0, 0, 0])
        );
    }
}
