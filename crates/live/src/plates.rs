use pfx_bake::plate::{Depth, Plate, PlateCamera, anchor_blend};
use pfx_gpu::{Gpu, GpuProfiler, wgpu};
use wgpu::util::DeviceExt;

use pfx_bake::plate::read_plate;
use pfx_load::scene::{Draw, PlateCasters, ProxyShape, Scene};

use crate::frame::{Frame, Instance, Matrix, MeshData, MeshHandle};
use crate::renderer::{Renderer, invert};
use crate::shadow::light_basis;

pub const SHADOW_SIZE: u32 = 2048;
pub const DEPTH_BIAS: f32 = 0.0015;
pub const ITERATIONS: u32 = 8;
const UNIFORM_BYTES: usize = 7 * 64 + 15 * 16;

pub const PLATE_WGSL: &str = r#"
struct PlateUniform {
    inverse: mat4x4f,
    still_inverse: mat4x4f,
    view_projection: mat4x4f,
    current: mat4x4f,
    previous: mat4x4f,
    view: mat4x4f,
    shadow: mat4x4f,
    origin: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
    size: vec4f,
    blend: vec4f,
    eye: vec4f,
    params: vec4f,
    sun: vec4f,
    extra: vec4f,
    light_a: vec4f,
    light_b: vec4f,
    light_now: vec4f,
    toward_a: vec4f,
    toward_b: vec4f,
}
struct PlateCascades {
    splits: vec4f,
    filter_params: vec4f,
    sun: vec4f,
    texels: vec4f,
    view_proj_0: mat4x4f,
    view_proj_1: mat4x4f,
    view_proj_2: mat4x4f,
}
@group(0) @binding(0) var<uniform> plate: PlateUniform;
@group(0) @binding(1) var plate_color: texture_2d_array<f32>;
@group(0) @binding(2) var plate_sun: texture_2d_array<f32>;
@group(0) @binding(3) var plate_depth_codes: texture_2d<u32>;
@group(0) @binding(4) var plate_depth_floats: texture_2d<f32>;
@group(0) @binding(5) var plate_normal: texture_2d<u32>;
@group(0) @binding(6) var plate_ids: texture_2d<u32>;
@group(0) @binding(7) var plate_sampler: sampler;
@group(0) @binding(8) var plate_shadow: texture_depth_2d;
@group(0) @binding(9) var plate_compare: sampler_comparison;
@group(0) @binding(10) var plate_transfer: texture_2d_array<f32>;
@group(0) @binding(11) var plate_point: sampler;
@group(1) @binding(0) var<uniform> cascades: PlateCascades;
@group(1) @binding(1) var cascade_atlas: texture_depth_2d_array;

struct PlateOut {
    @location(0) color: vec4f,
    @location(1) id: u32,
    @location(2) velocity: vec2f,
    @location(3) normal_roughness: vec4f,
    @location(4) reactive: f32,
    @builtin(frag_depth) depth: f32,
}

fn plate_inverse_at(texel: vec2i) -> f32 {
    let size = vec2i(plate.size.xy);
    let at = clamp(texel, vec2i(0), size - vec2i(1));
    if (plate.forward.w < 0.5) {
        let code = textureLoad(plate_depth_codes, at, 0).r;
        return f32(code) / (65535.0 * plate.origin.w);
    }
    let depth = textureLoad(plate_depth_floats, at, 0).r;
    return select(0.0, 1.0 / depth, depth > 0.0);
}

fn plate_inverse(position: vec2f) -> f32 {
    let q = position - vec2f(0.5);
    let base = vec2i(floor(q));
    let f = q - floor(q);
    var a: f32;
    var b: f32;
    var c: f32;
    var d: f32;
    if (plate.forward.w < 0.5) {
        let codes = vec4f(textureGather(0, plate_depth_codes, plate_point, (vec2f(base) + vec2f(1.0)) / plate.size.xy)) / (65535.0 * plate.origin.w);
        a = codes.w;
        b = codes.z;
        c = codes.x;
        d = codes.y;
    } else {
        a = plate_inverse_at(base);
        b = plate_inverse_at(base + vec2i(1, 0));
        c = plate_inverse_at(base + vec2i(0, 1));
        d = plate_inverse_at(base + vec2i(1, 1));
    }
    let lo = min(min(a, b), min(c, d));
    let hi = max(max(a, b), max(c, d));
    if (lo > 0.0 && hi <= lo * 1.03) {
        return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    }
    return plate_inverse_at(vec2i(floor(position)));
}

