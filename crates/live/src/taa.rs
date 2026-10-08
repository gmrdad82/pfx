use pfx_gpu::wgpu;

pub const CYCLE: u32 = 8;
pub const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
pub const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
pub const REACTIVE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
pub const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Snorm;

const HISTORY_VALID: u32 = 1;
const HAS_ID: u32 = 2;
const HAS_NORMAL: u32 = 4;
const WORKGROUP: u32 = 8;

const OFFSETS: [[f32; 2]; CYCLE as usize] = offsets();

const fn halton_f64(mut index: u32, base: u32) -> f64 {
    let mut fraction = 1.0f64;
    let mut result = 0.0f64;
    let step = base as f64;
    while index > 0 {
        fraction /= step;
        result += fraction * (index % base) as f64;
        index /= base;
    }
    result
}

const fn offsets() -> [[f32; 2]; CYCLE as usize] {
    let mut raw = [(0.0f64, 0.0f64); CYCLE as usize];
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut index = 0u32;
    while index < CYCLE {
        let x = halton_f64(index, 2);
        let y = halton_f64(index, 3);
        raw[index as usize] = (x, y);
        sum_x += x;
        sum_y += y;
        index += 1;
    }
    let mut out = [[0.0f32; 2]; CYCLE as usize];
    index = 0;
    while index < CYCLE {
        out[index as usize] = [
            (raw[index as usize].0 - sum_x / CYCLE as f64) as f32,
            (raw[index as usize].1 - sum_y / CYCLE as f64) as f32,
        ];
        index += 1;
    }
    out
}

pub fn halton(index: u32, base: u32) -> f32 {
    if base < 2 {
        return 0.0;
    }
    halton_f64(index, base) as f32
}

pub fn jitter_pixels(frame: u32) -> [f32; 2] {
    OFFSETS[(frame % CYCLE) as usize]
}

