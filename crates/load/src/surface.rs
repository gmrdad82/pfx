use std::sync::Arc;

use pfx_materials::{Material, UvTransform};
use serde_json::Value;

use crate::image::{ColorSpace, Image, Pixels};

pub const SHEEN: &str = "KHR_materials_sheen";
pub const ANISOTROPY: &str = "KHR_materials_anisotropy";
pub const DETAIL_NORMAL: &str = "detailNormalTexture";
pub const TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceMaps {
    pub base: Option<Arc<Image>>,
    pub normal: Option<Arc<Image>>,
    pub orm: Option<Arc<Image>>,
    pub detail: Option<Arc<Image>>,
    pub anisotropy: Option<Arc<Image>>,
}

impl SurfaceMaps {
    pub fn any(&self) -> bool {
        self.base.is_some()
            || self.normal.is_some()
            || self.orm.is_some()
            || self.detail.is_some()
            || self.anisotropy.is_some()
    }

    pub fn wants_tangents(&self) -> bool {
        self.normal.is_some() || self.detail.is_some() || self.anisotropy.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Surface {
    pub name: String,
    pub material: Material,
    pub maps: SurfaceMaps,
    pub declared: bool,
}

struct Images<'a, F> {
    gltf: &'a gltf::Gltf,
    buffers: &'a [Vec<u8>],
    read: &'a F,
    decoded: Vec<Option<Option<Image>>>,
}

impl<F> Images<'_, F>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    fn image(&mut self, texture: usize) -> Result<Option<Image>, String> {
        let Some(source) = self
            .gltf
            .textures()
            .nth(texture)
            .map(|texture| texture.source().index())
        else {
            return Ok(None);
        };
        if let Some(Some(found)) = self.decoded.get(source) {
            return Ok(found.clone());
        }
        let Some(image) = self.gltf.images().nth(source) else {
            return Ok(None);
        };
        let (bytes, mime, label) = match image.source() {
            gltf::image::Source::View { view, mime_type } => {
                let bytes = self
                    .buffers
                    .get(view.buffer().index())
                    .and_then(|buffer| buffer.get(view.offset()..view.offset() + view.length()))
                    .map(<[u8]>::to_vec);
                let label = match image.name() {
                    Some(name) if !name.is_empty() => format!("image {source} ({name})"),
                    _ => format!("image {source}"),
                };
                (bytes, Some(mime_type), label)
            }
            gltf::image::Source::Uri { uri, mime_type } => {
                ((self.read)(uri).ok(), mime_type, uri.to_string())
            }
        };
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let decoded = if bytes.starts_with(b"\x89PNG") {
            Image::png(&bytes).ok()
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || mime == Some("image/jpeg") {
            Some(Image::jpeg(&bytes).map_err(|e| format!("{label}: {e}"))?)
        } else {
            None
        };
        if let Some(slot) = self.decoded.get_mut(source) {
            *slot = Some(decoded.clone());
        }
        Ok(decoded)
    }

    fn map(
        &mut self,
        texture: Option<usize>,
        space: ColorSpace,
    ) -> Result<Option<Arc<Image>>, String> {
        let Some(texture) = texture else {
            return Ok(None);
        };
        Ok(self.image(texture)?.map(|mut image| {
            image.space = space;
            Arc::new(image)
        }))
    }
}

fn number(value: Option<&Value>, fallback: f32) -> f32 {
    value
        .and_then(Value::as_f64)
        .map_or(fallback, |value| value as f32)
}

fn texture_index(value: Option<&Value>) -> Option<usize> {
    value?.get("index")?.as_u64().map(|index| index as usize)
}

fn pair(value: Option<&Value>, fallback: [f32; 2]) -> [f32; 2] {
    let Some(list) = value.and_then(Value::as_array) else {
        return fallback;
    };
    match (
        list.first().and_then(Value::as_f64),
        list.get(1).and_then(Value::as_f64),
    ) {
        (Some(x), Some(y)) if list.len() == 2 => [x as f32, y as f32],
        _ => fallback,
    }
}

fn uv_transform(set: u32, transform: Option<&Value>) -> UvTransform {
    let Some(transform) = transform else {
        return UvTransform {
            set: set.min(1),
            ..UvTransform::default()
        };
    };
    let set = transform
        .get("texCoord")
        .and_then(Value::as_u64)
        .map_or(set, |value| value as u32);
    UvTransform {
        set: set.min(1),
        offset: pair(transform.get("offset"), [0.0; 2]),
        rotation: number(transform.get("rotation"), 0.0),
        scale: pair(transform.get("scale"), [1.0; 2]),
    }
}

fn transform_of(extensions: Option<&serde_json::Map<String, Value>>) -> Option<&Value> {
    extensions?.get(TEXTURE_TRANSFORM)
}

