use bytemuck::{Pod, Zeroable};
use pfx_bake::detail::atlas::{Atlas, Format, Texture, level_size};
use pfx_gpu::wgpu;
use pfx_load::{ColorSpace, Image, Pixels};
use pfx_materials::{
    CoatWobble, GPU_PARAMS, LAYER_CAP, Material, NoiseKind, NoiseLayer, NormalSource, PARAM_ROWS,
    encode_channel, linear_channel,
};

use crate::{lacquer, noise_tiles};

pub const ANISOTROPY: u16 = 16;
pub const CONTENT_LOD_BIAS: f32 = -0.25;
pub const DETAIL_LOD_BIAS: f32 = -0.25;
pub const CONTENT_SLOTS: usize = 4;
pub const CONTENT_BINDING: u32 = 18;
pub const CAUSTIC_BINDING: u32 = CONTENT_BINDING + CONTENT_SLOTS as u32 + 5;
pub const CONTENT_WGSL: &str = r#"
override use_content: bool = true;
override use_content_cutout: bool = true;
@group(3) @binding(18) var content0: texture_2d<f32>;
@group(3) @binding(19) var content1: texture_2d<f32>;
@group(3) @binding(20) var content2: texture_2d<f32>;
@group(3) @binding(21) var content3: texture_2d<f32>;
@group(3) @binding(22) var content_sampler: sampler;
struct ContentCaustic {
    rect: vec4f,
    plane: vec4f,
    info: vec4f,
}
@group(3) @binding(27) var<uniform> content_caustic: ContentCaustic;
fn caustic_uv(world: vec3f) -> vec2f {
    let toward = content_caustic.plane.xyz;
    let along = (content_caustic.plane.w - world.y) / max(toward.y, 1e-4);
    let q = world.xz + toward.xz * along;
    return (q - content_caustic.rect.xy) / content_caustic.rect.zw;
}
fn caustic_gain(at: vec2f, dx: vec2f, dy: vec2f) -> vec3f {
    if (content_caustic.info.z < 0.5 || any(at < vec2f(0.0)) || any(at > vec2f(1.0))) {
        return vec3f(1.0);
    }
    let texel = content_texel(i32(content_caustic.info.x), at, dx, dy).rgb;
    return max(vec3f(1.0) + texel * content_caustic.info.y, vec3f(0.0));
}
fn content_size(slot: i32) -> vec2f {
    switch slot {
        case 0: { return vec2f(textureDimensions(content0)); }
        case 1: { return vec2f(textureDimensions(content1)); }
        case 2: { return vec2f(textureDimensions(content2)); }
        case 3: { return vec2f(textureDimensions(content3)); }
        default: { return vec2f(1.0); }
    }
}
fn content_texel(slot: i32, at: vec2f, dx: vec2f, dy: vec2f) -> vec4f {
    switch slot {
        case 0: { return textureSampleGrad(content0, content_sampler, at, dx, dy); }
        case 1: { return textureSampleGrad(content1, content_sampler, at, dx, dy); }
        case 2: { return textureSampleGrad(content2, content_sampler, at, dx, dy); }
        case 3: { return textureSampleGrad(content3, content_sampler, at, dx, dy); }
        default: { return vec4f(0.0); }
    }
}
fn content_alpha(slot: i32, at: vec2f) -> f32 {
    switch slot {
        case 0: { return textureSampleLevel(content0, content_sampler, at, 0.0).a; }
        case 1: { return textureSampleLevel(content1, content_sampler, at, 0.0).a; }
        case 2: { return textureSampleLevel(content2, content_sampler, at, 0.0).a; }
        case 3: { return textureSampleLevel(content3, content_sampler, at, 0.0).a; }
        default: { return 0.0; }
    }
}
fn content_cut(material_index: u32, uv: vec2f, transform: vec4f, crop: vec4f, cutoff: f32) -> bool {
    let slot = i32(round(map_materials[material_index].content.x));
    if (slot < 0) {
        return true;
    }
    let at = content_uv(uv, transform);
    if (!content_inside(at, crop)) {
        return true;
    }
    return content_alpha(slot, at) < cutoff;
}
fn content_apply(surface_in: Shaded, material_index: u32, uv: vec2f, uv_dx: vec2f, uv_dy: vec2f, transform: vec4f, crop: vec4f, tangent: vec3f, bitangent: vec3f, geometric: vec3f) -> Shaded {
    var surface = surface_in;
    if (!use_content) {
        return surface;
    }
    let layer = map_materials[material_index].content;
    let slot = i32(round(layer.x));
    if (slot < 0) {
        return surface;
    }
    let at = content_uv(uv, transform);
    if (!content_inside(at, crop)) {
        return surface;
    }
    let dx = uv_dx * transform.zw;
    let dy = uv_dy * transform.zw;
    let sharpen = map_materials[material_index].content_extra.y;
    let texel = content_texel(slot, at, dx * sharpen, dy * sharpen);
    let blend = u32(round(layer.y));
    var out = content_composite(surface, blend, texel, layer.z, map_materials[material_index].content_extra.x);
    if (layer.w != 0.0) {
        let step = max(1.5 / content_size(slot), max(abs(dx), abs(dy)));
        let ax = content_texel(slot, at + vec2f(step.x, 0.0), dx, dy).a - content_texel(slot, at - vec2f(step.x, 0.0), dx, dy).a;
        let ay = content_texel(slot, at + vec2f(0.0, step.y), dx, dy).a - content_texel(slot, at - vec2f(0.0, step.y), dx, dy).a;
        var n = content_emboss(out.normal, tangent, bitangent, vec2f(ax, ay), layer.w);
        if (dot(n, geometric) < 0.05) {
            n = normalize(n + geometric * 0.1);
        }
        out.normal = n;
    }
    return out;
}
"#;
pub const MAPS_WGSL: &str = r#"
struct MapMaterial {
    indices: vec4i,
    factors: vec4f,
    age_a: vec4f,
    age_b: vec4f,
    seeds: vec4u,
    layer0: vec4f,
    layer1: vec4f,
    layer2: vec4f,
    layer3: vec4f,
    content: vec4f,
    content_extra: vec4f,
    packed: vec4f,
    atlas: vec4f,
    params: array<vec4f, 16>,
}
struct MapResult {
    base: vec3f,
    roughness: f32,
    metalness: f32,
    normal: vec3f,
    coat_roughness: f32,
    thin_film: f32,
}
@group(3) @binding(0) var map_base: texture_2d_array<f32>;
@group(3) @binding(1) var map_normal: texture_2d_array<f32>;
@group(3) @binding(2) var map_roughness: texture_2d_array<f32>;
@group(3) @binding(3) var map_metal: texture_2d_array<f32>;
@group(3) @binding(4) var map_sampler: sampler;
@group(3) @binding(5) var<storage, read> map_materials: array<MapMaterial>;
override use_maps: bool = true;
override use_detail: bool = true;
override use_ageing: bool = true;
override use_fine_noise: bool = false;
override use_uv1: bool = true;
override use_lacquer: bool = true;
override use_procedural_lacquer: bool = false;
override use_scratch: bool = true;
override use_grime: bool = true;
override use_wood: bool = true;
override use_wall: bool = true;
override use_fibre: bool = true;
override use_crinkle: bool = true;
override use_procedural_detail: bool = false;
var<private> map_ink_base: vec4f;
var<private> map_pixel: f32;
var<private> map_dx: vec3f;
var<private> map_dy: vec3f;
var<private> map_planes: vec3f = vec3f(0.0, 0.0, 1.0);
var<private> map_gain: f32 = 1.0;
const tile_cells: f32 = 64.0;
const lacquer_broad_range: f32 = 2.5;
const lacquer_fine_range: f32 = 1.5;
const lacquer_blend: f32 = 0.5;
const map_ring_mean: f32 = 0.25;
const map_line_mean: f32 = 0.0897;
const map_wear_mean: f32 = 0.3814;
const map_speck_mean: f32 = 0.0015;
fn map_footprint(position: vec3f) {
    let dx = dpdx(position);
    let dy = dpdy(position);
    map_pixel = max(length(dx), length(dy));
    if ((use_lacquer && !use_procedural_lacquer) || map_tiled()) {
        map_dx = dx;
        map_dy = dy;
        let face = cross(dx, dy);
        let size = length(face);
        var w = vec3f(0.0, 0.0, 1.0);
        if (size > 0.0) {
            w = max(abs(face) / size - vec3f(lacquer_blend), vec3f(0.0));
            w = w / (w.x + w.y + w.z);
        }
        map_planes = w;
        map_gain = inverseSqrt(dot(w, w));
    }
}
fn map_tiled() -> bool {
    return !use_procedural_detail && (use_scratch || use_wall || use_fibre || use_crinkle);
}
fn lacquer_plane(at: vec2f, dx: vec2f, dy: vec2f, layer: i32, fine_scale: f32) -> vec2f {
    let broad = textureSampleGrad(map_metal, map_sampler, at, layer, dx, dy).rg;
    let fine = textureSampleGrad(map_metal, map_sampler, at * fine_scale, layer, dx * fine_scale, dy * fine_scale).ba;
    return (broad * 2.0 - 1.0) * lacquer_broad_range + (fine * 2.0 - 1.0) * lacquer_fine_range;
}
fn map_lacquer(p: vec3f, amp: f32, tile: f32, a: vec4f, b: vec4f, d: vec4f) -> vec3f {
    if (use_procedural_lacquer || tile < 0.0 || d.w == 0.0) {
        return value_noise3_gradient(p, a.x) * amp + value_noise3_gradient(p, a.y) * (amp * a.w) + value_noise3_gradient(p, a.z) * (amp * b.x);
    }
    let layer = i32(tile);
    let w = map_planes;
    var g = vec3f(0.0);
    if (w.z > 0.0) {
        g += vec3f(lacquer_plane(p.xy, map_dx.xy, map_dy.xy, layer, d.z), 0.0) * w.z;
    }
    if (w.x > 0.0) {
        g += vec3f(0.0, lacquer_plane(p.yz + vec2f(0.31, 0.67), map_dx.yz, map_dy.yz, layer, d.z)) * w.x;
    }
    if (w.y > 0.0) {
        let t = lacquer_plane(p.zx + vec2f(0.53, 0.19), map_dx.zx, map_dy.zx, layer, d.z);
        g += vec3f(t.y, 0.0, t.x) * w.y;
    }
    return g * (amp * map_gain);
}
fn map_weight(rate: f32) -> f32 {
    if (use_fine_noise) {
        return 1.0;
    }
    return octave_weight(map_pixel * rate);
}
fn map_faded(mean: f32, value: f32, weight: f32) -> f32 {
    return select(mix(mean, value, weight), value, weight >= 1.0);
}
fn map_noise(q: vec3f, rate: f32) -> f32 {
    let weight = map_weight(rate);
    if (weight <= 0.0) {
        return 0.5;
    }
    return map_faded(0.5, value_noise3(q, false), weight);
}
fn tile_pair(at: vec2f, dx: vec2f, dy: vec2f, layer: i32, high: bool) -> vec2f {
    let s = 1.0 / tile_cells;
    let t = textureSampleGrad(map_metal, map_sampler, at * s, layer, dx * s, dy * s);
    return select(t.xy, t.zw, high);
}
fn tile_slice(pair: vec2f, depth: f32) -> f32 {
    let k = floor(depth);
    let w = fade_cubic(depth - k);
    let odd = k - 2.0 * floor(k * 0.5) > 0.5;
    return select(mix(pair.x, pair.y, w), mix(pair.y, pair.x, w), odd);
}
fn tile_flat(at: vec2f, dx: vec2f, dy: vec2f, depth: f32, tile: f32, octave: u32) -> f32 {
    return tile_slice(tile_pair(at, dx, dy, i32(tile) + 1 + i32(octave >> 1u), (octave & 1u) == 1u), depth);
}
fn tile_noise(q: vec3f, rate: vec3f, tile: f32, octave: u32) -> f32 {
    if (map_weight(max(rate.x, max(rate.y, rate.z))) <= 0.0) {
        return 0.5;
    }
    let dx = map_dx * rate;
    let dy = map_dy * rate;
    let w = map_planes;
    var sum = 0.0;
    if (w.z > 0.0) {
        sum += (tile_flat(q.xy, dx.xy, dy.xy, q.z, tile, octave) - 0.5) * w.z;
    }
    if (w.x > 0.0) {
        sum += (tile_flat(q.yz + vec2f(17.0, 41.0), dx.yz, dy.yz, q.x, tile, octave) - 0.5) * w.x;
    }
    if (w.y > 0.0) {
        sum += (tile_flat(q.zx + vec2f(29.0, 7.0), dx.zx, dy.zx, q.y, tile, octave) - 0.5) * w.y;
    }
    return 0.5 + sum * map_gain;
}
fn tile_gradient(at: vec2f, dx: vec2f, dy: vec2f, layer: i32) -> vec2f {
    let s = 1.0 / tile_cells;
    let t = textureSampleGrad(map_metal, map_sampler, at * s, layer, dx * s, dy * s).ba;
    return (t * 2.0 - 1.0) * lacquer_fine_range;
}
fn tile_wobble(p: vec3f, scale: f32, shift: vec2f, tile: f32) -> vec3f {
    if (map_weight(scale) <= 0.0) {
        return vec3f(0.0);
    }
    let q = p * scale;
    let dx = map_dx * scale;
    let dy = map_dy * scale;
    let layer = i32(tile);
    let w = map_planes;
    var g = vec3f(0.0);
    if (w.z > 0.0) {
        g += vec3f(tile_gradient(q.xy + shift, dx.xy, dy.xy, layer), 0.0) * w.z;
    }
    if (w.x > 0.0) {
        g += vec3f(0.0, tile_gradient(q.yz + shift.yx, dx.yz, dy.yz, layer)) * w.x;
    }
    if (w.y > 0.0) {
        let t = tile_gradient(q.zx + shift * 1.7, dx.zx, dy.zx, layer);
        g += vec3f(t.y, 0.0, t.x) * w.y;
    }
    return g * map_gain;
}
fn ink_sample(at: vec2f) -> vec4f {
    return map_ink_base;
}
fn map_perturb(n: vec3f, g: vec3f) -> vec3f {
    return normalize(n - (g - n * dot(g, n)));
}
fn map_scratch_coat(base: vec3f, roughness: f32, amount: f32, position: vec3f, seed: u32) -> vec4f {
    if (amount <= 0.0) {
        return vec4f(base, roughness);
    }
    let q = position * 900.0;
    let n = value_noise3(vec3f(q.x * 0.02 + f32(seed), q.y, q.z * 0.02), false);
    let line = 1.0 - smoothstep(0.0, 0.045, abs(n - 0.5));
    let smudge = smoothstep(0.52, 0.78, value_noise3(position * 22.0, false) * 0.65 + value_noise3(position * 61.0, false) * 0.35);
    let cell = vec3i(floor(position * 2600.0));
    let speck = select(0.0, 1.0, hash3_xor(vec3i(cell.x, cell.y, cell.z ^ i32(seed))) > 0.9985);
    let cover = min(amount, 1.0);
    let breakup = clamp(smudge * 0.55 + line * 0.35 + speck, 0.0, 1.0) * cover;
    let rough2 = roughness * roughness;
    let mixed = mix(rough2, min(rough2 * 4.0 + 0.02, 0.35), breakup);
    let albedo = mix(base, base * 0.9 + vec3f(0.06), speck * cover);
    return vec4f(albedo, sqrt(mixed));
}
fn map_scratch(base: vec3f, roughness: f32, amount: f32, position: vec3f, seed: u32, coated: bool, tile: f32, scales: vec4f, stretch: f32) -> vec4f {
    if (amount <= 0.0) {
        return vec4f(base, roughness);
    }
    let tiled = !use_procedural_detail && tile >= 0.0;
    var line = map_line_mean;
    let line_weight = map_weight(scales.x);
    if (line_weight > 0.0) {
        let at = vec3f(position.x * stretch + f32(seed), position.y * scales.x, position.z * stretch);
        var n: f32;
        if (tiled) {
            n = tile_noise(at, vec3f(stretch, scales.x, stretch), tile, 2u);
        } else {
            n = value_noise3(at, false);
        }
        line = map_faded(map_line_mean, 1.0 - smoothstep(0.0, 0.045, abs(n - 0.5)), line_weight);
    }
    var smudge: f32;
    if (tiled) {
        smudge = smoothstep(0.52, 0.78, tile_noise(position * scales.y, vec3f(scales.y), tile, 3u) * 0.65 + tile_noise(position * scales.z, vec3f(scales.z), tile, 1u) * 0.35);
    } else {
        smudge = smoothstep(0.52, 0.78, map_noise(position * scales.y, scales.y) * 0.65 + map_noise(position * scales.z, scales.z) * 0.35);
    }
    var speck = map_speck_mean;
    let speck_weight = map_weight(scales.w);
    if (speck_weight > 0.0) {
        let cell = vec3i(floor(position * scales.w));
        speck = map_faded(map_speck_mean, select(0.0, 1.0, hash3_xor(vec3i(cell.x, cell.y, cell.z ^ i32(seed))) > 0.9985), speck_weight);
    }
    let cover = min(amount, 1.0);
    let breakup = clamp(smudge * 0.55 + line * 0.35 + speck, 0.0, 1.0) * cover;
    let rough2 = roughness * roughness;
    var mixed = mix(rough2, min(rough2 * 3.0 + 0.03, 0.5), breakup);
    if (coated) {
        mixed = mix(rough2, min(rough2 * 4.0 + 0.02, 0.35), breakup);
    }
    let albedo = mix(base, base * 0.9 + vec3f(0.06), speck * cover);
    return vec4f(albedo, sqrt(mixed));
}
fn map_layer(result_in: MapResult, layer: vec4f, pa: vec4f, pb: vec4f, pc: vec4f, pd: vec4f, uv: vec2f, position: vec3f, time: f32, coated: bool, tile: f32) -> MapResult {
    var result = result_in;
    let kind = u32(round(layer.x));
    let amp = layer.z;
    if (kind == 0u || amp == 0.0) {
        return result;
    }
    let seed = bitcast<u32>(layer.w);
    let p = position + vec3f(f32(seed) * 0.001, f32(seed) * 0.001, 0.0);
    let tiled = !use_procedural_detail && tile >= 0.0;
    if (use_fibre && kind == 1u) {
        var fibre: f32;
        var mottle: f32;
        var g: vec3f;
        let streak = vec3f(pa.x, pa.y, pa.x);
        if (tiled) {
            fibre = tile_noise(p * streak, streak, tile, 0u) * 0.6 + tile_noise(p * pa.z, vec3f(pa.z), tile, 1u) * 0.4;
            mottle = tile_noise(p * pa.w, vec3f(pa.w), tile, 2u);
            g = tile_wobble(p, pb.x, vec2f(3.0, 5.0), tile) * (pc.x * amp) + tile_wobble(p, pb.y, vec2f(11.0, 23.0), tile) * (pc.y * amp);
        } else {
            fibre = map_noise(p * streak, max(pa.x, pa.y)) * 0.6 + map_noise(p * pa.z, pa.z) * 0.4;
            mottle = map_noise(p * pa.w, pa.w);
            g = value_noise3_gradient(p, pb.x) * (pc.x * amp) + value_noise3_gradient(p, pb.y) * (pc.y * amp);
        }
        result.base *= 1.0 + ((fibre - 0.5) * pb.z + (mottle - 0.5) * pb.w) * amp;
        result.normal = map_perturb(result.normal, g);
    } else if (use_crinkle && kind == 2u) {
        var g: vec3f;
        if (tiled) {
            g = tile_wobble(p, pa.x, vec2f(7.0, 19.0), tile) * (pa.z * amp) + tile_wobble(p, pa.y, vec2f(31.0, 2.0), tile) * (pa.w * amp);
        } else {
            g = value_noise3_gradient(p, pa.x) * (pa.z * amp) + value_noise3_gradient(p, pa.y) * (pa.w * amp);
        }
        result.normal = map_perturb(result.normal, g);
    } else if (use_wood && kind == 3u) {
        let plank = floor(p.z * pd.x);
        let tone = hash3_xor(vec3i(i32(plank), 7, 3));
        let across = fract(p.z * pd.x);
        let seam = smoothstep(0.0, 0.006, across) + smoothstep(0.0, 0.006, 1.0 - across) - 1.0;
        var grain = map_ring_mean;
        let ring_weight = map_weight(pd.z);
        if (ring_weight > 0.0) {
            let warp = map_noise(vec3f(p.x * 1.4, plank * 3.7, p.z * 6.0), 6.0) * 0.9 + map_noise(vec3f(p.x * 6.0, plank, p.z * 20.0), 20.0) * 0.25;
            let rings = fract(fma(fma(warp, pa.z, p.z), pa.y, map_noise(vec3f(p.x * 0.8, plank, 0.0), 0.8) * pc.y));
            grain = map_faded(map_ring_mean, smoothstep(0.55, 0.95, rings), ring_weight);
        }
        let figure = grain * 0.55 + map_noise(vec3f(p.x * pa.w, p.z * pb.x, plank), pb.x) * 0.25;
        let factor = (0.78 + tone * pb.y) * (1.08 - figure * pb.z) * (pb.w + clamp(seam, 0.0, 1.0) * pd.y);
        result.base *= mix(1.0, factor, clamp(amp, 0.0, 1.0));
        let alpha = result.roughness * result.roughness;
        result.roughness = mix(result.roughness, sqrt(min(alpha * (0.85 + figure * pc.x), 1.0)), clamp(amp, 0.0, 1.0));
    } else if (use_wall && kind == 4u) {
        let broad = map_noise(p * pa.x, pa.x) * 0.6 + map_noise(p * pa.y, pa.y) * 0.4;
        var fine: f32;
        var g: vec3f;
        if (tiled) {
            fine = tile_noise(p * pa.z, vec3f(pa.z), tile, 2u) * 0.6 + tile_noise(p * pa.w, vec3f(pa.w), tile, 3u) * 0.4;
            g = tile_wobble(p, pb.x, vec2f(5.0, 13.0), tile) * (pc.x * amp) + tile_wobble(p, pb.y, vec2f(29.0, 3.0), tile) * (pc.y * amp);
        } else {
            fine = map_noise(p * pa.z, pa.z) * 0.6 + map_noise(p * pa.w, pa.w) * 0.4;
            g = value_noise3_gradient(p, pb.x) * (pc.x * amp) + value_noise3_gradient(p, pb.y) * (pc.y * amp);
        }
        result.base *= mix(1.0, 1.0 + (broad - 0.5) * pb.z + (fine - 0.5) * pb.w, clamp(amp, 0.0, 1.0));
        result.normal = map_perturb(result.normal, g);
    } else if (use_grime && kind == 5u) {
        var wear = map_wear_mean;
        let grime_weight = map_weight(pa.x);
        if (grime_weight > 0.0) {
            let grime = value_noise3(p * pa.x, false) * 0.6 + map_noise(p * pa.y, pa.y) * 0.4;
            wear = map_faded(map_wear_mean, smoothstep(0.35, 0.75, grime), grime_weight);
        }
        let tint = mix(vec3f(pa.z, pa.w, pb.x), vec3f(pb.y, pb.z, pb.w), wear);
        result.base *= mix(vec3f(1.0), tint, clamp(amp, 0.0, 1.0));
        let alpha = result.roughness * result.roughness;
        result.roughness = mix(result.roughness, sqrt(min(alpha * mix(pc.x, pc.y, wear), 1.0)), clamp(amp, 0.0, 1.0));
    } else if (kind == 7u) {
        let spots = fbm3(vec3f(uv * 12.0, 2.0), 4, true);
        result.base *= mix(1.0, 0.7 + 0.6 * spots, clamp(amp, 0.0, 1.0));
    } else if (kind == 8u) {
        let value = fbm3(p * 300.0, 4, true);
        result.base *= mix(1.0, 0.75 + 0.5 * value, clamp(amp, 0.0, 1.0));
    } else if (use_scratch && kind == 17u) {
        if (coated) {
            let scratched = map_scratch(result.base, result.coat_roughness, amp, p, seed, true, tile, pa, pd.x);
            result.base = scratched.xyz;
            result.coat_roughness = scratched.w;
        } else {
            let scratched = map_scratch(result.base, result.roughness, amp, p, seed, false, tile, pa, pd.x);
            result.base = scratched.xyz;
            result.roughness = scratched.w;
        }
    } else if (use_lacquer && kind == 18u) {
        result.normal = map_perturb(result.normal, map_lacquer(p, amp, tile, pa, pb, pd));
    } else if (kind == 14u) {
        let value = flow(uv * layer.y, time);
        result.thin_film = max(result.thin_film + amp * (value - 0.5) * 2.0, 0.0);
    } else if (kind == 15u) {
        let value = map_noise(p * layer.y, abs(layer.y));
        result.base *= mix(1.0, 1.0 + (value - 0.5) * 2.0, clamp(amp, 0.0, 1.0));
    } else if (kind == 16u) {
        let value = fbm3(p * layer.y, 4, true);
        result.base *= mix(1.0, 1.0 + (value - 0.5) * 2.0, clamp(amp, 0.0, 1.0));
    }
    return result;
}
fn map_evaluate(base: vec3f, roughness: f32, metalness: f32, coat_roughness: f32, thin_film: f32, normal: vec3f, tangent: vec4f, uv: vec2f, object_position: vec3f, edge: f32, age_in: f32, time: f32, material_index: u32) -> MapResult {
    return map_evaluate_with(base, roughness, metalness, coat_roughness, thin_film, normal, tangent, uv, uv, object_position, edge, age_in, time, material_index);
}
fn map_evaluate_with(base: vec3f, roughness: f32, metalness: f32, coat_roughness: f32, thin_film: f32, normal: vec3f, tangent: vec4f, uv: vec2f, uv1: vec2f, object_position: vec3f, edge: f32, age_in: f32, time: f32, material_index: u32) -> MapResult {
    let material = map_materials[material_index];
    let baked = material.seeds.w != 0u;
    var atlas_uv = uv;
    if (use_uv1 && material.packed.y != 0.0) {
        atlas_uv = uv1;
    }
    let tiled_uv = select(uv * select(max(material.factors.x, 0.000001), 1.0, baked), atlas_uv * material.atlas.xy + material.atlas.zw, material.seeds.w >= 2u);
    var result = MapResult(base, roughness, metalness, normalize(normal), coat_roughness, thin_film);
    let tile_dx = dpdx(tiled_uv) * material.content_extra.z;
    let tile_dy = dpdy(tiled_uv) * material.content_extra.z;
    if (use_maps && material.indices.x >= 0) {
        let sample = textureSampleGrad(map_base, map_sampler, tiled_uv, material.indices.x, tile_dx, tile_dy);
        if (baked) {
            result.base = sample.rgb;
        } else {
            result.base *= mix(vec3f(1.0), sample.rgb, clamp(material.factors.y, 0.0, 1.0));
        }
    }
    if (use_maps && material.indices.z >= 0) {
        let sampled_roughness = textureSampleGrad(map_roughness, map_sampler, tiled_uv, material.indices.z, tile_dx, tile_dy).r;
        if (baked) {
            result.roughness = sampled_roughness;
        } else {
            result.roughness *= sampled_roughness;
        }
    }
    if (use_maps && material.indices.w >= 0) {
        result.metalness *= textureSampleGrad(map_metal, map_sampler, tiled_uv, material.indices.w, tile_dx, tile_dy).r;
    }
    if (use_maps && material.indices.y >= 0) {
        var mapped = textureSampleGrad(map_normal, map_sampler, tiled_uv, material.indices.y, tile_dx, tile_dy).xyz * 2.0 - vec3f(1.0);
        if (material.seeds.w == 3u) {
            mapped.z = sqrt(max(0.0, 1.0 - dot(mapped.xy, mapped.xy)));
        }
        let t = normalize(tangent.xyz - result.normal * dot(tangent.xyz, result.normal));
        let b = cross(result.normal, t) * tangent.w;
        let transformed = normalize(t * mapped.x + b * mapped.y + result.normal * mapped.z);
        result.normal = normalize(mix(result.normal, transformed, clamp(material.factors.z, 0.0, 1.0)));
    }
    let age = clamp(age_in, 0.0, 1.0);
    let seed = material.seeds.x;
    if (use_ageing && age > 0.0) {
        result.base = fade_colour(result.base, age * material.age_a.x, object_position, seed);
        result.base = yellow(result.base, age * material.age_a.y, uv, seed);
        if (material.age_a.z > 0.0) {
            map_ink_base = vec4f(result.base, 1.0);
            result.base = mix(result.base, ink_age(object_position.xy, 1.0, seed).xyz, clamp(age * material.age_a.z, 0.0, 1.0));
        }
        if (material.seeds.y != 0u) {
            let scratched = map_scratch_coat(result.base, result.coat_roughness, age * material.age_a.w, object_position, seed);
            result.base = scratched.xyz;
            result.coat_roughness = scratched.w;
        } else {
            let scratched = scratch(result.base, result.roughness, age * material.age_a.w, object_position, seed);
            result.base = scratched.xyz;
            result.roughness = scratched.w;
        }
        let worn = worn_edge(result.base, result.roughness, age * material.age_b.x, edge);
        result.base = worn.xyz;
        result.roughness = worn.w;
        let dusty = dust(result.base, result.roughness, age * material.age_b.y, object_position, seed);
        result.base = dusty.xyz;
        result.roughness = dusty.w;
        let patinated = patina(result.base, result.roughness, result.metalness, age * material.age_b.z, object_position, seed);
        result.base = patinated.xyz;
        result.roughness = patinated.w;
        if (material.age_b.z > 0.0) {
            let p = object_position + vec3f(f32(seed) * 0.001);
            let green = smoothstep(0.62, 0.9, fbm3(p * 1500.0 + vec3f(9.0), 3, true)) * clamp(age * material.age_b.z, 0.0, 1.0) * 0.45;
            result.metalness = mix(result.metalness, 0.5, green);
        }
    }
    if (use_detail && material.seeds.w == 0u) {
        result = map_layer(result, material.layer0, material.params[0], material.params[1], material.params[2], material.params[3], uv, object_position, time, material.seeds.y != 0u, material.packed.z);
        result = map_layer(result, material.layer1, material.params[4], material.params[5], material.params[6], material.params[7], uv, object_position, time, material.seeds.y != 0u, material.packed.z);
        result = map_layer(result, material.layer2, material.params[8], material.params[9], material.params[10], material.params[11], uv, object_position, time, material.seeds.y != 0u, material.packed.z);
        result = map_layer(result, material.layer3, material.params[12], material.params[13], material.params[14], material.params[15], uv, object_position, time, material.seeds.y != 0u, material.packed.z);
    }
    return result;
}
"#;

