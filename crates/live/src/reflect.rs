use pfx_gpu::{GpuProfiler, wgpu};

pub const REFLECTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GUIDE_BINDING: u32 = 26;
pub const REFINE_STEPS: u32 = 7;
pub const REACH: f32 = 64.0;
pub const EDGE_MARGIN: f32 = 0.08;
pub const DEPTH_TOLERANCE: f32 = 0.03;
pub const THICKNESS: f32 = 0.03;
pub const ROUGH_FADE: [f32; 2] = [0.6, 0.75];
pub const CONSISTENT: f32 = 0.8;
pub const JOIN: f32 = 0.005;

pub type Matrix = [[f32; 4]; 4];

pub fn half_size(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(2), height.div_ceil(2))
}

pub fn march_steps(roughness: f32) -> u32 {
    if roughness > 0.45 {
        8
    } else if roughness > 0.18 {
        12
    } else {
        16
    }
}

pub fn rough_fade(roughness: f32) -> f32 {
    1.0 - smoothstep(ROUGH_FADE[0], ROUGH_FADE[1], roughness)
}

pub fn trace_offset(frame_index: u32) -> [u32; 2] {
    match frame_index % 4 {
        0 => [0, 0],
        1 => [1, 1],
        2 => [1, 0],
        _ => [0, 1],
    }
}

pub fn edge_confidence(uv: [f32; 2], margin: f32) -> f32 {
    if margin <= 0.0 {
        return 0.0;
    }
    let distance = uv[0].min(1.0 - uv[0]).min(uv[1]).min(1.0 - uv[1]);
    (distance / margin).clamp(0.0, 1.0)
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub struct DepthImage<'a> {
    pub width: u32,
    pub height: u32,
    pub depth: &'a [f32],
}

impl DepthImage<'_> {
    pub fn at(&self, pixel: [f32; 2]) -> f32 {
        let x = pixel[0].clamp(0.0, self.width as f32 - 1.0) as usize;
        let y = pixel[1].clamp(0.0, self.height as f32 - 1.0) as usize;
        self.depth[y * self.width as usize + x]
    }
}

