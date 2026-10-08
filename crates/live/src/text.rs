use crate::canopy::Gobo;
use crate::deform::DeformerId;
use bytemuck::{Pod, Zeroable};
use pfx_gpu::wgpu;
use pfx_gpu::wgpu::util::DeviceExt;
use pfx_load::{Asset, Cache, ColorSpace, Handle, Pixels};
use pfx_text::{
    Atlas, AtlasChanges, CellUpload, GlyphInstance, MSDF_SPREAD, MSDF_WGSL, PageInfo, PageKind,
    PageQuad, Paragraph, Representation,
};
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};

pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

const GLYPH_WGSL: &str = r#"
struct Transform {
    matrix: mat4x4f,
    view_projection: mat4x4f,
    model: mat4x4f,
    rest_normal: vec4f,
    atlas_size: vec2f,
    mode: u32,
    deformer: u32,
    cells: vec4u,
}
struct Instance {
    @location(0) pose: vec4f,
    @location(1) rect: vec4f,
    @location(2) uv_rect: vec4f,
    @location(3) color: vec4f,
    @location(4) clip_rect: vec4f,
    @location(5) turn: vec4f,
}
struct Placed {
    point: vec2f,
    turned: vec2f,
}
const ID_CUTOUT: f32 = 0.5;
const MSDF_ANISOTROPY_FROM: f32 = 1.2;
const MSDF_ANISOTROPY_FULL: f32 = 1.5;
const MSDF_MOST_TAPS: u32 = 8u;

fn msdf_texel_distance(atlas_texture: texture_2d<f32>, atlas_filter: sampler, uv: vec2f) -> f32 {
    return (msdf_median(textureSampleLevel(atlas_texture, atlas_filter, uv, 0.0).rgb) - 0.5) * MSDF_SPREAD;
}

fn msdf_major_axis(a: f32, b: f32, c: f32, major: f32) -> vec2f {
    let first = vec2f(b, major - a);
    let second = vec2f(major - c, b);
    let longer = select(second, first, dot(first, first) >= dot(second, second));
    if (dot(longer, longer) <= 1e-12) {
        return vec2f(1.0, 0.0);
    }
    return normalize(longer);
}

fn msdf_anisotropic(
    atlas_texture: texture_2d<f32>,
    atlas_filter: sampler,
    atlas_size: vec2f,
    uv: vec2f,
    low: vec2f,
    high: vec2f,
    jx: vec2f,
    jy: vec2f,
    axis: vec2f,
    taps: u32,
) -> f32 {
    let along = jx * axis.x + jy * axis.y;
    let across = jx * -axis.y + jy * axis.x;
    let texel = vec2f(1.0) / atlas_size;
    let count = f32(taps);
    var total = 0.0;
    for (var tap = 0u; tap < taps; tap++) {
        let offset = (f32(tap) + 0.5) / count - 0.5;
        let at = clamp(uv + along * offset * texel, low, high);
        let distance = msdf_texel_distance(atlas_texture, atlas_filter, at);
        let gradient = vec2f(
            msdf_texel_distance(atlas_texture, atlas_filter, at + vec2f(texel.x, 0.0)) - distance,
            msdf_texel_distance(atlas_texture, atlas_filter, at + vec2f(0.0, texel.y)) - distance,
        );
        let width = abs(dot(gradient, along)) / count + abs(dot(gradient, across));
        total += clamp(0.5 + distance / max(width, 1e-4), 0.0, 1.0);
    }
    return total / count;
}

fn glyph_place(instance: Instance, corner: vec2f) -> Placed {
    let local = (instance.rect.xy + corner * instance.rect.zw) * instance.pose.w;
    let c = cos(instance.pose.z);
    let s = sin(instance.pose.z);
    let point = instance.pose.xy + vec2f(c * local.x - s * local.y, s * local.x + c * local.y);
    let pivot = instance.turn.xy;
    let ct = cos(instance.turn.z);
    let st = sin(instance.turn.z);
    let offset = point - pivot;
    let turned = pivot + vec2f(ct * offset.x - st * offset.y, st * offset.x + ct * offset.y);
    return Placed(point, turned);
}

fn glyph_corner(index: u32) -> vec2f {
    var corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0)
    );
    return corners[index];
}

fn glyph_paint(
    atlas_texture: texture_2d<f32>,
    atlas_filter: sampler,
    atlas_size: vec2f,
    mode: u32,
    uv_in: vec2f,
    bounds: vec4f,
    color: vec4f,
    clip_rect: vec4f,
    point: vec2f,
    anisotropic: bool,
) -> vec4f {
    let dx = dpdx(uv_in);
    let dy = dpdy(uv_in);
    let footprint = max(length(dx * atlas_size), length(dy * atlas_size));
    let extent = (bounds.zw - bounds.xy) * atlas_size;
    let safe_lod = floor(log2(max(min(extent.x, extent.y), 1.0)));
    let max_lod = select(min(safe_lod, 3.0), 3.0, mode == 2u);
    let lod = clamp(log2(max(footprint, 1.0)), 0.0, max_lod);
    let level = select(lod, 0.0, mode == 1u);
    let half_texel = vec2f(exp2(level) * 0.5) / atlas_size;
    let low = min(bounds.xy + half_texel, bounds.zw);
    let high = max(bounds.zw - half_texel, low);
    let uv = clamp(uv_in, low, high);
    let sampled = textureSampleLevel(atlas_texture, atlas_filter, uv, level);
    var alpha = sampled.r;
    if (mode == 1u) {
        let sampled = textureSampleLevel(atlas_texture, atlas_filter, uv, 0.0);
        let signed_distance = msdf_median(sampled.rgb) - 0.5;
        alpha = clamp(0.5 + signed_distance / max(fwidth(signed_distance), 1e-5), 0.0, 1.0);
        let jx = dx * atlas_size;
        let jy = dy * atlas_size;
        let a = dot(jx, jx);
        let b = dot(jx, jy);
        let c = dot(jy, jy);
        let area = abs(jx.x * jy.y - jx.y * jy.x);
        let major_squared = 0.5 * (a + c) + sqrt(0.25 * (a - c) * (a - c) + b * b);
        if (anisotropic && major_squared > MSDF_ANISOTROPY_FROM * area) {
            let major = sqrt(major_squared);
            let minor = area / max(major, 1e-6);
            let blend = smoothstep(MSDF_ANISOTROPY_FROM, MSDF_ANISOTROPY_FULL, major / max(minor, 1e-6));
            let taps = u32(clamp(ceil(major / max(minor, 1.0)), 1.0, f32(MSDF_MOST_TAPS)));
            let axis = msdf_major_axis(a, b, c, major_squared);
            let filtered = msdf_anisotropic(atlas_texture, atlas_filter, atlas_size, uv_in, low, high, jx, jy, axis, taps);
            alpha = mix(alpha, filtered, blend);
        }
    }
    let inside = step(clip_rect.x, point.x) * step(point.x, clip_rect.z)
        * step(clip_rect.y, point.y) * step(point.y, clip_rect.w);
    if (mode == 2u) {
        let opacity = sampled.a * color.a * inside;
        return vec4f(sampled.rgb * color.rgb * color.a * inside, opacity);
    }
    let opacity = alpha * color.a * inside;
    return vec4f(color.rgb * opacity, opacity);
}
"#;

const SHADER: &str = r#"
@group(0) @binding(0) var<uniform> transform: Transform;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) bounds: vec4f,
    @location(2) @interpolate(flat) color: vec4f,
    @location(3) @interpolate(flat) clip_rect: vec4f,
    @location(4) point: vec2f,
}

@vertex
fn vs_main(instance: Instance, @builtin(vertex_index) index: u32) -> VertexOut {
    let corner = glyph_corner(index);
    let placed = glyph_place(instance, corner);
    var out: VertexOut;
    out.clip = transform.matrix * vec4f(placed.turned, 0.0, 1.0);
    out.uv = mix(instance.uv_rect.xy, instance.uv_rect.zw, corner);
    out.bounds = instance.uv_rect;
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.point = placed.point;
    return out;
}

@vertex
fn vs_deformed(instance: Instance, @builtin(vertex_index) index: u32) -> VertexOut {
    let cells = max(transform.cells.x, 1u);
    let cell = index / 6u;
    let corner = (vec2f(f32(cell % cells), f32(cell / cells)) + glyph_corner(index % 6u)) / f32(cells);
    let placed = glyph_place(instance, corner);
    let rest = (transform.matrix * vec4f(placed.turned, 0.0, 1.0)).xyz;
    let shape = deform_vertex(rest, transform.rest_normal.xyz, vec4f(1.0, 0.0, 0.0, 1.0), transform.deformer);
    let lifted = shape.position + normalize(shape.normal) * transform.rest_normal.w;
    var out: VertexOut;
    out.clip = transform.view_projection * transform.model * vec4f(lifted, 1.0);
    out.uv = mix(instance.uv_rect.xy, instance.uv_rect.zw, corner);
    out.bounds = instance.uv_rect;
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.point = placed.point;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    return glyph_paint(atlas, atlas_sampler, transform.atlas_size, transform.mode, in.uv, in.bounds, in.color, in.clip_rect, in.point, true);
}

@fragment
fn fs_overlay(in: VertexOut) -> @location(0) vec4f {
    return glyph_paint(atlas, atlas_sampler, transform.atlas_size, transform.mode, in.uv, in.bounds, in.color, in.clip_rect, in.point, false);
}

fn glyph_id(in: VertexOut, anisotropic: bool) -> u32 {
    let coverage = glyph_paint(atlas, atlas_sampler, transform.atlas_size, transform.mode, in.uv, in.bounds, vec4f(1.0), in.clip_rect, in.point, anisotropic).a;
    if (coverage < ID_CUTOUT || in.color.a <= 0.0) {
        discard;
    }
    return transform.cells.y;
}

@fragment
fn fs_id(in: VertexOut) -> @location(0) u32 {
    return glyph_id(in, true);
}

@fragment
fn fs_overlay_id(in: VertexOut) -> @location(0) u32 {
    return glyph_id(in, false);
}
"#;

const LIT_SHADER: &str = r#"
@group(2) @binding(2) var<uniform> transform: Transform;
@group(2) @binding(3) var atlas: texture_2d<f32>;
@group(2) @binding(4) var atlas_sampler: sampler;

struct LitOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) bounds: vec4f,
    @location(2) @interpolate(flat) color: vec4f,
    @location(3) @interpolate(flat) clip_rect: vec4f,
    @location(4) point: vec2f,
    @location(5) world: vec3f,
    @location(6) normal: vec3f,
}

fn world_normal(model: mat4x4f, normal: vec3f) -> vec3f {
    let a = model[0].xyz;
    let b = model[1].xyz;
    let c = model[2].xyz;
    return normalize(cross(b, c) * normal.x + cross(c, a) * normal.y + cross(a, b) * normal.z);
}

@vertex
fn vs_lit_surface(instance: Instance, @builtin(vertex_index) index: u32) -> LitOut {
    let corner = glyph_corner(index);
    let placed = glyph_place(instance, corner);
    var out: LitOut;
    out.clip = transform.matrix * vec4f(placed.turned, 0.0, 1.0);
    out.uv = mix(instance.uv_rect.xy, instance.uv_rect.zw, corner);
    out.bounds = instance.uv_rect;
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.point = placed.point;
    out.world = (transform.model * vec4f(placed.turned, 0.0, 1.0)).xyz;
    out.normal = world_normal(transform.model, vec3f(0.0, 0.0, 1.0));
    return out;
}

@vertex
fn vs_lit_deformed(instance: Instance, @builtin(vertex_index) index: u32) -> LitOut {
    let cells = max(transform.cells.x, 1u);
    let cell = index / 6u;
    let corner = (vec2f(f32(cell % cells), f32(cell / cells)) + glyph_corner(index % 6u)) / f32(cells);
    let placed = glyph_place(instance, corner);
    let rest = (transform.matrix * vec4f(placed.turned, 0.0, 1.0)).xyz;
    let shape = deform_vertex(rest, transform.rest_normal.xyz, vec4f(1.0, 0.0, 0.0, 1.0), transform.deformer);
    let lifted = shape.position + normalize(shape.normal) * transform.rest_normal.w;
    var out: LitOut;
    out.clip = transform.view_projection * transform.model * vec4f(lifted, 1.0);
    out.uv = mix(instance.uv_rect.xy, instance.uv_rect.zw, corner);
    out.bounds = instance.uv_rect;
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.point = placed.point;
    out.world = (transform.model * vec4f(lifted, 1.0)).xyz;
    out.normal = world_normal(transform.model, normalize(shape.normal));
    return out;
}

fn ink_local_light(world: vec3f, n: vec3f) -> vec3f {
    var total = vec3f(0.0);
    let count = min(local_lights.info.x, 4u);
    for (var index = 0u; index < count; index++) {
        let light = local_lights.lights[index];
        let to_light = light.position_radius.xyz - world;
        let distance = length(to_light);
        let range = light.colour_range.w;
        if (distance >= range || distance <= 1e-6) {
            continue;
        }
        let l = to_light / distance;
        let ndl = dot(n, l);
        if (ndl <= 0.0) {
            continue;
        }
        let ratio = distance / range;
        let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
        let reach = max(distance, light.position_radius.w);
        let radiance = light.colour_range.xyz * window * window / (reach * reach);
        total += radiance * ndl * light_shadow(light, world, n, l) * light_tint(light, world, n, l);
    }
    return total;
}

fn ink_irradiance(world: vec3f, facing: vec3f) -> vec3f {
    let view = normalize(frame.camera_position.xyz - world);
    var n = normalize(facing);
    if (dot(n, view) < 0.0) {
        n = -n;
    }
    let light = normalize(frame.sun_direction.xyz);
    var sun = frame.sun_colour_intensity.xyz * frame.sun_colour_intensity.w * max(dot(n, light), 0.0) * shadow_visibility(world, n);
    if (any(sun > vec3f(0.0))) {
        sun *= shadow_tint(world, n, 0xfffffffeu);
    }
    if (frame.flags.z != 0u) {
        sun *= canopy_light(world);
    }
    return sun + ambient_irradiance(world, n) + ink_local_light(world, n);
}

@fragment
fn fs_lit(in: LitOut) -> @location(0) vec4f {
    let paint = glyph_paint(atlas, atlas_sampler, transform.atlas_size, transform.mode, in.uv, in.bounds, in.color, in.clip_rect, in.point, true);
    if (paint.a <= 0.0) {
        return paint;
    }
    return vec4f(paint.rgb * ink_irradiance(in.world, in.normal) / PI, paint.a);
}

