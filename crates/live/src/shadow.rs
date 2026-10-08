use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU64;

use crate::effects::{CreatureInstance, CreatureMesh};
use pfx_gpu::{Gpu, GpuProfiler, wgpu};
use wgpu::util::DeviceExt;

pub const CASCADE_COUNT: usize = 3;
pub const DEFAULT_RESOLUTION: u32 = 4096;
pub const SAMPLE_UNIFORM_BYTES: usize = 288;
pub const SOFT_TAPS: u32 = 16;
pub const BLOCKER_TAPS: u32 = 8;
pub const FILTER_TAPS: u32 = 12;
pub const SOFT_TAP_LIMIT: u32 = 64;
pub const BLOCKER_GATHERS: u32 = 0;
pub const SOFT_REACH_TEXELS: f32 = 48.0;
pub const SOFT_SLOPE_LIMIT: f32 = 4.0;
pub const OUTER_SCALE: f32 = 3.0;
pub const SOFT_OFFSET: f32 = 0.0;

pub const SHADOW_WGSL: &str = r#"
override use_soft_search: bool = true;
override use_soft_filter: bool = true;
override use_cascade_blend: bool = true;
struct ShadowSample {
    splits: vec4f,
    filter_params: vec4f,
    sun: vec4f,
    texels: vec4f,
    view_proj_0: mat4x4f,
    view_proj_1: mat4x4f,
    view_proj_2: mat4x4f,
    soft: vec4f,
    soft_taps: vec4f,
}

fn shadow_select(distance: f32, splits: vec4f, blend: f32) -> vec3f {
    var cascade = 0.0;
    if (distance > splits.y) {
        cascade = 1.0;
    }
    if (distance > splits.z) {
        cascade = 2.0;
    }
    var next = cascade;
    var t = 0.0;
    if (cascade < 2.0) {
        let start = select(splits.y, splits.x, cascade < 0.5);
        let end = select(splits.z, splits.y, cascade < 0.5);
        let band = (end - start) * blend;
        if (band > 0.0 && distance > end - band) {
            next = cascade + 1.0;
            t = clamp((distance - (end - band)) / band, 0.0, 1.0);
        }
    }
    return vec3f(cascade, next, t);
}

fn shadow_blend(closer: f32, farther: f32, t: f32) -> f32 {
    return mix(closer, farther, t);
}

fn shadow_project(view_proj: mat4x4f, world: vec3f) -> vec3f {
    let clip = view_proj * vec4f(world, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2f(ndc.x, -ndc.y) * 0.5 + vec2f(0.5, 0.5);
    return vec3f(uv, ndc.z);
}

fn shadow_pcf(
    map: texture_depth_2d_array,
    samp: sampler_comparison,
    uv: vec2f,
    layer: i32,
    depth: f32,
    radius: f32,
) -> f32 {
    let size = textureDimensions(map);
    let texel = vec2f(1.0 / f32(size.x), 1.0 / f32(size.y));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = uv + vec2f(f32(x), f32(y)) * texel * radius;
            lit += textureSampleCompareLevel(map, samp, at, layer, depth);
        }
    }
    return lit / 9.0;
}

fn shadow_disc(index: u32, count: u32, turn: f32) -> vec2f {
    let radius = sqrt((f32(index) + 0.5) / f32(count));
    let angle = f32(index) * 2.39996323 + turn;
    return vec2f(cos(angle), sin(angle)) * radius;
}

fn shadow_turn(world: vec3f) -> f32 {
    let p = fract(world * vec3f(443.897, 441.423, 437.195));
    let q = p + dot(p, p.yzx + 19.19);
    return fract((q.x + q.y) * q.z) * 6.2831853;
}

fn shadow_slope(view_proj: mat4x4f, normal: vec3f, size: vec2f, scales: vec2f, limit: f32) -> vec2f {
    let a0 = view_proj[0].xyz;
    let a1 = view_proj[1].xyz;
    let a2 = view_proj[2].xyz;
    let len = length(normal);
    if (len < 1e-6) {
        return vec2f(0.0);
    }
    let n = normal / len;
    let m = n.x * cross(a1, a2) + n.y * cross(a2, a0) + n.z * cross(a0, a1);
    let denom = select(-1e-6, m.z, abs(m.z) >= 1e-6);
    let g = -m.xy / denom;
    var per_texel = vec2f(g.x * 2.0 / size.x, -g.y * 2.0 / size.y);
    let tangent = length(per_texel) * scales.x * scales.y;
    if (tangent > limit) {
        per_texel = per_texel * (limit / tangent);
    }
    return per_texel;
}

fn shadow_soft(
    map: texture_depth_2d_array,
    samp: sampler_comparison,
    gather: sampler,
    view_proj: mat4x4f,
    projected: vec3f,
    layer: i32,
    depth: f32,
    radius: f32,
    soft: vec4f,
    taps: vec4f,
    world: vec3f,
    normal: vec3f,
) -> f32 {
    let size = vec2f(textureDimensions(map));
    let per_depth = 1.0 / max(length(vec3f(view_proj[0].z, view_proj[1].z, view_proj[2].z)), 1e-8);
    let per_world = 0.5 * length(vec3f(view_proj[0].x, view_proj[1].x, view_proj[2].x)) * size.x;
    let slope = shadow_slope(view_proj, normal, size, vec2f(per_depth, per_world), soft.w);
    let turn = shadow_turn(world);
    let reach = min(depth * per_depth * soft.x * per_world, soft.y);
    let count = u32(soft.z);
    let filter_count = u32(taps.y);
    let search = max(reach, radius);
    if (!use_soft_search) {
        return shadow_soft_filter(map, samp, projected, layer, depth, search, filter_count, turn, slope);
    }
    let centre = projected.xy * size;
    let gathers = u32(taps.x);
    var blockers = 0.0;
    var total = 0.0;
    var near = vec3f(-1e30, 0.0, 0.0);
    var far = false;
    var inner = false;
    var tested = count;
    if (gathers > 0u) {
        tested = gathers * 4u;
        for (var i = 0u; i < gathers; i++) {
            let at = round(centre + shadow_disc(i, gathers, turn) * search);
            let stored = textureGather(map, gather, at / size, layer);
            let corner = at - vec2f(0.5) - centre;
            let offsets = array<vec2f, 4>(corner + vec2f(0.0, 1.0), corner + vec2f(1.0, 1.0), corner + vec2f(1.0, 0.0), corner);
            for (var k = 0u; k < 4u; k++) {
                let shift = dot(slope, offsets[k]);
                if (stored[k] < depth + shift) {
                    let level = stored[k] - shift;
                    far = far || stored[k] <= 0.0;
                    inner = inner || stored[k] > 0.0;
                    blockers += 1.0;
                    total += level;
                    near = shadow_near(near, level, depth);
                }
            }
        }
    } else {
        for (var i = 0u; i < count; i++) {
            let at = centre + shadow_disc(i, count, turn) * search;
            let texel = clamp(vec2i(floor(at)), vec2i(0), vec2i(size) - vec2i(1));
            let stored = textureLoad(map, texel, layer, 0);
            let here = depth + dot(slope, vec2f(texel) + vec2f(0.5) - centre);
            if (stored < here) {
                let level = stored - dot(slope, vec2f(texel) + vec2f(0.5) - centre);
                far = far || stored <= 0.0;
                inner = inner || stored > 0.0;
                blockers += 1.0;
                total += level;
                near = shadow_near(near, level, depth);
            }
        }
    }
    let flags = u32(taps.z);
    if (blockers < 0.5 && (flags & 8u) == 0u) {
        return 1.0;
    }
    if (!use_soft_filter) {
        return 1.0 - blockers / f32(tested);
    }
    if ((flags & 2u) != 0u && blockers > f32(tested) - 0.5) {
        return 0.0;
    }
    let clustered = (flags & 1u) != 0u && near.z * 2.0 >= blockers;
    let blocker = select(select(total / blockers, near.y / near.z, clustered), depth, blockers < 0.5);
    let width = clamp((depth - blocker) * per_depth * soft.x * per_world, radius, soft.y);
    var lit = 1.0;
    if ((flags & 16u) == 0u || clustered || near.z < 0.5) {
        lit = shadow_soft_filter(map, samp, projected, layer, depth, width, filter_count, turn, slope);
    } else {
        let above = textureLoad(map, clamp(vec2i(floor(centre)), vec2i(0), vec2i(size) - vec2i(1)), layer, 0);
        let seen = above < depth && above > 0.0;
        let top = select(near.x, above, seen);
        let level = select(near.y / near.z, above, seen);
        let close = clamp((depth - level) * per_depth * soft.x * per_world, radius, soft.y);
        if (close >= width * 0.5) {
            lit = shadow_soft_filter(map, samp, projected, layer, depth, width, filter_count, turn, slope);
        } else {
            let band = top - 0.5 * max(depth - top, 0.0);
            lit = shadow_layered_filter(map, samp, projected, layer, depth, band, width, close, filter_count, turn, slope);
        }
    }
    if ((flags & 64u) == 0u || !far || lit <= 0.0) {
        return lit;
    }
    let hidden = clamp(select(0.0, (depth - near.y / near.z) * per_depth * soft.x * per_world, inner), radius, soft.y);
    return min(lit, shadow_soft_filter(map, samp, projected, 1, depth, hidden, filter_count, turn, slope));
}

fn shadow_layered_filter(
    map: texture_depth_2d_array,
    samp: sampler_comparison,
    projected: vec3f,
    layer: i32,
    depth: f32,
    band: f32,
    width: f32,
    close: f32,
    count: u32,
    turn: f32,
    slope: vec2f,
) -> f32 {
    let size = vec2f(textureDimensions(map));
    var lit = 0.0;
    for (var i = 0u; i < count; i++) {
        let disc = shadow_disc(i, count, turn + 1.7);
        let far = disc * width;
        let far_lit = textureSampleCompareLevel(map, samp, projected.xy + far / size, layer, band + dot(slope, far));
        let near = disc * close;
        let at = projected.xy + near / size;
        let shift = dot(slope, near);
        let near_blocked = textureSampleCompareLevel(map, samp, at, layer, band + shift) - textureSampleCompareLevel(map, samp, at, layer, depth + shift);
        lit += far_lit * (1.0 - clamp(near_blocked, 0.0, 1.0));
    }
    return lit / f32(count);
}

fn shadow_near(near: vec3f, level: f32, depth: f32) -> vec3f {
    if (level > near.x + 0.5 * max(depth - level, 0.0)) {
        return vec3f(level, level, 1.0);
    }
    if (level >= near.x - 0.5 * max(depth - near.x, 0.0)) {
        return vec3f(max(near.x, level), near.y + level, near.z + 1.0);
    }
    return near;
}

