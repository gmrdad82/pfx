struct Frame {
    size: vec4u,
    counts: vec4u,
    origin: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
    sun_dir: vec4f,
    sun_color: vec4f,
    lens: vec4f,
    window: vec4u,
    columns: vec4u,
    fog: vec4f,
    fog_tint: vec4f,
    fog_light: vec4f,
}
override TOTAL_BOUNCES: u32 = 16u;
override DIFFUSE_BOUNCES: u32 = 4u;
override GLOSSY_BOUNCES: u32 = 8u;
override TRANSMISSION_BOUNCES: u32 = 12u;
override BLOCK_STRIDE: u32 = 1u;
override TRANSMISSIVE_SHADOWS: bool = false;
override CLAMP_INDIRECT: f32 = 0.0;
override FILTER_GLOSSY: f32 = 0.0;
override SOBOL: bool = false;
override TRACE_LOBES: bool = true;
override TRACE_MAPS: bool = true;
override TRACE_WET: bool = true;
override TRACE_THIN: bool = true;
override TRACE_ALPHA: bool = true;
override TRACE_FLAKES: bool = true;
override TRACE_MOTION: bool = false;
override TRACE_FOG: bool = false;
override TRACE_SPRITES: bool = false;
override RT_WALK: bool = true;
override SOBOL_LENGTH: u32 = 256u;
override STEAM_SPREAD_BASE: f32 = 0.7;
override STEAM_SPREAD_GROWTH: f32 = 4.0;
override STEAM_DRIFT_X_QUAD: f32 = -0.9;
override STEAM_DRIFT_X_LIN: f32 = -0.12;
override STEAM_DRIFT_Z_QUAD: f32 = 0.0;
override STEAM_DRIFT_Z_LIN: f32 = 0.02;
override STEAM_SWAY_X_WAVE: f32 = 17.0;
override STEAM_SWAY_X_RATE: f32 = 1.3;
override STEAM_SWAY_X_AMP: f32 = 0.012;
override STEAM_SWAY_X_BASE: f32 = 0.4;
override STEAM_SWAY_X_GROWTH: f32 = 6.0;
override STEAM_SWAY_Z_WAVE: f32 = 13.0;
override STEAM_SWAY_Z_RATE: f32 = 0.9;
override STEAM_SWAY_Z_AMP: f32 = 0.01;
override STEAM_SWAY_Z_BASE: f32 = 0.4;
override STEAM_SWAY_Z_GROWTH: f32 = 5.0;
override STEAM_FADE_IN: f32 = 0.02;
override STEAM_FADE_OUT_START: f32 = 0.07;
override STEAM_FADE_OUT_END: f32 = 0.2;
override STEAM_GRAIN_H: f32 = 70.0;
override STEAM_GRAIN_V: f32 = 42.0;
override STEAM_LIFT_RATE: f32 = 0.05;
override STEAM_WARP_AMP: f32 = 2.2;
override STEAM_WARP_RATE: f32 = 0.2;
override STEAM_CHURN: f32 = 0.7;
override STEAM_THRESHOLD_LOW: f32 = 0.42;
override STEAM_THRESHOLD_HIGH: f32 = 0.9;
struct Tri { a: vec4f, b: vec4f, c: vec4f, na: vec4f, nb: vec4f, nc: vec4f }
struct Node { lo: vec4f, hi: vec4f, first: u32, count: u32, pad: vec2u }
struct Shape { a: vec4f, b: vec4f, c: vec4f, d: vec4f }
struct Material {
    base_roughness: vec4f,
    metal_spec_coat: vec4f,
    sheen_trans_ior_disp: vec4f,
    thick_sub_film: vec4f,
    tint_absorption: vec4f,
    emission_film: vec4f,
    extension: vec4f,
    wet: vec4f,
}
struct MaterialMaps { textures: vec4f, scales: vec4f, extra: vec4f, transform: vec4f, detail: vec4f, normal: vec4f, orm: vec4f, turns: vec4f }
struct MaterialLobes { sheen: vec4f, lobes: vec4f, scatter: vec4f }
struct Hit { t: f32, n: vec3f, geometric: vec3f, material: u32, uv: vec2f, tangent: vec3f, bitangent: vec3f, surface: u32, shape: u32, record: u32 }
struct ContentInfo { offset: u32, width: u32, height: u32, kind: u32 }
struct InstanceSurface { clip0: vec4f, clip1: vec4f, transform: vec4f, crop: vec4f, info: vec4u, forward0: vec4f, forward1: vec4f, forward2: vec4f, inverse0: vec4f, inverse1: vec4f, inverse2: vec4f }
struct ContentLayer { layer: vec4f, extra: vec4f }
struct LightSample { dir: vec3f, pdf: f32, channel: u32, weight: vec3f, lobe: u32 }
struct Accum { color: vec4f, albedo: vec4f, normal: vec4f }
struct DetailResult { shade: Shaded, normal: vec3f }
struct Candidate { wi: vec3f, scale: vec3f, pdf: f32, roughness: f32, direction: vec3f, limit: f32, shape: u32 }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> triangles: array<Tri>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> shapes: array<Shape>;
@group(0) @binding(4) var<storage, read> materials: array<vec4f>;
@group(0) @binding(5) var<storage, read> triangle_uv_v: array<vec4f>;
@group(0) @binding(6) var<storage, read> content_words: array<vec4u>;
@group(0) @binding(7) var<storage, read_write> accum: array<Accum>;
@group(0) @binding(8) var output: texture_storage_2d<rgba32float, write>;
@group(0) @binding(9) var albedo: texture_storage_2d<rgba32float, write>;
@group(0) @binding(10) var normal: texture_storage_2d<rgba32float, write>;
@group(0) @binding(11) var<storage, read> content_info: array<ContentInfo>;
@group(0) @binding(12) var<storage, read> instance_surfaces: array<InstanceSurface>;
@group(0) @binding(13) var<storage, read> content_layers: array<ContentLayer>;

const HOT_ROWS: u32 = 8u;
const EXTENSION_LAYERS: u32 = 7u;
const EXTENSION_MAPS: u32 = 8u;
const EXTENSION_LOBES: u32 = 16u;
const EXTENSION_THIN: u32 = 32u;
const EXTENSION_ALPHA: u32 = 64u;
const EXTENSION_FLAKES: u32 = 128u;
var<private> traced_flaked: bool = false;
var<private> traced_flake: vec3f = vec3f(0.0, 0.0, 1.0);
var<private> traced_thin: bool = false;
var<private> volume_vertex: bool = false;
var<private> sprites_hidden: bool = false;
const SPRITE: u32 = 16777216u;
const SPRITE_PASSES: u32 = 256u;
const COUNT_PATHS: u32 = 0u;
const COUNT_PRIMARY: u32 = 1u;
const COUNT_BOUNCE: u32 = 2u;
const COUNT_CLOSEST: u32 = 3u;
const COUNT_SHADOW: u32 = 4u;
const COUNT_SURFACE_SHADOW: u32 = 5u;
const COUNT_CLOSEST_CANDIDATES: u32 = 6u;
const COUNT_SHADOW_CANDIDATES: u32 = 7u;
const COUNT_MOVER_QUERIES: u32 = 8u;
const COUNT_MOVER_WALKS: u32 = 9u;
const COUNT_KINDS: u32 = 10u;
fn trace_counters_start() {}
fn trace_count(kind: u32) {}
fn trace_count_by(kind: u32, n: u32) {}
fn trace_count_flush() {}
fn trace_counters_end() {}
fn record_at(at: u32) -> Material {
    return Material(materials[at],materials[at+1u],materials[at+2u],materials[at+3u],
        materials[at+4u],materials[at+5u],materials[at+6u],materials[at+7u]);
}
fn material_at(material: u32) -> Material {
    return record_at((material+1u)*HOT_ROWS);
}
fn material_row(material: u32, row: u32) -> vec4f {
    return materials[(material+1u)*HOT_ROWS+row];
}
fn effects() -> Material {
    return record_at(0u);
}
fn extension_flags(m: Material) -> u32 {
    return bitcast<u32>(m.extension.y);
}
fn extension_lobes(m: Material) -> u32 {
    return bitcast<u32>(m.extension.z)+select(0u,8u,(extension_flags(m)&EXTENSION_MAPS)!=0u);
}
fn extension_layers(m: Material) -> u32 {
    return extension_lobes(m)+select(0u,3u,(extension_flags(m)&EXTENSION_LOBES)!=0u);
}
fn extension_alpha(m: Material) -> u32 {
    return extension_layers(m)+(extension_flags(m)&EXTENSION_LAYERS)*5u;
}
fn extension_flakes(m: Material) -> u32 {
    return extension_alpha(m)+select(0u,1u,(extension_flags(m)&EXTENSION_ALPHA)!=0u);
}
fn flake_eval(s: Shaded, wo: vec3f, wi: vec3f) -> vec3f {
    if (wo.z<=1e-5 || wi.z<=1e-8) { return vec3f(0.0); }
    let m=traced_flake;
    let mv=dot(wo,m);
    let ml=dot(wi,m);
    var facet=vec3f(0.0);
    if (mv>1e-5 && ml>1e-8) {
        let h=normalize(wo+wi);
        let a=alpha_rough(s.roughness);
        facet=ggx_spec(mv,ml,max(dot(h,m),0.0),a*a,schlick(s.base,max(dot(wo,h),0.0),5.0))*ml/wi.z;
    }
    return flake_coat(s,wo,wi)+facet*flake_under_coat(s,wo.z);
}
fn material_maps(m: Material) -> MaterialMaps {
    let at=bitcast<u32>(m.extension.z);
    return MaterialMaps(materials[at],materials[at+1u],materials[at+2u],materials[at+3u],materials[at+4u],materials[at+5u],materials[at+6u],materials[at+7u]);
}
fn material_lobes(m: Material) -> MaterialLobes {
    if ((extension_flags(m)&EXTENSION_LOBES)==0u) { return MaterialLobes(vec4f(0.0),vec4f(0.0),vec4f(0.0)); }
    let at=extension_lobes(m);
    return MaterialLobes(materials[at],materials[at+1u],materials[at+2u]);
}

fn trace_noise_start() {}
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn hash3_xor(p: vec3i) -> f32 {
    let h = u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u;
    return f32(pcg(h) >> 8u) / 16777216.0;
}
fn hash3_nested(p: vec3i) -> f32 {
    let h = pcg(u32(p.x) * 73856093u ^ pcg(u32(p.y) * 19349663u ^ pcg(u32(p.z) * 83492791u)));
    return f32(h >> 8u) / 16777216.0;
}
fn value_noise3(p: vec3f, nested: bool) -> f32 {
    let i=vec3i(floor(p));
    let f=fract(p);
    let u=f*f*(3.0-2.0*f);
    var a0: f32;
    var a1: f32;
    var a2: f32;
    var a3: f32;
    var b0: f32;
    var b1: f32;
    var b2: f32;
    var b3: f32;
    if (nested) {
        a0=hash3_nested(i);
        a1=hash3_nested(i+vec3i(1,0,0));
        a2=hash3_nested(i+vec3i(0,1,0));
        a3=hash3_nested(i+vec3i(1,1,0));
        b0=hash3_nested(i+vec3i(0,0,1));
        b1=hash3_nested(i+vec3i(1,0,1));
        b2=hash3_nested(i+vec3i(0,1,1));
        b3=hash3_nested(i+vec3i(1,1,1));
    } else {
        a0=hash3_xor(i);
        a1=hash3_xor(i+vec3i(1,0,0));
        a2=hash3_xor(i+vec3i(0,1,0));
        a3=hash3_xor(i+vec3i(1,1,0));
        b0=hash3_xor(i+vec3i(0,0,1));
        b1=hash3_xor(i+vec3i(1,0,1));
        b2=hash3_xor(i+vec3i(0,1,1));
        b3=hash3_xor(i+vec3i(1,1,1));
    }
    let a=mix(mix(a0,a1,u.x),mix(a2,a3,u.x),u.y);
    let b=mix(mix(b0,b1,u.x),mix(b2,b3,u.x),u.y);
    return mix(a,b,u.z);
}
fn hash2(p: vec2i, seed: u32) -> f32 {
    var h = u32(p.x) * 0x8da6b343u ^ u32(p.y) * 0xd8163841u;
    h ^= seed;
    h ^= h >> 15u;
    h *= 0x2c1b3c6du;
    h ^= h >> 12u;
    h *= 0x297a2d39u;
    h ^= h >> 15u;
    return f32(h >> 8u) / 16777216.0;
}
fn octave_weight(footprint: f32) -> f32 {
    let t = clamp((footprint - 0.5) * 2.0, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}
fn fbm3_filtered(p: vec3f, octaves: i32, footprint: f32, nested: bool) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    var width = footprint;
    for (var i = 0; i < max(octaves, 0); i++) {
        let weight = octave_weight(width);
        if (weight == 1.0) {
            sum += amp * value_noise3(q, nested);
        } else if (weight == 0.0) {
            sum += amp * 0.5;
        } else {
            sum += amp * (weight * value_noise3(q, nested) + (1.0 - weight) * 0.5);
        }
        q = q * 2.03 + vec3f(1.7, 9.2, 3.1);
        amp *= 0.5;
        width *= 2.03;
    }
    return sum;
}
fn hash21(p: vec2f) -> f32 {
    var q = fract(p * vec2f(123.34, 456.21));
    let d = q.x * (q.x + 45.32) + q.y * (q.y + 45.32);
    q += vec2f(d);
    return fract(q.x * q.y);
}
fn value_noise2_signed(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash21(i), hash21(i + vec2f(1.0, 0.0)), u.x);
    let b = mix(hash21(i + vec2f(0.0, 1.0)), hash21(i + vec2f(1.0, 1.0)), u.x);
    return mix(a, b, u.y) * 2.0 - 1.0;
}
fn flow(p: vec2f, time: f32) -> f32 {
    let warp = vec2f(
        value_noise2_signed(p * 0.006 + vec2f(time * 0.010, 3.1)),
        value_noise2_signed(p * 0.006 + vec2f(7.7, -time * 0.008)),
    );
    return value_noise2_signed(p * 0.009 + warp * 1.6 + vec2f(time * 0.006, 0.0)) * 0.65
        + value_noise2_signed(p * 0.021 - warp + vec2f(0.0, time * 0.009)) * 0.35;
}
fn voronoi_point2(cell: vec2i, seed: u32, randomness: f32) -> vec2f {
    return vec2f(hash2(cell, seed), hash2(cell, seed ^ 0x9e3779b9u)) * randomness;
}

fn voronoi_edge2(p: vec2f, randomness: f32, seed: u32) -> f32 {
    let cell = floor(p);
    let local = p - cell;
    let at = vec2i(cell);
    var closest = vec2f(0.0);
    var nearest = 3.4e38;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let v = vec2f(f32(i), f32(j)) + voronoi_point2(at + vec2i(i, j), seed, randomness) - local;
            let d = dot(v, v);
            if (d < nearest) {
                nearest = d;
                closest = v;
            }
        }
    }
    var edge = 3.4e38;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let v = vec2f(f32(i), f32(j)) + voronoi_point2(at + vec2i(i, j), seed, randomness) - local;
            let across = v - closest;
            let length2 = dot(across, across);
            if (length2 > 0.0001) {
                edge = min(edge, dot((closest + v) * 0.5, across) / sqrt(length2));
            }
        }
    }
    return edge;
}

fn pavement_plane(p: vec3f, n: vec3f) -> vec2f {
    let a = abs(n);
    if (a.y >= a.x && a.y >= a.z) {
        return p.xz;
    }
    if (a.x >= a.z) {
        return p.zy;
    }
    return p.xy;
}

fn pavement_crack(p: vec3f, n: vec3f, a: vec4f, b: vec4f, c: vec4f, seed: u32, pixel: f32) -> f32 {
    if (a.x <= 0.0 || a.y <= 0.0) {
        return 0.0;
    }
    let w = p * a.w;
    let warp = vec2f(value_noise3(w, false), value_noise3(w + vec3f(17.3, 5.1, 11.7), false)) - vec2f(0.5);
    let q = (pavement_plane(p, n) + warp * 2.0 * a.z) / a.x;
    let edge = voronoi_edge2(q, c.x, seed) * a.x;
    let wide = max(a.y, pixel);
    let line = (1.0 - smoothstep(0.0, wide, edge)) * (a.y / wide);
    let open = value_noise3(p * b.y + vec3f(3.7, 1.3, 8.9), false);
    let threshold = 1.0 - b.x;
    return line * smoothstep(threshold - 0.12, threshold + 0.12, open);
}

fn pavement_track(uv: vec2f, a: vec4f, b: vec4f, seed: u32, pixel: f32) -> f32 {
    if (a.x <= 0.0 || a.w <= 0.0) {
        return 0.0;
    }
    let slice = f32(seed % 4096u) * 0.37;
    let wander = (value_noise3(vec3f(uv.y * b.y, slice, 0.0), false) - 0.5) * 2.0 * b.x;
    let across = (fract((uv.x - a.y + wander) / a.x + 0.5) - 0.5) * a.x;
    let off = abs(abs(across) - a.z);
    let band = 1.0 - smoothstep(0.0, a.w, off);
    let streak = 0.6 + 0.8 * value_noise3(vec3f(uv.x * b.z, uv.y * b.w, slice + 5.0), false);
    let weight = octave_weight(pixel * max(b.z, b.w));
    return clamp(band * (1.0 + (streak - 1.0) * weight), 0.0, 1.0);
}