@fragment
fn fs_lit_id(in: LitOut) -> @location(0) u32 {
    let coverage = glyph_paint(atlas, atlas_sampler, transform.atlas_size, transform.mode, in.uv, in.bounds, vec4f(1.0), in.clip_rect, in.point, true).a;
    if (coverage < ID_CUTOUT || in.color.a <= 0.0) {
        discard;
    }
    return transform.cells.y;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PackedGlyph {
    pub pose: [f32; 4],
    pub rect: [f32; 4],
    pub uv_rect: [f32; 4],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    pub turn: [f32; 4],
}

impl From<&GlyphInstance> for PackedGlyph {
    fn from(glyph: &GlyphInstance) -> Self {
        Self {
            pose: [
                glyph.position[0],
                glyph.position[1],
                glyph.rotation,
                glyph.scale,
            ],
            rect: glyph.local_rect,
            uv_rect: glyph.uv,
            color: [1.0; 4],
            clip: [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
            turn: [0.0; 4],
        }
    }
}

pub fn pack_glyphs(glyphs: &[GlyphInstance]) -> Vec<PackedGlyph> {
    glyphs.iter().map(PackedGlyph::from).collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RichQuad {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub origin: [f32; 2],
    pub alpha: f32,
    pub clip: [f32; 4],
    pub turn: [f32; 3],
    pub color: [f32; 4],
}

impl From<&pfx_text::RichGlyphQuad> for RichQuad {
    fn from(quad: &pfx_text::RichGlyphQuad) -> Self {
        Self {
            rect: quad.rect,
            uv: quad.uv,
            origin: quad.origin,
            alpha: 1.0,
            clip: quad.clip,
            turn: quad.turn,
            color: quad.color,
        }
    }
}

pub fn rich_quads(paragraph: &pfx_text::RichParagraph) -> Vec<RichQuad> {
    paragraph.quads.iter().map(RichQuad::from).collect()
}

impl From<&RichQuad> for PackedGlyph {
    fn from(quad: &RichQuad) -> Self {
        Self {
            pose: [0.0, 0.0, 0.0, 1.0],
            rect: quad.rect,
            uv_rect: quad.uv,
            color: [
                quad.color[0],
                quad.color[1],
                quad.color[2],
                quad.color[3] * quad.alpha,
            ],
            clip: quad.clip,
            turn: [quad.turn[0], quad.turn[1], quad.turn[2], 0.0],
        }
    }
}

pub fn pack_quads(quads: &[RichQuad]) -> Vec<PackedGlyph> {
    quads.iter().map(PackedGlyph::from).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadRegion {
    pub level: u32,
    pub first_row: u32,
    pub rows: u32,
    pub first_column: u32,
    pub columns: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AtlasError {
    Empty,
    InvalidChannels,
    InvalidLevel,
    TooLarge,
}

fn format_for(atlas: &Atlas) -> Result<wgpu::TextureFormat, AtlasError> {
    match atlas.channels {
        1 => Ok(wgpu::TextureFormat::R8Unorm),
        3 => Ok(wgpu::TextureFormat::Rgba8Unorm),
        _ => Err(AtlasError::InvalidChannels),
    }
}

fn converted_levels(atlas: &Atlas) -> Result<Vec<Vec<u8>>, AtlasError> {
    let bytes_per_pixel = match atlas.channels {
        1 => 1usize,
        3 => 4usize,
        _ => return Err(AtlasError::InvalidChannels),
    };
    let mut converted = Vec::with_capacity(atlas.levels.len().min(4));
    for (index, level) in atlas.levels.iter().take(4).enumerate() {
        let expected_width = if index == 0 {
            level.width
        } else {
            (atlas.levels[index - 1].width / 2).max(1)
        };
        let expected_height = if index == 0 {
            level.height
        } else {
            (atlas.levels[index - 1].height / 2).max(1)
        };
        if index > 0 && atlas.levels[index - 1].width == 1 && atlas.levels[index - 1].height == 1 {
            return Err(AtlasError::InvalidLevel);
        }
        let pixels = level
            .width
            .checked_mul(level.height)
            .ok_or(AtlasError::TooLarge)?;
        let count = usize::try_from(pixels).map_err(|_| AtlasError::TooLarge)?;
        let source_bytes = count
            .checked_mul(atlas.channels as usize)
            .ok_or(AtlasError::TooLarge)?;
        let target_bytes = count
            .checked_mul(bytes_per_pixel)
            .ok_or(AtlasError::TooLarge)?;
        if level.width == 0
            || level.height == 0
            || level.width != expected_width
            || level.height != expected_height
            || level.bytes.len() != source_bytes
        {
            return Err(AtlasError::InvalidLevel);
        }
        let mut bytes = Vec::with_capacity(target_bytes);
        if atlas.channels == 1 {
            bytes.extend_from_slice(&level.bytes);
        } else {
            for rgb in level.bytes.chunks_exact(3) {
                bytes.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        converted.push(bytes);
    }
    if converted.is_empty() {
        return Err(AtlasError::Empty);
    }
    Ok(converted)
}

fn changed_regions(
    atlas: &Atlas,
    current: &[Vec<u8>],
    previous: Option<&[Vec<u8>]>,
) -> Vec<UploadRegion> {
    let pixel_bytes = if atlas.channels == 1 { 1 } else { 4 };
    let mut regions = Vec::new();
    for (level_index, level) in atlas.levels.iter().take(current.len()).enumerate() {
        let row_bytes = level.width as usize * pixel_bytes;
        let old = previous.and_then(|levels| levels.get(level_index));
        let mut first = None;
        for row in 0..level.height {
            let start = row as usize * row_bytes;
            let end = start + row_bytes;
            let changed =
                old.is_none_or(|bytes| bytes[start..end] != current[level_index][start..end]);
            if changed {
                first.get_or_insert(row);
            } else if let Some(begin) = first.take() {
                regions.push(UploadRegion {
                    level: level_index as u32,
                    first_row: begin,
                    rows: row - begin,
                    first_column: 0,
                    columns: level.width,
                });
            }
        }
        if let Some(begin) = first {
            regions.push(UploadRegion {
                level: level_index as u32,
                first_row: begin,
                rows: level.height - begin,
                first_column: 0,
                columns: level.width,
            });
        }
    }
    regions
}

pub struct GpuAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    levels: Vec<Vec<u8>>,
    dimensions: Vec<(u32, u32)>,
}

impl GpuAtlas {
    pub fn icons(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &IconAtlas) -> Self {
        let first = &atlas.levels[0];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("icon atlas"),
            size: wgpu::Extent3d {
                width: first.width,
                height: first.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: atlas.levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (index, level) in atlas.levels.iter().enumerate() {
            let pixels = level
                .pixels
                .iter()
                .map(|sample| half::f16::from_f32(*sample as f32 / 65535.0).to_bits())
                .collect::<Vec<_>>();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: index as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&pixels),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(level.width * 8),
                    rows_per_image: Some(level.height),
                },
                wgpu::Extent3d {
                    width: level.width,
                    height: level.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            format: wgpu::TextureFormat::Rgba16Float,
            width: first.width,
            height: first.height,
            levels: Vec::new(),
            dimensions: atlas
                .levels
                .iter()
                .map(|level| (level.width, level.height))
                .collect(),
        }
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &Atlas,
    ) -> Result<Self, AtlasError> {
        let format = format_for(atlas)?;
        let levels = converted_levels(atlas)?;
        let first = &atlas.levels[0];
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text atlas"),
            size: wgpu::Extent3d {
                width: first.width,
                height: first.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut result = Self {
            texture,
            view,
            format,
            width: first.width,
            height: first.height,
            levels: Vec::new(),
            dimensions: Vec::new(),
        };
        result.upload_converted(queue, atlas, levels, true);
        Ok(result)
    }

    fn upload_converted(
        &mut self,
        queue: &wgpu::Queue,
        atlas: &Atlas,
        levels: Vec<Vec<u8>>,
        all: bool,
    ) -> Vec<UploadRegion> {
        let regions = changed_regions(atlas, &levels, if all { None } else { Some(&self.levels) });
        let pixel_bytes = if atlas.channels == 1 { 1 } else { 4 };
        for region in &regions {
            let mip = &atlas.levels[region.level as usize];
            let row_bytes = mip.width as usize * pixel_bytes;
            let begin = region.first_row as usize * row_bytes;
            let end = begin + region.rows as usize * row_bytes;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: region.level,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: region.first_row,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &levels[region.level as usize][begin..end],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes as u32),
                    rows_per_image: Some(region.rows),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: region.rows,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.dimensions = atlas
            .levels
            .iter()
            .take(levels.len())
            .map(|mip| (mip.width, mip.height))
            .collect();
        self.levels = levels;
        regions
    }

    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &Atlas,
    ) -> Result<Vec<UploadRegion>, AtlasError> {
        let format = format_for(atlas)?;
        let levels = converted_levels(atlas)?;
        let first = &atlas.levels[0];
        if self.format != format
            || self.width != first.width
            || self.height != first.height
            || self.dimensions
                != atlas
                    .levels
                    .iter()
                    .take(levels.len())
                    .map(|mip| (mip.width, mip.height))
                    .collect::<Vec<_>>()
        {
            *self = Self::new(device, queue, atlas)?;
            return Ok(atlas
                .levels
                .iter()
                .take(levels.len())
                .enumerate()
                .map(|(level, mip)| UploadRegion {
                    level: level as u32,
                    first_row: 0,
                    rows: mip.height,
                    first_column: 0,
                    columns: mip.width,
                })
                .collect());
        }
        Ok(self.upload_converted(queue, atlas, levels, false))
    }

    pub fn blank(device: &wgpu::Device, page: &PageInfo) -> Result<Self, AtlasError> {
        let format = match page.channels {
            1 => wgpu::TextureFormat::R8Unorm,
            3 => wgpu::TextureFormat::Rgba8Unorm,
            _ => return Err(AtlasError::InvalidChannels),
        };
        if page.size == 0 || page.levels == 0 || page.levels > 4 {
            return Err(AtlasError::InvalidLevel);
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph page"),
            size: wgpu::Extent3d {
                width: page.size,
                height: page.size,
                depth_or_array_layers: 1,
            },
            mip_level_count: page.levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(Self {
            texture,
            view,
            format,
            width: page.size,
            height: page.size,
            levels: Vec::new(),
            dimensions: (0..page.levels)
                .map(|level| ((page.size >> level).max(1), (page.size >> level).max(1)))
                .collect(),
        })
    }

    pub fn write_cells(
        &mut self,
        queue: &wgpu::Queue,
        cells: &[&CellUpload],
    ) -> Result<Vec<UploadRegion>, AtlasError> {
        let (channels, pixel_bytes) = match self.format {
            wgpu::TextureFormat::R8Unorm => (1usize, 1usize),
            wgpu::TextureFormat::Rgba8Unorm => (3, 4),
            _ => return Err(AtlasError::InvalidChannels),
        };
        let mut regions = Vec::with_capacity(cells.len());
        for cell in cells {
            let [x, y, width, height] = cell.rect;
            let &(level_width, level_height) = self
                .dimensions
                .get(cell.level as usize)
                .ok_or(AtlasError::InvalidLevel)?;
            if width == 0
                || height == 0
                || x + width > level_width
                || y + height > level_height
                || cell.bytes.len() != (width * height) as usize * channels
            {
                return Err(AtlasError::InvalidLevel);
            }
            let bytes = if channels == 3 {
                cell.bytes
                    .chunks_exact(3)
                    .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
                    .collect::<Vec<_>>()
            } else {
                cell.bytes.clone()
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: cell.level,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * pixel_bytes as u32),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            regions.push(UploadRegion {
                level: cell.level,
                first_row: y,
                rows: height,
                first_column: x,
                columns: width,
            });
        }
        if !regions.is_empty() {
            self.levels.clear();
        }
        Ok(regions)
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }
    pub fn size(&self) -> [u32; 2] {
        [self.width, self.height]
    }
}

#[derive(Default)]
pub struct GlyphPages {
    pages: BTreeMap<u32, GpuAtlas>,
    skipped: BTreeSet<u32>,
}

impl GlyphPages {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        changes: &AtlasChanges,
    ) -> Result<Vec<(u32, UploadRegion)>, AtlasError> {
        for id in &changes.dropped {
            self.pages.remove(id);
            self.skipped.remove(id);
        }
        for page in &changes.created {
            if page.kind == PageKind::Color {
                self.skipped.insert(page.id);
                continue;
            }
            self.pages.insert(page.id, GpuAtlas::blank(device, page)?);
        }
        let mut uploads = BTreeMap::<u32, Vec<&CellUpload>>::new();
        for cell in &changes.cells {
            if self.skipped.contains(&cell.page) {
                continue;
            }
            uploads.entry(cell.page).or_default().push(cell);
        }
        let mut regions = Vec::new();
        for (id, cells) in uploads {
            let page = self.pages.get_mut(&id).ok_or(AtlasError::Empty)?;
            regions.extend(
                page.write_cells(queue, &cells)?
                    .into_iter()
                    .map(|region| (id, region)),
            );
        }
        Ok(regions)
    }

    pub fn page(&self, id: u32) -> Option<&GpuAtlas> {
        self.pages.get(&id)
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

pub fn page_quads(quads: &[PageQuad]) -> Vec<(u32, Vec<RichQuad>)> {
    let mut groups: Vec<(u32, Vec<RichQuad>)> = Vec::new();
    for quad in quads {
        let rich = RichQuad::from(&quad.quad);
        match groups.iter_mut().find(|(page, _)| *page == quad.page) {
            Some((_, list)) => list.push(rich),
            None => groups.push((quad.page, vec![rich])),
        }
    }
    groups
}

#[derive(Clone, Debug)]
pub struct IconLevel {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u16>,
}

pub struct IconAtlas {
    pub levels: Vec<IconLevel>,
    rects: Vec<(Handle, [u32; 4])>,
}

impl IconAtlas {
    pub fn from_cache(cache: &Cache, handles: &[Handle]) -> Result<Self, String> {
        let width = 2048u32;
        let padding = 8u32;
        let mut x = 0u32;
        let mut y = 0u32;
        let mut row_height = 0u32;
        let mut rects = Vec::new();
        for &handle in handles {
            if rects.iter().any(|(existing, _)| *existing == handle) {
                continue;
            }
            let Some(Asset::Image(image)) = cache.get(handle) else {
                return Err("icon handle is not an image".into());
            };
            if image.width > i32::MAX as u32 - padding || image.height > i32::MAX as u32 - padding {
                return Err("icon is too large".into());
            }
            let cell_width = image
                .width
                .checked_add(padding * 2)
                .ok_or("icon is too large")?
                .next_multiple_of(8);
            let cell_height = image
                .height
                .checked_add(padding * 2)
                .ok_or("icon is too large")?
                .next_multiple_of(8);
            if cell_width > width {
                return Err("icon is wider than the atlas".into());
            }
            if x + cell_width > width {
                y = y.checked_add(row_height).ok_or("icon atlas is too large")?;
                x = 0;
                row_height = 0;
            }
            rects.push((
                handle,
                [x + padding, y + padding, image.width, image.height],
            ));
            x += cell_width;
            row_height = row_height.max(cell_height);
        }
        let height = y
            .checked_add(row_height)
            .ok_or("icon atlas is too large")?
            .checked_next_power_of_two()
            .ok_or("icon atlas is too large")?
            .max(8);
        if height > 16384 {
            return Err("icon atlas is too tall".into());
        }
        let mut pixels = vec![0u16; width as usize * height as usize * 4];
        for &(handle, rect) in &rects {
            let Some(Asset::Image(image)) = cache.get(handle) else {
                unreachable!()
            };
            let cell_width = (image.width + padding * 2).next_multiple_of(8);
            let cell_height = (image.height + padding * 2).next_multiple_of(8);
            for dy in -(padding as i32)..(cell_height - padding) as i32 {
                for dx in -(padding as i32)..(cell_width - padding) as i32 {
                    let sx = dx.clamp(0, image.width as i32 - 1) as u32;
                    let sy = dy.clamp(0, image.height as i32 - 1) as u32;
                    let rgba = image_pixel(image, sx, sy);
                    let at = (((rect[1] as i32 + dy) as u32 * width + (rect[0] as i32 + dx) as u32)
                        * 4) as usize;
                    pixels[at..at + 4].copy_from_slice(&rgba);
                }
            }
        }
        let mut levels = vec![IconLevel {
            width,
            height,
            pixels,
        }];
        while levels.len() < 4 {
            let previous = levels.last().unwrap();
            let width = (previous.width / 2).max(1);
            let height = (previous.height / 2).max(1);
            let mut pixels = vec![0u16; width as usize * height as usize * 4];
            for row in 0..height {
                for col in 0..width {
                    for channel in 0..4 {
                        let mut sum = 0u32;
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let sx = (col * 2 + dx).min(previous.width - 1);
                                let sy = (row * 2 + dy).min(previous.height - 1);
                                sum += previous.pixels
                                    [((sy * previous.width + sx) * 4 + channel) as usize]
                                    as u32;
                            }
                        }
                        pixels[((row * width + col) * 4 + channel) as usize] =
                            ((sum + 2) / 4) as u16;
                    }
                }
            }
            levels.push(IconLevel {
                width,
                height,
                pixels,
            });
        }
        Ok(Self { levels, rects })
    }

    pub fn uv(&self, handle: Handle) -> Option<[f32; 4]> {
        let rect = self.rects.iter().find(|(entry, _)| *entry == handle)?.1;
        let first = &self.levels[0];
        Some([
            rect[0] as f32 / first.width as f32,
            rect[1] as f32 / first.height as f32,
            (rect[0] + rect[2]) as f32 / first.width as f32,
            (rect[1] + rect[3]) as f32 / first.height as f32,
        ])
    }
}

fn image_pixel(image: &pfx_load::Image, x: u32, y: u32) -> [u16; 4] {
    let at = ((y * image.width + x) * 4) as usize;
    let rgba = match &image.pixels {
        Pixels::Eight(bytes) => <[u8; 4]>::try_from(&bytes[at..at + 4])
            .unwrap()
            .map(|value| value as f32 / 255.0),
        Pixels::Sixteen(samples) => <[u16; 4]>::try_from(&samples[at..at + 4])
            .unwrap()
            .map(|value| value as f32 / 65535.0),
    };
    let linear = |value: f32| {
        if image.space == ColorSpace::Srgb {
            pfx_materials::linear_channel(value)
        } else {
            value
        }
    };
    [
        (linear(rgba[0]) * rgba[3] * 65535.0).round() as u16,
        (linear(rgba[1]) * rgba[3] * 65535.0).round() as u16,
        (linear(rgba[2]) * rgba[3] * 65535.0).round() as u16,
        (rgba[3] * 65535.0).round() as u16,
    ]
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    matrix: [[f32; 4]; 4],
    view_projection: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    rest_normal: [f32; 4],
    atlas_size: [f32; 2],
    mode: u32,
    deformer: u32,
    cells: [u32; 4],
}

pub const DEFORMED_CELLS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextSpace {
    Surface {
        model_view_projection: [[f32; 4]; 4],
    },
    Deformed {
        view_projection: [[f32; 4]; 4],
        model: [[f32; 4]; 4],
        layout: [[f32; 4]; 4],
        normal: [f32; 3],
        lift: f32,
        deformer: DeformerId,
    },
    Overlay {
        width: u32,
        height: u32,
    },
    LitSurface {
        model_view_projection: [[f32; 4]; 4],
        model: [[f32; 4]; 4],
    },
    LitDeformed {
        view_projection: [[f32; 4]; 4],
        model: [[f32; 4]; 4],
        layout: [[f32; 4]; 4],
        normal: [f32; 3],
        lift: f32,
        deformer: DeformerId,
    },
}

impl TextSpace {
    pub fn is_lit(&self) -> bool {
        matches!(self, Self::LitSurface { .. } | Self::LitDeformed { .. })
    }

    fn matrix(&self) -> [[f32; 4]; 4] {
        match *self {
            Self::Surface {
                model_view_projection,
            }
            | Self::LitSurface {
                model_view_projection,
                ..
            } => model_view_projection,
            Self::Deformed { layout, .. } | Self::LitDeformed { layout, .. } => layout,
            Self::Overlay { width, height } => [
                [2.0 / width as f32, 0.0, 0.0, 0.0],
                [0.0, -2.0 / height as f32, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
        }
    }
}

pub struct TextPass {
    surface: wgpu::RenderPipeline,
    deformed: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    surface_ids: wgpu::RenderPipeline,
    deformed_ids: wgpu::RenderPipeline,
    overlay_ids: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    deformer_layout: wgpu::BindGroupLayout,
    empty_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    lit: OnceCell<LitPipelines>,
    gobo: Gobo,
    viewport: Option<[u32; 2]>,
}

struct LitPipelines {
    surface: wgpu::RenderPipeline,
    deformed: wgpu::RenderPipeline,
    surface_ids: wgpu::RenderPipeline,
    deformed_ids: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

pub struct LitBindings<'a> {
    pub scene: &'a wgpu::BindGroup,
    pub shadow: &'a wgpu::BindGroup,
    pub lighting: &'a wgpu::BindGroup,
    pub deformers: &'a wgpu::Buffer,
    pub plan: &'a wgpu::Buffer,
}

fn text_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    entries: (&str, &str),
    depth: bool,
    ids: bool,
) -> wgpu::RenderPipeline {
    let attribute = |offset: u64, shader_location: u32| wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset,
        shader_location,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entries.0),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<PackedGlyph>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &[
                    attribute(0, 0),
                    attribute(16, 1),
                    attribute(32, 2),
                    attribute(48, 3),
                    attribute(64, 4),
                    attribute(80, 5),
                ],
            }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: depth.then_some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entries.1),
            compilation_options: Default::default(),
            targets: &[Some(if ids {
                wgpu::ColorTargetState {
                    format: ID_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                }
            } else {
                wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }
            })],
        }),
        multiview: None,
        cache: None,
    })
}

