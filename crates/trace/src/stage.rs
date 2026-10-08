use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use pfx_core::anim::{Skinned, skin as skin_vertices};
use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_load::scene::{
    Camera as SceneCamera, Environment as SceneEnvironment, Geometry, Posed,
    Projection as SceneProjection, Scene as LoadedScene, surface as sprite_surface,
};
use pfx_materials::{ContentLayer, Material};

use crate::detail::{
    ContentFace, ContentImage, Detail, Instance, InstanceSurface, Lens, Transform, TriangleDetail,
};
use crate::gpu::{Camera, Projection, Scene, Sun, Trace, TraceError};
use crate::lights::LocalLight;
use crate::sky::Environment;
use crate::text::TextContent;

pub const ALPHA_CUTOFF: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct Mesh<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub tangents: &'a [[f32; 4]],
    pub uvs: &'a [[f32; 2]],
    pub alpha: Option<&'a [f32]>,
    pub indices: &'a [u32],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub texels: &'a [[u8; 4]],
    pub srgb: bool,
}

impl Image<'_> {
    pub fn same(&self, other: &Image<'_>) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.srgb == other.srgb
            && (std::ptr::eq(self.texels, other.texels) || self.texels == other.texels)
    }

    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && (self.width as usize)
                .checked_mul(self.height as usize)
                .is_some_and(|count| self.texels.len() == count)
    }

    pub fn linear(&self) -> ContentImage {
        let decode = |byte: u8| {
            let value = f32::from(byte) / 255.0;
            if !self.srgb {
                value
            } else {
                pfx_materials::linear_channel(value)
            }
        };
        ContentImage {
            width: self.width,
            height: self.height,
            texels: self
                .texels
                .iter()
                .map(|&[r, g, b, a]| [decode(r), decode(g), decode(b), f32::from(a) / 255.0])
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContentKind<'a> {
    Image(Image<'a>),
    Text(TextContent<'a>),
}

impl ContentKind<'_> {
    pub fn same(&self, other: &ContentKind<'_>) -> bool {
        match (self, other) {
            (Self::Image(a), ContentKind::Image(b)) => a.same(b),
            (Self::Text(a), ContentKind::Text(b)) => a.same(b),
            _ => false,
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Image(image) => image.valid(),
            Self::Text(text) => text.valid(),
        }
    }

    pub fn image(&self) -> Option<Image<'_>> {
        match self {
            Self::Image(image) => Some(*image),
            Self::Text(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementContent<'a> {
    pub kind: ContentKind<'a>,
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    pub crop: [f32; 4],
    pub face: ContentFace,
    pub cutout: bool,
}

impl<'a> PlacementContent<'a> {
    pub fn new(image: Image<'a>) -> Self {
        Self::of(ContentKind::Image(image))
    }

    pub fn text(text: TextContent<'a>) -> Self {
        Self::of(ContentKind::Text(text))
    }

    pub fn of(kind: ContentKind<'a>) -> Self {
        Self {
            kind,
            uv_offset: [0.0; 2],
            uv_scale: [1.0; 2],
            crop: [0.0, 0.0, 1.0, 1.0],
            face: ContentFace::Both,
            cutout: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement<'a> {
    pub mesh: u32,
    pub model: Transform,
    pub material: u32,
    pub casts_shadow: bool,
    pub content: Option<PlacementContent<'a>>,
    pub clip: [[f32; 4]; 2],
    pub shadow_only: bool,
    pub alpha_cutoff: f32,
    pub two_sided: bool,
}

impl Placement<'_> {
    pub fn new(mesh: u32, model: Transform, material: u32) -> Self {
        Self {
            mesh,
            model,
            material,
            casts_shadow: true,
            content: None,
            clip: [[0.0; 4]; 2],
            shadow_only: false,
            alpha_cutoff: ALPHA_CUTOFF,
            two_sided: true,
        }
    }
}

pub struct Stage<'a, E = Sky> {
    pub meshes: &'a [Mesh<'a>],
    pub instances: &'a [Placement<'a>],
    pub materials: &'a [Material],
    pub sky: E,
    pub sun: Sun,
    pub camera: Camera,
    pub projection: Projection,
    pub lens: Lens,
}

pub struct Staged<E = Sky> {
    pub scene: Scene<E>,
    pub detail: Detail,
}

impl<E> Stage<'_, E> {
    pub fn build(self) -> Result<Staged<E>, String> {
        if self.materials.is_empty() {
            return Err("staged scene needs at least one material".into());
        }
        if !self.projection.valid() || !self.lens.valid() {
            return Err("staged camera has an invalid projection or lens".into());
        }
        for mesh in self.meshes {
            check_mesh(mesh)?;
        }
        let first_slot = self
            .materials
            .iter()
            .filter(|material| material.content_layer.active())
            .map(|material| material.content_layer.slot as usize + 1)
            .max()
            .unwrap_or(0);
        let mut sources: Vec<ContentKind<'_>> = Vec::new();
        let mut cuts: Vec<(u32, u32, Vec<TriangleDetail>)> = Vec::new();
        let mut instances = Vec::with_capacity(self.instances.len());
        for placement in self.instances {
            if !(0.0..=1.0).contains(&placement.alpha_cutoff) {
                return Err("staged alpha cutoff must be a fraction from 0 to 1".into());
            }
            if !placement
                .clip
                .iter()
                .flatten()
                .all(|value| value.is_finite())
            {
                return Err("staged clip plane is not finite".into());
            }
            let mesh = self
                .meshes
                .get(placement.mesh as usize)
                .ok_or("staged instance names a missing mesh")?;
            if placement.material as usize >= self.materials.len() {
                return Err("staged instance names a missing material".into());
            }
            let mut surface = InstanceSurface {
                casts_shadow: placement.casts_shadow,
                clip: placement.clip,
                alpha_cutoff: placement.alpha_cutoff,
                ..InstanceSurface::default()
            };
            if let Some(content) = placement.content {
                if !content.kind.valid() {
                    return Err("staged content image has the wrong texel count".into());
                }
                let index = match sources.iter().position(|source| source.same(&content.kind)) {
                    Some(index) => index,
                    None => {
                        sources.push(content.kind);
                        sources.len() - 1
                    }
                };
                let material = &self.materials[placement.material as usize];
                surface.uv_offset = content.uv_offset;
                surface.uv_scale = content.uv_scale;
                surface.crop = content.crop;
                surface.face = content.face;
                surface.cutout = content.cutout;
                surface.layer = ContentLayer {
                    slot: (first_slot + index) as i32,
                    ..material.content_layer
                };
                if !surface.valid() {
                    return Err("staged content placement is not finite".into());
                }
            }
            let key = placement.alpha_cutoff.to_bits();
            let mut kept = if let Some((_, _, ready)) = cuts
                .iter()
                .find(|(index, bits, _)| *index == placement.mesh && *bits == key)
            {
                ready.clone()
            } else {
                let ready = triangles(mesh, placement.alpha_cutoff)?;
                cuts.push((placement.mesh, key, ready.clone()));
                ready
            };
            for triangle in &mut kept {
                triangle.material = placement.material;
            }
            instances.push(Instance {
                triangles: kept,
                transform: placement.model,
                surface,
                shadow_only: placement.shadow_only,
                two_sided: placement.two_sided,
            });
        }
        let content_slots = if sources.is_empty() {
            Vec::new()
        } else {
            std::iter::repeat_n(None, first_slot)
                .chain(
                    sources
                        .iter()
                        .map(|source| source.image().map(|image| image.linear())),
                )
                .collect()
        };
        let text_slots = if sources
            .iter()
            .any(|source| matches!(source, ContentKind::Text(_)))
        {
            let mut slots = vec![None; first_slot];
            for source in &sources {
                slots.push(match source {
                    ContentKind::Text(text) => Some(text.records()?),
                    ContentKind::Image(_) => None,
                });
            }
            slots
        } else {
            Vec::new()
        };
        Ok(Staged {
            scene: Scene {
                triangles: Vec::new(),
                shapes: Vec::new(),
                materials: self.materials.to_vec(),
                sky: self.sky,
                camera: self.camera,
                sun: self.sun,
            },
            detail: Detail {
                instances,
                lens: self.lens,
                projection: self.projection,
                content_slots,
                text_slots,
                ..Detail::default()
            },
        })
    }
}

impl<E: Clone + Into<Environment>> Staged<E> {
    pub fn trace(&self, gpu: &Gpu, width: u32, height: u32) -> Result<Trace, TraceError> {
        Trace::new_detailed(gpu, &self.scene, &self.detail, width, height)
    }
}

pub fn scene_camera(camera: &SceneCamera, aspect: f32) -> (Camera, Projection) {
    let (forward, right, up) = camera.axes();
    match camera.projection {
        SceneProjection::Perspective { fov } => {
            let tan = (fov.to_radians() * 0.5).tan();
            (
                Camera {
                    origin: camera.at,
                    forward,
                    right: right.map(|v| v * tan * aspect),
                    up: up.map(|v| v * tan),
                },
                Projection::Perspective,
            )
        }
        SceneProjection::Orthographic { height } => (
            Camera {
                origin: camera.at,
                forward,
                right,
                up,
            },
            Projection::Orthographic {
                width: height * aspect,
                height,
            },
        ),
    }
}

pub fn scene(scene: &LoadedScene, aspect: f32) -> Result<Staged<Environment>, String> {
    scene_at(scene, aspect, 0.0)
}

pub fn scene_lens(camera: &SceneCamera) -> Lens {
    match camera.depth_of_field {
        Some(depth) => Lens {
            shift: camera.shift,
            aperture: depth.aperture(),
            focus_distance: depth.distance,
        },
        None => Lens {
            shift: camera.shift,
            ..Lens::default()
        },
    }
}

pub fn scene_at(
    scene: &LoadedScene,
    aspect: f32,
    time: f32,
) -> Result<Staged<Environment>, String> {
    scene_posed(scene, aspect, &Posed::at(time))
}

pub fn scene_posed(
    scene: &LoadedScene,
    aspect: f32,
    posed: &Posed,
) -> Result<Staged<Environment>, String> {
    let draws = scene.draws();
    let viewpoint = scene.camera_or_default();
    let computed;
    let objects = if posed.models.is_empty() {
        computed = scene.posed(&viewpoint, posed.time);
        &computed
    } else {
        &posed.models
    };
    let models = draws.models_posed(objects, &posed.palettes);
    let mut baked: Vec<Option<Skinned>> = Vec::with_capacity(draws.items.len());
    for draw in &draws.items {
        let (Some(skin), Some(palette)) = (
            &draw.geometry.skin,
            draw.owner.and_then(|owner| posed.palettes.get(&owner)),
        ) else {
            baked.push(None);
            continue;
        };
        let geometry = &draw.geometry;
        let skinned = skin_vertices(
            &geometry.positions,
            &geometry.normals,
            &geometry.tangents,
            &skin.joints,
            &skin.weights,
            palette,
        )
        .map_err(|error| format!("object {}: {error}", draw.object))?;
        baked.push(Some(skinned));
    }
    let mut geometries: Vec<&Geometry> = Vec::new();
    let mut slots = BTreeMap::new();
    for draw in &draws.items {
        if let Entry::Vacant(slot) = slots.entry(draw.geometry.hash) {
            slot.insert(geometries.len() as u32);
            geometries.push(&draw.geometry);
        }
    }
    let mut letterings = Vec::new();
    for text in scene.texts.values() {
        let paragraph = text.paragraph()?;
        let glyphs = crate::text::glyphs(&paragraph);
        if glyphs.is_empty() {
            continue;
        }
        let lettering = crate::text::lettering(&paragraph.atlas, &glyphs)?;
        letterings.push((lettering, text.model(), text.lit));
    }
    let carriers: Vec<crate::text::Carrier> = letterings
        .iter()
        .map(|(lettering, _, _)| lettering.carrier())
        .collect();
    let mut meshes: Vec<Mesh<'_>> = geometries
        .iter()
        .map(|geometry| Mesh {
            positions: &geometry.positions,
            normals: &geometry.normals,
            tangents: &geometry.tangents,
            uvs: &geometry.uvs,
            alpha: None,
            indices: &geometry.indices,
        })
        .collect();
    let mut placements = Vec::with_capacity(draws.items.len() + letterings.len());
    for ((draw, model), skinned) in draws.items.iter().zip(&models).zip(&baked) {
        let mesh = match skinned {
            Some(skinned) => {
                let geometry = &draw.geometry;
                meshes.push(Mesh {
                    positions: &skinned.positions,
                    normals: &skinned.normals,
                    tangents: &skinned.tangents,
                    uvs: &geometry.uvs,
                    alpha: None,
                    indices: &geometry.indices,
                });
                (meshes.len() - 1) as u32
            }
            None => *slots
                .get(&draw.geometry.hash)
                .ok_or("a staged draw lost its mesh")?,
        };
        let material = &draws.materials[draw.material as usize];
        let content = match &draw.content {
            Some(name) if material.content_layer.active() => {
                let content = scene
                    .contents
                    .get(name)
                    .ok_or_else(|| format!("content {name} is missing"))?;
                let mut placed = PlacementContent::new(Image {
                    width: content.width,
                    height: content.height,
                    texels: bytemuck::cast_slice(&content.rgba),
                    srgb: content.srgb,
                });
                let frame = draw.owner.and_then(|owner| {
                    posed.frames.get(&owner).copied().or_else(|| {
                        scene
                            .animations
                            .get(&scene.objects.get(owner)?.name)?
                            .still_frame()
                    })
                });
                if let Some((offset, scale, crop)) = frame.map(sprite_surface) {
                    placed.uv_offset = offset;
                    placed.uv_scale = scale;
                    placed.crop = crop;
                }
                Some(placed)
            }
            _ => None,
        };
        placements.push(Placement {
            casts_shadow: draw.shadow.casts(),
            shadow_only: !draw.shadow.drawn(),
            two_sided: draw.two_sided,
            clip: draw.clip,
            content,
            alpha_cutoff: draw.alpha_cutoff,
            ..Placement::new(mesh, *model, draw.material)
        });
    }
    let mut materials = Cow::Borrowed(draws.materials.as_slice());
    let text_material = materials.len() as u32;
    if !letterings.is_empty() {
        materials
            .to_mut()
            .extend([crate::text::material(false), crate::text::material(true)]);
    }
    for ((lettering, model, lit), carrier) in letterings.iter().zip(&carriers) {
        let mesh = meshes.len() as u32;
        meshes.push(carrier.mesh());
        placements.push(Placement {
            casts_shadow: false,
            content: Some(lettering.content()),
            ..Placement::new(mesh, *model, text_material + u32::from(*lit))
        });
    }
    let sun = scene.sun_or_dark();
    let (camera, projection) = scene_camera(&viewpoint, aspect);
    let sky = match scene.environment() {
        SceneEnvironment::Analytic(sky) => Environment::Analytic(sky),
        SceneEnvironment::Hdr(sky) => Environment::Hdr(std::sync::Arc::unwrap_or_clone(sky)),
    };
    let mut staged = Stage {
        meshes: &meshes,
        instances: &placements,
        materials: &materials,
        sky,
        sun: Sun {
            direction: sun.direction,
            color: sun.color,
            intensity: sun.intensity,
        },
        camera,
        projection,
        lens: scene_lens(&viewpoint),
    }
    .build()?;
    staged.detail.sun_radius_deg = sun.radius;
    staged.detail.lights = scene
        .lights
        .values()
        .map(|light| LocalLight {
            radius: light.radius,
            shadow: light.shadow,
            ..LocalLight::point(light.position, light.color, light.intensity, light.range)
        })
        .collect();
    staged.detail.transmissive_shadows = scene.trace.transmissive_shadows;
    staged.detail.clamp_indirect = scene.trace.clamp_indirect;
    staged.detail.filter_glossy = scene.trace.filter_glossy;
    Ok(staged)
}

#[derive(Clone, Copy)]
struct Corner {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    alpha: f32,
}

fn lerp<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

fn check_mesh(mesh: &Mesh<'_>) -> Result<(), String> {
    let count = mesh.positions.len();
    if count == 0 || mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3) {
        return Err("mesh needs vertices and triangle indices".into());
    }
    if mesh.normals.len() != count
        || mesh.tangents.len() != count
        || mesh.uvs.len() != count
        || mesh.alpha.is_some_and(|alpha| alpha.len() != count)
    {
        return Err("mesh attributes have different vertex counts".into());
    }
    if mesh.indices.iter().any(|&index| index as usize >= count) {
        return Err("mesh index is outside the vertex buffer".into());
    }
    Ok(())
}

