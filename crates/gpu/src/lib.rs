pub mod floor;
pub mod pace;
pub mod screens;
pub mod stats;
pub mod window;

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

pub use floor::{
    Capabilities, Floor, FormatNeed, GpuRefusal, GpuShortfall, StageBindings, backend_name,
    layout_shortfall, live_floor, stage_bindings, trace_floor,
};
pub use wgpu;

const QUERY_PAIRS: u32 = 64;
const QUERY_COUNT: u32 = QUERY_PAIRS * 2;
const READ_SLOTS: usize = 3;
const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

#[derive(Clone)]
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
}

pub struct OffscreenTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
}

#[cfg(feature = "window")]
pub struct WindowTarget {
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub present: window::PresentChoice,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PassTiming {
    pub label: String,
    pub milliseconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum EntryKind {
    Pass,
    Mark,
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    label: String,
    kind: EntryKind,
    index: u32,
}

struct Timeline {
    entries: Vec<Entry>,
    used: u32,
    marks: bool,
}

impl Timeline {
    fn new(marks: bool) -> Self {
        Self {
            entries: Vec::new(),
            used: 0,
            marks,
        }
    }

    fn push(&mut self, label: String, kind: EntryKind, width: u32) -> Option<u32> {
        if self.used + width > QUERY_COUNT {
            return None;
        }
        let index = self.used;
        self.used += width;
        self.entries.push(Entry { label, kind, index });
        Some(index)
    }

    fn pass(&mut self, label: String) -> Option<u32> {
        self.push(label, EntryKind::Pass, 2)
    }

    fn mark(&mut self, label: String) -> Option<u32> {
        if !self.marks {
            return None;
        }
        self.push(label, EntryKind::Mark, 1)
    }

    fn take(&mut self) -> (Vec<Entry>, u32) {
        let used = std::mem::take(&mut self.used);
        (std::mem::take(&mut self.entries), used)
    }
}

fn resolve_timings(entries: &[Entry], stamps: &[u64], period: f64) -> Vec<PassTiming> {
    let milliseconds =
        |first: u64, last: u64| last.saturating_sub(first) as f64 * period / 1_000_000.0;
    let mut timings = Vec::with_capacity(entries.len());
    let mut previous_mark: Option<u64> = None;
    for entry in entries {
        let index = entry.index as usize;
        match entry.kind {
            EntryKind::Pass => timings.push(PassTiming {
                label: entry.label.clone(),
                milliseconds: milliseconds(stamps[index], stamps[index + 1]),
            }),
            EntryKind::Mark => {
                let stamp = stamps[index];
                if let Some(earlier) = previous_mark {
                    timings.push(PassTiming {
                        label: entry.label.clone(),
                        milliseconds: milliseconds(earlier, stamp),
                    });
                }
                previous_mark = Some(stamp);
            }
        }
    }
    timings
}

struct ReadSlot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    entries: Vec<Entry>,
    queries: u32,
    busy: bool,
    frame: u64,
}

struct Queries {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<ReadSlot>,
    timeline: Timeline,
    period: f64,
    dropped: u64,
}

pub struct GpuProfiler {
    queries: Option<Queries>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameTimings {
    pub frame: u64,
    pub passes: Vec<PassTiming>,
}

fn engine_backends() -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::VULKAN | wgpu::Backends::DX12
    }
    #[cfg(not(target_os = "windows"))]
    {
        wgpu::Backends::VULKAN | wgpu::Backends::METAL
    }
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: engine_backends(),
        ..Default::default()
    })
}

fn engine_limits(supported: &wgpu::Limits) -> wgpu::Limits {
    let floors = live_floor()
        .limits
        .or_better_values_from(&trace_floor().limits);
    wgpu::Limits {
        max_sampled_textures_per_shader_stage: supported
            .max_sampled_textures_per_shader_stage
            .min(floors.max_sampled_textures_per_shader_stage),
        max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
        max_buffer_size: supported.max_buffer_size,
        max_storage_buffers_per_shader_stage: supported
            .max_storage_buffers_per_shader_stage
            .min(floors.max_storage_buffers_per_shader_stage.max(16)),
        max_storage_textures_per_shader_stage: supported
            .max_storage_textures_per_shader_stage
            .min(floors.max_storage_textures_per_shader_stage),
        ..wgpu::Limits::default()
    }
}

fn default_limits(supported: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_texture_dimension_2d: supported.max_texture_dimension_2d,
        max_texture_array_layers: supported.max_texture_array_layers,
        ..engine_limits(supported)
    }
}

