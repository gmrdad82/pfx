use crate::chain::{Aux, Buffers, Chain, Pass, Uniforms};
use crate::hash::BLUE_NOISE;
use wgpu::{self, util::DeviceExt};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

struct LowPair {
    a: Target,
    b: Target,
}

#[derive(Clone, Copy)]
enum ImageSlot {
    Main,
    LowA,
    LowB,
}

struct Stage {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    source: ImageSlot,
    target: ImageSlot,
    aux: Aux,
    label: String,
}

struct Group {
    pass: Pass,
    stages: Vec<Stage>,
    exposure: Vec<Option<usize>>,
    low: Option<LowPair>,
    factor: u32,
    data: Option<wgpu::Buffer>,
}

pub struct GpuChain {
    device: wgpu::Device,
    groups: Vec<Group>,
    tape: TapePlan,
    scratch: [Target; 2],
    bloom_sampler: Option<wgpu::Sampler>,
    depth_buffer: wgpu::Buffer,
    normal_fallback: Target,
    depth_pipeline: wgpu::ComputePipeline,
    width: u32,
    height: u32,
    viewport: (u32, u32),
}

struct Scissor<'a> {
    rect: [u32; 4],
    after: &'a [u32],
}

impl Scissor<'_> {
    fn grown(&self, group: usize, chain: &GpuChain) -> [u32; 4] {
        let reach = self.after[group];
        [
            self.rect[0].saturating_sub(reach),
            self.rect[1].saturating_sub(reach),
            (self.rect[2] + reach).min(chain.viewport.0),
            (self.rect[3] + reach).min(chain.viewport.1),
        ]
    }
}

pub struct GpuStages {
    pub bloom_grade: GpuChain,
    pub final_stage: GpuChain,
}