fn colour(value: Option<&Value>) -> Option<[f32; 3]> {
    let list = value?.as_array()?;
    if list.len() != 3 {
        return None;
    }
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(list) {
        *slot = item.as_f64()? as f32;
    }
    Some(out)
}

pub(crate) fn surfaces<F>(
    gltf: &gltf::Gltf,
    buffers: &[Vec<u8>],
    read: &F,
) -> Result<Vec<Surface>, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    let mut images = Images {
        gltf,
        buffers,
        read,
        decoded: vec![None; gltf.images().len()],
    };
    gltf.materials()
        .enumerate()
        .map(|(index, material)| {
            let mut found = surface(&material, &mut images)?;
            found.name = match material.name() {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => format!("material{index}"),
            };
            Ok(found)
        })
        .collect()
}

fn surface<F>(material: &gltf::Material<'_>, images: &mut Images<'_, F>) -> Result<Surface, String>
where
    F: Fn(&str) -> Result<Vec<u8>, String>,
{
    let pbr = material.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    let mut out = Material {
        base: [factor[0], factor[1], factor[2]],
        roughness: pbr.roughness_factor(),
        metalness: pbr.metallic_factor(),
        emission: material.emissive_factor(),
        ..Material::default()
    };
    let mut maps = SurfaceMaps::default();
    let mut main: Option<UvTransform> = None;
    let mut place = |set: u32, extensions: Option<&serde_json::Map<String, Value>>| {
        if set <= 1 {
            main.get_or_insert(uv_transform(set, transform_of(extensions)));
        }
        set <= 1
    };
    let base_texture = pbr
        .base_color_texture()
        .filter(|info| place(info.tex_coord(), info.extensions()));
    maps.base = images.map(
        base_texture.map(|info| info.texture().index()),
        ColorSpace::Srgb,
    )?;
    let normal = material
        .normal_texture()
        .filter(|info| place(info.tex_coord(), info.extensions()));
    if let Some(info) = &normal {
        out.textures.normal_scale = info.scale();
    }
    maps.normal = images.map(
        normal.map(|info| info.texture().index()),
        ColorSpace::Linear,
    )?;
    let rough = pbr
        .metallic_roughness_texture()
        .filter(|info| place(info.tex_coord(), info.extensions()))
        .map(|info| info.texture().index());
    let occlusion = material
        .occlusion_texture()
        .filter(|info| place(info.tex_coord(), info.extensions()));
    let occlusion_index = occlusion.as_ref().map(|info| info.texture().index());
    let rough_image = images.map(rough, ColorSpace::Linear)?;
    let packed = occlusion_index.is_some() && occlusion_index == rough;
    let occlusion_image = if packed {
        None
    } else {
        images.map(occlusion_index, ColorSpace::Linear)?
    };
    let shaded = (packed && rough_image.is_some()) || occlusion_image.is_some();
    out.textures.occlusion = match (&occlusion, shaded) {
        (Some(info), true) => info.strength(),
        _ => 0.0,
    };
    maps.orm = orm(rough_image.as_deref(), occlusion_image.as_deref()).map(Arc::new);
    let mut extended = false;
    if let Some(sheen) = material.extension_value(SHEEN) {
        extended = true;
        out.fabric.color = colour(sheen.get("sheenColorFactor")).unwrap_or([0.0; 3]);
        out.fabric.roughness = number(sheen.get("sheenRoughnessFactor"), 0.0);
    }
    if let Some(anisotropy) = material.extension_value(ANISOTROPY) {
        extended = true;
        out.anisotropy.strength = number(anisotropy.get("anisotropyStrength"), 0.0);
        out.anisotropy.rotation = number(anisotropy.get("anisotropyRotation"), 0.0);
        maps.anisotropy = images.map(
            texture_index(anisotropy.get("anisotropyTexture")),
            ColorSpace::Linear,
        )?;
    }
    if let Some(detail) = material
        .extras()
        .as_ref()
        .and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok())
        .and_then(|extras| extras.get(DETAIL_NORMAL).cloned())
    {
        maps.detail = images.map(texture_index(Some(&detail)), ColorSpace::Linear)?;
        out.textures.detail_scale = number(detail.get("scale"), 1.0);
        let set = detail
            .get("texCoord")
            .and_then(Value::as_u64)
            .map_or(1, |value| value as u32);
        out.textures.detail_uv = uv_transform(
            set,
            detail
                .get("extensions")
                .and_then(|extensions| extensions.get(TEXTURE_TRANSFORM)),
        );
    }
    out.textures.main = main.unwrap_or_default();
    let declared = maps.any()
        || extended
        || factor != [1.0; 4]
        || pbr.metallic_factor() != 1.0
        || pbr.roughness_factor() != 1.0
        || material.emissive_factor() != [0.0; 3];
    Ok(Surface {
        name: String::new(),
        material: out,
        maps,
        declared,
    })
}

