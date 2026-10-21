use pfx_gpu::{Gpu, OffscreenTarget, wgpu};

use crate::Error;

const SUM: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const MIX_BYTES: u64 = 16;

pub struct Mean {
    size: [u32; 2],
    sums: [wgpu::TextureView; 2],
    current: usize,
    total: u32,
    out: OffscreenTarget,
    layout: wgpu::BindGroupLayout,
    add: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    mix: wgpu::Buffer,
}

impl Mean {
    pub fn new(gpu: &Gpu, size: [u32; 2]) -> Result<Mean, Error> {
        let out = gpu.offscreen(size[0], size[1], OUT).map_err(Error)?;
        let device = &gpu.device;
        let sum = || {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("pfx shutter sum"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SUM,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pfx shutter mean"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pfx shutter mean"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(MIX_BYTES),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pfx shutter mean"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("pfx shutter mean"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("cover"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mix = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pfx shutter mean"),
            size: MIX_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Mean {
            size,
            sums: [sum(), sum()],
            current: 0,
            total: 0,
            out,
            add: pipeline("add", SUM),
            resolve: pipeline("resolve", OUT),
            layout,
            mix,
        })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn total(&self) -> u32 {
        self.total
    }

    pub fn clear(&mut self) {
        self.total = 0;
    }

    pub fn add(&mut self, gpu: &Gpu, frame: &wgpu::TextureView, weight: u32) {
        let keep = if self.total == 0 { 0.0 } else { 1.0 };
        let (from, to) = (self.current, 1 - self.current);
        self.draw(gpu, (frame, from), &self.sums[to], &self.add, [weight as f32, keep, 0.0, 0.0]);
        self.current = to;
        self.total += weight;
    }

    pub fn resolve(&mut self, gpu: &Gpu) -> Result<Vec<u8>, Error> {
        if self.total == 0 {
            return Err(Error("a shutter's mean has no subframes".into()));
        }
        let sum = &self.sums[self.current];
        self.draw(
            gpu,
            (sum, self.current),
            &self.out.view,
            &self.resolve,
            [0.0, 0.0, self.total as f32, 0.0],
        );
        self.total = 0;
        gpu.readback_rgba8(&self.out).map_err(Error)
    }

    fn draw(
        &self,
        gpu: &Gpu,
        (frame, sum): (&wgpu::TextureView, usize),
        target: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        mix: [f32; 4],
    ) {
        let bytes: Vec<u8> = mix.iter().flat_map(|value| value.to_le_bytes()).collect();
        gpu.queue.write_buffer(&self.mix, 0, &bytes);
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pfx shutter mean"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(frame),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.sums[sum]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.mix.as_entire_binding(),
                },
            ],
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx shutter mean"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pfx shutter mean"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
    }
}

pub fn linear(code: u8) -> f64 {
    let value = f64::from(code) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub fn encoded(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    let code = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    code * 255.0
}

pub fn mean_of(frames: &[(&[u8], u32)]) -> Vec<f64> {
    let total: u32 = frames.iter().map(|(_, weight)| weight).sum();
    let length = frames.first().map_or(0, |(rgba, _)| rgba.len());
    (0..length)
        .map(|at| {
            let sum: f64 = frames
                .iter()
                .map(|(rgba, weight)| {
                    let value = if at % 4 == 3 {
                        f64::from(rgba[at]) / 255.0
                    } else {
                        linear(rgba[at])
                    };
                    value * f64::from(*weight)
                })
                .sum();
            let value = sum / f64::from(total.max(1));
            if at % 4 == 3 {
                value * 255.0
            } else {
                encoded(value)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;

const SHADER: &str = r#"
struct Mix {
    weight: f32,
    keep: f32,
    total: f32,
    unused: f32,
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var sum: texture_2d<f32>;
@group(0) @binding(2) var<uniform> mix: Mix;

@vertex
fn cover(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}

@fragment
fn add(@builtin(position) at: vec4f) -> @location(0) vec4f {
    let pixel = vec2i(at.xy);
    return textureLoad(sum, pixel, 0) * mix.keep + textureLoad(frame, pixel, 0) * mix.weight;
}

@fragment
fn resolve(@builtin(position) at: vec4f) -> @location(0) vec4f {
    return textureLoad(sum, vec2i(at.xy), 0) / mix.total;
}
"#;
