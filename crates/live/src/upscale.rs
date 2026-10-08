use bytemuck::{Pod, Zeroable};
use pfx_gpu::GpuProfiler;
use pfx_gpu::window::Rect;
use pfx_post::color::srgb_channel;

use crate::flat::look::{FlatLooks, Look, LookFrame};
use crate::flat::{FlatPass, FlatScene};

pub const WGSL: &str = r#"
struct Upscale {
    content: vec4f,
    source: vec2f,
    mode: u32,
    transfer: u32,
    bars: vec4f,
    backdrop: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var upscale_source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> upscale: Upscale;

@vertex
fn upscale_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(xy[index], 0.0, 1.0);
}

fn upscale_texel(p: vec2i) -> vec4f {
    let last = vec2i(upscale.source) - vec2i(1);
    return textureLoad(upscale_source, clamp(p, vec2i(0), last), 0);
}

fn upscale_weights(t: f32) -> vec4f {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4f(
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    );
}

fn upscale_decode(c: vec3f) -> vec3f {
    return select(pow((c + vec3f(0.055)) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}

fn upscale_encode(c: vec3f) -> vec3f {
    let v = max(c, vec3f(0.0));
    return select(1.055 * pow(v, vec3f(1.0 / 2.4)) - vec3f(0.055), v * 12.92, v <= vec3f(0.0031308));
}

fn upscale_transfer(c: vec4f) -> vec4f {
    if (upscale.transfer == 1u) {
        return vec4f(upscale_decode(c.rgb), c.a);
    }
    if (upscale.transfer == 2u) {
        return vec4f(upscale_encode(c.rgb), c.a);
    }
    return c;
}

@fragment
fn upscale_fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let c = upscale.content;
    let p = position.xy;
    if (p.x < c.x || p.y < c.y || p.x >= c.x + c.z || p.y >= c.y + c.w) {
        if (upscale.backdrop == 1u) {
            discard;
        }
        return upscale.bars;
    }
    return upscale_transfer(upscale_filtered(p));
}

fn upscale_filtered(p: vec2f) -> vec4f {
    let c = upscale.content;
    let uv = (p - c.xy) / c.zw * upscale.source - vec2f(0.5);
    if (upscale.mode == 1u) {
        return upscale_texel(vec2i(floor(uv + vec2f(0.5))));
    }
    let base = floor(uv);
    let f = uv - base;
    let wx = upscale_weights(f.x);
    let wy = upscale_weights(f.y);
    let b = vec2i(base);
    var sum = vec4f(0.0);
    for (var j = 0; j < 4; j++) {
        var row = vec4f(0.0);
        for (var i = 0; i < 4; i++) {
            row += upscale_texel(b + vec2i(i - 1, j - 1)) * wx[i];
        }
        sum += row * wy[j];
    }
    let t00 = upscale_texel(b);
    let t10 = upscale_texel(b + vec2i(1, 0));
    let t01 = upscale_texel(b + vec2i(0, 1));
    let t11 = upscale_texel(b + vec2i(1, 1));
    let lo = min(min(t00, t10), min(t01, t11));
    let hi = max(max(t00, t10), max(t01, t11));
    return clamp(sum, lo, hi);
}
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Filter {
    #[default]
    Sharp,
    Nearest,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Transfer {
    #[default]
    Copy,
    Decode,
    Encode,
}

impl Transfer {
    pub fn between(source: wgpu::TextureFormat, output: wgpu::TextureFormat) -> Self {
        match (source.is_srgb(), output.is_srgb()) {
            (false, true) => Transfer::Decode,
            (true, false) => Transfer::Encode,
            _ => Transfer::Copy,
        }
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Uniform {
    content: [f32; 4],
    source: [f32; 2],
    mode: u32,
    transfer: u32,
    bars: [f32; 4],
    backdrop: u32,
    pad: [u32; 3],
}

pub struct Compose<'a> {
    pub source: &'a wgpu::TextureView,
    pub source_format: wgpu::TextureFormat,
    pub source_size: [u32; 2],
    pub output: &'a wgpu::TextureView,
    pub output_format: wgpu::TextureFormat,
    pub output_size: [u32; 2],
    pub content: Rect,
    pub bars: [f32; 4],
    pub filter: Filter,
    pub backdrop: Option<&'a FlatScene<'a>>,
}

pub struct Overlay<'a> {
    pub scene: &'a FlatScene<'a>,
    pub look: Option<&'a LookFrame<'a>>,
    pub output: &'a wgpu::TextureView,
    pub output_format: wgpu::TextureFormat,
    pub output_size: [u32; 2],
    pub content: Rect,
}

struct Source {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    format: wgpu::TextureFormat,
}

pub struct Upscale {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    uniform: wgpu::Buffer,
    group: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    source: Option<Source>,
    flat: Option<FlatPass>,
    backdrop: Option<FlatPass>,
    looks: Option<FlatLooks>,
}

impl Upscale {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("upscale"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("upscale"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscale"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline_layout,
            module,
            pipelines: Vec::new(),
            uniform,
            group: None,
            source: None,
            flat: None,
            backdrop: None,
            looks: None,
        }
    }

    pub fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.pipeline(device, format);
    }

    pub fn warm_overlay<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        looks: impl IntoIterator<Item = &'a Look>,
    ) -> Result<(), String> {
        if size[0] == 0 || size[1] == 0 {
            return Err("warming the overlay needs a size of at least 1 by 1".into());
        }
        self.pipeline(device, format);
        self.flat
            .get_or_insert_with(|| FlatPass::new(device, queue))
            .warm(device, size, &[(format, format.is_srgb(), true)], true);
        self.looks
            .get_or_insert_with(|| FlatLooks::new(device, queue))
            .warm(device, queue, looks, size)
    }

    pub fn overlay_pipelines_made(&self) -> usize {
        self.flat.as_ref().map_or(0, FlatPass::pipelines_made)
            + self.looks.as_ref().map_or(0, FlatLooks::pipelines_made)
    }

    pub fn source_target(
        &mut self,
        device: &wgpu::Device,
        size: [u32; 2],
        format: wgpu::TextureFormat,
    ) -> Result<wgpu::TextureView, String> {
        if size[0] == 0 || size[1] == 0 {
            return Err("the render size must be at least 1 by 1".into());
        }
        let fresh = self
            .source
            .as_ref()
            .is_some_and(|source| source.size == size && source.format == format);
        if !fresh {
            let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC;
            if format == wgpu::TextureFormat::Rgba16Float {
                usage |= wgpu::TextureUsages::STORAGE_BINDING;
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("upscale source"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.source = Some(Source {
                texture,
                view,
                size,
                format,
            });
        }
        self.source
            .as_ref()
            .map(|source| source.view.clone())
            .ok_or_else(|| "the render target is missing".to_owned())
    }

    pub fn source_texture(&self) -> Option<&wgpu::Texture> {
        self.source.as_ref().map(|source| &source.texture)
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> usize {
        if let Some(index) = self.pipelines.iter().position(|(made, _)| *made == format) {
            return index;
        }
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("upscale"),
            layout: Some(&self.pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("upscale_vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("upscale_fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
            cache: None,
        });
        self.pipelines.push((format, pipeline));
        self.pipelines.len() - 1
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        compose: &Compose<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<(), String> {
        check(compose)?;
        let index = self.pipeline(device, compose.output_format);
        let content = compose.content;
        let uniform = Uniform {
            content: [content.x, content.y, content.width, content.height],
            source: [compose.source_size[0] as f32, compose.source_size[1] as f32],
            mode: compose.filter as u32,
            transfer: Transfer::between(compose.source_format, compose.output_format) as u32,
            bars: bar_value(compose.bars, compose.output_format),
            backdrop: u32::from(compose.backdrop.is_some()),
            pad: [0; 3],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        if self
            .group
            .as_ref()
            .is_none_or(|(source, _)| source != compose.source)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("upscale"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(compose.source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.uniform.as_entire_binding(),
                    },
                ],
            });
            self.group = Some((compose.source.clone(), group));
        }
        if let Some(backdrop) = compose.backdrop {
            let flat = self
                .backdrop
                .get_or_insert_with(|| FlatPass::new(device, queue));
            flat.prepare(device, queue, backdrop, compose.output_size)?;
            let target = flat.colour_target(device, compose.output_size);
            let [r, g, b, a] = compose.bars;
            flat.encode_colour(
                encoder,
                &target,
                Some([r * a, g * a, b * a, a]),
                profiler.as_deref_mut(),
            );
            flat.encode_finish(
                device,
                encoder,
                &target,
                compose.output,
                compose.output_format,
                compose.output_format.is_srgb(),
                false,
                profiler.as_deref_mut(),
            );
        }
        let timing = profiler
            .as_deref_mut()
            .and_then(|profiler| profiler.pass("upscale"));
        let writes = profiler
            .as_deref()
            .and_then(|profiler| profiler.render_writes(timing));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("upscale"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: compose.output,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: if compose.backdrop.is_some() {
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
            pass.set_pipeline(&self.pipelines[index].1);
            pass.set_bind_group(0, self.group.as_ref().map(|(_, group)| group), &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }

    pub fn encode_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        overlay: &Overlay<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<(), String> {
        if overlay.output_size[0] == 0 || overlay.output_size[1] == 0 {
            return Err("the overlay needs an output of at least 1 by 1".into());
        }
        let flat = self
            .flat
            .get_or_insert_with(|| FlatPass::new(device, queue));
        let look = overlay.look.filter(|look| !look.is_identity());
        if look.is_none() {
            flat.prepare(device, queue, overlay.scene, overlay.output_size)?;
        }
        let Some(clip) = clip(overlay.content, overlay.output_size) else {
            return Ok(());
        };
        flat.set_clip(Some(clip));
        let target = flat.colour_target(device, overlay.output_size);
        if let Some(look) = look {
            self.looks
                .get_or_insert_with(|| FlatLooks::new(device, queue))
                .encode(
                    device,
                    queue,
                    encoder,
                    flat,
                    overlay.scene,
                    look,
                    overlay.output_size,
                    &target,
                    profiler.as_deref_mut(),
                )?;
        } else {
            flat.encode_colour(encoder, &target, Some([0.0; 4]), profiler.as_deref_mut());
        }
        flat.encode_finish(
            device,
            encoder,
            &target,
            overlay.output,
            overlay.output_format,
            overlay.output_format.is_srgb(),
            true,
            profiler,
        );
        Ok(())
    }
}

pub fn clip(content: Rect, output_size: [u32; 2]) -> Option<[u32; 4]> {
    let edge = |from: f32, limit: u32| ((from - 0.5).ceil().max(0.0) as u32).min(limit);
    let x0 = edge(content.x, output_size[0]);
    let y0 = edge(content.y, output_size[1]);
    let x1 = edge(content.x + content.width, output_size[0]);
    let y1 = edge(content.y + content.height, output_size[1]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

fn check(compose: &Compose<'_>) -> Result<(), String> {
    let [sw, sh] = compose.source_size;
    let [ow, oh] = compose.output_size;
    if sw == 0 || sh == 0 || ow == 0 || oh == 0 {
        return Err("the upscale needs a source and an output of at least 1 by 1".into());
    }
    let c = compose.content;
    let finite = [c.x, c.y, c.width, c.height].iter().all(|v| v.is_finite());
    if !finite || c.width <= 0.0 || c.height <= 0.0 {
        return Err("the content rect must be finite and not empty".into());
    }
    if compose
        .bars
        .iter()
        .any(|channel| !(0.0..=1.0).contains(channel))
    {
        return Err("the bar colour's channels must be from 0 to 1".into());
    }
    Ok(())
}

pub fn bar_value(bars: [f32; 4], format: wgpu::TextureFormat) -> [f32; 4] {
    let [r, g, b, a] = bars;
    let channel = |c: f32| if format.is_srgb() { srgb_channel(c) } else { c } * a;
    [channel(r), channel(g), channel(b), a]
}

fn weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

pub fn cpu(
    source: &[[f32; 4]],
    source_size: [u32; 2],
    output_size: [u32; 2],
    content: Rect,
    bars: [f32; 4],
    filter: Filter,
) -> Vec<[f32; 4]> {
    let [sw, sh] = source_size.map(|v| v as i32);
    let texel = |x: i32, y: i32| source[(y.clamp(0, sh - 1) * sw + x.clamp(0, sw - 1)) as usize];
    let mut out = Vec::with_capacity((output_size[0] * output_size[1]) as usize);
    for y in 0..output_size[1] {
        for x in 0..output_size[0] {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            if p[0] < content.x
                || p[1] < content.y
                || p[0] >= content.x + content.width
                || p[1] >= content.y + content.height
            {
                out.push(bars);
                continue;
            }
            let u = (p[0] - content.x) / content.width * sw as f32 - 0.5;
            let v = (p[1] - content.y) / content.height * sh as f32 - 0.5;
            if filter == Filter::Nearest {
                out.push(texel((u + 0.5).floor() as i32, (v + 0.5).floor() as i32));
                continue;
            }
            let (bx, by) = (u.floor(), v.floor());
            let (wx, wy) = (weights(u - bx), weights(v - by));
            let (bx, by) = (bx as i32, by as i32);
            let mut sum = [0.0f32; 4];
            for (j, wy) in wy.iter().enumerate() {
                let mut row = [0.0f32; 4];
                for (i, wx) in wx.iter().enumerate() {
                    let t = texel(bx + i as i32 - 1, by + j as i32 - 1);
                    for k in 0..4 {
                        row[k] += t[k] * wx;
                    }
                }
                for k in 0..4 {
                    sum[k] += row[k] * wy;
                }
            }
            let corners = [
                texel(bx, by),
                texel(bx + 1, by),
                texel(bx, by + 1),
                texel(bx + 1, by + 1),
            ];
            out.push(std::array::from_fn(|k| {
                let lo = corners.iter().map(|t| t[k]).fold(f32::INFINITY, f32::min);
                let hi = corners
                    .iter()
                    .map(|t| t[k])
                    .fold(f32::NEG_INFINITY, f32::max);
                sum[k].clamp(lo, hi)
            }));
        }
    }
    out
}

#[cfg(test)]
mod tests;