pub fn jitter_clip(frame: u32, width: u32, height: u32) -> [f32; 2] {
    if width == 0 || height == 0 {
        return [0.0, 0.0];
    }
    let pixels = jitter_pixels(frame);
    [
        pixels[0] * 2.0 / width as f32,
        pixels[1] * 2.0 / height as f32,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub history_weight: f32,
    pub variance_gamma: f32,
    pub depth_threshold: f32,
    pub normal_min_dot: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            history_weight: 1.0 - 1.0 / CYCLE as f32,
            variance_gamma: 1.0,
            depth_threshold: 0.1,
            normal_min_dot: 0.9,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub width: u32,
    pub height: u32,
    pub colour: Vec<[f32; 3]>,
    pub depth: Vec<f32>,
    pub motion: Vec<[f32; 2]>,
    pub reactive: Vec<f32>,
    pub id: Option<Vec<u32>>,
    pub normal: Option<Vec<[f32; 3]>>,
    pub exposure: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct History {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
    pub depth: Vec<f32>,
    pub id: Vec<u32>,
    pub normal: Vec<[f32; 3]>,
    pub exposure: f32,
    pub valid: bool,
}

impl History {
    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            rgba: Vec::new(),
            depth: Vec::new(),
            id: Vec::new(),
            normal: Vec::new(),
            exposure: 1.0,
            valid: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub colour: Vec<[f32; 3]>,
    pub history: History,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Size,
    Length,
    Exposure,
    Settings,
}

pub const RESOLVE_WGSL: &str = r#"
struct Uniforms {
    width: u32,
    height: u32,
    flags: u32,
    exposure: f32,
    history_exposure: f32,
    depth_threshold: f32,
    normal_min_dot: f32,
    variance_gamma: f32,
    history_weight: f32,
    history_width: u32,
    history_height: u32,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var colour_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_2d<f32>;
@group(0) @binding(3) var motion_tex: texture_2d<f32>;
@group(0) @binding(4) var id_tex: texture_2d<u32>;
@group(0) @binding(5) var normal_tex: texture_2d<f32>;
@group(0) @binding(6) var reactive_tex: texture_2d<f32>;
@group(0) @binding(7) var history_colour: texture_2d<f32>;
@group(0) @binding(8) var history_depth: texture_2d<f32>;
@group(0) @binding(9) var history_id: texture_2d<u32>;
@group(0) @binding(10) var history_normal: texture_2d<f32>;
@group(0) @binding(11) var out_colour: texture_storage_2d<rgba16float, write>;
@group(0) @binding(12) var out_depth: texture_storage_2d<r32float, write>;
@group(0) @binding(13) var out_id: texture_storage_2d<r32uint, write>;
@group(0) @binding(14) var out_normal: texture_storage_2d<rgba8snorm, write>;

fn rgb_to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}

fn ycocg_to_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn outside(uv: vec2<f32>) -> bool {
    return uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0;
}

fn clamp_pixel(p: vec2<i32>, size: vec2<i32>) -> vec2<i32> {
    return clamp(p, vec2<i32>(0), size - vec2<i32>(1));
}

fn load3(tex: texture_2d<f32>, p: vec2<i32>, size: vec2<i32>) -> vec3<f32> {
    return textureLoad(tex, clamp_pixel(p, size), 0).rgb;
}

fn bilinear3(tex: texture_2d<f32>, uv: vec2<f32>, size: vec2<u32>) -> vec3<f32> {
    let s = vec2<i32>(size);
    let pf = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(pf));
    let f = pf - floor(pf);
    let c00 = load3(tex, base, s);
    let c10 = load3(tex, base + vec2<i32>(1, 0), s);
    let c01 = load3(tex, base + vec2<i32>(0, 1), s);
    let c11 = load3(tex, base + vec2<i32>(1, 1), s);
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

fn nearest(uv: vec2<f32>, size: vec2<u32>) -> vec2<i32> {
    return clamp_pixel(vec2<i32>(floor(uv * vec2<f32>(size))), vec2<i32>(size));
}

fn clip_aabb(lo: vec3<f32>, hi: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let center = 0.5 * (hi + lo);
    let extent = max(0.5 * (hi - lo), vec3<f32>(1.0e-8));
    let delta = p - center;
    let unit = delta / extent;
    let m = max(abs(unit.x), max(abs(unit.y), abs(unit.z)));
    if (m > 1.0) {
        return center + delta / m;
    }
    return p;
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<u32>(u.width, u.height);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let pix = vec2<i32>(gid.xy);
    let uv = (vec2<f32>(gid.xy) + vec2<f32>(0.5)) / vec2<f32>(size);
    let current = textureLoad(colour_tex, pix, 0).rgb;
    let depth = textureLoad(depth_tex, pix, 0).r;
    let motion = textureLoad(motion_tex, pix, 0).rg;
    let reactive = clamp(textureLoad(reactive_tex, pix, 0).r, 0.0, 1.0);
    var pixel_id = 0u;
    if ((u.flags & 2u) != 0u) {
        pixel_id = textureLoad(id_tex, pix, 0).r;
    }
    var nrm = vec3<f32>(0.0);
    if ((u.flags & 4u) != 0u) {
        nrm = textureLoad(normal_tex, pix, 0).rgb;
    }

    var acc = vec3<f32>(0.0);
    var acc2 = vec3<f32>(0.0);
    let s = vec2<i32>(size);
    for (var oy = -1; oy <= 1; oy = oy + 1) {
        for (var ox = -1; ox <= 1; ox = ox + 1) {
            let texel = load3(colour_tex, pix + vec2<i32>(ox, oy), s) * u.exposure;
            let ycocg = rgb_to_ycocg(texel);
            acc = acc + ycocg;
            acc2 = acc2 + ycocg * ycocg;
        }
    }
    let mean = acc / 9.0;
    let sigma = sqrt(max(acc2 / 9.0 - mean * mean, vec3<f32>(0.0))) * u.variance_gamma;
    let lo = mean - sigma;
    let hi = mean + sigma;
    let current_e = current * u.exposure;
    var history_e = current_e;
    var weight = 0.0;
    if ((u.flags & 1u) != 0u) {
        let prev = uv + motion;
        if (!outside(prev)) {
            let stored = max(u.history_exposure, 1.0e-8);
            let history_size = vec2<u32>(u.history_width, u.history_height);
            let history_hdr = bilinear3(history_colour, prev, history_size);
            history_e = (history_hdr * stored) * (u.exposure / stored);
            var accept = true;
            let at = nearest(prev, history_size);
            let previous_depth = textureLoad(history_depth, at, 0).r;
            let scale = max(max(abs(depth), abs(previous_depth)), 1.0e-5);
            if (abs(depth - previous_depth) / scale > u.depth_threshold) {
                accept = false;
            }
            if ((u.flags & 2u) != 0u) {
                let previous_id = textureLoad(history_id, at, 0).r;
                if (previous_id != pixel_id) {
                    accept = false;
                }
            }
            if ((u.flags & 4u) != 0u) {
                let previous_normal = textureLoad(history_normal, at, 0).rgb;
                let current_len2 = dot(nrm, nrm);
                let previous_len2 = dot(previous_normal, previous_normal);
                if (current_len2 > 1.0e-12 && previous_len2 > 1.0e-12) {
                    let aligned = dot(nrm, previous_normal) * inverseSqrt(current_len2 * previous_len2);
                    if (aligned < u.normal_min_dot) {
                        accept = false;
                    }
                }
            }
            if (accept) {
                history_e = ycocg_to_rgb(clip_aabb(lo, hi, rgb_to_ycocg(history_e)));
                weight = u.history_weight * (1.0 - reactive);
            }
        }
    }
    let resolved_e = history_e * weight + current_e * (1.0 - weight);
    let resolved = resolved_e / u.exposure;
    textureStore(out_colour, pix, vec4<f32>(resolved, u.exposure));
    textureStore(out_depth, pix, vec4<f32>(depth, 0.0, 0.0, 0.0));
    textureStore(out_id, pix, vec4<u32>(pixel_id, 0u, 0u, 0u));
    textureStore(out_normal, pix, vec4<f32>(nrm, 0.0));
}
"#;

fn finite_settings(settings: &Settings) -> Result<(f32, f32, f32, f32), Error> {
    if !settings.history_weight.is_finite()
        || !settings.variance_gamma.is_finite()
        || !settings.depth_threshold.is_finite()
        || !settings.normal_min_dot.is_finite()
    {
        return Err(Error::Settings);
    }
    Ok((
        settings.history_weight.clamp(0.0, 1.0),
        settings.variance_gamma.max(0.0),
        settings.depth_threshold.max(0.0),
        settings.normal_min_dot,
    ))
}

fn pixel_count(width: u32, height: u32) -> Result<usize, Error> {
    usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| Error::Size)
}

fn rgb_to_ycocg(c: [f32; 3]) -> [f32; 3] {
    [
        0.25 * c[0] + 0.5 * c[1] + 0.25 * c[2],
        0.5 * c[0] - 0.5 * c[2],
        -0.25 * c[0] + 0.5 * c[1] - 0.25 * c[2],
    ]
}

fn ycocg_to_rgb(c: [f32; 3]) -> [f32; 3] {
    [c[0] + c[1] - c[2], c[0] + c[2], c[0] - c[1] - c[2]]
}

fn outside(uv: [f32; 2]) -> bool {
    uv[0] < 0.0 || uv[1] < 0.0 || uv[0] >= 1.0 || uv[1] >= 1.0
}

fn clamp_index(value: i32, size: i32) -> i32 {
    value.clamp(0, size - 1)
}

fn load3(tex: &[[f32; 3]], width: i32, height: i32, x: i32, y: i32) -> [f32; 3] {
    let x = clamp_index(x, width);
    let y = clamp_index(y, height);
    tex[(y as u32 * width as u32 + x as u32) as usize]
}

fn bilinear3(tex: &[[f32; 3]], width: u32, height: u32, uv: [f32; 2]) -> [f32; 3] {
    let w = width as i32;
    let h = height as i32;
    let px = uv[0] * width as f32 - 0.5;
    let py = uv[1] * height as f32 - 0.5;
    let x0 = px.floor();
    let y0 = py.floor();
    let fx = px - x0;
    let fy = py - y0;
    let x0 = x0 as i32;
    let y0 = y0 as i32;
    let c00 = load3(tex, w, h, x0, y0);
    let c10 = load3(tex, w, h, x0 + 1, y0);
    let c01 = load3(tex, w, h, x0, y0 + 1);
    let c11 = load3(tex, w, h, x0 + 1, y0 + 1);
    let mut top = [0.0; 3];
    let mut bottom = [0.0; 3];
    let mut out = [0.0; 3];
    for channel in 0..3 {
        top[channel] = c00[channel] * (1.0 - fx) + c10[channel] * fx;
        bottom[channel] = c01[channel] * (1.0 - fx) + c11[channel] * fx;
        out[channel] = top[channel] * (1.0 - fy) + bottom[channel] * fy;
    }
    out
}

fn nearest(uv: f32, size: u32) -> i32 {
    clamp_index((uv * size as f32).floor() as i32, size as i32)
}

fn clip_aabb(lo: [f32; 3], hi: [f32; 3], p: [f32; 3]) -> [f32; 3] {
    let mut center = [0.0; 3];
    let mut extent = [0.0; 3];
    let mut delta = [0.0; 3];
    let mut unit = [0.0; 3];
    for channel in 0..3 {
        center[channel] = 0.5 * (hi[channel] + lo[channel]);
        extent[channel] = (0.5 * (hi[channel] - lo[channel])).max(1.0e-8);
        delta[channel] = p[channel] - center[channel];
        unit[channel] = delta[channel] / extent[channel];
    }
    let m = unit[0].abs().max(unit[1].abs()).max(unit[2].abs());
    if m > 1.0 {
        [
            center[0] + delta[0] / m,
            center[1] + delta[1] / m,
            center[2] + delta[2] / m,
        ]
    } else {
        p
    }
}

fn depth_rejects(current: f32, previous: f32, threshold: f32) -> bool {
    let scale = current.abs().max(previous.abs()).max(1.0e-5);
    (current - previous).abs() / scale > threshold
}

fn normal_rejects(current: [f32; 3], previous: [f32; 3], min_dot: f32) -> bool {
    let current_len =
        (current[0] * current[0] + current[1] * current[1] + current[2] * current[2]).sqrt();
    let previous_len =
        (previous[0] * previous[0] + previous[1] * previous[1] + previous[2] * previous[2]).sqrt();
    if current_len <= 1.0e-6 || previous_len <= 1.0e-6 {
        return false;
    }
    let aligned = (current[0] / current_len) * (previous[0] / previous_len)
        + (current[1] / current_len) * (previous[1] / previous_len)
        + (current[2] / current_len) * (previous[2] / previous_len);
    aligned < min_dot
}

pub fn resolve(inputs: &Inputs, history: &History, settings: &Settings) -> Result<Output, Error> {
    let (history_weight, variance_gamma, depth_threshold, normal_min_dot) =
        finite_settings(settings)?;
    if !inputs.exposure.is_finite() || inputs.exposure <= 0.0 {
        return Err(Error::Exposure);
    }
    let n = pixel_count(inputs.width, inputs.height)?;
    if inputs.width == 0 || inputs.height == 0 {
        return Ok(Output {
            colour: Vec::new(),
            history: History::empty(),
        });
    }
    if inputs.colour.len() != n
        || inputs.depth.len() != n
        || inputs.motion.len() != n
        || inputs.reactive.len() != n
        || inputs.id.as_ref().is_some_and(|id| id.len() != n)
        || inputs
            .normal
            .as_ref()
            .is_some_and(|normal| normal.len() != n)
    {
        return Err(Error::Length);
    }
    let stored = pixel_count(history.width, history.height)?;
    let use_history = history.valid
        && history.width > 0
        && history.height > 0
        && history.rgba.len() == stored
        && history.depth.len() == stored
        && history.id.len() == stored
        && history.normal.len() == stored
        && history.exposure.is_finite()
        && history.exposure > 0.0;
    let stored_exposure = if use_history {
        history.exposure.max(1.0e-8)
    } else {
        1.0
    };
    let history_hdr: Vec<[f32; 3]> = if use_history {
        history
            .rgba
            .iter()
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect()
    } else {
        Vec::new()
    };
    let width = inputs.width as i32;
    let height = inputs.height as i32;
    let mut colour = Vec::with_capacity(n);
    let mut rgba = Vec::with_capacity(n);
    let mut depth_out = Vec::with_capacity(n);
    let mut id_out = Vec::with_capacity(n);
    let mut normal_out = Vec::with_capacity(n);
    for y in 0..inputs.height {
        for x in 0..inputs.width {
            let index = (y * inputs.width + x) as usize;
            let uv = [
                (x as f32 + 0.5) / inputs.width as f32,
                (y as f32 + 0.5) / inputs.height as f32,
            ];
            let current = inputs.colour[index];
            let depth = inputs.depth[index];
            let motion = inputs.motion[index];
            let reactive = inputs.reactive[index].clamp(0.0, 1.0);
            let pixel_id = inputs.id.as_ref().map(|id| id[index]).unwrap_or(0);
            let nrm = inputs
                .normal
                .as_ref()
                .map(|normal| normal[index])
                .unwrap_or([0.0; 3]);
            let mut acc = [0.0; 3];
            let mut acc2 = [0.0; 3];
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let texel = load3(&inputs.colour, width, height, x as i32 + ox, y as i32 + oy);
                    let exposed = [
                        texel[0] * inputs.exposure,
                        texel[1] * inputs.exposure,
                        texel[2] * inputs.exposure,
                    ];
                    let ycocg = rgb_to_ycocg(exposed);
                    for channel in 0..3 {
                        acc[channel] += ycocg[channel];
                        acc2[channel] += ycocg[channel] * ycocg[channel];
                    }
                }
            }
            let mut mean = [0.0; 3];
            let mut lo = [0.0; 3];
            let mut hi = [0.0; 3];
            for channel in 0..3 {
                mean[channel] = acc[channel] / 9.0;
                let variance = (acc2[channel] / 9.0 - mean[channel] * mean[channel]).max(0.0);
                let sigma = variance.sqrt() * variance_gamma;
                lo[channel] = mean[channel] - sigma;
                hi[channel] = mean[channel] + sigma;
            }
            let current_e = [
                current[0] * inputs.exposure,
                current[1] * inputs.exposure,
                current[2] * inputs.exposure,
            ];
            let mut history_e = current_e;
            let mut weight = 0.0;
            if use_history {
                let prev = [uv[0] + motion[0], uv[1] + motion[1]];
                if !outside(prev) {
                    let history_hdr = bilinear3(&history_hdr, history.width, history.height, prev);
                    history_e = [
                        (history_hdr[0] * stored_exposure) * (inputs.exposure / stored_exposure),
                        (history_hdr[1] * stored_exposure) * (inputs.exposure / stored_exposure),
                        (history_hdr[2] * stored_exposure) * (inputs.exposure / stored_exposure),
                    ];
                    let at_x = nearest(prev[0], history.width);
                    let at_y = nearest(prev[1], history.height);
                    let at = (at_y as u32 * history.width + at_x as u32) as usize;
                    let mut accept = !depth_rejects(depth, history.depth[at], depth_threshold);
                    if accept && inputs.id.is_some() && history.id[at] != pixel_id {
                        accept = false;
                    }
                    if accept
                        && inputs.normal.is_some()
                        && normal_rejects(nrm, history.normal[at], normal_min_dot)
                    {
                        accept = false;
                    }
                    if accept {
                        history_e = ycocg_to_rgb(clip_aabb(lo, hi, rgb_to_ycocg(history_e)));
                        weight = history_weight * (1.0 - reactive);
                    }
                }
            }
            let resolved_e = [
                history_e[0] * weight + current_e[0] * (1.0 - weight),
                history_e[1] * weight + current_e[1] * (1.0 - weight),
                history_e[2] * weight + current_e[2] * (1.0 - weight),
            ];
            let resolved = [
                resolved_e[0] / inputs.exposure,
                resolved_e[1] / inputs.exposure,
                resolved_e[2] / inputs.exposure,
            ];
            colour.push(resolved);
            rgba.push([resolved[0], resolved[1], resolved[2], inputs.exposure]);
            depth_out.push(depth);
            id_out.push(pixel_id);
            normal_out.push(nrm);
        }
    }
    Ok(Output {
        colour,
        history: History {
            width: inputs.width,
            height: inputs.height,
            rgba,
            depth: depth_out,
            id: id_out,
            normal: normal_out,
            exposure: inputs.exposure,
            valid: true,
        },
    })
}

