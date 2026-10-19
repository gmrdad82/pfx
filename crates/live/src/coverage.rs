use crate::frame::CachedGroup;
use pfx_gpu::{GpuProfiler, wgpu};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

const UPDATE_WGSL: &str = r#"
@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var motion_tex: texture_2d<f32>;
@group(0) @binding(2) var history_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> info: vec4f;

fn covered(p: vec2i, size: vec2i) -> f32 {
    return select(0.0, 1.0, textureLoad(depth_tex, clamp(p, vec2i(0), size - vec2i(1)), 0) > 0.0);
}

@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2i(info.yz);
    let p = vec2i(position.xy);
    let now = covered(p, size);
    var lo = now;
    var hi = now;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let c = covered(p + vec2i(x, y), size);
            lo = min(lo, c);
            hi = max(hi, c);
        }
    }
    let prev = position.xy / info.yz + textureLoad(motion_tex, p, 0).rg;
    var alpha = now;
    if (info.x > 0.0 && all(prev >= vec2f(0.0)) && all(prev < vec2f(1.0))) {
        let past = textureLoad(history_tex, vec2i(prev * info.yz), 0).r;
        alpha = mix(now, clamp(past, lo, hi), info.x);
    }
    return vec4f(alpha, 0.0, 0.0, 1.0);
}
"#;

const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var coverage_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> info: vec4f;

fn encode(c: vec3f) -> vec3f {
    return select(1.055 * pow(c, vec3f(1.0 / 2.4)) - vec3f(0.055), c * 12.92, c <= vec3f(0.0031308));
}

@fragment
fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2i(info.yz);
    let p = vec2i(position.xy);
    let alpha = clamp(textureLoad(coverage_tex, p, 0).r, 0.0, 1.0);
    var colour = vec3f(0.0);
    if (textureLoad(depth_tex, p, 0) > 0.0) {
        colour = textureLoad(source, p, 0).rgb;
    } else {
        var sum = vec3f(0.0);
        var count = 0.0;
        for (var y = -1; y <= 1; y += 1) {
            for (var x = -1; x <= 1; x += 1) {
                let q = clamp(p + vec2i(x, y), vec2i(0), size - vec2i(1));
                if (textureLoad(depth_tex, q, 0) > 0.0) {
                    sum += textureLoad(source, q, 0).rgb;
                    count += 1.0;
                }
            }
        }
        if (count > 0.0) {
            colour = sum / count;
        }
    }
    colour = clamp(colour, vec3f(0.0), vec3f(1.0));
    ENCODE
    return vec4f(colour, alpha);
}
"#;

pub(crate) struct Coverage {
    history: [(wgpu::Texture, wgpu::TextureView); 2],
    index: usize,
    valid: bool,
    restarts: u64,
    info: wgpu::Buffer,
    update_layout: wgpu::BindGroupLayout,
    update: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    blit: wgpu::RenderPipeline,
    update_groups: [CachedGroup; 2],
    blit_groups: [CachedGroup; 2],
}

fn history(device: &wgpu::Device, size: [u32; 2]) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("live coverage history"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn float(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn depth(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(16),
        },
        count: None,
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &str,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{source}", crate::renderer::FULLSCREEN).into(),
        ),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn blit_source(encode: bool) -> String {
    BLIT_WGSL.replace(
        "ENCODE",
        if encode {
            "colour = encode(colour);"
        } else {
            ""
        },
    )
}

impl Coverage {
    pub(crate) fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        output: wgpu::TextureFormat,
        encode: bool,
    ) -> Self {
        let update_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live coverage"),
            entries: &[depth(0), float(1), float(2), uniform(3)],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live coverage blit"),
            entries: &[float(0), depth(1), float(2), uniform(3)],
        });
        let update = pipeline(device, &update_layout, UPDATE_WGSL, FORMAT, "live coverage");
        let blit = pipeline(
            device,
            &blit_layout,
            &blit_source(encode),
            output,
            "live coverage blit",
        );
        Self {
            history: [history(device, size), history(device, size)],
            index: 0,
            valid: false,
            restarts: 0,
            info: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("live coverage info"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            update_layout,
            update,
            blit_layout,
            blit,
            update_groups: Default::default(),
            blit_groups: Default::default(),
        }
    }

    pub(crate) fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        self.history = [history(device, size), history(device, size)];
        self.update_groups = Default::default();
        self.blit_groups = Default::default();
        self.valid = false;
    }

    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
    }

    pub(crate) fn textures(&self) -> [wgpu::Texture; 2] {
        [self.history[0].0.clone(), self.history[1].0.clone()]
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        viewport: [u32; 2],
        restarts: u64,
        weight: f32,
        profiler: &mut GpuProfiler,
    ) {
        if restarts != self.restarts {
            self.restarts = restarts;
            self.valid = false;
        }
        let weight = if self.valid { weight } else { 0.0 };
        queue.write_buffer(
            &self.info,
            0,
            bytemuck::cast_slice(&[weight, viewport[0] as f32, viewport[1] as f32, 0.0]),
        );
        let next = 1 - self.index;
        let group = self.update_groups[next].get(
            device,
            "live coverage",
            &self.update_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(motion),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.history[self.index].1),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.info.as_entire_binding(),
                },
            ],
        );
        let timing = profiler.pass("coverage");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("live coverage"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.history[next].1,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: profiler.render_writes(timing),
            occlusion_query_set: None,
        });
        crate::viewport::apply(&mut pass, viewport);
        pass.set_pipeline(&self.update);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        self.index = next;
        self.valid = true;
    }

    pub(crate) fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.blit
    }

    pub(crate) fn blit_group(
        &mut self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> &wgpu::BindGroup {
        let index = self.index;
        self.blit_groups[index].get(
            device,
            "live coverage blit",
            &self.blit_layout,
            &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.history[index].1),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.info.as_entire_binding(),
                },
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coverage_shaders_parse() {
        for source in [
            format!("{}\n{UPDATE_WGSL}", crate::renderer::FULLSCREEN),
            format!("{}\n{}", crate::renderer::FULLSCREEN, blit_source(false)),
            format!("{}\n{}", crate::renderer::FULLSCREEN, blit_source(true)),
        ] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
