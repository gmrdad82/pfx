use crate::frame::Matrix;
use bytemuck::{Pod, Zeroable};
use pfx_gpu::GpuProfiler;
use wgpu::util::DeviceExt;

pub const COMMAND_BYTES: u64 = 20;

const CULL_WGSL: &str = r#"
struct CullInput {
    low: vec4f,
    high: vec4f,
    index_count: u32,
    testable: u32,
    pad0: u32,
    pad1: u32,
}

struct Run {
    start: u32,
    count: u32,
}

struct Command {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

struct Params {
    view_projection: mat4x4f,
    viewport: vec4u,
    options: vec4u,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> inputs: array<CullInput>;
@group(0) @binding(2) var<storage, read_write> visible: array<u32>;
@group(0) @binding(3) var<storage, read_write> fresh: array<u32>;
@group(0) @binding(4) var<storage, read> runs: array<Run>;
@group(0) @binding(5) var<storage, read_write> early: array<Command>;
@group(0) @binding(6) var<storage, read_write> late: array<Command>;
@group(0) @binding(7) var<storage, read_write> last: array<Command>;
@group(0) @binding(8) var<storage, read_write> counts: array<u32>;
@group(0) @binding(9) var pyramid: texture_2d<f32>;
@group(0) @binding(10) var<storage, read> members: array<u32>;
@group(0) @binding(11) var<storage, read_write> tally: atomic<u32>;

fn command(index: u32, count: u32) -> Command {
    return Command(inputs[index].index_count, count, 0u, 0, index);
}

fn was_visible(index: u32) -> bool {
    return inputs[index].testable == 0u || visible[index] != 0u;
}

@compute @workgroup_size(64)
fn early_commands(@builtin(global_invocation_id) id: vec3u) {
    let slot = id.x;
    if (slot == 0u) {
        atomicStore(&tally, 0u);
    }
    if (slot >= params.viewport.z) {
        return;
    }
    let run = runs[slot];
    var kept = 0u;
    for (var k = 0u; k < run.count; k++) {
        let index = members[run.start + k];
        if (was_visible(index)) {
            early[run.start + kept] = command(index, 1u);
            kept += 1u;
        }
    }
    if (params.options.x != 0u) {
        for (var k = kept; k < run.count; k++) {
            early[run.start + k] = command(members[run.start], 0u);
        }
    }
    counts[slot] = kept;
}

fn unoccluded(low: vec3f, high: vec3f) -> bool {
    var least = vec2f(1e30);
    var most = vec2f(-1e30);
    var nearest = -1e30;
    for (var corner = 0u; corner < 8u; corner++) {
        let pick = vec3<bool>((corner & 1u) != 0u, (corner & 2u) != 0u, (corner & 4u) != 0u);
        let clip = params.view_projection * vec4f(select(low, high, pick), 1.0);
        if (clip.w <= 1e-6) {
            return true;
        }
        let ndc = clip.xyz / clip.w;
        least = min(least, ndc.xy);
        most = max(most, ndc.xy);
        nearest = max(nearest, ndc.z);
    }
    if (any(most < vec2f(-1.0)) || any(least > vec2f(1.0)) || nearest < 0.0) {
        return false;
    }
    if (nearest >= 1.0) {
        return true;
    }
    let size = vec2f(params.viewport.xy);
    let edge = size - 1.0;
    let first = vec2u(clamp(floor((vec2f(least.x, -most.y) * 0.5 + 0.5) * size), vec2f(0.0), edge));
    let final_ = vec2u(clamp(floor((vec2f(most.x, -least.y) * 0.5 + 0.5) * size), vec2f(0.0), edge));
    let extent = max(final_.x - first.x, final_.y - first.y) + 1u;
    let levels = textureNumLevels(pyramid);
    var level = 0u;
    while ((2u << level) < extent && level + 1u < levels) {
        level += 1u;
    }
    let shift = vec2u(level + 1u);
    let top = textureDimensions(pyramid, level) - 1u;
    let a = min(first >> shift, top);
    let b = min(final_ >> shift, top);
    let far = min(
        min(textureLoad(pyramid, a, level).r, textureLoad(pyramid, vec2u(b.x, a.y), level).r),
        min(textureLoad(pyramid, vec2u(a.x, b.y), level).r, textureLoad(pyramid, b, level).r),
    );
    return nearest >= far;
}

@compute @workgroup_size(64)
fn test_instances(@builtin(global_invocation_id) id: vec3u) {
    let index = id.x;
    if (index >= params.viewport.w) {
        return;
    }
    let input = inputs[index];
    fresh[index] = select(u32(unoccluded(input.low.xyz, input.high.xyz)), 1u, input.testable == 0u);
}

@compute @workgroup_size(64)
fn late_commands(@builtin(global_invocation_id) id: vec3u) {
    let slot = id.x;
    if (slot >= params.viewport.z) {
        return;
    }
    let run = runs[slot];
    var newly = 0u;
    var kept = 0u;
    for (var k = 0u; k < run.count; k++) {
        let index = members[run.start + k];
        let now = fresh[index] != 0u;
        if (now && !was_visible(index)) {
            late[run.start + newly] = command(index, 1u);
            newly += 1u;
        }
        if (now) {
            last[run.start + kept] = command(index, 1u);
            kept += 1u;
        }
        visible[index] = u32(now);
    }
    if (params.options.x != 0u) {
        for (var k = newly; k < run.count; k++) {
            late[run.start + k] = command(members[run.start], 0u);
        }
        for (var k = kept; k < run.count; k++) {
            last[run.start + k] = command(members[run.start], 0u);
        }
    }
    counts[params.viewport.z + slot] = newly;
    counts[2u * params.viewport.z + slot] = kept;
    atomicAdd(&tally, kept);
}
"#;

const REDUCE_WGSL: &str = r#"
@group(0) @binding(0) var source: SOURCE;
@group(0) @binding(1) var output: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var<uniform> sizes: vec4u;

@compute @workgroup_size(8, 8)
fn reduce(@builtin(global_invocation_id) id: vec3u) {
    if (id.x >= sizes.z || id.y >= sizes.w) {
        return;
    }
    let edge = sizes.xy - 1u;
    let base = id.xy * 2u;
    let a = textureLoad(source, min(base, edge), 0)SWIZZLE;
    let b = textureLoad(source, min(base + vec2u(1u, 0u), edge), 0)SWIZZLE;
    let c = textureLoad(source, min(base + vec2u(0u, 1u), edge), 0)SWIZZLE;
    let d = textureLoad(source, min(base + vec2u(1u, 1u), edge), 0)SWIZZLE;
    textureStore(output, id.xy, vec4f(min(min(a, b), min(c, d)), 0.0, 0.0, 1.0));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct CullInput {
    pub low: [f32; 4],
    pub high: [f32; 4],
    pub index_count: u32,
    pub testable: u32,
    pub pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view_projection: Matrix,
    viewport: [u32; 4],
    options: [u32; 4],
}

enum Tally {
    Idle,
    Copied(u32),
    Mapping(
        u32,
        std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    ),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Early,
    Late,
    Last,
}

struct Level {
    group: wgpu::BindGroup,
}

struct Pyramid {
    texture: wgpu::Texture,
    depth: wgpu::Texture,
    levels: Vec<Level>,
    viewport: [u32; 2],
}

pub struct Occlusion {
    capacity: usize,
    count: u32,
    run_capacity: usize,
    run_count: u32,
    count_draws: bool,
    inputs: wgpu::Buffer,
    visible: wgpu::Buffer,
    fresh: wgpu::Buffer,
    runs: wgpu::Buffer,
    members: wgpu::Buffer,
    early: wgpu::Buffer,
    late: wgpu::Buffer,
    last: wgpu::Buffer,
    counts: wgpu::Buffer,
    tally: wgpu::Buffer,
    tally_read: wgpu::Buffer,
    tally_state: std::cell::RefCell<Tally>,
    occluded: Option<u32>,
    params: wgpu::Buffer,
    uploaded: Vec<CullInput>,
    uploaded_runs: Vec<[u32; 2]>,
    uploaded_members: Vec<u32>,
    pyramid: Option<Pyramid>,
    cull_layout: wgpu::BindGroupLayout,
    first_layout: wgpu::BindGroupLayout,
    reduce_layout: wgpu::BindGroupLayout,
    early_pipeline: wgpu::ComputePipeline,
    test_pipeline: wgpu::ComputePipeline,
    late_pipeline: wgpu::ComputePipeline,
    first_pipeline: wgpu::ComputePipeline,
    reduce_pipeline: wgpu::ComputePipeline,
    cull_group: Option<wgpu::BindGroup>,
}

pub fn supported(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE)
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn target_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn compute(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &str,
    entry: &str,
    label: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub fn pyramid_size(viewport: [u32; 2]) -> [u32; 2] {
    viewport.map(|side| side.div_ceil(2).max(1).next_power_of_two())
}

pub fn level_size(viewport: [u32; 2], level: u32) -> [u32; 2] {
    viewport.map(|side| side.div_ceil(2u32 << level).max(1))
}

impl Occlusion {
    pub fn new(device: &wgpu::Device) -> Self {
        let cull_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion cull"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, false),
                storage_entry(4, true),
                storage_entry(5, false),
                storage_entry(6, false),
                storage_entry(7, false),
                storage_entry(8, false),
                texture_entry(9, wgpu::TextureSampleType::Float { filterable: false }),
                storage_entry(10, true),
                storage_entry(11, false),
            ],
        });
        let first_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion first level"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Depth),
                target_entry(1),
                uniform_entry(2),
            ],
        });
        let reduce_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("occlusion reduce"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                target_entry(1),
                uniform_entry(2),
            ],
        });
        let first_source = REDUCE_WGSL
            .replace("SOURCE", "texture_depth_2d")
            .replace("SWIZZLE", "");
        let reduce_source = REDUCE_WGSL
            .replace("SOURCE", "texture_2d<f32>")
            .replace("SWIZZLE", ".r");
        let early_pipeline = compute(
            device,
            &cull_layout,
            CULL_WGSL,
            "early_commands",
            "occlusion early",
        );
        let test_pipeline = compute(
            device,
            &cull_layout,
            CULL_WGSL,
            "test_instances",
            "occlusion test",
        );
        let late_pipeline = compute(
            device,
            &cull_layout,
            CULL_WGSL,
            "late_commands",
            "occlusion late",
        );
        let first_pipeline = compute(
            device,
            &first_layout,
            &first_source,
            "reduce",
            "occlusion first level",
        );
        let reduce_pipeline = compute(
            device,
            &reduce_layout,
            &reduce_source,
            "reduce",
            "occlusion reduce",
        );
        let params = buffer(
            device,
            "occlusion params",
            std::mem::size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let empty = |label| buffer(device, label, 16, wgpu::BufferUsages::STORAGE);
        let mut occlusion = Self {
            capacity: 0,
            count: 0,
            run_capacity: 0,
            run_count: 0,
            count_draws: device
                .features()
                .contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
            inputs: empty("occlusion inputs"),
            visible: empty("occlusion visible"),
            fresh: empty("occlusion fresh"),
            runs: empty("occlusion runs"),
            members: empty("occlusion members"),
            early: empty("occlusion early"),
            late: empty("occlusion late"),
            last: empty("occlusion last"),
            counts: empty("occlusion counts"),
            tally: buffer(
                device,
                "occlusion tally",
                4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            ),
            tally_read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("occlusion tally readback"),
                size: 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            tally_state: std::cell::RefCell::new(Tally::Idle),
            occluded: None,
            params,
            uploaded: Vec::new(),
            uploaded_runs: Vec::new(),
            uploaded_members: Vec::new(),
            pyramid: None,
            cull_layout,
            first_layout,
            reduce_layout,
            early_pipeline,
            test_pipeline,
            late_pipeline,
            first_pipeline,
            reduce_pipeline,
            cull_group: None,
        };
        occlusion.grow(device, 64);
        occlusion.grow_runs(device, 64);
        occlusion
    }

    fn grow(&mut self, device: &wgpu::Device, capacity: usize) {
        let commands = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT;
        let count = capacity as u64;
        self.inputs = buffer(
            device,
            "occlusion inputs",
            count * std::mem::size_of::<CullInput>() as u64,
            wgpu::BufferUsages::STORAGE,
        );
        self.visible = buffer(
            device,
            "occlusion visible",
            count * 4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        self.fresh = buffer(
            device,
            "occlusion fresh",
            count * 4,
            wgpu::BufferUsages::STORAGE,
        );
        self.members = buffer(
            device,
            "occlusion members",
            count * 4,
            wgpu::BufferUsages::STORAGE,
        );
        self.uploaded_members.clear();
        self.early = buffer(device, "occlusion early", count * COMMAND_BYTES, commands);
        self.late = buffer(device, "occlusion late", count * COMMAND_BYTES, commands);
        self.last = buffer(device, "occlusion last", count * COMMAND_BYTES, commands);
        self.capacity = capacity;
        self.uploaded.clear();
        self.cull_group = None;
    }

    fn grow_runs(&mut self, device: &wgpu::Device, capacity: usize) {
        let count = capacity as u64;
        self.runs = buffer(
            device,
            "occlusion runs",
            count * 8,
            wgpu::BufferUsages::STORAGE,
        );
        self.counts = buffer(
            device,
            "occlusion counts",
            count * 12,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
        );
        self.run_capacity = capacity;
        self.uploaded_runs.clear();
        self.cull_group = None;
    }

    fn ensure_pyramid(&mut self, device: &wgpu::Device, depth: &wgpu::Texture, viewport: [u32; 2]) {
        let size = pyramid_size([depth.width(), depth.height()]);
        let fresh = self.pyramid.as_ref().is_none_or(|pyramid| {
            pyramid.depth != *depth
                || pyramid.viewport != viewport
                || [pyramid.texture.width(), pyramid.texture.height()] != size
        });
        if !fresh {
            return;
        }
        let mips = 32 - size[0].max(size[1]).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("occlusion pyramid"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mip_view = |level: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("occlusion pyramid level"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor {
            label: Some("occlusion depth"),
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let levels = (0..mips)
            .map(|level| {
                let source = if level == 0 {
                    viewport
                } else {
                    level_size(viewport, level - 1)
                };
                let target = level_size(viewport, level);
                let sizes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("occlusion level sizes"),
                    contents: bytemuck::cast_slice(&[source[0], source[1], target[0], target[1]]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let input = if level == 0 {
                    depth_view.clone()
                } else {
                    mip_view(level - 1)
                };
                let output = mip_view(level);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("occlusion level"),
                    layout: if level == 0 {
                        &self.first_layout
                    } else {
                        &self.reduce_layout
                    },
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&output),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: sizes.as_entire_binding(),
                        },
                    ],
                });
                Level { group }
            })
            .collect();
        self.pyramid = Some(Pyramid {
            texture,
            depth: depth.clone(),
            levels,
            viewport,
        });
        self.cull_group = None;
    }

    pub fn set_inputs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        inputs: &[CullInput],
    ) -> bool {
        let grown = inputs.len() > self.capacity;
        if grown {
            self.grow(device, inputs.len().next_power_of_two());
        }
        self.count = inputs.len() as u32;
        let changed = |index: &usize| {
            self.uploaded
                .get(*index)
                .is_none_or(|old| *old != inputs[*index])
        };
        if let Some(first) = (0..inputs.len()).find(changed) {
            let last = (first..inputs.len()).rfind(changed).unwrap_or(first);
            queue.write_buffer(
                &self.inputs,
                (first * std::mem::size_of::<CullInput>()) as u64,
                bytemuck::cast_slice(&inputs[first..=last]),
            );
        }
        self.uploaded.clear();
        self.uploaded.extend_from_slice(inputs);
        grown
    }

    pub fn set_runs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        members: &[u32],
        runs: &[[u32; 2]],
    ) {
        if runs.len() > self.run_capacity {
            self.grow_runs(device, runs.len().next_power_of_two());
        }
        self.run_count = runs.len() as u32;
        if self.uploaded_members.as_slice() != members {
            queue.write_buffer(&self.members, 0, bytemuck::cast_slice(members));
            self.uploaded_members.clear();
            self.uploaded_members.extend_from_slice(members);
        }
        if self.uploaded_runs.as_slice() != runs {
            queue.write_buffer(&self.runs, 0, bytemuck::cast_slice(runs));
            self.uploaded_runs.clear();
            self.uploaded_runs.extend_from_slice(runs);
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view_projection: Matrix,
        depth: &wgpu::Texture,
        viewport: [u32; 2],
    ) {
        self.ensure_pyramid(device, depth, viewport);
        let params = Params {
            view_projection,
            viewport: [viewport[0], viewport[1], self.run_count, self.count],
            options: [u32::from(!self.count_draws), 0, 0, 0],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        if self.cull_group.is_none()
            && let Some(pyramid) = &self.pyramid
        {
            let view = pyramid.texture.create_view(&Default::default());
            let entries = [
                &self.params,
                &self.inputs,
                &self.visible,
                &self.fresh,
                &self.runs,
                &self.early,
                &self.late,
                &self.last,
                &self.counts,
            ];
            let mut bound: Vec<wgpu::BindGroupEntry<'_>> = entries
                .iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect();
            bound.push(wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(&view),
            });
            bound.push(wgpu::BindGroupEntry {
                binding: 10,
                resource: self.members.as_entire_binding(),
            });
            bound.push(wgpu::BindGroupEntry {
                binding: 11,
                resource: self.tally.as_entire_binding(),
            });
            self.cull_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("occlusion cull"),
                layout: &self.cull_layout,
                entries: &bound,
            }));
        }
    }

    pub fn read_visible(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u32> {
        let bytes = u64::from(self.count) * 4;
        if bytes == 0 {
            return Vec::new();
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occlusion visible readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.visible, 0, &staging, 0, bytes);
        queue.submit(Some(encoder.finish()));
        staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
            return Vec::new();
        }
        let Ok(mapped) = staging.slice(..).get_mapped_range() else {
            return Vec::new();
        };
        let visible = bytemuck::cast_slice(&mapped).to_vec();
        drop(mapped);
        staging.unmap();
        visible
    }

    pub fn submitted(&self) {
        let mut state = self.tally_state.borrow_mut();
        if let Tally::Copied(members) = *state {
            let (sender, receiver) = std::sync::mpsc::channel();
            self.tally_read
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            *state = Tally::Mapping(members, receiver);
        }
    }

    pub fn gather(&mut self) {
        let state = self.tally_state.get_mut();
        let Tally::Mapping(members, receiver) = state else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(())) => {
                let members = *members;
                let kept = self
                    .tally_read
                    .slice(..)
                    .get_mapped_range()
                    .ok()
                    .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                self.tally_read.unmap();
                self.occluded = kept.map(|kept| members.saturating_sub(kept));
                *state = Tally::Idle;
            }
            Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                *state = Tally::Idle;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    pub fn occluded(&self) -> Option<u32> {
        self.occluded
    }

    fn workgroups(count: u32) -> u32 {
        count.div_ceil(64)
    }

    pub fn set_count_draws(&mut self, device: &wgpu::Device, on: bool) {
        self.count_draws = on
            && device
                .features()
                .contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        phase: Phase,
        slot: usize,
        start: u32,
        len: u32,
    ) {
        let (commands, section) = match phase {
            Phase::Early => (&self.early, 0),
            Phase::Late => (&self.late, 1),
            Phase::Last => (&self.last, 2),
        };
        let offset = u64::from(start) * COMMAND_BYTES;
        if self.count_draws {
            let at = (u64::from(section * self.run_count) + slot as u64) * 4;
            pass.multi_draw_indexed_indirect_count(commands, offset, &self.counts, at, len);
        } else {
            pass.multi_draw_indexed_indirect(commands, offset, len);
        }
    }

    pub fn encode_early(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        profiler: Option<&mut GpuProfiler>,
    ) {
        let Some(group) = &self.cull_group else {
            return;
        };
        let timing = profiler.map(|timer| {
            let index = timer.pass("occlusion commands");
            (timer as &GpuProfiler, index)
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("occlusion commands"),
            timestamp_writes: timing.and_then(|(timer, index)| timer.compute_writes(index)),
        });
        pass.set_pipeline(&self.early_pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(Self::workgroups(self.run_count), 1, 1);
    }

    pub fn encode_late(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        profiler: Option<&mut GpuProfiler>,
    ) {
        let (Some(group), Some(pyramid)) = (&self.cull_group, &self.pyramid) else {
            return;
        };
        let timing = profiler.map(|timer| {
            let index = timer.pass("occlusion test");
            (timer as &GpuProfiler, index)
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("occlusion test"),
            timestamp_writes: timing.and_then(|(timer, index)| timer.compute_writes(index)),
        });
        for (index, level) in pyramid.levels.iter().enumerate() {
            let size = level_size(pyramid.viewport, index as u32);
            pass.set_pipeline(if index == 0 {
                &self.first_pipeline
            } else {
                &self.reduce_pipeline
            });
            pass.set_bind_group(0, &level.group, &[]);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        pass.set_bind_group(0, group, &[]);
        pass.set_pipeline(&self.test_pipeline);
        pass.dispatch_workgroups(Self::workgroups(self.count), 1, 1);
        pass.set_pipeline(&self.late_pipeline);
        pass.dispatch_workgroups(Self::workgroups(self.run_count), 1, 1);
        drop(pass);
        let mut state = self.tally_state.borrow_mut();
        if matches!(*state, Tally::Idle) {
            encoder.copy_buffer_to_buffer(&self.tally, 0, &self.tally_read, 0, 4);
            *state = Tally::Copied(self.uploaded_members.len() as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pyramid_level_covers_the_whole_viewport_and_fits_its_texture() {
        for viewport in [[2560, 1440], [3840, 2160], [1, 1], [7, 3], [1919, 1081]] {
            let size = pyramid_size(viewport);
            let mips = 32 - size[0].max(size[1]).leading_zeros();
            for level in 0..mips {
                let logical = level_size(viewport, level);
                let texture = size.map(|side| (side >> level).max(1));
                assert!(logical[0] <= texture[0] && logical[1] <= texture[1]);
                for axis in 0..2 {
                    assert!(logical[axis] * (2 << level) >= viewport[axis]);
                }
            }
            assert_eq!(level_size(viewport, mips - 1), [1, 1]);
        }
    }

    #[test]
    fn the_shaders_parse() {
        let first = REDUCE_WGSL
            .replace("SOURCE", "texture_depth_2d")
            .replace("SWIZZLE", "");
        let reduce = REDUCE_WGSL
            .replace("SOURCE", "texture_2d<f32>")
            .replace("SWIZZLE", ".r");
        for source in [CULL_WGSL, first.as_str(), reduce.as_str()] {
            let module = naga::front::wgsl::parse_str(source).expect("wgsl parses");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("wgsl validates");
        }
    }
}
