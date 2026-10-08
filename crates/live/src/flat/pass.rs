use super::icons::{FORMAT as ICON_FORMAT, GpuIcons, Icons};
use super::{
    Draw, FX, Fill, Fit, FlatScene, Glyphs, INSTANCE_ATTRIBUTES, Instance, KIND_SHADOW, LIT, Order,
    Shape, Step, pack_shadow, pack_shape, shader,
};
use crate::text::{GpuAtlas, PackedGlyph, TextSpace, pack_glyphs, pack_quads, screen_bounds};
use bytemuck::{Pod, Zeroable};
use pfx_gpu::{GpuProfiler, wgpu};
use std::ops::Range;
use wgpu::util::DeviceExt;

pub const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const TEXT_SLOT: u64 = 256;
const TEXT_CACHE: usize = 64;
const BASE_PIPELINES: usize = 6;
const ABSORBED_AREA: f32 = 128.0 * 128.0;
const GRADIENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const GRADIENT_TEXELS: u32 = 12;

pub struct FlatIcons {
    pub icons: Icons,
    gpu: GpuIcons,
}

impl FlatIcons {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, icons: Icons) -> Self {
        let gpu = icons.upload(device, queue);
        Self { icons, gpu }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.gpu.view
    }
}

pub struct Environment {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    mips: u32,
    pub intensity: f32,
}

impl Environment {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        linear: &[[f32; 4]],
        intensity: f32,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || linear.len() != (width * height) as usize {
            return Err("the environment needs width × height linear RGBA texels".into());
        }
        if width > 4096 || height > 4096 {
            return Err("the environment is larger than 4096 texels a side".into());
        }
        if !intensity.is_finite() || intensity < 0.0 {
            return Err("the environment intensity must be finite and nonnegative".into());
        }
        let mut levels = vec![(width, height, linear.to_vec())];
        while let Some((w, h, texels)) = levels.last().filter(|(w, h, _)| *w > 1 || *h > 1) {
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut next = vec![[0.0f32; 4]; (nw * nh) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    let mut sum = [0.0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let texel = texels[(sy * w + sx) as usize];
                        for channel in 0..4 {
                            sum[channel] += texel[channel] * 0.25;
                        }
                    }
                    next[(y * nw + x) as usize] = sum;
                }
            }
            levels.push((nw, nh, next));
        }
        let mips = levels.len() as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flat environment"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, texels)) in levels.iter().enumerate() {
            let halves = texels
                .iter()
                .flatten()
                .map(|value| half::f16::from_f32(*value).to_bits())
                .collect::<Vec<_>>();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&halves),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 8),
                    rows_per_image: Some(*h),
                },
                wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&Default::default());
        Ok(Self {
            _texture: texture,
            view,
            mips,
            intensity,
        })
    }

    pub fn mips(&self) -> u32 {
        self.mips
    }
}

