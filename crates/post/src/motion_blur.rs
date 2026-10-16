use wgpu::util::DeviceExt;

pub const SHUTTER: f32 = 0.5;
pub const MAX_PX: f32 = 64.0;
pub const SAMPLES: u32 = 8;
pub const MAX_PX_RANGE: std::ops::RangeInclusive<f32> = 1.0..=256.0;
pub const SAMPLES_RANGE: std::ops::RangeInclusive<u32> = 2..=64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlur {
    pub shutter: f32,
    pub max_px: f32,
    pub samples: u32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        Self {
            shutter: SHUTTER,
            max_px: MAX_PX,
            samples: SAMPLES,
        }
    }
}

impl MotionBlur {
    pub fn active(self) -> bool {
        self.shutter > 0.0 && self.max_px > 0.0 && self.samples > 0
    }

    pub fn max_blur(self) -> u32 {
        self.max_px
            .clamp(*MAX_PX_RANGE.start(), *MAX_PX_RANGE.end())
            .floor() as u32
    }

    pub fn tile(self) -> u32 {
        (self.max_blur() - 1) / 8 * 8 + 8
    }

    pub fn loop_count(self) -> f32 {
        (self
            .samples
            .clamp(*SAMPLES_RANGE.start(), *SAMPLES_RANGE.end())
            / 2) as f32
    }

    pub fn velocity_scale(self, spans_shutter: bool) -> f32 {
        if spans_shutter { 1.0 } else { self.shutter }
    }
}

const COMMON: &str = r#"
struct Params {
    size: vec2u, tiles: vec2u,
    blur: vec4f,
    extra: vec4f,
}
@group(0) @binding(0) var<uniform> params: Params;
fn vmax(v1: vec2f, v2: vec2f) -> vec2f {
    return select(v1, v2, dot(v1, v1) < dot(v2, v2));
}
"#;

const SETUP_WGSL: &str = r#"
@group(0) @binding(1) var motion: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var packed: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= params.size)) { return; }
    let pixel = vec2i(id.xy);
    var v = textureLoad(motion, pixel, 0).xy;
    v *= (params.blur.x * 0.5) * vec2f(params.size);
    v /= max(1.0, length(v) * params.blur.z);
    let d = textureLoad(depth, pixel, 0).x;
    textureStore(packed, pixel, vec4f(v, d, 0.0));
}
"#;

const TILE_MAX_WGSL: &str = r#"
@group(0) @binding(1) var packed: texture_2d<f32>;
@group(0) @binding(2) var tiles: texture_storage_2d<rgba16float, write>;
var<workgroup> longest: array<vec2f, 64>;
@compute @workgroup_size(8, 8)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_id) local: vec3u, @builtin(local_invocation_index) index: u32) {
    let tile = u32(params.extra.x);
    let span = tile / 8u;
    let origin = group.xy * tile + local.xy;
    var vo = vec2f(0.0);
    for (var iy = 0u; iy < span; iy++) {
        for (var ix = 0u; ix < span; ix++) {
            let at = origin + vec2u(ix, iy) * 8u;
            if (all(at < params.size)) {
                vo = vmax(vo, textureLoad(packed, vec2i(at), 0).xy);
            }
        }
    }
    longest[index] = vo;
    workgroupBarrier();
    for (var reach = 32u; reach > 0u; reach = reach / 2u) {
        if (index < reach) {
            longest[index] = vmax(longest[index], longest[index + reach]);
        }
        workgroupBarrier();
    }
    if (index == 0u && all(group.xy < params.tiles)) {
        textureStore(tiles, vec2i(group.xy), vec4f(longest[0], 0.0, 0.0));
    }
}
"#;