pub fn lit_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let storage = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    vec![
        storage(0),
        storage(1),
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
}

impl LitPipelines {
    fn new(device: &wgpu::Device, gobo: &Gobo) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lit text shader"),
            source: wgpu::ShaderSource::Wgsl(lit_shader_source_with(gobo).into()),
        });
        let group = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let scene = group("lit text scene", &crate::frame::scene_entries());
        let shadow = group("lit text shadows", &crate::frame::shadow_entries());
        let own = group("lit text bindings", &lit_entries());
        let lighting = group("lit text lighting", &crate::frame::lighting_entries());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit text pipeline layout"),
            bind_group_layouts: &[&scene, &shadow, &own, &lighting],
            push_constant_ranges: &[],
        });
        Self {
            surface: text_pipeline(
                device,
                "lit surface text",
                &layout,
                &shader,
                ("vs_lit_surface", "fs_lit"),
                true,
                false,
            ),
            deformed: text_pipeline(
                device,
                "lit deformed text",
                &layout,
                &shader,
                ("vs_lit_deformed", "fs_lit"),
                true,
                false,
            ),
            surface_ids: text_pipeline(
                device,
                "lit surface text ids",
                &layout,
                &shader,
                ("vs_lit_surface", "fs_lit_id"),
                true,
                true,
            ),
            deformed_ids: text_pipeline(
                device,
                "lit deformed text ids",
                &layout,
                &shader,
                ("vs_lit_deformed", "fs_lit_id"),
                true,
                true,
            ),
            layout: own,
        }
    }
}

pub struct TextDraw<'a> {
    pub target: &'a wgpu::TextureView,
    pub depth: Option<&'a wgpu::TextureView>,
    pub atlas: &'a GpuAtlas,
    pub paragraph: &'a Paragraph,
    pub space: TextSpace,
    pub color: [f32; 4],
    pub deformers: Option<&'a wgpu::BindGroup>,
}

pub struct QuadDraw<'a> {
    pub target: &'a wgpu::TextureView,
    pub depth: Option<&'a wgpu::TextureView>,
    pub atlas: &'a GpuAtlas,
    pub quads: &'a [RichQuad],
    pub space: TextSpace,
    pub deformers: Option<&'a wgpu::BindGroup>,
    pub icons: bool,
}

