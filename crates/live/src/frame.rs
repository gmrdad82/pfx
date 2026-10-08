use crate::canopy::{Canopy, CanopyFrame, CanopyMode, Gobo, canopy_light_wgsl};
use crate::deform::{self, Deformer, DeformerBuffer, DeformerId, WindFrame};
use crate::lights::{CASTER_BYTES, CASTER_SLOTS, LIGHTS_WGSL, LightQuality, Lights, LocalLight};
use crate::lod::LOD_WGSL;
use crate::maps::{
    self, CAUSTIC_BINDING, CONTENT_BINDING, CONTENT_SLOTS, Caustic, ContentFormat, ContentSlots,
    GpuMapMaterial, MapImages, MapIndices, MapTextures,
};
use crate::pipeline_cache::{CacheLoad, PipelineCache};
use crate::probes::{
    Ambient, ContactDesc, ContactField, ContactUniform, PROBE_WGSL, ProbeLighting,
};
use crate::ranges::FreeRanges;
use crate::reflect;
use crate::shadow::{CASCADE_COUNT, Fit, SAMPLE_UNIFORM_BYTES, SHADOW_WGSL, Shadows};
use crate::skin::{
    InstancePose, NO_SKIN, PoseLayout, SKIN_BINDING, SKIN_WGSL, SkinData, SkinStore,
};
use crate::sky::{AnalyticSky, SkyPass, SkySource};
use crate::warm::{self, Arrival, Warming};
use bytemuck::{Pod, Zeroable};
use pfx_bake::detail::atlas::Atlas;
use pfx_bake::reflection::{
    ProbeRecord, ReflectionArray, ReflectionArtifact, nearest_two, upload_reflections,
};
use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_core::sky::SUN_RADIUS;
use pfx_geom::tree::{SwayVertex, TREE_SWAY_WGSL, Tree};
use pfx_gpu::{Gpu, GpuProfiler, OffscreenTarget, PassTiming};
use pfx_materials::{BRDF, Material, PackedMaterial};
use pfx_physics::BEND_WGSL;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::num::NonZeroU64;
use std::ops::{Deref, Range};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use wgpu::util::DeviceExt;

pub type Matrix = [[f32; 4]; 4];

pub const NO_SWAY: u32 = u32::MAX;
pub const SKY_EDGE: u32 = 128;
pub const CONTACT_BINDING: u32 = CONTENT_BINDING + CONTENT_SLOTS as u32 + 1;
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(crate) const MESH_SLOT_BITS: u32 = 20;
pub(crate) const MESH_SLOT_MASK: u32 = (1 << MESH_SLOT_BITS) - 1;
pub(crate) const MESH_GENERATION_LAST: u32 = u32::MAX >> MESH_SLOT_BITS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

impl MeshHandle {
    pub fn slot(self) -> usize {
        (self.0 & MESH_SLOT_MASK) as usize
    }

    pub fn generation(self) -> u32 {
        self.0 >> MESH_SLOT_BITS
    }

