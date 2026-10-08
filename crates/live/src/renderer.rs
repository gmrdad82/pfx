use crate::canopy::{CanopyMode, Gobo, petal_emitters};
use crate::crisp;
use crate::demand::{self, DemandStats, Limits, Need, Next, Print, Region, Shown};
use crate::effects::{
    self, CreatureInstance, CreatureMesh, EffectCommands, EffectFrame, EffectTargets,
    ParticleInstance, RoomHaze, VolumeKind, VolumeTimestamps, pack_emitter,
};
use crate::flat::look::{FlatLooks, Look, LookFrame};
use crate::flat::{FlatPass, FlatScene};
use crate::frame::{self, Frame, Instance, Matrix, MeshData, MeshHandle, OpaqueKey, Scene};
use crate::glass;
use crate::impostor::{ImpostorInstance, ImpostorParams, ImpostorPass};
use crate::lights::{LightQuality, LocalLight};
use crate::lod::{LodSet, LodStats};
use crate::maps::{MapImages, TextureFilter};
use crate::pipeline_cache::CacheLoad;
use crate::probes::{Ambient, ContactDesc, ProbeLighting};
use crate::reflect;
use crate::shadow::{Quality, Shadows, View};
use crate::sky::SkySource;
use crate::stats::{FrameKind, FrameStats};
use crate::taa::{self, GpuInputs};
use crate::text::{GpuAtlas, PackedGlyph, QuadDraw, RichQuad, TextDraw, TextPass, TextSpace};
use crate::warm::Warming;
use pfx_geom::mesh::Mesh;
use pfx_geom::tree::{PetalSource, Tree, tree_sway};
use pfx_gpu::screens::{Budget, DynamicResolution};
use pfx_gpu::{FrameTimings, Gpu, GpuProfiler, GpuRefusal, PassTiming, wgpu};
use pfx_physics::{Emitter, ParticleKind, Wind};
use pfx_post::{Bloom, Chain, Pass as PostPass, Style, Tape, Tone, dof, gpu::GpuChain};
use pfx_text::Paragraph;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use wgpu::util::DeviceExt;

pub const SCENE_COLOUR_MIPS: u32 = 7;
pub const SHADOW_RANGE: f32 = 40.0;
pub const TIMING_LAG: u64 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Submitted {
    pub frame: u64,
    pub arrived: Vec<FrameTimings>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimingDrops {
    pub ring: u64,
    pub late: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RendererError {
    Refused(GpuRefusal),
    Setup(String),
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Setup(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RendererError {}

impl From<String> for RendererError {
    fn from(message: String) -> Self {
        Self::Setup(message)
    }
}

impl From<GpuRefusal> for RendererError {
    fn from(refusal: GpuRefusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<RendererError> for String {
    fn from(error: RendererError) -> Self {
        error.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finish {
    Standard,
    Style(Style),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exposure {
    Fixed(f32),
    Auto { bias: f32 },
}

impl Exposure {
    fn value(self, metered: f32) -> f32 {
        match self {
            Self::Fixed(value) => value,
            Self::Auto { bias } => metered * 2.0_f32.powf(bias),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picked {
    pub x: u32,
    pub y: u32,
    pub id: u32,
    pub depth: f32,
}

struct PickReadback {
    buffer: wgpu::Buffer,
    receiver: Receiver<Result<(), wgpu::BufferAsyncError>>,
    x: u32,
    y: u32,
}

pub struct TextItem<'a> {
    pub atlas: &'a GpuAtlas,
    pub paragraph: &'a Paragraph,
    pub color: [f32; 4],
    pub space: TextSpace,
    pub id: u32,
}
pub struct TextQuadItem<'a> {
    pub atlas: &'a GpuAtlas,
    pub quads: &'a [RichQuad],
    pub space: TextSpace,
    pub icons: bool,
    pub id: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextList {
    Surface,
    SurfaceQuads,
    Overlay,
    OverlayQuads,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBounds {
    pub list: TextList,
    pub index: usize,
    pub id: u32,
    pub rect: [f32; 4],
}
#[derive(Default)]
pub struct Text<'a> {
    pub surface: Vec<TextItem<'a>>,
    pub overlay: Vec<TextItem<'a>>,
    pub surface_quads: Vec<TextQuadItem<'a>>,
    pub overlay_quads: Vec<TextQuadItem<'a>>,
}
pub struct Volume {
    pub kind: VolumeKind,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub color: [f32; 3],
    pub density: f32,
    pub anisotropy: f32,
    pub time_scale: f32,
}
pub struct Liquid<'a> {
    pub fluid: &'a wgpu::TextureView,
    pub ripple: &'a wgpu::TextureView,
}
#[derive(Default)]
pub struct Effects<'a> {
    pub volume: Option<Volume>,
    pub particles: &'a [ParticleInstance],
    pub creatures: &'a [CreatureInstance],
    pub creature_mesh: Option<&'a CreatureMesh>,
    pub glass: &'a [glass::Surface],
    pub liquid: Option<Liquid<'a>>,
    pub props: &'a [ImpostorInstance],
}

pub const HISTORY_GAP: f32 = 0.25;

#[derive(Clone, Debug)]
pub struct History {
    previous_time: Option<f32>,
    last_view_projection: Option<Matrix>,
    reset: bool,
    max_gap: f32,
}

impl Default for History {
    fn default() -> Self {
        Self {
            previous_time: None,
            last_view_projection: None,
            reset: false,
            max_gap: HISTORY_GAP,
        }
    }
}

impl History {
    pub fn invalidate(&mut self) {
        self.reset = true;
    }

    pub fn set_max_gap(&mut self, seconds: f32) {
        self.max_gap = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            HISTORY_GAP
        };
    }

    pub fn max_gap(&self) -> f32 {
        self.max_gap
    }

    pub fn advance(
        &mut self,
        time: f32,
        previous_view_projection: Matrix,
        view_projection: Matrix,
    ) -> bool {
        let continuous = self
            .previous_time
            .is_some_and(|previous| time >= previous && time - previous <= self.max_gap);
        let chained = self
            .last_view_projection
            .is_some_and(|last| close(last, previous_view_projection));
        let valid = !self.reset && continuous && chained;
        self.reset = false;
        self.previous_time = Some(time);
        self.last_view_projection = Some(view_projection);
        valid
    }

    pub fn previous_time(&self) -> Option<f32> {
        self.previous_time
    }

    pub fn bridge(&mut self, time: f32) {
        if self.previous_time.is_some_and(|previous| time >= previous) {
            self.previous_time = Some(time);
        }
    }

    pub fn last_view_projection(&self) -> Option<Matrix> {
        self.last_view_projection
    }
}

fn close(a: Matrix, b: Matrix) -> bool {
    let scale = a
        .iter()
        .flatten()
        .chain(b.iter().flatten())
        .fold(1.0_f32, |high, value| high.max(value.abs()));
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .all(|(x, y)| (x - y).abs() <= scale * 1e-4)
}

struct Petals {
    origin: [f32; 3],
    sources: Vec<PetalSource>,
    emitters: Vec<Emitter>,
    seed: u32,
    color: [f32; 3],
    last_time: Option<f32>,
}

impl Petals {
    fn new(tree: &Tree, origin: [f32; 3], rate: f32, color: [f32; 3]) -> Self {
        let emitters = petal_emitters(tree, origin, 0.0, 0.0, [1.0, 0.0, 0.0], rate);
        let count = emitters.len();
        let sources = (0..count)
            .map(|i| {
                let index = ((i as f32 + 0.5) * tree.petal_sources.len() as f32 / count as f32)
                    .floor() as usize;
                tree.petal_sources[index.min(tree.petal_sources.len() - 1)]
            })
            .collect();
        Self {
            origin,
            sources,
            emitters,
            seed: (tree.seed % 997) as u32 + 1,
            color,
            last_time: None,
        }
    }

    fn step(&mut self, time: f32, wind: &crate::deform::WindFrame) -> Vec<ParticleInstance> {
        let dt = self
            .last_time
            .map_or(0.0, |last| (time - last).clamp(0.0, 0.25));
        self.last_time = Some(time);
        let blow = Wind::new(self.seed, wind.direction, wind.strength);
        let mut out = Vec::new();
        for (emitter, source) in self.emitters.iter_mut().zip(&self.sources) {
            let at = tree_sway(
                source.position,
                source.sway.pivot,
                source.sway.level,
                source.sway.stiffness,
                wind.time,
                wind.strength,
                wind.direction,
            );
            emitter.set_origin([
                at[0] + self.origin[0],
                at[1] + self.origin[1],
                at[2] + self.origin[2],
            ]);
            emitter.step(dt, time, &blow);
            out.extend(pack_emitter(emitter, ParticleKind::Petal, time, self.color));
        }
        out
    }
}

struct Motion {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
}

struct Mips {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    levels: Vec<wgpu::TextureView>,
    extents: Vec<wgpu::Buffer>,
    binds: Vec<wgpu::BindGroup>,
    written: Option<[u32; 2]>,
    pipeline: wgpu::RenderPipeline,
}

impl Mips {
    fn bordered(&self, rect: [u32; 2], level: u32) -> [u32; 2] {
        let size = self.texture.size();
        [
            (rect[0] + 1).min(crate::viewport::mip(size.width, level)),
            (rect[1] + 1).min(crate::viewport::mip(size.height, level)),
        ]
    }

    fn copy_scene(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::Texture,
        viewport: [u32; 2],
    ) {
        let [width, height] = viewport;
        let copy =
            |encoder: &mut wgpu::CommandEncoder, from: [u32; 2], to: [u32; 2], extent: [u32; 2]| {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: scene,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: from[0],
                            y: from[1],
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: to[0],
                            y: to[1],
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: extent[0],
                        height: extent[1],
                        depth_or_array_layers: 1,
                    },
                );
            };
        copy(encoder, [0, 0], [0, 0], [width, height]);
        let size = self.texture.size();
        let right = width < size.width;
        let below = height < size.height;
        if right {
            copy(encoder, [width - 1, 0], [width, 0], [1, height]);
        }
        if below {
            copy(encoder, [0, height - 1], [0, height], [width, 1]);
        }
        if right && below {
            copy(encoder, [width - 1, height - 1], [width, height], [1, 1]);
        }
    }
}

struct ExposureMeter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    value: wgpu::Buffer,
    readback: wgpu::Buffer,
    extent: wgpu::Buffer,
    written: Option<[u32; 2]>,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    stale: bool,
    exposure: f32,
    last_time: Option<f32>,
}

impl ExposureMeter {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live exposure layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live exposure pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live exposure shader"),
            source: wgpu::ShaderSource::Wgsl(EXPOSURE_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("live exposure pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let value = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live log luminance"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live luminance readback"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let extent = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live exposure extent"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            value,
            readback,
            extent,
            written: None,
            pending: None,
            stale: false,
            exposure: 1.0,
            last_time: None,
        }
    }

    fn advance(&mut self, time: f32, valid: bool) -> f32 {
        let seconds = self
            .last_time
            .map_or(1.0 / 60.0, |last| (time - last).max(0.0));
        self.last_time = Some(time);
        if !valid {
            self.exposure = 1.0;
            self.stale = self.pending.is_some();
        }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    let bytes = self.readback.slice(..).get_mapped_range();
                    let log_luminance = f32::from_le_bytes(bytes[..4].try_into().unwrap());
                    drop(bytes);
                    self.readback.unmap();
                    self.pending = None;
                    if valid && !self.stale && log_luminance.is_finite() {
                        self.exposure = pfx_core::sky::adapt_exposure(
                            self.exposure,
                            &[log_luminance.exp()],
                            seconds,
                        );
                    }
                    self.stale = false;
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.stale = false;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        self.exposure
    }

    fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        viewport: [u32; 2],
    ) {
        if self.pending.is_some() {
            return;
        }
        if self.written != Some(viewport) {
            queue.write_buffer(
                &self.extent,
                0,
                bytemuck::cast_slice(&[viewport[0], viewport[1], 0, 0]),
            );
            self.written = Some(viewport);
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live exposure bindings"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.value.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.extent.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("live exposure reduction"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        drop(pass);
        encoder.copy_buffer_to_buffer(&self.value, 0, &self.readback, 0, 4);
    }

    fn submitted(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.pending = Some(receiver);
    }
}

const EXPOSURE_WGSL: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;
@group(0) @binding(2) var<uniform> extent: vec4u;
var<workgroup> values: array<f32, 256>;
@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) index: u32) {
    let size = min(textureDimensions(source), extent.xy);
    var total = 0.0;
    for (var sample = 0u; sample < 4u; sample += 1u) {
        let point = index * 4u + sample;
        let x = min((point % 64u) * size.x / 64u + size.x / 128u, size.x - 1u);
        let y = min((point / 64u) * size.y / 16u + size.y / 32u, size.y - 1u);
        let rgb = textureLoad(source, vec2i(i32(x), i32(y)), 0).rgb;
        total += log(max(dot(rgb, vec3f(0.2126, 0.7152, 0.0722)), 0.001));
    }
    values[index] = total;
    workgroupBarrier();
    var stride = 128u;
    loop {
        if (index < stride) { values[index] += values[index + stride]; }
        workgroupBarrier();
        if (stride == 1u) { break; }
        stride /= 2u;
    }
    if (index == 0u) { output[0] = values[0] / 1024.0; }
}
"#;

pub struct Renderer {
    frame: Frame,
    shadows: Shadows,
    taa: taa::Pass,
    crisp: crisp::Pass,
    sharpen: f32,
    text_after_taa: bool,
    text: TextPass,
    effects: effects::Effects,
    reflections: reflect::Pass,
    glass: glass::Pass,
    impostors: Option<ImpostorPass>,
    petals: Option<Petals>,
    lod: LodSet,
    post: GpuChain,
    dof: dof::GpuDof,
    lens: Option<dof::Lens>,
    haze: Option<RoomHaze>,
    exposure: ExposureMeter,
    exposure_mode: Exposure,
    custom_finish: bool,
    tape: Option<Tape>,
    pick_request: Option<(u32, u32)>,
    pick_readback: Option<PickReadback>,
    style: Finish,
    output_format: wgpu::TextureFormat,
    display: Option<(wgpu::Texture, wgpu::TextureView)>,
    blit: Option<(wgpu::BindGroupLayout, wgpu::RenderPipeline)>,
    depths: [(wgpu::Texture, wgpu::TextureView); 2],
    depth_index: usize,
    reactive: wgpu::Texture,
    reactive_view: wgpu::TextureView,
    depth_layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::ComputePipeline,
    motion: Motion,
    mips: Mips,
    black: wgpu::TextureView,
    profiler: GpuProfiler,
    frames_submitted: u64,
    frames_done: Arc<AtomicU64>,
    submissions: VecDeque<(u64, wgpu::SubmissionIndex)>,
    arrivals: Vec<FrameTimings>,
    latest_timings: Option<FrameTimings>,
    late_timings: u64,
    stats: FrameStats,
    resolution: Option<DynamicResolution>,
    resolution_content: Option<[u32; 2]>,
    resolution_frame: bool,
    sky_hours: Vec<(u64, f32)>,
    history: History,
    history_valid: bool,
    hour: f32,
    probe_emission: f32,
    pass_log: Vec<&'static str>,
    text_bounds: Vec<TextBounds>,
    flat: Option<FlatPass>,
    flat_looks: Option<FlatLooks>,
    plates: Option<crate::plates::Plates>,
    plate_casters: Vec<Instance>,
    plate_mask: bool,
    demand: Option<demand::Cache>,
    revision: AtomicU64,
    custom_local: Option<bool>,
}

pub struct GeometryReadback {
    pub colour: Vec<[f32; 4]>,
    pub depth: Vec<f32>,
    pub normals: Vec<[f32; 3]>,
}

fn chain(style: Finish, encode: bool) -> Chain {
    let mut chain = match style {
        Finish::Standard => Chain {
            passes: vec![
                PostPass::Bloom(Bloom::neutral(0.0)),
                PostPass::Tone(Tone::aces()),
            ],
            frame: 0,
            seed: 0,
        },
        Finish::Style(style) => style.chain(),
    };
    chain.passes.insert(0, PostPass::Exposure(1.0));
    if encode {
        chain.passes.push(PostPass::Encode);
    }
    chain
}

fn plain_passes(passes: Vec<PassTiming>) -> Vec<PassTiming> {
    let mut merged: Vec<PassTiming> = Vec::new();
    for pass in passes {
        let label = if pass.label.starts_with("post ") {
            "post"
        } else if pass.label.starts_with("volume ") {
            "volume"
        } else {
            &pass.label
        };
        if let Some(existing) = merged.iter_mut().find(|item| item.label == label) {
            existing.milliseconds += pass.milliseconds;
        } else {
            merged.push(PassTiming {
                label: label.to_owned(),
                milliseconds: pass.milliseconds,
            });
        }
    }
    merged.sort_by(|a, b| {
        b.milliseconds
            .total_cmp(&a.milliseconds)
            .then_with(|| a.label.cmp(&b.label))
    });
    merged.truncate(16);
    merged
}

const FULLSCREEN: &str = "@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f { let xy = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(xy[index], 0.0, 1.0); }";

const MOTION_WGSL: &str = r#"
struct MotionUniform {
    inverse: mat4x4f,
    current: mat4x4f,
    previous: mat4x4f,
    size: vec4f,
}
@group(0) @binding(0) var scene_depth: texture_depth_2d;
@group(0) @binding(1) var scene_ids: texture_2d<u32>;
@group(0) @binding(2) var<uniform> motion: MotionUniform;
@fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec2f {
    let pixel = vec2i(position.xy);
    if (textureLoad(scene_ids, pixel, 0).r != 0u) {
        discard;
    }
    let z = textureLoad(scene_depth, pixel, 0);
    let uv = position.xy * motion.size.zw;
    let world = motion.inverse * vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, z, 1.0);
    let point = vec4f(world.xyz / world.w, 1.0);
    let now = motion.current * point;
    let before = motion.previous * point;
    let moved = (before.xy / before.w - now.xy / now.w) * vec2f(0.5, -0.5);
    return select(moved, vec2f(0.0), abs(moved) < vec2f(1e-8));
}
"#;

const DOWNSAMPLE_WGSL: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> extent: vec4u;
@fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(source)) * 0.5;
    let pixel = min(position.xy, vec2f(extent.xy) - vec2f(0.5));
    return textureSampleLevel(source, source_sampler, pixel / size, 0.0);
}
"#;

pub fn motion_source() -> String {
    format!("{FULLSCREEN}\n{MOTION_WGSL}")
}

pub fn downsample_source() -> String {
    format!("{FULLSCREEN}\n{DOWNSAMPLE_WGSL}")
}