impl TextPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text bindings"),
            entries: &text_entries(),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text pipeline layout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text empty"),
            entries: &[],
        });
        let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text empty"),
            layout: &empty_layout,
            entries: &[],
        });
        let deformer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text deformers"),
            entries: &deformer_entries(),
        });
        let deformed_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("deformed text pipeline layout"),
            bind_group_layouts: &[&bind_layout, &empty_layout, &deformer_layout],
            push_constant_ranges: &[],
        });
        let surface = text_pipeline(
            device,
            "surface text",
            &layout,
            &shader,
            ("vs_main", "fs_main"),
            true,
            false,
        );
        let deformed = text_pipeline(
            device,
            "deformed text",
            &deformed_layout,
            &shader,
            ("vs_deformed", "fs_main"),
            true,
            false,
        );
        let overlay = text_pipeline(
            device,
            "overlay text",
            &layout,
            &shader,
            ("vs_main", "fs_overlay"),
            false,
            false,
        );
        let surface_ids = text_pipeline(
            device,
            "surface text ids",
            &layout,
            &shader,
            ("vs_main", "fs_id"),
            true,
            true,
        );
        let deformed_ids = text_pipeline(
            device,
            "deformed text ids",
            &deformed_layout,
            &shader,
            ("vs_deformed", "fs_id"),
            true,
            true,
        );
        let overlay_ids = text_pipeline(
            device,
            "overlay text ids",
            &layout,
            &shader,
            ("vs_main", "fs_overlay_id"),
            false,
            true,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            lod_max_clamp: 3.0,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        Self {
            surface,
            deformed,
            overlay,
            surface_ids,
            deformed_ids,
            overlay_ids,
            bind_layout,
            deformer_layout,
            empty_group,
            sampler,
            lit: OnceCell::new(),
            gobo: Gobo::default(),
            viewport: None,
        }
    }

    pub fn set_viewport(&mut self, viewport: Option<[u32; 2]>) {
        self.viewport = viewport;
    }

    pub fn viewport(&self) -> Option<[u32; 2]> {
        self.viewport
    }

    pub fn set_canopy_gobo(&mut self, gobo: Gobo) {
        if gobo != self.gobo {
            self.gobo = gobo;
            self.lit = OnceCell::new();
        }
    }

    pub fn deformer_group(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        plan: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text deformers"),
            layout: &self.deformer_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: plan.as_entire_binding(),
                },
            ],
        })
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: TextDraw<'_>,
    ) -> Result<(), &'static str> {
        self.encode_timed(device, encoder, draw, None)
    }

    pub fn encode_timed(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: TextDraw<'_>,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        self.encode_paragraph(device, encoder, draw, None, None, profiler)
    }

    pub fn encode_lit_timed(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: TextDraw<'_>,
        lit: &LitBindings<'_>,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        self.encode_paragraph(device, encoder, draw, Some(lit), None, profiler)
    }

    pub fn encode_ids(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: TextDraw<'_>,
        lit: Option<&LitBindings<'_>>,
        id: u32,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        if id == 0 {
            return Ok(());
        }
        self.encode_paragraph(device, encoder, draw, lit, Some(id), profiler)
    }

    fn encode_paragraph(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: TextDraw<'_>,
        lit: Option<&LitBindings<'_>>,
        id: Option<u32>,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        let TextDraw {
            target,
            depth,
            atlas,
            paragraph,
            space,
            color,
            deformers,
        } = draw;
        if paragraph.glyphs.is_empty() {
            return Ok(());
        }
        if atlas.format
            != match paragraph.atlas.channels {
                1 => wgpu::TextureFormat::R8Unorm,
                3 => wgpu::TextureFormat::Rgba8Unorm,
                _ => return Err("invalid atlas channels"),
            }
        {
            return Err("atlas format does not match paragraph");
        }
        if atlas.size()
            != [
                paragraph.atlas.levels[0].width,
                paragraph.atlas.levels[0].height,
            ]
        {
            return Err("atlas size does not match paragraph");
        }
        let mut packed = pack_glyphs(&paragraph.glyphs);
        for glyph in &mut packed {
            for (channel, tint) in glyph.color.iter_mut().zip(color) {
                *channel *= tint;
            }
        }
        self.encode_packed(
            device,
            encoder,
            Packed {
                target,
                depth,
                atlas,
                space,
                deformers,
                lit,
                mode: u32::from(paragraph.atlas.channels == 3),
                id,
                glyphs: &packed,
            },
            profiler,
        )
    }

    pub fn encode_quads(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: QuadDraw<'_>,
    ) -> Result<(), &'static str> {
        self.encode_quads_timed(device, encoder, draw, None)
    }

    pub fn encode_quads_timed(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: QuadDraw<'_>,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        self.encode_quad_list(device, encoder, draw, None, None, profiler)
    }

    pub fn encode_quads_lit_timed(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: QuadDraw<'_>,
        lit: &LitBindings<'_>,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        self.encode_quad_list(device, encoder, draw, Some(lit), None, profiler)
    }

    pub fn encode_quad_ids(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: QuadDraw<'_>,
        lit: Option<&LitBindings<'_>>,
        id: u32,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        if id == 0 {
            return Ok(());
        }
        self.encode_quad_list(device, encoder, draw, lit, Some(id), profiler)
    }

    fn encode_quad_list(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: QuadDraw<'_>,
        lit: Option<&LitBindings<'_>>,
        id: Option<u32>,
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        if draw.quads.is_empty() {
            return Ok(());
        }
        let mode = if draw.icons {
            2
        } else if draw.atlas.format == wgpu::TextureFormat::Rgba8Unorm {
            1
        } else {
            0
        };
        if draw.icons != (draw.atlas.format == wgpu::TextureFormat::Rgba16Float) {
            return Err("icon atlas format does not match draw mode");
        }
        let packed = pack_quads(draw.quads);
        self.encode_packed(
            device,
            encoder,
            Packed {
                target: draw.target,
                depth: draw.depth,
                atlas: draw.atlas,
                space: draw.space,
                deformers: draw.deformers,
                lit,
                mode,
                id,
                glyphs: &packed,
            },
            profiler,
        )
    }

    fn encode_packed(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        packed: Packed<'_>,
        mut profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Result<(), &'static str> {
        let Packed {
            target,
            depth,
            atlas,
            space,
            deformers,
            lit,
            mode,
            id,
            glyphs: packed,
        } = packed;
        let deformed = matches!(
            space,
            TextSpace::Deformed { .. } | TextSpace::LitDeformed { .. }
        );
        if space.is_lit() != lit.is_some() {
            return Err(if space.is_lit() {
                "lit text needs the frame's lighting bindings"
            } else {
                "lighting bindings need lit text"
            });
        }
        if !matches!(space, TextSpace::Overlay { .. }) && depth.is_none() {
            return Err("surface text needs a depth view");
        }
        if deformed && !space.is_lit() && deformers.is_none() {
            return Err("deformed text needs the deformer bindings");
        }
        if matches!(
            space,
            TextSpace::Overlay { width: 0, .. } | TextSpace::Overlay { height: 0, .. }
        ) {
            return Err("overlay size must be positive");
        }
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text instances"),
            contents: bytemuck::cast_slice(packed),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let (view_projection, model, rest_normal, deformer) = match space {
            TextSpace::Deformed {
                view_projection,
                model,
                normal,
                lift,
                deformer,
                ..
            }
            | TextSpace::LitDeformed {
                view_projection,
                model,
                normal,
                lift,
                deformer,
                ..
            } => (
                view_projection,
                model,
                [normal[0], normal[1], normal[2], lift],
                deformer.0,
            ),
            TextSpace::LitSurface { model, .. } => {
                (identity, model, [0.0, 0.0, 1.0, 0.0], DeformerId::NONE.0)
            }
            _ => (identity, identity, [0.0, 0.0, 1.0, 0.0], DeformerId::NONE.0),
        };
        let uniform = Uniform {
            matrix: space.matrix(),
            view_projection,
            model,
            rest_normal,
            atlas_size: [atlas.width as f32, atlas.height as f32],
            mode,
            deformer,
            cells: [DEFORMED_CELLS, id.unwrap_or(0), 0, 0],
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("text uniform"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let lit_pipelines = lit.map(|_| {
            self.lit
                .get_or_init(|| LitPipelines::new(device, &self.gobo))
        });
        let bind = match (lit, lit_pipelines) {
            (Some(lit), Some(pipelines)) => device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lit text bind group"),
                layout: &pipelines.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: lit.deformers.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: lit.plan.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(atlas.view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }),
            _ => device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("text bind group"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(atlas.view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }),
        };
        let surface = !matches!(space, TextSpace::Overlay { .. });
        let depth_attachment =
            depth
                .filter(|_| surface)
                .map(|view| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                });
        let ids = id.is_some();
        let timing = profiler.as_deref_mut().and_then(|timer| {
            timer.pass(match (ids, surface, lit.is_some()) {
                (true, _, _) => "text ids",
                (false, false, _) => "overlay text",
                (false, true, true) => "lit surface text",
                (false, true, false) => "surface text",
            })
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(if ids { "text ids" } else { "text pass" }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth_attachment,
            timestamp_writes: profiler
                .as_deref()
                .and_then(|timer| timer.render_writes(timing)),
            occlusion_query_set: None,
        });
        if let Some(viewport) = self.viewport {
            crate::viewport::apply(&mut pass, viewport);
        }
        if let (Some(lit), Some(pipelines)) = (lit, lit_pipelines) {
            pass.set_pipeline(match (deformed, ids) {
                (true, false) => &pipelines.deformed,
                (false, false) => &pipelines.surface,
                (true, true) => &pipelines.deformed_ids,
                (false, true) => &pipelines.surface_ids,
            });
            pass.set_bind_group(0, lit.scene, &[]);
            pass.set_bind_group(1, lit.shadow, &[]);
            pass.set_bind_group(2, &bind, &[]);
            pass.set_bind_group(3, lit.lighting, &[]);
        } else {
            pass.set_pipeline(match (deformed, surface, ids) {
                (true, _, false) => &self.deformed,
                (false, true, false) => &self.surface,
                (false, false, false) => &self.overlay,
                (true, _, true) => &self.deformed_ids,
                (false, true, true) => &self.surface_ids,
                (false, false, true) => &self.overlay_ids,
            });
            pass.set_bind_group(0, &bind, &[]);
            if let Some(group) = deformers.filter(|_| deformed) {
                pass.set_bind_group(1, &self.empty_group, &[]);
                pass.set_bind_group(2, group, &[]);
            }
        }
        pass.set_vertex_buffer(0, instances.slice(..));
        let vertices = if deformed {
            6 * DEFORMED_CELLS * DEFORMED_CELLS
        } else {
            6
        };
        pass.draw(0..vertices, 0..packed.len() as u32);
        Ok(())
    }
}

struct Packed<'a> {
    target: &'a wgpu::TextureView,
    depth: Option<&'a wgpu::TextureView>,
    atlas: &'a GpuAtlas,
    space: TextSpace,
    deformers: Option<&'a wgpu::BindGroup>,
    lit: Option<&'a LitBindings<'a>>,
    mode: u32,
    id: Option<u32>,
    glyphs: &'a [PackedGlyph],
}

fn turned(angle: f32, pivot: [f32; 2], point: [f32; 2]) -> [f32; 2] {
    let (sin, cos) = angle.sin_cos();
    let offset = [point[0] - pivot[0], point[1] - pivot[1]];
    [
        pivot[0] + cos * offset[0] - sin * offset[1],
        pivot[1] + sin * offset[0] + cos * offset[1],
    ]
}

pub fn screen_bounds(
    space: &TextSpace,
    glyphs: impl IntoIterator<Item = PackedGlyph>,
    size: [u32; 2],
) -> Option<[f32; 4]> {
    let (rest, lift, clip_from_rest) = match *space {
        TextSpace::Deformed {
            view_projection,
            model,
            layout,
            normal,
            lift,
            ..
        }
        | TextSpace::LitDeformed {
            view_projection,
            model,
            layout,
            normal,
            lift,
            ..
        } => (
            layout,
            normal.map(|n| n * lift),
            crate::frame::multiply(view_projection, model),
        ),
        _ => (
            space.matrix(),
            [0.0; 3],
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        ),
    };
    let width = size[0] as f32;
    let height = size[1] as f32;
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for glyph in glyphs {
        let [x, y, angle, scale] = glyph.pose;
        let mut box_low = [f32::INFINITY; 2];
        let mut box_high = [f32::NEG_INFINITY; 2];
        for corner in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            let local = [
                (glyph.rect[0] + corner[0] * glyph.rect[2]) * scale,
                (glyph.rect[1] + corner[1] * glyph.rect[3]) * scale,
            ];
            let point = turned(angle, [0.0, 0.0], local);
            for (axis, offset) in [x, y].into_iter().enumerate() {
                box_low[axis] = box_low[axis].min(point[axis] + offset);
                box_high[axis] = box_high[axis].max(point[axis] + offset);
            }
        }
        let left = box_low[0].max(glyph.clip[0]);
        let top = box_low[1].max(glyph.clip[1]);
        let right = box_high[0].min(glyph.clip[2]);
        let bottom = box_high[1].min(glyph.clip[3]);
        if !(left < right && top < bottom) {
            continue;
        }
        for point in [[left, top], [right, top], [left, bottom], [right, bottom]] {
            let [u, v] = turned(glyph.turn[2], [glyph.turn[0], glyph.turn[1]], point);
            let placed = crate::frame::transform(rest, [u, v, 0.0, 1.0]);
            let clip = crate::frame::transform(
                clip_from_rest,
                [
                    placed[0] + lift[0],
                    placed[1] + lift[1],
                    placed[2] + lift[2],
                    placed[3],
                ],
            );
            if clip[3] <= 1e-6 {
                continue;
            }
            let pixel = [
                (clip[0] / clip[3] * 0.5 + 0.5) * width,
                (0.5 - clip[1] / clip[3] * 0.5) * height,
            ];
            for axis in 0..2 {
                low[axis] = low[axis].min(pixel[axis]);
                high[axis] = high[axis].max(pixel[axis]);
            }
        }
    }
    let rect = [
        low[0].max(0.0),
        low[1].max(0.0),
        high[0].min(width),
        high[1].min(height),
    ];
    (rect[0] < rect[2] && rect[1] < rect[3]).then_some(rect)
}

fn glyph_source() -> String {
    format!("const MSDF_SPREAD: f32 = {MSDF_SPREAD:?};\n{GLYPH_WGSL}")
}

pub fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        MSDF_WGSL,
        crate::frame::deform_source(),
        glyph_source(),
        SHADER
    )
}

pub fn lit_shader_source() -> String {
    lit_shader_source_with(&Gobo::default())
}

pub fn lit_shader_source_with(gobo: &Gobo) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        MSDF_WGSL,
        crate::frame::shader_source_with(gobo),
        glyph_source(),
        LIT_SHADER
    )
}

pub fn representation(atlas: &Atlas) -> Result<Representation, AtlasError> {
    match atlas.channels {
        1 => Ok(Representation::Coverage),
        3 => Ok(Representation::Msdf),
        _ => Err(AtlasError::InvalidChannels),
    }
}

pub fn text_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
}

pub fn deformer_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_text::{Anchor, Baseline, MipLevel, Style, TextEngine};

    const NUMBERS: &str = "0123456789,.e+-Na";

    const FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;

    const LEGACY_SHADER: &str = r#"
struct Transform {
    matrix: mat4x4f,
    view_projection: mat4x4f,
    model: mat4x4f,
    rest_normal: vec4f,
    atlas_size: vec2f,
    mode: u32,
    deformer: u32,
    cells: vec4u,
}
@group(0) @binding(0) var<uniform> transform: Transform;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    @location(0) pose: vec4f,
    @location(1) rect: vec4f,
    @location(2) uv_rect: vec4f,
    @location(3) color: vec4f,
    @location(4) clip_rect: vec4f,
    @location(5) turn: vec4f,
}
struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) bounds: vec4f,
    @location(2) @interpolate(flat) color: vec4f,
    @location(3) @interpolate(flat) clip_rect: vec4f,
    @location(4) point: vec2f,
}

@vertex
fn vs_main(instance: Instance, @builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0)
    );
    let corner = corners[index];
    let local = (instance.rect.xy + corner * instance.rect.zw) * instance.pose.w;
    let c = cos(instance.pose.z);
    let s = sin(instance.pose.z);
    let point = instance.pose.xy + vec2f(c * local.x - s * local.y, s * local.x + c * local.y);
    let pivot = instance.turn.xy;
    let ct = cos(instance.turn.z);
    let st = sin(instance.turn.z);
    let offset = point - pivot;
    let turned = pivot + vec2f(ct * offset.x - st * offset.y, st * offset.x + ct * offset.y);
    var out: VertexOut;
    out.clip = transform.matrix * vec4f(turned, 0.0, 1.0);
    out.uv = mix(instance.uv_rect.xy, instance.uv_rect.zw, corner);
    out.bounds = instance.uv_rect;
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.point = point;
    return out;
}

