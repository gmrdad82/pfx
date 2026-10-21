use std::cell::{Cell, Ref, RefCell};
use std::ops::Range;

use crate::ranges::FreeRanges;

pub const NO_MORPH: u32 = u32::MAX;
pub const MORPH_BINDING: u32 = 5;
pub const MORPH_WEIGHTS_BINDING: u32 = 6;
const NORMALS: u32 = 1 << 31;

pub const MORPH_WGSL: &str = r#"
@group(0) @binding(5) var<storage, read> morph_data: array<vec4u>;
@group(0) @binding(6) var<storage, read> morph_weights: array<vec4u>;
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
        let weight = morph_weights[morph.z + i];
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
        let weight = morph_weights[morph.z + i];
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
    extent: Vec<()>,
    free: FreeRanges,
    data: wgpu::Buffer,
    data_capacity: usize,
    last_growth: Option<wgpu::SubmissionIndex>,
    weight_buffer: wgpu::Buffer,
    weight_capacity: usize,
    weights: RefCell<Vec<StoredWeights>>,
    layout: RefCell<WeightLayout>,
    pending: Cell<bool>,
    stats: MorphStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MorphStats {
    pub data_buffers: u32,
    pub data_capacity: u64,
    pub data_written: u64,
    pub data_copied: u64,
    pub weight_buffers: u32,
    pub weight_capacity: u64,
    pub weight_written: u64,
}

const WORD: u64 = 16;
const FIRST_DATA: usize = 64;
const FIRST_WEIGHTS: usize = 4;

fn grown(capacity: usize, needed: usize, limit: usize) -> usize {
    if needed <= capacity {
        return capacity;
    }
    needed
        .next_power_of_two()
        .max(capacity.saturating_mul(2))
        .min(limit)
        .max(needed)
}

fn limit_words(device: &wgpu::Device) -> usize {
    let limits = device.limits();
    let bytes = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    usize::try_from(bytes / WORD).unwrap_or(usize::MAX)
}