pub struct FlatTarget<'a> {
    pub colour: &'a wgpu::TextureView,
    pub ids: Option<&'a wgpu::TextureView>,
    pub size: [u32; 2],
    pub clear: Option<[f32; 4]>,
    pub clear_ids: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct View {
    size: [f32; 4],
    light: [f32; 4],
    environment: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LayerQuad {
    rect: [f32; 4],
    opacity: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TextUniform {
    matrix: [[f32; 4]; 4],
    view_projection: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    rest_normal: [f32; 4],
    atlas_size: [f32; 2],
    mode: u32,
    deformer: u32,
    cells: [u32; 4],
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Instances {
        range: Range<u32>,
        lit: bool,
    },
    Text {
        glyphs: Range<u32>,
        slot: u32,
        atlas: usize,
        id: u32,
    },
    Layer {
        quad: u32,
    },
    EndLayer {
        quad: u32,
    },
}

struct Pool<T: Pod> {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    items: Vec<T>,
}

impl<T: Pod> Pool<T> {
    fn new(label: &'static str, usage: wgpu::BufferUsages) -> Self {
        Self {
            label,
            usage,
            buffer: None,
            capacity: 0,
            items: Vec::new(),
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let bytes = (self.items.len() * std::mem::size_of::<T>()) as u64;
        if bytes == 0 {
            return false;
        }
        let grown = self.buffer.is_none() || bytes > self.capacity;
        if grown {
            self.capacity = bytes.next_power_of_two().max(4096);
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.capacity,
                usage: self.usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.items));
        }
        grown
    }
}

struct Finish {
    format: wgpu::TextureFormat,
    linear: bool,
    blend: bool,
    pipeline: wgpu::RenderPipeline,
}

pub struct FlatPass {
    view_layout: wgpu::BindGroupLayout,
    layer_layout: wgpu::BindGroupLayout,
    text_layout: wgpu::BindGroupLayout,
    finish_layout: wgpu::BindGroupLayout,
    colour: wgpu::RenderPipeline,
    ids: wgpu::RenderPipeline,
    lit: Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>,
    module: wgpu::ShaderModule,
    main_layout: wgpu::PipelineLayout,
    clear: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    text: wgpu::RenderPipeline,
    text_ids: wgpu::RenderPipeline,
    finish_module: wgpu::ShaderModule,
    finish_pipeline_layout: wgpu::PipelineLayout,
    finishes: Vec<Finish>,
    uniform: wgpu::Buffer,
    icon_sampler: wgpu::Sampler,
    environment_sampler: wgpu::Sampler,
    text_sampler: wgpu::Sampler,
    empty_icons: wgpu::TextureView,
    empty_sprites: wgpu::TextureView,
    sprite_sampler: wgpu::Sampler,
    empty_environment: wgpu::TextureView,
    empty_gradients: wgpu::TextureView,
    gradients: Option<(wgpu::Texture, wgpu::TextureView, u32)>,
    gradient_rows: Vec<[[f32; 4]; GRADIENT_TEXELS as usize]>,
    uploaded_rows: Vec<[[f32; 4]; GRADIENT_TEXELS as usize]>,
    bind: Option<(
        wgpu::TextureView,
        wgpu::TextureView,
        wgpu::TextureView,
        wgpu::TextureView,
        wgpu::BindGroup,
    )>,
    text_binds: Vec<(wgpu::TextureView, wgpu::BindGroup)>,
    instances: Pool<Instance>,
    quads: Pool<LayerQuad>,
    glyphs: Pool<PackedGlyph>,
    text_uniforms: Pool<[u8; TEXT_SLOT as usize]>,
    scratch: Vec<PackedGlyph>,
    text_items: Vec<(usize, usize, Range<u32>, u32)>,
    order: Order,
    ops: Vec<Op>,
    layer: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup, [u32; 2])>,
    colour_target: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    size: [u32; 2],
    capacity: Option<[u32; 2]>,
    clip: Option<[u32; 4]>,
    passes: u32,
    made: usize,
}

fn instance_attributes() -> [wgpu::VertexAttribute; INSTANCE_ATTRIBUTES as usize] {
    std::array::from_fn(|index| wgpu::VertexAttribute {
        format: if index == 9 {
            wgpu::VertexFormat::Uint32x4
        } else {
            wgpu::VertexFormat::Float32x4
        },
        offset: (index * 16) as u64,
        shader_location: index as u32,
    })
}

pub fn view_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
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
    let sampler = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        texture(1),
        sampler(2),
        texture(3),
        sampler(4),
        texture(6),
        sampler(7),
        wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
    ]
}

pub fn layer_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }]
}

pub fn text_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = crate::text::text_entries();
    entries[0].ty = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: true,
        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<TextUniform>() as u64),
    };
    entries
}

pub fn finish_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    layer_entries()
}

fn empty(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    bytes: &[u8],
) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("flat empty input"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytes,
        )
        .create_view(&Default::default())
}

fn colour_state(format: wgpu::TextureFormat, blend: bool) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format,
        blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entries: (&str, &str),
    buffers: &[wgpu::VertexBufferLayout<'_>],
    target: wgpu::ColorTargetState,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(entries.0),
            buffers,
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entries.1),
            targets: &[Some(target)],
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}

fn rect_union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