fn triangles(mesh: &Mesh<'_>, cutoff: f32) -> Result<Vec<TriangleDetail>, String> {
    check_mesh(mesh)?;
    let corner = |index: u32| {
        let index = index as usize;
        Corner {
            position: mesh.positions[index],
            normal: mesh.normals[index],
            uv: mesh.uvs[index],
            alpha: mesh.alpha.map_or(1.0, |alpha| alpha[index]),
        }
    };
    let mut out = Vec::with_capacity(mesh.indices.len() / 3);
    for face in mesh.indices.chunks_exact(3) {
        let corners = [corner(face[0]), corner(face[1]), corner(face[2])];
        let kept = if cutoff.to_bits() == ALPHA_CUTOFF.to_bits() {
            cut(corners)
        } else {
            cut_at(corners, cutoff)
        };
        for k in 1..kept.len().saturating_sub(1) {
            let [a, b, c] = [kept[0], kept[k], kept[k + 1]];
            out.push(TriangleDetail {
                vertices: [a.position, b.position, c.position],
                normals: [a.normal, b.normal, c.normal],
                uvs: [a.uv, b.uv, c.uv],
                material: 0,
            });
        }
    }
    Ok(out)
}

fn cut(corners: [Corner; 3]) -> Vec<Corner> {
    if corners.iter().all(|c| c.alpha >= ALPHA_CUTOFF) {
        return corners.to_vec();
    }
    let mut kept = Vec::with_capacity(4);
    for k in 0..3 {
        let a = corners[k];
        let b = corners[(k + 1) % 3];
        let inside_a = a.alpha >= ALPHA_CUTOFF;
        let inside_b = b.alpha >= ALPHA_CUTOFF;
        if inside_a {
            kept.push(a);
        }
        if inside_a != inside_b {
            let t = (ALPHA_CUTOFF - a.alpha) / (b.alpha - a.alpha);
            kept.push(Corner {
                position: lerp(a.position, b.position, t),
                normal: lerp(a.normal, b.normal, t),
                uv: lerp(a.uv, b.uv, t),
                alpha: ALPHA_CUTOFF,
            });
        }
    }
    kept
}