fn shadow_soft_filter(
    map: texture_depth_2d_array,
    samp: sampler_comparison,
    projected: vec3f,
    layer: i32,
    depth: f32,
    width: f32,
    count: u32,
    turn: f32,
    slope: vec2f,
) -> f32 {
    let size = vec2f(textureDimensions(map));
    var lit = 0.0;
    for (var i = 0u; i < count; i++) {
        let step = shadow_disc(i, count, turn + 1.7) * width;
        let at = projected.xy + step / size;
        lit += textureSampleCompareLevel(map, samp, at, layer, depth + dot(slope, step));
    }
    return lit / f32(count);
}

fn shadow_offset(world: vec3f, normal: vec3f, sun: vec3f, filter_params: vec4f, texel: f32, soft: f32) -> vec3f {
    let len = length(normal);
    let n = select(vec3f(0.0, 1.0, 0.0), normal / max(len, 1e-8), len > 1e-6);
    let slen = length(sun);
    let toward = select(vec3f(0.0, 1.0, 0.0), sun / max(slen, 1e-8), slen > 1e-6);
    let ndotl = clamp(dot(n, toward), 0.0, 1.0);
    let slope = sqrt(max(1.0 - ndotl * ndotl, 0.0));
    let scale = select(1.0, soft, soft >= 0.0 && ndotl * ndotl * 17.0 >= 1.0);
    return world + n * texel * (filter_params.y + filter_params.z * slope) * scale;
}

fn shadow_one(
    map: texture_depth_2d_array,
    samp: sampler_comparison,
    gather: sampler,
    data: ShadowSample,
    world: vec3f,
    normal: vec3f,
    cascade: i32,
) -> f32 {
    var view_proj = data.view_proj_0;
    var texel = data.texels.x;
    if (cascade == 1) {
        view_proj = data.view_proj_1;
        texel = data.texels.y;
    } else if (cascade == 2) {
        view_proj = data.view_proj_2;
        texel = data.texels.z;
    }
    var layer = cascade;
    let soft = select(-1.0, data.soft_taps.w, (use_soft_search || use_soft_filter) && data.soft.x > 0.0);
    var projected = shadow_project(view_proj, shadow_offset(world, normal, data.sun.xyz, data.filter_params, texel, soft));
    if ((any(projected < vec3f(0.0)) || any(projected > vec3f(1.0))) && (u32(data.soft_taps.z) & 32u) != 0u && cascade != 2) {
        view_proj = data.view_proj_2;
        texel = data.texels.z;
        layer = 2;
        projected = shadow_project(view_proj, shadow_offset(world, normal, data.sun.xyz, data.filter_params, texel, soft));
    }
    if (any(projected < vec3f(0.0)) || any(projected > vec3f(1.0))) {
        return 1.0;
    }
    if ((use_soft_search || use_soft_filter) && data.soft.x > 0.0) {
        return shadow_soft(
            map,
            samp,
            gather,
            view_proj,
            projected,
            layer,
            projected.z - data.sun.w,
            data.filter_params.x,
            data.soft,
            data.soft_taps,
            world,
            normal,
        );
    }
    return shadow_pcf(
        map,
        samp,
        projected.xy,
        layer,
        projected.z - data.sun.w,
        data.filter_params.x,
    );
}

fn shadow_sample(
    map: texture_depth_2d_array,
    samp: sampler_comparison,
    gather: sampler,
    data: ShadowSample,
    world: vec3f,
    normal: vec3f,
    view_distance: f32,
) -> f32 {
    if ((u32(data.soft_taps.z) & 32u) != 0u) {
        return shadow_one(map, samp, gather, data, world, normal, 0);
    }
    let choice = shadow_select(view_distance, data.splits, data.filter_params.w);
    if (use_cascade_blend && choice.z > 0.0 && (u32(data.soft_taps.z) & 4u) != 0u) {
        let dither = fract(shadow_turn(world.zxy) * 0.15915494);
        return shadow_one(map, samp, gather, data, world, normal, i32(select(choice.x, choice.y, dither < choice.z)));
    }
    let lit = shadow_one(map, samp, gather, data, world, normal, i32(choice.x));
    if (!use_cascade_blend || choice.z <= 0.0) {
        return lit;
    }
    let next = shadow_one(map, samp, gather, data, world, normal, i32(choice.y));
    return shadow_blend(lit, next, choice.z);
}
"#;

const DEPTH_WGSL: &str = r#"
struct Light {
    view_proj: mat4x4f,
}

@group(0) @binding(0) var<uniform> light: Light;

struct VertexIn {
    @location(0) position: vec3f,
    @location(1) model0: vec4f,
    @location(2) model1: vec4f,
    @location(3) model2: vec4f,
    @location(4) model3: vec4f,
}

@vertex
fn vs_main(in: VertexIn) -> @builtin(position) vec4f {
    let model = mat4x4f(in.model0, in.model1, in.model2, in.model3);
    return light.view_proj * model * vec4f(in.position, 1.0);
}
"#;

const CREATURE_DEPTH_WGSL: &str = r#"
struct Light { view_proj: mat4x4f, }
struct Creature { position_scale: vec4f, right: vec4f, up: vec4f, forward: vec4f, motion: vec4f, }
@group(0) @binding(0) var<uniform> light: Light;
@group(1) @binding(0) var<storage, read> vertices: array<vec4f>;
@group(1) @binding(1) var<storage, read> creatures: array<Creature>;
@group(1) @binding(2) var<uniform> time: vec4f;
@vertex fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> @builtin(position) vec4f {
    let instance = creatures[index];
    var local = vertices[vertex].xyz;
    let phase = time.x * (4.0 + instance.motion.y * 1.3) + instance.motion.x;
    if (instance.motion.z < 0.5) {
        local.y += sin(phase) * abs(local.x) * 0.5;
    } else {
        local.x += sin(phase + local.z * 4.0) * max(-local.z, 0.0) * 0.2;
    }
    let world = instance.position_scale.xyz + instance.position_scale.w * (instance.right.xyz * local.x + instance.up.xyz * local.y + instance.forward.xyz * local.z);
    return light.view_proj * vec4f(world, 1.0);
}
"#;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const TRANSMISSION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub columns: [[f32; 4]; 4],
}

