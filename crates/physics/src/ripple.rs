use bytemuck::{Pod, Zeroable};
use half::f16;

pub const DAMP: f32 = 0.991;
pub const DROPS: usize = 8;
pub const FIXED_DT: f32 = 1.0 / 60.0;
pub const RIPPLE_WGSL: &str = include_str!("ripple.wgsl");
pub const CAUSTICS_WGSL: &str = include_str!("caustics.wgsl");
pub const CAUSTIC_GRID: [u32; 2] = [640, 400];

fn caustic_target(device: &wgpu::Device, frame_size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Caustics"),
        size: wgpu::Extent3d {
            width: (frame_size[0] / 2).max(1),
            height: (frame_size[1] / 2).max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CausticParams {
    size: [f32; 2],
    time: f32,
    calm: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Drop {
    pub uv: [f32; 2],
    pub radius: f32,
    pub amount: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DropBlock {
    head: [f32; 4],
    items: [[f32; 4]; DROPS],
}

pub struct Ripple {
    width: u32,
    height: u32,
    now: Vec<f32>,
    prev: Vec<f32>,
    damp: f32,
    fixed_dt: f32,
    clock: f32,
}

impl Ripple {
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_damp(width, height, DAMP)
    }

    pub fn undamped(width: u32, height: u32) -> Self {
        Self::with_damp(width, height, 1.0)
    }

    fn with_damp(width: u32, height: u32, damp: f32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let n = (width * height) as usize;
        Self {
            width,
            height,
            now: vec![0.0; n],
            prev: vec![0.0; n],
            damp,
            fixed_dt: FIXED_DT,
            clock: 0.0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn heights(&self) -> &[f32] {
        &self.now
    }

    pub fn previous(&self) -> &[f32] {
        &self.prev
    }

    pub fn damp(&self) -> f32 {
        self.damp
    }

    pub fn write(&mut self, x: u32, y: u32, height: f32) {
        if let Some(slot) = self.index(x, y) {
            self.now[slot] = height;
            self.prev[slot] = height;
        }
    }

    pub fn height_at(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.now[(y * self.width + x) as usize]
    }

    pub fn step(&mut self, dt: f32, drops: &[Drop]) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.clock = (self.clock + dt).min(self.fixed_dt * 8.0);
        let steps = (self.clock / self.fixed_dt).floor() as usize;
        self.clock -= steps as f32 * self.fixed_dt;
        for step in 0..steps {
            self.iterate(if step == 0 { drops } else { &[] });
        }
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y * self.width + x) as usize)
        } else {
            None
        }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.now[(y * self.width + x) as usize]
    }

    fn iterate(&mut self, drops: &[Drop]) {
        let w = self.width as i32;
        let h = self.height as i32;
        let mut next = vec![0.0; self.now.len()];
        for y in 0..h {
            for x in 0..w {
                let around =
                    self.at(x - 1, y) + self.at(x + 1, y) + self.at(x, y - 1) + self.at(x, y + 1);
                let prev = self.prev[(y as u32 * self.width + x as u32) as usize];
                let mut height = (around * 0.5 - prev) * self.damp;
                let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
                let aspect = [w as f32 / h as f32, 1.0];
                for drop in drops.iter().take(DROPS) {
                    let q = [
                        (uv[0] - drop.uv[0]) * aspect[0] / drop.radius.max(1e-4),
                        (uv[1] - drop.uv[1]) * aspect[1] / drop.radius.max(1e-4),
                    ];
                    height += drop.amount * (-(q[0] * q[0] + q[1] * q[1])).exp();
                }
                next[(y as u32 * self.width + x as u32) as usize] = height;
            }
        }
        self.prev.clone_from(&self.now);
        self.now = next;
    }
}

pub struct RippleGpu {
    width: u32,
    height: u32,
    textures: [wgpu::Texture; 2],
    current: usize,
    drops: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    binds: [[wgpu::BindGroup; 2]; 2],
    caustic_texture: wgpu::Texture,
    caustic_pipeline: wgpu::RenderPipeline,
    caustic_binds: [wgpu::BindGroup; 2],
    caustic_params: wgpu::Buffer,
    frame_size: [u32; 2],
    fixed_dt: f32,
    clock: f32,
}

impl RippleGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let texture = |label: &str| {
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
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let textures = [texture("Ripple A"), texture("Ripple B")];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        let block = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<DropBlock>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let drops = block("Ripple drops");
        let none = block("Ripple no drops");
        queue.write_buffer(&none, 0, bytemuck::bytes_of(&block_of(&[])));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ripple"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind = |src: usize, dst: usize, input: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Ripple"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[src]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[dst]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: input.as_entire_binding(),
                    },
                ],
            })
        };
        let binds = [
            [bind(0, 1, &drops), bind(0, 1, &none)],
            [bind(1, 0, &drops), bind(1, 0, &none)],
        ];
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ripple"),
            source: wgpu::ShaderSource::Wgsl(RIPPLE_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Ripple"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Ripple"),
                    bind_group_layouts: &[&layout],
                    push_constant_ranges: &[],
                }),
            ),
            module: &module,
            entry_point: Some("ripple"),
            compilation_options: Default::default(),
            cache: None,
        });
        let caustic_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Caustic parameters"),
            size: std::mem::size_of::<CausticParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Caustic ripple sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let caustic_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Caustics"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let caustic_binds = std::array::from_fn(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Caustics"),
                layout: &caustic_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[i]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: caustic_params.as_entire_binding(),
                    },
                ],
            })
        });
        let caustic_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Caustics"),
            source: wgpu::ShaderSource::Wgsl(CAUSTICS_WGSL.into()),
        });
        let caustic_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Caustics"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Caustics"),
                    bind_group_layouts: &[&caustic_layout],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &caustic_module,
                entry_point: Some("vs_caustic"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &caustic_module,
                entry_point: Some("fs_caustic"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let frame_size = [width, height];
        let caustic_texture = caustic_target(device, frame_size);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Ripple clear"),
        });
        for view in &views {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Ripple clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            drop(pass);
        }
        queue.submit(Some(encoder.finish()));
        Self {
            width,
            height,
            textures,
            current: 0,
            drops,
            pipeline,
            binds,
            caustic_texture,
            caustic_pipeline,
            caustic_binds,
            caustic_params,
            frame_size,
            fixed_dt: FIXED_DT,
            clock: 0.0,
        }
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.textures[self.current]
    }

    pub fn caustics(&self) -> &wgpu::Texture {
        &self.caustic_texture
    }

    pub fn resize_caustics(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let frame_size = [width.max(1), height.max(1)];
        if frame_size != self.frame_size {
            self.caustic_texture = caustic_target(device, frame_size);
            self.frame_size = frame_size;
        }
    }

    pub fn render_caustics(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        time: f32,
        calm: f32,
    ) {
        queue.write_buffer(
            &self.caustic_params,
            0,
            bytemuck::bytes_of(&CausticParams {
                size: [self.frame_size[0] as f32, self.frame_size[1] as f32],
                time,
                calm,
            }),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Caustics"),
        });
        let view = self.caustic_texture.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Caustics"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.caustic_pipeline);
            pass.set_bind_group(0, &self.caustic_binds[self.current], &[]);
            pass.draw(0..CAUSTIC_GRID[0] * CAUSTIC_GRID[1] * 6, 0..3);
        }
        queue.submit(Some(encoder.finish()));
    }

    pub fn read_caustics(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<[f16; 4]> {
        let size = self.caustic_texture.size();
        let padded = (size.width * 8).div_ceil(256) * 256;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Caustic readback"),
            size: u64::from(padded) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.caustic_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size.height),
                },
            },
            size,
        );
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (send, recv) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("caustic poll");
        recv.recv()
            .expect("caustic map")
            .expect("caustic buffer map");
        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((size.width * size.height) as usize);
        for y in 0..size.height as usize {
            for x in 0..size.width as usize {
                let at = y * padded as usize + x * 8;
                pixels.push(std::array::from_fn(|channel| {
                    let i = at + channel * 2;
                    f16::from_bits(u16::from_le_bytes([data[i], data[i + 1]]))
                }));
            }
        }
        drop(data);
        staging.unmap();
        pixels
    }

    pub fn step(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, dt: f32, drops: &[Drop]) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.clock = (self.clock + dt).min(self.fixed_dt * 8.0);
        let steps = (self.clock / self.fixed_dt).floor() as usize;
        self.clock -= steps as f32 * self.fixed_dt;
        if steps == 0 {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Ripple"),
        });
        queue.write_buffer(&self.drops, 0, bytemuck::bytes_of(&block_of(drops)));
        for step in 0..steps {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Ripple"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.binds[self.current][usize::from(step > 0)], &[]);
                pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
            }
            self.current = 1 - self.current;
        }
        queue.submit(Some(encoder.finish()));
    }
}