fn requested_limits(
    requested: &wgpu::Limits,
    supported: &wgpu::Limits,
) -> Result<wgpu::Limits, GpuShortfall> {
    let mut refused = None;
    requested.check_limits_with_fail_fn(supported, true, |name, needed, has| {
        refused = Some(GpuShortfall::Limit { name, needed, has });
    });
    match refused {
        Some(shortfall) => Err(shortfall),
        None => Ok(requested
            .clone()
            .or_better_values_from(&engine_limits(supported))),
    }
}

fn windows_backend_order() -> [wgpu::Backends; 2] {
    [wgpu::Backends::VULKAN, wgpu::Backends::DX12]
}

fn preferred_adapter(
    adapters: &[(wgpu::Backend, wgpu::DeviceType)],
    windows: bool,
) -> Option<usize> {
    if windows {
        adapters
            .iter()
            .enumerate()
            .min_by_key(|(_, (backend, device))| {
                (
                    match backend {
                        wgpu::Backend::Vulkan => 0u8,
                        wgpu::Backend::Dx12 => 1,
                        _ => 2,
                    },
                    u8::from(*device != wgpu::DeviceType::DiscreteGpu),
                )
            })
            .map(|(index, _)| index)
    } else {
        adapters
            .iter()
            .position(|(_, device)| *device == wgpu::DeviceType::DiscreteGpu)
    }
}

fn choose_from(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
    backends: wgpu::Backends,
    windows: bool,
) -> Option<wgpu::Adapter> {
    let mut adapters: Vec<_> = instance
        .enumerate_adapters(backends)
        .into_iter()
        .filter(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
        .collect();
    let offers: Vec<_> = adapters
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            (info.backend, info.device_type)
        })
        .collect();
    preferred_adapter(&offers, windows).map(|index| adapters.swap_remove(index))
}

fn pick_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
) -> Option<wgpu::Adapter> {
    if cfg!(target_os = "windows") {
        for backends in windows_backend_order() {
            if let Some(adapter) = choose_from(instance, surface, backends, true) {
                return Some(adapter);
            }
        }
        None
    } else {
        choose_from(instance, surface, engine_backends(), false)
    }
}

fn device_format_features(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::TextureFormatFeatures {
    let features = device.features();
    if features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
        adapter.get_texture_format_features(format)
    } else {
        format.guaranteed_format_features(features)
    }
}

async fn open(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
    limits: Option<&wgpu::Limits>,
    floor: Option<&Floor>,
) -> Result<Gpu, GpuRefusal> {
    let adapter = match pick_adapter(instance, surface) {
        Some(adapter) => adapter,
        None => instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: surface,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| GpuRefusal::new(GpuShortfall::NoAdapter, None))?,
    };
    let info = adapter.get_info();
    let refuse = |shortfall| GpuRefusal::new(shortfall, Some(info.clone()));
    let available = adapter.features();
    let supported = adapter.limits();
    let offered = Capabilities {
        features: available,
        downlevel: adapter.get_downlevel_capabilities().flags,
        limits: supported.clone(),
    };
    if let Some(floor) = floor
        && let Some(shortfall) = floor::shortfall(floor, &offered, |format| {
            adapter.get_texture_format_features(format)
        })
    {
        return Err(refuse(shortfall));
    }
    let required_features = available & floor::OPTIONAL_FEATURES;
    let required_limits = match limits {
        Some(requested) => requested_limits(requested, &supported).map_err(refuse)?,
        None => default_limits(&supported),
    };
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("pfx"),
            required_features,
            required_limits,
            ..Default::default()
        })
        .await
        .map_err(|error| refuse(GpuShortfall::Device(error.to_string())))?;
    let gpu = Gpu {
        adapter,
        device,
        queue,
        info,
    };
    if let Some(floor) = floor {
        gpu.check(floor)?;
    }
    Ok(gpu)
}

pub fn format_bytes_per_pixel(format: wgpu::TextureFormat) -> Option<u32> {
    match format {
        wgpu::TextureFormat::Rgba8UnormSrgb => Some(4),
        wgpu::TextureFormat::Rgba16Float => Some(8),
        wgpu::TextureFormat::Rgba32Float => Some(16),
        _ => None,
    }
}

pub fn padded_bytes_per_row(width: u32, format: wgpu::TextureFormat) -> Option<u32> {
    let bytes = width.checked_mul(format_bytes_per_pixel(format)?)?;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    bytes
        .checked_add(alignment - 1)
        .map(|n| n / alignment * alignment)
}