pub fn shader_source() -> String {
    format!(
        "{}\n{}\n{}",
        pfx_materials::NOISE,
        pfx_materials::AGEING,
        MAPS_WGSL
    )
}

pub fn content_source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        pfx_materials::BRDF,
        pfx_materials::NOISE,
        pfx_materials::AGEING,
        MAPS_WGSL,
        content_wgsl()
    )
}

pub fn content_wgsl() -> String {
    format!("{}\n{}", pfx_materials::CONTENT, CONTENT_WGSL)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MapIndices {
    pub base: Option<u32>,
    pub normal: Option<u32>,
    pub roughness: Option<u32>,
    pub metal: Option<u32>,
}

impl MapIndices {
    pub fn from_material(material: &Material) -> Result<Self, String> {
        let layer = material.maps.layer;
        if layer < 0.0 {
            return Ok(Self::default());
        }
        if !layer.is_finite() || layer.fract() != 0.0 || layer >= i32::MAX as f32 {
            return Err("material map layer must be a nonnegative integer".into());
        }
        let index = Some(layer as u32);
        Ok(Self {
            base: index,
            normal: index,
            roughness: index,
            metal: if material.maps.tile == 0.0 {
                None
            } else {
                index
            },
        })
    }

    pub fn validate(self, images: &MapImages<'_>) -> Result<(), String> {
        for (name, index, count) in [
            ("base", self.base, images.base.len()),
            ("normal", self.normal, images.normal.len()),
            ("roughness", self.roughness, images.roughness.len()),
            ("metal", self.metal, images.metal.len()),
        ] {
            if let Some(index) = index
                && index as usize >= count
            {
                return Err(format!("{name} layer {index} is outside {count} layers"));
            }
        }
        Ok(())
    }

    fn packed(self) -> [i32; 4] {
        [self.base, self.normal, self.roughness, self.metal]
            .map(|index| index.map_or(-1, |index| index as i32))
    }
}

#[derive(Default)]
pub struct MapImages<'a> {
    pub base: &'a [Image],
    pub normal: &'a [Image],
    pub roughness: &'a [Image],
    pub metal: &'a [Image],
}