const NEIGHBOR_MAX_WGSL: &str = r#"
@group(0) @binding(1) var tiles: texture_2d<f32>;
@group(0) @binding(2) var neighbors: texture_storage_2d<rgba16float, write>;
fn tile_at(at: vec2i) -> vec2f {
    return textureLoad(tiles, clamp(at, vec2i(0), vec2i(params.tiles) - vec2i(1)), 0).xy;
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= params.tiles)) { return; }
    let cw = 1.01;
    let at = vec2i(id.xy);
    let v1 = tile_at(at + vec2i(-1, -1));
    let v2 = tile_at(at + vec2i(0, -1));
    let v3 = tile_at(at + vec2i(1, -1));
    let v4 = tile_at(at + vec2i(-1, 0));
    let v5 = tile_at(at) * cw;
    let v6 = tile_at(at + vec2i(1, 0));
    let v7 = tile_at(at + vec2i(-1, 1));
    let v8 = tile_at(at + vec2i(0, 1));
    let v9 = tile_at(at + vec2i(1, 1));
    let va = vmax(v1, vmax(v2, v3));
    let vb = vmax(v4, vmax(v5, v6));
    let vc = vmax(v7, vmax(v8, v9));
    textureStore(neighbors, at, vec4f(vmax(va, vmax(vb, vc)) / cw, 0.0, 0.0));
}
"#;

const RECONSTRUCT_WGSL: &str = r#"
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var packed: texture_2d<f32>;
@group(0) @binding(3) var neighbors: texture_2d<f32>;
@group(0) @binding(4) var linear_clamp: sampler;
@group(0) @binding(5) var output_tex: texture_storage_2d<rgba16float, write>;
fn interval(phase: f32, period: f32) -> bool {
    return fract(phase / period) > 0.499;
}
fn gradient_noise(at: vec2f) -> f32 {
    let uv = floor(at + params.extra.y * vec2f(params.size));
    let f = dot(vec2f(0.06711056, 0.00583715), uv);
    return fract(52.9829189 * fract(f));
}
fn jitter_tile(at: vec2f) -> vec2f {
    let angle = gradient_noise(at + vec2f(2.0 * f32(params.size.x), 0.0)) * 6.2831853;
    return vec2f(cos(angle), sin(angle)) / 4.0;
}
fn screen_uv(at: vec2f, dims: vec2u) -> vec2f {
    return clamp(at, vec2f(0.5), vec2f(params.size) - vec2f(0.5)) / vec2f(dims);
}
fn sample_velocity(at: vec2f) -> vec3f {
    return textureSampleLevel(packed, linear_clamp, screen_uv(at, textureDimensions(packed)), 0.0).xyz;
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= params.size)) { return; }
    let pixel = vec2i(id.xy);
    let centre = vec2f(pixel) + vec2f(0.5);
    let c_p = textureLoad(source, pixel, 0);
    let vd_p = textureLoad(packed, pixel, 0).xyz;
    let l_v_p = max(length(vd_p.xy), 0.5);
    let rcp_d_p = 1.0 / vd_p.z;
    let tile_pos = clamp(centre / params.extra.x + jitter_tile(centre), vec2f(0.5), vec2f(params.tiles) - vec2f(0.5));
    let v_max = textureSampleLevel(neighbors, linear_clamp, tile_pos / vec2f(textureDimensions(neighbors)), 0.0).xy;
    let l_v_max = length(v_max);
    if (l_v_max < 2.0) {
        textureStore(output_tex, pixel, c_p);
        return;
    }
    let v_alt = select(v_max, vd_p.xy * (l_v_max / l_v_p), l_v_p * 2.0 > l_v_max);
    let sc = floor(min(params.blur.w, l_v_max / 2.0));
    let dt = 1.0 / sc;
    let t_offs = (gradient_noise(centre) - 0.5) * dt;
    var t = 1.0 - dt / 2.0;
    var count = 0.0;
    var l_v_bg = max(l_v_p, 1.0);
    var acc = vec4f(0.0);
    loop {
        if (!(t > dt / 4.0)) { break; }
        let v_s = select(v_max, v_alt, interval(count, 4.0));
        let t_s = select(t, -t, interval(count, 2.0)) + t_offs;
        let l_t = l_v_max * abs(t_s);
        let at = centre + v_s * t_s;
        let c = textureSampleLevel(source, linear_clamp, screen_uv(at, textureDimensions(source)), 0.0).rgb;
        let vd = sample_velocity(at);
        let fg = saturate((vd_p.z - vd.z) * 20.0 * rcp_d_p);
        let l_v = mix(l_v_bg, length(vd.xy), fg);
        let w = saturate(l_v - l_t) / l_v * (1.2 - t);
        acc += vec4f(c, 1.0) * w;
        l_v_bg = max(l_v_bg, l_v);
        t = select(t, t - dt, interval(count, 2.0));
        count += 1.0;
    }
    acc += vec4f(c_p.rgb, 1.0) * (1.2 / (l_v_bg * sc * 2.0));
    textureStore(output_tex, pixel, vec4f(acc.rgb / acc.a, c_p.a));
}
"#;