fn value_noise3_trio(a: vec3f, b: vec3f, c: vec3f, nested: bool) -> vec3f {
    let cell = floor(a);
    if (any(floor(b) != cell) || any(floor(c) != cell)) {
        return vec3f(value_noise3(a, nested), value_noise3(b, nested), value_noise3(c, nested));
    }
    let i = vec3i(cell);
    var h0: vec4f;
    var h1: vec4f;
    if (nested) {
        h0 = vec4f(hash3_nested(i), hash3_nested(i + vec3i(1, 0, 0)), hash3_nested(i + vec3i(0, 1, 0)), hash3_nested(i + vec3i(1, 1, 0)));
        h1 = vec4f(hash3_nested(i + vec3i(0, 0, 1)), hash3_nested(i + vec3i(1, 0, 1)), hash3_nested(i + vec3i(0, 1, 1)), hash3_nested(i + vec3i(1, 1, 1)));
    } else {
        h0 = vec4f(hash3_xor(i), hash3_xor(i + vec3i(1, 0, 0)), hash3_xor(i + vec3i(0, 1, 0)), hash3_xor(i + vec3i(1, 1, 0)));
        h1 = vec4f(hash3_xor(i + vec3i(0, 0, 1)), hash3_xor(i + vec3i(1, 0, 1)), hash3_xor(i + vec3i(0, 1, 1)), hash3_xor(i + vec3i(1, 1, 1)));
    }
    return vec3f(lattice_mix(h0, h1, fract(a)), lattice_mix(h0, h1, fract(b)), lattice_mix(h0, h1, fract(c)));
}

fn lattice_mix(h0: vec4f, h1: vec4f, f: vec3f) -> f32 {
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(h0.x, h0.y, u.x), mix(h0.z, h0.w, u.x), u.y);
    let b = mix(mix(h1.x, h1.y, u.x), mix(h1.z, h1.w, u.x), u.y);
    return mix(a, b, u.z);
}

fn fbm3_trio(a: vec3f, b: vec3f, c: vec3f, octaves: i32, footprint: f32) -> vec3f {
    var sum = vec3f(0.0);
    var amp = 0.5;
    var qa = a;
    var qb = b;
    var qc = c;
    var width = footprint;
    for (var i = 0; i < octaves; i++) {
        let weight = octave_weight(width);
        if (weight == 1.0) {
            sum += amp * value_noise3_trio(qa, qb, qc, true);
        } else if (weight == 0.0) {
            sum += vec3f(amp * 0.5);
        } else {
            sum += amp * (weight * value_noise3_trio(qa, qb, qc, true) + (1.0 - weight) * 0.5);
        }
        qa = qa * 2.03 + vec3f(1.7, 9.2, 3.1);
        qb = qb * 2.03 + vec3f(1.7, 9.2, 3.1);
        qc = qc * 2.03 + vec3f(1.7, 9.2, 3.1);
        amp *= 0.5;
        width *= 2.03;
    }
    return sum;
}

fn layer_leaf_vein(uv: vec2f) -> f32 {
    let s = abs(sin(uv.y * 40.0 + uv.x * 3.0));
    let s4 = s * s * s * s;
    return s4 * s4 * s4;
}

fn layer_relief(normal: vec3f, uv: vec2f, position: vec3f, amp: f32, bark: bool, footprint: f32) -> vec3f {
    let s = select(-1.0, 1.0, normal.z >= 0.0);
    let a = -1.0 / (s + normal.z);
    let b = normal.x * normal.y * a;
    let t = vec3f(1.0 + s * normal.x * normal.x * a, s * b, -s * normal.x);
    let bt = vec3f(b, s + normal.y * normal.y * a, -normal.y);
    let e = 0.00012;
    var h: vec3f;
    if (bark) {
        let k = vec3f(200.0, 60.0, 200.0);
        h = 0.0006 * fbm3_trio(position * k, (position + t * e) * k, (position + bt * e) * k, 4, footprint);
    } else {
        let u = uv + vec2f(e, 0.0);
        let v = uv + vec2f(0.0, e);
        h = 0.0002 * fbm3_trio(vec3f(uv * 30.0, 1.0), vec3f(u * 30.0, 1.0), vec3f(v * 30.0, 1.0), 3, footprint) - 0.00008 * vec3f(layer_leaf_vein(uv), layer_leaf_vein(u), layer_leaf_vein(v));
    }
    let bent = normalize(normal - t * ((h.y - h.x) / e * amp) - bt * ((h.z - h.x) / e * amp));
    return select(normal, bent, dot(bent, normal) > 0.05);
}
fn trace_noise_end() {}