pub fn to_pixel(projection: &Matrix, size: [f32; 2], point: [f32; 3]) -> [f32; 3] {
    let p = [point[0], point[1], point[2], 1.0];
    let clip: [f32; 4] = std::array::from_fn(|row| {
        (0..4)
            .map(|column| projection[column][row] * p[column])
            .sum()
    });
    [
        (clip[0] / clip[3] * 0.5 + 0.5) * size[0],
        (clip[1] / clip[3] * -0.5 + 0.5) * size[1],
        clip[3],
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub pixel: [f32; 2],
    pub confidence: f32,
}

pub fn march(
    depth: &DepthImage<'_>,
    projection: &Matrix,
    origin: [f32; 3],
    direction: [f32; 3],
    steps: u32,
    thickness: f32,
) -> Option<Hit> {
    let size = [depth.width as f32, depth.height as f32];
    let near = -origin[2];
    let mut reach = near * REACH;
    if direction[2] > 0.0 {
        reach = reach.min(0.9 * near / direction[2]);
    }
    let start = to_pixel(projection, size, origin);
    let end = to_pixel(
        projection,
        size,
        std::array::from_fn(|axis| origin[axis] + direction[axis] * reach),
    );
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut limit = 1.0f32;
    for axis in 0..2 {
        if delta[axis] > 0.0 {
            limit = limit.min((size[axis] - 0.5 - start[axis]) / delta[axis]);
        }
        if delta[axis] < 0.0 {
            limit = limit.min((0.5 - start[axis]) / delta[axis]);
        }
    }
    let span = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    let pixels = span * limit;
    if pixels.is_nan() || pixels < 1.0 {
        return None;
    }
    let k = [1.0 / start[2], 1.0 / end[2]];
    let ray_depth = |s: f32| 1.0 / mix(k[0], k[1], s);
    let place = |s: f32| [start[0] + delta[0] * s, start[1] + delta[1] * s];
    let mut previous = 0.0f32;
    let mut previous_depth = start[2];
    let mut front = start[2] <= depth.at([start[0], start[1]]);
    for i in 1..=steps {
        let u = i as f32 / steps as f32;
        let travelled = (pixels * u * u).max(i as f32).min(pixels);
        let s = travelled / span;
        let ray = ray_depth(s);
        let scene = depth.at(place(s));
        if ray > scene && front && previous_depth.min(ray) <= scene * (1.0 + thickness) {
            let mut lo = previous;
            let mut hi = s;
            for _ in 0..REFINE_STEPS {
                let mid = 0.5 * (lo + hi);
                if ray_depth(mid) > depth.at(place(mid)) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let pixel = place(hi);
            let hit_ray = ray_depth(hi);
            let hit_scene = depth.at(pixel);
            let span_depth = (hit_ray - ray_depth(lo)).abs();
            let beyond = (depth.at(place(hi + 1.0 / span)) - hit_scene).abs();
            if hit_ray - hit_scene <= hit_scene * thickness + span_depth
                && (depth.at(place(lo)) - hit_scene).abs()
                    <= hit_scene * JOIN + 2.0 * beyond + span_depth
            {
                let fade = 1.0 - smoothstep(0.8, 1.0, hi);
                let uv = [pixel[0] / size[0], pixel[1] / size[1]];
                return Some(Hit {
                    pixel,
                    confidence: edge_confidence(uv, EDGE_MARGIN) * fade,
                });
            }
        }
        previous = s;
        previous_depth = ray;
        front = ray <= scene;
        if travelled >= pixels {
            break;
        }
    }
    None
}

pub fn similarity(
    depth: f32,
    normal: [f32; 3],
    other_depth: f32,
    other_normal: [f32; 3],
    tolerance: f32,
) -> f32 {
    if depth <= 0.0 || other_depth <= 0.0 {
        return 0.0;
    }
    let near = (1.0 - (depth - other_depth).abs() / tolerance.max(1e-6)).clamp(0.0, 1.0);
    let facing = smoothstep(0.6, 0.85, dot(normal, other_normal));
    near * facing
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Texel {
    pub colour: [f32; 4],
    pub depth: f32,
    pub normal: [f32; 3],
}

pub fn consistent(width: u32, height: u32, texels: &[Texel], x: u32, y: u32) -> bool {
    let at = |dx: u32, dy: u32| {
        let cx = (x + dx).min(width - 1) as usize;
        let cy = (y + dy).min(height - 1) as usize;
        texels[cy * width as usize + cx]
    };
    let centre = at(0, 0);
    if centre.depth <= 0.0 {
        return false;
    }
    let right = at(1, 0);
    let down = at(0, 1);
    let slope = (right.depth - centre.depth)
        .abs()
        .max((down.depth - centre.depth).abs())
        .min(centre.depth * DEPTH_TOLERANCE);
    let tolerance = centre.depth * DEPTH_TOLERANCE + slope;
    [
        (right, tolerance),
        (down, tolerance),
        (at(1, 1), tolerance + slope),
    ]
    .iter()
    .all(|(other, tolerance)| {
        similarity(
            centre.depth,
            centre.normal,
            other.depth,
            other.normal,
            *tolerance,
        ) > CONSISTENT
    })
}

pub fn upsample(
    width: u32,
    height: u32,
    texels: &[Texel],
    uv: [f32; 2],
    depth: f32,
    normal: [f32; 3],
) -> [f32; 4] {
    let f = [uv[0] * width as f32 - 0.5, uv[1] * height as f32 - 0.5];
    let base = [f[0].floor(), f[1].floor()];
    let t = [f[0] - base[0], f[1] - base[1]];
    let tolerance = depth * DEPTH_TOLERANCE;
    let cell = |i: usize| {
        let offset = [(i & 1) as i32, (i >> 1) as i32];
        let x = (base[0] as i32 + offset[0]).clamp(0, width as i32 - 1) as u32;
        let y = (base[1] as i32 + offset[1]).clamp(0, height as i32 - 1) as u32;
        let bilinear = if offset[0] == 1 { t[0] } else { 1.0 - t[0] }
            * if offset[1] == 1 { t[1] } else { 1.0 - t[1] };
        (x, y, bilinear)
    };
    let (x0, y0, _) = cell(0);
    let first = texels[(y0 * width + x0) as usize];
    let fast = consistent(width, height, texels, x0, y0)
        && similarity(depth, normal, first.depth, first.normal, tolerance) > CONSISTENT;
    let mut sum = [0.0f32; 4];
    let mut total = 0.0f32;
    for i in 0..4 {
        let (x, y, bilinear) = cell(i);
        let texel = texels[(y * width + x) as usize];
        let weight = if fast {
            bilinear
        } else {
            (bilinear + 0.01) * similarity(depth, normal, texel.depth, texel.normal, tolerance)
        };
        for (channel, value) in sum.iter_mut().zip(texel.colour) {
            *channel += value * weight;
        }
        total += weight;
    }
    if total < 0.001 {
        return [0.0; 4];
    }
    sum.map(|value| value / total)
}

pub fn accumulate(
    current: [f32; 4],
    low: [f32; 4],
    high: [f32; 4],
    history: [f32; 4],
    coverage: f32,
    roughness: f32,
    history_valid: bool,
) -> [f32; 4] {
    if !history_valid || coverage <= 0.001 {
        return current;
    }
    let weight = mix(0.6, 0.9, roughness.clamp(0.0, 1.0)) * (coverage * 2.0).clamp(0.0, 1.0);
    std::array::from_fn(|channel| {
        let old = history[channel].clamp(low[channel], high[channel]);
        mix(current[channel], old, weight)
    })
}

macro_rules! guide_wgsl {
    () => {
        r#"
fn ssr_encode_normal(n: vec3f) -> vec2f {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    if (n.z >= 0.0) { return p; }
    return (vec2f(1.0) - abs(p.yx)) * select(vec2f(-1.0), vec2f(1.0), p >= vec2f(0.0));
}
fn ssr_decode_normal(e: vec2f) -> vec3f {
    var n = vec3f(e, 1.0 - abs(e.x) - abs(e.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}
fn ssr_similarity(depth: f32, normal: vec3f, other_depth: f32, other_normal: vec3f, tolerance: f32) -> f32 {
    if (depth <= 0.0 || other_depth <= 0.0) { return 0.0; }
    let near = clamp(1.0 - abs(depth - other_depth) / max(tolerance, 0.000001), 0.0, 1.0);
    let facing = smoothstep(0.6, 0.85, dot(normal, other_normal));
    return near * facing;
}
const SSR_DEPTH_TOLERANCE: f32 = 0.03;
"#
    };
}

macro_rules! params_wgsl {
    () => {
        r#"
struct ReflectParams {
    size: vec2u,
    half_size: vec2u,
    projection: mat4x4f,
    inverse_projection: mat4x4f,
    history_valid: u32,
    frame_index: u32,
    refine: u32,
    thickness: f32,
    history_half: vec2u,
}
@group(0) @binding(0) var<uniform> params: ReflectParams;
fn inside(uv: vec2f) -> bool {
    return all(uv >= vec2f(0.0)) && all(uv < vec2f(1.0));
}
fn trace_pixel(cell: vec2u) -> vec2u {
    let phase = params.frame_index % 4u;
    let offset = vec2u(select(0u, 1u, phase == 1u || phase == 2u), select(0u, 1u, phase == 1u || phase == 3u));
    return min(cell * 2u + offset, params.size - 1u);
}
"#
    };
}

pub const TRACE_WGSL: &str = concat!(
    guide_wgsl!(),
    params_wgsl!(),
    r#"
@group(0) @binding(1) var depth_tex: texture_2d<f32>;
@group(0) @binding(2) var normal_roughness: texture_2d<f32>;
@group(0) @binding(3) var source_colour: texture_2d<f32>;
@group(0) @binding(4) var colour_sampler: sampler;
@group(0) @binding(5) var raw_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var guide_out: texture_storage_2d<rgba16float, write>;
fn edge_weight(uv: vec2f) -> f32 {
    let d = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    return clamp(d / 0.08, 0.0, 1.0);
}
fn to_pixel(p: vec3f) -> vec3f {
    let clip = params.projection * vec4f(p, 1.0);
    let ndc = clip.xy / clip.w;
    return vec3f((ndc * vec2f(0.5, -0.5) + vec2f(0.5)) * vec2f(params.size), clip.w);
}
fn scene_depth(pixel: vec2f) -> f32 {
    let at = vec2i(clamp(pixel, vec2f(0.0), vec2f(params.size) - vec2f(1.0)));
    return textureLoad(depth_tex, at, 0).r;
}
fn ray_depth(k: vec2f, s: f32) -> f32 {
    return 1.0 / mix(k.x, k.y, s);
}
fn march_steps(roughness: f32) -> u32 {
    return select(select(16u, 12u, roughness > 0.18), 8u, roughness > 0.45);
}
fn march(origin: vec3f, direction: vec3f, steps: u32) -> vec3f {
    let near = -origin.z;
    var reach = near * 64.0;
    if (direction.z > 0.0) {
        reach = min(reach, 0.9 * near / direction.z);
    }
    let start = to_pixel(origin);
    let end = to_pixel(origin + direction * reach);
    let delta = end.xy - start.xy;
    let size = vec2f(params.size);
    var limit = 1.0;
    if (delta.x > 0.0) { limit = min(limit, (size.x - 0.5 - start.x) / delta.x); }
    if (delta.x < 0.0) { limit = min(limit, (0.5 - start.x) / delta.x); }
    if (delta.y > 0.0) { limit = min(limit, (size.y - 0.5 - start.y) / delta.y); }
    if (delta.y < 0.0) { limit = min(limit, (0.5 - start.y) / delta.y); }
    let span = length(delta);
    let pixels = span * limit;
    if (!(pixels >= 1.0)) { return vec3f(0.0); }
    let k = vec2f(1.0 / start.z, 1.0 / end.z);
    var previous = 0.0;
    var previous_depth = start.z;
    var front = start.z <= scene_depth(start.xy);
    for (var i = 1u; i <= steps; i++) {
        let u = f32(i) / f32(steps);
        let travelled = min(max(pixels * u * u, f32(i)), pixels);
        let s = travelled / span;
        let ray = ray_depth(k, s);
        let scene = scene_depth(start.xy + delta * s);
        if (ray > scene && front && min(previous_depth, ray) <= scene * (1.0 + params.thickness)) {
            var lo = previous;
            var hi = s;
            for (var j = 0u; j < params.refine; j++) {
                let mid = 0.5 * (lo + hi);
                if (ray_depth(k, mid) > scene_depth(start.xy + delta * mid)) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let pixel = start.xy + delta * hi;
            let hit_ray = ray_depth(k, hi);
            let hit_scene = scene_depth(pixel);
            let span_depth = abs(hit_ray - ray_depth(k, lo));
            let beyond = abs(scene_depth(start.xy + delta * (hi + 1.0 / span)) - hit_scene);
            let joined = abs(scene_depth(start.xy + delta * lo) - hit_scene) <= hit_scene * 0.005 + 2.0 * beyond + span_depth;
            if (hit_ray - hit_scene <= hit_scene * params.thickness + span_depth && joined) {
                let fade = 1.0 - smoothstep(0.8, 1.0, hi);
                return vec3f(pixel, edge_weight(pixel / size) * fade);
            }
        }
        previous = s;
        previous_depth = ray;
        front = ray <= scene;
        if (travelled >= pixels) { break; }
    }
    return vec3f(0.0);
}
fn view_position(pixel: vec2i) -> vec3f {
    let at = clamp(pixel, vec2i(0), vec2i(params.size) - vec2i(1));
    let depth = textureLoad(depth_tex, at, 0).r;
    let uv = (vec2f(at) + vec2f(0.5)) / vec2f(params.size);
    let clip = vec4f(uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 1.0, 1.0);
    let unprojected = params.inverse_projection * clip;
    return unprojected.xyz / unprojected.w * (depth / max(-unprojected.z / unprojected.w, 0.0001));
}
fn nearer(origin: vec3f, a: vec3f, b: vec3f) -> vec3f {
    return select(origin - b, a - origin, abs(a.z - origin.z) < abs(b.z - origin.z));
}
@compute @workgroup_size(8, 8)
fn trace_pass(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= params.half_size)) { return; }
    let pixel = trace_pixel(id.xy);
    let depth = textureLoad(depth_tex, vec2i(pixel), 0).r;
    let nr = textureLoad(normal_roughness, vec2i(pixel), 0);
    if (depth <= 0.0 || length(nr.xyz) < 0.1) {
        textureStore(raw_out, vec2i(id.xy), vec4f(0.0));
        textureStore(guide_out, vec2i(id.xy), vec4f(0.0));
        return;
    }
    let p = vec2i(pixel);
    let origin = view_position(p);
    let across = nearer(origin, view_position(p + vec2i(1, 0)), view_position(p - vec2i(1, 0)));
    let along = nearer(origin, view_position(p + vec2i(0, 1)), view_position(p - vec2i(0, 1)));
    let normal = normalize(nr.xyz);
    var surface = cross(along, across);
    surface = select(normal, normalize(surface), dot(surface, surface) > 0.0);
    surface = select(surface, -surface, dot(surface, origin) > 0.0);
    let roughness = clamp(nr.w, 0.0, 1.0);
    let view_ray = normalize(origin);
    let angle = 6.2831853 * f32(params.frame_index % 8u) * 0.125;
    let jitter = vec3f(cos(angle), sin(angle), 0.0) * roughness * roughness * 0.15;
    var direction = normalize(reflect(view_ray, normal) + jitter);
    let lift = 0.05 - dot(direction, surface);
    if (lift > 0.0) {
        direction = normalize(direction + surface * lift);
    }
    var result = vec4f(0.0);
    var hit = vec3f(0.0);
    let fade = 1.0 - smoothstep(0.6, 0.75, roughness);
    if (fade > 0.0) {
        hit = march(origin + surface * depth * 0.002, direction, march_steps(roughness));
    }
    if (hit.z > 0.0) {
        let at = vec2i(clamp(hit.xy, vec2f(0.0), vec2f(params.size) - vec2f(1.0)));
        let facing = dot(textureLoad(normal_roughness, at, 0).xyz, direction);
        let confidence = hit.z * fade * clamp(1.0 - 4.0 * (facing - 0.1), 0.0, 1.0);
        let lod = clamp(roughness * 4.0 - 0.5, 0.0, f32(textureNumLevels(source_colour) - 1u));
        let colour = textureSampleLevel(source_colour, colour_sampler, hit.xy / vec2f(textureDimensions(source_colour)), lod).rgb;
        result = vec4f(colour * confidence, confidence);
    }
    textureStore(raw_out, vec2i(id.xy), result);
    textureStore(guide_out, vec2i(id.xy), vec4f(depth, ssr_encode_normal(normal), roughness));
}
"#
);

pub const RESOLVE_WGSL: &str = concat!(
    guide_wgsl!(),
    params_wgsl!(),
    r#"
@group(0) @binding(1) var raw_tex: texture_2d<f32>;
@group(0) @binding(2) var guide_tex: texture_2d<f32>;
@group(0) @binding(3) var history_tex: texture_2d<f32>;
@group(0) @binding(4) var history_guide: texture_2d<f32>;
@group(0) @binding(5) var motion_tex: texture_2d<f32>;
@group(0) @binding(6) var output_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var guide_out: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(8, 8)
fn resolve_pass(@builtin(global_invocation_id) id: vec3u) {
    if (any(id.xy >= params.half_size)) { return; }
    let cell = vec2i(id.xy);
    let last = vec2i(params.half_size) - vec2i(1);
    let current = textureLoad(raw_tex, cell, 0);
    let guide = textureLoad(guide_tex, cell, 0);
    if (guide.x <= 0.0) {
        textureStore(output_tex, cell, vec4f(0.0));
        textureStore(guide_out, cell, vec4f(0.0));
        return;
    }
    let normal = ssr_decode_normal(guide.yz);
    let right = textureLoad(guide_tex, clamp(cell + vec2i(1, 0), vec2i(0), last), 0);
    let down = textureLoad(guide_tex, clamp(cell + vec2i(0, 1), vec2i(0), last), 0);
    let diagonal = textureLoad(guide_tex, clamp(cell + vec2i(1, 1), vec2i(0), last), 0);
    let slope = min(max(abs(right.x - guide.x), abs(down.x - guide.x)), guide.x * SSR_DEPTH_TOLERANCE);
    let tolerance = guide.x * SSR_DEPTH_TOLERANCE + slope;
    let consistent = ssr_similarity(guide.x, normal, right.x, ssr_decode_normal(right.yz), tolerance) > 0.8
        && ssr_similarity(guide.x, normal, down.x, ssr_decode_normal(down.yz), tolerance) > 0.8
        && ssr_similarity(guide.x, normal, diagonal.x, ssr_decode_normal(diagonal.yz), tolerance + slope) > 0.8;
    textureStore(guide_out, cell, vec4f(guide.xyz, select(-1.0 - guide.w, guide.w, consistent)));
    var low = current;
    var high = current;
    for (var i = 0; i < 4; i++) {
        let offset = select(vec2i(0, select(-1, 1, i == 3)), vec2i(select(-1, 1, i == 1), 0), i < 2);
        let near = textureLoad(raw_tex, clamp(cell + offset, vec2i(0), last), 0);
        low = min(low, near);
        high = max(high, near);
    }
    var result = current;
    let pixel = trace_pixel(id.xy);
    let prev = (vec2f(pixel) + vec2f(0.5)) / vec2f(params.size) + textureLoad(motion_tex, vec2i(pixel), 0).rg;
    if (params.history_valid != 0u && inside(prev)) {
        let history_last = vec2i(params.history_half) - vec2i(1);
        let f = prev * vec2f(params.history_half) - vec2f(0.5);
        let base = vec2i(floor(f));
        let t = f - floor(f);
        var sum = vec4f(0.0);
        var total = 0.0;
        var coverage = 0.0;
        for (var i = 0; i < 4; i++) {
            let offset = vec2i(i & 1, i >> 1u);
            let at = clamp(base + offset, vec2i(0), history_last);
            let bilinear = select(1.0 - t.x, t.x, offset.x == 1) * select(1.0 - t.y, t.y, offset.y == 1);
            let g = textureLoad(history_guide, at, 0);
            let similar = ssr_similarity(guide.x, normal, g.x, ssr_decode_normal(g.yz), tolerance);
            let weight = (bilinear + 0.01) * similar;
            sum += textureLoad(history_tex, at, 0) * weight;
            total += weight;
            coverage += bilinear * similar;
        }
        if (total > 0.001 && coverage > 0.001) {
            let old = clamp(sum / total, low, high);
            let weight = mix(0.6, 0.9, clamp(guide.w, 0.0, 1.0)) * clamp(coverage * 2.0, 0.0, 1.0);
            result = mix(current, old, weight);
        }
    }
    textureStore(output_tex, cell, result);
}
"#
);

pub const DFG_FIT: [[f32; 3]; 24] = [
    [0.8148172, 0.1125372, -0.01374308],
    [-0.3511763, -0.1810853, 0.1673936],
    [-0.02032731, -0.4825735, 0.04973246],
    [0.462142, -0.411372, -0.06177693],
    [0.02489464, 0.1060682, 0.07428504],
    [-0.4534661, 0.6691472, -0.1939928],
    [0.05596158, -0.1952609, -0.01097813],
    [0.5484627, 0.5771128, -0.3130064],
    [0.2092212, 2.050402, 1.003509],
    [-2.763494, -3.524146, -0.6253516],
    [-0.6803499, -2.683833, -1.808813],
    [2.698063, 3.846977, 1.655182],
    [0.03557167, -0.07356264, 0.03812399],
    [0.1204449, -0.2777732, 0.1592967],
    [-0.1640982, 0.3660604, -0.2035734],
    [-0.3198297, 0.7459178, -0.4316107],
    [0.2148219, -0.494718, 0.2827545],
    [0.1391939, -0.3242128, 0.1875587],
    [0.1873088, 0.3210023, 0.1747016],
    [-1.316489, -1.468577, -1.091297],
    [1.369269, -0.2278983, -0.9679774],
    [1.550273, 4.079052, 4.483775],
    [-1.484991, -0.5113827, 0.5854544],
    [-0.3898746, -2.342672, -3.369676],
];

pub fn environment_dfg(roughness: f32, ndv: f32) -> [f32; 2] {
    let u = 2.0 * roughness.clamp(0.05, 1.0) - 1.0;
    let n = ndv.clamp(0.02, 1.0);
    let s = 2.0 * n.sqrt() - 1.0;
    let p = [1.0, s, s * s];
    let f5 = (1.0 - n).powi(5);
    let row = |k: usize| {
        (0..3)
            .map(|j| p[j] * (DFG_FIT[k][j] + f5 * DFG_FIT[k + 6][j]))
            .sum::<f32>()
    };
    let mut dfg = [0.0f32; 2];
    for i in (0..6).rev() {
        dfg = [dfg[0] * u + row(i), dfg[1] * u + row(12 + i)];
    }
    dfg.map(|v| v.clamp(0.0, 1.0))
}

pub const PROBE_WGSL: &str = concat!(
    guide_wgsl!(),
    r#"
override use_local_reflections: bool = true;
const LOCAL_DEPTH_LEVELS: f32 = 6.0;
const LOCAL_DEPTH_START: f32 = 0.02;
const LOCAL_DEPTH_GROWTH: f32 = 2.5;
const LOCAL_DEPTH_STEPS: u32 = 9u;
const LOCAL_DEPTH_REFINE: u32 = 4u;
const LOCAL_DEPTH_TOLERANCE: f32 = 0.02;
const LOCAL_DEPTH_THICKNESS: f32 = 0.03;
struct ReflectionProbe {
    center: vec3f,
    layer: i32,
    box_min: vec3f,
    priority: f32,
    box_max: vec3f,
    fade: f32,
}
fn local_probe_direction(probe: ReflectionProbe, world: vec3f, direction: vec3f) -> vec3f {
    let safe = select(vec3f(-1.0), vec3f(1.0), direction >= vec3f(0.0)) * max(abs(direction), vec3f(0.00001));
    let a = (probe.box_min - world) / safe;
    let b = (probe.box_max - world) / safe;
    let reach = min(max(a.x, b.x), min(max(a.y, b.y), max(a.z, b.z)));
    return normalize(world + direction * max(reach, 0.0) - probe.center);
}
fn ssr_resolve(colour: texture_2d<f32>, guide: texture_2d<f32>, samp: sampler, uv: vec2f, written: vec2f, depth: f32, normal: vec3f) -> vec4f {
    let dimensions = vec2i(textureDimensions(colour));
    if (any(dimensions != vec2i(textureDimensions(guide)))) { return vec4f(0.0); }
    let size = min(dimensions, vec2i(written));
    let f = uv * vec2f(size) - vec2f(0.5);
    let base = vec2i(floor(f));
    let t = f - floor(f);
    let tolerance = depth * SSR_DEPTH_TOLERANCE;
    let first = textureLoad(guide, clamp(base, vec2i(0), size - vec2i(1)), 0);
    if (first.w >= 0.0 && ssr_similarity(depth, normal, first.x, ssr_decode_normal(first.yz), tolerance) > 0.8) {
        let inner = any(size < dimensions);
        let texel = select(uv * vec2f(size), min(uv * vec2f(size), vec2f(size) - vec2f(0.5)), inner);
        return textureSampleLevel(colour, samp, select(uv, texel / vec2f(dimensions), inner), 0.0);
    }
    var sum = vec4f(0.0);
    var total = 0.0;
    for (var i = 0; i < 4; i++) {
        let offset = vec2i(i & 1, i >> 1u);
        let at = clamp(base + offset, vec2i(0), size - vec2i(1));
        let bilinear = select(1.0 - t.x, t.x, offset.x == 1) * select(1.0 - t.y, t.y, offset.y == 1);
        let g = textureLoad(guide, at, 0);
        let weight = (bilinear + 0.01) * ssr_similarity(depth, normal, g.x, ssr_decode_normal(g.yz), tolerance);
        sum += textureLoad(colour, at, 0) * weight;
        total += weight;
    }
    if (total < 0.001) { return vec4f(0.0); }
    return sum / total;
}
fn local_probe_weight(probe: ReflectionProbe, world: vec3f) -> vec2f {
    if (any(world < probe.box_min) || any(world > probe.box_max)) {
        return vec2f(0.0);
    }
    let low = world - probe.box_min;
    let high = probe.box_max - world;
    let edge = min(min(min(low.x, low.y), low.z), min(min(high.x, high.y), high.z));
    let seam = select(clamp(edge / probe.fade, 0.0, 1.0), 1.0, probe.fade == 0.0);
    return vec2f(seam * (1.0 + max(probe.priority, 0.0)) / (length(world - probe.center) + 0.001), seam);
}
fn local_probe_weights(probe_a: ReflectionProbe, probe_b: ReflectionProbe, present: vec2f, world: vec3f) -> vec2f {
    let a = local_probe_weight(probe_a, world) * present.x;
    let b = local_probe_weight(probe_b, world) * present.y;
    let total = a.x + b.x;
    if (total <= 0.0) {
        return vec2f(0.0);
    }
    let coverage = max(a.y, b.y);
    return vec2f(a.x, b.x) * coverage / total;
}
fn local_probe_gap(probe: ReflectionProbe, reached: vec3f, local: texture_cube_array<f32>, samp: sampler, level: f32) -> f32 {
    let distance = max(length(reached), 0.000001);
    let stored = 1.0 / max(-1.0 - textureSampleLevel(local, samp, reached / distance, probe.layer, level).a, 0.00001);
    return (distance - stored) / distance;
}
fn local_probe_lookup(probe: ReflectionProbe, world: vec3f, direction: vec3f, local: texture_cube_array<f32>, samp: sampler, local_mip: f32) -> vec3f {
    let level = max(local_mip - LOCAL_DEPTH_LEVELS, 0.0);
    if (textureSampleLevel(local, samp, direction, probe.layer, level).a >= 0.0) {
        return local_probe_direction(probe, world, direction);
    }
    let offset = world - probe.center;
    var ahead = local_probe_gap(probe, offset, local, samp, level) < LOCAL_DEPTH_THICKNESS;
    var near = 0.0;
    var t = LOCAL_DEPTH_START;
    for (var step = 0u; step < LOCAL_DEPTH_STEPS; step++) {
        let outer = local_probe_gap(probe, offset + direction * t, local, samp, level);
        if (outer <= LOCAL_DEPTH_TOLERANCE) {
            ahead = true;
        } else {
            if (ahead) {
                var low = near;
                var high = t;
                var gap = outer;
                for (var refine = 0u; refine < LOCAL_DEPTH_REFINE; refine++) {
                    let middle = (low + high) * 0.5;
                    let found = local_probe_gap(probe, offset + direction * middle, local, samp, level);
                    if (found > LOCAL_DEPTH_TOLERANCE) {
                        high = middle;
                        gap = found;
                    } else {
                        low = middle;
                    }
                }
                let reached = offset + direction * high;
                if (gap < LOCAL_DEPTH_THICKNESS + (high - low) / max(length(reached), 0.000001)) {
                    return select(direction, normalize(reached), dot(reached, reached) > 0.0000001);
                }
            }
            ahead = false;
        }
        near = t;
        t *= LOCAL_DEPTH_GROWTH;
    }
    return direction;
}
fn local_probe_reference(probe: ReflectionProbe, world: vec3f, n: vec3f, local: texture_cube_array<f32>, samp: sampler, local_mip: f32) -> vec3f {
    let along = textureSampleLevel(local, samp, n, probe.layer, local_mip);
    if (along.a < 0.0) {
        return along.rgb;
    }
    return textureSampleLevel(local, samp, local_probe_direction(probe, world, n), probe.layer, local_mip).rgb;
}
struct LocalLookup {
    weights: vec2f,
    first: vec3f,
    second: vec3f,
}
fn local_lookup(probe_a: ReflectionProbe, probe_b: ReflectionProbe, present: vec2f, world: vec3f, direction: vec3f, local: texture_cube_array<f32>, samp: sampler, local_mip: f32) -> LocalLookup {
    var lookup = LocalLookup(vec2f(0.0), direction, direction);
    if (!use_local_reflections) {
        return lookup;
    }
    let weights = local_probe_weights(probe_a, probe_b, present, world);
    let a = clamp(weights.x, 0.0, 1.0);
    lookup.weights = vec2f(a, clamp(weights.y, 0.0, 1.0 - a));
    if (lookup.weights.x > 0.0) {
        lookup.first = local_probe_lookup(probe_a, world, direction, local, samp, local_mip);
    }
    if (lookup.weights.y > 0.0) {
        lookup.second = local_probe_lookup(probe_b, world, direction, local, samp, local_mip);
    }
    return lookup;
}
fn reflection_dfg(roughness: f32, ndv: f32) -> vec2f {
    let u = 2.0 * clamp(roughness, 0.05, 1.0) - 1.0;
    let n = clamp(ndv, 0.02, 1.0);
    let s = 2.0 * sqrt(n) - 1.0;
    let p = vec3f(1.0, s, s * s);
    let f5 = pow(1.0 - n, 5.0);
    var dfg = vec2f(0.0);
    dfg = dfg * u + vec2f(dot(p, vec3f(-0.4534661, 0.6691472, -0.1939928) + f5 * vec3f(2.698063, 3.846977, 1.655182)), dot(p, vec3f(0.1391939, -0.3242128, 0.1875587) + f5 * vec3f(-0.3898746, -2.342672, -3.369676)));
    dfg = dfg * u + vec2f(dot(p, vec3f(0.02489464, 0.1060682, 0.07428504) + f5 * vec3f(-0.6803499, -2.683833, -1.808813)), dot(p, vec3f(0.2148219, -0.494718, 0.2827545) + f5 * vec3f(-1.484991, -0.5113827, 0.5854544)));
    dfg = dfg * u + vec2f(dot(p, vec3f(0.462142, -0.411372, -0.06177693) + f5 * vec3f(-2.763494, -3.524146, -0.6253516)), dot(p, vec3f(-0.3198297, 0.7459178, -0.4316107) + f5 * vec3f(1.550273, 4.079052, 4.483775)));
    dfg = dfg * u + vec2f(dot(p, vec3f(-0.02032731, -0.4825735, 0.04973246) + f5 * vec3f(0.2092212, 2.050402, 1.003509)), dot(p, vec3f(-0.1640982, 0.3660604, -0.2035734) + f5 * vec3f(1.369269, -0.2278983, -0.9679774)));
    dfg = dfg * u + vec2f(dot(p, vec3f(-0.3511763, -0.1810853, 0.1673936) + f5 * vec3f(0.5484627, 0.5771128, -0.3130064)), dot(p, vec3f(0.1204449, -0.2777732, 0.1592967) + f5 * vec3f(-1.316489, -1.468577, -1.091297)));
    dfg = dfg * u + vec2f(dot(p, vec3f(0.8148172, 0.1125372, -0.01374308) + f5 * vec3f(0.05596158, -0.1952609, -0.01097813)), dot(p, vec3f(0.03557167, -0.07356264, 0.03812399) + f5 * vec3f(0.1873088, 0.3210023, 0.1747016)));
    return clamp(dfg, vec2f(0.0), vec2f(1.0));
}
fn reflection_environment(surface: Shaded, ndv: f32, film: vec3f, film_normal: vec3f) -> vec3f {
    let f0 = reflection_filmed(surface, 1.0, film_normal);
    if (all(f0 <= vec3f(0.0)) && surface.thin_film_amount <= 0.0) {
        return vec3f(0.0);
    }
    let dfg = reflection_dfg(surface.roughness, ndv);
    let fitted = clamp(f0 * (dfg.x - dfg.y) + vec3f(dfg.y), vec3f(0.0), vec3f(1.0));
    let filmed = reflection_filmed(surface, ndv, film) - (f0 + (vec3f(1.0) - f0) * pow(1.0 - ndv, 5.0));
    return clamp(fitted + select(vec3f(0.0), filmed, surface.thin_film_amount > 0.0), vec3f(0.0), vec3f(1.0));
}
fn reflection_diffuse_weight(surface: Shaded, view: vec3f, normal: vec3f, film: vec3f, film_normal: vec3f) -> vec3f {
    let ndv = clamp(dot(normal, view), 0.0, 1.0);
    let fresnel = reflection_environment(surface, ndv, film, film_normal);
    let coat = clamp(surface.clearcoat, 0.0, 1.0);
    let coat_f = schlick(vec3f(0.04), ndv, 5.0).r;
    return (vec3f(1.0) - fresnel) * (1.0 - clamp(surface.metalness, 0.0, 1.0)) * (1.0 - clamp(surface.transmission, 0.0, 1.0)) * (1.0 - coat * coat_f);
}
fn reflection_fallback(
    direction: vec3f,
    lookup: LocalLookup,
    rough: f32,
    open: f32,
    sky: texture_cube<f32>,
    local: texture_cube_array<f32>,
    probe_a: ReflectionProbe,
    probe_b: ReflectionProbe,
    samp: sampler,
    local_mip: f32,
    sky_mip: f32,
) -> vec3f {
    var fallback = vec3f(0.0);
    if (open != 0.0) {
        fallback = textureSampleLevel(sky, samp, direction, rough * sky_mip).rgb * open;
    }
    if (lookup.weights.x != 0.0) {
        fallback += textureSampleLevel(local, samp, lookup.first, probe_a.layer, rough * local_mip).rgb * lookup.weights.x;
    }
    if (lookup.weights.y != 0.0) {
        fallback += textureSampleLevel(local, samp, lookup.second, probe_b.layer, rough * local_mip).rgb * lookup.weights.y;
    }
    return fallback;
}
fn reflection_radiance(
    surface: Shaded,
    view: vec3f,
    normal: vec3f,
    hit: vec4f,
    sky: texture_cube<f32>,
    local: texture_cube_array<f32>,
    probe_a: ReflectionProbe,
    probe_b: ReflectionProbe,
    lookup: LocalLookup,
    samp: sampler,
    local_mip: f32,
    sky_mip: f32,
    film: vec3f,
    film_normal: vec3f,
    sky_visible: f32,
) -> vec3f {
    let ndv = clamp(dot(normal, view), 0.0, 1.0);
    let direction = reflect(-view, normal);
    let rough = clamp(surface.roughness, 0.0, 1.0);
    let open = (1.0 - lookup.weights.x - lookup.weights.y) * sky_visible;
    let confidence = clamp(hit.a, 0.0, 1.0);
    let caught = max(hit.rgb, vec3f(0.0));
    let radiance = reflection_fallback(direction, lookup, rough, open, sky, local, probe_a, probe_b, samp, local_mip, sky_mip) * (1.0 - confidence) + caught;
    let fresnel = reflection_environment(surface, ndv, film, film_normal);
    let coat = clamp(surface.clearcoat, 0.0, 1.0);
    let coat_f = schlick(vec3f(0.04), ndv, 5.0).r;
    let coat_spec = coat * coat_f;
    let substrate = min(fresnel * (1.0 - coat_spec), vec3f(1.0 - coat_spec));
    if (coat_spec <= 0.0) {
        return radiance * substrate;
    }
    let coat_rough = clamp(surface.clearcoat_roughness, 0.0, 1.0);
    var coated = radiance;
    if (coat_rough != rough) {
        coated = reflection_fallback(direction, lookup, coat_rough, open, sky, local, probe_a, probe_b, samp, local_mip, sky_mip) * (1.0 - confidence) + caught;
    }
    return radiance * substrate + coated * coat_spec;
}
"#
);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TraceUniform {
    size: [u32; 2],
    half_size: [u32; 2],
    projection: [[f32; 4]; 4],
    inverse_projection: [[f32; 4]; 4],
    history_valid: u32,
    frame_index: u32,
    refine: u32,
    thickness: f32,
    history_half: [u32; 2],
    pad: [u32; 2],
}

fn sampled(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
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

fn storage(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: REFLECTION_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
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

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn texture(device: &wgpu::Device, label: &'static str, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: REFLECTION_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

pub struct Inputs<'a> {
    pub depth: &'a wgpu::TextureView,
    pub normal_roughness: &'a wgpu::TextureView,
    pub colour: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    pub projection: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
    pub frame_index: u32,
    pub thickness: f32,
    pub history_valid: bool,
}

#[derive(Clone, Debug)]
pub struct Reflections {
    pub colour: wgpu::TextureView,
    pub guide: wgpu::TextureView,
    pub size: [u32; 2],
}

pub struct Pass {
    width: u32,
    height: u32,
    viewport: (u32, u32),
    history: [u32; 2],
    _raw: [wgpu::Texture; 2],
    raw_view: wgpu::TextureView,
    raw_guide: wgpu::TextureView,
    _results: [wgpu::Texture; 2],
    result_views: [wgpu::TextureView; 2],
    _guides: [wgpu::Texture; 2],
    guide_views: [wgpu::TextureView; 2],
    sampler: wgpu::Sampler,
    next: usize,
    valid: bool,
    trace_layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    trace_pipeline: wgpu::ComputePipeline,
    resolve_pipeline: wgpu::ComputePipeline,
}

impl Pass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0);
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflection trace"),
            entries: &[
                uniform(0),
                sampled(1, false),
                sampled(2, false),
                sampled(3, true),
                sampler_entry(4),
                storage(5),
                storage(6),
            ],
        });
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflection resolve"),
            entries: &[
                uniform(0),
                sampled(1, false),
                sampled(2, false),
                sampled(3, false),
                sampled(4, false),
                sampled(5, false),
                storage(6),
                storage(7),
            ],
        });
        let pipeline = |label, source: &str, layout: &wgpu::BindGroupLayout, entry| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace_pipeline = pipeline("reflection trace", TRACE_WGSL, &trace_layout, "trace_pass");
        let resolve_pipeline = pipeline(
            "reflection resolve",
            RESOLVE_WGSL,
            &resolve_layout,
            "resolve_pass",
        );
        let (half_width, half_height) = half_size(width, height);
        let raw = [
            texture(device, "reflection raw", half_width, half_height),
            texture(device, "reflection raw guide", half_width, half_height),
        ];
        let raw_view = raw[0].create_view(&Default::default());
        let raw_guide = raw[1].create_view(&Default::default());
        let results = [
            texture(device, "reflection result 0", half_width, half_height),
            texture(device, "reflection result 1", half_width, half_height),
        ];
        let result_views = [
            results[0].create_view(&Default::default()),
            results[1].create_view(&Default::default()),
        ];
        let guides = [
            texture(device, "reflection guide 0", half_width, half_height),
            texture(device, "reflection guide 1", half_width, half_height),
        ];
        let guide_views = [
            guides[0].create_view(&Default::default()),
            guides[1].create_view(&Default::default()),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection colour"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            width,
            height,
            viewport: (width, height),
            history: [half_width, half_height],
            _raw: raw,
            raw_view,
            raw_guide,
            _results: results,
            result_views,
            _guides: guides,
            guide_views,
            sampler,
            next: 0,
            valid: false,
            trace_layout,
            resolve_layout,
            trace_pipeline,
            resolve_pipeline,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.size() != (width, height) {
            *self = Self::new(device, width, height);
        }
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    pub fn textures(&self) -> Vec<wgpu::Texture> {
        self._raw
            .iter()
            .chain(&self._results)
            .chain(&self._guides)
            .cloned()
            .collect()
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 || width > self.width || height > self.height {
            return Err(format!(
                "the viewport {width}x{height} must be inside the reflections' {}x{}",
                self.width, self.height
            ));
        }
        self.viewport = (width, height);
        Ok(())
    }

    pub fn result(&self) -> Option<Reflections> {
        let last = 1 - self.next;
        self.valid.then(|| Reflections {
            colour: self.result_views[last].clone(),
            guide: self.guide_views[last].clone(),
            size: self.history,
        })
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &Inputs<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        use wgpu::util::DeviceExt;
        let (width, height) = self.viewport;
        let (half_width, half_height) = half_size(width, height);
        let params = TraceUniform {
            size: [width, height],
            half_size: [half_width, half_height],
            projection: inputs.projection,
            inverse_projection: inputs.inverse_projection,
            history_valid: u32::from(self.valid && inputs.history_valid),
            frame_index: inputs.frame_index,
            refine: REFINE_STEPS,
            thickness: inputs.thickness,
            history_half: self.history,
            pad: [0; 2],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("reflection uniforms"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let current = self.next;
        let last = 1 - current;
        let view = wgpu::BindingResource::TextureView;
        let bind =
            |layout: &wgpu::BindGroupLayout, label, resources: Vec<wgpu::BindingResource>| {
                let entries = std::iter::once(buffer.as_entire_binding())
                    .chain(resources)
                    .enumerate()
                    .map(|(binding, resource)| wgpu::BindGroupEntry {
                        binding: binding as u32,
                        resource,
                    })
                    .collect::<Vec<_>>();
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout,
                    entries: &entries,
                })
            };
        let trace_bind = bind(
            &self.trace_layout,
            "reflection trace bindings",
            vec![
                view(inputs.depth),
                view(inputs.normal_roughness),
                view(inputs.colour),
                wgpu::BindingResource::Sampler(&self.sampler),
                view(&self.raw_view),
                view(&self.raw_guide),
            ],
        );
        let resolve_bind = bind(
            &self.resolve_layout,
            "reflection resolve bindings",
            vec![
                view(&self.raw_view),
                view(&self.raw_guide),
                view(&self.result_views[last]),
                view(&self.guide_views[last]),
                view(inputs.motion),
                view(&self.result_views[current]),
                view(&self.guide_views[current]),
            ],
        );
        let groups = [half_width.div_ceil(8), half_height.div_ceil(8)];
        for (label, pipeline, group) in [
            ("reflections", &self.trace_pipeline, &trace_bind),
            ("reflection resolve", &self.resolve_pipeline, &resolve_bind),
        ] {
            let timing = profiler.as_deref_mut().and_then(|p| p.pass(label));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: profiler.as_deref().and_then(|p| p.compute_writes(timing)),
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], 1);
        }
        self.next = last;
        self.valid = true;
        self.history = [half_width, half_height];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAR: f32 = 80.0;

    fn traced_dfg(roughness: f32, ndv: f32) -> [f32; 2] {
        let a2 = pfx_materials::alpha(roughness.max(0.05)).powi(2);
        let view = [(1.0 - ndv * ndv).sqrt(), 0.0, ndv];
        let count = 4096u32;
        let mut sum = [0.0f64; 2];
        for k in 0..count {
            let u1 = (k as f32 + 0.5) / count as f32;
            let u2 = k.reverse_bits() as f32 / 4_294_967_296.0;
            let theta = (a2.sqrt() * (u1 / (1.0 - u1)).sqrt()).atan();
            let phi = std::f32::consts::TAU * u2;
            let h = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ];
            let vh = view[0] * h[0] + view[2] * h[2];
            let ndl = 2.0 * vh * h[2] - view[2];
            if ndl <= 0.0 || vh <= 0.0 {
                continue;
            }
            let g = pfx_materials::smith_g(ndv, ndl, a2);
            let weight = f64::from(g * vh / (h[2] * ndv));
            let fresnel = f64::from((1.0 - vh).powi(5));
            sum[0] += weight;
            sum[1] += weight * fresnel;
        }
        sum.map(|v| (v / f64::from(count)) as f32)
    }

    #[test]
    fn the_environment_brdf_follows_the_tracers_ggx() {
        let mut worst = [0.0f32; 2];
        for i in 0..20 {
            for j in 0..20 {
                let roughness = 0.05 + 0.95 * (i as f32 + 0.37) / 20.0;
                let ndv = 0.1 + 0.9 * (j as f32 + 0.61) / 20.0;
                let truth = traced_dfg(roughness, ndv);
                let fit = environment_dfg(roughness, ndv);
                let metal = (fit[0] - truth[0]).abs();
                let dielectric = (0.04 * (fit[0] - fit[1]) + fit[1]
                    - (0.04 * (truth[0] - truth[1]) + truth[1]))
                    .abs();
                worst = [worst[0].max(metal), worst[1].max(dielectric)];
            }
        }
        assert!(worst[0] < 0.02, "metal off by {}", worst[0]);
        assert!(worst[1] < 0.012, "dielectric off by {}", worst[1]);
    }

    #[test]
    fn a_rough_metal_keeps_the_light_the_old_fit_lost() {
        let truth = traced_dfg(0.35, 0.75);
        let fit = environment_dfg(0.35, 0.75);
        assert!(
            (fit[0] - truth[0]).abs() < 0.01,
            "{fit:?} against {truth:?}"
        );
        assert!(truth[0] > 0.95, "{truth:?}");
    }

    #[test]
    fn the_shader_carries_the_same_fit() {
        for row in DFG_FIT {
            let text = format!("vec3f({}, {}, {})", row[0], row[1], row[2]);
            assert!(PROBE_WGSL.contains(&text), "{text} is not in the shader");
        }
    }

    fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Matrix {
        let b = 1.0 / (fovy / 2.0).tan();
        let a = b / aspect;
        let c = far / (near - far);
        let d = far * near / (near - far);
        [
            [a, 0.0, 0.0, 0.0],
            [0.0, b, 0.0, 0.0],
            [0.0, 0.0, c, -1.0],
            [0.0, 0.0, d, 0.0],
        ]
    }

    fn inverse_perspective(m: &Matrix) -> Matrix {
        let (a, b, c, d) = (m[0][0], m[1][1], m[2][2], m[3][2]);
        [
            [1.0 / a, 0.0, 0.0, 0.0],
            [0.0, 1.0 / b, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0 / d],
            [0.0, 0.0, -1.0, c / d],
        ]
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Surface {
        Sky,
        Floor,
        Card,
        Wall,
    }

    struct Desk {
        gap: f32,
        width: u32,
        height: u32,
        projection: Matrix,
        pitch: f32,
        eye: [f32; 3],
        depth: Vec<f32>,
        surface: Vec<Surface>,
        position: Vec<[f32; 3]>,
    }

    const CARD_X: [f32; 2] = [-0.25, 0.06];
    const CARD_TOP: f32 = 0.4;
    const WALL_Z: f32 = -0.6;

    fn to_view(pitch: f32, eye: [f32; 3], world: [f32; 3]) -> [f32; 3] {
        let p = [world[0] - eye[0], world[1] - eye[1], world[2] - eye[2]];
        let (s, c) = pitch.sin_cos();
        [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c]
    }

    fn to_world(pitch: f32, v: [f32; 3]) -> [f32; 3] {
        let (s, c) = pitch.sin_cos();
        [v[0], v[1] * c + v[2] * s, -v[1] * s + v[2] * c]
    }

    fn cast(gap: f32, origin: [f32; 3], direction: [f32; 3]) -> Option<(f32, Surface)> {
        let mut best: Option<(f32, Surface)> = None;
        let mut keep = |t: f32, surface| {
            if t > 1e-5 && best.is_none_or(|(b, _)| t < b) {
                best = Some((t, surface));
            }
        };
        if direction[1] < 0.0 {
            keep(-origin[1] / direction[1], Surface::Floor);
        }
        if direction[2] != 0.0 {
            let t = -origin[2] / direction[2];
            let x = origin[0] + direction[0] * t;
            let y = origin[1] + direction[1] * t;
            if (CARD_X[0]..CARD_X[1]).contains(&x) && (gap..CARD_TOP).contains(&y) {
                keep(t, Surface::Card);
            }
            keep((WALL_Z - origin[2]) / direction[2], Surface::Wall);
        }
        best
    }

    fn pixel_ray(desk: &Desk, x: f32, y: f32) -> [f32; 3] {
        let ndc = [
            x / desk.width as f32 * 2.0 - 1.0,
            1.0 - y / desk.height as f32 * 2.0,
        ];
        let v = [
            ndc[0] / desk.projection[0][0],
            ndc[1] / desk.projection[1][1],
            -1.0,
        ];
        let length = dot(v, v).sqrt();
        v.map(|value| value / length)
    }

    fn desk(width: u32, height: u32) -> Desk {
        raised_desk(width, height, 0.03)
    }

    fn raised_desk(width: u32, height: u32, gap: f32) -> Desk {
        let projection = perspective(0.75, width as f32 / height as f32, 0.05, FAR);
        let pitch = 0.32f32;
        let eye = [-0.04, 0.32, 1.0];
        let mut d = Desk {
            gap,
            width,
            height,
            projection,
            pitch,
            eye,
            depth: Vec::new(),
            surface: Vec::new(),
            position: Vec::new(),
        };
        for y in 0..height {
            for x in 0..width {
                let ray = pixel_ray(&d, x as f32 + 0.5, y as f32 + 0.5);
                match cast(gap, eye, to_world(pitch, ray)) {
                    Some((t, surface)) => {
                        let view = ray.map(|value| value * t);
                        d.depth.push(-view[2]);
                        d.surface.push(surface);
                        d.position.push(view);
                    }
                    None => {
                        d.depth.push(FAR);
                        d.surface.push(Surface::Sky);
                        d.position.push([0.0, 0.0, -FAR]);
                    }
                }
            }
        }
        d
    }

    fn floor_normal(pitch: f32) -> [f32; 3] {
        let n = to_view(pitch, [0.0; 3], [0.0, 1.0, 0.0]);
        let length = dot(n, n).sqrt();
        n.map(|value| value / length)
    }

    fn reflect(v: [f32; 3], n: [f32; 3]) -> [f32; 3] {
        let d = 2.0 * dot(v, n);
        [v[0] - d * n[0], v[1] - d * n[1], v[2] - d * n[2]]
    }

    fn reflects_card(desk: &Desk, index: usize) -> bool {
        let origin = desk.position[index];
        let length = dot(origin, origin).sqrt();
        let n = floor_normal(desk.pitch);
        let direction = reflect(origin.map(|v| v / length), n);
        let image = DepthImage {
            width: desk.width,
            height: desk.height,
            depth: &desk.depth,
        };
        let depth = -origin[2];
        let start = std::array::from_fn(|axis| origin[axis] + n[axis] * depth * 0.002);
        march(
            &image,
            &desk.projection,
            start,
            direction,
            march_steps(0.05),
            THICKNESS,
        )
        .is_some_and(|hit| {
            let x = hit.pixel[0] as usize;
            let y = hit.pixel[1] as usize;
            desk.surface[y * desk.width as usize + x] == Surface::Card
        })
    }

    fn truly_reflects_card(desk: &Desk, index: usize) -> bool {
        let origin = desk.position[index];
        let length = dot(origin, origin).sqrt();
        let direction = reflect(origin.map(|v| v / length), floor_normal(desk.pitch));
        let world = to_world(desk.pitch, origin);
        let world = std::array::from_fn(|axis| world[axis] + desk.eye[axis]);
        match cast(desk.gap, world, to_world(desk.pitch, direction)) {
            Some((t, Surface::Card)) => {
                let hit = std::array::from_fn(|axis| origin[axis] + direction[axis] * t);
                let size = [desk.width as f32, desk.height as f32];
                let p = to_pixel(&desk.projection, size, hit);
                (0.0..size[0]).contains(&p[0]) && (0.0..size[1]).contains(&p[1])
            }
            _ => false,
        }
    }

    fn boundaries(
        desk: &Desk,
        card: impl Fn(usize) -> bool,
        rows: std::ops::Range<u32>,
    ) -> Vec<(u32, f32)> {
        let mut found = Vec::new();
        for y in rows {
            let row = (y * desk.width) as usize;
            let mut edge = None;
            for x in 1..desk.width as usize {
                let a = row + x - 1;
                let b = row + x;
                if desk.surface[a] != Surface::Floor || desk.surface[b] != Surface::Floor {
                    continue;
                }
                if card(a) && !card(b) {
                    edge = Some(x as f32);
                }
            }
            if let Some(x) = edge {
                found.push((y, x));
            }
        }
        found
    }

    #[test]
    fn reflected_vertical_edge_is_monotonic_and_step_free() {
        let desk = desk(640, 360);
        let rows = 190..330;
        let traced = boundaries(&desk, |i| reflects_card(&desk, i), rows.clone());
        let expected = boundaries(&desk, |i| truly_reflects_card(&desk, i), rows);
        assert!(traced.len() > 100, "{} rows reflect the edge", traced.len());
        let truth_at = expected
            .iter()
            .copied()
            .collect::<std::collections::BTreeMap<_, _>>();
        let direction = (traced.last().unwrap().1 - traced[0].1).signum();
        for pair in traced.windows(2) {
            let (y0, x0) = pair[0];
            let (y1, x1) = pair[1];
            assert_eq!(
                y1,
                y0 + 1,
                "the reflected edge breaks between rows {y0} and {y1}"
            );
            let step = x1 - x0;
            assert!(step.abs() <= 1.0, "a stair-step of {step} px at row {y1}");
        }
        for (i, &(y0, x0)) in traced.iter().enumerate() {
            for &(y1, x1) in &traced[i + 1..] {
                assert!(
                    (x1 - x0) * direction >= -1.0,
                    "the reflected edge turns back by more than a pixel: {x0} at row {y0}, {x1} at row {y1}"
                );
            }
        }
        for (y, x) in &traced {
            let want = truth_at[y];
            assert!(
                (x - want).abs() <= 1.5,
                "row {y}: the reflected edge at {x}, the true edge at {want}"
            );
        }
    }

    #[test]
    fn floor_behind_a_low_card_never_reflects_it() {
        for gap in [0.03, 0.008, 0.004] {
            let desk = raised_desk(640, 360, gap);
            let mut behind = 0;
            for index in 0..desk.surface.len() {
                if desk.surface[index] != Surface::Floor {
                    continue;
                }
                let world = to_world(desk.pitch, desk.position[index]);
                if world[2] + desk.eye[2] >= 0.0 {
                    continue;
                }
                behind += 1;
                assert!(
                    !reflects_card(&desk, index),
                    "floor pixel {} behind a card {gap} m off the floor reflects the card's face",
                    index
                );
            }
            assert!(behind > 10000, "{behind} floor pixels behind the card");
        }
    }

    #[test]
    fn reflected_points_land_within_a_pixel_of_the_truth() {
        let desk = desk(640, 360);
        let size = [desk.width as f32, desk.height as f32];
        let n = floor_normal(desk.pitch);
        let image = DepthImage {
            width: desk.width,
            height: desk.height,
            depth: &desk.depth,
        };
        let mut checked = 0;
        let mut missed = 0;
        let mut worst = 0.0f32;
        for y in (180..330).step_by(3) {
            for x in (200..400).step_by(3) {
                let index = y * desk.width as usize + x;
                if desk.surface[index] != Surface::Floor || !truly_reflects_card(&desk, index) {
                    continue;
                }
                let origin = desk.position[index];
                let length = dot(origin, origin).sqrt();
                let direction = reflect(origin.map(|v| v / length), n);
                let world = to_world(desk.pitch, origin);
                let world = std::array::from_fn(|axis| world[axis] + desk.eye[axis]);
                let (t, _) = cast(desk.gap, world, to_world(desk.pitch, direction)).unwrap();
                let want = to_pixel(
                    &desk.projection,
                    size,
                    std::array::from_fn(|axis| origin[axis] + direction[axis] * t),
                );
                let start = std::array::from_fn(|axis| origin[axis] - origin[2] * 0.002 * n[axis]);
                match march(
                    &image,
                    &desk.projection,
                    start,
                    direction,
                    march_steps(0.05),
                    THICKNESS,
                ) {
                    Some(hit) => {
                        let error = (hit.pixel[0] - want[0]).hypot(hit.pixel[1] - want[1]);
                        worst = worst.max(error);
                        checked += 1;
                    }
                    None => missed += 1,
                }
            }
        }
        assert!(checked > 1000, "{checked} reflected points checked");
        assert!(
            missed * 50 < checked,
            "{missed} of {checked} reflected points missed"
        );
        assert!(worst <= 1.0, "a reflected point {worst} px from the truth");
    }

    #[test]
    fn reflected_lower_edge_holds_on_every_column() {
        let desk = desk(640, 360);
        let width = desk.width as usize;
        let mut traced = Vec::new();
        let rise = |card: &dyn Fn(usize) -> bool, x: usize| {
            let mut seen = false;
            for y in (0..desk.height as usize).rev() {
                let index = y * width + x;
                if desk.surface[index] != Surface::Floor {
                    return None;
                }
                if card(index) {
                    seen = true;
                } else if seen {
                    return Some(y as f32);
                }
            }
            None
        };
        for x in 240..350usize {
            let first_wall = rise(&|i| reflects_card(&desk, i), x);
            let true_wall = rise(&|i| truly_reflects_card(&desk, i), x);
            if let (Some(found), Some(want)) = (first_wall, true_wall) {
                assert!(
                    (found - want).abs() <= 1.5,
                    "column {x}: the reflected lower edge at row {found}, the true edge at {want}"
                );
                traced.push(found);
            }
        }
        assert!(
            traced.len() > 80,
            "{} columns reflect the lower edge",
            traced.len()
        );
        for pair in traced.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= 1.0,
                "a stair-step along the lower edge: {} then {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn upsample_does_not_bleed_across_a_depth_edge() {
        let (width, height) = (8u32, 4u32);
        let up = [0.0, 0.0, 1.0];
        let near = Texel {
            colour: [1.0, 0.0, 0.0, 1.0],
            depth: 1.0,
            normal: up,
        };
        let far = Texel {
            colour: [0.0, 0.0, 1.0, 1.0],
            depth: 4.0,
            normal: up,
        };
        let texels = (0..width * height)
            .map(|i| if i % width < 4 { near } else { far })
            .collect::<Vec<_>>();
        for step in 0..=40 {
            let x = 0.25 * step as f32 / 40.0;
            for (depth, keep, lose, u) in [(4.0, 2, 0, 0.5 + x), (1.0, 0, 2, 0.4999 - x)] {
                let value = upsample(width, height, &texels, [u, 0.5], depth, up);
                assert!(
                    value[lose] == 0.0 && (value[keep] - 1.0).abs() < 1e-6,
                    "at u {u} and depth {depth}: {value:?}"
                );
            }
        }
        let side = Texel {
            colour: [0.0, 1.0, 0.0, 1.0],
            depth: 4.0,
            normal: [1.0, 0.0, 0.0],
        };
        let crease = (0..width * height)
            .map(|i| if i % width < 4 { side } else { far })
            .collect::<Vec<_>>();
        let value = upsample(width, height, &crease, [0.5, 0.5], 4.0, up);
        assert_eq!(
            value,
            [0.0, 0.0, 1.0, 1.0],
            "a crease at one depth keeps its faces apart"
        );
        let lone = upsample(width, height, &texels, [0.3, 0.5], 9.0, up);
        assert_eq!(lone, [0.0; 4], "nothing near falls back to the probes");
        let bilinear = upsample(width, height, &texels, [0.7, 0.5], 4.0, up);
        assert_eq!(bilinear, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn history_resets_on_a_cut() {
        use crate::renderer::History;
        let a = perspective(0.75, 16.0 / 9.0, 0.05, FAR);
        let mut b = a;
        b[3][0] = 0.4;
        let current = [0.2, 0.3, 0.4, 0.5];
        let old = [0.25, 0.35, 0.45, 0.55];
        let low = [0.1, 0.1, 0.1, 0.1];
        let high = [0.6, 0.6, 0.6, 0.6];
        let mut history = History::default();
        let mut frame = |n: f32, previous, now| {
            let valid = history.advance(n / 60.0, previous, now);
            (valid, accumulate(current, low, high, old, 1.0, 0.1, valid))
        };
        assert_eq!(frame(0.0, a, a), (false, current));
        let (valid, kept) = frame(1.0, a, a);
        assert!(
            valid && kept != current,
            "a continuing camera keeps history"
        );
        assert_eq!(frame(2.0, b, b), (false, current), "a cut drops history");
        let (valid, kept) = frame(3.0, b, b);
        assert!(valid && kept != current);
        let ghost = accumulate(current, low, high, [5.0, 5.0, 5.0, 1.0], 1.0, 0.1, true);
        assert!(
            ghost
                .iter()
                .zip(high)
                .all(|(value, top)| *value <= top + 1e-6),
            "history outside this frame's neighbourhood is clamped: {ghost:?}"
        );
        assert_eq!(
            accumulate(current, low, high, old, 0.0, 0.1, true),
            current,
            "a disoccluded texel has no history"
        );
    }

    #[test]
    fn steps_shrink_with_roughness_and_offsets_cover_the_quad() {
        assert!(march_steps(0.05) > march_steps(0.3));
        assert!(march_steps(0.3) > march_steps(0.8));
        assert_eq!(rough_fade(0.3), 1.0);
        assert_eq!(rough_fade(0.8), 0.0);
        assert!((0.0..1.0).contains(&rough_fade(0.7)));
        let mut seen = (0..4).map(trace_offset).collect::<Vec<_>>();
        seen.sort();
        assert_eq!(seen, vec![[0, 0], [0, 1], [1, 0], [1, 1]]);
    }

    #[test]
    fn shaders_validate() {
        for source in [
            TRACE_WGSL.to_string(),
            RESOLVE_WGSL.to_string(),
            format!("{}\n{}", pfx_materials::BRDF, PROBE_WGSL),
        ] {
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
    fn confidence_fades_at_each_edge() {
        assert_eq!(edge_confidence([0.0, 0.5], 0.1), 0.0);
        assert_eq!(edge_confidence([1.0, 0.5], 0.1), 0.0);
        assert_eq!(edge_confidence([0.5, 0.0], 0.1), 0.0);
        assert_eq!(edge_confidence([0.5, 1.0], 0.1), 0.0);
        assert!((edge_confidence([0.05, 0.5], 0.1) - 0.5).abs() < 1e-6);
        assert_eq!(edge_confidence([0.5, 0.5], 0.1), 1.0);
    }

    struct Views {
        depth: wgpu::TextureView,
        normal: wgpu::TextureView,
        colour: wgpu::TextureView,
        motion: wgpu::TextureView,
    }

    fn upload(
        gpu: &pfx_gpu::Gpu,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        bytes: &[u8],
    ) -> wgpu::TextureView {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection test input"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let stride = bytes.len() as u32 / (width * height);
        gpu.queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * stride),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&Default::default())
    }

    fn halves(values: impl Iterator<Item = f32>) -> Vec<u8> {
        values
            .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            .collect()
    }

    fn views(gpu: &pfx_gpu::Gpu, desk: &Desk, card: [f32; 3]) -> Views {
        let (w, h) = (desk.width, desk.height);
        let depth_bytes = desk
            .depth
            .iter()
            .flat_map(|d| d.to_le_bytes())
            .collect::<Vec<_>>();
        let up = floor_normal(desk.pitch);
        let normals = desk.surface.iter().flat_map(|surface| match surface {
            Surface::Floor => [up[0], up[1], up[2], 0.05],
            Surface::Sky => [0.0; 4],
            _ => [0.0, 0.0, 1.0, 0.6],
        });
        let colours = desk.surface.iter().flat_map(|surface| match surface {
            Surface::Card => [card[0], card[1], card[2], 1.0],
            Surface::Wall => [0.0, 0.0, 1.0, 1.0],
            Surface::Floor | Surface::Sky => [0.0, 0.0, 0.0, 1.0],
        });
        Views {
            depth: upload(gpu, w, h, wgpu::TextureFormat::R32Float, &depth_bytes),
            normal: upload(
                gpu,
                w,
                h,
                wgpu::TextureFormat::Rgba16Float,
                &halves(normals),
            ),
            colour: upload(
                gpu,
                w,
                h,
                wgpu::TextureFormat::Rgba16Float,
                &halves(colours),
            ),
            motion: upload(
                gpu,
                w,
                h,
                wgpu::TextureFormat::Rg16Float,
                &vec![0u8; (w * h * 4) as usize],
            ),
        }
    }

    fn inputs<'a>(
        views: &'a Views,
        desk: &Desk,
        frame_index: u32,
        history_valid: bool,
    ) -> Inputs<'a> {
        Inputs {
            depth: &views.depth,
            normal_roughness: &views.normal,
            colour: &views.colour,
            motion: &views.motion,
            projection: desk.projection,
            inverse_projection: inverse_perspective(&desk.projection),
            frame_index,
            thickness: THICKNESS,
            history_valid,
        }
    }

    fn run(
        gpu: &pfx_gpu::Gpu,
        pass: &mut Pass,
        views: &Views,
        desk: &Desk,
        frame_index: u32,
        history_valid: bool,
    ) {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        pass.encode(
            &gpu.device,
            &mut encoder,
            &inputs(views, desk, frame_index, history_valid),
            None,
        );
        gpu.queue.submit(Some(encoder.finish()));
    }

    fn read(gpu: &pfx_gpu::Gpu, texture: &wgpu::Texture) -> Vec<[f32; 4]> {
        let (width, height) = (texture.width(), texture.height());
        let row = (width * 8).div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reflection test readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
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
        gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        let mut out = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            let start = (y * row) as usize;
            for pixel in mapped[start..start + width as usize * 8].chunks_exact(8) {
                out.push(std::array::from_fn(|c| {
                    half::f16::from_bits(u16::from_le_bytes([pixel[2 * c], pixel[2 * c + 1]]))
                        .to_f32()
                }));
            }
        }
        out
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_reflected_edge_holds_and_resets_on_a_cut() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let desk = desk(640, 360);
        let red = views(&gpu, &desk, [1.0, 0.0, 0.0]);
        let mut pass = Pass::new(&gpu.device, desk.width, desk.height);
        run(&gpu, &mut pass, &red, &desk, 0, false);
        let raw = read(&gpu, &pass._raw[0]);
        let (hw, hh) = (
            pass._raw[0].width() as usize,
            pass._raw[0].height() as usize,
        );
        let width = desk.width as usize;
        let mut edges = Vec::new();
        for y in 0..hh {
            let mut edge = None;
            for x in 1..hw {
                let floor = desk.surface[2 * y * width + 2 * x] == Surface::Floor
                    && desk.surface[2 * y * width + 2 * x - 2] == Surface::Floor;
                let card = |texel: [f32; 4]| texel[3] > 0.05 && texel[0] > texel[2];
                if floor && card(raw[y * hw + x - 1]) && !card(raw[y * hw + x]) {
                    edge = Some(x as f32);
                }
            }
            if let Some(x) = edge {
                edges.push((y, x));
            }
        }
        assert!(
            edges.len() > 30,
            "{} half rows reflect the edge",
            edges.len()
        );
        for pair in edges.windows(2) {
            assert!(
                pair[1].0 == pair[0].0 + 1 && (pair[1].1 - pair[0].1).abs() <= 1.0,
                "a stair-step on the GPU between half rows {} and {}: {} then {}",
                pair[0].0,
                pair[1].0,
                pair[0].1,
                pair[1].1
            );
        }
        for frame in 1..8 {
            run(&gpu, &mut pass, &red, &desk, frame, true);
        }
        let kept = read(&gpu, &pass._results[1 - pass.next]);
        assert!(
            kept.iter().any(|texel| texel[0] > 0.1),
            "red history builds up"
        );
        let green = views(&gpu, &desk, [0.0, 1.0, 0.0]);
        run(&gpu, &mut pass, &green, &desk, 8, false);
        let after = read(&gpu, &pass._results[1 - pass.next]);
        let fresh = read(&gpu, &pass._raw[0]);
        assert_eq!(
            after, fresh,
            "after a cut the result is this frame's trace alone"
        );
        assert!(
            after.iter().all(|texel| texel[0] < 1e-3),
            "no red survives the cut"
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_bench_4k() {
        let gpu = &pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let desk = desk(3840, 2160);
        let views = views(gpu, &desk, [1.0, 0.0, 0.0]);
        let mut pass = Pass::new(&gpu.device, desk.width, desk.height);
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut samples: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
        let mut turns = pfx_gpu::pace::Turns::default();
        for frame_index in 0..14 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            pass.encode(
                &gpu.device,
                &mut encoder,
                &inputs(&views, &desk, frame_index, frame_index > 0),
                Some(&mut profiler),
            );
            let slot = profiler.finish(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            if let Some(slot) = slot {
                profiler.submitted(slot);
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let timings: Vec<_> = profiler
                    .collect(&gpu.device)
                    .into_iter()
                    .flatten()
                    .collect();
                turns.add(timings.iter().map(|timing| timing.milliseconds).sum());
                if frame_index >= 4 {
                    for timing in timings {
                        samples
                            .entry(timing.label)
                            .or_default()
                            .push(timing.milliseconds);
                    }
                }
            }
        }
        assert!(!samples.is_empty(), "GPU timestamp query unavailable");
        let mut total = 0.0;
        for (label, values) in &mut samples {
            values.sort_by(f64::total_cmp);
            let median = values[values.len() / 2];
            total += median;
            eprintln!("4K {label} median: {median:.3} ms");
        }
        eprintln!("4K SSR median total: {total:.3} ms; target 1.000 ms");
        assert!(
            total <= 1.0,
            "4K SSR missed the 1.0 ms target: {total:.3} ms"
        );
        let result = read(gpu, &pass._results[1 - pass.next]);
        let hits = result.iter().filter(|texel| texel[3] > 0.0).count();
        eprintln!("4K SSR confident texels: {hits}");
        assert!(hits > 0, "synthetic scene produced no SSR hits");
    }
}