pub fn select_baked_detail(material: &mut Material, layer: u32) -> Result<(), String> {
    if layer >= i32::MAX as u32 {
        return Err("baked detail layer is out of range".into());
    }
    material.maps.layer = layer as f32;
    material.maps.tile = 0.0;
    material.maps.albedo = 1.0;
    material.maps.normal = 1.0;
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuMapMaterial {
    pub indices: [i32; 4],
    pub factors: [f32; 4],
    pub age_a: [f32; 4],
    pub age_b: [f32; 4],
    pub seeds: [u32; 4],
    pub layers: [[f32; 4]; LAYER_CAP],
    pub content: [f32; 4],
    pub content_extra: [f32; 4],
    pub packed: [f32; 4],
    pub atlas: [f32; 4],
    pub params: [[f32; 4]; PARAM_ROWS],
}

impl GpuMapMaterial {
    pub fn new(material: &Material, indices: MapIndices) -> Self {
        let baked = material.maps.tile == 0.0 && indices.base.is_some();
        let layer = |i: usize| {
            let layer = material.layers[i];
            let kind = if layer.active() || layer.amplitude == 0.0 {
                noise_id(layer.kind)
            } else {
                0
            };
            [
                kind as f32,
                layer.frequency,
                layer.amplitude,
                f32::from_bits(layer.seed),
            ]
        };
        let params: Vec<[f32; GPU_PARAMS]> = material.layers.iter().map(layer_params).collect();
        Self {
            indices: indices.packed(),
            factors: [
                material.maps.tile,
                material.maps.albedo,
                material.maps.normal,
                material.normal.strength,
            ],
            age_a: [
                material.ageing.fade,
                material.ageing.yellow,
                material.ageing.ink,
                material.ageing.scratch,
            ],
            age_b: [
                material.ageing.edge,
                material.ageing.dust,
                material.ageing.patina,
                0.0,
            ],
            seeds: [
                material.ageing.seed,
                u32::from(material.clearcoat > 0.0),
                match material.normal.source {
                    NormalSource::Flat => 0,
                    NormalSource::Bump => 1,
                    NormalSource::Map => 2,
                },
                u32::from(baked),
            ],
            layers: [layer(0), layer(1), layer(2), layer(3)],
            content: content_row(material),
            content_extra: [material.content_layer.strength, 1.0, 1.0, 0.0],
            packed: [0.0, 0.0, -1.0, 0.0],
            atlas: [1.0, 1.0, 0.0, 0.0],
            params: std::array::from_fn(|row| {
                let at = row % (GPU_PARAMS / 4) * 4;
                let p = &params[row / (GPU_PARAMS / 4)];
                [p[at], p[at + 1], p[at + 2], p[at + 3]]
            }),
        }
    }
}

pub const RING_FADE: f32 = 8.0 / 7.0;

fn layer_params(layer: &NoiseLayer) -> [f32; GPU_PARAMS] {
    let mut p = pfx_materials::gpu_params(layer);
    match layer.kind {
        NoiseKind::PlankWood => p[GPU_PARAMS - 2] = p[1] * RING_FADE,
        NoiseKind::CoatWobble => {
            p[GPU_PARAMS - 2] = p[0] / lacquer::FINE_CELLS as f32;
            p[GPU_PARAMS - 1] = 1.0;
        }
        _ => {}
    }
    p
}

fn coat_of(material: &Material) -> Option<CoatWobble> {
    material
        .layers
        .iter()
        .find(|layer| layer.kind == NoiseKind::CoatWobble && layer.active())
        .map(|layer| CoatWobble::from_params(&layer.params))
}

fn content_row(material: &Material) -> [f32; 4] {
    let layer = material.content_layer;
    if !layer.active() || layer.slot as usize >= CONTENT_SLOTS {
        return [-1.0, 0.0, -1.0, 0.0];
    }
    [
        layer.slot as f32,
        layer.blend.id() as f32,
        layer.ink_roughness,
        layer.emboss,
    ]
}

fn noise_id(kind: NoiseKind) -> u32 {
    match kind {
        NoiseKind::None => 0,
        NoiseKind::Fibre => 1,
        NoiseKind::Crinkle => 2,
        NoiseKind::PlankWood => 3,
        NoiseKind::WallMottle => 4,
        NoiseKind::Grime => 5,
        NoiseKind::Leaf => 7,
        NoiseKind::Bark => 8,
        NoiseKind::Flow => 14,
        NoiseKind::Value => 15,
        NoiseKind::Fbm => 16,
        NoiseKind::Scratch => 17,
        NoiseKind::CoatWobble => 18,
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Base,
    Normal,
    Scalar,
}

fn channel(value: u16, wide: bool) -> f32 {
    value as f32 / if wide { 65535.0 } else { 255.0 }
}

fn texels(image: &Image, kind: Kind) -> Vec<[f32; 4]> {
    let decode = |rgba: [f32; 4]| match kind {
        Kind::Base => {
            let mut rgba = rgba;
            if image.space == ColorSpace::Srgb {
                for value in &mut rgba[..3] {
                    *value = linear_channel(*value);
                }
            }
            let a = rgba[3];
            [rgba[0] * a, rgba[1] * a, rgba[2] * a, a]
        }
        Kind::Normal | Kind::Scalar => rgba,
    };
    match &image.pixels {
        Pixels::Eight(values) => values
            .chunks_exact(4)
            .map(|p| decode(std::array::from_fn(|c| channel(p[c] as u16, false))))
            .collect(),
        Pixels::Sixteen(values) => values
            .chunks_exact(4)
            .map(|p| decode(std::array::from_fn(|c| channel(p[c], true))))
            .collect(),
    }
}

fn quantize(level: &[[f32; 4]], kind: Kind) -> Vec<u8> {
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    level
        .iter()
        .flat_map(|&texel| {
            let rgba = match kind {
                Kind::Base => {
                    let a = texel[3];
                    let rgb: [f32; 3] = std::array::from_fn(|c| {
                        if a > 0.0 {
                            encode_channel((texel[c] / a).clamp(0.0, 1.0))
                        } else {
                            0.0
                        }
                    });
                    [rgb[0], rgb[1], rgb[2], a]
                }
                Kind::Normal | Kind::Scalar => texel,
            };
            rgba.map(byte)
        })
        .collect()
}

fn mip_chain(image: &Image, kind: Kind) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
    if image.width == 0 || image.height == 0 {
        return Err("map image is empty".into());
    }
    let len = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("map image is too large")?;
    let valid = match &image.pixels {
        Pixels::Eight(p) => p.len() == len,
        Pixels::Sixteen(p) => p.len() == len,
    };
    if !valid {
        return Err("map image has the wrong pixel count".into());
    }
    let mut level = texels(image, kind);
    let (mut width, mut height) = (image.width, image.height);
    let mut chain = vec![(width, height, quantize(&level, kind))];
    while width > 1 || height > 1 {
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        let mut next = Vec::with_capacity(next_width as usize * next_height as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut total = [0.0; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (2 * x + dx).min(width - 1);
                        let sy = (2 * y + dy).min(height - 1);
                        let texel = level[(sy * width + sx) as usize];
                        for c in 0..4 {
                            total[c] += texel[c] * 0.25;
                        }
                    }
                }
                if matches!(kind, Kind::Normal) {
                    let normal: [f32; 3] = std::array::from_fn(|c| total[c] * 2.0 - 1.0);
                    let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
                    for c in 0..3 {
                        total[c] = normal[c] / length * 0.5 + 0.5;
                    }
                }
                next.push(total);
            }
        }
        chain.push((next_width, next_height, quantize(&next, kind)));
        level = next;
        width = next_width;
        height = next_height;
    }
    Ok(chain)
}

impl Kind {
    fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Base => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Normal | Self::Scalar => wgpu::TextureFormat::Rgba8Unorm,
        }
    }
}

pub struct TextureArray {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub mips: u32,
}

