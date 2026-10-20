use std::borrow::Cow;

use pfx_core::memory;
use std::sync::Arc;

use pfx_load::Image;
use pfx_load::scene::textures::Textures as SceneTextures;
use pfx_load::surface::rgba8;
use pfx_materials::PackedMaterial;

use crate::detail::{ContentImage, Detail, Textures};
use crate::gpu::GpuContent;
use crate::gpu::{Camera, Projection, Scene};
use crate::math::dot;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    pub colour: (usize, usize),
    pub linear: (usize, usize),
    pub orm: (usize, usize),
}

fn slot(index: f32, (first, count): (usize, usize)) -> f32 {
    if index < 0.0 || index as usize >= count {
        return -1.0;
    }
    (first + index as usize) as f32
}

impl Slots {
    pub fn remap(&self, packed: &mut PackedMaterial) {
        let [base, normal, orm, detail] = packed.textures;
        packed.textures = [
            slot(base, self.colour),
            slot(normal, self.linear),
            slot(orm, self.orm),
            slot(detail, self.linear),
        ];
        packed.texture_extra[0] = slot(packed.texture_extra[0], self.linear);
        packed.texture_extra[1] = slot(packed.texture_extra[1], self.linear);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shrink {
    pub content: Vec<u32>,
    pub colour: Vec<u32>,
    pub linear: Vec<u32>,
    pub orm: Vec<u32>,
}

pub(crate) fn at(list: &[u32], index: usize) -> u32 {
    list.get(index).copied().unwrap_or(0)
}

pub(crate) const SRGB_ROWS: usize = 64;

pub(crate) fn srgb_rows() -> Result<Vec<u8>, String> {
    let mut bytes = memory::vec(SRGB_ROWS * 16, "the tracer's content")?;
    bytes.extend(
        (0..SRGB_ROWS * 4)
            .flat_map(|byte| pfx_materials::linear_channel(byte as f32 / 255.0).to_le_bytes()),
    );
    Ok(bytes)
}

pub(crate) fn row(bytes: &[u8]) -> u32 {
    (bytes.len() / 16) as u32
}

pub(crate) fn put_row(bytes: &mut Vec<u8>, row: [u32; 4]) {
    for word in row {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
}

fn decoded(image: &ContentImage) -> Result<Vec<[f32; 4]>, String> {
    let mut out = memory::vec(image.texels.len(), "a texture being shrunk")?;
    out.extend((0..image.texels.len()).map(|at| image.linear(at)));
    Ok(out)
}

fn encoded(value: f32, srgb: bool) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let value = if srgb {
        pfx_materials::encode_channel(value)
    } else {
        value
    };
    (value * 255.0).round() as u8
}

pub fn shrunk(image: &ContentImage, times: u32) -> Result<Cow<'_, ContentImage>, String> {
    let mut times = times;
    while times > 0 && (image.width >> times == 0 || image.height >> times == 0) {
        times -= 1;
    }
    if times == 0 {
        return Ok(Cow::Borrowed(image));
    }
    let mut width = image.width as usize;
    let mut height = image.height as usize;
    let mut texels = decoded(image)?;
    for _ in 0..times {
        let half = [width.div_ceil(2), height.div_ceil(2)];
        let mut next = memory::filled(half[0] * half[1], [0.0f32; 4], "a texture being shrunk")?;
        for y in 0..half[1] {
            for x in 0..half[0] {
                let mut sum = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (2 * x + dx).min(width - 1);
                    let sy = (2 * y + dy).min(height - 1);
                    let texel = texels[sy * width + sx];
                    for c in 0..4 {
                        sum[c] += texel[c];
                    }
                }
                next[y * half[0] + x] = sum.map(|v| v * 0.25);
            }
        }
        texels = next;
        [width, height] = half;
    }
    let mut bytes = memory::vec(texels.len(), "a shrunk texture")?;
    bytes.extend(texels.iter().map(|&[r, g, b, a]| {
        [
            encoded(r, image.srgb),
            encoded(g, image.srgb),
            encoded(b, image.srgb),
            encoded(a, false),
        ]
    }));
    Ok(Cow::Owned(ContentImage {
        width: width as u32,
        height: height as u32,
        texels: bytes,
        srgb: image.srgb,
    }))
}