struct Slot {
    colour: wgpu::Texture,
    colour_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    id_view: wgpu::TextureView,
    normal_view: wgpu::TextureView,
    textures: [wgpu::Texture; 3],
}

pub struct GpuInputs<'a> {
    pub colour: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    pub reactive: &'a wgpu::TextureView,
    pub id: Option<&'a wgpu::TextureView>,
    pub normal: Option<&'a wgpu::TextureView>,
    pub exposure: f32,
}

pub struct Pass {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    uniform: wgpu::Buffer,
    dummy_id: wgpu::Texture,
    dummy_id_view: wgpu::TextureView,
    dummy_normal: wgpu::Texture,
    dummy_normal_view: wgpu::TextureView,
    slots: [Option<Slot>; 2],
    source: usize,
    width: u32,
    height: u32,
    viewport: (u32, u32),
    history_size: (u32, u32),
    restarts: u64,
    exposure: f32,
    valid: bool,
    dummies_ready: bool,
}

fn texture(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
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
    (texture, view)
}

fn float_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uint_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
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

fn history_slot(device: &wgpu::Device, width: u32, height: u32, index: usize) -> Slot {
    let usage = wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::STORAGE_BINDING
        | wgpu::TextureUsages::COPY_SRC;
    let (colour, colour_view) = texture(
        device,
        if index == 0 {
            "taa history colour 0"
        } else {
            "taa history colour 1"
        },
        width,
        height,
        COLOUR_FORMAT,
        usage,
    );
    let (depth, depth_view) = texture(
        device,
        if index == 0 {
            "taa history depth 0"
        } else {
            "taa history depth 1"
        },
        width,
        height,
        DEPTH_FORMAT,
        usage,
    );
    let (id, id_view) = texture(
        device,
        if index == 0 {
            "taa history id 0"
        } else {
            "taa history id 1"
        },
        width,
        height,
        ID_FORMAT,
        usage,
    );
    let (normal, normal_view) = texture(
        device,
        if index == 0 {
            "taa history normal 0"
        } else {
            "taa history normal 1"
        },
        width,
        height,
        NORMAL_FORMAT,
        usage,
    );
    Slot {
        colour,
        colour_view,
        depth_view,
        id_view,
        normal_view,
        textures: [depth, id, normal],
    }
}

