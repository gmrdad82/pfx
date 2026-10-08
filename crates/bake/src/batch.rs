use bytemuck::{Pod, Zeroable};
use pfx_gpu::{Gpu, OffscreenTarget, wgpu};
use pfx_trace::detail::Detail;
use pfx_trace::gpu::SceneBuffers;
use pfx_trace::sky::{ENVIRONMENT_WGSL, Environment, SkyCdf};
use wgpu::util::DeviceExt;

use crate::BakeScene;

pub const FACES_PER_BATCH: usize = 3072;
const WIDTH: u32 = 64;
const HEIGHT: u32 = 768;
const EMISSION: &str = "result+=indirect_capped(throughput*s.emission,bounce>1u);";
const BOUNCED_EMISSION: &str = "if (bounce>0u || frame.counts.z==0u) { result+=indirect_capped(throughput*s.emission,bounce>1u); }";

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Frame {
    size: [u32; 4],
    counts: [u32; 4],
    origin: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    lens: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FaceRecord {
    origin: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    seed: u32,
    pad: [u32; 3],
}

impl FaceRecord {
    pub fn new(origin: [f32; 3], basis: ([f32; 3], [f32; 3], [f32; 3]), seed: u32) -> Self {
        Self {
            origin: vec4(origin, 0.0),
            forward: vec4(basis.0, 0.0),
            right: vec4(basis.1, 0.0),
            up: vec4(basis.2, 0.0),
            seed,
            pad: [0; 3],
        }
    }
}

fn vec4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

fn storage(gpu: &Gpu, label: &'static str, data: &[u8]) -> wgpu::Buffer {
    if data.is_empty() {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    } else {
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: data,
                usage: wgpu::BufferUsages::STORAGE,
            })
    }
}

fn shader_source() -> Result<String, String> {
    let source = pfx_trace::TRACE_WGSL;
    let head = source
        .split_once("@compute @workgroup_size(8,8)\nfn main")
        .ok_or("trace shader entry is missing")?
        .0;
    if !head.contains(EMISSION) {
        return Err("trace shader emission term is missing".into());
    }
    let head = head.replace(EMISSION, BOUNCED_EMISSION);
    Ok(format!(
        "{}\n{}\n{}\n{}\n{}",
        pfx_materials::BRDF,
        pfx_materials::CONTENT,
        ENVIRONMENT_WGSL.as_str(),
        head,
        include_str!("batch.wgsl")
    ))
}

fn target(gpu: &Gpu) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("batch probe target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    OffscreenTarget {
        view: texture.create_view(&Default::default()),
        texture,
        format: wgpu::TextureFormat::Rgba32Float,
        width: WIDTH,
        height: HEIGHT,
    }
}

fn rows(faces: usize) -> u32 {
    let per_row = (WIDTH / 4) as usize;
    (faces.div_ceil(per_row) as u32 * 4).min(HEIGHT)
}

pub struct Batch {
    frame: wgpu::Buffer,
    records: wgpu::Buffer,
    binding: wgpu::BindGroup,
    environment: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    output: OffscreenTarget,
    base: Frame,
}