fn plate_texel(world: vec3f) -> vec3f {
    let v = world - plate.origin.xyz;
    let z = dot(v, plate.forward.xyz);
    let r = plate.right.xyz;
    let u = plate.up.xyz;
    let a = dot(v, r) / (dot(r, r) * z);
    let b = dot(v, u) / (dot(u, u) * z);
    let x = a - plate.right.w;
    let y = plate.up.w - b;
    return vec3f((x + 1.0) * 0.5 * plate.size.x, (y + 1.0) * 0.5 * plate.size.y, z);
}

fn plate_along(eye: vec3f, direction: vec3f, depth: f32) -> f32 {
    let offset = dot(eye - plate.origin.xyz, plate.forward.xyz);
    return (depth - offset) / max(dot(direction, plate.forward.xyz), 1e-6);
}

struct PlateHit {
    t: f32,
    texel: vec2f,
    inverse: f32,
}

fn plate_march(eye: vec3f, direction: vec3f, start: f32, steps: u32) -> PlateHit {
    var t = start;
    var hit = PlateHit(t, vec2f(0.0), 0.0);
    for (var i = 0u; i < steps; i++) {
        let at = plate_texel(eye + direction * t);
        let inverse = plate_inverse(at.xy);
        hit.texel = at.xy;
        hit.inverse = inverse;
        if (inverse <= 0.0) {
            return hit;
        }
        let next = plate_along(eye, direction, 1.0 / inverse);
        let settled = abs(next - t) <= 1e-5 * max(t, 1e-3);
        t = next;
        hit.t = t;
        if (settled) {
            break;
        }
    }
    return hit;
}

fn plate_ray(inverse: mat4x4f, ndc: vec2f) -> vec3f {
    let far = inverse * vec4f(ndc, 1.0, 1.0);
    return normalize(far.xyz / far.w - plate.eye.xyz);
}

fn plate_start(eye: vec3f, direction: vec3f) -> f32 {
    let far = plate_texel(eye + direction * 1e4);
    let inverse = plate_inverse(far.xy);
    return plate_along(eye, direction, select(1e4, 1.0 / inverse, inverse > 0.0));
}

fn plate_octahedral(packed: u32) -> vec3f {
    let x = max(f32(bitcast<i32>(packed << 16u) >> 16u) / 32767.0, -1.0);
    let y = max(f32(bitcast<i32>(packed) >> 16u) / 32767.0, -1.0);
    var n = vec3f(x, y, 1.0 - abs(x) - abs(y));
    if (n.z < 0.0) {
        let flip = vec2f(select(-1.0, 1.0, n.x >= 0.0), select(-1.0, 1.0, n.y >= 0.0));
        n = vec3f((1.0 - abs(n.yx)) * flip, n.z);
    }
    return normalize(n);
}

fn plate_shadowed(world: vec3f, normal: vec3f) -> f32 {
    let offset = world + normal * plate.params.z * 1.5 + plate.sun.xyz * plate.params.z;
    let clip = plate.shadow * vec4f(offset, 1.0);
    let uv = vec2f(clip.x, -clip.y) * 0.5 + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || clip.z >= 1.0) {
        return 1.0;
    }
    let texel = 1.0 / f32(textureDimensions(plate_shadow).x);
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = uv + vec2f(f32(x), f32(y)) * texel * 1.5;
            lit += textureSampleCompareLevel(plate_shadow, plate_compare, at, clip.z);
        }
    }
    return lit / 9.0;
}

fn plate_sunlit(world: vec3f, normal: vec3f) -> f32 {
    if (cascades.splits.w <= 0.0) {
        return 1.0;
    }
    let distance = length(world - plate.eye.xyz);
    var layer = 0;
    var matrix = cascades.view_proj_0;
    var texel = cascades.texels.x;
    if (distance > cascades.splits.y) {
        layer = 1;
        matrix = cascades.view_proj_1;
        texel = cascades.texels.y;
    }
    if (distance > cascades.splits.z) {
        layer = 2;
        matrix = cascades.view_proj_2;
        texel = cascades.texels.z;
    }
    let clip = matrix * vec4f(world + normal * texel * 1.5, 1.0);
    let point = clip.xyz / clip.w;
    let uv = point.xy * vec2f(0.5, -0.5) + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || point.z < 0.0 || point.z > 1.0) {
        return 1.0;
    }
    let size = vec2f(textureDimensions(cascade_atlas));
    let centre = uv * size;
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = clamp(vec2i(centre + vec2f(f32(x), f32(y))), vec2i(0), vec2i(size) - vec2i(1));
            lit += select(0.0, 1.0, point.z - cascades.sun.w <= textureLoad(cascade_atlas, at, layer, 0));
        }
    }
    return lit / 9.0;
}