impl Pass {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa resolve"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                float_entry(1),
                float_entry(2),
                float_entry(3),
                uint_entry(4),
                float_entry(5),
                float_entry(6),
                float_entry(7),
                float_entry(8),
                uint_entry(9),
                float_entry(10),
                storage_entry(11, COLOUR_FORMAT),
                storage_entry(12, DEPTH_FORMAT),
                storage_entry(13, ID_FORMAT),
                storage_entry(14, NORMAL_FORMAT),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("taa resolve"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("taa resolve"),
            source: wgpu::ShaderSource::Wgsl(RESOLVE_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("taa resolve"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("resolve"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa uniforms"),
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (dummy_id, dummy_id_view) = texture(
            device,
            "taa dummy id",
            1,
            1,
            ID_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let (dummy_normal, dummy_normal_view) = texture(
            device,
            "taa dummy normal",
            1,
            1,
            NORMAL_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        Self {
            layout,
            pipeline,
            uniform,
            dummy_id,
            dummy_id_view,
            dummy_normal,
            dummy_normal_view,
            slots: [None, None],
            source: 0,
            width: 0,
            height: 0,
            viewport: (0, 0),
            history_size: (0, 0),
            restarts: 0,
            exposure: 1.0,
            valid: false,
            dummies_ready: false,
        }
    }

    pub fn width(&self) -> u32 {
        self.viewport.0
    }

    pub fn height(&self) -> u32 {
        self.viewport.1
    }

    pub fn capacity(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn history_valid(&self) -> bool {
        self.valid
    }

    pub fn textures(&self) -> Vec<wgpu::Texture> {
        self.slots
            .iter()
            .flatten()
            .flat_map(|slot| std::iter::once(&slot.colour).chain(&slot.textures))
            .cloned()
            .collect()
    }

    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == self.width && height == self.height && self.slots[0].is_some() {
            self.viewport = (width, height);
            return;
        }
        self.invalidate();
        self.source = 0;
        self.exposure = 1.0;
        if width == 0 || height == 0 {
            self.slots = [None, None];
            self.width = 0;
            self.height = 0;
            self.viewport = (0, 0);
            self.history_size = (0, 0);
            return;
        }
        self.slots = [
            Some(history_slot(device, width, height, 0)),
            Some(history_slot(device, width, height, 1)),
        ];
        self.width = width;
        self.height = height;
        self.viewport = (width, height);
        self.history_size = (width, height);
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        crate::viewport::check([width, height], [self.width, self.height])?;
        self.viewport = (width, height);
        Ok(())
    }

    pub fn invalidate(&mut self) {
        if self.valid {
            self.restarts += 1;
        }
        self.valid = false;
    }

    pub fn history_view(&self) -> Option<&wgpu::TextureView> {
        if !self.valid {
            return None;
        }
        self.slots[self.source]
            .as_ref()
            .map(|slot| &slot.colour_view)
    }

    pub fn resolve(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &GpuInputs<'_>,
        settings: &Settings,
    ) -> Result<(), String> {
        self.resolve_timed(device, queue, encoder, inputs, settings, None)
    }

    pub fn resolve_timed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &GpuInputs<'_>,
        settings: &Settings,
        mut profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), String> {
        let (width, height) = self.viewport;
        if width == 0 || height == 0 || self.slots[0].is_none() {
            return Err("viewport is empty".into());
        }
        if !inputs.exposure.is_finite() || inputs.exposure <= 0.0 {
            return Err("exposure must be positive".into());
        }
        let (history_weight, variance_gamma, depth_threshold, normal_min_dot) =
            finite_settings(settings).map_err(|_| "settings must be finite".to_string())?;
        if !self.dummies_ready {
            queue.write_texture(
                self.dummy_id.as_image_copy(),
                &[0, 0, 0, 0],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                self.dummy_id.size(),
            );
            queue.write_texture(
                self.dummy_normal.as_image_copy(),
                &[0u8; 4],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                self.dummy_normal.size(),
            );
            self.dummies_ready = true;
        }
        let mut flags = 0;
        if self.valid {
            flags |= HISTORY_VALID;
        }
        if inputs.id.is_some() {
            flags |= HAS_ID;
        }
        if inputs.normal.is_some() {
            flags |= HAS_NORMAL;
        }
        let mut bytes = Vec::with_capacity(44);
        for value in [
            width.to_le_bytes(),
            height.to_le_bytes(),
            flags.to_le_bytes(),
        ] {
            bytes.extend_from_slice(&value);
        }
        for value in [
            inputs.exposure,
            if self.valid { self.exposure } else { 1.0 },
            depth_threshold,
            normal_min_dot,
            variance_gamma,
            history_weight,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [self.history_size.0, self.history_size.1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        let dest = 1 - self.source;
        let source = self.slots[self.source].as_ref().unwrap();
        let target = self.slots[dest].as_ref().unwrap();
        let id_view = inputs.id.unwrap_or(&self.dummy_id_view);
        let normal_view = inputs.normal.unwrap_or(&self.dummy_normal_view);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("taa resolve"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                view_entry(1, inputs.colour),
                view_entry(2, inputs.depth),
                view_entry(3, inputs.motion),
                view_entry(4, id_view),
                view_entry(5, normal_view),
                view_entry(6, inputs.reactive),
                view_entry(7, &source.colour_view),
                view_entry(8, &source.depth_view),
                view_entry(9, &source.id_view),
                view_entry(10, &source.normal_view),
                view_entry(11, &target.colour_view),
                view_entry(12, &target.depth_view),
                view_entry(13, &target.id_view),
                view_entry(14, &target.normal_view),
            ],
        });
        let timing = profiler
            .as_deref_mut()
            .and_then(|timer| timer.pass("TAA resolve"));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("taa resolve"),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|timer| timer.compute_writes(timing)),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(WORKGROUP), height.div_ceil(WORKGROUP), 1);
        }
        self.source = dest;
        self.exposure = inputs.exposure;
        self.valid = true;
        self.history_size = (width, height);
        Ok(())
    }

    pub fn read_colour(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<[f32; 4]>, String> {
        if !self.valid {
            return Err("history is empty".into());
        }
        let slot = self.slots[self.source].as_ref().ok_or("history is empty")?;
        read_rgba16(
            device,
            queue,
            &slot.colour,
            self.history_size.0,
            self.history_size.1,
        )
    }
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn read_rgba16(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<[f32; 4]>, String> {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let raw = width * 8;
    let row = raw.div_ceil(align) * align;
    let size = u64::from(row) * u64::from(height);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("taa readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("taa readback"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let (send, receive) = std::sync::mpsc::channel();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| format!("{error:?}"))?;
    receive
        .recv()
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let mapped = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for padded in mapped.chunks_exact(row as usize) {
        for texel in padded[..raw as usize].chunks_exact(8) {
            pixels.push([
                half::f16::from_le_bytes(texel[0..2].try_into().unwrap()).to_f32(),
                half::f16::from_le_bytes(texel[2..4].try_into().unwrap()).to_f32(),
                half::f16::from_le_bytes(texel[4..6].try_into().unwrap()).to_f32(),
                half::f16::from_le_bytes(texel[6..8].try_into().unwrap()).to_f32(),
            ]);
        }
    }
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_near(actual: f32, expected: f32, tol: f32) {
        assert!(
            (actual - expected).abs() <= tol,
            "{actual} != {expected} within {tol}"
        );
    }

    fn pattern(reactive: f32, depth: f32, motion: [f32; 2], exposure: f32) -> Inputs {
        let values = [1.0, 0.0, 1.0, 0.0, 0.25, 0.0, 1.0, 0.0, 1.0];
        Inputs {
            width: 3,
            height: 3,
            colour: values.into_iter().map(|v| [v, v, v]).collect(),
            depth: (0..9).map(|i| if i == 4 { depth } else { 0.5 }).collect(),
            motion: (0..9)
                .map(|i| if i == 4 { motion } else { [0.0, 0.0] })
                .collect(),
            reactive: (0..9)
                .map(|i| if i == 4 { reactive } else { 0.0 })
                .collect(),
            id: None,
            normal: None,
            exposure,
        }
    }

    fn filled(hdr: f32, exposure: f32, depth: f32) -> History {
        History {
            width: 3,
            height: 3,
            rgba: vec![[hdr, hdr, hdr, exposure]; 9],
            depth: vec![depth; 9],
            id: vec![0; 9],
            normal: vec![[0.0, 0.0, 1.0]; 9],
            exposure,
            valid: true,
        }
    }

    #[test]
    fn halton_values_and_cycle() {
        assert_eq!(halton(0, 2).to_bits(), 0.0f32.to_bits());
        assert_eq!(halton(1, 2).to_bits(), 0.5f32.to_bits());
        assert_eq!(halton(2, 2).to_bits(), 0.25f32.to_bits());
        assert_eq!(halton(3, 2).to_bits(), 0.75f32.to_bits());
        assert_eq!(halton(4, 2).to_bits(), 0.125f32.to_bits());
        assert_eq!(halton(7, 2).to_bits(), 0.875f32.to_bits());
        assert_near(halton(1, 3), 1.0 / 3.0, 1e-6);
        assert_near(halton(2, 3), 2.0 / 3.0, 1e-6);
        assert_near(halton(3, 3), 1.0 / 9.0, 1e-6);
        assert_near(halton(4, 3), 4.0 / 9.0, 1e-6);
        assert_near(halton(5, 3), 7.0 / 9.0, 1e-6);
        assert_near(halton(6, 3), 2.0 / 9.0, 1e-6);
        assert_near(halton(7, 3), 5.0 / 9.0, 1e-6);
        assert_eq!(halton(1, 0).to_bits(), 0.0f32.to_bits());
        let expected_x = [
            -0.4375f32, 0.0625, -0.1875, 0.3125, -0.3125, 0.1875, -0.0625, 0.4375,
        ];
        for frame in 0..CYCLE {
            assert_eq!(
                jitter_pixels(frame)[0].to_bits(),
                expected_x[frame as usize].to_bits()
            );
            assert_eq!(
                jitter_pixels(frame)[0].to_bits(),
                jitter_pixels(frame + CYCLE)[0].to_bits()
            );
            assert_eq!(
                jitter_pixels(frame)[1].to_bits(),
                jitter_pixels(frame + CYCLE * 3)[1].to_bits()
            );
        }
        for frame in 0..CYCLE {
            for other in 0..frame {
                let a = jitter_pixels(frame);
                let b = jitter_pixels(other);
                assert!(a[0].to_bits() != b[0].to_bits() || a[1].to_bits() != b[1].to_bits());
            }
        }
    }

    #[test]
    fn jitter_averages_to_zero() {
        let mut sum = [0.0f32; 2];
        for frame in 0..CYCLE {
            let jitter = jitter_pixels(frame);
            assert!(jitter[0].abs() <= 0.5 && jitter[1].abs() <= 0.5);
            sum[0] += jitter[0];
            sum[1] += jitter[1];
            let clip = jitter_clip(frame, 320, 180);
            assert_near(clip[0], jitter[0] * 2.0 / 320.0, 1e-7);
            assert_near(clip[1], jitter[1] * 2.0 / 180.0, 1e-7);
        }
        assert_near(sum[0], 0.0, 1e-6);
        assert_near(sum[1], 0.0, 1e-6);
        assert_eq!(jitter_clip(0, 0, 10)[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(jitter_clip(0, 10, 0)[1].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn ycocg_roundtrip_and_clip() {
        let colour = [0.2, 0.5, 0.8];
        let back = ycocg_to_rgb(rgb_to_ycocg(colour));
        for channel in 0..3 {
            assert_near(back[channel], colour[channel], 1e-6);
        }
        let inside = clip_aabb([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.25, 0.5, 0.75]);
        assert_eq!(inside[0].to_bits(), 0.25f32.to_bits());
        let clipped = clip_aabb([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
        assert_near(clipped[0], 1.0, 1e-5);
        assert_near(clipped[1], 0.0, 1e-5);
    }

    fn coverage(pixel: u32, edge: f32) -> f32 {
        let start = pixel as f32;
        let end = start + 1.0;
        if end <= edge {
            0.0
        } else if start >= edge {
            1.0
        } else {
            end - edge
        }
    }

    fn edge_frame(frame: u32) -> Inputs {
        let width = 8;
        let height = 4;
        let edge = 3.35;
        let jitter = jitter_pixels(frame);
        let mut colour = Vec::with_capacity((width * height) as usize);
        for _y in 0..height {
            for x in 0..width {
                let sample = x as f32 + 0.5 + jitter[0];
                let value = if sample >= edge { 1.0 } else { 0.0 };
                colour.push([value, value, value]);
            }
        }
        Inputs {
            width,
            height,
            colour,
            depth: vec![1.0; (width * height) as usize],
            motion: vec![[0.0, 0.0]; (width * height) as usize],
            reactive: vec![0.0; (width * height) as usize],
            id: None,
            normal: None,
            exposure: 1.0,
        }
    }

    #[test]
    fn static_jittered_edge_converges() {
        let edge = 3.35;
        let index = 8 + 3;
        let mut saw_black = false;
        let mut saw_white = false;
        for frame in 0..CYCLE {
            let value = edge_frame(frame).colour[index][0];
            saw_black |= value.to_bits() == 0.0f32.to_bits();
            saw_white |= value.to_bits() == 1.0f32.to_bits();
        }
        assert!(saw_black && saw_white);
        let mut history = History::empty();
        let mut tail = Vec::new();
        for frame in 0..64 {
            let output = resolve(&edge_frame(frame), &history, &Settings::default()).unwrap();
            if frame >= 56 {
                tail.push(output.colour[index][0]);
            }
            history = output.history;
        }
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        let expected = coverage(3, edge);
        assert_near(mean, expected, 0.05);
        assert!(mean > 0.4 && mean < 0.9);
        let flat_black = history.rgba[8][0];
        let flat_white = history.rgba[8 + 7][0];
        assert_near(flat_black, coverage(0, edge), 0.02);
        assert_near(flat_white, coverage(7, edge), 0.02);
    }

    #[test]
    fn depth_discontinuity_rejects_history() {
        let current = pattern(0.0, 0.8, [0.0, 0.0], 1.0);
        let rejected = resolve(&current, &filled(0.75, 1.0, 0.2), &Settings::default()).unwrap();
        assert_near(rejected.colour[4][0], 0.25, 1e-5);
        let kept = resolve(
            &pattern(0.0, 0.5, [0.0, 0.0], 1.0),
            &filled(0.75, 1.0, 0.5),
            &Settings::default(),
        )
        .unwrap();
        assert_near(kept.colour[4][0], 0.6875, 1e-5);
        let mut resized = filled(0.75, 1.0, 0.5);
        resized.width = 2;
        let dropped = resolve(
            &pattern(0.0, 0.5, [0.0, 0.0], 1.0),
            &resized,
            &Settings::default(),
        )
        .unwrap();
        assert_near(dropped.colour[4][0], 0.25, 1e-5);
    }

    #[test]
    fn a_history_of_another_size_is_resampled_not_dropped() {
        let smaller = History {
            width: 2,
            height: 2,
            rgba: vec![[0.75, 0.75, 0.75, 1.0]; 4],
            depth: vec![0.5; 4],
            id: vec![0; 4],
            normal: vec![[0.0, 0.0, 1.0]; 4],
            exposure: 1.0,
            valid: true,
        };
        let kept = resolve(
            &pattern(0.0, 0.5, [0.0, 0.0], 1.0),
            &smaller,
            &Settings::default(),
        )
        .unwrap();
        assert_near(kept.colour[4][0], 0.6875, 1e-5);
        assert_eq!((kept.history.width, kept.history.height), (3, 3));
        let mut ramp = smaller.clone();
        ramp.width = 4;
        ramp.height = 1;
        ramp.rgba = (0..4)
            .map(|x| {
                let v = x as f32 / 3.0;
                [v, v, v, 1.0]
            })
            .collect();
        let mut current = pattern(1.0, 0.5, [0.0, 0.0], 1.0);
        current.reactive = vec![0.0; 9];
        let settings = Settings {
            history_weight: 1.0,
            variance_gamma: 1.0e6,
            ..Settings::default()
        };
        let resampled = resolve(&current, &ramp, &settings).unwrap();
        for x in 0..3 {
            let u = (x as f32 + 0.5) / 3.0 * 4.0 - 0.5;
            assert_near(resampled.colour[x][0], u.clamp(0.0, 3.0) / 3.0, 1e-5);
        }
    }

    #[test]
    fn reactive_mask_limits_history() {
        let history = filled(0.75, 1.0, 0.5);
        let open = resolve(
            &pattern(0.0, 0.5, [0.0, 0.0], 1.0),
            &history,
            &Settings::default(),
        )
        .unwrap();
        let shut = resolve(
            &pattern(1.0, 0.5, [0.0, 0.0], 1.0),
            &history,
            &Settings::default(),
        )
        .unwrap();
        let over = resolve(
            &pattern(2.0, 0.5, [0.0, 0.0], 1.0),
            &history,
            &Settings::default(),
        )
        .unwrap();
        let half = resolve(
            &pattern(0.5, 0.5, [0.0, 0.0], 1.0),
            &history,
            &Settings::default(),
        )
        .unwrap();
        assert_near(open.colour[4][0], 0.6875, 1e-5);
        assert_near(shut.colour[4][0], 0.25, 1e-5);
        assert_near(over.colour[4][0], 0.25, 1e-5);
        assert!((half.colour[4][0] - 0.25).abs() < (open.colour[4][0] - 0.25).abs());
        assert!((half.colour[4][0] - 0.25).abs() > 1e-4);
    }

    #[test]
    fn id_and_normal_rejection_and_exposure() {
        let mut current = pattern(0.0, 0.5, [0.0, 0.0], 1.0);
        current.id = Some(vec![7; 9]);
        let mut matched = filled(0.75, 1.0, 0.5);
        matched.id = vec![7; 9];
        let kept = resolve(&current, &matched, &Settings::default()).unwrap();
        assert_near(kept.colour[4][0], 0.6875, 1e-5);
        matched.id = vec![8; 9];
        let rejected = resolve(&current, &matched, &Settings::default()).unwrap();
        assert_near(rejected.colour[4][0], 0.25, 1e-5);

        current.id = None;
        current.normal = Some(vec![[0.0, 0.0, 1.0]; 9]);
        let mut aligned = filled(0.75, 1.0, 0.5);
        aligned.normal = vec![[0.0, 0.0, 1.0]; 9];
        let kept = resolve(&current, &aligned, &Settings::default()).unwrap();
        assert_near(kept.colour[4][0], 0.6875, 1e-5);
        aligned.normal = vec![[0.0, 1.0, 0.0]; 9];
        let turned = resolve(&current, &aligned, &Settings::default()).unwrap();
        assert_near(turned.colour[4][0], 0.25, 1e-5);
        aligned.normal = vec![[0.0; 3]; 9];
        let missing = resolve(&current, &aligned, &Settings::default()).unwrap();
        assert_near(missing.colour[4][0], 0.6875, 1e-5);

        let history = filled(0.75, 1.0, 0.5);
        let rescaled = resolve(
            &pattern(0.0, 0.5, [0.0, 0.0], 4.0),
            &history,
            &Settings::default(),
        )
        .unwrap();
        assert_near(rescaled.colour[4][0], 0.6875, 1e-5);
        assert_near(rescaled.history.rgba[4][3], 4.0, 1e-6);
        assert_near(rescaled.history.exposure, 4.0, 1e-6);
        let mut kept_hdr = history.clone();
        kept_hdr.exposure = 2.0;
        let still = resolve(
            &pattern(0.0, 0.5, [0.0, 0.0], 4.0),
            &kept_hdr,
            &Settings::default(),
        )
        .unwrap();
        assert_near(still.colour[4][0], 0.6875, 1e-5);
    }

    fn wide_pattern(motion: [f32; 2]) -> Inputs {
        let mut colour = vec![[0.5; 3]; 12];
        let mut depth = vec![0.5; 12];
        let mut reactive = vec![0.0; 12];
        for y in 0..3 {
            for x in 0..3 {
                let value = match (x, y) {
                    (1, 1) => 0.25,
                    (1, _) | (_, 1) => 0.0,
                    _ => 1.0,
                };
                colour[y * 4 + x] = [value, value, value];
                depth[y * 4 + x] = 0.5;
                reactive[y * 4 + x] = 0.0;
            }
        }
        Inputs {
            width: 4,
            height: 3,
            colour,
            depth,
            motion: (0..12)
                .map(|index| if index == 5 { motion } else { [0.0, 0.0] })
                .collect(),
            reactive,
            id: None,
            normal: None,
            exposure: 1.0,
        }
    }

    #[test]
    fn motion_reprojects_or_rejects() {
        let mut history = History {
            width: 4,
            height: 3,
            rgba: vec![[0.5, 0.5, 0.5, 1.0]; 12],
            depth: vec![0.5; 12],
            id: vec![0; 12],
            normal: vec![[0.0; 3]; 12],
            exposure: 1.0,
            valid: true,
        };
        history.rgba[5] = [0.5, 0.5, 0.5, 1.0];
        history.rgba[6] = [0.75, 0.75, 0.75, 1.0];
        let stayed = resolve(&wide_pattern([0.0, 0.0]), &history, &Settings::default()).unwrap();
        let shifted = resolve(&wide_pattern([0.25, 0.0]), &history, &Settings::default()).unwrap();
        let between = resolve(&wide_pattern([0.125, 0.0]), &history, &Settings::default()).unwrap();
        let left = resolve(&wide_pattern([2.0, 0.0]), &history, &Settings::default()).unwrap();
        let border = resolve(&wide_pattern([0.625, 0.0]), &history, &Settings::default()).unwrap();
        assert_near(stayed.colour[5][0], 0.46875, 1e-5);
        assert_near(shifted.colour[5][0], 0.6875, 1e-5);
        assert_near(between.colour[5][0], 0.578125, 1e-5);
        assert_near(left.colour[5][0], 0.25, 1e-5);
        assert_near(border.colour[5][0], 0.25, 1e-5);
    }

    #[test]
    fn bad_inputs_are_rejected() {
        let mut inputs = pattern(0.0, 0.5, [0.0, 0.0], 1.0);
        inputs.colour.pop();
        assert_eq!(
            resolve(&inputs, &History::empty(), &Settings::default()).unwrap_err(),
            Error::Length
        );
        inputs = pattern(0.0, 0.5, [0.0, 0.0], 0.0);
        assert_eq!(
            resolve(&inputs, &History::empty(), &Settings::default()).unwrap_err(),
            Error::Exposure
        );
        let settings = Settings {
            history_weight: f32::NAN,
            ..Settings::default()
        };
        assert_eq!(
            resolve(
                &pattern(0.0, 0.5, [0.0, 0.0], 1.0),
                &History::empty(),
                &settings
            )
            .unwrap_err(),
            Error::Settings
        );
        assert!(RESOLVE_WGSL.contains("fn resolve"));
        assert!(RESOLVE_WGSL.contains("rgb_to_ycocg"));
    }

    fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, width: u32, height: u32, bytes: &[u8]) {
        let row = bytes.len() as u32 / height;
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn f16_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| half::f16::from_f32(*value).to_le_bytes())
            .collect()
    }

    struct GpuFrame {
        colour: wgpu::Texture,
        colour_view: wgpu::TextureView,
        depth: wgpu::Texture,
        depth_view: wgpu::TextureView,
        motion: wgpu::Texture,
        motion_view: wgpu::TextureView,
        reactive: wgpu::Texture,
        reactive_view: wgpu::TextureView,
        id: wgpu::Texture,
        id_view: wgpu::TextureView,
        normal: wgpu::Texture,
        normal_view: wgpu::TextureView,
    }

    impl GpuFrame {
        fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
            let sampled = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
            let (colour, colour_view) =
                texture(device, "taa colour", width, height, COLOUR_FORMAT, sampled);
            let (depth, depth_view) =
                texture(device, "taa depth", width, height, DEPTH_FORMAT, sampled);
            let (motion, motion_view) =
                texture(device, "taa motion", width, height, MOTION_FORMAT, sampled);
            let (reactive, reactive_view) = texture(
                device,
                "taa reactive",
                width,
                height,
                REACTIVE_FORMAT,
                sampled,
            );
            let (id, id_view) = texture(device, "taa id", width, height, ID_FORMAT, sampled);
            let (normal, normal_view) =
                texture(device, "taa normal", width, height, NORMAL_FORMAT, sampled);
            Self {
                colour,
                colour_view,
                depth,
                depth_view,
                motion,
                motion_view,
                reactive,
                reactive_view,
                id,
                id_view,
                normal,
                normal_view,
            }
        }

        fn upload(&self, queue: &wgpu::Queue, inputs: &Inputs) {
            let width = inputs.width;
            let height = inputs.height;
            let mut colour = Vec::new();
            for pixel in &inputs.colour {
                colour.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0.0]);
            }
            upload(queue, &self.colour, width, height, &f16_bytes(&colour));
            upload(queue, &self.depth, width, height, &f32_bytes(&inputs.depth));
            let mut motion = Vec::new();
            for pixel in &inputs.motion {
                motion.extend_from_slice(pixel);
            }
            upload(queue, &self.motion, width, height, &f16_bytes(&motion));
            upload(
                queue,
                &self.reactive,
                width,
                height,
                &inputs
                    .reactive
                    .iter()
                    .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
                    .collect::<Vec<_>>(),
            );
            let ids = inputs
                .id
                .clone()
                .unwrap_or_else(|| vec![0; inputs.colour.len()]);
            let id_bytes: Vec<u8> = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
            upload(queue, &self.id, width, height, &id_bytes);
            let mut normal = Vec::new();
            let normals = inputs
                .normal
                .clone()
                .unwrap_or_else(|| vec![[0.0; 3]; inputs.colour.len()]);
            for pixel in normals {
                normal.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0.0]);
            }
            let normal_bytes: Vec<u8> = normal
                .iter()
                .map(|value| (value.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8)
                .collect();
            upload(queue, &self.normal, width, height, &normal_bytes);
        }
    }

    fn gpu_resolve(
        pass: &mut Pass,
        gpu: &pfx_gpu::Gpu,
        frame: &GpuFrame,
        inputs: &Inputs,
    ) -> Vec<[f32; 4]> {
        frame.upload(&gpu.queue, inputs);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        pass.resolve(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &GpuInputs {
                colour: &frame.colour_view,
                depth: &frame.depth_view,
                motion: &frame.motion_view,
                reactive: &frame.reactive_view,
                id: inputs.id.as_ref().map(|_| &frame.id_view),
                normal: inputs.normal.as_ref().map(|_| &frame.normal_view),
                exposure: inputs.exposure,
            },
            &Settings::default(),
        )
        .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        pass.read_colour(&gpu.device, &gpu.queue).unwrap()
    }

    fn sized_frame(step: u32, width: u32, height: u32) -> Inputs {
        let n = (width * height) as usize;
        Inputs {
            width,
            height,
            colour: (0..n)
                .map(|i| {
                    let x = (i as u32 % width) as f32 / width as f32;
                    let y = (i as u32 / width) as f32 / height as f32;
                    let wave = ((x * 7.0 + y * 3.0 + step as f32 * 0.7).sin() + 1.0) * 0.5;
                    [wave, x, y]
                })
                .collect(),
            depth: (0..n).map(|i| 0.4 + 0.01 * (i % 3) as f32).collect(),
            motion: vec![[0.03, -0.02]; n],
            reactive: vec![0.0; n],
            id: Some(vec![2; n]),
            normal: None,
            exposure: 1.0,
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_resamples_history_across_viewport_changes() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut pass = Pass::new(&gpu.device);
        pass.resize(&gpu.device, 8, 4);
        let frame = GpuFrame::new(&gpu.device, 8, 4);
        let mut history = History::empty();
        let sizes = [(8, 4), (8, 4), (5, 3), (7, 4), (8, 4), (3, 2), (6, 4)];
        for (step, (width, height)) in sizes.into_iter().enumerate() {
            pass.set_viewport(width, height).unwrap();
            let inputs = sized_frame(step as u32, width, height);
            let cpu = resolve(&inputs, &history, &Settings::default()).unwrap();
            let got = gpu_resolve(&mut pass, &gpu, &frame, &inputs);
            assert_eq!(got.len(), cpu.colour.len());
            for (index, pixel) in cpu.colour.iter().enumerate() {
                for channel in 0..3 {
                    assert_near(got[index][channel], pixel[channel], 1.5e-3);
                }
            }
            history = cpu.history;
        }
        assert!(pass.history_valid());
        assert_eq!(pass.restarts(), 0);
        assert!(pass.set_viewport(9, 4).is_err());
        pass.invalidate();
        assert_eq!(pass.restarts(), 1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_resolves_a_sequence() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut pass = Pass::new(&gpu.device);
        pass.resize(&gpu.device, 8, 4);
        let frame = GpuFrame::new(&gpu.device, 8, 4);
        let mut history = History::empty();
        for step in 0..8 {
            let inputs = edge_frame(step);
            let cpu = resolve(&inputs, &history, &Settings::default()).unwrap();
            let got = gpu_resolve(&mut pass, &gpu, &frame, &inputs);
            for (index, pixel) in cpu.colour.iter().enumerate() {
                for channel in 0..3 {
                    assert_near(got[index][channel], pixel[channel], 1.5e-3);
                }
                assert_near(got[index][3], inputs.exposure, 1.0e-5);
            }
            history = cpu.history;
        }
        assert!(pass.history_valid());
        pass.resize(&gpu.device, 8, 4);
        assert!(pass.history_valid());
        pass.resize(&gpu.device, 5, 4);
        assert!(!pass.history_valid());
        let small = GpuFrame::new(&gpu.device, 5, 4);
        let mut inputs = pattern(1.0, 0.8, [2.0, 0.0], 4.0);
        inputs.width = 5;
        inputs.height = 4;
        inputs.colour = vec![[0.25; 3]; 20];
        inputs.depth = vec![0.8; 20];
        inputs.motion = vec![[0.0; 2]; 20];
        inputs.reactive = vec![1.0; 20];
        inputs.reactive[0] = 0.0;
        inputs.id = Some(vec![3; 20]);
        inputs.normal = None;
        let cpu = resolve(&inputs, &History::empty(), &Settings::default()).unwrap();
        let got = gpu_resolve(&mut pass, &gpu, &small, &inputs);
        for (index, pixel) in cpu.colour.iter().enumerate() {
            for channel in 0..3 {
                assert_near(got[index][channel], pixel[channel], 1.5e-3);
            }
            assert_near(got[index][3], 4.0, 1.0e-5);
        }
        let mut second = pattern(0.0, 0.5, [0.125, 0.0], 2.0);
        second.normal = Some(vec![[0.0, 0.0, 1.0]; 9]);
        let mut pass = Pass::new(&gpu.device);
        pass.resize(&gpu.device, 3, 3);
        let frame = GpuFrame::new(&gpu.device, 3, 3);
        let mut history = History::empty();
        let mut first = pattern(0.0, 0.5, [0.0, 0.0], 1.0);
        first.normal = Some(vec![[0.0, 0.0, 1.0]; 9]);
        for inputs in [first, second] {
            let cpu = resolve(&inputs, &history, &Settings::default()).unwrap();
            let got = gpu_resolve(&mut pass, &gpu, &frame, &inputs);
            for (index, pixel) in cpu.colour.iter().enumerate() {
                for channel in 0..3 {
                    assert_near(got[index][channel], pixel[channel], 1.5e-3);
                }
            }
            history = cpu.history;
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_resolve_4k_timing() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        assert!(
            gpu.device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY)
        );
        assert!(
            gpu.device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
        );
        let mut pass = Pass::new(&gpu.device);
        pass.resize(&gpu.device, 3840, 2160);
        let frame = GpuFrame::new(&gpu.device, 3840, 2160);
        let mut colour = Vec::with_capacity(3840 * 2160);
        let mut depth = Vec::with_capacity(3840 * 2160);
        let mut motion = Vec::with_capacity(3840 * 2160);
        let mut reactive = Vec::with_capacity(3840 * 2160);
        let mut id = Vec::with_capacity(3840 * 2160);
        let mut normal = Vec::with_capacity(3840 * 2160);
        for y in 0..2160 {
            for x in 0..3840 {
                colour.push([
                    x as f32 / 3840.0,
                    y as f32 / 2160.0,
                    ((x ^ y) & 31) as f32 / 16.0,
                ]);
                depth.push(0.1 + (x % 127) as f32 / 128.0);
                motion.push([1.0 / 3840.0, (y % 3) as f32 / 2160.0]);
                reactive.push((x % 17) as f32 / 17.0);
                id.push(x / 32);
                normal.push([0.0, (y % 5) as f32 / 5.0, 1.0]);
            }
        }
        frame.upload(
            &gpu.queue,
            &Inputs {
                width: 3840,
                height: 2160,
                colour,
                depth,
                motion,
                reactive,
                id: Some(id),
                normal: Some(normal),
                exposure: 1.0,
            },
        );
        let inputs = GpuInputs {
            colour: &frame.colour_view,
            depth: &frame.depth_view,
            motion: &frame.motion_view,
            reactive: &frame.reactive_view,
            id: Some(&frame.id_view),
            normal: None,
            exposure: 1.0,
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        pass.resolve(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &inputs,
            &Settings::default(),
        )
        .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let queries = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("taa 4k timing"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolved = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa 4k timing resolved"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa 4k timing read"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut timings = Vec::new();
        let mut turns = pfx_gpu::pace::Turns::default();
        for _ in 0..12 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.write_timestamp(&queries, 0);
            pass.resolve(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &inputs,
                &Settings::default(),
            )
            .unwrap();
            encoder.write_timestamp(&queries, 1);
            encoder.resolve_query_set(&queries, 0..2, &resolved, 0);
            encoder.copy_buffer_to_buffer(&resolved, 0, &read, 0, 16);
            gpu.queue.submit(Some(encoder.finish()));
            let (send, receive) = std::sync::mpsc::channel();
            let slice = read.slice(..);
            slice.map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            receive.recv().unwrap().unwrap();
            let data = slice.get_mapped_range();
            let start = u64::from_le_bytes(data[0..8].try_into().unwrap());
            let end = u64::from_le_bytes(data[8..16].try_into().unwrap());
            timings.push((end - start) as f64 * f64::from(gpu.queue.get_timestamp_period()) / 1e6);
            turns.add(timings[timings.len() - 1]);
            drop(data);
            read.unmap();
        }
        timings.sort_by(f64::total_cmp);
        println!("4K TAA GPU ms: {timings:?}");
    }
}