impl TextureArray {
    pub fn pages(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pages: &[Texture],
        blocks: bool,
        label: &str,
    ) -> Result<Self, String> {
        let first = pages.first().ok_or_else(|| format!("{label} are empty"))?;
        let decoded;
        let pages = if first.format.compressed() && !blocks {
            decoded = pages.iter().map(Texture::decode).collect::<Vec<_>>();
            &decoded[..]
        } else {
            pages
        };
        let first = &pages[0];
        let (width, height, mips) = (first.width, first.height, first.levels.len() as u32);
        if pages.iter().any(|page| {
            page.format != first.format
                || page.width != width
                || page.height != height
                || page.levels.len() as u32 != mips
                || page.check().is_err()
        }) {
            return Err(format!("{label} must share one format and size"));
        }
        let limits = device.limits();
        if width > limits.max_texture_dimension_2d || height > limits.max_texture_dimension_2d {
            return Err(format!("{label} exceed device limits"));
        }
        if pages.len() > limits.max_texture_array_layers as usize {
            return Err(format!("too many {label}"));
        }
        let format = first.format;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: pages.len() as u32,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: detail_format(format),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let block = format.block();
        for (layer, page) in pages.iter().enumerate() {
            for (level, bytes) in page.levels.iter().enumerate() {
                let [w, h] = level_size(width, height, level as u32);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w.div_ceil(block) * format.block_bytes()),
                        rows_per_image: Some(h.div_ceil(block)),
                    },
                    wgpu::Extent3d {
                        width: w.div_ceil(block) * block,
                        height: h.div_ceil(block) * block,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Ok(Self {
            texture,
            view,
            width,
            height,
            layers: pages.len() as u32,
            mips,
        })
    }

    fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &[Image],
        kind: Kind,
        label: &str,
    ) -> Result<Self, String> {
        let fallback = match kind {
            Kind::Base | Kind::Scalar => [255, 255, 255, 255],
            Kind::Normal => [128, 128, 255, 255],
        };
        let placeholder = Image {
            width: 1,
            height: 1,
            space: ColorSpace::Linear,
            pixels: Pixels::Eight(fallback.to_vec()),
        };
        let entries = if images.is_empty() {
            std::slice::from_ref(&placeholder)
        } else {
            images
        };
        let width = entries[0].width;
        let height = entries[0].height;
        if width == 0 || height == 0 {
            return Err(format!("{label} map is empty"));
        }
        if entries.len() > device.limits().max_texture_array_layers as usize {
            return Err(format!("too many {label} layers"));
        }
        if width > device.limits().max_texture_dimension_2d
            || height > device.limits().max_texture_dimension_2d
        {
            return Err(format!("{label} map exceeds device limits"));
        }
        let chains = entries
            .iter()
            .map(|image| {
                if image.width != width || image.height != height {
                    return Err(format!("{label} maps must have the same dimensions"));
                }
                mip_chain(
                    image,
                    match kind {
                        Kind::Base => Kind::Base,
                        Kind::Normal => Kind::Normal,
                        Kind::Scalar => Kind::Scalar,
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: chains.len() as u32,
            },
            mip_level_count: chains[0].len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: kind.format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        for (layer, chain) in chains.iter().enumerate() {
            write_chain(queue, &texture, layer as u32, chain);
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        Ok(Self {
            texture,
            view,
            width,
            height,
            layers: images.len() as u32,
            mips: chains[0].len() as u32,
        })
    }

    fn write_layer(&self, queue: &wgpu::Queue, layer: u32, image: &Image, kind: Kind) {
        if layer >= self.layers || image.width != self.width || image.height != self.height {
            return;
        }
        if let Ok(chain) = mip_chain(image, kind) {
            write_chain(queue, &self.texture, layer, &chain);
        }
    }
}

fn write_chain(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    chain: &[(u32, u32, Vec<u8>)],
) {
    for (level, (w, h, bytes)) in chain.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(*h),
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
    }
}

pub fn metal_with_noise(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    metal: &[Image],
    coat: &CoatWobble,
) -> Result<(TextureArray, Option<u32>), String> {
    let fits = metal
        .iter()
        .all(|image| image.width == lacquer::SIZE && image.height == lacquer::SIZE);
    if !fits {
        let array = TextureArray::upload(device, queue, metal, Kind::Scalar, "metal maps")?;
        return Ok((array, None));
    }
    let mut layers = metal.to_vec();
    layers.push(lacquer::image(coat));
    layers.extend(noise_tiles::images());
    let array = TextureArray::upload(device, queue, &layers, Kind::Scalar, "metal maps")?;
    Ok((array, Some(metal.len() as u32)))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureFilter {
    pub content_bias: f32,
    pub detail_bias: f32,
    pub anisotropy: u16,
}

impl Default for TextureFilter {
    fn default() -> Self {
        Self {
            content_bias: CONTENT_LOD_BIAS,
            detail_bias: DETAIL_LOD_BIAS,
            anisotropy: ANISOTROPY,
        }
    }
}

impl TextureFilter {
    pub const UNBIASED: Self = Self {
        content_bias: 0.0,
        detail_bias: 0.0,
        anisotropy: 8,
    };

    pub fn check(&self) -> Result<(), String> {
        let bias = |b: f32| b.is_finite() && (-4.0..=4.0).contains(&b);
        if !bias(self.content_bias) || !bias(self.detail_bias) {
            return Err("texture LOD biases must be between -4 and 4".into());
        }
        if !(1..=16).contains(&self.anisotropy) {
            return Err("anisotropy must be between 1 and 16".into());
        }
        Ok(())
    }

    pub fn apply(&self, material: &mut GpuMapMaterial) {
        material.content_extra[1] = self.content_bias.exp2();
        material.content_extra[2] = self.detail_bias.exp2();
    }

    pub fn sampler(&self) -> wgpu::SamplerDescriptor<'static> {
        wgpu::SamplerDescriptor {
            anisotropy_clamp: self.anisotropy,
            ..sampler_descriptor()
        }
    }

    pub fn content_sampler(&self) -> wgpu::SamplerDescriptor<'static> {
        wgpu::SamplerDescriptor {
            anisotropy_clamp: self.anisotropy,
            ..content_sampler_descriptor()
        }
    }
}

pub struct MapTextures {
    pub base: TextureArray,
    pub normal: TextureArray,
    pub roughness: TextureArray,
    pub metal: TextureArray,
    pub sampler: wgpu::Sampler,
    pub filter: TextureFilter,
    pub detail: Vec<DetailPart>,
    pub two_channel_normals: bool,
    pub noise: Option<u32>,
    pub coat: CoatWobble,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailPart {
    pub page: u32,
    pub transform: [f32; 4],
    pub uv_set: u32,
}

pub fn detail_format(format: Format) -> wgpu::TextureFormat {
    match format {
        Format::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
        Format::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        Format::Bc4 => wgpu::TextureFormat::Bc4RUnorm,
        Format::Bc5 => wgpu::TextureFormat::Bc5RgUnorm,
        Format::Bc7Srgb => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
    }
}

pub fn uploads_blocks(device: &wgpu::Device) -> bool {
    device
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
}

pub fn sampler_descriptor() -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("material map sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: ANISOTROPY,
        ..Default::default()
    }
}

impl MapTextures {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        images: &MapImages<'_>,
    ) -> Result<Self, String> {
        let base = TextureArray::upload(device, queue, images.base, Kind::Base, "base maps")?;
        let normal =
            TextureArray::upload(device, queue, images.normal, Kind::Normal, "normal maps")?;
        let roughness = TextureArray::upload(
            device,
            queue,
            images.roughness,
            Kind::Scalar,
            "roughness maps",
        )?;
        let coat = CoatWobble::default();
        let (metal, noise) = metal_with_noise(device, queue, images.metal, &coat)?;
        let sampler = device.create_sampler(&sampler_descriptor());
        Ok(Self {
            base,
            normal,
            roughness,
            metal,
            sampler,
            filter: TextureFilter::default(),
            detail: Vec::new(),
            two_channel_normals: false,
            noise,
            coat,
        })
    }

    pub fn detail(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &Atlas,
    ) -> Result<Self, String> {
        let blocks = uploads_blocks(device);
        let base = TextureArray::pages(device, queue, &atlas.base, blocks, "base detail pages")?;
        let normal =
            TextureArray::pages(device, queue, &atlas.normal, blocks, "normal detail pages")?;
        let roughness = TextureArray::pages(
            device,
            queue,
            &atlas.roughness,
            blocks,
            "roughness detail pages",
        )?;
        let coat = CoatWobble::default();
        let (metal, noise) = metal_with_noise(device, queue, &[], &coat)?;
        let sampler = device.create_sampler(&sampler_descriptor());
        Ok(Self {
            base,
            normal,
            roughness,
            metal,
            sampler,
            filter: TextureFilter::default(),
            detail: atlas
                .parts
                .iter()
                .map(|part| DetailPart {
                    page: part.page,
                    transform: atlas.transform(part),
                    uv_set: part.uv_set,
                })
                .collect(),
            two_channel_normals: atlas
                .normal
                .first()
                .is_some_and(|page| page.format == Format::Bc5),
            noise,
            coat,
        })
    }

    pub fn fit_coat(&mut self, queue: &wgpu::Queue, materials: &[Material]) {
        let Some(layer) = self.noise else {
            return;
        };
        let Some(coat) = materials.iter().find_map(coat_of) else {
            return;
        };
        if coat == self.coat {
            return;
        }
        self.metal
            .write_layer(queue, layer, &lacquer::image(&coat), Kind::Scalar);
        self.coat = coat;
    }

    pub fn set_filter(&mut self, device: &wgpu::Device, filter: TextureFilter) {
        if filter.anisotropy != self.filter.anisotropy {
            self.sampler = device.create_sampler(&filter.sampler());
        }
        self.filter = filter;
    }

    pub fn material(&self, material: &Material, indices: MapIndices) -> GpuMapMaterial {
        let mut gpu = GpuMapMaterial::new(material, indices);
        self.filter.apply(&mut gpu);
        gpu.packed[2] = self.noise.map_or(-1.0, |layer| layer as f32);
        for (slot, layer) in material.layers.iter().enumerate() {
            if layer.kind == NoiseKind::CoatWobble
                && CoatWobble::from_params(&layer.params) != self.coat
            {
                gpu.params[slot * GPU_PARAMS / 4 + 3][3] = 0.0;
            }
        }
        if self.detail.is_empty() {
            return gpu;
        }
        let part = indices
            .base
            .filter(|_| gpu.seeds[3] == 1)
            .and_then(|part| self.detail.get(part as usize));
        match part {
            Some(part) => {
                let page = part.page as i32;
                gpu.indices = [page, page, page, -1];
                gpu.atlas = part.transform;
                gpu.packed[1] = part.uv_set as f32;
                gpu.seeds[3] = if self.two_channel_normals { 3 } else { 2 };
            }
            None => {
                gpu.indices = [-1; 4];
                gpu.seeds[3] = 0;
            }
        }
        gpu
    }

    pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material maps"),
            entries: &[
                texture(0),
                texture(1),
                texture(2),
                texture(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    pub fn bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        materials: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material maps"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.base.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.normal.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.roughness.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.metal.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: materials.as_entire_binding(),
                },
            ],
        })
    }
}

const MIP_WGSL: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
struct MipOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
}
@vertex fn mip_vertex(@builtin(vertex_index) index: u32) -> MipOut {
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    return MipOut(vec4f(uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), uv);
}
@fragment fn mip_fragment(input: MipOut) -> @location(0) vec4f {
    return textureSampleLevel(source, source_sampler, input.uv, 0.0);
}
"#;

const MIP_COMPUTE_WGSL: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var smaller: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(8, 8) fn mip_compute(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(smaller);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    textureStore(smaller, id.xy, textureSampleLevel(source, source_sampler, uv, 0.0));
}
@group(1) @binding(0) var<uniform> region: vec4u;
@compute @workgroup_size(8, 8) fn mip_compute_region(@builtin(global_invocation_id) id: vec3u) {
    if (id.x >= region.z || id.y >= region.w) {
        return;
    }
    let size = textureDimensions(smaller);
    let at = id.xy + region.xy;
    let uv = (vec2f(at) + vec2f(0.5)) / vec2f(size);
    textureStore(smaller, at, textureSampleLevel(source, source_sampler, uv, 0.0));
}
"#;

pub const REGION_STRIDE: u64 = 256;

pub fn mip_compute_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        },
    ]
}

pub fn mip_region_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(16),
        },
        count: None,
    }]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipRegion {
    pub level: u32,
    pub rect: [u32; 4],
}

fn level_extent(side: u32, level: u32) -> u32 {
    (side >> level).max(1)
}

fn footprint(start: u32, end: u32, from: u32, to: u32) -> [u32; 2] {
    let scale = f64::from(to) / f64::from(from);
    let low = ((f64::from(start) - 0.5) * scale - 0.5).ceil() - 1.0;
    let high = ((f64::from(end) + 0.5) * scale - 0.5).ceil() + 1.0;
    let low = (low.max(0.0) as u32).min(to - 1);
    let high = (high.max(0.0) as u32).min(to - 1);
    [low, high + 1]
}

pub fn mip_regions(size: [u32; 2], rect: [u32; 4]) -> Vec<MipRegion> {
    let levels = mip_count(size[0], size[1]);
    let mut regions = Vec::with_capacity(levels.saturating_sub(1) as usize);
    let mut x = [rect[0], rect[0] + rect[2]];
    let mut y = [rect[1], rect[1] + rect[3]];
    for level in 1..levels {
        let from = [
            level_extent(size[0], level - 1),
            level_extent(size[1], level - 1),
        ];
        let to = [level_extent(size[0], level), level_extent(size[1], level)];
        x = footprint(x[0], x[1], from[0], to[0]);
        y = footprint(y[0], y[1], from[1], to[1]);
        regions.push(MipRegion {
            level,
            rect: [x[0], y[0], x[1] - x[0], y[1] - y[0]],
        });
    }
    regions
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentFormat {
    Srgb8,
    Linear16,
}

impl ContentFormat {
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Srgb8 => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Linear16 => wgpu::TextureFormat::Rgba16Float,
        }
    }

    pub fn texel_bytes(self) -> u32 {
        match self {
            Self::Srgb8 => 4,
            Self::Linear16 => 8,
        }
    }
}

pub fn mip_count(width: u32, height: u32) -> u32 {
    u32::BITS - width.max(height).max(1).leading_zeros()
}

