use pfx_post::quantise::{Dither, NOISE, NOISE_SIZE};
use wgpu::util::DeviceExt;

pub fn wgsl(first: u32) -> String {
    format!(
        "@group(0) @binding({first}) var dither_noise: texture_2d<f32>;\n@group(0) @binding({}) var<uniform> dither_on: vec4u;\n{CODE}",
        first + 1
    )
}

const CODE: &str = "
fn srgb_encode(l: vec3f) -> vec3f { return select(1.055 * pow(l, vec3f(1.0 / 2.4)) - vec3f(0.055), l * 12.92, l <= vec3f(0.0031308)); }
fn srgb_decode(e: vec3f) -> vec3f { return select(pow((e + vec3f(0.055)) / 1.055, vec3f(2.4)), e / 12.92, e <= vec3f(0.04045)); }
fn dithered(e: vec3f, position: vec2f) -> vec3f {
    let at = (vec2u(position) + dither_on.yz) % 64u;
    let n = (textureLoad(dither_noise, vec2i(at), 0).r * 255.0 + 0.5) / 128.0 - 1.0;
    let t = sign(n) * (1.0 - sqrt(1.0 - abs(n)));
    let inside = e > vec3f(0.0) & e < vec3f(1.0);
    return select(e, clamp(e + vec3f(t / 255.0), vec3f(0.0), vec3f(1.0)), inside & vec3(dither_on.x != 0u));
}
";

pub struct OutputDither {
    noise: wgpu::TextureView,
    uniform: wgpu::Buffer,
    pub dither: Option<Dither>,
}

impl OutputDither {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> OutputDither {
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("live output dither noise"),
                size: wgpu::Extent3d {
                    width: NOISE_SIZE,
                    height: NOISE_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            NOISE,
        );
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live output dither"),
            contents: bytemuck::cast_slice(&words(None)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        OutputDither {
            noise: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            uniform,
            dither: None,
        }
    }

    pub fn set(&mut self, queue: &wgpu::Queue, dither: Option<Dither>) {
        self.dither = dither;
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&words(dither)));
    }

    pub fn layout_entries(first: u32) -> [wgpu::BindGroupLayoutEntry; 2] {
        [
            wgpu::BindGroupLayoutEntry {
                binding: first,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: first + 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    }

    pub fn entries(&self, first: u32) -> [wgpu::BindGroupEntry<'_>; 2] {
        [
            wgpu::BindGroupEntry {
                binding: first,
                resource: wgpu::BindingResource::TextureView(&self.noise),
            },
            wgpu::BindGroupEntry {
                binding: first + 1,
                resource: self.uniform.as_entire_binding(),
            },
        ]
    }
}

fn words(dither: Option<Dither>) -> [u32; 4] {
    match dither {
        Some(d) => [1, d.offset[0], d.offset[1], 0],
        None => [0; 4],
    }
}
