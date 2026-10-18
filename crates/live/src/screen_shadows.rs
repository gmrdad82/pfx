use bytemuck::{Pod, Zeroable};
use pfx_gpu::GpuProfiler;

use crate::frame::{Matrix, jitter_matrix, multiply, transform};
use crate::shadow::{CASCADE_COUNT, DEFAULT_RESOLUTION, Quality};
use pfx_load::scene::{ShadowQuality, Shadows as Look};

pub const BINDING: u32 = 28;
pub const DEPTH_MIPS: u32 = 5;
pub const RADIUS: f32 = 0.5;
pub const RADIUS_MULTIPLIER: f32 = 1.457;
pub const FALLOFF_RANGE: f32 = 0.615;
pub const SAMPLE_DISTRIBUTION_POWER: f32 = 2.0;
pub const THIN_OCCLUDER_COMPENSATION: f32 = 0.0;
pub const FINAL_VALUE_POWER: f32 = 2.2;
pub const DEPTH_MIP_SAMPLING_OFFSET: f32 = 3.3;
pub const DENOISE_BLUR_BETA: f32 = 1.2;
pub const CONTACT_LENGTH: f32 = 0.3;
pub const CONTACT_STEPS: u32 = 8;
pub const CONTACT_STEP_LIMIT: u32 = 64;

const OCCLUSION: u32 = 1;
const CONTACT: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Low,
    Medium,
    High,
    Ultra,
}