    fn pack(slot: usize, generation: u32) -> Self {
        Self(slot as u32 | generation << MESH_SLOT_BITS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshStats {
    pub live: usize,
    pub slots: usize,
    pub buffers: usize,
    pub glass_live: usize,
    pub glass_slots: usize,
    pub glass_buffers: usize,
}

pub struct MeshData<'a> {
    pub positions: &'a [[f32; 3]],
    pub normals: &'a [[f32; 3]],
    pub tangents: &'a [[f32; 4]],
    pub uvs: &'a [[f32; 2]],
    pub uvs1: Option<&'a [[f32; 2]]>,
    pub alpha: Option<&'a [f32]>,
    pub indices: &'a [u32],
}

pub struct MeshBuffers<'a> {
    pub vertices: &'a wgpu::Buffer,
    pub positions: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub count: u32,
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub view: Matrix,
    pub projection: Matrix,
    pub previous_view_projection: Matrix,
    pub position: [f32; 3],
}

#[derive(Clone, Copy)]
pub struct Sun {
    pub direction: [f32; 3],
    pub colour: [f32; 3],
    pub intensity: f32,
}

impl Sun {
    pub fn from_daylight(daylight: Daylight, reference_hour: f64) -> Self {
        let sun = daylight.sun();
        let light = daylight.light(reference_hour);
        Self {
            direction: sun.y_up.map(|value| value as f32),
            colour: light.colour.map(|value| value as f32),
            intensity: light.intensity as f32,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Instance {
    pub mesh: MeshHandle,
    pub model: Matrix,
    pub previous_model: Matrix,
    pub material: u32,
    pub id: u32,
    pub opacity: f32,
    pub alpha_cutoff: f32,
    pub deformer: DeformerId,
    pub age: f32,
    pub coverage: f32,
    pub shadow_only: bool,
    pub casts_shadow: bool,
    pub two_sided: bool,
}

impl Instance {
    pub fn new(mesh: MeshHandle, model: Matrix, material: u32, id: u32) -> Self {
        Self {
            mesh,
            model,
            previous_model: model,
            material,
            id,
            opacity: 1.0,
            alpha_cutoff: 0.5,
            deformer: DeformerId::NONE,
            age: 0.0,
            coverage: 1.0,
            shadow_only: false,
            casts_shadow: true,
            two_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContentFace {
    #[default]
    Both,
    Front,
    Back,
}

impl ContentFace {
    fn code(self) -> u32 {
        match self {
            Self::Both => 0,
            Self::Front => 1,
            Self::Back => 2,
        }
    }

    pub fn shows(self, front: bool) -> bool {
        match self {
            Self::Both => true,
            Self::Front => front,
            Self::Back => !front,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceSurface {
    pub clip: [[f32; 4]; 2],
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    pub crop: [f32; 4],
    pub face: ContentFace,
    pub contact: bool,
    pub authored_ambient: bool,
    pub cutout: bool,
    pub reflection_occlusion: f32,
}

impl Default for InstanceSurface {
    fn default() -> Self {
        Self {
            clip: [[0.0; 4]; 2],
            uv_offset: [0.0; 2],
            uv_scale: [1.0; 2],
            crop: [0.0, 0.0, 1.0, 1.0],
            face: ContentFace::Both,
            contact: false,
            authored_ambient: false,
            cutout: false,
            reflection_occlusion: 1.0,
        }
    }
}

impl InstanceSurface {
    pub fn clipped(&self) -> bool {
        self.clip
            .iter()
            .any(|plane| plane[..3].iter().any(|&value| value != 0.0))
    }

    pub fn cuts(&self, point: [f32; 3]) -> bool {
        self.clip.iter().any(|plane| {
            plane[..3].iter().any(|&value| value != 0.0)
                && plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] > plane[3]
        })
    }

    pub fn content_uv(&self, uv: [f32; 2]) -> Option<[f32; 2]> {
        pfx_materials::content_uv(uv, self.uv_offset, self.uv_scale, self.crop)
    }

    fn valid(&self) -> bool {
        self.clip.iter().flatten().all(|value| value.is_finite())
            && self.uv_offset.iter().all(|value| value.is_finite())
            && self.uv_scale.iter().all(|value| value.is_finite())
            && self.crop.iter().all(|value| value.is_finite())
            && (0.0..=1.0).contains(&self.reflection_occlusion)
    }

    fn occlusion_release(&self) -> u32 {
        ((1.0 - self.reflection_occlusion) * 255.0).round() as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneWind {
    pub current: WindFrame,
    pub previous: WindFrame,
}

impl Default for SceneWind {
    fn default() -> Self {
        let calm = WindFrame {
            time: 0.0,
            strength: 0.0,
            direction: [1.0, 0.0, 0.0],
        };
        Self {
            current: calm,
            previous: calm,
        }
    }
}

pub struct Scene<'a> {
    pub camera: Camera,
    pub time: f32,
    pub seed: u32,
    pub sun: Sun,
    pub instances: &'a [Instance],
    pub materials: &'a [Material],
    pub deformers: &'a [Deformer],
    pub wind: SceneWind,
}

pub struct FrameTargets {
    pub hdr: OffscreenTarget,
    pub depth: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    pub velocity: wgpu::Texture,
    pub velocity_view: wgpu::TextureView,
    pub ids: wgpu::Texture,
    pub ids_view: wgpu::TextureView,
    pub normal_roughness: wgpu::Texture,
    pub normal_roughness_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub(crate) struct CountedBuffer {
    buffer: wgpu::Buffer,
    live: Arc<AtomicUsize>,
}

impl CountedBuffer {
    pub(crate) fn new(buffer: wgpu::Buffer, live: &Arc<AtomicUsize>) -> Self {
        live.fetch_add(1, Ordering::Relaxed);
        Self {
            buffer,
            live: live.clone(),
        }
    }
}

impl Deref for CountedBuffer {
    type Target = wgpu::Buffer;

    fn deref(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

impl Drop for CountedBuffer {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Mesh {
    vertices: CountedBuffer,
    positions: CountedBuffer,
    indices: CountedBuffer,
    vertex_capacity: usize,
    index_capacity: usize,
    count: u32,
    sway: u32,
    sway_len: u32,
    skin: u32,
    skin_len: u32,
    skin_joints: u32,
}

struct MeshSlot {
    generation: u32,
    mesh: Option<Mesh>,
}

fn live_mesh(meshes: &[MeshSlot], handle: MeshHandle) -> Option<&Mesh> {
    let slot = meshes.get(handle.slot())?;
    if slot.generation == handle.generation() {
        slot.mesh.as_ref()
    } else {
        None
    }
}

fn check_mesh(data: &MeshData<'_>) -> Result<(), String> {
    let count = data.positions.len();
    if count == 0 || data.indices.is_empty() || !data.indices.len().is_multiple_of(3) {
        return Err("mesh needs vertices and triangle indices".into());
    }
    if data.normals.len() != count
        || data.tangents.len() != count
        || data.uvs.len() != count
        || data.uvs1.is_some_and(|uvs1| uvs1.len() != count)
        || data.alpha.is_some_and(|alpha| alpha.len() != count)
    {
        return Err("mesh attributes have different vertex counts".into());
    }
    if data.indices.iter().any(|&index| index as usize >= count) {
        return Err("mesh index is outside the vertex buffer".into());
    }
    Ok(())
}

fn mesh_vertices(data: &MeshData<'_>) -> Vec<Vertex> {
    (0..data.positions.len())
        .map(|index| {
            let uv1 = data.uvs1.map_or([0.0; 2], |uvs1| uvs1[index]);
            Vertex {
                position: [
                    data.positions[index][0],
                    data.positions[index][1],
                    data.positions[index][2],
                    1.0,
                ],
                normal: [
                    data.normals[index][0],
                    data.normals[index][1],
                    data.normals[index][2],
                    uv1[1],
                ],
                tangent: data.tangents[index],
                uv_alpha: [
                    data.uvs[index][0],
                    data.uvs[index][1],
                    data.alpha.map_or(1.0, |alpha| alpha[index]),
                    uv1[0],
                ],
            }
        })
        .collect()
}

fn mesh_buffer(
    device: &wgpu::Device,
    live: &Arc<AtomicUsize>,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> CountedBuffer {
    CountedBuffer::new(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        }),
        live,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub mesh: MeshHandle,
    pub two_sided: bool,
    pub moving: bool,
    pub clipped: bool,
    pub cut: bool,
    pub shadow_only: bool,
    pub casts_shadow: bool,
    pub range: Range<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 4],
    normal: [f32; 4],
    tangent: [f32; 4],
    uv_alpha: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuSway {
    pivot_level: [f32; 4],
    stiffness: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuInstance {
    model: Matrix,
    previous_model: Matrix,
    normal: [[f32; 4]; 3],
    meta: [u32; 4],
    extra: [u32; 4],
    clip: [[f32; 4]; 2],
    content_uv: [f32; 4],
    content_crop: [f32; 4],
    content_face: [u32; 4],
    skin: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_projection: Matrix,
    motion_view_projection: Matrix,
    previous_view_projection: Matrix,
    view: Matrix,
    camera_position: [f32; 4],
    sun_direction: [f32; 4],
    sun_colour_intensity: [f32; 4],
    time_size: [f32; 4],
    flags: [u32; 4],
    wind: [f32; 4],
    wind_direction: [f32; 4],
    previous_wind_direction: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyLight {
    sh: [[f32; 4]; 9],
    info: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuReflectionProbe {
    center: [f32; 3],
    layer: i32,
    box_min: [f32; 3],
    priority: f32,
    box_max: [f32; 3],
    fade: f32,
}

impl From<ProbeRecord> for GpuReflectionProbe {
    fn from(value: ProbeRecord) -> Self {
        Self {
            center: value.center,
            layer: value.layer,
            box_min: value.box_min,
            priority: value.priority,
            box_max: value.box_max,
            fade: value.fade,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReflectionSelection {
    first: GpuReflectionProbe,
    second: GpuReflectionProbe,
    weights_mip: [f32; 4],
}

fn local_candidates(records: &[ProbeRecord], camera: [f32; 3]) -> [usize; 2] {
    let chosen = nearest_two(records, camera);
    if chosen[0].1 > 0.0 {
        let second = if chosen[1].1 > 0.0 {
            chosen[1].0
        } else {
            chosen[0].0
        };
        return [chosen[0].0, second];
    }
    let mut order: Vec<(usize, f32)> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let outside = (0..3)
                .map(|axis| {
                    let below = record.box_min[axis] - camera[axis];
                    let above = camera[axis] - record.box_max[axis];
                    below.max(above).max(0.0).powi(2)
                })
                .sum::<f32>();
            (index, outside)
        })
        .collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let first = order.first().map_or(0, |entry| entry.0);
    [first, order.get(1).map_or(first, |entry| entry.0)]
}

struct LocalReflections {
    artifacts: Vec<ReflectionArtifact>,
    array: ReflectionArray,
}

struct Foliage {
    canopy: Canopy,
    origin: [f32; 3],
    receiver_y: f32,
    id: u32,
    prepass: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OpaqueFeatures {
    pub sun_shadow: bool,
    pub soft_search: bool,
    pub soft_filter: bool,
    pub cascade_blend: bool,
    pub probes: bool,
    pub local_reflections: bool,
    pub ssr: bool,
    pub canopy: bool,
    pub contact: bool,
    pub authored_ambient: bool,
    pub caustics: bool,
    pub content: bool,
    pub maps: bool,
    pub detail: bool,
    pub ageing: bool,
    pub fine_noise: bool,
    pub uv1: bool,
    pub content_cutout: bool,
    pub lacquer: bool,
    pub procedural_lacquer: bool,
    pub scratch: bool,
    pub grime: bool,
    pub wood: bool,
    pub wall: bool,
    pub fibre: bool,
    pub crinkle: bool,
    pub procedural_detail: bool,
    pub film: bool,
}

const OPAQUE_FEATURES: usize = 28;

impl OpaqueFeatures {
    pub const NAMES: [&'static str; OPAQUE_FEATURES] = [
        "sun_shadow",
        "soft_search",
        "soft_filter",
        "cascade_blend",
        "probes",
        "local_reflections",
        "ssr",
        "canopy",
        "contact",
        "authored_ambient",
        "caustics",
        "content",
        "maps",
        "detail",
        "ageing",
        "fine_noise",
        "uv1",
        "content_cutout",
        "lacquer",
        "procedural_lacquer",
        "scratch",
        "grime",
        "wood",
        "wall",
        "fibre",
        "crinkle",
        "procedural_detail",
        "film",
    ];

    pub const ALL: Self = Self::from_flags([true; OPAQUE_FEATURES]);

    pub const NONE: Self = Self::from_flags([false; OPAQUE_FEATURES]);

    pub const fn from_flags(flags: [bool; OPAQUE_FEATURES]) -> Self {
        Self {
            sun_shadow: flags[0],
            soft_search: flags[1],
            soft_filter: flags[2],
            cascade_blend: flags[3],
            probes: flags[4],
            local_reflections: flags[5],
            ssr: flags[6],
            canopy: flags[7],
            contact: flags[8],
            authored_ambient: flags[9],
            caustics: flags[10],
            content: flags[11],
            maps: flags[12],
            detail: flags[13],
            ageing: flags[14],
            fine_noise: flags[15],
            uv1: flags[16],
            content_cutout: flags[17],
            lacquer: flags[18],
            procedural_lacquer: flags[19],
            scratch: flags[20],
            grime: flags[21],
            wood: flags[22],
            wall: flags[23],
            fibre: flags[24],
            crinkle: flags[25],
            procedural_detail: flags[26],
            film: flags[27],
        }
    }

    pub const fn flags(self) -> [bool; OPAQUE_FEATURES] {
        [
            self.sun_shadow,
            self.soft_search,
            self.soft_filter,
            self.cascade_blend,
            self.probes,
            self.local_reflections,
            self.ssr,
            self.canopy,
            self.contact,
            self.authored_ambient,
            self.caustics,
            self.content,
            self.maps,
            self.detail,
            self.ageing,
            self.fine_noise,
            self.uv1,
            self.content_cutout,
            self.lacquer,
            self.procedural_lacquer,
            self.scratch,
            self.grime,
            self.wood,
            self.wall,
            self.fibre,
            self.crinkle,
            self.procedural_detail,
            self.film,
        ]
    }

    pub fn named(name: &str) -> Option<Self> {
        let index = Self::NAMES.iter().position(|known| *known == name)?;
        let mut flags = [false; OPAQUE_FEATURES];
        flags[index] = true;
        Some(Self::from_flags(flags))
    }

    pub fn union(self, other: Self) -> Self {
        let (a, b) = (self.flags(), other.flags());
        Self::from_flags(std::array::from_fn(|i| a[i] || b[i]))
    }

    pub fn without(self, other: Self) -> Self {
        let (a, b) = (self.flags(), other.flags());
        Self::from_flags(std::array::from_fn(|i| a[i] && !b[i]))
    }

    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .zip(self.flags())
            .filter_map(|(name, on)| on.then_some(*name))
            .collect()
    }

    fn constants(self) -> Vec<(String, f64)> {
        Self::NAMES
            .iter()
            .zip(self.flags())
            .map(|(name, on)| (format!("use_{name}"), f64::from(u8::from(on))))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OpaqueKey {
    pub features: OpaqueFeatures,
    pub lit: bool,
    pub two_sided: bool,
}

impl OpaqueKey {
    pub fn both_sides(features: OpaqueFeatures, lit: bool) -> [Self; 2] {
        [false, true].map(|two_sided| Self {
            features,
            lit,
            two_sided,
        })
    }
}

pub(crate) fn opaque_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    cache: Option<&wgpu::PipelineCache>,
    key: OpaqueKey,
) -> wgpu::RenderPipeline {
    let constants = key.features.constants();
    let constants: Vec<(&str, f64)> = constants
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();
    let buffers = [vertex_layout()];
    pipeline_with(
        device,
        PipelineSpec {
            label: if key.lit {
                "live opaque PBR with local lights"
            } else {
                "live opaque PBR"
            },
            layout,
            module,
            vertex: "vertex_main",
            fragment: Some(if key.lit {
                "opaque_lit_main"
            } else {
                "opaque_main"
            }),
            buffers: &buffers,
            targets: &opaque_targets(),
            cull: (!key.two_sided).then_some(wgpu::Face::Back),
            write_depth: false,
            cache,
        },
        &constants,
    )
}

struct CutPipelines {
    prepass: [wgpu::RenderPipeline; 2],
    shadow: wgpu::RenderPipeline,
}

struct Pipelines {
    prepass: [wgpu::RenderPipeline; 2],
    shadow: [wgpu::RenderPipeline; 2],
    cut: Option<CutPipelines>,
    cut_layout: wgpu::PipelineLayout,
    transmit: wgpu::RenderPipeline,
    layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    opaque: HashMap<OpaqueKey, wgpu::RenderPipeline>,
    cache: Option<PipelineCache>,
    pending: HashSet<OpaqueKey>,
    arrivals: Receiver<Arrival>,
    sender: Sender<Arrival>,
    made_here: usize,
    workers: usize,
}

impl Pipelines {
    fn collect(&mut self) {
        while let Ok(arrival) = self.arrivals.try_recv() {
            self.arrive(arrival);
        }
    }

    fn retarget(&mut self, module: wgpu::ShaderModule) {
        while !self.pending.is_empty() {
            match self.arrivals.recv() {
                Ok((key, _)) => {
                    self.pending.remove(&key);
                }
                Err(_) => break,
            }
        }
        self.opaque.clear();
        self.module = module;
    }

    fn arrive(&mut self, (key, made): Arrival) {
        self.pending.remove(&key);
        if let Some(made) = made {
            self.opaque.entry(key).or_insert(made);
        }
    }

    fn ensure_cut(&mut self, device: &wgpu::Device) {
        if self.cut.is_some() {
            return;
        }
        let buffers = [vertex_layout()];
        let prepass = prepass_pipelines(device, &self.layout, &self.module, true);
        let shadow = pipeline(
            device,
            PipelineSpec {
                label: "live cut shadow casters",
                layout: &self.cut_layout,
                module: &self.module,
                vertex: "shadow_cut_vertex",
                fragment: Some("shadow_cut_fragment"),
                buffers: &buffers,
                targets: &[],
                cull: None,
                write_depth: true,
                cache: None,
            },
        );
        self.cut = Some(CutPipelines { prepass, shadow });
    }

    fn ensure_opaque(&mut self, device: &wgpu::Device, key: OpaqueKey) {
        self.collect();
        while !self.opaque.contains_key(&key) && self.pending.contains(&key) {
            match self.arrivals.recv() {
                Ok(arrival) => self.arrive(arrival),
                Err(_) => break,
            }
        }
        if self.opaque.contains_key(&key) {
            return;
        }
        let made = opaque_pipeline(
            device,
            &self.layout,
            &self.module,
            self.cache.as_ref().map(PipelineCache::cache),
            key,
        );
        self.made_here += 1;
        self.opaque.insert(key, made);
    }

    fn warm(&mut self, device: &wgpu::Device, first: &[OpaqueKey], later: &[OpaqueKey]) -> Warming {
        self.collect();
        let mut queued: Vec<OpaqueKey> = Vec::new();
        let mut first_count = 0;
        for (index, key) in first.iter().chain(later).enumerate() {
            if self.opaque.contains_key(key) || self.pending.contains(key) || queued.contains(key) {
                continue;
            }
            queued.push(*key);
            if index < first.len() {
                first_count += 1;
            }
        }
        self.pending.extend(queued.iter().copied());
        warm::spawn(
            warm::Job {
                device: device.clone(),
                layout: self.layout.clone(),
                module: self.module.clone(),
                cache: self.cache.as_ref().map(|cache| cache.cache().clone()),
            },
            queued,
            first_count,
            self.workers,
            self.sender.clone(),
        )
    }
}

struct Fallbacks {
    black: wgpu::TextureView,
    local_probes: wgpu::TextureView,
    canopy_params: wgpu::Buffer,
    linear: wgpu::Sampler,
}

pub struct Frame {
    pub gpu: Gpu,
    pub targets: FrameTargets,
    meshes: Vec<MeshSlot>,
    free_meshes: Vec<usize>,
    mesh_buffers_live: Arc<AtomicUsize>,
    mesh_epoch: u64,
    sway_free: FreeRanges,
    uniform: wgpu::Buffer,
    instances: wgpu::Buffer,
    materials: wgpu::Buffer,
    map_materials: wgpu::Buffer,
    sway: wgpu::Buffer,
    sway_data: Vec<GpuSway>,
    skins: SkinStore,
    instance_capacity: usize,
    material_capacity: usize,
    scene_layout: wgpu::BindGroupLayout,
    scene_group: wgpu::BindGroup,
    shadow_group: wgpu::BindGroup,
    shadow_layout: wgpu::BindGroupLayout,
    shadow_uniform: wgpu::Buffer,
    _shadow_atlas: wgpu::Texture,
    _shadow_view: wgpu::TextureView,
    _shadow_sampler: wgpu::Sampler,
    cascade_uniform: wgpu::Buffer,
    cascade_group: wgpu::BindGroup,
    cascade_layout: wgpu::BindGroupLayout,
    cascade_align: u32,
    lights: Lights,
    lighting_layout: wgpu::BindGroupLayout,
    lighting_group: Option<wgpu::BindGroup>,
    sky_light: wgpu::Buffer,
    ambient: Ambient,
    ambient_light: wgpu::Buffer,
    contact: Option<ContactField>,
    contact_uniform: wgpu::Buffer,
    reflection_selection: wgpu::Buffer,
    local_reflections: RefCell<Option<LocalReflections>>,
    deformers: DeformerBuffer,
    maps: MapTextures,
    map_layers: [usize; 4],
    content: RefCell<ContentSlots>,
    content_epoch: Cell<u64>,
    caustic: RefCell<Option<Caustic>>,
    surfaces: RefCell<Vec<InstanceSurface>>,
    probes: ProbeLighting,
    sky: SkyPass,
    ssr: Option<reflect::Reflections>,
    viewport: [u32; 2],
    foliage: Option<Foliage>,
    fallbacks: Fallbacks,
    pipelines: Pipelines,
    canopy_gobo: Gobo,
    shadow_sample: [u8; SAMPLE_UNIFORM_BYTES],
    instance_features: Vec<OpaqueFeatures>,
    reflections_planned: bool,
    opaque_on: Cell<OpaqueFeatures>,
    opaque_off: Cell<OpaqueFeatures>,
    opaque_drawn: Vec<OpaqueFeatures>,
    jitter: [f32; 2],
    profiler: GpuProfiler,
}

pub fn multiply(a: Matrix, b: Matrix) -> Matrix {
    let mut result = [[0.0; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            result[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    result
}

pub fn transform(matrix: Matrix, point: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| {
        (0..4)
            .map(|column| matrix[column][row] * point[column])
            .sum()
    })
}

pub fn velocity_uv(current: [f32; 4], previous: [f32; 4]) -> [f32; 2] {
    if current[3].abs() < 1e-8 || previous[3].abs() < 1e-8 {
        return [0.0; 2];
    }
    [
        (previous[0] / previous[3] - current[0] / current[3]) * 0.5,
        (current[1] / current[3] - previous[1] / previous[3]) * 0.5,
    ]
}

pub fn velocity_pixels(current: [f32; 4], previous: [f32; 4], size: [f32; 2]) -> [f32; 2] {
    let uv = velocity_uv(current, previous);
    [uv[0] * size[0], uv[1] * size[1]]
}

pub fn jitter_matrix(jitter: [f32; 2]) -> Matrix {
    let mut shift = [[0.0; 4]; 4];
    for (i, column) in shift.iter_mut().enumerate() {
        column[i] = 1.0;
    }
    shift[3][0] = jitter[0];
    shift[3][1] = jitter[1];
    shift
}

fn same_bits(a: Matrix, b: Matrix) -> bool {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .all(|(x, y)| x.to_bits() == y.to_bits())
}

pub fn batches(
    instances: &[Instance],
    swaying: impl Fn(MeshHandle) -> bool,
    clipped: impl Fn(usize) -> bool,
) -> Vec<Batch> {
    surface_batches(instances, swaying, clipped, |_| false)
}

pub fn surface_batches(
    instances: &[Instance],
    swaying: impl Fn(MeshHandle) -> bool,
    clipped: impl Fn(usize) -> bool,
    cut: impl Fn(usize) -> bool,
) -> Vec<Batch> {
    let mut out: Vec<Batch> = Vec::new();
    for (index, instance) in instances.iter().enumerate() {
        let two_sided = instance.two_sided || instance.deformer != DeformerId::NONE;
        let moving = instance.model != instance.previous_model
            || instance.deformer != DeformerId::NONE
            || swaying(instance.mesh);
        let clipped = clipped(index);
        let cut = cut(index);
        let index = index as u32;
        if let Some(last) = out.last_mut()
            && last.mesh == instance.mesh
            && last.two_sided == two_sided
            && last.moving == moving
            && last.clipped == clipped
            && last.cut == cut
            && last.shadow_only == instance.shadow_only
            && last.casts_shadow == instance.casts_shadow
            && last.range.end == index
        {
            last.range.end += 1;
            continue;
        }
        out.push(Batch {
            mesh: instance.mesh,
            two_sided,
            moving,
            clipped,
            cut,
            shadow_only: instance.shadow_only,
            casts_shadow: instance.casts_shadow,
            range: index..index + 1,
        });
    }
    out
}

fn opaque_instance_features(
    instance: &GpuInstance,
    material: &GpuMapMaterial,
    filmed: bool,
) -> OpaqueFeatures {
    let packed = material.seeds[2] == 3 && material.indices[3] >= 0;
    let unbaked = material.seeds[3] == 0;
    let layered = material
        .layers
        .iter()
        .any(|layer| layer[0] != 0.0 && layer[2] != 0.0);
    let bump = material.seeds[2] == 1 && material.factors[3] != 0.0;
    let lacquered = !packed
        && material
            .layers
            .iter()
            .any(|layer| layer[0] == 18.0 && layer[2] != 0.0);
    let carries = |kinds: &[f32]| {
        unbaked
            && !packed
            && material
                .layers
                .iter()
                .any(|layer| kinds.contains(&layer[0]) && layer[2] != 0.0)
    };
    let unplaced = material.content[0] < -0.5;
    OpaqueFeatures {
        contact: instance.content_face[2] & 1 != 0,
        caustics: instance.content_face[2] & 2 != 0,
        authored_ambient: instance.content_face[3] != 0,
        content: !unplaced,
        maps: material.indices.iter().any(|&index| index >= 0),
        detail: unbaked && ((!packed && layered) || bump),
        lacquer: unbaked && (lacquered || bump),
        scratch: carries(&[17.0]),
        grime: carries(&[5.0]),
        wood: carries(&[3.0]),
        wall: carries(&[4.0]),
        fibre: carries(&[1.0, 11.0]),
        crinkle: carries(&[2.0]),
        ageing: f32::from_bits(instance.extra[2]) > 0.0,
        uv1: material.seeds[3] >= 2 && material.packed[1] != 0.0,
        content_cutout: instance.content_face[2] & 4 != 0,
        film: filmed,
        ..OpaqueFeatures::NONE
    }
}

fn scene_instance_features(
    packed: &[GpuInstance],
    map_materials: &[GpuMapMaterial],
    materials: &[Material],
) -> Vec<OpaqueFeatures> {
    packed
        .iter()
        .map(|instance| {
            let index = instance.meta[0] as usize;
            opaque_instance_features(
                instance,
                &map_materials[index],
                materials[index].thin_film_amount > 0.0,
            )
        })
        .collect()
}

pub fn shadow_batches(batches: &[Batch]) -> Vec<&Batch> {
    batches.iter().filter(|batch| batch.casts_shadow).collect()
}

pub fn transmission_batches<'a>(
    batches: &'a [Batch],
    instances: &[Instance],
    transmissive: impl Fn(u32) -> bool,
) -> Vec<&'a Batch> {
    batches
        .iter()
        .filter(|batch| {
            batch.casts_shadow
                && batch
                    .range
                    .clone()
                    .any(|index| transmissive(instances[index as usize].material))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn draw_casters(
    pass: &mut wgpu::RenderPass<'_>,
    casters: &[&Batch],
    statics: bool,
    deformers: &wgpu::BindGroup,
    lighting: Option<&wgpu::BindGroup>,
    pipelines: &Pipelines,
    meshes: &[MeshSlot],
) {
    pass.set_bind_group(2, deformers, &[]);
    if casters.iter().any(|batch| batch.cut)
        && let Some(lighting) = lighting
    {
        pass.set_bind_group(3, lighting, &[]);
    }
    let mut current = None;
    for batch in casters.iter().filter(|batch| batch.moving != statics) {
        let kind = if batch.cut {
            2
        } else {
            usize::from(batch.clipped)
        };
        if current != Some(kind) {
            pass.set_pipeline(match &pipelines.cut {
                Some(cut) if batch.cut => &cut.shadow,
                _ => &pipelines.shadow[usize::from(batch.clipped)],
            });
            current = Some(kind);
        }
        let Some(mesh) = live_mesh(meshes, batch.mesh) else {
            continue;
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.count, 0, batch.range.clone());
    }
}

fn content_row_key(material: &Material) -> (bool, i32) {
    let layer = material.content_layer;
    (layer.active(), layer.slot)
}

fn mix_key(key: u64, other: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    other.hash(&mut hasher);
    hasher.finish()
}

pub fn static_key(instances: &[Instance], batches: &[Batch], surfaces: &[InstanceSurface]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for batch in shadow_batches(batches)
        .into_iter()
        .filter(|batch| !batch.moving)
    {
        batch.mesh.hash(&mut hasher);
        for index in batch.range.start as usize..batch.range.end as usize {
            for value in instances[index].model.iter().flatten() {
                value.to_bits().hash(&mut hasher);
            }
            if batch.clipped {
                for value in surfaces[index].clip.iter().flatten() {
                    value.to_bits().hash(&mut hasher);
                }
            }
        }
    }
    hasher.finish()
}

pub fn sky_irradiance(sh: &[[f32; 3]; 9], normal: [f32; 3]) -> [f32; 3] {
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
        .sqrt()
        .max(1e-8);
    let d = normal.map(|value| value / length);
    let basis = [
        0.282095,
        0.488603 * d[1],
        0.488603 * d[2],
        0.488603 * d[0],
        1.092548 * d[0] * d[1],
        1.092548 * d[1] * d[2],
        0.315392 * (3.0 * d[2] * d[2] - 1.0),
        1.092548 * d[0] * d[2],
        0.546274 * (d[0] * d[0] - d[1] * d[1]),
    ];
    [0, 1, 2].map(|channel| {
        (0..9)
            .map(|i| sh[i][channel] * basis[i])
            .sum::<f32>()
            .max(0.0)
    })
}

pub fn ambient_irradiance(
    grid: Option<[f32; 3]>,
    sh: &[[f32; 3]; 9],
    normal: [f32; 3],
) -> [f32; 3] {
    match grid {
        Some(value) if value[0] >= 0.0 => value,
        _ => sky_irradiance(sh, normal),
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normal_columns(model: Matrix) -> Result<[[f32; 4]; 3], String> {
    let a = [model[0][0], model[0][1], model[0][2]];
    let b = [model[1][0], model[1][1], model[1][2]];
    let c = [model[2][0], model[2][1], model[2][2]];
    let columns = [cross(b, c), cross(c, a), cross(a, b)];
    let determinant = a.iter().zip(columns[0]).map(|(x, y)| x * y).sum::<f32>();
    if !determinant.is_finite() || determinant.abs() < 1e-8 {
        return Err("instance model has a singular normal transform".into());
    }
    Ok(columns.map(|v| {
        [
            v[0] / determinant,
            v[1] / determinant,
            v[2] / determinant,
            0.0,
        ]
    }))
}

pub fn shader_source() -> String {
    shader_source_with(&Gobo::default())
}

pub fn shader_source_with(gobo: &Gobo) -> String {
    [
        BRDF,
        SHADOW_WGSL,
        LOD_WGSL,
        &deform_source(),
        SKIN_WGSL,
        TREE_SWAY_WGSL,
        pfx_materials::NOISE,
        pfx_materials::AGEING,
        maps::MAPS_WGSL,
        &maps::content_wgsl(),
        PROBE_WGSL,
        reflect::PROBE_WGSL,
        &canopy_light_wgsl(gobo),
        SHADER,
        LIGHTS_WGSL,
    ]
    .join("\n")
}

pub fn deform_source() -> String {
    format!(
        "{}\n{}\n{}",
        pfx_geom::strip::ROLL_WGSL,
        BEND_WGSL.replace("pcg(", "bend_pcg("),
        deform::DEFORM_WGSL
    )
}

pub fn card_source() -> String {
    format!("{SHADOW_WGSL}\n{LOD_WGSL}\n{CARD_SHADER}")
}

fn target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn storage(device: &wgpu::Device, label: &str, bytes: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

pub fn scene_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let read = wgpu::BufferBindingType::Storage { read_only: true };
    vec![
        buffer_entry(
            0,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BufferBindingType::Uniform,
        ),
        buffer_entry(1, wgpu::ShaderStages::VERTEX_FRAGMENT, read),
        buffer_entry(2, wgpu::ShaderStages::FRAGMENT, read),
        buffer_entry(3, wgpu::ShaderStages::VERTEX, read),
        buffer_entry(SKIN_BINDING, wgpu::ShaderStages::VERTEX, read),
    ]
}

pub fn shadow_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        buffer_entry(
            0,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::BufferBindingType::Uniform,
        ),
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        buffer_entry(
            5,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::BufferBindingType::Uniform,
        ),
        wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
    ]
}

pub fn cascade_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: NonZeroU64::new(CASTER_BYTES),
            },
            count: None,
        },
        buffer_entry(
            8,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BufferBindingType::Storage { read_only: true },
        ),
    ]
}

pub fn deformer_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    (0..2)
        .map(|binding| {
            buffer_entry(
                binding,
                wgpu::ShaderStages::VERTEX,
                wgpu::BufferBindingType::Storage { read_only: true },
            )
        })
        .collect()
}

pub fn card_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        buffer_entry(
            0,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BufferBindingType::Uniform,
        ),
        buffer_entry(
            1,
            wgpu::ShaderStages::VERTEX,
            wgpu::BufferBindingType::Storage { read_only: true },
        ),
        texture_entry(2, wgpu::TextureViewDimension::D2),
        sampler_entry(3),
    ]
}

pub fn lighting_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let read = wgpu::BufferBindingType::Storage { read_only: true };
    let uniform = wgpu::BufferBindingType::Uniform;
    let fragment = wgpu::ShaderStages::FRAGMENT;
    vec![
        texture_entry(0, wgpu::TextureViewDimension::D2Array),
        texture_entry(1, wgpu::TextureViewDimension::D2Array),
        texture_entry(2, wgpu::TextureViewDimension::D2Array),
        texture_entry(3, wgpu::TextureViewDimension::D2Array),
        sampler_entry(4),
        buffer_entry(5, fragment, read),
        buffer_entry(6, fragment, read),
        buffer_entry(7, fragment, read),
        buffer_entry(8, fragment, uniform),
        texture_entry(9, wgpu::TextureViewDimension::D2),
        sampler_entry(10),
        buffer_entry(11, fragment, uniform),
        buffer_entry(12, fragment, uniform),
        texture_entry(13, wgpu::TextureViewDimension::Cube),
        texture_entry(14, wgpu::TextureViewDimension::CubeArray),
        sampler_entry(15),
        texture_entry(16, wgpu::TextureViewDimension::D2),
        buffer_entry(17, fragment, uniform),
        texture_entry(CONTENT_BINDING, wgpu::TextureViewDimension::D2),
        texture_entry(CONTENT_BINDING + 1, wgpu::TextureViewDimension::D2),
        texture_entry(CONTENT_BINDING + 2, wgpu::TextureViewDimension::D2),
        texture_entry(CONTENT_BINDING + 3, wgpu::TextureViewDimension::D2),
        sampler_entry(CONTENT_BINDING + CONTENT_SLOTS as u32),
        texture_entry(CONTACT_BINDING, wgpu::TextureViewDimension::D2),
        buffer_entry(CONTACT_BINDING + 1, fragment, uniform),
        buffer_entry(CONTACT_BINDING + 2, fragment, uniform),
        texture_entry(reflect::GUIDE_BINDING, wgpu::TextureViewDimension::D2),
        buffer_entry(CAUSTIC_BINDING, fragment, uniform),
    ]
}

fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Float32x4,
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

fn prepass_targets() -> [Option<wgpu::ColorTargetState>; 2] {
    [
        Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rg16Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::R32Uint,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
    ]
}

fn opaque_targets() -> [Option<wgpu::ColorTargetState>; 2] {
    [
        Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: NORMAL_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        }),
    ]
}

struct PipelineSpec<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vertex: &'a str,
    fragment: Option<&'a str>,
    buffers: &'a [wgpu::VertexBufferLayout<'a>],
    targets: &'a [Option<wgpu::ColorTargetState>],
    cull: Option<wgpu::Face>,
    write_depth: bool,
    cache: Option<&'a wgpu::PipelineCache>,
}

fn prepass_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    cut: bool,
) -> [wgpu::RenderPipeline; 2] {
    let buffers = [vertex_layout()];
    [Some(wgpu::Face::Back), None].map(|cull| {
        pipeline_with(
            device,
            PipelineSpec {
                label: "live depth and velocity",
                layout,
                module,
                vertex: "vertex_main",
                fragment: Some("prepass_main"),
                buffers: &buffers,
                targets: &prepass_targets(),
                cull,
                write_depth: true,
                cache: None,
            },
            &[("use_content_cutout", f64::from(u8::from(cut)))],
        )
    })
}

fn pipeline(device: &wgpu::Device, spec: PipelineSpec<'_>) -> wgpu::RenderPipeline {
    pipeline_with(device, spec, &[])
}

fn pipeline_with(
    device: &wgpu::Device,
    spec: PipelineSpec<'_>,
    constants: &[(&str, f64)],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.module,
            entry_point: Some(spec.vertex),
            buffers: spec.buffers,
            compilation_options: Default::default(),
        },
        primitive: wgpu::PrimitiveState {
            cull_mode: spec.cull,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: spec.write_depth,
            depth_compare: if spec.write_depth {
                wgpu::CompareFunction::Less
            } else {
                wgpu::CompareFunction::LessEqual
            },
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: spec.fragment.map(|entry| wgpu::FragmentState {
            module: spec.module,
            entry_point: Some(entry),
            targets: spec.targets,
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
        }),
        multiview: None,
        cache: spec.cache,
    })
}

fn default_sky() -> SkySource {
    SkySource::Analytic(AnalyticSky::new(
        Daylight {
            hour: 12.0,
            day: Daylight::DAY,
            latitude: Daylight::LATITUDE,
            heading: Daylight::HEADING,
        },
        REFERENCE_HOUR,
        2.5,
        [0.3; 3],
    ))
}

fn new_targets(gpu: &Gpu, width: u32, height: u32) -> Result<FrameTargets, String> {
    let hdr = gpu.offscreen(width, height, wgpu::TextureFormat::Rgba16Float)?;
    let device = &gpu.device;
    let (depth, depth_view) = target(
        device,
        width,
        height,
        wgpu::TextureFormat::Depth32Float,
        "live depth",
    );
    let (velocity, velocity_view) = target(
        device,
        width,
        height,
        wgpu::TextureFormat::Rg16Float,
        "live velocity",
    );
    let (ids, ids_view) = target(
        device,
        width,
        height,
        wgpu::TextureFormat::R32Uint,
        "live ids",
    );
    let (normal_roughness, normal_roughness_view) =
        target(device, width, height, NORMAL_FORMAT, "live normals");
    Ok(FrameTargets {
        hdr,
        depth,
        depth_view,
        velocity,
        velocity_view,
        ids,
        ids_view,
        normal_roughness,
        normal_roughness_view,
        width,
        height,
    })
}

fn solid(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: u32,
    dimension: wgpu::TextureViewDimension,
    label: &str,
) -> wgpu::TextureView {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &vec![0; 8 * layers as usize],
    );
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(dimension),
        ..Default::default()
    })
}

impl Frame {
    pub fn new(gpu: Gpu, width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("viewport is empty".into());
        }
        let targets = new_targets(&gpu, width, height)?;
        let device = &gpu.device;
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live frame uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = storage(device, "live instances", std::mem::size_of::<GpuInstance>());
        let materials = storage(device, "live materials", PackedMaterial::SIZE);
        let map_materials = storage(
            device,
            "live map materials",
            std::mem::size_of::<GpuMapMaterial>(),
        );
        let sway = storage(device, "live sway", std::mem::size_of::<GpuSway>());
        let skins = SkinStore::new(device);
        let scene_layout = layout(device, "live scene layout", &scene_entries());
        let scene_group = Self::make_scene_group(
            device,
            &scene_layout,
            [&uniform, &instances, &materials, &sway, skins.buffer()],
        );
        let shadow_layout = layout(device, "live shadow seam", &shadow_entries());
        let mut fallback_sample = [0u8; SAMPLE_UNIFORM_BYTES];
        fallback_sample[60..64].copy_from_slice(&1.0f32.to_le_bytes());
        let shadow_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live shadow sample"),
            contents: &fallback_sample,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live fallback shadow"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 3,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let shadow_view = shadow_atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let lights = Lights::new(device);
        let shadow_group = Self::make_shadow_group(
            device,
            &shadow_layout,
            &shadow_uniform,
            &shadow_view,
            &shadow_sampler,
            lights.empty_transmission(),
            &lights,
        );
        let cascade_layout = layout(device, "live cascade", &cascade_entries());
        let cascade_align = device.limits().min_uniform_buffer_offset_alignment.max(256);
        let cascade_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live cascades"),
            size: u64::from(cascade_align) * CASTER_SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cascade_group =
            Self::make_cascade_group(device, &cascade_layout, &cascade_uniform, lights.table());
        let lighting_layout = layout(device, "live lighting", &lighting_entries());
        let sky_light = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live sky light"),
            size: std::mem::size_of::<SkyLight>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ambient_light = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live authored ambient"),
            size: std::mem::size_of::<SkyLight>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let contact_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live contact field"),
            contents: bytemuck::bytes_of(&ContactUniform::off()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let reflection_selection = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live reflection selection"),
            size: std::mem::size_of::<ReflectionSelection>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let deformers = DeformerBuffer::new(device);
        let maps = MapTextures::new(device, &gpu.queue, &MapImages::default())?;
        let content = ContentSlots::new(device, &gpu.queue);
        let probes = ProbeLighting::fallback(device, &gpu.queue, [-1.0; 3]);
        let mut sky = SkyPass::new(
            device,
            &gpu.queue,
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::TextureFormat::Depth32Float),
            default_sky(),
            SKY_EDGE,
        );
        while !sky.lighting.complete() {
            sky.update(
                &gpu.queue,
                [0.0, 0.0, -1.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            );
        }
        let fallbacks = Fallbacks {
            black: solid(
                device,
                &gpu.queue,
                1,
                wgpu::TextureViewDimension::D2,
                "live black",
            ),
            local_probes: solid(
                device,
                &gpu.queue,
                6,
                wgpu::TextureViewDimension::CubeArray,
                "live local probes",
            ),
            canopy_params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("live canopy fallback"),
                size: 96,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            }),
            linear: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("live reflection sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        };
        let pipelines = Self::make_pipelines(
            device,
            [
                &scene_layout,
                &shadow_layout,
                &deformers.layout,
                &lighting_layout,
            ],
            &cascade_layout,
        );
        let profiler = GpuProfiler::new(device, &gpu.queue);
        Ok(Self {
            targets,
            meshes: Vec::new(),
            uniform,
            instances,
            materials,
            map_materials,
            sway,
            sway_data: Vec::new(),
            skins,
            free_meshes: Vec::new(),
            mesh_buffers_live: Arc::new(AtomicUsize::new(0)),
            mesh_epoch: 0,
            sway_free: FreeRanges::default(),
            instance_capacity: 1,
            material_capacity: 1,
            scene_layout,
            scene_group,
            shadow_group,
            shadow_layout,
            shadow_uniform,
            _shadow_atlas: shadow_atlas,
            _shadow_view: shadow_view,
            _shadow_sampler: shadow_sampler,
            cascade_uniform,
            cascade_group,
            cascade_layout,
            cascade_align,
            lights,
            lighting_layout,
            lighting_group: None,
            sky_light,
            ambient: Ambient::Sky,
            ambient_light,
            contact: None,
            contact_uniform,
            reflection_selection,
            local_reflections: RefCell::new(None),
            deformers,
            maps,
            map_layers: [0; 4],
            content: RefCell::new(content),
            content_epoch: Cell::new(0),
            caustic: RefCell::new(None),
            surfaces: RefCell::new(Vec::new()),
            probes,
            sky,
            ssr: None,
            viewport: [width, height],
            foliage: None,
            fallbacks,
            pipelines,
            canopy_gobo: Gobo::default(),
            shadow_sample: fallback_sample,
            instance_features: Vec::new(),
            reflections_planned: false,
            opaque_on: Cell::new(OpaqueFeatures::NONE),
            opaque_off: Cell::new(OpaqueFeatures::NONE),
            opaque_drawn: Vec::new(),
            jitter: [0.0; 2],
            profiler,
            gpu,
        })
    }

    fn make_pipelines(
        device: &wgpu::Device,
        layouts: [&wgpu::BindGroupLayout; 4],
        cascade: &wgpu::BindGroupLayout,
    ) -> Pipelines {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live frame layout"),
            bind_group_layouts: &layouts,
            push_constant_ranges: &[],
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live shadow layout"),
            bind_group_layouts: &[layouts[0], cascade, layouts[2]],
            push_constant_ranges: &[],
        });
        let cut_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live cut shadow layout"),
            bind_group_layouts: &[layouts[0], cascade, layouts[2], layouts[3]],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live frame shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let buffers = [vertex_layout()];
        let prepass = prepass_pipelines(device, &pipeline_layout, &module, false);
        let shadow = [
            ("shadow_vertex", None),
            ("shadow_clip_vertex", Some("shadow_clip_fragment")),
        ]
        .map(|(vertex, fragment)| {
            pipeline(
                device,
                PipelineSpec {
                    label: "live shadow casters",
                    layout: &shadow_layout,
                    module: &module,
                    vertex,
                    fragment,
                    buffers: &buffers,
                    targets: &[],
                    cull: None,
                    write_depth: true,
                    cache: None,
                },
            )
        });
        let transmit = pipeline(
            device,
            PipelineSpec {
                label: "live transmissive casters",
                layout: &shadow_layout,
                module: &module,
                vertex: "transmit_vertex",
                fragment: Some("transmit_fragment"),
                buffers: &buffers,
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::shadow::TRANSMISSION_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                cull: None,
                write_depth: true,
                cache: None,
            },
        );
        let (sender, arrivals) = channel();
        Pipelines {
            prepass,
            shadow,
            cut: None,
            cut_layout,
            transmit,
            layout: pipeline_layout,
            module,
            opaque: HashMap::new(),
            cache: None,
            pending: HashSet::new(),
            arrivals,
            sender,
            made_here: 0,
            workers: warm::default_workers(),
        }
    }

    fn make_cascade_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        tints: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live cascades"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: uniform,
                        offset: 0,
                        size: NonZeroU64::new(CASTER_BYTES),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: tints.as_entire_binding(),
                },
            ],
        })
    }

    fn make_scene_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffers: [&wgpu::Buffer; 5],
    ) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live scene"),
            layout,
            entries: &entries,
        })
    }

    fn rebuild_scene_group(&mut self) {
        self.scene_group = Self::make_scene_group(
            &self.gpu.device,
            &self.scene_layout,
            [
                &self.uniform,
                &self.instances,
                &self.materials,
                &self.sway,
                self.skins.buffer(),
            ],
        );
    }

    fn make_shadow_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        transmission: &wgpu::TextureView,
        lights: &Lights,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live shadow sample"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(transmission),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: lights.uniform().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(lights.faces_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(lights.sampler()),
                },
            ],
        })
    }

    pub fn set_shadows(&mut self, shadows: &Shadows, fit: &Fit) {
        self.shadow_sample = shadows.sample_uniform(fit);
        self.gpu
            .queue
            .write_buffer(&self.shadow_uniform, 0, &self.shadow_sample);
        self.shadow_group = Self::make_shadow_group(
            &self.gpu.device,
            &self.shadow_layout,
            &self.shadow_uniform,
            shadows.atlas_view(),
            shadows.sampler(),
            shadows
                .transmission_view()
                .unwrap_or(self.lights.empty_transmission()),
            &self.lights,
        );
    }

    pub fn set_probe_corner_floor(&self, floor: f32) -> Result<(), String> {
        self.probes.set_corner_floor(&self.gpu.queue, floor)
    }

    pub fn probe_corner_floor(&self) -> f32 {
        self.probes.corner_floor()
    }

    pub fn set_reflection_occlusion(&mut self, threshold: f32) -> Result<(), String> {
        self.probes
            .set_reflection_occlusion(&self.gpu.queue, threshold)
    }

    pub fn set_sky_visibility(&mut self, on: bool) {
        self.probes.set_sky_visibility(&self.gpu.queue, on);
    }

    pub fn set_opaque_on(&self, on: OpaqueFeatures) {
        self.opaque_on.set(on);
    }

    pub fn set_opaque_off(&self, off: OpaqueFeatures) {
        self.opaque_off.set(off);
    }

    pub fn warm_opaque(&mut self, features: OpaqueFeatures, lit: bool) {
        for key in OpaqueKey::both_sides(features, lit) {
            self.pipelines.ensure_opaque(&self.gpu.device, key);
        }
    }

    pub fn warm(&mut self, first: &[OpaqueKey], later: &[OpaqueKey]) -> Warming {
        self.pipelines.warm(&self.gpu.device, first, later)
    }

    pub fn set_warm_workers(&mut self, workers: usize) {
        self.pipelines.workers = workers.max(1);
    }

    pub fn opaque_pipelines(&self) -> Vec<OpaqueKey> {
        self.pipelines.opaque.keys().copied().collect()
    }

    pub fn opaque_made_on_render_thread(&self) -> usize {
        self.pipelines.made_here
    }

    pub fn use_pipeline_cache(&mut self, path: &std::path::Path) -> CacheLoad {
        let (cache, load) = PipelineCache::open(&self.gpu.device, &self.gpu.info, path);
        if cache.is_some() {
            self.pipelines.cache = cache;
        }
        load
    }

    pub fn save_pipeline_cache(&self) -> Result<Option<usize>, String> {
        self.pipelines
            .cache
            .as_ref()
            .map_or(Ok(None), |cache| cache.save().map(Some))
    }

    pub fn set_reflections_planned(&mut self, planned: bool) {
        self.reflections_planned = planned;
    }

    pub fn opaque_keys(
        &self,
        scene: &Scene<'_>,
        shadows: Option<(&Shadows, &Fit)>,
    ) -> Result<Vec<OpaqueKey>, String> {
        if scene.materials.is_empty() {
            return Err("scene needs at least one material".into());
        }
        let surfaces = self.scene_surfaces(scene.instances.len());
        let packed = self.pack_instances(scene, &surfaces, &self.skins.layout())?;
        let map_materials = self.pack_map_materials(scene);
        let instance_features = scene_instance_features(&packed, &map_materials, scene.materials);
        let batches = surface_batches(
            scene.instances,
            |mesh| self.deforms(mesh),
            |index| surfaces[index].clipped(),
            |index| surfaces[index].cutout,
        );
        let sample = shadows.map_or(self.shadow_sample, |(shadows, fit)| {
            shadows.sample_uniform(fit)
        });
        let features = self.batch_features(
            &batches,
            self.scene_features_from(&sample),
            &instance_features,
        );
        let lit = self.lights.extended();
        let mut keys: Vec<OpaqueKey> = Vec::new();
        for (batch, features) in batches.iter().zip(features) {
            let key = OpaqueKey {
                features,
                lit,
                two_sided: batch.two_sided,
            };
            if !batch.shadow_only && !keys.contains(&key) {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    pub fn opaque_drawn(&self) -> &[OpaqueFeatures] {
        &self.opaque_drawn
    }

    pub fn scene_features(&self) -> OpaqueFeatures {
        self.scene_features_from(&self.shadow_sample)
    }

    fn scene_features_from(&self, sample: &[u8; SAMPLE_UNIFORM_BYTES]) -> OpaqueFeatures {
        let read = |offset: usize| {
            f32::from_le_bytes(sample[offset..offset + 4].try_into().expect("four bytes"))
        };
        let unshadowed = read(60) > 0.5;
        let shadowed = !unshadowed;
        let soft = shadowed && read(256) > 0.0;
        OpaqueFeatures {
            sun_shadow: shadowed,
            soft_search: soft,
            soft_filter: soft,
            cascade_blend: shadowed && read(28) > 0.0,
            probes: self.probes.live(),
            local_reflections: self.local_reflections.borrow().is_some(),
            ssr: self.ssr.is_some() || self.reflections_planned,
            canopy: self.foliage.is_some(),
            ..OpaqueFeatures::NONE
        }
    }

    pub fn opaque_features(&self, batches: &[Batch]) -> Vec<OpaqueFeatures> {
        self.batch_features(batches, self.scene_features(), &self.instance_features)
    }

    fn batch_features(
        &self,
        batches: &[Batch],
        scene: OpaqueFeatures,
        instance_features: &[OpaqueFeatures],
    ) -> Vec<OpaqueFeatures> {
        batches
            .iter()
            .map(|batch| {
                let used = batch
                    .range
                    .clone()
                    .filter_map(|index| instance_features.get(index as usize))
                    .fold(scene, |all, features| all.union(*features));
                used.union(self.opaque_on.get())
                    .without(self.opaque_off.get())
            })
            .collect()
    }

    fn set_unshadowed(&mut self) {
        self.shadow_group = Self::make_shadow_group(
            &self.gpu.device,
            &self.shadow_layout,
            &self.shadow_uniform,
            &self._shadow_view,
            &self._shadow_sampler,
            self.lights.empty_transmission(),
            &self.lights,
        );
    }

    pub fn set_lights(&mut self, lights: &[LocalLight]) -> Result<(), String> {
        self.lights.set(lights)
    }

    pub fn set_light_quality(&mut self, quality: LightQuality) -> Result<(), String> {
        self.lights.set_quality(quality)
    }

    pub fn set_transmission(&mut self, tints: &[Option<[f32; 3]>]) -> Result<(), String> {
        self.lights.set_transmission(tints)
    }

    pub fn lights(&self) -> &Lights {
        &self.lights
    }

    pub fn set_probe_emission(&mut self, scale: f32) -> Result<(), String> {
        self.probes.set_emission(&self.gpu.queue, scale)
    }

    pub fn set_jitter(&mut self, jitter: [f32; 2]) {
        self.jitter = jitter;
    }

    pub fn set_reflections(&mut self, history: Option<reflect::Reflections>) {
        self.ssr = history;
    }

    pub fn set_local_reflections(
        &self,
        artifacts: Option<Vec<ReflectionArtifact>>,
        hour: f32,
    ) -> Result<(), String> {
        let reflections = artifacts
            .map(|artifacts| {
                let array =
                    upload_reflections(&self.gpu.device, &self.gpu.queue, &artifacts, hour)?;
                Ok::<_, String>(LocalReflections { artifacts, array })
            })
            .transpose()?;
        *self.local_reflections.borrow_mut() = reflections;
        Ok(())
    }

    pub fn set_local_reflection_hour(&self, hour: f32) -> Result<(), String> {
        let mut reflections = self.local_reflections.borrow_mut();
        if let Some(ref mut current) = *reflections {
            current.array =
                upload_reflections(&self.gpu.device, &self.gpu.queue, &current.artifacts, hour)?;
        }
        Ok(())
    }

    pub fn set_sky(&mut self, source: SkySource, settle: bool) -> u64 {
        if !settle {
            return self.sky.request_source(source);
        }
        self.sky
            .set_source(&self.gpu.device, &self.gpu.queue, source);
        while !self.sky.lighting.complete() {
            self.sky.update(
                &self.gpu.queue,
                [0.0, 0.0, -1.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            );
        }
        self.sky.shown()
    }

    pub fn sky(&self) -> &SkyPass {
        &self.sky
    }

    pub fn sky_mut(&mut self) -> &mut SkyPass {
        &mut self.sky
    }

    pub fn set_ambient(&mut self, ambient: Ambient) -> Result<(), String> {
        if !ambient.valid() {
            return Err("an ambient must be finite, and a constant nonnegative".into());
        }
        self.ambient = ambient;
        Ok(())
    }

    pub fn ambient(&self) -> Ambient {
        self.ambient
    }

    pub fn set_contact(&mut self, desc: Option<ContactDesc>) -> Result<(), String> {
        let uniform = match desc {
            Some(desc) => {
                self.contact = Some(ContactField::new(&self.gpu.device, &self.gpu.queue, desc)?);
                ContactUniform::on(&desc)
            }
            None => {
                self.contact = None;
                ContactUniform::off()
            }
        };
        self.gpu
            .queue
            .write_buffer(&self.contact_uniform, 0, bytemuck::bytes_of(&uniform));
        Ok(())
    }

    pub fn contact(&self) -> Option<&ContactField> {
        self.contact.as_ref()
    }

    pub fn write_contact(&self, values: &[f32]) -> Result<(), String> {
        match &self.contact {
            Some(field) => field.write(&self.gpu.queue, values),
            None => Err("no contact field is set".into()),
        }
    }

    pub fn set_probes(&mut self, probes: ProbeLighting) {
        self.probes = probes;
        self.probes.set_fallback(&self.gpu.queue, [-1.0; 3]);
    }

    pub fn set_probe_hour(&mut self, hour: f32) -> Result<(), String> {
        self.set_local_reflection_hour(hour)?;
        if self.probes.manifest.is_none() {
            return Ok(());
        }
        self.probes
            .set_hour(&self.gpu.device, &self.gpu.queue, hour)
    }

    pub fn probes(&self) -> &ProbeLighting {
        &self.probes
    }

    pub fn set_maps(&mut self, images: &MapImages<'_>) -> Result<(), String> {
        let filter = self.maps.filter;
        self.maps = MapTextures::new(&self.gpu.device, &self.gpu.queue, images)?;
        self.maps.set_filter(&self.gpu.device, filter);
        self.map_layers = [
            images.base.len(),
            images.normal.len(),
            images.roughness.len(),
            images.metal.len(),
        ];
        Ok(())
    }

    pub fn set_detail(&mut self, atlas: &Atlas) -> Result<(), String> {
        let filter = self.maps.filter;
        self.maps = MapTextures::detail(&self.gpu.device, &self.gpu.queue, atlas)?;
        self.maps.set_filter(&self.gpu.device, filter);
        let parts = atlas.parts.len();
        self.map_layers = [parts, parts, parts, 0];
        Ok(())
    }

    pub fn set_texture_filter(&mut self, filter: maps::TextureFilter) -> Result<(), String> {
        filter.check()?;
        let anisotropy = self.maps.filter.anisotropy;
        self.maps.set_filter(&self.gpu.device, filter);
        if filter.anisotropy != anisotropy {
            self.content
                .borrow_mut()
                .set_filter(&self.gpu.device, filter);
        }
        Ok(())
    }

    pub fn texture_filter(&self) -> maps::TextureFilter {
        self.maps.filter
    }

    pub fn maps(&self) -> &MapTextures {
        &self.maps
    }

    pub fn set_content(
        &self,
        slot: usize,
        width: u32,
        height: u32,
        format: ContentFormat,
    ) -> Result<(), String> {
        self.content_changed();
        self.content
            .borrow_mut()
            .allocate(&self.gpu.device, slot, width, height, format)
    }

    pub fn write_content(&self, slot: usize, rect: [u32; 4], bytes: &[u8]) -> Result<(), String> {
        self.content_changed();
        self.content
            .borrow_mut()
            .write(&self.gpu.device, &self.gpu.queue, slot, rect, bytes)
    }

    fn content_changed(&self) {
        self.content_epoch
            .set(self.content_epoch.get().wrapping_add(1));
    }

    pub fn copy_content(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        source: &wgpu::Texture,
    ) -> Result<maps::ContentCopy, String> {
        self.content_changed();
        self.content
            .borrow_mut()
            .copy(&self.gpu.device, encoder, slot, source)
    }

    pub fn regenerate_content(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
    ) -> Result<(), String> {
        self.content_changed();
        self.content.borrow_mut().regenerate(encoder, slot)
    }

    pub fn regenerate_content_regions(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        regions: &[maps::MipRegion],
    ) -> Result<(), String> {
        self.content_changed();
        self.content
            .borrow_mut()
            .regenerate_regions(&self.gpu.device, encoder, slot, regions)
    }

    pub fn set_caustic(&self, caustic: Option<Caustic>) -> Result<(), String> {
        if let Some(caustic) = &caustic {
            caustic.check()?;
        }
        *self.caustic.borrow_mut() = caustic;
        Ok(())
    }

    pub fn caustic(&self) -> Option<Caustic> {
        self.caustic.borrow().clone()
    }

    pub fn release_content(&self, slot: usize) {
        self.content_changed();
        self.content.borrow_mut().release(slot);
    }

    pub fn content(&self) -> std::cell::Ref<'_, ContentSlots> {
        self.content.borrow()
    }

    pub fn set_surfaces(&self, surfaces: &[InstanceSurface]) -> Result<(), String> {
        if surfaces.iter().any(|surface| !surface.valid()) {
            return Err(
                "instance surface planes, offsets and crops must be finite and reflection occlusion between 0 and 1"
                    .into(),
            );
        }
        let mut current = self.surfaces.borrow_mut();
        current.clear();
        current.extend_from_slice(surfaces);
        Ok(())
    }

    pub fn surfaces(&self) -> std::cell::Ref<'_, Vec<InstanceSurface>> {
        self.surfaces.borrow()
    }

    pub fn lit_bindings(&self) -> Option<crate::text::LitBindings<'_>> {
        Some(crate::text::LitBindings {
            scene: &self.scene_group,
            shadow: &self.shadow_group,
            lighting: self.lighting_group.as_ref()?,
            deformers: self.deformers.buffer(),
            plan: self.deformers.plan_buffer(),
        })
    }

    pub fn deformer_buffer(&self) -> &DeformerBuffer {
        &self.deformers
    }

    pub fn has_tree(&self) -> bool {
        self.foliage.is_some()
    }

    pub fn set_canopy_mode(&mut self, mode: CanopyMode) -> Result<(), String> {
        let foliage = self.foliage.as_mut().ok_or("no tree is set")?;
        foliage.canopy.set_mode(mode);
        Ok(())
    }

    pub fn set_canopy_sharpness(&mut self, sharpness: f32) -> Result<(), String> {
        let foliage = self.foliage.as_mut().ok_or("no tree is set")?;
        foliage.canopy.set_sharpness(sharpness)
    }

    pub fn set_canopy_strength(&mut self, strength: f32) -> Result<(), String> {
        let foliage = self.foliage.as_mut().ok_or("no tree is set")?;
        foliage.canopy.set_strength(strength)
    }

    pub fn set_canopy_gobo(&mut self, gobo: Gobo) -> Result<(), String> {
        gobo.validate()?;
        if gobo == self.canopy_gobo {
            return Ok(());
        }
        let module = self
            .gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("live frame shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source_with(&gobo).into()),
            });
        self.pipelines.retarget(module);
        self.canopy_gobo = gobo;
        if let Some(foliage) = self.foliage.as_mut() {
            foliage.canopy.set_weight(gobo.weight);
        }
        Ok(())
    }

    pub fn canopy_gobo(&self) -> Gobo {
        self.canopy_gobo
    }

    pub fn visible_card_count(&self) -> u32 {
        self.foliage
            .as_ref()
            .map_or(0, |foliage| foliage.canopy.card_count)
    }

    pub fn set_tree(
        &mut self,
        tree: &Tree,
        origin: [f32; 3],
        receiver_y: f32,
        id: u32,
    ) -> Result<MeshHandle, String> {
        let wood = &tree.wood;
        let handle = self.upload_swaying_mesh(
            MeshData {
                positions: &wood.positions,
                normals: &wood.normals,
                tangents: &wood.tangents,
                uvs: &wood.uvs,
                uvs1: None,
                alpha: None,
                indices: &wood.indices,
            },
            &tree.sway,
        )?;
        let device = &self.gpu.device;
        let mut canopy = Canopy::new(
            device,
            &self.gpu.queue,
            tree,
            crate::canopy::DEFAULT_RESOLUTION,
        );
        canopy.set_weight(self.canopy_gobo.weight);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live cards layout"),
            bind_group_layouts: &[
                &self.scene_layout,
                &self.shadow_layout,
                canopy.cookie_layout(),
                &self.lighting_layout,
            ],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live cards"),
            source: wgpu::ShaderSource::Wgsl(card_source().into()),
        });
        let prepass = pipeline(
            device,
            PipelineSpec {
                label: "live card depth",
                layout: &pipeline_layout,
                module: &module,
                vertex: "card_vertex",
                fragment: Some("card_prepass"),
                buffers: &[],
                targets: &prepass_targets(),
                cull: None,
                write_depth: true,
                cache: None,
            },
        );
        let opaque = pipeline(
            device,
            PipelineSpec {
                label: "live card shading",
                layout: &pipeline_layout,
                module: &module,
                vertex: "card_vertex",
                fragment: Some("card_opaque"),
                buffers: &[],
                targets: &opaque_targets(),
                cull: None,
                write_depth: false,
                cache: None,
            },
        );
        self.foliage = Some(Foliage {
            canopy,
            origin,
            receiver_y,
            id,
            prepass,
            opaque,
        });
        Ok(handle)
    }

    pub fn upload_mesh(&mut self, data: MeshData<'_>) -> Result<MeshHandle, String> {
        self.upload(data, None, None)
    }

    pub fn upload_skinned_mesh(
        &mut self,
        data: MeshData<'_>,
        skin: SkinData<'_>,
    ) -> Result<MeshHandle, String> {
        if skin.joints.len() != data.positions.len() || skin.weights.len() != data.positions.len() {
            return Err("skin joints and weights need one entry per vertex".into());
        }
        if skin
            .weights
            .iter()
            .flatten()
            .any(|weight| !weight.is_finite())
        {
            return Err("a skin weight is not finite".into());
        }
        self.upload(data, None, Some(skin))
    }

    pub fn upload_swaying_mesh(
        &mut self,
        data: MeshData<'_>,
        sway: &[SwayVertex],
    ) -> Result<MeshHandle, String> {
        if sway.len() != data.positions.len() {
            return Err("sway attributes need one entry per vertex".into());
        }
        self.upload(data, Some(sway), None)
    }

    fn upload(
        &mut self,
        data: MeshData<'_>,
        sway: Option<&[SwayVertex]>,
        skin: Option<SkinData<'_>>,
    ) -> Result<MeshHandle, String> {
        check_mesh(&data)?;
        let slot = match self.free_meshes.last() {
            Some(&slot) => slot,
            None if self.meshes.len() <= MESH_SLOT_MASK as usize => self.meshes.len(),
            None => return Err("too many meshes".into()),
        };
        let generation = self.meshes.get(slot).map_or(0, |slot| slot.generation);
        let vertices = mesh_vertices(&data);
        let device = &self.gpu.device;
        let live = &self.mesh_buffers_live;
        let vertex_buffer = mesh_buffer(
            device,
            live,
            "live mesh vertices",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let positions = mesh_buffer(
            device,
            live,
            "live mesh positions",
            bytemuck::cast_slice(data.positions),
            wgpu::BufferUsages::VERTEX,
        );
        let indices = mesh_buffer(
            device,
            live,
            "live mesh indices",
            bytemuck::cast_slice(data.indices),
            wgpu::BufferUsages::INDEX,
        );
        let (base, sway_len) = match sway {
            Some(sway) => (self.allocate_sway(sway)?, sway.len() as u32),
            None => (NO_SWAY, 0),
        };
        let (skin_base, skin_len, skin_joints) = match skin {
            Some(skin) => {
                let range = self.skins.allocate(&self.gpu.device, skin)?;
                self.rebuild_scene_group();
                (range.start, range.end - range.start, skin.palette() as u32)
            }
            None => (NO_SKIN, 0, 0),
        };
        let mesh = Mesh {
            vertices: vertex_buffer,
            positions,
            indices,
            vertex_capacity: data.positions.len(),
            index_capacity: data.indices.len(),
            count: data.indices.len() as u32,
            sway: base,
            sway_len,
            skin: skin_base,
            skin_len,
            skin_joints,
        };
        if slot == self.meshes.len() {
            self.meshes.push(MeshSlot {
                generation,
                mesh: Some(mesh),
            });
        } else {
            self.free_meshes.pop();
            self.meshes[slot].mesh = Some(mesh);
        }
        Ok(MeshHandle::pack(slot, generation))
    }

    fn allocate_sway(&mut self, sway: &[SwayVertex]) -> Result<u32, String> {
        let base = self.sway_free.take(
            &mut self.sway_data,
            sway.len(),
            GpuSway {
                pivot_level: [0.0; 4],
                stiffness: [0.0; 4],
            },
            "sway vertices",
        )?;
        for (slot, vertex) in self.sway_data[base as usize..].iter_mut().zip(sway) {
            *slot = GpuSway {
                pivot_level: [
                    vertex.pivot[0],
                    vertex.pivot[1],
                    vertex.pivot[2],
                    vertex.level,
                ],
                stiffness: [vertex.stiffness, 0.0, 0.0, 0.0],
            };
        }
        self.sway = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("live sway"),
                contents: bytemuck::cast_slice(&self.sway_data),
                usage: wgpu::BufferUsages::STORAGE,
            });
        self.rebuild_scene_group();
        Ok(base)
    }

    fn check_handle(&self, handle: MeshHandle) -> Result<usize, String> {
        match live_mesh(&self.meshes, handle) {
            Some(_) => Ok(handle.slot()),
            None => Err("mesh handle is unknown or already released".into()),
        }
    }

    pub fn release_mesh(&mut self, handle: MeshHandle) -> Result<(), String> {
        let slot = self.check_handle(handle)?;
        let Some(mesh) = self.meshes[slot].mesh.take() else {
            return Err("mesh handle is unknown or already released".into());
        };
        if mesh.sway != NO_SWAY {
            self.sway_free.give(mesh.sway..mesh.sway + mesh.sway_len);
        }
        if mesh.skin != NO_SKIN {
            self.skins.release(mesh.skin..mesh.skin + mesh.skin_len);
        }
        let entry = &mut self.meshes[slot];
        entry.generation += 1;
        if entry.generation < MESH_GENERATION_LAST {
            self.free_meshes.push(slot);
        }
        self.mesh_epoch += 1;
        Ok(())
    }

    pub fn replace_mesh(&mut self, handle: MeshHandle, data: MeshData<'_>) -> Result<(), String> {
        check_mesh(&data)?;
        let slot = self.check_handle(handle)?;
        let Some(mesh) = self.meshes[slot].mesh.as_mut() else {
            return Err("mesh handle is unknown or already released".into());
        };
        if mesh.sway != NO_SWAY {
            return Err("a swaying mesh cannot be replaced".into());
        }
        if mesh.skin != NO_SKIN {
            return Err("a skinned mesh cannot be replaced".into());
        }
        let device = &self.gpu.device;
        let queue = &self.gpu.queue;
        let live = &self.mesh_buffers_live;
        let vertices = mesh_vertices(&data);
        if data.positions.len() <= mesh.vertex_capacity {
            queue.write_buffer(&mesh.vertices, 0, bytemuck::cast_slice(&vertices));
            queue.write_buffer(&mesh.positions, 0, bytemuck::cast_slice(data.positions));
        } else {
            mesh.vertices = mesh_buffer(
                device,
                live,
                "live mesh vertices",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            );
            mesh.positions = mesh_buffer(
                device,
                live,
                "live mesh positions",
                bytemuck::cast_slice(data.positions),
                wgpu::BufferUsages::VERTEX,
            );
            mesh.vertex_capacity = data.positions.len();
        }
        if data.indices.len() <= mesh.index_capacity {
            queue.write_buffer(&mesh.indices, 0, bytemuck::cast_slice(data.indices));
        } else {
            mesh.indices = mesh_buffer(
                device,
                live,
                "live mesh indices",
                bytemuck::cast_slice(data.indices),
                wgpu::BufferUsages::INDEX,
            );
            mesh.index_capacity = data.indices.len();
        }
        mesh.count = data.indices.len() as u32;
        self.mesh_epoch += 1;
        Ok(())
    }

    pub fn mesh_stats(&self) -> MeshStats {
        let live = self
            .meshes
            .iter()
            .filter(|slot| slot.mesh.is_some())
            .count();
        MeshStats {
            live,
            slots: self.meshes.len(),
            buffers: self.mesh_buffers_live.load(Ordering::Relaxed),
            glass_live: 0,
            glass_slots: 0,
            glass_buffers: 0,
        }
    }

    pub fn shadow_mesh(&self, handle: MeshHandle) -> Option<(&wgpu::Buffer, &wgpu::Buffer, u32)> {
        live_mesh(&self.meshes, handle)
            .map(|mesh| (&mesh.positions.buffer, &mesh.indices.buffer, mesh.count))
    }

    pub fn mesh_buffers(&self, handle: MeshHandle) -> Option<MeshBuffers<'_>> {
        live_mesh(&self.meshes, handle).map(|mesh| MeshBuffers {
            vertices: &mesh.vertices,
            positions: &mesh.positions,
            indices: &mesh.indices,
            count: mesh.count,
        })
    }

    pub fn swaying(&self, handle: MeshHandle) -> bool {
        live_mesh(&self.meshes, handle).is_some_and(|mesh| mesh.sway != NO_SWAY)
    }

    pub fn skinned(&self, handle: MeshHandle) -> bool {
        live_mesh(&self.meshes, handle).is_some_and(|mesh| mesh.skin != NO_SKIN)
    }

    fn deforms(&self, handle: MeshHandle) -> bool {
        live_mesh(&self.meshes, handle)
            .is_some_and(|mesh| mesh.sway != NO_SWAY || mesh.skin != NO_SKIN)
    }

    pub fn set_poses(&self, poses: &[InstancePose<'_>]) -> Result<(), String> {
        self.skins.set_poses(poses)
    }

    pub fn same_poses(&self, poses: &[InstancePose<'_>]) -> bool {
        self.skins.same(poses)
    }

    pub fn render(&mut self, scene: &Scene<'_>) -> Result<&FrameTargets, String> {
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("live frame"),
            });
        let mut profiler = std::mem::replace(
            &mut self.profiler,
            GpuProfiler::new(&self.gpu.device, &self.gpu.queue),
        );
        let result = self.encode(scene, &mut encoder, None, Some(&mut profiler));
        if let Err(error) = result {
            self.profiler = profiler;
            return Err(error);
        }
        let slot = profiler.finish(&mut encoder);
        self.gpu.queue.submit(Some(encoder.finish()));
        if let Some(slot) = slot {
            profiler.submitted(slot);
        }
        self.profiler = profiler;
        Ok(&self.targets)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err("viewport is empty".into());
        }
        if (width, height) == (self.targets.width, self.targets.height) {
            self.viewport = [width, height];
            return Ok(());
        }
        self.targets = new_targets(&self.gpu, width, height)?;
        self.ssr = None;
        self.viewport = [width, height];
        Ok(())
    }

    pub fn viewport(&self) -> [u32; 2] {
        self.viewport
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        crate::viewport::check([width, height], [self.targets.width, self.targets.height])?;
        self.viewport = [width, height];
        Ok(())
    }

    fn scene_surfaces(&self, count: usize) -> Vec<InstanceSurface> {
        let given = self.surfaces.borrow();
        (0..count)
            .map(|index| given.get(index).copied().unwrap_or_default())
            .collect()
    }

    fn pack_instances(
        &self,
        scene: &Scene<'_>,
        surfaces: &[InstanceSurface],
        poses: &PoseLayout,
    ) -> Result<Vec<GpuInstance>, String> {
        let mut packed_instances = Vec::with_capacity(scene.instances.len());
        let caustic = self.caustic.borrow();
        for (index, (instance, surface)) in scene.instances.iter().zip(surfaces).enumerate() {
            let Some(mesh) = live_mesh(&self.meshes, instance.mesh) else {
                return Err("instance references a released or unknown mesh".into());
            };
            if instance.material as usize >= scene.materials.len() {
                return Err("instance references an unknown mesh or material".into());
            }
            if !(0.0..=1.0).contains(&instance.opacity)
                || !(0.0..=1.0).contains(&instance.alpha_cutoff)
                || !(0.0..=1.0).contains(&instance.coverage)
                || !instance.age.is_finite()
            {
                return Err("instance alpha, coverage and age must be finite fractions".into());
            }
            if instance.deformer != DeformerId::NONE
                && instance.deformer.0 as usize >= scene.deformers.len()
            {
                return Err("instance references an unknown deformer".into());
            }
            let skin = match poses.get(index).filter(|_| mesh.skin != NO_SKIN) {
                Some(placed) if placed.joints < mesh.skin_joints => {
                    return Err(format!(
                        "instance {index} is posed with {} joints where its mesh needs {}",
                        placed.joints, mesh.skin_joints
                    ));
                }
                Some(placed) => [mesh.skin, placed.current, placed.previous, placed.joints],
                None => [NO_SKIN, 0, 0, 0],
            };
            let still = u32::from(
                instance.deformer == DeformerId::NONE
                    && mesh.sway == NO_SWAY
                    && skin[0] == NO_SKIN
                    && same_bits(instance.model, instance.previous_model),
            );
            packed_instances.push(GpuInstance {
                model: instance.model,
                previous_model: instance.previous_model,
                normal: normal_columns(instance.model)?,
                meta: [
                    instance.material,
                    instance.id,
                    instance.opacity.to_bits(),
                    instance.alpha_cutoff.to_bits(),
                ],
                extra: [
                    instance.deformer.0,
                    mesh.sway,
                    instance.age.clamp(0.0, 1.0).to_bits(),
                    instance.coverage.to_bits(),
                ],
                clip: surface.clip,
                content_uv: [
                    surface.uv_offset[0],
                    surface.uv_offset[1],
                    surface.uv_scale[0],
                    surface.uv_scale[1],
                ],
                content_crop: surface.crop,
                content_face: [
                    surface.face.code(),
                    still,
                    u32::from(surface.contact)
                        | u32::from(caustic.as_ref().is_some_and(|c| c.receives(index))) << 1
                        | u32::from(surface.cutout) << 2
                        | surface.occlusion_release() << 8,
                    u32::from(surface.authored_ambient),
                ],
                skin,
            });
        }
        Ok(packed_instances)
    }

    fn pack_map_materials(&self, scene: &Scene<'_>) -> Vec<GpuMapMaterial> {
        scene
            .materials
            .iter()
            .map(|material| {
                let indices = MapIndices::from_material(material)
                    .ok()
                    .filter(|indices| {
                        [
                            indices.base,
                            indices.normal,
                            indices.roughness,
                            indices.metal,
                        ]
                        .iter()
                        .zip(self.map_layers)
                        .all(|(index, count)| index.is_none_or(|index| (index as usize) < count))
                    })
                    .unwrap_or_default();
                self.maps.material(material, indices)
            })
            .collect()
    }

    fn prepare(&mut self, scene: &Scene<'_>) -> Result<(Vec<Batch>, Vec<InstanceSurface>), String> {
        if scene.materials.is_empty() {
            return Err("scene needs at least one material".into());
        }
        let surfaces = self.scene_surfaces(scene.instances.len());
        self.content.borrow().write_caustic(
            &self.gpu.queue,
            Caustic::rows(self.caustic.borrow().as_ref(), scene.sun.direction),
        );
        let packed_instances = self.pack_instances(scene, &surfaces, &self.skins.layout())?;
        if self.skins.upload(&self.gpu.device, &self.gpu.queue) {
            self.rebuild_scene_group();
        }
        self.deformers
            .upload(&self.gpu.device, &self.gpu.queue, scene.deformers)?;
        let materials: Vec<PackedMaterial> =
            scene.materials.iter().map(PackedMaterial::pack).collect();
        self.maps.fit_coat(&self.gpu.queue, scene.materials);
        let map_materials = self.pack_map_materials(scene);
        self.instance_features =
            scene_instance_features(&packed_instances, &map_materials, scene.materials);
        let mut resized = false;
        if packed_instances.len() > self.instance_capacity {
            self.instance_capacity = packed_instances.len().next_power_of_two();
            self.instances = storage(
                &self.gpu.device,
                "live instances",
                self.instance_capacity * std::mem::size_of::<GpuInstance>(),
            );
            resized = true;
        }
        if materials.len() > self.material_capacity {
            self.material_capacity = materials.len().next_power_of_two();
            self.materials = storage(
                &self.gpu.device,
                "live materials",
                self.material_capacity * PackedMaterial::SIZE,
            );
            self.map_materials = storage(
                &self.gpu.device,
                "live map materials",
                self.material_capacity * std::mem::size_of::<GpuMapMaterial>(),
            );
            resized = true;
        }
        if resized {
            self.rebuild_scene_group();
        }
        if self
            .lights
            .prepare(&self.gpu.device, &self.gpu.queue, scene.materials.len())
        {
            self.cascade_group = Self::make_cascade_group(
                &self.gpu.device,
                &self.cascade_layout,
                &self.cascade_uniform,
                self.lights.table(),
            );
        }
        let [width, height] = self
            .ssr
            .as_ref()
            .map_or(self.viewport, |history| history.size);
        let motion_view_projection = multiply(scene.camera.projection, scene.camera.view);
        let wind = scene.wind;
        let uniforms = Uniforms {
            view_projection: multiply(jitter_matrix(self.jitter), motion_view_projection),
            motion_view_projection,
            previous_view_projection: scene.camera.previous_view_projection,
            view: scene.camera.view,
            camera_position: [
                scene.camera.position[0],
                scene.camera.position[1],
                scene.camera.position[2],
                if same_bits(
                    motion_view_projection,
                    scene.camera.previous_view_projection,
                ) {
                    1.0
                } else {
                    0.0
                },
            ],
            sun_direction: [
                scene.sun.direction[0],
                scene.sun.direction[1],
                scene.sun.direction[2],
                0.0,
            ],
            sun_colour_intensity: [
                scene.sun.colour[0],
                scene.sun.colour[1],
                scene.sun.colour[2],
                scene.sun.intensity,
            ],
            time_size: [
                scene.time,
                width as f32,
                height as f32,
                (self.sky.lighting.mip_count - 1) as f32,
            ],
            flags: [
                scene.seed,
                u32::from(self.ssr.is_some()),
                u32::from(self.foliage.is_some()),
                self.foliage.as_ref().map_or(0, |foliage| foliage.id),
            ],
            wind: [
                wind.current.time,
                wind.current.strength,
                wind.previous.time,
                wind.previous.strength,
            ],
            wind_direction: [
                wind.current.direction[0],
                wind.current.direction[1],
                wind.current.direction[2],
                0.0,
            ],
            previous_wind_direction: [
                wind.previous.direction[0],
                wind.previous.direction[1],
                wind.previous.direction[2],
                0.0,
            ],
        };
        let queue = &self.gpu.queue;
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
        if !packed_instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&packed_instances));
        }
        queue.write_buffer(&self.materials, 0, bytemuck::cast_slice(&materials));
        queue.write_buffer(&self.map_materials, 0, bytemuck::cast_slice(&map_materials));
        let mut light = SkyLight {
            sh: [[0.0; 4]; 9],
            info: [(self.sky.lighting.mip_count - 1) as f32, 0.0, 0.0, 0.0],
        };
        for (out, value) in light.sh.iter_mut().zip(self.sky.lighting.sh) {
            *out = [value[0], value[1], value[2], 0.0];
        }
        queue.write_buffer(&self.sky_light, 0, bytemuck::bytes_of(&light));
        let mut authored = light;
        for (out, value) in authored
            .sh
            .iter_mut()
            .zip(self.ambient.sh(self.sky.lighting.sh))
        {
            *out = [value[0], value[1], value[2], 0.0];
        }
        queue.write_buffer(&self.ambient_light, 0, bytemuck::bytes_of(&authored));
        let mut selection = ReflectionSelection::zeroed();
        selection.first.box_min = [-1.0; 3];
        selection.first.box_max = [1.0; 3];
        selection.second.box_min = [-1.0; 3];
        selection.second.box_max = [1.0; 3];
        let sky_mip = (self.sky.lighting.mip_count - 1) as f32;
        selection.weights_mip = [0.0, 0.0, sky_mip, sky_mip];
        if let Some(ref reflections) = *self.local_reflections.borrow() {
            let records = &reflections.array.records;
            let [first, second] = local_candidates(records, scene.camera.position);
            selection.first = records[first].into();
            selection.second = records[second].into();
            selection.weights_mip = [
                1.0,
                if second == first { 0.0 } else { 1.0 },
                reflections.array.max_mip,
                sky_mip,
            ];
        }
        queue.write_buffer(
            &self.reflection_selection,
            0,
            bytemuck::bytes_of(&selection),
        );
        self.lighting_group = Some(self.make_lighting_group());
        let batches = surface_batches(
            scene.instances,
            |mesh| self.deforms(mesh),
            |index| surfaces[index].clipped(),
            |index| surfaces[index].cutout,
        );
        Ok((batches, surfaces))
    }

    fn make_lighting_group(&self) -> wgpu::BindGroup {
        let [lower, upper, grid] = self.probes.bindings();
        let cube = self.sky.cube_view();
        let (ssr, ssr_guide) = self
            .ssr
            .as_ref()
            .map_or((&self.fallbacks.black, &self.fallbacks.black), |ssr| {
                (&ssr.colour, &ssr.guide)
            });
        let reflections = self.local_reflections.borrow();
        let local = reflections
            .as_ref()
            .map_or(&self.fallbacks.local_probes, |current| &current.array.view);
        let (cookie, cookie_sampler, canopy_params) = match &self.foliage {
            Some(foliage) => (
                &foliage.canopy.view,
                &foliage.canopy.sampler,
                foliage.canopy.params(),
            ),
            None => (
                &self.fallbacks.black,
                &self.fallbacks.linear,
                &self.fallbacks.canopy_params,
            ),
        };
        let view = wgpu::BindingResource::TextureView;
        let content = self.content.borrow();
        let entries = [
            view(&self.maps.base.view),
            view(&self.maps.normal.view),
            view(&self.maps.roughness.view),
            view(&self.maps.metal.view),
            wgpu::BindingResource::Sampler(&self.maps.sampler),
            self.map_materials.as_entire_binding(),
            lower.as_entire_binding(),
            upper.as_entire_binding(),
            grid.as_entire_binding(),
            view(cookie),
            wgpu::BindingResource::Sampler(cookie_sampler),
            canopy_params.as_entire_binding(),
            self.sky_light.as_entire_binding(),
            view(&cube),
            view(local),
            wgpu::BindingResource::Sampler(&self.fallbacks.linear),
            view(ssr),
            self.reflection_selection.as_entire_binding(),
            view(content.view(0)),
            view(content.view(1)),
            view(content.view(2)),
            view(content.view(3)),
            wgpu::BindingResource::Sampler(&content.sampler),
            view(
                self.contact
                    .as_ref()
                    .map_or(&self.fallbacks.black, ContactField::view),
            ),
            self.contact_uniform.as_entire_binding(),
            self.ambient_light.as_entire_binding(),
            view(ssr_guide),
            content.caustic.as_entire_binding(),
        ];
        let entries: Vec<wgpu::BindGroupEntry<'_>> = entries
            .into_iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource,
            })
            .collect();
        self.gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("live lighting"),
                layout: &self.lighting_layout,
                entries: &entries,
            })
    }

    pub fn encode(
        &mut self,
        scene: &Scene<'_>,
        encoder: &mut wgpu::CommandEncoder,
        shadows: Option<(&mut Shadows, &Fit)>,
        profiler: Option<&mut GpuProfiler>,
    ) -> Result<(), String> {
        self.encode_with_shadow_hook(scene, encoder, shadows, profiler, |_, _, _, _| Ok(()))
    }

    pub fn encode_with_shadow_hook(
        &mut self,
        scene: &Scene<'_>,
        encoder: &mut wgpu::CommandEncoder,
        shadows: Option<(&mut Shadows, &Fit)>,
        profiler: Option<&mut GpuProfiler>,
        shadow_hook: impl FnOnce(
            &Gpu,
            &mut wgpu::CommandEncoder,
            &mut Shadows,
            &Fit,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        self.encode_with_casts(scene, encoder, shadows, profiler, None, shadow_hook)
    }

    pub fn encode_with_casts(
        &mut self,
        scene: &Scene<'_>,
        encoder: &mut wgpu::CommandEncoder,
        shadows: Option<(&mut Shadows, &Fit)>,
        mut profiler: Option<&mut GpuProfiler>,
        casts: Option<&mut crate::shadow::LayerDraw<'_>>,
        shadow_hook: impl FnOnce(
            &Gpu,
            &mut wgpu::CommandEncoder,
            &mut Shadows,
            &Fit,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        let (batches, surfaces) = self.prepare(scene)?;
        if batches.iter().any(|batch| batch.cut) {
            self.pipelines.ensure_cut(&self.gpu.device);
        }
        if let Some(foliage) = &mut self.foliage {
            foliage.canopy.select_lod(
                &self.gpu.queue,
                scene.camera,
                self.viewport[1],
                foliage.origin,
            );
            let timing = profiler
                .as_deref_mut()
                .and_then(|timer| timer.pass("canopy cookie"));
            let sun = scene.sun.direction;
            foliage.canopy.update(
                &self.gpu.queue,
                encoder,
                CanopyFrame {
                    origin: foliage.origin,
                    time: scene.wind.current.time,
                    strength: scene.wind.current.strength,
                    direction: scene.wind.current.direction,
                    sun,
                    receiver_y: foliage.receiver_y,
                    angular_radius: SUN_RADIUS,
                },
                profiler
                    .as_deref()
                    .and_then(|timer| timer.render_writes(timing)),
            );
        }
        let mut key = mix_key(
            static_key(scene.instances, &batches, &surfaces),
            self.mesh_epoch,
        );
        if batches.iter().any(|batch| batch.cut && batch.casts_shadow) {
            key = mix_key(key, self.cut_key(scene, &batches, &surfaces));
        }
        if let Some((shadows, fit)) = shadows {
            let cascades = mix_key(key, self.lights.table_key());
            self.encode_shadows(
                cascades,
                &batches,
                shadows,
                fit,
                encoder,
                profiler.as_deref_mut(),
            );
            let transmitting = self.encode_transmission(
                cascades,
                &batches,
                scene.instances,
                shadows,
                fit,
                encoder,
                casts,
                profiler.as_deref_mut(),
            );
            shadow_hook(&self.gpu, encoder, shadows, fit)?;
            self.encode_lights(key, &batches, encoder, profiler.as_deref_mut());
            self.lights
                .write_uniform(&self.gpu.queue, transmitting, shadows.face_tiles());
            self.set_shadows(shadows, fit);
        } else {
            self.encode_lights(key, &batches, encoder, profiler.as_deref_mut());
            self.lights
                .write_uniform(&self.gpu.queue, false, Default::default());
            self.set_unshadowed();
        }
        let features = self.opaque_features(&batches);
        let lit = self.lights.extended();
        for (batch, used) in batches.iter().zip(&features) {
            if batch.shadow_only {
                continue;
            }
            self.pipelines.ensure_opaque(
                &self.gpu.device,
                OpaqueKey {
                    features: *used,
                    lit,
                    two_sided: batch.two_sided,
                },
            );
        }
        self.encode_surfaces(&batches, &features, encoder, profiler)?;
        self.opaque_drawn = batches
            .iter()
            .zip(features)
            .filter(|(batch, _)| !batch.shadow_only)
            .map(|(_, used)| used)
            .collect();
        Ok(())
    }

    fn cut_key(&self, scene: &Scene<'_>, batches: &[Batch], surfaces: &[InstanceSurface]) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.content_epoch.get().hash(&mut hasher);
        for batch in batches
            .iter()
            .filter(|batch| batch.cut && batch.casts_shadow)
        {
            for index in batch.range.clone() {
                let index = index as usize;
                let instance = &scene.instances[index];
                let surface = &surfaces[index];
                instance.alpha_cutoff.to_bits().hash(&mut hasher);
                instance.opacity.to_bits().hash(&mut hasher);
                content_row_key(&scene.materials[instance.material as usize]).hash(&mut hasher);
                for value in surface
                    .uv_offset
                    .iter()
                    .chain(&surface.uv_scale)
                    .chain(&surface.crop)
                {
                    value.to_bits().hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_transmission(
        &self,
        key: u64,
        batches: &[Batch],
        instances: &[Instance],
        shadows: &mut Shadows,
        fit: &Fit,
        encoder: &mut wgpu::CommandEncoder,
        mut casts: Option<&mut crate::shadow::LayerDraw<'_>>,
        profiler: Option<&mut GpuProfiler>,
    ) -> bool {
        let chosen = if self.lights.any_transmissive() {
            transmission_batches(batches, instances, |material| {
                self.lights.transmissive(material)
            })
        } else {
            Vec::new()
        };
        if chosen.is_empty() && casts.is_none() {
            return false;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        for batch in &chosen {
            batch.range.hash(&mut hasher);
            for index in batch.range.clone() {
                instances[index as usize].material.hash(&mut hasher);
            }
        }
        let moving = casts.is_some() || chosen.iter().any(|batch| batch.moving);
        let faces = if casts.is_some() {
            self.lights.faces().len()
        } else {
            0
        };
        let mut draw = |pass: &mut wgpu::RenderPass<'_>, cascade: usize, statics: bool| {
            if !chosen.is_empty() && cascade < CASCADE_COUNT {
                pass.set_pipeline(&self.pipelines.transmit);
                pass.set_bind_group(0, &self.scene_group, &[]);
                pass.set_bind_group(
                    1,
                    &self.cascade_group,
                    &[self.cascade_align * cascade as u32],
                );
                pass.set_bind_group(2, &self.deformers.group, &[]);
                for batch in chosen.iter().filter(|batch| batch.moving != statics) {
                    let Some(mesh) = live_mesh(&self.meshes, batch.mesh) else {
                        continue;
                    };
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.count, 0, batch.range.clone());
                }
            }
            if !statics && let Some(casts) = casts.as_mut() {
                casts(pass, cascade, statics);
            }
        };
        shadows.render_transmission(
            &self.gpu.device,
            encoder,
            fit,
            hasher.finish(),
            moving,
            (faces, self.lights.quality().resolution),
            &mut draw,
            profiler,
        );
        true
    }

    fn encode_lights(
        &mut self,
        key: u64,
        batches: &[Batch],
        encoder: &mut wgpu::CommandEncoder,
        profiler: Option<&mut GpuProfiler>,
    ) {
        let faces = self.lights.faces();
        if faces.is_empty() {
            return;
        }
        for (layer, matrix) in &faces {
            let mut slot = [0u8; CASTER_BYTES as usize];
            slot[..64].copy_from_slice(bytemuck::cast_slice(matrix));
            self.gpu.queue.write_buffer(
                &self.cascade_uniform,
                u64::from(self.cascade_align) * (CASCADE_COUNT as u64 + u64::from(*layer)),
                &slot,
            );
        }
        let casters = shadow_batches(batches);
        let moving = casters.iter().any(|batch| batch.moving);
        let scene_group = &self.scene_group;
        let cascade_group = &self.cascade_group;
        let deformers = &self.deformers.group;
        let lighting = self.lighting_group.as_ref();
        let pipelines = &self.pipelines;
        let meshes = &self.meshes;
        let align = self.cascade_align;
        let mut draw = |pass: &mut wgpu::RenderPass<'_>, layer: usize, statics: bool| {
            pass.set_bind_group(0, scene_group, &[]);
            pass.set_bind_group(1, cascade_group, &[align * (CASCADE_COUNT + layer) as u32]);
            draw_casters(
                pass, &casters, statics, deformers, lighting, pipelines, meshes,
            );
        };
        self.lights
            .render(&self.gpu.device, encoder, key, moving, &mut draw, profiler);
    }

    fn encode_shadows(
        &self,
        key: u64,
        batches: &[Batch],
        shadows: &mut Shadows,
        fit: &Fit,
        encoder: &mut wgpu::CommandEncoder,
        profiler: Option<&mut GpuProfiler>,
    ) {
        let mode = if self.lights.any_transmissive() {
            1.0f32
        } else {
            0.0
        };
        let near_map = shadows.quality().splits_near_map();
        for (index, cascade) in fit.cascades.iter().enumerate() {
            let pancake = if near_map && index == 1 { 0.0f32 } else { 1.0 };
            let mut slot = [0u8; CASTER_BYTES as usize];
            slot[..64].copy_from_slice(bytemuck::cast_slice(&cascade.view_proj.columns));
            slot[64..68].copy_from_slice(&mode.to_le_bytes());
            slot[68..72].copy_from_slice(&pancake.to_le_bytes());
            self.gpu.queue.write_buffer(
                &self.cascade_uniform,
                u64::from(self.cascade_align) * index as u64,
                &slot,
            );
        }
        let casters = shadow_batches(batches);
        let moving = casters.iter().any(|batch| batch.moving);
        let mut draw = |pass: &mut wgpu::RenderPass<'_>, cascade: usize, statics: bool| {
            pass.set_bind_group(0, &self.scene_group, &[]);
            pass.set_bind_group(
                1,
                &self.cascade_group,
                &[self.cascade_align * cascade as u32],
            );
            self.draw_casters(pass, &casters, statics);
        };
        shadows.render_with(encoder, fit, key, moving, &mut draw, profiler);
    }

    fn draw_casters(&self, pass: &mut wgpu::RenderPass<'_>, casters: &[&Batch], statics: bool) {
        draw_casters(
            pass,
            casters,
            statics,
            &self.deformers.group,
            self.lighting_group.as_ref(),
            &self.pipelines,
            &self.meshes,
        );
    }

    fn encode_surfaces(
        &self,
        batches: &[Batch],
        features: &[OpaqueFeatures],
        encoder: &mut wgpu::CommandEncoder,
        mut profiler: Option<&mut GpuProfiler>,
    ) -> Result<(), String> {
        let lighting = self
            .lighting_group
            .as_ref()
            .ok_or("frame lighting is not prepared")?;
        let depth_timing = profiler
            .as_deref_mut()
            .and_then(|timer| timer.pass("depth and velocity"));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("depth and velocity"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.velocity_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.ids_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|timer| timer.render_writes(depth_timing)),
            });
            crate::viewport::apply(&mut pass, self.viewport);
            self.draw_with(&mut pass, batches, lighting, |index, two_sided| {
                let pair = match &self.pipelines.cut {
                    Some(cut) if batches[index].cut => &cut.prepass,
                    _ => &self.pipelines.prepass,
                };
                &pair[usize::from(two_sided)]
            });
            if let Some(foliage) = &self.foliage {
                self.draw_cards(&mut pass, foliage, &foliage.prepass, lighting);
            }
        }
        let opaque_timing = profiler
            .as_deref_mut()
            .and_then(|timer| timer.pass("opaque PBR"));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opaque PBR"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.hdr.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.targets.normal_roughness_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|timer| timer.render_writes(opaque_timing)),
            });
            crate::viewport::apply(&mut pass, self.viewport);
            let lit = self.lights.extended();
            self.draw_with(&mut pass, batches, lighting, |index, two_sided| {
                &self.pipelines.opaque[&OpaqueKey {
                    features: features[index],
                    lit,
                    two_sided,
                }]
            });
            if let Some(foliage) = &self.foliage {
                self.draw_cards(&mut pass, foliage, &foliage.opaque, lighting);
            }
        }
        Ok(())
    }

    fn draw_with<'p>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        batches: &[Batch],
        lighting: &wgpu::BindGroup,
        pipelines: impl Fn(usize, bool) -> &'p wgpu::RenderPipeline,
    ) {
        pass.set_bind_group(0, &self.scene_group, &[]);
        pass.set_bind_group(1, &self.shadow_group, &[]);
        pass.set_bind_group(2, &self.deformers.group, &[]);
        pass.set_bind_group(3, lighting, &[]);
        let mut current: Option<&wgpu::RenderPipeline> = None;
        for (index, batch) in batches
            .iter()
            .enumerate()
            .filter(|(_, batch)| !batch.shadow_only)
        {
            let chosen = pipelines(index, batch.two_sided);
            if !current.is_some_and(|last| std::ptr::eq(last, chosen)) {
                pass.set_pipeline(chosen);
                current = Some(chosen);
            }
            let Some(mesh) = live_mesh(&self.meshes, batch.mesh) else {
                continue;
            };
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, batch.range.clone());
        }
    }

    fn draw_cards(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        foliage: &Foliage,
        pipeline: &wgpu::RenderPipeline,
        lighting: &wgpu::BindGroup,
    ) {
        if foliage.canopy.card_count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.scene_group, &[]);
        pass.set_bind_group(1, &self.shadow_group, &[]);
        pass.set_bind_group(2, foliage.canopy.cookie_bind(), &[]);
        pass.set_bind_group(3, lighting, &[]);
        pass.draw(0..6, 0..foliage.canopy.card_count);
    }

    pub fn collect_timings(&mut self) -> Vec<Vec<PassTiming>> {
        self.profiler.collect(&self.gpu.device)
    }

    pub fn readback_velocity(&self) -> Result<Vec<[u16; 2]>, String> {
        let [width, height] = self.viewport;
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = u64::from(row) * u64::from(height);
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live velocity readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("live velocity readback"),
            });
        encoder.copy_texture_to_buffer(
            self.targets.velocity.as_image_copy(),
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
        self.gpu.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("{error:?}"))?;
        receive
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range();
        let mut values = Vec::with_capacity((width * height) as usize);
        for padded in mapped.chunks_exact(row as usize) {
            for pixel in padded[..width as usize * 4].chunks_exact(4) {
                values.push([
                    u16::from_le_bytes([pixel[0], pixel[1]]),
                    u16::from_le_bytes([pixel[2], pixel[3]]),
                ]);
            }
        }
        drop(mapped);
        buffer.unmap();
        Ok(values)
    }
}