impl Batch {
    pub fn new(gpu: &Gpu, scene: &BakeScene, anchor_index: usize) -> Result<Self, String> {
        let anchor = scene
            .anchors
            .get(anchor_index)
            .ok_or("bake anchor is missing")?;
        let buffers = SceneBuffers::build(
            &scene.triangles,
            &scene.shapes,
            &scene.materials,
            &Detail::default(),
        )?;
        let shader = shader_source()?;
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("batch probe shader"),
                source: wgpu::ShaderSource::Wgsl(shader.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("batch probe pipeline"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let frame = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("batch probe frame"),
            size: std::mem::size_of::<Frame>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let records = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("batch probe faces"),
            size: (FACES_PER_BATCH * std::mem::size_of::<FaceRecord>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let triangle_buffer = storage(gpu, "batch triangles", &buffers.triangles);
        let node_buffer = storage(gpu, "batch nodes", &buffers.nodes);
        let shape_buffer = storage(gpu, "batch shapes", &buffers.shapes);
        let material_buffer = storage(gpu, "batch materials", &buffers.materials);
        let uv_buffer = storage(gpu, "batch triangle uv v", &buffers.uv);
        let content_buffer = storage(gpu, "batch content", &buffers.content);
        let content_info_buffer = storage(gpu, "batch content info", &buffers.content_info);
        let surface_buffer = storage(gpu, "batch surfaces", &buffers.surfaces);
        let layer_buffer = storage(gpu, "batch layers", &buffers.layers);
        let accum = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("batch accumulation"),
            size: u64::from(WIDTH * HEIGHT) * 48,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let output = target(gpu);
        let albedo = target(gpu);
        let normal = target(gpu);
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("batch probe bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: triangle_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: node_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: shape_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uv_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: content_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: accum.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&output.view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&albedo.view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&normal.view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: content_info_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: surface_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: layer_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: records.as_entire_binding(),
                },
            ],
        });
        let sky = &anchor.sky;
        if sky.width == 0
            || sky.height == 0
            || sky.texels.len() != sky.width as usize * sky.height as usize
        {
            return Err("invalid sky".into());
        }
        let environment_source = Environment::Hdr(sky.clone());
        let cdf = SkyCdf::build(&environment_source, 256, 128).without_sun();
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("batch sky"),
            size: wgpu::Extent3d {
                width: sky.width,
                height: sky.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&sky.texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(sky.width * 16),
                rows_per_image: Some(sky.height),
            },
            texture.size(),
        );
        let view = texture.create_view(&Default::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("batch sky uniform"),
                contents: bytemuck::bytes_of(&environment_source.uniform()),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let rows = storage(gpu, "batch sky rows", bytemuck::cast_slice(&cdf.rows));
        let columns = storage(gpu, "batch sky columns", bytemuck::cast_slice(&cdf.columns));
        let cdf_uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("batch sky cdf uniform"),
                contents: bytemuck::bytes_of(&cdf.uniform()),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let environment = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("batch sky bindings"),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: rows.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: columns.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: cdf_uniform.as_entire_binding(),
                },
            ],
        });
        let base = Frame {
            size: [WIDTH, HEIGHT, 0, 0],
            counts: [buffers.triangle_count, buffers.shape_count, 0, 0],
            origin: [0.0; 4],
            forward: [0.0; 4],
            right: [0.0; 4],
            up: [0.0; 4],
            sun_dir: vec4(anchor.sun.direction, 1.0),
            sun_color: vec4(anchor.sun.color, anchor.sun.intensity),
            lens: [0.0, 0.0, 0.0, 1.0],
        };
        Ok(Self {
            frame,
            records,
            binding,
            environment,
            pipeline,
            output,
            base,
        })
    }

    pub fn without_direct_emission(mut self) -> Self {
        self.base.counts[2] = 1;
        self
    }

    pub fn load(&self, gpu: &Gpu, faces: &[FaceRecord]) -> Result<(), String> {
        if faces.is_empty() || faces.len() > FACES_PER_BATCH {
            return Err("invalid batch probe request".into());
        }
        gpu.queue
            .write_buffer(&self.records, 0, bytemuck::cast_slice(faces));
        Ok(())
    }

    pub fn encode(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        faces: usize,
        accumulated: u32,
        samples: u32,
    ) {
        let mut frame = self.base;
        frame.size[2] = accumulated;
        frame.size[3] = samples;
        frame.counts[3] = faces as u32;
        let staging = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("batch probe frame slice"),
                contents: bytemuck::bytes_of(&frame),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            &self.frame,
            0,
            std::mem::size_of::<Frame>() as u64,
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("batch probe trace"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_bind_group(1, &self.environment, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), rows(faces).div_ceil(8), 1);
    }

    pub fn read(&self, gpu: &Gpu) -> Result<Vec<u8>, String> {
        gpu.readback_bytes(&self.output)
    }

    pub fn sample(&self, gpu: &Gpu, faces: &[FaceRecord], samples: u32) -> Result<Vec<u8>, String> {
        if samples == 0 {
            return Err("invalid batch probe request".into());
        }
        self.load(gpu, faces)?;
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("batch probe samples"),
            });
        self.encode(gpu, &mut encoder, faces.len(), 0, samples);
        gpu.queue.submit(Some(encoder.finish()));
        self.read(gpu)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn batch_shader_validates() {
        let shader = super::shader_source().unwrap();
        assert!(shader.contains(super::BOUNCED_EMISSION));
        assert!(!shader.contains(&format!("\n        {}", super::EMISSION)));
        let module = naga::front::wgsl::parse_str(&shader).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let group_zero: std::collections::BTreeSet<u32> = module
            .global_variables
            .iter()
            .filter_map(|(_, variable)| variable.binding.as_ref())
            .filter(|binding| binding.group == 0)
            .map(|binding| binding.binding)
            .collect();
        let expected: std::collections::BTreeSet<u32> = (0..=13).chain([20]).collect();
        assert_eq!(group_zero, expected);
        let triangle = pfx_trace::bvh::Triangle {
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            material: 0,
        };
        let buffers = pfx_trace::gpu::SceneBuffers::build(
            &[triangle],
            &[],
            &[pfx_materials::Material::default()],
            &pfx_trace::detail::Detail::default(),
        )
        .unwrap();
        assert_eq!(buffers.triangles.len(), 96);
        assert_eq!(buffers.uv.len(), 16);
        assert_eq!(buffers.surfaces.len(), 64);
        assert_eq!(buffers.content_info.len(), 16);
        assert_eq!(buffers.layers.len(), 32);
        assert_eq!(std::mem::size_of::<super::Frame>(), 144);
        assert_eq!(std::mem::size_of::<super::FaceRecord>(), 80);
        assert_eq!(super::rows(1), 4);
        assert_eq!(super::rows(16), 4);
        assert_eq!(super::rows(17), 8);
        assert_eq!(super::rows(super::FACES_PER_BATCH), super::HEIGHT);
    }
}