impl Level {
    pub fn slices_and_steps(self) -> (u32, u32) {
        match self {
            Self::Low => (1, 2),
            Self::Medium => (2, 2),
            Self::High => (3, 3),
            Self::Ultra => (9, 3),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occlusion {
    pub level: Level,
    pub radius: f32,
}

impl Default for Occlusion {
    fn default() -> Self {
        Self {
            level: Level::High,
            radius: RADIUS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactRay {
    pub length: f32,
    pub steps: u32,
}

impl Default for ContactRay {
    fn default() -> Self {
        Self {
            length: CONTACT_LENGTH,
            steps: CONTACT_STEPS,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenShadows {
    pub occlusion: Option<Occlusion>,
    pub contact: Option<ContactRay>,
}

impl ScreenShadows {
    pub const OFF: Self = Self {
        occlusion: None,
        contact: None,
    };

    pub fn on(&self) -> bool {
        self.occlusion.is_some() || self.contact.is_some()
    }

    pub fn check(&self) -> Result<(), String> {
        if let Some(occlusion) = self.occlusion
            && !(occlusion.radius.is_finite() && occlusion.radius > 0.0)
        {
            return Err(format!(
                "ambient occlusion needs a finite radius above 0, found {}",
                occlusion.radius
            ));
        }
        if let Some(contact) = self.contact {
            if !(contact.length.is_finite() && contact.length > 0.0) {
                return Err(format!(
                    "contact shadows need a finite length above 0, found {}",
                    contact.length
                ));
            }
            if !(1..=CONTACT_STEP_LIMIT).contains(&contact.steps) {
                return Err(format!(
                    "contact shadows take 1 to {CONTACT_STEP_LIMIT} steps, found {}",
                    contact.steps
                ));
            }
        }
        Ok(())
    }

    fn flags(&self) -> u32 {
        let occlusion = if self.occlusion.is_some() {
            OCCLUSION
        } else {
            0
        };
        let contact = if self.contact.is_some() { CONTACT } else { 0 };
        occlusion | contact
    }

    pub fn code(&self, stride: u32) -> f32 {
        if self.on() {
            (stride * 4 + self.flags()) as f32
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub cascades: u32,
    pub resolution: u32,
    pub occlusion: bool,
    pub contact: bool,
    pub level: Level,
    pub contact_steps: u32,
}

pub fn preset(quality: ShadowQuality) -> Preset {
    let medium = Preset {
        cascades: CASCADE_COUNT as u32,
        resolution: DEFAULT_RESOLUTION,
        occlusion: false,
        contact: false,
        level: Level::Medium,
        contact_steps: CONTACT_STEPS,
    };
    match quality {
        ShadowQuality::Low => Preset {
            cascades: 2,
            resolution: DEFAULT_RESOLUTION / 2,
            level: Level::Low,
            ..medium
        },
        ShadowQuality::Medium => medium,
        ShadowQuality::High => Preset {
            occlusion: true,
            contact: true,
            level: Level::High,
            ..medium
        },
        ShadowQuality::Ultra => Preset {
            resolution: DEFAULT_RESOLUTION * 2,
            occlusion: true,
            contact: true,
            level: Level::Ultra,
            contact_steps: CONTACT_STEPS * 2,
            ..medium
        },
    }
}

pub fn look_quality(look: &Look, base: Quality) -> Quality {
    let chosen = preset(look.quality.unwrap_or_default());
    let defaults = Quality::default();
    let softness = look.softness.unwrap_or(1.0);
    let bias = look.bias.unwrap_or(1.0);
    Quality {
        cascades: look.cascades.unwrap_or(chosen.cascades),
        resolution: look.resolution.unwrap_or(chosen.resolution),
        pcf_radius: defaults.pcf_radius * softness,
        normal_bias: defaults.normal_bias * bias,
        slope_bias: defaults.slope_bias * bias,
        depth_bias: defaults.depth_bias * bias,
        ..base
    }
}

pub fn look_screen(look: &Look) -> ScreenShadows {
    let chosen = preset(look.quality.unwrap_or_default());
    ScreenShadows {
        occlusion: look
            .occlusion
            .unwrap_or(chosen.occlusion)
            .then(|| Occlusion {
                level: chosen.level,
                radius: look.occlusion_radius.unwrap_or(RADIUS),
            }),
        contact: look.contact.unwrap_or(chosen.contact).then(|| ContactRay {
            length: look.contact_length.unwrap_or(CONTACT_LENGTH),
            steps: chosen.contact_steps,
        }),
    }
}

pub struct View {
    pub view: Matrix,
    pub projection: Matrix,
    pub jitter: [f32; 2],
    pub toward_sun: [f32; 3],
    pub seed: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    projection: Matrix,
    sun: [f32; 4],
    size: [u32; 4],
    pixel: [f32; 4],
    gtao: [f32; 4],
    tuning: [f32; 4],
    contact: [f32; 4],
}

pub struct Pass {
    capacity: [u32; 2],
    stride: u32,
    params: wgpu::Buffer,
    output: wgpu::Buffer,
    _mips: wgpu::Texture,
    _working: wgpu::Texture,
    _edges: wgpu::Texture,
    prefilter: (wgpu::ComputePipeline, wgpu::BindGroup),
    prefilter_last: (wgpu::ComputePipeline, wgpu::BindGroup),
    main: (wgpu::ComputePipeline, wgpu::BindGroup),
    denoise: (wgpu::ComputePipeline, wgpu::BindGroup),
    viewport: [u32; 2],
}

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

fn sampled(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn written(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    mips: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn mip_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

impl Pass {
    pub fn new(device: &wgpu::Device, depth: &wgpu::TextureView, size: [u32; 2]) -> Self {
        assert!(size[0] > 0 && size[1] > 0);
        let r32f = wgpu::TextureFormat::R32Float;
        let r32u = wgpu::TextureFormat::R32Uint;
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let depth_type = wgpu::TextureSampleType::Depth;
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen shadow params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen shadows"),
            size: u64::from(size[0]) * u64::from(size[1]) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let mips = texture(
            device,
            "screen shadow depth",
            [size[0].max(16), size[1].max(16)],
            DEPTH_MIPS,
            r32f,
        );
        let mip_views: Vec<wgpu::TextureView> = (0..DEPTH_MIPS)
            .map(|level| mip_view(&mips, level))
            .collect();
        let all_mips = mips.create_view(&Default::default());
        let working = texture(device, "screen shadow term", size, 1, r32u);
        let working_view = working.create_view(&Default::default());
        let edges = texture(device, "screen shadow edges", size, 1, r32f);
        let edges_view = edges.create_view(&Default::default());
        let build = |label: &str,
                     entry: &str,
                     entries: &[wgpu::BindGroupLayoutEntry],
                     resources: Vec<wgpu::BindingResource>| {
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
                source: wgpu::ShaderSource::Wgsl(shader_source(entry).into()),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
            let entries: Vec<wgpu::BindGroupEntry> = resources
                .into_iter()
                .enumerate()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource,
                })
                .collect();
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &entries,
            });
            (pipeline, group)
        };
        let view = wgpu::BindingResource::TextureView;
        let prefilter = build(
            "screen shadow depth prefilter",
            "prefilter_depths",
            &[
                uniform(0),
                sampled(1, depth_type),
                written(2, r32f),
                written(3, r32f),
                written(4, r32f),
                written(5, r32f),
            ],
            vec![
                params.as_entire_binding(),
                view(depth),
                view(&mip_views[0]),
                view(&mip_views[1]),
                view(&mip_views[2]),
                view(&mip_views[3]),
            ],
        );
        let prefilter_last = build(
            "screen shadow depth last mip",
            "prefilter_last_mip",
            &[uniform(0), sampled(1, unfiltered), written(2, r32f)],
            vec![
                params.as_entire_binding(),
                view(&mip_views[3]),
                view(&mip_views[4]),
            ],
        );
        let main = build(
            "screen shadow occlusion",
            "main_pass",
            &[
                uniform(0),
                sampled(1, depth_type),
                sampled(2, unfiltered),
                written(3, r32u),
                written(4, r32f),
            ],
            vec![
                params.as_entire_binding(),
                view(depth),
                view(&all_mips),
                view(&working_view),
                view(&edges_view),
            ],
        );
        let denoise = build(
            "screen shadow denoise",
            "denoise_pass",
            &[
                uniform(0),
                sampled(1, wgpu::TextureSampleType::Uint),
                sampled(2, unfiltered),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            vec![
                params.as_entire_binding(),
                view(&working_view),
                view(&edges_view),
                output.as_entire_binding(),
            ],
        );
        Self {
            capacity: size,
            stride: size[0],
            params,
            output,
            _mips: mips,
            _working: working,
            _edges: edges,
            prefilter,
            prefilter_last,
            main,
            denoise,
            viewport: size,
        }
    }

    pub fn capacity(&self) -> [u32; 2] {
        self.capacity
    }

    pub fn stride(&self) -> u32 {
        self.stride
    }

    pub fn output(&self) -> &wgpu::Buffer {
        &self.output
    }

    pub fn write(
        &mut self,
        queue: &wgpu::Queue,
        settings: &ScreenShadows,
        view: &View,
        viewport: [u32; 2],
    ) {
        self.viewport = [
            viewport[0].min(self.capacity[0]),
            viewport[1].min(self.capacity[1]),
        ];
        let toward = transform(
            view.view,
            [
                view.toward_sun[0],
                view.toward_sun[1],
                view.toward_sun[2],
                0.0,
            ],
        );
        let length = (toward[0] * toward[0] + toward[1] * toward[1] + toward[2] * toward[2]).sqrt();
        let toward = if length > 0.0 {
            [toward[0] / length, toward[1] / length, toward[2] / length]
        } else {
            [0.0; 3]
        };
        let occlusion = settings.occlusion.unwrap_or_default();
        let (slices, steps) = occlusion.level.slices_and_steps();
        let contact = settings.contact.unwrap_or_default();
        let params = Params {
            projection: multiply(jitter_matrix(view.jitter), view.projection),
            sun: [
                toward[0],
                toward[1],
                toward[2],
                if settings.contact.is_some() {
                    contact.length
                } else {
                    0.0
                },
            ],
            size: [
                self.viewport[0],
                self.viewport[1],
                self.stride,
                settings.flags(),
            ],
            pixel: [
                1.0 / self.viewport[0] as f32,
                1.0 / self.viewport[1] as f32,
                (view.seed % 64) as f32,
                (DEPTH_MIPS - 1) as f32,
            ],
            gtao: [
                occlusion.radius * RADIUS_MULTIPLIER,
                FALLOFF_RANGE,
                FINAL_VALUE_POWER,
                SAMPLE_DISTRIBUTION_POWER,
            ],
            tuning: [
                DEPTH_MIP_SAMPLING_OFFSET,
                slices as f32,
                steps as f32,
                DENOISE_BLUR_BETA,
            ],
            contact: [
                contact.steps.clamp(1, CONTACT_STEP_LIMIT) as f32,
                THIN_OCCLUDER_COMPENSATION,
                occlusion.radius,
                0.0,
            ],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let [width, height] = self.viewport;
        let tiles = [width.div_ceil(16), height.div_ceil(16)];
        let pixels = [width.div_ceil(8), height.div_ceil(8)];
        let passes = [
            ("screen shadow depth", &self.prefilter, tiles),
            (
                "screen shadow depth last mip",
                &self.prefilter_last,
                [tiles[0].div_ceil(8), tiles[1].div_ceil(8)],
            ),
            ("screen shadows", &self.main, pixels),
            ("screen shadow denoise", &self.denoise, pixels),
        ];
        for (label, (pipeline, group), groups) in passes {
            let timing = profiler.as_deref_mut().and_then(|p| p.pass(label));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: profiler.as_deref().and_then(|p| p.compute_writes(timing)),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], 1);
        }
    }
}

pub const FRAGMENT_WGSL: &str = r#"
@group(3) @binding(28) var<storage, read> screen_shadow: array<u32>;
fn screen_terms(position: vec4f) -> vec2f {
    let code = u32(frame.sun_direction.w);
    if (code == 0u) {
        return vec2f(1.0);
    }
    let index = u32(position.y) * (code >> 2u) + u32(position.x);
    let terms = unpack2x16unorm(screen_shadow[index]);
    return vec2f(select(1.0, terms.x, (code & 1u) != 0u), select(1.0, terms.y, (code & 2u) != 0u));
}
fn screen_specular_occlusion(n_dot_v: f32, visibility: f32, roughness: f32) -> f32 {
    return clamp(pow(n_dot_v + visibility, exp2(-16.0 * roughness - 1.0)) - 1.0 + visibility, 0.0, 1.0);
}
"#;

pub const ENTRIES: [&str; 4] = [
    "prefilter_depths",
    "prefilter_last_mip",
    "main_pass",
    "denoise_pass",
];

pub fn shader_source(entry: &str) -> String {
    match entry {
        "prefilter_depths" => format!("{COMMON}{DEPTH}{PREFILTER}"),
        "prefilter_last_mip" => format!("{COMMON}{LAST_MIP}"),
        "main_pass" => format!("{COMMON}{DEPTH}{MAIN}"),
        _ => format!("{COMMON}{DENOISE}"),
    }
}

const COMMON: &str = r#"
struct Params {
    projection: mat4x4f,
    sun: vec4f,
    size: vec4u,
    pixel: vec4f,
    gtao: vec4f,
    tuning: vec4f,
    contact: vec4f,
}
@group(0) @binding(0) var<uniform> params: Params;

const PI: f32 = 3.1415926535897932384626433832795;
const PI_HALF: f32 = 1.5707963267948966192313216916398;
const OCCLUSION_TERM_SCALE: f32 = 1.5;
const HILBERT_WIDTH: u32 = 64u;

fn view_z(ndc_z: f32) -> f32 {
    let p = params.projection;
    return (p[3][2] - ndc_z * p[3][3]) / (ndc_z * p[2][3] - p[2][2]);
}

fn screen_to_view_depth(ndc_z: f32) -> f32 {
    return -view_z(ndc_z);
}

fn compute_viewspace_position(screen: vec2f, depth: f32) -> vec3f {
    let p = params.projection;
    let ndc = vec2f(screen.x * 2.0 - 1.0, 1.0 - screen.y * 2.0);
    let z = -depth;
    let w = p[2][3] * z + p[3][3];
    let x = (ndc.x * w - p[2][0] * z - p[3][0]) / p[0][0];
    let y = (ndc.y * w - p[2][1] * z - p[3][1]) / p[1][1];
    return vec3f(x, y, depth);
}

fn perspective() -> bool {
    return params.projection[2][3] != 0.0;
}

fn viewspace_pixel_size(depth: f32) -> vec2f {
    let p = params.projection;
    let w = p[2][3] * -depth + p[3][3];
    return vec2f(2.0 * w / p[0][0], 2.0 * w / p[1][1]) * params.pixel.xy;
}

fn clamp_depth(depth: f32) -> f32 {
    return clamp(depth, 0.0, 3.402823466e+38);
}

fn fast_sqrt(x: f32) -> f32 {
    return bitcast<f32>(0x1fbd1df5 + (bitcast<i32>(x) >> 1u));
}

fn fast_acos(in_x: f32) -> f32 {
    let x = abs(in_x);
    var res = -0.156583 * x + 1.570796;
    res *= fast_sqrt(1.0 - x);
    return select(3.141593 - res, res, in_x >= 0.0);
}

fn hilbert_index(x: u32, y: u32) -> u32 {
    var index = 0u;
    var pos_x = x;
    var pos_y = y;
    for (var level = HILBERT_WIDTH / 2u; level > 0u; level /= 2u) {
        let region_x = u32((pos_x & level) > 0u);
        let region_y = u32((pos_y & level) > 0u);
        index += level * level * ((3u * region_x) ^ region_y);
        if (region_y == 0u) {
            if (region_x == 1u) {
                pos_x = (HILBERT_WIDTH - 1u) - pos_x;
                pos_y = (HILBERT_WIDTH - 1u) - pos_y;
            }
            let temp = pos_x;
            pos_x = pos_y;
            pos_y = temp;
        }
    }
    return index;
}

fn spatio_temporal_noise(pix: vec2u, temporal: u32) -> vec2f {
    var index = hilbert_index(pix.x, pix.y);
    index += 288u * (temporal % 64u);
    return fract(0.5 + f32(index) * vec2f(0.75487766624669276005, 0.5698402909980532659114));
}

fn interleaved_gradient_noise(w: vec2f) -> f32 {
    let m = vec3f(0.06711056, 0.00583715, 52.9829189);
    return fract(m.z * fract(dot(w, m.xy)));
}

fn depth_mip_filter(depth0: f32, depth1: f32, depth2: f32, depth3: f32) -> f32 {
    let max_depth = max(max(depth0, depth1), max(depth2, depth3));
    let depth_range_scale_factor = 0.75;
    let effect_radius = depth_range_scale_factor * params.gtao.x;
    let falloff_range = params.gtao.y * effect_radius;
    let falloff_from = effect_radius * (1.0 - params.gtao.y);
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = falloff_from / falloff_range + 1.0;
    let weight0 = saturate((max_depth - depth0) * falloff_mul + falloff_add);
    let weight1 = saturate((max_depth - depth1) * falloff_mul + falloff_add);
    let weight2 = saturate((max_depth - depth2) * falloff_mul + falloff_add);
    let weight3 = saturate((max_depth - depth3) * falloff_mul + falloff_add);
    let weight_sum = weight0 + weight1 + weight2 + weight3;
    return (weight0 * depth0 + weight1 * depth1 + weight2 * depth2 + weight3 * depth3) / weight_sum;
}

"#;

const DEPTH: &str = r#"
@group(0) @binding(1) var depth_raw: texture_depth_2d;

fn raw_depth(coord: vec2i) -> f32 {
    return textureLoad(depth_raw, clamp(coord, vec2i(0), vec2i(params.size.xy) - 1), 0);
}
"#;

const PREFILTER: &str = r#"
@group(0) @binding(2) var out_depth0: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var out_depth1: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var out_depth2: texture_storage_2d<r32float, write>;
@group(0) @binding(5) var out_depth3: texture_storage_2d<r32float, write>;
var<workgroup> scratch_depths: array<array<f32, 8>, 8>;

@compute @workgroup_size(8, 8, 1)
fn prefilter_depths(@builtin(global_invocation_id) dispatch: vec3u, @builtin(local_invocation_id) group: vec3u) {
    let base = vec2i(dispatch.xy);
    let pix = base * 2;
    let depth0 = clamp_depth(screen_to_view_depth(raw_depth(pix)));
    let depth1 = clamp_depth(screen_to_view_depth(raw_depth(pix + vec2i(1, 0))));
    let depth2 = clamp_depth(screen_to_view_depth(raw_depth(pix + vec2i(0, 1))));
    let depth3 = clamp_depth(screen_to_view_depth(raw_depth(pix + vec2i(1, 1))));
    textureStore(out_depth0, pix, vec4f(depth0, 0.0, 0.0, 0.0));
    textureStore(out_depth0, pix + vec2i(1, 0), vec4f(depth1, 0.0, 0.0, 0.0));
    textureStore(out_depth0, pix + vec2i(0, 1), vec4f(depth2, 0.0, 0.0, 0.0));
    textureStore(out_depth0, pix + vec2i(1, 1), vec4f(depth3, 0.0, 0.0, 0.0));
    let dm1 = depth_mip_filter(depth0, depth1, depth2, depth3);
    textureStore(out_depth1, base, vec4f(dm1, 0.0, 0.0, 0.0));
    scratch_depths[group.x][group.y] = dm1;
    workgroupBarrier();
    if (all((group.xy % vec2u(2u)) == vec2u(0u))) {
        let in_tl = scratch_depths[group.x + 0u][group.y + 0u];
        let in_tr = scratch_depths[group.x + 1u][group.y + 0u];
        let in_bl = scratch_depths[group.x + 0u][group.y + 1u];
        let in_br = scratch_depths[group.x + 1u][group.y + 1u];
        let dm2 = depth_mip_filter(in_tl, in_tr, in_bl, in_br);
        textureStore(out_depth2, base / 2, vec4f(dm2, 0.0, 0.0, 0.0));
        scratch_depths[group.x][group.y] = dm2;
    }
    workgroupBarrier();
    if (all((group.xy % vec2u(4u)) == vec2u(0u))) {
        let in_tl = scratch_depths[group.x + 0u][group.y + 0u];
        let in_tr = scratch_depths[group.x + 2u][group.y + 0u];
        let in_bl = scratch_depths[group.x + 0u][group.y + 2u];
        let in_br = scratch_depths[group.x + 2u][group.y + 2u];
        let dm3 = depth_mip_filter(in_tl, in_tr, in_bl, in_br);
        textureStore(out_depth3, base / 4, vec4f(dm3, 0.0, 0.0, 0.0));
    }
}

"#;

const LAST_MIP: &str = r#"
@group(0) @binding(1) var last_source: texture_2d<f32>;
@group(0) @binding(2) var last_target: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn prefilter_last_mip(@builtin(global_invocation_id) dispatch: vec3u) {
    let base = vec2i(dispatch.xy);
    let last = vec2i(textureDimensions(last_source)) - 1;
    let in_tl = textureLoad(last_source, min(base * 2 + vec2i(0, 0), last), 0).r;
    let in_tr = textureLoad(last_source, min(base * 2 + vec2i(1, 0), last), 0).r;
    let in_bl = textureLoad(last_source, min(base * 2 + vec2i(0, 1), last), 0).r;
    let in_br = textureLoad(last_source, min(base * 2 + vec2i(1, 1), last), 0).r;
    textureStore(last_target, base, vec4f(depth_mip_filter(in_tl, in_tr, in_bl, in_br), 0.0, 0.0, 0.0));
}

"#;

const MAIN: &str = r#"
@group(0) @binding(2) var depth_mips: texture_2d<f32>;
@group(0) @binding(3) var out_term: texture_storage_2d<r32uint, write>;
@group(0) @binding(4) var out_edges: texture_storage_2d<r32float, write>;

fn working_depth(coord: vec2i) -> f32 {
    return textureLoad(depth_mips, clamp(coord, vec2i(0), vec2i(params.size.xy) - 1), 0).r;
}

fn sample_depth(screen: vec2f, mip_level: f32) -> f32 {
    let level = u32(min(floor(mip_level + 0.5), params.pixel.w));
    let valid = max(vec2i(params.size.xy) >> vec2u(level), vec2i(1));
    let coord = vec2i(floor(screen * vec2f(params.size.xy) / f32(1u << level)));
    return textureLoad(depth_mips, clamp(coord, vec2i(0), valid - 1), i32(level)).r;
}

fn calculate_edges(center_z: f32, left_z: f32, right_z: f32, top_z: f32, bottom_z: f32) -> vec4f {
    var edges_lrtb = vec4f(left_z, right_z, top_z, bottom_z) - center_z;
    let slope_lr = (edges_lrtb.y - edges_lrtb.x) * 0.5;
    let slope_tb = (edges_lrtb.w - edges_lrtb.z) * 0.5;
    let edges_lrtb_slope_adjusted = edges_lrtb + vec4f(slope_lr, -slope_lr, slope_tb, -slope_tb);
    edges_lrtb = min(abs(edges_lrtb), abs(edges_lrtb_slope_adjusted));
    return saturate(1.25 - edges_lrtb / (center_z * 0.011));
}

fn pack_edges(edges_lrtb: vec4f) -> f32 {
    let rounded = round(saturate(edges_lrtb) * 2.9);
    return dot(rounded, vec4f(64.0 / 255.0, 16.0 / 255.0, 4.0 / 255.0, 1.0 / 255.0));
}

fn calculate_normal(edges_lrtb: vec4f, pix_center_pos: vec3f, left: vec3f, right: vec3f, top: vec3f, bottom: vec3f) -> vec3f {
    let accepted_normals = saturate(vec4f(edges_lrtb.x * edges_lrtb.z, edges_lrtb.z * edges_lrtb.y, edges_lrtb.y * edges_lrtb.w, edges_lrtb.w * edges_lrtb.x) + 0.01);
    let pix_l = normalize(left - pix_center_pos);
    let pix_r = normalize(right - pix_center_pos);
    let pix_t = normalize(top - pix_center_pos);
    let pix_b = normalize(bottom - pix_center_pos);
    let pixel_normal = accepted_normals.x * cross(pix_l, pix_t)
        + accepted_normals.y * cross(pix_t, pix_r)
        + accepted_normals.z * cross(pix_r, pix_b)
        + accepted_normals.w * cross(pix_b, pix_l);
    return normalize(pixel_normal);
}

fn ambient_visibility(pix: vec2u, viewspace_z_in: f32, viewspace_normal: vec3f, normalized_screen_pos: vec2f) -> f32 {
    let viewspace_z = viewspace_z_in * 0.99999;
    let pix_center_pos = compute_viewspace_position(normalized_screen_pos, viewspace_z);
    let view_vec = select(vec3f(0.0, 0.0, -1.0), normalize(-pix_center_pos), perspective());
    let effect_radius = params.gtao.x;
    let sample_distribution_power = params.gtao.w;
    let thin_occluder_compensation = params.contact.y;
    let falloff_range = params.gtao.y * effect_radius;
    let falloff_from = effect_radius * (1.0 - params.gtao.y);
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = falloff_from / falloff_range + 1.0;
    let slice_count = params.tuning.y;
    let steps_per_slice = params.tuning.z;
    let local_noise = spatio_temporal_noise(pix, u32(params.pixel.z));
    let noise_slice = local_noise.x;
    let noise_sample = local_noise.y;
    let pixel_too_close_threshold = 1.3;
    let pixel_dir_rb_viewspace_size_at_center_z = viewspace_pixel_size(viewspace_z);
    let screenspace_radius = effect_radius / pixel_dir_rb_viewspace_size_at_center_z.x;
    var visibility = saturate((10.0 - screenspace_radius) / 100.0) * 0.5;
    let min_s = pixel_too_close_threshold / screenspace_radius;
    for (var slice = 0.0; slice < slice_count; slice += 1.0) {
        let slice_k = (slice + noise_slice) / slice_count;
        let phi = slice_k * PI;
        let cos_phi = cos(phi);
        let sin_phi = sin(phi);
        var omega = vec2f(cos_phi, -sin_phi);
        omega *= screenspace_radius;
        let direction_vec = vec3f(cos_phi, sin_phi, 0.0);
        let ortho_direction_vec = direction_vec - (dot(direction_vec, view_vec) * view_vec);
        let axis_vec = normalize(cross(ortho_direction_vec, view_vec));
        let projected_normal_vec = viewspace_normal - axis_vec * dot(viewspace_normal, axis_vec);
        let sign_norm = sign(dot(ortho_direction_vec, projected_normal_vec));
        var projected_normal_vec_length = length(projected_normal_vec);
        let cos_norm = saturate(dot(projected_normal_vec, view_vec) / projected_normal_vec_length);
        let n = sign_norm * fast_acos(cos_norm);
        let low_horizon_cos0 = cos(n + PI_HALF);
        let low_horizon_cos1 = cos(n - PI_HALF);
        var horizon_cos0 = low_horizon_cos0;
        var horizon_cos1 = low_horizon_cos1;
        for (var step_index = 0.0; step_index < steps_per_slice; step_index += 1.0) {
            let step_base_noise = (slice + step_index * steps_per_slice) * 0.6180339887498948482;
            let step_noise = fract(noise_sample + step_base_noise);
            var s = (step_index + step_noise) / steps_per_slice;
            s = pow(s, sample_distribution_power);
            s += min_s;
            var sample_offset = s * omega;
            let sample_offset_length = length(sample_offset);
            let mip_level = clamp(log2(sample_offset_length) - params.tuning.x, 0.0, params.pixel.w);
            sample_offset = round(sample_offset) * params.pixel.xy;
            let sample_screen_pos0 = normalized_screen_pos + sample_offset;
            let sz0 = sample_depth(sample_screen_pos0, mip_level);
            let sample_pos0 = compute_viewspace_position(sample_screen_pos0, sz0);
            let sample_screen_pos1 = normalized_screen_pos - sample_offset;
            let sz1 = sample_depth(sample_screen_pos1, mip_level);
            let sample_pos1 = compute_viewspace_position(sample_screen_pos1, sz1);
            let sample_delta0 = sample_pos0 - pix_center_pos;
            let sample_delta1 = sample_pos1 - pix_center_pos;
            let sample_dist0 = length(sample_delta0);
            let sample_dist1 = length(sample_delta1);
            let sample_horizon_vec0 = sample_delta0 / sample_dist0;
            let sample_horizon_vec1 = sample_delta1 / sample_dist1;
            let falloff_base0 = length(vec3f(sample_delta0.x, sample_delta0.y, sample_delta0.z * (1.0 + thin_occluder_compensation)));
            let falloff_base1 = length(vec3f(sample_delta1.x, sample_delta1.y, sample_delta1.z * (1.0 + thin_occluder_compensation)));
            let weight0 = saturate(falloff_base0 * falloff_mul + falloff_add);
            let weight1 = saturate(falloff_base1 * falloff_mul + falloff_add);
            var shc0 = dot(sample_horizon_vec0, view_vec);
            var shc1 = dot(sample_horizon_vec1, view_vec);
            shc0 = mix(low_horizon_cos0, shc0, weight0);
            shc1 = mix(low_horizon_cos1, shc1, weight1);
            horizon_cos0 = max(horizon_cos0, shc0);
            horizon_cos1 = max(horizon_cos1, shc1);
        }
        projected_normal_vec_length = mix(projected_normal_vec_length, 1.0, 0.05);
        let h0 = -fast_acos(horizon_cos1);
        let h1 = fast_acos(horizon_cos0);
        let iarc0 = (cos_norm + 2.0 * h0 * sin(n) - cos(2.0 * h0 - n)) / 4.0;
        let iarc1 = (cos_norm + 2.0 * h1 * sin(n) - cos(2.0 * h1 - n)) / 4.0;
        let local_visibility = projected_normal_vec_length * (iarc0 + iarc1);
        visibility += local_visibility;
    }
    visibility /= slice_count;
    visibility = pow(visibility, params.gtao.z);
    visibility = max(0.03, visibility);
    return visibility;
}

fn ndc_to_uv(ndc: vec2f) -> vec2f {
    return vec2f(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn contact_visibility(pix: vec2u, depth: f32, normalized_screen_pos: vec2f) -> f32 {
    let p = params.projection;
    let reach = params.sun.w;
    let position = compute_viewspace_position(normalized_screen_pos, depth);
    let ws_ray_start = vec3f(position.xy, -position.z);
    let ws_ray_end = ws_ray_start + params.sun.xyz * reach;
    let cs_ray_start = p * vec4f(ws_ray_start, 1.0);
    let cs_ray_end = p * vec4f(ws_ray_end, 1.0);
    let cs_view_ray_end = cs_ray_start + p * vec4f(0.0, 0.0, reach, 0.0);
    let ss_ray_start = cs_ray_start.xyz * (1.0 / cs_ray_start.w);
    let ss_ray_end = cs_ray_end.xyz * (1.0 / cs_ray_end.w);
    let ss_view_ray_end = cs_view_ray_end.xyz * (1.0 / cs_view_ray_end.w);
    let uv_ray_end = vec3f(ndc_to_uv(ss_ray_end.xy), ss_ray_end.z);
    let uv_ray_start = vec3f(ndc_to_uv(ss_ray_start.xy), ss_ray_start.z);
    let uv_ray = uv_ray_end - uv_ray_start;
    let step_count = u32(params.contact.x);
    let dt = 1.0 / f32(step_count);
    let tolerance = abs(ss_view_ray_end.z - ss_ray_start.z) * dt;
    let dither = interleaved_gradient_noise(vec2f(pix) + 0.5) - 0.5;
    var t = dt * dither + dt;
    var occlusion = 0.0;
    var ray = uv_ray_start;
    for (var i = 0u; i < step_count; i++) {
        ray = uv_ray_start + uv_ray * t;
        let z = raw_depth(vec2i(floor(ray.xy * vec2f(params.size.xy))));
        let dz = ray.z - z;
        if (abs(tolerance - dz) < tolerance) {
            occlusion = 1.0;
            break;
        }
        t += dt;
    }
    let fade = max(12.0 * abs(ray.xy - 0.5) - 5.0, vec2f(0.0));
    occlusion *= saturate(1.0 - dot(fade, fade));
    return 1.0 - occlusion;
}

@compute @workgroup_size(8, 8, 1)
fn main_pass(@builtin(global_invocation_id) dispatch: vec3u) {
    if (any(dispatch.xy >= params.size.xy)) {
        return;
    }
    let pix = dispatch.xy;
    let pix_coord = vec2i(pix);
    let normalized_screen_pos = (vec2f(pix) + 0.5) * params.pixel.xy;
    let viewspace_z = working_depth(pix_coord);
    let pix_lz = working_depth(pix_coord + vec2i(-1, 0));
    let pix_tz = working_depth(pix_coord + vec2i(0, -1));
    let pix_rz = working_depth(pix_coord + vec2i(1, 0));
    let pix_bz = working_depth(pix_coord + vec2i(0, 1));
    let edges_lrtb = calculate_edges(viewspace_z, pix_lz, pix_rz, pix_tz, pix_bz);
    textureStore(out_edges, pix_coord, vec4f(pack_edges(edges_lrtb), 0.0, 0.0, 0.0));
    let raw = raw_depth(pix_coord);
    var visibility = 1.0;
    var contact = 1.0;
    if (raw < 1.0) {
        if ((params.size.w & 1u) != 0u) {
            let center = compute_viewspace_position(normalized_screen_pos, viewspace_z);
            let left = compute_viewspace_position(normalized_screen_pos + vec2f(-1.0, 0.0) * params.pixel.xy, pix_lz);
            let right = compute_viewspace_position(normalized_screen_pos + vec2f(1.0, 0.0) * params.pixel.xy, pix_rz);
            let top = compute_viewspace_position(normalized_screen_pos + vec2f(0.0, -1.0) * params.pixel.xy, pix_tz);
            let bottom = compute_viewspace_position(normalized_screen_pos + vec2f(0.0, 1.0) * params.pixel.xy, pix_bz);
            let viewspace_normal = calculate_normal(edges_lrtb, center, left, right, top, bottom);
            visibility = ambient_visibility(pix, viewspace_z, viewspace_normal, normalized_screen_pos);
        }
        if ((params.size.w & 2u) != 0u && params.sun.w > 0.0) {
            contact = contact_visibility(pix, viewspace_z, normalized_screen_pos);
        }
    }
    textureStore(out_term, pix_coord, vec4u(pack2x16unorm(vec2f(saturate(visibility / OCCLUSION_TERM_SCALE), contact)), 0u, 0u, 0u));
}

"#;

const DENOISE: &str = r#"
@group(0) @binding(1) var source_term: texture_2d<u32>;
@group(0) @binding(2) var source_edges: texture_2d<f32>;
@group(0) @binding(3) var<storage, read_write> final_term: array<u32>;

fn unpack_edges(packed_value: f32) -> vec4f {
    let packed = u32(packed_value * 255.5);
    var edges_lrtb: vec4f;
    edges_lrtb.x = f32((packed >> 6u) & 3u) / 3.0;
    edges_lrtb.y = f32((packed >> 4u) & 3u) / 3.0;
    edges_lrtb.z = f32((packed >> 2u) & 3u) / 3.0;
    edges_lrtb.w = f32((packed >> 0u) & 3u) / 3.0;
    return saturate(edges_lrtb);
}

fn term_at(coord: vec2i) -> vec2f {
    return unpack2x16unorm(textureLoad(source_term, clamp(coord, vec2i(0), vec2i(params.size.xy) - 1), 0).r);
}

fn edges_at(coord: vec2i) -> vec4f {
    return unpack_edges(textureLoad(source_edges, clamp(coord, vec2i(0), vec2i(params.size.xy) - 1), 0).r);
}

@compute @workgroup_size(8, 8, 1)
fn denoise_pass(@builtin(global_invocation_id) dispatch: vec3u) {
    if (any(dispatch.xy >= params.size.xy)) {
        return;
    }
    let pix_coord = vec2i(dispatch.xy);
    let blur_amount = params.tuning.w;
    let diag_weight = 0.85 * 0.5;
    let edges_l_lrtb = edges_at(pix_coord + vec2i(-1, 0));
    let edges_t_lrtb = edges_at(pix_coord + vec2i(0, -1));
    let edges_r_lrtb = edges_at(pix_coord + vec2i(1, 0));
    let edges_b_lrtb = edges_at(pix_coord + vec2i(0, 1));
    var edges_c_lrtb = edges_at(pix_coord);
    edges_c_lrtb *= vec4f(edges_l_lrtb.y, edges_r_lrtb.x, edges_t_lrtb.w, edges_b_lrtb.z);
    let leak_threshold = 2.5;
    let leak_strength = 0.5;
    let edginess = (saturate(4.0 - leak_threshold - dot(edges_c_lrtb, vec4f(1.0))) / (4.0 - leak_threshold)) * leak_strength;
    edges_c_lrtb = saturate(edges_c_lrtb + edginess);
    let weight_tl = diag_weight * (edges_c_lrtb.x * edges_l_lrtb.z + edges_c_lrtb.z * edges_t_lrtb.x);
    let weight_tr = diag_weight * (edges_c_lrtb.z * edges_t_lrtb.y + edges_c_lrtb.y * edges_r_lrtb.z);
    let weight_bl = diag_weight * (edges_c_lrtb.w * edges_b_lrtb.x + edges_c_lrtb.x * edges_l_lrtb.w);
    let weight_br = diag_weight * (edges_c_lrtb.y * edges_r_lrtb.w + edges_c_lrtb.w * edges_b_lrtb.y);
    var sum_weight = blur_amount;
    var sum = term_at(pix_coord) * sum_weight;
    sum += edges_c_lrtb.x * term_at(pix_coord + vec2i(-1, 0));
    sum += edges_c_lrtb.y * term_at(pix_coord + vec2i(1, 0));
    sum += edges_c_lrtb.z * term_at(pix_coord + vec2i(0, -1));
    sum += edges_c_lrtb.w * term_at(pix_coord + vec2i(0, 1));
    sum += weight_tl * term_at(pix_coord + vec2i(-1, -1));
    sum += weight_tr * term_at(pix_coord + vec2i(1, -1));
    sum += weight_bl * term_at(pix_coord + vec2i(-1, 1));
    sum += weight_br * term_at(pix_coord + vec2i(1, 1));
    sum_weight += dot(edges_c_lrtb, vec4f(1.0)) + weight_tl + weight_tr + weight_bl + weight_br;
    let term = sum / sum_weight;
    final_term[dispatch.y * params.size.z + dispatch.x] = pack2x16unorm(vec2f(term.x * OCCLUSION_TERM_SCALE, term.y));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pass_validates() {
        for entry in ENTRIES {
            let module = naga::front::wgsl::parse_str(&shader_source(entry))
                .unwrap_or_else(|error| panic!("{entry}: {error}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{entry}: {error:?}"));
            assert!(module.entry_points.iter().any(|point| point.name == entry));
        }
    }

    #[test]
    fn the_default_look_is_todays_shadows_with_nothing_on_screen() {
        let base = Quality {
            sun_radius_deg: 0.5,
            ..Quality::default()
        };
        assert_eq!(look_quality(&Look::default(), base), base);
        assert_eq!(look_screen(&Look::default()), ScreenShadows::OFF);
        let high = Look {
            quality: Some(ShadowQuality::High),
            contact: Some(false),
            ..Look::default()
        };
        assert_eq!(look_quality(&high, base), base);
        let screen = look_screen(&high);
        assert_eq!(screen.occlusion.map(|o| o.level), Some(Level::High));
        assert!(screen.contact.is_none());
    }

    #[test]
    fn the_frame_code_carries_the_stride_and_both_terms() {
        assert_eq!(ScreenShadows::OFF.code(1920), 0.0);
        let both = ScreenShadows {
            occlusion: Some(Occlusion::default()),
            contact: Some(ContactRay::default()),
        };
        let code = both.code(3840) as u32;
        assert_eq!((code >> 2, code & 3), (3840, 3));
        assert!(
            ScreenShadows {
                contact: Some(ContactRay {
                    length: 0.3,
                    steps: 0
                }),
                ..ScreenShadows::OFF
            }
            .check()
            .is_err()
        );
    }
}