@fragment fn fragment(@builtin(position) position: vec4f) -> PlateOut {
    let ndc = vec2f(position.x * plate.size.z * 2.0 - 1.0, 1.0 - position.y * plate.size.w * 2.0);
    let eye = plate.eye.xyz;
    let steps = u32(plate.params.y);
    let jittered = plate_ray(plate.inverse, ndc);
    let hit = plate_march(eye, jittered, plate_start(eye, jittered), steps);
    if (hit.inverse <= 0.0) {
        discard;
    }
    let still = plate_ray(plate.still_inverse, ndc);
    let seen = plate_march(eye, still, plate_along(eye, still, 1.0 / hit.inverse), 3u);
    let world = eye + still * seen.t;
    let uv = seen.texel / plate.size.xy;
    let a = i32(plate.blend.x);
    let b = i32(plate.blend.y);
    let color_a = textureSampleLevel(plate_color, plate_sampler, uv, a, 0.0).rgb;
    let color_b = textureSampleLevel(plate_color, plate_sampler, uv, b, 0.0).rgb;
    var color = mix(color_a, color_b, plate.blend.z);
    let texel = clamp(vec2i(floor(seen.texel)), vec2i(0), vec2i(plate.size.xy) - vec2i(1));
    var normal = vec3f(0.0, 1.0, 0.0);
    if (plate.extra.x > 0.5) {
        normal = plate_octahedral(textureLoad(plate_normal, texel, 0).r);
    }
    if (plate.blend.w > 0.5) {
        let share_a = textureSampleLevel(plate_sun, plate_sampler, uv, a, 0.0).rgb;
        let share_b = textureSampleLevel(plate_sun, plate_sampler, uv, b, 0.0).rgb;
        let unit_a = select(vec3f(0.0), share_a / plate.light_a.rgb, plate.light_a.rgb > vec3f(0.0));
        let unit_b = select(vec3f(0.0), share_b / plate.light_b.rgb, plate.light_b.rgb > vec3f(0.0));
        var share = mix(unit_a, unit_b, plate.blend.z) * plate.light_now.rgb;
        let rest = max(mix(color_a - share_a, color_b - share_b, plate.blend.z), vec3f(0.0));
        if (plate.sun.w > 0.5) {
            share *= 1.0 - (1.0 - plate_shadowed(world, normal)) * plate.eye.w;
        }
        let weight = (1.0 - abs(1.0 - 2.0 * plate.blend.z)) * plate.extra.w;
        if (weight > 0.0) {
            let open_a = textureSampleLevel(plate_transfer, plate_sampler, uv, a, 0.0).rgb;
            let open_b = textureSampleLevel(plate_transfer, plate_sampler, uv, b, 0.0).rgb;
            let facing_a = max(dot(normal, plate.toward_a.xyz), 0.0);
            let facing_b = max(dot(normal, plate.toward_b.xyz), 0.0);
            let facing = max(dot(normal, plate.sun.xyz), 0.0);
            let per_a = select(vec3f(0.0), open_a / plate.light_a.rgb, plate.light_a.rgb > vec3f(0.0));
            let per_b = select(vec3f(0.0), open_b / plate.light_b.rgb, plate.light_b.rgb > vec3f(0.0));
            let albedo = (per_a * facing_a + per_b * facing_b) / max(facing_a * facing_a + facing_b * facing_b, 1e-4);
            let relit = albedo * facing * plate.light_now.rgb * plate_sunlit(world, normal);
            share = mix(share, relit, weight);
        }
        color = rest + share;
    }
    var out: PlateOut;
    out.color = vec4f(color, 1.0);
    out.id = select(0u, textureLoad(plate_ids, texel, 0).r, plate.extra.y > 0.5);
    let front = eye + jittered * hit.t * (1.0 + plate.params.x);
    let clip = plate.view_projection * vec4f(front, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    let now = plate.current * vec4f(world, 1.0);
    let before = plate.previous * vec4f(world, 1.0);
    let moved = (before.xy / before.w - now.xy / now.w) * vec2f(0.5, -0.5);
    out.velocity = select(moved, vec2f(0.0), abs(moved) < vec2f(1e-8));
    out.normal_roughness = vec4f(normalize((plate.view * vec4f(normal, 0.0)).xyz), 1.0);
    out.reactive = plate.extra.z;
    return out;
}
"#;

pub const PLATE_SHADOW_WGSL: &str = r#"
@group(0) @binding(0) var<uniform> light: mat4x4f;
struct Caster {
    @location(0) position: vec3f,
    @location(1) c0: vec4f,
    @location(2) c1: vec4f,
    @location(3) c2: vec4f,
    @location(4) c3: vec4f,
}
@vertex fn vertex(input: Caster) -> @builtin(position) vec4f {
    let model = mat4x4f(input.c0, input.c1, input.c2, input.c3);
    return light * model * vec4f(input.position, 1.0);
}
"#;

const FULLSCREEN: &str = "@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f { let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(xy[index], 0.0, 1.0); }";

pub fn plate_source() -> String {
    format!("{FULLSCREEN}\n{PLATE_WGSL}")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateSettings {
    pub depth_bias: f32,
    pub iterations: u32,
    pub shadow_strength: f32,
    pub temporal: bool,
    pub relight: bool,
}

impl Default for PlateSettings {
    fn default() -> Self {
        Self {
            depth_bias: DEPTH_BIAS,
            iterations: ITERATIONS,
            shadow_strength: 1.0,
            temporal: false,
            relight: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateInfo {
    pub width: u32,
    pub height: u32,
    pub anchors: usize,
    pub bytes_per_anchor: u64,
    pub shared_bytes: u64,
    pub camera: PlateCamera,
    pub bounds: pfx_bake::plate::Bounds,
}

pub struct PlateFrame<'a> {
    pub view_projection: Matrix,
    pub jittered: Matrix,
    pub previous: Matrix,
    pub view: Matrix,
    pub eye: [f32; 3],
    pub toward_sun: [f32; 3],
    pub sun_light: [f32; 3],
    pub sun_up: bool,
    pub instances: &'a [Instance],
}

pub struct Plates {
    info: PlateInfo,
    anchors: Vec<f32>,
    lights: Vec<[f32; 3]>,
    towards: Vec<[f32; 3]>,
    has_transfer: bool,
    cascades: wgpu::Buffer,
    cascade_layout: wgpu::BindGroupLayout,
    depth_near: f32,
    float_depth: bool,
    has_sun: bool,
    has_normal: bool,
    has_ids: bool,
    hour: f32,
    settings: PlateSettings,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    shadow_uniform: wgpu::Buffer,
    shadow_bind: wgpu::BindGroup,
    shadow_pipeline: wgpu::RenderPipeline,
    casters: wgpu::Buffer,
    caster_capacity: u64,
    shadow_texel: f32,
    _textures: Vec<wgpu::Texture>,
}

fn upload(
    gpu: &Gpu,
    label: &str,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    layers: u32,
    bytes: &[u8],
) -> wgpu::Texture {
    gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytes,
    )
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

fn layout_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dimension,
            multisampled: false,
        },
    )
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

pub fn composite_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let unfilterable = wgpu::TextureSampleType::Float { filterable: false };
    let filterable = wgpu::TextureSampleType::Float { filterable: true };
    let flat = wgpu::TextureViewDimension::D2;
    let array = wgpu::TextureViewDimension::D2Array;
    vec![
        uniform_entry(0),
        texture_entry(1, filterable, array),
        texture_entry(2, filterable, array),
        texture_entry(3, wgpu::TextureSampleType::Uint, flat),
        texture_entry(4, unfilterable, flat),
        texture_entry(5, wgpu::TextureSampleType::Uint, flat),
        texture_entry(6, wgpu::TextureSampleType::Uint, flat),
        layout_entry(
            7,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        ),
        texture_entry(8, wgpu::TextureSampleType::Depth, flat),
        layout_entry(
            9,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        ),
        texture_entry(10, filterable, array),
        layout_entry(
            11,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        ),
    ]
}

pub fn cascade_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        uniform_entry(0),
        texture_entry(
            1,
            wgpu::TextureSampleType::Depth,
            wgpu::TextureViewDimension::D2Array,
        ),
    ]
}