fn cut_at(corners: [Corner; 3], cutoff: f32) -> Vec<Corner> {
    if corners.iter().all(|c| c.alpha >= cutoff) {
        return corners.to_vec();
    }
    let mut kept = Vec::with_capacity(4);
    for k in 0..3 {
        let a = corners[k];
        let b = corners[(k + 1) % 3];
        let inside_a = a.alpha >= cutoff;
        let inside_b = b.alpha >= cutoff;
        if inside_a {
            kept.push(a);
        }
        if inside_a != inside_b {
            let t = (cutoff - a.alpha) / (b.alpha - a.alpha);
            kept.push(Corner {
                position: lerp(a.position, b.position, t),
                normal: lerp(a.normal, b.normal, t),
                uv: lerp(a.uv, b.uv, t),
                alpha: cutoff,
            });
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: Transform = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    fn stage<'a>(
        meshes: &'a [Mesh<'a>],
        instances: &'a [Placement<'a>],
        materials: &'a [Material],
    ) -> Stage<'a> {
        Stage {
            meshes,
            instances,
            materials,
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [1.0; 3],
                intensity: 0.0,
            },
            camera: Camera {
                origin: [0.0; 3],
                forward: [0.0, 0.0, -1.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
            },
            projection: Projection::Perspective,
            lens: Lens::default(),
        }
    }

    const POSITIONS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    const NORMALS: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    const TANGENTS: [[f32; 4]; 3] = [[1.0, 0.0, 0.0, 1.0]; 3];
    const UVS: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    fn triangle(alpha: Option<&[f32]>) -> Mesh<'_> {
        Mesh {
            positions: &POSITIONS,
            normals: &NORMALS,
            tangents: &TANGENTS,
            uvs: &UVS,
            alpha,
            indices: &[0, 1, 2],
        }
    }

    fn area(triangle: &TriangleDetail) -> f32 {
        let [a, b, c] = triangle.vertices;
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        0.5 * (e1[0] * e2[1] - e1[1] * e2[0]).abs()
    }

    #[test]
    fn instances_share_a_mesh_with_their_own_material_and_transform() {
        let meshes = [triangle(None)];
        let mut moved = IDENTITY;
        moved[3] = [2.0, 0.0, -1.0, 1.0];
        let instances = [Placement::new(0, IDENTITY, 1), Placement::new(0, moved, 0)];
        let materials = [Material::default(), Material::default()];
        let staged = stage(&meshes, &instances, &materials).build().unwrap();
        assert!(staged.scene.triangles.is_empty());
        assert_eq!(staged.scene.materials.len(), 2);
        let placed = &staged.detail.instances;
        assert_eq!(placed.len(), 2);
        assert_eq!(placed[0].triangles[0].material, 1);
        assert_eq!(placed[1].triangles[0].material, 0);
        assert_eq!(placed[0].triangles[0].uvs, UVS);
        assert_eq!(
            placed[1].transformed().unwrap()[0].vertices[1],
            [3.0, 0.0, -1.0]
        );
    }

    #[test]
    fn alpha_below_the_cutoff_is_cut_where_the_live_renderer_discards() {
        let materials = [Material::default()];
        let instances = [Placement::new(0, IDENTITY, 0)];
        let cases: [(&[f32], usize, f32); 4] = [
            (&[1.0, 1.0, 1.0], 1, 0.5),
            (&[0.0, 0.0, 0.0], 0, 0.0),
            (&[1.0, 0.0, 0.0], 1, 0.125),
            (&[1.0, 1.0, 0.0], 2, 0.375),
        ];
        for (alpha, count, expected) in cases {
            let meshes = [triangle(Some(alpha))];
            let staged = stage(&meshes, &instances, &materials).build().unwrap();
            let kept = &staged.detail.instances[0].triangles;
            assert_eq!(kept.len(), count, "{alpha:?}");
            let total: f32 = kept.iter().map(area).sum();
            assert!((total - expected).abs() < 1e-6, "{alpha:?}: {total}");
            for triangle in kept {
                for (vertex, uv) in triangle.vertices.iter().zip(triangle.uvs) {
                    assert_eq!([vertex[0], vertex[1]], uv);
                }
            }
        }
    }

    #[test]
    fn broken_input_is_refused() {
        let materials = [Material::default()];
        let good = [Placement::new(0, IDENTITY, 0)];
        let short = Mesh {
            uvs: &UVS[..2],
            ..triangle(None)
        };
        let outside = Mesh {
            indices: &[0, 1, 3],
            ..triangle(None)
        };
        for mesh in [short, outside] {
            assert!(stage(&[mesh], &good, &materials).build().is_err());
        }
        let meshes = [triangle(None)];
        let missing_mesh = [Placement { mesh: 1, ..good[0] }];
        let missing_material = [Placement {
            material: 1,
            ..good[0]
        }];
        assert!(stage(&meshes, &missing_mesh, &materials).build().is_err());
        assert!(
            stage(&meshes, &missing_material, &materials)
                .build()
                .is_err()
        );
        assert!(stage(&meshes, &good, &[]).build().is_err());
        let mut flat = stage(&meshes, &good, &materials);
        flat.projection = Projection::Orthographic {
            width: 0.0,
            height: 1.0,
        };
        assert!(flat.build().is_err());
    }

    #[test]
    fn placements_carry_their_shadow_flag_and_share_content_slots_by_image() {
        let meshes = [triangle(None)];
        let red = [[255, 0, 0, 255]; 4];
        let red_copy = red;
        let grey = [[188, 188, 188, 128]; 4];
        fn image(texels: &[[u8; 4]], srgb: bool) -> Image<'_> {
            Image {
                width: 2,
                height: 2,
                texels,
                srgb,
            }
        }
        let shown = PlacementContent {
            uv_offset: [0.5, 0.0],
            uv_scale: [0.5, 1.0],
            crop: [0.5, 0.0, 1.0, 1.0],
            face: ContentFace::Front,
            cutout: true,
            ..PlacementContent::new(image(&red, true))
        };
        let instances = [
            Placement {
                casts_shadow: false,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                content: Some(shown),
                ..Placement::new(0, IDENTITY, 1)
            },
            Placement {
                content: Some(PlacementContent::new(image(&red_copy, true))),
                ..Placement::new(0, IDENTITY, 1)
            },
            Placement {
                content: Some(PlacementContent::new(image(&grey, false))),
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                content: Some(PlacementContent::new(image(&grey, true))),
                ..Placement::new(0, IDENTITY, 0)
            },
        ];
        let screen = Material {
            content_layer: ContentLayer::from_kind(
                pfx_materials::Content::Screen,
                1.0,
                1,
                &pfx_materials::ContentLook {
                    screen_gain: 1.5,
                    ..Default::default()
                },
            ),
            ..Material::default()
        };
        let materials = [Material::default(), screen];
        let staged = stage(&meshes, &instances, &materials).build().unwrap();
        let surfaces: Vec<InstanceSurface> = staged
            .detail
            .instances
            .iter()
            .map(|instance| instance.surface)
            .collect();
        assert!(!surfaces[0].casts_shadow);
        assert!(!surfaces[0].layer.active());
        assert!(surfaces[1..].iter().all(|surface| surface.casts_shadow));
        assert_eq!(
            surfaces.iter().map(|s| s.layer.slot).collect::<Vec<_>>(),
            [-1, 2, 2, 3, 4]
        );
        assert_eq!(surfaces[1].layer.blend, screen.content_layer.blend);
        assert_eq!(surfaces[1].layer.strength, screen.content_layer.strength);
        assert_eq!(
            surfaces[3].layer,
            ContentLayer {
                slot: 3,
                ..ContentLayer::default()
            }
        );
        assert_eq!(surfaces[1].uv_offset, [0.5, 0.0]);
        assert_eq!(surfaces[1].crop, [0.5, 0.0, 1.0, 1.0]);
        assert_eq!(surfaces[1].face, ContentFace::Front);
        assert!(surfaces[1].cutout && !surfaces[2].cutout);
        let slots = &staged.detail.content_slots;
        assert_eq!(slots.len(), 5);
        assert!(slots[..2].iter().all(Option::is_none));
        assert_eq!(slots[2].as_ref().unwrap().texels[0], [1.0, 0.0, 0.0, 1.0]);
        let linear = slots[3].as_ref().unwrap().texels[0];
        assert!((linear[0] - 188.0 / 255.0).abs() < 1e-6);
        assert!((linear[3] - 128.0 / 255.0).abs() < 1e-6);
        let decoded = slots[4].as_ref().unwrap().texels[0];
        assert!((decoded[0] - 0.5029).abs() < 1e-3, "{decoded:?}");
        assert_eq!(decoded[3], linear[3]);
    }

    #[test]
    fn placements_without_content_leave_the_slots_empty() {
        let meshes = [triangle(None)];
        let instances = [Placement::new(0, IDENTITY, 0)];
        let materials = [Material::default()];
        let staged = stage(&meshes, &instances, &materials).build().unwrap();
        assert!(staged.detail.content_slots.is_empty());
        assert_eq!(
            staged.detail.instances[0].surface,
            InstanceSurface::default()
        );
    }

    #[test]
    fn broken_content_is_refused() {
        let meshes = [triangle(None)];
        let materials = [Material::default()];
        let texels = [[0_u8; 4]; 3];
        let short = Image {
            width: 2,
            height: 2,
            texels: &texels,
            srgb: true,
        };
        let instances = [Placement {
            content: Some(PlacementContent::new(short)),
            ..Placement::new(0, IDENTITY, 0)
        }];
        assert!(stage(&meshes, &instances, &materials).build().is_err());
        let fine = Image {
            width: 3,
            height: 1,
            ..short
        };
        let nan = [Placement {
            content: Some(PlacementContent {
                uv_scale: [f32::NAN, 1.0],
                ..PlacementContent::new(fine)
            }),
            ..Placement::new(0, IDENTITY, 0)
        }];
        assert!(stage(&meshes, &nan, &materials).build().is_err());
    }

    fn details_match(left: &Detail, right: &Detail) {
        assert_eq!(left.triangles, right.triangles);
        assert_eq!(left.instances, right.instances);
        assert_eq!(left.lens, right.lens);
        assert_eq!(left.projection, right.projection);
        assert_eq!(left.sun_radius_deg, right.sun_radius_deg);
        assert_eq!(left.content, right.content);
        assert_eq!(left.content_slots, right.content_slots);
        assert_eq!(left.leaf, right.leaf);
        assert_eq!(left.steam, right.steam);
        assert_eq!(left.lights, right.lights);
    }

    #[test]
    fn a_placement_carries_clip_and_shadow_only_into_the_detail() {
        let meshes = [triangle(None)];
        let materials = [Material::default()];
        let clip = [[0.0, 1.0, 0.0, -0.5], [1.0, 0.0, 0.0, 0.2]];
        let placed = [Placement {
            clip,
            shadow_only: true,
            ..Placement::new(0, IDENTITY, 0)
        }];
        let built = stage(&meshes, &placed, &materials).build().unwrap();
        let mut patched = stage(&meshes, &[Placement::new(0, IDENTITY, 0)], &materials)
            .build()
            .unwrap();
        patched.detail.instances[0].shadow_only = true;
        patched.detail.instances[0].surface.clip = clip;
        details_match(&built.detail, &patched.detail);
        assert!(built.detail.instances[0].surface.cuts([0.0, 0.0, 0.0]));
        assert!(!built.detail.instances[0].surface.cuts([0.0, -1.0, 0.0]));
    }

    #[test]
    fn a_placement_is_two_sided_unless_it_says_otherwise() {
        let meshes = [triangle(None)];
        let materials = [Material::default()];
        let open = stage(&meshes, &[Placement::new(0, IDENTITY, 0)], &materials)
            .build()
            .unwrap();
        assert!(open.detail.instances[0].two_sided);
        let closed = [Placement {
            two_sided: false,
            ..Placement::new(0, IDENTITY, 0)
        }];
        let built = stage(&meshes, &closed, &materials).build().unwrap();
        assert!(!built.detail.instances[0].two_sided);
    }

    #[test]
    fn one_mesh_keeps_a_polygon_for_each_cutoff() {
        let alpha = [1.0_f32, 0.0, 0.0];
        let meshes = [triangle(Some(&alpha))];
        let materials = [Material::default()];
        let instances = [
            Placement {
                alpha_cutoff: 0.25,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                alpha_cutoff: 0.9,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement::new(0, IDENTITY, 0),
        ];
        let staged = stage(&meshes, &instances, &materials).build().unwrap();
        let areas: Vec<f32> = staged
            .detail
            .instances
            .iter()
            .map(|instance| instance.triangles.iter().map(area).sum())
            .collect();
        assert!((areas[0] - 0.28125).abs() < 1e-6, "{}", areas[0]);
        assert!((areas[1] - 0.005).abs() < 1e-6, "{}", areas[1]);
        assert!((areas[2] - 0.125).abs() < 1e-6, "{}", areas[2]);
        assert_eq!(staged.detail.instances[0].surface.alpha_cutoff, 0.25);
        assert_eq!(
            staged.detail.instances[2].surface.alpha_cutoff,
            ALPHA_CUTOFF
        );
    }

    #[test]
    fn a_cutoff_outside_zero_to_one_or_a_non_finite_clip_is_refused() {
        let meshes = [triangle(None)];
        let materials = [Material::default()];
        let bad = [
            Placement {
                alpha_cutoff: 1.1,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                alpha_cutoff: -0.01,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                alpha_cutoff: f32::NAN,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                alpha_cutoff: f32::INFINITY,
                ..Placement::new(0, IDENTITY, 0)
            },
            Placement {
                clip: [[f32::NAN, 0.0, 0.0, 0.0], [0.0; 4]],
                ..Placement::new(0, IDENTITY, 0)
            },
        ];
        for placement in bad {
            assert!(stage(&meshes, &[placement], &materials).build().is_err());
        }
        for cutoff in [0.0, 1.0] {
            let placement = [Placement {
                alpha_cutoff: cutoff,
                ..Placement::new(0, IDENTITY, 0)
            }];
            assert!(stage(&meshes, &placement, &materials).build().is_ok());
        }
        let spare = Mesh {
            indices: &[0, 1, 3],
            ..triangle(None)
        };
        assert!(
            stage(
                &[triangle(None), spare],
                &[Placement::new(0, IDENTITY, 0)],
                &materials
            )
            .build()
            .is_err()
        );
    }

    #[test]
    fn a_loaded_scene_stages_its_lens_sun_radius_cutoff_movers_and_text() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../tmp/trace-scene-tests/keys");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for entry in std::fs::read_dir(manifest.join("../load/tests/scenes")).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        std::fs::copy(
            manifest.join("../text/fonts/EBGaramond[wght].ttf"),
            root.join("serif.ttf"),
        )
        .unwrap();
        let path = root.join("keys.scene.toml");
        std::fs::write(
            &path,
            "materials = [\"materials.toml\"]\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[[object]]\nname = \"lid\"\nmesh = \"block\"\nalpha_cutoff = 0.3\n\n[[mover]]\nname = \"slide\"\nobjects = [\"lid\"]\nkind = \"slide\"\naxis = [1.0, 0.0, 0.0]\ntravel = [0.0, 2.0]\nperiod = 2.0\n\n[sun]\nmodel = \"authored\"\ntoward = [0.0, 1.0, 0.0]\nirradiance = 1.0\nradius = 1.5\n\n[camera]\nat = [0.0, 0.0, 5.0]\nfocal = 50.0\nfstop = 2.0\nshift = [0.1, 0.2]\n\n[text.sign]\ntext = \"Hi\"\nfont = \"serif.ttf\"\nsize = 0.5\nat = [0.0, 1.0, 0.0]\n",
        )
        .unwrap();
        let loaded = LoadedScene::open(&path).unwrap();
        let rest = scene(&loaded, 1.5).unwrap();
        let moved = scene_at(&loaded, 1.5, 1.0).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(rest.detail.sun_radius_deg, 1.5);
        assert_eq!(rest.detail.lens.shift, [0.1, 0.2]);
        assert!((rest.detail.lens.aperture - 0.025).abs() < 1e-6);
        assert!((rest.detail.lens.focus_distance - 5.0).abs() < 1e-5);
        assert_eq!(rest.detail.instances.len(), 2);
        assert_eq!(rest.detail.instances[0].surface.alpha_cutoff, 0.3);
        let travelled =
            moved.detail.instances[0].transform[3][0] - rest.detail.instances[0].transform[3][0];
        assert!((travelled - 2.0).abs() < 1e-5, "{travelled}");
        let text = &rest.detail.instances[1];
        assert!(!text.surface.casts_shadow);
        assert!((text.transform[3][1] - 1.0).abs() < 1e-6);
        assert_eq!(text.transform[1][1], -1.0);
        assert!(rest.detail.text_slots.iter().any(Option::is_some));
        assert_eq!(
            rest.scene.materials.len(),
            loaded.draws().materials.len() + 2
        );
    }

    #[test]
    fn a_scene_trace_table_turns_on_transmissive_shadows() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/stage-trace");
        std::fs::create_dir_all(&folder).unwrap();
        let mut staged = Vec::new();
        for (name, text) in [
            ("plain.scene.toml", ""),
            ("on.scene.toml", "[trace]\ntransmissive_shadows = true\n"),
        ] {
            let path = folder.join(name);
            let glow = "[[emitter]]\nname = \"glow\"\nposition = [0.0, 1.0, 0.0]\nradius = 0.2\nintensity = 1.0\n\n";
            std::fs::write(&path, format!("{glow}{text}")).unwrap();
            let scene = LoadedScene::open(&path).unwrap();
            staged.push(
                super::scene(&scene, 1.0)
                    .unwrap()
                    .detail
                    .transmissive_shadows,
            );
        }
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(staged, [false, true]);
    }

    #[test]
    fn a_scene_trace_table_sets_the_firefly_clamp_and_the_glossy_filter() {
        let folder =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/stage-fireflies");
        std::fs::create_dir_all(&folder).unwrap();
        let mut staged = Vec::new();
        for (name, text) in [
            ("plain.scene.toml", ""),
            (
                "on.scene.toml",
                "[trace]\nclamp_indirect = 12.0\nfilter_glossy = 0.2\n",
            ),
        ] {
            let path = folder.join(name);
            let glow = "[[emitter]]\nname = \"glow\"\nposition = [0.0, 1.0, 0.0]\nradius = 0.2\nintensity = 1.0\n\n";
            std::fs::write(&path, format!("{glow}{text}")).unwrap();
            let scene = LoadedScene::open(&path).unwrap();
            let detail = super::scene(&scene, 1.0).unwrap().detail;
            staged.push((detail.clamp_indirect, detail.filter_glossy));
        }
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(staged, [(0.0, 0.0), (12.0, 0.2)]);
    }
}