fn random(state: ptr<function,u32>) -> f32 {
    *state = pcg(*state);
    return f32(*state >> 8u) / 16777216.0;
}
var<private> rng: vec3u;
fn hash_iqnt2d(x: u32, y: u32) -> u32 {
    let qx=1103515245u*((x>>1u)^y);
    let qy=1103515245u*((y>>1u)^x);
    return 1103515245u*(qx^(qy>>3u));
}
fn trace_sobol_start() {}
fn sobol(dimension: u32) -> vec4f {
    return vec4f(0.0);
}
fn trace_sobol_end() {}
fn rnd(state: ptr<function,u32>, value: f32) -> f32 {
    if (SOBOL) { return value; }
    return random(state);
}
fn light_dim(k: u32) -> u32 {
    if (k==0u) { return 1u; }
    if (k<13u) { return 3u+k; }
    return k<<16u;
}
const ENTRYPOINT_SENTINEL: u32 = 0x76543210u;
const INSTANCE_LEAF: u32 = 1u;
fn to_object(m: Node, p: vec3f, w: f32) -> vec3f {
    let q=vec4f(p,w);
    return vec3f(dot(m.lo,q),dot(m.hi,q),dot(bitcast<vec4f>(vec4u(m.first,m.count,m.pad.x,m.pad.y)),q));
}
fn box_hit(node: Node, o: vec3f, d: vec3f, limit: f32) -> bool {
    var near = 0.0;
    var far = limit;
    for (var axis=0u; axis<3u; axis++) {
        if (abs(d[axis]) < 1e-12) {
            if (o[axis] < node.lo[axis] || o[axis] > node.hi[axis]) { return false; }
        } else {
            let a = (node.lo[axis]-o[axis])/d[axis];
            let b = (node.hi[axis]-o[axis])/d[axis];
            near = max(near,min(a,b));
            far = min(far,max(a,b));
            if (near > far) { return false; }
        }
    }
    return true;
}
struct Rows { x: vec4f, y: vec4f, z: vec4f }
struct Moved { ro: vec3f, rd: vec3f, inv_det: f32 }
const MOTION_SQRT2: f32 = 1.4142135;
var<private> ray_time: f32 = 0.5;
var<private> deform_steps: u32 = 0u;
var<private> deform_len: u32 = 0u;
var<private> deform_first: u32 = 0u;
var<private> deform_base: u32 = 0u;
var<private> deform_step: u32 = 0u;
var<private> deform_t: f32 = 0.0;
fn node_at(i: u32) -> Node {
    return nodes[i];
}
fn motion_vec(k: u32) -> vec4f {
    let node=nodes[frame.columns.y+k/3u];
    let lane=k%3u;
    if (lane==0u) { return node.lo; }
    if (lane==1u) { return node.hi; }
    return bitcast<vec4f>(vec4u(node.first,node.count,node.pad.x,node.pad.y));
}
fn motion_word(k: u32) -> vec4u {
    return bitcast<vec4u>(motion_vec(k));
}
fn quat_mix(q1: vec4f, q2: vec4f, t: f32) -> vec4f {
    let costheta=dot(q1,q2);
    if (costheta>0.9995) { return normalize((1.0-t)*q1+t*q2); }
    let theta=acos(clamp(costheta,-1.0,1.0));
    let qperp=normalize(q2-q1*costheta);
    let thetap=theta*t;
    return q1*cos(thetap)+qperp*sin(thetap);
}
fn rows_compose(q: vec4f, y: vec4f, z: vec4f, w: vec4f) -> Rows {
    let q0=MOTION_SQRT2*q.w;
    let q1=MOTION_SQRT2*q.x;
    let q2=MOTION_SQRT2*q.y;
    let q3=MOTION_SQRT2*q.z;
    let qda=q0*q1;
    let qdb=q0*q2;
    let qdc=q0*q3;
    let qaa=q1*q1;
    let qab=q1*q2;
    let qac=q1*q3;
    let qbb=q2*q2;
    let qbc=q2*q3;
    let qcc=q3*q3;
    let rx=vec3f(1.0-qbb-qcc,-qdc+qab,qdb+qac);
    let ry=vec3f(qdc+qab,1.0-qaa-qcc,-qda+qbc);
    let rz=vec3f(-qdb+qac,qda+qbc,1.0-qaa-qbb);
    let sx=vec3f(y.w,z.z,w.y);
    let sy=vec3f(z.x,z.w,w.z);
    let sz=vec3f(z.y,w.x,w.w);
    return Rows(vec4f(dot(rx,sx),dot(rx,sy),dot(rx,sz),y.x),
        vec4f(dot(ry,sx),dot(ry,sy),dot(ry,sz),y.y),
        vec4f(dot(rz,sx),dot(rz,sy),dot(rz,sz),y.z));
}
fn motion_at(at: u32, steps: u32, time: f32) -> Rows {
    let maxstep=i32(steps)-1;
    let step=min(i32(time*f32(maxstep)),maxstep-1);
    let t=time*f32(maxstep)-f32(step);
    let a=at+u32(step)*4u;
    let b=a+4u;
    return rows_compose(quat_mix(motion_vec(a),motion_vec(b),t),
        (1.0-t)*motion_vec(a+1u)+t*motion_vec(b+1u),
        (1.0-t)*motion_vec(a+2u)+t*motion_vec(b+2u),
        (1.0-t)*motion_vec(a+3u)+t*motion_vec(b+3u));
}
fn rows_point(m: Rows, p: vec3f) -> vec3f {
    let q=vec4f(p,1.0);
    return vec3f(dot(m.x,q),dot(m.y,q),dot(m.z,q));
}
fn rows_vector(m: Rows, v: vec3f) -> vec3f {
    return vec3f(dot(m.x.xyz,v),dot(m.y.xyz,v),dot(m.z.xyz,v));
}
fn rows_transposed(m: Rows, v: vec3f) -> vec3f {
    return m.x.xyz*v.x+m.y.xyz*v.y+m.z.xyz*v.z;
}
fn rows_det(m: Rows) -> f32 {
    return dot(m.x.xyz,cross(m.y.xyz,m.z.xyz));
}
fn rows_inverse(m: Rows) -> Rows {
    var x=vec3f(m.x.x,m.y.x,m.z.x);
    var y=vec3f(m.x.y,m.y.y,m.z.y);
    var z=vec3f(m.x.z,m.y.z,m.z.z);
    let w=vec3f(m.x.w,m.y.w,m.z.w);
    var det=dot(x,cross(y,z));
    if (det==0.0) {
        x.x+=1e-8;
        y.y+=1e-8;
        z.z+=1e-8;
        det=dot(x,cross(y,z));
        if (det==0.0) { det=3.4028235e38; }
    }
    let ix=cross(y,z)/det;
    let iy=cross(z,x)/det;
    let iz=cross(x,y)/det;
    return Rows(vec4f(ix,-dot(ix,w)),vec4f(iy,-dot(iy,w)),vec4f(iz,-dot(iz,w)));
}
fn motion_head() -> vec4u {
    return motion_word(0u);
}
fn slot_at(s: u32) -> u32 {
    let head=motion_head();
    return 1u+4u*head.y+64u*head.z+s*(2u+4u*head.x);
}
fn shutter_time(u: f32) -> f32 {
    let head=motion_head();
    if (head.z==0u) { return u; }
    let first=1u+4u*head.y;
    let x=saturate(u)*255.0;
    let index=min(u32(x),255u);
    let next=min(index+1u,255u);
    let t=x-f32(index);
    let a=motion_vec(first+index/4u)[index%4u];
    if (t==0.0) { return a; }
    let b=motion_vec(first+next/4u)[next%4u];
    return (1.0-t)*a+t*b;
}
fn mover_moves(s: u32) -> bool {
    return motion_word(slot_at(s)).x!=0u;
}
fn mover_rigid(s: u32) -> bool {
    return (motion_word(slot_at(s)).x&1u)!=0u;
}
fn mover_slot(k: u32) -> u32 {
    return motion_word(slot_at(motion_head().w)+k/4u)[k%4u];
}
fn mover_now(s: u32) -> Rows {
    return motion_at(slot_at(s)+2u,motion_head().x,ray_time);
}
fn mover_enter(s: u32, o: vec3f, d: vec3f) -> Moved {
    let info=motion_word(slot_at(s));
    var m=Moved(o,d,bitcast<f32>(instance_surfaces[s+1u].info.w));
    if ((info.x&1u)!=0u) {
        let inverse=rows_inverse(mover_now(s));
        m.ro=rows_point(inverse,o);
        m.rd=rows_vector(inverse,d);
        m.inv_det=rows_det(inverse);
    } else {
        m.ro=record_ray(s+1u,o,1.0);
        m.rd=record_ray(s+1u,d,0.0);
    }
    deform_steps=info.x>>1u;
    if (deform_steps>1u) {
        deform_len=info.y;
        deform_first=info.z;
        deform_base=info.w;
        let maxstep=i32(deform_steps)-1;
        let step=min(i32(ray_time*f32(maxstep)),maxstep-1);
        deform_step=u32(step);
        deform_t=ray_time*f32(maxstep)-f32(step);
    }
    return m;
}
fn mover_leave() {
    deform_steps=0u;
}
fn hit_ray(record: u32, p: vec3f, w: f32) -> vec3f {
    if (TRACE_MOTION && record>0u && mover_rigid(record-1u)) {
        let inverse=rows_inverse(mover_now(record-1u));
        if (w==0.0) { return rows_vector(inverse,p); }
        return rows_point(inverse,p);
    }
    return record_ray(record,p,w);
}
fn tri_at(i: u32) -> Tri {
    let tri=triangles[i];
    if (!TRACE_MOTION || deform_steps<2u) { return tri; }
    let k=i-deform_base;
    let a=triangles[deform_first+deform_step*deform_len+k];
    let b=triangles[deform_first+(deform_step+1u)*deform_len+k];
    let t=deform_t;
    return Tri(vec4f(mix(a.a.xyz,b.a.xyz,t),tri.a.w),
        vec4f(mix(a.b.xyz,b.b.xyz,t),tri.b.w),
        vec4f(mix(a.c.xyz,b.c.xyz,t),tri.c.w),
        vec4f(normalize(mix(a.na.xyz,b.na.xyz,t)),tri.na.w),
        vec4f(normalize(mix(a.nb.xyz,b.nb.xyz,t)),tri.nb.w),
        vec4f(normalize(mix(a.nc.xyz,b.nc.xyz,t)),tri.nc.w));
}
fn moved_world_hit(h: Hit) -> Hit {
    let forward=mover_now(h.record-1u);
    let inverse=rows_inverse(forward);
    var w=h;
    let mirror=select(1.0,-1.0,rows_det(inverse)<0.0);
    w.geometric=normalize(rows_transposed(inverse,h.geometric))*mirror;
    let shaded=rows_transposed(inverse,h.n);
    w.n=select(normalize(shaded),w.geometric,all(h.n==vec3f(0.0)));
    w.tangent=normalize(rows_vector(forward,h.tangent));
    let handed=select(-1.0,1.0,dot(cross(h.geometric,h.tangent),h.bitangent)>=0.0);
    w.bitangent=normalize(cross(w.geometric,w.tangent))*handed;
    return w;
}
const ONE_SIDED: u32 = 8388608u;
const LAYER_MASK: u32 = 262143u;
const NO_MATERIAL: u32 = 4294967295u;
fn hit_layer(word: u32, material: u32) -> ContentLayer {
    let layer=(word>>5u) & LAYER_MASK;
    if (layer>0u) { return content_layers[layer-1u]; }
    return content_layers[material];
}
fn record_material(info: vec4u, tri: Tri) -> u32 {
    return select(info.y,u32(tri.a.w),info.y==NO_MATERIAL);
}
fn record_ray(record: u32, p: vec3f, w: f32) -> vec3f {
    if (record==0u) { return p; }
    let s=instance_surfaces[record];
    let q=vec4f(p,w);
    return vec3f(dot(s.inverse0,q),dot(s.inverse1,q),dot(s.inverse2,q));
}
fn clipped_at(surface: InstanceSurface, point: vec3f) -> bool {
    return (any(surface.clip0.xyz != vec3f(0.0)) && dot(surface.clip0.xyz,point)>surface.clip0.w)
        || (any(surface.clip1.xyz != vec3f(0.0)) && dot(surface.clip1.xyz,point)>surface.clip1.w);
}
fn world_hit(h: Hit) -> Hit {
    if (h.record==0u || h.shape!=0u) { return h; }
    if (TRACE_MOTION && mover_rigid(h.record-1u)) { return moved_world_hit(h); }
    let s=instance_surfaces[h.record];
    var w=h;
    let mirror=select(1.0,-1.0,bitcast<f32>(s.info.w)<0.0);
    w.geometric=normalize(s.inverse0.xyz*h.geometric.x+s.inverse1.xyz*h.geometric.y+s.inverse2.xyz*h.geometric.z)*mirror;
    let shaded=s.inverse0.xyz*h.n.x+s.inverse1.xyz*h.n.y+s.inverse2.xyz*h.n.z;
    w.n=select(normalize(shaded),w.geometric,all(h.n==vec3f(0.0)));
    w.tangent=normalize(vec3f(dot(s.forward0.xyz,h.tangent),dot(s.forward1.xyz,h.tangent),dot(s.forward2.xyz,h.tangent)));
    let handed=select(-1.0,1.0,dot(cross(h.geometric,h.tangent),h.bitangent)>=0.0);
    w.bitangent=normalize(cross(w.geometric,w.tangent))*handed;
    return w;
}
fn alpha_hash(i: u32, o: vec3f, d: vec3f) -> f32 {
    let a=pcg(i^pcg(bitcast<u32>(o.x)^pcg(bitcast<u32>(o.y)^pcg(bitcast<u32>(o.z)))));
    let h=pcg(a^pcg(bitcast<u32>(d.x)^pcg(bitcast<u32>(d.y)^pcg(bitcast<u32>(d.z)))));
    return f32(h>>8u)*(1.0/16777216.0);
}
fn alpha_cut(i: u32, u: f32, v: f32, m: Material, o: vec3f, d: vec3f) -> bool {
    let coverage=materials[extension_alpha(m)];
    var alpha=coverage.x;
    if (TRACE_MAPS && (extension_flags(m)&EXTENSION_MAPS)!=0u) {
        let maps=material_maps(m);
        if (maps.textures.x>=0.0) {
            let tri=triangles[i];
            let uv_v=triangle_uv_v[i];
            var uv=vec2f(tri.na.w*(1.0-u-v)+tri.nb.w*u+tri.nc.w*v,uv_v.x*(1.0-u-v)+uv_v.y*u+uv_v.z*v);
            let first=bitcast<u32>(effects().extension.w);
            if ((u32(round(maps.scales.w))&1u)!=0u && first!=0u) {
                let r0=content_row(first-1u+i*2u);
                let r1=content_row(first-1u+i*2u+1u);
                uv=r0.xy*(1.0-u-v)+r0.zw*u+r1.xy*v;
            }
            alpha*=traced_map(maps.textures.x,uv_place(uv,maps.transform,maps.extra.z)).a;
        }
    }
    if (coverage.z<1.5) { return alpha<coverage.y; }
    return alpha<=alpha_hash(i,o,d);
}
fn sprite_shape(uv: vec2f) -> f32 {
    let c=uv*2.0-1.0;
    let r2=dot(c,c);
    return select(0.0,(1.0-r2)*(1.0-r2),r2<1.0);
}
fn sprite_cut(i: u32, record: u32, u: f32, v: f32, o: vec3f, d: vec3f) -> bool {
    if (sprites_hidden) { return true; }
    let tri=triangles[i];
    let uv_v=triangle_uv_v[i];
    let uv=vec2f(tri.na.w*(1.0-u-v)+tri.nb.w*u+tri.nc.w*v,uv_v.x*(1.0-u-v)+uv_v.y*u+uv_v.z*v);
    let coverage=instance_surfaces[record].transform.w*sprite_shape(uv);
    return coverage<=alpha_hash(i^pcg(record^0x5bd1e995u),o,d);
}
fn cut_out(i: u32, record: u32, u: f32, v: f32, o: vec3f, d: vec3f) -> bool {
    let surface=instance_surfaces[record];
    if (TRACE_SPRITES && (surface.info.x&SPRITE)!=0u) { return sprite_cut(i,record,u,v,o,d); }
    let tri = triangles[i];
    let material=record_material(surface.info,tri);
    if (TRACE_ALPHA) {
        let m=material_at(material);
        if ((extension_flags(m)&EXTENSION_ALPHA)!=0u) { return alpha_cut(i,u,v,m,o,d); }
    }
    if ((surface.info.x&4u)==0u) { return false; }
    let slot=i32(round(hit_layer(surface.info.x,material).layer.x));
    if (slot<0) { return true; }
    let uv_v=triangle_uv_v[i];
    let uv=vec2f(tri.na.w*(1.0-u-v)+tri.nb.w*u+tri.nc.w*v,uv_v.x*(1.0-u-v)+uv_v.y*u+uv_v.z*v);
    let at=content_uv(uv,surface.transform);
    if (!content_inside(at,surface.crop)) { return true; }
    let cutoff=bitcast<f32>(surface.info.z);
    if (cutoff==0.5) { return content_at(slot,at).a<0.5; }
    return content_at(slot,at).a<cutoff;
}
fn tri_fill(i: u32, record: u32, t: f32, u: f32, v: f32, hit: ptr<function,Hit>) {
    let tri = tri_at(i);
    let e1 = tri.b.xyz-tri.a.xyz;
    let e2 = tri.c.xyz-tri.a.xyz;
    (*hit).t=t;
    (*hit).surface=i;
    (*hit).shape=0u;
    (*hit).record=record;
    let geometric=normalize(cross(e1,e2));
    (*hit).geometric=geometric;
    let shaded=tri.na.xyz*(1.0-u-v)+tri.nb.xyz*u+tri.nc.xyz*v;
    (*hit).n=select(select(geometric,vec3f(0.0),record>0u),normalize(shaded),dot(shaded,shaded)>1e-12);
    (*hit).material=record_material(instance_surfaces[record].info,tri);
    let uv_v=triangle_uv_v[i];
    (*hit).uv=vec2f(tri.na.w*(1.0-u-v)+tri.nb.w*u+tri.nc.w*v,
        uv_v.x*(1.0-u-v)+uv_v.y*u+uv_v.z*v);
    let du1=tri.nb.w-tri.na.w;
    let du2=tri.nc.w-tri.na.w;
    let dv1=uv_v.y-uv_v.x;
    let dv2=uv_v.z-uv_v.x;
    let uv_det=du1*dv2-du2*dv1;
    var tangent=normalize(e1);
    if (abs(uv_det)>1e-10) { tangent=normalize((e1*dv2-e2*dv1)/uv_det); }
    (*hit).tangent=tangent;
    (*hit).bitangent=normalize(cross(geometric,tangent))*select(-1.0,1.0,uv_det>=0.0);
}
fn tri_hit(i: u32, record: u32, word: u32, inv_det: f32, o: vec3f, d: vec3f, wo: vec3f, wd: vec3f, shadow_ray: bool, hit: ptr<function,Hit>) {
    let tri = tri_at(i);
    if ((word&1u)!=0u && !shadow_ray) { return; }
    if ((word&2u)!=0u && shadow_ray) { return; }
    let e1 = tri.b.xyz-tri.a.xyz;
    let e2 = tri.c.xyz-tri.a.xyz;
    let p = cross(d,e2);
    let det = dot(e1,p);
    let seen = det*inv_det;
    if (abs(seen)<1e-9) { return; }
    if (!shadow_ray && (word & ONE_SIDED) != 0u && seen < 0.0) { return; }
    let inv = 1.0/det;
    let s = o-tri.a.xyz;
    let u = dot(s,p)*inv;
    if (u<0.0 || u>1.0) { return; }
    let q = cross(s,e1);
    let v = dot(d,q)*inv;
    if (v<0.0 || u+v>1.0) { return; }
    let t = dot(e2,q)*inv;
    if (t>0.0001 && t<(*hit).t) {
        if (clipped_at(instance_surfaces[record],wo+wd*t)) { return; }
        if (cut_out(i,record,u,v,wo,wd)) { return; }
        tri_fill(i,record,t,u,v,hit);
    }
}
fn ellipsoid(p: vec3f, radius: vec3f) -> f32 {
    let k0 = length(p/radius);
    let k1 = length(p/(radius*radius));
    return k0*(k0-1.0)/max(k1,1e-8);
}
fn cone(p: vec3f, a: vec3f, b: vec3f, r1: f32, r2: f32) -> f32 {
    let ba=b-a;
    let length_axis=length(ba);
    if (length_axis<=abs(r1-r2)) {
        return select(length(p-b)-r2,length(p-a)-r1,r1>=r2);
    }
    let l2=dot(ba,ba);
    let rr=r1-r2;
    let a2=l2-rr*rr;
    let pa=p-a;
    let y=dot(pa,ba);
    let z=y-l2;
    let xv=pa*l2-ba*y;
    let x2=dot(xv,xv);
    let y2=y*y*l2;
    let z2=z*z*l2;
    let k=sign(rr)*rr*rr*x2;
    if (sign(z)*a2*z2>k) { return sqrt(x2+z2)/l2-r2; }
    if (sign(y)*a2*y2<k) { return sqrt(x2+y2)/l2-r1; }
    return sqrt(max(x2*a2/l2,0.0))/l2+y*rr/l2-r1;
}
fn base_distance(i: u32, p: vec3f) -> f32 {
    let s=shapes[i];
    let kind=u32(s.a.w);
    if (kind==1u) {
        let q=abs(p-s.a.xyz)-(s.b.xyz-vec3f(s.b.w));
        return length(max(q,vec3f(0.0)))+min(max(q.x,max(q.y,q.z)),0.0)-s.b.w;
    }
    if (kind==2u) { return cone(p,s.a.xyz,s.b.xyz,s.c.x,s.c.y); }
    if (kind==3u) { return ellipsoid(p-s.a.xyz,s.b.xyz); }
    if (kind==4u) {
        let ba=s.b.xyz-s.a.xyz;
        let h=clamp(dot(p-s.a.xyz,ba)/max(dot(ba,ba),1e-9),0.0,1.0);
        return length(p-s.a.xyz-ba*h)-s.c.x;
    }
    return 1e30;
}
fn shape_distance(i: u32, p: vec3f) -> f32 {
    let s=shapes[i];
    if (u32(s.a.w)==5u) {
        let a=base_distance(u32(s.b.x),p);
        let b=base_distance(u32(s.b.y),p);
        let radius=max(s.b.z,1e-6);
        let h=clamp(0.5+0.5*(b-a)/radius,0.0,1.0);
        return mix(b,a,h)-radius*h*(1.0-h);
    }
    return base_distance(i,p);
}
fn roots(a: f32, b: f32, c: f32) -> vec2f {
    if (abs(a)<1e-10) {
        if (abs(b)<1e-10) { return vec2f(1e30); }
        return vec2f(-c/(2.0*b),1e30);
    }
    let disc=b*b-a*c;
    if (disc<0.0) { return vec2f(1e30); }
    let root=sqrt(disc);
    return vec2f((-b-root)/a,(-b+root)/a);
}
fn consider(i: u32, o: vec3f, d: vec3f, t: f32, best: ptr<function,f32>) {
    if (t>0.0001 && t<*best && abs(shape_distance(i,o+d*t))<0.002) { *best=t; }
}
fn primitive_hit(i: u32, o: vec3f, d: vec3f, limit: f32) -> f32 {
    let shape=shapes[i];
    let kind=u32(shape.a.w);
    var best=limit;
    if (kind==3u) {
        let origin=(o-shape.a.xyz)/shape.b.xyz;
        let direction=d/shape.b.xyz;
        let candidates=roots(dot(direction,direction),dot(origin,direction),dot(origin,origin)-1.0);
        consider(i,o,d,candidates.x,&best);
        consider(i,o,d,candidates.y,&best);
    }
    if (kind==1u) {
        let core=max(shape.b.xyz-vec3f(shape.b.w),vec3f(0.0));
        let radius=shape.b.w;
        for (var axis=0u;axis<3u;axis++) {
            if (abs(d[axis])>1e-9) {
                for (var side=0u;side<2u;side++) {
                    let sign_side=select(-1.0,1.0,side==1u);
                    consider(i,o,d,(shape.a[axis]+sign_side*(core[axis]+radius)-o[axis])/d[axis],&best);
                }
            }
            let j=(axis+1u)%3u;
            let k=(axis+2u)%3u;
            for (var sj=0u;sj<2u;sj++) {
                for (var sk=0u;sk<2u;sk++) {
                    let js=select(-1.0,1.0,sj==1u);
                    let ks=select(-1.0,1.0,sk==1u);
                    let oj=o[j]-shape.a[j]-js*core[j];
                    let ok=o[k]-shape.a[k]-ks*core[k];
                    let candidates=roots(d[j]*d[j]+d[k]*d[k],oj*d[j]+ok*d[k],oj*oj+ok*ok-radius*radius);
                    for (var root=0u;root<2u;root++) {
                        let t=candidates[root];
                        if (abs(o[axis]+d[axis]*t-shape.a[axis])<=core[axis]+0.002) { consider(i,o,d,t,&best); }
                    }
                }
            }
        }
        for (var sx=0u;sx<2u;sx++) {
            for (var sy=0u;sy<2u;sy++) {
                for (var sz=0u;sz<2u;sz++) {
                    let corner=shape.a.xyz+vec3f(select(-1.0,1.0,sx==1u),select(-1.0,1.0,sy==1u),select(-1.0,1.0,sz==1u))*core;
                    let v=o-corner;
                    let candidates=roots(dot(d,d),dot(v,d),dot(v,v)-radius*radius);
                    consider(i,o,d,candidates.x,&best);
                    consider(i,o,d,candidates.y,&best);
                }
            }
        }
    }
    if (kind==4u) {
        let ba=shape.b.xyz-shape.a.xyz;
        let oa=o-shape.a.xyz;
        let baba=dot(ba,ba);
        let bard=dot(ba,d);
        let baoa=dot(ba,oa);
        let rdoa=dot(d,oa);
        let oaoa=dot(oa,oa);
        let radius=shape.c.x;
        let candidates=roots(baba*dot(d,d)-bard*bard,baba*rdoa-baoa*bard,baba*oaoa-baoa*baoa-radius*radius*baba);
        for (var root=0u;root<2u;root++) {
            let t=candidates[root];
            let y=baoa+t*bard;
            if (y>=0.0 && y<=baba) { consider(i,o,d,t,&best); }
        }
        for (var end=0u;end<2u;end++) {
            let center=select(shape.a.xyz,shape.b.xyz,end==1u);
            let v=o-center;
            let candidates=roots(dot(d,d),dot(v,d),dot(v,v)-radius*radius);
            consider(i,o,d,candidates.x,&best);
            consider(i,o,d,candidates.y,&best);
        }
    }
    if (kind==2u) {
        let ba=shape.b.xyz-shape.a.xyz;
        let len=length(ba);
        if (len>1e-8) {
            let axis=ba/len;
            let slope=(shape.c.x-shape.c.y)/len;
            if (abs(slope)<1.0) {
                let side=sqrt(1.0-slope*slope);
                let oa=o-shape.a.xyz;
                let y0=dot(oa,axis);
                let yd=dot(d,axis);
                let radial0=oa-axis*y0;
                let radiald=d-axis*yd;
                let e=shape.c.x-slope*y0;
                let candidates=roots(side*side*dot(radiald,radiald)-slope*slope*yd*yd,side*side*dot(radial0,radiald)+e*slope*yd,side*side*dot(radial0,radial0)-e*e);
                for (var root=0u;root<2u;root++) {
                    let t=candidates[root];
                    let y=y0+t*yd;
                    if (y>=slope*shape.c.x-0.002 && y<=len+slope*shape.c.y+0.002) { consider(i,o,d,t,&best); }
                }
            }
        }
        for (var end=0u;end<2u;end++) {
            let center=select(shape.a.xyz,shape.b.xyz,end==1u);
            let radius=select(shape.c.x,shape.c.y,end==1u);
            let v=o-center;
            let candidates=roots(dot(d,d),dot(v,d),dot(v,v)-radius*radius);
            consider(i,o,d,candidates.x,&best);
            consider(i,o,d,candidates.y,&best);
        }
    }
    return best;
}
fn union_hit(i: u32, o: vec3f, d: vec3f, limit: f32) -> f32 {
    var t=0.0;
    var last=shape_distance(i,o);
    for (var step=0u;step<384u;step++) {
        let previous=t;
        t+=max(abs(last),0.00005);
        if (t>=limit) { return limit; }
        let distance=shape_distance(i,o+d*t);
        if (abs(distance)<0.0001) { return t; }
        if ((last>0.0 && distance<0.0) || (last<0.0 && distance>0.0)) {
            var lo=previous;
            var hi=t;
            for (var refine=0u;refine<24u;refine++) {
                let mid=(lo+hi)*0.5;
                let value=shape_distance(i,o+d*mid);
                if (sign(value)==sign(last)) { lo=mid; } else { hi=mid; }
            }
            return (lo+hi)*0.5;
        }
        last=distance;
    }
    return limit;
}
fn scene(o: vec3f, d: vec3f, limit: f32, shadow_ray: bool) -> Hit {
    trace_count(COUNT_CLOSEST);
    var hit=Hit(limit,vec3f(0.0),vec3f(0.0),0u,vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,0u,0u);
    if (frame.counts.x>0u) {
        var stack: array<u32,64>;
        var top=1u;
        stack[0]=0u;
        var ro=o;
        var rd=d;
        var base=0u;
        var record=0u;
        var word=0u;
        var inv_det=1.0;
        while (top>0u) {
            top-=1u;
            let index=stack[top];
            if (index==ENTRYPOINT_SENTINEL) {
                ro=o;
                rd=d;
                base=0u;
                record=0u;
                word=0u;
                inv_det=1.0;
                mover_leave();
                continue;
            }
            let node=nodes[index];
            if (!box_hit(node,ro,rd,hit.t)) { continue; }
            if (node.pad.x==INSTANCE_LEAF) {
                if (TRACE_SPRITES && sprites_hidden && (instance_surfaces[bitcast<u32>(node.lo.w)].info.x&SPRITE)!=0u) { continue; }
                if (top+2u<=64u) {
                    stack[top]=ENTRYPOINT_SENTINEL;
                    stack[top+1u]=node.first;
                    top+=2u;
                    base=node.count;
                    record=bitcast<u32>(node.lo.w);
                    let info=instance_surfaces[record].info;
                    word=info.x;
                    inv_det=bitcast<f32>(info.w);
                    if (node.pad.y>0u) {
                        let m=nodes[node.pad.y];
                        ro=to_object(m,o,1.0);
                        rd=to_object(m,d,0.0);
                        if (TRACE_MOTION && record>0u && mover_moves(record-1u)) {
                            let moved=mover_enter(record-1u,o,d);
                            ro=moved.ro;
                            rd=moved.rd;
                            inv_det=moved.inv_det;
                        }
                    }
                }
            } else if (node.count==0u) {
                if (top+2u<=64u) {
                    stack[top]=node.first;
                    stack[top+1u]=index+1u;
                    top+=2u;
                }
            } else {
                for (var j=0u;j<node.count;j++) { tri_hit(base+node.first+j,record,word,inv_det,ro,rd,o,d,shadow_ray,&hit); }
            }
        }
    }
    for (var i=0u;i<frame.counts.y;i++) {
        if (shapes[i].d.x > 0.5) { continue; }
        var t=hit.t;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,o,d,hit.t); } else { t=primitive_hit(i,o,d,hit.t); }
        if (t<hit.t && t>0.0001) {
            let p=o+d*t;
            let e=0.0002;
            let n=normalize(vec3f(
                shape_distance(i,p+vec3f(e,0.0,0.0))-shape_distance(i,p-vec3f(e,0.0,0.0)),
                shape_distance(i,p+vec3f(0.0,e,0.0))-shape_distance(i,p-vec3f(0.0,e,0.0)),
                shape_distance(i,p+vec3f(0.0,0.0,e))-shape_distance(i,p-vec3f(0.0,0.0,e))));
            hit=Hit(t,n,n,u32(shapes[i].c.w),vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,i+1u,0u);
        }
    }
    return world_hit(hit);
}
fn tri_blocks(i: u32, record: u32, word: u32, inv_det: f32, o: vec3f, d: vec3f, wo: vec3f, wd: vec3f, limit: f32) -> bool {
    let tri = tri_at(i);
    if ((word&2u)!=0u) { return false; }
    let e1 = tri.b.xyz-tri.a.xyz;
    let e2 = tri.c.xyz-tri.a.xyz;
    let p = cross(d,e2);
    let det = dot(e1,p);
    if (abs(det*inv_det)<1e-9) { return false; }
    let inv = 1.0/det;
    let s = o-tri.a.xyz;
    let u = dot(s,p)*inv;
    if (u<0.0 || u>1.0) { return false; }
    let q = cross(s,e1);
    let v = dot(d,q)*inv;
    if (v<0.0 || u+v>1.0) { return false; }
    let t = dot(e2,q)*inv;
    if (t<=0.0001 || t>=limit) { return false; }
    if (clipped_at(instance_surfaces[record],wo+wd*t)) { return false; }
    return !cut_out(i,record,u,v,wo,wd);
}
fn occluded(o: vec3f, d: vec3f, limit: f32) -> bool {
    trace_count(COUNT_SHADOW);
    if (frame.counts.x>0u) {
        var stack: array<u32,64>;
        var top=1u;
        stack[0]=0u;
        var ro=o;
        var rd=d;
        var base=0u;
        var record=0u;
        var word=0u;
        var inv_det=1.0;
        while (top>0u) {
            top-=1u;
            let index=stack[top];
            if (index==ENTRYPOINT_SENTINEL) {
                ro=o;
                rd=d;
                base=0u;
                record=0u;
                word=0u;
                inv_det=1.0;
                mover_leave();
                continue;
            }
            let node=nodes[index];
            if (!box_hit(node,ro,rd,limit)) { continue; }
            if (node.pad.x==INSTANCE_LEAF) {
                if (TRACE_SPRITES && sprites_hidden && (instance_surfaces[bitcast<u32>(node.lo.w)].info.x&SPRITE)!=0u) { continue; }
                if (top+2u<=64u) {
                    stack[top]=ENTRYPOINT_SENTINEL;
                    stack[top+1u]=node.first;
                    top+=2u;
                    base=node.count;
                    record=bitcast<u32>(node.lo.w);
                    let info=instance_surfaces[record].info;
                    word=info.x;
                    inv_det=bitcast<f32>(info.w);
                    if (node.pad.y>0u) {
                        let m=nodes[node.pad.y];
                        ro=to_object(m,o,1.0);
                        rd=to_object(m,d,0.0);
                        if (TRACE_MOTION && record>0u && mover_moves(record-1u)) {
                            let moved=mover_enter(record-1u,o,d);
                            ro=moved.ro;
                            rd=moved.rd;
                            inv_det=moved.inv_det;
                        }
                    }
                }
            } else if (node.count==0u) {
                if (top+2u<=64u) {
                    stack[top]=node.first;
                    stack[top+1u]=index+1u;
                    top+=2u;
                }
            } else {
                for (var j=0u;j<node.count;j++) {
                    if (tri_blocks(base+node.first+j,record,word,inv_det,ro,rd,o,d,limit)) {
                        mover_leave();
                        return true;
                    }
                }
            }
        }
    }
    for (var i=0u;i<frame.counts.y;i++) {
        if (shapes[i].d.x > 0.5) { continue; }
        var t=limit;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,o,d,limit); } else { t=primitive_hit(i,o,d,limit); }
        if (t<limit && t>0.0001) { return true; }
    }
    return false;
}
fn cast_tint(material: u32) -> vec3f {
    let transmission=clamp(material_row(material,2u).y,0.0,1.0)*(1.0-clamp(material_row(material,1u).x,0.0,1.0));
    if (transmission<=0.0) { return vec3f(0.0); }
    var passed=material_row(material,0u).xyz;
    let thickness=material_row(material,3u).x;
    let absorption=material_row(material,4u).w;
    if (thickness>0.0 && absorption>0.0) { passed=beer(thickness,passed,absorption); }
    return clamp(passed*transmission,vec3f(0.0),vec3f(1.0));
}
fn tri_passes(i: u32, record: u32, word: u32, inv_det: f32, o: vec3f, d: vec3f, wo: vec3f, wd: vec3f, limit: f32) -> vec3f {
    if (!tri_blocks(i,record,word,inv_det,o,d,wo,wd,limit)) { return vec3f(1.0); }
    return sqrt(cast_tint(record_material(instance_surfaces[record].info,triangles[i])));
}
fn shadow_passed(o: vec3f, d: vec3f, limit: f32) -> vec3f {
    trace_count(COUNT_SHADOW);
    var passed=vec3f(1.0);
    if (frame.counts.x>0u) {
        var stack: array<u32,64>;
        var top=1u;
        stack[0]=0u;
        var ro=o;
        var rd=d;
        var base=0u;
        var record=0u;
        var word=0u;
        var inv_det=1.0;
        while (top>0u) {
            top-=1u;
            let index=stack[top];
            if (index==ENTRYPOINT_SENTINEL) {
                ro=o;
                rd=d;
                base=0u;
                record=0u;
                word=0u;
                inv_det=1.0;
                mover_leave();
                continue;
            }
            let node=nodes[index];
            if (!box_hit(node,ro,rd,limit)) { continue; }
            if (node.pad.x==INSTANCE_LEAF) {
                if (TRACE_SPRITES && sprites_hidden && (instance_surfaces[bitcast<u32>(node.lo.w)].info.x&SPRITE)!=0u) { continue; }
                if (top+2u<=64u) {
                    stack[top]=ENTRYPOINT_SENTINEL;
                    stack[top+1u]=node.first;
                    top+=2u;
                    base=node.count;
                    record=bitcast<u32>(node.lo.w);
                    let info=instance_surfaces[record].info;
                    word=info.x;
                    inv_det=bitcast<f32>(info.w);
                    if (node.pad.y>0u) {
                        let m=nodes[node.pad.y];
                        ro=to_object(m,o,1.0);
                        rd=to_object(m,d,0.0);
                        if (TRACE_MOTION && record>0u && mover_moves(record-1u)) {
                            let moved=mover_enter(record-1u,o,d);
                            ro=moved.ro;
                            rd=moved.rd;
                            inv_det=moved.inv_det;
                        }
                    }
                }
            } else if (node.count==0u) {
                if (top+2u<=64u) {
                    stack[top]=node.first;
                    stack[top+1u]=index+1u;
                    top+=2u;
                }
            } else {
                for (var j=0u;j<node.count;j++) {
                    passed*=tri_passes(base+node.first+j,record,word,inv_det,ro,rd,o,d,limit);
                    if (all(passed<=vec3f(0.0))) {
                        mover_leave();
                        return vec3f(0.0);
                    }
                }
            }
        }
    }
    for (var i=0u;i<frame.counts.y;i++) {
        if (shapes[i].d.x > 0.5) { continue; }
        var t=limit;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,o,d,limit); } else { t=primitive_hit(i,o,d,limit); }
        if (t<limit && t>0.0001) { passed*=cast_tint(u32(shapes[i].c.w)); }
    }
    return passed;
}
fn offset_axis(position: f32, normal: f32) -> f32 {
    if (abs(position) < 1.0/32.0) { return position + normal/65536.0; }
    let bits=bitcast<i32>(position);
    let shift=i32(normal*256.0);
    return bitcast<f32>(bits + select(shift, -shift, position < 0.0));
}
fn offset_ray(p: vec3f, n: vec3f) -> vec3f {
    return vec3f(offset_axis(p.x,n.x), offset_axis(p.y,n.y), offset_axis(p.z,n.z));
}
fn spawn_point(p: vec3f, dir: vec3f, origin: vec3f, t: f32) -> vec3f {
    let err=4e-7*(max(abs(origin.x),max(abs(origin.y),abs(origin.z)))+t);
    return offset_ray(p, dir)+dir*err;
}
fn basis(n: vec3f) -> mat3x3f {
    let t=normalize(cross(select(vec3f(0.0,1.0,0.0),vec3f(1.0,0.0,0.0),abs(n.y)>0.9),n));
    return mat3x3f(t,cross(n,t),n);
}
const ROUGHNESS_FLOOR: f32 = 0.05;
fn surface(material: Material) -> Shaded {
    return Shaded(material.base_roughness.xyz,material.base_roughness.w,
        material.metal_spec_coat.x,material.metal_spec_coat.y,material.metal_spec_coat.z,material.metal_spec_coat.w,
        material.sheen_trans_ior_disp.x,material.sheen_trans_ior_disp.y,material.sheen_trans_ior_disp.z,material.sheen_trans_ior_disp.w,
        material.thick_sub_film.x,material.thick_sub_film.y,material.tint_absorption.xyz,material.tint_absorption.w,
        material.thick_sub_film.z,material.thick_sub_film.w,material.emission_film.w,material.emission_film.xyz,
        material.extension.x,vec3f(0.0,0.0,1.0));
}
fn detail_wobble(p: vec3f, scale: f32) -> vec3f {
    let q=p*scale;
    let e=0.35;
    return vec3f(value_noise3(q+vec3f(e,0.0,0.0),false)-value_noise3(q-vec3f(e,0.0,0.0),false),
        value_noise3(q+vec3f(0.0,e,0.0),false)-value_noise3(q-vec3f(0.0,e,0.0),false),
        value_noise3(q+vec3f(0.0,0.0,e),false)-value_noise3(q-vec3f(0.0,0.0,e),false))/(2.0*e);
}
fn detail_perturb(n: vec3f, g: vec3f) -> vec3f {
    return normalize(n-(g-n*dot(g,n)));
}
fn detail_fbm(p: vec3f) -> f32 {
    var sum=0.0;
    var amp=0.5;
    var q=p;
    for (var i=0;i<4;i++) {
        sum+=amp*value_noise3(q,true);
        q=q*2.03+vec3f(1.7,9.2,3.1);
        amp*=0.5;
    }
    return sum;
}
fn film_ramp(pa: vec4f, pb: vec4f, local: vec3f, uv: vec2f) -> f32 {
    var along=dot(local,pa.yzw);
    if (pa.x>=1.5) { along=dot(uv,pa.yz); }
    let span=pb.y-pb.x;
    var t=select(0.0,1.0,along>=pb.y);
    if (abs(span)>1e-12) { t=clamp((along-pb.x)/span,0.0,1.0); }
    return max(mix(pb.z,pb.w,t),0.0);
}
fn detail_layer(value: Shaded, normal_in: vec3f, layer: vec4f, pa: vec4f, pb: vec4f, pc: vec4f, pd: vec4f, position: vec3f, local: vec3f, uv: vec2f) -> DetailResult {
    var shade=value;
    var base=value.base;
    var roughness=value.roughness;
    var coat_roughness=value.clearcoat_roughness;
    var n=normal_in;
    let kind=u32(round(layer.x));
    let amp=layer.z;
    let seed=bitcast<u32>(layer.w);
    let p=position+vec3f(f32(seed)*0.001,f32(seed)*0.001,0.0);
    if (kind==1u) {
        let fibre=value_noise3(p*vec3f(pa.x,pa.y,pa.x),false)*0.6+value_noise3(p*pa.z,false)*0.4;
        let mottle=value_noise3(p*pa.w,false);
        base*=1.0+((fibre-0.5)*pb.z+(mottle-0.5)*pb.w)*amp;
        n=detail_perturb(n,detail_wobble(p,pb.x)*(pc.x*amp)+detail_wobble(p,pb.y)*(pc.y*amp));
    } else if (kind==2u) {
        n=detail_perturb(n,detail_wobble(p,pa.x)*(pa.z*amp)+detail_wobble(p,pa.y)*(pa.w*amp));
    } else if (kind==3u && pa.x>0.0) {
        let plank=floor(p.z*pd.x);
        let tone=hash3_xor(vec3i(i32(plank),7,3));
        let across=fract(p.z*pd.x);
        let seam=smoothstep(0.0,0.006,across)+smoothstep(0.0,0.006,1.0-across)-1.0;
        let warp=value_noise3(vec3f(p.x*1.4,plank*3.7,p.z*6.0),false)*0.9+value_noise3(vec3f(p.x*6.0,plank,p.z*20.0),false)*0.25;
        let rings=fract(fma(fma(warp,pa.z,p.z),pa.y,value_noise3(vec3f(p.x*0.8,plank,0.0),false)*pc.y));
        let figure=smoothstep(0.55,0.95,rings)*0.55+value_noise3(vec3f(p.x*pa.w,p.z*pb.x,plank),false)*0.25;
        let factor=(0.78+tone*pb.y)*(1.08-figure*pb.z)*(pb.w+clamp(seam,0.0,1.0)*pd.y);
        base*=mix(1.0,factor,clamp(amp,0.0,1.0));
        roughness=mix(roughness,sqrt(min(roughness*roughness*(0.85+figure*pc.x),1.0)),clamp(amp,0.0,1.0));
    } else if (kind==4u) {
        let broad=value_noise3(p*pa.x,false)*0.6+value_noise3(p*pa.y,false)*0.4;
        let fine=value_noise3(p*pa.z,false)*0.6+value_noise3(p*pa.w,false)*0.4;
        base*=mix(1.0,1.0+(broad-0.5)*pb.z+(fine-0.5)*pb.w,clamp(amp,0.0,1.0));
        n=detail_perturb(n,detail_wobble(p,pb.x)*(pc.x*amp)+detail_wobble(p,pb.y)*(pc.y*amp));
    } else if (kind==5u) {
        let grime=value_noise3(p*pa.x,false)*0.6+value_noise3(p*pa.y,false)*0.4;
        let wear=smoothstep(0.35,0.75,grime);
        base*=mix(vec3f(1.0),mix(vec3f(pa.z,pa.w,pb.x),vec3f(pb.y,pb.z,pb.w),wear),clamp(amp,0.0,1.0));
        roughness=mix(roughness,sqrt(min(roughness*roughness*mix(pc.x,pc.y,wear),1.0)),clamp(amp,0.0,1.0));
    } else if (kind==7u) {
        base*=mix(1.0,0.7+0.6*detail_fbm(vec3f(uv*12.0,2.0)),clamp(amp,0.0,1.0));
        n=layer_relief(n,uv,position,amp,false,0.0);
    } else if (kind==8u) {
        base*=mix(1.0,0.75+0.5*detail_fbm(p*300.0),clamp(amp,0.0,1.0));
        n=layer_relief(n,uv,position,amp,true,0.0);
    } else if (kind==14u) {
        shade.thin_film=max(shade.thin_film+amp*(flow(uv*layer.y,pa.x)-0.5)*2.0,0.0);
    } else if (kind==21u && pa.x>0.5) {
        shade.thin_film=mix(shade.thin_film,film_ramp(pa,pb,local,uv),clamp(amp,0.0,1.0));
    } else if (kind==15u) {
        base*=mix(1.0,1.0+(value_noise3(p*layer.y,false)-0.5)*2.0,clamp(amp,0.0,1.0));
    } else if (kind==16u) {
        base*=mix(1.0,1.0+(detail_fbm(p*layer.y)-0.5)*2.0,clamp(amp,0.0,1.0));
    } else if (kind==17u) {
        let line=1.0-smoothstep(0.0,0.045,abs(value_noise3(vec3f(p.x*pd.x+f32(seed),p.y*pa.x,p.z*pd.x),false)-0.5));
        let smudge=smoothstep(0.52,0.78,value_noise3(p*pa.y,false)*0.65+value_noise3(p*pa.z,false)*0.35);
        let cell=vec3i(floor(p*pa.w));
        let speck=select(0.0,1.0,hash3_xor(vec3i(cell.x,cell.y,cell.z^i32(seed)))>0.9985);
        let breakup=clamp(smudge*0.55+line*0.35+speck,0.0,1.0)*min(amp,1.0);
        if (value.clearcoat>0.0) {
            coat_roughness=sqrt(mix(coat_roughness*coat_roughness,min(coat_roughness*coat_roughness*4.0+0.02,0.35),breakup));
        } else {
            roughness=sqrt(mix(roughness*roughness,min(roughness*roughness*3.0+0.03,0.5),breakup));
        }
        base=mix(base,base*0.9+vec3f(0.06),speck*min(amp,1.0));
    } else if (kind==19u) {
        let crack=pavement_crack(p,n,pa,pb,pc,seed,0.0)*clamp(amp,0.0,1.0);
        base*=1.0-pb.z*crack;
        roughness=mix(roughness,pb.w,crack);
    } else if (kind==20u) {
        let worn=pavement_track(uv,pa,pb,seed,0.0)*clamp(amp,0.0,1.0);
        base*=mix(1.0,pc.x,worn);
        roughness=mix(roughness,roughness*pc.y,worn);
    } else if (kind==18u) {
        n=detail_perturb(n,detail_wobble(p,pa.x)*amp+detail_wobble(p,pa.y)*(amp*pa.w)+detail_wobble(p,pa.z)*(amp*pb.x));
    }
    shade.base=base;
    shade.roughness=roughness;
    shade.clearcoat_roughness=coat_roughness;
    return DetailResult(shade,n);
}
fn leaf_hash(p: vec2f) -> f32 {
    let q=fract(p*vec2f(123.34,456.21));
    return fract((q.x+45.32)*(q.y+45.32)*34.23+q.x*13.7);
}
fn leaf_noise(p: vec2f) -> f32 {
    let i=floor(p);
    let f=fract(p);
    let u=f*f*(3.0-2.0*f);
    return mix(mix(leaf_hash(i),leaf_hash(i+vec2f(1.0,0.0)),u.x),
        mix(leaf_hash(i+vec2f(0.0,1.0)),leaf_hash(i+vec2f(1.0,1.0)),u.x),u.y);
}
fn leaf_fbm(p: vec2f) -> f32 {
    return leaf_noise(p)*0.55+leaf_noise(p*2.13+3.1)*0.28+leaf_noise(p*4.37+7.7)*0.17;
}
fn canopy(p: vec3f) -> f32 {
    let sun=normalize(frame.sun_dir.xyz);
    let e1=normalize(cross(sun,vec3f(0.0,1.0,0.0)));
    let e2=cross(e1,sun);
    let q=vec2f(dot(p-effects().metal_spec_coat.xyz,e1),dot(p-effects().metal_spec_coat.xyz,e2));
    let t=effects().base_roughness.x;
    let gust=sin(t*0.21)*0.6+sin(t*0.37+1.3)*0.4;
    let sway=vec2f(sin(t*0.9+q.y*9.0)*0.005+gust*0.011,cos(t*0.7+q.x*7.0)*0.004+gust*0.004);
    let shifted=q+sway;
    let branch=smoothstep(-0.55,0.15,shifted.x*0.8+shifted.y*0.6);
    let clusters=smoothstep(0.3,0.62,leaf_fbm(shifted*3.4+vec2f(5.0,2.0)));
    let leaves=smoothstep(0.36,0.56,leaf_fbm(shifted*17.0+vec2f(sin(t*1.3)*0.08,cos(t*1.1)*0.08)));
    return mix(1.0,1.0-leaves*clusters*branch*0.88,effects().base_roughness.y);
}
fn steam_fbm(p: vec3f) -> f32 {
    var sum=0.0;
    var amp=0.55;
    var q=p;
    for (var k=0;k<4;k++) {
        sum+=value_noise3(q,false)*amp;
        q=q*2.07+vec3f(1.7,9.2,3.1);
        amp*=0.5;
    }
    return sum;
}
fn steam_density(p: vec3f) -> f32 {
    let t=effects().base_roughness.w;
    let source=effects().sheen_trans_ior_disp.xyz;
    let rise=p.y-source.y;
    if (rise<0.0 || rise>STEAM_FADE_OUT_END) { return 0.0; }
    let drift=vec3f(STEAM_DRIFT_X_QUAD*rise*rise+STEAM_DRIFT_X_LIN*rise,0.0,STEAM_DRIFT_Z_QUAD*rise*rise+STEAM_DRIFT_Z_LIN*rise);
    let sway=vec3f(sin(rise*STEAM_SWAY_X_WAVE-t*STEAM_SWAY_X_RATE)*STEAM_SWAY_X_AMP*(STEAM_SWAY_X_BASE+rise*STEAM_SWAY_X_GROWTH),0.0,
        cos(rise*STEAM_SWAY_Z_WAVE-t*STEAM_SWAY_Z_RATE)*STEAM_SWAY_Z_AMP*(STEAM_SWAY_Z_BASE+rise*STEAM_SWAY_Z_GROWTH));
    let axis=source+vec3f(0.0,rise,0.0)+drift+sway;
    let width=effects().sheen_trans_ior_disp.w*(STEAM_SPREAD_BASE+rise*STEAM_SPREAD_GROWTH);
    let r=length((p-axis).xz);
    let core=exp(-r*r/(width*width));
    let fade=smoothstep(0.0,STEAM_FADE_IN,rise)*(1.0-smoothstep(STEAM_FADE_OUT_START,STEAM_FADE_OUT_END,rise));
    if (core*fade<0.002) { return 0.0; }
    let q=vec3f(p.x*STEAM_GRAIN_H,(p.y-t*STEAM_LIFT_RATE)*STEAM_GRAIN_V,p.z*STEAM_GRAIN_H);
    let warp=vec3f(steam_fbm(q*0.5+vec3f(t*STEAM_WARP_RATE,0.0,0.0)),0.0,
        steam_fbm(q*0.5+vec3f(3.1,0.0,t*STEAM_WARP_RATE)))*STEAM_WARP_AMP;
    let turbulence=smoothstep(STEAM_THRESHOLD_LOW,STEAM_THRESHOLD_HIGH,steam_fbm(q+warp+vec3f(0.0,-t*STEAM_CHURN,0.0)));
    return core*turbulence*fade*effects().thick_sub_film.w;
}
fn steam_composite(origin: vec3f, direction: vec3f, surface_depth: f32, color: vec3f) -> vec3f {
    if (effects().thick_sub_film.w<=0.0) { return color; }
    let inv=1.0/select(direction,sign(direction)*vec3f(1e-8),abs(direction)<vec3f(1e-8));
    let t0=(effects().thick_sub_film.xyz-origin)*inv;
    let t1=(effects().tint_absorption.xyz-origin)*inv;
    let enter=max(max(min(t0.x,t1.x),min(t0.y,t1.y)),max(min(t0.z,t1.z),0.0));
    let leave=min(min(max(t0.x,t1.x),max(t0.y,t1.y)),min(max(t0.z,t1.z),surface_depth));
    if (enter>=leave) { return color; }
    let dt=(leave-enter)/20.0;
    var transmittance=1.0;
    var scattered=vec3f(0.0);
    let sun=normalize(frame.sun_dir.xyz);
    let g=effects().metal_spec_coat.w;
    let phase=(1.0-g*g)/pow(1.0+g*g-2.0*g*dot(direction,sun),1.5);
    for (var k=0u;k<20u;k++) {
        let p=origin+direction*(enter+(f32(k)+0.5)*dt);
        let density=steam_density(p);
        if (density>1e-4) {
            let extinction=exp(-density*90.0*dt);
            let open=select(1.0,0.0,scene(spawn_point(p,sun,p,0.0),sun,1e20,true).t<1e19);
            let lit=frame.sun_color.xyz*frame.sun_color.w*open*phase;
            scattered+=transmittance*(lit+vec3f(effects().tint_absorption.w))*(1.0-extinction)*0.85;
            transmittance*=extinction;
        }
    }
    return scattered+transmittance*color;
}
fn texels_at(offset: u32, width: u32, height: u32, uv: vec2f) -> vec4f {
    let size=vec2f(f32(width),f32(height));
    let p=clamp(uv*size-vec2f(0.5),vec2f(0.0),size-vec2f(1.0));
    let lo=vec2u(floor(p));
    let hi=min(lo+vec2u(1u),vec2u(width-1u,height-1u));
    let f=fract(p);
    let a=content_row(offset+lo.y*width+lo.x);
    let b=content_row(offset+lo.y*width+hi.x);
    let c=content_row(offset+hi.y*width+lo.x);
    let d=content_row(offset+hi.y*width+hi.x);
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn srgb_channel(byte: u32) -> f32 {
    return bitcast<f32>(content_words[byte/4u][byte&3u]);
}
fn content_row(at: u32) -> vec4f {
    return bitcast<vec4f>(content_words[at]);
}
fn byte_texel(offset: u32, width: u32, x: u32, y: u32, srgb: bool) -> vec4f {
    let at=y*width+x;
    let packed=content_words[offset+at/4u][at&3u];
    let bytes=vec4u(packed&255u,(packed>>8u)&255u,(packed>>16u)&255u,packed>>24u);
    if (srgb) { return vec4f(srgb_channel(bytes.x),srgb_channel(bytes.y),srgb_channel(bytes.z),f32(bytes.w)/255.0); }
    return vec4f(bytes)/255.0;
}
fn bytes_at(offset: u32, width: u32, height: u32, uv: vec2f, srgb: bool) -> vec4f {
    let size=vec2f(f32(width),f32(height));
    let p=clamp(uv*size-vec2f(0.5),vec2f(0.0),size-vec2f(1.0));
    let lo=vec2u(floor(p));
    let hi=min(lo+vec2u(1u),vec2u(width-1u,height-1u));
    let f=fract(p);
    let a=byte_texel(offset,width,lo.x,lo.y,srgb);
    let b=byte_texel(offset,width,hi.x,lo.y,srgb);
    let c=byte_texel(offset,width,lo.x,hi.y,srgb);
    let d=byte_texel(offset,width,hi.x,hi.y,srgb);
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn image_at(info: ContentInfo, uv: vec2f) -> vec4f {
    if (info.kind>=2u) { return bytes_at(info.offset,info.width,info.height,uv,info.kind==3u); }
    return texels_at(info.offset,info.width,info.height,uv);
}
fn atlas_texel(offset: u32, width: u32, x: u32, y: u32) -> vec3f {
    let at=y*width+x;
    let packed=u32(content_row(offset+at/4u)[at&3u]);
    return vec3f(f32(packed&255u),f32((packed>>8u)&255u),f32(packed>>16u))/255.0;
}
fn atlas_at(offset: u32, width: u32, height: u32, uv: vec2f) -> vec3f {
    let p=uv*vec2f(f32(width),f32(height))-vec2f(0.5);
    let start=floor(p);
    let f=p-start;
    let lo=vec2u(clamp(start,vec2f(0.0),vec2f(f32(width-1u),f32(height-1u))));
    let hi=vec2u(clamp(start+vec2f(1.0),vec2f(0.0),vec2f(f32(width-1u),f32(height-1u))));
    let a=atlas_texel(offset,width,lo.x,lo.y);
    let b=atlas_texel(offset,width,hi.x,lo.y);
    let c=atlas_texel(offset,width,lo.x,hi.y);
    let d=atlas_texel(offset,width,hi.x,hi.y);
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn text_at(offset: u32, uv: vec2f) -> vec4f {
    let grid=content_row(offset);
    let shape=vec4u(content_row(offset+1u));
    let atlas=vec4u(content_row(offset+2u));
    let picture=vec4u(content_row(offset+3u));
    var color=vec4f(0.0);
    if (picture.x>0u) { color=texels_at(offset+picture.z,picture.x,picture.y,uv); }
    let cell=floor((uv-grid.xy)*grid.zw);
    if (cell.x<0.0 || cell.y<0.0 || cell.x>=f32(shape.x) || cell.y>=f32(shape.y)) { return color; }
    let index=u32(cell.y)*shape.x+u32(cell.x);
    let pair=content_row(offset+4u+index/2u);
    let entry=vec2u(select(pair.xy,pair.zw,(index&1u)==1u));
    let size=vec2f(f32(atlas.x),f32(atlas.y));
    let half=vec2f(0.5)/size;
    for (var k=0u;k<entry.y;k++) {
        let at=entry.x+k;
        let glyph=offset+shape.z+u32(content_row(offset+shape.w+at/4u)[at&3u])*5u;
        let m=content_row(glyph);
        let t=content_row(glyph+1u);
        let clip=content_row(glyph+2u);
        let box=content_row(glyph+3u);
        let ink=content_row(glyph+4u);
        let f=vec2f(m.x*uv.x+m.y*uv.y+t.x,m.z*uv.x+m.w*uv.y+t.y);
        if (any(f<max(clip.xy,vec2f(0.0))) || any(f>min(clip.zw,vec2f(1.0)))) { continue; }
        let low=min(box.xy+half,box.zw);
        let high=max(box.zw-half,low);
        let sampled=atlas_at(offset+atlas.z,atlas.x,atlas.y,clamp(box.xy+(box.zw-box.xy)*f,low,high));
        var coverage=sampled.x;
        if (atlas.w==3u) {
            let median=max(min(sampled.x,sampled.y),min(max(sampled.x,sampled.y),sampled.z));
            coverage=select(0.0,1.0,median>=0.5);
        }
        let opacity=coverage*ink.a;
        color=vec4f(ink.rgb*opacity+color.rgb*(1.0-opacity),opacity+color.a*(1.0-opacity));
    }
    return color;
}
fn content_at(slot: i32, uv: vec2f) -> vec4f {
    if (slot < 0) { return vec4f(0.0); }
    let info=content_info[u32(slot)];
    if (info.width==0u || info.height==0u) { return vec4f(0.0); }
    if (info.kind==1u) { return text_at(info.offset,uv); }
    return image_at(info,uv);
}
const CF_WEIGHTS = array<vec3f, 81>(
    vec3f(5.039987e-05, 0.0, 0.00035778867),
    vec3f(6.775547e-05, 0.0, 0.0005768482),
    vec3f(0.00013374595, 0.0, 0.001105031),
    vec3f(0.000244155, -8.0429134e-05, 0.001990094),
    vec3f(0.0004620315, -0.00031065758, 0.0037327078),
    vec3f(0.00075178174, -0.00079914776, 0.0060585286),
    vec3f(0.0013901147, -0.0016784245, 0.01140122),
    vec3f(0.0024506692, -0.0030452313, 0.020409657),
    vec3f(0.0041841697, -0.00489408, 0.035484146),
    vec3f(0.006491386, -0.0070713423, 0.05709876),
    vec3f(0.008225769, -0.009268977, 0.07611217),
    vec3f(0.00893323, -0.0110506825, 0.08910742),
    vec3f(0.008651618, -0.011844616, 0.09585401),
    vec3f(0.0075308583, -0.0111710355, 0.09771433),
    vec3f(0.00574333, -0.009713182, 0.0970225),
    vec3f(0.0034815131, -0.0075853216, 0.09534322),
    vec3f(0.00069844024, -0.0048573813, 0.091060266),
    vec3f(-0.0036886954, -0.0017018986, 0.083120264),
    vec3f(-0.0063273897, 0.0017186198, 0.06969027),
    vec3f(-0.008724144, 0.005763426, 0.05593676),
    vec3f(-0.011347991, 0.009961927, 0.04306675),
    vec3f(-0.014388451, 0.014161423, 0.031928584),
    vec3f(-0.017747117, 0.018677758, 0.023228362),
    vec3f(-0.021200482, 0.023959707, 0.016556388),
    vec3f(-0.024560854, 0.030223139, 0.011426527),
    vec3f(-0.027776832, 0.03802389, 0.00728438),
    vec3f(-0.030845057, 0.046441782, 0.0033540253),
    vec3f(-0.03346058, 0.055137716, 0.0),
    vec3f(-0.03508027, 0.06285677, -0.0025753065),
    vec3f(-0.035013195, 0.068301246, -0.004734569),
    vec3f(-0.032630157, 0.071965545, -0.006312869),
    vec3f(-0.027983842, 0.07395785, -0.00737929),
    vec3f(-0.02177047, 0.07446333, -0.008106243),
    vec3f(-0.0140442625, 0.07356939, -0.008540686),
    vec3f(-0.0049150633, 0.07139566, -0.008724192),
    vec3f(0.0046487506, 0.06808836, -0.008696091),
    vec3f(0.0153741175, 0.063680954, -0.008493348),
    vec3f(0.026958833, 0.058147695, -0.008150433),
    vec3f(0.03911687, 0.051680956, -0.007698217),
    vec3f(0.05145111, 0.04444711, -0.0071432022),
    vec3f(0.063475594, 0.036714625, -0.006503886),
    vec3f(0.07455033, 0.028763412, -0.005807879),
    vec3f(0.08411886, 0.020992132, -0.0050818375),
    vec3f(0.091669224, 0.0137893725, -0.00435086),
    vec3f(0.09618923, 0.007610871, -0.0036377995),
    vec3f(0.097940184, 0.002461353, -0.0029763517),
    vec3f(0.09633994, -0.0015017615, -0.0023928043),
    vec3f(0.0919427, -0.004451467, -0.0018944021),
    vec3f(0.08495084, -0.0062283436, -0.0014817494),
    vec3f(0.07555293, -0.0070042815, -0.0011499332),
    vec3f(0.06515909, -0.006994329, -0.00089005305),
    vec3f(0.05535661, -0.0064279693, -0.00069090264),
    vec3f(0.046014983, -0.005523557, -0.0005405573),
    vec3f(0.037232462, -0.004469, -0.00042767118),
    vec3f(0.029350668, -0.0034102271, -0.0003423706),
    vec3f(0.022698376, -0.0024474342, -0.00027671963),
    vec3f(0.017146055, -0.001637843, -0.00022480598),
    vec3f(0.012616245, -0.0010029421, -0.00018254654),
    vec3f(0.00910757, -0.0005379737, -0.00014732922),
    vec3f(0.0066325837, -0.00022167167, -0.00011759913),
    vec3f(0.004884865, -2.4797864e-05, -9.247229e-05),
    vec3f(0.0034370946, 0.0, -7.142631e-05),
    vec3f(0.0023721277, 0.0, -5.40864e-05),
    vec3f(0.0016515272, 0.0, -4.010171e-05),
    vec3f(0.0011923393, 0.0, -2.9092696e-05),
    vec3f(0.00084797293, 0.0, -2.064557e-05),
    vec3f(0.000605779, 0.0, -1.4331459e-05),
    vec3f(0.0004273203, 0.0, -9.733269e-06),
    vec3f(0.00030592916, 0.0, -6.469539e-06),
    vec3f(0.00021032631, 0.0, -4.210301e-06),
    vec3f(0.00014668911, 0.0, -2.683982e-06),
    vec3f(0.0001021725, 0.0, -1.6768181e-06),
    vec3f(7.628618e-05, 0.0, -1.027186e-06),
    vec3f(5.108625e-05, 0.0, -6.1728224e-07),
    vec3f(3.186762e-05, 0.0, -3.6408065e-07),
    vec3f(1.9218627e-05, 0.0, -2.1085688e-07),
    vec3f(1.9218627e-05, 0.0, -1.1996055e-07),
    vec3f(1.2648995e-05, 0.0, 2.5954927e-07),
    vec3f(1.2648995e-05, 0.0, 2.5954927e-07),
    vec3f(1.2648995e-05, 0.0, 2.5954927e-07),
    vec3f(-5.4965028e-08, 0.0, -1.0592136e-08),
);
fn cf_fresnel(cos_i: f32, na: f32, nb: f32) -> vec3f {
    let sin2=(na/nb)*(na/nb)*(1.0-cos_i*cos_i);
    if (sin2>=1.0) { return vec3f(1.0,1.0,0.0); }
    let cos_t=sqrt(1.0-sin2);
    let rs=(na*cos_i-nb*cos_t)/(na*cos_i+nb*cos_t);
    let rp=(nb*cos_i-na*cos_t)/(nb*cos_i+na*cos_t);
    return vec3f(rs,rp,cos_t);
}
fn cf_film(cosine: f32, thickness: f32, n1: f32, nf: f32, n3: f32) -> vec3f {
    let first=cf_fresnel(clamp(cosine,0.0,1.0),n1,nf);
    if (first.z<=0.0) { return vec3f(1.0); }
    let second=cf_fresnel(first.z,nf,n3);
    if (second.z<=0.0) { return vec3f(1.0); }
    let r12=first.xy;
    let r23=second.xy;
    let a=r12*r12+r23*r23;
    let b=2.0*r12*r23;
    let e=1.0+r12*r12*r23*r23;
    let path=4.0*PI*nf*thickness*first.z;
    var weights=CF_WEIGHTS;
    var total=vec3f(0.0);
    for (var k=0u;k<81u;k++) {
        let c=cos(path/(380.0+5.0*f32(k)));
        let reflect=0.5*((a.x+b.x*c)/(e.x+b.x*c)+(a.y+b.y*c)/(e.y+b.y*c));
        total+=weights[k]*reflect;
    }
    return clamp(total,vec3f(0.0),vec3f(1.0));
}
fn film_active(s: Shaded) -> bool {
    return s.thin_film>0.0 && s.thin_film_amount>0.0;
}
fn cf_surface(cosine: f32, thickness: f32, film_ior: f32, ior: f32, entering: bool) -> vec3f {
    let nf=max(film_ior,1.01);
    if (entering) { return cf_film(cosine,thickness,1.0,nf,ior); }
    return cf_film(cosine,thickness,1.0,nf/ior,1.0/ior);
}
fn transmissive_fresnel(s: Shaded, cosine: f32, ratio: f32, entering: bool, ior: f32) -> vec3f {
    let plain=vec3f(fresnel_dielectric(cosine,ratio));
    if (!film_active(s)) { return plain; }
    let filmed=cf_surface(cosine,s.thin_film,s.thin_film_ior,ior,entering);
    return mix(plain,filmed,clamp(s.thin_film_amount,0.0,1.0));
}
const IOR_SPLIT: f32 = 1e-4;
fn channel_mask(channel: u32) -> vec3f {
    return vec3f(select(0.0,1.0,channel==0u),select(0.0,1.0,channel==1u),select(0.0,1.0,channel==2u));
}
fn channel_ior(s: Shaded, channel: u32) -> f32 {
    if (channel>2u) { return max(s.ior,1.01); }
    return max(dispersed_ior(s.ior,s.dispersion)[channel],1.01);
}
fn glossy_share(s: Shaded) -> f32 {
    return 1.0-clamp(s.transmission,0.0,1.0)*(1.0-clamp(s.metalness,0.0,1.0));
}
fn sample_bsdf(s: Shaded, wo: vec3f, entering: bool, carried: u32, state: ptr<function,u32>, u: vec4f) -> LightSample {
    let transmission=clamp(s.transmission,0.0,1.0)*(1.0-clamp(s.metalness,0.0,1.0));
    if (rnd(state,u.w)<transmission) {
        if (TRACE_THIN && traced_thin) {
            let ior=max(s.ior,1.01);
            let live=select(channel_mask(carried),vec3f(1.0),carried>2u);
            let face=transmissive_fresnel(s,max(wo.z,0.0),ior,true,ior);
            let slab=vec3f(thin_dielectric(face.x).x,thin_dielectric(face.y).x,thin_dielectric(face.z).x)*live;
            let chance=dot(slab,vec3f(1.0))/dot(live,vec3f(1.0));
            if (rnd(state,u.x)<chance) {
                return LightSample(reflect(-wo,vec3f(0.0,0.0,1.0)),-1.0,carried,slab/chance,1u);
            }
            return LightSample(-wo,-2.0,carried,(live-slab)/(1.0-chance),2u);
        }
        let iors=dispersed_ior(s.ior,s.dispersion);
        var channel=carried;
        var weight=vec3f(1.0);
        if (channel>2u && max(abs(iors.x-iors.y),abs(iors.z-iors.y))>IOR_SPLIT) {
            channel=min(u32(rnd(state,u.z)*3.0),2u);
            weight=channel_mask(channel)*3.0;
        }
        let ior=channel_ior(s,channel);
        let ratio=select(1.0/ior,ior,entering);
        let live=select(channel_mask(channel),vec3f(1.0),channel>2u);
        let fresnel=transmissive_fresnel(s,max(wo.z,0.0),ratio,entering,ior)*live;
        let chance=dot(fresnel,vec3f(1.0))/dot(live,vec3f(1.0));
        let incident=-wo;
        let mirrored=reflect(incident,vec3f(0.0,0.0,1.0));
        if (rnd(state,u.x)<chance) {
            return LightSample(mirrored,-1.0,channel,weight*fresnel/chance,1u);
        }
        let refracted=refract_dir(incident,vec3f(0.0,0.0,1.0),1.0/ratio);
        if (refracted.w>0.0) { return LightSample(normalize(refracted.xyz),-2.0,channel,weight*(live-fresnel)/(1.0-chance),2u); }
        return LightSample(mirrored,-1.0,channel,weight*live,1u);
    }
    let coat_weight=clamp(s.clearcoat*(0.12+0.88*(0.04+0.96*pow(1.0-max(wo.z,0.0),5.0))),0.0,0.9);
    let spec_weight=(1.0-coat_weight)*clamp(s.metalness+s.specular,0.0,0.9);
    let pick=rnd(state,u.z);
    if (pick<coat_weight+spec_weight) {
        let coat=pick<coat_weight;
        let alpha=alpha_rough(select(s.roughness,s.clearcoat_roughness,coat));
        let u1=rnd(state,u.x);
        let u2=rnd(state,u.y);
        let phi=2.0*PI*u1;
        let cos_theta=sqrt((1.0-u2)/(1.0+(alpha*alpha-1.0)*u2));
        var h=vec3f(cos(phi)*sqrt(max(0.0,1.0-cos_theta*cos_theta)),sin(phi)*sqrt(max(0.0,1.0-cos_theta*cos_theta)),cos_theta);
        if (TRACE_LOBES && !coat && lobes_anisotropy_on()) { h=anisotropic_half(s.roughness,u1,u2); }
        if (TRACE_FLAKES && !coat && traced_flaked) { h=basis(traced_flake)*h; }
        let wi=reflect(-wo,h);
        return LightSample(wi,bsdf_pdf(s,wo,wi),3u,vec3f(1.0),1u);
    }
    let u1=rnd(state,u.x);
    let u2=rnd(state,u.y);
    let phi=2.0*PI*u1;
    let r=sqrt(u2);
    let wi=vec3f(cos(phi)*r,sin(phi)*r,sqrt(1.0-u2));
    return LightSample(wi,bsdf_pdf(s,wo,wi),3u,vec3f(1.0),0u);
}
fn bsdf_pdf(s: Shaded, wo: vec3f, wi: vec3f) -> f32 {
    if (wi.z<=0.0) { return 0.0; }
    let trans=clamp(s.transmission,0.0,1.0)*(1.0-clamp(s.metalness,0.0,1.0));
    let coat=clamp(s.clearcoat*(0.12+0.88*(0.04+0.96*pow(1.0-max(wo.z,0.0),5.0))),0.0,0.9);
    let spec=(1.0-coat)*clamp(s.metalness+s.specular,0.0,0.9);
    let h=normalize(wo+wi);
    let a=alpha_rough(s.roughness);
    let ca=alpha_rough(s.clearcoat_roughness);
    let denom=max(4.0*abs(dot(wi,h)),1e-8);
    var base_pdf=ggx_d(max(h.z,0.0),a*a)*max(h.z,0.0)/denom;
    if (TRACE_LOBES && lobes_anisotropy_on()) { base_pdf=anisotropic_density(s.roughness,h)*max(h.z,0.0)/denom; }
    if (TRACE_FLAKES && traced_flaked) { base_pdf=ggx_d(max(dot(h,traced_flake),0.0),a*a)*max(dot(h,traced_flake),0.0)/denom; }
    let coat_pdf=ggx_d(max(h.z,0.0),ca*ca)*max(h.z,0.0)/denom;
    return (1.0-trans)*(coat*coat_pdf+spec*base_pdf+(1.0-coat-spec)*wi.z/PI);
}
fn trace_lights_start() {}
fn emitter_weight(hit: Hit, p: vec3f, previous_point: vec3f, previous_pdf: f32) -> f32 {
    return 1.0;
}
fn light_hits(o: vec3f, d: vec3f, limit: f32, previous_point: vec3f, previous_pdf: f32) -> vec3f {
    return vec3f(0.0);
}
fn emitters_looped() -> u32 {
    return 0u;
}
fn light_position(i: u32) -> vec3f {
    return vec3f(0.0);
}
fn light_slots() -> u32 {
    return 0u;
}
fn sampled_emitter(hit: Hit) -> bool {
    return false;
}
fn direct_light(slot: u32, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>, u: vec4f) -> Candidate {
    return Candidate(vec3f(0.0,0.0,1.0),vec3f(0.0),0.0,0.0,vec3f(0.0,0.0,1.0),-1.0,0u);
}
fn trace_lights_end() {}
fn catcher_visible(origin: vec3f, direction: vec3f, limit: f32) -> f32 {
    if (TRANSMISSIVE_SHADOWS) { return dot(shadow_passed(origin,direction,limit),vec3f(0.2126,0.7152,0.0722)); }
    return select(1.0,0.0,occluded(origin,direction,limit));
}
fn catcher_shadow(p: vec3f, n: vec3f, o: vec3f, t: f32, state: ptr<function,u32>) -> f32 {
    let transform=basis(n);
    let origin=spawn_point(p,n,o,t);
    var open=0.0;
    var seen=0.0;
    for (var k=0u;k<2u+light_slots();k++) {
        let u=sobol(light_dim(k));
        var direction=vec3f(0.0,0.0,1.0);
        var strength=0.0;
        var limit=1e19;
        var shape=0u;
        if (k==0u) {
            if (frame.sun_color.w<=0.0) { continue; }
            direction=normalize(frame.sun_dir.xyz);
            if (frame.sun_dir.w<1.0) {
                let height=1.0-rnd(state,u.x)*(1.0-frame.sun_dir.w);
                let radius=sqrt(max(0.0,1.0-height*height));
                let phi=2.0*PI*rnd(state,u.y);
                direction=normalize(basis(direction)*vec3f(radius*cos(phi),radius*sin(phi),height));
            }
            strength=dot(frame.sun_color.xyz,vec3f(0.2126,0.7152,0.0722))*frame.sun_color.w*max(dot(direction,n),0.0);
        } else if (k==1u) {
            if (sky_cdf.width==0u) { continue; }
            for (var j=0u;j<4u;j++) {
                let light=sky_sample(vec2f(random(state),random(state)));
                if (!(light.pdf>0.0)) { continue; }
                let part=dot(sky_surface_radiance(light.direction),vec3f(0.2126,0.7152,0.0722))*max(dot(light.direction,n),0.0)/light.pdf*0.25;
                if (!(part>0.0)) { continue; }
                open+=part;
                seen+=part*catcher_visible(origin,light.direction,1e19);
            }
            continue;
        } else {
            let picked=direct_light(k-2u,transform,p,origin,n,state,u);
            if (picked.limit<0.0) { continue; }
            direction=picked.direction;
            limit=picked.limit;
            shape=picked.shape;
            strength=dot(picked.scale,vec3f(0.2126,0.7152,0.0722))*max(picked.wi.z,0.0)/select(1.0,picked.pdf,picked.pdf>0.0);
        }
        if (!(strength>0.0)) { continue; }
        if (shape>0u) {
            let i=shape-1u;
            var hit_t=1e20;
            if (u32(shapes[i].a.w)==5u) { hit_t=union_hit(i,origin,direction,1e20); } else { hit_t=primitive_hit(i,origin,direction,1e20); }
            if (hit_t<=0.0001 || hit_t>=1e19) { continue; }
            limit=hit_t*0.9999-0.0002;
        }
        open+=strength;
        if (limit>0.0) { seen+=strength*catcher_visible(origin,direction,limit); } else { seen+=strength; }
    }
    if (open<=0.0) { return 0.0; }
    return clamp(1.0-seen/open,0.0,1.0);
}
const INDIRECT_LUMA: vec3f = vec3f(0.2126,0.7152,0.0722);
fn indirect_capped(value: vec3f, deep: bool) -> vec3f {
    if (!(CLAMP_INDIRECT>0.0) || !deep) { return value; }
    let luma=dot(value,INDIRECT_LUMA);
    if (!(luma>CLAMP_INDIRECT)) { return value; }
    return value*(CLAMP_INDIRECT/luma);
}
const FOG_DIM: u32 = 0x100000u;
const FOG_LIGHT_FLOOR: f32 = 0.25;
const FOG_PDF_CUTOFF: f32 = 1e-8;
fn fog_density(y: f32) -> f32 {
    return frame.fog.z*exp(min(-frame.fog.y*(y-frame.fog.x),30.0));
}
fn fog_span(o: vec3f, d: vec3f, limit: f32) -> vec2f {
    let q=o-frame.origin.xyz;
    let b=dot(q,d);
    let h=b*b-(dot(q,q)-frame.fog.w*frame.fog.w);
    if (!(h>0.0) || !(frame.fog.z>0.0)) { return vec2f(1.0,0.0); }
    let s=sqrt(h);
    return vec2f(max(-b-s,0.0),min(-b+s,limit));
}
fn fog_depth(o: vec3f, d: vec3f, lo: f32, hi: f32) -> f32 {
    let length=hi-lo;
    if (!(length>0.0)) { return 0.0; }
    let density=fog_density(o.y+d.y*lo);
    let x=frame.fog.y*d.y*length;
    if (abs(x)>1e-3) { return density*length*(1.0-exp(-x))/x; }
    return density*length*(1.0-x*(0.5-x/6.0));
}
fn fog_transmit(o: vec3f, d: vec3f, limit: f32) -> f32 {
    let span=fog_span(o,d,limit);
    if (span.y<=span.x) { return 1.0; }
    return exp(-fog_depth(o,d,span.x,span.y));
}
fn fog_distance(o: vec3f, d: vec3f, lo: f32, hi: f32, depth: f32) -> f32 {
    let density=fog_density(o.y+d.y*lo);
    if (!(density>0.0)) { return hi; }
    let c=frame.fog.y*d.y;
    let x=depth*c/density;
    var s=depth/density;
    if (abs(x)>1e-3) { s=-log(max(1.0-x,1e-30))/c; } else { s*=1.0+x*(0.5+x/3.0); }
    return clamp(lo+s,lo,hi);
}
fn fog_distance_pdf(o: vec3f, d: vec3f, span: vec2f, attenuation: f32, t: f32) -> f32 {
    return fog_density(o.y+d.y*t)*exp(-fog_depth(o,d,span.x,t))/attenuation;
}
fn fog_equiangular(o: vec3f, d: vec3f, light: vec3f, span: vec2f, u: f32) -> vec2f {
    let delta=dot(light-o,d);
    let D=length(light-o-d*delta);
    if (D==0.0) { return vec2f(0.0); }
    let theta_a=atan2(span.x-delta,D);
    let theta_b=atan2(span.y-delta,D);
    let theta_d=theta_b-theta_a;
    if (theta_d<1e-6) { return vec2f(mix(span.x,span.y,u),1.0/max(span.y-span.x,1e-30)); }
    let t=D*tan(u*theta_b+(1.0-u)*theta_a);
    return vec2f(clamp(delta+t,span.x,span.y),D/(theta_d*(D*D+t*t)));
}
fn fog_equiangular_pdf(o: vec3f, d: vec3f, light: vec3f, span: vec2f, t: f32) -> f32 {
    let delta=dot(light-o,d);
    let D=length(light-o-d*delta);
    if (D==0.0) { return 0.0; }
    let theta_d=atan2(span.y-delta,D)-atan2(span.x-delta,D);
    if (theta_d<1e-6) { return 1.0/max(span.y-span.x,1e-30); }
    let s=t-delta;
    return D/(theta_d*(D*D+s*s));
}
fn fog_phase(cosine: f32, g: f32) -> f32 {
    if (abs(g)<1e-3) { return 1.0/(4.0*PI); }
    let fac=1.0+g*(g-2.0*cosine);
    return (1.0-g*g)/(4.0*PI*fac*sqrt(max(fac,0.0)));
}
fn fog_phase_sample(d: vec3f, g: f32, u: vec2f) -> vec4f {
    var cosine=1.0-2.0*u.x;
    if (abs(g)>=1e-3) {
        let k=(1.0-g*g)/(1.0-g*cosine);
        cosine=(1.0+g*g-k*k)/(2.0*g);
    }
    cosine=clamp(cosine,-1.0,1.0);
    let sine=sqrt(max(1.0-cosine*cosine,0.0));
    let phi=2.0*PI*u.y;
    return vec4f(normalize(basis(d)*vec3f(sine*cos(phi),sine*sin(phi),cosine)),fog_phase(cosine,g));
}
fn fog_light_at(k: u32) -> vec4f {
    if (k<3u) { return vec4f(0.0); }
    let slot=k-2u;
    if (slot>u32(emitters_looped())) { return vec4f(0.0); }
    return vec4f(light_position(slot-1u),1.0);
}
fn fog_lit(k: u32, x: vec3f, d: vec3f, mis: bool, state: ptr<function,u32>, u: vec4f) -> vec3f {
    let g=frame.fog_tint.w;
    var direction=vec3f(0.0,0.0,1.0);
    var strength=vec3f(0.0);
    var limit=1e19;
    var pdf=0.0;
    var shape=0u;
    if (k==0u) {
        if (frame.sun_color.w<=0.0) { return vec3f(0.0); }
        direction=normalize(frame.sun_dir.xyz);
        if (frame.sun_dir.w<1.0) {
            let height=1.0-rnd(state,u.x)*(1.0-frame.sun_dir.w);
            let radius=sqrt(max(0.0,1.0-height*height));
            let phi=2.0*PI*rnd(state,u.y);
            direction=normalize(basis(direction)*vec3f(radius*cos(phi),radius*sin(phi),height));
        }
        strength=frame.sun_color.xyz*frame.sun_color.w;
    } else if (k==1u) {
        if (sky_cdf.width==0u) { return vec3f(0.0); }
        let light=sky_sample(vec2f(rnd(state,u.x),rnd(state,u.y)));
        direction=light.direction;
        strength=sky_surface_radiance(direction);
        pdf=light.pdf;
        if (!(pdf>0.0)) { return vec3f(0.0); }
    } else {
        let transform=basis(d);
        volume_vertex=true;
        let picked=direct_light(k-2u,transform,x,x,d,state,u);
        volume_vertex=false;
        if (picked.limit<0.0) { return vec3f(0.0); }
        direction=normalize(transform*picked.wi);
        strength=picked.scale;
        pdf=picked.pdf;
        limit=picked.limit;
        shape=picked.shape;
    }
    let phase=fog_phase(dot(d,direction),g);
    if (pdf>0.0 && !mis) {
        strength/=pdf;
    } else if (pdf>0.0) {
        if (k==1u) {
            strength/=pdf+phase;
        } else {
            let ratio=phase/pdf;
            strength/=pdf*(1.0+ratio*ratio);
        }
    }
    if (shape>0u) {
        let i=shape-1u;
        var t=1e20;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,x,direction,1e20); } else { t=primitive_hit(i,x,direction,1e20); }
        if (t<=0.0001 || t>=1e19) { return vec3f(0.0); }
        limit=t*0.9999-0.0002;
    }
    var seen=vec3f(1.0);
    if (limit>0.0) {
        if (TRANSMISSIVE_SHADOWS) { seen=shadow_passed(x,direction,limit); } else if (occluded(x,direction,limit)) { seen=vec3f(0.0); }
        seen*=fog_transmit(x,direction,limit);
    }
    if (k==0u) { seen=mix(vec3f(1.0),seen,frame.fog_light.x); }
    return strength*phase*seen;
}
fn fog_direct(o: vec3f, d: vec3f, span: vec2f, attenuation: f32, mis: bool, state: ptr<function,u32>) -> vec3f {
    var total=vec3f(0.0);
    let albedo=frame.fog_tint.xyz;
    let sun=dot(frame.sun_color.xyz,INDIRECT_LUMA)*max(frame.sun_color.w,0.0);
    let sky=select(0.0,max(sky_cdf.energy,0.0),sky_cdf.width>0u);
    var pick=select(0.0,1.0,sun>0.0);
    if (sun>0.0 && sky>0.0) { pick=clamp(sun/(sun+sky),0.1,0.9); }
    let sun_picked=rnd(state,sobol(FOG_DIM).y)<pick;
    for (var k=0u;k<2u+light_slots();k++) {
        var chance=1.0;
        if (k==0u) {
            if (!sun_picked) { continue; }
            chance=pick;
        } else if (k==1u) {
            if (sun_picked) { continue; }
            chance=1.0-pick;
        }
        let u=sobol(FOG_DIM+2u+2u*k);
        var r=rnd(state,u.x);
        let light=fog_light_at(k);
        var t=0.0;
        var weight=0.0;
        let equiangular=light.w>0.5;
        var chose_equiangular=false;
        if (equiangular) {
            if (r<0.5) { r*=2.0; } else { r=(r-0.5)*2.0; chose_equiangular=true; }
        }
        if (chose_equiangular) {
            let sample=fog_equiangular(o,d,light.xyz,span,r);
            if (!(sample.y>FOG_PDF_CUTOFF)) { continue; }
            t=sample.x;
            let density=fog_density(o.y+d.y*t);
            let transmit=exp(-fog_depth(o,d,span.x,t));
            let distance=fog_distance_pdf(o,d,span,attenuation,t);
            let ratio=distance/sample.y;
            weight=density*transmit/sample.y*2.0/(1.0+ratio*ratio);
        } else {
            let depth=-log(max(1.0-r*attenuation,1e-30));
            t=fog_distance(o,d,span.x,span.y,depth);
            weight=attenuation;
            if (equiangular) {
                let distance=fog_density(o.y+d.y*t)*exp(-depth)/attenuation;
                let ratio=fog_equiangular_pdf(o,d,light.xyz,span,t)/max(distance,1e-30);
                weight*=2.0/(1.0+ratio*ratio);
            }
        }
        if (!(weight>0.0)) { continue; }
        total+=albedo*(weight/chance)*fog_lit(k,o+d*t,d,mis,state,sobol(FOG_DIM+3u+2u*k));
    }
    return total;
}
fn sprite_phase() -> f32 {
    return select(0.5,frame.fog_tint.w,TRACE_FOG);
}
fn sprite_lobe(d: vec3f, n: vec3f, wi: vec3f, thick: f32) -> f32 {
    return mix(fog_phase(dot(d,wi),sprite_phase()),max(dot(n,wi)+0.5,0.0)/(2.25*PI),thick);
}
fn sprite_sample(d: vec3f, n: vec3f, thick: f32, u: vec3f) -> vec3f {
    if (u.z>=thick) { return fog_phase_sample(d,sprite_phase(),u.xy).xyz; }
    let cosine=1.5*sqrt(u.x)-0.5;
    let sine=sqrt(max(1.0-cosine*cosine,0.0));
    let phi=2.0*PI*u.y;
    return normalize(basis(n)*vec3f(sine*cos(phi),sine*sin(phi),cosine));
}
fn sprite_lit(k: u32, x: vec3f, d: vec3f, n: vec3f, thick: f32, state: ptr<function,u32>, u: vec4f) -> vec3f {
    var direction=vec3f(0.0,0.0,1.0);
    var strength=vec3f(0.0);
    var limit=1e19;
    var pdf=0.0;
    var shape=0u;
    if (k==0u) {
        if (frame.sun_color.w<=0.0) { return vec3f(0.0); }
        direction=normalize(frame.sun_dir.xyz);
        if (frame.sun_dir.w<1.0) {
            let height=1.0-rnd(state,u.x)*(1.0-frame.sun_dir.w);
            let radius=sqrt(max(0.0,1.0-height*height));
            let phi=2.0*PI*rnd(state,u.y);
            direction=normalize(basis(direction)*vec3f(radius*cos(phi),radius*sin(phi),height));
        }
        strength=frame.sun_color.xyz*frame.sun_color.w;
    } else if (k==1u) {
        if (sky_cdf.width==0u) { return vec3f(0.0); }
        let light=sky_sample(vec2f(rnd(state,u.x),rnd(state,u.y)));
        direction=light.direction;
        strength=sky_surface_radiance(direction);
        pdf=light.pdf;
        if (!(pdf>0.0)) { return vec3f(0.0); }
    } else {
        let transform=basis(n);
        volume_vertex=true;
        let picked=direct_light(k-2u,transform,x,x,n,state,u);
        volume_vertex=false;
        if (picked.limit<0.0) { return vec3f(0.0); }
        direction=normalize(transform*picked.wi);
        strength=picked.scale;
        pdf=picked.pdf;
        limit=picked.limit;
        shape=picked.shape;
    }
    let lobe=sprite_lobe(d,n,direction,thick);
    if (pdf>0.0) {
        if (k==1u) {
            strength/=pdf+lobe;
        } else {
            let ratio=lobe/pdf;
            strength/=pdf*(1.0+ratio*ratio);
        }
    }
    let origin=spawn_point(x,direction,x,0.0);
    if (shape>0u) {
        let i=shape-1u;
        var t=1e20;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,origin,direction,1e20); } else { t=primitive_hit(i,origin,direction,1e20); }
        if (t<=0.0001 || t>=1e19) { return vec3f(0.0); }
        limit=t*0.9999-0.0002;
    }
    var seen=vec3f(1.0);
    if (limit>0.0) {
        sprites_hidden=true;
        if (TRANSMISSIVE_SHADOWS) { seen=shadow_passed(origin,direction,limit); } else if (occluded(origin,direction,limit)) { seen=vec3f(0.0); }
        sprites_hidden=false;
        if (TRACE_FOG) { seen*=fog_transmit(origin,direction,limit); }
    }
    if (k==0u && TRACE_FOG) { seen=mix(vec3f(1.0),seen,frame.fog_light.x); }
    return strength*lobe*seen;
}
fn sprite_direct(x: vec3f, d: vec3f, n: vec3f, thick: f32, state: ptr<function,u32>) -> vec3f {
    var total=vec3f(0.0);
    for (var k=0u;k<2u+light_slots();k++) {
        total+=sprite_lit(k,x,d,n,thick,state,sobol(light_dim(k)));
    }
    return total;
}
fn radiance(o0: vec3f, d0: vec3f, state: ptr<function,u32>, first_albedo: ptr<function,vec4f>, first_normal: ptr<function,vec4f>) -> vec3f {
    var o=o0;
    var d=d0;
    var throughput=vec3f(1.0);
    var result=vec3f(0.0);
    var previous_pdf=0.0;
    var carried=3u;
    var lobes=vec3u(0u);
    var last=false;
    var lit_vertex=false;
    var through=false;
    var diffused=false;
    var fog_albedo=vec3f(0.0);
    var medium=false;
    var scatters=0u;
    var passes=0u;
    for (var bounce=0u;bounce<=TOTAL_BOUNCES+1u;bounce++) {
        if (SOBOL) { rng.z=16u*(bounce+1u); }
        trace_count(select(COUNT_BOUNCE,COUNT_PRIMARY,bounce==0u && passes==0u));
        let hit=scene(o,d,1e20,false);
        let seen=TRANSMISSIVE_SHADOWS && through;
        var reach=hit.t;
        var scatter=-1.0;
        var ends=false;
        if (TRACE_FOG) {
            let span=fog_span(o,d,hit.t);
            if (span.y>span.x) {
                let transmit=exp(-fog_depth(o,d,span.x,span.y));
                let attenuation=1.0-transmit;
                if (bounce==0u) { fog_albedo=frame.fog_tint.xyz*attenuation; }
                if (last) {
                    throughput*=transmit;
                } else if (attenuation>0.0) {
                    ends=scatters+1u>=u32(frame.fog_light.y);
                    let keep=min(attenuation/FOG_LIGHT_FLOOR,1.0);
                    if (keep>=1.0 || rnd(state,sobol(FOG_DIM).z)<keep) {
                        result+=indirect_capped(throughput*fog_direct(o,d,span,attenuation,!ends,state),bounce>0u)/keep;
                    }
                    let r=rnd(state,sobol(FOG_DIM).x);
                    if (r<attenuation) {
                        scatter=fog_distance(o,d,span.x,span.y,-log(max(1.0-r,1e-30)));
                        reach=scatter;
                    }
                }
            }
        }
        let origin_point=select(spawn_point(o,-d,o,0.0),o,medium);
        if (!seen) { result+=indirect_capped(throughput*light_hits(o,d,reach,origin_point,previous_pdf),bounce>1u); }
        if (scatter>=0.0) {
            let p=o+d*scatter;
            if (bounce==0u) {
                *first_albedo=vec4f(fog_albedo,1.0);
                *first_normal=vec4f(-d,1.0);
            }
            if (ends) { break; }
            throughput*=frame.fog_tint.xyz;
            let sample=fog_phase_sample(d,frame.fog_tint.w,vec2f(rnd(state,sobol(FOG_DIM+1u).x),rnd(state,sobol(FOG_DIM+1u).y)));
            diffused=true;
            lobes.x+=1u;
            scatters+=1u;
            last=bounce>=TOTAL_BOUNCES || lobes.x>DIFFUSE_BOUNCES || scatters>=u32(frame.fog_light.y);
            through=false;
            lit_vertex=true;
            medium=true;
            d=sample.xyz;
            previous_pdf=sample.w;
            o=p;
            if (bounce>=3u) {
                let keep=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);
                if (rnd(state,sobol(0u).x)>keep) { break; }
                throughput/=keep;
            }
            continue;
        }
        if (hit.t>=1e19) {
            if (TRACE_FOG && bounce==0u) { *first_albedo=vec4f(fog_albedo,0.0); }
            if (bounce==0u && frame.forward.w>0.0) { break; }
            if (seen && sky_cdf.width>0u) { break; }
            let pdf=sky_pdf(d);
            let weight=select(1.0,previous_pdf/(previous_pdf+pdf),bounce>0u && previous_pdf>0.0);
            let environment=select(sky_seen(d,sky_radiance(d)),sky_surface_radiance(d),bounce>0u);
            result+=indirect_capped(throughput*environment*weight,bounce>1u);
            break;
        }
        let p=o+d*hit.t;
        if (TRACE_SPRITES && hit.shape==0u && (instance_surfaces[hit.record].info.x&SPRITE)!=0u) {
            let look=instance_surfaces[hit.record];
            var keep=1.0;
            if (look.crop.z>0.0) {
                sprites_hidden=true;
                let behind=scene(p,d,look.crop.z,false);
                sprites_hidden=false;
                keep=clamp(behind.t/look.crop.z,0.0,1.0);
            }
            let additive=look.crop.y>0.5;
            let thick=look.transform.w*sprite_shape(hit.uv)*keep;
            let c=hit.uv*2.0-1.0;
            let sn=normalize(hit.tangent*c.x+hit.bitangent*c.y-d*sqrt(max(1.0-dot(c,c),0.0)));
            let shown=keep>=1.0 || alpha_hash(hit.surface^pcg(hit.record^0x27d4eb2du),o,d)<keep;
            if (shown) {
                let lit=look.crop.x>0.5;
                if (bounce==0u && !additive) {
                    *first_albedo=vec4f(select(vec3f(0.0),look.transform.xyz,lit),1.0);
                    *first_normal=vec4f(sn,1.0);
                }
                if (!lit) {
                    result+=indirect_capped(throughput*look.transform.xyz,bounce>1u);
                } else if (!last) {
                    result+=indirect_capped(throughput*look.transform.xyz*sprite_direct(p,d,sn,thick,state),bounce>0u);
                }
                if (!additive) {
                    if (!lit || last) { break; }
                    let u=sobol(3u);
                    let wi=sprite_sample(d,sn,thick,vec3f(rnd(state,u.x),rnd(state,u.y),rnd(state,u.z)));
                    throughput*=look.transform.xyz;
                    previous_pdf=sprite_lobe(d,sn,wi,thick);
                    if (!(previous_pdf>0.0)) { break; }
                    diffused=true;
                    lobes.x+=1u;
                    last=bounce>=TOTAL_BOUNCES || lobes.x>DIFFUSE_BOUNCES;
                    through=false;
                    lit_vertex=true;
                    medium=false;
                    d=wi;
                    o=spawn_point(p,d,o,hit.t);
                    if (bounce>=3u) {
                        let keep=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);
                        if (rnd(state,sobol(0u).x)>keep) { break; }
                        throughput/=keep;
                    }
                    continue;
                }
            }
            passes+=1u;
            if (passes>=SPRITE_PASSES) { break; }
            o=spawn_point(p,d,o,hit.t);
            bounce-=1u;
            continue;
        }
        var n=select(hit.n,-hit.n,dot(hit.n,d)>0.0);
        if (bounce==0u && frame.forward.w>1.5 && hit.material==u32(frame.forward.w)-2u) {
            *first_albedo=vec4f(0.0,0.0,0.0,catcher_shadow(p,n,o,hit.t,state));
            *first_normal=vec4f(n,1.0);
            break;
        }
        let m=material_at(hit.material);
        var s=surface(m);
        let flags=extension_flags(m);
        traced_thin=TRACE_THIN && (flags&EXTENSION_THIN)!=0u;
        let layered=flags&EXTENSION_LAYERS;
        if (layered>0u) {
            let first=extension_layers(m);
            for (var i=0u;i<layered;i++) {
                let at=first+i*5u;
                var local=p;
                if (u32(round(materials[at].x))==21u && hit.shape==0u && hit.record>0u) { local=hit_ray(hit.record,p,1.0); }
                let detailed=detail_layer(s,n,materials[at],materials[at+1u],materials[at+2u],materials[at+3u],materials[at+4u],p,local,hit.uv);
                s=detailed.shade;
                n=detailed.normal;
            }
        }
        traced_occlusion=1.0;
        traced_direction=vec3f(1.0,0.0,1.0);
        if (TRACE_MAPS && (flags&EXTENSION_MAPS)!=0u) {
            let textured=traced_maps(s,n,material_maps(m),hit,p);
            s=textured.shade;
            n=textured.normal;
        }
        let geometric=select(hit.geometric,-hit.geometric,dot(hit.geometric,d)>0.0);
        if (dot(n,geometric)<0.05) { n=normalize(n+geometric*0.1); }
        var layer=content_layers[hit.material];
        var shown=true;
        if (hit.shape==0u) {
            let word=instance_surfaces[hit.record].info.x;
            layer=hit_layer(word,hit.material);
            let face=(word>>3u)&3u;
            let front=dot(hit.geometric,d)<0.0;
            shown=face==0u || (face==1u && front) || (face==2u && !front);
        }
        let slot=i32(round(layer.layer.x));
        if (slot>=0 && shown) {
            let surface_data=instance_surfaces[hit.record];
            let at=content_uv(hit.uv,surface_data.transform);
            if (content_inside(at,surface_data.crop)) {
                let texel=content_at(slot,at);
                let blend=u32(round(layer.layer.y));
                s=content_composite(s,blend,texel,layer.layer.z,layer.extra.x);
                if (layer.layer.w!=0.0) {
                    let info=content_info[u32(slot)];
                    if (info.width>0u && info.height>0u) {
                        let tangent=normalize(hit.tangent-n*dot(hit.tangent,n));
                        let bitangent=normalize(hit.bitangent-n*dot(hit.bitangent,n));
                        let step=vec2f(1.5/f32(info.width),1.5/f32(info.height));
                        let gx=content_at(slot,at+vec2f(step.x,0.0)).a-content_at(slot,at-vec2f(step.x,0.0)).a;
                        let gy=content_at(slot,at+vec2f(0.0,step.y)).a-content_at(slot,at-vec2f(0.0,step.y)).a;
                        n=content_emboss(n,tangent,bitangent,vec2f(gx,gy),layer.layer.w);
                        let geometric=select(hit.geometric,-hit.geometric,dot(hit.geometric,d)>0.0);
                        if (dot(n,geometric)<0.05) { n=normalize(n+geometric*0.1); }
                    }
                }
            }
        }
        if (TRACE_WET && wet_active(m.wet)) {
            let soaked=wet_soak(s,n,geometric,p,m.wet,0.0);
            s=soaked.shade;
            n=soaked.normal;
        }
        traced_flaked=false;
        if (TRACE_FLAKES && (flags&EXTENSION_FLAKES)!=0u) {
            let at=extension_flakes(m);
            let flake=flake_normal(p,n,materials[at]);
            if (flake.w>0.5) {
                traced_flaked=true;
                traced_flake=flake.xyz;
                let tint=materials[at+1u].xyz;
                let coat=s.clearcoat;
                let coat_roughness=s.clearcoat_roughness;
                s=flake_facet(s,tint);
                s.roughness=materials[at].w;
                s.clearcoat=coat;
                s.clearcoat_roughness=coat_roughness;
            }
        }
        s.roughness=max(s.roughness,ROUGHNESS_FLOOR);
        s.clearcoat_roughness=max(s.clearcoat_roughness,ROUGHNESS_FLOOR);
        if (FILTER_GLOSSY>0.0 && diffused) {
            s.roughness=max(s.roughness,FILTER_GLOSSY);
            s.clearcoat_roughness=max(s.clearcoat_roughness,FILTER_GLOSSY);
        }
        if (bounce==0u) {
            *first_albedo=vec4f(select(s.base,s.base+fog_albedo,TRACE_FOG),1.0);
            *first_normal=vec4f(n,1.0);
        }
        s.emission*=emitter_weight(hit,p,origin_point,previous_pdf);
        if (seen && sampled_emitter(hit)) { s.emission=vec3f(0.0); }
        result+=indirect_capped(throughput*s.emission,bounce>1u);
        if (last) { break; }
        let transform=basis(n);
        if (TRACE_FLAKES && traced_flaked) { traced_flake=transpose(transform)*traced_flake; }
        if (TRACE_LOBES) { lobes_traced(material_lobes(m),transform,hit,n); }
        let wo=transpose(transform)*(-d);
        let origin=spawn_point(p,n,o,hit.t);
        let entering=dot(hit.n,d)<0.0;
        for (var k=0u;k<2u+light_slots();k++) {
            let u=sobol(light_dim(k));
            var local=vec3f(0.0,0.0,1.0);
            var direction=vec3f(0.0,0.0,1.0);
            var strength=vec3f(0.0);
            var limit=1e19;
            var shape=0u;
            var pdf=0.0;
            var lit=s;
            if (k==0u) {
                if (frame.sun_color.w<=0.0) { continue; }
                direction=normalize(frame.sun_dir.xyz);
                if (frame.sun_dir.w<1.0) {
                    let height=1.0-rnd(state,u.x)*(1.0-frame.sun_dir.w);
                    let radius=sqrt(max(0.0,1.0-height*height));
                    let phi=2.0*PI*rnd(state,u.y);
                    let orient=basis(direction);
                    direction=normalize(orient*vec3f(radius*cos(phi),radius*sin(phi),height));
                }
                local=transpose(transform)*direction;
                if (local.z<=0.0 && local.z<=-scatter_reach()) { continue; }
                if (local.z<=0.0) { direction=normalize(direction-n*dot(direction,n)+n*0.25); }
                var shade=1.0;
                if (bounce==0u && effects().base_roughness.y>0.0) { shade=canopy(p); }
                strength=frame.sun_color.xyz*(frame.sun_color.w*shade);
            } else if (k==1u) {
                if (sky_cdf.width==0u) { continue; }
                let light=sky_sample(vec2f(rnd(state,u.x),rnd(state,u.y)));
                direction=light.direction;
                local=transpose(transform)*direction;
                if (local.z<=0.0) { continue; }
                strength=sky_surface_radiance(direction)*traced_occlusion;
                pdf=light.pdf;
                if (!(pdf>0.0)) { continue; }
            } else {
                let picked=direct_light(k-2u,transform,p,origin,geometric,state,u);
                if (picked.limit<0.0) { continue; }
                local=picked.wi;
                direction=picked.direction;
                strength=picked.scale;
                pdf=picked.pdf;
                limit=picked.limit;
                shape=picked.shape;
                lit.roughness=max(s.roughness,picked.roughness);
            }
            if (pdf>0.0) {
                let bsdf=bsdf_pdf(s,wo,local);
                if (k==1u) {
                    strength/=pdf+bsdf;
                } else {
                    let ratio=bsdf/pdf;
                    strength/=pdf*(1.0+ratio*ratio);
                }
            }
            if (shape>0u) {
                let i=shape-1u;
                var t=1e20;
                if (u32(shapes[i].a.w)==5u) { t=union_hit(i,origin,direction,1e20); } else { t=primitive_hit(i,origin,direction,1e20); }
                if (t<=0.0001 || t>=1e19) { continue; }
                limit=t*0.9999-0.0002;
            }
            if (TRACE_FOG && limit>0.0) { strength*=fog_transmit(origin,direction,limit); }
            if (limit>0.0) {
                if (TRANSMISSIVE_SHADOWS) {
                    var unshadowed=throughput*traced_eval(lit,wo,local)*select(1.0,glossy_share(s),pdf>0.0)*local.z*strength;
                    let reach=throughput*strength;
                    let keep=clamp(16.0*PI*max(unshadowed.r,max(unshadowed.g,unshadowed.b))/max(max(reach.r,max(reach.g,reach.b)),1e-30),0.0,1.0);
                    if (keep<1.0) {
                        if (rnd(state,u.w)>=keep) { continue; }
                        unshadowed/=keep;
                    }
                    trace_count(COUNT_SURFACE_SHADOW);
                    result+=indirect_capped(unshadowed*shadow_passed(origin,direction,limit),bounce>0u);
                    continue;
                }
                trace_count(COUNT_SURFACE_SHADOW);
                if (occluded(origin,direction,limit)) { continue; }
            }
            result+=indirect_capped(throughput*traced_eval(lit,wo,local)*select(1.0,glossy_share(s),pdf>0.0)*local.z*strength,bounce>0u);
        }
        let sample=sample_bsdf(s,wo,entering,carried,state,sobol(3u));
        let wi=sample.dir;
        diffused=diffused || sample.lobe==0u;
        lobes+=vec3u(select(0u,1u,sample.lobe==0u),select(0u,1u,sample.lobe==1u),select(0u,1u,sample.lobe==2u));
        last=bounce>=TOTAL_BOUNCES || any(lobes>vec3u(DIFFUSE_BOUNCES,GLOSSY_BOUNCES,TRANSMISSION_BOUNCES));
        through=sample.pdf< -1.5 && lit_vertex;
        lit_vertex=sample.pdf>0.0 || through;
        if (sample.pdf<0.0) {
            throughput*=sample.weight;
            carried=sample.channel;
            if (sample.pdf< -1.5) {
                throughput*=sqrt(clamp(s.base,vec3f(0.0),vec3f(1.0)))*beer(s.thickness,s.subsurface_tint,s.absorption);
            }
            d=normalize(transform*wi);
            previous_pdf=0.0;
        } else {
            if (wi.z<=0.0 || sample.pdf<=0.0) { break; }
            throughput*=eval_bounce(s,wo,wi)*glossy_share(s)*wi.z/sample.pdf*traced_occlusion;
            d=normalize(transform*wi);
            previous_pdf=sample.pdf;
        }
        o=spawn_point(p,d,o,hit.t);
        medium=false;
        if (bounce>=3u) {
            let keep=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);
            if (rnd(state,sobol(0u).x)>keep) { break; }
            throughput/=keep;
        }
    }
    return result;
}
@compute @workgroup_size(8,8)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_id) local: vec3u) {
    let lane=local.y*8u+local.x;
    let first_lane=(frame.counts.w>>16u)&0x7fu;
    let end_lane=frame.counts.w>>23u;
    let blocks=(frame.size.y+7u)/8u;
    let block=((frame.counts.w&0xffffu)+group.y)*BLOCK_STRIDE%blocks;
    let gid=vec3u((group.x+frame.columns.x)*8u+local.x,block*8u+local.y,0u);
    if (lane<first_lane || lane>=end_lane || gid.x>=frame.size.x || gid.y>=frame.size.y) { return; }
    let index=gid.y*frame.size.x+gid.x;
    let pixel=gid.xy+frame.window.zw;
    let stream=pixel.y*frame.window.x+pixel.x;
    var sum=Accum(vec4f(0.0),vec4f(0.0),vec4f(0.0));
    if (frame.size.z>0u) { sum=accum[index]; }
    if (sum.color.w<0.0) { return; }
    trace_count_by(COUNT_PATHS,frame.size.w);
    for (var sample=0u;sample<frame.size.w;sample++) {
        var first_albedo=vec4f(0.0);
        var first_normal=vec4f(0.0);
        var state=pcg(stream*1973u+pcg(frame.counts.z*9277u+(frame.size.z+sample)*7919u+26699u));
        if (SOBOL) { rng=vec3u(hash_iqnt2d(pixel.x,pixel.y)^frame.counts.z,frame.size.z+sample,0u); }
        let pixel_filter=sobol(0u);
        let jitter=vec2f(rnd(&state,pixel_filter.x),rnd(&state,pixel_filter.y));
        let ndc=(vec2f(pixel)+jitter)/vec2f(frame.window.xy)*2.0-1.0;
        let kind=u32(round(frame.origin.w));
        var cam_origin=frame.origin.xyz;
        var cam_forward=frame.forward.xyz;
        var cam_right=frame.right.xyz;
        var cam_up=frame.up.xyz;
        if (TRACE_MOTION) {
            var u=f32(pcg(state^0x2545f491u)>>8u)/16777216.0;
            if (SOBOL) { u=sobol(1u).z; }
            ray_time=shutter_time(u);
            let head=motion_head();
            if (head.y>1u) {
                let moved=motion_at(1u,head.y,ray_time);
                cam_right=vec3f(moved.x.x,moved.y.x,moved.z.x);
                cam_up=vec3f(moved.x.y,moved.y.y,moved.z.y);
                cam_forward=vec3f(moved.x.z,moved.y.z,moved.z.z);
                cam_origin=vec3f(moved.x.w,moved.y.w,moved.z.w);
            }
        }
        var direction=normalize(cam_forward+cam_right*(ndc.x+frame.lens.x)+cam_up*(-ndc.y+frame.lens.y));
        var origin=cam_origin;
        if (kind==1u) {
            direction=normalize(cam_forward);
            origin+=normalize(cam_right)*((ndc.x+frame.lens.x)*frame.right.w)+normalize(cam_up)*((-ndc.y+frame.lens.y)*frame.up.w);
        } else if (kind==2u) {
            let phi=(ndc.x*0.5)*2.0*PI;
            let theta=(ndc.y*0.5+0.5)*PI;
            let local=vec3f(sin(theta)*sin(phi),cos(theta),-sin(theta)*cos(phi));
            direction=normalize(normalize(cam_right)*local.x+normalize(cam_up)*local.y-normalize(cam_forward)*local.z);
        } else if (frame.lens.z>0.0) {
            let focus=origin+direction*(frame.lens.w/dot(direction,normalize(cam_forward)));
            let aperture=sobol(1u);
            let radius=sqrt(rnd(&state,aperture.x))*frame.lens.z;
            let phi=2.0*PI*rnd(&state,aperture.y);
            origin+=normalize(cam_right)*(radius*cos(phi))+normalize(cam_up)*(radius*sin(phi));
            direction=normalize(focus-origin);
        }
        let traced=radiance(origin,direction,&state,&first_albedo,&first_normal);
        var value=traced;
        if (effects().thick_sub_film.w>0.0) {
            let depth=scene(origin,direction,1e20,false).t;
            value=steam_composite(origin,direction,depth,traced);
        }
        sum.color=vec4f(sum.color.xyz+value,sum.color.w);
        sum.albedo+=first_albedo;
        sum.normal+=first_normal;
    }
    sum.color.w=f32(frame.size.z+frame.size.w);
    accum[index]=sum;
    let count=max(sum.color.w,1.0);
    let n=sum.normal.xyz/count;
    textureStore(output,vec2i(gid.xy),vec4f(sum.color.xyz/count,select(1.0,sum.albedo.w/count,frame.forward.w>0.0)));
    textureStore(albedo,vec2i(gid.xy),sum.albedo/count);
    textureStore(normal,vec2i(gid.xy),vec4f(select(vec3f(0.0),normalize(n),length(n)>1e-6),sum.normal.w/count));
    trace_count_flush();
}