fn words<T: bytemuck::Pod>(values: &[T]) -> Vec<u8> {
    bytemuck::cast_slice(values).to_vec()
}

impl Plates {
    pub fn new(gpu: &Gpu, plate: &Plate, settings: PlateSettings) -> Result<Self, String> {
        let manifest = &plate.manifest;
        let size = [manifest.width, manifest.height];
        let limit = gpu.device.limits().max_texture_dimension_2d;
        if size[0] > limit || size[1] > limit {
            return Err(format!(
                "a {}x{} plate is larger than this device's {limit} texels",
                size[0], size[1]
            ));
        }
        let anchors = manifest.anchors.len() as u32;
        let color_bytes: Vec<u8> = plate.color.iter().flat_map(|layer| words(layer)).collect();
        let color = upload(
            gpu,
            "plate colour",
            wgpu::TextureFormat::Rgb9e5Ufloat,
            size,
            anchors,
            &color_bytes,
        );
        let sun = match &plate.sun {
            Some(layers) => {
                let bytes: Vec<u8> = layers.iter().flat_map(|layer| words(layer)).collect();
                upload(
                    gpu,
                    "plate sun",
                    wgpu::TextureFormat::Rgb9e5Ufloat,
                    size,
                    anchors,
                    &bytes,
                )
            }
            None => upload(
                gpu,
                "plate sun",
                wgpu::TextureFormat::Rgb9e5Ufloat,
                [1, 1],
                1,
                &[0; 4],
            ),
        };
        let transfer = match &plate.transfer {
            Some(layers) => {
                let bytes: Vec<u8> = layers.iter().flat_map(|layer| words(layer)).collect();
                upload(
                    gpu,
                    "plate transfer",
                    wgpu::TextureFormat::Rgb9e5Ufloat,
                    size,
                    anchors,
                    &bytes,
                )
            }
            None => upload(
                gpu,
                "plate transfer",
                wgpu::TextureFormat::Rgb9e5Ufloat,
                [1, 1],
                1,
                &[0; 4],
            ),
        };
        let (codes, floats, float_depth) = match &plate.depth {
            Depth::Inverse(values) => (
                upload(
                    gpu,
                    "plate depth",
                    wgpu::TextureFormat::R16Uint,
                    size,
                    1,
                    &words(values),
                ),
                upload(
                    gpu,
                    "plate depth",
                    wgpu::TextureFormat::R32Float,
                    [1, 1],
                    1,
                    &[0; 4],
                ),
                false,
            ),
            Depth::Float(values) => (
                upload(
                    gpu,
                    "plate depth",
                    wgpu::TextureFormat::R16Uint,
                    [1, 1],
                    1,
                    &[0; 2],
                ),
                upload(
                    gpu,
                    "plate depth",
                    wgpu::TextureFormat::R32Float,
                    size,
                    1,
                    &words(values),
                ),
                true,
            ),
        };
        let normal = match &plate.normal {
            Some(values) => upload(
                gpu,
                "plate normal",
                wgpu::TextureFormat::R32Uint,
                size,
                1,
                &words(values),
            ),
            None => upload(
                gpu,
                "plate normal",
                wgpu::TextureFormat::R32Uint,
                [1, 1],
                1,
                &[0; 4],
            ),
        };
        let ids = match &plate.id {
            Some(values) => upload(
                gpu,
                "plate ids",
                wgpu::TextureFormat::R16Uint,
                size,
                1,
                &words(values),
            ),
            None => upload(
                gpu,
                "plate ids",
                wgpu::TextureFormat::R16Uint,
                [1, 1],
                1,
                &[0; 2],
            ),
        };
        let shadow = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("plate shadow"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow.create_view(&Default::default());
        let device = &gpu.device;
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plate uniform"),
            size: UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plate"),
            entries: &composite_entries(),
        });
        let cascade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plate cascades"),
            entries: &cascade_entries(),
        });
        let cascades = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plate cascades"),
            size: crate::shadow::SAMPLE_UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("plate sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let point = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("plate depth gather"),
            ..Default::default()
        });
        let compare = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("plate shadow compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let views = [
            array_view(&color),
            array_view(&sun),
            codes.create_view(&Default::default()),
            floats.create_view(&Default::default()),
            normal.create_view(&Default::default()),
            ids.create_view(&Default::default()),
            array_view(&transfer),
        ];
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plate"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&views[4]),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&views[5]),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Sampler(&compare),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&views[6]),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::Sampler(&point),
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("plate composite"),
            source: wgpu::ShaderSource::Wgsl(plate_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("plate composite"),
            bind_group_layouts: &[&layout, &cascade_layout],
            push_constant_ranges: &[],
        });
        let target = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plate composite"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                targets: &[
                    target(wgpu::TextureFormat::Rgba16Float),
                    target(wgpu::TextureFormat::R32Uint),
                    target(wgpu::TextureFormat::Rg16Float),
                    target(crate::frame::NORMAL_FORMAT),
                    target(wgpu::TextureFormat::R8Unorm),
                ],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let shadow_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plate shadow light"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("plate shadow"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shadow_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plate shadow"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_uniform.as_entire_binding(),
            }],
        });
        let shadow_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("plate shadow"),
            source: wgpu::ShaderSource::Wgsl(PLATE_SHADOW_WGSL.into()),
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("plate shadow"),
                bind_group_layouts: &[&shadow_layout],
                push_constant_ranges: &[],
            });
        let instance_attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 32,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 48,
                shader_location: 4,
            },
        ];
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("plate shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vertex"),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        }],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: 64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &instance_attributes,
                    },
                ],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let casters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("plate casters"),
            size: 64 * 64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            info: PlateInfo {
                width: size[0],
                height: size[1],
                anchors: manifest.anchors.len(),
                bytes_per_anchor: plate.bytes_per_anchor(),
                shared_bytes: plate.shared_bytes(),
                camera: manifest.camera,
                bounds: manifest.bounds,
            },
            anchors: manifest.anchors.clone(),
            lights: manifest.suns.iter().map(|sun| sun.light()).collect(),
            towards: manifest.suns.iter().map(|sun| sun.direction).collect(),
            has_transfer: plate.transfer.is_some(),
            cascades,
            cascade_layout,
            depth_near: manifest.depth_near,
            float_depth,
            has_sun: plate.sun.is_some(),
            has_normal: plate.normal.is_some(),
            has_ids: plate.id.is_some(),
            hour: manifest.anchors[0],
            settings,
            uniform,
            bind,
            pipeline,
            shadow_view,
            shadow_uniform,
            shadow_bind,
            shadow_pipeline,
            casters,
            caster_capacity: 64,
            shadow_texel: 0.0,
            _textures: vec![color, sun, codes, floats, normal, ids, transfer, shadow],
        })
    }

    pub fn info(&self) -> PlateInfo {
        self.info
    }

    pub fn hour(&self) -> f32 {
        self.hour
    }

    pub fn set_hour(&mut self, hour: f32) {
        self.hour = hour;
    }

    pub fn blend(&self) -> (usize, usize, f32) {
        anchor_blend(&self.anchors, self.hour)
    }

    pub fn settings(&self) -> PlateSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: PlateSettings) {
        self.settings = settings;
    }

    pub fn light(&self, toward_sun: [f32; 3]) -> (Matrix, f32) {
        let bounds = self.info.bounds;
        let [right, up, forward] = light_basis(toward_sun);
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for corner in 0..8 {
            let point = [
                if corner & 1 == 0 {
                    bounds.min[0]
                } else {
                    bounds.max[0]
                },
                if corner & 2 == 0 {
                    bounds.min[1]
                } else {
                    bounds.max[1]
                },
                if corner & 4 == 0 {
                    bounds.min[2]
                } else {
                    bounds.max[2]
                },
            ];
            for (k, axis) in [right, up, forward].iter().enumerate() {
                let value = point[0] * axis[0] + point[1] * axis[1] + point[2] * axis[2];
                lo[k] = lo[k].min(value);
                hi[k] = hi[k].max(value);
            }
        }
        let extent = (0..3)
            .map(|k| hi[k] - lo[k])
            .fold(0.0f32, f32::max)
            .max(1e-3);
        let margin = extent * 0.02;
        let near = lo[2] - extent * 2.0;
        let far = hi[2] + margin;
        let half = [
            ((hi[0] - lo[0]) * 0.5 + margin).max(1e-3),
            ((hi[1] - lo[1]) * 0.5 + margin).max(1e-3),
        ];
        let centre = [(hi[0] + lo[0]) * 0.5, (hi[1] + lo[1]) * 0.5];
        let range = far - near;
        let row = |axis: [f32; 3], scale: f32| axis.map(|v| v * scale);
        let r = row(right, 1.0 / half[0]);
        let u = row(up, 1.0 / half[1]);
        let f = row(forward, 1.0 / range);
        let matrix = [
            [r[0], u[0], f[0], 0.0],
            [r[1], u[1], f[1], 0.0],
            [r[2], u[2], f[2], 0.0],
            [
                -centre[0] / half[0],
                -centre[1] / half[1],
                -near / range,
                1.0,
            ],
        ];
        let texel = 2.0 * half[0].max(half[1]) / SHADOW_SIZE as f32;
        (matrix, texel)
    }

    fn uniform_bytes(
        &self,
        frame: &PlateFrame<'_>,
        light: Matrix,
        casters: bool,
        size: [u32; 2],
    ) -> Result<Vec<u8>, String> {
        let inverse = invert(frame.jittered).ok_or("camera projection is singular")?;
        let still = invert(frame.view_projection).ok_or("camera projection is singular")?;
        let (a, b, t) = self.blend();
        let camera = self.info.camera;
        let mut floats: Vec<f32> = Vec::with_capacity(UNIFORM_BYTES / 4);
        for matrix in [
            inverse,
            still,
            frame.jittered,
            frame.view_projection,
            frame.previous,
            frame.view,
            light,
        ] {
            floats.extend(matrix.iter().flatten());
        }
        floats.extend([
            camera.origin[0],
            camera.origin[1],
            camera.origin[2],
            self.depth_near,
            camera.forward[0],
            camera.forward[1],
            camera.forward[2],
            if self.float_depth { 1.0 } else { 0.0 },
            camera.right[0],
            camera.right[1],
            camera.right[2],
            camera.shift[0],
            camera.up[0],
            camera.up[1],
            camera.up[2],
            camera.shift[1],
            self.info.width as f32,
            self.info.height as f32,
            1.0 / size[0] as f32,
            1.0 / size[1] as f32,
            a as f32,
            b as f32,
            t,
            if self.has_sun { 1.0 } else { 0.0 },
            frame.eye[0],
            frame.eye[1],
            frame.eye[2],
            self.settings.shadow_strength,
            self.settings.depth_bias,
            self.settings.iterations.max(1) as f32,
            self.shadow_texel,
            0.0,
            frame.toward_sun[0],
            frame.toward_sun[1],
            frame.toward_sun[2],
            if casters && frame.sun_up { 1.0 } else { 0.0 },
            if self.has_normal { 1.0 } else { 0.0 },
            if self.has_ids { 1.0 } else { 0.0 },
            if self.settings.temporal { 0.0 } else { 1.0 },
            if self.has_transfer && self.settings.relight {
                1.0
            } else {
                0.0
            },
        ]);
        for light in [self.lights[a], self.lights[b], frame.sun_light] {
            floats.extend([light[0], light[1], light[2], 0.0]);
        }
        for toward in [self.towards[a], self.towards[b]] {
            floats.extend([toward[0], toward[1], toward[2], 0.0]);
        }
        Ok(bytemuck::cast_slice(&floats).to_vec())
    }

    pub fn encode(
        &mut self,
        live: &Frame,
        sun: (&crate::shadow::Shadows, &crate::shadow::Fit),
        reactive: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        frame: &PlateFrame<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<bool, String> {
        let gpu = &live.gpu;
        let targets = &live.targets;
        let mesh = |instance: &Instance| live.shadow_mesh(instance.mesh);
        let casting: Vec<&Instance> = frame
            .instances
            .iter()
            .filter(|instance| instance.casts_shadow && mesh(instance).is_some())
            .collect();
        let shadows = self.has_sun && frame.sun_up && !casting.is_empty();
        let (light, texel) = self.light(frame.toward_sun);
        self.shadow_texel = texel;
        if shadows {
            let bytes: Vec<u8> = casting
                .iter()
                .flat_map(|instance| {
                    bytemuck::cast_slice::<f32, u8>(instance.model.as_flattened()).to_vec()
                })
                .collect();
            let needed = casting.len() as u64;
            if needed > self.caster_capacity {
                self.caster_capacity = needed.next_power_of_two();
                self.casters = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("plate casters"),
                    size: 64 * self.caster_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            gpu.queue.write_buffer(&self.casters, 0, &bytes);
            gpu.queue.write_buffer(
                &self.shadow_uniform,
                0,
                bytemuck::cast_slice(light.as_flattened()),
            );
            let timing = profiler
                .as_deref_mut()
                .and_then(|p| p.pass("plate shadows"));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plate shadows"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: profiler.as_deref().and_then(|p| p.render_writes(timing)),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind, &[]);
            pass.set_vertex_buffer(1, self.casters.slice(..));
            for (index, instance) in casting.iter().enumerate() {
                let Some((positions, indices, count)) = mesh(instance) else {
                    continue;
                };
                pass.set_vertex_buffer(0, positions.slice(..));
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                let at = index as u32;
                pass.draw_indexed(0..count, 0, at..at + 1);
            }
        }
        let bytes = self.uniform_bytes(frame, light, shadows, live.viewport())?;
        gpu.queue.write_buffer(&self.uniform, 0, &bytes);
        gpu.queue
            .write_buffer(&self.cascades, 0, &sun.0.sample_uniform(sun.1));
        let cascade_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("plate cascades"),
            layout: &self.cascade_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cascades.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(sun.0.atlas_view()),
                },
            ],
        });
        let timing = profiler
            .as_deref_mut()
            .and_then(|p| p.pass("plate composite"));
        let load = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("plate composite"),
            color_attachments: &[
                load(&targets.hdr.view),
                load(&targets.ids_view),
                load(&targets.velocity_view),
                load(&targets.normal_roughness_view),
                Some(wgpu::RenderPassColorAttachment {
                    view: reactive,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: profiler.as_deref().and_then(|p| p.render_writes(timing)),
            occlusion_query_set: None,
        });
        crate::viewport::apply(&mut pass, live.viewport());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_bind_group(1, &cascade_bind, &[]);
        pass.draw(0..3, 0..1);
        Ok(shadows)
    }
}