pub fn check_write(
    size: [u32; 2],
    format: ContentFormat,
    rect: [u32; 4],
    bytes: usize,
) -> Result<(), String> {
    let [x, y, width, height] = rect;
    if width == 0 || height == 0 {
        return Err("content write is empty".into());
    }
    if x.checked_add(width).is_none_or(|end| end > size[0])
        || y.checked_add(height).is_none_or(|end| end > size[1])
    {
        return Err(format!(
            "content write {rect:?} falls outside the {}x{} slot",
            size[0], size[1]
        ));
    }
    let expected = width as usize * height as usize * format.texel_bytes() as usize;
    if bytes != expected {
        return Err(format!(
            "content write of {width}x{height} needs {expected} bytes, got {bytes}"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CausticProjection {
    pub rect: [f32; 4],
    pub height: f32,
    pub toward_sun: [f32; 3],
}

impl CausticProjection {
    pub fn uv(&self, point: [f32; 3]) -> Option<[f32; 2]> {
        let length = self.toward_sun.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length.is_nan() || length <= 0.0 || self.toward_sun[1] / length <= 1e-4 {
            return None;
        }
        let toward = self.toward_sun.map(|v| v / length);
        let along = (self.height - point[1]) / toward[1];
        let q = [point[0] + toward[0] * along, point[2] + toward[2] * along];
        let at = [
            (q[0] - self.rect[0]) / self.rect[2],
            (q[1] - self.rect[1]) / self.rect[3],
        ];
        at.iter().all(|v| (0.0..=1.0).contains(v)).then_some(at)
    }

    pub fn rows(projection: Option<Self>, slot: usize, strength: f32) -> [[f32; 4]; 3] {
        let Some(projection) = projection else {
            return [[0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 0.0], [0.0; 4]];
        };
        let length = projection
            .toward_sun
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt();
        let toward = if length > 0.0 {
            projection.toward_sun.map(|v| v / length)
        } else {
            [0.0, 0.0, 0.0]
        };
        let on = toward[1] > 1e-4;
        [
            projection.rect,
            [toward[0], toward[1], toward[2], projection.height],
            [slot as f32, strength, f32::from(u8::from(on)), 0.0],
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Caustic {
    pub slot: usize,
    pub rect: [f32; 4],
    pub height: f32,
    pub strength: f32,
    pub receivers: Vec<usize>,
}

impl Caustic {
    pub fn check(&self) -> Result<(), String> {
        if self.slot >= CONTENT_SLOTS {
            return Err(format!(
                "caustic slot {} is outside the {CONTENT_SLOTS} slots",
                self.slot
            ));
        }
        if !self.rect.iter().all(|v| v.is_finite())
            || self.rect[2] <= 0.0
            || self.rect[3] <= 0.0
            || !self.height.is_finite()
            || !self.strength.is_finite()
        {
            return Err(
                "caustic rectangle, height and strength must be finite, with a positive size"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn projection(&self, toward_sun: [f32; 3]) -> CausticProjection {
        CausticProjection {
            rect: self.rect,
            height: self.height,
            toward_sun,
        }
    }

    pub fn receives(&self, instance: usize) -> bool {
        self.receivers.contains(&instance)
    }

    pub fn rows(caustic: Option<&Self>, toward_sun: [f32; 3]) -> [[f32; 4]; 3] {
        match caustic {
            Some(caustic) => CausticProjection::rows(
                Some(caustic.projection(toward_sun)),
                caustic.slot,
                caustic.strength,
            ),
            None => CausticProjection::rows(None, 0, 0.0),
        }
    }
}

pub struct ContentTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub format: ContentFormat,
    pub mips: u32,
    pub writes: u64,
    levels: Vec<wgpu::TextureView>,
    groups: Vec<wgpu::BindGroup>,
    compute_groups: Vec<wgpu::BindGroup>,
}

impl ContentTexture {
    pub fn level(&self, level: u32) -> Option<&wgpu::TextureView> {
        self.levels.get(level as usize)
    }
}

struct Mipper {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipelines: [wgpu::RenderPipeline; 2],
    compute_layout: wgpu::BindGroupLayout,
    compute: wgpu::ComputePipeline,
    region_layout: wgpu::BindGroupLayout,
    region_compute: wgpu::ComputePipeline,
}

impl Mipper {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("content mips"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("content mips"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("content mips"),
            source: wgpu::ShaderSource::Wgsl(MIP_WGSL.into()),
        });
        let pipelines = [ContentFormat::Srgb8, ContentFormat::Linear16].map(|format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("content mips"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("mip_vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("mip_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: format.texture_format(),
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            })
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("content mips"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("content compute mips"),
            entries: &mip_compute_entries(),
        });
        let compute_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("content compute mips"),
            source: wgpu::ShaderSource::Wgsl(MIP_COMPUTE_WGSL.into()),
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("content compute mips"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("content compute mips"),
                    bind_group_layouts: &[&compute_layout],
                    push_constant_ranges: &[],
                }),
            ),
            module: &compute_module,
            entry_point: Some("mip_compute"),
            compilation_options: Default::default(),
            cache: None,
        });
        let region_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("content compute mip region"),
            entries: &mip_region_entries(),
        });
        let region_compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("content compute mip region"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("content compute mip region"),
                    bind_group_layouts: &[&compute_layout, &region_layout],
                    push_constant_ranges: &[],
                }),
            ),
            module: &compute_module,
            entry_point: Some("mip_compute_region"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            layout,
            sampler,
            pipelines,
            compute_layout,
            compute,
            region_layout,
            region_compute,
        }
    }

    fn pipeline(&self, format: ContentFormat) -> &wgpu::RenderPipeline {
        &self.pipelines[usize::from(format == ContentFormat::Linear16)]
    }
}

pub struct ContentSlots {
    slots: Vec<Option<ContentTexture>>,
    blank: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub caustic: wgpu::Buffer,
    mipper: Mipper,
}

pub fn content_sampler_descriptor() -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("content sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: ANISOTROPY,
        ..Default::default()
    }
}

impl ContentSlots {
    pub fn set_filter(&mut self, device: &wgpu::Device, filter: TextureFilter) {
        self.sampler = device.create_sampler(&filter.content_sampler());
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let blank = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank content"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            blank.as_image_copy(),
            &[0; 8],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Self {
            slots: (0..CONTENT_SLOTS).map(|_| None).collect(),
            blank: blank.create_view(&Default::default()),
            sampler: device.create_sampler(&content_sampler_descriptor()),
            caustic: wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("content caustic"),
                    contents: bytemuck::cast_slice(&CausticProjection::rows(None, 0, 0.0)),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                },
            ),
            mipper: Mipper::new(device),
        }
    }

    pub fn write_caustic(&self, queue: &wgpu::Queue, rows: [[f32; 4]; 3]) {
        queue.write_buffer(&self.caustic, 0, bytemuck::cast_slice(&rows));
    }

    pub fn get(&self, slot: usize) -> Option<&ContentTexture> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    pub fn view(&self, slot: usize) -> &wgpu::TextureView {
        self.get(slot).map_or(&self.blank, |content| &content.view)
    }

    pub fn allocate(
        &mut self,
        device: &wgpu::Device,
        slot: usize,
        width: u32,
        height: u32,
        format: ContentFormat,
    ) -> Result<(), String> {
        if slot >= CONTENT_SLOTS {
            return Err(format!(
                "content slot {slot} is outside the {CONTENT_SLOTS} slots"
            ));
        }
        let limit = device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit {
            return Err(format!("content slot {slot} cannot hold {width}x{height}"));
        }
        if self.get(slot).is_some_and(|content| {
            (content.width, content.height, content.format) == (width, height, format)
        }) {
            return Ok(());
        }
        let mips = mip_count(width, height);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("content"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: format.texture_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | if format == ContentFormat::Linear16 {
                    wgpu::TextureUsages::STORAGE_BINDING
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        });
        let levels: Vec<wgpu::TextureView> = (0..mips)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("content level"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let render_levels = if format == ContentFormat::Linear16 {
            0
        } else {
            levels.len() - 1
        };
        let groups = levels[..render_levels]
            .iter()
            .map(|source| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("content mip source"),
                    layout: &self.mipper.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.mipper.sampler),
                        },
                    ],
                })
            })
            .collect();
        let compute_groups = if format == ContentFormat::Linear16 {
            levels
                .windows(2)
                .map(|pair| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("content compute mip"),
                        layout: &self.mipper.compute_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&pair[0]),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.mipper.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&pair[1]),
                            },
                        ],
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        self.slots[slot] = Some(ContentTexture {
            view: texture.create_view(&Default::default()),
            texture,
            width,
            height,
            format,
            mips,
            writes: 0,
            levels,
            groups,
            compute_groups,
        });
        Ok(())
    }

    pub fn release(&mut self, slot: usize) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = None;
        }
    }

    pub fn write(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: usize,
        rect: [u32; 4],
        bytes: &[u8],
    ) -> Result<(), String> {
        let content = self
            .slots
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("content slot {slot} is not allocated"))?;
        check_write(
            [content.width, content.height],
            content.format,
            rect,
            bytes.len(),
        )?;
        let [x, y, width, height] = rect;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &content.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * content.format.texel_bytes()),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("content mips"),
        });
        encode_mips(&self.mipper, &mut encoder, content);
        queue.submit(Some(encoder.finish()));
        content.writes += 1;
        Ok(())
    }

    pub fn copy(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        source: &wgpu::Texture,
    ) -> Result<ContentCopy, String> {
        let content = self
            .slots
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("content slot {slot} is not allocated"))?;
        let kind = copy_kind(
            source,
            content.format.texture_format(),
            [content.width, content.height],
        )?;
        match kind {
            ContentCopy::Copied => encoder.copy_texture_to_texture(
                source.as_image_copy(),
                content.texture.as_image_copy(),
                wgpu::Extent3d {
                    width: content.width,
                    height: content.height,
                    depth_or_array_layers: 1,
                },
            ),
            ContentCopy::Drawn => {
                let view = source.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("content source"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: 0,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("content source"),
                    layout: &self.mipper.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.mipper.sampler),
                        },
                    ],
                });
                mip_pass(&self.mipper, encoder, content, &content.levels[0], &group);
            }
        }
        encode_mips(&self.mipper, encoder, content);
        content.writes += 1;
        Ok(kind)
    }
}

impl ContentSlots {
    pub fn regenerate_regions(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        regions: &[MipRegion],
    ) -> Result<(), String> {
        let content = self
            .slots
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("content slot {slot} is not allocated"))?;
        for region in regions {
            let [x, y, width, height] = region.rect;
            let wide = level_extent(content.width, region.level);
            let high = level_extent(content.height, region.level);
            if region.level == 0
                || region.level >= content.mips
                || width == 0
                || height == 0
                || x + width > wide
                || y + height > high
            {
                return Err(format!(
                    "mip region {:?} at level {} falls outside the {wide}x{high} level",
                    region.rect, region.level
                ));
            }
        }
        if regions.is_empty() {
            return Ok(());
        }
        if content.compute_groups.is_empty() {
            for region in regions {
                let level = region.level as usize;
                let [x, y, width, height] = region.rect;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("content mip region"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &content.levels[level],
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(self.mipper.pipeline(content.format));
                pass.set_bind_group(0, &content.groups[level - 1], &[]);
                pass.set_scissor_rect(x, y, width, height);
                pass.draw(0..3, 0..1);
            }
        } else {
            let mut origins = vec![0u8; regions.len() * REGION_STRIDE as usize];
            for (index, region) in regions.iter().enumerate() {
                origins[index * REGION_STRIDE as usize..][..16]
                    .copy_from_slice(bytemuck::cast_slice(&region.rect));
            }
            let buffer = wgpu::util::DeviceExt::create_buffer_init(
                device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("content mip regions"),
                    contents: &origins,
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("content mip regions"),
                layout: &self.mipper.region_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                }],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("content mip regions"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.mipper.region_compute);
            for (index, region) in regions.iter().enumerate() {
                pass.set_bind_group(0, &content.compute_groups[region.level as usize - 1], &[]);
                pass.set_bind_group(1, &group, &[(index as u64 * REGION_STRIDE) as u32]);
                pass.dispatch_workgroups(region.rect[2].div_ceil(8), region.rect[3].div_ceil(8), 1);
            }
        }
        content.writes += 1;
        Ok(())
    }

    pub fn regenerate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
    ) -> Result<(), String> {
        let content = self
            .slots
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("content slot {slot} is not allocated"))?;
        encode_mips(&self.mipper, encoder, content);
        content.writes += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentCopy {
    Copied,
    Drawn,
}

pub fn copy_kind(
    source: &wgpu::Texture,
    format: wgpu::TextureFormat,
    size: [u32; 2],
) -> Result<ContentCopy, String> {
    if source.dimension() != wgpu::TextureDimension::D2 || source.sample_count() != 1 {
        return Err("a content source must be a single-sampled 2D texture".into());
    }
    let same_format = source.format().remove_srgb_suffix() == format.remove_srgb_suffix();
    let same_size = source.width() == size[0] && source.height() == size[1];
    if same_format && same_size && source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        return Ok(ContentCopy::Copied);
    }
    let filterable = matches!(
        source.format().sample_type(None, None),
        Some(wgpu::TextureSampleType::Float { filterable: true })
    );
    if filterable
        && source
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Ok(ContentCopy::Drawn);
    }
    Err(format!(
        "a {:?} {}x{} source cannot fill a {format:?} {}x{} slot: copy needs the same format and size with COPY_SRC, drawing needs a filterable TEXTURE_BINDING",
        source.format(),
        source.width(),
        source.height(),
        size[0],
        size[1]
    ))
}