const SHADER: &str = r#"
override use_sun_shadow: bool = true;
override use_ssr: bool = true;
override use_canopy: bool = true;
override use_contact: bool = true;
override use_authored_ambient: bool = true;
override use_caustics: bool = true;
struct FrameUniforms {
    view_projection: mat4x4f,
    motion_view_projection: mat4x4f,
    previous_view_projection: mat4x4f,
    view: mat4x4f,
    camera_position: vec4f,
    sun_direction: vec4f,
    sun_colour_intensity: vec4f,
    time_size: vec4f,
    flags: vec4u,
    wind: vec4f,
    wind_direction: vec4f,
    previous_wind_direction: vec4f,
}
struct InstanceData {
    model: mat4x4f,
    previous_model: mat4x4f,
    normal0: vec4f,
    normal1: vec4f,
    normal2: vec4f,
    fields: vec4u,
    extra: vec4u,
    clip0: vec4f,
    clip1: vec4f,
    content_uv: vec4f,
    content_crop: vec4f,
    content_face: vec4u,
    skin: vec4u,
}
struct SwayData {
    pivot_level: vec4f,
    stiffness: vec4f,
}
struct SkyLight {
    sh: array<vec4f, 9>,
    info: vec4f,
}
struct MaterialData {
    base_roughness: vec4f,
    metal_spec_coat: vec4f,
    sheen_trans_ior_disp: vec4f,
    thick_sub_film: vec4f,
    tint_absorption: vec4f,
    emission_film: vec4f,
    normal_maps: vec4f,
    maps_content: vec4f,
    age_a: vec4f,
    age_b: vec4f,
    layer0: vec4f,
    layer1: vec4f,
    layer2: vec4f,
    layer3: vec4f,
    tail: vec4f,
    params: array<vec4f, 16>,
}
@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var<storage, read> instances: array<InstanceData>;
@group(0) @binding(2) var<storage, read> materials: array<MaterialData>;
@group(0) @binding(3) var<storage, read> sway_data: array<SwayData>;
@group(1) @binding(0) var<uniform> shadow_setting: ShadowSample;
@group(1) @binding(1) var shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var shadow_sampler: sampler_comparison;
struct CasterView {
    view_proj: mat4x4f,
    mode: vec4f,
}
@group(1) @binding(3) var<uniform> caster: CasterView;
@group(3) @binding(12) var<uniform> sky_light: SkyLight;
@group(3) @binding(13) var sky_cube: texture_cube<f32>;
@group(3) @binding(14) var local_probes: texture_cube_array<f32>;
@group(3) @binding(15) var reflect_sampler: sampler;
@group(3) @binding(16) var ssr_history: texture_2d<f32>;
@group(3) @binding(26) var ssr_guide: texture_2d<f32>;
struct ReflectionSelection {
    first: ReflectionProbe,
    second: ReflectionProbe,
    weights_mip: vec4f,
}
@group(3) @binding(17) var<uniform> selected_reflections: ReflectionSelection;
struct ContactParams {
    rect: vec4f,
    tint: vec4f,
}
@group(3) @binding(23) var contact_field: texture_2d<f32>;
@group(3) @binding(24) var<uniform> contact_params: ContactParams;
@group(3) @binding(25) var<uniform> authored_light: SkyLight;
struct VertexInput {
    @location(0) position: vec4f,
    @location(1) normal: vec4f,
    @location(2) tangent: vec4f,
    @location(3) uv_alpha: vec4f,
    @builtin(instance_index) instance_index: u32,
    @builtin(vertex_index) vertex_index: u32,
}
struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) world_position: vec3f,
    @location(1) normal: vec4f,
    @location(2) tangent: vec4f,
    @location(3) uv_alpha: vec4f,
    @location(4) previous_clip: vec4f,
    @location(5) current_clip: vec4f,
    @location(6) @interpolate(flat) material: u32,
    @location(7) @interpolate(flat) id: u32,
    @location(8) @interpolate(flat) settings: vec4f,
    @location(9) object_position: vec3f,
    @location(10) @interpolate(flat) instance: u32,
}
fn surface_shape(input: VertexInput, instance: InstanceData) -> DeformedVertex {
    if (instance.skin.x != 0xffffffffu) {
        return skin_shape(input.position.xyz, input.normal.xyz, input.tangent, input.vertex_index, instance.skin);
    }
    var shape = deform_vertex(input.position.xyz, input.normal.xyz, input.tangent, instance.extra.x);
    if (instance.extra.y != 0xffffffffu) {
        let sway = sway_data[instance.extra.y + input.vertex_index];
        shape.position = tree_sway(shape.position, sway.pivot_level.xyz, sway.pivot_level.w, sway.stiffness.x, frame.wind.x, frame.wind.y, frame.wind_direction.xyz);
        shape.previous_position = tree_sway(shape.previous_position, sway.pivot_level.xyz, sway.pivot_level.w, sway.stiffness.x, frame.wind.z, frame.wind.w, frame.previous_wind_direction.xyz);
    }
    return shape;
}
@vertex fn vertex_main(input: VertexInput) -> VertexOutput {
    let instance = instances[input.instance_index];
    let shape = surface_shape(input, instance);
    let world = instance.model * vec4f(shape.position, 1.0);
    let normal_matrix = mat3x3f(instance.normal0.xyz, instance.normal1.xyz, instance.normal2.xyz);
    var output: VertexOutput;
    output.position = frame.view_projection * world;
    output.current_clip = frame.motion_view_projection * world;
    let previous_world = select(instance.previous_model * vec4f(shape.previous_position, 1.0), world, instance.content_face.y != 0u);
    output.previous_clip = frame.previous_view_projection * previous_world;
    output.world_position = world.xyz;
    output.normal = vec4f(normalize(normal_matrix * shape.normal), input.normal.w);
    output.tangent = vec4f(normalize((instance.model * vec4f(shape.tangent.xyz, 0.0)).xyz), shape.tangent.w);
    output.uv_alpha = input.uv_alpha;
    output.material = instance.fields.x;
    output.id = instance.fields.y;
    output.settings = vec4f(bitcast<f32>(instance.fields.z), bitcast<f32>(instance.fields.w), bitcast<f32>(instance.extra.z), bitcast<f32>(instance.extra.w));
    output.object_position = input.position.xyz;
    output.instance = input.instance_index;
    return output;
}
fn caster_position(input: VertexInput, instance: InstanceData) -> vec3f {
    if (instance.skin.x != 0xffffffffu) {
        return skin_position(input.position.xyz, input.vertex_index, instance.skin);
    }
    var position = deform_position(input.position.xyz, instance.extra.x, false);
    if (instance.extra.y != 0xffffffffu) {
        let sway = sway_data[instance.extra.y + input.vertex_index];
        position = tree_sway(position, sway.pivot_level.xyz, sway.pivot_level.w, sway.stiffness.x, frame.wind.x, frame.wind.y, frame.wind_direction.xyz);
    }
    return position;
}
@vertex fn shadow_vertex(input: VertexInput) -> @builtin(position) vec4f {
    let instance = instances[input.instance_index];
    if (caster_skips(instance.fields.x)) {
        return vec4f(2.0, 2.0, 2.0, 1.0);
    }
    let position = caster_position(input, instance);
    return pancake(caster.view_proj * instance.model * vec4f(position, 1.0));
}
fn pancake(clip: vec4f) -> vec4f {
    if (caster.mode.y > 0.5) {
        return vec4f(clip.xy, max(clip.z, 0.0), clip.w);
    }
    return clip;
}
fn clip_plane(plane: vec4f, world: vec3f) -> bool {
    return any(plane.xyz != vec3f(0.0)) && dot(plane.xyz, world) > plane.w;
}
fn clipped(instance: u32, world: vec3f) -> bool {
    let data = instances[instance];
    return clip_plane(data.clip0, world) || clip_plane(data.clip1, world);
}
struct ShadowClipOutput {
    @builtin(position) position: vec4f,
    @location(0) world_position: vec3f,
    @location(1) @interpolate(flat) instance: u32,
}
@vertex fn shadow_clip_vertex(input: VertexInput) -> ShadowClipOutput {
    let instance = instances[input.instance_index];
    if (caster_skips(instance.fields.x)) {
        var skipped: ShadowClipOutput;
        skipped.position = vec4f(2.0, 2.0, 2.0, 1.0);
        return skipped;
    }
    let position = caster_position(input, instance);
    let world = instance.model * vec4f(position, 1.0);
    var output: ShadowClipOutput;
    output.position = pancake(caster.view_proj * world);
    output.world_position = world.xyz;
    output.instance = input.instance_index;
    return output;
}
@fragment fn shadow_clip_fragment(input: ShadowClipOutput) {
    if (clipped(input.instance, input.world_position)) {
        discard;
    }
}
struct ShadowCutOutput {
    @builtin(position) position: vec4f,
    @location(0) world_position: vec3f,
    @location(1) @interpolate(flat) instance: u32,
    @location(2) uv_alpha: vec4f,
}
@vertex fn shadow_cut_vertex(input: VertexInput) -> ShadowCutOutput {
    let instance = instances[input.instance_index];
    var output: ShadowCutOutput;
    if (caster_skips(instance.fields.x)) {
        output.position = vec4f(2.0, 2.0, 2.0, 1.0);
        return output;
    }
    let position = caster_position(input, instance);
    let world = instance.model * vec4f(position, 1.0);
    output.position = pancake(caster.view_proj * world);
    output.world_position = world.xyz;
    output.instance = input.instance_index;
    output.uv_alpha = input.uv_alpha;
    return output;
}
@fragment fn shadow_cut_fragment(input: ShadowCutOutput) {
    if (clipped(input.instance, input.world_position)) {
        discard;
    }
    let data = instances[input.instance];
    let cutoff = bitcast<f32>(data.fields.w);
    if (input.uv_alpha.z * bitcast<f32>(data.fields.z) < cutoff) {
        discard;
    }
    if (cut_by_content(input.instance, data.fields.x, input.uv_alpha.xy, cutoff)) {
        discard;
    }
}
fn cut_by_content(instance: u32, material: u32, uv: vec2f, cutoff: f32) -> bool {
    let data = instances[instance];
    return (data.content_face.z & 4u) != 0u && content_cut(material, uv, data.content_uv, data.content_crop, cutoff);
}
fn cutout(input: VertexOutput) {
    if (clipped(input.instance, input.world_position)) {
        discard;
    }
    if (input.uv_alpha.z * input.settings.x < input.settings.y) {
        discard;
    }
    if (use_content_cutout && cut_by_content(input.instance, input.material, input.uv_alpha.xy, input.settings.y)) {
        discard;
    }
    if (input.settings.w < 1.0 && !lod_keep(vec2u(input.position.xy), input.id, input.settings.w)) {
        discard;
    }
}
struct PrepassOutput {
    @location(0) velocity: vec2f,
    @location(1) id: u32,
}
fn motion(current_clip: vec4f, previous_clip: vec4f) -> vec2f {
    let current = current_clip.xy / current_clip.w;
    let previous = previous_clip.xy / previous_clip.w;
    let moved = (previous - current) * vec2f(0.5, -0.5);
    return select(moved, vec2f(0.0), abs(moved) < vec2f(1e-8));
}
@fragment fn prepass_main(input: VertexOutput) -> PrepassOutput {
    cutout(input);
    var output: PrepassOutput;
    if (instances[input.instance].content_face.y != 0u && frame.camera_position.w != 0.0) {
        output.velocity = vec2f(0.0);
    } else {
        output.velocity = motion(input.current_clip, input.previous_clip);
    }
    output.id = input.id;
    return output;
}
fn shadow_visibility(world_position: vec3f, normal: vec3f) -> f32 {
    if (!use_sun_shadow || shadow_setting.texels.w > 0.5) { return 1.0; }
    let distance = length(world_position - frame.camera_position.xyz);
    return shadow_sample(shadow_map, shadow_sampler, reflect_sampler, shadow_setting, world_position, normal, distance);
}
fn sky_irradiance(n: vec3f) -> vec3f {
    return sh_irradiance(sky_light.sh, n);
}
fn sh_irradiance(sh: array<vec4f, 9>, n: vec3f) -> vec3f {
    let d = normalize(n);
    let basis0 = vec3f(0.282095, 0.488603 * d.y, 0.488603 * d.z);
    let basis1 = vec3f(0.488603 * d.x, 1.092548 * d.x * d.y, 1.092548 * d.y * d.z);
    let basis2 = vec3f(0.315392 * (3.0 * d.z * d.z - 1.0), 1.092548 * d.x * d.z, 0.546274 * (d.x * d.x - d.y * d.y));
    let value = sh[0].xyz * basis0.x + sh[1].xyz * basis0.y + sh[2].xyz * basis0.z
        + sh[3].xyz * basis1.x + sh[4].xyz * basis1.y + sh[5].xyz * basis1.z
        + sh[6].xyz * basis2.x + sh[7].xyz * basis2.y + sh[8].xyz * basis2.z;
    return max(value, vec3f(0.0));
}
fn ambient_irradiance(position: vec3f, normal: vec3f) -> vec3f {
    let grid = sample_live_irradiance(position, normal);
    if (grid.x >= 0.0) {
        return grid;
    }
    return sky_irradiance(normal);
}
fn surface_irradiance(grid: vec3f, normal: vec3f, authored: bool) -> vec3f {
    if (grid.x >= 0.0) {
        return grid;
    }
    if (authored) {
        return sh_irradiance(authored_light.sh, normal);
    }
    return sky_irradiance(normal);
}
fn reflection_reflective(surface: Shaded) -> bool {
    return surface.specular > 0.0 || surface.metalness > 0.0 || surface.clearcoat > 0.0 || surface.transmission > 0.0 || surface.thin_film_amount > 0.0;
}
fn reflection_wants_probes(surface: Shaded, release: f32) -> bool {
    if (!use_probes || probe_grid.shade.y <= 0.0 || probe_grid.blend.y < 0.5 || release >= 1.0) {
        return false;
    }
    return reflection_reflective(surface);
}
fn reflection_sky_visibility(seen: f32, direction: vec3f, lookup: LocalLookup, grid: vec3f, release: f32) -> f32 {
    if (seen < 0.0 || grid.x < 0.0 || lookup.weights.x + lookup.weights.y >= 1.0) {
        return 1.0;
    }
    let reference = dot(sky_irradiance(direction), vec3f(0.2126, 0.7152, 0.0722));
    if (!(reference > 0.000001)) {
        return 1.0;
    }
    return mix(clamp(seen / reference, 0.0, 1.0), 1.0, release);
}
fn reflection_irradiance(n: vec3f, world: vec3f, sky_visible: f32) -> vec3f {
    var a = 0.0;
    var b = 0.0;
    if (use_local_reflections) {
        let weights = local_probe_weights(selected_reflections.first, selected_reflections.second, selected_reflections.weights_mip.xy, world);
        a = clamp(weights.x, 0.0, 1.0);
        b = clamp(weights.y, 0.0, 1.0 - a);
    }
    var reference = sky_irradiance(n) * ((1.0 - a - b) * sky_visible);
    if (a != 0.0) {
        reference += local_probe_reference(selected_reflections.first, world, n, local_probes, reflect_sampler, selected_reflections.weights_mip.z) * (PI * a);
    }
    if (b != 0.0) {
        reference += local_probe_reference(selected_reflections.second, world, n, local_probes, reflect_sampler, selected_reflections.weights_mip.z) * (PI * b);
    }
    return reference;
}
fn contact_factor(world_position: vec3f) -> vec3f {
    if (contact_params.tint.w < 0.5) { return vec3f(1.0); }
    let uv = (world_position.xz - contact_params.rect.xy) * contact_params.rect.zw;
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0))) { return vec3f(1.0); }
    let open = textureSampleLevel(contact_field, reflect_sampler, uv, 0.0).r;
    return mix(contact_params.tint.xyz, vec3f(1.0), open * open);
}
fn shaded(material: MaterialData, normal: vec3f) -> Shaded {
    return Shaded(
        material.base_roughness.xyz,
        material.base_roughness.w,
        material.metal_spec_coat.x,
        material.metal_spec_coat.y,
        material.metal_spec_coat.z,
        material.metal_spec_coat.w,
        material.sheen_trans_ior_disp.x,
        material.sheen_trans_ior_disp.y,
        material.sheen_trans_ior_disp.z,
        material.sheen_trans_ior_disp.w,
        material.thick_sub_film.x,
        material.thick_sub_film.y,
        material.tint_absorption.xyz,
        material.tint_absorption.w,
        material.thick_sub_film.z,
        material.thick_sub_film.w,
        material.emission_film.w,
        material.emission_film.xyz,
        material.age_b.w,
        normal,
    );
}
struct OpaqueOutput {
    @location(0) colour: vec4f,
    @location(1) normal_roughness: vec4f,
}
@fragment fn opaque_main(input: VertexOutput, @builtin(front_facing) front: bool) -> OpaqueOutput {
    return opaque_shade(input, front, false);
}
@fragment fn opaque_lit_main(input: VertexOutput, @builtin(front_facing) front: bool) -> OpaqueOutput {
    return opaque_shade(input, front, true);
}
fn opaque_shade(input: VertexOutput, front: bool, lit: bool) -> OpaqueOutput {
    let uv_dx = dpdx(input.uv_alpha.xy);
    let uv_dy = dpdy(input.uv_alpha.xy);
    let caustic_at = caustic_uv(input.world_position);
    let caustic_dx = dpdx(caustic_at);
    let caustic_dy = dpdy(caustic_at);
    map_footprint(input.object_position);
    cutout(input);
    let material = materials[input.material];
    var geometric = normalize(input.normal.xyz);
    if (!front) {
        geometric = -geometric;
    }
    let mapped = map_evaluate_with(material.base_roughness.xyz, material.base_roughness.w, material.metal_spec_coat.x, material.metal_spec_coat.w, material.thick_sub_film.z, geometric, input.tangent, input.uv_alpha.xy, vec2f(input.uv_alpha.w, input.normal.w), input.object_position, 0.0, input.settings.z, frame.time_size.x, input.material);
    var surface = shaded(material, mapped.normal);
    surface.base = mapped.base;
    surface.roughness = mapped.roughness;
    surface.metalness = mapped.metalness;
    surface.clearcoat_roughness = mapped.coat_roughness;
    surface.thin_film = mapped.thin_film;
    let placed = instances[input.instance];
    let face = placed.content_face.x;
    let shown = face == 0u || (face == 1u && front) || (face == 2u && !front);
    let content_crop = select(vec4f(2.0, 2.0, -1.0, -1.0), placed.content_crop, shown);
    let content_t = normalize(input.tangent.xyz - mapped.normal * dot(input.tangent.xyz, mapped.normal));
    let content_b = cross(mapped.normal, content_t) * input.tangent.w;
    surface = content_apply(surface, input.material, input.uv_alpha.xy, uv_dx, uv_dy, placed.content_uv, content_crop, content_t, content_b, geometric);
    let n = surface.normal;
    let t = normalize(input.tangent.xyz - n * dot(input.tangent.xyz, n));
    let b = cross(n, t) * input.tangent.w;
    let view = normalize(frame.camera_position.xyz - input.world_position);
    let light = normalize(frame.sun_direction.xyz);
    let wo = vec3f(dot(view, t), dot(view, b), dot(view, n));
    let wi = vec3f(dot(light, t), dot(light, b), dot(light, n));
    let facing = max(dot(n, light), 0.0);
    var sun = frame.sun_colour_intensity.xyz * frame.sun_colour_intensity.w * facing;
    if (facing > 0.0) {
        sun *= shadow_visibility(input.world_position, geometric);
        if (lit && any(sun > vec3f(0.0))) {
            sun *= shadow_tint(input.world_position, geometric, input.instance);
        }
        let shaded = all(sun == vec3f(0.0));
        if (use_canopy && frame.flags.z != 0u && !shaded) {
            sun *= canopy_light(input.world_position);
        }
        if (use_caustics && (placed.content_face.z & 2u) != 0u && !shaded) {
            sun *= caustic_gain(caustic_at, caustic_dx, caustic_dy);
        }
    }
    let contacted = use_contact && (placed.content_face.z & 1u) != 0u;
    var contact = vec3f(1.0);
    if (contacted) {
        contact = contact_factor(input.world_position);
        sun *= contact;
    }
    var film = vec3f(0.0);
    var film_normal = vec3f(0.0);
    if (filmed(surface)) {
        film = film_colour(surface, clamp(wo.z, 0.0, 1.0));
        film_normal = film_colour(surface, 1.0);
    }
    var direct = eval_filmed(surface, wo, wi, film) * sun;
    if (lit) {
        var local = local_light_radiance(surface, wo, t, b, n, input.world_position, geometric, film);
        if (contacted) {
            local *= contact;
        }
        direct += local;
    }
    let release = f32((placed.content_face.z >> 8u) & 255u) / 255.0;
    let sky_direction = reflect(-view, n);
    var seen = -1.0;
    let toward = select(vec3f(0.0), sky_direction, reflection_wants_probes(surface, release));
    let grid = sample_live_irradiance_toward(input.world_position, n, toward, &seen);
    var irradiance = surface_irradiance(grid, n, use_authored_ambient && placed.content_face.w != 0u);
    if (contacted) {
        irradiance *= contact;
    }
    let diffuse = surface.base * irradiance / PI * reflection_diffuse_weight(surface, view, n, film, film_normal);
    let previous_uv = input.previous_clip.xy / input.previous_clip.w * vec2f(0.5, -0.5) + vec2f(0.5);
    let view_normal = normalize((frame.view * vec4f(n, 0.0)).xyz);
    var ssr = vec4f(0.0);
    if (use_ssr) {
        ssr = ssr_resolve(ssr_history, ssr_guide, reflect_sampler, previous_uv, frame.time_size.yz, input.previous_clip.w, view_normal);
    }
    var lookup = LocalLookup(vec2f(0.0), sky_direction, sky_direction);
    if (reflection_reflective(surface)) {
        lookup = local_lookup(selected_reflections.first, selected_reflections.second, selected_reflections.weights_mip.xy, input.world_position, sky_direction, local_probes, reflect_sampler, selected_reflections.weights_mip.z);
    }
    let sky_visible = select(1.0, reflection_sky_visibility(seen, sky_direction, lookup, grid, release), any(toward != vec3f(0.0)));
    var specular = reflection_radiance(surface, view, n, ssr, sky_cube, local_probes, selected_reflections.first, selected_reflections.second, lookup, reflect_sampler, selected_reflections.weights_mip.z, selected_reflections.weights_mip.w, film, film_normal, sky_visible);
    if (probe_grid.shade.x > 0.0 && grid.x >= 0.0) {
        let occlusion = mix(probe_reflection_occlusion(grid, reflection_irradiance(n, input.world_position, sky_visible)), 1.0, release);
        if (occlusion < 1.0) {
            specular *= mix(occlusion, 1.0, clamp(ssr.a, 0.0, 1.0));
        }
    }
    var output: OpaqueOutput;
    output.colour = vec4f(max(direct + diffuse + specular + surface.emission, vec3f(0.0)), 1.0);
    output.normal_roughness = vec4f(view_normal, surface.roughness);
    return output;
}
"#;