impl Mat4 {
    pub const fn identity() -> Self {
        Self {
            columns: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub const fn from_columns(columns: [[f32; 4]; 4]) -> Self {
        Self { columns }
    }

    pub fn multiply(self, right: Self) -> Self {
        let mut columns = [[0.0; 4]; 4];
        for (col, column) in columns.iter_mut().enumerate() {
            for (row, value) in column.iter_mut().enumerate() {
                *value = self.columns[0][row] * right.columns[col][0]
                    + self.columns[1][row] * right.columns[col][1]
                    + self.columns[2][row] * right.columns[col][2]
                    + self.columns[3][row] * right.columns[col][3];
            }
        }
        Self { columns }
    }

    pub fn transform_point(self, point: [f32; 3]) -> [f32; 3] {
        let mut clip = [0.0; 4];
        let source = [point[0], point[1], point[2], 1.0];
        for (row, value) in clip.iter_mut().enumerate() {
            *value = self.columns[0][row] * source[0]
                + self.columns[1][row] * source[1]
                + self.columns[2][row] * source[2]
                + self.columns[3][row] * source[3];
        }
        let scale = if clip[3].abs() < 1.0e-12 {
            1.0
        } else {
            clip[3]
        };
        [clip[0] / scale, clip[1] / scale, clip[2] / scale]
    }

    fn to_bytes(self) -> [u8; 64] {
        let mut bytes = [0; 64];
        for (col, column) in self.columns.iter().enumerate() {
            for (row, value) in column.iter().enumerate() {
                let start = (col * 4 + row) * 4;
                bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    pub fov_y: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
    pub resolution: u32,
    pub lambda: f32,
    pub blend: f32,
    pub pcf_radius: f32,
    pub normal_bias: f32,
    pub slope_bias: f32,
    pub depth_bias: f32,
    pub caster_margin: f32,
    pub receiver: Option<ReceiverBox>,
    pub sun_radius_deg: f32,
    pub blocker_taps: u32,
    pub filter_taps: u32,
    pub blocker_gathers: u32,
    pub nearest_blocker: bool,
    pub umbra_skip: bool,
    pub lit_skip: bool,
    pub blend_dither: bool,
    pub layered: bool,
    pub outer_scale: f32,
    pub near_map: bool,
    pub soft_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReceiverBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for Quality {
    fn default() -> Self {
        Self {
            resolution: DEFAULT_RESOLUTION,
            lambda: 0.5,
            blend: 0.1,
            pcf_radius: 1.0,
            normal_bias: 1.0,
            slope_bias: 2.0,
            depth_bias: 0.0008,
            caster_margin: 0.0,
            receiver: None,
            sun_radius_deg: 0.0,
            blocker_taps: BLOCKER_TAPS,
            filter_taps: FILTER_TAPS,
            blocker_gathers: BLOCKER_GATHERS,
            nearest_blocker: true,
            umbra_skip: true,
            lit_skip: true,
            blend_dither: true,
            layered: false,
            outer_scale: OUTER_SCALE,
            near_map: true,
            soft_offset: SOFT_OFFSET,
        }
    }
}

impl Quality {
    pub fn splits_near_map(&self) -> bool {
        self.near_map && self.receiver.is_some() && outer(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choice {
    pub cascade: usize,
    pub next: usize,
    pub blend: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascade {
    pub near: f32,
    pub far: f32,
    pub view_proj: Mat4,
    pub texel: f32,
    pub light_center: [f32; 3],
    pub snapped_center: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub splits: [f32; 4],
    pub cascades: [Cascade; CASCADE_COUNT],
    pub toward_sun: [f32; 3],
    pub resolution: u32,
}

pub struct Caster<'a> {
    pub positions: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub first_index: u32,
    pub index_count: u32,
    pub instances: &'a [Mat4],
    pub moving: bool,
    pub generation: u64,
}

struct Draw<'a> {
    positions: &'a wgpu::Buffer,
    indices: &'a wgpu::Buffer,
    first_index: u32,
    index_count: u32,
    instance_offset: u64,
    instance_count: u32,
}

#[derive(PartialEq)]
struct StaticDraw {
    positions: u64,
    indices: u64,
    first_index: u32,
    index_count: u32,
    generation: u64,
    instances: Vec<u32>,
}

#[derive(PartialEq)]
struct StaticKey {
    sun: [u32; 3],
    matrices: [[u32; 16]; CASCADE_COUNT],
    draws: Vec<StaticDraw>,
    content: u64,
    near_map: bool,
}

pub type LayerDraw<'d> = dyn FnMut(&mut wgpu::RenderPass<'_>, usize, bool) + 'd;

struct Transmission {
    size: u32,
    tiles: FaceTiles,
    map: wgpu::Texture,
    _depth: wgpu::Texture,
    view: wgpu::TextureView,
    layers: Vec<wgpu::TextureView>,
    depth_layers: Vec<wgpu::TextureView>,
    key: Option<StaticKey>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FaceTiles {
    pub faces: u32,
    pub tile: u32,
    pub columns: u32,
}

impl FaceTiles {
    pub fn new(faces: usize, size: u32, face_resolution: u32) -> Self {
        if faces == 0 || size == 0 {
            return Self::default();
        }
        let tile = transmission_size(face_resolution).min(size).max(1);
        Self {
            faces: faces as u32,
            tile,
            columns: (size / tile).max(1),
        }
    }

    pub fn per_layer(&self) -> u32 {
        self.columns * self.columns
    }

    pub fn layers(&self) -> u32 {
        if self.faces == 0 {
            0
        } else {
            self.faces.div_ceil(self.per_layer())
        }
    }

    pub fn place(&self, face: u32) -> (u32, [u32; 2]) {
        let per = self.per_layer().max(1);
        let slot = face % per;
        (
            CASCADE_COUNT as u32 + face / per,
            [
                slot % self.columns.max(1) * self.tile,
                slot / self.columns.max(1) * self.tile,
            ],
        )
    }
}

pub struct Shadows {
    quality: Cell<Quality>,
    resolution: u32,
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    layers: [wgpu::TextureView; CASCADE_COUNT],
    backup: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    creature_pipeline: wgpu::RenderPipeline,
    creature_layout: wgpu::BindGroupLayout,
    compare: wgpu::Sampler,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    align: u32,
    instances: wgpu::Buffer,
    instance_capacity: u64,
    cache: Option<StaticKey>,
    atlas_clean: bool,
    reused: bool,
    transmission: Option<Transmission>,
}

fn outer(quality: &Quality) -> bool {
    finite(quality.outer_scale) > 1.0
}

pub fn transmission_size(resolution: u32) -> u32 {
    (resolution / 4).max(256).min(resolution)
}

pub fn splits(near: f32, far: f32, lambda: f32) -> [f32; 4] {
    let near = finite(near).max(1.0e-4);
    let far = finite(far).max(near + 1.0e-3);
    let lambda = finite(lambda).clamp(0.0, 1.0);
    let ratio = far / near;
    let mut out = [near, 0.0, 0.0, far];
    for (index, slot) in out.iter_mut().enumerate().skip(1).take(CASCADE_COUNT - 1) {
        let portion = index as f32 / CASCADE_COUNT as f32;
        let logarithmic = near * ratio.powf(portion);
        let uniform = near + (far - near) * portion;
        *slot = lambda * logarithmic + (1.0 - lambda) * uniform;
    }
    out
}

pub fn select_cascade(distance: f32, splits: [f32; 4], blend: f32) -> Choice {
    let blend = finite(blend).clamp(0.0, 1.0);
    let mut cascade = 0;
    if distance > splits[1] {
        cascade = 1;
    }
    if distance > splits[2] {
        cascade = 2;
    }
    if cascade < CASCADE_COUNT - 1 {
        let start = splits[cascade];
        let end = splits[cascade + 1];
        let band = (end - start) * blend;
        if band > 0.0 && distance > end - band {
            let t = ((distance - (end - band)) / band).clamp(0.0, 1.0);
            return Choice {
                cascade,
                next: cascade + 1,
                blend: t,
            };
        }
    }
    Choice {
        cascade,
        next: cascade,
        blend: 0.0,
    }
}

pub fn light_basis(toward_sun: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = if length(toward_sun) < 1.0e-8 {
        [0.0, -1.0, 0.0]
    } else {
        normalize(scale(toward_sun, -1.0))
    };
    let helper = least_aligned(forward);
    let right = normalize(cross(forward, helper));
    let up = cross(right, forward);
    [right, up, forward]
}

pub fn normal_offset(
    world: [f32; 3],
    normal: [f32; 3],
    toward_sun: [f32; 3],
    texel: f32,
    normal_bias: f32,
    slope_bias: f32,
) -> [f32; 3] {
    let normal = if length(normal) > 1.0e-6 {
        normalize(normal)
    } else {
        [0.0, 1.0, 0.0]
    };
    let toward = if length(toward_sun) > 1.0e-6 {
        normalize(toward_sun)
    } else {
        [0.0, 1.0, 0.0]
    };
    let ndotl = dot(normal, toward).clamp(0.0, 1.0);
    let slope = (1.0 - ndotl * ndotl).max(0.0).sqrt();
    add(
        world,
        scale(normal, texel * (normal_bias + slope_bias * slope)),
    )
}

pub fn frustum_corners(view: &View, near: f32, far: f32) -> [[f32; 3]; 8] {
    let [right, up, forward] = camera_axes(view);
    let (fov_y, aspect, _, _) = camera_limits(view);
    let tan_y = (fov_y * 0.5).tan();
    let tan_x = tan_y * aspect;
    let mut corners = [[0.0; 3]; 8];
    for (plane, distance) in [near, far].into_iter().enumerate() {
        let center = add(view.eye, scale(forward, distance));
        let hx = tan_x * distance;
        let hy = tan_y * distance;
        for (corner, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            corners[plane * 4 + corner] =
                add(center, add(scale(right, sx * hx), scale(up, sy * hy)));
        }
    }
    corners
}

pub fn camera_view_proj(view: &View) -> Mat4 {
    let [right, up, forward] = camera_axes(view);
    let (fov_y, aspect, near, far) = camera_limits(view);
    perspective(fov_y, aspect, near, far).multiply(view_matrix(right, up, forward, view.eye))
}

pub fn fit(view: &View, toward_sun: [f32; 3], quality: &Quality) -> Fit {
    let (fov_y, aspect, near, far) = camera_limits(view);
    let distances = splits(near, far, quality.lambda);
    let [_, _, forward] = camera_axes(view);
    let [light_right, light_up, light_forward] = light_basis(toward_sun);
    let resolution = quality.resolution.max(2);
    let light_view = view_matrix(light_right, light_up, light_forward, [0.0, 0.0, 0.0]);
    let mut cascades = [Cascade {
        near: distances[0],
        far: distances[1],
        view_proj: Mat4::identity(),
        texel: 1.0,
        light_center: [0.0; 3],
        snapped_center: [0.0; 3],
    }; CASCADE_COUNT];
    for (index, cascade) in cascades.iter_mut().enumerate() {
        let slice_near = distances[index];
        let slice_far = distances[index + 1];
        let fit_near = if index == 0 {
            slice_near
        } else {
            let previous = distances[index - 1];
            slice_near - (slice_near - previous) * finite(quality.blend).clamp(0.0, 1.0)
        };
        if let Some(receiver) = quality.receiver {
            let grow = if index == CASCADE_COUNT - 1 && outer(quality) {
                finite(quality.outer_scale)
            } else {
                1.0
            };
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for x in [receiver.min[0], receiver.max[0]] {
                for y in [receiver.min[1], receiver.max[1]] {
                    for z in [receiver.min[2], receiver.max[2]] {
                        let point = [x, y, z];
                        let light = [
                            dot(light_right, point),
                            dot(light_up, point),
                            -dot(light_forward, point),
                        ];
                        for axis in 0..3 {
                            lo[axis] = lo[axis].min(light[axis]);
                            hi[axis] = hi[axis].max(light[axis]);
                        }
                    }
                }
            }
            let center = [
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            ];
            if grow != 1.0 {
                for axis in 0..3 {
                    lo[axis] = center[axis] + (lo[axis] - center[axis]) * grow;
                    hi[axis] = center[axis] + (hi[axis] - center[axis]) * grow;
                }
            }
            let span_x = (hi[0] - lo[0]).max(1.0e-4);
            let span_y = (hi[1] - lo[1]).max(1.0e-4);
            let texel = span_x.max(span_y) / (resolution as f32 - 3.0).max(1.0);
            let snapped = [snap(center[0], texel), snap(center[1], texel), center[2]];
            let half_x = span_x * 0.5 + texel * 1.5;
            let half_y = span_y * 0.5 + texel * 1.5;
            let z_lo = lo[2] - texel;
            let z_hi = hi[2] + texel + quality.caster_margin.max(0.0);
            let projection = ortho(
                snapped[0] - half_x,
                snapped[0] + half_x,
                snapped[1] - half_y,
                snapped[1] + half_y,
                -z_hi,
                -z_lo,
            );
            *cascade = Cascade {
                near: slice_near,
                far: slice_far,
                view_proj: projection.multiply(light_view),
                texel,
                light_center: center,
                snapped_center: snapped,
            };
            continue;
        }
        let (center_distance, radius) = slice_sphere(fit_near, slice_far, fov_y, aspect);
        let center = add(view.eye, scale(forward, center_distance));
        let light_center = [
            dot(light_right, center),
            dot(light_up, center),
            -dot(light_forward, center),
        ];
        let texel = (2.0 * radius) / (resolution as f32 - 1.0).max(1.0);
        let half = radius + texel * 0.5;
        let snapped = [
            snap(light_center[0], texel),
            snap(light_center[1], texel),
            snap(light_center[2], texel),
        ];
        let margin = half + quality.caster_margin.max(0.0);
        let z_lo = snapped[2] - half;
        let z_hi = snapped[2] + margin;
        let projection = ortho(
            snapped[0] - half,
            snapped[0] + half,
            snapped[1] - half,
            snapped[1] + half,
            -z_hi,
            -z_lo,
        );
        *cascade = Cascade {
            near: slice_near,
            far: slice_far,
            view_proj: projection.multiply(light_view),
            texel,
            light_center,
            snapped_center: snapped,
        };
    }
    Fit {
        splits: distances,
        cascades,
        toward_sun: normalize(toward_sun),
        resolution,
    }
}

pub fn sample_uniform(fit: &Fit, quality: &Quality) -> [u8; SAMPLE_UNIFORM_BYTES] {
    let mut bytes = [0; SAMPLE_UNIFORM_BYTES];
    let radius = finite(quality.sun_radius_deg).clamp(0.0, 10.0);
    write_vec4(&mut bytes, 0, fit.splits);
    write_vec4(
        &mut bytes,
        16,
        [
            quality.pcf_radius.max(0.0),
            quality.normal_bias.max(0.0),
            quality.slope_bias.max(0.0),
            quality.blend.clamp(0.0, 1.0),
        ],
    );
    write_vec4(
        &mut bytes,
        32,
        [
            fit.toward_sun[0],
            fit.toward_sun[1],
            fit.toward_sun[2],
            quality.depth_bias.max(0.0),
        ],
    );
    write_vec4(
        &mut bytes,
        48,
        [
            fit.cascades[0].texel,
            fit.cascades[1].texel,
            fit.cascades[2].texel,
            0.0,
        ],
    );
    for (index, cascade) in fit.cascades.iter().enumerate() {
        let start = 64 + index * 64;
        bytes[start..start + 64].copy_from_slice(&cascade.view_proj.to_bytes());
    }
    let flags = u8::from(quality.nearest_blocker)
        | u8::from(quality.umbra_skip) << 1
        | u8::from(quality.blend_dither) << 2
        | u8::from(!quality.lit_skip) << 3
        | u8::from(quality.layered) << 4
        | u8::from(quality.receiver.is_some() && outer(quality)) << 5
        | u8::from(quality.splits_near_map()) << 6;
    write_vec4(&mut bytes, 272, [0.0, 0.0, f32::from(flags), 0.0]);
    if radius > 0.0 {
        write_vec4(
            &mut bytes,
            256,
            [
                radius.to_radians().tan(),
                SOFT_REACH_TEXELS,
                quality.blocker_taps.clamp(1, SOFT_TAP_LIMIT) as f32,
                SOFT_SLOPE_LIMIT,
            ],
        );
        write_vec4(
            &mut bytes,
            272,
            [
                quality.blocker_gathers.min(SOFT_TAP_LIMIT) as f32,
                quality.filter_taps.clamp(1, SOFT_TAP_LIMIT) as f32,
                f32::from(flags),
                finite(quality.soft_offset).max(0.0),
            ],
        );
    }
    bytes
}

impl Shadows {
    pub fn new(device: &wgpu::Device, quality: Quality) -> Self {
        let resolution = quality
            .resolution
            .clamp(2, device.limits().max_texture_dimension_2d);
        let quality = Quality {
            resolution,
            ..quality
        };
        let size = wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: CASCADE_COUNT as u32,
        };
        let atlas = depth_texture(device, size, "shadow atlas");
        let backup = depth_texture(device, size, "shadow static");
        let atlas_view = array_view(&atlas);
        let layers = [0, 1, 2].map(|layer| layer_view(&atlas, layer));
        let align = device.limits().min_uniform_buffer_offset_alignment.max(256);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow light"),
            size: u64::from(align) * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow light"),
            entries: &light_entries(),
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow light"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform,
                    offset: 0,
                    size: NonZeroU64::new(64),
                }),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow depth"),
            source: wgpu::ShaderSource::Wgsl(DEPTH_WGSL.into()),
        });
        let compare = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow compare"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow depth"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
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
                        attributes: &[
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
                        ],
                    },
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview: None,
            cache: None,
        });
        let creature_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("creature shadow data"),
            entries: &creature_entries(),
        });
        let creature_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("creature shadow depth"),
            source: wgpu::ShaderSource::Wgsl(CREATURE_DEPTH_WGSL.into()),
        });
        let creature_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("creature shadow depth"),
                bind_group_layouts: &[&bind_layout, &creature_layout],
                push_constant_ranges: &[],
            });
        let creature_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("creature shadow depth"),
            layout: Some(&creature_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &creature_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview: None,
            cache: None,
        });
        let instance_capacity = 256 * 64;
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow instances"),
            size: instance_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            quality: Cell::new(quality),
            resolution,
            atlas,
            atlas_view,
            layers,
            backup,
            pipeline,
            creature_pipeline,
            creature_layout,
            compare,
            bind,
            uniform,
            align,
            instances,
            instance_capacity,
            cache: None,
            atlas_clean: false,
            reused: false,
            transmission: None,
        }
    }

    pub fn fit(&self, view: &View, toward_sun: [f32; 3]) -> Fit {
        fit(view, toward_sun, &self.quality.get())
    }

    pub fn resolution(&self) -> u32 {
        self.resolution
    }

    pub fn quality(&self) -> Quality {
        self.quality.get()
    }

    pub fn set_quality(&self, quality: Quality) -> Result<(), String> {
        if quality.resolution != self.resolution {
            return Err("shadow resolution needs a new atlas".into());
        }
        self.quality.set(quality);
        Ok(())
    }

    pub fn update_quality(&mut self, device: &wgpu::Device, quality: Quality) {
        let resolution = quality
            .resolution
            .clamp(2, device.limits().max_texture_dimension_2d);
        if resolution != self.resolution {
            let size = wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: CASCADE_COUNT as u32,
            };
            self.atlas = depth_texture(device, size, "shadow atlas");
            self.backup = depth_texture(device, size, "shadow static");
            self.atlas_view = array_view(&self.atlas);
            self.layers = [0, 1, 2].map(|layer| layer_view(&self.atlas, layer));
            self.resolution = resolution;
            self.cache = None;
            self.atlas_clean = false;
            self.reused = false;
            self.transmission = None;
        }
        self.quality.set(Quality {
            resolution,
            ..quality
        });
    }

    pub fn atlas(&self) -> &wgpu::Texture {
        &self.atlas
    }

    pub fn atlas_view(&self) -> &wgpu::TextureView {
        &self.atlas_view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.compare
    }

    pub fn sample_uniform(&self, fit: &Fit) -> [u8; SAMPLE_UNIFORM_BYTES] {
        sample_uniform(fit, &self.quality.get())
    }

    pub fn reused_static(&self) -> bool {
        self.reused
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        fit: &Fit,
        casters: &[Caster<'_>],
    ) {
        self.render_timed(device, queue, encoder, fit, casters, None);
    }

    pub fn render_creatures(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        fit: &Fit,
        mesh: &CreatureMesh,
        creatures: &[CreatureInstance],
        time: f32,
    ) {
        let (vertices, indices, index_count) = mesh.shadow_buffers();
        if creatures.is_empty() || index_count == 0 {
            return;
        }
        for (index, cascade) in fit.cascades.iter().enumerate() {
            gpu.queue.write_buffer(
                &self.uniform,
                u64::from(self.align) * index as u64,
                &cascade.view_proj.to_bytes(),
            );
        }
        let instances = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("creature shadow instances"),
                contents: bytemuck::cast_slice(creatures),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let time = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("creature shadow time"),
                contents: bytemuck::cast_slice(&[time, 0.0_f32, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("creature shadow data"),
            layout: &self.creature_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: time.as_entire_binding(),
                },
            ],
        });
        for (index, layer) in self.layers.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("creature shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.creature_pipeline);
            pass.set_bind_group(0, &self.bind, &[self.align * index as u32]);
            pass.set_bind_group(1, &bind, &[]);
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..index_count, 0, 0..creatures.len() as u32);
        }
        self.atlas_clean = false;
    }

    pub fn render_timed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        fit: &Fit,
        casters: &[Caster<'_>],
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let key = static_key(fit, casters);
        let hit = self.cache.as_ref() == Some(&key);
        if hit && self.atlas_clean && !casters.iter().any(is_moving) {
            self.reused = true;
            return;
        }
        let mut bytes = Vec::new();
        let mut static_draws = Vec::new();
        let mut moving_draws = Vec::new();
        for caster in casters {
            if caster.index_count == 0 || caster.instances.is_empty() {
                continue;
            }
            if hit && !caster.moving {
                continue;
            }
            let offset = bytes.len() as u64;
            for instance in caster.instances {
                bytes.extend_from_slice(&instance.to_bytes());
            }
            let draw = Draw {
                positions: caster.positions,
                indices: caster.indices,
                first_index: caster.first_index,
                index_count: caster.index_count,
                instance_offset: offset,
                instance_count: u32::try_from(caster.instances.len()).unwrap_or(u32::MAX),
            };
            if caster.moving {
                moving_draws.push(draw);
            } else {
                static_draws.push(draw);
            }
        }
        self.ensure_instances(device, bytes.len() as u64);
        if !bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &bytes);
        }
        for (index, cascade) in fit.cascades.iter().enumerate() {
            queue.write_buffer(
                &self.uniform,
                u64::from(self.align) * index as u64,
                &cascade.view_proj.to_bytes(),
            );
        }
        self.reused = hit;
        if hit {
            if !self.atlas_clean {
                copy_depth(encoder, &self.backup, &self.atlas, self.resolution);
            }
            self.encode(
                encoder,
                &moving_draws,
                wgpu::LoadOp::Load,
                profiler.as_deref_mut(),
            );
            self.atlas_clean = moving_draws.is_empty();
            return;
        }
        self.encode(
            encoder,
            &static_draws,
            wgpu::LoadOp::Clear(1.0),
            profiler.as_deref_mut(),
        );
        copy_depth(encoder, &self.atlas, &self.backup, self.resolution);
        self.encode(encoder, &moving_draws, wgpu::LoadOp::Load, profiler);
        self.atlas_clean = moving_draws.is_empty();
        self.cache = Some(key);
    }

    pub fn render_with(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        fit: &Fit,
        content: u64,
        moving: bool,
        draw: &mut LayerDraw<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let mut key = static_key(fit, &[]);
        key.content = content;
        key.near_map = self.quality.get().splits_near_map();
        let hit = self.cache.as_ref() == Some(&key);
        if hit && self.atlas_clean && !moving {
            self.reused = true;
            return;
        }
        self.reused = hit;
        if hit {
            if !self.atlas_clean {
                copy_depth(encoder, &self.backup, &self.atlas, self.resolution);
            }
        } else {
            self.encode_layers(
                encoder,
                wgpu::LoadOp::Clear(1.0),
                true,
                draw,
                profiler.as_deref_mut(),
            );
            copy_depth(encoder, &self.atlas, &self.backup, self.resolution);
            self.cache = Some(key);
        }
        if moving {
            self.encode_layers(encoder, wgpu::LoadOp::Load, false, draw, profiler);
        }
        self.atlas_clean = !moving;
    }

    pub fn transmission_view(&self) -> Option<&wgpu::TextureView> {
        self.transmission
            .as_ref()
            .map(|transmission| &transmission.view)
    }

    pub fn transmission_texture(&self) -> Option<&wgpu::Texture> {
        self.transmission
            .as_ref()
            .map(|transmission| &transmission.map)
    }

    pub fn face_tiles(&self) -> FaceTiles {
        self.transmission
            .as_ref()
            .map_or_else(FaceTiles::default, |transmission| transmission.tiles)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_transmission(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        fit: &Fit,
        content: u64,
        moving: bool,
        faces: (usize, u32),
        draw: &mut LayerDraw<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        let size = transmission_size(self.resolution);
        let tiles = FaceTiles::new(faces.0, size, faces.1);
        let mut key = static_key(fit, &[]);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        tiles.hash(&mut hasher);
        key.content = hasher.finish();
        if self
            .transmission
            .as_ref()
            .is_none_or(|transmission| transmission.size != size || transmission.tiles != tiles)
        {
            self.transmission = Some(transmission_maps(device, size, tiles));
        }
        let Some(transmission) = self.transmission.as_mut() else {
            return;
        };
        if !moving && transmission.key.as_ref() == Some(&key) {
            return;
        }
        let timing = profiler
            .as_deref_mut()
            .and_then(|timer| timer.pass("shadow transmission"));
        for (index, (layer, depth)) in transmission
            .layers
            .iter()
            .zip(&transmission.depth_layers)
            .take(CASCADE_COUNT)
            .enumerate()
        {
            let writes = profiler
                .as_deref()
                .and_then(|timer| timer.render_writes(timing))
                .map(|writes| wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: writes
                        .beginning_of_pass_write_index
                        .filter(|_| index == 0),
                    end_of_pass_write_index: writes
                        .end_of_pass_write_index
                        .filter(|_| index + 1 == CASCADE_COUNT),
                })
                .filter(|writes| {
                    writes.beginning_of_pass_write_index.is_some()
                        || writes.end_of_pass_write_index.is_some()
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow transmission"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: layer,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(u32::MAX),
                            g: f64::from(1.0f32.to_bits()),
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: writes,
                occlusion_query_set: None,
            });
            draw(&mut pass, index, true);
            draw(&mut pass, index, false);
        }
        let light_timing = (tiles.faces > 0)
            .then(|| {
                profiler
                    .as_deref_mut()
                    .and_then(|timer| timer.pass("light transmission"))
            })
            .flatten();
        let extra = tiles.layers() as usize;
        for (index, (layer, depth)) in transmission
            .layers
            .iter()
            .zip(&transmission.depth_layers)
            .skip(CASCADE_COUNT)
            .enumerate()
        {
            let writes = profiler
                .as_deref()
                .and_then(|timer| timer.render_writes(light_timing))
                .map(|writes| wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: writes
                        .beginning_of_pass_write_index
                        .filter(|_| index == 0),
                    end_of_pass_write_index: writes
                        .end_of_pass_write_index
                        .filter(|_| index + 1 == extra),
                })
                .filter(|writes| {
                    writes.beginning_of_pass_write_index.is_some()
                        || writes.end_of_pass_write_index.is_some()
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("light transmission"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: layer,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(u32::MAX),
                            g: f64::from(1.0f32.to_bits()),
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: writes,
                occlusion_query_set: None,
            });
            let first = index as u32 * tiles.per_layer();
            for face in first..(first + tiles.per_layer()).min(tiles.faces) {
                let (_, origin) = tiles.place(face);
                pass.set_viewport(
                    origin[0] as f32,
                    origin[1] as f32,
                    tiles.tile as f32,
                    tiles.tile as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(origin[0], origin[1], tiles.tile, tiles.tile);
                draw(&mut pass, CASCADE_COUNT + face as usize, false);
            }
        }
        transmission.key = Some(key);
    }

    pub fn render_impostors(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        props: &crate::impostor::ImpostorPass,
        count: usize,
    ) -> Result<(), String> {
        if count == 0 {
            return Ok(());
        }
        for (index, layer) in self.layers.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impostor shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            props.draw_shadow(&mut pass, count, index)?;
        }
        self.atlas_clean = false;
        Ok(())
    }

    fn encode_layers(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<f32>,
        statics: bool,
        draw: &mut LayerDraw<'_>,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        for (index, layer) in self.layers.iter().enumerate() {
            let timing = profiler
                .as_deref_mut()
                .and_then(|timer| timer.pass(format!("shadow cascade {}", index + 1)));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer,
                    depth_ops: Some(wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|timer| timer.render_writes(timing)),
                occlusion_query_set: None,
            });
            draw(&mut pass, index, statics);
        }
    }

    fn ensure_instances(&mut self, device: &wgpu::Device, bytes: u64) {
        if bytes <= self.instance_capacity {
            return;
        }
        let size = bytes
            .max(self.instance_capacity.saturating_mul(2))
            .max(4096);
        self.instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_capacity = size;
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[Draw<'_>],
        load: wgpu::LoadOp<f32>,
        mut profiler: Option<&mut GpuProfiler>,
    ) {
        if matches!(load, wgpu::LoadOp::Load) && draws.is_empty() {
            return;
        }
        for (index, layer) in self.layers.iter().enumerate() {
            let timing = profiler
                .as_deref_mut()
                .and_then(|timer| timer.pass(format!("shadow cascade {}", index + 1)));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer,
                    depth_ops: Some(wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|timer| timer.render_writes(timing)),
                occlusion_query_set: None,
            });
            if draws.is_empty() {
                continue;
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[self.align * index as u32]);
            for draw in draws {
                let end = draw.instance_offset + u64::from(draw.instance_count) * 64;
                pass.set_vertex_buffer(0, draw.positions.slice(..));
                pass.set_vertex_buffer(1, self.instances.slice(draw.instance_offset..end));
                pass.set_index_buffer(draw.indices.slice(..), wgpu::IndexFormat::Uint32);
                let first = draw.first_index;
                pass.draw_indexed(first..first + draw.index_count, 0, 0..draw.instance_count);
            }
        }
    }
}