fn mip_pass(
    mipper: &Mipper,
    encoder: &mut wgpu::CommandEncoder,
    content: &ContentTexture,
    target: &wgpu::TextureView,
    group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("content mip"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_pipeline(mipper.pipeline(content.format));
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

fn encode_mips(mipper: &Mipper, encoder: &mut wgpu::CommandEncoder, content: &ContentTexture) {
    if content.compute_groups.is_empty() {
        for (level, group) in content.groups.iter().enumerate() {
            mip_pass(mipper, encoder, content, &content.levels[level + 1], group);
        }
        return;
    }
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("content mips"),
        timestamp_writes: None,
    });
    pass.set_pipeline(&mipper.compute);
    for (level, group) in content.compute_groups.iter().enumerate() {
        let width = (content.width >> (level + 1)).max(1);
        let height = (content.height >> (level + 1)).max(1);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_gpu::wgpu::util::DeviceExt;
    use pfx_materials::{At, CoatWobble, Grime, PlankWood, Scratch};

    const LOOK: pfx_materials::ContentLook = pfx_materials::ContentLook {
        ink_roughness: 0.5,
        ink_emboss_coated: 0.2,
        ink_emboss_bare: -0.1,
        photo_gain: 1.1,
        screen_gain: 1.5,
        photo_window: [0.0, 0.0, 1.0, 1.0],
    };

    const PLANE_WGSL: &str = r#"
@group(0) @binding(0) var<uniform> plane_age: vec4f;
struct PlaneOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
}
@vertex fn plane_vertex(@builtin(vertex_index) index: u32) -> PlaneOut {
    let corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0)
    );
    let uv = corners[index];
    return PlaneOut(vec4f(uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), uv);
}
@fragment fn plane_fragment(input: PlaneOut) -> @location(0) vec4f {
    let result = map_evaluate(vec3f(1.0), 0.8, 0.0, 0.05, 0.0, vec3f(0.0, 0.0, 1.0), vec4f(1.0, 0.0, 0.0, 1.0), input.uv * 0.02, vec3f(input.uv, 0.0), 0.0, plane_age.x, 0.0, 0u);
    return vec4f(result.base, 1.0);
}
"#;

    fn image(width: u32, height: u32, colour: [u8; 4]) -> Image {
        Image {
            width,
            height,
            space: ColorSpace::Srgb,
            pixels: Pixels::Eight(colour.repeat((width * height) as usize)),
        }
    }

    #[test]
    fn layer_indices_and_packing() {
        let base = [
            image(2, 2, [255, 255, 255, 255]),
            image(2, 2, [0, 0, 0, 255]),
        ];
        let normal = [image(2, 2, [128, 128, 255, 255])];
        let images = MapImages {
            base: &base,
            normal: &normal,
            ..Default::default()
        };
        let indices = MapIndices {
            base: Some(1),
            normal: Some(0),
            ..Default::default()
        };
        assert!(indices.validate(&images).is_ok());
        assert_eq!(indices.packed(), [1, 0, -1, -1]);
        assert!(
            MapIndices {
                base: Some(2),
                ..Default::default()
            }
            .validate(&images)
            .is_err()
        );
        let mut material = Material::default();
        material.layers[0] = PlankWood::SAMPLE.layer(0.6, u32::MAX);
        material.maps.layer = 1.0;
        assert_eq!(
            MapIndices::from_material(&material).unwrap().packed(),
            [1, 1, 1, 1]
        );
        let packed = GpuMapMaterial::new(&material, indices);
        assert_eq!(packed.indices, [1, 0, -1, -1]);
        assert_eq!(packed.layers[0][0], 3.0);
        assert_eq!(packed.layers[0][3].to_bits(), u32::MAX);
        assert_eq!(std::mem::size_of::<GpuMapMaterial>(), 464);
        assert_eq!(packed.params[0][0], PlankWood::SAMPLE.width);
        assert_eq!(packed.params[3][0], 1.0 / PlankWood::SAMPLE.width);
        assert_eq!(packed.params[3][1], 1.0 - PlankWood::SAMPLE.seam_floor);
        assert_eq!(packed.params[3][2], PlankWood::SAMPLE.rings * RING_FADE);
        let mut flat = material;
        flat.layers[0].params[0] = 0.0;
        assert_eq!(GpuMapMaterial::new(&flat, indices).layers[0][0], 0.0);
    }

    #[test]
    fn colour_mips_average_light_not_encoded_values() {
        let checker = Image {
            width: 2,
            height: 2,
            space: ColorSpace::Srgb,
            pixels: Pixels::Eight(
                [[0, 0, 0, 255], [255, 255, 255, 255]]
                    .repeat(2)
                    .concat()
                    .to_vec(),
            ),
        };
        let chain = mip_chain(&checker, Kind::Base).unwrap();
        assert_eq!(chain[0].2[..8], [0, 0, 0, 255, 255, 255, 255, 255]);
        let half = (encode_channel(0.5) * 255.0).round() as u8;
        assert_eq!(half, 188);
        assert_eq!(chain[1].2, [half, half, half, 255]);
        let scalar = mip_chain(&checker, Kind::Scalar).unwrap();
        assert_eq!(scalar[1].2, [128, 128, 128, 255]);
        let cutout = Image {
            width: 2,
            height: 1,
            space: ColorSpace::Srgb,
            pixels: Pixels::Eight(vec![255, 0, 0, 255, 0, 255, 0, 0]),
        };
        let chain = mip_chain(&cutout, Kind::Base).unwrap();
        assert_eq!(chain[1].2, [255, 0, 0, 128]);
        let deep = Image {
            width: 2,
            height: 1,
            space: ColorSpace::Srgb,
            pixels: Pixels::Sixteen(vec![771, 771, 771, 65535, 771, 771, 771, 65535]),
        };
        let dark = mip_chain(&deep, Kind::Base).unwrap();
        assert_eq!(dark[0].2[0], 3);
        for value in 0..=255u8 {
            let linear = linear_channel(value as f32 / 255.0);
            assert_eq!((encode_channel(linear) * 255.0).round() as u8, value);
        }
        let tilted = Image {
            width: 2,
            height: 1,
            space: ColorSpace::Linear,
            pixels: Pixels::Eight(vec![204, 128, 230, 255, 51, 128, 230, 255]),
        };
        let normal = mip_chain(&tilted, Kind::Normal).unwrap();
        assert_eq!(normal[0].2[..4], [204, 128, 230, 255]);
        assert_eq!(normal[1].2[2], 255, "{:?}", normal[1].2);
    }

    #[test]
    fn baked_detail_selects_maps_and_bypasses_layers() {
        let mut material = Material::default();
        material.layers[0] = PlankWood::SAMPLE.layer(1.0, 37);
        select_baked_detail(&mut material, 3).unwrap();
        let packed = GpuMapMaterial::new(&material, MapIndices::from_material(&material).unwrap());
        assert_eq!(packed.indices, [3, 3, 3, -1]);
        assert_eq!(packed.seeds[3], 1);
        assert_eq!(packed.layers[0][0], 3.0);
        assert!(select_baked_detail(&mut material, i32::MAX as u32).is_err());
    }

    #[test]
    fn mip_chain_and_sampling_setup() {
        let image = image(4, 4, [128, 128, 128, 255]);
        let chain = mip_chain(&image, Kind::Base).unwrap();
        assert_eq!(
            chain.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(),
            [(4, 4), (2, 2), (1, 1)]
        );
        assert_eq!(chain[0].2[..4], [128, 128, 128, 255]);
        assert_eq!(chain[2].2, [128, 128, 128, 255]);
        assert_eq!(chain[2].2.len(), 4);
        assert_eq!(Kind::Base.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(Kind::Scalar.format(), wgpu::TextureFormat::Rgba8Unorm);
        let sampler = sampler_descriptor();
        assert_eq!(sampler.anisotropy_clamp, 16);
        assert_eq!(sampler.min_filter, wgpu::FilterMode::Linear);
        assert_eq!(sampler.mipmap_filter, wgpu::FilterMode::Linear);
    }

    #[test]
    fn shader_parses_and_validates() {
        let source = format!("{}\n{}", shader_source(), PLANE_WGSL);
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn age_zero_keeps_material_unaged() {
        let mut material = Material::default();
        material.ageing.yellow = 1.0;
        material.ageing.fade = 0.5;
        material.ageing.scratch = 0.2;
        material.ageing.seed = 23;
        let at = At {
            position: [0.3, 0.4, 0.0],
            uv: [0.3, 0.4],
            ..Default::default()
        };
        assert_eq!(pfx_materials::apply(&material, 0.0, &at), material);
        let packed = GpuMapMaterial::new(&material, MapIndices::default());
        assert_eq!(packed.age_a, [0.5, 1.0, 0.0, 0.2]);
    }

    fn render_plane(
        gpu: &pfx_gpu::Gpu,
        pipeline: &wgpu::RenderPipeline,
        age_group: &wgpu::BindGroup,
        empty_group: &wgpu::BindGroup,
        maps_group: &wgpu::BindGroup,
    ) -> [f32; 3] {
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sheet plane"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sheet plane readback"),
            size: 64 * 64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sheet plane"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sheet plane"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, age_group, &[]);
            pass.set_bind_group(1, empty_group, &[]);
            pass.set_bind_group(2, empty_group, &[]);
            pass.set_bind_group(3, maps_group, &[]);
            pass.draw(0..6, 0..1);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            target.size(),
        );
        gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let mut mean = [0.0; 3];
        for pixel in mapped.chunks_exact(4) {
            for c in 0..3 {
                mean[c] += pixel[c] as f32 / 255.0 / 4096.0;
            }
        }
        drop(mapped);
        readback.unmap();
        mean
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_sheet_yellows_with_age() {
        let started = std::time::Instant::now();
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let sheet = image(4, 4, [230, 220, 190, 255]);
        let images = MapImages {
            base: std::slice::from_ref(&sheet),
            ..Default::default()
        };
        let textures = MapTextures::new(&gpu.device, &gpu.queue, &images).unwrap();
        assert_eq!(textures.base.mips, 3);
        let mut material = Material::default();
        material.ageing.yellow = 1.0;
        material.ageing.seed = 42;
        let indices = MapIndices {
            base: Some(0),
            ..Default::default()
        };
        let maps = [GpuMapMaterial::new(&material, indices)];
        let map_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sheet map material"),
                contents: bytemuck::cast_slice(&maps),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
        let map_layout = MapTextures::layout(&gpu.device);
        let maps_group = textures.bind_group(&gpu.device, &map_layout, &map_buffer);
        let age_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sheet age"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let empty_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("empty"),
                entries: &[],
            });
        let empty_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("empty"),
            layout: &empty_layout,
            entries: &[],
        });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sheet pipeline"),
                bind_group_layouts: &[&age_layout, &empty_layout, &empty_layout, &map_layout],
                push_constant_ranges: &[],
            });
        let source = format!("{}\n{}", shader_source(), PLANE_WGSL);
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("sheet maps shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sheet maps pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("plane_vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("plane_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });
        let draw = |age: f32| {
            let age_buffer = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("sheet age input"),
                    contents: bytemuck::cast_slice(&[age, 0.0, 0.0, 0.0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let age_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sheet age input"),
                layout: &age_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: age_buffer.as_entire_binding(),
                }],
            });
            render_plane(&gpu, &pipeline, &age_group, &empty_group, &maps_group)
        };
        let young = draw(0.0);
        let aged = draw(1.0);
        let mut unaged_material = material;
        unaged_material.ageing = Default::default();
        let unaged = GpuMapMaterial::new(&unaged_material, indices);
        gpu.queue
            .write_buffer(&map_buffer, 0, bytemuck::bytes_of(&unaged));
        let baseline = draw(1.0);
        for channel in 0..3 {
            assert!((young[channel] - baseline[channel]).abs() < 1.0 / 255.0);
        }
        assert!(
            aged[0] - aged[2] > young[0] - young[2] + 0.01,
            "young {young:?}, aged {aged:?}"
        );
        println!(
            "sheet plane age 0/1: {:?}; young {young:?}; aged {aged:?}",
            started.elapsed()
        );
    }
    const CONTENT_TEST_WGSL: &str = r#"