const CARD_SHADER: &str = r#"
struct FrameUniforms {
    view_projection: mat4x4f,
    motion_view_projection: mat4x4f,
    previous_view_projection: mat4x4f,
    view: mat4x4f,
    camera_position: vec4f,
    sun_direction: vec4f,
    sun_colour_intensity: vec4f,
    time_size: vec4f,
    flags: vec4u,
    wind: vec4f,
    wind_direction: vec4f,
    previous_wind_direction: vec4f,
}
struct SkyLight {
    sh: array<vec4f, 9>,
    info: vec4f,
}
struct CanopyParams {
    center_extent: vec4f,
    sun_receiver: vec4f,
    wind: vec4f,
    direction: vec4f,
    projected_extent: vec4f,
    mode_sharpness: vec4f,
}
struct Card {
    position_size: vec4f,
    right_size: vec4f,
    up: vec4f,
    pivot_level: vec4f,
    motion_color: vec4f,
    atlas: vec4f,
    lod: vec4f,
}
@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(1) @binding(0) var<uniform> shadow_setting: ShadowSample;
@group(1) @binding(1) var shadow_map: texture_depth_2d_array;
@group(1) @binding(2) var shadow_sampler: sampler_comparison;
@group(2) @binding(0) var<uniform> params: CanopyParams;
@group(2) @binding(1) var<storage, read> cards: array<Card>;
@group(2) @binding(2) var alpha_atlas: texture_2d<f32>;
@group(2) @binding(3) var alpha_sampler: sampler;
@group(3) @binding(9) var canopy_cookie: texture_2d<f32>;
@group(3) @binding(10) var canopy_sampler: sampler;
@group(3) @binding(12) var<uniform> sky_light: SkyLight;
fn tree_sway(rest: vec3f, pivot: vec3f, level: f32, stiffness: f32, wind_time: f32, strength: f32, direction: vec3f) -> vec3f {
    let d = normalize(direction.xz + vec2f(0.00001, 0.0));
    let arm = length(rest - pivot);
    let phase = wind_time * (0.7 + level * 0.41) + pivot.x * 0.83 + pivot.z * 0.67;
    let bend = sin(phase) * strength * (1.0 - stiffness) * arm * (0.012 + 0.009 * level);
    return rest + vec3f(d.x * bend, -abs(bend) * 0.12, d.y * bend);
}
struct CardOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) atlas: vec4f,
    @location(2) @interpolate(flat) color: vec3f,
    @location(3) world: vec3f,
    @location(4) @interpolate(flat) normal: vec3f,
    @location(5) current_clip: vec4f,
    @location(6) previous_clip: vec4f,
    @location(7) @interpolate(flat) lod: vec4f,
}
@vertex fn card_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> CardOut {
    let card = cards[instance];
    let corners = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0), vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0));
    let corner = corners[vertex];
    let rest = card.position_size.xyz + card.right_size.xyz * card.position_size.w * corner.x + card.up.xyz * card.right_size.w * corner.y;
    let origin = vec3f(params.center_extent.x, params.wind.z, params.center_extent.y);
    let moved = tree_sway(rest, card.pivot_level.xyz, card.pivot_level.w, card.motion_color.x, frame.wind.x, frame.wind.y, frame.wind_direction.xyz) + origin;
    let before = tree_sway(rest, card.pivot_level.xyz, card.pivot_level.w, card.motion_color.x, frame.wind.z, frame.wind.w, frame.previous_wind_direction.xyz) + origin;
    var out: CardOut;
    out.clip = frame.view_projection * vec4f(moved, 1.0);
    out.current_clip = frame.motion_view_projection * vec4f(moved, 1.0);
    out.previous_clip = frame.previous_view_projection * vec4f(before, 1.0);
    out.uv = (corner + vec2f(1.0)) * 0.5;
    out.atlas = card.atlas;
    out.color = card.motion_color.yzw;
    out.world = moved;
    out.normal = normalize(cross(card.right_size.xyz, card.up.xyz));
    out.lod = card.lod;
    return out;
}
fn card_texel(input: CardOut) -> vec4f {
    return textureSample(alpha_atlas, alpha_sampler, input.atlas.xy + input.uv * input.atlas.zw);
}
fn card_keep(input: CardOut) -> bool {
    if (input.lod.x >= 1.0) { return true; }
    let near = lod_keep(vec2u(input.clip.xy), u32(input.lod.y), input.lod.x);
    return near == (input.lod.z > 0.5);
}
struct PrepassOutput {
    @location(0) velocity: vec2f,
    @location(1) id: u32,
}
@fragment fn card_prepass(input: CardOut) -> PrepassOutput {
    if (card_texel(input).r < 0.5 || !card_keep(input)) {
        discard;
    }
    let current = input.current_clip.xy / input.current_clip.w;
    let previous = input.previous_clip.xy / input.previous_clip.w;
    var output: PrepassOutput;
    output.velocity = (previous - current) * vec2f(0.5, -0.5);
    output.id = frame.flags.w;
    return output;
}
fn card_sky(n: vec3f) -> vec3f {
    let d = normalize(n);
    let sh = sky_light.sh;
    let value = sh[0].xyz * 0.282095 + sh[1].xyz * (0.488603 * d.y) + sh[2].xyz * (0.488603 * d.z)
        + sh[3].xyz * (0.488603 * d.x) + sh[4].xyz * (1.092548 * d.x * d.y) + sh[5].xyz * (1.092548 * d.y * d.z)
        + sh[6].xyz * (0.315392 * (3.0 * d.z * d.z - 1.0)) + sh[7].xyz * (1.092548 * d.x * d.z) + sh[8].xyz * (0.546274 * (d.x * d.x - d.y * d.y));
    return max(value, vec3f(0.0));
}
struct OpaqueOutput {
    @location(0) colour: vec4f,
    @location(1) normal_roughness: vec4f,
}
@fragment fn card_opaque(input: CardOut, @builtin(front_facing) front: bool) -> OpaqueOutput {
    let texel = card_texel(input);
    if (texel.r < 0.5 || !card_keep(input)) {
        discard;
    }
    let n = select(-input.normal, input.normal, front);
    let light = normalize(frame.sun_direction.xyz);
    let facing = dot(n, light);
    let diffuse = max(facing, 0.0);
    let transmission = max(-facing, 0.0);
    var lit = 1.0;
    if (shadow_setting.texels.w < 0.5) {
        lit = shadow_sample(shadow_map, shadow_sampler, canopy_sampler, shadow_setting, input.world, n, length(input.world - frame.camera_position.xyz));
    }
    let projected = input.world.xz - light.xz / max(light.y, 0.05) * (input.world.y - params.sun_receiver.w);
    let cookie_uv = (projected - params.projected_extent.xy) / params.projected_extent.zw + vec2f(0.5);
    if (all(cookie_uv >= vec2f(0.0)) && all(cookie_uv <= vec2f(1.0))) {
        let occlusion = textureSample(canopy_cookie, canopy_sampler, cookie_uv).r;
        let depth = clamp((params.wind.w - input.world.y) / max(params.wind.w - params.wind.z, 0.01), 0.0, 1.0);
        lit *= 1.0 - occlusion * depth * 0.75;
    }
    let pigment = mix(input.color, vec3f(1.0, 0.44, 0.61) * length(input.color) * 0.58, texel.g * 0.7);
    let petal = mix(pigment, vec3f(1.0, 0.72, 0.38) * length(input.color) * 0.58, texel.b * 0.68);
    let sun = frame.sun_colour_intensity.xyz * frame.sun_colour_intensity.w * lit;
    let colour = petal * (card_sky(n) + sun * diffuse) * 0.31830988 + vec3f(1.0, 0.47, 0.64) * sun * transmission * 0.17;
    var output: OpaqueOutput;
    output.colour = vec4f(colour, 1.0);
    output.normal_roughness = vec4f(normalize((frame.view * vec4f(n, 0.0)).xyz), 0.8);
    return output;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deform::{Deformer, RollerFrame};
    use crate::lod::{LodSet, MeshUploader};
    use pfx_geom::shapes::Shape;
    use std::f32::consts::PI;

    fn identity() -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn perspective(aspect: f32) -> Matrix {
        let near = 0.1;
        let far = 100.0;
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ]
    }

    fn look_down(height: f32) -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, -height, 1.0],
        ]
    }

    #[test]
    fn matrix_and_velocity() {
        let mut moved = identity();
        moved[3] = [2.0, -3.0, 4.0, 1.0];
        assert_eq!(
            transform(moved, [1.0, 2.0, 3.0, 1.0]),
            [3.0, -1.0, 7.0, 1.0]
        );
        assert_eq!(multiply(identity(), moved), moved);
        assert_eq!(multiply(moved, identity()), moved);
        let previous = [0.0, 0.0, 0.5, 1.0];
        let current = [0.5, -0.5, 0.5, 1.0];
        assert_eq!(velocity_uv(current, previous), [-0.25, -0.25]);
        assert_eq!(
            velocity_pixels(current, previous, [200.0, 100.0]),
            [-50.0, -25.0]
        );
        assert_eq!(
            velocity_pixels(previous, previous, [200.0, 100.0]),
            [0.0, 0.0]
        );
        let mut scaled = identity();
        scaled[0][0] = 2.0;
        scaled[1][1] = 4.0;
        let normals = normal_columns(scaled).unwrap();
        assert_eq!(normals[0][0], 0.5);
        assert_eq!(normals[1][1], 0.25);
        let shift = jitter_matrix([0.25, -0.5]);
        assert_eq!(
            transform(shift, [0.0, 0.0, 0.3, 1.0]),
            [0.25, -0.5, 0.3, 1.0]
        );
    }

    #[test]
    fn gpu_layouts_and_shader_entry_points() {
        assert_eq!(std::mem::size_of::<Vertex>(), 64);
        assert_eq!(std::mem::size_of::<GpuInstance>(), 304);
        assert_eq!(std::mem::size_of::<Uniforms>(), 384);
        assert_eq!(std::mem::size_of::<SkyLight>(), 160);
        assert_eq!(std::mem::size_of::<GpuSway>(), 32);
        assert_eq!(std::mem::size_of::<PackedMaterial>(), PackedMaterial::SIZE);
        let material = Material {
            base: [0.2, 0.4, 0.6],
            roughness: 0.3,
            ..Default::default()
        };
        let packed = PackedMaterial::pack(&material);
        assert_eq!(packed.base_roughness, [0.2, 0.4, 0.6, 0.3]);
        assert_eq!(packed.metal_spec_coat[1], material.specular);
        let shader = shader_source();
        for entry in [
            "fn eval(",
            "@vertex fn vertex_main",
            "@vertex fn shadow_vertex",
            "@vertex fn shadow_clip_vertex",
            "@fragment fn shadow_clip_fragment",
            "@vertex fn shadow_cut_vertex",
            "@fragment fn shadow_cut_fragment",
            "fn content_cut(",
            "fn content_apply(",
            "fn content_composite(",
            "@fragment fn prepass_main",
            "@fragment fn opaque_main",
            "fn shadow_visibility",
            "fn map_evaluate(",
            "fn sample_live_irradiance(",
            "fn reflection_radiance(",
            "fn canopy_light(",
            "fn deform_vertex(",
            "fn tree_sway(",
            "fn lod_keep(",
            "fn local_light_radiance(",
            "fn shadow_tint(",
            "@vertex fn transmit_vertex",
            "@fragment fn transmit_fragment",
        ] {
            assert!(shader.contains(entry), "{entry}");
        }
    }

    fn binding_kind(module: &naga::Module, variable: &naga::GlobalVariable) -> String {
        match (&variable.space, &module.types[variable.ty].inner) {
            (naga::AddressSpace::Uniform, _) => "uniform".into(),
            (naga::AddressSpace::Storage { access }, _) => {
                if access.contains(naga::StorageAccess::STORE) {
                    "storage".into()
                } else {
                    "read storage".into()
                }
            }
            (_, naga::TypeInner::Sampler { comparison }) => {
                if *comparison {
                    "comparison sampler".into()
                } else {
                    "sampler".into()
                }
            }
            (
                _,
                naga::TypeInner::Image {
                    dim,
                    arrayed,
                    class,
                },
            ) => {
                let class = match class {
                    naga::ImageClass::Depth { .. } => "depth",
                    naga::ImageClass::Sampled {
                        kind: naga::ScalarKind::Uint,
                        ..
                    } => "uint",
                    naga::ImageClass::Sampled { .. } => "float",
                    _ => "storage",
                };
                format!("{class} {dim:?} {arrayed}")
            }
            other => format!("{other:?}"),
        }
    }

    fn entry_kind(entry: &wgpu::BindGroupLayoutEntry) -> String {
        match entry.ty {
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                ..
            } => "uniform".into(),
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                ..
            } => if read_only { "read storage" } else { "storage" }.into(),
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison) => {
                "comparison sampler".into()
            }
            wgpu::BindingType::Sampler(_) => "sampler".into(),
            wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                ..
            } => {
                let class = match sample_type {
                    wgpu::TextureSampleType::Depth => "depth",
                    wgpu::TextureSampleType::Uint => "uint",
                    _ => "float",
                };
                let (dim, arrayed) = match view_dimension {
                    wgpu::TextureViewDimension::D2 => ("D2", false),
                    wgpu::TextureViewDimension::D2Array => ("D2", true),
                    wgpu::TextureViewDimension::Cube => ("Cube", false),
                    wgpu::TextureViewDimension::CubeArray => ("Cube", true),
                    wgpu::TextureViewDimension::D3 => ("D3", false),
                    wgpu::TextureViewDimension::D1 => ("D1", false),
                };
                format!("{class} {dim} {arrayed}")
            }
            _ => "other".into(),
        }
    }

    pub fn check_pipeline(
        source: &str,
        entries: &[&str],
        layouts: &[Vec<wgpu::BindGroupLayoutEntry>],
    ) -> usize {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let mut checked = 0;
        for name in entries {
            let index = module
                .entry_points
                .iter()
                .position(|entry| entry.name == *name)
                .unwrap_or_else(|| panic!("missing entry {name}"));
            let stage = match module.entry_points[index].stage {
                naga::ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
                naga::ShaderStage::Fragment => wgpu::ShaderStages::FRAGMENT,
                _ => wgpu::ShaderStages::COMPUTE,
            };
            let function = info.get_entry_point(index);
            for (handle, variable) in module.global_variables.iter() {
                let Some(binding) = &variable.binding else {
                    continue;
                };
                if function[handle].is_empty() {
                    continue;
                }
                let layout = layouts
                    .get(binding.group as usize)
                    .unwrap_or_else(|| panic!("{name} uses group {}", binding.group));
                let entry = layout
                    .iter()
                    .find(|entry| entry.binding == binding.binding)
                    .unwrap_or_else(|| {
                        panic!(
                            "{name} uses @group({}) @binding({}) missing from the layout",
                            binding.group, binding.binding
                        )
                    });
                assert_eq!(
                    binding_kind(&module, variable),
                    entry_kind(entry),
                    "{name} @group({}) @binding({})",
                    binding.group,
                    binding.binding
                );
                assert!(
                    entry.visibility.contains(stage),
                    "{name} @group({}) @binding({}) is not visible to its stage",
                    binding.group,
                    binding.binding
                );
                checked += 1;
            }
        }
        checked
    }

    #[test]
    fn bind_layout_validates_against_every_pipeline() {
        let frame_layouts = [
            scene_entries(),
            shadow_entries(),
            deformer_entries(),
            lighting_entries(),
        ];
        let checked = check_pipeline(
            &shader_source(),
            &[
                "vertex_main",
                "prepass_main",
                "opaque_main",
                "opaque_lit_main",
            ],
            &frame_layouts,
        );
        assert!(checked >= 20, "{checked} frame bindings checked");
        check_pipeline(
            &shader_source(),
            &[
                "shadow_vertex",
                "shadow_clip_vertex",
                "shadow_clip_fragment",
                "transmit_vertex",
                "transmit_fragment",
            ],
            &[scene_entries(), cascade_entries(), deformer_entries()],
        );
        check_pipeline(
            &shader_source(),
            &["shadow_cut_vertex", "shadow_cut_fragment"],
            &[
                scene_entries(),
                cascade_entries(),
                deformer_entries(),
                lighting_entries(),
            ],
        );
        check_pipeline(
            &card_source(),
            &["card_vertex", "card_prepass", "card_opaque"],
            &[
                scene_entries(),
                shadow_entries(),
                card_entries(),
                lighting_entries(),
            ],
        );
        let maps = format!(
            "{}\n{}",
            maps::shader_source(),
            "@fragment fn probe() -> @location(0) vec4f { let r = map_evaluate(vec3f(1.0), 0.5, 0.0, 0.1, 0.0, vec3f(0.0, 1.0, 0.0), vec4f(1.0, 0.0, 0.0, 1.0), vec2f(0.5), vec3f(0.0), 0.0, 0.5, 0.0, 0u); return vec4f(r.base, 1.0); }"
        );
        check_pipeline(
            &maps,
            &["probe"],
            &[vec![], vec![], vec![], lighting_entries()],
        );
        let probes = format!(
            "{PROBE_WGSL}\n@fragment fn probe() -> @location(0) vec4f {{ return vec4f(sample_live_irradiance(vec3f(0.0), vec3f(0.0, 1.0, 0.0)), 1.0); }}"
        );
        check_pipeline(
            &probes,
            &["probe"],
            &[vec![], vec![], vec![], lighting_entries()],
        );
        let canopy = format!(
            "{}\n@fragment fn probe() -> @location(0) vec4f {{ return vec4f(canopy_light(vec3f(0.0))); }}",
            canopy_light_wgsl(&Gobo::default())
        );
        check_pipeline(
            &canopy,
            &["probe"],
            &[vec![], vec![], vec![], lighting_entries()],
        );
    }

    struct Counter(u32);

    impl MeshUploader for Counter {
        fn upload_mesh(&mut self, _data: MeshData<'_>) -> Result<MeshHandle, String> {
            self.0 += 1;
            Ok(MeshHandle(self.0 + 100))
        }
    }

    fn lod_camera(z: f32) -> Camera {
        let mut projection = identity();
        projection[1][1] = 1.0 / (0.75_f32 * 0.5).tan();
        Camera {
            view: identity(),
            projection,
            previous_view_projection: identity(),
            position: [0.0, 0.0, z],
        }
    }

    #[test]
    fn lod_output_feeds_the_draws() {
        let slab = Shape::Slab {
            half_length: 2.8,
            radius: 1.8,
            height: 0.16,
            fillet: 0.12,
        };
        let mut set = LodSet::new(2_000_000);
        let near = Instance::new(MeshHandle(0), identity(), 0, 1);
        let mut far_model = identity();
        far_model[3][2] = -30.0;
        let far = Instance::new(MeshHandle(0), far_model, 0, 2);
        set.add_shape(near, slab);
        set.add_shape(far, slab);
        let mut uploader = Counter(0);
        set.select(lod_camera(5.0), 2160);
        let close = set.upload_selected(&mut uploader).unwrap();
        let drawn = batches(&close, |_| false, |_| false);
        assert_eq!(drawn.len(), 2);
        assert_eq!(drawn[0].mesh, close[0].mesh);
        assert_eq!(drawn[1].mesh, close[1].mesh);
        assert_ne!(drawn[0].mesh, MeshHandle(0));
        assert_ne!(drawn[0].mesh, drawn[1].mesh);
        assert_eq!(drawn[0].range, 0..1);
        set.select(lod_camera(-25.0), 2160);
        let moved = set.upload_selected(&mut uploader).unwrap();
        let redrawn = batches(&moved, |_| false, |_| false);
        assert_ne!(redrawn[0].mesh, drawn[0].mesh);
        assert_ne!(redrawn[1].mesh, drawn[1].mesh);
        assert!(redrawn.iter().all(|batch| batch.mesh.0 > 100));
    }

    #[test]
    fn batches_split_on_mesh_sidedness_and_motion() {
        let mut instances = vec![Instance::new(MeshHandle(1), identity(), 0, 1); 3];
        instances[1].deformer = DeformerId(0);
        instances.push(Instance::new(MeshHandle(2), identity(), 0, 4));
        let drawn = batches(&instances, |mesh| mesh == MeshHandle(2), |_| false);
        assert_eq!(drawn.len(), 4);
        assert!(!drawn[0].two_sided && !drawn[0].moving);
        assert!(drawn[1].two_sided && drawn[1].moving);
        assert_eq!(drawn[2].range, 2..3);
        assert!(drawn[3].moving && !drawn[3].two_sided);
        let mut surfaces = vec![InstanceSurface::default(); instances.len()];
        let key = static_key(&instances, &drawn, &surfaces);
        instances[3].model[3][0] = 4.0;
        assert_eq!(static_key(&instances, &drawn, &surfaces), key);
        instances[0].model[3][0] = 4.0;
        let moved = static_key(&instances, &drawn, &surfaces);
        assert_ne!(moved, key);
        surfaces[2].clip[0] = [0.0, 1.0, 0.0, 0.1];
        let cut = batches(
            &instances,
            |mesh| mesh == MeshHandle(2),
            |i| surfaces[i].clipped(),
        );
        assert_eq!(cut.len(), 4);
        assert!(cut[2].clipped && !cut[0].clipped);
        assert_ne!(static_key(&instances, &cut, &surfaces), moved);
        let unclipped = vec![Instance::new(MeshHandle(1), identity(), 0, 1); 3];
        let split = batches(&unclipped, |_| false, |i| i == 1);
        assert_eq!(split.len(), 3);
        assert_eq!(split[1].range, 1..2);
    }

    #[test]
    fn instances_that_cast_no_shadow_stay_out_of_every_caster_list() {
        let mut instances = vec![
            Instance::new(MeshHandle(1), identity(), 0, 1),
            Instance::new(MeshHandle(1), identity(), 1, 2),
            Instance::new(MeshHandle(1), identity(), 1, 3),
        ];
        let surfaces = vec![InstanceSurface::default(); instances.len()];
        let all = batches(&instances, |_| false, |_| false);
        assert_eq!(all.len(), 1);
        assert!(all[0].casts_shadow);
        assert_eq!(shadow_batches(&all).len(), 1);
        let key = static_key(&instances, &all, &surfaces);
        instances[1].casts_shadow = false;
        let split = batches(&instances, |_| false, |_| false);
        assert_eq!(split.len(), 3);
        assert_eq!(
            split
                .iter()
                .map(|batch| batch.casts_shadow)
                .collect::<Vec<_>>(),
            [true, false, true]
        );
        let casters = shadow_batches(&split);
        assert_eq!(casters.len(), 2);
        assert!(casters.iter().all(|batch| batch.range != (1..2)));
        let transmitting = transmission_batches(&split, &instances, |material| material == 1);
        assert_eq!(transmitting.len(), 1);
        assert_eq!(transmitting[0].range, 2..3);
        assert_ne!(static_key(&instances, &split, &surfaces), key);
        let before = static_key(&instances, &split, &surfaces);
        instances[1].model[3][0] = 9.0;
        let moved = batches(&instances, |_| false, |_| false);
        assert_eq!(static_key(&instances, &moved, &surfaces), before);
        instances[1].previous_model = identity();
        let restless = batches(&instances, |_| false, |_| false);
        assert!(restless[1].moving);
        assert!(shadow_batches(&restless).iter().all(|batch| !batch.moving));
        assert_eq!(static_key(&instances, &restless, &surfaces), before);
        instances[1].previous_model = instances[1].model;
        instances[1].casts_shadow = true;
        let back = batches(&instances, |_| false, |_| false);
        assert_ne!(static_key(&instances, &back, &surfaces), before);
    }

    #[test]
    fn instance_surfaces_clip_and_place_content() {
        let plain = InstanceSurface::default();
        assert!(!plain.clipped());
        assert!(!plain.cuts([5.0, 5.0, 5.0]));
        assert_eq!(plain.content_uv([0.3, 0.7]), Some([0.3, 0.7]));
        let strip = InstanceSurface {
            clip: [[0.0, 1.0, 0.0, 0.1449], [0.0, -1.0, 0.0, 0.0916]],
            ..Default::default()
        };
        assert!(strip.clipped());
        assert!(strip.cuts([0.0, 0.2, 0.0]));
        assert!(strip.cuts([0.0, -0.1, 0.0]));
        assert!(!strip.cuts([0.0, 0.0, 0.0]));
        let tape = InstanceSurface {
            uv_offset: [0.0, 0.25],
            crop: [0.0, 0.0, 1.0, 2000.0 / 2048.0],
            ..Default::default()
        };
        assert_eq!(tape.content_uv([0.5, 0.5]), Some([0.5, 0.75]));
        assert_eq!(tape.content_uv([0.5, 0.9]), None);
        let broken = InstanceSurface {
            crop: [0.0, f32::NAN, 1.0, 1.0],
            ..Default::default()
        };
        assert!(!broken.valid());
    }

    #[test]
    fn an_instance_takes_all_of_the_reflection_occlusion_unless_it_says_otherwise() {
        let plain = InstanceSurface::default();
        assert_eq!(plain.reflection_occlusion, 1.0);
        assert!(plain.valid());
        assert_eq!(plain.occlusion_release(), 0);
        let release = |scale: f32| {
            InstanceSurface {
                reflection_occlusion: scale,
                ..Default::default()
            }
            .occlusion_release()
        };
        assert_eq!(release(0.0), 255);
        assert_eq!(release(0.5), 128);
        assert_eq!(release(0.25), 191);
        for scale in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            let surface = InstanceSurface {
                reflection_occlusion: scale,
                ..Default::default()
            };
            assert!(!surface.valid(), "{scale}");
        }
    }

    #[test]
    fn ambient_prefers_the_grid_then_the_sky() {
        let mut sh = [[0.0; 3]; 9];
        sh[0] = [1.0, 2.0, 3.0];
        sh[1] = [0.5, 0.5, 0.5];
        let up = sky_irradiance(&sh, [0.0, 1.0, 0.0]);
        let down = sky_irradiance(&sh, [0.0, -1.0, 0.0]);
        assert!(up[0] > down[0]);
        assert_eq!(ambient_irradiance(None, &sh, [0.0, 1.0, 0.0]), up);
        assert_eq!(
            ambient_irradiance(Some([-1.0; 3]), &sh, [0.0, 1.0, 0.0]),
            up
        );
        assert_eq!(
            ambient_irradiance(Some([0.2, 0.3, 0.4]), &sh, [0.0, 1.0, 0.0]),
            [0.2, 0.3, 0.4]
        );
        let lighting = crate::sky::SkyLighting::new(default_sky(), 4);
        let mut settled = lighting.clone();
        while !settled.complete() {
            settled.step();
        }
        for normal in [[0.0, 1.0, 0.0], [0.6, 0.0, 0.8], [0.0, -1.0, 0.0]] {
            let reference = settled.irradiance(normal);
            let ours = sky_irradiance(&settled.sh, normal);
            for channel in 0..3 {
                assert!((reference[channel] - ours[channel]).abs() < 1e-5);
            }
        }
    }

    fn sphere(frame: &mut Frame) -> MeshHandle {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut tangents = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        let rings = 20;
        let sides = 32;
        for ring in 0..=rings {
            let phi = std::f32::consts::PI * ring as f32 / rings as f32;
            for side in 0..=sides {
                let theta = std::f32::consts::TAU * side as f32 / sides as f32;
                let normal = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
                positions.push([normal[0] * 0.75, normal[1] * 0.75, normal[2] * 0.75]);
                normals.push(normal);
                tangents.push([-theta.sin(), 0.0, theta.cos(), 1.0]);
                uvs.push([side as f32 / sides as f32, ring as f32 / rings as f32]);
            }
        }
        for ring in 0..rings {
            for side in 0..sides {
                let a = ring * (sides + 1) + side;
                let b = a + sides + 1;
                indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        frame
            .upload_mesh(MeshData {
                positions: &positions,
                normals: &normals,
                tangents: &tangents,
                uvs: &uvs,
                uvs1: None,
                alpha: None,
                indices: &indices,
            })
            .unwrap()
    }

    fn quad(frame: &mut Frame, corners: [[f32; 3]; 4]) -> MeshHandle {
        frame
            .upload_mesh(MeshData {
                positions: &corners,
                normals: &[[0.0, 1.0, 0.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &[0, 2, 1, 1, 2, 3],
            })
            .unwrap()
    }

    fn plane(frame: &mut Frame) -> MeshHandle {
        quad(
            frame,
            [
                [-3.0, -1.0, -1.0],
                [3.0, -1.0, -1.0],
                [-3.0, -1.0, -9.0],
                [3.0, -1.0, -9.0],
            ],
        )
    }

    fn half(value: u16) -> f32 {
        half::f16::from_bits(value).to_f32()
    }

    fn calm_scene<'a>(
        camera: Camera,
        instances: &'a [Instance],
        materials: &'a [Material],
        sun: Sun,
    ) -> Scene<'a> {
        Scene {
            camera,
            time: 0.0,
            seed: 7,
            sun,
            instances,
            materials,
            deformers: &[],
            wind: SceneWind::default(),
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn warm_opaque_compiles_a_pipeline_on_the_frame_before_any_draw() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = crate::renderer::Renderer::new(gpu, 32, 32).unwrap();
        assert!(renderer.frame().pipelines.opaque.is_empty());
        renderer.warm_opaque(OpaqueFeatures::NONE, false);
        assert_eq!(renderer.frame().pipelines.opaque.len(), 2);
        for key in OpaqueKey::both_sides(OpaqueFeatures::NONE, false) {
            assert!(renderer.frame().pipelines.opaque.contains_key(&key));
        }
        renderer.warm_opaque(OpaqueFeatures::NONE, false);
        assert_eq!(renderer.frame().pipelines.opaque.len(), 2);
        renderer.warm_opaque(OpaqueFeatures::NONE, true);
        assert_eq!(renderer.frame().pipelines.opaque.len(), 4);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_shadow_only_scene_compiles_no_opaque_pipeline() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 32, 32).unwrap();
        let mesh = quad(
            &mut frame,
            [
                [-1.0, 0.0, -2.0],
                [1.0, 0.0, -2.0],
                [-1.0, 0.0, -4.0],
                [1.0, 0.0, -4.0],
            ],
        );
        let sun = Sun {
            direction: [0.2, 0.9, 0.1],
            colour: [1.0; 3],
            intensity: 1.0,
        };
        let materials = [Material::default()];
        let mut hidden = Instance::new(mesh, identity(), 0, 1);
        hidden.shadow_only = true;
        let mut shifted = identity();
        shifted[3][0] = 0.4;
        let mut other = Instance::new(mesh, shifted, 0, 2);
        other.shadow_only = true;
        let draw = |frame: &mut Frame, instances: &[Instance]| {
            let camera = Camera {
                view: look_down(4.0),
                projection: perspective(1.0),
                previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
                position: [0.0, 4.0, 0.0],
            };
            let scene = calm_scene(camera, instances, &materials, sun);
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame.encode(&scene, &mut encoder, None, None).unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
        };
        assert!(frame.pipelines.opaque.is_empty());
        draw(&mut frame, &[hidden, other]);
        assert!(frame.pipelines.opaque.is_empty());
        assert!(frame.opaque_drawn().is_empty());
        draw(&mut frame, &[Instance::new(mesh, identity(), 0, 3)]);
        assert_eq!(frame.pipelines.opaque.len(), 1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn shadow_only_cube_casts_without_colour_or_id() {
        use crate::shadow::{Quality, View};
        use pfx_geom::shapes::Shape;

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 128, 128).unwrap();
        let floor = frame
            .upload_mesh(MeshData {
                positions: &[
                    [-3.0, -1.0, -1.0],
                    [3.0, -1.0, -1.0],
                    [-3.0, -1.0, -9.0],
                    [3.0, -1.0, -9.0],
                ],
                normals: &[[0.0, -1.0, 0.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &[0, 2, 1, 1, 2, 3],
            })
            .unwrap();
        let cube_mesh = Shape::RoundBox {
            half: [0.35; 3],
            radius: 0.01,
        }
        .mesh(0.05);
        let cube = frame
            .upload_mesh(MeshData {
                positions: &cube_mesh.positions,
                normals: &cube_mesh.normals,
                tangents: &cube_mesh.tangents,
                uvs: &cube_mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &cube_mesh.indices,
            })
            .unwrap();
        let mut shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 512,
                ..Default::default()
            },
        );
        let view = View {
            eye: [0.0, 4.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, -1.0],
            fov_y: 0.9,
            aspect: 1.0,
            near: 0.1,
            far: 12.0,
        };
        let fit = shadows.fit(&view, [0.4, 0.9, 0.1]);
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
            position: [0.0, 4.0, 0.0],
        };
        let sun = Sun {
            direction: [0.4, 0.9, 0.1],
            colour: [1.0; 3],
            intensity: 2.0,
        };
        let materials = [Material::default()];
        let mut floor_instance = Instance::new(floor, identity(), 0, 1);
        floor_instance.two_sided = true;
        let mut cube_model = identity();
        cube_model[3][2] = -3.0;
        let mut cube_instance = Instance::new(cube, cube_model, 0, 2);
        cube_instance.shadow_only = true;
        let mut capture = |instances: &[Instance]| {
            let scene = calm_scene(camera, instances, &materials, sun);
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame
                .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
                .unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            (
                read_target(&frame, &frame.targets.ids, 4),
                frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap(),
                read_atlas(&frame, &shadows),
            )
        };
        let empty = capture(&[]);
        let hidden = capture(&[cube_instance]);
        assert_eq!(empty.0, hidden.0);
        assert_eq!(empty.1, hidden.1);
        let baseline = capture(&[floor_instance]);
        let shadowed = capture(&[floor_instance, cube_instance]);
        let ids = shadowed
            .0
            .chunks_exact(4)
            .map(|pixel| u32::from_le_bytes(pixel.try_into().unwrap()));
        assert!(ids.into_iter().all(|id| id != 2));
        assert!(baseline.2.iter().zip(&shadowed.2).any(|(a, b)| a != b));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_sphere_that_casts_no_shadow_leaves_the_floor_as_if_it_were_gone() {
        use crate::shadow::{Quality, View};

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 128, 128).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-3.0, -1.0, -3.0],
                [3.0, -1.0, -3.0],
                [-3.0, -1.0, 3.0],
                [3.0, -1.0, 3.0],
            ],
        );
        let ball = sphere(&mut frame);
        let mut shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 512,
                ..Default::default()
            },
        );
        let view = View {
            eye: [0.0, 4.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, -1.0],
            fov_y: 0.9,
            aspect: 1.0,
            near: 0.1,
            far: 12.0,
        };
        let fit = shadows.fit(&view, [0.4, 0.9, 0.1]);
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
            position: [0.0, 4.0, 0.0],
        };
        let sun = Sun {
            direction: [0.4, 0.9, 0.1],
            colour: [1.0; 3],
            intensity: 2.0,
        };
        let materials = [Material::default()];
        let mut floor_instance = Instance::new(floor, identity(), 0, 1);
        floor_instance.two_sided = true;
        let ball_model = identity();
        let casting = Instance::new(ball, ball_model, 0, 2);
        let mut lit_only = casting;
        lit_only.casts_shadow = false;
        let mut capture = |instances: &[Instance]| {
            let scene = calm_scene(camera, instances, &materials, sun);
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame
                .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
                .unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            (
                read_target(&frame, &frame.targets.ids, 4),
                frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap(),
                read_atlas(&frame, &shadows),
            )
        };
        let removed = capture(&[floor_instance]);
        let unshadowed = capture(&[floor_instance, lit_only]);
        let shadowed = capture(&[floor_instance, casting]);
        let mut nothing = lit_only;
        nothing.shadow_only = true;
        let gone = capture(&[floor_instance, nothing]);
        assert!(gone.0 == removed.0 && gone.1 == removed.1 && gone.2 == removed.2);
        let sphere_pixels = |ids: &[u8]| -> Vec<bool> {
            ids.chunks_exact(4)
                .map(|pixel| u32::from_le_bytes(pixel.try_into().unwrap()) == 2)
                .collect()
        };
        let covered = sphere_pixels(&unshadowed.0);
        assert!(covered.iter().filter(|hit| **hit).count() > 200);
        assert_eq!(covered, sphere_pixels(&shadowed.0));
        assert!(removed.2 == unshadowed.2);
        assert!(removed.2.iter().zip(&shadowed.2).any(|(a, b)| a != b));
        let mut worst = 0.0f32;
        let mut shadow_depth = 0.0f32;
        for (pixel, hit) in covered.iter().enumerate() {
            if *hit {
                continue;
            }
            for channel in 0..3 {
                let at = pixel * 4 + channel;
                worst = worst.max((half(removed.1[at]) - half(unshadowed.1[at])).abs());
                shadow_depth = shadow_depth.max((half(removed.1[at]) - half(shadowed.1[at])).abs());
            }
        }
        assert!(worst <= 1.0 / 255.0, "floor moved by {worst}");
        assert!(shadow_depth > 0.05, "control shadow only {shadow_depth}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_caustic_lights_its_receiver_only_where_the_sun_reaches() {
        use crate::shadow::{Quality, View};
        use pfx_geom::shapes::Shape;

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 128, 128).unwrap();
        let floor = frame
            .upload_mesh(MeshData {
                positions: &[
                    [-3.0, -1.0, -3.0],
                    [3.0, -1.0, -3.0],
                    [-3.0, -1.0, 3.0],
                    [3.0, -1.0, 3.0],
                ],
                normals: &[[0.0, 1.0, 0.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &[0, 2, 1, 1, 2, 3],
            })
            .unwrap();
        let cube_mesh = Shape::RoundBox {
            half: [0.35; 3],
            radius: 0.01,
        }
        .mesh(0.05);
        let cube = frame
            .upload_mesh(MeshData {
                positions: &cube_mesh.positions,
                normals: &cube_mesh.normals,
                tangents: &cube_mesh.tangents,
                uvs: &cube_mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &cube_mesh.indices,
            })
            .unwrap();
        frame.set_content(1, 4, 4, ContentFormat::Linear16).unwrap();
        let texel: Vec<u8> = [1.0f32, 0.5, 0.0, 1.0]
            .iter()
            .flat_map(|v| half::f16::from_f32(*v).to_bits().to_le_bytes())
            .collect();
        frame
            .write_content(1, [0, 0, 4, 4], &texel.repeat(16))
            .unwrap();
        let mut shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 512,
                ..Default::default()
            },
        );
        let toward = [0.1, 1.0, 0.05];
        let fit = shadows.fit(
            &View {
                eye: [0.0, 4.0, 0.0],
                forward: [0.0, -1.0, 0.0],
                up: [0.0, 0.0, -1.0],
                fov_y: 0.9,
                aspect: 1.0,
                near: 0.1,
                far: 12.0,
            },
            toward,
        );
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
            position: [0.0, 4.0, 0.0],
        };
        let materials = [Material::default()];
        let mut floor_instance = Instance::new(floor, identity(), 0, 1);
        floor_instance.two_sided = true;
        let mut caster = Instance::new(cube, identity(), 0, 2);
        caster.shadow_only = true;
        let instances = [floor_instance, caster];
        let caustic = Caustic {
            slot: 1,
            rect: [-3.0, -3.0, 6.0, 6.0],
            height: -1.0,
            strength: 1.0,
            receivers: vec![0],
        };
        let mut capture = |caustic: Option<Caustic>, intensity: f32| {
            frame.set_caustic(caustic).unwrap();
            let sun = Sun {
                direction: toward,
                colour: [1.0; 3],
                intensity,
            };
            let scene = calm_scene(camera, &instances, &materials, sun);
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame
                .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
                .unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            frame
                .gpu
                .readback_rgba16(&frame.targets.hdr)
                .unwrap()
                .chunks_exact(4)
                .map(|p| std::array::from_fn::<f32, 3, _>(|c| half::f16::from_bits(p[c]).to_f32()))
                .collect::<Vec<_>>()
        };
        let plain = capture(None, 2.0);
        let lit = capture(Some(caustic.clone()), 2.0);
        let ambient = capture(Some(caustic.clone()), 0.0);
        let elsewhere = capture(
            Some(Caustic {
                receivers: vec![1],
                ..caustic.clone()
            }),
            2.0,
        );
        let off = capture(
            Some(Caustic {
                rect: [10.0, 10.0, 1.0, 1.0],
                ..caustic
            }),
            2.0,
        );
        assert_eq!(plain, elsewhere, "a caustic lights only its receivers");
        assert_eq!(plain, off, "a caustic lights only inside its rectangle");
        let at = |x: usize, y: usize| y * 128 + x;
        let (open, shade) = (at(104, 62), at(61, 62));
        let sun_open: [f32; 3] = std::array::from_fn(|c| plain[open][c] - ambient[open][c]);
        let sun_shade: [f32; 3] = std::array::from_fn(|c| plain[shade][c] - ambient[shade][c]);
        assert!(
            sun_open.iter().all(|v| *v > 0.1),
            "open floor is sunlit: {sun_open:?}"
        );
        assert!(
            (0..3).all(|c| sun_shade[c] < 0.05 * sun_open[c]),
            "the caster shades the floor: {sun_shade:?} against {sun_open:?}"
        );
        for (c, gain) in [2.0f32, 1.5, 1.0].into_iter().enumerate() {
            let got = (lit[open][c] - ambient[open][c]) / sun_open[c];
            assert!(
                (got - gain).abs() < 0.02,
                "channel {c}: gain {got}, want {gain}"
            );
            assert!(
                (lit[shade][c] - plain[shade][c]).abs() <= 0.01 * plain[shade][c].max(0.05),
                "channel {c}: the caustic shows in the shadow: {} against {}",
                lit[shade][c],
                plain[shade][c]
            );
        }
        assert!(
            frame
                .set_caustic(Some(Caustic {
                    slot: 9,
                    rect: [0.0, 0.0, 1.0, 1.0],
                    height: 0.0,
                    strength: 1.0,
                    receivers: vec![0],
                }))
                .is_err()
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn two_sided_back_face_flips_lit_normal() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let positions = [
            [-0.8, -0.8, -3.0],
            [0.8, -0.8, -3.0],
            [-0.8, 0.8, -3.0],
            [0.8, 0.8, -3.0],
        ];
        let quad = frame
            .upload_mesh(MeshData {
                positions: &positions,
                normals: &[[0.0, 0.0, -1.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &[0, 2, 1, 1, 2, 3],
            })
            .unwrap();
        let camera = Camera {
            view: identity(),
            projection: perspective(1.0),
            previous_view_projection: perspective(1.0),
            position: [0.0; 3],
        };
        let material = [Material::default()];
        let sun = Sun {
            direction: [0.0, 0.0, 1.0],
            colour: [1.0; 3],
            intensity: 4.0,
        };
        let mut instance = Instance::new(quad, identity(), 0, 7);
        let mut capture = |instance: Instance| {
            frame
                .render(&calm_scene(camera, &[instance], &material, sun))
                .unwrap();
            let center = 32 * 64 + 32;
            let ids = read_target(&frame, &frame.targets.ids, 4);
            let normals = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
            (
                u32::from_le_bytes(ids[center * 4..center * 4 + 4].try_into().unwrap()),
                half(normals[center * 4]),
            )
        };
        let one_sided = capture(instance);
        instance.two_sided = true;
        let two_sided = capture(instance);
        assert_eq!(one_sided.0, 0);
        assert_eq!(two_sided.0, 7);
        assert!(two_sided.1 > one_sided.1 + 0.1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn chrome_sphere_reflects_nearby_red_box() {
        use pfx_bake::reflection::{ReflectionManifest, ReflectionSpec, bake_reflection};
        use pfx_bake::{Anchor, BakeScene};
        use pfx_load::Sky;
        use pfx_trace::shapes::Shape as TraceShape;

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let bake_scene = BakeScene {
            triangles: Vec::new(),
            shapes: vec![TraceShape::RoundedBox {
                center: [1.6, 0.0, -3.0],
                half: [0.4; 3],
                radius: 0.02,
                material: 0,
            }],
            materials: vec![Material {
                base: [1.0, 0.0, 0.0],
                emission: [6.0, 0.0, 0.0],
                ..Default::default()
            }],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 1,
                    height: 1,
                    texels: vec![[0.0, 0.0, 0.0, 1.0]],
                },
                sun: pfx_trace::Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [0.0; 3],
                    intensity: 0.0,
                },
            }],
        };
        let spec = ReflectionSpec {
            name: Some("red-box".into()),
            position: [0.0, 0.0, -3.0],
            min: [-2.0, -2.0, -5.0],
            max: [2.0, 2.0, 1.0],
            resolution: 16,
            anchors: None,
            fade: 0.25,
            priority: 0.0,
        };
        let cube = bake_reflection(&gpu, &bake_scene, &spec, 4, 7, 0).unwrap();
        let mut frame = Frame::new(gpu, 128, 128).unwrap();
        let sphere_handle = sphere(&mut frame);
        let box_mesh = Shape::RoundBox {
            half: [0.4; 3],
            radius: 0.02,
        }
        .mesh(0.02);
        let box_handle = frame
            .upload_mesh(MeshData {
                positions: &box_mesh.positions,
                normals: &box_mesh.normals,
                tangents: &box_mesh.tangents,
                uvs: &box_mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &box_mesh.indices,
            })
            .unwrap();
        let mut sphere_model = identity();
        sphere_model[3][2] = -3.0;
        let mut box_model = identity();
        box_model[3] = [1.6, 0.0, -3.0, 1.0];
        let instances = [
            Instance::new(sphere_handle, sphere_model, 0, 1),
            Instance::new(box_handle, box_model, 1, 2),
        ];
        let materials = [
            Material {
                base: [0.95; 3],
                metalness: 1.0,
                roughness: 0.04,
                ..Default::default()
            },
            bake_scene.materials[0],
        ];
        let camera = Camera {
            view: identity(),
            projection: perspective(1.0),
            previous_view_projection: perspective(1.0),
            position: [0.0; 3],
        };
        let scene = calm_scene(
            camera,
            &instances,
            &materials,
            Sun {
                direction: [0.0, 1.0, 0.0],
                colour: [0.0; 3],
                intensity: 0.0,
            },
        );
        frame.render(&scene).unwrap();
        let without = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
        let artifact = ReflectionArtifact {
            manifest: ReflectionManifest {
                schema_version: 1,
                scene_hash: String::new(),
                spec,
                anchors: vec![12.0],
                sample_count: 4,
                seed: 7,
                format: String::new(),
                color_space: String::new(),
                coordinates: String::new(),
                files: Vec::new(),
            },
            cubes: vec![cube],
        };
        frame
            .set_local_reflections(Some(vec![artifact]), 12.0)
            .unwrap();
        frame.render(&scene).unwrap();
        let with = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
        let red = |pixels: &[u16]| {
            (45..80)
                .flat_map(|y| (85..95).map(move |x| (y * 128 + x) * 4))
                .map(|index| half(pixels[index]))
                .sum::<f32>()
        };
        assert!(
            red(&with) > red(&without) + 20.0,
            "without: {}, with: {}",
            red(&without),
            red(&with)
        );
    }

    fn velocity_capture(
        model: Matrix,
        previous_model: Matrix,
        view: Matrix,
        previous_view: Matrix,
    ) -> (Vec<[u16; 2]>, Vec<bool>) {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 128, 128).unwrap();
        let sphere_handle = sphere(&mut frame);
        let mut instance = Instance::new(sphere_handle, model, 0, 7);
        instance.previous_model = previous_model;
        let materials = [Material::default()];
        let camera = Camera {
            view,
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), previous_view),
            position: [0.0; 3],
        };
        let sun = Sun {
            direction: [0.4, 0.8, 0.7],
            colour: [1.0; 3],
            intensity: 2.0,
        };
        let instances = [instance];
        frame
            .render(&calm_scene(camera, &instances, &materials, sun))
            .unwrap();
        let ids = read_target(&frame, &frame.targets.ids, 4);
        let covered = ids
            .chunks_exact(4)
            .map(|pixel| u32::from_le_bytes(pixel.try_into().unwrap()) == 7)
            .collect();
        (frame.readback_velocity().unwrap(), covered)
    }

    fn posed(x: f32, y: f32, z: f32) -> Matrix {
        let mut model = identity();
        model[3] = [x, y, z, 1.0];
        model
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn unmoved_instance_under_static_camera_has_exact_zero_velocity() {
        let model = posed(0.37, -0.21, -3.3);
        let (velocity, covered) = velocity_capture(model, model, identity(), identity());
        assert!(covered.iter().filter(|&&covered| covered).count() > 500);
        let nonzero: Vec<_> = velocity
            .iter()
            .enumerate()
            .filter(|(_, pair)| **pair != [0, 0])
            .map(|(index, pair)| (index % 128, index / 128, covered[index], *pair))
            .collect();
        assert!(
            nonzero.is_empty(),
            "{} nonzero, covered {}, first: {:?}",
            nonzero.len(),
            nonzero.iter().filter(|entry| entry.2).count(),
            &nonzero[..nonzero.len().min(8)]
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn unmoved_instance_under_moving_camera_reads_camera_motion() {
        let model = posed(0.37, -0.21, -3.3);
        let mut previous_view = identity();
        previous_view[3][0] = 0.05;
        let (velocity, covered) = velocity_capture(model, model, identity(), previous_view);
        let focal = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        let near = 0.05 * focal * 0.5 / (3.3 + 0.75);
        let far = 0.05 * focal * 0.5 / (3.3 - 0.75);
        let mut seen = 0;
        for (pair, _) in velocity
            .iter()
            .zip(&covered)
            .filter(|(_, covered)| **covered)
        {
            let x = half(pair[0]);
            assert!(
                x > near * 0.9 && x < far * 1.1,
                "x velocity {x} outside {near}..{far}"
            );
            assert!(half(pair[1]).abs() < far * 0.01);
            seen += 1;
        }
        assert!(seen > 500);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn slowly_moving_instance_keeps_its_velocity() {
        let (velocity, covered) = velocity_capture(
            posed(0.37, -0.21, -3.3),
            posed(0.37 - 1e-5, -0.21, -3.3),
            identity(),
            identity(),
        );
        assert!(covered.iter().filter(|&&covered| covered).count() > 500);
        assert!(
            velocity
                .iter()
                .zip(&covered)
                .any(|(pair, covered)| *covered && pair[0] != 0 && half(pair[0]) < 0.0)
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn headless_sphere_plane_and_4k_timing() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 128, 128).unwrap();
        let sphere_handle = sphere(&mut frame);
        let plane_handle = plane(&mut frame);
        let mut sphere_model = identity();
        sphere_model[3][2] = -3.0;
        let instances = [
            Instance::new(sphere_handle, sphere_model, 0, 17),
            Instance::new(plane_handle, identity(), 1, 23),
        ];
        let materials = [
            Material {
                base: [0.8, 0.2, 0.1],
                roughness: 0.6,
                ..Default::default()
            },
            Material {
                base: [0.4, 0.4, 0.4],
                ..Default::default()
            },
        ];
        let camera = Camera {
            view: identity(),
            projection: perspective(1.0),
            previous_view_projection: perspective(1.0),
            position: [0.0, 0.0, 0.0],
        };
        let sun = Sun {
            direction: [0.4, 0.8, 0.7],
            colour: [1.0; 3],
            intensity: 2.0,
        };
        let scene = calm_scene(camera, &instances, &materials, sun);
        frame.render(&scene).unwrap();
        let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
        let center = (64 * 128 + 64) * 4;
        let luminance = 0.2126 * half(hdr[center])
            + 0.7152 * half(hdr[center + 1])
            + 0.0722 * half(hdr[center + 2]);
        assert!(
            (0.02..3.0).contains(&luminance),
            "sphere luminance: {luminance}"
        );
        assert_eq!(half(hdr[0]), 0.0);
        let velocity = frame.readback_velocity().unwrap();
        assert_eq!(velocity[64 * 128 + 64], [0, 0]);
        drop(frame);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 3840, 2160).unwrap();
        let sphere = sphere(&mut frame);
        let plane = plane(&mut frame);
        let instances = [
            Instance {
                mesh: sphere,
                ..instances[0]
            },
            Instance {
                mesh: plane,
                ..instances[1]
            },
        ];
        let camera = Camera {
            projection: perspective(3840.0 / 2160.0),
            previous_view_projection: perspective(3840.0 / 2160.0),
            ..camera
        };
        let scene = calm_scene(camera, &instances, &materials, sun);
        frame.render(&scene).unwrap();
        frame
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let timings = frame.collect_timings();
        assert!(!timings.is_empty(), "GPU timestamps unavailable");
        for pass in &timings[0] {
            println!("3840x2160 {}: {:.3} ms", pass.label, pass.milliseconds);
        }
    }

    fn write_grid(root: &std::path::Path, irradiance: [f32; 3]) {
        use pfx_bake::{FileEntry, Grid, GridSpec, Manifest, Probe};
        use sha2::{Digest, Sha256};
        let digest = |bytes: &[u8]| -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        if root.exists() {
            std::fs::remove_dir_all(root).unwrap();
        }
        std::fs::create_dir_all(root).unwrap();
        let spec = GridSpec {
            min: [-1.0, -0.5, -1.0],
            max: [0.0, 0.5, 1.0],
            spacing: 0.5,
        };
        let dims = spec.dimensions().unwrap();
        let count = dims.iter().product::<u32>() as usize;
        let probe = Probe {
            lobes: [irradiance; 6],
            visibility: [100.0; 6],
        };
        let grid = Grid::new(spec, vec![probe; count]).unwrap();
        let bytes = grid.bytes();
        std::fs::write(root.join("probe-000.bin"), &bytes).unwrap();
        let manifest = Manifest {
            schema_version: 1,
            scene_hash: "ambient-test".into(),
            anchors: vec![12.0],
            sample_count: 1,
            seed: 0,
            grid: spec,
            dimensions: dims,
            format: "ambient_cube_rgb16f_visibility_f16_le_v1".into(),
            color_space: "linear_rec709_irradiance".into(),
            coordinates: "right_handed_y_up_xyz_x_fastest".into(),
            files: vec![FileEntry {
                path: "probe-000.bin".into(),
                bytes: bytes.len() as u64,
                sha256: digest(&bytes),
            }],
            emitters: None,
        };
        let json = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(root.join("manifest.json"), &json).unwrap();
        std::fs::write(root.join("manifest.sha256"), digest(&json)).unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn ambient_uses_the_grid_where_bound_and_the_sky_elsewhere() {
        let size = 128;
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-2.0, 0.0, -2.0],
                [2.0, 0.0, -2.0],
                [-2.0, 0.0, 2.0],
                [2.0, 0.0, 2.0],
            ],
        );
        let instances = [Instance::new(floor, identity(), 0, 1)];
        let base = 0.8;
        let materials = [Material {
            base: [base; 3],
            roughness: 1.0,
            specular: 0.0,
            ..Default::default()
        }];
        let projection = perspective(1.0);
        let view = look_down(3.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: [0.0, 3.0, 0.0],
        };
        let dark = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let read = |frame: &mut Frame| {
            let scene = calm_scene(camera, &instances, &materials, dark);
            frame.render(&scene).unwrap();
            let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
            let at = |x: u32, y: u32| {
                let i = ((y * size + x) * 4) as usize;
                [half(hdr[i]), half(hdr[i + 1]), half(hdr[i + 2])]
            };
            (at(size / 4, size / 2), at(size * 3 / 4, size / 2))
        };
        let sky_only = read(&mut frame);
        let sh = frame.sky().lighting.sh;
        let expected_sky = sky_irradiance(&sh, [0.0, 1.0, 0.0]).map(|e| base * e / PI);
        for side in [sky_only.0, sky_only.1] {
            for channel in 0..3 {
                let ratio = side[channel] / expected_sky[channel];
                assert!(
                    (0.85..1.2).contains(&ratio),
                    "sky ambient {side:?} against {expected_sky:?}"
                );
            }
        }
        let grid = [0.6, 0.2, 0.1];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/ambient-grid");
        write_grid(&root, grid);
        let probes =
            ProbeLighting::load(&frame.gpu.device, &frame.gpu.queue, &root, 12.0, [-1.0; 3])
                .unwrap();
        frame.set_probes(probes);
        let (inside, outside) = read(&mut frame);
        let expected_grid = grid.map(|e| base * e / PI);
        for channel in 0..3 {
            let ratio = inside[channel] / expected_grid[channel];
            assert!(
                (0.85..1.2).contains(&ratio),
                "grid ambient {inside:?} against {expected_grid:?}"
            );
            assert!((outside[channel] - sky_only.1[channel]).abs() < 0.01);
        }
        assert!(inside[0] > inside[2] * 3.0);
        assert!(outside[2] > outside[0]);
        println!(
            "grid side {inside:?}, sky side {outside:?}, expected {expected_grid:?} and {expected_sky:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn reflection_occlusion_dims_a_dark_grid_and_an_instance_can_keep_its_reflection() {
        let size = 64;
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-2.0, 0.0, -2.0],
                [2.0, 0.0, -2.0],
                [-2.0, 0.0, 2.0],
                [2.0, 0.0, 2.0],
            ],
        );
        let instances = [Instance::new(floor, identity(), 0, 1)];
        let materials = [Material {
            base: [0.9; 3],
            metalness: 1.0,
            roughness: 0.3,
            ..Default::default()
        }];
        let projection = perspective(1.0);
        let view = look_down(3.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: [0.0, 3.0, 0.0],
        };
        let dark = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/reflection-occlusion-grid");
        write_grid(&root, [0.01; 3]);
        let probes =
            ProbeLighting::load(&frame.gpu.device, &frame.gpu.queue, &root, 12.0, [-1.0; 3])
                .unwrap();
        frame.set_probes(probes);
        frame.set_sky_visibility(false);
        let read = |frame: &mut Frame, scale: f32| {
            frame
                .set_surfaces(&[InstanceSurface {
                    reflection_occlusion: scale,
                    ..Default::default()
                }])
                .unwrap();
            let scene = calm_scene(camera, &instances, &materials, dark);
            frame.render(&scene).unwrap();
            let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
            let at = |x: u32, y: u32| {
                let i = ((y * size + x) * 4) as usize;
                [half(hdr[i]), half(hdr[i + 1]), half(hdr[i + 2])]
            };
            (at(size / 4, size / 2), at(size * 3 / 4, size / 2))
        };
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let open = read(&mut frame, 1.0);
        for scale in [0.0, 0.5] {
            assert_eq!(
                read(&mut frame, scale),
                open,
                "occlusion off, scale {scale}"
            );
        }
        frame.set_reflection_occlusion(1.0).unwrap();
        assert_eq!(frame.probes().reflection_occlusion(), 1.0);
        let full = read(&mut frame, 1.0);
        let half_kept = read(&mut frame, 0.5);
        let kept = read(&mut frame, 0.0);
        println!(
            "under the dark grid: open {:.4}, scale 1 {:.4}, scale 0.5 {:.4}, scale 0 {:.4}",
            luma(open.0),
            luma(full.0),
            luma(half_kept.0),
            luma(kept.0)
        );
        assert!(luma(full.0) < luma(open.0) * 0.5);
        assert!(luma(full.0) < luma(half_kept.0) && luma(half_kept.0) < luma(open.0));
        assert_eq!(kept, open);
        assert_eq!(full.1, open.1);
        assert!(frame.set_reflection_occlusion(1.5).is_err());
        assert!(
            frame
                .set_surfaces(&[InstanceSurface {
                    reflection_occlusion: -0.5,
                    ..Default::default()
                }])
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_authored_ambient_is_flat_and_the_default_is_the_sky() {
        let mut sh = [[0.0; 3]; 9];
        sh[0] = [1.0, 2.0, 3.0];
        sh[1] = [0.5, 0.5, 0.5];
        assert_eq!(Ambient::default(), Ambient::Sky);
        assert_eq!(Ambient::Sky.sh(sh), sh);
        let constant = [0.31 * PI, 0.305 * PI, 0.43 * PI];
        let flat = Ambient::Constant(constant).sh(sh);
        for normal in [[0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.6, 0.0, 0.8]] {
            let got = sky_irradiance(&flat, normal);
            for channel in 0..3 {
                assert!((got[channel] - constant[channel]).abs() < 1e-5);
            }
        }
        let own = [[0.25; 3]; 9];
        assert_eq!(Ambient::Sh(own).sh(sh), own);
        assert!(Ambient::Constant(constant).valid());
        assert!(!Ambient::Constant([-0.1, 0.0, 0.0]).valid());
        assert!(!Ambient::Constant([f32::NAN, 0.0, 0.0]).valid());
        assert!(!Ambient::Sh([[f32::INFINITY; 3]; 9]).valid());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn an_authored_ambient_survives_set_probes_and_the_default_is_unchanged() {
        let size = 128;
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-2.0, 0.0, -2.0],
                [2.0, 0.0, -2.0],
                [-2.0, 0.0, 2.0],
                [2.0, 0.0, 2.0],
            ],
        );
        let instances = [Instance::new(floor, identity(), 0, 1)];
        let base = 0.8;
        let materials = [Material {
            base: [base; 3],
            roughness: 1.0,
            specular: 0.0,
            ..Default::default()
        }];
        let projection = perspective(1.0);
        let view = look_down(3.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: [0.0, 3.0, 0.0],
        };
        let dark = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let read = |frame: &mut Frame| {
            let scene = calm_scene(camera, &instances, &materials, dark);
            frame.render(&scene).unwrap();
            let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
            let i = (((size / 2) * size + size / 2) * 4) as usize;
            [half(hdr[i]), half(hdr[i + 1]), half(hdr[i + 2])]
        };
        let sky_first = read(&mut frame);
        let fresh = ProbeLighting::fallback(&frame.gpu.device, &frame.gpu.queue, [-1.0; 3]);
        frame.set_probes(fresh);
        let sky_after = read(&mut frame);
        for channel in 0..3 {
            assert_eq!(sky_first[channel], sky_after[channel]);
        }
        let authored = [0.31 * PI, 0.305 * PI, 0.43 * PI];
        frame.set_ambient(Ambient::Constant(authored)).unwrap();
        let fresh = ProbeLighting::fallback(&frame.gpu.device, &frame.gpu.queue, [-1.0; 3]);
        frame.set_probes(fresh);
        assert_eq!(frame.ambient(), Ambient::Constant(authored));
        let unflagged = read(&mut frame);
        for channel in 0..3 {
            assert_eq!(sky_first[channel], unflagged[channel]);
        }
        frame
            .set_surfaces(&[InstanceSurface {
                authored_ambient: true,
                ..Default::default()
            }])
            .unwrap();
        let flat = read(&mut frame);
        for channel in 0..3 {
            let expected = base * authored[channel] / PI;
            assert!(
                (flat[channel] / expected - 1.0).abs() < 0.03,
                "authored ambient {flat:?} against {authored:?}"
            );
        }
        assert!(flat[2] > flat[0]);
        assert!(
            frame
                .set_ambient(Ambient::Constant([-1.0, 0.0, 0.0]))
                .is_err()
        );
        assert_eq!(frame.ambient(), Ambient::Constant(authored));
        frame.set_ambient(Ambient::Sky).unwrap();
        let back = read(&mut frame);
        for channel in 0..3 {
            assert_eq!(sky_first[channel], back[channel]);
        }
        println!("sky {sky_first:?}, authored {flat:?}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn contact_darkens_flagged_receivers_by_mix_of_tint_and_one_over_the_field_squared() {
        let size = 128;
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-2.0, 0.0, -2.0],
                [2.0, 0.0, -2.0],
                [-2.0, 0.0, 2.0],
                [2.0, 0.0, 2.0],
            ],
        );
        let instances = [Instance::new(floor, identity(), 0, 1)];
        let materials = [Material {
            base: [0.8; 3],
            roughness: 1.0,
            specular: 0.0,
            ..Default::default()
        }];
        let projection = perspective(1.0);
        let view = look_down(3.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: [0.0, 3.0, 0.0],
        };
        let dark = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let read = |frame: &mut Frame| {
            let scene = calm_scene(camera, &instances, &materials, dark);
            frame.render(&scene).unwrap();
            let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
            let at = |x: u32, y: u32| {
                let i = ((y * size + x) * 4) as usize;
                [half(hdr[i]), half(hdr[i + 1]), half(hdr[i + 2])]
            };
            (at(size / 4, size / 2), at(size * 3 / 4, size / 2))
        };
        let open = read(&mut frame);
        let tint = [0.34, 0.34, 0.50];
        frame
            .set_contact(Some(ContactDesc {
                size: [64, 2],
                min: [-2.0, -2.0],
                max: [2.0, 2.0],
                tint,
            }))
            .unwrap();
        let field: Vec<f32> = (0..128)
            .map(|i| if i % 64 < 32 { 0.5 } else { 1.0 })
            .collect();
        frame.write_contact(&field).unwrap();
        let unflagged = read(&mut frame);
        for channel in 0..3 {
            assert_eq!(unflagged.0[channel], open.0[channel]);
            assert_eq!(unflagged.1[channel], open.1[channel]);
        }
        frame
            .set_surfaces(&[InstanceSurface {
                contact: true,
                ..Default::default()
            }])
            .unwrap();
        let (dim, clear) = read(&mut frame);
        for channel in 0..3 {
            let wanted = tint[channel] + (1.0 - tint[channel]) * 0.25;
            let got = dim[channel] / open.0[channel];
            assert!(
                (got / wanted - 1.0).abs() < 0.02,
                "contact {got} against {wanted} in channel {channel}"
            );
            assert!((clear[channel] / open.1[channel] - 1.0).abs() < 0.01);
        }
        println!("open {open:?}, flagged {dim:?} and {clear:?}");
        frame.set_contact(None).unwrap();
        let removed = read(&mut frame);
        for channel in 0..3 {
            assert_eq!(removed.0[channel], open.0[channel]);
        }
        assert!(frame.write_contact(&field).is_err());
    }

    fn read_atlas(frame: &Frame, shadows: &Shadows) -> Vec<f32> {
        let resolution = shadows.resolution();
        let row = resolution * 4;
        let buffer = frame.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas readback"),
            size: u64::from(row * resolution) * CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: shadows.atlas(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(resolution),
                },
            },
            wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: CASCADE_COUNT as u32,
            },
        );
        frame.gpu.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        frame
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let values = bytemuck::cast_slice::<u8, f32>(&buffer.slice(..).get_mapped_range()).to_vec();
        buffer.unmap();
        values
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn deformation_reaches_the_shadow_pass() {
        use crate::shadow::{Quality, View};
        use pfx_geom::strip::{Roller, Strip};
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 256, 256).unwrap();
        let strip = Strip {
            length: 3.0,
            width: 1.2,
        };
        let mesh = strip.mesh(0.12, 0.002, None, 4);
        let handle = frame
            .upload_mesh(MeshData {
                positions: &mesh.positions,
                normals: &mesh.normals,
                tangents: &mesh.tangents,
                uvs: &mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &mesh.indices,
            })
            .unwrap();
        let mut shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 512,
                ..Default::default()
            },
        );
        let view = View {
            eye: [1.5, 4.0, 4.0],
            forward: [0.0, -0.7, -0.7],
            up: [0.0, 1.0, 0.0],
            fov_y: 0.9,
            aspect: 1.0,
            near: 0.1,
            far: 12.0,
        };
        let fit = shadows.fit(&view, [0.3, 0.9, 0.2]);
        let mut instance = Instance::new(handle, identity(), 0, 1);
        instance.deformer = DeformerId(0);
        let instances = [instance];
        let materials = [Material::default()];
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: perspective(1.0),
            position: [0.0, 4.0, 0.0],
        };
        let mut render = |at: f32| {
            let roller = Roller {
                at,
                core: 0.12,
                thickness: 0.01,
            };
            let state = RollerFrame {
                at,
                outer: roller.outer(&strip),
                thickness: roller.thickness,
            };
            let deformers = [Deformer::Roller {
                current: state,
                previous: state,
            }];
            let scene = Scene {
                deformers: &deformers,
                ..calm_scene(
                    camera,
                    &instances,
                    &materials,
                    Sun {
                        direction: [0.3, 0.9, 0.2],
                        colour: [1.0; 3],
                        intensity: 1.0,
                    },
                )
            };
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame
                .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
                .unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            read_atlas(&frame, &shadows)
        };
        let flat = render(3.0);
        let rolled = render(1.2);
        let flat_again = render(3.0);
        assert_eq!(flat, flat_again);
        let covered = |depths: &[f32]| depths.iter().filter(|depth| **depth < 1.0).count();
        let changed = flat
            .iter()
            .zip(&rolled)
            .filter(|(a, b)| (**a - **b).abs() > 1e-4)
            .count();
        println!(
            "flat casters {} texels, rolled casters {} texels, {changed} texels changed",
            covered(&flat),
            covered(&rolled)
        );
        assert!(covered(&flat) > 1000);
        assert!(changed > 500);
    }
    fn read_target(frame: &Frame, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
        let size = texture.size();
        let row = (size.width * texel).div_ceil(256) * 256;
        let buffer = frame.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test readback"),
            size: u64::from(row * size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size.height),
                },
            },
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        frame.gpu.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        frame
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        let mut out = Vec::new();
        for y in 0..size.height {
            let start = (y * row) as usize;
            out.extend_from_slice(&mapped[start..start + (size.width * texel) as usize]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }

    struct Outputs {
        ids: Vec<u32>,
        depth: Vec<f32>,
        colour: Vec<[f32; 3]>,
        atlas: Vec<f32>,
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn clip_planes_cut_prepass_opaque_shadow_and_ids_alike() {
        use crate::shadow::{Quality, View};
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let size = 256;
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-4.0, 0.0, -4.0],
                [4.0, 0.0, -4.0],
                [-4.0, 0.0, 4.0],
                [4.0, 0.0, 4.0],
            ],
        );
        let card = |x0: f32, x1: f32| {
            [
                [x0, 1.0, -1.0],
                [x1, 1.0, -1.0],
                [x0, 1.0, 1.0],
                [x1, 1.0, 1.0],
            ]
        };
        let whole = quad(&mut frame, card(-1.0, 1.0));
        let half_card = quad(&mut frame, card(-1.0, 0.0));
        let mut shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 1024,
                ..Default::default()
            },
        );
        let sun_toward = {
            let v: [f32; 3] = [0.6, 0.8, 0.0];
            let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.map(|x| x / length)
        };
        let view = View {
            eye: [0.0, 4.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, -1.0],
            fov_y: 50.0_f32.to_radians(),
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        };
        let fit = shadows.fit(&view, sun_toward);
        let materials = [
            Material {
                base: [0.6, 0.6, 0.6],
                roughness: 0.8,
                ..Default::default()
            },
            Material {
                base: [0.8, 0.1, 0.1],
                roughness: 0.8,
                ..Default::default()
            },
        ];
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
            position: [0.0, 4.0, 0.0],
        };
        let sun = Sun {
            direction: sun_toward,
            colour: [1.0; 3],
            intensity: 3.0,
        };
        let mut render = |card: MeshHandle, surfaces: &[InstanceSurface]| {
            let instances = [
                Instance::new(floor, identity(), 0, 1),
                Instance::new(card, identity(), 1, 2),
            ];
            frame.set_surfaces(surfaces).unwrap();
            let scene = calm_scene(camera, &instances, &materials, sun);
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame
                .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
                .unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            let ids = read_target(&frame, &frame.targets.ids, 4)
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let depth = read_target(&frame, &frame.targets.depth, 4)
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let colour = frame
                .gpu
                .readback_rgba16(&frame.targets.hdr)
                .unwrap()
                .chunks_exact(4)
                .map(|p| [half(p[0]), half(p[1]), half(p[2])])
                .collect();
            Outputs {
                ids,
                depth,
                colour,
                atlas: read_atlas(&frame, &shadows),
            }
        };
        let cut = InstanceSurface {
            clip: [[1.0, 0.0, 0.0, 0.0], [0.0; 4]],
            ..Default::default()
        };
        let clipped = render(whole, &[InstanceSurface::default(), cut]);
        let reference = render(half_card, &[]);
        let uncut = render(whole, &[]);
        let differ = |a: &[f32], b: &[f32], tolerance: f32| {
            a.iter()
                .zip(b)
                .filter(|(x, y)| (**x - **y).abs() > tolerance)
                .count()
        };
        let id_mismatch = clipped
            .ids
            .iter()
            .zip(&reference.ids)
            .filter(|(a, b)| a != b)
            .count();
        let depth_mismatch = differ(&clipped.depth, &reference.depth, 1e-6);
        let colour_mismatch = clipped
            .colour
            .iter()
            .zip(&reference.colour)
            .filter(|(a, b)| (0..3).any(|c| (a[c] - b[c]).abs() > 0.02))
            .count();
        let atlas_mismatch = differ(&clipped.atlas, &reference.atlas, 1e-4);
        let atlas_cut = differ(&uncut.atlas, &reference.atlas, 1e-4);
        let id_cut = uncut
            .ids
            .iter()
            .zip(&reference.ids)
            .filter(|(a, b)| a != b)
            .count();
        println!(
            "clip against a half card: ids {id_mismatch}, depth {depth_mismatch}, colour {colour_mismatch} of {} pixels, shadow {atlas_mismatch} of {} texels; the uncut card differs in {id_cut} ids and {atlas_cut} shadow texels",
            clipped.ids.len(),
            clipped.atlas.len()
        );
        assert!(id_cut > 2000 && atlas_cut > 1000);
        let seam = size as usize * 2;
        assert!(id_mismatch <= seam, "{id_mismatch} ids differ");
        assert!(depth_mismatch <= seam, "{depth_mismatch} depths differ");
        assert!(
            colour_mismatch <= seam * 2,
            "{colour_mismatch} colours differ"
        );
        assert!(
            atlas_mismatch * 20 <= atlas_cut,
            "{atlas_mismatch} shadow texels differ"
        );
        let at = |x: f32, z: f32| {
            let ndc = transform(multiply(perspective(1.0), look_down(4.0)), [x, 0.0, z, 1.0]);
            let px = ((ndc[0] / ndc[3] * 0.5 + 0.5) * size as f32) as usize;
            let py = ((0.5 - ndc[1] / ndc[3] * 0.5) * size as f32) as usize;
            py * size as usize + px
        };
        let shown = at(0.6, 0.0);
        assert_eq!(clipped.ids[shown], 1);
        assert_eq!(uncut.ids[shown], 2);
        assert_eq!(clipped.ids[at(-0.6, 0.0)], 2);
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let open_floor = luma(clipped.colour[at(1.6, 0.0)]);
        let under_cut = luma(clipped.colour[at(0.15, 0.0)]);
        let under_card = luma(clipped.colour[at(-1.5, 0.0)]);
        println!(
            "floor luma: open {open_floor}, under the cut half {under_cut}, in the card's shadow {under_card}"
        );
        assert!((under_cut - open_floor).abs() < 0.1 * open_floor);
        assert!(under_card < 0.6 * open_floor);
    }

    const RING: u32 = 64;

    fn ring_alpha(i: u32, j: u32) -> u8 {
        let u = (i as f32 + 0.5) / RING as f32 - 0.5;
        let v = (j as f32 + 0.5) / RING as f32 - 0.5;
        let r = (u * u + v * v).sqrt();
        if (0.25..=0.45).contains(&r) { 255 } else { 0 }
    }

    fn traced_alpha(uv: [f32; 2]) -> f32 {
        let size = RING as f32;
        let p = [0, 1].map(|k| (uv[k] * size - 0.5).clamp(0.0, size - 1.0));
        let lo = p.map(|value| value.floor() as u32);
        let hi = lo.map(|value| (value + 1).min(RING - 1));
        let f = [p[0] - p[0].floor(), p[1] - p[1].floor()];
        let a = |i: u32, j: u32| f32::from(ring_alpha(i, j)) / 255.0;
        let top = a(lo[0], lo[1]) * (1.0 - f[0]) + a(hi[0], lo[1]) * f[0];
        let bottom = a(lo[0], hi[1]) * (1.0 - f[0]) + a(hi[0], hi[1]) * f[0];
        top * (1.0 - f[1]) + bottom * f[1]
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn content_alpha_cuts_a_ring_out_of_one_quad_in_every_pass() {
        use crate::maps::ContentFormat;
        use crate::shadow::{Quality, View};
        use pfx_materials::ContentLayer;
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let size = 256;
        let height = 6.0;
        let half_card = 0.6;
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let floor = quad(
            &mut frame,
            [
                [-4.0, 0.0, -4.0],
                [4.0, 0.0, -4.0],
                [-4.0, 0.0, 4.0],
                [4.0, 0.0, 4.0],
            ],
        );
        let card = quad(
            &mut frame,
            [
                [-half_card, 1.0, -half_card],
                [half_card, 1.0, -half_card],
                [-half_card, 1.0, half_card],
                [half_card, 1.0, half_card],
            ],
        );
        let texels: Vec<u8> = (0..RING * RING)
            .flat_map(|index| [230, 40, 30, ring_alpha(index % RING, index / RING)])
            .collect();
        frame
            .set_content(0, RING, RING, ContentFormat::Srgb8)
            .unwrap();
        frame.write_content(0, [0, 0, RING, RING], &texels).unwrap();
        let mut shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 1024,
                ..Default::default()
            },
        );
        let sun_toward = [0.8, 0.6, 0.0];
        let view = View {
            eye: [0.0, height, 0.0],
            forward: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, -1.0],
            fov_y: 50.0_f32.to_radians(),
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        };
        let fit = shadows.fit(&view, sun_toward);
        let materials = [
            Material {
                base: [0.6, 0.6, 0.6],
                roughness: 0.8,
                ..Default::default()
            },
            Material {
                base: [0.2, 0.2, 0.8],
                roughness: 0.8,
                content_layer: ContentLayer {
                    slot: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        let camera = Camera {
            view: look_down(height),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(height)),
            position: [0.0, height, 0.0],
        };
        let sun = Sun {
            direction: sun_toward,
            colour: [1.0; 3],
            intensity: 3.0,
        };
        let mut render = |cutout: bool| {
            let instances = [
                Instance::new(floor, identity(), 0, 1),
                Instance::new(card, identity(), 1, 2),
            ];
            let surfaces = [
                InstanceSurface::default(),
                InstanceSurface {
                    cutout,
                    ..Default::default()
                },
            ];
            frame.set_surfaces(&surfaces).unwrap();
            let scene = calm_scene(camera, &instances, &materials, sun);
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            frame
                .encode(&scene, &mut encoder, Some((&mut shadows, &fit)), None)
                .unwrap();
            frame.gpu.queue.submit(Some(encoder.finish()));
            let ids: Vec<u32> = read_target(&frame, &frame.targets.ids, 4)
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            let colour: Vec<[f32; 3]> = frame
                .gpu
                .readback_rgba16(&frame.targets.hdr)
                .unwrap()
                .chunks_exact(4)
                .map(|p| [half(p[0]), half(p[1]), half(p[2])])
                .collect();
            (ids, colour, frame.opaque_drawn().to_vec())
        };
        let (ids, colour, drawn) = render(true);
        let (whole_ids, whole_colour, whole_drawn) = render(false);
        assert!(drawn.iter().any(|used| used.content_cutout));
        assert!(whole_drawn.iter().all(|used| !used.content_cutout));
        let at = |x: f32, y: f32, z: f32| {
            let ndc = transform(
                multiply(perspective(1.0), look_down(height)),
                [x, y, z, 1.0],
            );
            let px = ((ndc[0] / ndc[3] * 0.5 + 0.5) * size as f32) as usize;
            let py = ((0.5 - ndc[1] / ndc[3] * 0.5) * size as f32) as usize;
            py * size as usize + px
        };
        assert_eq!(ids[at(0.0, 1.0, 0.0)], 1, "the hole shows the floor");
        assert_eq!(ids[at(0.42, 1.0, 0.0)], 2, "the ring shows");
        assert_eq!(
            ids[at(0.55, 1.0, 0.55)],
            1,
            "the clear corner shows the floor"
        );
        assert_eq!(whole_ids[at(0.0, 1.0, 0.0)], 2);
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        let mut checked = 0;
        let mut wrong = 0;
        for py in 0..size {
            for px in 0..size {
                let ndc = [
                    (px as f32 + 0.5) / size as f32 * 2.0 - 1.0,
                    1.0 - (py as f32 + 0.5) / size as f32 * 2.0,
                ];
                let x = ndc[0] * (height - 1.0) / f;
                let z = -ndc[1] * (height - 1.0) / f;
                if x.abs() > half_card || z.abs() > half_card {
                    continue;
                }
                let uv = [
                    (x + half_card) / (2.0 * half_card),
                    (z + half_card) / (2.0 * half_card),
                ];
                let alpha = traced_alpha(uv);
                if (alpha - 0.5).abs() < 0.05 {
                    continue;
                }
                checked += 1;
                let shown = ids[(py * size + px) as usize] == 2;
                if shown != (alpha >= 0.5) {
                    wrong += 1;
                }
            }
        }
        println!("{wrong} of {checked} card pixels disagree with the tracer's cutout");
        assert!(checked > 3000, "{checked}");
        assert_eq!(wrong, 0);
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let shift = sun_toward[0] / sun_toward[1];
        let shadow_of = |x: f32, z: f32| at(x - shift, 0.0, z);
        let open = luma(colour[at(1.5, 0.0, -1.5)]);
        let through_hole = luma(colour[shadow_of(0.0, 0.0)]);
        let under_ring = luma(colour[shadow_of(-0.42, 0.0)]);
        let past_corner = luma(colour[shadow_of(-0.55, 0.55)]);
        let whole_hole = luma(whole_colour[shadow_of(0.0, 0.0)]);
        println!(
            "floor luma: open {open}, through the hole {through_hole}, under the ring {under_ring}, past the clear corner {past_corner}; the uncut card shades the hole's spot to {whole_hole}"
        );
        assert!((through_hole - open).abs() < 0.05 * open);
        assert!((past_corner - open).abs() < 0.05 * open);
        assert!(under_ring < 0.6 * open);
        assert!(whole_hole < 0.6 * open);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn content_updates_keep_the_arrays_and_regenerate_mips() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let paper = pfx_load::Image {
            width: 4,
            height: 4,
            space: pfx_load::ColorSpace::Srgb,
            pixels: pfx_load::Pixels::Eight([200, 190, 170, 255].repeat(16)),
        };
        frame
            .set_maps(&MapImages {
                base: std::slice::from_ref(&paper),
                ..Default::default()
            })
            .unwrap();
        let arrays = [
            frame.maps.base.texture.clone(),
            frame.maps.normal.texture.clone(),
            frame.maps.roughness.texture.clone(),
            frame.maps.metal.texture.clone(),
        ];
        frame
            .set_content(3, 1280, 720, ContentFormat::Srgb8)
            .unwrap();
        let video = frame.content().get(3).unwrap().texture.clone();
        assert_eq!(frame.content().get(3).unwrap().mips, 11);
        let started = std::time::Instant::now();
        for (index, colour) in [[255u8, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]]
            .iter()
            .enumerate()
        {
            frame
                .write_content(3, [0, 0, 1280, 720], &colour.repeat(1280 * 720))
                .unwrap();
            frame
                .set_content(3, 1280, 720, ContentFormat::Srgb8)
                .unwrap();
            let content = frame.content();
            let slot = content.get(3).unwrap();
            assert_eq!(slot.texture, video);
            assert_eq!(slot.writes, index as u64 + 1);
            let last = slot.mips - 1;
            let buffer = frame.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("smallest mip"),
                size: 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &slot.texture,
                    mip_level: last,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            frame.gpu.queue.submit(Some(encoder.finish()));
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            frame
                .gpu
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let texel = buffer.slice(..).get_mapped_range()[..4].to_vec();
            assert_eq!(texel, colour.to_vec(), "frame {index}: the smallest mip");
        }
        println!(
            "three 1280x720 frames written with mips in {:.2} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(frame.maps.base.texture, arrays[0]);
        assert_eq!(frame.maps.normal.texture, arrays[1]);
        assert_eq!(frame.maps.roughness.texture, arrays[2]);
        assert_eq!(frame.maps.metal.texture, arrays[3]);
        assert!(frame.write_content(2, [0, 0, 1, 1], &[0; 4]).is_err());
        assert!(frame.write_content(3, [1279, 0, 2, 1], &[0; 8]).is_err());
        assert!(frame.set_content(4, 4, 4, ContentFormat::Linear16).is_err());
        frame
            .set_content(3, 640, 360, ContentFormat::Srgb8)
            .unwrap();
        assert_ne!(frame.content().get(3).unwrap().texture, video);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn screen_content_emits_through_the_frame() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let size = 64;
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let screen = quad(
            &mut frame,
            [
                [-2.0, 0.0, -2.0],
                [2.0, 0.0, -2.0],
                [-2.0, 0.0, 2.0],
                [2.0, 0.0, 2.0],
            ],
        );
        frame.set_content(0, 4, 4, ContentFormat::Srgb8).unwrap();
        frame
            .write_content(0, [0, 0, 4, 4], &[0u8, 255, 0, 255].repeat(16))
            .unwrap();
        let dark = Material {
            base: [0.01; 3],
            roughness: 0.3,
            ..Default::default()
        };
        let lit = Material {
            content_layer: pfx_materials::ContentLayer::from_kind(
                pfx_materials::Content::Screen,
                1.0,
                0,
                &pfx_materials::ContentLook {
                    screen_gain: 1.5,
                    ..Default::default()
                },
            ),
            ..dark
        };
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
            position: [0.0, 4.0, 0.0],
        };
        let sun = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let mut centre = |materials: &[Material]| {
            let instances = [Instance::new(screen, identity(), 0, 1)];
            frame
                .render(&calm_scene(camera, &instances, materials, sun))
                .unwrap();
            let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
            let i = ((size / 2) * size + size / 2) as usize * 4;
            [half(hdr[i]), half(hdr[i + 1]), half(hdr[i + 2])]
        };
        let off = centre(&[dark]);
        let on = centre(&[lit]);
        println!("screen off {off:?}, on {on:?}");
        assert!((on[1] - off[1] - 1.5).abs() < 0.02);
        assert!((on[0] - off[0]).abs() < 0.01 && (on[2] - off[2]).abs() < 0.01);
    }

    fn screen_materials() -> [Material; 2] {
        let dark = Material {
            base: [0.01; 3],
            roughness: 0.3,
            ..Default::default()
        };
        let lit = Material {
            content_layer: pfx_materials::ContentLayer::from_kind(
                pfx_materials::Content::Screen,
                1.0,
                0,
                &pfx_materials::ContentLook {
                    screen_gain: 1.5,
                    ..Default::default()
                },
            ),
            ..dark
        };
        [dark, lit]
    }

    fn green_screen(frame: &mut Frame) {
        frame.set_content(0, 4, 4, ContentFormat::Srgb8).unwrap();
        frame
            .write_content(0, [0, 0, 4, 4], &[0u8, 255, 0, 255].repeat(16))
            .unwrap();
    }

    fn hdr_green(frame: &Frame) -> Vec<f32> {
        frame
            .gpu
            .readback_rgba16(&frame.targets.hdr)
            .unwrap()
            .chunks_exact(4)
            .map(|pixel| half(pixel[1]))
            .collect()
    }

    #[test]
    fn content_faces_choose_the_side_that_shows() {
        assert!(ContentFace::Both.shows(true) && ContentFace::Both.shows(false));
        assert!(ContentFace::Front.shows(true) && !ContentFace::Front.shows(false));
        assert!(!ContentFace::Back.shows(true) && ContentFace::Back.shows(false));
        assert_eq!(ContentFace::default(), ContentFace::Both);
        assert_eq!(InstanceSurface::default().face, ContentFace::Both);
        assert_eq!(ContentFace::Both.code(), 0);
        assert_eq!(ContentFace::Front.code(), 1);
        assert_eq!(ContentFace::Back.code(), 2);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn two_sided_content_shows_on_the_chosen_face_only() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let size = 64;
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let screen = quad(
            &mut frame,
            [
                [-2.0, 0.0, -2.0],
                [2.0, 0.0, -2.0],
                [-2.0, 0.0, 2.0],
                [2.0, 0.0, 2.0],
            ],
        );
        green_screen(&mut frame);
        let materials = screen_materials();
        let camera = Camera {
            view: look_down(4.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(4.0)),
            position: [0.0, 4.0, 0.0],
        };
        let sun = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let mut flipped = identity();
        flipped[1][1] = -1.0;
        flipped[2][2] = -1.0;
        let centre = (size / 2 * size + size / 2) as usize;
        let mut shown = |material: u32, model: Matrix, surface: Option<InstanceSurface>| {
            let mut instance = Instance::new(screen, model, material, 1);
            instance.two_sided = true;
            frame
                .set_surfaces(&surface.map_or_else(Vec::new, |surface| vec![surface]))
                .unwrap();
            frame
                .render(&calm_scene(camera, &[instance], &materials, sun))
                .unwrap();
            hdr_green(&frame)
        };
        let off = shown(0, identity(), None)[centre];
        let gain = 1.5;
        let faced = |face| {
            Some(InstanceSurface {
                face,
                ..Default::default()
            })
        };
        let default_front = shown(1, identity(), None);
        let default_back = shown(1, flipped, None);
        assert!((default_front[centre] - off - gain).abs() < 0.02);
        assert!((default_back[centre] - off - gain).abs() < 0.02);
        for (face, front_shows, back_shows) in [
            (ContentFace::Both, true, true),
            (ContentFace::Front, true, false),
            (ContentFace::Back, false, true),
        ] {
            let upright = shown(1, identity(), faced(face));
            let turned = shown(1, flipped, faced(face));
            for (image, expected) in [(&upright, front_shows), (&turned, back_shows)] {
                let lifted = image[centre] - off;
                if expected {
                    assert!((lifted - gain).abs() < 0.02, "{face:?}: {lifted}");
                } else {
                    assert!(lifted.abs() < 0.02, "{face:?}: {lifted}");
                }
            }
            if face == ContentFace::Both {
                assert_eq!(upright, default_front);
                assert_eq!(turned, default_back);
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn front_content_keeps_a_rolled_strip_outside_blank() {
        use pfx_geom::strip::{Roller, Strip};
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let size = 128;
        let mut frame = Frame::new(gpu, size, size).unwrap();
        let strip = Strip {
            length: 3.0,
            width: 1.2,
        };
        let mesh = strip.mesh(0.12, 0.002, None, 4);
        let handle = frame
            .upload_mesh(MeshData {
                positions: &mesh.positions,
                normals: &mesh.normals,
                tangents: &mesh.tangents,
                uvs: &mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &mesh.indices,
            })
            .unwrap();
        green_screen(&mut frame);
        let materials = screen_materials();
        let mut instance = Instance::new(handle, identity(), 1, 1);
        instance.deformer = DeformerId(0);
        let camera = Camera {
            view: look_down(6.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(6.0)),
            position: [0.0, 6.0, 0.0],
        };
        let sun = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 0.0,
        };
        let roller = Roller {
            at: 1.2,
            core: 0.12,
            thickness: 0.01,
        };
        let state = RollerFrame {
            at: roller.at,
            outer: roller.outer(&strip),
            thickness: roller.thickness,
        };
        let deformers = [Deformer::Roller {
            current: state,
            previous: state,
        }];
        let instances = [instance];
        let mut lit = |face: ContentFace| {
            frame
                .set_surfaces(&[InstanceSurface {
                    face,
                    ..Default::default()
                }])
                .unwrap();
            let scene = Scene {
                deformers: &deformers,
                ..calm_scene(camera, &instances, &materials, sun)
            };
            frame.render(&scene).unwrap();
            hdr_green(&frame)
                .into_iter()
                .map(|green| green > 0.8)
                .collect::<Vec<bool>>()
        };
        let both = lit(ContentFace::Both);
        let front = lit(ContentFace::Front);
        let back = lit(ContentFace::Back);
        let count = |image: &[bool]| image.iter().filter(|&&on| on).count();
        println!(
            "printed pixels: both {}, front {}, back {}",
            count(&both),
            count(&front),
            count(&back)
        );
        assert!(count(&front) > 0 && count(&back) > 0);
        assert!(count(&front) < count(&both));
        assert!(count(&back) < count(&both));
        for ((both, front), back) in both.iter().zip(&front).zip(&back) {
            assert_eq!(*both, *front || *back);
            assert!(!(*front && *back));
        }
    }

    fn floor_quad(frame: &mut Frame, centre: [f32; 2], half: f32) -> MeshHandle {
        quad(frame, floor_corners(centre, half))
    }

    fn floor_corners(centre: [f32; 2], half: f32) -> [[f32; 3]; 4] {
        [
            [centre[0] - half, 0.0, centre[1] - half],
            [centre[0] + half, 0.0, centre[1] - half],
            [centre[0] - half, 0.0, centre[1] + half],
            [centre[0] + half, 0.0, centre[1] + half],
        ]
    }

    fn quad_data(corners: &[[f32; 3]; 4]) -> MeshData<'_> {
        MeshData {
            positions: corners,
            normals: &[[0.0, 1.0, 0.0]; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 2, 1, 1, 2, 3],
        }
    }

    fn top_camera() -> Camera {
        Camera {
            view: look_down(8.0),
            projection: perspective(1.0),
            previous_view_projection: multiply(perspective(1.0), look_down(8.0)),
            position: [0.0, 8.0, 0.0],
        }
    }

    fn lit_pixels(frame: &mut Frame, instances: &[Instance]) -> Result<usize, String> {
        let materials = [Material {
            base: [0.8; 3],
            roughness: 0.5,
            ..Default::default()
        }];
        let sun = Sun {
            direction: [0.0, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 3.0,
        };
        frame.render(&calm_scene(top_camera(), instances, &materials, sun))?;
        let hdr = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
        Ok(hdr
            .chunks_exact(4)
            .filter(|pixel| half(pixel[1]) > 0.1)
            .count())
    }

    #[test]
    fn mesh_handles_pack_a_slot_and_a_generation() {
        let first = MeshHandle(7);
        assert_eq!((first.slot(), first.generation()), (7, 0));
        let later = MeshHandle::pack(7, 3);
        assert_eq!((later.slot(), later.generation()), (7, 3));
        assert_ne!(first, later);
        assert_eq!(
            MeshHandle::pack(MESH_SLOT_MASK as usize, 1).slot(),
            MESH_SLOT_MASK as usize
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn released_mesh_stops_drawing_and_the_rest_still_render() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let big = floor_quad(&mut frame, [0.0, 0.0], 2.0);
        let small = floor_quad(&mut frame, [3.0, 3.0], 0.5);
        let both = [
            Instance::new(big, identity(), 0, 1),
            Instance::new(small, identity(), 0, 2),
        ];
        let before = lit_pixels(&mut frame, &both).unwrap();
        frame.release_mesh(big).unwrap();
        let after = lit_pixels(&mut frame, &both[1..]).unwrap();
        println!("lit pixels before {before}, after release {after}");
        assert!(before > after * 4);
        assert!(after > 0);
        assert_eq!(frame.mesh_stats().live, 1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn stale_handles_are_refused_and_never_alias_a_newer_mesh() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let old = floor_quad(&mut frame, [0.0, 0.0], 1.0);
        frame.release_mesh(old).unwrap();
        assert!(frame.release_mesh(old).is_err());
        let corners = floor_corners([0.0, 0.0], 1.0);
        assert!(frame.replace_mesh(old, quad_data(&corners)).is_err());
        assert!(frame.shadow_mesh(old).is_none());
        let fresh = floor_quad(&mut frame, [0.0, 0.0], 1.0);
        assert_eq!(fresh.slot(), old.slot());
        assert_ne!(fresh, old);
        assert!(frame.shadow_mesh(fresh).is_some());
        assert!(frame.shadow_mesh(old).is_none());
        assert!(frame.release_mesh(old).is_err());
        assert!(frame.replace_mesh(old, quad_data(&corners)).is_err());
        assert!(
            frame
                .replace_mesh(MeshHandle(900), quad_data(&corners))
                .is_err()
        );
        assert_eq!(frame.mesh_stats().live, 1);
        assert!(frame.release_mesh(fresh).is_ok());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn instance_with_a_released_mesh_is_an_error_from_render() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let kept = floor_quad(&mut frame, [0.0, 0.0], 1.0);
        let dropped = floor_quad(&mut frame, [2.0, 2.0], 1.0);
        let instances = [
            Instance::new(kept, identity(), 0, 1),
            Instance::new(dropped, identity(), 0, 2),
        ];
        assert!(lit_pixels(&mut frame, &instances).is_ok());
        frame.release_mesh(dropped).unwrap();
        let error = lit_pixels(&mut frame, &instances).unwrap_err();
        assert!(error.contains("released"), "{error}");
        let replaced = floor_quad(&mut frame, [-2.0, -2.0], 1.0);
        assert_eq!(replaced.slot(), dropped.slot());
        let error = lit_pixels(&mut frame, &instances).unwrap_err();
        assert!(error.contains("released"), "{error}");
        assert!(lit_pixels(&mut frame, &instances[..1]).is_ok());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn replacing_a_mesh_keeps_the_handle_and_reuses_buffers_that_fit() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let handle = floor_quad(&mut frame, [0.0, 0.0], 2.0);
        let instances = [Instance::new(handle, identity(), 0, 1)];
        let first = lit_pixels(&mut frame, &instances).unwrap();
        let buffer = frame.shadow_mesh(handle).unwrap().0.clone();

        let smaller = floor_corners([0.0, 0.0], 1.0);
        frame.replace_mesh(handle, quad_data(&smaller)).unwrap();
        let same_size = lit_pixels(&mut frame, &instances).unwrap();
        println!("lit pixels {first} then {same_size} with the same buffers");
        assert!(same_size * 3 < first * 2 && same_size > 0);
        assert!(*frame.shadow_mesh(handle).unwrap().0 == buffer);
        assert_eq!(frame.mesh_stats().live, 1);

        let large = sphere(&mut frame);
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut tangents = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        for row in 0..10u32 {
            for column in 0..10u32 {
                let at = [
                    column as f32 * 4.0 / 9.0 - 2.0,
                    0.0,
                    row as f32 * 4.0 / 9.0 - 2.0,
                ];
                positions.push(at);
                normals.push([0.0, 1.0, 0.0]);
                tangents.push([1.0, 0.0, 0.0, 1.0]);
                uvs.push([column as f32 / 9.0, row as f32 / 9.0]);
                if row < 9 && column < 9 {
                    let a = row * 10 + column;
                    indices.extend_from_slice(&[a, a + 10, a + 1, a + 1, a + 10, a + 11]);
                }
            }
        }
        frame.release_mesh(large).unwrap();
        frame
            .replace_mesh(
                handle,
                MeshData {
                    positions: &positions,
                    normals: &normals,
                    tangents: &tangents,
                    uvs: &uvs,
                    uvs1: None,
                    alpha: None,
                    indices: &indices,
                },
            )
            .unwrap();
        let grown = lit_pixels(&mut frame, &instances).unwrap();
        println!(
            "lit pixels {grown} after growing to {} vertices",
            positions.len()
        );
        assert!(grown.abs_diff(first) * 20 < first);
        assert!(*frame.shadow_mesh(handle).unwrap().0 != buffer);
        assert_eq!(frame.shadow_mesh(handle).unwrap().2, indices.len() as u32);
        assert_eq!(frame.mesh_stats().live, 1);

        let tiny = floor_corners([0.0, 0.0], 0.5);
        frame.replace_mesh(handle, quad_data(&tiny)).unwrap();
        let shrunk = lit_pixels(&mut frame, &instances).unwrap();
        assert!(shrunk < same_size && shrunk > 0);
        assert_eq!(frame.shadow_mesh(handle).unwrap().2, 6);
        let broken = [[0.0; 3]; 3];
        let error = frame.replace_mesh(
            handle,
            MeshData {
                positions: &broken,
                normals: &broken,
                tangents: &[[0.0; 4]; 3],
                uvs: &[[0.0; 2]; 3],
                uvs1: None,
                alpha: None,
                indices: &[0, 1, 7],
            },
        );
        assert!(error.is_err());
        assert_eq!(lit_pixels(&mut frame, &instances).unwrap(), shrunk);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn upload_and_release_cycles_keep_the_buffers_bounded() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let keep = floor_quad(&mut frame, [0.0, 0.0], 1.0);
        let corners = floor_corners([0.0, 0.0], 1.0);
        let mut seen = std::collections::HashSet::new();
        for cycle in 0..1000 {
            let handle = frame.upload_mesh(quad_data(&corners)).unwrap();
            assert!(seen.insert(handle), "cycle {cycle} reused a handle");
            let stats = frame.mesh_stats();
            assert_eq!((stats.live, stats.slots, stats.buffers), (2, 2, 6));
            frame.release_mesh(handle).unwrap();
        }
        let stats = frame.mesh_stats();
        println!("after 1000 cycles: {stats:?}");
        assert_eq!((stats.live, stats.slots, stats.buffers), (1, 2, 3));
        println!(
            "allocator: {:?}",
            frame
                .gpu
                .device
                .generate_allocator_report()
                .map(|report| (report.total_allocated_bytes, report.total_reserved_bytes))
        );
        for _ in 0..5000 {
            let handle = frame.upload_mesh(quad_data(&corners)).unwrap();
            frame.release_mesh(handle).unwrap();
        }
        let stats = frame.mesh_stats();
        println!("after 6000 cycles: {stats:?}");
        assert_eq!(stats.live, 1);
        assert!(stats.slots <= 4);
        assert!(frame.shadow_mesh(keep).is_some());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn replace_cycles_count_buffers_by_their_own_life() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let start = frame.mesh_stats().buffers;
        let keep = floor_quad(&mut frame, [0.0, 0.0], 1.0);
        assert_eq!(frame.mesh_stats().buffers, start + 3);
        let grid = |cells: u32| {
            let side = cells + 1;
            let mut positions = Vec::new();
            let mut indices = Vec::new();
            for row in 0..side {
                for column in 0..side {
                    positions.push([column as f32, 0.0, row as f32]);
                    if row < cells && column < cells {
                        let a = row * side + column;
                        indices.extend_from_slice(&[
                            a,
                            a + side,
                            a + 1,
                            a + 1,
                            a + side,
                            a + side + 1,
                        ]);
                    }
                }
            }
            let count = positions.len();
            (
                positions,
                vec![[0.0, 1.0, 0.0]; count],
                vec![[1.0, 0.0, 0.0, 1.0]; count],
                vec![[0.0, 0.0]; count],
                indices,
            )
        };
        let handle = floor_quad(&mut frame, [2.0, 2.0], 1.0);
        let mut most = 0;
        for cycle in 0..1000u32 {
            let cells = 1 + (cycle * 7) % 23;
            let (positions, normals, tangents, uvs, indices) = grid(cells);
            frame
                .replace_mesh(
                    handle,
                    MeshData {
                        positions: &positions,
                        normals: &normals,
                        tangents: &tangents,
                        uvs: &uvs,
                        uvs1: None,
                        alpha: None,
                        indices: &indices,
                    },
                )
                .unwrap();
            let stats = frame.mesh_stats();
            most = most.max(stats.buffers);
            assert_eq!(stats.live, 2);
            assert_eq!(stats.buffers, start + 6, "cycle {cycle} with {cells} cells");
        }
        frame.release_mesh(handle).unwrap();
        let stats = frame.mesh_stats();
        println!(
            "buffers: start {start}, most {most}, after release {}",
            stats.buffers
        );
        assert_eq!(stats.buffers, start + 3);
        frame.release_mesh(keep).unwrap();
        assert_eq!(frame.mesh_stats().buffers, start);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn releasing_a_swaying_mesh_frees_its_sway_range_for_the_next() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, 64, 64).unwrap();
        let corners = floor_corners([0.0, 0.0], 1.0);
        let sway = [SwayVertex {
            pivot: [0.0; 3],
            level: 1.0,
            stiffness: 0.5,
        }; 4];
        let first = frame
            .upload_swaying_mesh(quad_data(&corners), &sway)
            .unwrap();
        let second = frame
            .upload_swaying_mesh(quad_data(&corners), &sway)
            .unwrap();
        assert_eq!(frame.sway_data.len(), 8);
        assert!(frame.swaying(first));
        assert!(frame.replace_mesh(first, quad_data(&corners)).is_err());
        frame.release_mesh(first).unwrap();
        assert!(!frame.swaying(first));
        let third = frame
            .upload_swaying_mesh(quad_data(&corners), &sway)
            .unwrap();
        assert_eq!(frame.sway_data.len(), 8);
        assert!(frame.swaying(third) && frame.swaying(second));
        frame.release_mesh(second).unwrap();
        frame.release_mesh(third).unwrap();
        assert_eq!(frame.sway_free.0, vec![0..8]);
    }
}