pub fn rgba8(image: &Image) -> Vec<[u8; 4]> {
    match &image.pixels {
        Pixels::Eight(values) => values
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
        Pixels::Sixteen(values) => values
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|c| (p[c] >> 8) as u8))
            .collect(),
    }
}

pub fn resample(image: &Image, width: u32, height: u32) -> Vec<[u8; 4]> {
    let texels = rgba8(image);
    if image.width == width && image.height == height {
        return texels;
    }
    let (w, h) = (image.width as usize, image.height as usize);
    let at = |x: usize, y: usize| texels[y.min(h - 1) * w + x.min(w - 1)];
    let mut out = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        let fy = ((y as f32 + 0.5) * h as f32 / height as f32 - 0.5).max(0.0);
        let y0 = fy.floor() as usize;
        let ty = fy - y0 as f32;
        for x in 0..width {
            let fx = ((x as f32 + 0.5) * w as f32 / width as f32 - 0.5).max(0.0);
            let x0 = fx.floor() as usize;
            let tx = fx - x0 as f32;
            let (a, b, c, d) = (
                at(x0, y0),
                at(x0 + 1, y0),
                at(x0, y0 + 1),
                at(x0 + 1, y0 + 1),
            );
            out.push(std::array::from_fn(|k| {
                let top = a[k] as f32 + (b[k] as f32 - a[k] as f32) * tx;
                let bottom = c[k] as f32 + (d[k] as f32 - c[k] as f32) * tx;
                (top + (bottom - top) * ty).round().clamp(0.0, 255.0) as u8
            }));
        }
    }
    out
}

pub fn from_rgba8(width: u32, height: u32, space: ColorSpace, texels: &[[u8; 4]]) -> Image {
    Image {
        width,
        height,
        space,
        pixels: Pixels::Eight(texels.iter().flatten().copied().collect()),
    }
}

fn orm(rough: Option<&Image>, occlusion: Option<&Image>) -> Option<Image> {
    match (rough, occlusion) {
        (None, None) => None,
        (Some(rough), None) => Some(rough.clone()),
        (None, Some(occlusion)) => {
            let texels: Vec<[u8; 4]> = rgba8(occlusion)
                .into_iter()
                .map(|[r, _, _, _]| [r, 255, 255, 255])
                .collect();
            Some(from_rgba8(
                occlusion.width,
                occlusion.height,
                ColorSpace::Linear,
                &texels,
            ))
        }
        (Some(rough), Some(occlusion)) => {
            let shade = resample(occlusion, rough.width, rough.height);
            let texels: Vec<[u8; 4]> = rgba8(rough)
                .into_iter()
                .zip(shade)
                .map(|([_, g, b, _], [r, _, _, _])| [r, g, b, 255])
                .collect();
            Some(from_rgba8(
                rough.width,
                rough.height,
                ColorSpace::Linear,
                &texels,
            ))
        }
    }
}

pub fn tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let scale = |a: [f32; 3], s: f32| [a[0] * s, a[1] * s, a[2] * s];
    let add = |a: [f32; 3], b: [f32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let normalize = |a: [f32; 3]| {
        let length = dot(a, a).sqrt();
        if length > 0.0 {
            scale(a, 1.0 / length)
        } else {
            a
        }
    };
    let mut along_u = vec![[0.0; 3]; positions.len()];
    let mut along_v = vec![[0.0; 3]; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let edge1 = sub(positions[i1], positions[i0]);
        let edge2 = sub(positions[i2], positions[i0]);
        let (w0, w1, w2) = (uvs[i0], uvs[i1], uvs[i2]);
        let du1 = w1[0] - w0[0];
        let du2 = w2[0] - w0[0];
        let dv1 = w1[1] - w0[1];
        let dv2 = w2[1] - w0[1];
        let denom = du1 * dv2 - du2 * dv1;
        let factor = if denom.abs() < 1e-12 {
            0.0
        } else {
            1.0 / denom
        };
        let sdir = scale(sub(scale(edge1, dv2), scale(edge2, dv1)), factor);
        let tdir = scale(sub(scale(edge2, du1), scale(edge1, du2)), factor);
        for index in [i0, i1, i2] {
            along_u[index] = add(along_u[index], sdir);
            along_v[index] = add(along_v[index], tdir);
        }
    }
    along_u
        .iter()
        .zip(&along_v)
        .zip(normals)
        .map(|((sdir, tdir), normal)| {
            let normal = normalize(*normal);
            let mut tangent = sub(*sdir, scale(normal, dot(normal, *sdir)));
            if dot(tangent, tangent) <= 1e-16 {
                let axis = if normal[0].abs() < 0.9 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                tangent = sub(axis, scale(normal, dot(normal, axis)));
            }
            let tangent = normalize(tangent);
            let handed = if dot(cross(normal, tangent), *tdir) < 0.0 {
                -1.0
            } else {
                1.0
            };
            [tangent[0], tangent[1], tangent[2], handed]
        })
        .collect()
}