fn shader(part: &str) -> String {
    format!("{COMMON}{part}")
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

fn compute(
    device: &wgpu::Device,
    label: &'static str,
    source: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(shader(source).into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (layout, pipeline)
}

fn texture(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

struct Tiles {
    size: u32,
    _tiles: wgpu::Texture,
    tiles: wgpu::TextureView,
    _neighbors: wgpu::Texture,
    neighbors: wgpu::TextureView,
}

impl Tiles {
    fn new(device: &wgpu::Device, width: u32, height: u32, size: u32) -> Self {
        let (tiles_x, tiles_y) = (width.div_ceil(size), height.div_ceil(size));
        let (tiles_texture, tiles) = texture(device, "motion blur tile max", tiles_x, tiles_y);
        let (neighbors_texture, neighbors) =
            texture(device, "motion blur neighbor max", tiles_x, tiles_y);
        Self {
            size,
            _tiles: tiles_texture,
            tiles,
            _neighbors: neighbors_texture,
            neighbors,
        }
    }
}

pub struct GpuMotionBlur {
    device: wgpu::Device,
    width: u32,
    height: u32,
    viewport: (u32, u32),
    _packed: wgpu::Texture,
    packed: wgpu::TextureView,
    _output: wgpu::Texture,
    output: wgpu::TextureView,
    tiles: Option<Tiles>,
    sampler: wgpu::Sampler,
    setup_layout: wgpu::BindGroupLayout,
    setup_pipeline: wgpu::ComputePipeline,
    tile_layout: wgpu::BindGroupLayout,
    tile_pipeline: wgpu::ComputePipeline,
    neighbor_layout: wgpu::BindGroupLayout,
    neighbor_pipeline: wgpu::ComputePipeline,
    reconstruct_layout: wgpu::BindGroupLayout,
    reconstruct_pipeline: wgpu::ComputePipeline,
}

impl GpuMotionBlur {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (setup_layout, setup_pipeline) = compute(
            device,
            "motion blur velocity setup",
            SETUP_WGSL,
            &[
                uniform_entry(),
                texture_entry(1, false),
                texture_entry(2, false),
                storage_entry(3),
            ],
        );
        let (tile_layout, tile_pipeline) = compute(
            device,
            "motion blur tile max",
            TILE_MAX_WGSL,
            &[uniform_entry(), texture_entry(1, false), storage_entry(2)],
        );
        let (neighbor_layout, neighbor_pipeline) = compute(
            device,
            "motion blur neighbor max",
            NEIGHBOR_MAX_WGSL,
            &[uniform_entry(), texture_entry(1, false), storage_entry(2)],
        );
        let (reconstruct_layout, reconstruct_pipeline) = compute(
            device,
            "motion blur reconstruction",
            RECONSTRUCT_WGSL,
            &[
                uniform_entry(),
                texture_entry(1, true),
                texture_entry(2, true),
                texture_entry(3, true),
                sampler_entry(4),
                storage_entry(5),
            ],
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("motion blur linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (packed_texture, packed) = texture(device, "motion blur velocity", width, height);
        let (output_texture, output) = texture(device, "motion blur target", width, height);
        Self {
            device: device.clone(),
            width,
            height,
            viewport: (width, height),
            _packed: packed_texture,
            packed,
            _output: output_texture,
            output,
            tiles: None,
            sampler,
            setup_layout,
            setup_pipeline,
            tile_layout,
            tile_pipeline,
            neighbor_layout,
            neighbor_pipeline,
            reconstruct_layout,
            reconstruct_pipeline,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != (self.width, self.height) {
            (self._packed, self.packed) =
                texture(&self.device, "motion blur velocity", width, height);
            (self._output, self.output) =
                texture(&self.device, "motion blur target", width, height);
            self.tiles = None;
            self.width = width;
            self.height = height;
        }
        self.viewport = (width, height);
    }

    pub fn capacity(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.viewport = crate::gpu::fit_viewport((width, height), (self.width, self.height))?;
        Ok(())
    }

    pub fn output(&self) -> &wgpu::TextureView {
        &self.output
    }

    fn bind(
        &self,
        label: &'static str,
        layout: &wgpu::BindGroupLayout,
        resources: &[wgpu::BindingResource<'_>],
    ) -> wgpu::BindGroup {
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: resource.clone(),
            })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &entries,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render<'a>(
        &'a mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        blur: MotionBlur,
        spans_shutter: bool,
        time: f32,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> &'a wgpu::TextureView {
        let tile = blur.tile();
        if self.tiles.as_ref().is_none_or(|tiles| tiles.size != tile) {
            self.tiles = Some(Tiles::new(&self.device, self.width, self.height, tile));
        }
        let (width, rows) = self.viewport;
        let (tiles_x, tiles_y) = (width.div_ceil(tile), rows.div_ceil(tile));
        let max_blur = blur.max_blur() as f32;
        let params: [f32; 12] = [
            f32::from_bits(width),
            f32::from_bits(rows),
            f32::from_bits(tiles_x),
            f32::from_bits(tiles_y),
            blur.velocity_scale(spans_shutter),
            max_blur,
            1.0 / max_blur,
            blur.loop_count(),
            tile as f32,
            time.rem_euclid(64.0),
            0.0,
            0.0,
        ];
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("motion blur settings"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let tiles = self.tiles.as_ref().expect("tiles were made above");
        let setup = self.bind(
            "motion blur setup bindings",
            &self.setup_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(motion),
                wgpu::BindingResource::TextureView(depth),
                wgpu::BindingResource::TextureView(&self.packed),
            ],
        );
        let tile_max = self.bind(
            "motion blur tile max bindings",
            &self.tile_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(&self.packed),
                wgpu::BindingResource::TextureView(&tiles.tiles),
            ],
        );
        let neighbor_max = self.bind(
            "motion blur neighbor max bindings",
            &self.neighbor_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(&tiles.tiles),
                wgpu::BindingResource::TextureView(&tiles.neighbors),
            ],
        );
        let reconstruct = self.bind(
            "motion blur reconstruction bindings",
            &self.reconstruct_layout,
            &[
                uniform.as_entire_binding(),
                wgpu::BindingResource::TextureView(source),
                wgpu::BindingResource::TextureView(&self.packed),
                wgpu::BindingResource::TextureView(&tiles.neighbors),
                wgpu::BindingResource::Sampler(&self.sampler),
                wgpu::BindingResource::TextureView(&self.output),
            ],
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("motion blur"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.setup_pipeline);
        pass.set_bind_group(0, &setup, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), rows.div_ceil(8), 1);
        pass.set_pipeline(&self.tile_pipeline);
        pass.set_bind_group(0, &tile_max, &[]);
        pass.dispatch_workgroups(tiles_x, tiles_y, 1);
        pass.set_pipeline(&self.neighbor_pipeline);
        pass.set_bind_group(0, &neighbor_max, &[]);
        pass.dispatch_workgroups(tiles_x.div_ceil(8), tiles_y.div_ceil(8), 1);
        pass.set_pipeline(&self.reconstruct_pipeline);
        pass.set_bind_group(0, &reconstruct, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), rows.div_ceil(8), 1);
        drop(pass);
        &self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::tests::half;

    #[test]
    fn tiles_cover_the_blur_radius_in_multiples_of_eight() {
        for (max_px, tile) in [(1.0, 8), (8.0, 8), (9.0, 16), (64.0, 64), (65.5, 72)] {
            let blur = MotionBlur {
                max_px,
                ..MotionBlur::default()
            };
            assert_eq!(blur.tile(), tile, "{max_px}");
            assert!(blur.tile() as f32 >= blur.max_blur() as f32);
        }
    }

    #[test]
    fn velocity_already_spanning_the_shutter_is_not_scaled_again() {
        let blur = MotionBlur {
            shutter: 0.25,
            ..MotionBlur::default()
        };
        assert_eq!(blur.velocity_scale(false), 0.25);
        assert_eq!(blur.velocity_scale(true), 1.0);
        assert!(
            !MotionBlur {
                shutter: 0.0,
                ..blur
            }
            .active()
        );
    }

    #[test]
    fn shaders_parse() {
        for part in [
            SETUP_WGSL,
            TILE_MAX_WGSL,
            NEIGHBOR_MAX_WGSL,
            RECONSTRUCT_WGSL,
        ] {
            naga::front::wgsl::parse_str(&shader(part)).unwrap();
        }
    }

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    const SIZE: (u32, u32) = (192, 96);

    fn upload(
        gpu: &pfx_gpu::Gpu,
        pixel: impl Fn(u32, u32) -> [f32; 4],
    ) -> pfx_gpu::OffscreenTarget {
        let target = gpu.offscreen(SIZE.0, SIZE.1, FORMAT).unwrap();
        let data: Vec<u16> = (0..SIZE.1)
            .flat_map(|y| (0..SIZE.0).map(move |x| (x, y)))
            .flat_map(|(x, y)| pixel(x, y).map(half))
            .collect();
        gpu.upload_rgba16(&target, &data).unwrap();
        target
    }

    fn red(gpu: &pfx_gpu::Gpu, blur: &mut GpuMotionBlur, speed: f32) -> Vec<f32> {
        let square = |x: u32, y: u32| (40..56).contains(&x) && (40..56).contains(&y);
        let still = |x: u32, y: u32| (130..146).contains(&x) && (40..56).contains(&y);
        let source = upload(gpu, |x, y| {
            if square(x, y) || still(x, y) {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        });
        let motion = upload(gpu, |x, y| {
            if square(x, y) {
                [-speed / SIZE.0 as f32, 0.0, 0.0, 0.0]
            } else {
                [0.0; 4]
            }
        });
        let depth = upload(gpu, |x, y| {
            if square(x, y) || still(x, y) {
                [2.0, 0.0, 0.0, 0.0]
            } else {
                [20.0, 0.0, 0.0, 0.0]
            }
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("motion blur test"),
            });
        blur.render(
            &mut encoder,
            &source.view,
            &motion.view,
            &depth.view,
            MotionBlur {
                shutter: 1.0,
                max_px: 32.0,
                samples: 16,
            },
            false,
            0.0,
            None,
        );
        gpu.queue.submit(Some(encoder.finish()));
        let out = gpu
            .readback_rgba16(&pfx_gpu::OffscreenTarget {
                texture: blur._output.clone(),
                view: blur.output.clone(),
                format: FORMAT,
                width: SIZE.0,
                height: SIZE.1,
            })
            .unwrap();
        out.chunks(4)
            .map(|texel| half::f16::from_bits(texel[0]).to_f32())
            .collect()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_moving_square_streaks_along_its_motion_and_a_still_one_stays_sharp() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut blur = GpuMotionBlur::new(&gpu.device, SIZE.0, SIZE.1);
        let still = red(&gpu, &mut blur, 0.0);
        let moving = red(&gpu, &mut blur, 24.0);
        let row = |image: &[f32], x: u32| image[(48 * SIZE.0 + x) as usize];
        for x in 0..SIZE.0 {
            let sharp = if (40..56).contains(&x) || (130..146).contains(&x) {
                1.0
            } else {
                0.0
            };
            assert_eq!(row(&still, x), sharp, "a still frame changed at {x}");
        }
        for x in 120..156 {
            assert_eq!(
                row(&moving, x),
                row(&still, x),
                "the still square blurred at {x}"
            );
        }
        for x in [34, 36, 58, 60] {
            assert!(
                row(&moving, x) > 0.05,
                "no streak at {x}: {}",
                row(&moving, x)
            );
        }
        assert!(row(&moving, 48) < 0.999, "the moving square kept its core");
        for y in [30, 66] {
            assert_eq!(
                moving[(y * SIZE.0 + 48) as usize],
                0.0,
                "the streak leaked across its motion at row {y}"
            );
        }
    }
}