struct ContentProbe {
    material: vec4u,
    transform: vec4f,
    crop: vec4f,
}
@group(0) @binding(0) var<uniform> probe: ContentProbe;
struct ProbeOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
}
struct ProbeTargets {
    @location(0) base: vec4f,
    @location(1) emission: vec4f,
}
@vertex fn probe_vertex(@builtin(vertex_index) index: u32) -> ProbeOut {
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    return ProbeOut(vec4f(uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), uv);
}
@fragment fn probe_fragment(input: ProbeOut) -> ProbeTargets {
    let dx = dpdx(input.uv);
    let dy = dpdy(input.uv);
    var surface: Shaded;
    surface.base = vec3f(0.8, 0.6, 0.4);
    surface.roughness = 0.9;
    surface.subsurface = 0.5;
    surface.emission = vec3f(0.2, 0.1, 0.0);
    surface.specular = 0.04;
    surface.normal = vec3f(0.0, 0.0, 1.0);
    let out = content_apply(surface, probe.material.x, input.uv, dx, dy, probe.transform, probe.crop, vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0));
    if (probe.material.y == 1u) {
        return ProbeTargets(vec4f(out.normal, 0.0), vec4f(0.0));
    }
    if (probe.material.y == 2u) {
        return ProbeTargets(vec4f(out.base, out.roughness), vec4f(out.metalness, out.specular, out.emission.r, out.subsurface));
    }
    return ProbeTargets(vec4f(out.base, out.roughness), vec4f(out.emission, out.subsurface));
}
"#;

    fn ramp_texels(size: u32) -> (Vec<[f32; 4]>, Vec<u8>) {
        let mut texels = Vec::new();
        let mut bytes = Vec::new();
        for _y in 0..size {
            for x in 0..size {
                let a = half::f16::from_f32(x as f32 / (size - 1) as f32).to_f32();
                let rgba = [0.1 * a, 0.2 * a, 0.3 * a, a].map(half::f16::from_f32);
                texels.push(rgba.map(|v| v.to_f32()));
                for value in rgba {
                    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
        }
        (texels, bytes)
    }

    fn read_targets(
        gpu: &pfx_gpu::Gpu,
        targets: &[wgpu::Texture],
        size: u32,
    ) -> Vec<Vec<[f32; 4]>> {
        let row = (size * 16).div_ceil(256) * 256;
        targets
            .iter()
            .map(|texture| {
                let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("content probe readback"),
                    size: u64::from(row * size),
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
                            rows_per_image: Some(size),
                        },
                    },
                    texture.size(),
                );
                gpu.queue.submit(Some(encoder.finish()));
                buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let mapped = buffer.slice(..).get_mapped_range();
                let mut values = Vec::new();
                for y in 0..size {
                    let start = (y * row) as usize;
                    let floats: &[f32] =
                        bytemuck::cast_slice(&mapped[start..start + (size * 16) as usize]);
                    values.extend(floats.chunks_exact(4).map(|v| [v[0], v[1], v[2], v[3]]));
                }
                drop(mapped);
                buffer.unmap();
                values
            })
            .collect()
    }

    type Probe<'a> = Box<dyn Fn([u32; 2], [f32; 4], [f32; 4]) -> Vec<Vec<[f32; 4]>> + 'a>;

    fn content_probe<'a>(
        gpu: &'a pfx_gpu::Gpu,
        slots: &ContentSlots,
        materials: &[GpuMapMaterial],
        size: u32,
    ) -> Probe<'a> {
        let map_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("content probe materials"),
                contents: bytemuck::cast_slice(materials),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let content_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("content probe"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 5,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        texture_entry(CONTENT_BINDING),
                        texture_entry(CONTENT_BINDING + 1),
                        texture_entry(CONTENT_BINDING + 2),
                        texture_entry(CONTENT_BINDING + 3),
                        wgpu::BindGroupLayoutEntry {
                            binding: CONTENT_BINDING + 4,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let content_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("content probe"),
            layout: &content_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: map_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: CONTENT_BINDING,
                    resource: wgpu::BindingResource::TextureView(slots.view(0)),
                },
                wgpu::BindGroupEntry {
                    binding: CONTENT_BINDING + 1,
                    resource: wgpu::BindingResource::TextureView(slots.view(1)),
                },
                wgpu::BindGroupEntry {
                    binding: CONTENT_BINDING + 2,
                    resource: wgpu::BindingResource::TextureView(slots.view(2)),
                },
                wgpu::BindGroupEntry {
                    binding: CONTENT_BINDING + 3,
                    resource: wgpu::BindingResource::TextureView(slots.view(3)),
                },
                wgpu::BindGroupEntry {
                    binding: CONTENT_BINDING + 4,
                    resource: wgpu::BindingResource::Sampler(&slots.sampler),
                },
            ],
        });
        let probe_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("content probe uniform"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let empty_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("empty"),
                entries: &[],
            });
        let empty_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("empty"),
            layout: &empty_layout,
            entries: &[],
        });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("content probe"),
                bind_group_layouts: &[&probe_layout, &empty_layout, &empty_layout, &content_layout],
                push_constant_ranges: &[],
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("content probe"),
                source: wgpu::ShaderSource::Wgsl(
                    format!("{}\n{}", content_source(), CONTENT_TEST_WGSL).into(),
                ),
            });
        let target_state = Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba32Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("content probe"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("probe_vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("probe_fragment"),
                    targets: &[target_state.clone(), target_state],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });
        Box::new(
            move |material: [u32; 2], transform: [f32; 4], crop: [f32; 4]| {
                let targets: Vec<wgpu::Texture> = (0..2)
                    .map(|_| {
                        gpu.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("content probe target"),
                            size: wgpu::Extent3d {
                                width: size,
                                height: size,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba32Float,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::COPY_SRC,
                            view_formats: &[],
                        })
                    })
                    .collect();
                let views: Vec<wgpu::TextureView> = targets
                    .iter()
                    .map(|texture| texture.create_view(&Default::default()))
                    .collect();
                let mut uniform = [0u8; 48];
                uniform[..8].copy_from_slice(bytemuck::cast_slice(&material));
                uniform[16..32].copy_from_slice(bytemuck::cast_slice(&transform));
                uniform[32..48].copy_from_slice(bytemuck::cast_slice(&crop));
                let probe_buffer =
                    gpu.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("content probe uniform"),
                            contents: &uniform,
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let probe_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("content probe uniform"),
                    layout: &probe_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: probe_buffer.as_entire_binding(),
                    }],
                });
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                {
                    let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = views
                        .iter()
                        .map(|view| {
                            Some(wgpu::RenderPassColorAttachment {
                                view,
                                resolve_target: None,
                                depth_slice: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                    store: wgpu::StoreOp::Store,
                                },
                            })
                        })
                        .collect();
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("content probe"),
                        color_attachments: &attachments,
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &probe_group, &[]);
                    pass.set_bind_group(1, &empty_group, &[]);
                    pass.set_bind_group(2, &empty_group, &[]);
                    pass.set_bind_group(3, &content_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                gpu.queue.submit(Some(encoder.finish()));
                read_targets(gpu, &targets, size)
            },
        )
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn content_layers_match_the_cpu_reference() {
        use pfx_materials::{Blend, Content, ContentLayer, Shaded};
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let size = 8;
        let mut slots = ContentSlots::new(&gpu.device, &gpu.queue);
        let (ink, ink_bytes) = ramp_texels(size);
        slots
            .allocate(&gpu.device, 0, size, size, ContentFormat::Linear16)
            .unwrap();
        slots
            .write(&gpu.device, &gpu.queue, 0, [0, 0, size, size], &ink_bytes)
            .unwrap();
        let frame_srgb = [200u8, 100, 50, 255];
        slots
            .allocate(&gpu.device, 1, size, size, ContentFormat::Srgb8)
            .unwrap();
        slots
            .write(
                &gpu.device,
                &gpu.queue,
                1,
                [0, 0, size, size],
                &frame_srgb.repeat((size * size) as usize),
            )
            .unwrap();
        let decode = |v: u8| linear_channel(v as f32 / 255.0);
        let video = [
            decode(frame_srgb[0]),
            decode(frame_srgb[1]),
            decode(frame_srgb[2]),
            1.0,
        ];
        let layers = [
            ContentLayer::from_kind(Content::Ink, 1.0, 0, &LOOK),
            ContentLayer::from_kind(Content::Photo, 1.0, 1, &LOOK),
            ContentLayer::from_kind(Content::Screen, 1.0, 1, &LOOK),
            ContentLayer::from_kind(Content::Decal, 1.0, 0, &LOOK),
        ];
        assert_eq!(layers[2].blend, Blend::Emit);
        let materials: Vec<GpuMapMaterial> = layers
            .iter()
            .map(|layer| {
                let material = Material {
                    content_layer: *layer,
                    ..Material::default()
                };
                GpuMapMaterial::new(&material, MapIndices::default())
            })
            .collect();
        let run = content_probe(&gpu, &slots, &materials, size);
        let base = Shaded::from_material(&Material {
            base: [0.8, 0.6, 0.4],
            roughness: 0.9,
            subsurface: 0.5,
            emission: [0.2, 0.1, 0.0],
            ..Material::default()
        });
        let identity = [0.0, 0.0, 1.0, 1.0];
        let whole = [0.0, 0.0, 1.0, 1.0];
        let close = |got: f32, want: f32, what: &str| {
            assert!((got - want).abs() < 2e-3, "{what}: gpu {got}, cpu {want}");
        };
        let check = |out: &[Vec<[f32; 4]>], index: usize, expected: &Shaded, what: &str| {
            for (c, (base, emission)) in expected.base.iter().zip(expected.emission).enumerate() {
                close(out[0][index][c], *base, what);
                close(out[1][index][c], emission, what);
            }
            close(out[0][index][3], expected.roughness, what);
            close(out[1][index][3], expected.subsurface, what);
        };
        let over = run([0, 0], identity, whole);
        for y in 0..size as usize {
            for x in 0..size as usize {
                let index = y * size as usize + x;
                let expected = layers[0].composite(&base, ink[index]);
                check(&over, index, &expected, "ink over");
            }
        }
        let normals = run([0, 1], identity, whole);
        let row = 3 * size as usize;
        for x in 1..size as usize - 1 {
            let n = normals[0][row + x];
            assert!(
                n[0] < -0.01,
                "ink rising along +u tilts the normal to -u: {n:?}"
            );
            assert!(n[1].abs() < 1e-3 && n[2] > 0.9, "{n:?}");
        }
        let photo = run([1, 0], identity, whole);
        let expected = layers[1].composite(&base, video);
        check(&photo, 9, &expected, "photo multiply");
        let screen = run([2, 0], identity, whole);
        let expected = layers[2].composite(&base, video);
        check(&screen, 9, &expected, "screen emission");
        assert!(screen[1][9][0] > base.emission[0] + 0.5);
        let shifted = run([3, 0], [0.5, 0.0, 1.0, 1.0], whole);
        for y in 0..size as usize {
            for x in 0..size as usize {
                let index = y * size as usize + x;
                let expected = if x + 4 < size as usize {
                    layers[3].composite(&base, ink[y * size as usize + x + 4])
                } else {
                    base
                };
                check(&shifted, index, &expected, "offset and crop");
            }
        }
        let cropped = run([3, 0], identity, [0.0, 0.0, 0.5, 1.0]);
        let inside = layers[3].composite(&base, ink[2]);
        check(&cropped, 2, &inside, "inside the crop");
        check(&cropped, 6, &base, "outside the crop");
    }

    fn read_level(gpu: &pfx_gpu::Gpu, content: &ContentTexture, level: u32) -> Vec<u8> {
        let width = (content.width >> level).max(1);
        let height = (content.height >> level).max(1);
        let texel = content.format.texel_bytes();
        let row = (width * texel).div_ceil(256) * 256;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("content level"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &content.texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
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
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        mapped
            .chunks_exact(row as usize)
            .flat_map(|line| line[..(width * texel) as usize].to_vec())
            .collect()
    }

    fn halves(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(2)
            .map(|b| half::f16::from_bits(u16::from_le_bytes([b[0], b[1]])).to_f32())
            .collect()
    }

    fn pattern(width: u32, height: u32, phase: f32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = ((i % width) as f32, (i / width) as f32);
                [
                    (x * 0.37 + phase).sin() * 2.0,
                    (y * 0.21 - phase).cos() + 1.5,
                    ((x + y) % 5.0) * 0.25 - 0.5,
                    1.0,
                ]
            })
            .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            .collect()
    }

    fn source_texture(
        gpu: &pfx_gpu::Gpu,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        bytes: &[u8],
    ) -> wgpu::Texture {
        let texel = bytes.len() as u32 / (width * height);
        gpu.device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("content source"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &bytes[..(width * height * texel) as usize],
        )
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_gpu_texture_fills_a_slot_like_bytes_with_its_mips() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut slots = ContentSlots::new(&gpu.device, &gpu.queue);
        let (width, height) = (37u32, 22u32);
        let bytes = pattern(width, height, 0.0);
        for slot in 0..2 {
            slots
                .allocate(&gpu.device, slot, width, height, ContentFormat::Linear16)
                .unwrap();
        }
        slots
            .write(&gpu.device, &gpu.queue, 0, [0, 0, width, height], &bytes)
            .unwrap();
        let source = source_texture(
            &gpu,
            width,
            height,
            wgpu::TextureFormat::Rgba16Float,
            &bytes,
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let kind = slots.copy(&gpu.device, &mut encoder, 1, &source).unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        assert_eq!(kind, ContentCopy::Copied);
        let (written, copied) = (slots.get(0).unwrap(), slots.get(1).unwrap());
        assert_eq!(copied.mips, 6);
        assert_eq!(copied.writes, 1);
        for level in 0..copied.mips {
            assert_eq!(
                read_level(&gpu, written, level),
                read_level(&gpu, copied, level),
                "level {level}"
            );
        }

        let narrow: Vec<u8> = (0..width * height)
            .flat_map(|i| [(i * 7 % 256) as u8, (i * 3 % 256) as u8, 128, 255])
            .collect();
        let eight = source_texture(
            &gpu,
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            &narrow,
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        assert_eq!(
            slots.copy(&gpu.device, &mut encoder, 1, &eight).unwrap(),
            ContentCopy::Drawn
        );
        gpu.queue.submit(Some(encoder.finish()));
        let drawn = halves(&read_level(&gpu, slots.get(1).unwrap(), 0));
        for (got, want) in drawn.iter().zip(&narrow) {
            assert!(
                (got - f32::from(*want) / 255.0).abs() < 2e-3,
                "{got} {want}"
            );
        }
        let unusable = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("content unusable"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        assert!(slots.copy(&gpu.device, &mut encoder, 1, &unusable).is_err());
        assert!(slots.copy(&gpu.device, &mut encoder, 2, &source).is_err());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_copied_slot_regenerates_its_mips_every_frame() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut slots = ContentSlots::new(&gpu.device, &gpu.queue);
        let (width, height) = (64u32, 32u32);
        slots
            .allocate(&gpu.device, 3, width, height, ContentFormat::Linear16)
            .unwrap();
        for (index, phase) in [0.0f32, 1.3, 2.9].into_iter().enumerate() {
            let bytes = pattern(width, height, phase);
            let source = source_texture(
                &gpu,
                width,
                height,
                wgpu::TextureFormat::Rgba16Float,
                &bytes,
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            if index == 1 {
                encoder.copy_texture_to_texture(
                    source.as_image_copy(),
                    slots.get(3).unwrap().texture.as_image_copy(),
                    source.size(),
                );
                slots.regenerate(&mut encoder, 3).unwrap();
            } else {
                slots.copy(&gpu.device, &mut encoder, 3, &source).unwrap();
            }
            gpu.queue.submit(Some(encoder.finish()));
            let content = slots.get(3).unwrap();
            let mut above = halves(&bytes);
            for level in 1..content.mips {
                let (w, h) = ((width >> level).max(1), (height >> level).max(1));
                let (pw, ph) = (
                    (width >> (level - 1)).max(1),
                    (height >> (level - 1)).max(1),
                );
                let got = halves(&read_level(&gpu, content, level));
                let mut want = vec![0.0; (w * h * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        for c in 0..4 {
                            let mut sum = 0.0;
                            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                                let sx = (2 * x + dx).min(pw - 1);
                                let sy = (2 * y + dy).min(ph - 1);
                                sum += above[((sy * pw + sx) * 4 + c) as usize];
                            }
                            want[((y * w + x) * 4 + c) as usize] = sum / 4.0;
                        }
                    }
                }
                for (index, (g, e)) in got.iter().zip(&want).enumerate() {
                    assert!(
                        (g - e).abs() < 4e-3 * (1.0 + e.abs()),
                        "phase {phase} level {level} value {index}: {g} against {e}"
                    );
                }
                above = got;
            }
        }
        assert_eq!(slots.get(3).unwrap().writes, 3);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_floor_sized_copy_with_mips_stays_cheap() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut slots = ContentSlots::new(&gpu.device, &gpu.queue);
        let mut profiler = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        if !profiler.marks_supported() {
            println!("no encoder timestamps on this adapter; nothing timed");
            return;
        }
        for (width, height) in [(1968u32, 1192u32), (984, 596)] {
            slots
                .allocate(&gpu.device, 0, width, height, ContentFormat::Linear16)
                .unwrap();
            let bytes = pattern(width, height, 0.5);
            let source = source_texture(
                &gpu,
                width,
                height,
                wgpu::TextureFormat::Rgba16Float,
                &bytes,
            );
            let mut times = Vec::new();
            let mut parts = [Vec::new(), Vec::new()];
            for _ in 0..24 {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                profiler.mark(&mut encoder, "start");
                slots.copy(&gpu.device, &mut encoder, 0, &source).unwrap();
                profiler.mark(&mut encoder, "copy and mips");
                let content = slots.get(0).unwrap();
                encoder.copy_texture_to_texture(
                    source.as_image_copy(),
                    content.texture.as_image_copy(),
                    source.size(),
                );
                profiler.mark(&mut encoder, "copy");
                encode_mips(&slots.mipper, &mut encoder, content);
                profiler.mark(&mut encoder, "mips");
                let slot = profiler.finish(&mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                if let Some(slot) = slot {
                    profiler.submitted(slot);
                }
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                for frame in profiler.collect(&gpu.device) {
                    for timing in frame {
                        match timing.label.as_str() {
                            "copy" => parts[0].push(timing.milliseconds),
                            "mips" => parts[1].push(timing.milliseconds),
                            _ => times.push(timing.milliseconds),
                        }
                    }
                }
            }
            times.sort_by(f64::total_cmp);
            assert!(!times.is_empty());
            for (name, part) in ["copy", "mips"].iter().zip(&mut parts) {
                part.sort_by(f64::total_cmp);
                println!("  {name} alone: median {:.4} ms", part[part.len() / 2]);
            }
            println!(
                "{width}x{height} Rgba16Float slot from a GPU texture with {} mips: median {:.4} ms, max {:.4} ms over {} copies",
                slots.get(0).unwrap().mips,
                times[times.len() / 2],
                times[times.len() - 1],
                times.len()
            );
        }
    }

    #[test]
    fn mip_shaders_validate() {
        for source in [MIP_WGSL, MIP_COMPUTE_WGSL] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn caustics_project_along_the_sun_onto_their_plane() {
        let projection = CausticProjection {
            rect: [-1.0, -2.0, 4.0, 2.0],
            height: 0.5,
            toward_sun: [0.0, 2.0, 0.0],
        };
        assert_eq!(projection.uv([1.0, 0.0, -1.0]), Some([0.5, 0.5]));
        assert_eq!(projection.uv([3.5, 0.0, -1.0]), None);
        let slanted = CausticProjection {
            toward_sun: [1.0, 1.0, 0.0],
            ..projection
        };
        assert_eq!(slanted.uv([0.5, 0.0, -1.0]), Some([0.5, 0.5]));
        assert_eq!(slanted.uv([0.0, -0.5, -1.0]), Some([0.5, 0.5]));
        let set = CausticProjection {
            toward_sun: [1.0, -0.2, 0.0],
            ..projection
        };
        assert_eq!(set.uv([1.0, 0.0, -1.0]), None);
        let rows = CausticProjection::rows(Some(slanted), 2, 0.75);
        assert_eq!(rows[0], [-1.0, -2.0, 4.0, 2.0]);
        assert!((rows[1][0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(rows[1][3], 0.5);
        assert_eq!(rows[2], [2.0, 0.75, 1.0, 0.0]);
        assert_eq!(CausticProjection::rows(Some(set), 2, 0.75)[2][2], 0.0);
        assert_eq!(CausticProjection::rows(None, 2, 0.75)[2][2], 0.0);
        let caustic = Caustic {
            slot: 1,
            rect: [0.0, 0.0, 1.0, 1.0],
            height: 0.0,
            strength: 1.0,
            receivers: vec![0, 4],
        };
        assert!(caustic.check().is_ok());
        assert!(caustic.receives(4) && !caustic.receives(1));
        assert!(
            Caustic {
                slot: 4,
                ..caustic.clone()
            }
            .check()
            .is_err()
        );
        assert!(
            Caustic {
                rect: [0.0, 0.0, 0.0, 1.0],
                ..caustic.clone()
            }
            .check()
            .is_err()
        );
        assert!(
            Caustic {
                strength: f32::NAN,
                ..caustic
            }
            .check()
            .is_err()
        );
    }

    #[test]
    fn the_bakes_detail_mips_match_the_live_mip_chain() {
        use pfx_bake::detail::atlas::{Kind as Map, chain};
        let mut state = 9u32;
        let bytes: Vec<u8> = (0..32 * 16 * 4)
            .map(|index| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                if index % 4 == 3 && (index / 4) % 7 == 0 {
                    0
                } else {
                    (state >> 23) as u8
                }
            })
            .collect();
        for (map, kind, space) in [
            (Map::Base, Kind::Base, ColorSpace::Linear),
            (Map::Normal, Kind::Normal, ColorSpace::Linear),
            (Map::Roughness, Kind::Scalar, ColorSpace::Linear),
        ] {
            let image = Image {
                width: 32,
                height: 16,
                space,
                pixels: Pixels::Eight(bytes.clone()),
            };
            let live = mip_chain(&image, kind).unwrap();
            let baked = chain(map, 32, 16, &bytes, 5);
            assert_eq!(baked.len(), 5);
            for (level, (_, _, expected)) in live.iter().take(5).enumerate() {
                assert_eq!(&baked[level], expected, "{} level {level}", map.name());
            }
        }
    }

    const DETAIL_TEST_WGSL: &str = r#"
struct DetailProbe {
    lo: vec2f,
    hi: vec2f,
    mode: vec4u,
}
@group(0) @binding(0) var<uniform> detail_probe: DetailProbe;
struct DetailOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
}
@vertex fn detail_vertex(@builtin(vertex_index) index: u32) -> DetailOut {
    let corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0)
    );
    let corner = corners[index];
    return DetailOut(vec4f(corner * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), mix(detail_probe.lo, detail_probe.hi, corner));
}
@fragment fn detail_fragment(input: DetailOut) -> @location(0) vec4f {
    let r = map_evaluate(vec3f(1.0), 0.5, 0.0, 0.05, 0.0, vec3f(0.0, 0.0, 1.0), vec4f(1.0, 0.0, 0.0, 1.0), input.uv, vec3f(0.0), 0.0, 0.0, 0.0, 0u);
    if (detail_probe.mode.x == 1u) {
        return vec4f(r.normal * 0.5 + vec3f(0.5), 1.0);
    }
    return vec4f(r.base, r.roughness);
}
"#;

    struct DetailRig {
        device: wgpu::Device,
        queue: wgpu::Queue,
        layout: wgpu::BindGroupLayout,
        maps: wgpu::BindGroupLayout,
        empty: wgpu::BindGroup,
        pipeline: wgpu::RenderPipeline,
    }

    impl DetailRig {
        fn new(features: wgpu::Features) -> Option<Self> {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL,
                ..Default::default()
            });
            let adapter = instance
                .enumerate_adapters(wgpu::Backends::VULKAN | wgpu::Backends::METAL)
                .into_iter()
                .find(|adapter| adapter.get_info().device_type == wgpu::DeviceType::DiscreteGpu)
                .or_else(|| {
                    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                    }))
                    .ok()
                })?;
            if !adapter.features().contains(features) {
                return None;
            }
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("detail rig"),
                    required_features: features,
                    required_limits: wgpu::Limits::default(),
                    ..Default::default()
                }))
                .ok()?;
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("detail probe"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("empty"),
                entries: &[],
            });
            let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("empty"),
                layout: &empty_layout,
                entries: &[],
            });
            let maps = MapTextures::layout(&device);
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("detail probe"),
                bind_group_layouts: &[&layout, &empty_layout, &empty_layout, &maps],
                push_constant_ranges: &[],
            });
            let source = format!("{}\n{}", shader_source(), DETAIL_TEST_WGSL);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("detail probe"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("detail probe"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("detail_vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("detail_fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });
            Some(Self {
                device,
                queue,
                layout,
                maps,
                empty,
                pipeline,
            })
        }

        fn draw(
            &self,
            textures: &MapTextures,
            material: GpuMapMaterial,
            uv: [f32; 4],
            size: u32,
            mode: u32,
        ) -> Vec<f32> {
            let device = &self.device;
            let materials = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("detail material"),
                contents: bytemuck::bytes_of(&material),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let maps = textures.bind_group(device, &self.maps, &materials);
            let mut probe = [0u8; 32];
            probe[..16].copy_from_slice(bytemuck::cast_slice(&uv));
            probe[16..20].copy_from_slice(&mode.to_le_bytes());
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("detail probe"),
                contents: &probe,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("detail probe"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            });
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("detail probe"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let row = (size * 16).div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("detail probe readback"),
                size: u64::from(row * size),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("detail probe"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_bind_group(1, &self.empty, &[]);
                pass.set_bind_group(2, &self.empty, &[]);
                pass.set_bind_group(3, &maps, &[]);
                pass.draw(0..6, 0..1);
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(size),
                    },
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = slice.get_mapped_range();
            let mut out = Vec::with_capacity((size * size * 4) as usize);
            for y in 0..size {
                let start = (y * row) as usize;
                out.extend_from_slice(bytemuck::cast_slice::<u8, f32>(
                    &bytes[start..start + (size * 16) as usize],
                ));
            }
            out
        }
    }

    struct DetailCase {
        density: u32,
        lo: [f32; 2],
        hi: [f32; 2],
        material: Material,
    }

    fn detail_cases() -> Vec<DetailCase> {
        let noisy = |layers: [NoiseLayer; 2], seed: u32| {
            let mut material = Material {
                base: [0.6, 0.45, 0.3],
                ..Material::default()
            };
            for (slot, layer) in layers.into_iter().enumerate() {
                material.layers[slot] = NoiseLayer {
                    seed: seed + slot as u32,
                    ..layer
                };
            }
            material
        };
        vec![
            DetailCase {
                density: 512,
                lo: [0.10, 0.20],
                hi: [0.35, 0.40],
                material: noisy(
                    [
                        PlankWood::SAMPLE.layer(1.0, 0),
                        pfx_materials::Crinkle::SAMPLE.layer(0.6, 0),
                    ],
                    3,
                ),
            },
            DetailCase {
                density: 256,
                lo: [0.55, 0.05],
                hi: [0.90, 0.30],
                material: noisy(
                    [
                        NoiseLayer::new(NoiseKind::Value, 50.0, 0.8, 0),
                        Scratch::SAMPLE.layer(0.8, 0),
                    ],
                    17,
                ),
            },
            DetailCase {
                density: 512,
                lo: [0.60, 0.60],
                hi: [0.75, 0.95],
                material: noisy(
                    [
                        Grime::SAMPLE.layer(1.0, 0),
                        CoatWobble::SAMPLE.layer(1.0, 0),
                    ],
                    29,
                ),
            },
        ]
    }

    fn detail_input<'a>(
        case: &'a DetailCase,
        buffers: &'a DetailBuffers,
    ) -> pfx_bake::detail::Input<'a> {
        pfx_bake::detail::Input {
            positions: &buffers.positions,
            normals: &buffers.normals,
            tangents: &buffers.tangents,
            uvs: &buffers.uvs,
            uvs1: None,
            indices: &[0, 1, 2, 2, 1, 3],
            material: &case.material,
        }
    }

    struct DetailBuffers {
        positions: [[f32; 3]; 4],
        normals: [[f32; 3]; 4],
        tangents: [[f32; 4]; 4],
        uvs: [[f32; 2]; 4],
    }

    fn detail_buffers(case: &DetailCase) -> DetailBuffers {
        let uvs = [
            [case.lo[0], case.lo[1]],
            [case.hi[0], case.lo[1]],
            [case.lo[0], case.hi[1]],
            [case.hi[0], case.hi[1]],
        ];
        DetailBuffers {
            positions: uvs.map(|uv| [uv[0], uv[1], 0.0]),
            normals: [[0.0, 0.0, 1.0]; 4],
            tangents: [[1.0, 0.0, 0.0, 1.0]; 4],
            uvs,
        }
    }

    fn worst(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max)
    }

    fn psnr(a: &[f32], b: &[f32]) -> f64 {
        let mean = a
            .iter()
            .zip(b)
            .map(|(a, b)| f64::from((a - b) * 255.0).powi(2))
            .sum::<f64>()
            / a.len() as f64;
        if mean == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0f64 * 255.0 / mean).log10()
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn cropped_compressed_detail_samples_like_the_full_tile() {
        use pfx_bake::detail::{self, atlas};
        let started = std::time::Instant::now();
        let cases = detail_cases();
        let buffers: Vec<DetailBuffers> = cases.iter().map(detail_buffers).collect();
        let mut pieces = Vec::new();
        let mut tiles = Vec::new();
        for (index, (case, buffer)) in cases.iter().zip(&buffers).enumerate() {
            let input = detail_input(case, buffer);
            let crop = detail::crop(&input, case.density, 8).unwrap();
            pieces.push(atlas::Piece {
                index: index as u32,
                node: format!("part{index}"),
                input_hash: detail::input_hash(&input, case.density, 8).unwrap(),
                region: detail::bake_crop(&input, case.density, 8, crop).unwrap(),
            });
            tiles.push(detail::bake(&input, case.density, 8).unwrap());
        }
        let plain = atlas::assemble(pieces, 512).unwrap();
        let packed = plain.compress().unwrap();
        let blocks = DetailRig::new(wgpu::Features::TEXTURE_COMPRESSION_BC)
            .expect("the test GPU decodes BC textures");
        let fallback = DetailRig::new(wgpu::Features::empty()).unwrap();
        assert!(!uploads_blocks(&fallback.device));
        let plain_maps = MapTextures::detail(&blocks.device, &blocks.queue, &plain).unwrap();
        let packed_maps = MapTextures::detail(&blocks.device, &blocks.queue, &packed).unwrap();
        let decoded_maps = MapTextures::detail(&fallback.device, &fallback.queue, &packed).unwrap();
        assert_eq!(
            packed_maps.base.texture.format(),
            wgpu::TextureFormat::Bc7RgbaUnormSrgb
        );
        assert_eq!(
            packed_maps.normal.texture.format(),
            wgpu::TextureFormat::Bc5RgUnorm
        );
        assert_eq!(
            packed_maps.roughness.texture.format(),
            wgpu::TextureFormat::Bc4RUnorm
        );
        assert_eq!(
            decoded_maps.base.texture.format(),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
        assert!(packed_maps.two_channel_normals && decoded_maps.two_channel_normals);
        assert!(!plain_maps.two_channel_normals);
        for (index, (case, tile)) in cases.iter().zip(tiles).enumerate() {
            let images: Vec<Image> = [tile.base, tile.normal, tile.roughness]
                .into_iter()
                .map(|bytes| Image {
                    width: tile.size,
                    height: tile.size,
                    space: ColorSpace::Linear,
                    pixels: Pixels::Eight(bytes),
                })
                .collect();
            let full = MapTextures::new(
                &blocks.device,
                &blocks.queue,
                &MapImages {
                    base: &images[0..1],
                    normal: &images[1..2],
                    roughness: &images[2..3],
                    metal: &[],
                },
            )
            .unwrap();
            let mut material = case.material;
            select_baked_detail(&mut material, 0).unwrap();
            let tile_material =
                full.material(&material, MapIndices::from_material(&material).unwrap());
            assert_eq!(tile_material.seeds[3], 1);
            select_baked_detail(&mut material, index as u32).unwrap();
            let indices = MapIndices::from_material(&material).unwrap();
            let atlas_material = plain_maps.material(&material, indices);
            assert_eq!(atlas_material.seeds[3], 2);
            assert_eq!(packed_maps.material(&material, indices).seeds[3], 3);
            let uv = [case.lo[0], case.lo[1], case.hi[0], case.hi[1]];
            let texels = ((case.hi[0] - case.lo[0]) * case.density as f32) as u32;
            for size in [texels, texels / 4, texels.div_ceil(14)] {
                for mode in [0, 1] {
                    let reference = blocks.draw(&full, tile_material, uv, size, mode);
                    let cropped = blocks.draw(&plain_maps, atlas_material, uv, size, mode);
                    let compressed = blocks.draw(
                        &packed_maps,
                        packed_maps.material(&material, indices),
                        uv,
                        size,
                        mode,
                    );
                    let decoded = fallback.draw(
                        &decoded_maps,
                        decoded_maps.material(&material, indices),
                        uv,
                        size,
                        mode,
                    );
                    let crop_worst = worst(&reference, &cropped);
                    let block_worst = worst(&reference, &compressed);
                    let fallback_worst = worst(&compressed, &decoded);
                    println!(
                        "part {index} {}² at {size} px, {}: cropped {:.2}/255, compressed {:.2}/255 (PSNR {:.1} dB), CPU-decoded against BC {:.2}/255",
                        case.density,
                        ["base and roughness", "normal"][mode as usize],
                        crop_worst * 255.0,
                        block_worst * 255.0,
                        psnr(&reference, &compressed),
                        fallback_worst * 255.0
                    );
                    assert!(
                        crop_worst <= 1.0 / 255.0,
                        "cropped differs by {}",
                        crop_worst * 255.0
                    );
                    assert!(
                        fallback_worst <= 2.0 / 255.0,
                        "decoded differs by {}",
                        fallback_worst * 255.0
                    );
                    assert!(psnr(&reference, &compressed) > 30.0);
                }
            }
        }
        println!("detail atlas GPU check {:?}", started.elapsed());
    }
}