fn blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("live output blit"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("live output blit"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let source = format!(
        "{FULLSCREEN}\n@group(0) @binding(0) var source: texture_2d<f32>; @fragment fn fragment(@builtin(position) position: vec4f) -> @location(0) vec4f {{ return textureLoad(source, vec2i(position.xy), 0); }}"
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("live output blit"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = fullscreen(
        device,
        &pipeline_layout,
        &shader,
        format,
        "live output blit",
    );
    (layout, pipeline)
}

fn fullscreen(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fragment"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        multiview: None,
        cache: None,
    })
}

fn image(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
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
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn linear_depths(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> [(wgpu::Texture, wgpu::TextureView); 2] {
    [0, 1].map(|_| {
        image(
            device,
            width,
            height,
            wgpu::TextureFormat::R32Float,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            "live linear depth",
        )
    })
}

fn depth_pipeline(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("live linear depth"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::R32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("live linear depth"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("live linear depth"), source: wgpu::ShaderSource::Wgsl("@group(0) @binding(0) var source: texture_depth_2d; @group(0) @binding(1) var destination: texture_storage_2d<r32float, write>; @group(0) @binding(2) var<uniform> limits: vec4f; @compute @workgroup_size(8, 8) fn main(@builtin(global_invocation_id) id: vec3u) { let size = textureDimensions(source); if (id.x >= size.x || id.y >= size.y) { return; } let z = textureLoad(source, id.xy, 0); let distance = limits.y / (z + limits.x); textureStore(destination, id.xy, vec4f(select(limits.z, distance, z < 1.0 && distance > 0.0), 0.0, 0.0, 0.0)); }".into()) });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("live linear depth"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (layout, pipeline)
}

fn motion_pass(device: &wgpu::Device) -> Motion {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("live background motion"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("live background motion"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("live background motion"),
        source: wgpu::ShaderSource::Wgsl(motion_source().into()),
    });
    let pipeline = fullscreen(
        device,
        &pipeline_layout,
        &shader,
        wgpu::TextureFormat::Rg16Float,
        "live background motion",
    );
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("live background motion"),
        size: 208,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Motion {
        layout,
        pipeline,
        uniform,
    }
}

fn mips(device: &wgpu::Device, width: u32, height: u32) -> Mips {
    let levels_count = SCENE_COLOUR_MIPS.min(32 - width.max(height).leading_zeros());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("live scene colour"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let levels: Vec<wgpu::TextureView> = (0..levels_count)
        .map(|level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("live scene colour mips"),
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("live scene colour mips"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("live scene colour mips"),
        source: wgpu::ShaderSource::Wgsl(downsample_source().into()),
    });
    let pipeline = fullscreen(
        device,
        &pipeline_layout,
        &shader,
        wgpu::TextureFormat::Rgba16Float,
        "live scene colour mips",
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("live scene colour mips"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let extents: Vec<wgpu::Buffer> = (0..levels_count)
        .map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("live scene colour mip extent"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .collect();
    let binds = (1..levels.len())
        .map(|level| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("live scene colour mips"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&levels[level - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: extents[level].as_entire_binding(),
                    },
                ],
            })
        })
        .collect();
    Mips {
        texture,
        view,
        levels,
        extents,
        binds,
        written: None,
        pipeline,
    }
}

pub fn invert(m: Matrix) -> Option<Matrix> {
    let a = |c: usize, r: usize| f64::from(m[c][r]);
    let cofactor = |c: usize, r: usize| {
        let rows: Vec<usize> = (0..4).filter(|&i| i != c).collect();
        let columns: Vec<usize> = (0..4).filter(|&i| i != r).collect();
        let minor = |i: usize, j: usize| a(columns[i], rows[j]);
        let value = minor(0, 0) * (minor(1, 1) * minor(2, 2) - minor(1, 2) * minor(2, 1))
            - minor(0, 1) * (minor(1, 0) * minor(2, 2) - minor(1, 2) * minor(2, 0))
            + minor(0, 2) * (minor(1, 0) * minor(2, 1) - minor(1, 1) * minor(2, 0));
        if (c + r).is_multiple_of(2) {
            value
        } else {
            -value
        }
    };
    let inverse: [[f64; 4]; 4] = std::array::from_fn(|c| std::array::from_fn(|r| cofactor(c, r)));
    let determinant: f64 = (0..4).map(|k| a(k, 0) * inverse[0][k]).sum();
    if !determinant.is_finite() || determinant.abs() < 1e-20 {
        return None;
    }
    Some(inverse.map(|column| column.map(|value| (value / determinant) as f32)))
}

fn camera_view(scene: &Scene<'_>, width: u32, height: u32) -> View {
    let view = scene.camera.view;
    let projection = scene.camera.projection;
    let a = projection[2][2];
    let b = projection[3][2];
    View {
        eye: scene.camera.position,
        forward: [-view[0][2], -view[1][2], -view[2][2]],
        up: [view[0][1], view[1][1], view[2][1]],
        fov_y: 2.0 * (1.0 / projection[1][1]).atan(),
        aspect: width as f32 / height as f32,
        near: (b / a).abs().max(0.01),
        far: (b / (a + 1.0)).abs().clamp(1.0, SHADOW_RANGE),
    }
}

fn solid_view(device: &wgpu::Device, queue: &wgpu::Queue, label: &str) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0; 8],
        )
        .create_view(&Default::default())
}

impl Renderer {
    pub fn new(gpu: Gpu, width: u32, height: u32) -> Result<Self, RendererError> {
        Self::new_with_output_format(gpu, width, height, wgpu::TextureFormat::Rgba16Float)
    }

    pub fn new_with_output_format(
        gpu: Gpu,
        width: u32,
        height: u32,
        output_format: wgpu::TextureFormat,
    ) -> Result<Self, RendererError> {
        gpu.check_floor()?;
        if width == 0 || height == 0 {
            return Err(String::from("viewport is empty").into());
        }
        if !matches!(
            output_format,
            wgpu::TextureFormat::Rgba16Float
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(String::from("unsupported output format").into());
        }
        let device = &gpu.device;
        let shadows = Shadows::new(device, Quality::default());
        let mut taa = taa::Pass::new(device);
        taa.resize(device, width, height);
        let text = TextPass::new(device);
        let effects = effects::Effects::new(device, width, height, 4)?;
        let style = Finish::Standard;
        let post = GpuChain::new(
            chain(style, output_format == wgpu::TextureFormat::Rgba16Float),
            device,
            width,
            height,
        );
        let display = (output_format != wgpu::TextureFormat::Rgba16Float).then(|| {
            image(
                device,
                width,
                height,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                "live display intermediate",
            )
        });
        let blit = display
            .as_ref()
            .map(|_| blit_pipeline(device, output_format));
        let (reactive, reactive_view) = image(
            device,
            width,
            height,
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "live reactive mask",
        );
        let (depth_layout, depth_pipeline) = depth_pipeline(device);
        let profiler = GpuProfiler::new(device, &gpu.queue);
        let black = solid_view(device, &gpu.queue, "live empty input");
        let mut renderer = Self {
            shadows,
            taa,
            crisp: crisp::Pass::new(device, width, height),
            sharpen: 0.0,
            text_after_taa: true,
            text,
            effects,
            reflections: reflect::Pass::new(device, width, height),
            glass: glass::Pass::new(device, width, height),
            impostors: None,
            petals: None,
            lod: LodSet::new(2_000_000),
            post,
            dof: dof::GpuDof::new(device, width, height),
            lens: None,
            haze: None,
            exposure: ExposureMeter::new(device),
            exposure_mode: Exposure::Auto { bias: 0.0 },
            custom_finish: false,
            tape: None,
            pick_request: None,
            pick_readback: None,
            style,
            output_format,
            display,
            blit,
            depths: linear_depths(device, width, height),
            depth_index: 0,
            reactive,
            reactive_view,
            depth_layout,
            depth_pipeline,
            motion: motion_pass(device),
            mips: mips(device, width, height),
            black,
            profiler,
            frames_submitted: 0,
            frames_done: Arc::new(AtomicU64::new(0)),
            submissions: VecDeque::new(),
            arrivals: Vec::new(),
            latest_timings: None,
            late_timings: 0,
            stats: FrameStats::from_env(),
            resolution: None,
            resolution_content: None,
            resolution_frame: false,
            sky_hours: Vec::new(),
            history: History::default(),
            history_valid: false,
            hour: 12.0,
            probe_emission: 0.0,
            pass_log: Vec::new(),
            text_bounds: Vec::new(),
            flat: None,
            flat_looks: None,
            plates: None,
            plate_casters: Vec::new(),
            plate_mask: false,
            demand: None,
            revision: AtomicU64::new(0),
            custom_local: None,
            frame: Frame::new(gpu, width, height)?,
        };
        renderer.frame.set_reflections_planned(true);
        Ok(renderer)
    }

    pub fn gpu(&self) -> &Gpu {
        &self.frame.gpu
    }

    pub fn size(&self) -> (u32, u32) {
        let [width, height] = self.frame.viewport();
        (width, height)
    }

    pub fn capacity(&self) -> (u32, u32) {
        (self.frame.targets.width, self.frame.targets.height)
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        let (capacity_width, capacity_height) = self.capacity();
        crate::viewport::check([width, height], [capacity_width, capacity_height])?;
        if self.size() == (width, height) {
            return Ok(());
        }
        if self.demand.is_some() && (width, height) != self.capacity() {
            return Err("frames on demand render at the full size; turn them off to render into a smaller viewport".into());
        }
        self.touch();
        self.set_viewport_parts(width, height)
    }

    fn set_viewport_parts(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.frame.set_viewport(width, height)?;
        self.taa.set_viewport(width, height)?;
        self.crisp.set_viewport(width, height)?;
        self.effects.set_viewport(width, height)?;
        self.reflections.set_viewport(width, height)?;
        self.glass.set_viewport(width, height)?;
        self.post.set_viewport(width, height)?;
        self.dof.set_viewport(width, height)?;
        self.text_bounds.clear();
        if self
            .pick_request
            .is_some_and(|(x, y)| x >= width || y >= height)
        {
            self.pick_request = None;
        }
        Ok(())
    }

    pub fn history_restarts(&self) -> u64 {
        self.taa.restarts()
    }

    pub fn target_textures(&self) -> Vec<wgpu::Texture> {
        let targets = &self.frame.targets;
        let mut textures = vec![
            targets.hdr.texture.clone(),
            targets.depth.clone(),
            targets.velocity.clone(),
            targets.ids.clone(),
            targets.normal_roughness.clone(),
            self.reactive.clone(),
            self.mips.texture.clone(),
            self.depths[0].0.clone(),
            self.depths[1].0.clone(),
        ];
        textures.extend(self.taa.textures());
        textures.extend(self.crisp.target().cloned());
        textures.extend(self.effects.textures());
        textures.extend(self.reflections.textures());
        textures.extend(self.glass.textures());
        textures.extend(self.display.as_ref().map(|(texture, _)| texture.clone()));
        textures.extend(self.flat.iter().flat_map(FlatPass::target_textures));
        textures
    }

    fn post_chain(&self, finish: Chain) -> Result<GpuChain, String> {
        let (capacity_width, capacity_height) = self.capacity();
        let (width, height) = self.size();
        let mut post = GpuChain::new(
            finish,
            &self.frame.gpu.device,
            capacity_width,
            capacity_height,
        );
        post.set_viewport(width, height)?;
        Ok(post)
    }

    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    pub fn last_pass_order(&self) -> &[&'static str] {
        &self.pass_log
    }

    pub fn set_history_gap(&mut self, seconds: f32) {
        self.history.set_max_gap(seconds);
    }

    pub fn history_valid(&self) -> bool {
        self.history_valid
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn warm(&mut self, scenes: &[Scene<'_>]) -> Result<Warming, String> {
        let (width, height) = self.size();
        let mut keys: Vec<Vec<OpaqueKey>> = Vec::with_capacity(scenes.len());
        for scene in scenes {
            let fit = self
                .shadows
                .fit(&camera_view(scene, width, height), scene.sun.direction);
            keys.push(self.frame.opaque_keys(scene, Some((&self.shadows, &fit)))?);
        }
        let first = keys.first().cloned().unwrap_or_default();
        let later: Vec<OpaqueKey> = keys.into_iter().skip(1).flatten().collect();
        Ok(self.frame.warm(&first, &later))
    }

    pub fn set_warm_workers(&mut self, workers: usize) {
        self.frame.set_warm_workers(workers);
    }

    pub fn use_pipeline_cache(&mut self, path: &std::path::Path) -> CacheLoad {
        self.frame.use_pipeline_cache(path)
    }

    pub fn save_pipeline_cache(&self) -> Result<Option<usize>, String> {
        self.frame.save_pipeline_cache()
    }

    pub fn warm_opaque(&mut self, features: frame::OpaqueFeatures, lit: bool) {
        self.frame.warm_opaque(features, lit);
    }

    pub fn set_finish(&mut self, mut finish: Chain) {
        self.touch();
        finish.passes.insert(0, PostPass::Exposure(1.0));
        if self.display.is_none() {
            finish.passes.push(PostPass::Encode);
        }
        self.custom_local = Some(demand::local_post(&finish));
        if let Ok(post) = self.post_chain(finish) {
            self.post = post;
        }
        self.custom_finish = true;
    }

    pub fn set_tape(&mut self, tape: Option<Tape>) {
        if tape != self.tape {
            self.touch();
        }
        self.tape = tape;
        match tape {
            Some(tape) => self.post.set_tape(tape),
            None => self.post.reset_tape(),
        }
    }

    pub fn clear_finish(&mut self) {
        self.touch();
        self.custom_finish = false;
        self.custom_local = None;
        if let Ok(post) = self.post_chain(chain(self.style, self.display.is_none())) {
            self.post = post;
        }
    }

    pub fn set_text_after_taa(&mut self, after: bool) {
        if after != self.text_after_taa {
            self.touch();
        }
        self.text_after_taa = after;
    }

    pub fn text_after_taa(&self) -> bool {
        self.text_after_taa
    }

    pub fn set_sharpen(&mut self, strength: f32) -> Result<(), String> {
        let strength = crisp::check_strength(strength)?;
        if strength != self.sharpen {
            self.touch();
        }
        self.sharpen = strength;
        Ok(())
    }

    pub fn sharpen(&self) -> f32 {
        self.sharpen
    }

    pub fn set_texture_filter(&mut self, filter: TextureFilter) -> Result<(), String> {
        if filter != self.frame.texture_filter() {
            self.touch();
        }
        self.frame.set_texture_filter(filter)
    }

    pub fn texture_filter(&self) -> TextureFilter {
        self.frame.texture_filter()
    }

    pub fn set_lens(&mut self, lens: Option<dof::Lens>) {
        let lens = lens.filter(|lens| lens.enabled());
        if lens != self.lens {
            self.touch();
        }
        self.lens = lens;
    }

    pub fn set_haze(&mut self, haze: Option<RoomHaze>) {
        if haze != self.haze {
            self.touch();
        }
        self.haze = haze;
    }

    pub fn set_exposure(&mut self, exposure: Exposure) -> Result<(), String> {
        if exposure != self.exposure_mode {
            self.touch();
        }
        match exposure {
            Exposure::Fixed(value) if !value.is_finite() || value <= 0.0 => {
                return Err("fixed exposure must be finite and positive".into());
            }
            Exposure::Auto { bias }
                if !bias.is_finite()
                    || !(f32::MIN_POSITIVE..=f32::MAX / 8.0).contains(&2.0_f32.powf(bias)) =>
            {
                return Err("exposure bias must have a finite multiplier".into());
            }
            _ => {}
        }
        if self.exposure_mode != exposure {
            self.exposure.exposure = 1.0;
            self.exposure.stale = self.exposure.pending.is_some();
        }
        self.exposure_mode = exposure;
        Ok(())
    }

    pub fn pick(&mut self, x: u32, y: u32) -> Result<(), String> {
        let (width, height) = self.size();
        if x >= width || y >= height {
            return Err("pick pixel is outside the viewport".into());
        }
        self.pick_request = Some((x, y));
        Ok(())
    }

    pub fn picked(&mut self) -> Option<Picked> {
        let readback = self.pick_readback.as_ref()?;
        let _ = self.frame.gpu.device.poll(wgpu::PollType::Poll);
        let result = readback.receiver.try_recv();
        match result {
            Ok(Ok(())) => {
                let bytes = readback.buffer.slice(..).get_mapped_range();
                let id = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                let depth = f32::from_le_bytes(bytes[256..260].try_into().unwrap());
                drop(bytes);
                readback.buffer.unmap();
                let picked = Picked {
                    x: readback.x,
                    y: readback.y,
                    id,
                    depth,
                };
                self.pick_readback = None;
                Some(picked)
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                self.pick_readback = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    pub fn text_bounds(&self) -> &[TextBounds] {
        &self.text_bounds
    }

    pub fn text_at(&self, x: f32, y: f32) -> Option<&TextBounds> {
        self.text_bounds.iter().rev().find(|bounds| {
            let [left, top, right, bottom] = bounds.rect;
            (left..right).contains(&x) && (top..bottom).contains(&y)
        })
    }

    pub fn readback_geometry(&self) -> Result<GeometryReadback, String> {
        let targets = &self.frame.targets;
        let gpu = &self.frame.gpu;
        let [width, height] = self.frame.viewport();
        let colour = readback_texture(gpu, &targets.hdr.texture, width, height, 8)?;
        let depth = readback_texture(gpu, &targets.depth, width, height, 4)?;
        let normal = readback_texture(gpu, &targets.normal_roughness, width, height, 8)?;
        let colour = colour
            .chunks_exact(8)
            .map(|pixel| {
                std::array::from_fn(|i| {
                    half::f16::from_le_bytes(pixel[i * 2..i * 2 + 2].try_into().unwrap()).to_f32()
                })
            })
            .collect();
        let depth = depth
            .chunks_exact(4)
            .map(|pixel| f32::from_le_bytes(pixel.try_into().unwrap()))
            .collect();
        let normal = normal
            .chunks_exact(8)
            .map(|pixel| {
                std::array::from_fn(|i| {
                    half::f16::from_le_bytes(pixel[i * 2..i * 2 + 2].try_into().unwrap()).to_f32()
                })
            })
            .collect();
        Ok(GeometryReadback {
            colour,
            depth,
            normals: normal,
        })
    }

    pub fn shadows(&self) -> &Shadows {
        &self.shadows
    }

    pub fn set_shadow_quality(&mut self, quality: Quality) {
        self.touch();
        self.shadows.update_quality(&self.frame.gpu.device, quality);
    }

    pub fn upload_mesh(&mut self, data: MeshData<'_>) -> Result<MeshHandle, String> {
        self.touch();
        self.frame.upload_mesh(data)
    }

    pub fn upload_skinned_mesh(
        &mut self,
        data: MeshData<'_>,
        skin: crate::skin::SkinData<'_>,
    ) -> Result<MeshHandle, String> {
        self.touch();
        self.frame.upload_skinned_mesh(data, skin)
    }

    pub fn set_poses(&mut self, poses: &[crate::skin::InstancePose<'_>]) -> Result<(), String> {
        if self.frame.same_poses(poses) {
            return Ok(());
        }
        self.touch();
        self.frame.set_poses(poses)
    }

    pub fn set_surfaces(&mut self, surfaces: &[frame::InstanceSurface]) -> Result<(), String> {
        if self.frame.surfaces().as_slice() == surfaces {
            return Ok(());
        }
        self.touch();
        self.frame.set_surfaces(surfaces)
    }

    pub fn release_mesh(&mut self, handle: MeshHandle) -> Result<(), String> {
        self.touch();
        self.frame.release_mesh(handle)
    }

    pub fn replace_mesh(&mut self, handle: MeshHandle, data: MeshData<'_>) -> Result<(), String> {
        self.touch();
        self.frame.replace_mesh(handle, data)?;
        self.history.invalidate();
        Ok(())
    }

    pub fn mesh_stats(&self) -> frame::MeshStats {
        let glass = self.glass.mesh_stats();
        frame::MeshStats {
            glass_live: glass.live,
            glass_slots: glass.slots,
            glass_buffers: glass.buffers,
            ..self.frame.mesh_stats()
        }
    }

    pub fn glass_mesh_buffers(
        &self,
        handle: glass::MeshHandle,
    ) -> Option<(&wgpu::Buffer, &wgpu::Buffer, u32)> {
        self.glass.mesh_buffers(handle)
    }

    pub fn upload_glass(&mut self, mesh: &Mesh) -> Result<glass::MeshHandle, String> {
        self.touch();
        self.glass.upload_mesh(&self.frame.gpu.device, mesh)
    }

    pub fn release_glass(&mut self, handle: glass::MeshHandle) -> Result<(), String> {
        self.touch();
        self.glass.release_mesh(handle)
    }

    pub fn replace_glass(&mut self, handle: glass::MeshHandle, mesh: &Mesh) -> Result<(), String> {
        self.touch();
        let gpu = &self.frame.gpu;
        self.glass
            .replace_mesh(&gpu.device, &gpu.queue, handle, mesh)?;
        self.history.invalidate();
        Ok(())
    }

    pub fn lod_mut(&mut self) -> &mut LodSet {
        self.touch();
        &mut self.lod
    }

    pub fn lod_stats(&self) -> LodStats {
        self.lod.stats()
    }

    pub fn set_sky(&mut self, source: SkySource, hour: f32) -> Result<(), String> {
        self.touch();
        if !hour.is_finite() {
            return Err("hour must be finite".into());
        }
        self.frame.set_sky(source, true);
        self.sky_hours.clear();
        self.frame.set_probe_hour(hour)?;
        self.hour = hour;
        self.history.invalidate();
        Ok(())
    }

    pub fn request_sky(&mut self, source: SkySource, hour: f32) -> Result<u64, String> {
        self.touch();
        if !hour.is_finite() {
            return Err("hour must be finite".into());
        }
        let request = self.frame.set_sky(source, false);
        self.sky_hours.push((request, hour));
        Ok(request)
    }

    pub fn sky_settling(&self) -> bool {
        self.frame.sky().settling()
    }

    pub fn wait_sky(&mut self) -> Result<bool, String> {
        match self.frame.sky_mut().wait_settled() {
            Some(shown) => {
                self.sky_shown(shown)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn sky_shown(&mut self, shown: u64) -> Result<(), String> {
        self.touch();
        if let Some(&(_, hour)) = self.sky_hours.iter().find(|(request, _)| *request == shown) {
            self.frame.set_probe_hour(hour)?;
            self.hour = hour;
        }
        self.sky_hours.retain(|(request, _)| *request > shown);
        self.history.invalidate();
        Ok(())
    }

    pub fn set_probes(&mut self, probes: ProbeLighting) -> Result<(), String> {
        self.touch();
        self.frame.set_probes(probes);
        self.frame.set_probe_hour(self.hour)?;
        self.frame.set_probe_emission(self.probe_emission)
    }

    pub fn set_ambient(&mut self, ambient: Ambient) -> Result<(), String> {
        self.touch();
        self.frame.set_ambient(ambient)?;
        self.history.invalidate();
        Ok(())
    }

    pub fn set_contact(&mut self, desc: Option<ContactDesc>) -> Result<(), String> {
        self.touch();
        self.frame.set_contact(desc)?;
        self.history.invalidate();
        Ok(())
    }

    pub fn write_contact(&self, values: &[f32]) -> Result<(), String> {
        self.touch();
        self.frame.write_contact(values)
    }

    pub fn set_reflection_occlusion(&mut self, threshold: f32) -> Result<(), String> {
        self.touch();
        self.frame.set_reflection_occlusion(threshold)
    }

    pub fn set_sky_visibility(&mut self, on: bool) {
        self.touch();
        self.frame.set_sky_visibility(on);
    }

    pub fn set_probe_emission(&mut self, scale: f32) -> Result<(), String> {
        self.touch();
        self.frame.set_probe_emission(scale)?;
        self.probe_emission = scale;
        Ok(())
    }

    pub fn set_lights(&mut self, lights: &[LocalLight]) -> Result<(), String> {
        self.touch();
        self.frame.set_lights(lights)
    }

    pub fn set_light_quality(&mut self, quality: LightQuality) -> Result<(), String> {
        self.touch();
        self.frame.set_light_quality(quality)
    }

    pub fn set_transmission(&mut self, tints: &[Option<[f32; 3]>]) -> Result<(), String> {
        self.touch();
        self.frame.set_transmission(tints)
    }

    pub fn set_maps(&mut self, images: &MapImages<'_>) -> Result<(), String> {
        self.touch();
        self.frame.set_maps(images)
    }

    pub fn set_detail(&mut self, atlas: &pfx_bake::detail::atlas::Atlas) -> Result<(), String> {
        self.touch();
        self.frame.set_detail(atlas)
    }

    pub fn set_plates(
        &mut self,
        plate: Option<&pfx_bake::plate::Plate>,
        settings: crate::plates::PlateSettings,
    ) -> Result<(), String> {
        self.touch();
        self.plates = match plate {
            Some(plate) => {
                let mut plates = crate::plates::Plates::new(&self.frame.gpu, plate, settings)?;
                plates.set_hour(self.hour);
                Some(plates)
            }
            None => None,
        };
        self.taa.invalidate();
        Ok(())
    }

    pub fn plates(&self) -> Option<&crate::plates::Plates> {
        self.plates.as_ref()
    }

    pub fn set_plate_settings(&mut self, settings: crate::plates::PlateSettings) {
        if let Some(plates) = &mut self.plates
            && plates.settings() != settings
        {
            plates.set_settings(settings);
            self.touch();
        }
    }

    pub fn set_plate_hour(&mut self, hour: f32) {
        if let Some(plates) = &mut self.plates
            && plates.hour().to_bits() != hour.to_bits()
        {
            plates.set_hour(hour);
            self.touch();
        }
    }

    pub fn set_plate_casters(&mut self, casters: &[Instance]) {
        self.touch();
        self.plate_casters = casters
            .iter()
            .map(|caster| Instance {
                shadow_only: true,
                previous_model: caster.model,
                ..*caster
            })
            .collect();
    }

    pub fn plate_casters(&self) -> &[Instance] {
        &self.plate_casters
    }

    pub fn set_props(&mut self, props: Option<ImpostorPass>) {
        self.touch();
        self.impostors = props;
    }

    pub fn set_tree(
        &mut self,
        tree: &Tree,
        origin: [f32; 3],
        receiver_y: f32,
        id: u32,
        petal_rate: f32,
        petal_color: [f32; 3],
    ) -> Result<MeshHandle, String> {
        self.touch();
        let handle = self.frame.set_tree(tree, origin, receiver_y, id)?;
        self.petals = Some(Petals::new(tree, origin, petal_rate, petal_color));
        Ok(handle)
    }

    pub fn set_canopy_mode(&mut self, mode: CanopyMode) -> Result<(), String> {
        self.touch();
        self.frame.set_canopy_mode(mode)
    }

    pub fn set_canopy_sharpness(&mut self, sharpness: f32) -> Result<(), String> {
        self.touch();
        self.frame.set_canopy_sharpness(sharpness)
    }

    pub fn set_canopy_strength(&mut self, strength: f32) -> Result<(), String> {
        self.touch();
        self.frame.set_canopy_strength(strength)
    }

    pub fn set_canopy_gobo(&mut self, gobo: Gobo) -> Result<(), String> {
        self.touch();
        self.frame.set_canopy_gobo(gobo)?;
        self.text.set_canopy_gobo(gobo);
        Ok(())
    }

    pub fn volume_cuts(&self) -> effects::VolumeCuts {
        self.effects.volume_cuts()
    }

    pub fn set_volume_cuts(&mut self, cuts: effects::VolumeCuts) {
        self.touch();
        let device = self.frame.gpu.device.clone();
        self.effects.set_volume_cuts(&device, cuts);
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.touch();
        self.frame.resize(width, height)?;
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        self.taa.resize(device, width, height);
        self.crisp.resize(width, height);
        let cuts = self.effects.volume_cuts();
        self.effects = effects::Effects::new(device, width, height, 4)?;
        self.effects.set_volume_cuts(device, cuts);
        self.post.resize(width, height);
        self.dof.resize(width, height);
        self.reflections.resize(device, width, height);
        let meshes = self.glass.take_meshes();
        self.glass = glass::Pass::new(device, width, height);
        self.glass.set_meshes(meshes);
        if self.display.is_some() {
            self.display = Some(image(
                device,
                width,
                height,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                "live display intermediate",
            ));
        }
        self.depths = linear_depths(device, width, height);
        self.mips = mips(device, width, height);
        (self.reactive, self.reactive_view) = image(
            device,
            width,
            height,
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "live reactive mask",
        );
        if let Some(limits) = self.frames_on_demand() {
            self.demand = Some(demand::Cache::new(device, [width, height], limits));
        }
        self.history.invalidate();
        self.history_valid = false;
        self.pass_log.clear();
        self.text_bounds.clear();
        self.pick_request = None;
        self.pick_readback = None;
        self.set_viewport_parts(width, height)
    }

    pub fn render(
        &mut self,
        scene: &Scene<'_>,
        text: &Text<'_>,
        effects: &Effects<'_>,
        style: Finish,
        output: &wgpu::TextureView,
    ) -> Result<Vec<PassTiming>, String> {
        let frame = self.encode_frame(scene, text, effects, style, None, output, false)?;
        self.wait_for(frame)
    }

    pub fn render_with_flat(
        &mut self,
        scene: &Scene<'_>,
        text: &Text<'_>,
        effects: &Effects<'_>,
        style: Finish,
        flat: &FlatScene<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Vec<PassTiming>, String> {
        let frame = self.encode_frame(scene, text, effects, style, Some(flat), output, false)?;
        self.wait_for(frame)
    }

    pub fn render_flat(
        &mut self,
        flat: &FlatScene<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Vec<PassTiming>, String> {
        let frame = self.encode_flat_frame(flat, None, output)?;
        self.wait_for(frame)
    }

    pub fn render_flat_look(
        &mut self,
        flat: &FlatScene<'_>,
        look: &LookFrame<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Vec<PassTiming>, String> {
        let frame = self.encode_flat_frame(flat, Some(look), output)?;
        self.wait_for(frame)
    }

    fn wait_for(&mut self, frame: u64) -> Result<Vec<PassTiming>, String> {
        self.frame
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("{error:?}"))?;
        self.gather_timings(self.frames_submitted.saturating_sub(1));
        Ok(std::mem::take(&mut self.arrivals)
            .into_iter()
            .find(|timings| timings.frame == frame)
            .map(|timings| timings.passes)
            .unwrap_or_default())
    }

    pub fn submit(
        &mut self,
        scene: &Scene<'_>,
        text: &Text<'_>,
        effects: &Effects<'_>,
        style: Finish,
        output: &wgpu::TextureView,
    ) -> Result<Submitted, String> {
        let frame = self.encode_frame(scene, text, effects, style, None, output, false)?;
        Ok(self.submitted_frame(frame))
    }

    pub fn submit_with_flat(
        &mut self,
        scene: &Scene<'_>,
        text: &Text<'_>,
        effects: &Effects<'_>,
        style: Finish,
        flat: &FlatScene<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Submitted, String> {
        let frame = self.encode_frame(scene, text, effects, style, Some(flat), output, false)?;
        Ok(self.submitted_frame(frame))
    }

    pub fn submit_flat(
        &mut self,
        flat: &FlatScene<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Submitted, String> {
        let frame = self.encode_flat_frame(flat, None, output)?;
        Ok(self.submitted_frame(frame))
    }

    pub fn submit_flat_look(
        &mut self,
        flat: &FlatScene<'_>,
        look: &LookFrame<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Submitted, String> {
        let frame = self.encode_flat_frame(flat, Some(look), output)?;
        Ok(self.submitted_frame(frame))
    }

    pub fn flat_looks(&self) -> Option<&FlatLooks> {
        self.flat_looks.as_ref()
    }

    fn submitted_frame(&mut self, frame: u64) -> Submitted {
        let _ = self.frame.gpu.device.poll(wgpu::PollType::Poll);
        self.gather_timings(self.frames_submitted.saturating_sub(1));
        Submitted {
            frame,
            arrived: std::mem::take(&mut self.arrivals),
        }
    }

    pub fn flat_pass(&self) -> Option<&FlatPass> {
        self.flat.as_ref()
    }

    pub fn warm_flat<'a>(
        &mut self,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        looks: impl IntoIterator<Item = &'a Look>,
    ) -> Result<(), String> {
        if size[0] == 0 || size[1] == 0 {
            return Err("warm_flat needs a size of at least 1 by 1".into());
        }
        let device = self.frame.gpu.device.clone();
        let queue = self.frame.gpu.queue.clone();
        let hdr = wgpu::TextureFormat::Rgba16Float;
        let finishes = [
            (format, true, false),
            (hdr, true, false),
            (hdr, self.display.is_some(), true),
        ];
        let mut flat = self
            .flat
            .take()
            .unwrap_or_else(|| FlatPass::new(&device, &queue));
        flat.warm(&device, size, &finishes, true);
        self.flat = Some(flat);
        let mut set = self
            .flat_looks
            .take()
            .unwrap_or_else(|| FlatLooks::new(&device, &queue));
        let warmed = set.warm(&device, &queue, looks, size);
        self.flat_looks = Some(set);
        warmed
    }

    pub fn flat_pipelines_made_on_render_thread(&self) -> usize {
        self.flat.as_ref().map_or(0, FlatPass::pipelines_made)
            + self
                .flat_looks
                .as_ref()
                .map_or(0, FlatLooks::pipelines_made)
    }

    fn encode_flat_frame(
        &mut self,
        scene: &FlatScene<'_>,
        look: Option<&LookFrame<'_>>,
        output: &wgpu::TextureView,
    ) -> Result<u64, String> {
        let look = look.filter(|look| !look.is_identity());
        if look.is_some() && self.size() != self.capacity() {
            return Err("a flat look renders at the full size; set the viewport to the renderer's capacity first".into());
        }
        self.begin_frame(FrameKind::Full);
        self.pass_log.clear();
        self.text_bounds.clear();
        let (width, height) = self.size();
        let (capacity_width, capacity_height) = self.capacity();
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        let queue = self.frame.gpu.queue.clone();
        if let Some(tape) = self.tape {
            self.post.set_tape(tape);
        }
        let mut flat = self
            .flat
            .take()
            .unwrap_or_else(|| FlatPass::new(device, &queue));
        flat.set_capacity(Some([capacity_width, capacity_height]));
        if look.is_none() {
            let prepared = flat.prepare(device, &queue, scene, [width, height]);
            if let Err(error) = prepared {
                self.flat = Some(flat);
                return Err(error);
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live flat"),
        });
        let clear = Some(
            scene
                .clear
                .map_or([0.0; 4], |colour| colour.premultiplied()),
        );
        let direct = !scene.post && self.display.is_none();
        let target = if direct {
            output.clone()
        } else {
            flat.colour_target(device, [capacity_width, capacity_height])
        };
        if let Some(look) = look {
            let mut looks = self
                .flat_looks
                .take()
                .unwrap_or_else(|| FlatLooks::new(device, &queue));
            let encoded = looks.encode(
                device,
                &queue,
                &mut encoder,
                &mut flat,
                scene,
                look,
                [width, height],
                &target,
                Some(&mut self.profiler),
            );
            self.flat_looks = Some(looks);
            if let Err(error) = encoded {
                self.flat = Some(flat);
                return Err(error);
            }
            self.pass_log.push("flat look");
        } else {
            flat.encode_colour(&mut encoder, &target, clear, Some(&mut self.profiler));
            self.pass_log.push("flat");
        }
        if !direct {
            if scene.post {
                let hdr = &self.frame.targets.hdr.view;
                flat.encode_finish(
                    device,
                    &mut encoder,
                    &target,
                    hdr,
                    wgpu::TextureFormat::Rgba16Float,
                    true,
                    false,
                    Some(&mut self.profiler),
                );
                self.pass_log.push("flat finish");
                let post_output = self.display.as_ref().map_or(output, |(_, view)| view);
                self.post.run_timed_exposed(
                    &mut encoder,
                    hdr,
                    None,
                    None,
                    post_output,
                    scene.frame,
                    scene.seed,
                    1.0,
                    Some(&mut self.profiler),
                );
                self.pass_log.push("post");
                if self.display.is_some() {
                    let display = self.display.as_ref().map(|(_, view)| view.clone());
                    if let Some(display) = display {
                        self.encode_blit(device, &mut encoder, &display, output);
                    }
                }
            } else {
                flat.encode_finish(
                    device,
                    &mut encoder,
                    &target,
                    output,
                    self.output_format,
                    true,
                    false,
                    Some(&mut self.profiler),
                );
                self.pass_log.push("flat finish");
            }
        }
        flat.encode_ids(
            &mut encoder,
            &self.frame.targets.ids_view,
            true,
            Some(&mut self.profiler),
        );
        self.pass_log.push("flat ids");
        self.flat = Some(flat);
        self.finish_encoder(device, &queue, encoder, false)
    }

    pub fn frame_stats(&self) -> FrameStats {
        self.stats.clone()
    }

    pub fn set_frame_stats(&mut self, stats: FrameStats) {
        self.stats = stats;
    }

    fn begin_frame(&mut self, kind: FrameKind) {
        self.stats.begin(kind);
        self.resolution_frame = kind == FrameKind::Full;
    }

    pub fn set_dynamic_resolution(&mut self, budget: Option<Budget>) -> Result<(), String> {
        self.resolution = budget.map(DynamicResolution::new).transpose()?;
        self.resolution_content = None;
        Ok(())
    }

    pub fn dynamic_resolution(&self) -> Option<&DynamicResolution> {
        self.resolution.as_ref()
    }

    pub fn apply_dynamic_resolution(&mut self, content: [u32; 2]) -> Result<(u32, u32), String> {
        let Some(resolution) = self.resolution.as_ref() else {
            return Ok(self.size());
        };
        let [width, height] = resolution
            .render_size(content)
            .ok_or("the content size must be at least 1 by 1")?;
        let [capacity_width, capacity_height] = resolution
            .capacity(content)
            .ok_or("the content size must be at least 1 by 1")?;
        self.resolution_content = Some(content);
        if self.capacity() != (capacity_width, capacity_height) {
            self.resize(capacity_width, capacity_height)?;
        }
        let (width, height) = (width.min(capacity_width), height.min(capacity_height));
        self.set_viewport(width, height)?;
        Ok((width, height))
    }

    pub fn timings(&self) -> Option<&FrameTimings> {
        self.latest_timings.as_ref()
    }

    pub fn timing_drops(&self) -> TimingDrops {
        TimingDrops {
            ring: self.profiler.dropped(),
            late: self.late_timings,
        }
    }

    pub fn frames_submitted(&self) -> u64 {
        self.frames_submitted
    }

    pub fn frames_in_flight(&self) -> u64 {
        self.frames_submitted
            .saturating_sub(self.frames_done.load(Ordering::Acquire))
    }

    pub fn wait_frames(&mut self, in_flight: u64) -> Result<Vec<FrameTimings>, String> {
        let done = self.frames_done.load(Ordering::Acquire);
        while self
            .submissions
            .front()
            .is_some_and(|(frame, _)| *frame < done)
        {
            self.submissions.pop_front();
        }
        let waiting = self.submissions.len() as u64;
        if waiting > in_flight {
            let index = self.submissions[(waiting - in_flight - 1) as usize]
                .1
                .clone();
            self.frame
                .gpu
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(index),
                    timeout: None,
                })
                .map_err(|error| format!("{error:?}"))?;
        } else {
            let _ = self.frame.gpu.device.poll(wgpu::PollType::Poll);
        }
        self.gather_timings(self.frames_submitted.saturating_sub(1));
        Ok(std::mem::take(&mut self.arrivals))
    }

    fn gather_timings(&mut self, current: u64) {
        for timings in self.profiler.gather() {
            if self.stats.enabled() || self.resolution.is_some() {
                let total = timings.passes.iter().map(|pass| pass.milliseconds).sum();
                self.stats.gpu(timings.frame, total);
                if let Some(resolution) = self.resolution.as_mut() {
                    resolution.observe(timings.frame, total);
                }
            }
            if current.saturating_sub(timings.frame) > TIMING_LAG {
                self.late_timings += 1;
                continue;
            }
            let timings = FrameTimings {
                frame: timings.frame,
                passes: plain_passes(timings.passes),
            };
            if self
                .latest_timings
                .as_ref()
                .is_none_or(|latest| latest.frame < timings.frame)
            {
                self.latest_timings = Some(timings.clone());
            }
            self.arrivals.push(timings);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_frame(
        &mut self,
        scene: &Scene<'_>,
        text: &Text<'_>,
        effects: &Effects<'_>,
        style: Finish,
        hud: Option<&FlatScene<'_>>,
        output: &wgpu::TextureView,
        cached: bool,
    ) -> Result<u64, String> {
        if !cached {
            self.begin_frame(FrameKind::Full);
        }
        self.pass_log.clear();
        let (width, height) = self.size();
        self.text.set_viewport(Some([width, height]));
        self.text_bounds = text_bounds(text, [width, height]);
        if !scene.time.is_finite() || scene.time < 0.0 {
            return Err("scene time must be finite and nonnegative".into());
        }
        if let Some(shown) = self.frame.sky_mut().swap_settled() {
            self.sky_shown(shown)?;
        }
        let frame_number = (scene.time * 60.0).round() as u32;
        let view_projection = frame::multiply(scene.camera.projection, scene.camera.view);
        let mut camera = scene.camera;
        if cached {
            if let Some(last) = self.history.last_view_projection() {
                camera.previous_view_projection = last;
            }
            self.history.bridge(scene.time);
        }
        let valid =
            self.history
                .advance(scene.time, camera.previous_view_projection, view_projection);
        self.history_valid = valid;
        if !valid {
            self.taa.invalidate();
        }
        let metered = if matches!(self.exposure_mode, Exposure::Auto { .. }) {
            self.exposure.advance(scene.time, valid)
        } else {
            1.0
        };
        let exposure = self.exposure_mode.value(metered);
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        let queue = self.frame.gpu.queue.clone();
        if style != self.style {
            if !self.custom_finish {
                self.post = self.post_chain(chain(style, self.display.is_none()))?;
            }
            self.style = style;
        }
        if let Some(tape) = self.tape {
            self.post.set_tape(tape);
        }
        let mut lod_instances = Vec::new();
        if self.lod.instance_mut(0).is_some() {
            self.lod.select(scene.camera, height);
            let mut lod = std::mem::replace(&mut self.lod, LodSet::new(0));
            let uploaded = lod.upload_selected(&mut self.frame);
            self.lod = lod;
            lod_instances = uploaded?;
            self.pass_log.push("lod");
        }
        let dynamic = scene.instances;
        let stand_ins: &[Instance] = if self.plates.is_some() {
            &self.plate_casters
        } else {
            &[]
        };
        let combined: Vec<Instance>;
        let instances = if lod_instances.is_empty() && stand_ins.is_empty() {
            scene.instances
        } else {
            combined = lod_instances
                .into_iter()
                .chain(scene.instances.iter().copied())
                .chain(stand_ins.iter().copied())
                .collect();
            &combined
        };
        let scene = Scene {
            camera,
            time: scene.time,
            seed: scene.seed,
            sun: scene.sun,
            instances,
            materials: scene.materials,
            deformers: scene.deformers,
            wind: scene.wind,
        };
        let jitter = taa::jitter_clip(frame_number, width, height);
        self.frame.set_jitter(jitter);
        let projection = frame::multiply(frame::jitter_matrix(jitter), scene.camera.projection);
        let jittered = frame::multiply(projection, scene.camera.view);
        let view = camera_view(&scene, width, height);
        let right = [
            scene.camera.view[0][0],
            scene.camera.view[1][0],
            scene.camera.view[2][0],
        ];
        let lens = [
            1.0 / scene.camera.projection[0][0],
            1.0 / scene.camera.projection[1][1],
        ];
        let shift = [scene.camera.projection[2][0], scene.camera.projection[2][1]];
        let sky_forward = if shift == [0.0; 2] {
            view.forward
        } else {
            std::array::from_fn(|k| {
                view.forward[k] + right[k] * lens[0] * shift[0] + view.up[k] * lens[1] * shift[1]
            })
        };
        self.frame.sky_mut().update(
            &queue,
            sky_forward,
            right.map(|value| value * lens[0]),
            view.up.map(|value| value * lens[1]),
        );
        self.frame
            .set_reflections(self.reflections.result().filter(|_| valid));
        let fit = self.shadows.fit(&view, scene.sun.direction);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live renderer"),
        });
        let shadow_props = self
            .impostors
            .as_ref()
            .filter(|_| !effects.props.is_empty());
        let cascades: Vec<Matrix> = fit
            .cascades
            .iter()
            .map(|cascade| cascade.view_proj.columns)
            .collect();
        let (fluid, ripple) = effects
            .liquid
            .as_ref()
            .map_or((&self.black, &self.black), |liquid| {
                (liquid.fluid, liquid.ripple)
            });
        let faces = self.frame.lights().faces();
        let light_faces = faces.len();
        let cast_layers: Vec<Matrix> = cascades
            .iter()
            .copied()
            .chain(faces.into_iter().map(|(_, matrix)| matrix))
            .collect();
        let glass_casting =
            self.glass
                .prepare_casts(device, &queue, &cast_layers, effects.glass, fluid, ripple)?;
        let glass = &self.glass;
        let mut glass_casts = |pass: &mut wgpu::RenderPass<'_>, layer: usize, _: bool| {
            glass.draw_casts(pass, layer);
        };
        self.frame.encode_with_casts(
            &scene,
            &mut encoder,
            Some((&mut self.shadows, &fit)),
            Some(&mut self.profiler),
            if glass_casting {
                Some(&mut glass_casts)
            } else {
                None
            },
            |gpu, encoder, shadows, fit| {
                if let Some(props) = shadow_props {
                    props.update_shadows(gpu, fit, self.hour, effects.props)?;
                    shadows.render_impostors(encoder, props, effects.props.len())?;
                }
                if let Some(mesh) = effects.creature_mesh
                    && !effects.creatures.is_empty()
                {
                    shadows.render_creatures(
                        gpu,
                        encoder,
                        fit,
                        mesh,
                        effects.creatures,
                        scene.time,
                    );
                }
                Ok(())
            },
        )?;
        if self.frame.has_tree() {
            self.pass_log.push("canopy cookie");
        }
        self.pass_log.push("shadows");
        if self.frame.lights().any_transmissive() || glass_casting {
            self.pass_log.push("shadow transmission");
        }
        if glass_casting && light_faces > 0 {
            self.pass_log.push("light transmission");
        }
        if shadow_props.is_some() {
            self.pass_log.push("impostor shadows");
        }
        if effects.creature_mesh.is_some() && !effects.creatures.is_empty() {
            self.pass_log.push("creature shadows");
        }
        if !self.frame.lights().shadowed().is_empty() {
            self.pass_log.push("light shadows");
        }
        self.pass_log.extend(["prepass", "opaque"]);
        if let Some(props) = &self.impostors
            && !effects.props.is_empty()
        {
            props.update(
                &self.frame.gpu,
                ImpostorParams {
                    view_projection: jittered,
                    camera_position: [
                        scene.camera.position[0],
                        scene.camera.position[1],
                        scene.camera.position[2],
                        0.0,
                    ],
                    camera_right: [right[0], right[1], right[2], 0.0],
                    camera_up: [view.up[0], view.up[1], view.up[2], 0.0],
                    settings: [0.0; 4],
                    source: [0.0; 4],
                },
                self.hour,
                effects.props,
            )?;
            let timing = self.profiler.pass("impostors");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impostors"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frame.targets.hdr.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.frame.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.profiler.render_writes(timing),
                occlusion_query_set: None,
            });
            crate::viewport::apply(&mut pass, [width, height]);
            props.draw(&mut pass, effects.props.len())?;
            drop(pass);
            self.pass_log.push("impostors");
        }
        if let Some(plates) = self.plates.as_mut() {
            let shadowed = plates.encode(
                &self.frame,
                (&self.shadows, &fit),
                &self.reactive_view,
                &mut encoder,
                &crate::plates::PlateFrame {
                    view_projection,
                    jittered,
                    previous: scene.camera.previous_view_projection,
                    view: scene.camera.view,
                    eye: scene.camera.position,
                    toward_sun: scene.sun.direction,
                    sun_light: scene
                        .sun
                        .colour
                        .map(|channel| channel * scene.sun.intensity),
                    sun_up: scene.sun.intensity > 0.0,
                    instances: dynamic,
                },
                Some(&mut self.profiler),
            )?;
            if shadowed {
                self.pass_log.push("plate shadows");
            }
            self.pass_log.push("plate composite");
            self.plate_mask = true;
        } else if self.plate_mask {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("plate mask clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.reactive_view,
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
            self.plate_mask = false;
        }
        self.depth_index = 1 - self.depth_index;
        let current_depth = &self.depths[self.depth_index].1;
        let depth_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live depth limits"),
            contents: bytemuck::cast_slice(&[
                scene.camera.projection[2][2],
                scene.camera.projection[3][2],
                view.far,
                0.0,
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live linear depth"),
            layout: &self.depth_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.frame.targets.depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(current_depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: depth_uniform.as_entire_binding(),
                },
            ],
        });
        let timing = self.profiler.pass("linear depth");
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("linear depth"),
                timestamp_writes: self.profiler.compute_writes(timing),
            });
            pass.set_pipeline(&self.depth_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        self.pass_log.push("linear depth");
        let inverse = invert(jittered).ok_or("camera projection is singular")?;
        let mut motion = Vec::with_capacity(52);
        for matrix in [
            inverse,
            view_projection,
            scene.camera.previous_view_projection,
        ] {
            motion.extend(matrix.iter().flatten().copied());
        }
        motion.extend([
            width as f32,
            height as f32,
            1.0 / width as f32,
            1.0 / height as f32,
        ]);
        queue.write_buffer(&self.motion.uniform, 0, bytemuck::cast_slice(&motion));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live background motion"),
            layout: &self.motion.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.frame.targets.depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.frame.targets.ids_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.motion.uniform.as_entire_binding(),
                },
            ],
        });
        let timing = self.profiler.pass("background motion");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background motion"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frame.targets.velocity_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.profiler.render_writes(timing),
                occlusion_query_set: None,
            });
            crate::viewport::apply(&mut pass, [width, height]);
            pass.set_pipeline(&self.motion.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.pass_log.push("background motion");
        let timing = self.profiler.pass("sky");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frame.targets.hdr.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.frame.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.profiler.render_writes(timing),
                occlusion_query_set: None,
            });
            crate::viewport::apply(&mut pass, [width, height]);
            self.frame.sky().draw(&mut pass);
        }
        self.pass_log.push("sky");
        self.mips.copy_scene(
            &mut encoder,
            &self.frame.targets.hdr.texture,
            [width, height],
        );
        let timing = self.profiler.pass("scene colour mips");
        let fresh = self.mips.written != Some([width, height]);
        self.mips.written = Some([width, height]);
        for level in 1..self.mips.levels.len() {
            let rect = [width, height].map(|size| crate::viewport::mip(size, level as u32));
            if fresh {
                queue.write_buffer(
                    &self.mips.extents[level],
                    0,
                    bytemuck::cast_slice(&[rect[0], rect[1], 0, 0]),
                );
            }
            let last = level + 1 == self.mips.levels.len();
            let writes = if level == 1 && last {
                self.profiler.render_writes(timing)
            } else if level == 1 {
                self.profiler
                    .render_writes(timing)
                    .map(|writes| wgpu::RenderPassTimestampWrites {
                        end_of_pass_write_index: None,
                        ..writes
                    })
            } else if last {
                self.profiler
                    .render_writes(timing)
                    .map(|writes| wgpu::RenderPassTimestampWrites {
                        beginning_of_pass_write_index: None,
                        ..writes
                    })
            } else {
                None
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene colour mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.mips.levels[level],
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: writes,
                occlusion_query_set: None,
            });
            crate::viewport::apply(&mut pass, self.mips.bordered(rect, level as u32));
            pass.set_pipeline(&self.mips.pipeline);
            pass.set_bind_group(0, &self.mips.binds[level - 1], &[]);
            pass.draw(0..3, 0..1);
        }
        self.pass_log.push("scene colour");
        let inverse_projection = invert(projection).ok_or("camera projection is singular")?;
        self.reflections.encode(
            device,
            &mut encoder,
            &reflect::Inputs {
                depth: current_depth,
                normal_roughness: &self.frame.targets.normal_roughness_view,
                colour: &self.mips.view,
                motion: &self.frame.targets.velocity_view,
                projection,
                inverse_projection,
                frame_index: frame_number,
                thickness: reflect::THICKNESS,
                history_valid: valid,
            },
            Some(&mut self.profiler),
        );
        self.pass_log.push("reflections");
        let mut reactive = &self.reactive_view;
        if glass::drawn(effects.glass) {
            let cube = self.frame.sky().cube_view();
            let caustic = self.frame.caustic();
            let content = self.frame.content();
            let caustic_view = caustic
                .as_ref()
                .map_or(&self.black, |caustic| content.view(caustic.slot));
            let (fluid, ripple) = effects
                .liquid
                .as_ref()
                .map_or((&self.black, &self.black), |liquid| {
                    (liquid.fluid, liquid.ripple)
                });
            self.glass.encode(
                device,
                &queue,
                &mut encoder,
                glass::Render {
                    camera: glass::Camera {
                        view: scene.camera.view,
                        projection,
                    },
                    surfaces: effects.glass,
                    inputs: glass::Inputs {
                        opaque_hdr_mips: &self.mips.view,
                        opaque_depth: &self.frame.targets.depth_view,
                        reflection_cube: &cube,
                        caustic: caustic_view,
                        caustic_projection: caustic
                            .as_ref()
                            .map(|caustic| caustic.projection(scene.sun.direction)),
                        fluid_height: fluid,
                        ripple_height: ripple,
                        mip_count: self.mips.levels.len() as u32,
                        sky_sh: self.frame.sky().lighting.sh,
                    },
                    output: &self.frame.targets.hdr.view,
                    profiler: Some(&mut self.profiler),
                },
            )?;
            reactive = self.glass.reactive();
            self.pass_log.push("glass");
        }
        let mut effect_frame = EffectFrame::new(width, height, scene.time, scene.seed);
        effect_frame.view_proj = jittered;
        effect_frame.eye = [
            scene.camera.position[0],
            scene.camera.position[1],
            scene.camera.position[2],
            0.0,
        ];
        effect_frame.right = [right[0], right[1], right[2], 0.0];
        effect_frame.up = [view.up[0], view.up[1], view.up[2], 0.0];
        effect_frame.forward = [view.forward[0], view.forward[1], view.forward[2], 0.0];
        effect_frame.lens = [lens[0], lens[1], 0.0, 0.0];
        effect_frame.sun = [
            scene.sun.direction[0],
            scene.sun.direction[1],
            scene.sun.direction[2],
            0.0,
        ];
        effect_frame.sun_color = [
            scene.sun.colour[0],
            scene.sun.colour[1],
            scene.sun.colour[2],
            scene.sun.intensity,
        ];
        let shadow_data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("effects sun cascades"),
            contents: &self.shadows.sample_uniform(&fit),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let targets = EffectTargets {
            hdr: &self.frame.targets.hdr.view,
            depth: current_depth,
            shadow: Some((self.shadows.atlas_view(), &shadow_data)),
        };
        if let Some(haze) = self.haze {
            let mut room_frame = effect_frame;
            room_frame.set_room_haze(haze);
            self.effects.prepare_room(
                &mut EffectCommands {
                    device,
                    encoder: &mut encoder,
                },
                &haze,
            );
            let march = self.profiler.pass("haze march");
            let upsample = self.profiler.pass("haze upsample");
            self.effects.render_volume(
                &mut EffectCommands {
                    device,
                    encoder: &mut encoder,
                },
                &room_frame,
                targets,
                VolumeTimestamps {
                    march: self.profiler.render_writes(march),
                    upsample: self.profiler.render_writes(upsample),
                },
            );
            self.pass_log.extend(["haze march", "haze upsample"]);
        }
        if let Some(volume) = effects.volume.as_ref().filter(|_| !cached) {
            let mut volume_frame = effect_frame;
            volume_frame.set_volume(
                volume.kind,
                volume.lo,
                volume.hi,
                volume.color,
                volume.density,
                volume.anisotropy,
            );
            volume_frame.scale_time(volume.time_scale);
            let march = self.profiler.pass("volume march");
            let upsample = self.profiler.pass("volume upsample");
            self.effects.render_volume(
                &mut EffectCommands {
                    device,
                    encoder: &mut encoder,
                },
                &volume_frame,
                targets,
                VolumeTimestamps {
                    march: self.profiler.render_writes(march),
                    upsample: self.profiler.render_writes(upsample),
                },
            );
            self.pass_log.extend(["volume march", "volume upsample"]);
        }
        let petals = self
            .petals
            .as_mut()
            .map(|petals| petals.step(scene.time, &scene.wind.current))
            .unwrap_or_default();
        let own: &[ParticleInstance] = if cached { &[] } else { effects.particles };
        let particles: Vec<ParticleInstance>;
        let particles = if petals.is_empty() {
            own
        } else {
            particles = own.iter().copied().chain(petals).collect();
            &particles
        };
        if !particles.is_empty() {
            let timing = self.profiler.pass("particles");
            self.effects.render_particles(
                &mut EffectCommands {
                    device,
                    encoder: &mut encoder,
                },
                &effect_frame,
                targets,
                particles,
                self.profiler.render_writes(timing),
            );
            self.pass_log.push("particles");
        }
        if let Some(mesh) = effects.creature_mesh
            && !effects.creatures.is_empty()
        {
            let timing = self.profiler.pass("creatures");
            self.effects.render_creatures(
                &mut EffectCommands {
                    device,
                    encoder: &mut encoder,
                },
                &effect_frame,
                targets,
                mesh,
                effects.creatures,
                self.profiler.render_writes(timing),
            );
            self.pass_log.push("creatures");
        }
        let deformers = self.text.deformer_group(
            device,
            self.frame.deformer_buffer().buffer(),
            self.frame.deformer_buffer().plan_buffer(),
        );
        let surface_text = text
            .surface
            .iter()
            .any(|item| !item.paragraph.glyphs.is_empty())
            || text.surface_quads.iter().any(|item| !item.quads.is_empty());
        if surface_text && !self.text_after_taa {
            draw_surface_text(
                &mut self.text,
                &self.frame,
                &mut self.profiler,
                device,
                &mut encoder,
                text,
                &deformers,
                &self.frame.targets.hdr.view,
            )?;
            self.pass_log.push("surface text");
        }
        self.taa.resolve_timed(
            device,
            &queue,
            &mut encoder,
            &GpuInputs {
                colour: &self.frame.targets.hdr.view,
                depth: current_depth,
                motion: &self.frame.targets.velocity_view,
                reactive,
                id: Some(&self.frame.targets.ids_view),
                normal: Some(&self.frame.targets.normal_roughness_view),
                exposure,
            },
            &taa::Settings::default(),
            Some(&mut self.profiler),
        )?;
        self.pass_log.push("TAA");
        let resolved = self
            .taa
            .history_view()
            .ok_or("TAA output is empty")?
            .clone();
        let late_text = surface_text && self.text_after_taa;
        let resolved = if self.sharpen > 0.0 || late_text {
            let display = self.crisp.encode(
                device,
                &queue,
                &mut encoder,
                &resolved,
                self.sharpen,
                exposure,
                Some(&mut self.profiler),
            );
            self.pass_log.push(if self.sharpen > 0.0 {
                "sharpen"
            } else {
                "crisp copy"
            });
            if late_text {
                draw_surface_text(
                    &mut self.text,
                    &self.frame,
                    &mut self.profiler,
                    device,
                    &mut encoder,
                    text,
                    &deformers,
                    display,
                )?;
                self.pass_log.push("surface text");
            }
            display.clone()
        } else {
            resolved
        };
        let layered = cached && (effects.volume.is_some() || !effects.particles.is_empty());
        let resolved = if let Some(cache) = self.demand.as_ref().filter(|_| cached) {
            let whole = [Region::whole(width, height)];
            if layered {
                cache.encode_copy(
                    device,
                    &mut encoder,
                    &resolved,
                    &cache.still.view,
                    &whole,
                    "demand still",
                    &mut self.profiler,
                );
                self.pass_log.push("demand still");
                let layer = cache.layer.view.clone();
                cache.encode_copy(
                    device,
                    &mut encoder,
                    &resolved,
                    &layer,
                    &whole,
                    "demand layer",
                    &mut self.profiler,
                );
                let mut layer_frame = effect_frame;
                layer_frame.view_proj = view_projection;
                draw_layers(
                    device,
                    &mut encoder,
                    &mut self.effects,
                    &mut self.profiler,
                    effects,
                    &layer_frame,
                    EffectTargets {
                        hdr: &layer,
                        depth: current_depth,
                        shadow: Some((self.shadows.atlas_view(), &shadow_data)),
                    },
                    &mut self.pass_log,
                );
                layer
            } else {
                resolved
            }
        } else {
            resolved
        };
        let resolved = if let Some(lens) = self.lens {
            let timing = self.profiler.pass("depth of field");
            self.pass_log.push("depth of field");
            self.dof.render(
                &mut encoder,
                &resolved,
                current_depth,
                lens,
                dof::height_plane(scene.camera.view, scene.camera.projection),
                self.profiler.compute_writes(timing),
            )
        } else {
            &resolved
        };
        if matches!(self.exposure_mode, Exposure::Auto { .. }) {
            self.exposure
                .encode(device, &queue, &mut encoder, resolved, [width, height]);
        }
        let overlays = !text.overlay.is_empty() || !text.overlay_quads.is_empty() || hud.is_some();
        let colour = self
            .demand
            .as_ref()
            .filter(|_| cached)
            .map(|cache| cache.colour.view.clone());
        let post_output = match &colour {
            Some(view) => view,
            None => self.display.as_ref().map_or(output, |(_, view)| view),
        };
        self.post.run_timed_exposed(
            &mut encoder,
            resolved,
            Some(&self.frame.targets.depth_view),
            Some(&self.frame.targets.normal_roughness_view),
            post_output,
            frame_number,
            scene.seed,
            exposure,
            Some(&mut self.profiler),
        );
        self.pass_log.push("post");
        if let Some(cache) = self.demand.as_mut().filter(|_| cached) {
            cache.probe_depth(&mut encoder, &self.depths[self.depth_index].0);
            cache.encode_tiles(
                device,
                &mut encoder,
                &self.depths[self.depth_index].1,
                &mut self.profiler,
            );
            self.pass_log.push("demand tiles");
            let previous = cache.cached.as_ref().map(|cached| cached.exposure);
            cache.exposure_moving = previous
                .is_some_and(|previous| (previous - exposure).abs() > 1e-3 * exposure.abs());
            cache.cached = Some(demand::Cached {
                view: scene.camera.view,
                projection: scene.camera.projection,
                far: view.far,
                depth_index: self.depth_index,
                shadow: self.shadows.sample_uniform(&fit).to_vec(),
                frame_number,
                exposure,
            });
            cache.colour_layers = layered;
            cache.book.still = layered;
            cache.last = demand::Last::Colour;
            cache.overlaid = overlays;
            cache.overlay_print = None;
            if overlays {
                cache.encode_clear(&mut encoder);
            }
        }
        let overlay_target = match self.demand.as_ref().filter(|_| cached) {
            Some(cache) => cache.overlay.view.clone(),
            None => post_output.clone(),
        };
        for item in &text.overlay {
            self.text
                .encode_timed(
                    device,
                    &mut encoder,
                    TextDraw {
                        target: &overlay_target,
                        depth: None,
                        atlas: item.atlas,
                        paragraph: item.paragraph,
                        space: item.space,
                        color: item.color,
                        deformers: None,
                    },
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        for item in &text.overlay_quads {
            self.text
                .encode_quads_timed(
                    device,
                    &mut encoder,
                    QuadDraw {
                        target: &overlay_target,
                        depth: None,
                        atlas: item.atlas,
                        quads: item.quads,
                        space: item.space,
                        deformers: None,
                        icons: item.icons,
                    },
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        if !text.overlay.is_empty() || !text.overlay_quads.is_empty() {
            self.pass_log.push("overlay text");
        }
        let ids = &self.frame.targets.ids_view;
        if glass::drawn(effects.glass) && self.glass.has_ids() {
            self.glass
                .encode_ids(&mut encoder, ids, Some(&mut self.profiler));
            self.pass_log.push("glass ids");
        }
        let lit = self.frame.lit_bindings();
        let lit_for = |space: &TextSpace| {
            if space.is_lit() {
                lit.as_ref()
                    .map(Some)
                    .ok_or("frame lighting is not prepared")
            } else {
                Ok(None)
            }
        };
        for item in text.surface.iter().filter(|item| item.id != 0) {
            self.text
                .encode_ids(
                    device,
                    &mut encoder,
                    TextDraw {
                        target: ids,
                        depth: Some(&self.frame.targets.depth_view),
                        atlas: item.atlas,
                        paragraph: item.paragraph,
                        space: item.space,
                        color: item.color,
                        deformers: Some(&deformers),
                    },
                    lit_for(&item.space)?,
                    item.id,
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        for item in text.surface_quads.iter().filter(|item| item.id != 0) {
            self.text
                .encode_quad_ids(
                    device,
                    &mut encoder,
                    QuadDraw {
                        target: ids,
                        depth: Some(&self.frame.targets.depth_view),
                        atlas: item.atlas,
                        quads: item.quads,
                        space: item.space,
                        deformers: Some(&deformers),
                        icons: item.icons,
                    },
                    lit_for(&item.space)?,
                    item.id,
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        for item in text.overlay.iter().filter(|item| item.id != 0) {
            self.text
                .encode_ids(
                    device,
                    &mut encoder,
                    TextDraw {
                        target: ids,
                        depth: None,
                        atlas: item.atlas,
                        paragraph: item.paragraph,
                        space: item.space,
                        color: item.color,
                        deformers: None,
                    },
                    None,
                    item.id,
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        for item in text.overlay_quads.iter().filter(|item| item.id != 0) {
            self.text
                .encode_quad_ids(
                    device,
                    &mut encoder,
                    QuadDraw {
                        target: ids,
                        depth: None,
                        atlas: item.atlas,
                        quads: item.quads,
                        space: item.space,
                        deformers: None,
                        icons: item.icons,
                    },
                    None,
                    item.id,
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        if text.surface.iter().any(|item| item.id != 0)
            || text.surface_quads.iter().any(|item| item.id != 0)
            || text.overlay.iter().any(|item| item.id != 0)
            || text.overlay_quads.iter().any(|item| item.id != 0)
        {
            self.pass_log.push("text ids");
        }
        if let Some(hud) = hud {
            let mut flat = self
                .flat
                .take()
                .unwrap_or_else(|| FlatPass::new(device, &queue));
            let (capacity_width, capacity_height) = self.capacity();
            flat.set_capacity(Some([capacity_width, capacity_height]));
            let prepared = flat.prepare(device, &queue, hud, [width, height]);
            if let Err(error) = prepared {
                self.flat = Some(flat);
                return Err(error);
            }
            let target = flat.colour_target(device, [capacity_width, capacity_height]);
            flat.encode_colour(
                &mut encoder,
                &target,
                Some([0.0; 4]),
                Some(&mut self.profiler),
            );
            flat.encode_finish(
                device,
                &mut encoder,
                &target,
                &overlay_target,
                wgpu::TextureFormat::Rgba16Float,
                self.display.is_some(),
                true,
                Some(&mut self.profiler),
            );
            flat.encode_ids(&mut encoder, ids, false, Some(&mut self.profiler));
            self.flat = Some(flat);
            self.pass_log.extend(["flat", "flat finish", "flat ids"]);
        }
        let source = post_output.clone();
        if cached && overlays {
            if let Some(cache) = self.demand.as_mut() {
                cache.encode_composite(
                    device,
                    &mut encoder,
                    &source,
                    (output, self.output_format),
                    &mut self.profiler,
                );
                self.pass_log.push("composite");
            }
        } else if self.blit.is_some() {
            self.encode_blit(device, &mut encoder, &source, output);
        }
        self.finish_encoder(device, &queue, encoder, true)
    }

    fn encode_blit(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        post_output: &wgpu::TextureView,
        output: &wgpu::TextureView,
    ) {
        if let Some((layout, pipeline)) = &self.blit {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("live output blit"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(post_output),
                }],
            });
            let timing = self.profiler.pass("output blit");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live output blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: self.profiler.render_writes(timing),
                occlusion_query_set: None,
            });
            let [width, height] = self.frame.viewport();
            crate::viewport::apply(&mut pass, [width, height]);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            self.pass_log.push("output blit");
        }
    }

    fn finish_encoder(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        depth: bool,
    ) -> Result<u64, String> {
        let pick_readback = if self.pick_readback.is_none() {
            self.pick_request.take().map(|(x, y)| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("live pick readback"),
                    size: 512,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                for (texture, offset) in [
                    (&self.frame.targets.ids, 0),
                    (&self.depths[self.depth_index].0, 256),
                ]
                .into_iter()
                .take(if depth { 2 } else { 1 })
                {
                    encoder.copy_texture_to_buffer(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x, y, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyBufferInfo {
                            buffer: &buffer,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset,
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
                }
                (buffer, x, y)
            })
        } else {
            None
        };
        let number = self.frames_submitted;
        let _ = device.poll(wgpu::PollType::Poll);
        self.gather_timings(number);
        let slot = self.profiler.finish_frame(&mut encoder, number);
        let index = queue.submit(Some(encoder.finish()));
        let (width, height) = self.size();
        self.stats.submitted(number, [width, height]);
        if std::mem::take(&mut self.resolution_frame)
            && let Some(resolution) = self.resolution.as_mut()
        {
            let rendered = self
                .resolution_content
                .map_or(resolution.scale(), |content| {
                    let area = f64::from(width) * f64::from(height);
                    let full = f64::from(content[0]) * f64::from(content[1]);
                    (area / full).sqrt() as f32
                });
            resolution.begin_at(number, rendered);
        }
        self.frames_submitted += 1;
        let done = self.frames_done.clone();
        queue.on_submitted_work_done(move || {
            done.fetch_max(number + 1, Ordering::AcqRel);
        });
        self.submissions.push_back((number, index));
        if matches!(self.exposure_mode, Exposure::Auto { .. }) {
            self.exposure.submitted();
        }
        if let Some((buffer, x, y)) = pick_readback {
            let (sender, receiver) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            self.pick_readback = Some(PickReadback {
                buffer,
                receiver,
                x,
                y,
            });
        }
        if let Some(slot) = slot {
            self.profiler.submitted(slot);
        }
        let done = self.frames_done.load(Ordering::Acquire);
        while self
            .submissions
            .front()
            .is_some_and(|(frame, _)| *frame < done)
        {
            self.submissions.pop_front();
        }
        Ok(number)
    }
}

impl Renderer {
    fn touch(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_frames_on_demand(&mut self, limits: Option<Limits>) -> Result<(), String> {
        let Some(limits) = limits else {
            self.demand = None;
            return Ok(());
        };
        if self.display.is_none() {
            return Err("frames on demand need an sRGB output format".into());
        }
        if self.size() != self.capacity() {
            return Err("frames on demand render at the full size; set the viewport to the renderer's capacity first".into());
        }
        match &mut self.demand {
            Some(cache) => cache.limits = limits,
            None => {
                let (width, height) = self.size();
                self.demand = Some(demand::Cache::new(
                    &self.frame.gpu.device,
                    [width, height],
                    limits,
                ));
            }
        }
        Ok(())
    }

    pub fn frames_on_demand(&self) -> Option<Limits> {
        self.demand.as_ref().map(|cache| cache.limits)
    }

    pub fn demand_stats(&self) -> Option<DemandStats> {
        self.demand.as_ref().map(demand::Cache::stats)
    }

    pub fn frame_needed(&self, next: &Next<'_>) -> Need {
        let Some(cache) = &self.demand else {
            return Need::Full;
        };
        demand::decide(&cache.book, &self.summarize(next), &cache.limits)
    }

    pub fn submit_needed(
        &mut self,
        next: &Next<'_>,
        need: &Need,
        output: &wgpu::TextureView,
    ) -> Result<Option<Submitted>, String> {
        let Some(frame) = self.encode_needed(next, need, output)? else {
            return Ok(None);
        };
        Ok(Some(self.submitted_frame(frame)))
    }

    pub fn render_needed(
        &mut self,
        next: &Next<'_>,
        need: &Need,
        output: &wgpu::TextureView,
    ) -> Result<Option<Vec<PassTiming>>, String> {
        let Some(frame) = self.encode_needed(next, need, output)? else {
            return Ok(None);
        };
        self.wait_for(frame).map(Some)
    }

    pub fn present_last(&mut self, output: &wgpu::TextureView) -> Result<Submitted, String> {
        let cache = self.demand.as_ref().ok_or("frames on demand are off")?;
        cache.cached.as_ref().ok_or("no frame has been drawn")?;
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        let queue = self.frame.gpu.queue.clone();
        self.begin_frame(FrameKind::Idle);
        self.pass_log.clear();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live present"),
        });
        self.encode_present(device, &mut encoder, output)?;
        let frame = self.finish_encoder(device, &queue, encoder, false)?;
        if let Some(cache) = &mut self.demand {
            cache.submitted();
        }
        Ok(self.submitted_frame(frame))
    }

    fn encode_present(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
    ) -> Result<(), String> {
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        let cached = cache.cached.as_ref().ok_or("no frame has been drawn")?;
        let base = match cache.last {
            demand::Last::Warp(uniform) => {
                let depth = self.depths[cached.depth_index].1.clone();
                let colour = cache.colour.view.clone();
                cache.encode_warp(
                    device,
                    encoder,
                    &uniform,
                    &colour,
                    &depth,
                    (output, self.output_format),
                    false,
                    &mut self.profiler,
                );
                self.pass_log.push("reproject");
                return Ok(());
            }
            demand::Last::Colour => cache.colour.view.clone(),
            demand::Last::Scratch => cache.scratch.view.clone(),
        };
        if cache.overlaid {
            cache.encode_composite(
                device,
                encoder,
                &base,
                (output, self.output_format),
                &mut self.profiler,
            );
            self.pass_log.push("composite");
        } else {
            self.encode_blit(device, encoder, &base, output);
        }
        Ok(())
    }

    fn refresh_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        next: &Next<'_>,
        print: u64,
    ) -> Result<(), String> {
        let overlays = !next.text.overlay.is_empty()
            || !next.text.overlay_quads.is_empty()
            || next.hud.is_some();
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        cache.overlaid = overlays;
        if !overlays {
            cache.overlay_print = None;
            return Ok(());
        }
        if cache.overlay_print == Some(print) {
            return Ok(());
        }
        cache.encode_clear(encoder);
        let target = cache.overlay.view.clone();
        cache.overlay_print = None;
        self.encode_overlay(device, queue, encoder, next, &target)?;
        if let Some(cache) = &mut self.demand {
            cache.overlay_print = Some(print);
        }
        Ok(())
    }

    pub fn wake(&mut self, groups: u32) -> Result<Submitted, String> {
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        let queue = self.frame.gpu.queue.clone();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live wake"),
        });
        cache.encode_wake(device, &mut encoder, groups, &mut self.profiler);
        self.stats.discard();
        self.resolution_frame = false;
        self.pass_log.clear();
        self.pass_log.push("wake");
        let frame = self.finish_encoder(device, &queue, encoder, false)?;
        Ok(self.submitted_frame(frame))
    }

    pub fn submit_bridge(
        &mut self,
        next: &Next<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Option<Submitted>, String> {
        let Some(frame) = self.encode_bridge(next, output)? else {
            return Ok(None);
        };
        Ok(Some(self.submitted_frame(frame)))
    }

    pub fn render_bridge(
        &mut self,
        next: &Next<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Option<Vec<PassTiming>>, String> {
        let Some(frame) = self.encode_bridge(next, output)? else {
            return Ok(None);
        };
        self.wait_for(frame).map(Some)
    }

    fn encode_bridge(
        &mut self,
        next: &Next<'_>,
        output: &wgpu::TextureView,
    ) -> Result<Option<u64>, String> {
        let Some(cache) = &self.demand else {
            return Ok(None);
        };
        let Some(cached) = &cache.cached else {
            return Ok(None);
        };
        let camera = next.scene.camera;
        if demand::motion(
            (&cached.view, &cached.projection, [0.0; 3]),
            (&camera.view, &camera.projection, [0.0; 3]),
            None,
        )
        .is_none()
        {
            return Ok(None);
        }
        let summary = self.summarize(next);
        self.begin_frame(FrameKind::Reproject);
        let frame = self.encode_reproject(next, &summary, output)?;
        if let Some(cache) = &mut self.demand {
            cache.submitted();
        }
        Ok(Some(frame))
    }

    fn layer_reach(&self, style: Finish) -> Option<u32> {
        if self.lens.is_some() {
            return None;
        }
        if (self.custom_finish || style == self.style)
            && let Some(reach) = self.post.region_reach()
        {
            return Some(reach);
        }
        self.custom_local
            .unwrap_or_else(|| demand::local_post(&chain(style, false)))
            .then_some(0)
    }

    fn summarize(&self, next: &Next<'_>) -> demand::Summary {
        let (width, height) = self.size();
        let camera = next.scene.camera;
        let view_projection = frame::multiply(camera.projection, camera.view);
        let mut world = Print::new();
        world.wide(self.revision.load(Ordering::Relaxed));
        world.wide(next.content);
        world.debug(&next.style);
        demand::print_scene(&mut world, next.scene);
        demand::print_effects(&mut world, next.effects);
        let mut overlay = Print::new();
        overlay.wide(next.overlay);
        overlay.word(u32::from(next.hud.is_some()));
        let mut anchors = Vec::new();
        demand::print_text(
            &mut world,
            &mut overlay,
            &mut anchors,
            next.text,
            &view_projection,
        );
        let auto = matches!(self.exposure_mode, Exposure::Auto { .. });
        let ticking = (next.effects.creature_mesh.is_some() && !next.effects.creatures.is_empty())
            || next.effects.liquid.is_some()
            || self.frame.has_tree()
            || next.scene.wind.current.strength > 0.0
            || self.tape.is_some_and(|tape| !tape.is_off())
            || self.sky_settling()
            || (auto
                && self
                    .demand
                    .as_ref()
                    .is_some_and(|cache| cache.exposure_moving));
        let reach = self.layer_reach(next.style);
        let whole = Region::whole(width, height);
        let mut layered = Vec::new();
        if let Some(volume) = &next.effects.volume
            && let Some(region) = demand::box_region(
                &view_projection,
                volume.lo,
                volume.hi,
                8 + demand::MARGIN + reach.unwrap_or(0),
                width,
                height,
            )
        {
            layered.push(region);
        }
        if let Some(region) = demand::particle_region(
            &view_projection,
            next.effects.particles,
            demand::MARGIN + reach.unwrap_or(0),
            width,
            height,
        ) {
            layered.push(region);
        }
        if reach.is_none() && !layered.is_empty() {
            layered = vec![whole];
        }
        demand::Summary {
            view: camera.view,
            projection: camera.projection,
            position: camera.position,
            size: [width, height],
            world: world.finish(),
            anchors,
            overlay: overlay.finish(),
            ticking,
            layered: demand::merge(&layered, width, height),
            moving: next
                .moving
                .iter()
                .map(|region| region.clip(width, height))
                .filter(|region| !region.is_empty())
                .collect(),
        }
    }

    fn encode_needed(
        &mut self,
        next: &Next<'_>,
        need: &Need,
        output: &wgpu::TextureView,
    ) -> Result<Option<u64>, String> {
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        let _ = self.frame.gpu.device.poll(wgpu::PollType::Poll);
        cache.gather();
        let need = if cache.cached.is_none() && need.draws() {
            &Need::Full
        } else {
            need
        };
        let summary = self.summarize(next);
        let (frame, kind) = match need {
            Need::None => return Ok(None),
            Need::Full => {
                self.begin_frame(FrameKind::Full);
                (
                    self.encode_frame(
                        next.scene,
                        next.text,
                        next.effects,
                        next.style,
                        next.hud,
                        output,
                        true,
                    )?,
                    Shown::Full,
                )
            }
            Need::Reproject => {
                self.begin_frame(FrameKind::Reproject);
                (
                    self.encode_reproject(next, &summary, output)?,
                    Shown::Reproject,
                )
            }
            Need::Partial(regions) => {
                self.begin_frame(FrameKind::Partial);
                (
                    self.encode_partial(next, &summary, regions, output)?,
                    Shown::Partial,
                )
            }
        };
        let summary = if kind == Shown::Full {
            self.summarize(next)
        } else {
            summary
        };
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        cache.submitted();
        if kind == Shown::Full && cache.overlaid {
            cache.overlay_print = Some(summary.overlay);
        }
        let limits = cache.limits;
        demand::record(&mut cache.book, summary, kind, &limits);
        Ok(Some(frame))
    }

    fn layer_frame(&self, scene: &Scene<'_>, view_projection: Matrix) -> EffectFrame {
        let (width, height) = self.size();
        let view = camera_view(scene, width, height);
        let camera = scene.camera;
        let right = [camera.view[0][0], camera.view[1][0], camera.view[2][0]];
        let mut frame = EffectFrame::new(width, height, scene.time, scene.seed);
        frame.view_proj = view_projection;
        frame.eye = [
            camera.position[0],
            camera.position[1],
            camera.position[2],
            0.0,
        ];
        frame.right = [right[0], right[1], right[2], 0.0];
        frame.up = [view.up[0], view.up[1], view.up[2], 0.0];
        frame.forward = [view.forward[0], view.forward[1], view.forward[2], 0.0];
        frame.lens = [
            1.0 / camera.projection[0][0],
            1.0 / camera.projection[1][1],
            0.0,
            0.0,
        ];
        frame.sun = [
            scene.sun.direction[0],
            scene.sun.direction[1],
            scene.sun.direction[2],
            0.0,
        ];
        frame.sun_color = [
            scene.sun.colour[0],
            scene.sun.colour[1],
            scene.sun.colour[2],
            scene.sun.intensity,
        ];
        frame
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_layer_post(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        next: &Next<'_>,
        depth: &wgpu::TextureView,
        target: &wgpu::TextureView,
        rects: Option<&[Region]>,
    ) -> Result<(), String> {
        let cache = self.demand.as_ref().ok_or("frames on demand are off")?;
        let cached = cache.cached.as_ref().ok_or("no frame has been drawn")?;
        let camera = next.scene.camera;
        let view_projection = frame::multiply(camera.projection, camera.view);
        let layer_frame = self.layer_frame(next.scene, view_projection);
        let layer = cache.layer.view.clone();
        let shadow_data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("demand sun cascades"),
            contents: &cached.shadow,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (frame_number, exposure) = (cached.frame_number, cached.exposure);
        draw_layers(
            device,
            encoder,
            &mut self.effects,
            &mut self.profiler,
            next.effects,
            &layer_frame,
            EffectTargets {
                hdr: &layer,
                depth,
                shadow: Some((self.shadows.atlas_view(), &shadow_data)),
            },
            &mut self.pass_log,
        );
        let source = if let Some(lens) = self.lens {
            let timing = self.profiler.pass("depth of field");
            self.pass_log.push("depth of field");
            self.dof.render(
                encoder,
                &layer,
                depth,
                lens,
                dof::height_plane(camera.view, camera.projection),
                self.profiler.compute_writes(timing),
            )
        } else {
            &layer
        };
        match rects {
            Some(rects) => {
                let rects: Vec<[u32; 4]> = rects
                    .iter()
                    .map(|r| [r.x, r.y, r.x + r.width, r.y + r.height])
                    .collect();
                self.post.run_region(
                    encoder,
                    source,
                    Some(&self.frame.targets.depth_view),
                    Some(&self.frame.targets.normal_roughness_view),
                    target,
                    frame_number,
                    next.scene.seed,
                    exposure,
                    &rects,
                    Some(&mut self.profiler),
                );
            }
            None => self.post.run_timed_exposed(
                encoder,
                source,
                Some(&self.frame.targets.depth_view),
                Some(&self.frame.targets.normal_roughness_view),
                target,
                frame_number,
                next.scene.seed,
                exposure,
                Some(&mut self.profiler),
            ),
        }
        self.pass_log.push("post");
        Ok(())
    }

    fn encode_overlay(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        next: &Next<'_>,
        target: &wgpu::TextureView,
    ) -> Result<(), String> {
        let (width, height) = self.size();
        let text = next.text;
        for item in &text.overlay {
            self.text
                .encode_timed(
                    device,
                    encoder,
                    TextDraw {
                        target,
                        depth: None,
                        atlas: item.atlas,
                        paragraph: item.paragraph,
                        space: item.space,
                        color: item.color,
                        deformers: None,
                    },
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        for item in &text.overlay_quads {
            self.text
                .encode_quads_timed(
                    device,
                    encoder,
                    QuadDraw {
                        target,
                        depth: None,
                        atlas: item.atlas,
                        quads: item.quads,
                        space: item.space,
                        deformers: None,
                        icons: item.icons,
                    },
                    Some(&mut self.profiler),
                )
                .map_err(str::to_owned)?;
        }
        if !text.overlay.is_empty() || !text.overlay_quads.is_empty() {
            self.pass_log.push("overlay text");
        }
        if let Some(hud) = next.hud {
            let mut flat = self
                .flat
                .take()
                .unwrap_or_else(|| FlatPass::new(device, queue));
            let prepared = flat.prepare(device, queue, hud, [width, height]);
            if let Err(error) = prepared {
                self.flat = Some(flat);
                return Err(error);
            }
            let colour = flat.colour_target(device, [width, height]);
            flat.encode_colour(encoder, &colour, Some([0.0; 4]), Some(&mut self.profiler));
            flat.encode_finish(
                device,
                encoder,
                &colour,
                target,
                wgpu::TextureFormat::Rgba16Float,
                self.display.is_some(),
                true,
                Some(&mut self.profiler),
            );
            self.flat = Some(flat);
            self.pass_log.extend(["flat", "flat finish"]);
        }
        Ok(())
    }

    fn encode_reproject(
        &mut self,
        next: &Next<'_>,
        summary: &demand::Summary,
        output: &wgpu::TextureView,
    ) -> Result<u64, String> {
        self.pass_log.clear();
        let (width, height) = self.size();
        self.text_bounds = text_bounds(next.text, [width, height]);
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        let queue = self.frame.gpu.queue.clone();
        let camera = next.scene.camera;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live reproject"),
        });
        self.refresh_overlay(device, &queue, &mut encoder, next, summary.overlay)?;
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        let cached = cache.cached.as_ref().ok_or("no frame has been drawn")?;
        let mut uniform = demand::WarpUniform::new(
            (&cached.view, &cached.projection),
            (&camera.view, &camera.projection),
            [width, height],
            cached.far,
        )
        .ok_or("camera projection is singular")?;
        let depth = self.depths[cached.depth_index].1.clone();
        if !summary.layered.is_empty() || cache.colour_layers {
            let still = cache.still.view.clone();
            let layer = cache.layer.view.clone();
            let scratch = cache.scratch.view.clone();
            cache.encode_warp(
                device,
                &mut encoder,
                &uniform,
                &still,
                &depth,
                (&layer, wgpu::TextureFormat::Rgba16Float),
                true,
                &mut self.profiler,
            );
            cache.last = demand::Last::Scratch;
            let warped = cache.depth.view.clone();
            self.pass_log.push("reproject");
            self.encode_layer_post(device, &mut encoder, next, &warped, &scratch, None)?;
            self.encode_present(device, &mut encoder, output)?;
        } else {
            uniform.limits[3] = if cache.overlaid { 1.0 } else { 0.0 };
            cache.last = demand::Last::Warp(uniform);
            self.encode_present(device, &mut encoder, output)?;
        }
        self.finish_encoder(device, &queue, encoder, false)
    }

    fn encode_partial(
        &mut self,
        next: &Next<'_>,
        summary: &demand::Summary,
        regions: &[Region],
        output: &wgpu::TextureView,
    ) -> Result<u64, String> {
        self.pass_log.clear();
        let (width, height) = self.size();
        self.text_bounds = text_bounds(next.text, [width, height]);
        let owned_device = self.frame.gpu.device.clone();
        let device = &owned_device;
        let queue = self.frame.gpu.queue.clone();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live partial"),
        });
        let cache = self.demand.as_mut().ok_or("frames on demand are off")?;
        let cached = cache.cached.as_ref().ok_or("no frame has been drawn")?;
        let depth = self.depths[cached.depth_index].1.clone();
        let mut layered = summary.layered.clone();
        if let Some(shown) = &cache.book.shown {
            layered.extend(shown.layered.iter().copied());
        }
        let layered: Vec<Region> = demand::merge(&layered, width, height)
            .into_iter()
            .filter(|region| regions.iter().any(|outer| outer.touches(region, 0)))
            .collect();
        let colour = cache.colour.view.clone();
        cache.last = demand::Last::Colour;
        if !layered.is_empty() {
            let still = cache.still.view.clone();
            let layer = cache.layer.view.clone();
            let scratch = cache.scratch.view.clone();
            cache.encode_copy(
                device,
                &mut encoder,
                &still,
                &layer,
                &layered,
                "partial still",
                &mut self.profiler,
            );
            cache.colour_layers = !summary.layered.is_empty() || cache.colour_layers;
            self.pass_log.push("partial still");
            self.encode_layer_post(device, &mut encoder, next, &depth, &scratch, Some(&layered))?;
            let cache = self.demand.as_ref().ok_or("frames on demand are off")?;
            cache.encode_copy(
                device,
                &mut encoder,
                &scratch,
                &colour,
                &layered,
                "partial colour",
                &mut self.profiler,
            );
            self.pass_log.push("partial colour");
        }
        self.refresh_overlay(device, &queue, &mut encoder, next, summary.overlay)?;
        self.encode_present(device, &mut encoder, output)?;
        self.finish_encoder(device, &queue, encoder, false)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_layers(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    pass: &mut effects::Effects,
    profiler: &mut GpuProfiler,
    effects: &Effects<'_>,
    frame: &EffectFrame,
    targets: EffectTargets<'_>,
    log: &mut Vec<&'static str>,
) {
    if let Some(volume) = &effects.volume {
        let mut volume_frame = *frame;
        volume_frame.set_volume(
            volume.kind,
            volume.lo,
            volume.hi,
            volume.color,
            volume.density,
            volume.anisotropy,
        );
        volume_frame.scale_time(volume.time_scale);
        let march = profiler.pass("volume march");
        let upsample = profiler.pass("volume upsample");
        pass.render_volume(
            &mut EffectCommands { device, encoder },
            &volume_frame,
            targets,
            VolumeTimestamps {
                march: profiler.render_writes(march),
                upsample: profiler.render_writes(upsample),
            },
        );
        log.extend(["volume march", "volume upsample"]);
    }
    if !effects.particles.is_empty() {
        let timing = profiler.pass("particles");
        pass.render_particles(
            &mut EffectCommands { device, encoder },
            frame,
            targets,
            effects.particles,
            profiler.render_writes(timing),
        );
        log.push("particles");
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_surface_text(
    pass: &mut TextPass,
    frame: &Frame,
    profiler: &mut GpuProfiler,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    text: &Text<'_>,
    deformers: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) -> Result<(), String> {
    for item in &text.surface {
        let draw = TextDraw {
            target,
            depth: Some(&frame.targets.depth_view),
            atlas: item.atlas,
            paragraph: item.paragraph,
            space: item.space,
            color: item.color,
            deformers: Some(deformers),
        };
        if item.space.is_lit() {
            let lit = frame
                .lit_bindings()
                .ok_or("frame lighting is not prepared")?;
            pass.encode_lit_timed(device, encoder, draw, &lit, Some(&mut *profiler))
        } else {
            pass.encode_timed(device, encoder, draw, Some(&mut *profiler))
        }
        .map_err(str::to_owned)?;
    }
    for item in &text.surface_quads {
        let draw = QuadDraw {
            target,
            depth: Some(&frame.targets.depth_view),
            atlas: item.atlas,
            quads: item.quads,
            space: item.space,
            deformers: Some(deformers),
            icons: item.icons,
        };
        if item.space.is_lit() {
            let lit = frame
                .lit_bindings()
                .ok_or("frame lighting is not prepared")?;
            pass.encode_quads_lit_timed(device, encoder, draw, &lit, Some(&mut *profiler))
        } else {
            pass.encode_quads_timed(device, encoder, draw, Some(&mut *profiler))
        }
        .map_err(str::to_owned)?;
    }
    Ok(())
}

fn text_bounds(text: &Text<'_>, size: [u32; 2]) -> Vec<TextBounds> {
    let paragraphs = |list: TextList, items: &[TextItem<'_>]| {
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                crate::text::screen_bounds(
                    &item.space,
                    item.paragraph.glyphs.iter().map(PackedGlyph::from),
                    size,
                )
                .map(|rect| TextBounds {
                    list,
                    index,
                    id: item.id,
                    rect,
                })
            })
            .collect::<Vec<_>>()
    };
    let quads = |list: TextList, items: &[TextQuadItem<'_>]| {
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                crate::text::screen_bounds(
                    &item.space,
                    item.quads.iter().map(PackedGlyph::from),
                    size,
                )
                .map(|rect| TextBounds {
                    list,
                    index,
                    id: item.id,
                    rect,
                })
            })
            .collect::<Vec<_>>()
    };
    let mut bounds = paragraphs(TextList::Surface, &text.surface);
    bounds.extend(quads(TextList::SurfaceQuads, &text.surface_quads));
    bounds.extend(paragraphs(TextList::Overlay, &text.overlay));
    bounds.extend(quads(TextList::OverlayQuads, &text.overlay_quads));
    bounds
}

fn readback_texture(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
) -> Result<Vec<u8>, String> {
    let raw_row = width * bytes_per_pixel;
    let row =
        raw_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("live geometry readback"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("live geometry readback"),
        });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
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
    let (sender, receiver) = std::sync::mpsc::channel();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| format!("{error:?}"))?;
    receiver
        .recv()
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let mapped = slice.get_mapped_range();
    let mut bytes = Vec::with_capacity((raw_row * height) as usize);
    for padded in mapped.chunks_exact(row as usize) {
        bytes.extend_from_slice(&padded[..raw_row as usize]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Camera, SceneWind, Sun};
    use crate::sky::AnalyticSky;
    use pfx_geom::shapes::Shape;
    use pfx_gpu::OffscreenTarget;
    use pfx_materials::Material;

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
        let f = 1.0 / (40.0_f32.to_radians() * 0.5).tan();
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ]
    }

    #[test]
    fn fixed_and_biased_exposure() {
        assert_eq!(Exposure::Fixed(0.93).value(4.0), 0.93);
        assert_eq!(Exposure::Auto { bias: 1.0 }.value(0.75), 1.5);
        assert_eq!(Exposure::Auto { bias: -1.0 }.value(0.75), 0.375);
    }

    #[test]
    fn plain_passes_merge_post_and_keep_sixteen() {
        let mut passes = (0..16)
            .map(|index| PassTiming {
                label: format!("pass {index}"),
                milliseconds: index as f64,
            })
            .collect::<Vec<_>>();
        passes.extend(
            ["post 0:0", "post 1:0", "post 2:0", "post 3:0"].map(|label| PassTiming {
                label: label.to_owned(),
                milliseconds: 5.0,
            }),
        );
        passes.push(PassTiming {
            label: "volume march".into(),
            milliseconds: 0.2,
        });
        passes.push(PassTiming {
            label: "volume upsample".into(),
            milliseconds: 0.3,
        });
        let merged = plain_passes(passes);
        assert_eq!(merged.len(), 16);
        assert_eq!(merged[0].label, "post");
        assert_eq!(merged[0].milliseconds, 20.0);
        assert!(merged.iter().all(|pass| !pass.label.starts_with("post ")));
        assert!(!merged.iter().any(|pass| pass.label == "pass 0"));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn caller_finish_matches_gpu_chain() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, 32, 32).unwrap();
        let mut finish = Chain::new();
        finish.passes = vec![
            PostPass::Exposure(1.25),
            PostPass::Bloom(Bloom::neutral(0.2)),
            PostPass::Tone(Tone::aces()),
        ];
        renderer.set_finish(finish.clone());
        let mut expected = finish;
        expected.passes.insert(0, PostPass::Exposure(1.0));
        expected.passes.push(PostPass::Encode);
        let direct = GpuChain::new(expected, &renderer.gpu().device, 32, 32);
        let input = output(renderer.gpu(), 32, 32);
        let actual = output(renderer.gpu(), 32, 32);
        let reference = output(renderer.gpu(), 32, 32);
        let mut encoder = renderer
            .gpu()
            .device
            .create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("finish input"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &input.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.5,
                            g: 0.7,
                            b: 0.2,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        renderer
            .post
            .run(&mut encoder, &input.view, None, None, &actual.view, 7, 9);
        direct.run(&mut encoder, &input.view, None, None, &reference.view, 7, 9);
        renderer.gpu().queue.submit(Some(encoder.finish()));
        assert_eq!(
            renderer.gpu().readback_rgba16(&actual).unwrap(),
            renderer.gpu().readback_rgba16(&reference).unwrap()
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn set_tape_overrides_the_finish_until_cleared() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, 32, 32).unwrap();
        assert!(renderer.post.tape().is_off());
        let mut finish = Chain::new();
        finish.passes.push(PostPass::Tone(Tone::aces()));
        finish.set_tape(Tape::forward(0.5));
        renderer.set_finish(finish);
        assert_eq!(renderer.post.tape(), Tape::forward(0.5));
        renderer.set_tape(Some(Tape::rewind(1.0)));
        assert_eq!(renderer.post.tape(), Tape::rewind(1.0));
        renderer.set_tape(None);
        assert_eq!(renderer.post.tape(), Tape::forward(0.5));
        renderer.clear_finish();
        renderer.set_tape(Some(Tape::rewind(0.3)));
        assert_eq!(renderer.post.tape(), Tape::rewind(0.3));
        renderer.set_tape(None);
        assert!(renderer.post.tape().is_off());
    }

    fn sphere_still(size: u32) -> (Renderer, OffscreenTarget, Vec<Instance>, Vec<Material>) {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, size, size).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let shape = Shape::Ellipsoid {
            radius: 0.7,
            squash: 1.0,
        }
        .mesh(0.05);
        let mesh = renderer
            .upload_mesh(MeshData {
                positions: &shape.positions,
                normals: &shape.normals,
                tangents: &shape.tangents,
                uvs: &shape.uvs,
                uvs1: None,
                alpha: None,
                indices: &shape.indices,
            })
            .unwrap();
        let target = output(renderer.gpu(), size, size);
        let materials = vec![Material {
            base: [0.9; 3],
            metalness: 1.0,
            roughness: 0.3,
            ..Default::default()
        }];
        (
            renderer,
            target,
            vec![Instance::new(mesh, identity(), 0, 5)],
            materials,
        )
    }

    fn still_scene<'a>(
        instances: &'a [Instance],
        materials: &'a [Material],
        number: u32,
    ) -> Scene<'a> {
        let shift = |n: u32| (n as f32 * 0.05).sin() * 0.3;
        let projection = perspective(1.0);
        let mut view = identity();
        view[3][0] = shift(number);
        view[3][2] = -3.0;
        let mut before = view;
        before[3][0] = shift(number.saturating_sub(1));
        Scene {
            camera: Camera {
                view,
                projection,
                previous_view_projection: frame::multiply(projection, before),
                position: [-view[3][0], 0.0, 3.0],
            },
            time: number as f32 / 60.0,
            seed: 11,
            sun: Sun {
                direction: [0.3, 0.8, 0.5],
                colour: [1.0; 3],
                intensity: 2.0,
            },
            instances,
            materials,
            deformers: &[],
            wind: SceneWind::default(),
        }
    }

    fn varied_still(size: u32) -> (Renderer, OffscreenTarget, Vec<Instance>, Vec<Material>) {
        let (renderer, target, mut instances, materials) = sphere_still(size);
        let sphere = instances[0];
        let mut two_sided = sphere;
        two_sided.model = translated(-1.2, 0.0, 0.0);
        two_sided.previous_model = two_sided.model;
        two_sided.two_sided = true;
        two_sided.id = 6;
        let mut aged = sphere;
        aged.model = translated(1.2, 0.0, 0.0);
        aged.previous_model = aged.model;
        aged.age = 0.6;
        aged.id = 7;
        instances.extend([two_sided, aged]);
        (renderer, target, instances, materials)
    }

    fn sorted(mut keys: Vec<OpaqueKey>) -> Vec<OpaqueKey> {
        keys.sort_by_key(|key| (key.features.flags(), key.lit, key.two_sided));
        keys
    }

    fn draw_frames(
        renderer: &mut Renderer,
        target: &OffscreenTarget,
        instances: &[Instance],
        materials: &[Material],
        frames: u32,
    ) -> Vec<Vec<u16>> {
        (0..frames)
            .map(|number| {
                renderer
                    .render(
                        &still_scene(instances, materials, number),
                        &Text::default(),
                        &Effects::default(),
                        Finish::Standard,
                        &target.view,
                    )
                    .unwrap();
                renderer.gpu().readback_rgba16(target).unwrap()
            })
            .collect()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn warm_makes_the_opaque_pipelines_the_frames_draw_and_the_same_bytes() {
        let (mut warmed, target, instances, materials) = varied_still(64);
        let warming = warmed
            .warm(&[still_scene(&instances, &materials, 0)])
            .unwrap();
        let progress = warming.wait();
        assert!(warming.done() && warming.first_ready());
        assert_eq!(progress.failed, 0);
        assert_eq!(progress.total, 3);
        assert_eq!(progress.first_total, 3);
        assert!(warmed.frame().opaque_pipelines().len() <= 3);
        let warm_frames = draw_frames(&mut warmed, &target, &instances, &materials, 3);
        assert_eq!(warmed.frame().opaque_made_on_render_thread(), 0);
        let (mut cold, cold_target, cold_instances, cold_materials) = varied_still(64);
        let cold_frames = draw_frames(&mut cold, &cold_target, &cold_instances, &cold_materials, 3);
        assert_eq!(cold.frame().opaque_made_on_render_thread(), 3);
        assert_eq!(
            sorted(warmed.frame().opaque_pipelines()),
            sorted(cold.frame().opaque_pipelines())
        );
        for (number, (warm, cold)) in warm_frames.iter().zip(&cold_frames).enumerate() {
            assert!(warm == cold, "frame {number} differs after the warm");
        }
        let again = warmed
            .warm(&[still_scene(&instances, &materials, 3)])
            .unwrap();
        assert_eq!(again.progress().total, 0);
        assert!(again.done());
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn later_scenes_warm_after_the_first_and_a_frame_waits_for_its_own() {
        let (mut renderer, target, instances, materials) = varied_still(64);
        let plain: Vec<Instance> = instances[..1].to_vec();
        renderer.set_warm_workers(1);
        let warming = renderer
            .warm(&[
                still_scene(&plain, &materials, 0),
                still_scene(&instances, &materials, 0),
            ])
            .unwrap();
        let started = warming.progress();
        assert_eq!((started.first_total, started.total), (1, 3));
        draw_frames(&mut renderer, &target, &plain, &materials, 1);
        assert_eq!(renderer.frame().opaque_made_on_render_thread(), 0);
        assert!(warming.first_ready());
        warming.wait();
        draw_frames(&mut renderer, &target, &instances, &materials, 2);
        assert_eq!(renderer.frame().opaque_made_on_render_thread(), 0);
        assert_eq!(renderer.frame().opaque_pipelines().len(), 3);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_first_frame_draws_the_same_bytes_through_the_reflection_pipelines() {
        let ssr = frame::OpaqueFeatures::named("ssr").unwrap();
        let (mut merged, target, instances, materials) = varied_still(64);
        let (mut split, split_target, split_instances, split_materials) = varied_still(64);
        split.frame().set_opaque_off(ssr);
        let first = draw_frames(&mut merged, &target, &instances, &materials, 1);
        let before = draw_frames(
            &mut split,
            &split_target,
            &split_instances,
            &split_materials,
            1,
        );
        assert!(merged.frame().opaque_drawn().iter().all(|f| f.ssr));
        assert!(split.frame().opaque_drawn().iter().all(|f| !f.ssr));
        assert!(first[0] == before[0], "the first frame changed");
        split.frame().set_opaque_off(frame::OpaqueFeatures::NONE);
        let made = merged.frame().opaque_made_on_render_thread();
        for number in 1..3 {
            let scene = still_scene(&instances, &materials, number);
            let split_scene = still_scene(&split_instances, &split_materials, number);
            for (renderer, scene, target) in [
                (&mut merged, &scene, &target),
                (&mut split, &split_scene, &split_target),
            ] {
                renderer
                    .render(
                        scene,
                        &Text::default(),
                        &Effects::default(),
                        Finish::Standard,
                        &target.view,
                    )
                    .unwrap();
            }
            assert!(
                merged.gpu().readback_rgba16(&target).unwrap()
                    == split.gpu().readback_rgba16(&split_target).unwrap(),
                "frame {number} differs"
            );
        }
        assert_eq!(merged.frame().opaque_made_on_render_thread(), made);
    }

    fn cache_path(name: &str) -> std::path::PathBuf {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/pipeline-cache-tests")
            .join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    fn caches(gpu: &Gpu) -> bool {
        gpu.device
            .features()
            .contains(wgpu::Features::PIPELINE_CACHE)
            && crate::pipeline_cache::cache_key(&gpu.info).is_some()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_pipeline_cache_round_trips_where_the_backend_keeps_one() {
        let path = cache_path("round-trip.bin");
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let supported = caches(&gpu);
        let mut renderer = Renderer::new(gpu, 32, 32).unwrap();
        let load = renderer.use_pipeline_cache(&path);
        if !supported {
            assert_eq!(load, CacheLoad::Unsupported);
            assert_eq!(renderer.save_pipeline_cache(), Ok(None));
            return;
        }
        assert_eq!(load, CacheLoad::Fresh);
        renderer.warm_opaque(frame::OpaqueFeatures::NONE, false);
        let saved = renderer.save_pipeline_cache().unwrap().unwrap();
        assert!(saved > 0);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut again = Renderer::new(gpu, 32, 32).unwrap();
        assert_eq!(
            again.use_pipeline_cache(&path),
            CacheLoad::Loaded { bytes: saved }
        );
        again.warm_opaque(frame::OpaqueFeatures::NONE, false);
        let made = again.frame().opaque_made_on_render_thread();
        let pipelines = again.frame().opaque_pipelines().len();
        assert_eq!(made, pipelines);
        assert!(pipelines > 0);
        again.warm_opaque(frame::OpaqueFeatures::NONE, false);
        assert_eq!(again.frame().opaque_made_on_render_thread(), made);
        assert_eq!(again.frame().opaque_pipelines().len(), pipelines);
        let second = again.save_pipeline_cache().unwrap().unwrap();
        assert!(second > 0);
        assert_eq!(again.save_pipeline_cache().unwrap().unwrap(), second);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut third = Renderer::new(gpu, 32, 32).unwrap();
        assert_eq!(
            third.use_pipeline_cache(&path),
            CacheLoad::Loaded { bytes: second }
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn an_invalid_pipeline_cache_is_ignored_and_replaced() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        if !caches(&gpu) {
            return;
        }
        let key = crate::pipeline_cache::cache_key(&gpu.info).unwrap();
        let (mut renderer, target, instances, materials) = sphere_still(32);
        for (name, bytes) in [
            ("garbage.bin", b"not a cache at all".to_vec()),
            (
                "foreign.bin",
                crate::pipeline_cache::wrap(&key.replace("pito-engine", "older engine"), b"x"),
            ),
            ("damaged.bin", {
                let mut bytes = crate::pipeline_cache::wrap(&key, b"pipelines");
                let last = bytes.len() - 1;
                bytes[last] ^= 0x40;
                bytes
            }),
        ] {
            let path = cache_path(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            assert!(
                matches!(renderer.use_pipeline_cache(&path), CacheLoad::Ignored(_)),
                "{name} was not ignored"
            );
        }
        draw_frames(&mut renderer, &target, &instances, &materials, 1);
        let saved = renderer.save_pipeline_cache().unwrap().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/pipeline-cache-tests/damaged.bin");
        assert_eq!(
            renderer.use_pipeline_cache(&path),
            CacheLoad::Loaded { bytes: saved }
        );
        draw_frames(&mut renderer, &target, &instances, &materials, 1);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn submitted_frames_match_blocking_frames_and_timings_arrive_late() {
        let frames = 120;
        let (mut blocking, blocking_target, instances, materials) = sphere_still(96);
        let mut timed = 0;
        for number in 0..frames {
            let timings = blocking
                .render(
                    &still_scene(&instances, &materials, number),
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &blocking_target.view,
                )
                .unwrap();
            if !timings.is_empty() {
                timed += 1;
                assert_eq!(blocking.timings().unwrap().frame, u64::from(number));
                assert_eq!(blocking.timings().unwrap().passes, timings);
            }
        }
        let expected = blocking.gpu().readback_rgba16(&blocking_target).unwrap();
        let (mut renderer, target, instances, materials) = sphere_still(96);
        let mut arrived = std::collections::BTreeMap::new();
        let mut lags = [0u32; 3];
        let mut most_in_flight = 0;
        for number in 0..frames {
            let submitted = renderer
                .submit(
                    &still_scene(&instances, &materials, number),
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &target.view,
                )
                .unwrap();
            assert_eq!(submitted.frame, u64::from(number));
            most_in_flight = most_in_flight.max(renderer.frames_in_flight());
            for timings in submitted.arrived {
                let lag = submitted.frame - timings.frame;
                assert!(
                    lag <= TIMING_LAG,
                    "frame {} arrived {lag} late",
                    timings.frame
                );
                lags[lag as usize] += 1;
                assert!(!timings.passes.is_empty());
                assert!(arrived.insert(timings.frame, timings).is_none());
            }
            if let Some(latest) = renderer.timings() {
                assert_eq!(Some(&latest.frame), arrived.keys().last());
            }
        }
        for timings in renderer.wait_frames(0).unwrap() {
            assert!(u64::from(frames - 1) - timings.frame <= TIMING_LAG);
            assert!(arrived.insert(timings.frame, timings).is_none());
        }
        assert_eq!(renderer.frames_in_flight(), 0);
        let drops = renderer.timing_drops();
        println!(
            "{frames} submitted frames: {} timings arrived (lag 0: {}, 1: {}, 2: {}), dropped {} with the ring full and {} late; at most {most_in_flight} frames in flight; {timed} blocking frames timed",
            arrived.len(),
            lags[0],
            lags[1],
            lags[2],
            drops.ring,
            drops.late
        );
        if timed > 0 {
            assert_eq!(
                arrived.len() as u64 + drops.ring + drops.late,
                u64::from(frames)
            );
            assert_eq!(
                renderer.timings().map(|latest| latest.frame),
                arrived.keys().last().copied()
            );
        }
        let actual = renderer.gpu().readback_rgba16(&target).unwrap();
        assert!(
            actual == expected,
            "submitted frames differ from rendered ones"
        );
        if timed == 0 {
            return;
        }
        let mut paced = Vec::new();
        for number in frames..frames + 60 {
            let submitted = renderer
                .submit(
                    &still_scene(&instances, &materials, number),
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &target.view,
                )
                .unwrap();
            for timings in &submitted.arrived {
                assert!(submitted.frame - timings.frame <= TIMING_LAG);
            }
            paced.extend(submitted.arrived);
            for timings in renderer.wait_frames(2).unwrap() {
                assert!(submitted.frame - timings.frame <= TIMING_LAG);
                paced.push(timings);
            }
            assert!(renderer.frames_in_flight() <= 2);
        }
        paced.extend(renderer.wait_frames(0).unwrap());
        println!(
            "60 frames paced to two in flight: {} timings arrived, drops {:?}",
            paced.len(),
            renderer.timing_drops()
        );
        assert_eq!(
            paced
                .iter()
                .map(|timings| timings.frame)
                .collect::<Vec<_>>(),
            (u64::from(frames)..u64::from(frames) + 60).collect::<Vec<_>>()
        );
        assert_eq!(renderer.timing_drops(), drops);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_scene_without_a_volume_is_the_same_bytes_under_any_volume_cuts() {
        let mut shots = Vec::new();
        for cuts in [
            effects::VolumeCuts::default(),
            effects::VolumeCuts::FULL,
            effects::VolumeCuts {
                steps: 6,
                warp_octaves: 1,
                octaves: 1,
                early_out: true,
            },
        ] {
            let (mut renderer, target, instances, materials) = sphere_still(96);
            renderer.set_volume_cuts(cuts);
            assert_eq!(renderer.volume_cuts(), cuts);
            for number in 0..6 {
                renderer
                    .render(
                        &still_scene(&instances, &materials, number),
                        &Text::default(),
                        &Effects::default(),
                        Finish::Standard,
                        &target.view,
                    )
                    .unwrap();
            }
            assert!(!renderer.last_pass_order().contains(&"volume march"));
            shots.push(renderer.gpu().readback_rgba16(&target).unwrap());
        }
        assert!(shots[0] == shots[1] && shots[1] == shots[2]);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_stepped_sky_swaps_in_whole_and_renders_as_the_settled_one() {
        let old = AnalyticSky::new(
            pfx_core::daylight::Daylight {
                hour: 9.0,
                day: 172.0,
                latitude: 45.0,
                heading: 180.0,
            },
            pfx_core::daylight::REFERENCE_HOUR,
            2.5,
            [0.2, 0.25, 0.2],
        );
        let sky = |hour: f64| {
            SkySource::Analytic(AnalyticSky::new(
                pfx_core::daylight::Daylight {
                    hour,
                    day: 172.0,
                    latitude: 45.0,
                    heading: 180.0,
                },
                pfx_core::daylight::REFERENCE_HOUR,
                2.5,
                [0.2, 0.25, 0.2],
            ))
        };
        let sh_bits = |r: &Renderer| {
            r.frame()
                .sky()
                .lighting
                .sh
                .map(|band| band.map(f32::to_bits))
        };
        let (mut settled, settled_target, instances, materials) = sphere_still(64);
        settled.set_sky(SkySource::Analytic(old), 9.0).unwrap();
        let blocking = std::time::Instant::now();
        settled.set_sky(sky(15.0), 15.0).unwrap();
        let blocking_ms = blocking.elapsed().as_secs_f64() * 1000.0;
        let (mut stepped, stepped_target, _, _) = sphere_still(64);
        stepped.set_sky(SkySource::Analytic(old), 9.0).unwrap();
        let before = sh_bits(&stepped);
        let mut calls = Vec::new();
        let mut last = 0;
        for index in 0..20 {
            let started = std::time::Instant::now();
            last = stepped
                .request_sky(sky(10.0 + index as f64 * 0.25), 10.0 + index as f32 * 0.25)
                .unwrap();
            calls.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let started = std::time::Instant::now();
        let newest = stepped.request_sky(sky(15.0), 15.0).unwrap();
        calls.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(newest > last);
        assert!(stepped.sky_settling());
        let after = sh_bits(&settled);
        let mut number = 0;
        let mut whole = true;
        let settling = std::time::Instant::now();
        while stepped.sky_settling() && number < 600 {
            stepped
                .render(
                    &still_scene(&instances, &materials, number),
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &stepped_target.view,
                )
                .unwrap();
            let now = sh_bits(&stepped);
            whole &= now == before || now == after;
            number += 1;
        }
        if stepped.sky_settling() {
            stepped.wait_sky().unwrap();
        }
        let settled_ms = settling.elapsed().as_secs_f64() * 1000.0;
        assert!(
            whole,
            "a frame showed a sky that was neither the old nor the new one"
        );
        assert_eq!(sh_bits(&stepped), after);
        assert_eq!(stepped.frame().sky().shown(), newest);
        assert_eq!(stepped.hour, 15.0);
        for number in 1000..1008 {
            for (renderer, target) in [
                (&mut settled, &settled_target),
                (&mut stepped, &stepped_target),
            ] {
                renderer
                    .render(
                        &still_scene(&instances, &materials, number),
                        &Text::default(),
                        &Effects::default(),
                        Finish::Standard,
                        &target.view,
                    )
                    .unwrap();
            }
        }
        let mean = calls.iter().sum::<f64>() / calls.len() as f64;
        let most = calls.iter().copied().fold(0.0, f64::max);
        println!(
            "request_sky on the calling thread: mean {mean:.4} ms, max {most:.4} ms over {} calls; the new sky swapped in after {number} frames and {settled_ms:.1} ms; set_sky with settle blocked {blocking_ms:.1} ms",
            calls.len()
        );
        assert!(mean < 1.0);
        assert!(
            settled.gpu().readback_rgba16(&settled_target).unwrap()
                == stepped.gpu().readback_rgba16(&stepped_target).unwrap(),
            "the stepped sky renders differently from the settled one"
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn pick_returns_id_and_depth_after_submission() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, 64, 64).unwrap();
        renderer.shadows = Shadows::new(
            &renderer.gpu().device,
            Quality {
                resolution: 128,
                ..Default::default()
            },
        );
        let shape = Shape::RoundBox {
            half: [0.8; 3],
            radius: 0.05,
        }
        .mesh(0.2);
        let mesh = renderer
            .upload_mesh(MeshData {
                positions: &shape.positions,
                normals: &shape.normals,
                tangents: &shape.tangents,
                uvs: &shape.uvs,
                uvs1: None,
                alpha: None,
                indices: &shape.indices,
            })
            .unwrap();
        let instances = [Instance::new(mesh, identity(), 0, 37)];
        let mut view = identity();
        view[3][2] = -4.0;
        let projection = perspective(1.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: frame::multiply(projection, view),
            position: [0.0, 0.0, 4.0],
        };
        let materials = [Material::default()];
        let target = output(renderer.gpu(), 64, 64);
        renderer.pick(32, 32).unwrap();
        let mut picked = None;
        for frame_number in 0..20 {
            let scene = Scene {
                camera,
                time: frame_number as f32 / 60.0,
                seed: 3,
                sun: Sun {
                    direction: [0.0, 1.0, 0.0],
                    colour: [1.0; 3],
                    intensity: 2.0,
                },
                instances: &instances,
                materials: &materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            renderer
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &target.view,
                )
                .unwrap();
            picked = renderer.picked();
            if picked.is_some() {
                break;
            }
        }
        let picked = picked.expect("pick did not complete within twenty submitted frames");
        assert_eq!((picked.x, picked.y, picked.id), (32, 32, 37));
        assert!(picked.depth.is_finite() && picked.depth > 0.0 && picked.depth < 100.0);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn creature_depth_reaches_atlas_after_renderer_render() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, 64, 64).unwrap();
        renderer.set_shadow_quality(Quality {
            resolution: 64,
            receiver: Some(crate::shadow::ReceiverBox {
                min: [-1.0, -0.1, -1.0],
                max: [1.0, 2.0, 1.0],
            }),
            caster_margin: 2.0,
            ..Quality::default()
        });
        let mesh = CreatureMesh::new(
            &renderer.gpu().device,
            &[
                effects::CreatureVertex {
                    position: [-0.4, 0.0, -0.4, 0.0],
                },
                effects::CreatureVertex {
                    position: [0.4, 0.0, -0.4, 0.0],
                },
                effects::CreatureVertex {
                    position: [0.0, 0.0, 0.4, 0.0],
                },
            ],
            &[0, 1, 2],
        );
        let bird = [CreatureInstance {
            position_scale: [0.0, 1.0, 0.0, 1.0],
            right: [1.0, 0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0, 0.0],
            forward: [0.0, 0.0, 1.0, 0.0],
            motion: [0.0; 4],
        }];
        let mut view = identity();
        view[3][2] = -4.0;
        let projection = perspective(1.0);
        let scene = Scene {
            camera: Camera {
                view,
                projection,
                previous_view_projection: frame::multiply(projection, view),
                position: [0.0, 0.0, 4.0],
            },
            time: 0.0,
            seed: 5,
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                colour: [1.0; 3],
                intensity: 2.0,
            },
            instances: &[],
            materials: &[Material::default()],
            deformers: &[],
            wind: SceneWind::default(),
        };
        let output = output(renderer.gpu(), 64, 64);
        renderer
            .render(
                &scene,
                &Text::default(),
                &Effects {
                    creatures: &bird,
                    creature_mesh: Some(&mesh),
                    ..Default::default()
                },
                Finish::Standard,
                &output.view,
            )
            .unwrap();
        assert!(renderer.last_pass_order().contains(&"creature shadows"));
        let readback = renderer
            .gpu()
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("creature shadow readback"),
                size: 64 * 64 * 4 * crate::shadow::CASCADE_COUNT as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        let mut encoder = renderer
            .gpu()
            .device
            .create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: renderer.shadows().atlas(),
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
                depth_or_array_layers: crate::shadow::CASCADE_COUNT as u32,
            },
        );
        renderer.gpu().queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        renderer
            .gpu()
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        assert!(
            bytes
                .chunks_exact(4)
                .any(|pixel| { f32::from_le_bytes(pixel.try_into().unwrap()) < 0.999 })
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn prop_shadow_reaches_cascade_atlas_after_renderer_render() {
        use pfx_bake::impostor::{PropSpec, bake_prop, bounds};
        use pfx_bake::{Anchor, BakeScene};
        use pfx_load::Sky;

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let shape = Shape::RoundBox {
            half: [0.5; 3],
            radius: 0.04,
        }
        .mesh(0.03);
        let triangles = triangles(&shape, [0.0; 3], 0);
        let bounds = bounds(&triangles, &[]).unwrap();
        let sky = Sky {
            width: 4,
            height: 2,
            texels: vec![[0.5; 4]; 8],
        };
        let baked = BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: sky.clone(),
                sun: pfx_trace::Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 2.0,
                },
            }],
        };
        let artifact = bake_prop(
            &gpu,
            &baked,
            &PropSpec {
                name: "shadow-test".into(),
                node: Some("box".into()),
                file: None,
                frames_per_side: 8,
                atlas_size: 256,
                hemisphere: false,
                anchors: vec![12.0],
            },
            bounds,
            1,
            5,
        )
        .unwrap();
        let mut renderer = Renderer::new(gpu, 160, 160).unwrap();
        renderer.shadows = Shadows::new(
            &renderer.gpu().device,
            Quality {
                resolution: 128,
                ..Default::default()
            },
        );
        renderer.set_sky(SkySource::Hdr(sky), 12.0).unwrap();
        let props = ImpostorPass::new(
            renderer.gpu(),
            artifact,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
            1,
        )
        .unwrap();
        renderer.set_props(Some(props));
        let output = output(renderer.gpu(), 160, 160);
        let mut view = identity();
        view[3][2] = -4.0;
        let projection = perspective(1.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: frame::multiply(projection, view),
            position: [0.0, 0.0, 4.0],
        };
        let materials = [Material::default()];
        let scene = Scene {
            camera,
            time: 0.0,
            seed: 5,
            sun: Sun {
                direction: [0.0, 1.0, 0.0],
                colour: [1.0; 3],
                intensity: 2.0,
            },
            instances: &[],
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let item = [ImpostorInstance::new(bounds.center, bounds.radius, 1, 0.0).unwrap()];
        renderer
            .render(
                &scene,
                &Text::default(),
                &Effects {
                    props: &item,
                    ..Default::default()
                },
                Finish::Standard,
                &output.view,
            )
            .unwrap();
        assert!(renderer.last_pass_order().contains(&"impostor shadows"));
        let read_mask = |renderer: &Renderer| {
            let size = 128u32;
            let buffer = renderer
                .gpu()
                .device
                .create_buffer(&wgpu::BufferDescriptor {
                    label: Some("renderer shadow readback"),
                    size: u64::from(size * size * 4 * crate::shadow::CASCADE_COUNT as u32),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
            let mut encoder = renderer
                .gpu()
                .device
                .create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: renderer.shadows.atlas(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * 4),
                        rows_per_image: Some(size),
                    },
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: crate::shadow::CASCADE_COUNT as u32,
                },
            );
            renderer.gpu().queue.submit(Some(encoder.finish()));
            let (send, receive) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    send.send(result).unwrap();
                });
            renderer
                .gpu()
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = buffer.slice(..).get_mapped_range();
            let mask = mapped
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()) < 0.999)
                .collect::<Vec<_>>();
            drop(mapped);
            buffer.unmap();
            mask
        };
        let full = read_mask(&renderer);
        assert!(full.iter().any(|&covered| covered));
        let mesh = renderer
            .upload_mesh(MeshData {
                positions: &shape.positions,
                normals: &shape.normals,
                tangents: &shape.tangents,
                uvs: &shape.uvs,
                uvs1: None,
                alpha: None,
                indices: &shape.indices,
            })
            .unwrap();
        let mut mesh_instance = Instance::new(mesh, identity(), 0, 1);
        mesh_instance.coverage = 0.5;
        let instances = [mesh_instance];
        let mixed_scene = Scene {
            time: 1.0 / 60.0,
            instances: &instances,
            ..scene
        };
        let mixed_prop = [ImpostorInstance::new(bounds.center, bounds.radius, 1, 0.5).unwrap()];
        renderer
            .render(
                &mixed_scene,
                &Text::default(),
                &Effects {
                    props: &mixed_prop,
                    ..Default::default()
                },
                Finish::Standard,
                &output.view,
            )
            .unwrap();
        let mixed = read_mask(&renderer);
        let overlap = full.iter().zip(&mixed).filter(|(a, b)| **a && **b).count();
        let union = full.iter().zip(&mixed).filter(|(a, b)| **a || **b).count();
        assert!(
            union > 0 && overlap * 10 >= union * 7,
            "crossfade shadow footprint {overlap}/{union}"
        );
    }

    #[test]
    fn history_resets_on_a_cut() {
        let a = perspective(1.5);
        let mut b = a;
        b[3][0] = 0.4;
        let frame = |n: f32| n / 60.0;
        let mut history = History::default();
        assert!(!history.advance(frame(0.0), a, a));
        assert!(history.advance(frame(1.0), a, a));
        assert!(history.advance(frame(2.0), a, b));
        assert!(history.advance(frame(3.0), b, b));
        assert!(
            !history.advance(frame(4.0), a, a),
            "a camera that does not chain is a cut"
        );
        assert!(history.advance(frame(5.0), a, a));
        assert!(
            !history.advance(frame(30.0), a, a),
            "a jump in time is a cut"
        );
        assert!(history.advance(frame(31.0), a, a));
        history.invalidate();
        assert!(
            !history.advance(frame(32.0), a, a),
            "a resize or daylight jump resets"
        );
        assert!(history.advance(frame(33.0), a, a));
        assert_eq!(history.previous_time(), Some(frame(33.0)));
    }

    #[test]
    fn history_follows_a_scripted_clock() {
        let a = perspective(1.5);
        let mut history = History::default();
        assert!(!history.advance(0.0, a, a));
        for step in 1..=30 {
            assert!(
                history.advance(step as f32 / 30.0, a, a),
                "a 30 fps clip keeps its history"
            );
        }
        assert!(history.advance(1.0, a, a), "a paused clock keeps it");
        assert!(
            history.advance(1.0 + 4.0 / 30.0, a, a),
            "a fast clip keeps it"
        );
        assert!(
            history.advance(1.2, a, a),
            "a dropped window frame keeps it"
        );
        assert!(!history.advance(1.1, a, a), "time going back is a cut");
        history.set_max_gap(0.05);
        assert!(
            !history.advance(1.2, a, a),
            "a gap over the setting is a cut"
        );
        assert!(history.advance(1.25, a, a));
        history.set_max_gap(f32::NAN);
        assert_eq!(history.max_gap(), HISTORY_GAP);
    }

    #[test]
    fn inverse_and_fullscreen_shaders() {
        let mut view = identity();
        view[3] = [0.3, -0.2, -4.0, 1.0];
        let matrix = frame::multiply(perspective(1.7), view);
        let inverse = invert(matrix).unwrap();
        let product = frame::multiply(matrix, inverse);
        for (c, column) in product.iter().enumerate() {
            for (r, value) in column.iter().enumerate() {
                let expected = if c == r { 1.0 } else { 0.0 };
                assert!((value - expected).abs() < 1e-4, "{product:?}");
            }
        }
        assert!(invert([[0.0; 4]; 4]).is_none());
        for source in [motion_source(), downsample_source()] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
        let format = wgpu::TextureFormat::Rgba16Float;
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("prop output"),
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
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        OffscreenTarget {
            texture,
            view,
            format,
            width,
            height,
        }
    }

    fn triangles(mesh: &Mesh, offset: [f32; 3], material: u32) -> Vec<pfx_trace::bvh::Triangle> {
        mesh.indices
            .chunks_exact(3)
            .map(|tri| pfx_trace::bvh::Triangle {
                vertices: [tri[0], tri[1], tri[2]].map(|index| {
                    [0, 1, 2].map(|axis| mesh.positions[index as usize][axis] + offset[axis])
                }),
                material,
            })
            .collect()
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn prop_matches_its_mesh_under_the_sky() {
        use pfx_bake::impostor::{PropSpec, bake_prop, bounds};
        use pfx_bake::{Anchor, BakeScene};
        use pfx_load::Sky;
        let width = 3840;
        let height = 2160;
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let box_mesh = Shape::RoundBox {
            half: [0.8, 0.4, 0.6],
            radius: 0.12,
        }
        .mesh(0.004);
        let sphere_mesh = Shape::Ellipsoid {
            radius: 0.24,
            squash: 1.0,
        }
        .mesh(0.004);
        let sphere_offset = [0.0, 0.62, 0.0];
        let mut tris = triangles(&box_mesh, [0.0; 3], 0);
        tris.extend(triangles(&sphere_mesh, sphere_offset, 1));
        let materials = vec![
            Material {
                base: [0.75, 0.16, 0.08],
                roughness: 0.7,
                ..Default::default()
            },
            Material {
                base: [0.9; 3],
                metalness: 1.0,
                roughness: 0.22,
                ..Default::default()
            },
        ];
        let sky = Sky {
            width: 4,
            height: 2,
            texels: vec![[0.3, 0.4, 0.6, 1.0]; 8],
        };
        let sun = [0.4, 0.8, 0.4];
        let scene = BakeScene {
            triangles: tris,
            shapes: Vec::new(),
            materials: materials.clone(),
            anchors: vec![Anchor {
                hour: 12.0,
                sky: sky.clone(),
                sun: pfx_trace::Sun {
                    direction: sun,
                    color: [1.0; 3],
                    intensity: 2.0,
                },
            }],
        };
        let prop_bounds = bounds(&scene.triangles, &scene.shapes).unwrap();
        let spec = PropSpec {
            name: "rounded-box-chrome".into(),
            node: Some("test".into()),
            file: None,
            frames_per_side: 16,
            atlas_size: 2048,
            hemisphere: false,
            anchors: vec![12.0],
        };
        let artifact = bake_prop(&gpu, &scene, &spec, prop_bounds, 512, 7).unwrap();
        let mut renderer = Renderer::new(gpu, width, height).unwrap();
        renderer.set_sky(SkySource::Hdr(sky), 12.0).unwrap();
        let props = ImpostorPass::new(
            renderer.gpu(),
            artifact,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
            4,
        )
        .unwrap();
        renderer.set_props(Some(props));
        let box_handle = renderer
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
        let sphere_handle = renderer
            .upload_mesh(MeshData {
                positions: &sphere_mesh.positions,
                normals: &sphere_mesh.normals,
                tangents: &sphere_mesh.tangents,
                uvs: &sphere_mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &sphere_mesh.indices,
            })
            .unwrap();
        let mut sphere_model = identity();
        sphere_model[3] = [0.0, sphere_offset[1], 0.0, 1.0];
        let meshes = [
            Instance::new(box_handle, identity(), 0, 1),
            Instance::new(sphere_handle, sphere_model, 1, 2),
        ];
        let card = [ImpostorInstance::new(prop_bounds.center, prop_bounds.radius, 3, 0.0).unwrap()];
        let look = |eye: [f32; 3]| {
            let d = [
                prop_bounds.center[0] - eye[0],
                prop_bounds.center[1] - eye[1],
                prop_bounds.center[2] - eye[2],
            ];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let forward = d.map(|v| v / l);
            let r = [-forward[2], 0.0, forward[0]];
            let rl = (r[0] * r[0] + r[2] * r[2]).sqrt();
            let right = r.map(|v| v / rl);
            let up = [
                right[1] * forward[2] - right[2] * forward[1],
                right[2] * forward[0] - right[0] * forward[2],
                right[0] * forward[1] - right[1] * forward[0],
            ];
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let view = [
                [right[0], up[0], -forward[0], 0.0],
                [right[1], up[1], -forward[1], 0.0],
                [right[2], up[2], -forward[2], 0.0],
                [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
            ];
            let projection = perspective(width as f32 / height as f32);
            Camera {
                view,
                projection,
                previous_view_projection: frame::multiply(projection, view),
                position: eye,
            }
        };
        let views = [
            ("head-on", look([0.0, prop_bounds.center[1], 20.0])),
            ("oblique", look([8.4, prop_bounds.center[1] + 5.3, 17.6])),
        ];
        let target = output(renderer.gpu(), width, height);
        let mut start = 0;
        let mut results = Vec::new();
        for (name, camera) in views {
            let mut capture =
                |renderer: &mut Renderer, instances: &[Instance], props: &[ImpostorInstance]| {
                    for number in start..start + 16 {
                        let scene = Scene {
                            camera,
                            time: number as f32 / 60.0,
                            seed: 7,
                            sun: Sun {
                                direction: sun,
                                colour: [1.0; 3],
                                intensity: 2.0,
                            },
                            instances,
                            materials: &materials,
                            deformers: &[],
                            wind: SceneWind::default(),
                        };
                        renderer
                            .render(
                                &scene,
                                &Text::default(),
                                &Effects {
                                    props,
                                    ..Default::default()
                                },
                                Finish::Standard,
                                &target.view,
                            )
                            .unwrap();
                    }
                    start += 100;
                    renderer
                        .gpu()
                        .readback_rgba16(&target)
                        .unwrap()
                        .chunks_exact(4)
                        .map(|pixel| [0, 1, 2].map(|c| half::f16::from_bits(pixel[c]).to_f32()))
                        .collect::<Vec<_>>()
                };
            let background = capture(&mut renderer, &[], &[]);
            let mesh = capture(&mut renderer, &meshes, &[]);
            let prop = capture(&mut renderer, &[], &card);
            let differs = |a: [f32; 3], b: [f32; 3]| (0..3).any(|c| (a[c] - b[c]).abs() > 0.004);
            let mask: Vec<bool> = (0..mesh.len())
                .map(|i| differs(mesh[i], background[i]) || differs(prop[i], background[i]))
                .collect();
            let clip = frame::transform(
                frame::multiply(camera.projection, camera.view),
                [sphere_offset[0], sphere_offset[1], sphere_offset[2], 1.0],
            );
            let centre = [
                (clip[0] / clip[3] * 0.5 + 0.5) * width as f32,
                (0.5 - clip[1] / clip[3] * 0.5) * height as f32,
            ];
            let reach = 0.24 * 1.15 * camera.projection[1][1] * height as f32 * 0.5 / clip[3];
            let on_sphere = |i: usize| {
                let x = (i % width as usize) as f32 + 0.5 - centre[0];
                let y = (i / width as usize) as f32 + 0.5 - centre[1];
                x * x + y * y <= reach * reach
            };
            let mean = |keep: &dyn Fn(usize) -> bool| {
                let picked: Vec<usize> = (0..mesh.len()).filter(|i| mask[*i] && keep(*i)).collect();
                let sum = picked
                    .iter()
                    .map(|&i| (0..3).map(|c| (mesh[i][c] - prop[i][c]).abs()).sum::<f32>())
                    .sum::<f32>();
                (sum / (picked.len().max(1) * 3) as f32, picked.len())
            };
            let (error, count) = mean(&|_| true);
            let (box_error, box_count) = mean(&|i| !on_sphere(i));
            let (chrome_error, chrome_count) = mean(&|i| on_sphere(i));
            let both: Vec<usize> = (0..mesh.len())
                .filter(|&i| {
                    !on_sphere(i)
                        && differs(mesh[i], background[i])
                        && differs(prop[i], background[i])
                })
                .collect();
            let bias = [0, 1, 2].map(|c| {
                both.iter().map(|&i| prop[i][c] - mesh[i][c]).sum::<f32>()
                    / both.len().max(1) as f32
            });
            let interior = both
                .iter()
                .map(|&i| (0..3).map(|c| (mesh[i][c] - prop[i][c]).abs()).sum::<f32>())
                .sum::<f32>()
                / (both.len().max(1) * 3) as f32;
            println!(
                "{name}: box bias where both cover {bias:?}, MAE {interior:.4} over {} pixels",
                both.len()
            );
            println!(
                "{name}: rounded box MAE {box_error:.4} over {box_count} pixels, chrome sphere MAE {chrome_error:.4} over {chrome_count} pixels"
            );
            let covered = |image: &[[f32; 3]]| -> Vec<bool> {
                (0..image.len())
                    .map(|i| differs(image[i], background[i]))
                    .collect()
            };
            let speckles = |cover: &[bool]| {
                let w = width as usize;
                let h = height as usize;
                let mut lonely = 0;
                for y in 1..h - 1 {
                    for x in 1..w - 1 {
                        let here = cover[y * w + x];
                        let same = [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)]
                            .iter()
                            .filter(|(dx, dy)| {
                                cover[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize]
                                    == here
                            })
                            .count();
                        if same == 0 {
                            lonely += 1;
                        }
                    }
                }
                lonely
            };
            let mesh_speckles = speckles(&covered(&mesh));
            let prop_speckles = speckles(&covered(&prop));
            println!(
                "{name}: object-only display MAE {error:.4} over {count} pixels; isolated edge pixels: mesh {mesh_speckles}, prop {prop_speckles}"
            );
            results.push((name, error, count, mesh_speckles, prop_speckles, interior));
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
            std::fs::create_dir_all(&root).unwrap();
            let file =
                std::fs::File::create(root.join(format!("prop-against-mesh-{name}.png"))).unwrap();
            let mut encoder = png::Encoder::new(file, width * 2, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut bytes = Vec::with_capacity((width * 2 * height * 3) as usize);
            for y in 0..height as usize {
                for image in [&mesh, &prop] {
                    for x in 0..width as usize {
                        bytes.extend(
                            image[y * width as usize + x]
                                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                        );
                    }
                }
            }
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&bytes)
                .unwrap();
        }
        let (_, _, count, mesh_speckles, prop_speckles, _) = results[0];
        assert!(count > 20_000, "{count} object pixels");
        for (name, _, _, _, _, box_error) in &results {
            assert!(
                *box_error <= 0.02,
                "{name}: rounded box interior MAE {box_error:.4}"
            );
        }
        assert!(
            prop_speckles <= mesh_speckles + count / 2000,
            "speckled prop edges"
        );
    }

    const PICK_SIZE: u32 = 128;
    const FLOOR_ID: u32 = 3;
    const PANEL_ID: u32 = 4;
    const GLASS_PIXEL: (u32, u32) = (27, 64);
    const TEXT_PIXEL: (u32, u32) = (99, 64);
    const PANEL_PIXEL: (u32, u32) = (121, 64);
    const OVERLAY_PIXEL: (u32, u32) = (64, 112);
    const FLOOR_PIXEL: (u32, u32) = (64, 20);

    struct IdShot {
        colour: Vec<u16>,
        ids: Vec<u32>,
        picked: Picked,
        bounds: Vec<TextBounds>,
        at_overlay: Option<TextBounds>,
        passes: Vec<&'static str>,
    }

    impl IdShot {
        fn id(&self, (x, y): (u32, u32)) -> u32 {
            self.ids[(y * PICK_SIZE + x) as usize]
        }
    }

    fn translated(x: f32, y: f32, z: f32) -> Matrix {
        let mut model = identity();
        model[3] = [x, y, z, 1.0];
        model
    }

    fn upload_shape(renderer: &mut Renderer, mesh: &Mesh) -> MeshHandle {
        renderer
            .upload_mesh(MeshData {
                positions: &mesh.positions,
                normals: &mesh.normals,
                tangents: &mesh.tangents,
                uvs: &mesh.uvs,
                uvs1: None,
                alpha: None,
                indices: &mesh.indices,
            })
            .unwrap()
    }

    fn solid_atlas() -> pfx_text::Atlas {
        let level = |size: u32| pfx_text::MipLevel {
            width: size,
            height: size,
            bytes: vec![255; (size * size) as usize],
        };
        pfx_text::Atlas {
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

    fn id_shot(
        glass_id: u32,
        text_id: u32,
        overlay_id: u32,
        lit: bool,
        pick: (u32, u32),
    ) -> IdShot {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, PICK_SIZE, PICK_SIZE).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        renderer.shadows = Shadows::new(
            &renderer.gpu().device,
            Quality {
                resolution: 256,
                ..Default::default()
            },
        );
        let floor = Shape::RoundBox {
            half: [3.0, 3.0, 0.1],
            radius: 0.02,
        }
        .mesh(0.5);
        let panel = Shape::RoundBox {
            half: [0.6, 0.6, 0.05],
            radius: 0.02,
        }
        .mesh(0.2);
        let slab = Shape::RoundBox {
            half: [0.5, 0.5, 0.2],
            radius: 0.05,
        }
        .mesh(0.2);
        let floor = upload_shape(&mut renderer, &floor);
        let panel = upload_shape(&mut renderer, &panel);
        let slab = renderer.upload_glass(&slab).unwrap();
        let instances = [
            Instance::new(floor, translated(0.0, 0.0, -1.0), 0, FLOOR_ID),
            Instance::new(panel, translated(0.8, 0.0, 0.0), 0, PANEL_ID),
        ];
        let materials = [Material {
            base: [0.6; 3],
            roughness: 0.6,
            ..Default::default()
        }];
        let glass = [glass::Surface {
            mesh: slab,
            model: translated(-0.8, 0.0, 0.0),
            material: pfx_materials::fixtures::glass(),
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: glass_id,
            casts_shadow: false,
            shadow_only: false,
            clip: [[0.0; 4]; 2],
        }];
        let atlas = GpuAtlas::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            &solid_atlas(),
        )
        .unwrap();
        let ink = [solid_quad([0.0, 0.0], 30.0, [0.1, 0.1, 0.1, 1.0])];
        let badge = [solid_quad(
            [OVERLAY_PIXEL.0 as f32, OVERLAY_PIXEL.1 as f32],
            8.0,
            [1.0, 0.8, 0.2, 1.0],
        )];
        let mut view = identity();
        view[3][2] = -4.0;
        let projection = perspective(1.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: frame::multiply(projection, view),
            position: [0.0, 0.0, 4.0],
        };
        let text_model = [
            [0.01, 0.0, 0.0, 0.0],
            [0.0, -0.01, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.8, 0.0, 0.08, 1.0],
        ];
        let model_view_projection = frame::multiply(frame::multiply(projection, view), text_model);
        let text_space = if lit {
            TextSpace::LitSurface {
                model_view_projection,
                model: text_model,
            }
        } else {
            TextSpace::Surface {
                model_view_projection,
            }
        };
        let text = Text {
            surface_quads: vec![TextQuadItem {
                atlas: &atlas,
                quads: &ink,
                space: text_space,
                icons: false,
                id: text_id,
            }],
            overlay_quads: vec![TextQuadItem {
                atlas: &atlas,
                quads: &badge,
                space: TextSpace::Overlay {
                    width: PICK_SIZE,
                    height: PICK_SIZE,
                },
                icons: false,
                id: overlay_id,
            }],
            ..Default::default()
        };
        let effects = Effects {
            glass: &glass,
            ..Default::default()
        };
        let target = output(renderer.gpu(), PICK_SIZE, PICK_SIZE);
        renderer.pick(pick.0, pick.1).unwrap();
        for number in 0..4 {
            let scene = Scene {
                camera,
                time: number as f32 / 60.0,
                seed: 5,
                sun: Sun {
                    direction: [0.3, 0.5, 0.8],
                    colour: [1.0; 3],
                    intensity: 2.0,
                },
                instances: &instances,
                materials: &materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            renderer
                .render(&scene, &text, &effects, Finish::Standard, &target.view)
                .unwrap();
        }
        let colour = renderer.gpu().readback_rgba16(&target).unwrap();
        let ids = readback_texture(
            renderer.gpu(),
            &renderer.frame().targets.ids,
            PICK_SIZE,
            PICK_SIZE,
            4,
        )
        .unwrap()
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        .collect();
        let mut picked = None;
        for _ in 0..100 {
            picked = renderer.picked();
            if picked.is_some() {
                break;
            }
            let _ = renderer
                .gpu()
                .device
                .poll(wgpu::PollType::wait_indefinitely());
        }
        IdShot {
            colour,
            ids,
            picked: picked.expect("the pick never came back"),
            bounds: renderer.text_bounds().to_vec(),
            at_overlay: renderer
                .text_at(OVERLAY_PIXEL.0 as f32 + 0.5, OVERLAY_PIXEL.1 as f32 + 0.5)
                .copied(),
            passes: renderer.last_pass_order().to_vec(),
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Caster {
        Nothing,
        Copy { moving: bool },
        Glass,
    }

    const CAST_SIZE: u32 = 256;

    struct CastShot {
        colour: Vec<u16>,
        passes: Vec<&'static str>,
        transmission: f64,
        cascades: f64,
        opaque: f64,
    }

    fn cast_glass() -> Material {
        Material {
            base: [0.5, 0.62, 0.95],
            ..pfx_materials::fixtures::glass()
        }
    }

    fn cast_camera() -> Camera {
        let mut view = identity();
        view[3][2] = -4.0;
        let projection = perspective(1.0);
        Camera {
            view,
            projection,
            previous_view_projection: frame::multiply(projection, view),
            position: [0.0, 0.0, 4.0],
        }
    }

    fn cast_pixel(world: [f32; 3]) -> usize {
        let camera = cast_camera();
        let clip = frame::transform(
            frame::multiply(camera.projection, camera.view),
            [world[0], world[1], world[2], 1.0],
        );
        let x = ((clip[0] / clip[3] * 0.5 + 0.5) * CAST_SIZE as f32) as usize;
        let y = ((0.5 - clip[1] / clip[3] * 0.5) * CAST_SIZE as f32) as usize;
        y * CAST_SIZE as usize + x
    }

    fn cast_shot(caster: Caster, frames: usize) -> CastShot {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, CAST_SIZE, CAST_SIZE).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let floor = Shape::RoundBox {
            half: [3.0, 3.0, 0.1],
            radius: 0.02,
        }
        .mesh(0.5);
        let slab = Shape::RoundBox {
            half: [0.5, 0.5, 0.2],
            radius: 0.05,
        }
        .mesh(0.2);
        let floor = upload_shape(&mut renderer, &floor);
        let glass_mesh = renderer.upload_glass(&slab).unwrap();
        let mut instances = vec![Instance::new(floor, translated(0.0, 0.0, -1.0), 0, 1)];
        let glass_model = translated(-0.8, 0.0, 0.0);
        if let Caster::Copy { moving } = caster {
            let copy = upload_shape(&mut renderer, &slab);
            let mut instance = Instance::new(copy, glass_model, 1, 2);
            instance.shadow_only = true;
            if moving {
                instance.previous_model[3][1] = 1.0e-9;
            }
            instances.push(instance);
            renderer
                .set_transmission(&[None, Some(glass::cast_tint(&cast_glass()))])
                .unwrap();
        }
        let materials = [
            Material {
                base: [0.6; 3],
                roughness: 0.6,
                ..Default::default()
            },
            Material::default(),
        ];
        let glass = [glass::Surface {
            mesh: glass_mesh,
            model: glass_model,
            material: cast_glass(),
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: caster == Caster::Glass,
            shadow_only: false,
            clip: [[0.0; 4]; 2],
        }];
        let effects = Effects {
            glass: &glass,
            ..Default::default()
        };
        let target = output(renderer.gpu(), CAST_SIZE, CAST_SIZE);
        let mut sums = [0.0; 3];
        let mut timed = 0;
        let mut turns = pfx_gpu::pace::Turns::default();
        for number in 0..frames {
            let scene = Scene {
                camera: cast_camera(),
                time: number as f32 / 60.0,
                seed: 5,
                sun: Sun {
                    direction: [0.3, 0.5, 0.8],
                    colour: [1.0; 3],
                    intensity: 2.0,
                },
                instances: &instances,
                materials: &materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            let timings = renderer
                .render(
                    &scene,
                    &Text::default(),
                    &effects,
                    Finish::Standard,
                    &target.view,
                )
                .unwrap();
            turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
            if number < 4 || timings.is_empty() {
                continue;
            }
            timed += 1;
            for timing in &timings {
                let slot = if timing.label == "shadow transmission" {
                    0
                } else if timing.label.starts_with("shadow cascade") {
                    1
                } else if timing.label == "opaque PBR" {
                    2
                } else {
                    continue;
                };
                sums[slot] += timing.milliseconds;
            }
        }
        let mean = |sum: f64| sum / f64::from(timed.max(1));
        CastShot {
            colour: renderer.gpu().readback_rgba16(&target).unwrap(),
            passes: renderer.last_pass_order().to_vec(),
            transmission: mean(sums[0]),
            cascades: mean(sums[1]),
            opaque: mean(sums[2]),
        }
    }

    fn rgb(colour: &[u16], at: usize) -> [f32; 3] {
        [0, 1, 2].map(|k| half::f16::from_bits(colour[at * 4 + k]).to_f32())
    }

    #[test]
    fn a_glass_cast_tint_is_transmission_times_base_or_beer_lambert_over_its_thickness() {
        let clear = Material {
            base: [0.5, 0.8, 1.0],
            transmission: 0.9,
            ..Material::default()
        };
        let tint = glass::cast_tint(&clear);
        for (k, expected) in [0.45, 0.72, 0.9].into_iter().enumerate() {
            assert!((tint[k] - expected).abs() < 1e-6, "{tint:?}");
        }
        let thick = Material {
            thickness: 0.3,
            absorption: 4.0,
            ..clear
        };
        let beer = pfx_materials::beer(0.3, [0.5, 0.8, 1.0], 4.0);
        let tint = glass::cast_tint(&thick);
        for k in 0..3 {
            assert!((tint[k] - beer[k] * 0.9).abs() < 1e-6, "{tint:?}");
        }
        assert_eq!(tint[2], 0.9);
        let unthick = Material {
            absorption: 4.0,
            ..clear
        };
        assert_eq!(glass::cast_tint(&unthick), glass::cast_tint(&clear));
        let bright = Material {
            base: [2.0, 0.5, -1.0],
            transmission: 1.0,
            ..Material::default()
        };
        assert_eq!(glass::cast_tint(&bright), [1.0, 0.5, 0.0]);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn glass_casts_the_shadow_its_shadow_only_copy_cast() {
        let nothing = cast_shot(Caster::Nothing, 6);
        let copy = cast_shot(Caster::Copy { moving: false }, 6);
        let glass = cast_shot(Caster::Glass, 6);
        assert!(!nothing.passes.contains(&"shadow transmission"));
        assert!(glass.passes.contains(&"shadow transmission"));
        let shade = cast_pixel([-1.1, -0.9, -0.9]);
        let open = cast_pixel([1.5, -0.9, -0.9]);
        let ratio = |shot: &CastShot| {
            let (lit, shaded) = (rgb(&shot.colour, open), rgb(&shot.colour, shade));
            [0, 1, 2].map(|k| shaded[k] / lit[k])
        };
        println!(
            "under the slab / open floor: nothing {:?}, copy {:?}, glass {:?}",
            ratio(&nothing),
            ratio(&copy),
            ratio(&glass)
        );
        let glassed = ratio(&glass);
        assert!(glassed[2] > glassed[0] + 0.1, "the shadow is not tinted");
        assert!(
            glassed[0] < ratio(&nothing)[0] - 0.1,
            "the glass casts no shadow"
        );
        let mut worst = 0.0f32;
        let mut differing = 0;
        for (a, b) in copy
            .colour
            .chunks_exact(4)
            .zip(glass.colour.chunks_exact(4))
        {
            let gap = (0..3)
                .map(|k| {
                    (half::f16::from_bits(a[k]).to_f32() - half::f16::from_bits(b[k]).to_f32())
                        .abs()
                })
                .fold(0.0, f32::max);
            worst = worst.max(gap);
            differing += usize::from(gap > 0.0);
        }
        println!("copy against glass: {differing} pixels differ, worst {worst:.5}");
        assert!(worst <= 1.0 / 255.0, "worst {worst}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn glass_casting_costs_about_what_its_copy_costs() {
        for (name, caster) in [
            ("nothing", Caster::Nothing),
            ("still copy", Caster::Copy { moving: false }),
            ("moving copy", Caster::Copy { moving: true }),
            ("glass", Caster::Glass),
        ] {
            let shot = cast_shot(caster, 64);
            println!(
                "{name}: transmission {:.4} ms, cascades {:.4} ms, opaque {:.4} ms",
                shot.transmission, shot.cascades, shot.opaque
            );
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Card {
        Plain,
        Cut,
        Grid,
    }

    const CARD_IMAGE: u32 = 256;
    const CARD_CELLS: u32 = 128;

    fn card_alpha(u: f32, v: f32) -> f32 {
        let r = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
        if (0.25..=0.45).contains(&r) { 1.0 } else { 0.0 }
    }

    fn card_mesh(renderer: &mut Renderer, card: Card, half: f32) -> MeshHandle {
        let cells = if card == Card::Grid { CARD_CELLS } else { 1 };
        let side = cells + 1;
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut alpha = Vec::new();
        for j in 0..side {
            for i in 0..side {
                let uv = [i as f32 / cells as f32, j as f32 / cells as f32];
                positions.push([(uv[0] * 2.0 - 1.0) * half, 0.0, (uv[1] * 2.0 - 1.0) * half]);
                uvs.push(uv);
                alpha.push(card_alpha(uv[0], uv[1]));
            }
        }
        let mut indices = Vec::new();
        for j in 0..cells {
            for i in 0..cells {
                let a = j * side + i;
                indices.extend([a, a + side, a + 1, a + 1, a + side, a + side + 1]);
            }
        }
        let count = positions.len();
        renderer
            .upload_mesh(MeshData {
                positions: &positions,
                normals: &vec![[0.0, 1.0, 0.0]; count],
                tangents: &vec![[1.0, 0.0, 0.0, 1.0]; count],
                uvs: &uvs,
                uvs1: None,
                alpha: (card == Card::Grid).then_some(alpha.as_slice()),
                indices: &indices,
            })
            .unwrap()
    }

    fn card_costs(card: Card) -> [f64; 3] {
        let (width, height) = (3840, 2160);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, width, height).unwrap();
        renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
        let floor = card_mesh(&mut renderer, Card::Plain, 4.0);
        let mesh = card_mesh(&mut renderer, card, 0.3);
        let texels: Vec<u8> = (0..CARD_IMAGE * CARD_IMAGE)
            .flat_map(|index| {
                let u = ((index % CARD_IMAGE) as f32 + 0.5) / CARD_IMAGE as f32;
                let v = ((index / CARD_IMAGE) as f32 + 0.5) / CARD_IMAGE as f32;
                [220, 60, 40, (card_alpha(u, v) * 255.0) as u8]
            })
            .collect();
        renderer
            .frame()
            .set_content(0, CARD_IMAGE, CARD_IMAGE, crate::maps::ContentFormat::Srgb8)
            .unwrap();
        renderer
            .frame()
            .write_content(0, [0, 0, CARD_IMAGE, CARD_IMAGE], &texels)
            .unwrap();
        let mut instances = vec![Instance::new(floor, identity(), 0, 1)];
        for row in 0..4 {
            for column in 0..6 {
                let model = translated(
                    (column as f32 - 2.5) * 0.8,
                    0.8 + 0.05 * row as f32,
                    (row as f32 - 1.5) * 0.8,
                );
                let mut instance = Instance::new(mesh, model, 1, instances.len() as u32 + 1);
                instance.previous_model[3][1] += 1.0e-6;
                instances.push(instance);
            }
        }
        let mut surfaces = vec![
            frame::InstanceSurface {
                cutout: card == Card::Cut,
                ..Default::default()
            };
            instances.len()
        ];
        surfaces[0].cutout = false;
        renderer.frame().set_surfaces(&surfaces).unwrap();
        let materials = [
            Material {
                base: [0.6; 3],
                roughness: 0.7,
                ..Default::default()
            },
            Material {
                base: [0.9; 3],
                roughness: 0.5,
                content_layer: pfx_materials::ContentLayer {
                    slot: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        let view = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, -4.0, 1.0],
        ];
        let projection = perspective(width as f32 / height as f32);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: frame::multiply(projection, view),
            position: [0.0, 4.0, 0.0],
        };
        let target = output(renderer.gpu(), width, height);
        let mut sums = [0.0; 3];
        let mut timed = 0;
        let mut turns = pfx_gpu::pace::Turns::default();
        for number in 0..40 {
            let scene = Scene {
                camera,
                time: number as f32 / 60.0,
                seed: 5,
                sun: Sun {
                    direction: [0.8, 0.6, 0.0],
                    colour: [1.0; 3],
                    intensity: 3.0,
                },
                instances: &instances,
                materials: &materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            let timings = renderer
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &target.view,
                )
                .unwrap();
            turns.add(pfx_gpu::pace::gpu_ms(&timings).unwrap_or(f64::NAN));
            if number < 8 || timings.is_empty() {
                continue;
            }
            timed += 1;
            for timing in &timings {
                let slot = if timing.label == "depth and velocity" {
                    0
                } else if timing.label == "opaque PBR" {
                    1
                } else if timing.label.starts_with("shadow cascade") {
                    2
                } else {
                    continue;
                };
                sums[slot] += timing.milliseconds;
            }
        }
        sums.map(|sum| sum / f64::from(timed.max(1)))
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn card_scene_costs_with_content_cutouts_and_vertex_alpha_grids() {
        for card in [Card::Plain, Card::Cut, Card::Grid] {
            let [depth, opaque, cascades] = card_costs(card);
            println!(
                "{card:?} cards at 3840x2160: depth and velocity {depth:.4} ms, opaque {opaque:.4} ms, cascades {cascades:.4} ms"
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn glass_with_an_id_picks_over_the_floor_and_glass_without_one_lets_it_through() {
        let picked = id_shot(9, 0, 0, false, GLASS_PIXEL);
        assert_eq!(picked.picked.id, 9);
        assert_eq!(picked.id(GLASS_PIXEL), 9);
        assert_eq!(picked.id(FLOOR_PIXEL), FLOOR_ID);
        assert_eq!(picked.id(PANEL_PIXEL), PANEL_ID);
        assert!(picked.passes.contains(&"glass ids"));
        let through = id_shot(0, 0, 0, false, GLASS_PIXEL);
        assert_eq!(through.picked.id, FLOOR_ID);
        assert_eq!(through.id(GLASS_PIXEL), FLOOR_ID);
        assert!(!through.passes.contains(&"glass ids"));
        let slab = picked.ids.iter().filter(|id| **id == 9).count();
        let changed = picked
            .ids
            .iter()
            .zip(&through.ids)
            .filter(|(a, b)| a != b)
            .count();
        assert!(slab > 400, "the slab covers only {slab} pixels");
        assert_eq!(changed, slab, "ids changed outside the slab");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn surface_text_picks_as_its_own_id_on_its_panel() {
        for lit in [false, true] {
            let shot = id_shot(0, 21, 0, lit, TEXT_PIXEL);
            assert_eq!(shot.picked.id, 21, "lit {lit}");
            assert_eq!(shot.id(TEXT_PIXEL), 21, "lit {lit}");
            assert_eq!(shot.id(PANEL_PIXEL), PANEL_ID, "lit {lit}");
            assert!(shot.passes.contains(&"text ids"));
            let plain = id_shot(0, 0, 0, lit, TEXT_PIXEL);
            assert_eq!(plain.picked.id, PANEL_ID, "lit {lit}");
            assert!(!plain.passes.contains(&"text ids"));
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn overlay_text_picks_over_a_mesh_and_its_bounds_come_back_on_the_cpu() {
        let shot = id_shot(0, 21, 33, false, OVERLAY_PIXEL);
        assert_eq!(shot.picked.id, 33);
        assert_eq!(shot.id(OVERLAY_PIXEL), 33);
        assert_eq!(shot.id(FLOOR_PIXEL), FLOOR_ID);
        let plain = id_shot(0, 21, 0, false, OVERLAY_PIXEL);
        assert_eq!(plain.picked.id, FLOOR_ID);
        let overlay = shot.at_overlay.expect("no text bounds under the overlay");
        assert_eq!(
            (overlay.list, overlay.index, overlay.id),
            (TextList::OverlayQuads, 0, 33)
        );
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01);
        assert!(
            close(overlay.rect, [56.0, 104.0, 72.0, 120.0]),
            "{:?}",
            overlay.rect
        );
        let surface = shot
            .bounds
            .iter()
            .find(|bounds| bounds.list == TextList::SurfaceQuads)
            .expect("no surface text bounds");
        assert_eq!(surface.id, 21);
        let [left, top, right, bottom] = surface.rect;
        let (x, y) = (TEXT_PIXEL.0 as f32, TEXT_PIXEL.1 as f32);
        assert!(
            left < x && x < right && top < y && y < bottom,
            "{:?}",
            surface.rect
        );
        let covered = shot.ids.iter().enumerate().filter(|(_, id)| **id == 21);
        for (index, _) in covered {
            let px = (index as u32 % PICK_SIZE) as f32 + 0.5;
            let py = (index as u32 / PICK_SIZE) as f32 + 0.5;
            assert!(
                left <= px && px <= right && top <= py && py <= bottom,
                "text id at ({px}, {py}) outside its bounds {:?}",
                surface.rect
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn id_writes_leave_the_frames_colour_bytes_unchanged() {
        for lit in [false, true] {
            let before = id_shot(0, 0, 0, lit, GLASS_PIXEL);
            let after = id_shot(9, 21, 33, lit, GLASS_PIXEL);
            assert_eq!(after.id(GLASS_PIXEL), 9);
            assert_eq!(after.id(TEXT_PIXEL), 21);
            assert_eq!(after.id(OVERLAY_PIXEL), 33);
            let differ = before
                .colour
                .iter()
                .zip(&after.colour)
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(differ, 0, "lit {lit}: {differ} colour channels changed");
        }
    }

    fn glass_slab(half: f32, step: f32) -> Mesh {
        Shape::RoundBox {
            half: [half, half, 0.2],
            radius: 0.05,
        }
        .mesh(step)
    }

    fn render_glass(
        renderer: &mut Renderer,
        handle: glass::MeshHandle,
        target: &OffscreenTarget,
    ) -> Result<(), String> {
        let mut view = identity();
        view[3][2] = -4.0;
        let projection = perspective(1.0);
        let camera = Camera {
            view,
            projection,
            previous_view_projection: frame::multiply(projection, view),
            position: [0.0, 0.0, 4.0],
        };
        let materials = [Material::default()];
        let glass = [glass::Surface {
            mesh: handle,
            model: identity(),
            material: pfx_materials::fixtures::glass(),
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: false,
            shadow_only: false,
            clip: [[0.0; 4]; 2],
        }];
        let scene = Scene {
            camera,
            time: 0.0,
            seed: 5,
            sun: Sun {
                direction: [0.3, 0.5, 0.8],
                colour: [1.0; 3],
                intensity: 2.0,
            },
            instances: &[],
            materials: &materials,
            deformers: &[],
            wind: SceneWind::default(),
        };
        let effects = Effects {
            glass: &glass,
            ..Default::default()
        };
        renderer
            .render(
                &scene,
                &Text::default(),
                &effects,
                Finish::Standard,
                &target.view,
            )
            .map(|_| ())
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn glass_meshes_release_replace_and_keep_their_handles_through_a_resize() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, PICK_SIZE, PICK_SIZE).unwrap();
        let target = output(renderer.gpu(), PICK_SIZE, PICK_SIZE);
        let small = glass_slab(0.3, 0.3);
        let large = glass_slab(0.5, 0.1);
        let keep = renderer.upload_glass(&small).unwrap();
        let flat = renderer.mesh_stats();
        assert_eq!(
            (flat.glass_live, flat.glass_slots, flat.glass_buffers),
            (1, 1, 2)
        );
        render_glass(&mut renderer, keep, &target).unwrap();
        for cycle in 0..1000 {
            let (first, second) = if cycle % 2 == 0 {
                (&small, &large)
            } else {
                (&large, &small)
            };
            let handle = renderer.upload_glass(first).unwrap();
            assert_eq!(renderer.mesh_stats().glass_live, 2);
            renderer.replace_glass(handle, second).unwrap();
            renderer.replace_glass(handle, first).unwrap();
            renderer.release_glass(handle).unwrap();
            assert!(renderer.release_glass(handle).is_err());
            assert!(renderer.replace_glass(handle, first).is_err());
            assert!(renderer.glass_mesh_buffers(handle).is_none());
            let stats = renderer.mesh_stats();
            assert_eq!(
                (stats.glass_live, stats.glass_slots, stats.glass_buffers),
                (1, 2, 2),
                "cycle {cycle}"
            );
        }
        let before = renderer.glass_mesh_buffers(keep).unwrap().0.clone();
        renderer.replace_glass(keep, &small).unwrap();
        assert!(*renderer.glass_mesh_buffers(keep).unwrap().0 == before);
        renderer.replace_glass(keep, &large).unwrap();
        assert_eq!(renderer.mesh_stats().glass_buffers, 2);
        renderer.replace_glass(keep, &small).unwrap();
        assert_eq!(
            renderer.glass_mesh_buffers(keep).unwrap().2,
            small.indices.len() as u32
        );
        renderer.resize(96, 96).unwrap();
        let resized = renderer.mesh_stats();
        assert_eq!(
            (
                resized.glass_live,
                resized.glass_slots,
                resized.glass_buffers
            ),
            (1, 2, 2)
        );
        let target = output(renderer.gpu(), 96, 96);
        render_glass(&mut renderer, keep, &target).unwrap();
        renderer.release_glass(keep).unwrap();
        let error = render_glass(&mut renderer, keep, &target).unwrap_err();
        assert!(error.contains("released"), "{error}");
        let end = renderer.mesh_stats();
        assert_eq!(
            (end.glass_live, end.glass_buffers),
            (0, 0),
            "the last glass mesh was released"
        );
    }

    fn flat_board() -> (crate::flat::ShadowCurve, Vec<crate::flat::Draw>) {
        use crate::flat::{CurvePoint, Draw, ShadowCurve, Srgba, Stroke};
        let curve = ShadowCurve::new(
            Srgba::hex(0x2b3442),
            vec![
                CurvePoint {
                    elevation: 1.0,
                    offset: 4.0,
                    sigma: 5.0,
                    alpha: 0.22,
                },
                CurvePoint {
                    elevation: 2.0,
                    offset: 12.0,
                    sigma: 16.0,
                    alpha: 0.26,
                },
            ],
        )
        .unwrap();
        let draws = vec![
            Draw::rect([48.0, 40.0], [64.0, 40.0], 8.0)
                .fill(Srgba::WHITE)
                .elevation(1.0)
                .id(7),
            Draw::circle([30.0, 40.0], 9.0)
                .fill(Srgba::hex(0xe2614c))
                .elevation(1.0)
                .shadow(crate::flat::Shadow::None)
                .id(8),
            Draw::rect([70.0, 70.0], [40.0, 30.0], 10.0)
                .vertical(Srgba::hex(0xffd36b), Srgba::hex(0xf0a92e))
                .stroke(Stroke::solid(Srgba::hex(0xc4840c), 2.0))
                .elevation(2.0)
                .id(9),
        ];
        (curve, draws)
    }

    fn flat_scene<'a>(
        curve: &'a crate::flat::ShadowCurve,
        draws: &'a [crate::flat::Draw],
    ) -> crate::flat::FlatScene<'a> {
        crate::flat::FlatScene {
            layout: [96.0, 96.0],
            clear: Some(crate::flat::Srgba::hex(0xeceff3)),
            curve,
            light: crate::flat::Light::default(),
            draws,
            groups: &[],
            text: &[],
            icons: None,
            sprites: None,
            environment: None,
            post: false,
            frame: 0,
            seed: 0,
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn existing_scenes_render_the_same_bytes_beside_a_flat_hud() {
        let (mut plain, plain_target, instances, materials) = sphere_still(96);
        let (mut beside, beside_target, _, _) = sphere_still(96);
        let (curve, _) = flat_board();
        let empty = flat_scene(&curve, &[]);
        for number in 0..4 {
            let scene = still_scene(&instances, &materials, number);
            plain
                .render(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &plain_target.view,
                )
                .unwrap();
            beside
                .render_with_flat(
                    &scene,
                    &Text::default(),
                    &Effects::default(),
                    Finish::Standard,
                    &empty,
                    &beside_target.view,
                )
                .unwrap();
        }
        assert!(!plain.last_pass_order().contains(&"flat"));
        assert!(beside.last_pass_order().contains(&"flat"));
        let expected = plain.gpu().readback_rgba16(&plain_target).unwrap();
        let actual = beside.gpu().readback_rgba16(&beside_target).unwrap();
        assert!(expected == actual, "an empty flat HUD changed the frame");
        let (curve, draws) = flat_board();
        let hud = flat_scene(&curve, &draws);
        let scene = still_scene(&instances, &materials, 4);
        beside
            .render_with_flat(
                &scene,
                &Text::default(),
                &Effects::default(),
                Finish::Standard,
                &hud,
                &beside_target.view,
            )
            .unwrap();
        let with_hud = beside.gpu().readback_rgba16(&beside_target).unwrap();
        let at = |pixels: &[u16], x: usize, y: usize| -> [f32; 4] {
            std::array::from_fn(|c| half::f16::from_bits(pixels[(y * 96 + x) * 4 + c]).to_f32())
        };
        assert!(at(&with_hud, 48, 30)[..3].iter().all(|&c| c > 0.995));
        let red = at(&with_hud, 30, 40);
        assert!(
            (red[0] - 0xe2 as f32 / 255.0).abs() < 2e-3
                && (red[1] - 0x61 as f32 / 255.0).abs() < 2e-3
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_flat_scene_renders_alone_in_every_output_format_and_picks() {
        let (curve, draws) = flat_board();
        let scene = flat_scene(&curve, &draws);
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut direct = Renderer::new(gpu, 96, 96).unwrap();
        let target = output(direct.gpu(), 96, 96);
        direct.render_flat(&scene, &target.view).unwrap();
        assert_eq!(direct.last_pass_order(), ["flat", "flat ids"]);
        let gamma = direct
            .gpu()
            .readback_rgba16(&target)
            .unwrap()
            .chunks_exact(4)
            .map(|p| std::array::from_fn::<f32, 4, _>(|c| half::f16::from_bits(p[c]).to_f32()))
            .collect::<Vec<_>>();
        let twin = crate::flat::cpu::render(&scene, None, [96, 96]).unwrap();
        for (a, b) in gamma.iter().zip(&twin) {
            for c in 0..4 {
                assert!((a[c] - b[c]).abs() < 2.5 / 255.0, "{a:?} vs {b:?}");
            }
        }
        assert!(gamma[30 * 96 + 48][..3].iter().all(|&c| c > 0.995));
        direct.pick(30, 40).unwrap();
        direct.render_flat(&scene, &target.view).unwrap();
        let picked = direct.picked().unwrap();
        assert_eq!((picked.id, picked.depth), (8, 0.0));
        direct.pick(70, 70).unwrap();
        direct.render_flat(&scene, &target.view).unwrap();
        assert_eq!(direct.picked().unwrap().id, 9);
        direct.pick(2, 2).unwrap();
        direct.render_flat(&scene, &target.view).unwrap();
        assert_eq!(direct.picked().unwrap().id, 0);

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut srgb =
            Renderer::new_with_output_format(gpu, 96, 96, wgpu::TextureFormat::Rgba8UnormSrgb)
                .unwrap();
        let srgb_target = srgb
            .gpu()
            .offscreen(96, 96, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        srgb.render_flat(&scene, &srgb_target.view).unwrap();
        assert_eq!(srgb.last_pass_order(), ["flat", "flat finish", "flat ids"]);
        let bytes = srgb.gpu().readback_rgba8(&srgb_target).unwrap();
        for (pixel, expected) in bytes.chunks_exact(4).zip(&gamma) {
            for c in 0..3 {
                let want = (expected[c].clamp(0.0, 1.0) * 255.0).round();
                assert!(
                    (f32::from(pixel[c]) - want).abs() <= 1.0,
                    "{pixel:?} vs {expected:?}"
                );
            }
        }

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut posted = Renderer::new(gpu, 96, 96).unwrap();
        posted.set_finish(pfx_post::Chain {
            passes: Vec::new(),
            frame: 0,
            seed: 0,
        });
        let posted_target = output(posted.gpu(), 96, 96);
        let mut through_post = scene;
        through_post.post = true;
        posted
            .render_flat(&through_post, &posted_target.view)
            .unwrap();
        assert_eq!(
            posted.last_pass_order(),
            ["flat", "flat finish", "post", "flat ids"]
        );
        let finished = posted.gpu().readback_rgba16(&posted_target).unwrap();
        for (pixel, expected) in finished.chunks_exact(4).zip(&gamma) {
            for c in 0..3 {
                let value = half::f16::from_bits(pixel[c]).to_f32();
                assert!(
                    (value - expected[c]).abs() < 1.5 / 255.0,
                    "{value} vs {expected:?}"
                );
            }
        }
        posted.set_tape(Some(pfx_post::Tape::forward(1.0)));
        let mut glitched = through_post;
        glitched.frame = 7;
        posted.render_flat(&glitched, &posted_target.view).unwrap();
        let taped = posted.gpu().readback_rgba16(&posted_target).unwrap();
        assert!(
            taped != finished,
            "the tape glitch did not reach the flat frame"
        );
    }
}