impl GpuStages {
    pub fn new(chain: &Chain, device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (bloom_grade, final_stage) = chain.split_after_bloom();
        Self {
            bloom_grade: GpuChain::new(bloom_grade, device, width, height),
            final_stage: GpuChain::new(final_stage, device, width, height),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.bloom_grade.resize(width, height);
        self.final_stage.resize(width, height);
    }
}

fn buffer(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4),
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

fn auxiliary_data(device: &wgpu::Device, pass: &Pass) -> Option<wgpu::Buffer> {
    match pass {
        Pass::Lut(lut) => {
            let mut bytes = Vec::with_capacity(lut.values.len() * 12);
            for rgb in &lut.values {
                for value in rgb {
                    bytes.extend_from_slice(&value.to_ne_bytes());
                }
            }
            if bytes.is_empty() {
                bytes.extend_from_slice(&[0; 4]);
            }
            Some(buffer(device, "post lut", &bytes))
        }
        Pass::Grade(passes) => {
            let bytes: Vec<u8> = crate::chain::grade_data(passes)
                .iter()
                .flat_map(|value| value.to_ne_bytes())
                .collect();
            Some(buffer(device, "post grade", &bytes))
        }
        Pass::OneBit(bit) if bit.kind == crate::fx::NoiseKind::Blue => {
            let mut bytes = Vec::with_capacity(BLUE_NOISE.len() * 4);
            for value in BLUE_NOISE {
                bytes.extend_from_slice(&u32::from(*value).to_ne_bytes());
            }
            Some(buffer(device, "post blue noise", &bytes))
        }
        _ => None,
    }
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn sampled() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: false },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn storage_binding() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn filterable() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn make_stage(
    device: &wgpu::Device,
    shader: &str,
    aux: Aux,
    source: ImageSlot,
    target: ImageSlot,
    label: String,
) -> Stage {
    let mut entries = vec![
        entry(
            0,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
        entry(
            1,
            if matches!(aux, Aux::LinearSampler) {
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                }
            } else {
                sampled()
            },
        ),
        entry(
            2,
            wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
        ),
    ];
    match aux {
        Aux::Depth | Aux::Lut | Aux::Blue => entries.push(entry(3, storage_binding())),
        Aux::Normal => entries.push(entry(4, sampled())),
        Aux::DepthNormal => {
            entries.push(entry(3, storage_binding()));
            entries.push(entry(4, sampled()));
        }
        Aux::Bloom | Aux::BloomB => entries.push(entry(3, sampled())),
        Aux::BloomLinear | Aux::GradeBloomLinear => {
            entries.push(entry(3, filterable()));
            entries.push(entry(
                4,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ));
        }
        Aux::GradeBloom | Aux::GradeBloomB => entries.push(entry(3, sampled())),
        Aux::LinearSampler => entries.push(entry(
            3,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        )),
        Aux::Grade | Aux::None => {}
    }
    if matches!(
        aux,
        Aux::Grade | Aux::GradeBloom | Aux::GradeBloomB | Aux::GradeBloomLinear
    ) {
        entries.push(entry(5, storage_binding()));
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("post bindings"),
        entries: &entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post pipeline layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&label),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(&label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    Stage {
        pipeline,
        layout,
        source,
        target,
        aux,
        label,
    }
}

fn conversion_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let texture = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Depth,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("post geometry conversion"),
        entries: &[
            entry(0, texture),
            entry(
                1,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            ),
            entry(
                2,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            ),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post geometry conversion layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let source = "@group(0) @binding(0) var src: texture_depth_2d; @group(0) @binding(1) var<storage, read_write> dst: array<f32>; @group(0) @binding(2) var<uniform> extent: vec4<u32>; @compute @workgroup_size(8, 8) fn main(@builtin(global_invocation_id) id: vec3<u32>) { let dim = min(textureDimensions(src), extent.xy); if (id.x < dim.x && id.y < dim.y) { dst[id.y * dim.x + id.x] = textureLoad(src, id.xy, 0); } }";
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("post geometry conversion shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("post geometry conversion pipeline"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn plan_passes(passes: Vec<Pass>, fuse: bool) -> Vec<Pass> {
    let active: Vec<Pass> = passes
        .into_iter()
        .filter(|pass| !pass.is_identity() || matches!(pass, Pass::Exposure(_)))
        .collect();
    let active = fuse_cavity_rim(active);
    if fuse { fuse_grades(active) } else { active }
}

fn fuse_cavity_rim(passes: Vec<Pass>) -> Vec<Pass> {
    let mut out: Vec<Pass> = Vec::with_capacity(passes.len());
    for pass in passes {
        if matches!(pass, Pass::Rim(_))
            && matches!(out.last(), Some(Pass::Cavity(_)))
            && let Some(cavity) = out.pop()
        {
            out.push(Pass::FusedCavityRim(vec![cavity, pass]));
        } else {
            out.push(pass);
        }
    }
    out
}

fn joins_grade(run: &[Pass], pass: &Pass) -> bool {
    match pass {
        Pass::Bloom(_) => run.iter().all(|member| matches!(member, Pass::Exposure(_))),
        other => other.is_per_pixel(),
    }
}

fn fuse_grades(passes: Vec<Pass>) -> Vec<Pass> {
    let mut out = Vec::with_capacity(passes.len());
    let mut run: Vec<Pass> = Vec::new();
    for pass in passes {
        if !joins_grade(&run, &pass) {
            close_grade(&mut out, std::mem::take(&mut run));
        }
        if joins_grade(&run, &pass) {
            run.push(pass);
        } else {
            out.push(pass);
        }
    }
    close_grade(&mut out, run);
    out
}

fn close_grade(out: &mut Vec<Pass>, mut run: Vec<Pass>) {
    match run.len() {
        0 => {}
        1 => out.extend(run.pop()),
        _ => out.push(Pass::Grade(run)),
    }
}

fn exposure_slots(pass: &Pass, stages: usize) -> Vec<Option<usize>> {
    let mut slots = vec![None; stages];
    match pass {
        Pass::Exposure(_) => slots[0] = Some(0),
        Pass::Grade(passes) if matches!(passes.first(), Some(Pass::Exposure(_))) => {
            if let Some(last) = slots.last_mut() {
                *last = Some(0);
            }
            match passes
                .iter()
                .find(|member| !matches!(member, Pass::Exposure(_)))
            {
                Some(Pass::Bloom(bloom)) if bloom.kind == crate::bloom::BloomKind::Rings => {
                    slots[0] = Some(3)
                }
                Some(Pass::Bloom(_)) => slots[0] = Some(6),
                _ => {}
            }
        }
        _ => {}
    }
    slots
}

fn stage_name(pass: &Pass, stage: usize, count: usize) -> String {
    let parts: &[&str] = match count {
        3 => &["extract", "rings", "add"],
        4 => &["extract", "blur x", "blur y", "add"],
        _ => &[],
    };
    match parts.get(stage) {
        Some(part) => format!("{} {part}", pass.name()),
        None => pass.name().to_owned(),
    }
}

fn plan_groups(
    passes: Vec<Pass>,
    fuse: bool,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Vec<Group> {
    let active = plan_passes(passes, fuse);
    let passes = if active.is_empty() {
        vec![Pass::Exposure(1.0)]
    } else {
        active
    };
    let mut groups = Vec::with_capacity(passes.len());
    for (index, pass) in passes.into_iter().enumerate() {
        let dispatches = pass.dispatches(Buffers::GEOMETRY);
        let bloom = match &pass {
            Pass::Grade(passes) => passes.iter().find_map(|member| match member {
                Pass::Bloom(bloom) => Some(*bloom),
                _ => None,
            }),
            _ => None,
        };
        let (factor, slots): (u32, Vec<(ImageSlot, ImageSlot)>) = match (&pass, bloom) {
            (Pass::Grade(_), Some(bloom)) if dispatches.len() == 3 => (
                bloom.down.max(1),
                vec![
                    (ImageSlot::Main, ImageSlot::LowA),
                    (ImageSlot::LowA, ImageSlot::LowB),
                    (ImageSlot::Main, ImageSlot::Main),
                ],
            ),
            (Pass::Grade(_), Some(bloom)) => (
                bloom.down.max(1),
                vec![
                    (ImageSlot::Main, ImageSlot::LowA),
                    (ImageSlot::LowA, ImageSlot::LowB),
                    (ImageSlot::LowB, ImageSlot::LowA),
                    (ImageSlot::Main, ImageSlot::Main),
                ],
            ),
            (Pass::Bloom(bloom), _) if bloom.kind == crate::bloom::BloomKind::Rings => (
                bloom.down.max(1),
                vec![
                    (ImageSlot::Main, ImageSlot::LowA),
                    (ImageSlot::LowA, ImageSlot::LowB),
                    (ImageSlot::Main, ImageSlot::Main),
                ],
            ),
            (Pass::Bloom(bloom), _) if dispatches.len() == 4 => (
                bloom.down.max(1),
                vec![
                    (ImageSlot::Main, ImageSlot::LowA),
                    (ImageSlot::LowA, ImageSlot::LowB),
                    (ImageSlot::LowB, ImageSlot::LowA),
                    (ImageSlot::Main, ImageSlot::Main),
                ],
            ),
            (Pass::Halation(_), _) => (
                4,
                vec![
                    (ImageSlot::Main, ImageSlot::LowA),
                    (ImageSlot::LowA, ImageSlot::LowB),
                    (ImageSlot::LowB, ImageSlot::LowA),
                    (ImageSlot::Main, ImageSlot::Main),
                ],
            ),
            (Pass::Neon(_), _) if dispatches.len() == 4 => (
                2,
                vec![
                    (ImageSlot::Main, ImageSlot::LowA),
                    (ImageSlot::LowA, ImageSlot::LowB),
                    (ImageSlot::LowB, ImageSlot::LowA),
                    (ImageSlot::Main, ImageSlot::Main),
                ],
            ),
            _ => (1, vec![(ImageSlot::Main, ImageSlot::Main)]),
        };
        let stages = dispatches
            .iter()
            .zip(slots)
            .enumerate()
            .map(|(stage_index, (dispatch, (source, target)))| {
                let label = format!(
                    "post {index}:{stage_index} {}",
                    stage_name(&pass, stage_index, dispatches.len())
                );
                make_stage(
                    device,
                    &dispatch.shader,
                    dispatch.aux,
                    source,
                    target,
                    label,
                )
            })
            .collect();
        let low = if (bloom.is_some()
            || matches!(pass, Pass::Bloom(_) | Pass::Halation(_) | Pass::Neon(_)))
            && dispatches.len() > 1
        {
            let w = (width / factor).max(1);
            let h = (height / factor).max(1);
            Some(LowPair {
                a: Target::new(device, w, h),
                b: Target::new(device, w, h),
            })
        } else {
            None
        };
        let data = auxiliary_data(device, &pass);
        let exposure = exposure_slots(&pass, dispatches.len());
        groups.push(Group {
            pass,
            stages,
            exposure,
            low,
            factor,
            data,
        });
    }
    groups
}

struct TapePlan {
    source: Vec<Pass>,
    fuse: bool,
    groups: Vec<Group>,
    tape: crate::tape::Tape,
    encodes: bool,
}

impl TapePlan {
    fn plan(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let mut chain = Chain {
            passes: self.source.clone(),
            seed: 0,
            frame: 0,
        };
        chain.set_tape(self.tape);
        let mut groups = plan_groups(chain.passes, self.fuse, device, width, height);
        self.encodes = groups.len() >= 2
            && matches!(groups[groups.len() - 2].pass, Pass::Tape(_))
            && matches!(groups[groups.len() - 1].pass, Pass::Encode);
        if self.encodes {
            groups.pop();
        }
        self.groups = groups;
    }

    fn set(&mut self, tape: crate::tape::Tape) {
        self.tape = tape;
        for group in &mut self.groups {
            if matches!(group.pass, Pass::Tape(_)) {
                group.pass = Pass::Tape(tape);
            }
        }
    }
}

pub(crate) fn fit_viewport(
    viewport: (u32, u32),
    capacity: (u32, u32),
) -> Result<(u32, u32), String> {
    let (width, height) = viewport;
    if width == 0 || height == 0 {
        return Err(format!("a viewport of {width}x{height} is empty"));
    }
    if width > capacity.0 || height > capacity.1 {
        return Err(format!(
            "a viewport of {width}x{height} does not fit the {}x{} allocation",
            capacity.0, capacity.1
        ));
    }
    Ok(viewport)
}

impl GpuChain {
    pub fn new(chain: Chain, device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::build(chain, device, width, height, true)
    }

    #[cfg(test)]
    fn unfused(chain: Chain, device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::build(chain, device, width, height, false)
    }

    fn build(chain: Chain, device: &wgpu::Device, width: u32, height: u32, fuse: bool) -> Self {
        assert!(width > 0 && height > 0);
        let taped = chain.tape();
        let plain = chain
            .passes
            .iter()
            .filter(|pass| !matches!(pass, Pass::Tape(_)))
            .cloned()
            .collect();
        let groups = plan_groups(plain, fuse, device, width, height);
        let mut tape = TapePlan {
            source: chain.passes,
            fuse,
            groups: Vec::new(),
            tape: crate::tape::Tape::OFF,
            encodes: false,
        };
        if let Some(taped) = taped {
            tape.tape = taped;
            tape.plan(device, width, height);
        }
        let bloom_sampler = groups
            .iter()
            .chain(&tape.groups)
            .flat_map(|group| &group.stages)
            .any(|stage| {
                matches!(
                    stage.aux,
                    Aux::BloomLinear | Aux::LinearSampler | Aux::GradeBloomLinear
                )
            })
            .then(|| {
                device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                })
            });
        let pixels = u64::from(width) * u64::from(height);
        Self {
            device: device.clone(),
            groups,
            tape,
            scratch: [
                Target::new(device, width, height),
                Target::new(device, width, height),
            ],
            bloom_sampler,
            depth_buffer: storage(device, "post depth", pixels * 4),
            normal_fallback: Target::new(device, 1, 1),
            depth_pipeline: conversion_pipeline(device),
            width,
            height,
            viewport: (width, height),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn capacity(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.viewport = fit_viewport((width, height), (self.width, self.height))?;
        Ok(())
    }

    fn low_size(&self, group: &Group) -> (u32, u32) {
        (
            (self.viewport.0 / group.factor).max(1),
            (self.viewport.1 / group.factor).max(1),
        )
    }

    fn slot_size(&self, group: &Group, slot: ImageSlot) -> (u32, u32) {
        match slot {
            ImageSlot::Main => self.viewport,
            ImageSlot::LowA | ImageSlot::LowB => self.low_size(group),
        }
    }

    pub fn tape(&self) -> crate::tape::Tape {
        self.tape.tape
    }

    pub fn set_tape(&mut self, tape: crate::tape::Tape) {
        if self.tape.groups.is_empty() && !tape.is_off() {
            self.tape.tape = tape;
            self.tape.plan(&self.device, self.width, self.height);
            self.bloom_sampler.get_or_insert_with(|| {
                self.device.create_sampler(&wgpu::SamplerDescriptor {
                    mag_filter: wgpu::FilterMode::Linear,
                    min_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                })
            });
        }
        self.tape.set(tape);
    }

    pub fn reset_tape(&mut self) {
        let own = self
            .tape
            .source
            .iter()
            .find_map(|pass| match pass {
                Pass::Tape(tape) => Some(*tape),
                _ => None,
            })
            .unwrap_or(crate::tape::Tape::OFF);
        self.set_tape(own);
    }

    fn active(&self) -> &[Group] {
        if self.tape.tape.is_off() || self.tape.groups.is_empty() {
            &self.groups
        } else {
            &self.tape.groups
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.viewport = (width, height);
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.scratch = [
            Target::new(&self.device, width, height),
            Target::new(&self.device, width, height),
        ];
        let pixels = u64::from(width) * u64::from(height);
        self.depth_buffer = storage(&self.device, "post depth", pixels * 4);
        for group in self.groups.iter_mut().chain(&mut self.tape.groups) {
            if group.low.is_some() {
                let w = (width / group.factor).max(1);
                let h = (height / group.factor).max(1);
                group.low = Some(LowPair {
                    a: Target::new(&self.device, w, h),
                    b: Target::new(&self.device, w, h),
                });
            }
        }
    }

    fn convert(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        mut profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) {
        let pipeline = &self.depth_pipeline;
        let dst = &self.depth_buffer;
        let (width, height) = self.viewport;
        let extent = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("post geometry conversion extent"),
                contents: bytemuck::cast_slice(&[width, height, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let layout = pipeline.get_bind_group_layout(0);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post geometry conversion bindings"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dst.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: extent.as_entire_binding(),
                },
            ],
        });
        let timing = profiler
            .as_deref_mut()
            .and_then(|p| p.pass("post depth conversion"));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("post geometry conversion"),
            timestamp_writes: profiler.as_ref().and_then(|p| p.compute_writes(timing)),
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        normal: Option<&wgpu::TextureView>,
        output: &wgpu::TextureView,
        frame: u32,
        seed: u32,
    ) {
        self.run_timed(encoder, input, depth, normal, output, frame, seed, None);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_timed(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        normal: Option<&wgpu::TextureView>,
        output: &wgpu::TextureView,
        frame: u32,
        seed: u32,
        mut profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) {
        self.run_timed_exposed(
            encoder,
            input,
            depth,
            normal,
            output,
            frame,
            seed,
            1.0,
            profiler.take(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_timed_exposed(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        normal: Option<&wgpu::TextureView>,
        output: &wgpu::TextureView,
        frame: u32,
        seed: u32,
        exposure: f32,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) {
        self.encode(
            encoder, input, depth, normal, output, frame, seed, exposure, None, profiler,
        );
    }

    pub fn region_reach(&self) -> Option<u32> {
        let groups = self.active();
        let mut total = 0u32;
        for group in groups {
            let local = group.stages.len() == 1
                && matches!(group.stages[0].source, ImageSlot::Main)
                && matches!(group.stages[0].target, ImageSlot::Main)
                && !matches!(
                    group.stages[0].aux,
                    Aux::Depth | Aux::DepthNormal | Aux::LinearSampler
                );
            if !local {
                return None;
            }
            total += group.pass.region_reach()?;
        }
        Some(total)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_region(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        normal: Option<&wgpu::TextureView>,
        output: &wgpu::TextureView,
        frame: u32,
        seed: u32,
        exposure: f32,
        rects: &[[u32; 4]],
        mut profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> bool {
        let rects: Vec<[u32; 4]> = rects
            .iter()
            .map(|&[x0, y0, x1, y1]| [x0, y0, x1.min(self.viewport.0), y1.min(self.viewport.1)])
            .filter(|rect| rect[0] < rect[2] && rect[1] < rect[3])
            .collect();
        let Some(total) = self.region_reach() else {
            self.encode(
                encoder, input, depth, normal, output, frame, seed, exposure, None, profiler,
            );
            return false;
        };
        let mut after = Vec::with_capacity(self.active().len());
        let mut left = total;
        for group in self.active() {
            left -= group.pass.region_reach().unwrap_or(0);
            after.push(left);
        }
        for rect in rects {
            let scissor = Scissor {
                rect,
                after: &after,
            };
            self.encode(
                encoder,
                input,
                depth,
                normal,
                output,
                frame,
                seed,
                exposure,
                Some(scissor),
                profiler.as_deref_mut(),
            );
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        normal: Option<&wgpu::TextureView>,
        output: &wgpu::TextureView,
        frame: u32,
        seed: u32,
        exposure: f32,
        scissor: Option<Scissor<'_>>,
        mut profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) {
        let groups = self.active();
        if let Some(view) = depth
            && groups.iter().any(|group| {
                group
                    .stages
                    .iter()
                    .any(|stage| matches!(stage.aux, Aux::Depth | Aux::DepthNormal))
            })
        {
            self.convert(encoder, view, profiler.as_deref_mut());
        }
        let buffers = Buffers {
            depth: depth.is_some(),
            normal: normal.is_some(),
        };
        let mut current = input;
        let mut scratch_index = 0;
        for (group_index, group) in groups.iter().enumerate() {
            let mut params = group.pass.params(buffers);
            if self.tape.encodes && matches!(group.pass, Pass::Tape(_)) {
                params[0][14] = 1.0;
            }
            for (stage_index, (stage, mut params)) in group.stages.iter().zip(params).enumerate() {
                let last_stage = stage_index + 1 == group.stages.len();
                let last_group = group_index + 1 == groups.len();
                let target = match stage.target {
                    ImageSlot::Main if last_stage && last_group => output,
                    ImageSlot::Main => {
                        let view = &self.scratch[scratch_index].view;
                        scratch_index ^= 1;
                        view
                    }
                    ImageSlot::LowA => &group.low.as_ref().unwrap().a.view,
                    ImageSlot::LowB => &group.low.as_ref().unwrap().b.view,
                };
                let source = match stage.source {
                    ImageSlot::Main => current,
                    ImageSlot::LowA => &group.low.as_ref().unwrap().a.view,
                    ImageSlot::LowB => &group.low.as_ref().unwrap().b.view,
                };
                let (w, h) = self.slot_size(group, stage.target);
                if group_index == 0
                    && let Some(slot) = group.exposure[stage_index]
                {
                    params[slot] *= exposure;
                }
                let area = match (&scissor, stage.target) {
                    (Some(scissor), ImageSlot::Main) => Some(scissor.grown(group_index, self)),
                    _ => None,
                };
                let mut uniforms = Uniforms::new(w, h, frame, seed, params)
                    .extent(self.slot_size(group, stage.source), self.low_size(group));
                if let Some(area) = area {
                    uniforms = uniforms.region(area);
                }
                let uniform = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("post uniforms"),
                        contents: &uniforms.bytes(),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let mut entries = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(target),
                    },
                ];
                match stage.aux {
                    Aux::Depth => entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.depth_buffer.as_entire_binding(),
                    }),
                    Aux::Normal => entries.push(wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(
                            normal.unwrap_or(&self.normal_fallback.view),
                        ),
                    }),
                    Aux::DepthNormal => {
                        entries.push(wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.depth_buffer.as_entire_binding(),
                        });
                        entries.push(wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(
                                normal.unwrap_or(&self.normal_fallback.view),
                            ),
                        });
                    }
                    Aux::Bloom | Aux::BloomB => entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            if matches!(stage.aux, Aux::BloomB) {
                                &group.low.as_ref().unwrap().b.view
                            } else {
                                &group.low.as_ref().unwrap().a.view
                            },
                        ),
                    }),
                    Aux::BloomLinear => {
                        entries.push(wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(
                                &group.low.as_ref().unwrap().a.view,
                            ),
                        });
                        entries.push(wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(
                                self.bloom_sampler.as_ref().unwrap(),
                            ),
                        });
                    }
                    Aux::LinearSampler => entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(
                            self.bloom_sampler.as_ref().unwrap(),
                        ),
                    }),
                    Aux::Lut | Aux::Blue => entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: group.data.as_ref().unwrap().as_entire_binding(),
                    }),
                    Aux::GradeBloom | Aux::GradeBloomLinear => entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            &group.low.as_ref().unwrap().a.view,
                        ),
                    }),
                    Aux::GradeBloomB => entries.push(wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            &group.low.as_ref().unwrap().b.view,
                        ),
                    }),
                    Aux::Grade | Aux::None => {}
                }
                if stage.aux == Aux::GradeBloomLinear {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(
                            self.bloom_sampler.as_ref().unwrap(),
                        ),
                    });
                }
                if matches!(
                    stage.aux,
                    Aux::Grade | Aux::GradeBloom | Aux::GradeBloomB | Aux::GradeBloomLinear
                ) {
                    entries.push(wgpu::BindGroupEntry {
                        binding: 5,
                        resource: group.data.as_ref().unwrap().as_entire_binding(),
                    });
                }
                let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&stage.label),
                    layout: &stage.layout,
                    entries: &entries,
                });
                let timing = profiler
                    .as_deref_mut()
                    .and_then(|timer| timer.pass(stage.label.clone()));
                let writes = profiler
                    .as_deref()
                    .and_then(|timer| timer.compute_writes(timing));
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(&stage.label),
                    timestamp_writes: writes,
                });
                pass.set_pipeline(&stage.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                let (groups_x, groups_y) = match area {
                    Some([x0, y0, x1, y1]) => ((x1 - x0).div_ceil(8), (y1 - y0).div_ceil(8)),
                    None => (w.div_ceil(8), h.div_ceil(8)),
                };
                pass.dispatch_workgroups(groups_x, groups_y, 1);
                drop(pass);
                if matches!(stage.target, ImageSlot::Main) {
                    current = target;
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::image::Image;
    use crate::preset::Style;
    use pfx_gpu::{Gpu, OffscreenTarget};

    pub(crate) fn half(value: f32) -> u16 {
        let bits = value.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exponent = ((bits >> 23) & 255) as i32 - 127 + 15;
        let mantissa = bits & 0x7fffff;
        if exponent <= 0 {
            return sign;
        }
        if exponent >= 31 {
            return sign | 0x7c00;
        }
        sign | ((exponent as u16) << 10) | ((mantissa >> 13) as u16)
    }

    fn float(value: u16) -> f32 {
        let sign = if value & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = ((value >> 10) & 31) as i32;
        let mantissa = f32::from(value & 1023) / 1024.0;
        if exponent == 0 {
            sign * mantissa * 2f32.powi(-14)
        } else if exponent == 31 {
            f32::INFINITY
        } else {
            sign * (1.0 + mantissa) * 2f32.powi(exponent - 15)
        }
    }

    fn synthetic(width: u32, height: u32) -> Image {
        Image::from_fn(width, height, |x, y| {
            let shades = [0.0, 0.25, 0.5, 0.75, 1.0, 2.0];
            [
                shades[((x / 8 + y / 16) % 6) as usize],
                shades[((x / 16 + y / 8 + 2) % 6) as usize],
                shades[((x / 8 + 3) % 6) as usize],
                1.0,
            ]
        })
    }

    fn output_target(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post test output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        OffscreenTarget {
            texture,
            view,
            format: FORMAT,
            width,
            height,
        }
    }

    fn plan_names(passes: Vec<Pass>) -> Vec<String> {
        plan_passes(passes, true)
            .iter()
            .map(|pass| match pass {
                Pass::Grade(members) => format!(
                    "grade({})",
                    members.iter().map(Pass::name).collect::<Vec<_>>().join("+")
                ),
                other => other.name().to_owned(),
            })
            .collect()
    }

    fn fused_finish(warmth: f32, vignette: f32) -> Vec<Pass> {
        vec![
            Pass::Exposure(1.3),
            Pass::Bloom(crate::bloom::Bloom::tent(0.25, 0.5, 4, 16.0, 8)),
            Pass::Black(0.02),
            Pass::Tone(crate::tone::Tone::neutral(0.75, false)),
            Pass::Warmth(crate::grade::Warmth::luma_curve(
                warmth,
                [1.0, 1.0],
                [1.02, 0.98],
                0.2,
                0.8,
            )),
            Pass::Vignette(crate::grade::Vignette::smoothstep(vignette, 1.0, 0.2, 0.8)),
            Pass::Grain(crate::grade::Grain::clamped(0.02, 0.5)),
        ]
    }

    #[test]
    fn fused_finish_is_one_grade() {
        for (warmth, vignette, names) in [
            (
                0.5,
                0.3,
                "grade(exposure+bloom+black+tone+warmth+vignette+grain)",
            ),
            (0.0, 0.3, "grade(exposure+bloom+black+tone+vignette+grain)"),
            (0.7, 0.0, "grade(exposure+bloom+black+tone+warmth+grain)"),
            (0.0, 0.0, "grade(exposure+bloom+black+tone+grain)"),
        ] {
            assert_eq!(
                plan_names(fused_finish(warmth, vignette)),
                [names],
                "warmth {warmth} vignette {vignette}"
            );
        }
    }

    #[test]
    fn identity_passes_leave_the_grade() {
        let passes = vec![
            Pass::Exposure(1.0),
            Pass::Tone(crate::tone::Tone::neutral(1.0, false)),
            Pass::Warmth(crate::grade::Warmth::luma_curve(
                0.0, [1.0; 2], [1.0; 2], 0.2, 0.8,
            )),
            Pass::Grain(crate::grade::Grain::clamped(0.02, 0.5)),
            Pass::Warmth(crate::grade::Warmth::linear(0.0)),
            Pass::Vignette(crate::grade::Vignette::smoothstep(0.0, 1.0, 0.2, 0.8)),
        ];
        assert_eq!(plan_names(passes), ["grade(exposure+tone+grain)"]);
    }

    #[test]
    fn a_grade_takes_one_bloom_after_exposure_only() {
        let bloom = Pass::Bloom(crate::bloom::Bloom::neutral(0.2));
        let passes = vec![
            Pass::Exposure(1.0),
            Pass::Exposure(0.9),
            bloom.clone(),
            Pass::Vignette(crate::grade::Vignette::power(0.4, 2.0, 1.0, 1.0)),
            bloom.clone(),
            Pass::Encode,
        ];
        assert_eq!(
            plan_names(passes),
            [
                "grade(exposure+exposure+bloom+vignette)",
                "grade(bloom+encode)"
            ]
        );
        let passes = vec![
            Pass::Tone(crate::tone::Tone::aces()),
            bloom.clone(),
            Pass::Kuwahara(2),
            Pass::Encode,
        ];
        assert_eq!(plan_names(passes), ["tone", "bloom", "kuwahara", "encode"]);
        let unfused = plan_passes(vec![Pass::Exposure(0.9), bloom, Pass::Encode], false);
        assert_eq!(
            unfused.iter().map(Pass::name).collect::<Vec<_>>(),
            ["exposure", "bloom", "encode"]
        );
    }

    #[test]
    fn styles_fuse_their_per_pixel_runs() {
        let expected: [(&str, &[&str]); 21] = [
            ("cel", &["exposure", "cel", "fused_cavity_rim", "outline"]),
            ("bw", &["bw"]),
            ("noir", &["grade(noir+grain+vignette)"]),
            ("vignette", &["vignette"]),
            ("neon", &["neon", "bloom"]),
            ("rubber_hose", &["rubber_hose"]),
            ("sepia", &["sepia"]),
            (
                "film",
                &[
                    "distortion",
                    "aberration",
                    "halation",
                    "grade(tone+sepia+grain+vignette)",
                ],
            ),
            (
                "crt",
                &[
                    "curvature",
                    "aberration",
                    "grade(bloom+scanlines+aperture+vignette)",
                ],
            ),
            ("one_bit", &["one_bit"]),
            ("halftone", &["halftone"]),
            ("comic", &["comic"]),
            ("modern_comic", &["modern_comic"]),
            ("watercolor", &["watercolor"]),
            ("paper_grain", &["paper_grain"]),
            ("pixel", &["pixel"]),
            ("duotone", &["duotone"]),
            ("gradient_map", &["gradient_map"]),
            ("posterize", &["posterize"]),
            ("kuwahara", &["kuwahara"]),
            ("tilt_shift", &["tilt_shift"]),
        ];
        for (style, (name, names)) in Style::ALL.into_iter().zip(expected) {
            assert_eq!(style.name(), name);
            assert_eq!(plan_names(style.chain().passes), names, "{name}");
        }
        assert_eq!(
            plan_names(as_live(frame_finish()).passes),
            ["grade(exposure+exposure+bloom+vignette+saturation+tone+encode)"]
        );
    }

    #[test]
    fn frame_finish_is_four_dispatches_with_its_exposure_in_two() {
        let plan = plan_passes(as_live(frame_finish()).passes, true);
        let [Pass::Grade(members)] = plan.as_slice() else {
            panic!("{plan:?}");
        };
        let dispatches = plan[0].dispatches(Buffers::GEOMETRY);
        assert_eq!(dispatches.len(), 4);
        assert_eq!(dispatches[0].shader, crate::wgsl::BLOOM_DOWN_BOX2);
        assert_eq!(
            dispatches[1].shader.as_ref(),
            crate::wgsl::bloom_blur_linear(4, 16.0)
        );
        assert_eq!(dispatches[3].aux, Aux::GradeBloomLinear);
        assert!((dispatches[0].params[6] - 1.25).abs() < 1e-6);
        assert_eq!(dispatches[3].params[0], 1.0);
        assert!((dispatches[3].params[1] - 0.2).abs() < 1e-6);
        assert_eq!(
            exposure_slots(&plan[0], dispatches.len()),
            [Some(6), None, None, Some(0)]
        );
        assert_eq!(crate::chain::grade_data(members).len(), 6 * 16);
        let params = plan[0].params(Buffers::GEOMETRY);
        assert_eq!(params.len(), 4);
        for (params, dispatch) in params.iter().zip(&dispatches) {
            assert_eq!(params, &dispatch.params);
        }
    }

    #[test]
    fn every_planned_shader_validates_without_a_gpu() {
        let mut chains: Vec<Chain> = finishes().into_iter().map(|(_, chain)| chain).collect();
        chains.extend(Style::ALL.into_iter().map(|style| style.chain()));
        chains.push(Chain {
            passes: fused_finish(0.4, 0.3),
            seed: 0,
            frame: 0,
        });
        let mut rings = Chain::new();
        rings.passes.push(Pass::Bloom(crate::bloom::Bloom::rings(
            0.5,
            1.0,
            4,
            crate::bloom::NEUTRAL_RADII,
            crate::bloom::NEUTRAL_GAINS,
        )));
        chains.push(rings);
        let mut seen = std::collections::BTreeSet::new();
        for chain in chains {
            for unfused in [false, true] {
                for pass in plan_passes(chain.passes.clone(), !unfused) {
                    for dispatch in pass.dispatches(Buffers::GEOMETRY) {
                        if !seen.insert(dispatch.shader.to_string()) {
                            continue;
                        }
                        let module = naga::front::wgsl::parse_str(&dispatch.shader)
                            .unwrap_or_else(|error| panic!("{}: {error}", pass.name()));
                        naga::valid::Validator::new(
                            naga::valid::ValidationFlags::all(),
                            naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32,
                        )
                        .validate(&module)
                        .unwrap_or_else(|error| panic!("{}: {error:?}", pass.name()));
                    }
                }
            }
        }
        assert!(seen.len() > 40, "{}", seen.len());
    }

    fn encode(value: f32) -> i32 {
        (value.clamp(0.0, 1.0) * 255.0).round() as i32
    }

    fn original_point_bloom(post: &mut GpuChain, device: &wgpu::Device) {
        let stage = &mut post.groups[0].stages[0];
        *stage = make_stage(
            device,
            crate::wgsl::BLOOM_DOWN,
            Aux::None,
            stage.source,
            stage.target,
            stage.label.clone(),
        );
        for stage in &mut post.groups[0].stages[1..3] {
            *stage = make_stage(
                device,
                crate::wgsl::BLOOM_BLUR,
                Aux::None,
                stage.source,
                stage.target,
                stage.label.clone(),
            );
        }
        let stage = &mut post.groups[0].stages[3];
        *stage = make_stage(
            device,
            crate::wgsl::BLOOM_ADD,
            Aux::Bloom,
            stage.source,
            stage.target,
            stage.label.clone(),
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn styles_match_cpu_on_gpu() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = synthetic(64, 64);
        let input = gpu.offscreen(64, 64, FORMAT).unwrap();
        let output = output_target(&gpu, 64, 64);
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        for style in Style::ALL {
            let mut chain = style.chain();
            chain.frame = 7;
            chain.seed = 23;
            chain.passes.push(Pass::Encode);
            let expected = chain.apply(&source, None, None);
            let post = GpuChain::new(chain, &gpu.device, 64, 64);
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("post test"),
                });
            post.run(&mut encoder, &input.view, None, None, &output.view, 7, 23);
            gpu.queue.submit(Some(encoder.finish()));
            let actual = gpu.readback_rgba16(&output).unwrap();
            for (pixel, (got, want)) in actual.chunks_exact(4).zip(&expected.pixels).enumerate() {
                for channel in 0..3 {
                    let delta = (encode(float(got[channel])) - encode(want[channel])).abs();
                    assert!(
                        delta <= 1,
                        "{} pixel {pixel} channel {channel}: {} vs {}",
                        style.name(),
                        float(got[channel]),
                        want[channel]
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn fused_grade_matches_unfused_at_any_warmth() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = synthetic(64, 64);
        let input = gpu.offscreen(64, 64, FORMAT).unwrap();
        let output = output_target(&gpu, 64, 64);
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        for warmth in [0.0, 0.5, 1.0] {
            let mut chain = Chain {
                passes: fused_finish(warmth, 0.3),
                seed: 23,
                frame: 7,
            };
            chain.passes.push(Pass::Encode);
            let expected = chain.apply(&source, None, None);
            let post = GpuChain::new(chain, &gpu.device, 64, 64);
            assert!(
                post.groups
                    .iter()
                    .any(|group| matches!(group.pass, Pass::Grade(_))),
                "warmth {warmth} did not fuse"
            );
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("post fused test"),
                });
            post.run(&mut encoder, &input.view, None, None, &output.view, 7, 23);
            gpu.queue.submit(Some(encoder.finish()));
            let actual = gpu.readback_rgba16(&output).unwrap();
            for (pixel, (got, want)) in actual.chunks_exact(4).zip(&expected.pixels).enumerate() {
                for channel in 0..3 {
                    let delta = (encode(float(got[channel])) - encode(want[channel])).abs();
                    assert!(
                        delta <= 1,
                        "warmth {warmth} pixel {pixel} channel {channel}: {} vs {}",
                        float(got[channel]),
                        want[channel]
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn staged_gpu_matches_whole_chain_without_overlay() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = synthetic(64, 64);
        let input = gpu.offscreen(64, 64, FORMAT).unwrap();
        let middle = output_target(&gpu, 64, 64);
        let whole_output = output_target(&gpu, 64, 64);
        let staged_output = output_target(&gpu, 64, 64);
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        let mut chain = Chain::new();
        chain.passes = vec![
            Pass::Exposure(1.0),
            Pass::Bloom(crate::bloom::Bloom::neutral(0.45)),
            Pass::Tone(crate::tone::Tone::aces()),
        ];
        let whole = GpuChain::new(chain.clone(), &gpu.device, 64, 64);
        let stages = GpuStages::new(&chain, &gpu.device, 64, 64);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post staged test"),
            });
        whole.run(
            &mut encoder,
            &input.view,
            None,
            None,
            &whole_output.view,
            0,
            0,
        );
        stages
            .bloom_grade
            .run(&mut encoder, &input.view, None, None, &middle.view, 0, 0);
        stages.final_stage.run(
            &mut encoder,
            &middle.view,
            None,
            None,
            &staged_output.view,
            0,
            0,
        );
        gpu.queue.submit(Some(encoder.finish()));
        let whole_pixels = gpu.readback_rgba16(&whole_output).unwrap();
        let staged_pixels = gpu.readback_rgba16(&staged_output).unwrap();
        for (a, b) in whole_pixels.iter().zip(staged_pixels.iter()) {
            assert!((encode(float(*a)) - encode(float(*b))).abs() <= 1);
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn box_bloom_matches_the_point_gaussian() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (256, 128);
        let source = Image::from_fn(width, height, |x, y| {
            let a = (x as f32 - 61.0).hypot(y as f32 - 37.0);
            let b = (x as f32 - 194.0).hypot(y as f32 - 82.0);
            let light = (2.0 - a * 0.08).max(0.0) + (1.6 - b * 0.06).max(0.0);
            [light, light * 0.7, light * 0.4, 1.0]
        });
        let input = gpu.offscreen(width, height, FORMAT).unwrap();
        let original_output = output_target(&gpu, width, height);
        let optimized_output = output_target(&gpu, width, height);
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        let mut chain = Chain::new();
        chain.passes.push(Pass::Bloom(crate::bloom::Bloom::pyramid(
            0.45, 1.0, 0.0, 0.0, 0.0, 4, 16.0, 2,
        )));
        let expected = chain.apply(&source, None, None);
        let mut original = GpuChain::new(chain.clone(), &gpu.device, width, height);
        original_point_bloom(&mut original, &gpu.device);
        let optimized = GpuChain::new(chain, &gpu.device, width, height);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bloom comparison"),
            });
        original.run(
            &mut encoder,
            &input.view,
            None,
            None,
            &original_output.view,
            0,
            0,
        );
        optimized.run(
            &mut encoder,
            &input.view,
            None,
            None,
            &optimized_output.view,
            0,
            0,
        );
        gpu.queue.submit(Some(encoder.finish()));
        let old = gpu.readback_rgba16(&original_output).unwrap();
        let new = gpu.readback_rgba16(&optimized_output).unwrap();
        for (actual, want) in new.chunks_exact(4).zip(&expected.pixels) {
            for channel in 0..3 {
                assert!((encode(float(actual[channel])) - encode(want[channel])).abs() <= 2);
            }
        }
        let mut differences: Vec<i32> = old
            .chunks_exact(4)
            .zip(new.chunks_exact(4))
            .flat_map(|(a, b)| {
                (0..3).map(move |channel| {
                    (encode(float(a[channel])) - encode(float(b[channel]))).abs()
                })
            })
            .collect();
        differences.sort_unstable();
        let median = differences[differences.len() / 2];
        let maximum = differences.last().copied().unwrap();
        assert_eq!(median, 0);
        assert!(maximum <= 2);
        let mut either = 0;
        let mut both = 0;
        for ((base, a), b) in pixels
            .chunks_exact(4)
            .zip(old.chunks_exact(4))
            .zip(new.chunks_exact(4))
        {
            let old_bright = float(a[0]) - float(base[0]) > 1.0 / 255.0;
            let new_bright = float(b[0]) - float(base[0]) > 1.0 / 255.0;
            if old_bright || new_bright {
                either += 1;
                if old_bright && new_bright {
                    both += 1;
                }
            }
        }
        assert!(either > 0);
        let overlap = both as f32 / either as f32;
        println!("box bloom median {median}/255 max {maximum}/255 footprint {overlap:.3}");
        assert!(overlap >= 0.985);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn geometry_views_match_cpu() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = synthetic(64, 64);
        let input = gpu.offscreen(64, 64, FORMAT).unwrap();
        let output = output_target(&gpu, 64, 64);
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        let depth_values = vec![0.5; 4096];
        let normals: Vec<[f32; 3]> = (0..4096)
            .map(|i| {
                if i % 64 < 32 {
                    [0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.5, 0.5]
                }
            })
            .collect();
        let depth_texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post test depth"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut depth_encoder =
            gpu.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("post depth clear"),
                });
        {
            let _pass = depth_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post depth clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.5),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        gpu.queue.submit(Some(depth_encoder.finish()));
        let normal_target = gpu.offscreen(64, 64, FORMAT).unwrap();
        let normal_pixels: Vec<u16> = normals
            .iter()
            .flat_map(|n| [half(n[0]), half(n[1]), half(n[2]), half(1.0)])
            .collect();
        gpu.upload_rgba16(&normal_target, &normal_pixels).unwrap();
        for style in [
            Style::Cel,
            Style::RubberHose,
            Style::Comic,
            Style::ModernComic,
            Style::TiltShift,
        ] {
            let mut chain = style.chain();
            chain.passes.push(Pass::Encode);
            let expected = chain.apply(&source, Some(&depth_values), Some(&normals));
            let post = GpuChain::new(chain, &gpu.device, 64, 64);
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("post geometry test"),
                });
            post.run(
                &mut encoder,
                &input.view,
                Some(&depth_view),
                Some(&normal_target.view),
                &output.view,
                0,
                0,
            );
            gpu.queue.submit(Some(encoder.finish()));
            let actual = gpu.readback_rgba16(&output).unwrap();
            for (pixel, (got, want)) in actual.chunks_exact(4).zip(&expected.pixels).enumerate() {
                for channel in 0..3 {
                    let delta = (encode(float(got[channel])) - encode(want[channel])).abs();
                    assert!(
                        delta <= 1,
                        "{} pixel {pixel} channel {channel}: {} vs {}",
                        style.name(),
                        float(got[channel]),
                        want[channel]
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn pipeline_creation_and_resize() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        for style in Style::ALL {
            let mut post = GpuChain::new(style.chain(), &gpu.device, 64, 64);
            assert_eq!(post.size(), (64, 64));
            post.resize(113, 57);
            assert_eq!(post.size(), (113, 57));
            for group in &post.groups {
                if let Some(low) = &group.low {
                    assert_eq!(
                        (low.a._texture.width(), low.a._texture.height()),
                        ((113 / group.factor).max(1), (57 / group.factor).max(1))
                    );
                    assert_eq!(low.b._texture.size(), low.a._texture.size());
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_4k_styles() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let input = gpu.offscreen(3840, 2160, FORMAT).unwrap();
        let output = output_target(&gpu, 3840, 2160);
        let normal = gpu.offscreen(3840, 2160, FORMAT).unwrap();
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post benchmark depth"),
            size: wgpu::Extent3d {
                width: 3840,
                height: 2160,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let mut clear = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post benchmark geometry clear"),
            });
        {
            let _pass = clear.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post benchmark geometry"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &normal.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.5),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        gpu.queue.submit(Some(clear.finish()));
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut over_budget = Vec::new();
        let mut turns = pfx_gpu::pace::Turns::default();
        for style in Style::ALL {
            let post = turns.cpu(|| GpuChain::new(style.chain(), &gpu.device, 3840, 2160));
            let mut samples = Vec::new();
            for attempt in 0..12 {
                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("post benchmark"),
                        });
                let geometry = matches!(
                    style,
                    Style::Cel | Style::RubberHose | Style::Comic | Style::ModernComic
                );
                post.run_timed(
                    &mut encoder,
                    &input.view,
                    geometry.then_some(&depth_view),
                    geometry.then_some(&normal.view),
                    &output.view,
                    7,
                    23,
                    Some(&mut timer),
                );
                let slot = timer.finish(&mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                if let Some(slot) = slot {
                    timer.submitted(slot);
                }
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let timings = timer.collect(&gpu.device);
                if attempt == 11
                    && matches!(
                        style,
                        Style::Cel | Style::RubberHose | Style::Comic | Style::ModernComic
                    )
                {
                    for pass in timings.iter().flatten() {
                        println!(
                            "{} {} {:.3} ms",
                            style.name(),
                            pass.label,
                            pass.milliseconds
                        );
                    }
                }
                let total: f64 = timings.iter().flatten().map(|pass| pass.milliseconds).sum();
                turns.add(total);
                if attempt > 0 {
                    samples.push(total);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!("{} {:.3} ms", style.name(), samples[5]);
            if matches!(
                style,
                Style::Cel | Style::RubberHose | Style::Comic | Style::ModernComic
            ) && samples[5] >= 2.0
            {
                over_budget.push(format!("{} {:.3} ms", style.name(), samples[5]));
            }
        }
        assert!(over_budget.is_empty(), "{}", over_budget.join(", "));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn normals_reach_rim_chain() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let input = gpu.offscreen(64, 64, FORMAT).unwrap();
        let normal = gpu.offscreen(64, 64, FORMAT).unwrap();
        let output = output_target(&gpu, 64, 64);
        gpu.upload_rgba16(&input, &vec![half(0.2); 64 * 64 * 4])
            .unwrap();
        let normals: Vec<u16> = (0..64 * 64)
            .flat_map(|i| {
                let angled = i % 64 >= 16 && i % 64 < 48;
                [
                    half(if angled { 0.98 } else { 0.0 }),
                    half(0.0),
                    half(if angled { 0.2 } else { 1.0 }),
                    half(1.0),
                ]
            })
            .collect();
        gpu.upload_rgba16(&normal, &normals).unwrap();
        let chain = GpuChain::new(
            Chain {
                passes: vec![Pass::Rim(crate::fx::Rim {
                    strength: 0.8,
                    width: 0.3,
                    color: [0.3, 0.6, 1.0],
                })],
                frame: 0,
                seed: 0,
            },
            &gpu.device,
            64,
            64,
        );
        let count = |normal_view: Option<&wgpu::TextureView>| {
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rim normal test"),
                });
            chain.run(
                &mut encoder,
                &input.view,
                None,
                normal_view,
                &output.view,
                0,
                0,
            );
            gpu.queue.submit(Some(encoder.finish()));
            gpu.readback_rgba16(&output)
                .unwrap()
                .chunks_exact(4)
                .filter(|pixel| float(pixel[2]) > 0.25)
                .count()
        };
        let without = count(None);
        let with = count(Some(&normal.view));
        assert!(with > without + 1000, "{without} vs {with}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_bloom_sizes() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        for (width, height) in [(1586, 992), (3840, 2160)] {
            let input = gpu.offscreen(width, height, FORMAT).unwrap();
            let output = output_target(&gpu, width, height);
            for (name, bloom) in [
                ("box", crate::bloom::Bloom::neutral(0.45)),
                ("tent", crate::bloom::Bloom::tent(0.2, 1.0, 4, 16.0, 2)),
                (
                    "rings",
                    crate::bloom::Bloom::rings(
                        0.5,
                        1.0,
                        4,
                        crate::bloom::NEUTRAL_RADII,
                        crate::bloom::NEUTRAL_GAINS,
                    ),
                ),
            ] {
                let mut chain = Chain::new();
                chain.passes.push(Pass::Bloom(bloom));
                for version in ["before", "after"] {
                    let mut post =
                        turns.cpu(|| GpuChain::new(chain.clone(), &gpu.device, width, height));
                    if version == "before" && name == "box" {
                        original_point_bloom(&mut post, &gpu.device);
                    }
                    let mut samples = Vec::new();
                    for attempt in 0..15 {
                        let mut encoder =
                            gpu.device
                                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("bloom benchmark"),
                                });
                        post.run_timed(
                            &mut encoder,
                            &input.view,
                            None,
                            None,
                            &output.view,
                            0,
                            0,
                            Some(&mut timer),
                        );
                        let slot = timer.finish(&mut encoder);
                        gpu.queue.submit(Some(encoder.finish()));
                        if let Some(slot) = slot {
                            timer.submitted(slot);
                        }
                        gpu.device
                            .poll(wgpu::PollType::wait_indefinitely())
                            .unwrap();
                        let timings = timer.collect(&gpu.device);
                        let total: f64 =
                            timings.iter().flatten().map(|pass| pass.milliseconds).sum();
                        turns.add(total);
                        if attempt > 0 {
                            samples.push(total);
                        }
                    }
                    samples.sort_by(f64::total_cmp);
                    println!("{name} {width}x{height} {version} {:.3} ms", samples[7]);
                }
            }
        }
    }

    fn frame_finish() -> Chain {
        Chain {
            passes: vec![
                Pass::Exposure(1.25),
                Pass::Bloom(crate::bloom::Bloom::neutral(0.2)),
                Pass::Vignette(crate::grade::Vignette::power(0.4, 2.0, 1.0, 1.0)),
                Pass::Saturation(crate::grade::Saturation::rec709(1.1)),
                Pass::Warmth(crate::grade::Warmth::linear(0.0)),
                Pass::Tone(crate::tone::Tone::aces()),
            ],
            seed: 0,
            frame: 0,
        }
    }

    fn as_live(mut chain: Chain) -> Chain {
        chain.passes.insert(0, Pass::Exposure(1.0));
        chain.passes.push(Pass::Encode);
        chain
    }

    fn finishes() -> Vec<(&'static str, Chain)> {
        let mut out = vec![("frame finish", as_live(frame_finish()))];
        out.extend(
            Style::ALL
                .into_iter()
                .map(|style| (style.name(), as_live(style.chain()))),
        );
        out
    }

    fn uses_geometry(chain: &Chain) -> bool {
        chain.passes.iter().any(|pass| {
            matches!(
                pass,
                Pass::Cel(_)
                    | Pass::Outline(_)
                    | Pass::Cavity(_)
                    | Pass::Rim(_)
                    | Pass::Comic(_)
                    | Pass::RubberHose
                    | Pass::ModernComic(_)
                    | Pass::TiltShift(_)
            )
        })
    }

    fn captured_frame(width: u32, height: u32) -> Option<Image> {
        let path = std::env::var_os("POST_FRAME_PNG")?;
        let bytes = std::fs::read(path).ok()?;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        if (info.width, info.height) != (width, height) || info.color_type != png::ColorType::Rgba {
            return None;
        }
        Some(Image::from_fn(width, height, |x, y| {
            let at = ((y * width + x) * 4) as usize;
            let channel = |c: usize| pfx_materials::linear_channel(f32::from(buf[at + c]) / 255.0);
            [channel(0), channel(1), channel(2), 1.0]
        }))
    }

    fn lit_scene(width: u32, height: u32) -> Image {
        let lights = [(0.18, 0.22, 6.0), (0.71, 0.35, 3.5), (0.44, 0.8, 9.0)];
        Image::from_fn(width, height, |x, y| {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let mut c = [0.05 + 0.4 * u, 0.08 + 0.3 * v, 0.12 + 0.2 * (u * v), 1.0];
            for (lx, ly, power) in lights {
                let d = ((u - lx) * 16.0).hypot((v - ly) * 9.0);
                let glow = power / (1.0 + d * d * 40.0);
                c[0] += glow;
                c[1] += glow * 0.8;
                c[2] += glow * 0.55;
            }
            let stripe = ((x / 37 + y / 23) % 5) as f32 * 0.04;
            [c[0] + stripe, c[1] + stripe, c[2], 1.0]
        })
    }

    fn source_4k(gpu: &Gpu) -> (OffscreenTarget, &'static str) {
        let (width, height) = (3840, 2160);
        let (image, name) = match captured_frame(width, height) {
            Some(image) => (image, "a captured frame"),
            None => (lit_scene(width, height), "a lit synthetic scene"),
        };
        let input = gpu.offscreen(width, height, FORMAT).unwrap();
        let pixels: Vec<u16> = image
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        (input, name)
    }

    struct Geometry {
        _depth: wgpu::Texture,
        depth: wgpu::TextureView,
        normal: OffscreenTarget,
    }

    fn geometry_4k(gpu: &Gpu) -> Geometry {
        let normal = gpu.offscreen(3840, 2160, FORMAT).unwrap();
        let depth = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post finish depth"),
            size: wgpu::Extent3d {
                width: 3840,
                height: 2160,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = depth.create_view(&Default::default());
        let mut clear = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post finish geometry clear"),
            });
        {
            let _pass = clear.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post finish geometry"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &normal.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.5),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        gpu.queue.submit(Some(clear.finish()));
        Geometry {
            _depth: depth,
            depth: view,
            normal,
        }
    }

    fn median(values: &mut [f64]) -> f64 {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn fused_matches_unfused_at_4k() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut turns = pfx_gpu::pace::Turns::default();
        let (input, source) = turns.cpu(|| source_4k(&gpu));
        let geometry = turns.cpu(|| geometry_4k(&gpu));
        let fused_output = output_target(&gpu, 3840, 2160);
        let reference_output = output_target(&gpu, 3840, 2160);
        let levels: Vec<i16> = turns.cpu(|| {
            (0..=u16::MAX)
                .map(|bits| encode(float(bits)) as i16)
                .collect()
        });
        let mut failures = Vec::new();
        println!("3840x2160 from {source}");
        for (name, chain) in finishes() {
            let geometric = uses_geometry(&chain);
            let fused = turns.cpu(|| GpuChain::new(chain.clone(), &gpu.device, 3840, 2160));
            let reference = turns.cpu(|| GpuChain::unfused(chain, &gpu.device, 3840, 2160));
            let mut worst = 0;
            let mut differing = 0usize;
            let exposures: &[f32] = if name == "frame finish" {
                &[1.0, 0.8]
            } else {
                &[0.8]
            };
            for &exposure in exposures {
                for (post, output) in [(&fused, &fused_output), (&reference, &reference_output)] {
                    let mut encoder =
                        gpu.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("post fused comparison"),
                            });
                    post.run_timed_exposed(
                        &mut encoder,
                        &input.view,
                        geometric.then_some(&geometry.depth),
                        geometric.then_some(&geometry.normal.view),
                        &output.view,
                        7,
                        23,
                        exposure,
                        None,
                    );
                    turns.time(|| {
                        gpu.queue.submit(Some(encoder.finish()));
                        gpu.device
                            .poll(wgpu::PollType::wait_indefinitely())
                            .unwrap();
                    });
                }
                let got = turns.cpu(|| gpu.readback_rgba16(&fused_output).unwrap());
                let want = turns.cpu(|| gpu.readback_rgba16(&reference_output).unwrap());
                for (pixel, (a, b)) in got.chunks_exact(4).zip(want.chunks_exact(4)).enumerate() {
                    if pixel % 262_144 == 0 {
                        turns.poll();
                    }
                    if a == b {
                        continue;
                    }
                    for channel in 0..3 {
                        let delta = (levels[usize::from(a[channel])]
                            - levels[usize::from(b[channel])])
                        .abs();
                        if delta > 0 {
                            differing += 1;
                            worst = worst.max(delta);
                        }
                    }
                }
            }
            println!(
                "{name}: {} passes fused into {}, worst {worst}/255, {differing} channels differ",
                reference
                    .groups
                    .iter()
                    .map(|group| group.stages.len())
                    .sum::<usize>(),
                fused
                    .groups
                    .iter()
                    .map(|group| group.stages.len())
                    .sum::<usize>(),
            );
            if worst > 1 {
                failures.push(format!("{name} {worst}/255"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join(", "));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_4k_finish_breakdown() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut turns = pfx_gpu::pace::Turns::default();
        let (input, source) = turns.cpu(|| source_4k(&gpu));
        let output = output_target(&gpu, 3840, 2160);
        let geometry = turns.cpu(|| geometry_4k(&gpu));
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let frames = std::env::var("POST_BENCH_FRAMES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(300);
        println!("3840x2160 from {source}, medians over {frames} frames");
        let mut frame = f64::INFINITY;
        for (name, chain) in finishes() {
            let geometric = uses_geometry(&chain);
            let post = turns.cpu(|| GpuChain::new(chain, &gpu.device, 3840, 2160));
            let mut totals = Vec::with_capacity(frames);
            let mut passes: Vec<(String, Vec<f64>)> = Vec::new();
            for attempt in 0..frames + 5 {
                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("post finish benchmark"),
                        });
                post.run_timed_exposed(
                    &mut encoder,
                    &input.view,
                    geometric.then_some(&geometry.depth),
                    geometric.then_some(&geometry.normal.view),
                    &output.view,
                    attempt as u32,
                    23,
                    1.0,
                    Some(&mut timer),
                );
                let slot = timer.finish(&mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                if let Some(slot) = slot {
                    timer.submitted(slot);
                }
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let timings = timer.collect(&gpu.device);
                let total: f64 = timings.iter().flatten().map(|pass| pass.milliseconds).sum();
                turns.add(total);
                if attempt < 5 {
                    continue;
                }
                totals.push(total);
                for pass in timings.iter().flatten() {
                    let label = pass
                        .label
                        .strip_prefix("post ")
                        .unwrap_or(&pass.label)
                        .to_owned();
                    match passes.iter_mut().find(|(known, _)| *known == label) {
                        Some((_, values)) => values.push(pass.milliseconds),
                        None => passes.push((label, vec![pass.milliseconds])),
                    }
                }
            }
            let total = median(&mut totals);
            if name == "frame finish" {
                frame = total;
            }
            let breakdown: Vec<String> = passes
                .iter_mut()
                .map(|(label, values)| format!("{label} {:.3}", median(values)))
                .collect();
            println!("{name} {total:.3} ms: {}", breakdown.join(", "));
        }
        println!("turns taken {}", turns.taken());
        assert!(frame.is_finite());
    }

    fn region_chains() -> Vec<(&'static str, Chain)> {
        let finish = |extra: Vec<Pass>| {
            let mut passes = vec![Pass::Exposure(1.1)];
            passes.extend(extra);
            passes.push(Pass::Vignette(crate::grade::Vignette::power(
                0.4, 2.0, 1.0, 1.0,
            )));
            passes.push(Pass::Tone(crate::tone::Tone::aces()));
            passes.push(Pass::Grain(crate::grade::Grain::clamped(0.03, 0.5)));
            passes.push(Pass::Encode);
            Chain {
                passes,
                seed: 31,
                frame: 5,
            }
        };
        vec![
            ("pointwise finish", finish(vec![])),
            ("kuwahara", finish(vec![Pass::Kuwahara(3)])),
            (
                "watercolor",
                finish(vec![Pass::Watercolor(crate::fx::Watercolor {
                    darkening: 0.5,
                    grain: 0.2,
                    scale: 6.0,
                })]),
            ),
            (
                "weave and aberration",
                finish(vec![Pass::Weave(3.0), Pass::Aberration(2.5)]),
            ),
            (
                "two kernels",
                finish(vec![Pass::Kuwahara(2), Pass::Sepia(0.5), Pass::Kuwahara(4)]),
            ),
        ]
    }

    fn upload_source(gpu: &Gpu, source: &Image) -> OffscreenTarget {
        let input = gpu.offscreen(source.width, source.height, FORMAT).unwrap();
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half(*v)))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        input
    }

    fn post_into(
        gpu: &Gpu,
        post: &GpuChain,
        input: &OffscreenTarget,
        output: &OffscreenTarget,
        chain: &Chain,
        rects: Option<&[[u32; 4]]>,
    ) -> Option<bool> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post region test"),
            });
        let scissored = match rects {
            Some(rects) => Some(post.run_region(
                &mut encoder,
                &input.view,
                None,
                None,
                &output.view,
                chain.frame,
                chain.seed,
                1.0,
                rects,
                None,
            )),
            None => {
                post.run(
                    &mut encoder,
                    &input.view,
                    None,
                    None,
                    &output.view,
                    chain.frame,
                    chain.seed,
                );
                None
            }
        };
        gpu.queue.submit(Some(encoder.finish()));
        scissored
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn region_post_equals_the_full_post_inside_its_rects() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (203u32, 157u32);
        let source = synthetic(width, height);
        let input = upload_source(&gpu, &source);
        let sets: [&[[u32; 4]]; 4] = [
            &[[13, 7, 61, 41]],
            &[[0, 0, 9, 9], [100, 60, 190, 140], [150, 3, 203, 20]],
            &[[190, 140, 400, 400]],
            &[[20, 20, 140, 120], [60, 40, 180, 150]],
        ];
        for (name, chain) in region_chains() {
            let post = GpuChain::new(chain.clone(), &gpu.device, width, height);
            assert!(post.region_reach().is_some(), "{name}");
            let full = output_target(&gpu, width, height);
            post_into(&gpu, &post, &input, &full, &chain, None);
            let want = gpu.readback_rgba16(&full).unwrap();
            for rects in sets {
                let region = output_target(&gpu, width, height);
                let scissored = post_into(&gpu, &post, &input, &region, &chain, Some(rects));
                assert_eq!(scissored, Some(true), "{name}");
                let got = gpu.readback_rgba16(&region).unwrap();
                let mut inside_pixels = 0;
                for y in 0..height {
                    for x in 0..width {
                        let inside = rects
                            .iter()
                            .any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3]);
                        let at = ((y * width + x) * 4) as usize;
                        if inside {
                            inside_pixels += 1;
                            for channel in 0..4 {
                                let delta =
                                    (float(got[at + channel]) - float(want[at + channel])).abs();
                                assert!(
                                    delta <= 1.0 / 255.0,
                                    "{name} {rects:?} ({x},{y}) channel {channel}: {} vs {}",
                                    float(got[at + channel]),
                                    float(want[at + channel])
                                );
                            }
                        } else {
                            assert!(
                                got[at..at + 4].iter().all(|v| *v == 0),
                                "{name} {rects:?} ({x},{y}) outside the rects was written"
                            );
                        }
                    }
                }
                assert!(inside_pixels > 0);
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn region_post_falls_back_to_the_full_post_for_whole_frame_kernels() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (96u32, 64u32);
        let source = synthetic(width, height);
        let input = upload_source(&gpu, &source);
        let chains = [
            as_live(frame_finish()),
            Chain {
                passes: vec![Pass::Exposure(1.0), Pass::Distortion(0.05), Pass::Encode],
                seed: 3,
                frame: 2,
            },
            Chain {
                passes: vec![
                    Pass::Exposure(1.0),
                    Pass::Pixel(crate::fx::Pixelate {
                        size: 5,
                        levels: 0,
                        palette: false,
                    }),
                    Pass::Encode,
                ],
                seed: 3,
                frame: 2,
            },
        ];
        for chain in chains {
            let post = GpuChain::new(chain.clone(), &gpu.device, width, height);
            assert_eq!(post.region_reach(), None);
            let full = output_target(&gpu, width, height);
            post_into(&gpu, &post, &input, &full, &chain, None);
            let region = output_target(&gpu, width, height);
            let scissored = post_into(
                &gpu,
                &post,
                &input,
                &region,
                &chain,
                Some(&[[10, 10, 30, 30]]),
            );
            assert_eq!(scissored, Some(false));
            assert_eq!(
                gpu.readback_rgba16(&full).unwrap(),
                gpu.readback_rgba16(&region).unwrap()
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_region_post_4k() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        let (width, height) = (3840u32, 2160u32);
        let input = gpu.offscreen(width, height, FORMAT).unwrap();
        let output = output_target(&gpu, width, height);
        let mut chains = vec![(
            "standard",
            Chain {
                passes: vec![
                    Pass::Exposure(1.0),
                    Pass::Tone(crate::tone::Tone::aces()),
                    Pass::Encode,
                ],
                seed: 0,
                frame: 0,
            },
        )];
        chains.extend(
            region_chains().into_iter().filter(|(name, _)| {
                matches!(*name, "pointwise finish" | "kuwahara" | "two kernels")
            }),
        );
        let rects: [(&str, [u32; 4]); 2] = [
            ("64x64", [1200, 800, 1264, 864]),
            ("512x256", [1200, 800, 1712, 1056]),
        ];
        for (name, chain) in chains {
            let post = turns.cpu(|| GpuChain::new(chain.clone(), &gpu.device, width, height));
            for (label, rect) in std::iter::once(("full", None))
                .chain(rects.iter().map(|(label, rect)| (*label, Some([*rect]))))
            {
                let mut samples = Vec::new();
                for attempt in 0..15 {
                    let mut encoder =
                        gpu.device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("region benchmark"),
                            });
                    match &rect {
                        Some(rects) => {
                            post.run_region(
                                &mut encoder,
                                &input.view,
                                None,
                                None,
                                &output.view,
                                0,
                                0,
                                1.0,
                                rects,
                                Some(&mut timer),
                            );
                        }
                        None => post.run_timed(
                            &mut encoder,
                            &input.view,
                            None,
                            None,
                            &output.view,
                            0,
                            0,
                            Some(&mut timer),
                        ),
                    }
                    let slot = timer.finish(&mut encoder);
                    gpu.queue.submit(Some(encoder.finish()));
                    if let Some(slot) = slot {
                        timer.submitted(slot);
                    }
                    gpu.device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .unwrap();
                    let timings = timer.collect(&gpu.device);
                    let total: f64 = timings.iter().flatten().map(|pass| pass.milliseconds).sum();
                    turns.add(total);
                    if attempt > 0 {
                        samples.push(total);
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!("{name} {label} {:.4} ms", samples[7]);
            }
        }
    }

    pub(crate) const CAPACITY: (u32, u32) = (160, 96);
    pub(crate) const VIEWPORTS: [(u32, u32); 2] = [(120, 72), (117, 71)];
    const LOUD: [f32; 4] = [400.0, 0.0, 900.0, 1.0];
    const BENT: [f32; 4] = [5.0, -7.0, 3.0, 1.0];
    const SENTINEL: f32 = 77.0;

    #[test]
    fn a_viewport_refuses_empty_and_oversized_rects() {
        assert_eq!(fit_viewport((160, 96), (160, 96)), Ok((160, 96)));
        assert_eq!(fit_viewport((1, 1), (160, 96)), Ok((1, 1)));
        assert_eq!(fit_viewport((117, 71), (160, 96)), Ok((117, 71)));
        for viewport in [(0, 96), (160, 0), (0, 0), (161, 96), (160, 97), (400, 400)] {
            assert!(
                fit_viewport(viewport, (160, 96)).is_err(),
                "{viewport:?} was accepted"
            );
        }
    }

    fn detail(x: u32, y: u32) -> [f32; 4] {
        let shades = [0.0, 0.25, 0.5, 0.75, 1.0, 2.0];
        let fine = ((x * 7 + y * 3) % 17) as f32 * 0.02;
        [
            shades[((x / 8 + y / 16) % 6) as usize] + fine,
            shades[((x / 16 + y / 8 + 2) % 6) as usize],
            shades[((x / 8 + 3) % 6) as usize] + fine * 0.5,
            1.0,
        ]
    }

    fn tilted(x: u32, y: u32) -> [f32; 4] {
        match (x / 9 + y / 13) % 3 {
            0 => [0.0, 0.6, 0.8, 1.0],
            1 => [0.7, 0.0, 0.71, 1.0],
            _ => [0.0, 0.0, 1.0, 1.0],
        }
    }

    struct Scene {
        input: OffscreenTarget,
        normal: OffscreenTarget,
        _depth: wgpu::Texture,
        depth: wgpu::TextureView,
    }

    pub(crate) fn framed(
        size: (u32, u32),
        active: (u32, u32),
        inside: fn(u32, u32) -> [f32; 4],
        outside: [f32; 4],
    ) -> Vec<u16> {
        Image::from_fn(size.0, size.1, |x, y| {
            if x < active.0 && y < active.1 {
                inside(x, y)
            } else {
                outside
            }
        })
        .pixels
        .iter()
        .flat_map(|p| p.iter().map(|v| half(*v)))
        .collect()
    }

    fn depth_texture(
        gpu: &Gpu,
        size: (u32, u32),
        active: (u32, u32),
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post viewport depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let source = format!(
            "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{ return vec4<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0, 0.0, 1.0); }} @fragment fn fs(@builtin(position) at: vec4<f32>) -> @builtin(frag_depth) f32 {{ let p = vec2<u32>(at.xy); if (p.x >= {}u || p.y >= {}u) {{ return 0.02; }} return select(0.35, 0.7, ((p.x / 11u + p.y / 7u) % 2u) == 1u) + f32(p.x % 5u) * 0.01; }}",
            active.0, active.1
        );
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("post viewport depth"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("post viewport depth"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[],
                }),
                multiview: None,
                cache: None,
            });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post viewport depth"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post viewport depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        (texture, view)
    }

    fn scene(gpu: &Gpu, size: (u32, u32), active: (u32, u32)) -> Scene {
        let input = gpu.offscreen(size.0, size.1, FORMAT).unwrap();
        gpu.upload_rgba16(&input, &framed(size, active, detail, LOUD))
            .unwrap();
        let normal = gpu.offscreen(size.0, size.1, FORMAT).unwrap();
        gpu.upload_rgba16(&normal, &framed(size, active, tilted, BENT))
            .unwrap();
        let (texture, depth) = depth_texture(gpu, size, active);
        Scene {
            input,
            normal,
            _depth: texture,
            depth,
        }
    }

    fn sentinel(gpu: &Gpu, size: (u32, u32)) -> OffscreenTarget {
        let output = output_target(gpu, size.0, size.1);
        let fill = vec![half(SENTINEL); (size.0 * size.1 * 4) as usize];
        gpu.upload_rgba16(&output, &fill).unwrap();
        output
    }

    fn post_scene(
        gpu: &Gpu,
        post: &GpuChain,
        scene: &Scene,
        output: &OffscreenTarget,
        chain: &Chain,
        rects: Option<&[[u32; 4]]>,
    ) -> Vec<u16> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("post viewport test"),
            });
        match rects {
            Some(rects) => {
                post.run_region(
                    &mut encoder,
                    &scene.input.view,
                    Some(&scene.depth),
                    Some(&scene.normal.view),
                    &output.view,
                    chain.frame,
                    chain.seed,
                    1.0,
                    rects,
                    None,
                );
            }
            None => post.run(
                &mut encoder,
                &scene.input.view,
                Some(&scene.depth),
                Some(&scene.normal.view),
                &output.view,
                chain.frame,
                chain.seed,
            ),
        }
        gpu.queue.submit(Some(encoder.finish()));
        gpu.readback_rgba16(output).unwrap()
    }

    fn viewport_chains() -> Vec<(&'static str, Chain)> {
        let chain = |passes: Vec<Pass>| {
            let mut passes = passes;
            passes.push(Pass::Encode);
            Chain {
                passes,
                seed: 31,
                frame: 5,
            }
        };
        let mut chains = vec![
            ("standard finish", as_live(frame_finish())),
            ("tent finish", chain(fused_finish(0.5, 0.3))),
            (
                "linear bloom",
                chain(vec![
                    Pass::Tone(crate::tone::Tone::aces()),
                    Pass::Bloom(crate::bloom::Bloom::neutral(0.45)),
                ]),
            ),
            (
                "rings",
                chain(vec![
                    Pass::Tone(crate::tone::Tone::aces()),
                    Pass::Bloom(crate::bloom::Bloom::rings(
                        0.5,
                        0.6,
                        4,
                        crate::bloom::NEUTRAL_RADII,
                        crate::bloom::NEUTRAL_GAINS,
                    )),
                ]),
            ),
            (
                "aberration",
                chain(vec![Pass::Exposure(1.0), Pass::Aberration(2.5)]),
            ),
            (
                "distortion",
                chain(vec![Pass::Exposure(1.0), Pass::Distortion(0.08)]),
            ),
            (
                "pixelate",
                chain(vec![Pass::Pixel(crate::fx::Pixelate {
                    size: 5,
                    levels: 0,
                    palette: false,
                })]),
            ),
            (
                "scratches",
                chain(vec![Pass::Scratches(crate::fx::Scratches {
                    count: 9,
                    strength: 0.4,
                })]),
            ),
            (
                "halation",
                chain(vec![
                    Pass::Halation(crate::fx::Halation {
                        strength: 0.6,
                        threshold: 0.5,
                        radius: 6,
                    }),
                    Pass::Tone(crate::tone::Tone::aces()),
                ]),
            ),
            (
                "neon",
                chain(vec![Pass::Neon(crate::fx::Neon {
                    saturation: 1.8,
                    strength: 1.35,
                    radius: 3,
                })]),
            ),
        ];
        chains.extend(region_chains());
        chains.extend(
            Style::ALL
                .into_iter()
                .map(|style| (style.name(), as_live(style.chain()))),
        );
        let mut tape = as_live(Style::Crt.chain());
        tape.set_tape(crate::tape::Tape::rewind(1.0));
        tape.frame = 30;
        tape.seed = 7;
        chains.push(("tape", tape));
        chains
    }

    fn samples_in_hardware(post: &GpuChain) -> bool {
        post.active()
            .iter()
            .flat_map(|group| &group.stages)
            .any(|stage| {
                matches!(
                    stage.aux,
                    Aux::BloomLinear | Aux::LinearSampler | Aux::GradeBloomLinear
                )
            })
    }

    fn compare_viewport(
        name: &str,
        got: &[u16],
        want: &[u16],
        viewport: (u32, u32),
        hardware: bool,
    ) -> f32 {
        let (w, h) = viewport;
        let mut worst = 0.0f32;
        for y in 0..CAPACITY.1 {
            for x in 0..CAPACITY.0 {
                let at = ((y * CAPACITY.0 + x) * 4) as usize;
                if x >= w || y >= h {
                    assert!(
                        got[at..at + 4].iter().all(|v| *v == half(SENTINEL)),
                        "{name} {viewport:?} wrote ({x},{y}) outside its viewport"
                    );
                    continue;
                }
                let there = ((y * w + x) * 4) as usize;
                for channel in 0..4 {
                    let a = got[at + channel];
                    let b = want[there + channel];
                    let delta = if a == b {
                        0.0
                    } else {
                        (float(a) - float(b)).abs()
                    };
                    let delta = if delta.is_nan() { f32::INFINITY } else { delta };
                    worst = worst.max(delta);
                    if hardware {
                        assert!(
                            delta <= 1.0 / 255.0,
                            "{name} {viewport:?} ({x},{y}) channel {channel} must be within 1/255 of a chain at its size: {} vs {}",
                            float(a),
                            float(b)
                        );
                    } else {
                        assert!(
                            a == b,
                            "{name} {viewport:?} ({x},{y}) channel {channel} must equal a chain at its size exactly: {} vs {}",
                            float(a),
                            float(b)
                        );
                    }
                }
            }
        }
        worst
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_viewport_post_equals_a_chain_at_the_viewport_size() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let garbage = scene(&gpu, CAPACITY, (0, 0));
        let scenes: Vec<(Scene, Scene)> = VIEWPORTS
            .iter()
            .map(|&viewport| {
                (
                    scene(&gpu, CAPACITY, viewport),
                    scene(&gpu, viewport, viewport),
                )
            })
            .collect();
        let mut checked = 0;
        for (name, chain) in viewport_chains() {
            let mut post = GpuChain::new(chain.clone(), &gpu.device, CAPACITY.0, CAPACITY.1);
            let hardware = samples_in_hardware(&post);
            let mut report = Vec::new();
            for (&viewport, (framed, exact)) in VIEWPORTS.iter().zip(&scenes) {
                post.set_viewport(CAPACITY.0, CAPACITY.1).unwrap();
                post_scene(
                    &gpu,
                    &post,
                    &garbage,
                    &output_target(&gpu, CAPACITY.0, CAPACITY.1),
                    &chain,
                    None,
                );
                post.set_viewport(viewport.0, viewport.1).unwrap();
                assert_eq!(post.viewport(), viewport);
                assert_eq!(post.capacity(), CAPACITY);
                let got = post_scene(&gpu, &post, framed, &sentinel(&gpu, CAPACITY), &chain, None);
                let reference = GpuChain::new(chain.clone(), &gpu.device, viewport.0, viewport.1);
                let want = post_scene(
                    &gpu,
                    &reference,
                    exact,
                    &output_target(&gpu, viewport.0, viewport.1),
                    &chain,
                    None,
                );
                let worst = compare_viewport(name, &got, &want, viewport, hardware);
                report.push(format!("{viewport:?} {:.5}", worst * 255.0));
                if post.region_reach().is_some() {
                    let rects = [[0, 0, CAPACITY.0, CAPACITY.1]];
                    let region = post_scene(
                        &gpu,
                        &post,
                        framed,
                        &sentinel(&gpu, CAPACITY),
                        &chain,
                        Some(&rects),
                    );
                    let worst = compare_viewport(name, &region, &want, viewport, hardware);
                    report.push(format!("region {:.5}", worst * 255.0));
                }
                checked += 1;
            }
            println!(
                "{name} ({}): worst {} /255",
                if hardware { "sampled" } else { "exact" },
                report.join(", ")
            );
        }
        assert!(checked >= 2 * 30, "{checked}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_full_viewport_post_is_byte_identical() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let garbage = scene(&gpu, CAPACITY, (0, 0));
        let whole = scene(&gpu, CAPACITY, CAPACITY);
        for (name, chain) in viewport_chains() {
            let fresh = GpuChain::new(chain.clone(), &gpu.device, CAPACITY.0, CAPACITY.1);
            let want = post_scene(
                &gpu,
                &fresh,
                &whole,
                &output_target(&gpu, CAPACITY.0, CAPACITY.1),
                &chain,
                None,
            );
            let mut post = GpuChain::new(chain.clone(), &gpu.device, CAPACITY.0, CAPACITY.1);
            post.set_viewport(VIEWPORTS[1].0, VIEWPORTS[1].1).unwrap();
            post_scene(
                &gpu,
                &post,
                &garbage,
                &output_target(&gpu, CAPACITY.0, CAPACITY.1),
                &chain,
                None,
            );
            post.set_viewport(CAPACITY.0, CAPACITY.1).unwrap();
            let got = post_scene(
                &gpu,
                &post,
                &whole,
                &output_target(&gpu, CAPACITY.0, CAPACITY.1),
                &chain,
                None,
            );
            assert!(got == want, "{name} changed bytes at the full viewport");
            post.set_viewport(VIEWPORTS[0].0, VIEWPORTS[0].1).unwrap();
            post.resize(CAPACITY.0, CAPACITY.1);
            assert_eq!(post.viewport(), CAPACITY, "{name}");
        }
    }
}