fn image_bytes(width: u32, height: u32, format: wgpu::TextureFormat) -> Result<usize, String> {
    if width == 0 || height == 0 {
        return Err("image dimensions must be nonzero".into());
    }
    let bpp = format_bytes_per_pixel(format).ok_or("unsupported target format")?;
    usize::try_from(u64::from(width) * u64::from(height) * u64::from(bpp))
        .map_err(|error| error.to_string())
}

impl Gpu {
    pub async fn headless() -> Result<Self, String> {
        Ok(open(&instance(), None, None, None).await?)
    }

    pub async fn headless_with(limits: wgpu::Limits) -> Result<Self, String> {
        Ok(open(&instance(), None, Some(&limits), None).await?)
    }

    pub async fn headless_checked() -> Result<Self, GpuRefusal> {
        open(&instance(), None, None, Some(&live_floor())).await
    }

    #[cfg(feature = "window")]
    pub async fn window(
        window: Arc<winit::window::Window>,
    ) -> Result<(Self, WindowTarget), String> {
        Ok(Self::open_window(window, None, None, &window::PresentPreference::fifo()).await?)
    }

    #[cfg(feature = "window")]
    pub async fn window_with(
        window: Arc<winit::window::Window>,
        limits: wgpu::Limits,
    ) -> Result<(Self, WindowTarget), String> {
        Ok(Self::open_window(
            window,
            Some(&limits),
            None,
            &window::PresentPreference::fifo(),
        )
        .await?)
    }

    #[cfg(feature = "window")]
    pub async fn window_checked(
        window: Arc<winit::window::Window>,
    ) -> Result<(Self, WindowTarget), GpuRefusal> {
        Self::open_window(
            window,
            None,
            Some(&live_floor()),
            &window::PresentPreference::fifo(),
        )
        .await
    }

    #[cfg(feature = "window")]
    pub async fn window_present(
        window: Arc<winit::window::Window>,
        present: window::PresentPreference,
    ) -> Result<(Self, WindowTarget), String> {
        Ok(Self::open_window(window, None, None, &present).await?)
    }

