use bytemuck::{Pod, Zeroable};
use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_core::memory;
use pfx_gpu::pace::{Pacer, Slice, Stats, Turned, Turns, Work};
use pfx_gpu::{Gpu, GpuRefusal, GpuShortfall, OffscreenTarget, trace_floor};
use pfx_load::Sky;
use pfx_load::scene::TraceSettings;
use pfx_materials::{Content, ContentLayer, Material, PackedMaterial};
use wgpu::util::DeviceExt;

use crate::bvh::Triangle;
use crate::detail::{Bounces, Detail, InstanceSurface, Lens, PlumeLook, TriangleDetail};
use crate::lights::{Candidate, MAX_EMITTERS, MAX_LIGHTS, Source, build_emitters, emits};
use crate::shapes::Shape;
use crate::sky::{ENVIRONMENT_WGSL, Environment, SkyCdf};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuTriangle {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    na: [f32; 4],
    nb: [f32; 4],
    nc: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuSurface {
    clip0: [f32; 4],
    clip1: [f32; 4],
    transform: [f32; 4],
    crop: [f32; 4],
    info: [u32; 4],
    forward: [[f32; 4]; 3],
    inverse: [[f32; 4]; 3],
}

pub const NO_MATERIAL: u32 = u32::MAX;
const IDENTITY_ROWS: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

fn rows3(rows: [f32; 12]) -> [[f32; 4]; 3] {
    std::array::from_fn(|row| std::array::from_fn(|k| rows[row * 4 + k]))
}

impl GpuSurface {
    fn of(
        surface: InstanceSurface,
        info: [u32; 4],
        forward: [f32; 12],
        inverse: [f32; 12],
    ) -> Self {
        let mut packed = Self {
            clip0: surface.clip[0],
            clip1: surface.clip[1],
            transform: [
                surface.uv_offset[0],
                surface.uv_offset[1],
                surface.uv_scale[0],
                surface.uv_scale[1],
            ],
            crop: surface.crop,
            info,
            forward: rows3(forward),
            inverse: rows3(inverse),
        };
        if let Some(sprite) = surface.sprite {
            let [r, g, b] = sprite.colour;
            packed.transform = [r, g, b, sprite.alpha];
            packed.crop = [
                f32::from(u8::from(sprite.lit)),
                f32::from(u8::from(sprite.additive)),
                sprite.soft,
                0.0,
            ];
        }
        packed
    }

    fn world() -> Self {
        Self::of(
            InstanceSurface::default(),
            [0, NO_MATERIAL, 0, 1.0f32.to_bits()],
            IDENTITY_ROWS,
            IDENTITY_ROWS,
        )
    }

    fn placed(surface: InstanceSurface, placed: &crate::levels::Placed) -> Result<Self, String> {
        let pose = crate::detail::Pose::new(placed.transform)?;
        Ok(Self::of(
            surface,
            [
                placed.word,
                placed.material.unwrap_or(NO_MATERIAL),
                surface.alpha_cutoff.to_bits(),
                pose.inverse_determinant().to_bits(),
            ],
            pose.rows(),
            placed.inverse,
        ))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLayer {
    layer: [f32; 4],
    extra: [f32; 4],
}

impl From<ContentLayer> for GpuLayer {
    fn from(layer: ContentLayer) -> Self {
        Self {
            layer: [
                layer.slot as f32,
                layer.blend.id() as f32,
                layer.ink_roughness,
                layer.emboss,
            ],
            extra: [layer.strength, 0.0, 0.0, 0.0],
        }
    }
}

pub const SHADOW_ONLY: u32 = 1;
pub const CASTS_NO_SHADOW: u32 = 2;
pub const CUTOUT: u32 = 4;
pub const FACE_SHIFT: u32 = 3;
pub const LAYER_SHIFT: u32 = 5;
pub const ONE_SIDED: u32 = 1 << 23;
pub const SPRITE: u32 = 1 << 24;
pub const LAYER_MASK: u32 = (1 << 18) - 1;
const LAYER_LIMIT: usize = 1 << 18;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuContent {
    pub(crate) offset: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuShape {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Frame {
    pub(crate) size: [u32; 4],
    counts: [u32; 4],
    origin: [f32; 4],
    pub(crate) forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    lens: [f32; 4],
    window: [u32; 4],
    columns: [u32; 4],
    fog: [[f32; 4]; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub origin: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic {
        width: f32,
        height: f32,
    },
    Equirectangular,
}

impl Projection {
    pub fn valid(self) -> bool {
        match self {
            Self::Orthographic { width, height } => {
                width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0
            }
            Self::Perspective | Self::Equirectangular => true,
        }
    }

    fn fits(self, camera: &Camera) -> bool {
        self == Self::Perspective
            || [camera.forward, camera.right, camera.up]
                .into_iter()
                .all(|axis| axis.into_iter().map(|v| v * v).sum::<f32>() > 1e-12)
    }

    fn place(self, frame: &mut Frame) {
        let [kind, half_width, half_height] = match self {
            Self::Perspective => [0.0; 3],
            Self::Orthographic { width, height } => [1.0, width * 0.5, height * 0.5],
            Self::Equirectangular => [2.0, 0.0, 0.0],
        };
        frame.origin[3] = kind;
        frame.right[3] = half_width;
        frame.up[3] = half_height;
    }
}

pub fn equirect_direction(u: f32, v: f32) -> [f32; 3] {
    let phi = (u - 0.5) * std::f32::consts::TAU;
    let theta = v * std::f32::consts::PI;
    [
        theta.sin() * phi.sin(),
        theta.cos(),
        -theta.sin() * phi.cos(),
    ]
}

#[derive(Clone, Copy, Debug)]
pub struct Sun {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
}

impl Sun {
    pub fn from_daylight(daylight: Daylight) -> Self {
        let sun = daylight.sun();
        let light = daylight.light(REFERENCE_HOUR);
        Self {
            direction: sun.y_up.map(|value| value as f32),
            color: light.colour.map(|value| value as f32),
            intensity: light.energy as f32,
        }
    }
}

pub struct Scene<E = Sky> {
    pub triangles: Vec<Triangle>,
    pub shapes: Vec<Shape>,
    pub materials: Vec<Material>,
    pub sky: E,
    pub camera: Camera,
    pub sun: Sun,
}

pub const SAMPLE_LABEL: &str = "trace samples";
pub const BAND_STEP: u32 = 8;
pub const SAMPLE_SPAN_MS: f64 = 12.0;
pub const SAMPLE_TARGET_MS: f64 = 10.0;
const LANES: u32 = 64;
const PIECE_COLUMNS: u32 = 8;

#[derive(Clone, Debug, PartialEq)]
pub enum TraceError {
    Refused(GpuRefusal),
    Setup(String),
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Setup(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for TraceError {}

impl From<String> for TraceError {
    fn from(message: String) -> Self {
        Self::Setup(message)
    }
}

impl From<&str> for TraceError {
    fn from(message: &str) -> Self {
        Self::Setup(message.into())
    }
}

impl From<GpuRefusal> for TraceError {
    fn from(refusal: GpuRefusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<TraceError> for String {
    fn from(error: TraceError) -> Self {
        error.to_string()
    }
}

pub struct Trace {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) samples: u32,
    projection: Projection,
    pub(crate) base: Frame,
    pub(crate) frame: wgpu::Buffer,
    binding: wgpu::BindGroup,
    environment_binding: wgpu::BindGroup,
    look: [[f32; 4]; 2],
    pipeline: wgpu::ComputePipeline,
    pub(crate) materials: u32,
    pub(crate) accum: wgpu::Buffer,
    pub(crate) probe_parts: crate::probe::Parts,
    pacer: Pacer,
    turns: Turns,
    lanes: Lanes,
    pub color: OffscreenTarget,
    pub albedo: OffscreenTarget,
    pub normal: OffscreenTarget,
    rt: Option<crate::rt::Rt>,
    placing: Option<Placing>,
    surface_buffer: wgpu::Buffer,
    node_buffer: wgpu::Buffer,
    emitter_buffer: wgpu::Buffer,
    emitter_cdf_buffer: wgpu::Buffer,
    builds: Builds,
    gpu_bytes: u64,
    half_sky: bool,
    sky_halvings: u32,
    moving: Option<MovingState>,
    fogged: bool,
}

struct MovingState {
    moving: crate::moving::Moving,
    offset: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Builds {
    pub bottom: usize,
    pub top: usize,
}

#[derive(Debug, PartialEq)]
pub struct Output {
    pub color: Vec<u8>,
    pub albedo: Vec<u8>,
    pub normal: Vec<u8>,
}

pub struct SceneBuffers {
    pub triangles: Vec<u8>,
    pub nodes: Vec<u8>,
    pub shapes: Vec<u8>,
    pub materials: Vec<u8>,
    pub uv: Vec<u8>,
    pub content: Vec<u8>,
    pub surfaces: Vec<u8>,
    pub layers: Vec<u8>,
    pub content_info: Vec<u8>,
    pub emitters: Vec<u8>,
    pub emitter_cdf: Vec<u8>,
    pub lights: Vec<u8>,
    pub triangle_count: u32,
    pub shape_count: u32,
    pub emitter_count: u32,
    pub light_count: u32,
    pub rt: crate::rt::RtGeometry,
    levels: Option<Placing>,
    bottom_builds: usize,
    features: Features,
    moving: Option<crate::moving::Moving>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Features {
    lobes: bool,
    maps: bool,
    wet: bool,
    thin: bool,
    alpha: bool,
    flakes: bool,
}

impl Features {
    fn of(materials: &[PackedMaterial]) -> Self {
        Self {
            lobes: materials.iter().any(|m| {
                m.sheen[..3].iter().any(|&value| value > 0.0)
                    || m.lobes[0] != 0.0
                    || m.lobes[2] > 0.0
            }),
            maps: materials
                .iter()
                .any(|m| m.textures.iter().any(|&slot| slot >= 0.0) || m.texture_extra[0] >= 0.0),
            wet: materials.iter().any(|m| m.wet[0] > 0.0 || m.wet[1] > 0.0),
            thin: materials.iter().any(crate::record::thin_on),
            alpha: materials.iter().any(crate::record::alpha_on),
            flakes: materials.iter().any(crate::record::flakes_on),
        }
    }

    fn constants(self) -> impl Iterator<Item = (&'static str, f64)> {
        [
            ("TRACE_LOBES", self.lobes),
            ("TRACE_MAPS", self.maps),
            ("TRACE_WET", self.wet),
            ("TRACE_THIN", self.thin),
            ("TRACE_ALPHA", self.alpha),
            ("TRACE_FLAKES", self.flakes),
        ]
        .into_iter()
        .filter(|(_, on)| !on)
        .map(|(name, _)| (name, 0.0))
    }
}

struct Placing {
    levels: crate::levels::Levels,
    candidates: Vec<Candidate>,
    lights: usize,
    tree: usize,
}

fn pack_triangle(tri: &TriangleDetail, word: u32) -> GpuTriangle {
    GpuTriangle {
        a: vec4(tri.vertices[0], tri.material as f32),
        b: vec4(tri.vertices[1], word as f32),
        c: vec4(tri.vertices[2], 0.0),
        na: [
            tri.normals[0][0],
            tri.normals[0][1],
            tri.normals[0][2],
            tri.uvs[0][0],
        ],
        nb: [
            tri.normals[1][0],
            tri.normals[1][1],
            tri.normals[1][2],
            tri.uvs[1][0],
        ],
        nc: [
            tri.normals[2][0],
            tri.normals[2][1],
            tri.normals[2][2],
            tri.uvs[2][0],
        ],
    }
}

impl SceneBuffers {
    pub fn build(
        triangles: &[Triangle],
        shapes: &[Shape],
        materials: &[Material],
        detail: &Detail,
    ) -> Result<Self, String> {
        Self::build_with(
            triangles,
            shapes,
            materials,
            detail,
            true,
            &crate::textured::Shrink::default(),
        )
    }

    fn build_with(
        triangles: &[Triangle],
        shapes: &[Shape],
        materials: &[Material],
        detail: &Detail,
        emitter_sampling: bool,
        shrink: &crate::textured::Shrink,
    ) -> Result<Self, String> {
        if detail.lights.len() > MAX_LIGHTS || detail.lights.iter().any(|light| !light.valid()) {
            return Err("invalid trace light".into());
        }
        if materials.is_empty()
            || detail.content.len() > materials.len()
            || detail.content.iter().flatten().any(|image| !image.valid())
            || detail
                .content_slots
                .iter()
                .flatten()
                .any(|image| !image.valid())
            || detail.text_slots.iter().flatten().any(|text| !text.valid())
            || detail
                .instances
                .iter()
                .any(|instance| !instance.surface.valid())
        {
            return Err("invalid trace scene detail".into());
        }
        let mut world: Vec<TriangleDetail> = memory::vec(
            triangles.len() + detail.triangles.len(),
            "the tracer's world triangles",
        )?;
        world.extend(triangles.iter().copied().map(TriangleDetail::flat));
        world.extend_from_slice(&detail.triangles);
        if materials.len() + detail.instances.len() >= LAYER_LIMIT {
            return Err("trace scene has too many content layers".into());
        }
        let mut placed_layers = Vec::new();
        let mut words = Vec::with_capacity(detail.instances.len());
        for instance in &detail.instances {
            let surface = instance.surface;
            let mut word = surface.face.code() << FACE_SHIFT;
            if surface.layer.active() {
                placed_layers.push(GpuLayer::from(surface.layer));
                word |= ((materials.len() + placed_layers.len()) as u32) << LAYER_SHIFT;
            }
            if instance.shadow_only {
                word |= SHADOW_ONLY;
            }
            if !surface.casts_shadow {
                word |= CASTS_NO_SHADOW;
            }
            if surface.cutout {
                word |= CUTOUT;
            }
            if !instance.two_sided {
                word |= ONE_SIDED;
            }
            if surface.sprite.is_some() {
                word |= SPRITE;
            }
            words.push(word);
        }
        let broken = |tri: &TriangleDetail| {
            tri.vertices.into_iter().any(|vertex| !finite3(vertex))
                || tri.normals.into_iter().any(|normal| !finite3(normal))
                || tri
                    .uvs
                    .into_iter()
                    .flatten()
                    .any(|value| !value.is_finite())
                || tri.material as usize >= materials.len()
        };
        let mut checked: Vec<&std::sync::Arc<[TriangleDetail]>> = Vec::new();
        let mut broken_instance = false;
        for instance in &detail.instances {
            if checked
                .iter()
                .any(|seen| std::sync::Arc::ptr_eq(seen, &instance.triangles))
            {
                continue;
            }
            checked.push(&instance.triangles);
            broken_instance |= instance.triangles.iter().any(broken);
        }
        drop(checked);
        if broken_instance
            || world.iter().any(broken)
            || shapes
                .iter()
                .copied()
                .any(|shape| !valid_shape(shape) || shape.material() as usize >= materials.len())
        {
            return Err("invalid trace geometry".into());
        }
        for (index, shape) in shapes.iter().enumerate() {
            if let Shape::SmoothUnion { left, right, .. } = shape
                && (*left >= index
                    || *right >= index
                    || matches!(shapes[*left], Shape::SmoothUnion { .. })
                    || matches!(shapes[*right], Shape::SmoothUnion { .. }))
            {
                return Err("smooth union needs earlier primitive shapes".into());
            }
        }
        let mut effects = crate::record::Hot::default();
        if let Some(leaf) = detail.leaf {
            effects.base_roughness = [leaf.time, leaf.strength, 0.0, 0.0];
            effects.metal_spec_coat = vec4(leaf.center, 0.0);
        }
        if let Some(steam) = detail.steam {
            effects.base_roughness[3] = steam.time;
            effects.metal_spec_coat[3] = steam.anisotropy;
            effects.sheen_trans_ior_disp = vec4(steam.source, steam.radius);
            effects.thick_sub_film = vec4(steam.box_lo, steam.density);
            effects.tint_absorption = vec4(steam.box_hi, steam.ambient);
        }
        let slotted = !detail.content_slots.is_empty() || !detail.text_slots.is_empty();
        let slot_images = if !slotted {
            &detail.content
        } else {
            &detail.content_slots
        };
        let slot_count = slot_images.len().max(detail.text_slots.len());
        let mut content_info = vec![GpuContent::zeroed(); slot_count.max(1)];
        let mut content_bytes: Vec<u8> = crate::textured::srgb_rows()?;
        for (index, image) in slot_images.iter().enumerate() {
            if let Some(image) = image {
                let mut placed = Vec::with_capacity(1);
                crate::textured::push(
                    image,
                    crate::textured::at(&shrink.content, index),
                    &mut placed,
                    &mut content_bytes,
                )?;
                content_info[index] = placed[0];
            }
        }
        for (index, text) in detail.text_slots.iter().enumerate() {
            if let Some(text) = text {
                if slot_images.get(index).is_some_and(Option::is_some) {
                    return Err("a content slot holds both an image and text".into());
                }
                content_info[index] = GpuContent {
                    offset: crate::textured::row(&content_bytes),
                    width: text.width,
                    height: text.height,
                    kind: 1,
                };
                memory::more(
                    &mut content_bytes,
                    16 * text.records.len(),
                    "the tracer's content",
                )?;
                for record in &text.records {
                    crate::textured::put_row(&mut content_bytes, record.map(f32::to_bits));
                }
            }
        }
        let texture_slots = crate::textured::place(
            &detail.textures,
            shrink,
            &mut content_info,
            &mut content_bytes,
        )?;
        let mut content_layers: Vec<GpuLayer> = materials
            .iter()
            .enumerate()
            .map(|(index, material)| {
                let layer = if material.content_layer.active() {
                    material.content_layer
                } else if !slotted && detail.content.get(index).is_some_and(Option::is_some) {
                    if material.content == Content::None {
                        ContentLayer {
                            slot: index as i32,
                            ..ContentLayer::default()
                        }
                    } else {
                        ContentLayer::from_kind(
                            material.content,
                            material.clearcoat,
                            index as i32,
                            &detail.content_look,
                        )
                    }
                } else {
                    ContentLayer::default()
                };
                GpuLayer::from(layer)
            })
            .collect();
        content_layers.extend(placed_layers);
        if content_layers
            .iter()
            .any(|layer| layer.layer[0] >= content_info.len() as f32)
        {
            return Err("content layer slot is missing".into());
        }
        let mut packed_shapes: Vec<_> = shapes.iter().copied().map(pack_shape).collect();
        for shape in shapes {
            if let Shape::SmoothUnion { left, right, .. } = shape {
                packed_shapes[*left].d[0] = 1.0;
                packed_shapes[*right].d[0] = 1.0;
            }
        }
        let mut packed_materials: Vec<_> = materials.iter().map(PackedMaterial::pack).collect();
        for packed in &mut packed_materials {
            texture_slots.remap(packed);
        }
        let features = Features::of(&packed_materials);
        let emitting: Vec<bool> = packed_materials
            .iter()
            .zip(&content_layers)
            .zip(materials)
            .map(|((packed, layer), material)| {
                let film = packed.emission_film;
                emitter_sampling
                    && layer.layer[0] < 0.0
                    && material.alpha_mode.opaque()
                    && emits([film[0], film[1], film[2]])
            })
            .collect();
        let unique: Vec<bool> = detail
            .instances
            .iter()
            .zip(&words)
            .map(|(instance, &word)| {
                word & (SHADOW_ONLY | CUTOUT) == 0
                    && (word >> LAYER_SHIFT) & LAYER_MASK == 0
                    && instance
                        .triangles
                        .iter()
                        .any(|tri| emitting[tri.material as usize])
            })
            .collect();
        crate::moving::check(detail)?;
        let moves = crate::moving::moves(detail);
        let laid = crate::levels::lay(&world, &detail.instances, &words, &unique, moves)?;
        let order = &laid.order;
        let slots = laid.slots;
        let stride = std::mem::size_of::<GpuTriangle>();
        let mut triangle_bytes: Vec<u8> = memory::vec(slots * stride, "the tracer's triangles")?;
        let mut uv_v: Vec<[f32; 4]> = memory::vec(slots, "the tracer's triangle uvs")?;
        let mut put = |tri: &TriangleDetail| {
            triangle_bytes.extend_from_slice(bytemuck::bytes_of(&pack_triangle(tri, 0)));
            uv_v.push([tri.uvs[0][1], tri.uvs[1][1], tri.uvs[2][1], 0.0]);
        };
        for &index in order {
            put(&world[index as usize]);
        }
        if let Some(levels) = &laid.levels {
            for mesh in &levels.meshes {
                mesh.triangles.iter().for_each(&mut put);
            }
        }
        let mut records = vec![GpuSurface::world()];
        let mut opaque = Vec::new();
        if let Some(levels) = &laid.levels {
            for (instance, placed) in detail.instances.iter().zip(&levels.placed) {
                let Some(placed) = placed else { continue };
                records.push(GpuSurface::placed(instance.surface, placed)?);
                let mesh = &levels.meshes[placed.mesh];
                let clear = |material: u32| {
                    let material = &materials[material as usize];
                    material.transmitted() <= 0.0 && material.alpha_mode.opaque()
                };
                opaque.push(
                    placed.word & (SHADOW_ONLY | CASTS_NO_SHADOW | CUTOUT | ONE_SIDED | SPRITE)
                        == 0
                        && instance.surface.clip[0][..3] == [0.0; 3]
                        && instance.surface.clip[1][..3] == [0.0; 3]
                        && match placed.material {
                            Some(material) => clear(material),
                            None => mesh.triangles.iter().all(|tri| clear(tri.material)),
                        },
                );
            }
        }
        if packed_materials.iter().any(crate::textured::wants_uvs1) {
            let meshes = laid.levels.iter().flat_map(|levels| &levels.meshes);
            effects.set_uvs1(crate::textured::place_uvs1(
                order
                    .iter()
                    .map(|&index| world[index as usize].uvs1)
                    .chain(meshes.flat_map(|mesh| mesh.triangles.iter().map(|tri| tri.uvs1))),
                &mut content_bytes,
            )?);
        }
        if content_bytes.len() / 16 > u32::MAX as usize {
            return Err("trace content is too large".into());
        }
        let records_split = crate::record::Records::split(&packed_materials, effects);
        let emission = |material: u32| records_split.hot(material as usize).emission();
        let mut candidates = Vec::new();
        let mark = |bytes: &mut Vec<u8>, slot: usize, value: usize| {
            let at = slot * stride + 2 * 16 + 12;
            bytes[at..at + 4].copy_from_slice(&(value as f32).to_le_bytes());
        };
        for (slot, &index) in order.iter().enumerate() {
            let tri = world[index as usize];
            if emitting[tri.material as usize] && crate::lights::triangle_area(tri.vertices) > 1e-12
            {
                candidates.push(Candidate {
                    source: Source::Triangle {
                        vertices: tri.vertices,
                        slot: slot as u32,
                        record: 0,
                    },
                    emission: emission(tri.material),
                });
                mark(&mut triangle_bytes, slot, candidates.len());
            }
        }
        if let Some(levels) = &laid.levels {
            for (placed, &alone) in levels.placed.iter().zip(&unique) {
                let Some(placed) = placed.as_ref().filter(|_| alone) else {
                    continue;
                };
                let pose = crate::detail::Pose::new(placed.transform)?;
                let mesh = &levels.meshes[placed.mesh];
                for (j, tri) in mesh.triangles.iter().enumerate() {
                    let material = placed.material.unwrap_or(tri.material);
                    let vertices = tri.vertices.map(|point| pose.point(point));
                    if emitting[material as usize] && crate::lights::triangle_area(vertices) > 1e-12
                    {
                        let slot = mesh.first as usize + j;
                        candidates.push(Candidate {
                            source: Source::Triangle {
                                vertices,
                                slot: slot as u32,
                                record: placed.record,
                            },
                            emission: emission(material),
                        });
                        mark(&mut triangle_bytes, slot, candidates.len());
                    }
                }
            }
        }
        for (index, shape) in shapes.iter().enumerate() {
            if packed_shapes[index].d[0] > 0.5 || !emitting[shape.material() as usize] {
                continue;
            }
            let (center, radius) = crate::lights::bounding_sphere(*shape, shapes);
            let area = crate::lights::shape_area(*shape, shapes);
            candidates.push(Candidate {
                source: Source::Shape {
                    center,
                    radius,
                    index: index as u32,
                    area,
                },
                emission: emission(shape.material()),
            });
            packed_shapes[index].d[1] = candidates.len() as f32;
        }
        if candidates.len() > MAX_EMITTERS {
            return Err("trace scene has too many emitters".into());
        }
        let mut solid: Vec<bool> = memory::vec(order.len(), "the tracer's world flags")?;
        solid.extend(order.iter().map(|&index| {
            let material = &materials[world[index as usize].material as usize];
            material.transmitted() <= 0.0 && material.alpha_mode.opaque()
        }));
        let mut placed: Vec<[[f32; 3]; 3]> =
            memory::vec(order.len(), "the tracer's world vertices")?;
        placed.extend(order.iter().map(|&index| world[index as usize].vertices));
        drop(world);
        let boxes = shapes
            .iter()
            .enumerate()
            .filter(|(index, _)| packed_shapes[*index].d[0] <= 0.5)
            .map(|(index, shape)| {
                let (center, radius) = crate::lights::bounding_sphere(*shape, shapes);
                (
                    index as u32,
                    [
                        center[0] - radius,
                        center[1] - radius,
                        center[2] - radius,
                        center[0] + radius,
                        center[1] + radius,
                        center[2] + radius,
                    ],
                )
            })
            .collect();
        let boxes = if crate::rt::boxes() {
            boxes
        } else {
            Vec::new()
        };
        let mut rt = crate::rt::RtGeometry::new(&placed, &solid, boxes);
        if let Some(levels) = &laid.levels {
            rt.place(levels, &opaque);
        }
        let moving = match (&detail.motion, moves) {
            (Some(motion), true) => {
                let slots = laid
                    .levels
                    .as_ref()
                    .map_or(0, |levels| levels.placed.iter().flatten().count());
                let mut moving = crate::moving::Moving::new(motion, slots);
                if let Some(levels) = &laid.levels {
                    for (instance, placed) in detail.instances.iter().zip(&levels.placed) {
                        let Some(placed) = placed.as_ref().filter(|placed| placed.deform > 0)
                        else {
                            continue;
                        };
                        let Some(shapes) = instance.motion.as_ref().map(|motion| &motion.shapes)
                        else {
                            continue;
                        };
                        let mesh = &levels.meshes[placed.mesh];
                        moving.firsts[placed.record as usize - 1] =
                            (triangle_bytes.len() / stride) as u32;
                        for shape in shapes {
                            for &index in &mesh.order {
                                triangle_bytes.extend_from_slice(bytemuck::bytes_of(
                                    &pack_triangle(&shape[index as usize], 0),
                                ));
                            }
                        }
                    }
                }
                Some(moving)
            }
            _ => None,
        };
        let laid_lights = crate::tree::lay(&detail.lights);
        let list = build_emitters(&candidates, laid_lights.looped, laid_lights.tree);
        let lights = laid_lights.rows;
        Ok(Self {
            triangles: triangle_bytes,
            nodes: {
                let mut bytes = memory::vec(
                    std::mem::size_of_val(laid.nodes.as_slice()),
                    "the tracer's bvh nodes",
                )?;
                bytes.extend_from_slice(bytemuck::cast_slice(&laid.nodes));
                bytes
            },
            shapes: bytemuck::cast_slice(&packed_shapes).to_vec(),
            materials: records_split.bytes(),
            uv: {
                let mut bytes = memory::vec(uv_v.len() * 16, "the tracer's triangle uvs")?;
                bytes.extend_from_slice(bytemuck::cast_slice(&uv_v));
                bytes
            },
            content: content_bytes,
            surfaces: bytemuck::cast_slice(&records).to_vec(),
            layers: bytemuck::cast_slice(&content_layers).to_vec(),
            content_info: bytemuck::cast_slice(&content_info).to_vec(),
            emitters: bytemuck::cast_slice(&list.entries).to_vec(),
            emitter_cdf: bytemuck::cast_slice(&list.cdf).to_vec(),
            lights: bytemuck::cast_slice(&lights).to_vec(),
            triangle_count: slots as u32,
            shape_count: shapes.len() as u32,
            emitter_count: candidates.len() as u32,
            light_count: detail.lights.len() as u32,
            rt,
            levels: laid.levels.map(|levels| Placing {
                levels,
                candidates,
                lights: laid_lights.looped,
                tree: laid_lights.tree,
            }),
            bottom_builds: laid.bottom_builds,
            features,
            moving,
        })
    }
}

fn vec4(v: [f32; 3], w: f32) -> [f32; 4] {
    [v[0], v[1], v[2], w]
}

fn pack_shape(shape: Shape) -> GpuShape {
    match shape {
        Shape::RoundedBox {
            center,
            half,
            radius,
            material,
        } => GpuShape {
            a: vec4(center, 1.0),
            b: vec4(half, radius),
            c: [0.0, 0.0, 0.0, material as f32],
            d: [0.0; 4],
        },
        Shape::RoundCone {
            a,
            b,
            radius_a,
            radius_b,
            material,
        } => GpuShape {
            a: vec4(a, 2.0),
            b: vec4(b, 0.0),
            c: [radius_a, radius_b, 0.0, material as f32],
            d: [0.0; 4],
        },
        Shape::Ellipsoid {
            center,
            radii,
            material,
        } => GpuShape {
            a: vec4(center, 3.0),
            b: vec4(radii, 0.0),
            c: [0.0, 0.0, 0.0, material as f32],
            d: [0.0; 4],
        },
        Shape::Capsule {
            a,
            b,
            radius,
            material,
        } => GpuShape {
            a: vec4(a, 4.0),
            b: vec4(b, 0.0),
            c: [radius, 0.0, 0.0, material as f32],
            d: [0.0; 4],
        },
        Shape::SmoothUnion {
            left,
            right,
            radius,
            material,
        } => GpuShape {
            a: [0.0, 0.0, 0.0, 5.0],
            b: [left as f32, right as f32, radius, 0.0],
            c: [0.0, 0.0, 0.0, material as f32],
            d: [0.0; 4],
        },
    }
}

fn finite3(v: [f32; 3]) -> bool {
    v.into_iter().all(f32::is_finite)
}

fn bounce_limits(bounces: Bounces) -> [(&'static str, f64); 4] {
    [
        ("TOTAL_BOUNCES", bounces.total as f64),
        ("DIFFUSE_BOUNCES", bounces.diffuse as f64),
        ("GLOSSY_BOUNCES", bounces.glossy as f64),
        ("TRANSMISSION_BOUNCES", bounces.transmission as f64),
    ]
}

fn pipeline_constants(
    detail: &Detail,
    height: u32,
    features: Features,
    moving: bool,
) -> Vec<(&'static str, f64)> {
    let mut constants = bounce_limits(detail.bounces).to_vec();
    if moving {
        constants.push(("TRACE_MOTION", 1.0));
    }
    if detail.fog.is_some() {
        constants.push(("TRACE_FOG", 1.0));
    }
    if detail
        .instances
        .iter()
        .any(|instance| instance.surface.sprite.is_some())
    {
        constants.push(("TRACE_SPRITES", 1.0));
    }
    constants.extend(features.constants());
    constants.push((
        "BLOCK_STRIDE",
        f64::from(block_stride(height.div_ceil(BAND_STEP))),
    ));
    if detail.transmissive_shadows {
        constants.push(("TRANSMISSIVE_SHADOWS", 1.0));
    }
    if detail.clamp_indirect > 0.0 {
        constants.push(("CLAMP_INDIRECT", f64::from(detail.clamp_indirect)));
    }
    if detail.filter_glossy > 0.0 {
        constants.push(("FILTER_GLOSSY", f64::from(detail.filter_glossy)));
    }
    if let Some(sequence) = detail.sampler.sequence() {
        constants.push(("SOBOL", 1.0));
        constants.push(("SOBOL_LENGTH", f64::from(sequence)));
    }
    if let Some(steam) = detail.steam {
        constants.extend(
            PlumeLook::NAMES
                .into_iter()
                .zip(steam.look.floats())
                .map(|(name, value)| (name, f64::from(value))),
        );
    }
    constants
}

fn block_stride(blocks: u32) -> u32 {
    if blocks <= 2 {
        return 1;
    }
    let golden = (f64::from(blocks) * 0.381_966_011_25).round() as u32;
    (golden..blocks)
        .chain(1..golden)
        .find(|&stride| coprime(stride, blocks))
        .unwrap_or(1)
}

fn coprime(mut a: u32, mut b: u32) -> bool {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a == 1
}

fn valid_shape(shape: Shape) -> bool {
    match shape {
        Shape::RoundedBox {
            center,
            half,
            radius,
            ..
        } => {
            finite3(center)
                && finite3(half)
                && radius.is_finite()
                && radius >= 0.0
                && half.into_iter().all(|v| v > 0.0 && radius <= v)
        }
        Shape::RoundCone {
            a,
            b,
            radius_a,
            radius_b,
            ..
        } => {
            finite3(a)
                && finite3(b)
                && radius_a.is_finite()
                && radius_b.is_finite()
                && radius_a >= 0.0
                && radius_b >= 0.0
        }
        Shape::Ellipsoid { center, radii, .. } => {
            finite3(center) && finite3(radii) && radii.into_iter().all(|v| v > 0.0)
        }
        Shape::Capsule { a, b, radius, .. } => {
            finite3(a) && finite3(b) && radius.is_finite() && radius > 0.0
        }
        Shape::SmoothUnion { radius, .. } => radius.is_finite() && radius > 0.0,
    }
}

fn refused(gpu: &Gpu, shortfall: GpuShortfall) -> TraceError {
    TraceError::Refused(GpuRefusal::new(shortfall, Some(gpu.info.clone())))
}

fn fits_buffer(gpu: &Gpu, bytes: u64) -> Result<(), TraceError> {
    let limits = gpu.device.limits();
    [
        (
            "max_storage_buffer_binding_size",
            limits.max_storage_buffer_binding_size,
        ),
        ("max_buffer_size", limits.max_buffer_size),
    ]
    .into_iter()
    .find(|(_, has)| bytes > *has)
    .map_or(Ok(()), |(name, has)| {
        Err(refused(
            gpu,
            GpuShortfall::Limit {
                name,
                needed: bytes,
                has,
            },
        ))
    })
}

fn fits_texture(gpu: &Gpu, width: u32, height: u32) -> Result<(), TraceError> {
    let has = gpu.device.limits().max_texture_dimension_2d;
    let side = width.max(height);
    if side > has {
        return Err(refused(
            gpu,
            GpuShortfall::Limit {
                name: "max_texture_dimension_2d",
                needed: u64::from(side),
                has: u64::from(has),
            },
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Pass {
    lanes: (u32, u32),
    per_block: u32,
    columns: (u32, u32),
}

#[derive(Clone, Copy, Debug)]
struct Spread {
    longest: f64,
    single: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Held,
    Blocks,
    Lanes,
    Pieces,
}

#[derive(Clone, Copy, Debug)]
struct Lanes {
    width: u32,
    blocks: u32,
    widest: u32,
    most: u32,
    pieces: u32,
    fewest: u32,
    last: Step,
}

impl Lanes {
    fn new() -> Self {
        Self {
            width: LANES,
            blocks: 1,
            widest: LANES * 2,
            most: u32::MAX,
            pieces: 1,
            fewest: 1,
            last: Step::Held,
        }
    }

    fn strips(&self, blocks: u32) -> u32 {
        let blocks = blocks.max(1);
        let slices = blocks.div_ceil(self.blocks.clamp(1, blocks));
        blocks.div_ceil(slices)
    }

    fn learn(&mut self, longest: f64, blocks: u32) {
        if !longest.is_finite() || longest <= 0.0 {
            return;
        }
        let half = SAMPLE_SPAN_MS * 0.5;
        let last = std::mem::replace(&mut self.last, Step::Held);
        if longest > SAMPLE_SPAN_MS * 0.75 && last != Step::Held {
            if last == Step::Blocks {
                self.most = self.blocks;
                self.blocks = (self.blocks / 2).max(1);
            } else {
                self.widest = self.width;
                self.width = (self.width / 2).max(1);
            }
            return;
        }
        if longest > SAMPLE_SPAN_MS {
            if self.blocks > 1 {
                let fit = (f64::from(self.blocks) * half / longest).floor() as u32;
                self.most = self.blocks;
                self.blocks = fit.clamp(1, self.blocks / 2);
            } else if self.width > 1 {
                let fit = (f64::from(self.width) * half / longest).floor() as u32;
                self.widest = self.width;
                self.width = floor_power_of_two(fit.clamp(1, self.width / 2));
            }
            return;
        }
        if longest > half {
            return;
        }
        let more = (self.blocks * 2).min(blocks).min(self.most - 1);
        if self.blocks < blocks && more > self.blocks {
            self.blocks = more;
            self.last = Step::Blocks;
        } else if self.width * 2 < self.widest {
            self.width = (self.width * 2).min(LANES);
            self.last = Step::Lanes;
        }
    }

    fn learn_pieces(&mut self, longest: f64, blocks: u32, columns: u32) {
        if !longest.is_finite() || longest <= 0.0 {
            return;
        }
        let half = SAMPLE_SPAN_MS * 0.5;
        let last = std::mem::replace(&mut self.last, Step::Held);
        if longest > SAMPLE_SPAN_MS * 0.75 && last != Step::Held {
            if last == Step::Blocks {
                self.most = self.blocks;
                self.blocks = (self.blocks / 2).max(1);
            } else {
                self.fewest = self.pieces;
                self.pieces *= 2;
            }
            return;
        }
        if longest > SAMPLE_SPAN_MS {
            let most = most_pieces(columns);
            if self.blocks > 1 {
                let fit = (f64::from(self.blocks) * half / longest).floor() as u32;
                self.most = self.blocks;
                self.blocks = fit.clamp(1, self.blocks / 2);
            } else if self.pieces < most {
                self.pieces *= 2;
            } else {
                *self = Self::lane_passes();
            }
            return;
        }
        if longest > half {
            return;
        }
        let more = (self.blocks * 2).min(blocks).min(self.most - 1);
        if self.blocks < blocks && more > self.blocks {
            self.blocks = more;
            self.last = Step::Blocks;
        } else if self.pieces / 2 > self.fewest {
            self.pieces /= 2;
            self.last = Step::Pieces;
        }
    }

    fn learn_dense(&mut self, spread: Spread, columns: u32) {
        if spread.single.is_finite() && spread.single > SAMPLE_SPAN_MS {
            let most = most_pieces(columns);
            *self = if most > 1 {
                let fit = (spread.single / (SAMPLE_SPAN_MS * 0.5)).ceil() as u32;
                Self {
                    pieces: fit.next_power_of_two().clamp(2, most),
                    ..Self::new()
                }
            } else {
                Self::lane_passes()
            };
        }
    }

    fn lane_passes() -> Self {
        Self {
            width: 1,
            widest: LANES,
            ..Self::new()
        }
    }
}

fn most_pieces(columns: u32) -> u32 {
    floor_power_of_two(columns / PIECE_COLUMNS)
}

fn floor_power_of_two(value: u32) -> u32 {
    if value == 0 {
        1
    } else {
        1 << (31 - value.leading_zeros())
    }
}

#[derive(Debug, PartialEq)]
struct Piece {
    sample: u32,
    samples: u32,
    order: u32,
    blocks: u32,
    from: u32,
    to: u32,
}

fn pieces(blocks: u32, per_block: u32, start: u32, count: u32) -> Vec<Piece> {
    let per_sample = blocks * per_block;
    let end = start + count;
    let mut pieces = Vec::new();
    let mut at = start;
    while at < end {
        let sample = at / per_sample;
        let within = at % per_sample;
        let order = within / per_block;
        let offset = within % per_block;
        let left = end - at;
        let piece = if within == 0 && left >= per_sample {
            Piece {
                sample,
                samples: left / per_sample,
                order: 0,
                blocks,
                from: 0,
                to: per_block,
            }
        } else if offset == 0 && left >= per_block {
            Piece {
                sample,
                samples: 1,
                order,
                blocks: (left / per_block).min(blocks - order),
                from: 0,
                to: per_block,
            }
        } else {
            Piece {
                sample,
                samples: 1,
                order,
                blocks: 1,
                from: offset,
                to: per_block.min(offset + left),
            }
        };
        at += piece.samples * (piece.blocks * per_block - piece.from) - (per_block - piece.to);
        pieces.push(piece);
    }
    pieces
}

pub fn shader(lit: bool) -> String {
    shader_source(lit, false)
}

pub fn shader_rt(lit: bool) -> String {
    shader_source(lit, true)
}

fn shader_source(lit: bool, rt: bool) -> String {
    let (head, rest) = include_str!("trace.wgsl")
        .split_once("fn trace_sobol_start() {}\n")
        .unwrap();
    let (_, tail) = rest.split_once("fn trace_sobol_end() {}\n").unwrap();
    let source = format!("{head}{}{tail}", include_str!("sobol.wgsl"));
    let (start, rest) = source.split_once("fn trace_noise_start() {}\n").unwrap();
    let (_, end) = rest.split_once("fn trace_noise_end() {}\n").unwrap();
    let end = if lit {
        let (middle, rest) = end.split_once("fn trace_lights_start() {}\n").unwrap();
        let (_, tail) = rest.split_once("fn trace_lights_end() {}\n").unwrap();
        format!("{}\n{}\n{}", middle, include_str!("lights.wgsl"), tail)
    } else {
        end.to_string()
    };
    let source = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        pfx_materials::NOISE,
        pfx_materials::BRDF,
        pfx_materials::LOBES,
        pfx_materials::WET,
        pfx_materials::CONTENT,
        ENVIRONMENT_WGSL.as_str(),
        start,
        crate::TEXTURED_WGSL,
        end
    );
    if !rt {
        return source;
    }
    let source = [
        ("fn node_at(i: u32", "fn soft_node_at(i: u32"),
        ("fn motion_vec(k: u32", "fn soft_motion_vec(k: u32"),
        ("fn scene(o: vec3f", "fn soft_scene(o: vec3f"),
        ("fn occluded(o: vec3f", "fn soft_occluded(o: vec3f"),
        (
            "fn shadow_passed(o: vec3f",
            "fn soft_shadow_passed(o: vec3f",
        ),
    ]
    .into_iter()
    .fold(source, |source, (from, to)| source.replacen(from, to, 1));
    format!(
        "enable wgpu_ray_query;\nconst RT_BOXES: bool = {};\n{}\n{}",
        crate::rt::boxes(),
        source,
        crate::rt::RT_WGSL
    )
}

pub(crate) fn texture(gpu: &Gpu, width: u32, height: u32, label: &'static str) -> OffscreenTarget {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    OffscreenTarget {
        texture,
        view,
        format: wgpu::TextureFormat::Rgba32Float,
        width,
        height,
    }
}

fn storage(gpu: &Gpu, label: &'static str, data: &[u8], read_write: bool) -> wgpu::Buffer {
    if data.is_empty() {
        return gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 256,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
    }
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: data,
            usage: wgpu::BufferUsages::STORAGE
                | if read_write {
                    wgpu::BufferUsages::COPY_DST
                } else {
                    wgpu::BufferUsages::empty()
                },
        })
}

fn sobol_table(gpu: &Gpu, sampler: crate::sobol::Sampler) -> wgpu::Texture {
    let texels = sampler
        .sequence()
        .map_or_else(|| vec![[0.0; 4]], crate::sobol::table);
    let width = crate::sobol::TABLE_WIDTH.min(texels.len() as u32);
    gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("trace sobol table"),
            size: wgpu::Extent3d {
                width,
                height: texels.len() as u32 / width,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        bytemuck::cast_slice(&texels),
    )
}

fn ready<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
    {
        std::task::Poll::Ready(value) => Some(value),
        std::task::Poll::Pending => None,
    }
}

fn halved_sky(sky: &Sky) -> Result<Sky, String> {
    let width = sky.width.div_ceil(2);
    let height = sky.height.div_ceil(2);
    let at =
        |x: u32, y: u32| sky.texels[(y.min(sky.height - 1) * sky.width + x % sky.width) as usize];
    let mut texels = memory::vec(width as usize * height as usize, "the sky")?;
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let texel = at(2 * x + dx, 2 * y + dy);
                for c in 0..4 {
                    sum[c] += texel[c];
                }
            }
            texels.push(sum.map(|value| value * 0.25));
        }
    }
    Ok(Sky {
        width,
        height,
        texels,
    })
}

pub fn sky_halvings(
    sky: &Environment,
    camera: &Camera,
    projection: Projection,
    size: [u32; 2],
) -> u32 {
    let Environment::Hdr(hdr) = sky else {
        return 0;
    };
    let length = |v: [f32; 3]| f64::from(dot3f(v, v)).sqrt();
    let needed = match projection {
        Projection::Perspective => {
            let tangent = length(camera.right) / length(camera.forward).max(f64::MIN_POSITIVE);
            (std::f64::consts::TAU * f64::from(size[0]) / (2.0 * tangent)).ceil()
        }
        Projection::Equirectangular => f64::from(size[0]),
        Projection::Orthographic { .. } => return 0,
    };
    let (mut width, mut height, mut halvings) = (hdr.width, hdr.height, 0);
    while f64::from(width / 2) >= needed && height >= 2 {
        width = width.div_ceil(2);
        height = height.div_ceil(2);
        halvings += 1;
    }
    halvings
}

fn fitted(sky: Environment, halvings: u32) -> Result<Environment, String> {
    let Environment::Hdr(mut hdr) = sky else {
        return Ok(sky);
    };
    for _ in 0..halvings {
        hdr = halved_sky(&hdr)?;
    }
    Ok(Environment::Hdr(hdr))
}

fn dot3f(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn environment_binding(
    gpu: &Gpu,
    pipeline: &wgpu::ComputePipeline,
    environment: &Environment,
    look: [[f32; 4]; 2],
    half: bool,
) -> Result<(wgpu::BindGroup, u64), String> {
    let (width, height, texels) = match environment {
        Environment::Hdr(hdr) => {
            if hdr.width == 0
                || hdr.height == 0
                || hdr.texels.len() != u64::from(hdr.width) as usize * hdr.height as usize
            {
                return Err("invalid sky".into());
            }
            (hdr.width, hdr.height, hdr.texels.as_slice())
        }
        Environment::Analytic(_) => (1, 1, &[[0.0; 4]][..]),
    };
    let cdf = SkyCdf::build(environment, 256, 128).without_sun();
    let halved = half
        && texels
            .iter()
            .flatten()
            .all(|value| value.is_finite() && value.abs() <= f32::from(half::f16::MAX));
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("trace environment"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if halved {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Rgba32Float
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let texel_bytes = if halved { 8 } else { 16 };
    let packed: Vec<u16>;
    let data: &[u8] = if halved {
        packed = texels
            .iter()
            .flatten()
            .map(|&value| half::f16::from_f32(value).to_bits())
            .collect();
        bytemuck::cast_slice(&packed)
    } else {
        bytemuck::cast_slice(texels)
    };
    gpu.queue.write_texture(
        texture.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * texel_bytes),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    let view = texture.create_view(&Default::default());
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("trace environment uniform"),
            contents: bytemuck::bytes_of(&crate::sky::EnvironmentUniform {
                look: look[0],
                horizon: look[1],
                ..environment.uniform()
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let rows = storage(
        gpu,
        "trace sky rows",
        bytemuck::cast_slice(&cdf.rows),
        false,
    );
    let columns = storage(
        gpu,
        "trace sky columns",
        bytemuck::cast_slice(&cdf.columns),
        false,
    );
    let cdf_uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("trace sky cdf uniform"),
            contents: bytemuck::bytes_of(&cdf.uniform()),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let bytes = u64::from(width) * u64::from(height) * u64::from(texel_bytes)
        + 4 * (cdf.rows.len() + cdf.columns.len()) as u64;
    Ok((
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace environment bindings"),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: rows.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: columns.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: cdf_uniform.as_entire_binding(),
                },
            ],
        }),
        bytes,
    ))
}

impl Trace {
    pub fn new<E: Clone + Into<Environment> + 'static>(
        gpu: &Gpu,
        scene: &Scene<E>,
        width: u32,
        height: u32,
    ) -> Result<Self, TraceError> {
        Self::new_detailed(gpu, scene, &Detail::default(), width, height)
    }

    pub fn new_detailed<E: Clone + Into<Environment> + 'static>(
        gpu: &Gpu,
        scene: &Scene<E>,
        detail: &Detail,
        width: u32,
        height: u32,
    ) -> Result<Self, TraceError> {
        Self::build(gpu, scene, detail, width, height, true)
    }

    fn build<E: Clone + Into<Environment> + 'static>(
        gpu: &Gpu,
        scene: &Scene<E>,
        detail: &Detail,
        width: u32,
        height: u32,
        emitter_sampling: bool,
    ) -> Result<Self, TraceError> {
        gpu.check(&trace_floor())?;
        if width == 0 || height == 0 || scene.materials.is_empty() {
            return Err("trace needs dimensions and at least one material".into());
        }
        if !detail.lens.valid()
            || !detail.projection.valid()
            || !detail.bounces.valid()
            || !detail.sun_radius_deg.is_finite()
            || !(0.0..90.0).contains(&detail.sun_radius_deg)
            || detail.leaf.is_some_and(|leaf| !leaf.valid())
            || detail.steam.is_some_and(|steam| !steam.valid())
            || detail.fog.is_some_and(|fog| !fog.valid())
            || detail.content.iter().flatten().any(|image| !image.valid())
            || detail.content.len() > scene.materials.len()
            || detail
                .content_slots
                .iter()
                .flatten()
                .any(|image| !image.valid())
            || detail.text_slots.iter().flatten().any(|text| !text.valid())
            || detail
                .instances
                .iter()
                .any(|instance| !instance.surface.valid())
        {
            return Err("invalid trace detail".into());
        }
        TraceSettings {
            transmissive_shadows: detail.transmissive_shadows,
            clamp_indirect: detail.clamp_indirect,
            filter_glossy: detail.filter_glossy,
        }
        .check()
        .map_err(|message| TraceError::from(format!("invalid trace detail: {message}")))?;
        let pixels = u64::from(width) * u64::from(height);
        if pixels > u32::MAX as u64 {
            return Err("trace image is too large".into());
        }
        if height > u32::from(u16::MAX) {
            return Err("trace image is taller than 65535 rows".into());
        }
        fits_texture(gpu, width, height)?;
        fits_buffer(gpu, pixels * pfx_gpu::floor::TRACE_BYTES_PER_PIXEL)?;
        if !finite3(scene.camera.origin)
            || !finite3(scene.camera.forward)
            || !finite3(scene.camera.right)
            || !finite3(scene.camera.up)
            || !finite3(scene.sun.direction)
            || !finite3(scene.sun.color)
            || !scene.sun.intensity.is_finite()
        {
            return Err("camera or sun contains invalid parameters".into());
        }
        if !detail.projection.fits(&scene.camera) {
            return Err("an orthographic or equirect camera needs forward, right and up".into());
        }
        let shrink = crate::textured::budget(scene, detail, width, height);
        let mut buffers = SceneBuffers::build_with(
            &scene.triangles,
            &scene.shapes,
            &scene.materials,
            detail,
            emitter_sampling,
            &shrink,
        )?;
        let hardware = crate::rt::wanted(gpu)
            && !buffers.rt.is_empty()
            && buffers.rt.instances.len() <= crate::rt::MAX_INSTANCES;
        let mut motion_at = [0_u32; 3];
        let moving = match buffers.moving.take() {
            Some(moving) => {
                let levels = buffers.levels.as_ref().map(|placing| &placing.levels);
                let (block, movers) = moving.block(levels);
                let offset = if hardware {
                    let mut words = Vec::new();
                    let first_mesh = levels
                        .and_then(|levels| levels.meshes.first())
                        .map_or(0, |mesh| mesh.root);
                    if let Some(levels) = levels {
                        for mesh in &levels.meshes {
                            words.extend(crate::moving::node_words(&mesh.nodes));
                        }
                    }
                    let map = buffers.rt.map.len() as u32;
                    motion_at = [map + words.len() as u32, map, first_mesh];
                    buffers.rt.tail = words;
                    buffers.rt.tail.extend(crate::moving::as_words(&block));
                    buffers.rt.movers = movers;
                    u64::from(motion_at[0]) * 4
                } else {
                    let node = std::mem::size_of::<crate::bvh::Node>();
                    let at = (buffers.nodes.len() / node) as u32;
                    motion_at = [at, 0, 0];
                    buffers
                        .nodes
                        .extend_from_slice(bytemuck::cast_slice(&crate::moving::as_nodes(&block)));
                    u64::from(at) * node as u64
                };
                Some(MovingState { moving, offset })
            }
            None => None,
        };
        for bytes in [
            &buffers.triangles,
            &buffers.uv,
            &buffers.content,
            &buffers.surfaces,
            &buffers.layers,
            &buffers.content_info,
            &buffers.nodes,
            &buffers.shapes,
            &buffers.materials,
            &buffers.emitters,
            &buffers.emitter_cdf,
            &buffers.lights,
        ] {
            fits_buffer(gpu, bytes.len() as u64)?;
        }
        let frame = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace frame"),
            size: std::mem::size_of::<Frame>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let placed = buffers.levels.is_some();
        memory::admit_gpu(
            "the tracer's buffers",
            [
                &buffers.triangles,
                &buffers.uv,
                &buffers.content,
                &buffers.surfaces,
                &buffers.layers,
                &buffers.content_info,
                &buffers.nodes,
                &buffers.shapes,
                &buffers.materials,
            ]
            .iter()
            .map(|data| data.len() as u64)
            .sum::<u64>()
                + pixels * (48 + 3 * 16),
        )?;
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let mut gpu_bytes = 0u64;
        let mut upload = |label: &'static str, data: &mut Vec<u8>, read_write: bool| {
            gpu_bytes += data.len().max(256) as u64;
            let buffer = storage(gpu, label, data, read_write);
            *data = Vec::new();
            buffer
        };
        let tri_buffer = upload("trace triangles", &mut buffers.triangles, false);
        let uv_buffer = upload("trace triangle uv v", &mut buffers.uv, false);
        let content_buffer = upload("trace content", &mut buffers.content, false);
        let surface_buffer = upload("trace instance surfaces", &mut buffers.surfaces, placed);
        let layer_buffer = upload("trace content layers", &mut buffers.layers, false);
        let content_info_buffer = upload("trace content info", &mut buffers.content_info, false);
        let node_buffer = upload(
            "trace nodes",
            &mut buffers.nodes,
            placed || moving.is_some(),
        );
        let shape_buffer = upload("trace shapes", &mut buffers.shapes, false);
        let material_buffer = upload("trace materials", &mut buffers.materials, false);
        let emitter_buffer = upload("trace emitters", &mut buffers.emitters, placed);
        let emitter_cdf_buffer = upload("trace emitter cdf", &mut buffers.emitter_cdf, placed);
        let light_buffer = upload("trace lights", &mut buffers.lights, false);
        gpu_bytes += pixels * (48 + 3 * 16);
        let accum_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace accumulation"),
            size: pixels * 48,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let color = texture(gpu, width, height, "trace color");
        let albedo = texture(gpu, width, height, "trace albedo");
        let normal = texture(gpu, width, height, "trace normal");
        let lit = buffers.emitter_count > 0 || buffers.light_count > 0;
        let probe_parts = crate::probe::Parts {
            buffers: [
                (1, tri_buffer.clone()),
                (2, node_buffer.clone()),
                (3, shape_buffer.clone()),
                (4, material_buffer.clone()),
                (5, uv_buffer.clone()),
                (6, content_buffer.clone()),
                (11, content_info_buffer.clone()),
                (12, surface_buffer.clone()),
                (13, layer_buffer.clone()),
            ],
            lit,
        };
        let rt = hardware.then(|| crate::rt::Rt::build(gpu, &buffers.rt));
        if let Some(rt) = &rt {
            gpu_bytes += rt.bytes;
        }
        buffers.rt = crate::rt::RtGeometry::default();
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("trace shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source(lit, rt.is_some()).into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("trace pipeline"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &pipeline_constants(
                        detail,
                        height,
                        buffers.features,
                        moving.is_some(),
                    ),
                    ..Default::default()
                },
                cache: None,
            });
        let sobol_view =
            sobol_table(gpu, detail.sampler).create_view(&wgpu::TextureViewDescriptor::default());
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: tri_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: node_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: shape_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: material_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: uv_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: content_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: accum_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::TextureView(&color.view),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(&albedo.view),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::TextureView(&normal.view),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: content_info_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: surface_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 13,
                resource: layer_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 19,
                resource: wgpu::BindingResource::TextureView(&sobol_view),
            },
        ];
        if lit {
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: emitter_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: emitter_cdf_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: light_buffer.as_entire_binding(),
                },
            ]);
        }
        if let Some(rt) = &rt {
            entries.retain(|entry| entry.binding != 2);
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 17,
                    resource: rt.tlas.as_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 18,
                    resource: rt.map.as_entire_binding(),
                },
            ]);
        }
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let look = crate::sky::look(&detail.sky_look);
        let sky: Environment =
            match (&scene.sky as &dyn std::any::Any).downcast_ref::<Environment>() {
                Some(environment) => environment.try_clone()?,
                None => scene.sky.clone().into(),
            };
        let halvings = detail.fit_textures.map_or(0, |size| {
            sky_halvings(
                &sky,
                &scene.camera,
                detail.projection,
                [size[0].max(width), size[1].max(height)],
            )
        });
        let sky = fitted(sky, halvings)?;
        let (environment_binding, sky_bytes) =
            environment_binding(gpu, &pipeline, &sky, look, detail.fit_textures.is_some())?;
        drop(sky);
        if let Some(error) = ready(scope.pop()).flatten() {
            return Err(format!(
                "the tracer's buffers and textures need {gpu_bytes} bytes on the GPU: {error}"
            )
            .into());
        }
        gpu_bytes += sky_bytes;
        let base = Frame {
            size: [width, height, 0, 0],
            counts: [buffers.triangle_count, buffers.shape_count, 0, 0],
            origin: vec4(scene.camera.origin, 0.0),
            forward: vec4(scene.camera.forward, 0.0),
            right: vec4(scene.camera.right, 0.0),
            up: vec4(scene.camera.up, 0.0),
            sun_dir: vec4(scene.sun.direction, 0.0),
            sun_color: vec4(scene.sun.color, scene.sun.intensity),
            lens: [
                detail.lens.shift[0],
                detail.lens.shift[1],
                detail.lens.aperture * 0.5,
                detail.lens.focus_distance,
            ],
            window: [width, height, 0, 0],
            columns: [0, motion_at[0], motion_at[1], motion_at[2]],
            fog: detail.fog.map_or([[0.0; 4]; 3], |fog| fog.rows()),
        };
        let mut base = base;
        detail.projection.place(&mut base);
        base.sun_dir[3] = detail.sun_radius_deg.to_radians().cos();
        Ok(Self {
            width,
            height,
            samples: 0,
            projection: detail.projection,
            base,
            frame,
            binding,
            environment_binding,
            look,
            pipeline,
            materials: scene.materials.len() as u32,
            accum: accum_buffer,
            probe_parts,
            pacer: Self::banded_pacer(),
            turns: Turns::default(),
            lanes: Lanes::new(),
            color,
            albedo,
            normal,
            rt,
            placing: buffers.levels,
            surface_buffer: surface_buffer.clone(),
            node_buffer,
            emitter_buffer,
            emitter_cdf_buffer,
            builds: Builds {
                bottom: buffers.bottom_builds,
                top: usize::from(placed),
            },
            gpu_bytes,
            half_sky: detail.fit_textures.is_some(),
            sky_halvings: halvings,
            moving,
            fogged: detail.fog.is_some(),
        })
    }

    pub fn builds(&self) -> Builds {
        self.builds
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.gpu_bytes
    }

    pub fn instance_count(&self) -> usize {
        self.placing
            .as_ref()
            .map_or(0, |placing| placing.levels.instance_count())
    }

    pub fn set_instance_transforms(
        &mut self,
        gpu: &Gpu,
        transforms: &[crate::detail::Transform],
    ) -> Result<(), String> {
        let Some(placing) = self.placing.as_mut() else {
            return if transforms.is_empty() {
                Ok(())
            } else {
                Err("this trace has no instances to move".into())
            };
        };
        let levels = &mut placing.levels;
        if transforms.len() > levels.placed.len() {
            return Err(format!(
                "the trace has {} instances and {} transforms were given",
                levels.placed.len(),
                transforms.len()
            ));
        }
        let poses = transforms
            .iter()
            .map(|&transform| crate::detail::Pose::new(transform))
            .collect::<Result<Vec<_>, _>>()?;
        let moved = |placed: &crate::levels::Placed, transform: &crate::detail::Transform| {
            placed
                .transform
                .iter()
                .flatten()
                .zip(transform.iter().flatten())
                .any(|(a, b)| a.to_bits() != b.to_bits())
        };
        if !levels
            .placed
            .iter()
            .zip(transforms)
            .any(|(placed, transform)| placed.as_ref().is_some_and(|p| moved(p, transform)))
        {
            return Ok(());
        }
        let mut emitters_moved = false;
        let record_size = std::mem::size_of::<GpuSurface>() as u64;
        for ((placed, transform), pose) in levels.placed.iter_mut().zip(transforms).zip(&poses) {
            let Some(placed) = placed else { continue };
            if !moved(placed, transform) {
                continue;
            }
            let mesh = &levels.meshes[placed.mesh];
            placed.transform = *transform;
            placed.inverse = pose.inverse_rows();
            placed.bounds = crate::levels::placed_bounds(pose, mesh.bounds);
            for candidate in &mut placing.candidates {
                if let crate::lights::Source::Triangle {
                    vertices,
                    slot,
                    record,
                } = &mut candidate.source
                    && *record == placed.record
                {
                    let local = &mesh.triangles[(*slot - mesh.first) as usize];
                    *vertices = local.vertices.map(|point| pose.point(point));
                    emitters_moved = true;
                }
            }
            let offset = record_size * u64::from(placed.record);
            let forward = 4 * 16 + 16;
            gpu.queue.write_buffer(
                &self.surface_buffer,
                offset + 4 * 16 + 12,
                bytemuck::bytes_of(&pose.inverse_determinant().to_bits()),
            );
            gpu.queue.write_buffer(
                &self.surface_buffer,
                offset + forward,
                bytemuck::cast_slice(&[rows3(pose.rows()), rows3(placed.inverse)]),
            );
        }
        gpu.queue
            .write_buffer(&self.node_buffer, 0, bytemuck::cast_slice(&levels.top()));
        if emitters_moved {
            let list = build_emitters(&placing.candidates, placing.lights, placing.tree);
            gpu.queue
                .write_buffer(&self.emitter_buffer, 0, bytemuck::cast_slice(&list.entries));
            gpu.queue
                .write_buffer(&self.emitter_cdf_buffer, 0, bytemuck::cast_slice(&list.cdf));
        }
        if let Some(rt) = &mut self.rt {
            let rows: Vec<[f32; 12]> = levels
                .placed
                .iter()
                .flatten()
                .map(|placed| {
                    crate::detail::Pose::new(placed.transform).map_or([0.0; 12], |pose| pose.rows())
                })
                .collect();
            rt.set_transforms(gpu, &rows);
        }
        self.builds.top += 1;
        self.samples = 0;
        Ok(())
    }

    pub fn sky_halvings(&self) -> u32 {
        self.sky_halvings
    }

    pub fn hardware_rays(&self) -> bool {
        self.rt.is_some()
    }

    pub fn moving(&self) -> bool {
        self.moving.is_some()
    }

    pub fn set_motion(
        &mut self,
        gpu: &Gpu,
        transforms: &[Vec<crate::detail::Transform>],
        cameras: &[Camera],
    ) -> Result<(), String> {
        let Some(state) = self.moving.as_mut() else {
            return Err("this trace was staged without motion; stage it again to blur it".into());
        };
        let steps = state.moving.steps;
        let wanted = cameras.windows(2).any(|pair| {
            crate::motion::camera_rows(&pair[0]) != crate::motion::camera_rows(&pair[1])
        });
        match (state.moving.cameras.len(), wanted) {
            (0, true) => {
                return Err("this trace was staged with a still camera; stage it again".into());
            }
            (0, false) => {}
            (count, _) if cameras.len() != count => {
                return Err(format!("this trace takes {count} camera steps"));
            }
            _ => {
                if cameras
                    .iter()
                    .flat_map(|camera| [camera.origin, camera.forward, camera.right, camera.up])
                    .any(|v| !finite3(v))
                {
                    return Err("camera contains invalid parameters".into());
                }
                state.moving.cameras = crate::motion::decompose_steps(
                    &cameras
                        .iter()
                        .map(crate::motion::camera_rows)
                        .collect::<Vec<_>>(),
                );
            }
        }
        let levels = match self.placing.as_mut() {
            Some(placing) => Some(&mut placing.levels),
            None if transforms.iter().all(Vec::is_empty) => None,
            None => return Err("this trace has no instances to move".into()),
        };
        if let Some(levels) = levels {
            if transforms.len() > levels.placed.len() {
                return Err(format!(
                    "the trace has {} instances and {} motions were given",
                    levels.placed.len(),
                    transforms.len()
                ));
            }
            for moved in transforms {
                if !moved.is_empty() && moved.len() != steps {
                    return Err(format!(
                        "a moving instance has {} transforms for {steps} motion steps",
                        moved.len()
                    ));
                }
                for transform in moved {
                    crate::detail::Pose::new(*transform)?;
                }
            }
            for (index, moved) in transforms.iter().enumerate() {
                levels.set_motion(index, moved);
            }
        }
        let levels = self.placing.as_ref().map(|placing| &placing.levels);
        let (block, boxes) = state.moving.block(levels);
        match &mut self.rt {
            Some(rt) => {
                gpu.queue.write_buffer(
                    &rt.map,
                    state.offset,
                    bytemuck::cast_slice(&crate::moving::as_words(&block)),
                );
                if let Some(levels) = levels {
                    let flags: Vec<bool> = levels
                        .placed
                        .iter()
                        .flatten()
                        .map(|placed| placed.moving())
                        .collect();
                    rt.set_movers(gpu, &flags, &boxes);
                }
            }
            None => {
                gpu.queue.write_buffer(
                    &self.node_buffer,
                    state.offset,
                    bytemuck::cast_slice(&crate::moving::as_nodes(&block)),
                );
                if let Some(levels) = levels {
                    gpu.queue.write_buffer(
                        &self.node_buffer,
                        0,
                        bytemuck::cast_slice(&levels.top()),
                    );
                }
            }
        }
        self.builds.top += 1;
        self.samples = 0;
        Ok(())
    }

    fn banded_pacer() -> Pacer {
        let mut pacer = Pacer::default();
        pacer.set_step(BAND_STEP, BAND_STEP);
        pacer.set_target_ms(SAMPLE_TARGET_MS);
        pacer.set_ceiling_ms(SAMPLE_SPAN_MS);
        pacer
    }

    pub fn set_environment(&mut self, gpu: &Gpu, environment: &Environment) -> Result<(), String> {
        self.environment_binding =
            environment_binding(gpu, &self.pipeline, environment, self.look, self.half_sky)?.0;
        self.samples = 0;
        Ok(())
    }

    pub fn set_camera(&mut self, camera: Camera) -> Result<(), String> {
        if !finite3(camera.origin)
            || !finite3(camera.forward)
            || !finite3(camera.right)
            || !finite3(camera.up)
        {
            return Err("camera contains invalid parameters".into());
        }
        if !self.projection.fits(&camera) {
            return Err("an orthographic or equirect camera needs forward, right and up".into());
        }
        self.base.origin = vec4(camera.origin, self.base.origin[3]);
        self.base.forward = vec4(camera.forward, self.base.forward[3]);
        self.base.right = vec4(camera.right, self.base.right[3]);
        self.base.up = vec4(camera.up, self.base.up[3]);
        self.samples = 0;
        Ok(())
    }

    pub fn set_window(&mut self, full: [u32; 2], origin: [u32; 2]) -> Result<(), String> {
        let fits = |k: usize, side: u32| {
            origin[k]
                .checked_add(side)
                .is_some_and(|end| end <= full[k])
        };
        if !fits(0, self.width) || !fits(1, self.height) {
            return Err("the window must lie inside the full frame".into());
        }
        if u64::from(full[0]) * u64::from(full[1]) > u64::from(u32::MAX) {
            return Err("the full frame is too large".into());
        }
        self.base.window = [full[0], full[1], origin[0], origin[1]];
        self.samples = 0;
        Ok(())
    }

    pub fn window(&self) -> ([u32; 2], [u32; 2]) {
        let [width, height, x, y] = self.base.window;
        ([width, height], [x, y])
    }

    pub fn set_lens(&mut self, lens: Lens) -> Result<(), String> {
        if !lens.valid() {
            return Err("invalid lens".into());
        }
        self.base.lens = [
            lens.shift[0],
            lens.shift[1],
            lens.aperture * 0.5,
            lens.focus_distance,
        ];
        self.samples = 0;
        Ok(())
    }

    pub fn set_projection(&mut self, projection: Projection) -> Result<(), String> {
        if !projection.valid() {
            return Err("invalid projection".into());
        }
        let camera = Camera {
            origin: std::array::from_fn(|k| self.base.origin[k]),
            forward: std::array::from_fn(|k| self.base.forward[k]),
            right: std::array::from_fn(|k| self.base.right[k]),
            up: std::array::from_fn(|k| self.base.up[k]),
        };
        if !projection.fits(&camera) {
            return Err("an orthographic or equirect camera needs forward, right and up".into());
        }
        projection.place(&mut self.base);
        self.projection = projection;
        self.samples = 0;
        Ok(())
    }

    pub fn set_fog(&mut self, fog: crate::fog::Fog) -> Result<(), String> {
        if !self.fogged {
            return Err("the trace was built without fog".into());
        }
        if !fog.valid() {
            return Err("invalid trace fog".into());
        }
        self.base.fog = fog.rows();
        self.samples = 0;
        Ok(())
    }

    pub fn set_sun_radius(&mut self, radius_deg: f32) -> Result<(), String> {
        if !radius_deg.is_finite() || !(0.0..90.0).contains(&radius_deg) {
            return Err("invalid sun radius".into());
        }
        self.base.sun_dir[3] = radius_deg.to_radians().cos();
        self.samples = 0;
        Ok(())
    }

    pub fn set_sun(&mut self, sun: Sun) -> Result<(), String> {
        if !finite3(sun.direction) || !finite3(sun.color) || !sun.intensity.is_finite() {
            return Err("sun contains invalid parameters".into());
        }
        self.base.sun_dir = vec4(sun.direction, self.base.sun_dir[3]);
        self.base.sun_color = vec4(sun.color, sun.intensity);
        self.samples = 0;
        Ok(())
    }

    pub fn set_daylight(
        &mut self,
        gpu: &Gpu,
        daylight: Daylight,
        turbidity: f32,
        ground_albedo: [f32; 3],
    ) -> Result<(), String> {
        let environment = Environment::Analytic(crate::sky::AnalyticSky::new(
            daylight,
            REFERENCE_HOUR,
            turbidity,
            ground_albedo,
        ));
        self.set_environment(gpu, &environment)?;
        let sun = Sun::from_daylight(daylight);
        self.base.sun_dir = vec4(sun.direction, self.base.sun_dir[3]);
        self.base.sun_color = vec4(sun.color, sun.intensity);
        Ok(())
    }

    pub fn sample(&mut self, gpu: &Gpu, count: u32, seed: u32) -> Result<(), String> {
        let mut pacer = std::mem::take(&mut self.pacer);
        let mut turns = std::mem::take(&mut self.turns);
        let result = self.sample_sliced(gpu, count, seed, &mut pacer, |ms| turns.add(ms));
        self.pacer = pacer;
        self.turns = turns;
        result.map(|_| ())
    }

    pub fn sample_paced<T: Turned>(
        &mut self,
        gpu: &Gpu,
        count: u32,
        seed: u32,
        pacer: &mut Pacer,
        between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        let (rows, columns) = pacer.step();
        let (target, ceiling) = (pacer.target_ms(), pacer.ceiling_ms());
        pacer.set_step(BAND_STEP, BAND_STEP);
        pacer.set_target_ms(SAMPLE_TARGET_MS);
        pacer.set_ceiling_ms(SAMPLE_SPAN_MS);
        let result = self.sample_sliced(gpu, count, seed, pacer, between);
        pacer.set_step(rows, columns);
        pacer.set_target_ms(target);
        pacer.set_ceiling_ms(ceiling);
        result
    }

    pub fn lanes(&self) -> u32 {
        self.lanes.width
    }

    fn sample_sliced<T: Turned>(
        &mut self,
        gpu: &Gpu,
        count: u32,
        seed: u32,
        pacer: &mut Pacer,
        between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        self.samples
            .checked_add(count)
            .ok_or("sample count overflow")?;
        let step = pacer.step();
        let result = self.sample_passes(gpu, count, seed, pacer, between);
        pacer.set_step(step.0, step.1);
        result
    }

    fn sample_passes<T: Turned>(
        &mut self,
        gpu: &Gpu,
        count: u32,
        seed: u32,
        pacer: &mut Pacer,
        mut between: impl FnMut(f64) -> T,
    ) -> Result<Stats, String> {
        let mut stats = Stats::default();
        let fixed = pacer.fixed_units();
        let blocks = self.height.div_ceil(BAND_STEP);
        let columns = self.width.div_ceil(8);
        let per_run = (u32::MAX / (blocks * BAND_STEP)).max(1);
        let mut done = 0;
        while done < count {
            if fixed.is_some() || (self.lanes.width >= LANES && self.lanes.pieces == 1) {
                if fixed.is_none() {
                    pacer.set_step(BAND_STEP, BAND_STEP);
                    if pacer.learned_units(SAMPLE_LABEL).is_none() {
                        pacer.teach(SAMPLE_LABEL, self.lanes.strips(blocks) * BAND_STEP);
                    }
                }
                let whole = fixed.is_some()
                    || pacer
                        .learned_units(SAMPLE_LABEL)
                        .is_some_and(|units| units >= blocks * BAND_STEP);
                let samples = if whole {
                    (count - done).min(per_run)
                } else {
                    1
                };
                let pass = Pass {
                    lanes: (0, LANES),
                    per_block: BAND_STEP,
                    columns: (0, columns),
                };
                let spread =
                    self.pass(gpu, seed, samples, pass, pacer, &mut between, &mut stats)?;
                if fixed.is_none() {
                    self.lanes.learn_dense(spread, columns);
                }
                self.samples += samples;
                done += samples;
                continue;
            }
            if self.lanes.width >= LANES {
                let mut at = 0;
                while at < columns {
                    let pieces = if self.lanes.width < LANES {
                        most_pieces(columns)
                    } else {
                        self.lanes.pieces
                    };
                    let span = columns.div_ceil(pieces);
                    let end = (at + span).min(columns);
                    pacer.set_step(1, 1);
                    pacer.set_fixed_units(Some(self.lanes.strips(blocks)));
                    let pass = Pass {
                        lanes: (0, LANES),
                        per_block: 1,
                        columns: (at, end),
                    };
                    let spread = self.pass(gpu, seed, 1, pass, pacer, &mut between, &mut stats);
                    pacer.set_fixed_units(fixed);
                    let spread = spread?;
                    if end - at == span && self.lanes.width >= LANES {
                        self.lanes.learn_pieces(spread.longest, blocks, columns);
                    }
                    at = end;
                }
                self.samples += 1;
                done += 1;
                continue;
            }
            let mut lane = 0;
            while lane < LANES {
                let configured = self.lanes.width;
                let width = floor_power_of_two(configured.min(LANES - lane));
                pacer.set_step(1, 1);
                pacer.set_fixed_units(Some(self.lanes.strips(blocks)));
                let pass = Pass {
                    lanes: (lane, lane + width),
                    per_block: 1,
                    columns: (0, columns),
                };
                let spread = self.pass(gpu, seed, 1, pass, pacer, &mut between, &mut stats);
                pacer.set_fixed_units(fixed);
                let spread = spread?;
                if width == configured {
                    self.lanes.learn(spread.longest, blocks);
                }
                lane += width;
            }
            self.samples += 1;
            done += 1;
        }
        Ok(stats)
    }

    #[allow(clippy::too_many_arguments)]
    fn pass<T: Turned>(
        &self,
        gpu: &Gpu,
        seed: u32,
        samples: u32,
        pass: Pass,
        pacer: &mut Pacer,
        between: &mut impl FnMut(f64) -> T,
        stats: &mut Stats,
    ) -> Result<Spread, String> {
        let blocks = self.height.div_ceil(BAND_STEP);
        let accumulated = self.samples;
        let work = Work::Bands {
            width: self.width,
            height: blocks * pass.per_block * samples,
        };
        let mut sizes = Vec::new();
        let run = pacer.run(
            SAMPLE_LABEL,
            &gpu.device,
            &gpu.queue,
            work,
            |encoder, slice| {
                if let Slice::Pixels { height, .. } = slice {
                    sizes.push(height);
                }
                self.encode(gpu, encoder, slice, accumulated, seed, pass)
            },
            between,
        )?;
        let spread = Spread {
            longest: run.longest_ms(),
            single: sizes
                .iter()
                .zip(&run.milliseconds)
                .filter(|(units, _)| **units <= pass.per_block)
                .fold(0.0, |longest, (_, ms)| ms.max(longest)),
        };
        stats.milliseconds.extend(run.milliseconds);
        stats.timings.extend(run.timings);
        stats.wall_ms += run.wall_ms;
        Ok(spread)
    }

    fn encode(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        slice: Slice,
        accumulated: u32,
        seed: u32,
        pass: Pass,
    ) {
        let Slice::Pixels { y, height, .. } = slice else {
            return;
        };
        let blocks = self.height.div_ceil(BAND_STEP);
        let (first, end) = pass.lanes;
        let share = (end - first) / pass.per_block;
        for piece in pieces(blocks, pass.per_block, y, height) {
            self.dispatch(
                gpu,
                encoder,
                piece.order,
                piece.blocks,
                (first + piece.from * share, first + piece.to * share),
                pass.columns,
                accumulated + piece.sample,
                piece.samples,
                seed,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        order: u32,
        blocks: u32,
        lanes: (u32, u32),
        columns: (u32, u32),
        accumulated: u32,
        samples: u32,
        seed: u32,
    ) {
        let mut frame = self.base;
        frame.columns[0] = columns.0;
        frame.size[2] = accumulated;
        frame.size[3] = samples;
        frame.counts[2] = seed;
        frame.counts[3] = order | (lanes.0 << 16) | (lanes.1 << 23);
        let staging = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("trace frame slice"),
                contents: bytemuck::bytes_of(&frame),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            &self.frame,
            0,
            std::mem::size_of::<Frame>() as u64,
        );
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("trace"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_bind_group(1, &self.environment_binding, &[]);
        pass.dispatch_workgroups(columns.1 - columns.0, blocks, 1);
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }
    pub fn readback_color(&self, gpu: &Gpu) -> Result<Vec<u8>, String> {
        gpu.readback_bytes(&self.color)
    }

    pub fn readback(&self, gpu: &Gpu) -> Result<Output, String> {
        Ok(Output {
            color: self.readback_color(gpu)?,
            albedo: gpu.readback_bytes(&self.albedo)?,
            normal: gpu.readback_bytes(&self.normal)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detail::TriangleDetail;
    use crate::sobol::Sampler;

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn content_layer_matches_cpu_composite_and_clips() {
        use pfx_materials::{Blend, Shaded};

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let triangle = TriangleDetail {
            vertices: [[-2.0, -2.0, -2.0], [2.0, -2.0, -2.0], [0.0, 2.0, -2.0]],
            normals: [[0.0, 0.0, 1.0]; 3],
            uvs: [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
            material: 0,
            uvs1: [[0.0; 2]; 3],
        };
        let bytes = [31u8, 10, 5, 128];
        let texel = bytes.map(|byte| f32::from(byte) / 255.0);
        let mut material = Material {
            base: [0.8, 0.6, 0.4],
            roughness: 0.9,
            emission: [0.2, 0.1, 0.0],
            ..Material::default()
        };
        let scene = |material| Scene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![material],
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            camera: Camera {
                origin: [0.0; 3],
                forward: [0.0, 0.0, -1.0],
                right: [0.7, 0.0, 0.0],
                up: [0.0, 0.7, 0.0],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        };
        let mut detail = Detail {
            instances: vec![crate::detail::Instance {
                triangles: vec![triangle].into(),
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                surface: InstanceSurface::default(),
                shadow_only: false,
                two_sided: true,
                motion: None,
            }],
            content_slots: vec![Some(crate::detail::ContentImage {
                width: 1,
                height: 1,
                texels: vec![bytes],
                srgb: false,
            })],
            ..Detail::default()
        };
        let channel = |bytes: &[u8], x: usize, y: usize, c: usize| {
            let at = (y * 32 + x) * 16 + c * 4;
            f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
        };
        for blend in [Blend::Over, Blend::Multiply, Blend::Emit] {
            let layer = ContentLayer {
                slot: 0,
                blend,
                ink_roughness: 0.55,
                emboss: 0.0,
                strength: 1.2,
            };
            material.content_layer = layer;
            let expected = layer.composite(&Shaded::from_material(&material), texel);
            let mut trace = Trace::new_detailed(&gpu, &scene(material), &detail, 32, 32).unwrap();
            trace.sample(&gpu, 1, 29).unwrap();
            let output = trace.readback(&gpu).unwrap();
            let mut repeated =
                Trace::new_detailed(&gpu, &scene(material), &detail, 32, 32).unwrap();
            repeated.sample(&gpu, 1, 29).unwrap();
            assert_eq!(output, repeated.readback(&gpu).unwrap());
            for c in 0..3 {
                assert!((channel(&output.albedo, 16, 16, c) - expected.base[c]).abs() < 1e-5);
                if blend == Blend::Emit {
                    assert!(
                        (channel(&output.color, 16, 16, c) - expected.emission[c]).abs() < 1e-3
                    );
                }
            }
        }
        detail.instances[0].surface.clip = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]];
        let mut trace = Trace::new_detailed(&gpu, &scene(material), &detail, 32, 32).unwrap();
        trace.sample(&gpu, 1, 29).unwrap();
        let output = trace.readback(&gpu).unwrap();
        assert!(channel(&output.albedo, 8, 8, 0) < 1e-6);
        assert!(channel(&output.albedo, 24, 24, 0) < 1e-6);
        assert!(channel(&output.albedo, 8, 24, 0) > 0.1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn detailed_lens_instance_and_determinism() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = Scene {
            triangles: Vec::new(),
            shapes: Vec::new(),
            materials: vec![Material {
                emission: [4.0, 1.0, 0.5],
                ..Material::default()
            }],
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            camera: Camera {
                origin: [0.0; 3],
                forward: [0.0, 0.0, -1.0],
                right: [0.7, 0.0, 0.0],
                up: [0.0, 0.7, 0.0],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        };
        let triangle = TriangleDetail {
            vertices: [[-0.3, -0.3, -2.0], [0.3, -0.3, -2.0], [0.0, 0.3, -2.0]],
            normals: [[-0.3, 0.0, 0.95], [0.3, 0.0, 0.95], [0.0, 0.3, 0.95]],
            uvs: [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
            uvs1: [[0.0; 2]; 3],
            material: 0,
        };
        let baseline = Detail {
            triangles: vec![triangle],
            ..Detail::default()
        };
        let shifted = Detail {
            lens: Lens {
                shift: [0.25, 0.0],
                aperture: 0.12,
                focus_distance: 1.0,
            },
            ..baseline.clone()
        };
        let transformed = Detail {
            triangles: Vec::new(),
            instances: vec![crate::detail::Instance {
                triangles: vec![triangle].into(),
                surface: InstanceSurface::default(),
                shadow_only: false,
                two_sided: true,
                motion: None,
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.1, 0.0, 0.0, 1.0],
                ],
            }],
            ..Detail::default()
        };
        let textured = Detail {
            content: vec![Some(crate::detail::ContentImage {
                width: 2,
                height: 1,
                texels: vec![[0; 4], [255, 0, 0, 255]],
                srgb: false,
            })],
            ..baseline.clone()
        };
        let legacy_scene = Scene {
            triangles: vec![triangle.triangle()],
            shapes: Vec::new(),
            materials: scene.materials.clone(),
            sky: scene.sky.clone(),
            camera: scene.camera,
            sun: scene.sun,
        };
        let mut legacy = Trace::new(&gpu, &legacy_scene, 32, 32).unwrap();
        let mut pinhole = Trace::new_detailed(&gpu, &scene, &baseline, 32, 32).unwrap();
        let mut first = Trace::new_detailed(&gpu, &scene, &shifted, 32, 32).unwrap();
        let mut second = Trace::new_detailed(&gpu, &scene, &shifted, 32, 32).unwrap();
        let mut mover = Trace::new_detailed(&gpu, &scene, &transformed, 32, 32).unwrap();
        let mut content = Trace::new_detailed(&gpu, &scene, &textured, 32, 32).unwrap();
        for trace in [
            &mut legacy,
            &mut pinhole,
            &mut first,
            &mut second,
            &mut mover,
            &mut content,
        ] {
            trace.sample(&gpu, 8, 29).unwrap();
        }
        let legacy = legacy.readback(&gpu).unwrap();
        let pinhole = pinhole.readback(&gpu).unwrap();
        let first = first.readback(&gpu).unwrap();
        let second = second.readback(&gpu).unwrap();
        let mover = mover.readback(&gpu).unwrap();
        let content = content.readback(&gpu).unwrap();
        assert_eq!(legacy.color, pinhole.color);
        assert_eq!(first, second);
        assert_ne!(first.color, pinhole.color);
        assert_ne!(mover.color, pinhole.color);
        let red = |bytes: &[u8], x: usize| {
            f32::from_le_bytes(
                bytes[(16 * 32 + x) * 16..(16 * 32 + x) * 16 + 4]
                    .try_into()
                    .unwrap(),
            )
        };
        assert!(red(&content.albedo, 17) > red(&content.albedo, 15));
        assert!(red(&content.normal, 17) > red(&content.normal, 15));
    }
    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn retarget_matches_fresh_trace() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut scene = Scene {
            triangles: Vec::new(),
            shapes: vec![Shape::Ellipsoid {
                center: [0.0, 0.0, -3.0],
                radii: [1.0; 3],
                material: 0,
            }],
            materials: vec![Material::default()],
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.4, 0.5, 0.6, 1.0]],
            },
            camera: Camera {
                origin: [0.0; 3],
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
        let mut retargeted = Trace::new(&gpu, &scene, 16, 16).unwrap();
        retargeted.sample(&gpu, 2, 19).unwrap();
        scene.camera = Camera {
            origin: [0.4, 0.2, 0.0],
            forward: [-0.1, -0.05, -1.0],
            right: [1.0, 0.0, -0.1],
            up: [0.0, 1.0, -0.05],
        };
        retargeted.set_camera(scene.camera).unwrap();
        assert_eq!(retargeted.samples(), 0);
        let mut fresh = Trace::new(&gpu, &scene, 16, 16).unwrap();
        retargeted.sample(&gpu, 4, 29).unwrap();
        fresh.sample(&gpu, 4, 29).unwrap();
        assert_eq!(
            retargeted.readback(&gpu).unwrap(),
            fresh.readback(&gpu).unwrap()
        );
    }
    #[test]
    fn the_tracer_has_a_branch_for_every_layer_kind_the_cpu_evaluates() {
        let source = include_str!("trace.wgsl");
        for kind in pfx_materials::NoiseKind::ALL {
            if kind == pfx_materials::NoiseKind::None {
                continue;
            }
            let branch = format!("kind=={}u", kind.id());
            assert!(
                source.contains(&format!("({branch})")) || source.contains(&format!("({branch} ")),
                "{kind:?} draws plain traced: no `{branch}` in the tracer"
            );
        }
    }
    #[test]
    fn shader_assembles() {
        let lit = shader_source(true, false);
        let dark = shader_source(false, false);
        assert!(lit.contains(pfx_materials::NOISE));
        assert!(lit.contains("@binding(16)"));
        assert!(!dark.contains("@binding(14)"));
        let baked = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            pfx_materials::BRDF,
            pfx_materials::LOBES,
            pfx_materials::WET,
            pfx_materials::CONTENT,
            ENVIRONMENT_WGSL.as_str(),
            include_str!("trace.wgsl"),
            crate::TEXTURED_WGSL
        );
        let noise = pfx_materials::NOISE;
        let pavement = noise[noise.find("fn voronoi_point2(").unwrap()..].trim_end();
        assert!(include_str!("trace.wgsl").contains(pavement));
        for source in [lit, dark, baked] {
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
    fn the_shader_budget_defaults_to_the_reference_budget() {
        let source = include_str!("trace.wgsl");
        for (name, value) in bounce_limits(Bounces::REFERENCE) {
            let line = format!("override {name}: u32 = {}u;", value as u32);
            assert!(source.contains(&line), "{line}");
        }
    }
    #[test]
    fn the_firefly_settings_reach_the_shader_only_when_on() {
        let source = include_str!("trace.wgsl");
        assert!(source.contains("override CLAMP_INDIRECT: f32 = 0.0;"));
        assert!(source.contains("override FILTER_GLOSSY: f32 = 0.0;"));
        let named = |detail: &Detail| -> Vec<(&'static str, f64)> {
            pipeline_constants(detail, 64, Features::of(&[]), false)
                .into_iter()
                .filter(|(name, _)| ["CLAMP_INDIRECT", "FILTER_GLOSSY"].contains(name))
                .collect()
        };
        assert!(named(&Detail::default()).is_empty());
        let on = Detail {
            clamp_indirect: 8.0,
            filter_glossy: 0.15,
            ..Detail::default()
        };
        assert_eq!(
            named(&on),
            [
                ("CLAMP_INDIRECT", 8.0),
                ("FILTER_GLOSSY", f64::from(0.15f32))
            ]
        );
    }
    #[test]
    fn a_scene_skips_only_the_material_features_none_of_its_materials_use() {
        let off = |materials: &[Material]| -> Vec<&'static str> {
            let buffers = SceneBuffers::build(&[], &[], materials, &Detail::default()).unwrap();
            pipeline_constants(&Detail::default(), 64, buffers.features, false)
                .into_iter()
                .filter(|(name, value)| name.starts_with("TRACE_") && *value == 0.0)
                .map(|(name, _)| name)
                .collect()
        };
        let plain = Material::default();
        assert_eq!(
            off(&[plain]),
            [
                "TRACE_LOBES",
                "TRACE_MAPS",
                "TRACE_WET",
                "TRACE_THIN",
                "TRACE_ALPHA",
                "TRACE_FLAKES"
            ]
        );
        let mut fabric = plain;
        fabric.fabric.color = [0.4, 0.2, 0.1];
        let mut brushed = plain;
        brushed.anisotropy.strength = -0.5;
        let mut skin = plain;
        skin.scatter.amount = 0.3;
        for lobed in [fabric, brushed, skin] {
            assert_eq!(
                off(&[plain, lobed]),
                [
                    "TRACE_MAPS",
                    "TRACE_WET",
                    "TRACE_THIN",
                    "TRACE_ALPHA",
                    "TRACE_FLAKES"
                ]
            );
        }
        let mut wet = plain;
        wet.wet.amount = 0.5;
        let mut puddled = plain;
        puddled.wet.puddles = 0.5;
        for soaked in [wet, puddled] {
            assert_eq!(
                off(&[soaked, plain]),
                [
                    "TRACE_LOBES",
                    "TRACE_MAPS",
                    "TRACE_THIN",
                    "TRACE_ALPHA",
                    "TRACE_FLAKES"
                ]
            );
        }
        let lens = Material {
            transmission: 1.0,
            thin_walled: true,
            ..plain
        };
        let solid = Material {
            thin_walled: false,
            ..lens
        };
        let opaque = Material {
            transmission: 0.0,
            ..lens
        };
        assert_eq!(
            off(&[lens]),
            [
                "TRACE_LOBES",
                "TRACE_MAPS",
                "TRACE_WET",
                "TRACE_ALPHA",
                "TRACE_FLAKES"
            ]
        );
        for mode in [
            pfx_materials::AlphaMode::Mask,
            pfx_materials::AlphaMode::Blend,
        ] {
            let decal = Material {
                alpha_mode: mode,
                ..plain
            };
            assert_eq!(
                off(&[plain, decal]),
                [
                    "TRACE_LOBES",
                    "TRACE_MAPS",
                    "TRACE_WET",
                    "TRACE_THIN",
                    "TRACE_FLAKES"
                ]
            );
        }
        let paint = Material {
            clearcoat: 1.0,
            flakes: pfx_materials::Flakes {
                amount: 0.3,
                ..pfx_materials::Flakes::default()
            },
            ..plain
        };
        assert_eq!(
            off(&[paint]),
            [
                "TRACE_LOBES",
                "TRACE_MAPS",
                "TRACE_WET",
                "TRACE_THIN",
                "TRACE_ALPHA"
            ]
        );
        for unused in [solid, opaque] {
            assert_eq!(
                off(&[unused]),
                [
                    "TRACE_LOBES",
                    "TRACE_MAPS",
                    "TRACE_WET",
                    "TRACE_THIN",
                    "TRACE_ALPHA",
                    "TRACE_FLAKES"
                ]
            );
        }
        let source = include_str!("trace.wgsl");
        for name in [
            "TRACE_LOBES",
            "TRACE_MAPS",
            "TRACE_WET",
            "TRACE_THIN",
            "TRACE_ALPHA",
            "TRACE_FLAKES",
        ] {
            assert!(source.contains(&format!("override {name}: bool = true;")));
        }
    }
    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn shared_noise_matches_cpu_material() {
        use pfx_materials::{At, NoiseKind, NoiseLayer, resolve};

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut material = Material {
            base: [0.5, 0.4, 0.3],
            ..Material::default()
        };
        material.layers[0] = NoiseLayer::new(NoiseKind::Value, 0.0, 0.7, 0);
        let expected = resolve(&material, 0.0, &At::default()).material.base;
        let scene = Scene {
            triangles: vec![Triangle {
                vertices: [[-4.0, -4.0, -2.0], [4.0, -4.0, -2.0], [0.0, 4.0, -2.0]],
                material: 0,
            }],
            shapes: Vec::new(),
            materials: vec![material],
            sky: Sky {
                width: 1,
                height: 1,
                texels: vec![[0.0; 4]],
            },
            camera: Camera {
                origin: [0.0; 3],
                forward: [0.0, 0.0, -1.0],
                right: [0.1, 0.0, 0.0],
                up: [0.0, 0.1, 0.0],
            },
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                color: [0.0; 3],
                intensity: 0.0,
            },
        };
        let mut trace = Trace::new(&gpu, &scene, 4, 4).unwrap();
        trace.sample(&gpu, 1, 17).unwrap();
        let bytes = trace.readback(&gpu).unwrap().albedo;
        for (channel, target) in expected.into_iter().enumerate() {
            let at = (2 * 4 + 2) * 16 + channel * 4;
            let actual = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            assert!((actual - target).abs() < 1e-5, "{actual} != {target}");
        }
    }
    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn furnace_and_determinism() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut scene = Scene {
            triangles: Vec::new(),
            shapes: vec![Shape::Ellipsoid {
                center: [0.0, 0.0, -3.0],
                radii: [1.0; 3],
                material: 0,
            }],
            materials: vec![Material {
                base: [1.0; 3],
                specular: 0.0,
                roughness: 1.0,
                ..Material::default()
            }],
            sky: Sky {
                width: 4,
                height: 2,
                texels: vec![[1.0; 4]; 8],
            },
            camera: Camera {
                origin: [0.0; 3],
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
        let mut first = Trace::new(&gpu, &scene, 16, 16).unwrap();
        let mut second = Trace::new(&gpu, &scene, 16, 16).unwrap();
        for pass in 0..32 {
            first.sample(&gpu, 4, 73 + pass).unwrap();
            second.sample(&gpu, 4, 73 + pass).unwrap();
        }
        let a = first.readback(&gpu).unwrap();
        let b = second.readback(&gpu).unwrap();
        assert_eq!(a, b);
        let mid = (8 * 16 + 8) * 16;
        let channels: Vec<f32> = a.color[mid..mid + 12]
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        for value in channels {
            assert!((value - 1.0).abs() < 0.12, "{value}");
        }
        let feature: Vec<f32> = a.albedo[mid..mid + 16]
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert!(feature[..3].iter().all(|value| *value > 0.95));
        let surface: Vec<f32> = a.normal[mid..mid + 16]
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert!(surface[2] > 0.8);
        let mut large = Trace::new(&gpu, &scene, 1920, 1080).unwrap();
        let start = std::time::Instant::now();
        large.sample(&gpu, 1, 4).unwrap();
        let _ = large.readback(&gpu).unwrap();
        eprintln!(
            "trace 1920x1080: {:.3} ms/sample",
            start.elapsed().as_secs_f64() * 1000.0
        );
        scene.camera.right = [0.0; 3];
        scene.camera.up = [0.0; 3];
        let cases = [
            vec![Shape::RoundedBox {
                center: [0.0, 0.0, -3.0],
                half: [1.0; 3],
                radius: 0.2,
                material: 0,
            }],
            vec![Shape::RoundCone {
                a: [0.0, -1.0, -3.0],
                b: [0.0, 1.0, -3.0],
                radius_a: 0.5,
                radius_b: 0.3,
                material: 0,
            }],
            vec![Shape::Capsule {
                a: [0.0, -1.0, -3.0],
                b: [0.0, 1.0, -3.0],
                radius: 0.5,
                material: 0,
            }],
            vec![Shape::Ellipsoid {
                center: [0.0, 0.0, -3.0],
                radii: [1.0; 3],
                material: 0,
            }],
            vec![
                Shape::Ellipsoid {
                    center: [-0.3, 0.0, -3.0],
                    radii: [0.7; 3],
                    material: 0,
                },
                Shape::Ellipsoid {
                    center: [0.3, 0.0, -3.0],
                    radii: [0.7; 3],
                    material: 0,
                },
                Shape::SmoothUnion {
                    left: 0,
                    right: 1,
                    radius: 0.2,
                    material: 0,
                },
            ],
        ];
        for shapes in cases {
            scene.shapes = shapes;
            let mut trace = Trace::new(&gpu, &scene, 1, 1).unwrap();
            trace.sample(&gpu, 1, 9).unwrap();
            let Output { albedo, normal, .. } = trace.readback(&gpu).unwrap();
            let opacity = f32::from_le_bytes(albedo[12..16].try_into().unwrap());
            let front = f32::from_le_bytes(normal[8..12].try_into().unwrap());
            assert!(opacity > 0.99 && front > 0.8);
        }
        scene.triangles = vec![Triangle {
            vertices: [[-1.0, -1.0, -2.0], [1.0, -1.0, -2.0], [0.0, 1.0, -2.0]],
            material: 1,
        }];
        scene.materials.push(Material {
            base: [1.0, 0.0, 0.0],
            ..Material::default()
        });
        let mut trace = Trace::new(&gpu, &scene, 1, 1).unwrap();
        trace.sample(&gpu, 1, 9).unwrap();
        let Output { albedo, .. } = trace.readback(&gpu).unwrap();
        let red = f32::from_le_bytes(albedo[0..4].try_into().unwrap());
        let green = f32::from_le_bytes(albedo[4..8].try_into().unwrap());
        assert!(red > 0.99 && green < 0.01);
    }

    fn reference_hit(o: [f32; 3], d: [f32; 3]) -> Option<(f32, [f32; 3])> {
        let center = [0.0, 1.0, -3.0];
        let oc = crate::math::sub(o, center);
        let b = crate::math::dot(oc, d);
        let c = crate::math::dot(oc, oc) - 1.0;
        let discriminant = b * b - c;
        let sphere = if discriminant >= 0.0 {
            let t = -b - discriminant.sqrt();
            let t = if t > 0.0001 {
                t
            } else {
                -b + discriminant.sqrt()
            };
            if t > 0.0001 {
                let point = crate::math::add(o, crate::math::scale(d, t));
                Some((t, crate::math::normalize(crate::math::sub(point, center))))
            } else {
                None
            }
        } else {
            None
        };
        let plane = if d[1].abs() > 1e-6 {
            let t = -o[1] / d[1];
            let p = crate::math::add(o, crate::math::scale(d, t));
            if t > 0.0001 && p[0].abs() < 8.0 && (p[2] + 3.0).abs() < 8.0 {
                Some((t, [0.0, 1.0, 0.0]))
            } else {
                None
            }
        } else {
            None
        };
        match (sphere, plane) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            _ => None,
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn traced_and_live_sky_lighting_agree() {
        use crate::math::{add, cross, dot, normalize, scale, spawn_point};
        use pfx_materials::{Shaded, eval};
        use std::f32::consts::{PI, TAU};

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let daylight = Daylight {
            hour: 16.0,
            day: 172.0,
            latitude: 45.0,
            heading: 180.0,
        };
        let model = crate::sky::AnalyticSky::new(daylight, REFERENCE_HOUR, 2.5, [0.2, 0.25, 0.2]);
        let material = Material {
            base: [0.1; 3],
            roughness: 1.0,
            specular: 0.0,
            ..Material::default()
        };
        let camera = Camera {
            origin: [0.0, 1.8, 1.0],
            forward: normalize([0.0, -0.14, -1.0]),
            right: [0.75, 0.0, 0.0],
            up: [0.0, 0.75, -0.105],
        };
        let scene = Scene {
            triangles: vec![
                Triangle {
                    vertices: [[-8.0, 0.0, -11.0], [8.0, 0.0, -11.0], [8.0, 0.0, 5.0]],
                    material: 0,
                },
                Triangle {
                    vertices: [[-8.0, 0.0, -11.0], [8.0, 0.0, 5.0], [-8.0, 0.0, 5.0]],
                    material: 0,
                },
            ],
            shapes: vec![Shape::Ellipsoid {
                center: [0.0, 1.0, -3.0],
                radii: [1.0; 3],
                material: 0,
            }],
            materials: vec![material],
            sky: Environment::Analytic(model),
            camera,
            sun: Sun::from_daylight(daylight),
        };
        let pcg = Detail {
            sampler: Sampler::Pcg,
            ..Detail::default()
        };
        let mut trace = Trace::new_detailed(&gpu, &scene, &pcg, 8, 8).unwrap();
        for pass in 0..64 {
            trace.sample(&gpu, 32, pass).unwrap();
        }
        let output = trace.readback(&gpu).unwrap();
        let shaded = Shaded::from_material(&material);
        let mut live_sum = [0.0_f64; 3];
        let mut trace_sum = [0.0_f64; 3];
        let mut pixels = 0;
        for y in 0..8 {
            for x in 0..8 {
                let ndc_x = (x as f32 + 0.5) / 8.0 * 2.0 - 1.0;
                let ndc_y = (y as f32 + 0.5) / 8.0 * 2.0 - 1.0;
                let ray = normalize(add(
                    add(camera.forward, scale(camera.right, ndc_x)),
                    scale(camera.up, -ndc_y),
                ));
                let Some((distance, n)) = reference_hit(camera.origin, ray) else {
                    continue;
                };
                let point = add(camera.origin, scale(ray, distance));
                let axis = if n[1].abs() > 0.9 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                let tangent = normalize(cross(axis, n));
                let bitangent = cross(n, tangent);
                let local = |d: [f32; 3]| [dot(d, tangent), dot(d, bitangent), dot(d, n)];
                let wo = local(scale(ray, -1.0));
                let mut value = [0.0_f64; 3];
                for row in 0..64 {
                    let theta = PI * (row as f32 + 0.5) / 64.0;
                    for col in 0..128 {
                        let phi = TAU * (col as f32 + 0.5) / 128.0;
                        let direction = [
                            theta.sin() * phi.cos(),
                            theta.cos(),
                            theta.sin() * phi.sin(),
                        ];
                        let wi = local(direction);
                        if wi[2] <= 0.0 {
                            continue;
                        }
                        let origin = spawn_point(point, n, camera.origin, distance);
                        if reference_hit(origin, direction).is_some() {
                            continue;
                        }
                        let brdf = eval(&shaded, wo, wi);
                        let sky = model.diffuse(direction);
                        let weight = wi[2] * theta.sin() * PI * TAU / (64 * 128) as f32;
                        for channel in 0..3 {
                            value[channel] += (brdf[channel] * sky[channel] * weight) as f64;
                        }
                    }
                }
                let sun = scene.sun.direction;
                let wi = local(sun);
                if wi[2] > 0.0
                    && reference_hit(spawn_point(point, n, camera.origin, distance), sun).is_none()
                {
                    let brdf = eval(&shaded, wo, wi);
                    for channel in 0..3 {
                        value[channel] += (brdf[channel]
                            * wi[2]
                            * scene.sun.color[channel]
                            * scene.sun.intensity) as f64;
                    }
                }
                let offset = (y * 8 + x) * 16;
                let coverage =
                    f32::from_le_bytes(output.albedo[offset + 12..offset + 16].try_into().unwrap());
                if coverage < 0.98 {
                    continue;
                }
                for channel in 0..3 {
                    live_sum[channel] += value[channel];
                    trace_sum[channel] += f32::from_le_bytes(
                        output.color[offset + channel * 4..offset + channel * 4 + 4]
                            .try_into()
                            .unwrap(),
                    ) as f64;
                }
                pixels += 1;
            }
        }
        assert!(pixels >= 16, "{pixels}");
        let live_mean = live_sum.map(|v| v / pixels as f64);
        let trace_mean = trace_sum.map(|v| v / pixels as f64);
        eprintln!("16:00 live mean {live_mean:?}, traced mean {trace_mean:?}, pixels {pixels}");
        for channel in 0..3 {
            assert!((trace_mean[channel] / live_mean[channel] - 1.0).abs() < 0.05);
        }
    }

    fn floats(bytes: &[u8]) -> Vec<[f32; 4]> {
        bytes
            .chunks_exact(16)
            .map(|pixel| {
                std::array::from_fn(|i| {
                    f32::from_le_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())
                })
            })
            .collect()
    }

    fn quad(corners: [[f32; 3]; 4], material: u32) -> [Triangle; 2] {
        [
            Triangle {
                vertices: [corners[0], corners[1], corners[2]],
                material,
            },
            Triangle {
                vertices: [corners[0], corners[2], corners[3]],
                material,
            },
        ]
    }

    fn dark_sky() -> Sky {
        Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0; 4]],
        }
    }

    fn no_sun() -> Sun {
        Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        }
    }

    fn softbox_room() -> Scene {
        let (x0, x1, y0, y1, z0, z1) = (-2.0, 2.0, 0.0, 3.0, -2.0, 2.0);
        let mut triangles = Vec::new();
        for face in [
            [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
            [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
            [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
            [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
            [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        ] {
            triangles.extend(quad(face, 0));
        }
        for (cx, cy, cz, hx, hz) in [(-0.9, 2.6, -0.6, 0.3, 0.2), (1.0, 2.3, 0.5, 0.2, 0.25)] {
            triangles.extend(quad(
                [
                    [cx - hx, cy, cz - hz],
                    [cx + hx, cy, cz - hz],
                    [cx + hx, cy, cz + hz],
                    [cx - hx, cy, cz + hz],
                ],
                1,
            ));
        }
        Scene {
            triangles,
            shapes: vec![Shape::Ellipsoid {
                center: [-1.6, 2.6, 1.6],
                radii: [0.12; 3],
                material: 2,
            }],
            materials: vec![
                Material {
                    base: [0.7, 0.65, 0.6],
                    specular: 0.0,
                    roughness: 1.0,
                    ..Material::default()
                },
                Material {
                    base: [0.0; 3],
                    specular: 0.0,
                    emission: [8.0, 7.5, 7.0],
                    ..Material::default()
                },
                Material {
                    base: [0.0; 3],
                    specular: 0.0,
                    emission: [6.0, 3.0, 1.5],
                    ..Material::default()
                },
            ],
            sky: dark_sky(),
            camera: Camera {
                origin: [0.0, 1.4, 1.9],
                forward: [0.0, -0.35, -1.0],
                right: [0.8, 0.0, 0.0],
                up: [0.0, 0.47, -0.165],
            },
            sun: no_sun(),
        }
    }

    fn render(
        gpu: &Gpu,
        scene: &Scene,
        detail: &Detail,
        size: (u32, u32),
        samples: u32,
        seed: u32,
        emitter_sampling: bool,
    ) -> Vec<[f32; 4]> {
        let mut trace = Trace::build(gpu, scene, detail, size.0, size.1, emitter_sampling).unwrap();
        let chunk = samples.min(64);
        for _ in 0..samples / chunk {
            trace.sample(gpu, chunk, seed).unwrap();
        }
        floats(&trace.readback(gpu).unwrap().color)
    }

    fn brightness(pixel: [f32; 4]) -> f64 {
        (0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]) as f64
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn emitter_sampling_is_unbiased_and_quieter() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = softbox_room();
        let detail = Detail::default();
        let size = (48, 36);
        let pixels = (size.0 * size.1) as usize;
        let runs = 8;
        let mut images = [Vec::new(), Vec::new()];
        for (index, sampling) in [true, false].into_iter().enumerate() {
            for run in 0..runs {
                images[index].push(render(&gpu, &scene, &detail, size, 64, 101 + run, sampling));
            }
        }
        let reference = render(&gpu, &scene, &detail, size, 8192, 7, false);
        let variance = |runs: &[Vec<[f32; 4]>]| {
            (0..pixels)
                .map(|pixel| {
                    let values: Vec<f64> =
                        runs.iter().map(|image| brightness(image[pixel])).collect();
                    let mean = values.iter().sum::<f64>() / values.len() as f64;
                    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                        / (values.len() - 1) as f64
                })
                .sum::<f64>()
                / pixels as f64
        };
        let image_mean =
            |image: &[[f32; 4]]| image.iter().map(|p| brightness(*p)).sum::<f64>() / pixels as f64;
        let spread = |means: &[f64]| {
            let mean = means.iter().sum::<f64>() / means.len() as f64;
            let var =
                means.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (means.len() - 1) as f64;
            (mean, var)
        };
        let nee_variance = variance(&images[0]);
        let bsdf_variance = variance(&images[1]);
        let (nee_mean, nee_var) =
            spread(&images[0].iter().map(|i| image_mean(i)).collect::<Vec<_>>());
        let (_, bsdf_var) = spread(&images[1].iter().map(|i| image_mean(i)).collect::<Vec<_>>());
        let reference_mean = image_mean(&reference);
        let sigma = (nee_var / runs as f64 + bsdf_var / (8192.0 / 64.0)).sqrt();
        eprintln!(
            "softbox room: mean with NEE {nee_mean:.5}, BSDF-only reference {reference_mean:.5}, sigma {sigma:.5}; \
             pixel variance at 64 spp {nee_variance:.6} with NEE, {bsdf_variance:.6} without, ratio {:.1}",
            bsdf_variance / nee_variance
        );
        assert!(
            (nee_mean - reference_mean).abs() < 4.0 * sigma,
            "{nee_mean} against {reference_mean}"
        );
        assert!((nee_mean / reference_mean - 1.0).abs() < 0.02);
        assert!(bsdf_variance >= 10.0 * nee_variance);
        let blocks = (4, 3);
        for by in 0..blocks.1 {
            for bx in 0..blocks.0 {
                let mut nee = 0.0;
                let mut long = 0.0;
                for y in by * 12..(by + 1) * 12 {
                    for x in bx * 12..(bx + 1) * 12 {
                        let at = (y * size.0 + x) as usize;
                        nee += images[0]
                            .iter()
                            .map(|image| brightness(image[at]))
                            .sum::<f64>()
                            / runs as f64;
                        long += brightness(reference[at]);
                    }
                }
                assert!(
                    (nee / long - 1.0).abs() < 0.05,
                    "block {bx},{by}: {nee} against {long}"
                );
            }
        }
        let again = render(&gpu, &scene, &detail, size, 64, 101, true);
        assert_eq!(again, images[0][0]);
    }

    fn lamp_street(count: usize) -> (Scene, Vec<crate::lights::LocalLight>) {
        use crate::lights::{LightShape, LocalLight};
        let floor = Material {
            base: [0.5, 0.48, 0.45],
            roughness: 0.7,
            ..Material::default()
        };
        let shapes = (0..8)
            .map(|k| Shape::Ellipsoid {
                center: [
                    if k % 2 == 0 { -2.0 } else { 2.5 },
                    0.6,
                    4.0 + k as f32 * 7.5,
                ],
                radii: [0.6; 3],
                material: 0,
            })
            .collect();
        let lights = (0..count)
            .map(|k| {
                let side = if k % 2 == 0 { -5.0 } else { 5.0 };
                let z = (k / 2) as f32 * 64.0 / (count / 2).max(1) as f32;
                let warm = 0.6 + 0.4 * ((k * 7) % 5) as f32 / 4.0;
                let mut light = LocalLight {
                    radius: 0.05,
                    ..LocalLight::point([side, 3.0, z], [1.0, warm, warm * 0.8], 6.0, 9.0)
                };
                if k % 3 == 0 {
                    light.shape = LightShape::Spot {
                        direction: [-side * 0.1, -1.0, 0.0],
                        inner_deg: 30.0,
                        outer_deg: 55.0,
                    };
                }
                light
            })
            .collect();
        let scene = Scene {
            triangles: quad(
                [
                    [-12.0, 0.0, -6.0],
                    [12.0, 0.0, -6.0],
                    [12.0, 0.0, 70.0],
                    [-12.0, 0.0, 70.0],
                ],
                0,
            )
            .to_vec(),
            shapes,
            materials: vec![floor],
            sky: dark_sky(),
            camera: Camera {
                origin: [0.0, 60.0, 32.0],
                forward: [0.0, -1.0, 0.0],
                right: [0.2, 0.0, 0.0],
                up: [0.0, 0.0, -0.6],
            },
            sun: no_sun(),
        };
        (scene, lights)
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn many_lights_through_the_tree_match_the_sum_of_their_light_by_light_renders() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (scene, lights) = lamp_street(128);
        let size = (24, 72);
        let pixels = (size.0 * size.1) as usize;
        let mut summed = vec![[0.0_f64; 3]; pixels];
        for (k, group) in lights.chunks(crate::tree::LOOPED).enumerate() {
            let detail = Detail {
                lights: group.to_vec(),
                ..Detail::default()
            };
            let image = render(&gpu, &scene, &detail, size, 1024, 11 + k as u32, true);
            for (sum, pixel) in summed.iter_mut().zip(&image) {
                for c in 0..3 {
                    sum[c] += f64::from(pixel[c]);
                }
            }
        }
        let detail = Detail {
            lights,
            ..Detail::default()
        };
        let runs: Vec<Vec<[f32; 4]>> = (0..4)
            .map(|run| render(&gpu, &scene, &detail, size, 2048, 301 + run, true))
            .collect();
        let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let reference: Vec<f64> = summed.iter().map(|c| luma(*c)).collect();
        let treed: Vec<f64> = (0..pixels)
            .map(|at| {
                runs.iter().map(|image| brightness(image[at])).sum::<f64>() / runs.len() as f64
            })
            .collect();
        let means: Vec<f64> = runs
            .iter()
            .map(|image| image.iter().map(|p| brightness(*p)).sum::<f64>() / pixels as f64)
            .collect();
        let mean = means.iter().sum::<f64>() / means.len() as f64;
        let sigma = (means.iter().map(|m| (m - mean).powi(2)).sum::<f64>()
            / ((means.len() - 1) * means.len()) as f64)
            .sqrt();
        let reference_mean = reference.iter().sum::<f64>() / pixels as f64;
        let peak = reference.iter().cloned().fold(0.0, f64::max);
        let mut gaps: Vec<f64> = treed
            .iter()
            .zip(&reference)
            .map(|(t, r)| (t - r).abs() / peak)
            .collect();
        gaps.sort_by(f64::total_cmp);
        let p95 = gaps[gaps.len() * 95 / 100];
        eprintln!(
            "128 lights: tree mean {mean:.5} (sigma {sigma:.5}), light-by-light sum {reference_mean:.5}, \
             ratio {:.4}; per-pixel gap of the peak {peak:.3}: median {:.4}, p95 {p95:.4}",
            mean / reference_mean,
            gaps[gaps.len() / 2]
        );
        assert!(reference_mean > 0.0);
        assert!(
            (mean / reference_mean - 1.0).abs() < 0.01,
            "{mean} against {reference_mean}"
        );
        assert!(p95 < 0.03);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn local_lights_match_the_live_direct_term() {
        use crate::lights::{LightShape, LocalLight};
        use crate::math::{add, dot, length, normalize, scale, sub};
        use pfx_materials::{Shaded, eval};

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let floor = Material {
            base: [0.5, 0.45, 0.4],
            roughness: 0.6,
            ..Material::default()
        };
        let sphere = ([-0.4, 0.4, 0.3], 0.25);
        let scene = Scene {
            triangles: quad(
                [
                    [-4.0, 0.0, -4.0],
                    [4.0, 0.0, -4.0],
                    [4.0, 0.0, 4.0],
                    [-4.0, 0.0, 4.0],
                ],
                0,
            )
            .to_vec(),
            shapes: vec![Shape::Ellipsoid {
                center: sphere.0,
                radii: [sphere.1; 3],
                material: 1,
            }],
            materials: vec![
                floor,
                Material {
                    base: [0.0; 3],
                    specular: 0.0,
                    ..Material::default()
                },
            ],
            sky: dark_sky(),
            camera: Camera {
                origin: [0.0, 4.0, 0.0],
                forward: [0.0, -1.0, 0.0],
                right: [0.6, 0.0, 0.0],
                up: [0.0, 0.0, -0.6],
            },
            sun: no_sun(),
        };
        let point = LocalLight {
            radius: 0.1,
            ..LocalLight::point([-0.8, 1.2, 0.3], [1.0, 0.9, 0.8], 3.0, 8.0)
        };
        let spot = LocalLight {
            shape: LightShape::Spot {
                direction: [0.0, -1.0, 0.0],
                inner_deg: 15.0,
                outer_deg: 25.0,
            },
            ..LocalLight::point([1.2, 1.5, 1.2], [0.9, 1.0, 0.8], 4.0, 10.0)
        };
        let rect = LocalLight::rect(
            [0.9, 1.0, -0.4],
            [0.3, 0.0, 0.0],
            [0.0, 0.0, 0.2],
            [0.8, 0.9, 1.0],
            2.0,
        );
        let detail = Detail {
            lights: vec![point, spot, rect],
            ..Detail::default()
        };
        let size = 48;
        let traced = render(&gpu, &scene, &detail, (size, size), 65536, 3, true);
        let blocked = |from: [f32; 3], to: [f32; 3], margin: f32| {
            let segment = sub(to, from);
            let length = length(segment);
            let along = (dot(sub(sphere.0, from), segment) / (length * length)).clamp(0.0, 1.0);
            crate::math::length(sub(add(from, scale(segment, along)), sphere.0)) < sphere.1 + margin
        };
        let rim: Vec<[f32; 3]> = (0..9)
            .map(|k| {
                if k == 0 {
                    point.position
                } else {
                    let angle = k as f32 * std::f32::consts::TAU / 8.0;
                    add(point.position, [0.1 * angle.cos(), 0.0, 0.1 * angle.sin()])
                }
            })
            .collect();
        let grid: Vec<[f32; 3]> = (0..81)
            .map(|k| {
                let u = (k % 9) as f32 / 4.0 - 1.0;
                let v = (k / 9) as f32 / 4.0 - 1.0;
                add(
                    add([0.9, 1.0, -0.4], [0.3 * u, 0.0, 0.0]),
                    [0.0, 0.0, 0.2 * v],
                )
            })
            .collect();
        let shaded = Shaded::from_material(&floor);
        let radiance = rect.colour.map(|c| c * rect.intensity / rect.area());
        let mut compared = [0, 0];
        let mut worst = [0.0_f32; 2];
        for y in 0..size {
            for x in 0..size {
                let mut expected = [[0.0_f64; 3]; 2];
                let mut lit = true;
                let mut umbra = true;
                let mut floor_hit = true;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let ndc_x = (x as f32 + (sx as f32 + 0.5) / 4.0) / size as f32 * 2.0 - 1.0;
                        let ndc_y = (y as f32 + (sy as f32 + 0.5) / 4.0) / size as f32 * 2.0 - 1.0;
                        let ray = normalize(add(
                            add(scene.camera.forward, scale(scene.camera.right, ndc_x)),
                            scale(scene.camera.up, -ndc_y),
                        ));
                        let at = add(
                            scene.camera.origin,
                            scale(ray, -scene.camera.origin[1] / ray[1]),
                        );
                        if blocked(scene.camera.origin, at, 0.02) {
                            floor_hit = false;
                        }
                        let wo = [-ray[2], -ray[0], -ray[1]];
                        let local = |d: [f32; 3]| [d[2], d[0], d[1]];
                        if rim.iter().any(|&light| blocked(at, light, 0.03)) {
                            lit = false;
                        }
                        if !rim.iter().all(|&light| blocked(at, light, -0.03)) {
                            umbra = false;
                        }
                        if grid.iter().any(|&light| blocked(at, light, 0.05)) {
                            lit = false;
                            umbra = false;
                        }
                        let mut point_part = [0.0_f64; 3];
                        for light in [point, spot] {
                            let to = sub(light.position, at);
                            let distance = length(to);
                            let l = scale(to, 1.0 / distance);
                            let mut widened = shaded;
                            widened.roughness = shaded
                                .roughness
                                .max((light.radius / (2.0 * distance)).clamp(0.0, 1.0).sqrt());
                            let brdf = eval(&widened, wo, local(l));
                            let strength =
                                light.falloff(distance) * light.cone(scale(l, -1.0)) * l[1];
                            for c in 0..3 {
                                point_part[c] += (brdf[c] * light.colour[c] * strength) as f64;
                            }
                        }
                        let mut rect_part = [0.0_f64; 3];
                        let cells = 48;
                        let cell_area = rect.area() / (cells * cells) as f32;
                        for cv in 0..cells {
                            for cu in 0..cells {
                                let u = (cu as f32 + 0.5) / cells as f32 * 2.0 - 1.0;
                                let v = (cv as f32 + 0.5) / cells as f32 * 2.0 - 1.0;
                                let sample = add(
                                    add(rect.position, [0.3 * u, 0.0, 0.0]),
                                    [0.0, 0.0, 0.2 * v],
                                );
                                let to = sub(sample, at);
                                let d2 = dot(to, to);
                                let l = scale(to, 1.0 / d2.sqrt());
                                let brdf = eval(&shaded, wo, local(l));
                                let weight = l[1] * l[1] * cell_area / d2;
                                for c in 0..3 {
                                    rect_part[c] += (brdf[c] * radiance[c] * weight) as f64;
                                }
                            }
                        }
                        for c in 0..3 {
                            expected[0][c] += (point_part[c] + rect_part[c]) / 16.0;
                            expected[1][c] += rect_part[c] / 16.0;
                        }
                    }
                }
                if !floor_hit {
                    continue;
                }
                let pick = if lit {
                    0
                } else if umbra {
                    1
                } else {
                    continue;
                };
                let got = traced[(y * size + x) as usize];
                for c in 0..3 {
                    let error = (got[c] as f64 / expected[pick][c] - 1.0).abs() as f32;
                    worst[pick] = worst[pick].max(error);
                    assert!(
                        error < 0.02,
                        "{x},{y} channel {c}: {} against {}",
                        got[c],
                        expected[pick][c]
                    );
                }
                compared[pick] += 1;
            }
        }
        eprintln!(
            "local lights: {} lit pixels within {:.4}, {} point-umbra pixels within {:.4}",
            compared[0], worst[0], compared[1], worst[1]
        );
        assert!(compared[0] > size as usize * size as usize / 2);
        assert!(compared[1] >= 4);
    }

    #[test]
    fn instance_flags_and_placed_layers_reach_the_buffers() {
        let triangle = TriangleDetail {
            vertices: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: [[0.0, 0.0, 1.0]; 3],
            uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            uvs1: [[0.0; 2]; 3],
            material: 0,
        };
        let instance = |surface: InstanceSurface, shadow_only: bool, two_sided: bool, x: f32| {
            crate::detail::Instance {
                triangles: vec![triangle].into(),
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [x, 0.0, 0.0, 1.0],
                ],
                surface,
                shadow_only,
                two_sided,
                motion: None,
            }
        };
        let placed = ContentLayer {
            slot: 1,
            ..ContentLayer::from_kind(
                Content::Screen,
                0.0,
                0,
                &pfx_materials::ContentLook {
                    screen_gain: 1.5,
                    ..Default::default()
                },
            )
        };
        let detail = Detail {
            instances: vec![
                instance(InstanceSurface::default(), false, true, 0.0),
                instance(
                    InstanceSurface {
                        casts_shadow: false,
                        alpha_cutoff: 0.25,
                        ..InstanceSurface::default()
                    },
                    true,
                    true,
                    2.0,
                ),
                instance(
                    InstanceSurface {
                        layer: placed,
                        cutout: true,
                        face: crate::detail::ContentFace::Back,
                        alpha_cutoff: 0.9,
                        ..InstanceSurface::default()
                    },
                    false,
                    true,
                    4.0,
                ),
                instance(InstanceSurface::default(), false, false, 6.0),
            ],
            content_slots: vec![
                None,
                Some(crate::detail::ContentImage {
                    width: 1,
                    height: 1,
                    texels: vec![[255; 4]],
                    srgb: true,
                }),
            ],
            ..Detail::default()
        };
        let materials = [Material::default(), Material::default()];
        let buffers = SceneBuffers::build(&[], &[], &materials, &detail).unwrap();
        let layers: &[GpuLayer] = bytemuck::cast_slice(&buffers.layers);
        let records: &[GpuSurface] = bytemuck::cast_slice(&buffers.surfaces);
        assert_eq!(buffers.triangles.len(), std::mem::size_of::<GpuTriangle>());
        assert_eq!(buffers.triangle_count, 1);
        assert_eq!(std::mem::size_of::<GpuSurface>(), 176);
        assert_eq!(records.len(), 5);
        assert_eq!(
            records
                .iter()
                .map(|record| record.info[0])
                .collect::<Vec<_>>(),
            [
                0,
                0,
                SHADOW_ONLY | CASTS_NO_SHADOW,
                CUTOUT | 2 << FACE_SHIFT | 3 << LAYER_SHIFT,
                ONE_SIDED,
            ]
        );
        assert_eq!(
            records[1..]
                .iter()
                .map(|record| (f32::from_bits(record.info[2]), record.forward[0][3]))
                .collect::<Vec<_>>(),
            [(0.5, 0.0), (0.25, 2.0), (0.9, 4.0), (0.5, 6.0)]
        );
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[2].layer, [1.0, 2.0, -1.0, 0.0]);
        assert_eq!(layers[2].extra[0], placed.strength);
        let flat = Triangle {
            vertices: [[-2.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [-2.0, 1.0, 0.0]],
            material: 0,
        };
        let with_scene = SceneBuffers::build(&[flat], &[], &materials, &detail).unwrap();
        assert_eq!(with_scene.triangle_count, 2);
        let scene_records: &[GpuSurface] = bytemuck::cast_slice(&with_scene.surfaces);
        assert_eq!(scene_records[0].info, [0, NO_MATERIAL, 0, 1.0f32.to_bits()]);
        let glow = Material {
            emission: [1.0, 0.2, 0.0],
            ..Material::default()
        };
        let sided = |two_sided| Detail {
            instances: vec![instance(InstanceSurface::default(), false, two_sided, 0.0)],
            ..Detail::default()
        };
        assert_eq!(
            SceneBuffers::build(&[], &[], &[glow], &sided(false))
                .unwrap()
                .emitter_count,
            1
        );
        assert_eq!(
            SceneBuffers::build(&[], &[], &[glow], &sided(true))
                .unwrap()
                .emitter_count,
            1
        );
        let missing = Detail {
            content_slots: Vec::new(),
            ..detail
        };
        assert!(SceneBuffers::build(&[], &[], &materials, &missing).is_err());
    }

    #[test]
    fn a_band_splits_into_whole_samples_whole_blocks_and_partial_blocks() {
        let piece = |sample, samples, order, blocks, from, to| Piece {
            sample,
            samples,
            order,
            blocks,
            from,
            to,
        };
        assert_eq!(pieces(4, 8, 0, 32), vec![piece(0, 1, 0, 4, 0, 8)]);
        assert_eq!(
            pieces(4, 8, 7, 3),
            vec![piece(0, 1, 0, 1, 7, 8), piece(0, 1, 1, 1, 0, 2)]
        );
        assert_eq!(
            pieces(4, 8, 28, 40),
            vec![
                piece(0, 1, 3, 1, 4, 8),
                piece(1, 1, 0, 4, 0, 8),
                piece(2, 1, 0, 1, 0, 4)
            ]
        );
        assert_eq!(
            pieces(2, 1, 1, 6),
            vec![
                piece(0, 1, 1, 1, 0, 1),
                piece(1, 2, 0, 2, 0, 1),
                piece(3, 1, 0, 1, 0, 1)
            ]
        );
        for blocks in [1, 2, 4, 5] {
            for per_block in [1, 8] {
                for band in [1, 3, 7, 8, 9, 64, 100] {
                    let samples = 5;
                    let total = blocks * per_block * samples;
                    let mut seen = Vec::new();
                    let mut at = 0;
                    while at < total {
                        let count = band.min(total - at);
                        let mut covered = 0;
                        for piece in pieces(blocks, per_block, at, count) {
                            assert!(piece.order + piece.blocks <= blocks);
                            assert!(piece.from < piece.to && piece.to <= per_block);
                            assert!(piece.blocks == 1 || (piece.from, piece.to) == (0, per_block));
                            for sample in piece.sample..piece.sample + piece.samples {
                                for order in piece.order..piece.order + piece.blocks {
                                    for unit in piece.from..piece.to {
                                        seen.push((sample, order, unit));
                                        covered += 1;
                                    }
                                }
                            }
                        }
                        assert_eq!(covered, count);
                        at += count;
                    }
                    let expected: Vec<_> = (0..samples)
                        .flat_map(|sample| {
                            (0..blocks).flat_map(move |order| {
                                (0..per_block).map(move |unit| (sample, order, unit))
                            })
                        })
                        .collect();
                    assert_eq!(
                        seen, expected,
                        "{blocks} blocks, {per_block} per block, band {band}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_block_stride_visits_every_block_once() {
        for blocks in 1..200u32 {
            let stride = block_stride(blocks);
            let mut seen: Vec<u32> = (0..blocks).map(|k| k * stride % blocks).collect();
            seen.sort_unstable();
            assert_eq!(seen, (0..blocks).collect::<Vec<_>>(), "{blocks} blocks");
        }
        assert_eq!(block_stride(135), 52);
        let order: Vec<u32> = (0..8).map(|k| k * block_stride(16) % 16).collect();
        assert_eq!(order, [0, 7, 14, 5, 12, 3, 10, 1]);
    }

    #[test]
    fn the_lane_controller_widens_under_half_the_span_and_undoes_a_step_over_it() {
        assert_eq!(floor_power_of_two(1), 1);
        assert_eq!(floor_power_of_two(6), 4);
        assert_eq!(floor_power_of_two(64), 64);
        assert_eq!(Lanes::new().width, LANES);
        let mut lanes = Lanes::new();
        lanes.width = 1;
        assert_eq!(lanes.strips(6), 1);
        for blocks in [2, 4, 6, 6] {
            lanes.learn(4.3, 6);
            assert_eq!(lanes.blocks, blocks);
        }
        assert_eq!((lanes.width, lanes.last), (2, Step::Lanes));
        lanes.learn(4.5, 6);
        assert_eq!(lanes.width, 4);
        lanes.learn(11.5, 6);
        assert_eq!((lanes.width, lanes.widest), (2, 4));
        lanes.learn(4.5, 6);
        assert_eq!((lanes.width, lanes.last), (2, Step::Held));
        lanes.learn(7.0, 6);
        assert_eq!((lanes.width, lanes.blocks), (2, 6));
        lanes.learn(20.0, 6);
        assert_eq!((lanes.blocks, lanes.most), (1, 6));
        lanes.learn(5.0, 6);
        assert_eq!(lanes.blocks, 2);
        lanes.learn(f64::NAN, 6);
        assert_eq!(lanes.blocks, 2);
        lanes.blocks = 4;
        assert_eq!(lanes.strips(6), 3);
        assert_eq!(lanes.strips(135), 4);
        lanes.blocks = 200;
        assert_eq!(lanes.strips(135), 135);
        let mut alone = Lanes::new();
        alone.width = 8;
        alone.learn(30.0, 1);
        assert_eq!((alone.width, alone.widest), (1, 8));
        alone.learn(30.0, 1);
        assert_eq!(alone.width, 1);
        let mut dense = Lanes::new();
        dense.learn_dense(
            Spread {
                longest: 40.7,
                single: 0.0,
            },
            12,
        );
        assert_eq!(dense.width, LANES);
        dense.learn_dense(
            Spread {
                longest: 40.7,
                single: 40.7,
            },
            12,
        );
        assert_eq!((dense.width, dense.widest, dense.blocks), (1, LANES, 1));
    }

    #[test]
    fn a_dense_wide_strip_splits_into_column_pieces_before_it_drops_lanes() {
        assert_eq!(
            (most_pieces(480), most_pieces(60), most_pieces(12)),
            (32, 4, 1)
        );
        let mut wide = Lanes::new();
        wide.learn_dense(
            Spread {
                longest: 14.0,
                single: 14.0,
            },
            480,
        );
        assert_eq!((wide.width, wide.pieces, wide.blocks), (LANES, 4, 1));
        for blocks in [2, 4, 8] {
            wide.learn_pieces(3.0, 270, 480);
            assert_eq!(wide.blocks, blocks);
        }
        wide.learn_pieces(10.0, 270, 480);
        assert_eq!((wide.blocks, wide.most, wide.pieces), (4, 8, 4));
        wide.learn_pieces(5.0, 270, 480);
        assert_eq!((wide.blocks, wide.pieces), (7, 4));
        wide.learn_pieces(5.0, 270, 480);
        assert_eq!((wide.pieces, wide.last), (2, Step::Pieces));
        wide.learn_pieces(10.0, 270, 480);
        assert_eq!((wide.pieces, wide.fewest), (4, 2));
        wide.learn_pieces(4.0, 270, 480);
        assert_eq!((wide.pieces, wide.blocks), (4, 7));
        wide.blocks = 1;
        wide.learn_pieces(50.0, 270, 480);
        assert_eq!((wide.width, wide.pieces), (LANES, 8));
        wide.pieces = 32;
        wide.learn_pieces(50.0, 270, 480);
        assert_eq!((wide.width, wide.widest, wide.pieces), (1, LANES, 1));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn sliced_samples_trace_the_same_bytes() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = softbox_room();
        let detail = Detail::default();
        let (width, height) = (37, 29);
        let calls = [(3, 5), (2, 6)];
        let mut turns = Turns::default();
        for lit in [true, false] {
            let fresh = || Trace::build(&gpu, &scene, &detail, width, height, lit).unwrap();
            let mut whole = fresh();
            for (count, seed) in calls {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                whole.dispatch(
                    &gpu,
                    &mut encoder,
                    0,
                    height.div_ceil(BAND_STEP),
                    (0, LANES),
                    (0, width.div_ceil(8)),
                    whole.samples,
                    count,
                    seed,
                );
                gpu.queue.submit(Some(encoder.finish()));
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                whole.samples += count;
            }
            let unsliced = whole.readback(&gpu).unwrap();
            for (step, fixed) in [
                (1, Some(1)),
                (1, Some(7)),
                (1, Some(64)),
                (1, None),
                (8, Some(8)),
                (8, Some(16)),
                (8, None),
            ] {
                let mut trace = fresh();
                let mut pacer = Pacer::default();
                pacer.set_step(step, step);
                pacer.set_fixed_units(fixed);
                let mut slices = 0;
                for (count, seed) in calls {
                    let stats = trace
                        .sample_sliced(&gpu, count, seed, &mut pacer, |ms| turns.add(ms))
                        .unwrap();
                    slices += stats.count();
                }
                if let Some(rows) = fixed {
                    let expected: u32 = calls
                        .iter()
                        .map(|(count, _)| {
                            (height.next_multiple_of(BAND_STEP) * count).div_ceil(rows)
                        })
                        .sum();
                    assert_eq!(slices, expected as usize, "step {step}, {rows} rows");
                }
                assert_eq!(trace.samples(), 5);
                assert!(
                    trace.readback(&gpu).unwrap() == unsliced,
                    "lit {lit}, step {step}, fixed {fixed:?}: sliced bytes differ"
                );
            }
            let mut paced = fresh();
            let mut pacer = Pacer::default();
            let mut own = fresh();
            for (count, seed) in calls {
                paced
                    .sample_paced(&gpu, count, seed, &mut pacer, |ms| turns.add(ms))
                    .unwrap();
                own.sample(&gpu, count, seed).unwrap();
            }
            assert_eq!(pacer.step(), (1, 1));
            assert!(
                paced.readback(&gpu).unwrap() == unsliced,
                "lit {lit}: paced"
            );
            assert!(own.readback(&gpu).unwrap() == unsliced, "lit {lit}: sample");
            for pieces in [2, 4] {
                let mut split = fresh();
                split.lanes = Lanes {
                    pieces,
                    ..Lanes::new()
                };
                let mut pacer = Pacer::default();
                for (count, seed) in calls {
                    split
                        .sample_paced(&gpu, count, seed, &mut pacer, |ms| turns.add(ms))
                        .unwrap();
                }
                assert!(
                    split.readback(&gpu).unwrap() == unsliced,
                    "lit {lit}: {pieces} column pieces"
                );
            }
        }
    }

    fn device_with(label: &'static str, limits: wgpu::Limits) -> Gpu {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
            required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
            required_limits: limits,
            ..Default::default()
        }))
        .unwrap();
        Gpu {
            info: adapter.get_info(),
            adapter,
            device,
            queue,
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_device_below_the_trace_floor_is_refused() {
        let gpu = device_with("below the trace floor", wgpu::Limits::default());
        let Err(error) = Trace::new(&gpu, &softbox_room(), 8, 8) else {
            panic!("a device with 8 storage buffers per stage traced");
        };
        let TraceError::Refused(refusal) = &error else {
            panic!("{error}");
        };
        assert_eq!(
            refusal.shortfall,
            GpuShortfall::Limit {
                name: "max_storage_buffers_per_shader_stage",
                needed: u64::from(pfx_gpu::floor::TRACE_STORAGE_BUFFERS_PER_STAGE),
                has: 8,
            }
        );
        let message = String::from(error);
        assert!(
            message.contains("max_storage_buffers_per_shader_stage"),
            "{message}"
        );
        let fit = pollster::block_on(Gpu::headless()).unwrap();
        assert!(Trace::new(&fit, &softbox_room(), 8, 8).is_ok());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn default_buffer_limits_trace_a_small_image_and_refuse_a_4k_one() {
        let gpu = device_with(
            "default buffer limits",
            wgpu::Limits {
                max_storage_buffers_per_shader_stage:
                    pfx_gpu::floor::TRACE_STORAGE_BUFFERS_PER_STAGE,
                ..wgpu::Limits::default()
            },
        );
        let defaults = wgpu::Limits::default();
        assert_eq!(
            gpu.device.limits().max_storage_buffer_binding_size,
            defaults.max_storage_buffer_binding_size
        );
        gpu.check(&trace_floor()).unwrap();
        let mut small = Trace::new(&gpu, &softbox_room(), 64, 64).unwrap();
        small.sample(&gpu, 2, 3).unwrap();
        assert_eq!(small.samples(), 2);
        let (width, height) = (3840, 2160);
        let Err(error) = Trace::new(&gpu, &softbox_room(), width, height) else {
            panic!("a 4K trace was built on a 128 MiB storage binding range");
        };
        let TraceError::Refused(refusal) = &error else {
            panic!("{error}");
        };
        assert_eq!(
            refusal.shortfall,
            GpuShortfall::Limit {
                name: "max_storage_buffer_binding_size",
                needed: u64::from(width) * u64::from(height) * 48,
                has: defaults.max_storage_buffer_binding_size,
            }
        );
    }

    fn glass_material(base: [f32; 3], roughness: f32, ior: f32) -> Material {
        Material {
            base,
            roughness,
            transmission: 1.0,
            ior,
            specular: 0.04,
            ..Material::default()
        }
    }

    fn sheet(y: f32, x0: f32, x1: f32, z0: f32, z1: f32, material: u32) -> [Triangle; 2] {
        quad(
            [[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]],
            material,
        )
    }

    fn liquid_surface(material: u32, nx: u32, nz: u32) -> Vec<Triangle> {
        let (x0, x1, z0, z1) = (-1.6, 1.6, -2.4, -0.2);
        let mut points = Vec::with_capacity(((nx + 1) * (nz + 1)) as usize);
        for j in 0..=nz {
            for i in 0..=nx {
                let u = i as f32 / nx as f32;
                let v = j as f32 / nz as f32;
                let x = x0 + (x1 - x0) * u;
                let z = z0 + (z1 - z0) * v;
                let y = 0.12 + 0.07 * (u * 14.0).sin() * (v * 10.0).cos();
                points.push([x, y, z]);
            }
        }
        let at = |i: u32, j: u32| points[(j * (nx + 1) + i) as usize];
        let mut triangles = Vec::new();
        for j in 0..nz {
            for i in 0..nx {
                triangles.extend(quad(
                    [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)],
                    material,
                ));
            }
        }
        triangles.extend(sheet(0.02, x0, x1, z0, z1, material));
        triangles
    }

    fn liquid_volume() -> Vec<Shape> {
        let mut shapes = Vec::new();
        for (x, z) in [(-0.7, -1.7), (0.5, -1.3), (-0.2, -0.8), (0.9, -2.0)] {
            let left = shapes.len();
            shapes.push(Shape::Ellipsoid {
                center: [x, 0.35, z],
                radii: [0.45, 0.28, 0.38],
                material: 1,
            });
            shapes.push(Shape::Ellipsoid {
                center: [x + 0.28, 0.42, z + 0.16],
                radii: [0.32, 0.22, 0.3],
                material: 1,
            });
            shapes.push(Shape::SmoothUnion {
                left,
                right: left + 1,
                radius: 0.18,
                material: 1,
            });
        }
        shapes
    }

    fn stacked_glass(origin_z: f32) -> Scene {
        let mut triangles = Vec::new();
        triangles.extend(sheet(0.0, -4.0, 4.0, -3.2, 3.2, 2));
        for (pane, y) in [0.55, 0.95, 1.35, 1.75].into_iter().enumerate() {
            let shift = if pane.is_multiple_of(2) { -0.05 } else { 0.05 };
            triangles.extend(sheet(y, -1.7, 1.7, -2.45 + shift, -0.15, 0));
        }
        triangles.extend(liquid_surface(1, 24, 16));
        Scene {
            triangles,
            shapes: liquid_volume(),
            materials: vec![
                glass_material([0.92, 0.96, 1.0], 0.02, 1.5),
                glass_material([0.75, 0.88, 0.95], 0.08, 1.33),
                Material {
                    base: [0.55, 0.52, 0.48],
                    roughness: 0.8,
                    specular: 0.03,
                    ..Material::default()
                },
            ],
            sky: Sky {
                width: 2,
                height: 1,
                texels: vec![[1.0, 0.95, 0.9, 1.0], [0.75, 0.82, 0.95, 1.0]],
            },
            camera: Camera {
                origin: [0.0, 6.0, origin_z],
                forward: [0.0, -1.0, 0.0],
                right: [1.0, 0.0, 0.0],
                up: [0.0, 0.0, 1.0],
            },
            sun: Sun {
                direction: [0.2, 0.9, 0.1],
                color: [1.0, 0.98, 0.94],
                intensity: 2.0,
            },
        }
    }

    struct Pinned;

    impl Pinned {
        fn software_pcg() -> Self {
            unsafe {
                std::env::set_var("PFX_TRACE_RT", "0");
                std::env::set_var("PFX_TRACE_SAMPLER", "pcg");
            }
            Self
        }
    }

    impl Drop for Pinned {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("PFX_TRACE_RT");
                std::env::remove_var("PFX_TRACE_SAMPLER");
            }
        }
    }

    fn ortho(width: f32, height: f32) -> Detail {
        Detail {
            projection: Projection::Orthographic { width, height },
            ..Detail::default()
        }
    }

    fn dense_glass(nx: u32, nz: u32) -> Scene {
        let mut scene = stacked_glass(-1.3);
        scene.triangles.retain(|triangle| triangle.material != 1);
        scene.triangles.extend(liquid_surface(1, nx, nz));
        scene
    }

    const EIGHT_ROW_BANDS: [[u64; 3]; 3] = [
        [
            0x10cc_b82e_6e49_544b,
            0x4764_c25b_7669_ef25,
            0x11a9_7794_10d3_2325,
        ],
        [
            0xbaf6_c654_3b05_bee4,
            0x2b63_4909_53bb_71e3,
            0x2716_720e_ead3_6d2b,
        ],
        [
            0x0dd7_acd3_33e6_3335,
            0x2b63_4909_53bb_71e3,
            0x2716_720e_ead3_6d2b,
        ],
    ];

    fn fixed_pass(
        gpu: &Gpu,
        turns: &mut Turns,
        trace: &mut Trace,
        lanes: (u32, u32),
        per_block: u32,
        units: Option<u32>,
    ) -> Stats {
        let mut pacer = Pacer::default();
        pacer.set_step(1, 1);
        pacer.set_fixed_units(units);
        let mut stats = Stats::default();
        let pass = Pass {
            lanes,
            per_block,
            columns: (0, trace.width.div_ceil(8)),
        };
        trace
            .pass(
                gpu,
                3,
                1,
                pass,
                &mut pacer,
                &mut |ms| turns.add(ms),
                &mut stats,
            )
            .unwrap();
        stats
    }

    fn summary(stats: &Stats) -> String {
        format!(
            "{} slices, median {:.2} ms, longest {:.2} ms, sum {:.1} ms",
            stats.count(),
            stats.median_ms(),
            stats.longest_ms(),
            stats.milliseconds.iter().sum::<f64>()
        )
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn deep_glass_cost_by_lanes_and_blocks() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let scene = dense_glass(48, 32);
        let frame = ortho(3.2, 2.0);
        let (width, height): (u32, u32) = (96, 48);
        let blocks = height.div_ceil(BAND_STEP);
        let mut turns = Turns::default();
        let fresh = || Trace::new_detailed(&gpu, &scene, &frame, width, height).unwrap();
        let mut trace = fresh();
        let rows = fixed_pass(&gpu, &mut turns, &mut trace, (0, LANES), BAND_STEP, Some(1));
        println!("one row per submission: {}", summary(&rows));
        let mut trace = fresh();
        let bands = fixed_pass(
            &gpu,
            &mut turns,
            &mut trace,
            (0, LANES),
            BAND_STEP,
            Some(BAND_STEP),
        );
        println!("one 8-row band per submission: {}", summary(&bands));
        let mut trace = fresh();
        let whole = fixed_pass(
            &gpu,
            &mut turns,
            &mut trace,
            (0, LANES),
            BAND_STEP,
            Some(blocks * BAND_STEP),
        );
        println!("whole frame in one submission: {}", summary(&whole));
        for lanes in [2, 4] {
            for strips in 1..=blocks {
                let mut trace = fresh();
                let mut runs = Stats::default();
                for first in (0..LANES).step_by(lanes as usize) {
                    let run = fixed_pass(
                        &gpu,
                        &mut turns,
                        &mut trace,
                        (first, first + lanes),
                        1,
                        Some(strips),
                    );
                    runs.milliseconds.extend(run.milliseconds);
                }
                println!(
                    "{lanes} lanes, {strips} blocks per submission: {}",
                    summary(&runs)
                );
            }
        }
        for lanes in [1, 2, 4, 8, 16, 32] {
            let mut trace = fresh();
            let mut each = Stats::default();
            let mut all = Stats::default();
            for first in (0..LANES).step_by(lanes as usize) {
                let block = fixed_pass(
                    &gpu,
                    &mut turns,
                    &mut trace,
                    (first, first + lanes),
                    1,
                    Some(1),
                );
                each.milliseconds.extend(block.milliseconds);
                let whole = fixed_pass(
                    &gpu,
                    &mut turns,
                    &mut trace,
                    (first, first + lanes),
                    1,
                    Some(blocks),
                );
                all.milliseconds.extend(whole.milliseconds);
            }
            println!(
                "{lanes} lanes, one block per submission: {}",
                summary(&each)
            );
            println!(
                "{lanes} lanes, every block in one submission: {}",
                summary(&all)
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_wide_dense_frame_keeps_every_lane() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut turns = Turns::default();
        let scene = dense_glass(4, 4);
        let frame = ortho(3.2, 0.1);
        let (width, height, samples) = (5120, 64, 8);
        let mut run = |lanes: Lanes| {
            let mut trace = Trace::new_detailed(&gpu, &scene, &frame, width, height).unwrap();
            trace.lanes = lanes;
            let mut pacer = Pacer::default();
            let stats = trace
                .sample_paced(&gpu, samples, 3, &mut pacer, |ms| turns.add(ms))
                .unwrap();
            let gpu_ms: f64 = stats.milliseconds.iter().sum();
            println!(
                "{}, lanes {}, pieces {}, {:.2} M paths/s",
                summary(&stats),
                trace.lanes.width,
                trace.lanes.pieces,
                f64::from(width * height * samples) / gpu_ms / 1e3
            );
            (trace.lanes.width, gpu_ms)
        };
        let (width_now, now) = run(Lanes::new());
        let (_, one_lane) = run(Lanes::lane_passes());
        assert_eq!(width_now, LANES);
        assert!(
            now * 2.0 < one_lane,
            "{now:.0} ms against {one_lane:.0} ms at one lane"
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn stacked_glass_and_liquid_stay_within_the_span() {
        let _pinned = Pinned::software_pcg();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut turns = Turns::default();
        let scene = dense_glass(48, 32);
        let frame = ortho(3.2, 2.0);
        let (width, height) = (96, 48);
        let samples = 2;
        let mut banded = Trace::new_detailed(&gpu, &scene, &frame, width, height).unwrap();
        let mut bands = Pacer::default();
        bands.set_step(BAND_STEP, BAND_STEP);
        bands.set_fixed_units(Some(BAND_STEP));
        let before = banded
            .sample_sliced(&gpu, samples, 3, &mut bands, |ms| turns.add(ms))
            .unwrap();
        println!("8-row bands: {}", summary(&before));
        assert!(
            before.longest_ms() > SAMPLE_SPAN_MS,
            "an 8-row band no longer overshoots: {}",
            before.longest_ms()
        );
        let mut paced = Trace::new_detailed(&gpu, &scene, &frame, width, height).unwrap();
        let mut pacer = Pacer::default();
        let after = paced
            .sample_paced(&gpu, samples, 3, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        println!("paced: {}, {} lanes", summary(&after), paced.lanes());
        println!(
            "paced ms: {}",
            after
                .milliseconds
                .iter()
                .map(|ms| format!("{ms:.2}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let strips = height.div_ceil(BAND_STEP) as usize;
        let (probe, rest) = after.milliseconds.split_at(strips);
        assert!(
            probe.iter().all(|ms| *ms <= SAMPLE_SPAN_MS * 8.0),
            "probe {probe:?}"
        );
        let mut sorted = rest.to_vec();
        sorted.sort_by(f64::total_cmp);
        let p95 = sorted[(sorted.len() * 95).div_ceil(100).saturating_sub(1)];
        assert!(p95 <= SAMPLE_SPAN_MS, "p95 {p95}");
        let longest = sorted.last().copied().unwrap_or(0.0);
        assert!(longest <= SAMPLE_SPAN_MS * 1.5, "{longest}");
        let sum = |stats: &Stats| stats.milliseconds.iter().sum::<f64>();
        assert!(
            sum(&after) <= sum(&before) * 1.5,
            "paced {} against bands {}",
            sum(&after),
            sum(&before)
        );
        assert!(paced.readback(&gpu).unwrap() == banded.readback(&gpu).unwrap());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn transmissive_shadows_cost_on_deep_glass() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut turns = Turns::default();
        let scene = dense_glass(48, 32);
        let samples = 4;
        for round in 0..2 {
            for transmissive in [false, true] {
                let detail = Detail {
                    transmissive_shadows: transmissive,
                    ..ortho(3.2, 2.0)
                };
                let mut trace = Trace::new_detailed(&gpu, &scene, &detail, 96, 48).unwrap();
                let mut pacer = Pacer::default();
                trace
                    .sample_paced(&gpu, 1, 3, &mut pacer, |ms| turns.add(ms))
                    .unwrap();
                let stats = trace
                    .sample_paced(&gpu, samples, 4, &mut pacer, |ms| turns.add(ms))
                    .unwrap();
                println!(
                    "deep glass, round {round}, option {}: {:.2} ms per sample; {}, {} lanes",
                    if transmissive { "on" } else { "off" },
                    stats.milliseconds.iter().sum::<f64>() / f64::from(samples),
                    summary(&stats),
                    trace.lanes()
                );
                assert!(
                    stats.longest_ms() <= SAMPLE_SPAN_MS * 2.0,
                    "{}",
                    summary(&stats)
                );
            }
        }
    }

    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    fn hashes(output: &Output) -> [u64; 3] {
        [fnv(&output.color), fnv(&output.albedo), fnv(&output.normal)]
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn paced_traces_keep_the_bytes_of_eight_row_bands() {
        let _pinned = Pinned::software_pcg();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut traced = Vec::new();
        let mut glass =
            Trace::new_detailed(&gpu, &dense_glass(48, 32), &ortho(3.2, 2.0), 96, 48).unwrap();
        let mut pacer = Pacer::default();
        let mut turns = Turns::default();
        glass
            .sample_paced(&gpu, 2, 3, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        traced.push(hashes(&glass.readback(&gpu).unwrap()));
        for lit in [true, false] {
            let mut room =
                Trace::build(&gpu, &softbox_room(), &Detail::default(), 37, 29, lit).unwrap();
            room.sample(&gpu, 3, 5).unwrap();
            room.sample(&gpu, 2, 6).unwrap();
            traced.push(hashes(&room.readback(&gpu).unwrap()));
        }
        println!("hashes: {traced:x?}");
        assert_eq!(traced, EIGHT_ROW_BANDS);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_sobol_sampler_repeats_its_bytes_and_keeps_the_pcg_mean() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mean = |sampler| {
            let detail = Detail {
                sampler,
                ..Detail::default()
            };
            let mut room = Trace::build(&gpu, &softbox_room(), &detail, 64, 48, true).unwrap();
            room.sample(&gpu, 256, 9).unwrap();
            let output = room.readback(&gpu).unwrap();
            let pixels = floats(&output.color);
            let sum: f64 = pixels.iter().map(|p| f64::from(p[0] + p[1] + p[2])).sum();
            (fnv(&output.color), sum / pixels.len() as f64)
        };
        let sobol = Sampler::Sobol { samples: 256 };
        let (first, sobol_mean) = mean(sobol);
        let (again, _) = mean(sobol);
        let (_, pcg_mean) = mean(Sampler::Pcg);
        assert_eq!(first, again);
        assert!(
            (sobol_mean / pcg_mean - 1.0).abs() < 0.02,
            "{sobol_mean} against {pcg_mean}"
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_cheap_trace_returns_to_full_lanes() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut room =
            Trace::build(&gpu, &softbox_room(), &Detail::default(), 160, 96, false).unwrap();
        let mut pacer = Pacer::default();
        let mut turns = Turns::default();
        let first = room
            .sample_paced(&gpu, 1, 1, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        println!("first sample: {}, {} lanes", summary(&first), room.lanes());
        let later = room
            .sample_paced(&gpu, 8, 2, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        println!("eight more: {}, {} lanes", summary(&later), room.lanes());
        assert_eq!(room.lanes(), LANES);
        assert!(
            later.longest_ms() <= SAMPLE_SPAN_MS,
            "{}",
            later.longest_ms()
        );
        assert!(later.count() <= 16, "{}", later.count());
    }
}
