use super::{BLUR_WGSL, EDGE_WGSL, JFA_WGSL, Outline, RAISE_WGSL, Smoothing, Wire};
use pfx_gpu::GpuProfiler;

pub(super) struct Pipeline {
    blur: wgpu::ComputePipeline,
    blur_binds: [wgpu::BindGroup; 6],
    blur_uniforms: [wgpu::Buffer; 6],
    jfa_init: wgpu::ComputePipeline,
    jfa_step: wgpu::ComputePipeline,
    jfa_binds: Vec<wgpu::BindGroup>,
    edge: wgpu::ComputePipeline,
    edge_bind: wgpu::BindGroup,
    raise: wgpu::ComputePipeline,
    raise_bind: wgpu::BindGroup,
    wires: wgpu::Buffer,
    wire: wgpu::Buffer,
    outline: wgpu::Buffer,
    groups: [u32; 2],
    edge_texture: wgpu::Texture,
    edge_view: wgpu::TextureView,
}

fn texture(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&Default::default())
}

fn uniform(device: &wgpu::Device, size: u64, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn input(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn output(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba16Float,
            view_dimension: wgpu::TextureViewDimension::D2,
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

fn bind_three(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &wgpu::TextureView,
    target: &wgpu::TextureView,
    param: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Fluid field pass"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(target),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: param.as_entire_binding(),
            },
        ],
    })
}

fn compute(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &str,
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(entry),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            }),
        ),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