pub(crate) fn push(
    image: &ContentImage,
    times: u32,
    info: &mut Vec<GpuContent>,
    bytes: &mut Vec<u8>,
) -> Result<(), String> {
    let image = shrunk(image, times)?;
    info.push(GpuContent {
        offset: row(bytes),
        width: image.width,
        height: image.height,
        kind: if image.srgb { 3 } else { 2 },
    });
    memory::more(bytes, image.bytes() + 16, "the tracer's texture store")?;
    for words in image.words() {
        put_row(bytes, words);
    }
    Ok(())
}

pub(crate) fn place(
    textures: &Textures,
    shrink: &Shrink,
    info: &mut Vec<GpuContent>,
    bytes: &mut Vec<u8>,
) -> Result<Slots, String> {
    let mut list = |images: &[ContentImage], times: &[u32]| -> Result<(usize, usize), String> {
        let first = info.len();
        for (index, image) in images.iter().enumerate() {
            push(image, at(times, index), info, bytes)?;
        }
        Ok((first, images.len()))
    };
    Ok(Slots {
        colour: list(&textures.colour, &shrink.colour)?,
        linear: list(&textures.linear, &shrink.linear)?,
        orm: list(&textures.orm, &shrink.orm)?,
    })
}

pub(crate) fn wants_uvs1(packed: &PackedMaterial) -> bool {
    let sets = packed.texture_scales[3].round() as u32;
    let main = packed.textures[0] >= 0.0
        || packed.textures[1] >= 0.0
        || packed.textures[2] >= 0.0
        || packed.texture_extra[0] >= 0.0
        || packed.texture_extra[1] >= 0.0;
    let own = packed.map_turns[2].round() as u32;
    (main && sets & 1 != 0)
        || (packed.textures[3] >= 0.0 && sets & 2 != 0)
        || (packed.textures[1] >= 0.0 && own & 1 != 0)
        || (packed.textures[2] >= 0.0 && own & 2 != 0)
}

pub(crate) fn place_uvs1(
    triangles: impl Iterator<Item = [[f32; 2]; 3]>,
    bytes: &mut Vec<u8>,
) -> Result<u32, String> {
    let first = row(bytes) as usize + 1;
    let triangles = triangles.into_iter();
    memory::more(
        bytes,
        32 * triangles.size_hint().0,
        "the tracer's second uvs",
    )?;
    for [a, b, c] in triangles {
        put_row(bytes, [a[0], a[1], b[0], b[1]].map(f32::to_bits));
        put_row(bytes, [c[0], c[1], 0.0, 0.0].map(f32::to_bits));
    }
    u32::try_from(first).map_err(|_| "trace content is too large".to_string())
}

fn content(image: &Image, srgb: bool) -> ContentImage {
    ContentImage {
        width: image.width,
        height: image.height,
        texels: rgba8(image),
        srgb,
    }
}

pub fn from_scene(textures: &SceneTextures) -> Textures {
    let list = |images: &[Arc<Image>], srgb: bool| {
        images
            .iter()
            .map(|image| content(image, srgb))
            .collect::<Vec<_>>()
    };
    Textures {
        colour: list(&textures.colour, true),
        linear: list(&textures.linear, false),
        orm: list(&textures.orm, false),
    }
}

const UNSEEN_SIDE: u32 = 256;

struct View {
    origin: [f32; 3],
    axes: [[f32; 3]; 3],
    scale: [f32; 2],
    perspective: bool,
}

impl View {
    fn of(camera: &Camera, projection: Projection, size: [u32; 2]) -> Option<View> {
        let unit = |v: [f32; 3]| {
            let length = dot(v, v).sqrt();
            (length > 0.0).then(|| v.map(|x| x / length))
        };
        let forward = unit(camera.forward)?;
        let right = unit(camera.right)?;
        let up = unit(camera.up)?;
        let length = |v: [f32; 3]| dot(v, v).sqrt();
        match projection {
            Projection::Perspective => Some(View {
                origin: camera.origin,
                axes: [right, up, forward],
                scale: [
                    size[0] as f32 * 0.5 * length(camera.forward) / length(camera.right),
                    size[1] as f32 * 0.5 * length(camera.forward) / length(camera.up),
                ],
                perspective: true,
            }),
            Projection::Orthographic { width, height } => Some(View {
                origin: camera.origin,
                axes: [right, up, forward],
                scale: [size[0] as f32 / width, size[1] as f32 / height],
                perspective: false,
            }),
            Projection::Equirectangular => None,
        }
    }