fn layout(stored: &[StoredWeights]) -> WeightLayout {
    let mut words = Vec::new();
    let mut placed = Vec::with_capacity(stored.len());
    for weights in stored {
        let offset = words.len() as u32;
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
        let active = words.len() as u32 - offset;
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
    WeightLayout { placed, words }
}

impl MorphStore {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            extent: Vec::new(),
            free: FreeRanges::default(),
            data: morph_buffer(device, "live morphs", FIRST_DATA),
            data_capacity: FIRST_DATA,
            last_growth: None,
            weight_buffer: morph_buffer(device, "live morph weights", FIRST_WEIGHTS),
            weight_capacity: FIRST_WEIGHTS,
            weights: RefCell::new(Vec::new()),
            layout: RefCell::new(layout(&[])),
            pending: Cell::new(true),
            stats: MorphStats {
                data_buffers: 1,
                data_capacity: FIRST_DATA as u64 * WORD,
                weight_buffers: 1,
                weight_capacity: FIRST_WEIGHTS as u64 * WORD,
                ..MorphStats::default()
            },
        }
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.data
    }

    pub fn weight_buffer(&self) -> &wgpu::Buffer {
        &self.weight_buffer
    }

    pub fn stats(&self) -> MorphStats {
        self.stats
    }

    fn grow_data(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        needed: usize,
        keep: usize,
    ) -> Result<bool, String> {
        let capacity = grown(self.data_capacity, needed, limit_words(device));
        if capacity == self.data_capacity {
            return Ok(false);
        }
        if let Some(index) = self.last_growth.take() {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                })
                .map_err(|error| format!("growing the morph targets: {error}"))?;
        }
        let next = morph_buffer(device, "live morphs", capacity);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live morphs grow"),
        });
        let kept = keep as u64 * WORD;
        if kept > 0 {
            encoder.copy_buffer_to_buffer(&self.data, 0, &next, 0, kept);
        }
        self.last_growth = Some(queue.submit(Some(encoder.finish())));
        self.data = next;
        self.data_capacity = capacity;
        self.stats.data_buffers += 1;
        self.stats.data_capacity = capacity as u64 * WORD;
        self.stats.data_copied += kept;
        Ok(true)
    }

    pub fn reserve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        deltas: usize,
    ) -> Result<bool, String> {
        if deltas > limit_words(device) {
            return Err(format!(
                "{deltas} morph deltas are over the device's storage buffer limit"
            ));
        }
        let keep = self.extent.len();
        self.grow_data(device, queue, deltas, keep)
    }

    pub fn allocate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: usize,
        morph: MorphData<'_>,
    ) -> Result<(MorphRange, bool), String> {
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
        let limit = limit_words(device);
        let keep = self.extent.len();
        let base = self
            .free
            .take(&mut self.extent, words.len(), (), "morphed vertices")?;
        if self.extent.len() > limit {
            let bytes = self.extent.len() as u64 * WORD;
            self.extent.truncate(base as usize);
            return Err(format!(
                "morph targets need {bytes} bytes, over the device's storage buffer limit"
            ));
        }
        let range = MorphRange {
            base,
            len: words.len() as u32,
            targets: morph.targets.len() as u32,
            stride,
        };
        let grew = match self.grow_data(device, queue, self.extent.len(), keep) {
            Ok(grew) => grew,
            Err(error) => {
                if base as usize >= keep {
                    self.extent.truncate(keep);
                } else {
                    self.release(range.range());
                }
                return Err(error);
            }
        };
        if !words.is_empty() {
            let bytes: &[u8] = bytemuck::cast_slice(&words);
            queue.write_buffer(&self.data, u64::from(base) * WORD, bytes);
            self.stats.data_written += bytes.len() as u64;
        }
        Ok((range, grew))
    }

    pub fn release(&mut self, range: Range<u32>) {
        self.free.give(range);
    }

    pub fn reserve_weights(&mut self, device: &wgpu::Device, weights: usize) -> bool {
        if weights <= self.weight_capacity {
            return false;
        }
        self.grow_weights(device, weights);
        self.pending.set(true);
        true
    }

    fn grow_weights(&mut self, device: &wgpu::Device, needed: usize) {
        self.weight_capacity = grown(self.weight_capacity, needed, limit_words(device));
        self.weight_buffer = morph_buffer(device, "live morph weights", self.weight_capacity);
        self.stats.weight_buffers += 1;
        self.stats.weight_capacity = self.weight_capacity as u64 * WORD;
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
        *self.layout.borrow_mut() = layout(&stored);
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
        let grew = needed > self.weight_capacity;
        if grew {
            self.grow_weights(device, needed);
        }
        let words = &self.layout.get_mut().words;
        if !words.is_empty() {
            let bytes: &[u8] = bytemuck::cast_slice(words);
            queue.write_buffer(&self.weight_buffer, 0, bytes);
            self.stats.weight_written += bytes.len() as u64;
        }
        grew
    }
}

fn morph_buffer(device: &wgpu::Device, label: &str, words: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: words.max(1) as u64 * WORD,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
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
        let layout = layout(&stored);
        assert_eq!(
            layout.get(2),
            Some(Placed {
                offset: 0,
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

    #[test]
    fn appending_meshes_grows_the_capacity_a_logarithmic_number_of_times() {
        let mut capacity = FIRST_DATA;
        let mut growths = 0u32;
        let mut copied = 0usize;
        let mut extent = 0usize;
        for _ in 0..10_000 {
            let next = grown(capacity, extent + 37, usize::MAX);
            if next != capacity {
                assert!(next >= capacity * 2);
                growths += 1;
                copied += extent;
                capacity = next;
            }
            extent += 37;
            assert!(capacity >= extent);
        }
        assert!(growths <= extent.ilog2() + 1, "{growths} growths");
        assert!(copied < 2 * extent, "{copied} words copied for {extent}");
        assert_eq!(grown(8, 9, 12), 12);
        assert_eq!(grown(8, 20, 12), 20);
        assert_eq!(grown(8, 8, 12), 8);
    }
}