fn is_moving(caster: &Caster<'_>) -> bool {
    caster.moving && caster.index_count > 0 && !caster.instances.is_empty()
}

fn static_key(fit: &Fit, casters: &[Caster<'_>]) -> StaticKey {
    use std::collections::hash_map::DefaultHasher;
    let mut matrices = [[0; 16]; CASCADE_COUNT];
    for (index, cascade) in fit.cascades.iter().enumerate() {
        for (element, value) in cascade.view_proj.columns.iter().flatten().enumerate() {
            matrices[index][element] = value.to_bits();
        }
    }
    let mut draws = Vec::new();
    for caster in casters {
        if caster.moving || caster.index_count == 0 || caster.instances.is_empty() {
            continue;
        }
        let mut positions = DefaultHasher::new();
        caster.positions.hash(&mut positions);
        let mut indices = DefaultHasher::new();
        caster.indices.hash(&mut indices);
        let mut instance_bits = Vec::with_capacity(caster.instances.len() * 16);
        for instance in caster.instances {
            instance_bits.extend(
                instance
                    .columns
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits()),
            );
        }
        draws.push(StaticDraw {
            positions: positions.finish(),
            indices: indices.finish(),
            first_index: caster.first_index,
            index_count: caster.index_count,
            generation: caster.generation,
            instances: instance_bits,
        });
    }
    StaticKey {
        sun: [
            fit.toward_sun[0].to_bits(),
            fit.toward_sun[1].to_bits(),
            fit.toward_sun[2].to_bits(),
        ],
        matrices,
        draws,
        content: 0,
        near_map: false,
    }
}