impl Pipeline {
    pub(super) fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        raw: &wgpu::TextureView,
        height_out: &wgpu::TextureView,
    ) -> Self {
        let across = view(&texture(device, width, height, "Liquid across"));
        let mask_across = view(&texture(device, width, height, "Liquid outline across"));
        let mask = view(&texture(device, width, height, "Liquid outline"));
        let seeds = [
            view(&texture(device, width, height, "Edge seeds A")),
            view(&texture(device, width, height, "Edge seeds B")),
        ];
        let edge_texture = texture(device, width, height, "Edge distance");
        let edge_view = view(&edge_texture);
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Fluid blur"),
            entries: &[input(0), output(1), uniform_entry(2)],
        });
        let blur_uniforms = std::array::from_fn(|_| uniform(device, 32, "Fluid blur"));
        let blur_binds = [
            bind_three(device, &blur_layout, raw, &across, &blur_uniforms[0]),
            bind_three(device, &blur_layout, &across, raw, &blur_uniforms[1]),
            bind_three(device, &blur_layout, raw, &mask_across, &blur_uniforms[2]),
            bind_three(device, &blur_layout, &mask_across, &mask, &blur_uniforms[3]),
            bind_three(device, &blur_layout, raw, &across, &blur_uniforms[4]),
            bind_three(device, &blur_layout, &across, raw, &blur_uniforms[5]),
        ];
        let strides = [256.0f32, 128.0, 64.0, 32.0, 16.0, 8.0, 4.0, 2.0, 1.0, 1.0];
        let jump = |stride: f32| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Edge jump"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: true,
            });
            let mut data = buffer.slice(..).get_mapped_range_mut();
            data.copy_from_slice(bytemuck::cast_slice(&[stride, 0.0, 0.0, 0.0]));
            drop(data);
            buffer.unmap();
            buffer
        };
        let mut jfa_binds = vec![bind_three(
            device,
            &blur_layout,
            &mask,
            &seeds[0],
            &jump(0.0),
        )];
        for (k, stride) in strides.iter().enumerate() {
            jfa_binds.push(bind_three(
                device,
                &blur_layout,
                &seeds[k % 2],
                &seeds[(k + 1) % 2],
                &jump(*stride),
            ));
        }
        let edge_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Fluid edge"),
            entries: &[input(0), input(1), output(2)],
        });
        let edge_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Fluid edge"),
            layout: &edge_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mask),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&seeds[strides.len() % 2]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&edge_view),
                },
            ],
        });
        let wires = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Fluid wires"),
            size: 256 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let wire = uniform(device, 16, "Fluid wire look");
        let outline = uniform(device, 32, "Fluid outline");
        let raise_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Fluid raise"),
            entries: &[
                input(0),
                output(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                uniform_entry(3),
                input(4),
                uniform_entry(5),
            ],
        });
        let raise_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Fluid raise"),
            layout: &raise_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(raw),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(height_out),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wires.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wire.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&edge_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: outline.as_entire_binding(),
                },
            ],
        });
        Self {
            blur: compute(device, &blur_layout, BLUR_WGSL, "blur_pass"),
            blur_binds,
            blur_uniforms,
            jfa_init: compute(device, &blur_layout, JFA_WGSL, "jfa_init"),
            jfa_step: compute(device, &blur_layout, JFA_WGSL, "jfa_step"),
            jfa_binds,
            edge: compute(device, &edge_layout, EDGE_WGSL, "edge_final"),
            edge_bind,
            raise: compute(device, &raise_layout, RAISE_WGSL, "raise"),
            raise_bind,
            wires,
            wire,
            outline,
            groups: [width.div_ceil(8), height.div_ceil(8)],
            edge_texture,
            edge_view,
        }
    }

    pub(super) fn edge_view(&self) -> &wgpu::TextureView {
        &self.edge_view
    }

    pub(super) fn edge_texture(&self) -> &wgpu::Texture {
        &self.edge_texture
    }

    pub(super) fn update(
        &self,
        queue: &wgpu::Queue,
        smoothing: Smoothing,
        outline: Outline,
        segments: &[[f32; 4]],
        wire: Wire,
    ) {
        let [wide, pool] = smoothing.residuals();
        let data = [
            [1.0, 0.0, smoothing.fine, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, smoothing.fine, 0.0, 0.0, 0.0, 0.0, 0.0],
            [
                1.0,
                0.0,
                outline.shape[0],
                outline.shape[3],
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            [0.0, 1.0, outline.shape[0], 0.0, 0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, wide, -0.01, pool, 2.6, 0.0, 0.0],
            [0.0, 1.0, wide, -0.01, pool, 2.6, 0.0, 0.0],
        ];
        for (buffer, values) in self.blur_uniforms.iter().zip(&data) {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(values));
        }
        let segments = &segments[..segments.len().min(256)];
        if !segments.is_empty() {
            queue.write_buffer(&self.wires, 0, bytemuck::cast_slice(segments));
        }
        queue.write_buffer(
            &self.wire,
            0,
            bytemuck::cast_slice(&[segments.len() as f32, wire.radius, wire.lift, wire.period]),
        );
        let outline_data = [outline.shape, outline.size].concat();
        queue.write_buffer(&self.outline, 0, bytemuck::cast_slice(&outline_data));
    }

    pub(super) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        {
            let stamp = profiler
                .as_deref_mut()
                .and_then(|profile| profile.pass("fluid blur"));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid blur"),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|profile| profile.compute_writes(stamp)),
            });
            pass.set_pipeline(&self.blur);
            for bind in &self.blur_binds {
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(self.groups[0], self.groups[1], 1);
            }
        }
        {
            let stamp = profiler
                .as_deref_mut()
                .and_then(|profile| profile.pass("fluid edges"));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid edges"),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|profile| profile.compute_writes(stamp)),
            });
            for (k, bind) in self.jfa_binds.iter().enumerate() {
                pass.set_pipeline(if k == 0 {
                    &self.jfa_init
                } else {
                    &self.jfa_step
                });
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(self.groups[0], self.groups[1], 1);
            }
            pass.set_pipeline(&self.edge);
            pass.set_bind_group(0, &self.edge_bind, &[]);
            pass.dispatch_workgroups(self.groups[0], self.groups[1], 1);
        }
        let stamp = profiler
            .as_deref_mut()
            .and_then(|profile| profile.pass("fluid raise"));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Fluid raise"),
            timestamp_writes: profiler
                .as_deref()
                .and_then(|profile| profile.compute_writes(stamp)),
        });
        pass.set_pipeline(&self.raise);
        pass.set_bind_group(0, &self.raise_bind, &[]);
        pass.dispatch_workgroups(self.groups[0], self.groups[1], 1);
    }
}
