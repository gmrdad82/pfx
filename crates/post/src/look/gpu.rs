use super::{Backdrop, Blend, Crt, Dither, Place, Quantise, UpscaleMap, Vignette, wipe_front};
use pfx_gpu::GpuProfiler;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const SLOT: u64 = 512;
pub const PALETTE_BYTES: u64 = 8192;
const ALIGN: u64 = 256;
const GLOW_REACH: f32 = 2.0;
const GLOW_FACTOR_MAX: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Backdrop,
    Quantise,
    Upscale,
    Vignette,
    Mix,
    Copy,
    Bright,
    Blur,
    Crt,
}

const KINDS: [(Kind, &str); 9] = [
    (Kind::Backdrop, "look_backdrop"),
    (Kind::Quantise, "look_quantise"),
    (Kind::Upscale, "look_upscale"),
    (Kind::Vignette, "look_vignette"),
    (Kind::Mix, "look_mix"),
    (Kind::Copy, "look_copy"),
    (Kind::Bright, "look_bright"),
    (Kind::Blur, "look_blur"),
    (Kind::Crt, "look_crt"),
];

#[derive(Clone, Copy)]
pub struct Surface<'a> {
    pub view: &'a wgpu::TextureView,
    pub size: [u32; 2],
}

struct Job {
    kind: Kind,
    label: &'static str,
    uniform: u64,
    palette: Option<u64>,
    source: wgpu::TextureView,
    aux: Option<wgpu::TextureView>,
    target: wgpu::TextureView,
}

type Pooled = (wgpu::Texture, wgpu::TextureView, [u32; 2]);

pub struct LookGpu {
    layout: wgpu::BindGroupLayout,
    palette_layout: wgpu::BindGroupLayout,
    pipelines: Vec<(Kind, wgpu::RenderPipeline)>,
    sampler: wgpu::Sampler,
    empty: wgpu::TextureView,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    cursor: u64,
    flushed: u64,
    staging: Vec<u8>,
    jobs: Vec<Job>,
    targets: Vec<Option<Pooled>>,
    glow: [Option<Pooled>; 2],
}

pub fn entries(palette: bool) -> Vec<wgpu::BindGroupLayoutEntry> {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let uniform = |binding, size| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    };
    let mut entries = vec![
        uniform(0, SLOT),
        texture(1),
        texture(2),
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    if palette {
        entries.push(uniform(4, PALETTE_BYTES));
    }
    entries
}

fn pooled(device: &wgpu::Device, label: &str, size: [u32; 2]) -> Pooled {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view, size)
}

fn header(target: [u32; 2], place: &Place, source: [u32; 2]) -> [[f32; 4]; 3] {
    [
        [
            target[0] as f32,
            target[1] as f32,
            1.0 / target[0] as f32,
            1.0 / target[1] as f32,
        ],
        [place.scale, place.offset[0], place.offset[1], 0.0],
        [
            place.layout[0],
            place.layout[1],
            source[0] as f32,
            source[1] as f32,
        ],
    ]
}

fn unplaced(size: [u32; 2]) -> Place {
    Place {
        layout: [size[0] as f32, size[1] as f32],
        scale: 1.0,
        offset: [0.0; 2],
    }
}

pub fn glow_factor(sigma: f32) -> u32 {
    let wanted = (sigma / GLOW_REACH).ceil().max(1.0) as u32;
    wanted.next_power_of_two().min(GLOW_FACTOR_MAX)
}