impl FlatPass {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::with_source(device, queue, shader::source())
    }

    pub(crate) fn with_source(device: &wgpu::Device, queue: &wgpu::Queue, source: String) -> Self {
        let group = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let view_layout = group("flat view", &view_entries());
        let layer_layout = group("flat layer", &layer_entries());
        let text_layout = group("flat text", &text_entries());
        let finish_layout = group("flat finish", &finish_entries());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flat pass"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let text_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flat text"),
            source: wgpu::ShaderSource::Wgsl(crate::text::shader_source().into()),
        });
        let finish_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flat finish"),
            source: wgpu::ShaderSource::Wgsl(shader::FINISH_WGSL.into()),
        });
        let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flat"),
            bind_group_layouts: &[&view_layout],
            push_constant_ranges: &[],
        });
        let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flat layer composite"),
            bind_group_layouts: &[&view_layout, &layer_layout],
            push_constant_ranges: &[],
        });
        let text_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flat text"),
            bind_group_layouts: &[&text_layout],
            push_constant_ranges: &[],
        });
        let finish_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flat finish"),
                bind_group_layouts: &[&finish_layout],
                push_constant_ranges: &[],
            });
        let attributes = instance_attributes();
        let instance_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attributes,
        }];
        let quad_attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 1,
            },
        ];
        let quad_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LayerQuad>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &quad_attributes,
        }];
        let glyph_attributes: [wgpu::VertexAttribute; 6] =
            std::array::from_fn(|index| wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: (index * 16) as u64,
                shader_location: index as u32,
            });
        let glyph_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PackedGlyph>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &glyph_attributes,
        }];
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        let list = wgpu::PrimitiveTopology::TriangleList;
        let colour = pipeline(
            device,
            "flat colour",
            &main_layout,
            &module,
            ("flat_vertex", "flat_colour"),
            &instance_buffers,
            colour_state(COLOUR_FORMAT, true),
            strip,
        );
        let ids = pipeline(
            device,
            "flat ids",
            &main_layout,
            &module,
            ("flat_id_vertex", "flat_id"),
            &instance_buffers,
            wgpu::ColorTargetState {
                format: ID_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            strip,
        );
        let clear = pipeline(
            device,
            "flat layer clear",
            &main_layout,
            &module,
            ("flat_layer_vertex", "flat_layer_clear"),
            &quad_buffers,
            colour_state(COLOUR_FORMAT, false),
            strip,
        );
        let composite = pipeline(
            device,
            "flat layer composite",
            &composite_layout,
            &module,
            ("flat_layer_vertex", "flat_layer_composite"),
            &quad_buffers,
            colour_state(COLOUR_FORMAT, true),
            strip,
        );
        let text = pipeline(
            device,
            "flat text",
            &text_pipeline_layout,
            &text_module,
            ("vs_main", "fs_main"),
            &glyph_buffers,
            colour_state(COLOUR_FORMAT, true),
            list,
        );
        let text_ids = pipeline(
            device,
            "flat text ids",
            &text_pipeline_layout,
            &text_module,
            ("vs_main", "fs_id"),
            &glyph_buffers,
            wgpu::ColorTargetState {
                format: ID_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            list,
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flat view"),
            size: std::mem::size_of::<View>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let linear = |label, mipmaps: bool, repeat: bool| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                address_mode_u: if repeat {
                    wgpu::AddressMode::Repeat
                } else {
                    wgpu::AddressMode::ClampToEdge
                },
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: if mipmaps {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                },
                ..Default::default()
            })
        };
        let text_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("flat text"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            lod_max_clamp: 3.0,
            ..Default::default()
        });
        let far = half::f16::from_f32(1.0e4).to_bits().to_le_bytes();
        Self {
            view_layout,
            layer_layout,
            text_layout,
            finish_layout,
            colour,
            ids,
            lit: None,
            module,
            main_layout,
            clear,
            composite,
            text,
            text_ids,
            finish_module,
            finish_pipeline_layout,
            finishes: Vec::new(),
            uniform,
            icon_sampler: linear("flat icons", false, false),
            environment_sampler: linear("flat environment", true, true),
            text_sampler,
            empty_icons: empty(device, queue, ICON_FORMAT, &far),
            empty_sprites: empty(device, queue, super::sprites::SPRITE_FORMAT, &[255; 4]),
            sprite_sampler: super::sprites::sampler(device, super::SpriteFilter::Linear),
            empty_environment: empty(device, queue, wgpu::TextureFormat::Rgba16Float, &[0; 8]),
            empty_gradients: empty(device, queue, GRADIENT_FORMAT, &[0; 16]),
            gradients: None,
            gradient_rows: Vec::new(),
            uploaded_rows: Vec::new(),
            bind: None,
            text_binds: Vec::new(),
            instances: Pool::new("flat instances", wgpu::BufferUsages::VERTEX),
            quads: Pool::new("flat layer quads", wgpu::BufferUsages::VERTEX),
            glyphs: Pool::new("flat glyphs", wgpu::BufferUsages::VERTEX),
            text_uniforms: Pool::new("flat text uniforms", wgpu::BufferUsages::UNIFORM),
            scratch: Vec::new(),
            text_items: Vec::new(),
            order: Order::default(),
            ops: Vec::new(),
            layer: None,
            colour_target: None,
            size: [0, 0],
            capacity: None,
            clip: None,
            passes: 0,
            made: BASE_PIPELINES,
        }
    }

    pub fn target_textures(&self) -> Vec<wgpu::Texture> {
        self.layer
            .iter()
            .map(|layer| layer.0.clone())
            .chain(self.colour_target.iter().map(|target| target.0.clone()))
            .collect()
    }

    pub fn set_clip(&mut self, clip: Option<[u32; 4]>) {
        self.clip = clip;
    }

    pub fn set_capacity(&mut self, capacity: Option<[u32; 2]>) {
        self.capacity = capacity;
    }

    fn viewport(&self) -> Option<[u32; 2]> {
        self.capacity.map(|_| self.size)
    }

    pub fn pipelines_made(&self) -> usize {
        self.made
    }

    pub fn warm(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        finishes: &[(wgpu::TextureFormat, bool, bool)],
        colour: bool,
    ) {
        self.ensure_lit(device);
        self.ensure_layer(device, size);
        if colour {
            self.colour_target(device, size);
        }
        for &(format, linear, blend) in finishes {
            self.finish_index(device, format, linear, blend);
        }
    }

    pub fn instance_count(&self) -> usize {
        self.instances.items.len()
    }

    pub fn instances(&self) -> &[Instance] {
        &self.instances.items
    }

    pub fn layer_count(&self) -> usize {
        self.quads.items.len()
    }

    fn push_instance(&mut self, instance: Instance) {
        let start = self.instances.items.len() as u32;
        let lit = instance.info[0] & (LIT | FX) != 0;
        let [x0, y0, x1, y1] = instance.pixel_bounds();
        let small = (x1 - x0) * (y1 - y0) <= ABSORBED_AREA;
        self.instances.items.push(instance);
        match self.ops.last_mut() {
            Some(Op::Instances {
                range,
                lit: run_lit,
            }) if range.end == start && (*run_lit == lit || (*run_lit && small)) => {
                range.end = start + 1
            }
            _ => self.ops.push(Op::Instances {
                range: start..start + 1,
                lit,
            }),
        }
    }

    fn ensure_lit(&mut self, device: &wgpu::Device) {
        if self.lit.is_some() {
            return;
        }
        let attributes = instance_attributes();
        let buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attributes,
        }];
        let colour = pipeline(
            device,
            "flat colour lit",
            &self.main_layout,
            &self.module,
            ("flat_vertex", "flat_colour_lit"),
            &buffers,
            colour_state(COLOUR_FORMAT, true),
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let ids = pipeline(
            device,
            "flat ids lit",
            &self.main_layout,
            &self.module,
            ("flat_id_vertex_rich", "flat_id_lit"),
            &buffers,
            wgpu::ColorTargetState {
                format: ID_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        self.lit = Some((colour, ids));
        self.made += 2;
    }

    fn text_bind(&mut self, device: &wgpu::Device, atlas: &GpuAtlas) -> Result<usize, String> {
        if let Some(index) = self
            .text_binds
            .iter()
            .position(|(view, _)| view == atlas.view())
        {
            return Ok(index);
        }
        let buffer = self
            .text_uniforms
            .buffer
            .as_ref()
            .ok_or("flat text uniforms are missing")?;
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flat text"),
            layout: &self.text_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<TextUniform>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.text_sampler),
                },
            ],
        });
        self.text_binds.push((atlas.view().clone(), group));
        Ok(self.text_binds.len() - 1)
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &FlatScene<'_>,
        size: [u32; 2],
    ) -> Result<Fit, String> {
        let fit = Fit::new(scene.layout, size)?;
        self.size = size;
        self.instances.items.clear();
        self.quads.items.clear();
        self.glyphs.items.clear();
        self.text_uniforms.items.clear();
        self.ops.clear();
        self.gradient_rows.clear();
        self.order.build(scene.draws, scene.groups.len())?;
        for group in scene.groups {
            if !(0.0..=1.0).contains(&group.opacity) {
                return Err("a group's opacity must be in 0..=1".into());
            }
        }
        let icons = scene.icons.map(|icons| &icons.icons);
        let clip = fit.clip_from_layout();
        let mut open_quad: Option<(u32, [f32; 4])> = None;
        let mut text_items = std::mem::take(&mut self.text_items);
        text_items.clear();
        if self.text_binds.len() >= TEXT_CACHE {
            self.text_binds.clear();
        }
        let steps = std::mem::take(&mut self.order.steps);
        for step in &steps {
            match *step {
                Step::Layer(group) => {
                    let quad = self.quads.items.len() as u32;
                    self.quads.items.push(LayerQuad {
                        rect: [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
                        opacity: [scene.groups[usize::from(group)].opacity, 0.0, 0.0, 0.0],
                    });
                    self.ops.push(Op::Layer { quad });
                    open_quad = Some((quad, [f32::MAX, f32::MAX, f32::MIN, f32::MIN]));
                }
                Step::EndLayer(_) => {
                    if let Some((quad, rect)) = open_quad.take() {
                        let clamped = [
                            rect[0].floor().max(0.0),
                            rect[1].floor().max(0.0),
                            rect[2].ceil().min(size[0] as f32),
                            rect[3].ceil().min(size[1] as f32),
                        ];
                        self.quads.items[quad as usize].rect = clamped;
                        self.ops.push(Op::EndLayer { quad });
                    }
                }
                Step::Draw(index) => {
                    let draw = &scene.draws[index];
                    let bounds =
                        self.push_draw(draw, scene, &fit, icons, &clip, &mut text_items)?;
                    if let (Some((_, rect)), Some(bounds)) = (open_quad.as_mut(), bounds) {
                        *rect = rect_union(*rect, bounds);
                    }
                }
            }
        }
        self.order.steps = steps;
        self.instances.upload(device, queue);
        self.quads.upload(device, queue);
        self.glyphs.upload(device, queue);
        if self.text_uniforms.upload(device, queue) {
            self.text_binds.clear();
        }
        for (op_index, text_index, glyphs, slot) in text_items.drain(..) {
            let atlas = self.text_bind(device, scene.text[text_index].atlas)?;
            if let Op::Text {
                atlas: bound,
                glyphs: range,
                slot: at,
                ..
            } = &mut self.ops[op_index]
            {
                *bound = atlas;
                *range = glyphs;
                *at = slot;
            }
        }
        self.text_items = text_items;
        let light = scene.light;
        let key = super::normalize3(light.key);
        let (mips, intensity, present) = scene.environment.map_or((1.0, 0.0, 0.0), |environment| {
            (environment.mips as f32, environment.intensity, 1.0)
        });
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&View {
                size: [
                    size[0] as f32,
                    size[1] as f32,
                    1.0 / size[0] as f32,
                    1.0 / size[1] as f32,
                ],
                light: [key[0], key[1], key[2], light.ambient.clamp(0.0, 1.0)],
                environment: [mips, intensity, present, 0.0],
            }),
        );
        self.upload_gradients(device, queue);
        let icon_view = scene.icons.map_or(&self.empty_icons, |icons| icons.view());
        let environment_view = scene
            .environment
            .map_or(&self.empty_environment, |environment| &environment.view);
        let gradient_view = self
            .gradients
            .as_ref()
            .filter(|_| !self.gradient_rows.is_empty())
            .map_or(&self.empty_gradients, |gradients| &gradients.1);
        let (sprite_view, sprite_sampler) = scene
            .sprites
            .map_or((&self.empty_sprites, &self.sprite_sampler), |sprites| {
                (&sprites.view, &sprites.sampler)
            });
        let stale = self
            .bind
            .as_ref()
            .is_none_or(|(icons, environment, gradients, sprites, _)| {
                icons != icon_view
                    || environment != environment_view
                    || gradients != gradient_view
                    || sprites != sprite_view
            });
        if stale {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("flat view"),
                layout: &self.view_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(icon_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.icon_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(environment_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.environment_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(gradient_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(sprite_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::Sampler(sprite_sampler),
                    },
                ],
            });
            self.bind = Some((
                icon_view.clone(),
                environment_view.clone(),
                gradient_view.clone(),
                sprite_view.clone(),
                group,
            ));
        }
        if !self.quads.items.is_empty() {
            self.ensure_layer(device, self.capacity.unwrap_or(size));
        }
        if self
            .ops
            .iter()
            .any(|op| matches!(op, Op::Instances { lit: true, .. }))
        {
            self.ensure_lit(device);
        }
        self.passes = 1 + 2 * self.quads.items.len() as u32;
        Ok(fit)
    }

    fn push_draw(
        &mut self,
        draw: &Draw,
        scene: &FlatScene<'_>,
        fit: &Fit,
        icons: Option<&Icons>,
        clip: &super::Matrix,
        text_items: &mut Vec<(usize, usize, Range<u32>, u32)>,
    ) -> Result<Option<[f32; 4]>, String> {
        if let Shape::Text(index) = draw.shape {
            let text = scene
                .text
                .get(index)
                .ok_or_else(|| format!("a draw names text {index}, which is missing"))?;
            if !super::positive(draw.opacity) || super::placed(&draw.transform, fit).is_none() {
                return Ok(None);
            }
            self.scratch.clear();
            let mode = match text.glyphs {
                Glyphs::Paragraph(paragraph) => {
                    self.scratch.extend(pack_glyphs(&paragraph.glyphs));
                    u32::from(paragraph.atlas.channels == 3)
                }
                Glyphs::Quads(quads) => {
                    self.scratch.extend(pack_quads(quads));
                    match text.atlas.format() {
                        wgpu::TextureFormat::Rgba16Float => 2,
                        wgpu::TextureFormat::Rgba8Unorm => 1,
                        _ => 0,
                    }
                }
            };
            if self.scratch.is_empty() {
                return Ok(None);
            }
            let tint = text.colour.0;
            for glyph in &mut self.scratch {
                for (channel, value) in glyph.color.iter_mut().zip(tint).take(3) {
                    *channel *= value;
                }
                glyph.color[3] *= tint[3] * draw.opacity.min(1.0);
            }
            let matrix = super::multiply(*clip, draw.transform);
            let bounds = screen_bounds(
                &TextSpace::Surface {
                    model_view_projection: matrix,
                },
                self.scratch.iter().copied(),
                self.size,
            );
            let start = self.glyphs.items.len() as u32;
            self.glyphs.items.extend_from_slice(&self.scratch);
            let end = self.glyphs.items.len() as u32;
            let slot = self.text_uniforms.items.len() as u32;
            let size = text.atlas.size();
            let uniform = TextUniform {
                matrix,
                view_projection: super::IDENTITY,
                model: super::IDENTITY,
                rest_normal: [0.0, 0.0, 1.0, 0.0],
                atlas_size: [size[0] as f32, size[1] as f32],
                mode,
                deformer: u32::MAX,
                cells: [crate::text::DEFORMED_CELLS, draw.id, 0, 0],
            };
            let mut bytes = [0u8; TEXT_SLOT as usize];
            bytes[..std::mem::size_of::<TextUniform>()]
                .copy_from_slice(bytemuck::bytes_of(&uniform));
            self.text_uniforms.items.push(bytes);
            text_items.push((self.ops.len(), index, start..end, slot));
            self.ops.push(Op::Text {
                glyphs: start..end,
                slot,
                atlas: 0,
                id: draw.id,
            });
            return Ok(bounds);
        }
        let mut bounds = None;
        if let Some(shadow) = pack_shadow(draw, fit, scene.curve, &scene.light) {
            bounds = Some(shadow.pixel_bounds());
            self.push_instance(shadow);
        }
        if let Some(mut shape) = pack_shape(draw, fit, icons) {
            if let Fill::Gradient(gradient) = &draw.fill {
                let packed = gradient.packed();
                let row = match self.gradient_rows.iter().position(|row| *row == packed) {
                    Some(row) => row,
                    None => {
                        self.gradient_rows.push(packed);
                        self.gradient_rows.len() - 1
                    }
                };
                shape.fill_bottom[0] = row as f32;
            }
            let rect = shape.pixel_bounds();
            bounds = Some(bounds.map_or(rect, |bounds| rect_union(bounds, rect)));
            self.push_instance(shape);
        }
        Ok(bounds)
    }

    fn upload_gradients(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let rows = self.gradient_rows.len() as u32;
        if rows == 0 {
            return;
        }
        if self
            .gradients
            .as_ref()
            .is_none_or(|gradients| gradients.2 < rows)
        {
            let capacity = rows.next_power_of_two().max(8);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("flat gradients"),
                size: wgpu::Extent3d {
                    width: GRADIENT_TEXELS,
                    height: capacity,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: GRADIENT_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.gradients = Some((texture, view, capacity));
            self.uploaded_rows.clear();
        }
        if self.uploaded_rows == self.gradient_rows {
            return;
        }
        let texture = &self.gradients.as_ref().unwrap().0;
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&self.gradient_rows),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GRADIENT_TEXELS * 16),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: GRADIENT_TEXELS,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        self.uploaded_rows.clone_from(&self.gradient_rows);
    }

    fn ensure_layer(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.layer.as_ref().is_some_and(|layer| layer.3 == size) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flat group layer"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOUR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flat group layer"),
            layout: &self.layer_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.layer = Some((texture, view, group, size));
    }

    pub fn colour_target(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if self
            .colour_target
            .as_ref()
            .is_none_or(|target| target.2 != size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("flat colour"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOUR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.colour_target = Some((texture, view, size));
        }
        self.colour_target.as_ref().unwrap().1.clone()
    }

    fn draw_op(&self, pass: &mut wgpu::RenderPass<'_>, op: &Op, ids: bool) {
        match op {
            Op::Instances { range, lit } => {
                let Some(buffer) = &self.instances.buffer else {
                    return;
                };
                let pipeline = match (lit, &self.lit, ids) {
                    (false, _, false) => &self.colour,
                    (false, _, true) => &self.ids,
                    (true, Some((colour, _)), false) => colour,
                    (true, Some((_, lit_ids)), true) => lit_ids,
                    (true, None, _) => return,
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.bind.as_ref().unwrap().4, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..4, range.clone());
            }
            Op::Text {
                glyphs,
                slot,
                atlas,
                id,
            } => {
                if ids && *id == 0 {
                    return;
                }
                let (Some(buffer), Some((_, group))) =
                    (&self.glyphs.buffer, self.text_binds.get(*atlas))
                else {
                    return;
                };
                pass.set_pipeline(if ids { &self.text_ids } else { &self.text });
                pass.set_bind_group(0, group, &[(u64::from(*slot) * TEXT_SLOT) as u32]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..6, glyphs.clone());
            }
            Op::Layer { .. } | Op::EndLayer { .. } => {}
        }
    }

    fn quad(&self, pass: &mut wgpu::RenderPass<'_>, quad: u32, composite: bool) {
        let Some(buffer) = &self.quads.buffer else {
            return;
        };
        if composite {
            pass.set_pipeline(&self.composite);
            pass.set_bind_group(1, &self.layer.as_ref().unwrap().2, &[]);
        } else {
            pass.set_pipeline(&self.clear);
        }
        pass.set_bind_group(0, &self.bind.as_ref().unwrap().4, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..4, quad..quad + 1);
    }

    pub fn encode_colour(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clear: Option<[f32; 4]>,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let timing = profiler
            .as_deref_mut()
            .and_then(|profiler| profiler.pass("flat"));
        let mut index = 0;
        let last = self.passes.saturating_sub(1);
        let writes = |index: u32| {
            profiler.as_deref().and_then(|profiler| {
                profiler
                    .render_writes(timing)
                    .map(|writes| wgpu::RenderPassTimestampWrites {
                        beginning_of_pass_write_index: writes
                            .beginning_of_pass_write_index
                            .filter(|_| index == 0),
                        end_of_pass_write_index: writes
                            .end_of_pass_write_index
                            .filter(|_| index == last),
                        ..writes
                    })
                    .filter(|writes| {
                        writes.beginning_of_pass_write_index.is_some()
                            || writes.end_of_pass_write_index.is_some()
                    })
            })
        };
        let viewport = self.viewport();
        let begin = |encoder: &mut wgpu::CommandEncoder,
                     view: &wgpu::TextureView,
                     load: wgpu::LoadOp<wgpu::Color>,
                     index: u32| {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("flat"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: writes(index),
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            if let Some(viewport) = viewport {
                crate::viewport::apply(&mut pass, viewport);
            }
            pass
        };
        let load = clear.map_or(wgpu::LoadOp::Load, |c| {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(c[0]),
                g: f64::from(c[1]),
                b: f64::from(c[2]),
                a: f64::from(c[3]),
            })
        });
        let mut pass = begin(encoder, target, load, index);
        let layer_view = self.layer.as_ref().map(|layer| &layer.1);
        for op in &self.ops {
            match op {
                Op::Layer { quad } => {
                    drop(pass);
                    index += 1;
                    pass = begin(encoder, layer_view.unwrap(), wgpu::LoadOp::Load, index);
                    self.quad(&mut pass, *quad, false);
                }
                Op::EndLayer { quad } => {
                    drop(pass);
                    index += 1;
                    pass = begin(encoder, target, wgpu::LoadOp::Load, index);
                    self.quad(&mut pass, *quad, true);
                }
                op => self.draw_op(&mut pass, op, false),
            }
        }
    }

    pub fn encode_ids(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        ids: &wgpu::TextureView,
        clear: bool,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let timing = profiler
            .as_deref_mut()
            .and_then(|profiler| profiler.pass("flat ids"));
        let writes = profiler
            .as_deref()
            .and_then(|profiler| profiler.render_writes(timing));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flat ids"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ids,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: writes,
            occlusion_query_set: None,
        });
        if let Some(viewport) = self.viewport() {
            crate::viewport::apply(&mut pass, viewport);
        }
        for op in &self.ops {
            self.draw_op(&mut pass, op, true);
        }
    }

    pub fn has_ids(&self) -> bool {
        self.instances
            .items
            .iter()
            .any(|instance| instance.kind() != KIND_SHADOW)
            || self
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { id, .. } if *id != 0))
    }

    fn finish_index(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        linear: bool,
        blend: bool,
    ) -> usize {
        match self.finishes.iter().position(|finish| {
            finish.format == format && finish.linear == linear && finish.blend == blend
        }) {
            Some(index) => index,
            None => {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("flat finish"),
                    layout: Some(&self.finish_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.finish_module,
                        entry_point: Some("flat_finish_vertex"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.finish_module,
                        entry_point: Some(if linear {
                            "flat_finish_linear"
                        } else {
                            "flat_finish_copy"
                        }),
                        targets: &[Some(colour_state(format, blend))],
                        compilation_options: Default::default(),
                    }),
                    multiview: None,
                    cache: None,
                });
                self.finishes.push(Finish {
                    format,
                    linear,
                    blend,
                    pipeline,
                });
                self.made += 1;
                self.finishes.len() - 1
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode_finish(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        linear: bool,
        blend: bool,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let index = self.finish_index(device, format, linear, blend);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flat finish"),
            layout: &self.finish_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        });
        let timing = profiler
            .as_deref_mut()
            .and_then(|profiler| profiler.pass("flat finish"));
        let writes = profiler
            .as_deref()
            .and_then(|profiler| profiler.render_writes(timing));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flat finish"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: if blend {
                        wgpu::LoadOp::Load
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: writes,
            occlusion_query_set: None,
        });
        if let Some(viewport) = self.viewport() {
            crate::viewport::apply(&mut pass, viewport);
        }
        if let Some([x, y, width, height]) = self.clip {
            pass.set_scissor_rect(x, y, width, height);
        }
        pass.set_pipeline(&self.finishes[index].pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &FlatScene<'_>,
        target: FlatTarget<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<Fit, String> {
        let fit = self.prepare(device, queue, scene, target.size)?;
        self.encode_colour(
            encoder,
            target.colour,
            target.clear,
            profiler.as_deref_mut(),
        );
        if let Some(ids) = target.ids {
            self.encode_ids(encoder, ids, target.clear_ids, profiler);
        }
        Ok(fit)
    }
}