    fn project(&self, point: [f32; 3]) -> ([f32; 2], f32) {
        let q = std::array::from_fn(|k| point[k] - self.origin[k]);
        let [x, y, z] = self.axes.map(|axis| dot(q, axis));
        if self.perspective {
            let depth = z.max(1e-12);
            ([x / depth * self.scale[0], y / depth * self.scale[1]], z)
        } else {
            ([x * self.scale[0], y * self.scale[1]], z)
        }
    }

    fn need(&self, vertices: [[f32; 3]; 3], texels: [[f32; 2]; 3]) -> Option<f32> {
        let projected = vertices.map(|vertex| self.project(vertex));
        let behind = projected.iter().filter(|(_, depth)| *depth <= 0.0).count();
        if behind == 3 {
            return None;
        }
        if behind > 0 {
            return Some(0.0);
        }
        let edge = |a: [f32; 2], b: [f32; 2]| [b[0] - a[0], b[1] - a[1]];
        let e = [
            edge(projected[0].0, projected[1].0),
            edge(projected[0].0, projected[2].0),
        ];
        let t = [edge(texels[0], texels[1]), edge(texels[0], texels[2])];
        let det = e[0][0] * e[1][1] - e[1][0] * e[0][1];
        if det.is_nan() || det.abs() <= 1e-12 {
            return None;
        }
        let inverse = [
            [e[1][1] / det, -e[0][1] / det],
            [-e[1][0] / det, e[0][0] / det],
        ];
        let column = |k: usize| -> [f32; 2] {
            std::array::from_fn(|row| t[0][row] * inverse[k][0] + t[1][row] * inverse[k][1])
        };
        let [c0, c1] = [column(0), column(1)];
        let a = c0[0] * c0[0] + c0[1] * c0[1];
        let b = c0[0] * c1[0] + c0[1] * c1[1];
        let c = c1[0] * c1[0] + c1[1] * c1[1];
        let least = ((a + c) - ((a - c) * (a - c) + 4.0 * b * b).sqrt()) * 0.5;
        let sigma = least.max(0.0).sqrt();
        let depths = projected.map(|(_, depth)| depth);
        let nearest = depths.iter().copied().fold(f32::INFINITY, f32::min);
        let mean = depths.iter().sum::<f32>() / 3.0;
        let sigma = if self.perspective {
            sigma * nearest / mean
        } else {
            sigma
        };
        sigma.is_finite().then_some(sigma)
    }
}

#[derive(Clone, Copy)]
enum Use {
    Seen(f32),
    Unseen,
}

fn level(least: Option<Use>, width: u32, height: u32) -> u32 {
    let longest = width.max(height).max(1);
    match least {
        Some(Use::Seen(sigma)) if sigma >= 2.0 => sigma.log2().floor() as u32,
        Some(Use::Seen(_)) => 0,
        Some(Use::Unseen) | None => {
            let mut times = 0;
            while longest >> times > UNSEEN_SIDE {
                times += 1;
            }
            times
        }
    }
}

fn least(current: &mut Option<Use>, sigma: Option<f32>) {
    *current = match (*current, sigma) {
        (Some(Use::Seen(a)), Some(b)) => Some(Use::Seen(a.min(b))),
        (_, Some(b)) => Some(Use::Seen(b)),
        (None, None) => Some(Use::Unseen),
        (other, None) => other,
    };
}