    #[cfg(feature = "window")]
    async fn open_window(
        window: Arc<winit::window::Window>,
        limits: Option<&wgpu::Limits>,
        floor: Option<&Floor>,
        preference: &window::PresentPreference,
    ) -> Result<(Self, WindowTarget), GpuRefusal> {
        let instance = instance();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| GpuRefusal::new(GpuShortfall::Surface(error.to_string()), None))?;
        let gpu = open(&instance, Some(&surface), limits, floor).await?;
        let refuse = |message: &str| {
            GpuRefusal::new(
                GpuShortfall::Surface(message.into()),
                Some(gpu.info.clone()),
            )
        };
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| refuse("surface has no supported formats"))?;
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::CompositeAlphaMode::Opaque)
            .or_else(|| caps.alpha_modes.first().copied())
            .ok_or_else(|| refuse("surface has no supported alpha modes"))?;
        let size = window.inner_size();
        let present = window::choose_present(preference, &caps.present_modes);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: present.selected,
            desired_maximum_frame_latency: present.desired_maximum_frame_latency,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        Ok((
            gpu,
            WindowTarget {
                surface,
                config,
                present,
            },
        ))
    }

    pub fn check_floor(&self) -> Result<(), GpuRefusal> {
        self.check(&live_floor())
    }

    pub fn check(&self, floor: &Floor) -> Result<(), GpuRefusal> {
        let offered = Capabilities {
            features: self.device.features(),
            downlevel: self.adapter.get_downlevel_capabilities().flags,
            limits: self.device.limits(),
        };
        match floor::shortfall(floor, &offered, |format| {
            device_format_features(&self.adapter, &self.device, format)
        }) {
            Some(shortfall) => Err(GpuRefusal::new(shortfall, Some(self.info.clone()))),
            None => Ok(()),
        }
    }

    pub fn profiling(&self) -> bool {
        floor::timestamps(self.device.features())
    }

    pub fn offscreen(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<OffscreenTarget, String> {
        image_bytes(width, height, format)?;
        let supported = self.adapter.get_texture_format_features(format);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        if !supported.allowed_usages.contains(usage) {
            return Err(format!(
                "adapter cannot use {format:?} as an offscreen target"
            ));
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pfx offscreen"),
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
        Ok(OffscreenTarget {
            texture,
            view,
            format,
            width,
            height,
        })
    }

    pub fn upload_rgba8(&self, target: &OffscreenTarget, pixels: &[u8]) -> Result<(), String> {
        if target.format != wgpu::TextureFormat::Rgba8UnormSrgb {
            return Err("RGBA8 upload requires an RGBA8 target".into());
        }
        self.upload_bytes(target, pixels)
    }

    pub fn upload_rgba16(&self, target: &OffscreenTarget, pixels: &[u16]) -> Result<(), String> {
        if target.format != wgpu::TextureFormat::Rgba16Float {
            return Err("RGBA16 upload requires an RGBA16 target".into());
        }
        if pixels.len() * 2 != image_bytes(target.width, target.height, target.format)? {
            return Err("RGBA16 image length does not match target".into());
        }
        let bytes: Vec<u8> = pixels
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.upload_bytes(target, &bytes)
    }

    fn upload_bytes(&self, target: &OffscreenTarget, pixels: &[u8]) -> Result<(), String> {
        if pixels.len() != image_bytes(target.width, target.height, target.format)? {
            return Err("image length does not match target".into());
        }
        let row = target.width * format_bytes_per_pixel(target.format).unwrap();
        self.queue.write_texture(
            target.texture.as_image_copy(),
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(target.height),
            },
            target.texture.size(),
        );
        Ok(())
    }

    pub fn readback_bytes(&self, target: &OffscreenTarget) -> Result<Vec<u8>, String> {
        let row = padded_bytes_per_row(target.width, target.format).ok_or("row size overflow")?;
        let raw_row = target.width * format_bytes_per_pixel(target.format).unwrap();
        let size = u64::from(row) * u64::from(target.height);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pfx readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx readback"),
            });
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(target.height),
                },
            },
            target.texture.size(),
        );
        self.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("{error:?}"))?;
        receive
            .recv()
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range();
        let mut bytes =
            Vec::with_capacity(image_bytes(target.width, target.height, target.format)?);
        for padded in mapped.chunks_exact(row as usize) {
            bytes.extend_from_slice(&padded[..raw_row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(bytes)
    }

    pub fn readback_rgba8(&self, target: &OffscreenTarget) -> Result<Vec<u8>, String> {
        if target.format != wgpu::TextureFormat::Rgba8UnormSrgb {
            return Err("RGBA8 readback requires an RGBA8 target".into());
        }
        self.readback_bytes(target)
    }

    pub fn readback_rgba16(&self, target: &OffscreenTarget) -> Result<Vec<u16>, String> {
        if target.format != wgpu::TextureFormat::Rgba16Float {
            return Err("RGBA16 readback requires an RGBA16 target".into());
        }
        Ok(self
            .readback_bytes(target)?
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect())
    }
}

#[cfg(feature = "window")]
impl WindowTarget {
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn set_present(
        &mut self,
        gpu: &Gpu,
        preference: &window::PresentPreference,
    ) -> window::PresentChoice {
        let caps = self.surface.get_capabilities(&gpu.adapter);
        let choice = window::choose_present(preference, &caps.present_modes);
        self.config.present_mode = choice.selected;
        self.config.desired_maximum_frame_latency = choice.desired_maximum_frame_latency;
        self.present = choice.clone();
        self.surface.configure(&gpu.device, &self.config);
        choice
    }
}

impl GpuProfiler {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        if !floor::timestamps(device.features()) {
            return Self { queries: None };
        }
        let size = u64::from(QUERY_COUNT) * 8;
        let set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pfx timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pfx timestamp resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..READ_SLOTS)
            .map(|_| ReadSlot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pfx timestamp read"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
                entries: Vec::new(),
                queries: 0,
                busy: false,
                frame: 0,
            })
            .collect();
        Self {
            queries: Some(Queries {
                set,
                resolve,
                slots,
                timeline: Timeline::new(
                    device
                        .features()
                        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
                ),
                period: f64::from(queue.get_timestamp_period()),
                dropped: 0,
            }),
        }
    }

    pub fn read_slots(&self) -> usize {
        self.queries
            .as_ref()
            .map_or(0, |queries| queries.slots.len())
    }

    pub fn in_flight(&self) -> usize {
        self.queries.as_ref().map_or(0, |queries| {
            queries.slots.iter().filter(|slot| slot.busy).count()
        })
    }

    pub fn dropped(&self) -> u64 {
        self.queries.as_ref().map_or(0, |queries| queries.dropped)
    }

    pub fn pass(&mut self, label: impl Into<String>) -> Option<u32> {
        self.queries.as_mut()?.timeline.pass(label.into())
    }

    pub fn marks_supported(&self) -> bool {
        self.queries
            .as_ref()
            .is_some_and(|queries| queries.timeline.marks)
    }

    pub fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: impl Into<String>) {
        let Some(queries) = self.queries.as_mut() else {
            return;
        };
        if let Some(index) = queries.timeline.mark(label.into()) {
            encoder.write_timestamp(&queries.set, index);
        }
    }

    pub fn compute_writes(
        &self,
        index: Option<u32>,
    ) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let queries = self.queries.as_ref()?;
        index.map(|index| wgpu::ComputePassTimestampWrites {
            query_set: &queries.set,
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: Some(index + 1),
        })
    }

    pub fn render_writes(&self, index: Option<u32>) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let queries = self.queries.as_ref()?;
        index.map(|index| wgpu::RenderPassTimestampWrites {
            query_set: &queries.set,
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: Some(index + 1),
        })
    }

    pub fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) -> Option<usize> {
        self.finish_frame(encoder, 0)
    }

    pub fn finish_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: u64,
    ) -> Option<usize> {
        let queries = self.queries.as_mut()?;
        if queries.timeline.entries.is_empty() {
            return None;
        }
        let slot = queries.slots.iter().position(|slot| !slot.busy);
        let (entries, count) = queries.timeline.take();
        let Some(slot_index) = slot else {
            queries.dropped += 1;
            return None;
        };
        encoder.resolve_query_set(&queries.set, 0..count, &queries.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &queries.resolve,
            0,
            &queries.slots[slot_index].buffer,
            0,
            u64::from(count) * 8,
        );
        queries.slots[slot_index].entries = entries;
        queries.slots[slot_index].queries = count;
        queries.slots[slot_index].busy = true;
        queries.slots[slot_index].frame = frame;
        Some(slot_index)
    }

    pub fn submitted(&mut self, slot: usize) {
        let Some(read) = self
            .queries
            .as_mut()
            .and_then(|queries| queries.slots.get_mut(slot))
        else {
            return;
        };
        if !read.busy || read.state.load(Ordering::Acquire) != IDLE {
            return;
        }
        read.state.store(MAPPING, Ordering::Release);
        let state = read.state.clone();
        read.buffer.slice(..u64::from(read.queries) * 8).map_async(
            wgpu::MapMode::Read,
            move |result| {
                state.store(
                    if result.is_ok() { READY } else { FAILED },
                    Ordering::Release,
                );
            },
        );
    }

    pub fn collect(&mut self, device: &wgpu::Device) -> Vec<Vec<PassTiming>> {
        self.collect_frames(device)
            .into_iter()
            .map(|frame| frame.passes)
            .collect()
    }

    pub fn collect_frames(&mut self, device: &wgpu::Device) -> Vec<FrameTimings> {
        let _ = device.poll(wgpu::PollType::Poll);
        self.gather()
    }

    pub fn gather(&mut self) -> Vec<FrameTimings> {
        let Some(queries) = self.queries.as_mut() else {
            return Vec::new();
        };
        let mut frames = Vec::new();
        for slot in &mut queries.slots {
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let timings = {
                        let data = slot
                            .buffer
                            .slice(..u64::from(slot.queries) * 8)
                            .get_mapped_range();
                        let stamps: Vec<u64> = data
                            .chunks_exact(8)
                            .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
                            .collect();
                        resolve_timings(&slot.entries, &stamps, queries.period)
                    };
                    slot.buffer.unmap();
                    slot.busy = false;
                    slot.entries.clear();
                    slot.state.store(IDLE, Ordering::Release);
                    frames.push(FrameTimings {
                        frame: slot.frame,
                        passes: timings,
                    });
                }
                FAILED => {
                    slot.busy = false;
                    slot.entries.clear();
                    slot.state.store(IDLE, Ordering::Release);
                    queries.dropped += 1;
                }
                _ => {}
            }
        }
        frames.sort_by_key(|frame| frame.frame);
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_takes_vulkan_before_dx12_and_other_targets_keep_the_first_discrete() {
        let discrete = wgpu::DeviceType::DiscreteGpu;
        let integrated = wgpu::DeviceType::IntegratedGpu;
        let dx12_then_vulkan = [
            (wgpu::Backend::Dx12, discrete),
            (wgpu::Backend::Vulkan, discrete),
        ];
        assert_eq!(
            windows_backend_order(),
            [wgpu::Backends::VULKAN, wgpu::Backends::DX12]
        );
        assert_eq!(preferred_adapter(&dx12_then_vulkan, true), Some(1));
        assert_eq!(preferred_adapter(&dx12_then_vulkan, false), Some(0));
        assert_eq!(
            preferred_adapter(
                &[
                    (wgpu::Backend::Dx12, discrete),
                    (wgpu::Backend::Vulkan, integrated),
                ],
                true,
            ),
            Some(1)
        );
        assert_eq!(
            preferred_adapter(
                &[
                    (wgpu::Backend::Dx12, integrated),
                    (wgpu::Backend::Dx12, discrete),
                ],
                true,
            ),
            Some(1)
        );
        assert_eq!(
            preferred_adapter(
                &[
                    (wgpu::Backend::Vulkan, discrete),
                    (wgpu::Backend::Vulkan, discrete),
                ],
                true,
            ),
            Some(0)
        );
        assert_eq!(preferred_adapter(&[], true), None);
        assert_eq!(
            preferred_adapter(&[(wgpu::Backend::Vulkan, integrated)], false),
            None
        );
    }

    #[test]
    fn row_sizes_and_formats() {
        assert_eq!(
            format_bytes_per_pixel(wgpu::TextureFormat::Rgba8UnormSrgb),
            Some(4)
        );
        assert_eq!(
            format_bytes_per_pixel(wgpu::TextureFormat::Rgba16Float),
            Some(8)
        );
        assert_eq!(
            format_bytes_per_pixel(wgpu::TextureFormat::Rgba32Float),
            Some(16)
        );
        assert_eq!(format_bytes_per_pixel(wgpu::TextureFormat::R8Unorm), None);
        assert_eq!(
            padded_bytes_per_row(1, wgpu::TextureFormat::Rgba8UnormSrgb),
            Some(256)
        );
        assert_eq!(
            padded_bytes_per_row(64, wgpu::TextureFormat::Rgba8UnormSrgb),
            Some(256)
        );
        assert_eq!(
            padded_bytes_per_row(65, wgpu::TextureFormat::Rgba8UnormSrgb),
            Some(512)
        );
        assert_eq!(
            padded_bytes_per_row(33, wgpu::TextureFormat::Rgba16Float),
            Some(512)
        );
        assert_eq!(
            padded_bytes_per_row(17, wgpu::TextureFormat::Rgba32Float),
            Some(512)
        );
        assert_eq!(
            padded_bytes_per_row(u32::MAX, wgpu::TextureFormat::Rgba32Float),
            None
        );
    }

    #[test]
    fn marks_become_intervals_named_after_the_later_mark() {
        let mut timeline = Timeline::new(true);
        assert_eq!(timeline.mark("a".into()), Some(0));
        assert_eq!(timeline.mark("b".into()), Some(1));
        assert_eq!(timeline.mark("c".into()), Some(2));
        let (entries, used) = timeline.take();
        assert_eq!(used, 3);
        let timings = resolve_timings(&entries, &[1_000_000, 4_000_000, 10_000_000], 1.0);
        assert_eq!(
            timings,
            vec![
                PassTiming {
                    label: "b".into(),
                    milliseconds: 3.0
                },
                PassTiming {
                    label: "c".into(),
                    milliseconds: 6.0
                },
            ]
        );
    }

    #[test]
    fn a_single_mark_gives_no_timing() {
        let mut timeline = Timeline::new(true);
        timeline.mark("a".into());
        let (entries, _) = timeline.take();
        assert!(resolve_timings(&entries, &[5], 1.0).is_empty());
    }

    #[test]
    fn marks_and_passes_mix_in_call_order() {
        let mut timeline = Timeline::new(true);
        assert_eq!(timeline.mark("start".into()), Some(0));
        assert_eq!(timeline.pass("vis".into()), Some(1));
        assert_eq!(timeline.mark("trace".into()), Some(3));
        assert_eq!(timeline.pass("post".into()), Some(4));
        assert_eq!(timeline.mark("blit".into()), Some(6));
        let (entries, used) = timeline.take();
        assert_eq!(used, 7);
        let stamps = [0, 100, 300, 1_000, 2_000, 2_500, 4_000];
        let timings = resolve_timings(&entries, &stamps, 1_000_000.0);
        let summary: Vec<(&str, f64)> = timings
            .iter()
            .map(|timing| (timing.label.as_str(), timing.milliseconds))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("vis", 200.0),
                ("trace", 1_000.0),
                ("post", 500.0),
                ("blit", 3_000.0),
            ]
        );
    }

    #[test]
    fn passes_resolve_like_before() {
        let mut timeline = Timeline::new(false);
        assert_eq!(timeline.pass("a".into()), Some(0));
        assert_eq!(timeline.pass("b".into()), Some(2));
        let (entries, used) = timeline.take();
        assert_eq!(used, 4);
        let timings = resolve_timings(&entries, &[10, 30, 30, 90], 2.0);
        assert_eq!(timings[0].milliseconds, 40.0 / 1_000_000.0);
        assert_eq!(timings[1].milliseconds, 120.0 / 1_000_000.0);
    }

    #[test]
    fn marks_without_the_feature_record_nothing() {
        let mut timeline = Timeline::new(false);
        assert_eq!(timeline.mark("a".into()), None);
        assert_eq!(timeline.mark("b".into()), None);
        assert_eq!(timeline.pass("p".into()), Some(0));
        let (entries, used) = timeline.take();
        assert_eq!(used, 2);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn capacity_counts_queries_and_resets_after_take() {
        let mut timeline = Timeline::new(true);
        for _ in 0..QUERY_PAIRS {
            assert!(timeline.pass("p".into()).is_some());
        }
        assert_eq!(timeline.pass("p".into()), None);
        assert_eq!(timeline.mark("m".into()), None);
        timeline.take();
        assert_eq!(timeline.mark("m".into()), Some(0));
        for _ in 0..QUERY_COUNT - 1 {
            assert!(timeline.mark("m".into()).is_some());
        }
        assert_eq!(timeline.mark("m".into()), None);
        assert_eq!(timeline.pass("p".into()), None);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn device_and_readback() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let rgba8 = gpu
            .offscreen(65, 3, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let bytes = vec![137; 65 * 3 * 4];
        gpu.upload_rgba8(&rgba8, &bytes).unwrap();
        assert_eq!(gpu.readback_rgba8(&rgba8).unwrap(), bytes);
        let rgba16 = gpu
            .offscreen(33, 3, wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let halves = vec![0x3c00; 33 * 3 * 4];
        gpu.upload_rgba16(&rgba16, &halves).unwrap();
        assert_eq!(gpu.readback_rgba16(&rgba16).unwrap(), halves);
        if gpu
            .offscreen(17, 2, wgpu::TextureFormat::Rgba32Float)
            .is_ok()
        {
            assert_eq!(
                padded_bytes_per_row(17, wgpu::TextureFormat::Rgba32Float),
                Some(512)
            );
        }
    }

    #[test]
    fn default_limits_take_the_adapter_texture_sizes() {
        let supported = wgpu::Limits {
            max_texture_dimension_2d: 16_384,
            max_texture_array_layers: 2_048,
            max_buffer_size: 1 << 33,
            ..wgpu::Limits::default()
        };
        let limits = default_limits(&supported);
        assert_eq!(limits.max_texture_dimension_2d, 16_384);
        assert_eq!(limits.max_texture_array_layers, 2_048);
        assert_eq!(limits.max_buffer_size, 1 << 33);
        assert_eq!(
            limits.max_bind_groups,
            wgpu::Limits::default().max_bind_groups
        );
    }

    #[test]
    fn the_device_gets_the_floor_wherever_the_adapter_offers_it() {
        let supported = wgpu::Limits {
            max_texture_dimension_2d: 32_768,
            max_sampled_textures_per_shader_stage: 1_000_000,
            max_storage_buffers_per_shader_stage: 1_000_000,
            ..wgpu::Limits::default()
        };
        let floor = live_floor();
        assert!(floor.limits.check_limits(&default_limits(&supported)));
        let requested = wgpu::Limits {
            max_texture_dimension_2d: 16_384,
            ..wgpu::Limits::default()
        };
        assert!(
            floor
                .limits
                .check_limits(&requested_limits(&requested, &supported).unwrap())
        );
    }

    #[test]
    fn the_device_gets_the_trace_floor_wherever_the_adapter_offers_it() {
        let supported = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 1_000_000,
            max_storage_buffer_binding_size: u32::MAX,
            max_buffer_size: 1 << 40,
            ..wgpu::Limits::default()
        };
        let floor = trace_floor();
        assert!(floor.limits.check_limits(&default_limits(&supported)));
        assert!(
            floor
                .limits
                .check_limits(&requested_limits(&wgpu::Limits::default(), &supported).unwrap())
        );
        let at = floor.limits.clone();
        assert!(floor.limits.check_limits(&default_limits(&at)));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn this_machine_meets_the_live_floor() {
        let gpu = pollster::block_on(Gpu::headless_checked()).unwrap();
        gpu.check_floor().unwrap();
        let plain = pollster::block_on(Gpu::headless()).unwrap();
        plain.check_floor().unwrap();
        gpu.check(&trace_floor()).unwrap();
        plain.check(&trace_floor()).unwrap();
        println!(
            "{} ({} {}), profiling {}",
            gpu.info.name,
            gpu.info.driver,
            gpu.info.driver_info,
            gpu.profiling()
        );
    }

    #[test]
    fn a_request_beyond_the_adapter_names_the_limit() {
        let supported = wgpu::Limits::default();
        let request = wgpu::Limits {
            max_texture_dimension_2d: supported.max_texture_dimension_2d + 1,
            ..wgpu::Limits::default()
        };
        let error = requested_limits(&request, &supported).unwrap_err();
        assert!(
            matches!(
                error,
                GpuShortfall::Limit {
                    name: "max_texture_dimension_2d",
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_request_within_the_adapter_keeps_the_engine_baseline() {
        let supported = wgpu::Limits {
            max_texture_dimension_2d: 16_384,
            max_buffer_size: 1 << 33,
            ..wgpu::Limits::default()
        };
        let request = wgpu::Limits {
            max_texture_dimension_2d: 12_000,
            ..wgpu::Limits::default()
        };
        let limits = requested_limits(&request, &supported).unwrap();
        assert_eq!(limits.max_texture_dimension_2d, 12_000);
        assert_eq!(limits.max_buffer_size, 1 << 33);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn the_default_reports_the_adapter_texture_maximum() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let supported = gpu.adapter.limits();
        let limits = gpu.device.limits();
        assert_eq!(
            limits.max_texture_dimension_2d,
            supported.max_texture_dimension_2d
        );
        assert_eq!(
            limits.max_texture_array_layers,
            supported.max_texture_array_layers
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn headless_with_honours_a_requested_limit() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let supported = gpu.adapter.limits();
        let wanted = supported.max_texture_dimension_2d.min(10_000);
        drop(gpu);
        let gpu = pollster::block_on(Gpu::headless_with(wgpu::Limits {
            max_texture_dimension_2d: wanted,
            ..wgpu::Limits::default()
        }))
        .unwrap();
        assert_eq!(gpu.device.limits().max_texture_dimension_2d, wanted);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn headless_with_refuses_a_limit_beyond_the_adapter() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let beyond = gpu.adapter.limits().max_texture_dimension_2d + 1;
        drop(gpu);
        let error = pollster::block_on(Gpu::headless_with(wgpu::Limits {
            max_texture_dimension_2d: beyond,
            ..wgpu::Limits::default()
        }))
        .err()
        .unwrap();
        assert!(error.contains("max_texture_dimension_2d"), "{error}");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn timing() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let index = profiler.pass("clear");
        let target = gpu
            .offscreen(8, 8, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: profiler.render_writes(index),
                occlusion_query_set: None,
            });
        }
        let slot = profiler.finish(&mut encoder);
        gpu.queue.submit(Some(encoder.finish()));
        if let Some(slot) = slot {
            profiler.submitted(slot);
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let frames = profiler.collect(&gpu.device);
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0][0].label, "clear");
            assert!(frames[0][0].milliseconds.is_finite());
        } else {
            assert!(index.is_none());
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_full_ring_drops_the_frame_and_late_frames_come_back_tagged() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        if profiler.read_slots() == 0 {
            return;
        }
        let target = gpu
            .offscreen(8, 8, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let mut slots = Vec::new();
        for frame in 0..=READ_SLOTS as u64 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let index = profiler.pass("clear");
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: profiler.render_writes(index),
                occlusion_query_set: None,
            }));
            let slot = profiler.finish_frame(&mut encoder, 40 + frame);
            gpu.queue.submit(Some(encoder.finish()));
            slots.push(slot);
        }
        assert!(slots[..READ_SLOTS].iter().all(Option::is_some));
        assert!(slots[READ_SLOTS].is_none());
        assert_eq!(profiler.dropped(), 1);
        assert_eq!(profiler.in_flight(), READ_SLOTS);
        for slot in slots.into_iter().flatten() {
            profiler.submitted(slot);
        }
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let frames = profiler.collect_frames(&gpu.device);
        assert_eq!(
            frames.iter().map(|frame| frame.frame).collect::<Vec<_>>(),
            (40..40 + READ_SLOTS as u64).collect::<Vec<_>>()
        );
        assert_eq!(profiler.in_flight(), 0);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn marks_around_two_dispatches() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        if !profiler.marks_supported() {
            return;
        }
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("marks"),
                source: wgpu::ShaderSource::Wgsl("@compute @workgroup_size(1) fn main() {}".into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("marks"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        profiler.mark(&mut encoder, "start");
        for _ in 0..2 {
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.dispatch_workgroups(1, 1, 1);
            }
            profiler.mark(&mut encoder, "dispatch");
        }
        let slot = profiler.finish(&mut encoder).unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        profiler.submitted(slot);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let frames = profiler.collect(&gpu.device);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 2);
        for timing in &frames[0] {
            assert_eq!(timing.label, "dispatch");
            assert!(timing.milliseconds.is_finite());
            assert!(timing.milliseconds >= 0.0);
        }
    }
}