pub struct PlateMode {
    plated: bool,
    dynamic: Vec<bool>,
    casters: PlateCasters,
    proxies: Vec<Instance>,
    proxy_meshes: Vec<MeshHandle>,
}

impl Default for PlateMode {
    fn default() -> Self {
        Self {
            plated: false,
            dynamic: Vec::new(),
            casters: PlateCasters::Meshes,
            proxies: Vec::new(),
            proxy_meshes: Vec::new(),
        }
    }
}

fn unit_cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        for side in [-1.0f32, 1.0] {
            let mut normal = [0.0; 3];
            normal[axis] = side;
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let base = positions.len() as u32;
            for (a, b) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                let mut corner = [0.0; 3];
                corner[axis] = side * 0.5;
                corner[u] = a;
                corner[v] = b;
                positions.push(corner);
                normals.push(normal);
            }
            if side > 0.0 {
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            } else {
                indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
    }
    (positions, normals, indices)
}

fn hull(points: &[[f32; 3]], triangles: &[[u32; 3]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for triangle in triangles {
        let [a, b, c] = triangle.map(|corner| points[corner as usize]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        let base = positions.len() as u32;
        for corner in [a, b, c] {
            positions.push(corner);
            normals.push(n.map(|v| v / length));
        }
        indices.extend([base, base + 1, base + 2]);
    }
    (positions, normals, indices)
}

fn upload_shape(
    renderer: &mut Renderer,
    (positions, normals, indices): (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>),
) -> Result<MeshHandle, String> {
    let count = positions.len();
    let tangents = vec![[1.0, 0.0, 0.0, 1.0]; count];
    let uvs = vec![[0.0; 2]; count];
    renderer.upload_mesh(MeshData {
        positions: &positions,
        normals: &normals,
        tangents: &tangents,
        uvs: &uvs,
        uvs1: None,
        alpha: None,
        indices: &indices,
    })
}

impl PlateMode {
    pub fn load(
        &mut self,
        renderer: &mut Renderer,
        scene: &Scene,
        reload: bool,
    ) -> Result<(), String> {
        self.dynamic = (0..scene.objects.len())
            .map(|index| scene.dynamic(index))
            .collect();
        if !reload {
            return Ok(());
        }
        self.release_proxies(renderer)?;
        let settings = renderer.plates().map(Plates::settings).unwrap_or_default();
        match &scene.plates {
            Some(plates) if plates.manifest.is_some() => {
                let plate = read_plate(&plates.dir)?;
                renderer.set_plates(Some(&plate), settings)?;
                renderer.set_plate_hour(scene.sun_or_dark().hour);
                self.plated = true;
                self.casters = plates.casters;
                if let (PlateCasters::Proxies, Some(proxies)) = (plates.casters, &plates.proxies) {
                    let mut cube = None;
                    for proxy in &proxies.items {
                        let (mesh, model) = match &proxy.shape {
                            ProxyShape::Box { model, .. } => {
                                let mesh = match cube {
                                    Some(mesh) => mesh,
                                    None => {
                                        let mesh = upload_shape(renderer, unit_cube())?;
                                        self.proxy_meshes.push(mesh);
                                        cube = Some(mesh);
                                        mesh
                                    }
                                };
                                (mesh, *model)
                            }
                            ProxyShape::Hull { points, triangles } => {
                                let mesh = upload_shape(renderer, hull(points, triangles))?;
                                self.proxy_meshes.push(mesh);
                                (mesh, pfx_load::scene::IDENTITY)
                            }
                        };
                        self.proxies.push(Instance::new(mesh, model, 0, proxy.id));
                    }
                }
            }
            _ => {
                renderer.set_plates(None, settings)?;
                renderer.set_plate_casters(&[]);
                self.plated = false;
            }
        }
        Ok(())
    }

    pub fn plated(&self) -> bool {
        self.plated
    }

    pub fn baked(&self, draw: &Draw) -> bool {
        self.plated
            && !draw
                .owner
                .is_some_and(|owner| self.dynamic.get(owner).copied().unwrap_or(false))
    }

    pub fn cast(&self, renderer: &mut Renderer, stand_ins: &[Instance]) -> Result<(), String> {
        if !self.plated {
            return Ok(());
        }
        match self.casters {
            PlateCasters::Meshes => renderer.set_plate_casters(stand_ins),
            PlateCasters::Proxies => renderer.set_plate_casters(&self.proxies),
            PlateCasters::None => renderer.set_plate_casters(&[]),
        }
        Ok(())
    }

    fn release_proxies(&mut self, renderer: &mut Renderer) -> Result<(), String> {
        self.proxies.clear();
        for mesh in self.proxy_meshes.drain(..) {
            renderer.release_mesh(mesh)?;
        }
        Ok(())
    }

    pub fn release(mut self, renderer: &mut Renderer) -> Result<(), String> {
        self.release_proxies(renderer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plate_shaders_validate() {
        for source in [plate_source(), PLATE_SHADOW_WGSL.to_string()] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn the_uniform_layout_matches_the_shader() {
        let module = naga::front::wgsl::parse_str(&plate_source()).unwrap();
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("PlateUniform"))
            .unwrap();
        let naga::TypeInner::Struct { span, .. } = ty.inner else {
            panic!("a struct");
        };
        assert_eq!(span as usize, UNIFORM_BYTES);
    }
}