pub(crate) fn budget<E>(scene: &Scene<E>, detail: &Detail, width: u32, height: u32) -> Shrink {
    let Some(size) = detail.fit_textures else {
        return Shrink::default();
    };
    let size = [size[0].max(width), size[1].max(height)];
    let Some(view) = View::of(&scene.camera, detail.projection, size) else {
        return Shrink::default();
    };
    let materials = &scene.materials;
    let slotted = !detail.content_slots.is_empty() || !detail.text_slots.is_empty();
    let images: &[Option<ContentImage>] = if slotted {
        &detail.content_slots
    } else {
        &detail.content
    };
    let mut content: Vec<Option<Use>> = vec![None; images.len()];
    let lists = [
        &detail.textures.colour,
        &detail.textures.linear,
        &detail.textures.orm,
    ];
    let mut textures: [Vec<Option<Use>>; 3] = lists.map(|list| vec![None; list.len()]);
    let mut full: [Vec<bool>; 3] = lists.map(|list| vec![false; list.len()]);
    for material in materials {
        let maps = &material.textures;
        for (kind, index, uv) in [
            (0, maps.base, maps.main),
            (1, maps.normal, maps.normal_uv.unwrap_or(maps.main)),
            (2, maps.orm, maps.orm_uv.unwrap_or(maps.main)),
        ] {
            if uv.set != 0 && index >= 0 && (index as usize) < full[kind].len() {
                full[kind][index as usize] = true;
            }
        }
        for index in [maps.detail, maps.anisotropy, maps.thin_film] {
            if index >= 0 && (index as usize) < full[1].len() {
                full[1][index as usize] = true;
            }
        }
    }
    let mut visit = |vertices: [[f32; 3]; 3],
                     uvs: [[f32; 2]; 3],
                     material: u32,
                     layer: Option<(i32, [f32; 2])>| {
        let measure = |scale: [f32; 2], image: &ContentImage| {
            let texels = uvs.map(|uv| {
                [
                    uv[0] * scale[0] * image.width as f32,
                    uv[1] * scale[1] * image.height as f32,
                ]
            });
            view.need(vertices, texels)
        };
        if let Some((slot, scale)) = layer
            && slot >= 0
            && let Some(Some(image)) = images.get(slot as usize)
        {
            least(&mut content[slot as usize], measure(scale, image));
        }
        let Some(m) = materials.get(material as usize) else {
            return;
        };
        if m.content_layer.active() {
            let slot = m.content_layer.slot;
            if slot >= 0
                && let Some(Some(image)) = images.get(slot as usize)
            {
                let scale = layer.map_or([1.0; 2], |(_, scale)| scale);
                least(&mut content[slot as usize], measure(scale, image));
            }
        } else if !slotted && let Some(Some(image)) = images.get(material as usize) {
            let scale = layer.map_or([1.0; 2], |(_, scale)| scale);
            least(&mut content[material as usize], measure(scale, image));
        }
        let maps = &m.textures;
        for (kind, index, uv) in [
            (0usize, maps.base, maps.main),
            (1, maps.normal, maps.normal_uv.unwrap_or(maps.main)),
            (2, maps.orm, maps.orm_uv.unwrap_or(maps.main)),
        ] {
            if index < 0 {
                continue;
            }
            let Some(image) = lists[kind].get(index as usize) else {
                continue;
            };
            let sigma = measure(uv.scale, image);
            least(&mut textures[kind][index as usize], sigma);
        }
    };
    for tri in &detail.triangles {
        visit(tri.vertices, tri.uvs, tri.material, None);
    }
    for instance in &detail.instances {
        let Ok(pose) = crate::detail::Pose::new(instance.transform) else {
            continue;
        };
        let surface = instance.surface;
        let layer = Some((
            if surface.layer.active() {
                surface.layer.slot
            } else {
                -1
            },
            surface.uv_scale,
        ));
        let textured = |material: u32| {
            materials.get(material as usize).is_some_and(|m| {
                m.content_layer.active()
                    || (!slotted && images.get(material as usize).is_some_and(Option::is_some))
                    || m.textures.base >= 0
                    || m.textures.normal >= 0
                    || m.textures.orm >= 0
            })
        };
        if !surface.layer.active() && !instance.triangles.iter().any(|tri| textured(tri.material)) {
            continue;
        }
        for tri in instance.triangles.iter() {
            visit(
                tri.vertices.map(|point| pose.point(point)),
                tri.uvs,
                tri.material,
                layer,
            );
        }
    }
    let levels = |uses: &[Option<Use>], images: &[&ContentImage], full: &[bool]| -> Vec<u32> {
        uses.iter()
            .zip(images)
            .enumerate()
            .map(|(index, (found, image))| {
                if full.get(index).copied().unwrap_or(false) {
                    0
                } else {
                    level(*found, image.width, image.height)
                }
            })
            .collect()
    };
    let content_images: Vec<Option<&ContentImage>> = images.iter().map(Option::as_ref).collect();
    Shrink {
        content: content
            .iter()
            .zip(&content_images)
            .map(|(found, image)| image.map_or(0, |image| level(*found, image.width, image.height)))
            .collect(),
        colour: levels(&textures[0], &lists[0].iter().collect::<Vec<_>>(), &full[0]),
        linear: levels(&textures[1], &lists[1].iter().collect::<Vec<_>>(), &full[1]),
        orm: levels(&textures[2], &lists[2].iter().collect::<Vec<_>>(), &full[2]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detail::{Instance, InstanceSurface, TriangleDetail};
    use crate::gpu::Sun;
    use pfx_materials::{ContentLayer, Material};

    fn quad(half: f32) -> Vec<TriangleDetail> {
        let corner = |x: f32, y: f32| [x * half, y * half, 0.0];
        let uv = |x: f32, y: f32| [(x + 1.0) * 0.5, (1.0 - y) * 0.5];
        let triangle = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| TriangleDetail {
            vertices: [corner(a[0], a[1]), corner(b[0], b[1]), corner(c[0], c[1])],
            normals: [[0.0, 0.0, 1.0]; 3],
            uvs: [uv(a[0], a[1]), uv(b[0], b[1]), uv(c[0], c[1])],
            uvs1: [[0.0; 2]; 3],
            material: 0,
        };
        vec![
            triangle([-1.0, -1.0], [1.0, -1.0], [1.0, 1.0]),
            triangle([-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]),
        ]
    }

    fn image(side: u32) -> ContentImage {
        ContentImage {
            width: side,
            height: side,
            texels: vec![[128; 4]; (side * side) as usize],
            srgb: true,
        }
    }

    fn levels(half: f32, at: [f32; 3], fit: Option<[u32; 2]>) -> Shrink {
        let scene = Scene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![Material::default()],
            sky: pfx_load::Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            camera: Camera {
                origin: [0.0, 0.0, 5.0],
                forward: [0.0, 0.0, -1.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        };
        let mut transform = [[0.0; 4]; 4];
        for (k, column) in transform.iter_mut().enumerate() {
            column[k] = 1.0;
        }
        transform[3] = [at[0], at[1], at[2], 1.0];
        let detail = Detail {
            instances: vec![Instance {
                triangles: quad(half).into(),
                transform,
                surface: InstanceSurface {
                    layer: ContentLayer {
                        slot: 0,
                        ..ContentLayer::default()
                    },
                    ..InstanceSurface::default()
                },
                shadow_only: false,
                two_sided: true,
                motion: None,
            }],
            content_slots: vec![Some(image(1024))],
            projection: Projection::Orthographic {
                width: 2.0,
                height: 2.0,
            },
            fit_textures: fit,
            ..Detail::default()
        };
        budget(&scene, &detail, 200, 200)
    }

    #[test]
    fn a_map_keeps_the_level_its_pixels_need() {
        assert_eq!(levels(0.5, [0.0; 3], Some([200, 200])).content, [3]);
        assert_eq!(levels(0.5, [0.0; 3], Some([800, 800])).content, [1]);
        assert_eq!(levels(1.0, [0.0; 3], Some([1600, 1600])).content, [0]);
        assert_eq!(levels(0.5, [0.0, 0.0, 9.0], Some([200, 200])).content, [2]);
        assert_eq!(levels(0.5, [0.0; 3], None), Shrink::default());
    }

    #[test]
    fn a_shrunk_map_averages_in_linear_light() {
        let image = ContentImage {
            width: 2,
            height: 2,
            texels: vec![
                [0, 0, 0, 255],
                [255, 255, 255, 255],
                [0, 0, 0, 0],
                [255, 255, 255, 0],
            ],
            srgb: true,
        };
        let half = shrunk(&image, 1).unwrap();
        assert_eq!((half.width, half.height), (1, 1));
        let grey = (pfx_materials::encode_channel(0.5) * 255.0).round() as u8;
        assert_eq!(half.texels, [[grey, grey, grey, 128]]);
        assert!(matches!(shrunk(&image, 0), Ok(Cow::Borrowed(_))));
    }

    #[test]
    fn a_normal_or_orm_map_on_the_second_set_asks_for_the_second_uvs() {
        use pfx_materials::UvTransform;
        let mut material = Material::default();
        material.textures.base = 0;
        material.textures.normal = 1;
        material.textures.orm = 2;
        assert!(!wants_uvs1(&PackedMaterial::pack(&material)));
        let second = UvTransform {
            set: 1,
            ..UvTransform::default()
        };
        for slot in 0..2 {
            let mut own = material;
            if slot == 0 {
                own.textures.normal_uv = Some(second);
            } else {
                own.textures.orm_uv = Some(second);
            }
            assert!(wants_uvs1(&PackedMaterial::pack(&own)));
            let mut unused = own;
            unused.textures.normal = -1;
            unused.textures.orm = -1;
            assert!(!wants_uvs1(&PackedMaterial::pack(&unused)));
        }
    }
}