@vertex
fn vs_deformed(instance: Instance, @builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0)
    );
    let cells = max(transform.cells.x, 1u);
    let cell = index / 6u;
    let corner = (vec2f(f32(cell % cells), f32(cell / cells)) + corners[index % 6u]) / f32(cells);
    let local = (instance.rect.xy + corner * instance.rect.zw) * instance.pose.w;
    let c = cos(instance.pose.z);
    let s = sin(instance.pose.z);
    let point = instance.pose.xy + vec2f(c * local.x - s * local.y, s * local.x + c * local.y);
    let pivot = instance.turn.xy;
    let ct = cos(instance.turn.z);
    let st = sin(instance.turn.z);
    let offset = point - pivot;
    let turned = pivot + vec2f(ct * offset.x - st * offset.y, st * offset.x + ct * offset.y);
    let rest = (transform.matrix * vec4f(turned, 0.0, 1.0)).xyz;
    let shape = deform_vertex(rest, transform.rest_normal.xyz, vec4f(1.0, 0.0, 0.0, 1.0), transform.deformer);
    let lifted = shape.position + normalize(shape.normal) * transform.rest_normal.w;
    var out: VertexOut;
    out.clip = transform.view_projection * transform.model * vec4f(lifted, 1.0);
    out.uv = mix(instance.uv_rect.xy, instance.uv_rect.zw, corner);
    out.bounds = instance.uv_rect;
    out.color = instance.color;
    out.clip_rect = instance.clip_rect;
    out.point = point;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4f {
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let footprint = max(length(dx * transform.atlas_size), length(dy * transform.atlas_size));
    let extent = (in.bounds.zw - in.bounds.xy) * transform.atlas_size;
    let safe_lod = floor(log2(max(min(extent.x, extent.y), 1.0)));
    let max_lod = select(min(safe_lod, 3.0), 3.0, transform.mode == 2u);
    let lod = clamp(log2(max(footprint, 1.0)), 0.0, max_lod);
    let level = select(lod, 0.0, transform.mode == 1u);
    let half_texel = vec2f(exp2(level) * 0.5) / transform.atlas_size;
    let low = min(in.bounds.xy + half_texel, in.bounds.zw);
    let high = max(in.bounds.zw - half_texel, low);
    let uv = clamp(in.uv, low, high);
    let sampled = textureSampleLevel(atlas, atlas_sampler, uv, level);
    var alpha = sampled.r;
    if (transform.mode == 1u) {
        let sampled = textureSampleLevel(atlas, atlas_sampler, uv, 0.0);
        let signed_distance = msdf_median(sampled.rgb) - 0.5;
        alpha = clamp(0.5 + signed_distance / max(fwidth(signed_distance), 1e-5), 0.0, 1.0);
    }
    let inside = step(in.clip_rect.x, in.point.x) * step(in.point.x, in.clip_rect.z)
        * step(in.clip_rect.y, in.point.y) * step(in.point.y, in.clip_rect.w);
    if (transform.mode == 2u) {
        let opacity = sampled.a * in.color.a * inside;
        return vec4f(sampled.rgb * in.color.rgb * in.color.a * inside, opacity);
    }
    let opacity = alpha * in.color.a * inside;
    return vec4f(in.color.rgb * opacity, opacity);
}
"#;

    #[test]
    fn rich_glyph_conversion_keeps_layout_and_premultiplied_alpha() {
        let source = pfx_text::RichGlyphQuad {
            key: 7,
            atlas_page: pfx_text::AtlasPage::Monochrome,
            is_color: false,
            rect: [1.0, 2.0, 3.0, 4.0],
            uv: [0.1, 0.2, 0.3, 0.4],
            origin: [5.0, 6.0],
            alpha: 0.4,
            clip: [7.0, 8.0, 9.0, 10.0],
            turn: [11.0, 12.0, 0.3],
            color: [0.2, 0.4, 0.6, 0.32],
            line_index: 0,
            line_baseline: 6.0,
            pen: [5.0, 6.0],
        };
        let quad = RichQuad::from(&source);
        assert_eq!(quad.rect, source.rect);
        assert_eq!(quad.uv, source.uv);
        assert_eq!(quad.origin, source.origin);
        assert_eq!(quad.color, source.color);
        assert_eq!(quad.alpha, 1.0);
        assert_eq!(quad.clip, source.clip);
        assert_eq!(quad.turn, source.turn);
        assert_eq!(PackedGlyph::from(&quad).color[3], 0.32);
    }

    fn paragraph(size: f32) -> Paragraph {
        let mut engine = TextEngine::new(FONT).unwrap();
        engine
            .layout(
                "Paper",
                Style {
                    family: "EB Garamond",
                    size,
                    line_height: size * 1.2,
                    wrap_width: None,
                    pixels_per_unit: 1.0,
                    representation: Representation::Msdf,
                },
                &Baseline::Straight {
                    origin: [40.0, 650.0],
                    direction: [1.0, 0.0],
                },
            )
            .unwrap()
    }

    #[test]
    fn instance_packing_preserves_layout() {
        let paragraph = paragraph(48.0);
        let packed = pack_glyphs(&paragraph.glyphs);
        assert_eq!(packed.len(), 5);
        assert_eq!(std::mem::size_of::<PackedGlyph>(), 96);
        for (source, output) in paragraph.glyphs.iter().zip(packed) {
            assert_eq!(
                output.pose,
                [
                    source.position[0],
                    source.position[1],
                    source.rotation,
                    source.scale
                ]
            );
            assert_eq!(output.rect, source.local_rect);
            assert_eq!(output.uv_rect, source.uv);
        }
    }

    #[test]
    fn atlas_upload_sizes_mips_and_dirty_rows() {
        let mut atlas = Atlas {
            channels: 3,
            levels: vec![
                MipLevel {
                    width: 4,
                    height: 4,
                    bytes: vec![128; 48],
                },
                MipLevel {
                    width: 2,
                    height: 2,
                    bytes: vec![128; 12],
                },
                MipLevel {
                    width: 1,
                    height: 1,
                    bytes: vec![128; 3],
                },
            ],
        };
        let initial = converted_levels(&atlas).unwrap();
        assert_eq!(
            initial.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![64, 16, 4]
        );
        assert_eq!(&initial[0][..4], &[128, 128, 128, 255]);
        assert_eq!(
            changed_regions(&atlas, &initial, None),
            vec![
                UploadRegion {
                    level: 0,
                    first_row: 0,
                    rows: 4,
                    first_column: 0,
                    columns: 4,
                },
                UploadRegion {
                    level: 1,
                    first_row: 0,
                    rows: 2,
                    first_column: 0,
                    columns: 2,
                },
                UploadRegion {
                    level: 2,
                    first_row: 0,
                    rows: 1,
                    first_column: 0,
                    columns: 1,
                },
            ]
        );
        atlas.levels[0].bytes[13] = 129;
        atlas.levels[1].bytes[7] = 129;
        let updated = converted_levels(&atlas).unwrap();
        assert_eq!(
            changed_regions(&atlas, &updated, Some(&initial)),
            vec![
                UploadRegion {
                    level: 0,
                    first_row: 1,
                    rows: 1,
                    first_column: 0,
                    columns: 4,
                },
                UploadRegion {
                    level: 1,
                    first_row: 1,
                    rows: 1,
                    first_column: 0,
                    columns: 2,
                },
            ]
        );
        let coverage = Atlas {
            channels: 1,
            levels: vec![
                MipLevel {
                    width: 2,
                    height: 2,
                    bytes: vec![0, 1, 2, 3],
                },
                MipLevel {
                    width: 1,
                    height: 1,
                    bytes: vec![1],
                },
            ],
        };
        assert_eq!(format_for(&coverage), Ok(wgpu::TextureFormat::R8Unorm));
        assert_eq!(converted_levels(&coverage).unwrap()[0].len(), 4);
    }

    #[test]
    fn icon_mips_keep_neighbours_separate() {
        let mut cache = Cache::new();
        let red = [65535u16, 0, 0, 65535];
        let blue = [0u16, 0, 65535, 65535];
        let raw = |pixel: [u16; 4]| {
            (0..16)
                .flat_map(|_| pixel.into_iter().flat_map(u16::to_le_bytes))
                .collect::<Vec<_>>()
        };
        let a = cache
            .load_rgba16(&raw(red), 4, 4, ColorSpace::Linear)
            .unwrap();
        let b = cache
            .load_rgba16(&raw(blue), 4, 4, ColorSpace::Linear)
            .unwrap();
        let atlas = IconAtlas::from_cache(&cache, &[a, b]).unwrap();
        assert_eq!(atlas.levels.len(), 4);
        for (index, level) in atlas.levels.iter().enumerate() {
            for (handle, expected) in [(a, red), (b, blue)] {
                let uv = atlas.uv(handle).unwrap();
                let half = 2f32.powi(index as i32) * 0.5 / atlas.levels[0].width as f32;
                let low_x = (uv[0] + half).min(uv[2]);
                let high_x = (uv[2] - half).max(low_x);
                let half_y = 2f32.powi(index as i32) * 0.5 / atlas.levels[0].height as f32;
                let low_y = (uv[1] + half_y).min(uv[3]);
                let high_y = (uv[3] - half_y).max(low_y);
                for u in [low_x, (low_x + high_x) * 0.5, high_x] {
                    for v in [low_y, (low_y + high_y) * 0.5, high_y] {
                        let x = (u * level.width as f32 - 0.5).floor() as i32;
                        let y = (v * level.height as f32 - 0.5).floor() as i32;
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let sx = (x + dx).clamp(0, level.width as i32 - 1) as usize;
                                let sy = (y + dy).clamp(0, level.height as i32 - 1) as usize;
                                let at = (sy * level.width as usize + sx) * 4;
                                assert_eq!(&level.pixels[at..at + 4], &expected, "mip {index}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn glyph_mip_samples_stay_in_the_padded_cell() {
        let mut levels = Vec::new();
        for index in 0..4 {
            let width = 64u32 >> index;
            let height = 32u32 >> index;
            let mut bytes = vec![0u8; (width * height) as usize];
            let cell = 24u32 >> index;
            for y in 0..cell {
                for x in 0..cell {
                    bytes[(y * width + x) as usize] = 80;
                    bytes[(y * width + x + cell) as usize] = 200;
                }
            }
            levels.push(MipLevel {
                width,
                height,
                bytes,
            });
        }
        let atlas = Atlas {
            channels: 1,
            levels,
        };
        assert_eq!(converted_levels(&atlas).unwrap().len(), 4);
        for (index, level) in atlas.levels.iter().enumerate() {
            for (rect_x, expected) in [(8u32, 80u8), (32u32, 200u8)] {
                let low = (rect_x as f32 + 2f32.powi(index as i32) * 0.5) / 64.0;
                let high = ((rect_x + 8) as f32 - 2f32.powi(index as i32) * 0.5) / 64.0;
                let u = (low + high) * 0.5;
                let v = 12.0 / 32.0;
                let x = (u * level.width as f32 - 0.5).floor() as usize;
                let y = (v * level.height as f32 - 0.5).floor() as usize;
                for dy in 0..2 {
                    for dx in 0..2 {
                        assert_eq!(
                            level.bytes[(y + dy) * level.width as usize + x + dx],
                            expected,
                            "mip {index}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clip_and_rotation_keep_the_unturned_clip_space() {
        let quad = RichQuad {
            rect: [10.0, 20.0, 8.0, 6.0],
            uv: [0.0, 0.0, 0.5, 0.5],
            origin: [10.0, 20.0],
            alpha: 0.5,
            clip: [12.0, 20.0, 18.0, 26.0],
            turn: [10.0, 20.0, std::f32::consts::FRAC_PI_2],
            color: [0.25, 0.5, 0.75, 0.8],
        };
        let packed = PackedGlyph::from(&quad);
        assert!((packed.color[3] - 0.4).abs() < 1e-6);
        let point = [quad.rect[0] + 4.0, quad.rect[1] + 3.0];
        let inside = point[0] >= quad.clip[0]
            && point[0] <= quad.clip[2]
            && point[1] >= quad.clip[1]
            && point[1] <= quad.clip[3];
        assert!(inside);
        let offset = [point[0] - quad.turn[0], point[1] - quad.turn[1]];
        let turned = [quad.turn[0] - offset[1], quad.turn[1] + offset[0]];
        assert_eq!(turned, [7.0, 24.0]);
        assert!(turned[0] < quad.clip[0]);
    }

    #[test]
    fn assembled_wgsl_parses_and_validates() {
        let source = shader_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn lit_wgsl_validates_and_binds_inside_the_frame_layouts() {
        let source = lit_shader_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let layouts = [
            crate::frame::scene_entries(),
            crate::frame::shadow_entries(),
            lit_entries(),
            crate::frame::lighting_entries(),
        ];
        let mut used = 0;
        for name in ["vs_lit_surface", "vs_lit_deformed", "fs_lit"] {
            let index = module
                .entry_points
                .iter()
                .position(|entry| entry.name == name)
                .unwrap();
            let stage = match module.entry_points[index].stage {
                naga::ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
                _ => wgpu::ShaderStages::FRAGMENT,
            };
            let function = info.get_entry_point(index);
            for (handle, variable) in module.global_variables.iter() {
                let Some(binding) = &variable.binding else {
                    continue;
                };
                if function[handle].is_empty() {
                    continue;
                }
                let entry = layouts[binding.group as usize]
                    .iter()
                    .find(|entry| entry.binding == binding.binding)
                    .unwrap_or_else(|| {
                        panic!(
                            "{name} uses @group({}) @binding({}) missing from the layout",
                            binding.group, binding.binding
                        )
                    });
                assert!(
                    entry.visibility.contains(stage),
                    "{name} @group({}) @binding({}) is not visible to its stage",
                    binding.group,
                    binding.binding
                );
                used += 1;
            }
        }
        assert!(used >= 20, "{used} lit text bindings checked");
    }

    fn project(matrix: [[f32; 4]; 4], point: [f32; 3]) -> [f32; 4] {
        let mut out = [0.0; 4];
        for (row, value) in out.iter_mut().enumerate() {
            *value = (0..3).map(|col| matrix[col][row] * point[col]).sum::<f32>() + matrix[3][row];
        }
        out
    }

    fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> [[f32; 4]; 4] {
        let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let unit = |a: [f32; 3]| {
            let length = dot(a, a).sqrt();
            [a[0] / length, a[1] / length, a[2] / length]
        };
        let forward = unit(sub(target, eye));
        let right = unit(cross(forward, up));
        let above = cross(right, forward);
        [
            [right[0], above[0], -forward[0], 0.0],
            [right[1], above[1], -forward[1], 0.0],
            [right[2], above[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(above, eye), dot(forward, eye), 1.0],
        ]
    }

    fn perspective(aspect: f32) -> [[f32; 4]; 4] {
        let near = 0.1;
        let far = 100.0;
        let f = 1.0 / (40.0_f32.to_radians() * 0.5).tan();
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ]
    }

    fn solid_atlas() -> Atlas {
        let level = |size: u32| MipLevel {
            width: size,
            height: size,
            bytes: vec![255; (size * size) as usize],
        };
        Atlas {
            channels: 1,
            levels: vec![level(8), level(4), level(2), level(1)],
        }
    }

    fn solid_quad(center: [f32; 2], half: f32, color: [f32; 4]) -> RichQuad {
        RichQuad {
            rect: [center[0] - half, center[1] - half, 2.0 * half, 2.0 * half],
            uv: [0.0, 0.0, 1.0, 1.0],
            origin: center,
            alpha: 1.0,
            clip: [-1000.0, -1000.0, 1000.0, 1000.0],
            turn: [0.0; 3],
            color,
        }
    }

    #[test]
    fn screen_bounds_follow_the_space_clip_and_turn() {
        let space = TextSpace::Overlay {
            width: 200,
            height: 100,
        };
        let quad = solid_quad([50.0, 20.0], 10.0, INK);
        let bounds = |quads: &[RichQuad], space: &TextSpace| {
            screen_bounds(space, quads.iter().map(PackedGlyph::from), [400, 200])
        };
        let close = |rect: Option<[f32; 4]>, want: [f32; 4]| {
            rect.is_some_and(|rect| {
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() < 1e-3)
            })
        };
        assert!(close(bounds(&[quad], &space), [80.0, 20.0, 120.0, 60.0]));
        let clipped = RichQuad {
            clip: [45.0, -1000.0, 1000.0, 1000.0],
            ..quad
        };
        assert!(close(bounds(&[clipped], &space), [90.0, 20.0, 120.0, 60.0]));
        let hidden = RichQuad {
            clip: [100.0, -1000.0, 1000.0, 1000.0],
            ..quad
        };
        assert_eq!(bounds(&[hidden], &space), None);
        let turned = RichQuad {
            turn: [50.0, 20.0, std::f32::consts::FRAC_PI_4],
            ..quad
        };
        let rect = bounds(&[turned], &space).unwrap();
        let half = 10.0 * std::f32::consts::SQRT_2 * 2.0;
        for (got, want) in rect
            .iter()
            .zip([100.0 - half, 40.0 - half, 100.0 + half, 40.0 + half])
        {
            assert!((got - want).abs() < 1e-3, "{rect:?}");
        }
        let far = solid_quad([500.0, 20.0], 10.0, INK);
        assert!(close(
            bounds(&[quad, far], &space),
            [80.0, 20.0, 400.0, 60.0]
        ));
        let surface = TextSpace::Surface {
            model_view_projection: [
                [0.01, 0.0, 0.0, 0.0],
                [0.0, -0.01, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        assert!(close(
            bounds(&[quad], &surface),
            [280.0, 110.0, 320.0, 130.0]
        ));
        let behind = TextSpace::Surface {
            model_view_projection: [
                [0.01, 0.0, 0.0, 0.0],
                [0.0, -0.01, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, -1.0],
            ],
        };
        assert_eq!(bounds(&[quad], &behind), None);
    }

    const LAB: u32 = 256;
    const INK: [f32; 4] = [0.8, 0.6, 0.4, 1.0];

    struct Lab {
        size: [u32; 2],
        renderer: crate::renderer::Renderer,
        output: pfx_gpu::OffscreenTarget,
        atlas: GpuAtlas,
        view: [[f32; 4]; 4],
        projection: [[f32; 4]; 4],
        eye: [f32; 3],
        meshes: Vec<crate::frame::MeshHandle>,
        text_id: u32,
    }

    impl Lab {
        fn new(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Self {
            Self::sized([LAB, LAB], eye, target, up)
        }

        fn sized(size: [u32; 2], eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Self {
            let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
            let mut renderer = crate::renderer::Renderer::new(gpu, size[0], size[1]).unwrap();
            renderer.set_text_after_taa(false);
            renderer.set_shadow_quality(crate::shadow::Quality {
                resolution: 1024,
                receiver: Some(crate::shadow::ReceiverBox {
                    min: [-3.0, -0.1, -3.0],
                    max: [3.0, 1.5, 3.0],
                }),
                caster_margin: 4.0,
                ..crate::shadow::Quality::default()
            });
            let format = renderer.output_format();
            let texture = renderer
                .gpu()
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("lit text lab output"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
            let output = pfx_gpu::OffscreenTarget {
                view: texture.create_view(&Default::default()),
                texture,
                format,
                width: size[0],
                height: size[1],
            };
            let atlas = GpuAtlas::new(
                &renderer.gpu().device,
                &renderer.gpu().queue,
                &solid_atlas(),
            )
            .unwrap();
            Self {
                size,
                renderer,
                output,
                atlas,
                view: look_at(eye, target, up),
                projection: perspective(size[0] as f32 / size[1] as f32),
                eye,
                meshes: Vec::new(),
                text_id: 0,
            }
        }

        fn view_projection(&self) -> [[f32; 4]; 4] {
            crate::frame::multiply(self.projection, self.view)
        }

        fn occluder(&mut self, min: [f32; 2], max: [f32; 2], height: f32) {
            let positions = [
                [min[0], height, min[1]],
                [max[0], height, min[1]],
                [max[0], height, max[1]],
                [min[0], height, max[1]],
            ];
            let mesh = self
                .renderer
                .upload_mesh(crate::frame::MeshData {
                    positions: &positions,
                    normals: &[[0.0, 1.0, 0.0]; 4],
                    tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                    uvs: &[[0.0, 0.0]; 4],
                    uvs1: None,
                    alpha: None,
                    indices: &[0, 2, 1, 0, 3, 2],
                })
                .unwrap();
            self.meshes.push(mesh);
        }

        fn render(
            &mut self,
            sun_intensity: f32,
            sun: [f32; 3],
            deformers: &[crate::deform::Deformer],
            quads: &[RichQuad],
            space: TextSpace,
            lights: &[crate::lights::LocalLight],
        ) -> Vec<f32> {
            self.render_timed(sun_intensity, sun, deformers, quads, space, lights);
            self.renderer
                .gpu()
                .readback_rgba16(&self.renderer.frame().targets.hdr)
                .unwrap()
                .into_iter()
                .map(|value| half::f16::from_bits(value).to_f32())
                .collect()
        }

        fn render_timed(
            &mut self,
            sun_intensity: f32,
            sun: [f32; 3],
            deformers: &[crate::deform::Deformer],
            quads: &[RichQuad],
            space: TextSpace,
            lights: &[crate::lights::LocalLight],
        ) -> Vec<pfx_gpu::PassTiming> {
            self.renderer.set_lights(lights).unwrap();
            let mut model = [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];
            model[3][1] = 0.0;
            let instances: Vec<_> = self
                .meshes
                .iter()
                .enumerate()
                .map(|(index, mesh)| {
                    let mut instance =
                        crate::frame::Instance::new(*mesh, model, 0, index as u32 + 1);
                    instance.shadow_only = true;
                    instance
                })
                .collect();
            let scene = crate::frame::Scene {
                camera: crate::frame::Camera {
                    view: self.view,
                    projection: self.projection,
                    previous_view_projection: self.view_projection(),
                    position: self.eye,
                },
                time: 0.0,
                seed: 3,
                sun: crate::frame::Sun {
                    direction: sun,
                    colour: [1.0; 3],
                    intensity: sun_intensity,
                },
                instances: &instances,
                materials: &[pfx_materials::Material::default()],
                deformers,
                wind: crate::frame::SceneWind::default(),
            };
            let text = crate::renderer::Text {
                surface_quads: vec![crate::renderer::TextQuadItem {
                    atlas: &self.atlas,
                    quads,
                    space,
                    icons: false,
                    id: self.text_id,
                }],
                ..Default::default()
            };
            self.renderer
                .render(
                    &scene,
                    &text,
                    &crate::renderer::Effects::default(),
                    crate::renderer::Finish::Standard,
                    &self.output.view,
                )
                .unwrap()
        }

        fn at(&self, pixels: &[f32], point: [f32; 3]) -> [f32; 3] {
            let clip = project(self.view_projection(), point);
            let x = ((clip[0] / clip[3] * 0.5 + 0.5) * self.size[0] as f32) as usize;
            let y = ((0.5 - clip[1] / clip[3] * 0.5) * self.size[1] as f32) as usize;
            let base = (y * self.size[0] as usize + x) * 4;
            [pixels[base], pixels[base + 1], pixels[base + 2]]
        }
    }

    fn floor_model() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn ink_space(lab: &Lab, lit: bool) -> TextSpace {
        let model = floor_model();
        let model_view_projection = crate::frame::multiply(lab.view_projection(), model);
        if lit {
            TextSpace::LitSurface {
                model_view_projection,
                model,
            }
        } else {
            TextSpace::Surface {
                model_view_projection,
            }
        }
    }

    fn near(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(a.abs()).max(0.02)
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn lit_glyph_in_the_cascade_shadow_is_darker_by_the_suns_share() {
        let mut lab = Lab::new([0.0, 6.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, -1.0]);
        lab.occluder([-2.0, -1.0], [0.0, 1.0], 1.0);
        let quads = [
            solid_quad([-1.0, 0.0], 0.15, INK),
            solid_quad([1.0, 0.0], 0.15, INK),
        ];
        let sun = [0.0, 1.0, 0.0];
        let space = ink_space(&lab, true);
        let lit = lab.render(3.0, sun, &[], &quads, space, &[]);
        let dark = lab.render(0.0, sun, &[], &quads, space, &[]);
        let shadowed = lab.at(&lit, [-1.0, 0.0, 0.0]);
        let sunlit = lab.at(&lit, [1.0, 0.0, 0.0]);
        let ambient = lab.at(&dark, [1.0, 0.0, 0.0]);
        for channel in 0..3 {
            let sun_share = INK[channel] * 3.0 / std::f32::consts::PI;
            assert!(
                shadowed[channel] < sunlit[channel],
                "channel {channel}: {shadowed:?} against {sunlit:?}"
            );
            assert!(
                near(sunlit[channel] - ambient[channel], sun_share, 0.03),
                "channel {channel}: sun share {} against {sun_share}",
                sunlit[channel] - ambient[channel]
            );
            assert!(
                near(shadowed[channel], ambient[channel], 0.03),
                "channel {channel}: shadowed {} against ambient {}",
                shadowed[channel],
                ambient[channel]
            );
        }
        let factor = (shadowed[0] - ambient[0]) / (sunlit[0] - ambient[0]);
        assert!(factor.abs() < 0.03, "shadow factor {factor}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn unlit_text_ignores_sun_shadow_and_night() {
        let mut lab = Lab::new([0.0, 6.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, -1.0]);
        lab.occluder([-2.0, -1.0], [0.0, 1.0], 1.0);
        let quads = [
            solid_quad([-1.0, 0.0], 0.15, INK),
            solid_quad([1.0, 0.0], 0.15, INK),
        ];
        let sun = [0.0, 1.0, 0.0];
        for intensity in [3.0, 0.0] {
            let space = ink_space(&lab, false);
            let pixels = lab.render(intensity, sun, &[], &quads, space, &[]);
            for point in [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]] {
                let value = lab.at(&pixels, point);
                for channel in 0..3 {
                    assert!(
                        near(value[channel], INK[channel], 0.002),
                        "unlit {value:?} at {point:?} under sun {intensity}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn lit_glyph_on_a_roll_takes_the_deformed_normal() {
        let mut lab = Lab::new([-3.0, 0.5, 4.0], [0.0, 0.4, 0.0], [0.0, 1.0, 0.0]);
        let roller = crate::deform::RollerFrame {
            at: 0.0,
            outer: 0.5,
            thickness: 0.0,
        };
        let deformers = [crate::deform::Deformer::Roller {
            current: roller,
            previous: roller,
        }];
        let flat = [-0.7_f32, 0.0];
        let curled = [0.5 * std::f32::consts::FRAC_PI_2, 0.0];
        let quads = [solid_quad(flat, 0.05, INK), solid_quad(curled, 0.05, INK)];
        let layout = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let space = TextSpace::LitDeformed {
            view_projection: lab.view_projection(),
            model: identity,
            layout,
            normal: [0.0, 1.0, 0.0],
            lift: 0.0003,
            deformer: DeformerId(0),
        };
        let sun = [-1.0, 0.0, 0.0];
        let lit = lab.render(3.0, sun, &deformers, &quads, space, &[]);
        let dark = lab.render(0.0, sun, &deformers, &quads, space, &[]);
        let flat_point = [flat[0], 0.0003, 0.0];
        let curled_point = [0.5, 0.5, 0.0];
        let on_flat = lab.at(&lit, flat_point);
        let off_flat = lab.at(&dark, flat_point);
        let on_curl = lab.at(&lit, curled_point);
        let off_curl = lab.at(&dark, curled_point);
        for channel in 0..3 {
            let sun_share = INK[channel] * 3.0 / std::f32::consts::PI;
            assert!(
                on_flat[channel] - off_flat[channel] < 0.02 * sun_share,
                "the flat glyph's normal is up: {on_flat:?} against {off_flat:?}"
            );
            assert!(
                near(on_curl[channel] - off_curl[channel], sun_share, 0.05),
                "the curled glyph faces the sun: {} against {sun_share}",
                on_curl[channel] - off_curl[channel]
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn lit_glyph_takes_a_local_light() {
        let mut lab = Lab::new([0.0, 6.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, -1.0]);
        let quads = [solid_quad([0.0, 0.0], 0.15, INK)];
        let sun = [0.0, 1.0, 0.0];
        let space = ink_space(&lab, true);
        let bare = lab.render(0.0, sun, &[], &quads, space, &[]);
        let lamp = crate::lights::LocalLight {
            position: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 2.0,
            radius: 0.01,
            range: 4.0,
            shadow: false,
        };
        let lighted = lab.render(0.0, sun, &[], &quads, space, &[lamp]);
        let before = lab.at(&bare, [0.0, 0.0, 0.0]);
        let after = lab.at(&lighted, [0.0, 0.0, 0.0]);
        let window = 1.0 - (1.0_f32 / 4.0).powi(4);
        let irradiance = 2.0 * window * window / 1.0;
        for channel in 0..3 {
            let expected = INK[channel] * irradiance / std::f32::consts::PI;
            assert!(
                near(after[channel] - before[channel], expected, 0.05),
                "channel {channel}: {} against {expected}",
                after[channel] - before[channel]
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn unlit_text_is_byte_identical_to_the_shader_before_lit_text() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let paragraph = TextEngine::new(FONT)
            .unwrap()
            .layout(
                "Paper",
                Style {
                    family: "EB Garamond",
                    size: 96.0,
                    line_height: 115.0,
                    wrap_width: None,
                    pixels_per_unit: 1.0,
                    representation: Representation::Msdf,
                },
                &Baseline::Straight {
                    origin: [20.0, 160.0],
                    direction: [1.0, 0.0],
                },
            )
            .unwrap();
        let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
        let current = TextPass::new(&gpu.device);
        let mut before = TextPass::new(&gpu.device);
        let source = format!(
            "{}\n{}\n{}",
            MSDF_WGSL,
            crate::frame::deform_source(),
            LEGACY_SHADER
        );
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("legacy text shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("legacy text layout"),
                bind_group_layouts: &[&before.bind_layout],
                push_constant_ranges: &[],
            });
        before.overlay = text_pipeline(
            &gpu.device,
            "legacy overlay",
            &layout,
            &shader,
            ("vs_main", "fs_main"),
            false,
            false,
        );
        let draw = |pass: &TextPass, color: [f32; 4]| {
            let target = gpu.offscreen(512, 256, COLOR_FORMAT).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("legacy clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            pass.encode(
                &gpu.device,
                &mut encoder,
                TextDraw {
                    target: &target.view,
                    depth: None,
                    atlas: &atlas,
                    paragraph: &paragraph,
                    space: TextSpace::Overlay {
                        width: 512,
                        height: 256,
                    },
                    color,
                    deformers: None,
                },
            )
            .unwrap();
            gpu.queue.submit(Some(encoder.finish()));
            gpu.readback_rgba16(&target).unwrap()
        };
        let reference = draw(&before, [0.7, 0.2, 0.1, 0.9]);
        assert!(reference.iter().any(|value| *value != 0));
        assert_eq!(reference, draw(&current, [0.7, 0.2, 0.1, 0.9]));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn surface_text_seen_nearly_face_on_keeps_the_isotropic_coverage() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let size = [512u32, 256];
        let paragraph = TextEngine::new(FONT)
            .unwrap()
            .layout(
                "Quartz",
                Style {
                    family: "EB Garamond",
                    size: 96.0,
                    line_height: 115.0,
                    wrap_width: None,
                    pixels_per_unit: 1.0,
                    representation: Representation::Msdf,
                },
                &Baseline::Straight {
                    origin: [20.0, 160.0],
                    direction: [1.0, 0.0],
                },
            )
            .unwrap();
        let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
        let current = TextPass::new(&gpu.device);
        let mut before = TextPass::new(&gpu.device);
        let source = format!(
            "{}\n{}\n{}",
            MSDF_WGSL,
            crate::frame::deform_source(),
            LEGACY_SHADER
        );
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("legacy text shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("legacy text layout"),
                bind_group_layouts: &[&before.bind_layout],
                push_constant_ranges: &[],
            });
        before.surface = text_pipeline(
            &gpu.device,
            "legacy surface",
            &layout,
            &shader,
            ("vs_main", "fs_main"),
            true,
            false,
        );
        let depth = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("face-on depth"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let draw = |pass: &TextPass, model_view_projection: [[f32; 4]; 4]| {
            let target = gpu.offscreen(size[0], size[1], COLOR_FORMAT).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("face-on clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            pass.encode(
                &gpu.device,
                &mut encoder,
                TextDraw {
                    target: &target.view,
                    depth: Some(&depth),
                    atlas: &atlas,
                    paragraph: &paragraph,
                    space: TextSpace::Surface {
                        model_view_projection,
                    },
                    color: [0.7, 0.2, 0.1, 0.9],
                    deformers: None,
                },
            )
            .unwrap();
            gpu.queue.submit(Some(encoder.finish()));
            gpu.readback_rgba16(&target)
                .unwrap()
                .into_iter()
                .map(|value| half::f16::from_bits(value).to_f32())
                .collect::<Vec<_>>()
        };
        let metre = 1.0 / 256.0;
        let model = [
            [metre, 0.0, 0.0, 0.0],
            [0.0, -metre, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-256.0 * metre, 128.0 * metre, 0.0, 1.0],
        ];
        let aspect = size[0] as f32 / size[1] as f32;
        for degrees in [90.0f32, 85.0, 80.0] {
            let (s, c) = degrees.to_radians().sin_cos();
            let view = look_at([0.0, -1.5 * c, 1.5 * s], [0.0; 3], [0.0, 1.0, 0.0]);
            let model_view_projection =
                crate::frame::multiply(crate::frame::multiply(perspective(aspect), view), model);
            let reference = draw(&before, model_view_projection);
            let ours = draw(&current, model_view_projection);
            assert!(reference.iter().any(|value| *value > 0.0));
            let worst = reference
                .iter()
                .zip(&ours)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            let changed = reference.iter().zip(&ours).filter(|(a, b)| a != b).count();
            println!(
                "surface text at {degrees} degrees: {changed} values changed, worst {worst:.5}"
            );
            assert!(worst <= 1.0 / 255.0, "{degrees} degrees: {worst}");
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn two_thousand_lit_glyphs_at_4k_stay_under_budget() {
        let mut lab = Lab::sized(
            [3840, 2160],
            [0.0, 6.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
        );
        lab.occluder([-2.0, -1.0], [0.0, 1.0], 1.0);
        let quads = (0..2000)
            .map(|index| {
                let x = -2.1 + (index % 100) as f32 * 0.043;
                let z = -1.0 + (index / 100) as f32 * 0.105;
                solid_quad([x, z], 0.0125, INK)
            })
            .collect::<Vec<_>>();
        let lamp = crate::lights::LocalLight {
            position: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 2.0,
            radius: 0.05,
            range: 4.0,
            shadow: true,
        };
        let mut median = |label: &str, lit: bool| {
            let space = ink_space(&lab, lit);
            let mut samples = Vec::new();
            let mut turns = pfx_gpu::pace::Turns::default();
            for frame in 0..14 {
                let timings = lab.render_timed(3.0, [0.0, 1.0, 0.0], &[], &quads, space, &[lamp]);
                turns.add(timings.iter().map(|timing| timing.milliseconds).sum());
                if frame >= 4 {
                    samples.push(
                        timings
                            .iter()
                            .find(|timing| timing.label == label)
                            .unwrap_or_else(|| panic!("no {label} timing in {timings:?}"))
                            .milliseconds,
                    );
                }
            }
            samples.sort_by(f64::total_cmp);
            samples[samples.len() / 2]
        };
        let plain = median("surface text", false);
        let lit = median("lit surface text", true);
        eprintln!("4K, 2000 solid glyph quads: unlit {plain:.3} ms, lit {lit:.3} ms");
        assert!(lit < 0.2, "2000 lit glyphs at 4K took {lit:.3} ms");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn coloured_text_icon_and_clip_on_gpu() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(320, 160, COLOR_FORMAT).unwrap();
        let pass = TextPass::new(&gpu.device);
        let mut engine = TextEngine::new(FONT).unwrap();
        let paragraph = engine
            .layout(
                "Paper",
                Style {
                    family: "EB Garamond",
                    size: 48.0,
                    line_height: 58.0,
                    wrap_width: None,
                    pixels_per_unit: 1.0,
                    representation: Representation::Msdf,
                },
                &Baseline::Straight {
                    origin: [30.0, 90.0],
                    direction: [1.0, 0.0],
                },
            )
            .unwrap();
        let quads = paragraph
            .glyphs
            .iter()
            .map(|glyph| RichQuad {
                rect: [
                    glyph.position[0] + glyph.local_rect[0] * glyph.scale,
                    glyph.position[1] + glyph.local_rect[1] * glyph.scale,
                    glyph.local_rect[2] * glyph.scale,
                    glyph.local_rect[3] * glyph.scale,
                ],
                uv: glyph.uv,
                origin: glyph.position,
                alpha: 1.0,
                clip: [0.0, 0.0, 70.0, 160.0],
                turn: [0.0; 3],
                color: [1.0, 0.0, 0.0, 1.0],
            })
            .collect::<Vec<_>>();
        let text_atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
        let mut cache = Cache::new();
        let icon_bytes = (0..16)
            .flat_map(|_| {
                [0u16, 0, 65535, 65535]
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
            })
            .collect::<Vec<_>>();
        let icon = cache
            .load_rgba16(&icon_bytes, 4, 4, ColorSpace::Linear)
            .unwrap();
        let icon_atlas = IconAtlas::from_cache(&cache, &[icon]).unwrap();
        let icon_gpu = GpuAtlas::icons(&gpu.device, &gpu.queue, &icon_atlas);
        let icon_quad = RichQuad {
            rect: [140.0, 40.0, 40.0, 40.0],
            uv: icon_atlas.uv(icon).unwrap(),
            origin: [140.0, 40.0],
            alpha: 1.0,
            clip: [0.0, 0.0, 320.0, 160.0],
            turn: [0.0; 3],
            color: [1.0; 4],
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rich text gpu test"),
            });
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rich text clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        pass.encode_quads(
            &gpu.device,
            &mut encoder,
            QuadDraw {
                target: &target.view,
                depth: None,
                atlas: &text_atlas,
                quads: &quads,
                space: TextSpace::Overlay {
                    width: 320,
                    height: 160,
                },
                deformers: None,
                icons: false,
            },
        )
        .unwrap();
        pass.encode_quads(
            &gpu.device,
            &mut encoder,
            QuadDraw {
                target: &target.view,
                depth: None,
                atlas: &icon_gpu,
                quads: std::slice::from_ref(&icon_quad),
                space: TextSpace::Overlay {
                    width: 320,
                    height: 160,
                },
                deformers: None,
                icons: true,
            },
        )
        .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        let pixels = gpu.readback_rgba16(&target).unwrap();
        let channel = |x: usize, y: usize, component: usize| {
            half::f16::from_bits(pixels[(y * 320 + x) * 4 + component]).to_f32()
        };
        assert!((40..70).any(|x| (50..100).any(|y| channel(x, y, 0) > 0.5)));
        assert!((70..130).all(|x| (50..100).all(|y| channel(x, y, 0) < 0.01)));
        assert!(channel(160, 60, 2) > 0.9);
        assert!(channel(160, 60, 0) < 0.01);
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/text-live-gpu.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut png = png::Encoder::new(file, 320, 160);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        let mut writer = png.write_header().unwrap();
        let bytes = pixels
            .iter()
            .map(|value| (half::f16::from_bits(*value).to_f32().clamp(0.0, 1.0) * 255.0) as u8)
            .collect::<Vec<_>>();
        writer.write_image_data(&bytes).unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn two_thousand_glyphs_and_fifty_icons_at_4k() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(3840, 2160, COLOR_FORMAT).unwrap();
        let pass = TextPass::new(&gpu.device);
        let paragraph = paragraph(48.0);
        let glyph = &paragraph.glyphs[0];
        let quads = (0..2000)
            .map(|index| RichQuad {
                rect: [
                    8.0 + (index % 100) as f32 * 38.0,
                    8.0 + (index / 100) as f32 * 92.0,
                    24.0,
                    48.0,
                ],
                uv: glyph.uv,
                origin: [0.0; 2],
                alpha: 1.0,
                clip: [0.0, 0.0, 3840.0, 2160.0],
                turn: [0.0; 3],
                color: [1.0; 4],
            })
            .collect::<Vec<_>>();
        let text_atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
        let mut cache = Cache::new();
        let icon_bytes = (0..16)
            .flat_map(|_| [65535u16; 4].into_iter().flat_map(u16::to_le_bytes))
            .collect::<Vec<_>>();
        let icon = cache
            .load_rgba16(&icon_bytes, 4, 4, ColorSpace::Linear)
            .unwrap();
        let icon_atlas = IconAtlas::from_cache(&cache, &[icon]).unwrap();
        let icon_gpu = GpuAtlas::icons(&gpu.device, &gpu.queue, &icon_atlas);
        let icon_quads = (0..50)
            .map(|index| RichQuad {
                rect: [8.0 + index as f32 * 72.0, 1950.0, 40.0, 40.0],
                uv: icon_atlas.uv(icon).unwrap(),
                origin: [0.0; 2],
                alpha: 1.0,
                clip: [0.0, 0.0, 3840.0, 2160.0],
                turn: [0.0; 3],
                color: [1.0; 4],
            })
            .collect::<Vec<_>>();
        let mut profiler = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut samples = Vec::new();
        let mut turns = pfx_gpu::pace::Turns::default();
        for _ in 0..12 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            pass.encode_quads_timed(
                &gpu.device,
                &mut encoder,
                QuadDraw {
                    target: &target.view,
                    depth: None,
                    atlas: &text_atlas,
                    quads: &quads,
                    space: TextSpace::Overlay {
                        width: 3840,
                        height: 2160,
                    },
                    deformers: None,
                    icons: false,
                },
                Some(&mut profiler),
            )
            .unwrap();
            pass.encode_quads_timed(
                &gpu.device,
                &mut encoder,
                QuadDraw {
                    target: &target.view,
                    depth: None,
                    atlas: &icon_gpu,
                    quads: &icon_quads,
                    space: TextSpace::Overlay {
                        width: 3840,
                        height: 2160,
                    },
                    deformers: None,
                    icons: true,
                },
                Some(&mut profiler),
            )
            .unwrap();
            let slot = profiler.finish(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            if let Some(slot) = slot {
                profiler.submitted(slot);
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let timings = profiler.collect(&gpu.device);
                let elapsed = timings
                    .into_iter()
                    .flatten()
                    .map(|timing| timing.milliseconds)
                    .sum::<f64>();
                turns.add(elapsed);
                samples.push(elapsed);
            }
        }
        assert!(samples.len() >= 10, "GPU timestamp query unavailable");
        let measured = &mut samples[2..];
        measured.sort_by(f64::total_cmp);
        let median = measured[measured.len() / 2];
        eprintln!("4K text and icon median: {median:.3} ms");
        assert!(median < 0.3, "4K text and icons took {median:.3} ms");
    }

    fn page_draw_ms(
        gpu: &pfx_gpu::Gpu,
        pass: &TextPass,
        target: &pfx_gpu::OffscreenTarget,
        depth: &wgpu::TextureView,
        atlas: &GpuAtlas,
        quads: &[RichQuad],
        space: TextSpace,
    ) -> f64 {
        let mut profiler = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        let mut samples = Vec::new();
        for _ in 0..12 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("page clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            let overlay = matches!(space, TextSpace::Overlay { .. });
            pass.encode_quads_timed(
                &gpu.device,
                &mut encoder,
                QuadDraw {
                    target: &target.view,
                    depth: (!overlay).then_some(depth),
                    atlas,
                    quads,
                    space,
                    deformers: None,
                    icons: false,
                },
                Some(&mut profiler),
            )
            .unwrap();
            let slot = profiler.finish(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            if let Some(slot) = slot {
                profiler.submitted(slot);
                let elapsed = profiler
                    .collect(&gpu.device)
                    .into_iter()
                    .flatten()
                    .map(|timing| timing.milliseconds)
                    .sum::<f64>();
                turns.add(elapsed);
                samples.push(elapsed);
            }
        }
        turns.turn();
        assert!(samples.len() >= 10, "GPU timestamp query unavailable");
        let measured = &mut samples[2..];
        measured.sort_by(f64::total_cmp);
        measured[measured.len() / 2]
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_page_of_msdf_text_at_4k_face_on_and_tilted() {
        let size = [3840u32, 2160];
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(size[0], size[1], COLOR_FORMAT).unwrap();
        let depth = gpu
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("page depth"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let line = "Sphinx of black quartz, judge my vow. The five boxing wizards jump quickly over a lazy dog by the old mill. ";
        let mut engine = TextEngine::new(SANS).unwrap();
        let spans = [pfx_text::Span {
            text: line.repeat(64),
            face: shared_face("DM Sans", 36.0, 400),
            color: [1.0; 4],
        }];
        let page = engine
            .render_spans(
                &spans,
                Some(3700.0),
                1.0,
                Anchor::Start,
                Representation::Msdf,
                [70.0, 40.0],
                1.0,
                [0.0, 0.0, 3840.0, 2160.0],
                [0.0; 3],
            )
            .unwrap();
        let quads = rich_quads(&page);
        let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &page.atlas).unwrap();
        let pass = TextPass::new(&gpu.device);
        let width = size[0] as f32;
        let height = size[1] as f32;
        let flat = [
            [2.0 / width, 0.0, 0.0, 0.0],
            [0.0, -2.0 / height, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.5, 1.0],
        ];
        let metre = 0.001;
        let model = [
            [metre, 0.0, 0.0, 0.0],
            [0.0, 0.0, metre, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [-0.5 * width * metre, 0.0, -0.5 * height * metre, 1.0],
        ];
        let tilted = |degrees: f32| {
            let (s, c) = degrees.to_radians().sin_cos();
            let view = look_at([0.0, 3.5 * s, 3.5 * c], [0.0; 3], [0.0, 1.0, 0.0]);
            let view_projection = crate::frame::multiply(perspective(width / height), view);
            crate::frame::multiply(view_projection, model)
        };
        println!("page: {} glyphs at 4K", quads.len());
        assert!(quads.len() > 5000, "{} glyphs", quads.len());
        let cases = [
            (
                "overlay",
                TextSpace::Overlay {
                    width: size[0],
                    height: size[1],
                },
            ),
            (
                "surface, face on",
                TextSpace::Surface {
                    model_view_projection: flat,
                },
            ),
            (
                "surface, 45 degrees",
                TextSpace::Surface {
                    model_view_projection: tilted(45.0),
                },
            ),
            (
                "surface, 25 degrees",
                TextSpace::Surface {
                    model_view_projection: tilted(25.0),
                },
            ),
            (
                "surface, 10 degrees",
                TextSpace::Surface {
                    model_view_projection: tilted(10.0),
                },
            ),
        ];
        for (name, space) in cases {
            let ms = page_draw_ms(&gpu, &pass, &target, &depth, &atlas, &quads, space);
            println!("page of text, {name}: {ms:.3} ms");
            assert!(ms < 6.0, "{name}: {ms:.3} ms");
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn paper_edges_at_48_and_600_pixels() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(2200, 900, COLOR_FORMAT).unwrap();
        let pass = TextPass::new(&gpu.device);
        for size in [48.0, 600.0] {
            let paragraph = paragraph(size);
            let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("text test"),
                });
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("text clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            pass.encode(
                &gpu.device,
                &mut encoder,
                TextDraw {
                    target: &target.view,
                    depth: None,
                    atlas: &atlas,
                    paragraph: &paragraph,
                    space: TextSpace::Overlay {
                        width: 2200,
                        height: 900,
                    },
                    color: [1.0; 4],
                    deformers: None,
                },
            )
            .unwrap();
            gpu.queue.submit(Some(encoder.finish()));
            let pixels = gpu.readback_rgba16(&target).unwrap();
            let mut edge_width = usize::MAX;
            for y in 0..900usize {
                let row: Vec<f32> = (0..2200usize)
                    .map(|x| half::f16::from_bits(pixels[(y * 2200 + x) * 4 + 3]).to_f32())
                    .collect();
                for x in 0..2195 {
                    for width in 1..=4 {
                        let a = row[x];
                        let b = row[x + width];
                        if (a < 0.1 && b > 0.9) || (a > 0.9 && b < 0.1) {
                            edge_width = edge_width.min(width);
                        }
                    }
                }
                if edge_width == 1 {
                    break;
                }
            }
            assert!(
                edge_width <= 3,
                "Paper edge width at {size} px: {edge_width}"
            );
            println!("Paper edge width at {size} px: {edge_width}");
        }
    }

    const SANS: &[u8] = pfx_text::fixture::DM_SANS;
    const MONO: &[u8] = pfx_text::fixture::IBM_PLEX_MONO;

    fn shared_face(family: &str, size: f32, weight: u16) -> pfx_text::Face {
        pfx_text::Face {
            family: family.into(),
            size,
            line: size * 1.25,
            weight,
            italic: false,
            spacing: 0.0,
        }
    }

    fn shared_span(text: &str, face: &pfx_text::Face) -> pfx_text::Span {
        pfx_text::Span {
            text: text.into(),
            face: face.clone(),
            color: [1.0, 0.9, 0.8, 1.0],
        }
    }

    fn overlay(
        gpu: &pfx_gpu::Gpu,
        pass: &TextPass,
        target: &pfx_gpu::OffscreenTarget,
        draws: &[(&GpuAtlas, &[RichQuad])],
        profiler: Option<&mut pfx_gpu::GpuProfiler>,
    ) -> Option<f64> {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shared text clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        let mut profiler = profiler;
        for (atlas, quads) in draws {
            pass.encode_quads_timed(
                &gpu.device,
                &mut encoder,
                QuadDraw {
                    target: &target.view,
                    depth: None,
                    atlas,
                    quads,
                    space: TextSpace::Overlay {
                        width: target.width,
                        height: target.height,
                    },
                    deformers: None,
                    icons: false,
                },
                profiler.as_deref_mut(),
            )
            .unwrap();
        }
        let slot = profiler.as_deref_mut().and_then(|p| p.finish(&mut encoder));
        gpu.queue.submit(Some(encoder.finish()));
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let profiler = profiler?;
        profiler.submitted(slot?);
        Some(
            profiler
                .collect(&gpu.device)
                .into_iter()
                .flatten()
                .map(|timing| timing.milliseconds)
                .sum(),
        )
    }

    fn shared_frame(
        gpu: &pfx_gpu::Gpu,
        pass: &TextPass,
        target: &pfx_gpu::OffscreenTarget,
        pages: &GlyphPages,
        quads: &[pfx_text::PageQuad],
    ) -> Vec<f32> {
        let groups = page_quads(quads);
        let draws = groups
            .iter()
            .map(|(page, quads)| (pages.page(*page).unwrap(), quads.as_slice()))
            .collect::<Vec<_>>();
        overlay(gpu, pass, target, &draws, None);
        gpu.readback_rgba16(target)
            .unwrap()
            .into_iter()
            .map(|value| half::f16::from_bits(value).to_f32())
            .collect()
    }

    #[test]
    fn page_quads_group_by_page_in_first_seen_order() {
        let quad = |page: u32, x: f32| pfx_text::PageQuad {
            page,
            quad: pfx_text::RichGlyphQuad {
                key: 0,
                line_index: 0,
                line_baseline: 0.0,
                pen: [x, 0.0],
                atlas_page: pfx_text::AtlasPage::Monochrome,
                is_color: false,
                rect: [x, 0.0, 1.0, 1.0],
                uv: [0.0; 4],
                origin: [0.0; 2],
                alpha: 1.0,
                clip: [0.0; 4],
                turn: [0.0; 3],
                color: [1.0; 4],
            },
        };
        let groups = page_quads(&[quad(3, 0.0), quad(1, 1.0), quad(3, 2.0)]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, 3);
        assert_eq!(
            groups[0].1.iter().map(|q| q.rect[0]).collect::<Vec<_>>(),
            vec![0.0, 2.0]
        );
        assert_eq!(groups[1].0, 1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn shared_pages_draw_existing_text_within_one_level() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(480, 200, COLOR_FORMAT).unwrap();
        let pass = TextPass::new(&gpu.device);
        let mut legacy = TextEngine::new(FONT).unwrap();
        let mut shared = TextEngine::new(FONT).unwrap();
        let mut pages = GlyphPages::new();
        let spans = [
            shared_span("Ledger 1,284 ffi", &shared_face("EB Garamond", 22.0, 500)),
            shared_span(" été 4.2e9", &shared_face("EB Garamond", 30.0, 700)),
        ];
        for (scale, origin, turn) in [
            (1.0, [12.0, 20.0], [0.0; 3]),
            (1.5, [8.25, 30.6], [0.0; 3]),
            (2.0, [3.6, 11.1], [80.0, 40.0, 0.2]),
        ] {
            let old = legacy
                .render_spans(
                    &spans,
                    Some(220.0),
                    scale,
                    Anchor::Start,
                    Representation::Coverage,
                    origin,
                    1.0,
                    [0.0, 0.0, 480.0, 200.0],
                    turn,
                )
                .unwrap();
            let atlas = GpuAtlas::new(&gpu.device, &gpu.queue, &old.atlas).unwrap();
            let quads = rich_quads(&old)
                .into_iter()
                .map(|quad| RichQuad {
                    rect: quad.rect.map(|v| v * scale),
                    origin: quad.origin.map(|v| v * scale),
                    clip: quad.clip.map(|v| v * scale),
                    turn: [quad.turn[0] * scale, quad.turn[1] * scale, quad.turn[2]],
                    ..quad
                })
                .collect::<Vec<_>>();
            overlay(&gpu, &pass, &target, &[(&atlas, &quads)], None);
            let before = gpu
                .readback_rgba16(&target)
                .unwrap()
                .into_iter()
                .map(|value| half::f16::from_bits(value).to_f32())
                .collect::<Vec<_>>();
            let placed = shared
                .place_spans(
                    &spans,
                    Some(220.0),
                    Anchor::Start,
                    &pfx_text::Placement {
                        scale,
                        representation: Representation::Coverage,
                        origin,
                        alpha: 1.0,
                        clip: [0.0, 0.0, 480.0, 200.0],
                        turn,
                    },
                )
                .unwrap();
            pages
                .sync(&gpu.device, &gpu.queue, &shared.take_atlas_changes())
                .unwrap();
            let quads = placed
                .quads
                .iter()
                .map(|quad| {
                    let mut quad = quad.clone();
                    quad.quad.rect = quad.quad.rect.map(|v| v * scale);
                    quad.quad.origin = quad.quad.origin.map(|v| v * scale);
                    quad.quad.clip = quad.quad.clip.map(|v| v * scale);
                    quad.quad.turn = [
                        quad.quad.turn[0] * scale,
                        quad.quad.turn[1] * scale,
                        quad.quad.turn[2],
                    ];
                    quad
                })
                .collect::<Vec<_>>();
            let after = shared_frame(&gpu, &pass, &target, &pages, &quads);
            let ink = before
                .iter()
                .skip(3)
                .step_by(4)
                .filter(|&&a| a > 0.5)
                .count();
            assert!(ink > 500, "{ink}");
            let worst = before
                .iter()
                .zip(&after)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(worst <= 1.0 / 255.0, "scale {scale}: {worst}");
            eprintln!("scale {scale}: worst difference {:.3}/255", worst * 255.0);
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn evicted_glyphs_upload_again_and_draw_the_same_frame() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(320, 120, COLOR_FORMAT).unwrap();
        let pass = TextPass::new(&gpu.device);
        let mut engine = TextEngine::new(MONO).unwrap();
        engine.set_atlas_page(256).unwrap();
        engine.set_atlas_budget(PageKind::Msdf.page_bytes(256) * 2);
        let mut pages = GlyphPages::new();
        let face = shared_face("IBM Plex Mono", 40.0, 400);
        let at = pfx_text::Placement {
            scale: 1.0,
            representation: Representation::Msdf,
            origin: [10.0, 10.0],
            alpha: 1.0,
            clip: [0.0, 0.0, 320.0, 120.0],
            turn: [0.0; 3],
        };
        let frame = |engine: &mut TextEngine, pages: &mut GlyphPages, text: &str| {
            engine.begin_frame();
            let placed = engine
                .place_spans(&[shared_span(text, &face)], None, Anchor::Start, &at)
                .unwrap();
            engine.end_frame();
            let regions = pages
                .sync(&gpu.device, &gpu.queue, &engine.take_atlas_changes())
                .unwrap();
            (
                shared_frame(&gpu, &pass, &target, pages, &placed.quads),
                regions,
            )
        };
        let (first, regions) = frame(&mut engine, &mut pages, "AB12");
        assert!(!regions.is_empty());
        assert!(regions.iter().all(|(_, region)| region.columns < 256));
        let (repeat, regions) = frame(&mut engine, &mut pages, "AB12");
        assert!(regions.is_empty());
        assert_eq!(first, repeat);
        for filler in ["CDEFGHIJ", "KLMNOPQR", "STUVWXYZ", "cdefghij", "klmnopqr"] {
            frame(&mut engine, &mut pages, filler);
        }
        assert!(engine.atlas_stats().evicted > 0);
        let (again, regions) = frame(&mut engine, &mut pages, "AB12");
        assert!(!regions.is_empty());
        let worst = first
            .iter()
            .zip(&again)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 1.0 / 255.0, "{worst}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn eighty_changing_numbers_at_4k_upload_nothing_once_warm() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let target = gpu.offscreen(3840, 2160, COLOR_FORMAT).unwrap();
        let pass = TextPass::new(&gpu.device);
        let mut engine = TextEngine::new(SANS).unwrap();
        let mut pages = GlyphPages::new();
        let faces = [
            shared_face("DM Sans", 19.0, 400),
            shared_face("DM Sans", 32.0, 600),
            shared_face("DM Sans", 60.0, 800),
            shared_face("DM Sans", 150.0, 800),
        ];
        let at = |origin| pfx_text::Placement {
            scale: 2.0,
            representation: Representation::Msdf,
            origin,
            alpha: 1.0,
            clip: [0.0, 0.0, 1920.0, 1080.0],
            turn: [0.0; 3],
        };
        engine.begin_frame();
        for face in &faces {
            engine
                .place_number(
                    &shared_span(NUMBERS, face),
                    NUMBERS,
                    Anchor::Start,
                    &at([0.0, 0.0]),
                )
                .unwrap();
        }
        engine.end_frame();
        let warm = pages
            .sync(&gpu.device, &gpu.queue, &engine.take_atlas_changes())
            .unwrap();
        assert!(!warm.is_empty());
        let mut profiler = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        let mut samples = Vec::new();
        let mut seed = 85u64;
        for frame in 0..120 {
            engine.begin_frame();
            let mut quads = Vec::new();
            for label in 0..80usize {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let value = seed >> 33;
                let text = match value % 4 {
                    0 => "NaN".to_string(),
                    1 => format!("{},{:03}", value % 10, (value >> 4) % 1000),
                    2 => format!("{}.{}e{}", value % 9 + 1, (value >> 3) % 10, value % 300),
                    _ => format!("-{}", value % 10_000),
                };
                let face = &faces[label % faces.len()];
                let origin = [
                    20.0 + (label / 20) as f32 * 470.0 + frame as f32 * 0.3,
                    40.0 + (label % 20) as f32 * 52.0,
                ];
                let placed = engine
                    .place_number(
                        &shared_span(&text, face),
                        NUMBERS,
                        Anchor::Start,
                        &at(origin),
                    )
                    .unwrap();
                quads.extend(placed.quads.into_iter().map(|mut quad| {
                    quad.quad.rect = quad.quad.rect.map(|v| v * 2.0);
                    quad.quad.clip = quad.quad.clip.map(|v| v * 2.0);
                    quad
                }));
            }
            engine.end_frame();
            let regions = pages
                .sync(&gpu.device, &gpu.queue, &engine.take_atlas_changes())
                .unwrap();
            assert!(regions.is_empty(), "frame {frame} uploaded {regions:?}");
            let groups = page_quads(&quads);
            let draws = groups
                .iter()
                .map(|(page, quads)| (pages.page(*page).unwrap(), quads.as_slice()))
                .collect::<Vec<_>>();
            if let Some(elapsed) = overlay(&gpu, &pass, &target, &draws, Some(&mut profiler)) {
                turns.add(elapsed);
                samples.push(elapsed);
            }
        }
        let stats = engine.atlas_stats();
        assert_eq!(stats.shaped, 4);
        assert!(stats.bytes <= stats.budget);
        assert!(samples.len() >= 100, "GPU timestamp query unavailable");
        samples.sort_by(f64::total_cmp);
        let median = samples[samples.len() / 2];
        eprintln!(
            "80 changing numbers at 4K: {median:.3} ms median, {} pages, {} cells, {} bytes",
            stats.pages, stats.cells, stats.bytes
        );
        assert!(median < 0.5, "{median:.3} ms");
    }
}