fn block_of(drops: &[Drop]) -> DropBlock {
    let mut block = DropBlock {
        head: [drops.len().min(DROPS) as f32, 0.0, 0.0, 0.0],
        items: [[0.0; 4]; DROPS],
    };
    for (slot, drop) in block.items.iter_mut().zip(drops.iter().take(DROPS)) {
        *slot = [drop.uv[0], drop.uv[1], drop.radius, drop.amount];
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_height(heights: &[f32], size: [usize; 2], uv: [f32; 2]) -> f32 {
        let x = uv[0] * size[0] as f32 - 0.5;
        let y = uv[1] * size[1] as f32 - 0.5;
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let fx = x - ix as f32;
        let fy = y - iy as f32;
        let at = |dx: i32, dy: i32| {
            let x = (ix + dx).clamp(0, size[0] as i32 - 1) as usize;
            let y = (iy + dy).clamp(0, size[1] as i32 - 1) as usize;
            heights[y * size[0] + x]
        };
        let a = at(0, 0) * (1.0 - fx) + at(1, 0) * fx;
        let b = at(0, 1) * (1.0 - fx) + at(1, 1) * fx;
        a * (1.0 - fy) + b * fy
    }

    fn caustic_vertex(
        heights: &[f32],
        water_size: [usize; 2],
        frame_size: [f32; 2],
        cell: [u32; 2],
        channel: usize,
        time: f32,
        calm: f32,
    ) -> ([f32; 2], [f32; 2]) {
        let uv = [
            cell[0] as f32 / CAUSTIC_GRID[0] as f32 * 1.12 - 0.06,
            cell[1] as f32 / CAUSTIC_GRID[1] as f32 * 1.12 - 0.06,
        ];
        let texel = [1.0 / water_size[0] as f32, 1.0 / water_size[1] as f32];
        let l = sample_height(heights, water_size, [uv[0] - texel[0], uv[1]]);
        let r = sample_height(heights, water_size, [uv[0] + texel[0], uv[1]]);
        let d = sample_height(heights, water_size, [uv[0], uv[1] - texel[1]]);
        let u = sample_height(heights, water_size, [uv[0], uv[1] + texel[1]]);
        let aspect = frame_size[0] / frame_size[1].max(1.0);
        let c = 0.62f32.cos();
        let s = 0.62f32.sin();
        let at = [uv[0] * aspect, uv[1]];
        let p = [
            (c * at[0] - s * at[1]) * 0.55,
            (s * at[0] + c * at[1]) * 1.35,
        ];
        let waves = [
            [0.83, 0.56, 14.0, 1.122],
            [-0.44, 0.90, 19.0, 1.308],
            [0.97, -0.24, 26.0, 1.530],
            [-0.71, -0.70, 33.0, 1.723],
            [0.26, 0.97, 41.0, 1.921],
            [-0.93, 0.37, 53.0, 2.184],
            [0.60, -0.80, 67.0, 2.456],
            [-0.15, -0.99, 83.0, 2.733],
        ];
        let amps = [
            0.00829, 0.00450, 0.00241, 0.00150, 0.00096, 0.00057, 0.00036, 0.00023,
        ];
        let mut grad = [0.0; 2];
        for (wave, amp) in waves.into_iter().zip(amps) {
            let k = wave[2] * 1.25;
            let amplitude = amp / 1.25;
            let phase = (wave[0] * p[0] + wave[1] * p[1]) * k + time * calm * wave[3];
            grad[0] += amplitude * phase.cos() * wave[0] * k;
            grad[1] += amplitude * phase.cos() * wave[1] * k;
        }
        let g = [grad[0] * 0.55, grad[1] * 1.35];
        let swell = [c * g[0] + s * g[1], -s * g[0] + c * g[1]];
        let slope = [
            -((r - l) * 0.9 + swell[0]),
            -((u - d) * 0.9 + swell[1]),
            1.0,
        ];
        let length = (slope[0] * slope[0] + slope[1] * slope[1] + 1.0).sqrt();
        let n = [slope[0] / length, slope[1] / length, 1.0 / length];
        let eta = [0.736, 0.752, 0.770][channel];
        let dot = -n[2];
        let k = 1.0 - eta * eta * (1.0 - dot * dot);
        let factor = eta * dot + k.sqrt();
        let ray = [-factor * n[0], -factor * n[1], -eta - factor * n[2]];
        let landed = [
            uv[0] + ray[0] / (-ray[2]).max(0.2) * 0.66 / aspect,
            uv[1] + ray[1] / (-ray[2]).max(0.2) * 0.66,
        ];
        (uv, landed)
    }

    fn caustic_pixel(
        heights: &[f32],
        water_size: [usize; 2],
        output: [u32; 2],
        pixel: [u32; 2],
        time: f32,
        calm: f32,
    ) -> [f32; 4] {
        let frame = [output[0] as f32 * 2.0, output[1] as f32 * 2.0];
        let mut color = [0.0, 0.0, 0.0, 1.0];
        let at = [pixel[0] as f32 + 0.5, pixel[1] as f32 + 0.5];
        let near = [
            ((at[0] / output[0] as f32 + 0.06) / 1.12 * CAUSTIC_GRID[0] as f32) as i32,
            ((at[1] / output[1] as f32 + 0.06) / 1.12 * CAUSTIC_GRID[1] as f32) as i32,
        ];
        for (channel, slot) in color.iter_mut().enumerate().take(3) {
            for cy in (near[1] - 12).max(0)..=(near[1] + 12).min(CAUSTIC_GRID[1] as i32 - 1) {
                for cx in (near[0] - 12).max(0)..=(near[0] + 12).min(CAUSTIC_GRID[0] as i32 - 1) {
                    let corners = [[0, 0], [1, 0], [0, 1], [1, 1]].map(|corner| {
                        caustic_vertex(
                            heights,
                            water_size,
                            frame,
                            [cx as u32 + corner[0], cy as u32 + corner[1]],
                            channel,
                            time,
                            calm,
                        )
                    });
                    for indices in [[0, 1, 2], [2, 1, 3]] {
                        let v = indices.map(|i| corners[i]);
                        let p = v.map(|(_, after)| {
                            [after[0] * output[0] as f32, after[1] * output[1] as f32]
                        });
                        let cross = |a: [f32; 2], b: [f32; 2]| a[0] * b[1] - a[1] * b[0];
                        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
                        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1]];
                        let area = cross(e1, e2);
                        if area.abs() < 1e-9 {
                            continue;
                        }
                        let q = [at[0] - p[0][0], at[1] - p[0][1]];
                        let b = cross(q, e2) / area;
                        let c = cross(e1, q) / area;
                        if b < 0.0 || c < 0.0 || b + c > 1.0 {
                            continue;
                        }
                        let du = [v[1].0[0] - v[0].0[0], v[1].0[1] - v[0].0[1]];
                        let dv = [v[2].0[0] - v[0].0[0], v[2].0[1] - v[0].0[1]];
                        let dx = [
                            (du[0] * e2[1] - dv[0] * e1[1]) / area,
                            (du[1] * e2[1] - dv[1] * e1[1]) / area,
                        ];
                        let dy = [
                            (dv[0] * e1[0] - du[0] * e2[0]) / area,
                            (dv[1] * e1[0] - du[1] * e2[0]) / area,
                        ];
                        let before = (dx[0] * dx[0] + dx[1] * dx[1]).sqrt()
                            * (dy[0] * dy[0] + dy[1] * dy[1]).sqrt();
                        let light =
                            (before * (output[0] * output[1]) as f32).clamp(0.0, 14.0) * 0.34;
                        *slot += light;
                    }
                }
            }
        }
        color
    }

    #[test]
    fn caustic_cpu_port_maps_a_small_ripple_grid_into_three_channels() {
        let mut ripple = Ripple::new(8, 8);
        ripple.step(
            FIXED_DT,
            &[Drop {
                uv: [0.5, 0.5],
                radius: 0.2,
                amount: 0.04,
            }],
        );
        let heights: Vec<f32> = ripple
            .heights()
            .iter()
            .map(|&h| f16::from_f32(h).to_f32())
            .collect();
        let uv = caustic_vertex(&heights, [8, 8], [1280.0, 800.0], [320, 200], 0, 0.25, 1.0);
        assert!((uv.0[0] - 0.5).abs() < 1e-6);
        assert!((uv.0[1] - 0.5).abs() < 1e-6);
        let red = caustic_pixel(&heights, [8, 8], [640, 400], [320, 200], 0.25, 1.0);
        assert!(red[..3].iter().all(|v| v.is_finite() && *v > 0.0));
        assert!((red[0] - red[2]).abs() > 0.0001);
        assert_eq!(red[3], 1.0);
    }

    #[test]
    fn caustic_shader_parses() {
        naga::front::wgsl::parse_str(CAUSTICS_WGSL).unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn caustics_after_one_drop_match_the_cpu_port() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("caustic test"),
            required_features: wgpu::Features::empty(),
            ..Default::default()
        }))
        .expect("GPU device");
        let drop = Drop {
            uv: [0.5, 0.5],
            radius: 0.2,
            amount: 0.04,
        };
        let mut cpu = Ripple::new(8, 8);
        cpu.step(FIXED_DT, &[drop]);
        let heights: Vec<f32> = cpu
            .heights()
            .iter()
            .map(|&h| f16::from_f32(h).to_f32())
            .collect();
        let mut gpu = RippleGpu::new(&device, &queue, 8, 8);
        gpu.resize_caustics(&device, 1280, 800);
        gpu.step(&device, &queue, FIXED_DT, &[drop]);
        gpu.render_caustics(&device, &queue, 0.25, 1.0);
        let actual = gpu.read_caustics(&device, &queue);
        assert_eq!(actual.len(), 640 * 400);
        for pixel in [[320, 200], [200, 160], [430, 260]] {
            let expected = caustic_pixel(&heights, [8, 8], [640, 400], pixel, 0.25, 1.0);
            let seen = actual[pixel[1] as usize * 640 + pixel[0] as usize];
            for channel in 0..3 {
                let difference = (seen[channel].to_f32() - expected[channel]).abs();
                assert!(
                    difference <= 2.0 * f16::EPSILON.to_f32() * expected[channel].max(1.0),
                    "pixel {pixel:?} channel {channel}: gpu {} cpu {}",
                    seen[channel].to_f32(),
                    expected[channel]
                );
            }
        }
    }

    fn energy(now: &[f32], prev: &[f32], width: usize) -> f32 {
        let height = now.len() / width;
        let mut total = 0.0;
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let d = now[i] - prev[i];
                total += d * d;
                if x + 1 < width {
                    let g = now[i] - now[i + 1];
                    total += 0.25 * g * g;
                }
                if y + 1 < height {
                    let g = now[i] - now[i + width];
                    total += 0.25 * g * g;
                }
            }
        }
        total
    }

    #[test]
    fn one_step_matches_the_wave_stencil() {
        let mut ripple = Ripple::new(3, 3);
        ripple.write(1, 1, 1.0);
        ripple.step(FIXED_DT, &[]);
        assert_eq!(ripple.height_at(1, 1), -DAMP);
        assert_eq!(ripple.height_at(0, 1), 0.5 * DAMP);
        assert_eq!(ripple.height_at(0, 0), 0.0);
        assert_eq!(ripple.previous()[3 + 1], 1.0);
    }

    #[test]
    fn a_drop_raises_the_centre() {
        let mut ripple = Ripple::new(16, 16);
        ripple.step(
            FIXED_DT,
            &[Drop {
                uv: [0.5, 0.5],
                radius: 0.08,
                amount: 1.0,
            }],
        );
        assert!(ripple.height_at(8, 8) > ripple.height_at(0, 0));
        assert!(ripple.height_at(8, 8) > 0.5);
    }

    #[test]
    fn energy_holds_roughly_and_damping_spends_it() {
        let mut calm = Ripple::undamped(48, 48);
        let mut damped = Ripple::new(48, 48);
        for y in 0..48 {
            for x in 0..48 {
                let dx = x as f32 - 24.0;
                let dy = y as f32 - 24.0;
                let h = (-(dx * dx + dy * dy) / 36.0).exp();
                calm.write(x, y, h);
                damped.write(x, y, h);
            }
        }
        let start = energy(calm.heights(), calm.previous(), 48);
        for _ in 0..180 {
            calm.step(FIXED_DT, &[]);
            damped.step(FIXED_DT, &[]);
        }
        let held = energy(calm.heights(), calm.previous(), 48);
        let spent = energy(damped.heights(), damped.previous(), 48);
        let ratio = held / start;
        assert!(ratio > 0.5 && ratio < 2.0, "undamped ratio {ratio}");
        assert!(spent < held * 0.5, "damped {spent} undamped {held}");
        assert!(spent < start);
    }
}
