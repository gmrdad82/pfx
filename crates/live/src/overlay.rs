use bytemuck::{Pod, Zeroable};
use pfx_gpu::{GpuProfiler, wgpu};

use crate::frame::{Frame, Instance, Matrix};
use crate::morph::{MORPH_BINDING, MORPH_WGSL, NO_MORPH};
use crate::skin::{NO_SKIN, SKIN_BINDING, SKIN_WGSL};

pub const DEFORMED_STRIDE: u64 = 16;
const WORKGROUP: u32 = 64;
const JOB_STRIDE: u64 = 256;

const SHADER: &str = r#"
struct DeformedVertex {
    position: vec3f,
    normal: vec3f,
    tangent: vec4f,
    previous_position: vec3f,
}
struct Job {
    model: mat4x4f,
    skin: vec4u,
    morph: vec4u,
    span: vec4u,
}
@group(0) @binding(0) var<storage, read> rest: array<f32>;
@group(0) @binding(1) var<storage, read_write> world: array<vec4f>;
@group(0) @binding(2) var<uniform> job: Job;
@compute @workgroup_size(64) fn deform_positions(@builtin(global_invocation_id) id: vec3u) {
    let vertex = id.x;
    if (vertex >= job.span.x) {
        return;
    }
    var position = vec3f(rest[vertex * 3u], rest[vertex * 3u + 1u], rest[vertex * 3u + 2u]);
    if (job.morph.x != 0xffffffffu) {
        position = morph_position(position, vertex, job.morph);
    }
    if (job.skin.x != 0xffffffffu) {
        position = skin_position(position, vertex, job.skin);
    }
    world[job.span.y + vertex] = job.model * vec4f(position, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Job {
    model: Matrix,
    skin: [u32; 4],
    morph: [u32; 4],
    span: [u32; 4],
}

pub(crate) struct DeformSource<'a> {
    pub positions: &'a wgpu::Buffer,
    pub vertices: u32,
    pub model: Matrix,
    pub skin: [u32; 4],
    pub morph: [u32; 4],
}

impl DeformSource<'_> {
    pub fn deforms(&self) -> bool {
        self.skin[0] != NO_SKIN || self.morph[0] != NO_MORPH
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Deformed<'a> {
    pub buffer: &'a wgpu::Buffer,
    pub offset: u64,
    pub vertices: u32,
}

impl Deformed<'_> {
    pub fn size(&self) -> u64 {
        u64::from(self.vertices) * DEFORMED_STRIDE
    }

    pub fn slice(&self) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(self.offset..self.offset + self.size())
    }
}

pub(crate) struct DeformedPositions {
    requested: Vec<usize>,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    jobs: Option<wgpu::Buffer>,
    output: Option<wgpu::Buffer>,
    ranges: Vec<(usize, u64, u32)>,
}

pub fn deformed_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    vec![
        storage(0, true),
        storage(1, false),
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Job>() as u64),
            },
            count: None,
        },
        storage(SKIN_BINDING, true),
        storage(MORPH_BINDING, true),
    ]
}

impl DeformedPositions {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live deformed positions"),
            source: wgpu::ShaderSource::Wgsl(format!("{SKIN_WGSL}{MORPH_WGSL}{SHADER}").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live deformed positions"),
            entries: &deformed_entries(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live deformed positions"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("live deformed positions"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("deform_positions"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            requested: Vec::new(),
            pipeline,
            layout,
            jobs: None,
            output: None,
            ranges: Vec::new(),
        }
    }

    pub fn request(&mut self, instances: &[usize]) {
        self.requested = instances.to_vec();
        self.requested.sort_unstable();
        self.requested.dedup();
        self.ranges.clear();
    }

    pub fn get(&self, instance: usize) -> Option<Deformed<'_>> {
        let output = self.output.as_ref()?;
        let at = self
            .ranges
            .binary_search_by_key(&instance, |(index, _, _)| *index)
            .ok()?;
        let (_, offset, vertices) = self.ranges[at];
        Some(Deformed {
            buffer: output,
            offset,
            vertices,
        })
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        instances: &[Instance],
        profiler: &mut GpuProfiler,
    ) -> Result<(), String> {
        self.ranges.clear();
        let limit = device.limits().max_compute_workgroups_per_dimension;
        let mut sources = Vec::new();
        let mut total = 0u64;
        for &index in &self.requested {
            let Some(instance) = instances.get(index) else {
                continue;
            };
            let Some(source) = frame.deform_source(index, instance.mesh) else {
                continue;
            };
            if !source.deforms() || source.vertices == 0 {
                continue;
            }
            if source.vertices.div_ceil(WORKGROUP) > limit {
                return Err(format!(
                    "instance {index} has {} vertices, more than one deformed positions pass covers",
                    source.vertices
                ));
            }
            self.ranges
                .push((index, total * DEFORMED_STRIDE, source.vertices));
            sources.push((total as u32, source));
            total += u64::from(sources.last().unwrap().1.vertices);
        }
        if sources.is_empty() {
            return Ok(());
        }
        let bytes = total * DEFORMED_STRIDE;
        if self
            .output
            .as_ref()
            .is_none_or(|buffer| buffer.size() < bytes)
        {
            self.output = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("live deformed positions"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
        }
        let job_bytes = sources.len() as u64 * JOB_STRIDE;
        if self
            .jobs
            .as_ref()
            .is_none_or(|buffer| buffer.size() < job_bytes)
        {
            self.jobs = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("live deformed position jobs"),
                size: job_bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let (Some(output), Some(jobs)) = (self.output.as_ref(), self.jobs.as_ref()) else {
            return Ok(());
        };
        let mut packed = vec![0u8; job_bytes as usize];
        for (slot, (base, source)) in sources.iter().enumerate() {
            let job = Job {
                model: source.model,
                skin: source.skin,
                morph: source.morph,
                span: [source.vertices, *base, 0, 0],
            };
            let at = slot * JOB_STRIDE as usize;
            packed[at..at + std::mem::size_of::<Job>()].copy_from_slice(bytemuck::bytes_of(&job));
        }
        queue.write_buffer(jobs, 0, &packed);
        let (skins, morphs) = frame.deform_stores();
        let groups: Vec<wgpu::BindGroup> = sources
            .iter()
            .map(|(_, source)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("live deformed positions"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: source.positions.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: output.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: jobs,
                                offset: 0,
                                size: wgpu::BufferSize::new(std::mem::size_of::<Job>() as u64),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: SKIN_BINDING,
                            resource: skins.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: MORPH_BINDING,
                            resource: morphs.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        let timing = profiler.pass("deformed positions");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("live deformed positions"),
            timestamp_writes: profiler.compute_writes(timing),
        });
        pass.set_pipeline(&self.pipeline);
        for (slot, ((_, source), group)) in sources.iter().zip(&groups).enumerate() {
            pass.set_bind_group(0, group, &[(slot as u64 * JOB_STRIDE) as u32]);
            pass.dispatch_workgroups(source.vertices.div_ceil(WORKGROUP), 1, 1);
        }
        Ok(())
    }
}