fn transmission_maps(device: &wgpu::Device, size: u32, tiles: FaceTiles) -> Transmission {
    let count = CASCADE_COUNT as u32 + tiles.layers();
    let extent = wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: count,
    };
    let map = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow transmission"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TRANSMISSION_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow transmission depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let colour_view = |dimension, base_array_layer, count| {
        map.create_view(&wgpu::TextureViewDescriptor {
            label: Some("shadow transmission"),
            dimension: Some(dimension),
            base_array_layer,
            array_layer_count: Some(count),
            ..Default::default()
        })
    };
    Transmission {
        size,
        tiles,
        view: colour_view(wgpu::TextureViewDimension::D2Array, 0, count),
        layers: (0..count)
            .map(|layer| colour_view(wgpu::TextureViewDimension::D2, layer, 1))
            .collect(),
        depth_layers: (0..count).map(|layer| layer_view(&depth, layer)).collect(),
        map,
        _depth: depth,
        key: None,
    }
}

fn depth_texture(device: &wgpu::Device, size: wgpu::Extent3d, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("shadow array"),
        format: Some(DEPTH_FORMAT),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: Some(CASCADE_COUNT as u32),
    })
}

fn layer_view(texture: &wgpu::Texture, layer: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("shadow layer"),
        format: Some(DEPTH_FORMAT),
        dimension: Some(wgpu::TextureViewDimension::D2),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: layer,
        array_layer_count: Some(1),
    })
}

fn copy_depth(
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    destination: &wgpu::Texture,
    resolution: u32,
) {
    encoder.copy_texture_to_texture(
        source.as_image_copy(),
        destination.as_image_copy(),
        wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: CASCADE_COUNT as u32,
        },
    );
}

fn write_vec4(bytes: &mut [u8], offset: usize, value: [f32; 4]) {
    for (index, component) in value.iter().enumerate() {
        let start = offset + index * 4;
        bytes[start..start + 4].copy_from_slice(&component.to_le_bytes());
    }
}

fn camera_limits(view: &View) -> (f32, f32, f32, f32) {
    let near = finite(view.near).max(1.0e-4);
    let far = finite(view.far).max(near + 1.0e-3);
    let fov_y = finite(view.fov_y).clamp(1.0e-3, std::f32::consts::PI - 1.0e-3);
    let aspect = finite(view.aspect).max(1.0e-4);
    (fov_y, aspect, near, far)
}

fn camera_axes(view: &View) -> [[f32; 3]; 3] {
    let forward = normalize(view.forward);
    let mut right = cross(forward, view.up);
    if length(right) < 1.0e-6 {
        right = cross(forward, least_aligned(forward));
    }
    let right = normalize(right);
    let up = cross(right, forward);
    [right, up, forward]
}

fn slice_sphere(near: f32, far: f32, fov_y: f32, aspect: f32) -> (f32, f32) {
    let tan_y = (fov_y * 0.5).tan();
    let tan_x = tan_y * aspect;
    let tan_corner = (tan_x * tan_x + tan_y * tan_y).sqrt();
    let factor = 1.0 + tan_corner * tan_corner;
    let center = (far + near) * 0.5 * factor;
    let far_radius = far * tan_corner;
    let radius = ((center - far) * (center - far) + far_radius * far_radius).sqrt();
    (center, radius.max(1.0e-4))
}

fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let y = 1.0 / (fov_y * 0.5).tan();
    let span = near - far;
    Mat4::from_columns([
        [y / aspect, 0.0, 0.0, 0.0],
        [0.0, y, 0.0, 0.0],
        [0.0, 0.0, far / span, -1.0],
        [0.0, 0.0, near * far / span, 0.0],
    ])
}

fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    let width = right - left;
    let height = top - bottom;
    let depth = far - near;
    Mat4::from_columns([
        [2.0 / width, 0.0, 0.0, 0.0],
        [0.0, 2.0 / height, 0.0, 0.0],
        [0.0, 0.0, -1.0 / depth, 0.0],
        [
            -(right + left) / width,
            -(top + bottom) / height,
            -near / depth,
            1.0,
        ],
    ])
}

fn view_matrix(right: [f32; 3], up: [f32; 3], forward: [f32; 3], eye: [f32; 3]) -> Mat4 {
    Mat4::from_columns([
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ])
}

fn snap(value: f32, texel: f32) -> f32 {
    (value / texel).round() * texel
}

fn least_aligned(direction: [f32; 3]) -> [f32; 3] {
    let ax = direction[0].abs();
    let ay = direction[1].abs();
    let az = direction[2].abs();
    if ax <= ay && ax <= az {
        [1.0, 0.0, 0.0]
    } else if ay <= az {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn length(value: [f32; 3]) -> f32 {
    dot(value, value).sqrt()
}

fn normalize(value: [f32; 3]) -> [f32; 3] {
    let len = length(value);
    if len < 1.0e-8 {
        [0.0, 1.0, 0.0]
    } else {
        scale(value, 1.0 / len)
    }
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn add(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn scale(value: [f32; 3], factor: f32) -> [f32; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

pub fn light_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(64),
        },
        count: None,
    }]
}

pub fn creature_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::CreatureVertex;

    #[test]
    fn light_face_tiles_pack_into_layers_after_the_cascades() {
        let tiles = FaceTiles::new(12, 1024, 1024);
        assert_eq!((tiles.tile, tiles.columns, tiles.layers()), (256, 4, 1));
        assert_eq!(tiles.place(0), (CASCADE_COUNT as u32, [0, 0]));
        assert_eq!(tiles.place(5), (CASCADE_COUNT as u32, [256, 256]));
        assert_eq!(tiles.place(11), (CASCADE_COUNT as u32, [768, 512]));
        let large = FaceTiles::new(24, 512, 4096);
        assert_eq!((large.tile, large.columns, large.layers()), (512, 1, 24));
        assert_eq!(large.place(23), (CASCADE_COUNT as u32 + 23, [0, 0]));
        let spill = FaceTiles::new(24, 512, 1024);
        assert_eq!((spill.tile, spill.columns, spill.layers()), (256, 2, 6));
        assert_eq!(spill.place(7), (CASCADE_COUNT as u32 + 1, [256, 256]));
        assert_eq!(FaceTiles::new(0, 1024, 1024).layers(), 0);
        assert_eq!(FaceTiles::new(0, 1024, 1024), FaceTiles::default());
    }

    #[test]
    fn receiver_box_increases_shadow_texel_density() {
        let view = View {
            eye: [0.0, 0.2, 1.6],
            forward: [0.0, -0.1, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y: 0.5,
            aspect: 16.0 / 9.0,
            near: 0.4,
            far: 6.0,
        };
        let sun = [0.3, 0.8, 0.5];
        let default = fit(&view, sun, &Quality::default());
        let quality = Quality {
            resolution: 4096,
            receiver: Some(ReceiverBox {
                min: [-0.4, -0.25, -0.2],
                max: [0.4, 0.25, 0.35],
            }),
            caster_margin: 3.0,
            ..Quality::default()
        };
        let fitted = fit(&view, sun, &quality);
        let box_bound = quality.receiver.unwrap();
        let outer = fitted.cascades[CASCADE_COUNT - 1].texel / fitted.cascades[0].texel;
        assert!((outer - OUTER_SCALE).abs() < 1.0e-3, "{outer}");
        for (index, cascade) in fitted.cascades.iter().enumerate() {
            let texel = if index == CASCADE_COUNT - 1 {
                cascade.texel / OUTER_SCALE
            } else {
                cascade.texel
            };
            assert!(texel < default.cascades[2].texel * 0.5);
            assert!(texel < 0.0003);
            for x in [box_bound.min[0], box_bound.max[0]] {
                for y in [box_bound.min[1], box_bound.max[1]] {
                    for z in [box_bound.min[2], box_bound.max[2]] {
                        let point = cascade.view_proj.transform_point([x, y, z]);
                        assert!(point[0].abs() <= 1.0 && point[1].abs() <= 1.0);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn quality_change_reallocates_only_for_resolution() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let mut shadows = Shadows::new(
            &gpu.device,
            Quality {
                resolution: 64,
                ..Quality::default()
            },
        );
        let atlas = shadows.atlas.clone();
        let backup = shadows.backup.clone();
        shadows.update_quality(
            &gpu.device,
            Quality {
                resolution: 64,
                lambda: 0.8,
                ..Quality::default()
            },
        );
        assert_eq!(shadows.atlas, atlas);
        assert_eq!(shadows.backup, backup);
        assert_eq!(shadows.quality().lambda, 0.8);
        shadows.update_quality(
            &gpu.device,
            Quality {
                resolution: 128,
                ..Quality::default()
            },
        );
        assert_ne!(shadows.atlas, atlas);
        assert_ne!(shadows.backup, backup);
        assert_eq!(shadows.resolution(), 128);
        assert_eq!(shadows.atlas.size().width, 128);
        assert_eq!(shadows.backup.size().width, 128);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn bird_depth_appears_in_shadow_atlas() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let quality = Quality {
            resolution: 64,
            receiver: Some(ReceiverBox {
                min: [-1.0, -0.1, -1.0],
                max: [1.0, 0.1, 1.0],
            }),
            caster_margin: 2.0,
            ..Quality::default()
        };
        let mut shadows = Shadows::new(
            &gpu.device,
            Quality {
                resolution: 64,
                ..Quality::default()
            },
        );
        shadows.set_quality(quality).unwrap();
        let view = View {
            eye: [0.0, 2.0, 3.0],
            forward: [0.0, -0.5, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y: 1.0,
            aspect: 1.0,
            near: 0.1,
            far: 6.0,
        };
        let fit = shadows.fit(&view, [0.0, 1.0, 0.0]);
        let mesh = CreatureMesh::new(
            &gpu.device,
            &[
                CreatureVertex {
                    position: [-0.4, 0.0, -0.4, 0.0],
                },
                CreatureVertex {
                    position: [0.4, 0.0, -0.4, 0.0],
                },
                CreatureVertex {
                    position: [0.0, 0.0, 0.4, 0.0],
                },
            ],
            &[0, 1, 2],
        );
        let bird = CreatureInstance {
            position_scale: [0.0, 1.0, 0.0, 1.0],
            right: [1.0, 0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0, 0.0],
            forward: [0.0, 0.0, 1.0, 0.0],
            motion: [0.0; 4],
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        shadows.render(&gpu.device, &gpu.queue, &mut encoder, &fit, &[]);
        shadows.render_creatures(&gpu, &mut encoder, &fit, &mesh, &[bird], 0.0);
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bird shadow readback"),
            size: 64 * 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: shadows.atlas(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
            },
            wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        let at = (32 * 256 + 32 * 4) as usize;
        let depth = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert!(depth < 1.0, "{depth}");
    }

    fn desk() -> View {
        View {
            eye: [0.0, 1.6, 6.0],
            forward: [0.0, -0.15, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y: 50.0_f32.to_radians(),
            aspect: 16.0 / 9.0,
            near: 0.2,
            far: 40.0,
        }
    }

    fn overhead() -> View {
        View {
            eye: [0.0, 8.0, 0.2],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            fov_y: 45.0_f32.to_radians(),
            aspect: 1.0,
            near: 0.5,
            far: 30.0,
        }
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() <= 1.0e-4 * left.abs().max(right.abs()).max(1.0)
    }

    #[test]
    fn splits_cover_near_to_far_without_gaps() {
        for (near, far, lambda) in [(0.1, 100.0, 0.5), (0.05, 0.2, 0.0), (0.2, 10_000.0, 1.0)] {
            let distances = splits(near, far, lambda);
            assert!(close(distances[0], near));
            assert!(close(distances[3], far));
            for pair in distances.windows(2) {
                assert!(pair[0] < pair[1], "{distances:?}");
            }
            let uniform = splits(near, far, 0.0);
            let logarithmic = splits(near, far, 1.0);
            let practical = splits(near, far, 0.5);
            for (index, ((uniform_value, log_value), practical_value)) in uniform
                .iter()
                .zip(logarithmic.iter())
                .zip(practical.iter())
                .enumerate()
            {
                let portion = index as f32 / CASCADE_COUNT as f32;
                let expected_uniform = near + (far - near) * portion;
                let expected_log = near * (far / near).powf(portion);
                assert!(close(*uniform_value, expected_uniform), "{uniform:?}");
                assert!(
                    close(*log_value, expected_log),
                    "{logarithmic:?} {expected_log}"
                );
                assert!(close(
                    *practical_value,
                    0.5 * expected_log + 0.5 * expected_uniform
                ));
            }
            let mut previous = 0;
            for step in 0..=2000 {
                let t = step as f32 / 2000.0;
                let distance = distances[0] + (distances[3] - distances[0]) * t;
                let choice = select_cascade(distance, distances, 0.1);
                assert!(choice.cascade < CASCADE_COUNT);
                assert!(choice.cascade == previous || choice.cascade == previous + 1);
                assert!(choice.next == choice.cascade || choice.next == choice.cascade + 1);
                let start = distances[choice.cascade];
                let end = distances[choice.cascade + 1];
                assert!(
                    distance + 1.0e-3 >= start && distance <= end + 1.0e-3,
                    "{distance} {choice:?} {distances:?}"
                );
                previous = choice.cascade;
            }
        }
        let distances = splits(1.0, 90.0, 0.5);
        assert_eq!(select_cascade(distances[1], distances, 0.0).cascade, 0);
        assert_eq!(
            select_cascade(distances[1].next_up(), distances, 0.0).cascade,
            1
        );
        assert_eq!(select_cascade(distances[3], distances, 0.0).cascade, 2);
        let end = distances[1];
        let band = (end - distances[0]) * 0.1;
        let at_end = select_cascade(end, distances, 0.1);
        assert_eq!(at_end.cascade, 0);
        assert_eq!(at_end.next, 1);
        assert!(close(at_end.blend, 1.0));
        let mid = (distances[0] + end - band) * 0.5;
        assert_eq!(select_cascade(mid, distances, 0.1).blend, 0.0);
    }

    #[test]
    fn light_matrices_contain_each_cascades_corners() {
        let quality = Quality {
            resolution: 256,
            ..Quality::default()
        };
        let views = [desk(), overhead()];
        let suns = [[0.0, 1.0, 0.0], [0.35, 0.85, 0.2], [1.0, 0.18, -0.2]];
        for view in views {
            let projection = camera_view_proj(&view);
            for corner in frustum_corners(&view, view.near, view.far) {
                let ndc = projection.transform_point(corner);
                assert!(
                    ndc[0].abs() <= 1.0 + 1.0e-3
                        && ndc[1].abs() <= 1.0 + 1.0e-3
                        && (-1.0e-3..=1.0 + 1.0e-3).contains(&ndc[2]),
                    "{ndc:?}"
                );
            }
            for sun in suns {
                let fitted = fit(&view, sun, &quality);
                assert_eq!(fitted.cascades.len(), CASCADE_COUNT);
                for (index, cascade) in fitted.cascades.iter().enumerate() {
                    assert!(close(cascade.near, fitted.splits[index]));
                    assert!(close(cascade.far, fitted.splits[index + 1]));
                    let fit_near = if index == 0 {
                        cascade.near
                    } else {
                        cascade.near - (cascade.near - fitted.splits[index - 1]) * quality.blend
                    };
                    for corner in frustum_corners(&view, fit_near, cascade.far) {
                        let ndc = cascade.view_proj.transform_point(corner);
                        assert!(
                            ndc[0].abs() <= 1.0 + 2.0e-3
                                && ndc[1].abs() <= 1.0 + 2.0e-3
                                && (-2.0e-3..=1.0 + 2.0e-3).contains(&ndc[2]),
                            "cascade {index} {ndc:?} sun {sun:?}"
                        );
                    }
                    let [right, up, forward] = light_basis(sun);
                    let snapped = cascade.snapped_center;
                    let world = add(
                        add(scale(right, snapped[0]), scale(up, snapped[1])),
                        scale(forward, -snapped[2]),
                    );
                    let origin = cascade.view_proj.transform_point(world);
                    let stepped = cascade
                        .view_proj
                        .transform_point(add(world, scale(right, cascade.texel)));
                    assert!(origin[0].abs() < 1.0e-3, "{origin:?}");
                    assert!(origin[1].abs() < 1.0e-3, "{origin:?}");
                    let expected = 2.0 / fitted.resolution as f32;
                    assert!(
                        (stepped[0] - origin[0] - expected).abs() < 2.0e-3,
                        "{} {expected}",
                        stepped[0] - origin[0]
                    );
                }
            }
        }
    }

    #[test]
    fn texel_snap_holds_under_a_small_camera_move() {
        let quality = Quality {
            resolution: 128,
            ..Quality::default()
        };
        let sun = [0.25, 0.9, 0.15];
        let view = desk();
        let before = fit(&view, sun, &quality);
        let [right, _, _] = light_basis(sun);
        let mut slack = f32::MAX;
        for cascade in &before.cascades {
            let error = (cascade.light_center[0] - cascade.snapped_center[0]).abs();
            assert!(error <= cascade.texel * 0.5 + 1.0e-4, "{error}");
            slack = slack.min(cascade.texel * 0.5 - error);
        }
        assert!(slack > 0.0, "{slack}");
        let mut moved = view;
        moved.eye = add(moved.eye, scale(right, slack * 0.5));
        let after = fit(&moved, sun, &quality);
        for (first, second) in before.cascades.iter().zip(after.cascades.iter()) {
            assert_eq!(first.view_proj, second.view_proj);
            assert_eq!(first.snapped_center, second.snapped_center);
        }
        let mut jumped = view;
        jumped.eye = add(jumped.eye, scale(right, before.cascades[0].texel * 3.0));
        let changed = fit(&jumped, sun, &quality);
        assert_ne!(
            before.cascades[0].snapped_center[0],
            changed.cascades[0].snapped_center[0]
        );
    }

    #[test]
    fn sampling_shader_names_its_entry_points() {
        for name in [
            "shadow_select",
            "shadow_pcf",
            "shadow_blend",
            "shadow_sample",
        ] {
            assert!(
                SHADOW_WGSL.contains(&format!("fn {name}(")),
                "{name} missing"
            );
        }
        assert!(SHADOW_WGSL.contains("textureSampleCompareLevel"));
        assert!(SHADOW_WGSL.contains("shadow_offset"));
        assert_eq!(Quality::default().resolution, DEFAULT_RESOLUTION);
        assert_eq!(DEFAULT_RESOLUTION, 4096);
    }

    #[test]
    fn sample_uniform_packs_splits_matrices_and_bias() {
        let quality = Quality {
            resolution: 64,
            pcf_radius: 1.5,
            normal_bias: 0.75,
            slope_bias: 2.5,
            depth_bias: 0.002,
            blend: 0.2,
            ..Quality::default()
        };
        let fitted = fit(&desk(), [0.2, 1.0, 0.1], &quality);
        let bytes = sample_uniform(&fitted, &quality);
        for (index, split) in fitted.splits.iter().enumerate() {
            let start = index * 4;
            let value = f32::from_le_bytes(bytes[start..start + 4].try_into().unwrap());
            assert_eq!(value.to_bits(), split.to_bits());
        }
        let radius = f32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let normal = f32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let slope = f32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let blend = f32::from_le_bytes(bytes[28..32].try_into().unwrap());
        assert_eq!(radius, 1.5);
        assert_eq!(normal, 0.75);
        assert_eq!(slope, 2.5);
        assert_eq!(blend, 0.2);
        for (index, cascade) in fitted.cascades.iter().enumerate() {
            let start = 64 + index * 64;
            assert_eq!(&bytes[start..start + 64], &cascade.view_proj.to_bytes());
        }
        let flat = normal_offset(
            [1.0, 2.0, 3.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            0.2,
            1.0,
            2.0,
        );
        assert!(close(flat[0], 1.0) && close(flat[1], 2.2) && close(flat[2], 3.0));
        let grazing = normal_offset(
            [1.0, 2.0, 3.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            0.2,
            1.0,
            2.0,
        );
        assert!(close(grazing[0], 1.6) && close(grazing[1], 2.0));
    }

    #[test]
    fn sample_uniform_packs_the_soft_sun_taps() {
        let read =
            |bytes: &[u8], at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let hard = Quality::default();
        let fitted = fit(&desk(), [0.2, 1.0, 0.1], &hard);
        let bytes = sample_uniform(&fitted, &hard);
        assert!(bytes[256..280].iter().all(|byte| *byte == 0));
        assert_eq!(read(&bytes, 280), 7.0);
        let soft = Quality {
            sun_radius_deg: 0.8,
            blocker_taps: 12,
            filter_taps: 200,
            blocker_gathers: 6,
            nearest_blocker: false,
            blend_dither: false,
            ..Quality::default()
        };
        let bytes = sample_uniform(&fitted, &soft);
        assert!(close(read(&bytes, 256), 0.8_f32.to_radians().tan()));
        assert_eq!(read(&bytes, 264), 12.0);
        assert_eq!(read(&bytes, 272), 6.0);
        assert_eq!(read(&bytes, 276), SOFT_TAP_LIMIT as f32);
        assert_eq!(read(&bytes, 280), 2.0);
        let shipped = sample_uniform(
            &fitted,
            &Quality {
                sun_radius_deg: 0.8,
                ..Quality::default()
            },
        );
        assert_eq!(read(&shipped, 264), BLOCKER_TAPS as f32);
        assert_eq!(read(&shipped, 276), FILTER_TAPS as f32);
        assert_eq!(read(&shipped, 272), BLOCKER_GATHERS as f32);
        assert_eq!(read(&shipped, 280), 7.0);
    }

    #[test]
    fn the_outer_cascade_grows_the_receiver_box() {
        let read =
            |bytes: &[u8], at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let boxed = Quality {
            resolution: 1024,
            receiver: Some(ReceiverBox {
                min: [-0.5, -0.1, -0.3],
                max: [0.5, 0.4, 0.3],
            }),
            ..Quality::default()
        };
        let sun = [0.45, 0.62, 0.64];
        let grown = fit(&desk(), sun, &boxed);
        assert_eq!(grown.cascades[0].view_proj, grown.cascades[1].view_proj);
        let ratio = grown.cascades[2].texel / grown.cascades[0].texel;
        assert!((ratio - OUTER_SCALE).abs() < 1.0e-3, "{ratio}");
        assert_eq!(
            grown.cascades[2].snapped_center[2],
            grown.cascades[0].snapped_center[2]
        );
        let [right, _, _] = light_basis(sun);
        let receiver = boxed.receiver.unwrap();
        let centre: [f32; 3] = std::array::from_fn(|k| (receiver.min[k] + receiver.max[k]) * 0.5);
        let mut half = 0.0f32;
        for x in [receiver.min[0], receiver.max[0]] {
            for y in [receiver.min[1], receiver.max[1]] {
                for z in [receiver.min[2], receiver.max[2]] {
                    half = half.max(dot(right, add([x, y, z], scale(centre, -1.0))).abs());
                }
            }
        }
        let outside = add(centre, scale(right, half * OUTER_SCALE * 0.8));
        let inner = grown.cascades[0].view_proj.transform_point(outside);
        let outer = grown.cascades[2].view_proj.transform_point(outside);
        assert!(inner[0].abs() > 1.0, "{inner:?}");
        assert!(
            outer[0].abs() <= 1.0 && (0.0..=1.0).contains(&outer[2]),
            "{outer:?}"
        );
        assert_eq!(read(&sample_uniform(&grown, &boxed), 280), 103.0);
        let off = Quality {
            outer_scale: 0.0,
            ..boxed
        };
        let same = fit(&desk(), sun, &off);
        assert_eq!(same.cascades[0].view_proj, same.cascades[2].view_proj);
        assert_eq!(same.cascades[0], grown.cascades[0]);
        assert_eq!(read(&sample_uniform(&same, &off), 280), 7.0);
        let free = fit(&desk(), sun, &Quality::default());
        assert_eq!(read(&sample_uniform(&free, &Quality::default()), 280), 7.0);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn cube_shadows_a_plane() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let quality = Quality {
            resolution: 256,
            depth_bias: 0.0004,
            normal_bias: 0.5,
            slope_bias: 1.0,
            ..Quality::default()
        };
        let mut shadows = Shadows::new(&gpu.device, quality);
        let eye_y = 6.0_f32;
        let view = View {
            eye: [0.0, eye_y, 0.0],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            fov_y: 2.0 * (1.0 / eye_y).atan(),
            aspect: 1.0,
            near: 1.0,
            far: 12.0,
        };
        let sun = [0.0, 1.0, 0.0];
        let fitted = shadows.fit(&view, sun);
        let plane = select_cascade(eye_y, fitted.splits, quality.blend);
        assert_eq!(plane.cascade, 1, "{:?}", fitted.splits);
        assert_eq!(plane.blend, 0.0);
        let static_model = Mat4::from_columns([
            [0.4, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.3, 0.0],
            [0.35, 0.7, 0.25, 1.0],
        ]);
        let moving_a = Mat4::from_columns([
            [0.3, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.3, 0.0],
            [-0.4, 0.7, -0.25, 1.0],
        ]);
        let moving_b = Mat4::from_columns([
            [0.3, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.3, 0.0],
            [-0.4, 0.7, 0.35, 1.0],
        ]);
        for model in [static_model, moving_a, moving_b] {
            for sx in [-0.5_f32, 0.5] {
                for sy in [-0.5_f32, 0.5] {
                    for sz in [-0.5_f32, 0.5] {
                        let world = model.transform_point([sx, sy, sz]);
                        let ndc = fitted.cascades[1].view_proj.transform_point(world);
                        assert!(
                            ndc[0].abs() <= 1.0
                                && ndc[1].abs() <= 1.0
                                && (0.0..=1.0).contains(&ndc[2]),
                            "{world:?} {ndc:?}"
                        );
                    }
                }
            }
        }
        let (positions, indices) = unit_cube();
        let position_buffer = upload(
            &gpu.device,
            &gpu.queue,
            &positions,
            wgpu::BufferUsages::VERTEX,
        );
        let index_buffer = upload(&gpu.device, &gpu.queue, &indices, wgpu::BufferUsages::INDEX);
        let static_instances = [static_model];
        let moving_instances_a = [moving_a];
        let moving_instances_b = [moving_b];
        let count = render_receiver(
            &gpu,
            &mut shadows,
            &fitted,
            &position_buffer,
            &index_buffer,
            &static_instances,
            &[],
        );
        assert!(!shadows.reused_static());
        assert_shadow(&count, static_bounds(), None, "static");
        let again = render_receiver(
            &gpu,
            &mut shadows,
            &fitted,
            &position_buffer,
            &index_buffer,
            &static_instances,
            &[],
        );
        assert!(shadows.reused_static());
        assert_eq!(again.shadowed, count.shadowed);
        let with_moving = render_receiver(
            &gpu,
            &mut shadows,
            &fitted,
            &position_buffer,
            &index_buffer,
            &static_instances,
            &moving_instances_a,
        );
        assert!(shadows.reused_static());
        assert_shadow(
            &with_moving,
            static_bounds(),
            Some(bounds_of(&moving_a)),
            "moving a",
        );
        let moved = render_receiver(
            &gpu,
            &mut shadows,
            &fitted,
            &position_buffer,
            &index_buffer,
            &static_instances,
            &moving_instances_b,
        );
        assert!(shadows.reused_static());
        assert_shadow(
            &moved,
            static_bounds(),
            Some(bounds_of(&moving_b)),
            "moving b",
        );
        let old = bounds_of(&moving_a);
        assert!(
            !point_shadowed(&moved, (old.0 + old.1) * 0.5, (old.2 + old.3) * 0.5),
            "stale moving shadow\n{}",
            ascii(&moved.pixels)
        );
        println!(
            "shadowed texels: static {} with moving {} after move {}",
            count.shadowed, with_moving.shadowed, moved.shadowed
        );
    }

    struct Shot {
        pixels: Vec<u8>,
        shadowed: u32,
    }

    fn render_receiver(
        gpu: &pfx_gpu::Gpu,
        shadows: &mut Shadows,
        fitted: &Fit,
        positions: &wgpu::Buffer,
        indices: &wgpu::Buffer,
        static_instances: &[Mat4],
        moving_instances: &[Mat4],
    ) -> Shot {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let static_caster = Caster {
            positions,
            indices,
            first_index: 0,
            index_count: 36,
            instances: static_instances,
            moving: false,
            generation: 0,
        };
        let moving_caster = Caster {
            positions,
            indices,
            first_index: 0,
            index_count: 36,
            instances: moving_instances,
            moving: true,
            generation: 0,
        };
        let casters = if moving_instances.is_empty() {
            vec![static_caster]
        } else {
            vec![static_caster, moving_caster]
        };
        shadows.render(&gpu.device, &gpu.queue, &mut encoder, fitted, &casters);
        let uniform_bytes = shadows.sample_uniform(fitted);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow sample"),
            size: SAMPLE_UNIFORM_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&uniform, 0, &uniform_bytes);
        let sampler = shadows.sampler();
        let bind_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow receiver"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(SAMPLE_UNIFORM_BYTES as u64),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let gather = gpu
            .device
            .create_sampler(&wgpu::SamplerDescriptor::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow receiver"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&gather),
                },
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(shadows.atlas_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let source = format!(
            "{SHADOW_WGSL}\n\
@group(0) @binding(0) var map: texture_depth_2d_array;\n\
@group(0) @binding(1) var samp: sampler_comparison;\n\
@group(0) @binding(2) var<uniform> data: ShadowSample;\n\
@group(0) @binding(3) var gather: sampler;\n\
@vertex\n\
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {{\n\
    var positions = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));\n\
    return vec4f(positions[index], 0.0, 1.0);\n\
}}\n\
@fragment\n\
fn fs(@builtin(position) pixel: vec4f) -> @location(0) vec4f {{\n\
    let uv = pixel.xy / vec2f(64.0, 64.0);\n\
    let world = vec3f(-1.0 + uv.x * 2.0, 0.0, -1.0 + uv.y * 2.0);\n\
    let distance = dot(world - vec3f(0.0, 6.0, 0.0), vec3f(0.0, -1.0, 0.0));\n\
    let lit = shadow_sample(map, samp, gather, data, world, vec3f(0.0, 1.0, 0.0), distance);\n\
    return vec4f(1.0 - lit, 0.0, 0.0, 1.0);\n\
}}\n"
        );
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shadow receiver"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow receiver"),
                bind_group_layouts: &[&bind_layout],
                push_constant_ranges: &[],
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shadow receiver"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });
        let color = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow receiver"),
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
        let view = color.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow receiver"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let row = 256_u32;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow readback"),
            size: u64::from(row) * 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(64),
                },
            },
            color.size(),
        );
        gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity(64 * 64 * 4);
        for padded in mapped.chunks_exact(row as usize) {
            pixels.extend_from_slice(&padded[..256]);
        }
        drop(mapped);
        buffer.unmap();
        let shadowed = pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[0] > 127)
            .count() as u32;
        Shot { pixels, shadowed }
    }

    fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow mesh"),
            size: bytes.len() as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytes);
        buffer
    }

    fn unit_cube() -> (Vec<u8>, Vec<u8>) {
        let vertices = [
            [-0.5_f32, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ];
        let indices: [u32; 36] = [
            0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 5, 1, 0, 4, 5, 3, 2, 6, 3, 6, 7, 0, 3, 7, 0, 7,
            4, 1, 5, 6, 1, 6, 2,
        ];
        let mut positions = Vec::new();
        for vertex in vertices {
            for value in vertex {
                positions.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut packed = Vec::new();
        for index in indices {
            packed.extend_from_slice(&index.to_le_bytes());
        }
        (positions, packed)
    }

    fn bounds_of(model: &Mat4) -> (f32, f32, f32, f32) {
        let min = model.transform_point([-0.5, 0.0, -0.5]);
        let max = model.transform_point([0.5, 0.0, 0.5]);
        (min[0], max[0], min[2], max[2])
    }

    fn static_bounds() -> (f32, f32, f32, f32) {
        bounds_of(&Mat4::from_columns([
            [0.4, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.3, 0.0],
            [0.35, 0.7, 0.25, 1.0],
        ]))
    }

    fn world_at(i: u32, j: u32) -> (f32, f32) {
        (
            -1.0 + (i as f32 + 0.5) / 32.0,
            -1.0 + (j as f32 + 0.5) / 32.0,
        )
    }

    fn point_shadowed(shot: &Shot, x: f32, z: f32) -> bool {
        let i = ((x + 1.0) * 32.0 - 0.5).round().clamp(0.0, 63.0) as u32;
        let j = ((z + 1.0) * 32.0 - 0.5).round().clamp(0.0, 63.0) as u32;
        shot.pixels[((j * 64 + i) * 4) as usize] > 127
    }

    fn assert_shadow(
        shot: &Shot,
        stays: (f32, f32, f32, f32),
        extra: Option<(f32, f32, f32, f32)>,
        label: &str,
    ) {
        let margin = 0.08;
        let mut inside = 0_u32;
        let mut inside_shadow = 0_u32;
        let mut outside_shadow = 0_u32;
        for j in 0..64 {
            for i in 0..64 {
                let (x, z) = world_at(i, j);
                let in_static = x > stays.0 + margin
                    && x < stays.1 - margin
                    && z > stays.2 + margin
                    && z < stays.3 - margin;
                let in_extra = extra.is_some_and(|bounds| {
                    x > bounds.0 + margin
                        && x < bounds.1 - margin
                        && z > bounds.2 + margin
                        && z < bounds.3 - margin
                });
                let required = in_static || in_extra;
                let clear_of_static = x < stays.0 - margin
                    || x > stays.1 + margin
                    || z < stays.2 - margin
                    || z > stays.3 + margin;
                let clear_of_extra = extra.is_none_or(|bounds| {
                    x < bounds.0 - margin
                        || x > bounds.1 + margin
                        || z < bounds.2 - margin
                        || z > bounds.3 + margin
                });
                let shadowed = shot.pixels[((j * 64 + i) * 4) as usize] > 127;
                if required {
                    inside += 1;
                    if shadowed {
                        inside_shadow += 1;
                    }
                }
                if clear_of_static && clear_of_extra && shadowed {
                    outside_shadow += 1;
                }
            }
        }
        assert!(
            inside > 20 && inside_shadow * 10 >= inside * 9 && outside_shadow < 30,
            "{label}: inside {inside_shadow}/{inside}, outside {outside_shadow}, total {}\n{}",
            shot.shadowed,
            ascii(&shot.pixels)
        );
        assert!(
            (80..900).contains(&shot.shadowed),
            "{label}: {} shadowed\n{}",
            shot.shadowed,
            ascii(&shot.pixels)
        );
    }

    fn ascii(pixels: &[u8]) -> String {
        let mut out = String::new();
        for j in (0..64).step_by(2) {
            for i in (0..64).step_by(2) {
                out.push(if pixels[((j * 64 + i) * 4) as usize] > 127 {
                    '#'
                } else {
                    '.'
                });
            }
            out.push('\n');
        }
        out
    }
}
