pub const TEXTURED_WGSL: &str = r#"
var<private> textured_occlusion: f32 = 1.0;
var<private> textured_direction: vec3f = vec3f(1.0, 0.0, 1.0);
fn textured_on(material: MaterialData) -> bool {
    return any(material.textures >= vec4f(0.0)) || material.texture_extra.x >= 0.0 || material.texture_extra.y >= 0.0;
}
fn textured_normal(sample: vec4f, scale: f32) -> vec3f {
    let raw = sample.xyz * 2.0 - vec3f(1.0);
    return normalize(vec3f(raw.xy * scale, max(raw.z, 1e-4)));
}
fn textured_maps(mapped: MapResult, material: MaterialData, tangent: vec4f, uv0: vec2f, uv1: vec2f, dx0: vec2f, dy0: vec2f, dx1: vec2f, dy1: vec2f) -> MapResult {
    var result = mapped;
    let layers = vec4i(round(material.textures));
    let sets = u32(round(material.texture_scales.w));
    let main_one = (sets & 1u) != 0u;
    let detail_one = (sets & 2u) != 0u;
    let rotation = material.texture_extra.z;
    let uv = uv_place(select(uv0, uv1, main_one), material.texture_transform, rotation);
    let dx = uv_turn(select(dx0, dx1, main_one), rotation, material.texture_transform.zw);
    let dy = uv_turn(select(dy0, dy1, main_one), rotation, material.texture_transform.zw);
    if (layers.x >= 0) {
        result.base *= textureSampleGrad(map_base, map_sampler, uv, layers.x, dx, dy).rgb;
    }
    let own = u32(round(material.map_turns.z));
    if (layers.z >= 0) {
        let orm_one = (own & 2u) != 0u;
        let orm_turn = material.map_turns.y;
        let orm_at = uv_place(select(uv0, uv1, orm_one), material.orm_transform, orm_turn);
        let orm_dx = uv_turn(select(dx0, dx1, orm_one), orm_turn, material.orm_transform.zw);
        let orm_dy = uv_turn(select(dy0, dy1, orm_one), orm_turn, material.orm_transform.zw);
        let orm = textureSampleGrad(map_roughness, map_sampler, orm_at, layers.z, orm_dx, orm_dy);
        result.roughness *= orm.g;
        result.metalness *= orm.b;
        textured_occlusion = 1.0 + material.texture_scales.y * (orm.r - 1.0);
    }
    var local = vec3f(0.0, 0.0, 1.0);
    if (layers.y >= 0) {
        let normal_one = (own & 1u) != 0u;
        let normal_turn = material.map_turns.x;
        let normal_at = uv_place(select(uv0, uv1, normal_one), material.normal_transform, normal_turn);
        let normal_dx = uv_turn(select(dx0, dx1, normal_one), normal_turn, material.normal_transform.zw);
        let normal_dy = uv_turn(select(dy0, dy1, normal_one), normal_turn, material.normal_transform.zw);
        local = textured_normal(textureSampleGrad(map_normal, map_sampler, normal_at, layers.y, normal_dx, normal_dy), material.texture_scales.x);
    }
    if (layers.w >= 0) {
        let turn = material.texture_extra.w;
        let at = uv_place(select(uv0, uv1, detail_one), material.detail_transform, turn);
        let ddx = uv_turn(select(dx0, dx1, detail_one), turn, material.detail_transform.zw);
        let ddy = uv_turn(select(dy0, dy1, detail_one), turn, material.detail_transform.zw);
        let detail = textured_normal(textureSampleGrad(map_normal, map_sampler, at, layers.w, ddx, ddy), material.texture_scales.z);
        local = normalize(vec3f(local.xy + detail.xy, local.z * detail.z));
    }
    if (layers.y >= 0 || layers.w >= 0) {
        let n = result.normal;
        let t = normalize(tangent.xyz - n * dot(tangent.xyz, n));
        let b = cross(n, t) * tangent.w;
        result.normal = normalize(t * local.x + b * local.y + n * local.z);
    }
    let film = i32(round(material.texture_extra.y));
    if (film >= 0) {
        let sample = textureSampleGrad(map_normal, map_sampler, uv, film, dx, dy);
        result.thin_film = max(mix(material.map_turns.w, result.thin_film, sample.g), 0.0);
    }
    let anisotropy = i32(round(material.texture_extra.x));
    if (anisotropy >= 0) {
        let sample = textureSampleGrad(map_normal, map_sampler, uv, anisotropy, dx, dy);
        let along = sample.rg * 2.0 - vec2f(1.0);
        let length_along = length(along);
        textured_direction = vec3f(select(vec2f(1.0, 0.0), along / length_along, length_along > 1e-4), sample.b);
    }
    return result;
}
fn lobes_live(material: MaterialData, curvature: f32) {
    let rotation = material.lobes.y;
    let c = cos(rotation);
    let s = sin(rotation);
    let along = textured_direction.xy;
    let direction = vec2f(c * along.x - s * along.y, s * along.x + c * along.y);
    let wrap = clamp(material.lobes.w * curvature, 0.0, 1.0);
    lobes_set(
        material.sheen,
        vec4f(material.lobes.x * textured_direction.z, direction, 0.0),
        vec4f(material.scatter.xyz, material.lobes.z),
        vec4f(wrap, 0.0, 0.0, 0.0),
    );
}
"#;

use std::sync::Arc;

use pfx_load::Image;
use pfx_load::scene::textures::Textures;
use pfx_load::surface::{from_rgba8, resample};

use crate::maps::MapImages;
use crate::renderer::Renderer;

pub const LARGEST: u32 = 4096;

pub fn uniform(images: &[Arc<Image>]) -> Vec<Image> {
    let width = images
        .iter()
        .map(|image| image.width)
        .max()
        .unwrap_or(1)
        .min(LARGEST);
    let height = images
        .iter()
        .map(|image| image.height)
        .max()
        .unwrap_or(1)
        .min(LARGEST);
    images
        .iter()
        .map(|image| {
            if image.width == width && image.height == height {
                return (**image).clone();
            }
            from_rgba8(width, height, image.space, &resample(image, width, height))
        })
        .collect()
}

pub fn upload(renderer: &mut Renderer, textures: &Textures, had: bool) -> Result<bool, String> {
    if textures.is_empty() {
        if had {
            renderer.set_maps(&MapImages::default())?;
        }
        return Ok(false);
    }
    let base = uniform(&textures.colour);
    let normal = uniform(&textures.linear);
    let roughness = uniform(&textures.orm);
    renderer.set_maps(&MapImages {
        base: &base,
        normal: &normal,
        roughness: &roughness,
        metal: &[],
    })?;
    Ok(true)
}