impl LookGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look"),
            entries: &entries(false),
        });
        let palette_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look palette"),
            entries: &entries(true),
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("look"),
            source: wgpu::ShaderSource::Wgsl(super::shader::source().into()),
        });
        let plain = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("look"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let with_palette = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("look palette"),
            bind_group_layouts: &[&palette_layout],
            push_constant_ranges: &[],
        });
        let pipelines = KINDS
            .iter()
            .map(|&(kind, entry)| {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(entry),
                    layout: Some(if kind == Kind::Quantise {
                        &with_palette
                    } else {
                        &plain
                    }),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("look_vertex"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    multiview: None,
                    cache: None,
                });
                (kind, pipeline)
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("look"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let empty = pooled(device, "look empty", [1, 1]);
        queue.write_texture(
            empty.0.as_image_copy(),
            &[0u8; 8],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Self {
            layout,
            palette_layout,
            pipelines,
            sampler,
            empty: empty.1,
            buffer: None,
            capacity: 0,
            cursor: 0,
            flushed: 0,
            staging: Vec::new(),
            jobs: Vec::new(),
            targets: Vec::new(),
            glow: [None, None],
        }
    }

    pub fn pipelines(&self) -> usize {
        self.pipelines.len()
    }

    pub fn begin(&mut self) {
        self.cursor = 0;
        self.flushed = 0;
        self.staging.clear();
        self.jobs.clear();
    }

    pub fn target(
        &mut self,
        device: &wgpu::Device,
        slot: usize,
        size: [u32; 2],
    ) -> wgpu::TextureView {
        if self.targets.len() <= slot {
            self.targets.resize_with(slot + 1, || None);
        }
        let entry = &mut self.targets[slot];
        if entry.as_ref().is_none_or(|pooled| pooled.2 != size) {
            *entry = Some(pooled(device, "look target", size));
        }
        entry.as_ref().unwrap().1.clone()
    }

    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> u64 {
        let at = self.cursor;
        let local = (at - self.flushed) as usize;
        self.staging.resize(local, 0);
        self.staging.extend_from_slice(bytes);
        self.cursor = (at + bytes.len() as u64).div_ceil(ALIGN) * ALIGN;
        at
    }

    fn push_uniform(&mut self, head: [[f32; 4]; 3], params: &[[f32; 4]]) -> u64 {
        let mut slot = [[0.0f32; 4]; (SLOT / 16) as usize];
        slot[..3].copy_from_slice(&head);
        slot[3..3 + params.len()].copy_from_slice(params);
        self.push_bytes(bytemuck::cast_slice(&slot))
    }

    #[allow(clippy::too_many_arguments)]
    fn job(
        &mut self,
        kind: Kind,
        label: &'static str,
        uniform: u64,
        palette: Option<u64>,
        source: &wgpu::TextureView,
        aux: Option<&wgpu::TextureView>,
        target: &wgpu::TextureView,
    ) {
        self.jobs.push(Job {
            kind,
            label,
            uniform,
            palette,
            source: source.clone(),
            aux: aux.cloned(),
            target: target.clone(),
        });
    }

    pub fn backdrop(
        &mut self,
        source: Surface<'_>,
        target: Surface<'_>,
        backdrop: &Backdrop,
        place: &Place,
    ) {
        let mut p = [[0.0f32; 4]; 18];
        if let Some(gradient) = &backdrop.gradient {
            p[..12].copy_from_slice(&gradient.packed());
        }
        p[12] = [
            f32::from(u8::from(backdrop.colour.is_some())),
            f32::from(u8::from(backdrop.grid.is_some())),
            0.0,
            0.0,
        ];
        if let Some(grid) = &backdrop.grid {
            p[13] = [
                grid.spacing[0],
                grid.spacing[1],
                grid.origin[0],
                grid.origin[1],
            ];
            let major = grid.major.filter(|major| major.every > 0);
            p[14] = [
                grid.width,
                major.map_or(0.0, |major| major.every as f32),
                major.map_or(0.0, |major| major.width),
                0.0,
            ];
            p[15] = grid.colour;
            p[16] = major.map_or([0.0; 4], |major| major.colour);
        }
        p[17] = backdrop.colour.unwrap_or([0.0; 4]);
        let uniform = self.push_uniform(header(target.size, place, source.size), &p);
        self.job(
            Kind::Backdrop,
            "look backdrop",
            uniform,
            None,
            source.view,
            None,
            target.view,
        );
    }

    pub fn quantise(&mut self, source: Surface<'_>, target: Surface<'_>, quantise: &Quantise) {
        let (order, spread) = match quantise.dither {
            Dither::None => (0, 0.0),
            Dither::Bayer { order, spread } => (order, spread),
        };
        let p = [[
            quantise.palette.len() as f32,
            order as f32,
            spread,
            order.max(1).trailing_zeros() as f32,
        ]];
        let uniform =
            self.push_uniform(header(target.size, &unplaced(target.size), source.size), &p);
        let palette = self.push_bytes(bytemuck::cast_slice(&quantise.palette.packed()));
        self.job(
            Kind::Quantise,
            "look quantise",
            uniform,
            Some(palette),
            source.view,
            None,
            target.view,
        );
    }

    pub fn upscale(
        &mut self,
        source: Surface<'_>,
        target: Surface<'_>,
        map: &UpscaleMap,
        clear: [f32; 4],
    ) {
        let p = [
            [
                map.offset[0] as f32,
                map.offset[1] as f32,
                map.extent[0] as f32,
                map.extent[1] as f32,
            ],
            clear,
        ];
        let uniform =
            self.push_uniform(header(target.size, &unplaced(target.size), map.source), &p);
        self.job(
            Kind::Upscale,
            "look upscale",
            uniform,
            None,
            source.view,
            None,
            target.view,
        );
    }

    pub fn vignette(
        &mut self,
        source: Surface<'_>,
        target: Surface<'_>,
        vignette: &Vignette,
        place: &Place,
    ) {
        let p = [
            [
                vignette.centre[0],
                vignette.centre[1],
                vignette.radius[0],
                vignette.radius[1],
            ],
            [vignette.falloff, vignette.floor, 0.0, 0.0],
            vignette.colour,
        ];
        let uniform = self.push_uniform(header(target.size, place, source.size), &p);
        self.job(
            Kind::Vignette,
            "look vignette",
            uniform,
            None,
            source.view,
            None,
            target.view,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mix(
        &mut self,
        from: Surface<'_>,
        to: Surface<'_>,
        target: Surface<'_>,
        blend: &Blend,
        progress: f32,
        place: &Place,
    ) {
        let progress = progress.clamp(0.0, 1.0);
        let p = match *blend {
            Blend::Crossfade => [[progress, 0.0, 0.0, 0.0], [0.0; 4]],
            Blend::Wipe { angle, softness } => {
                let (front, along) = wipe_front(angle, softness, progress, place.area(target.size));
                [
                    [progress, 1.0, softness.max(1e-4), 0.0],
                    [along[0], along[1], front, 0.0],
                ]
            }
        };
        let uniform = self.push_uniform(header(target.size, place, from.size), &p);
        self.job(
            Kind::Mix,
            "look mix",
            uniform,
            None,
            from.view,
            Some(to.view),
            target.view,
        );
    }

    pub fn copy(&mut self, source: Surface<'_>, target: Surface<'_>) {
        let uniform = self.push_uniform(
            header(target.size, &unplaced(target.size), source.size),
            &[],
        );
        self.job(
            Kind::Copy,
            "look copy",
            uniform,
            None,
            source.view,
            None,
            target.view,
        );
    }

    pub fn crt(
        &mut self,
        device: &wgpu::Device,
        source: Surface<'_>,
        target: Surface<'_>,
        crt: &Crt,
        place: &Place,
    ) {
        let mut glow_view = None;
        let mut tint = [0.0; 4];
        if let Some(glow) = crt
            .glow
            .filter(|glow| glow.strength > 0.0 && glow.radius > 0.0)
        {
            let sigma = glow.radius * place.scale;
            let factor = glow_factor(sigma);
            let reduced = [
                source.size[0].div_ceil(factor),
                source.size[1].div_ceil(factor),
            ];
            for (index, slot) in self.glow.iter_mut().enumerate() {
                if slot.as_ref().is_none_or(|pooled| pooled.2 != reduced) {
                    *slot = Some(pooled(
                        device,
                        if index == 0 {
                            "look glow"
                        } else {
                            "look glow blur"
                        },
                        reduced,
                    ));
                }
            }
            let a = self.glow[0].as_ref().unwrap().1.clone();
            let b = self.glow[1].as_ref().unwrap().1.clone();
            let bright = self.push_uniform(
                header(reduced, &unplaced(reduced), source.size),
                &[[factor as f32, glow.threshold, 0.0, 0.0]],
            );
            self.job(
                Kind::Bright,
                "look glow",
                bright,
                None,
                source.view,
                None,
                &a,
            );
            let sigma_reduced = sigma / factor as f32;
            let reach = (3.0 * sigma_reduced).ceil().max(1.0);
            for (direction, from, to) in [([1.0, 0.0], &a, &b), ([0.0, 1.0], &b, &a)] {
                let uniform = self.push_uniform(
                    header(reduced, &unplaced(reduced), reduced),
                    &[[direction[0], direction[1], sigma_reduced, reach]],
                );
                self.job(Kind::Blur, "look glow", uniform, None, from, None, to);
            }
            let strength = glow.strength * glow.tint[3];
            tint = [
                glow.tint[0] * strength,
                glow.tint[1] * strength,
                glow.tint[2] * strength,
                1.0,
            ];
            glow_view = Some(a);
        }
        let lines = crt
            .scanlines
            .map_or([0.0; 4], |lines| [lines.period, lines.strength, 1.0, 0.0]);
        let curve = crt.curvature.map_or([0.0, 0.0, 0.0, 0.0], |curve| {
            [curve.amount, curve.corner, 1.0, 0.0]
        });
        let shift = crt
            .aberration
            .map_or(0.0, |aberration| aberration.shift * place.scale);
        let bezel = crt.curvature.map_or([0.0; 4], |curve| curve.bezel);
        let p = [[curve[0], curve[1], curve[2], shift.max(0.0)], bezel, tint];
        let uniform = self.push_uniform(
            header(target.size, place, source.size),
            &[lines, p[0], p[1], p[2]],
        );
        self.job(
            Kind::Crt,
            "look crt",
            uniform,
            None,
            source.view,
            glow_view.as_ref(),
            target.view,
        );
    }

    pub fn flush(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        if self.jobs.is_empty() {
            return;
        }
        let end = self.cursor.max(self.flushed + self.staging.len() as u64);
        if self.buffer.is_none() || end > self.capacity {
            self.capacity = end.next_power_of_two().max(64 * 1024);
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("look uniforms"),
                size: self.capacity,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.buffer.clone().unwrap();
        if !self.staging.is_empty() {
            let padded = self.staging.len().div_ceil(4) * 4;
            self.staging.resize(padded, 0);
            queue.write_buffer(&buffer, self.flushed, &self.staging);
        }
        self.staging.clear();
        self.flushed = self.cursor;
        for job in std::mem::take(&mut self.jobs) {
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(SLOT),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&job.source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        job.aux.as_ref().unwrap_or(&self.empty),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ];
            if job.palette.is_some() {
                entries.push(wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(PALETTE_BYTES),
                    }),
                });
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(job.label),
                layout: if job.palette.is_some() {
                    &self.palette_layout
                } else {
                    &self.layout
                },
                entries: &entries,
            });
            let timing = profiler
                .as_deref_mut()
                .and_then(|profiler| profiler.pass(job.label));
            let writes = profiler
                .as_deref()
                .and_then(|profiler| profiler.render_writes(timing));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(job.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &job.target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: writes,
                occlusion_query_set: None,
            });
            let pipeline = &self
                .pipelines
                .iter()
                .find(|(kind, _)| *kind == job.kind)
                .unwrap()
                .1;
            pass.set_pipeline(pipeline);
            let offsets = [job.uniform as u32, job.palette.unwrap_or(0) as u32];
            let count = if job.palette.is_some() { 2 } else { 1 };
            pass.set_bind_group(0, &group, &offsets[..count]);
            pass.draw(0..3, 0..1);
        }
    }
}
