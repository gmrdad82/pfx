use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuShortfall {
    NoAdapter,
    Device(String),
    Surface(String),
    Feature(&'static str),
    Limit {
        name: &'static str,
        needed: u64,
        has: u64,
    },
    Format {
        format: wgpu::TextureFormat,
        needs: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuRefusal {
    pub shortfall: GpuShortfall,
    pub adapter: Option<Box<wgpu::AdapterInfo>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatNeed {
    pub format: wgpu::TextureFormat,
    pub usages: wgpu::TextureUsages,
    pub flags: wgpu::TextureFormatFeatureFlags,
    pub needs: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Floor {
    pub features: wgpu::Features,
    pub downlevel: wgpu::DownlevelFlags,
    pub limits: wgpu::Limits,
    pub formats: Vec<FormatNeed>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Capabilities {
    pub features: wgpu::Features,
    pub downlevel: wgpu::DownlevelFlags,
    pub limits: wgpu::Limits,
}

pub const OPTIONAL_FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    .union(wgpu::Features::FLOAT32_FILTERABLE)
    .union(wgpu::Features::TEXTURE_COMPRESSION_BC)
    .union(wgpu::Features::PIPELINE_CACHE);

pub const SAMPLED_TEXTURES_PER_STAGE: u32 = 18;
pub const TEXTURE_SIDE: u32 = 16_384;
pub const TRACE_STORAGE_BUFFERS_PER_STAGE: u32 = 15;
pub const TRACE_BYTES_PER_PIXEL: u64 = 48;

fn need(
    format: wgpu::TextureFormat,
    usages: wgpu::TextureUsages,
    flags: wgpu::TextureFormatFeatureFlags,
    needs: &'static str,
) -> FormatNeed {
    FormatNeed {
        format,
        usages,
        flags,
        needs,
    }
}

pub fn live_floor() -> Floor {
    use wgpu::DownlevelFlags as D;
    use wgpu::TextureFormat as F;
    use wgpu::TextureFormatFeatureFlags as Flags;
    use wgpu::TextureUsages as U;
    let target = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
    let storage = U::STORAGE_BINDING | U::TEXTURE_BINDING;
    Floor {
        features: wgpu::Features::empty(),
        downlevel: D::COMPUTE_SHADERS
            | D::FRAGMENT_STORAGE
            | D::VERTEX_STORAGE
            | D::CUBE_ARRAY_TEXTURES
            | D::COMPARISON_SAMPLERS
            | D::DEPTH_TEXTURE_AND_BUFFER_COPIES
            | D::NON_POWER_OF_TWO_MIPMAPPED_TEXTURES
            | D::FULL_DRAW_INDEX_UINT32
            | D::BUFFER_BINDINGS_NOT_16_BYTE_ALIGNED
            | D::WEBGPU_TEXTURE_FORMAT_SUPPORT
            | D::SHADER_F16_IN_F32,
        limits: wgpu::Limits {
            max_texture_dimension_2d: TEXTURE_SIDE,
            max_sampled_textures_per_shader_stage: SAMPLED_TEXTURES_PER_STAGE,
            ..wgpu::Limits::default()
        },
        formats: vec![
            need(
                F::Rgba16Float,
                target | storage | U::COPY_SRC | U::COPY_DST,
                Flags::FILTERABLE | Flags::BLENDABLE | Flags::STORAGE_WRITE_ONLY,
                "scene colour, history, reflections and the post chain",
            ),
            need(
                F::R32Float,
                target | storage,
                Flags::STORAGE_WRITE_ONLY,
                "linear depth and depth history",
            ),
            need(
                F::R32Uint,
                target | storage,
                Flags::STORAGE_WRITE_ONLY,
                "object ids and id history",
            ),
            need(
                F::Rgba8Snorm,
                storage,
                Flags::STORAGE_WRITE_ONLY,
                "normal history",
            ),
            need(
                F::Rgba8Unorm,
                storage,
                Flags::FILTERABLE | Flags::STORAGE_WRITE_ONLY,
                "the room haze field and material maps",
            ),
            need(
                F::Rgba32Uint,
                target,
                Flags::empty(),
                "the shadow transmission map",
            ),
            need(F::Rg16Float, target, Flags::empty(), "velocity"),
            need(F::R16Float, target, Flags::FILTERABLE, "the contact field"),
            need(
                F::R8Unorm,
                target,
                Flags::FILTERABLE | Flags::BLENDABLE,
                "the reactive mask, canopy cookie and text atlas",
            ),
            need(
                F::Rgba8UnormSrgb,
                target,
                Flags::FILTERABLE,
                "base maps, content and the sRGB output",
            ),
            need(
                F::Rgba32Float,
                U::TEXTURE_BINDING,
                Flags::empty(),
                "impostor geometry",
            ),
            need(
                F::Depth32Float,
                target | U::COPY_SRC | U::COPY_DST,
                Flags::empty(),
                "depth, the shadow atlas and light faces",
            ),
        ],
    }
}

pub fn trace_floor() -> Floor {
    use wgpu::TextureFormat as F;
    use wgpu::TextureFormatFeatureFlags as Flags;
    use wgpu::TextureUsages as U;
    Floor {
        features: wgpu::Features::FLOAT32_FILTERABLE,
        downlevel: wgpu::DownlevelFlags::COMPUTE_SHADERS,
        limits: wgpu::Limits {
            max_storage_buffers_per_shader_stage: TRACE_STORAGE_BUFFERS_PER_STAGE,
            ..wgpu::Limits::default()
        },
        formats: vec![
            need(
                F::Rgba32Float,
                U::STORAGE_BINDING | U::COPY_SRC,
                Flags::STORAGE_WRITE_ONLY,
                "the traced colour, albedo and normal",
            ),
            need(
                F::Rgba32Float,
                U::TEXTURE_BINDING | U::COPY_DST,
                Flags::FILTERABLE,
                "the tracer's environment, sampled linearly",
            ),
        ],
    }
}

pub fn timestamps(features: wgpu::Features) -> bool {
    features.contains(wgpu::Features::TIMESTAMP_QUERY)
}

pub fn shortfall(
    floor: &Floor,
    offered: &Capabilities,
    format_features: impl Fn(wgpu::TextureFormat) -> wgpu::TextureFormatFeatures,
) -> Option<GpuShortfall> {
    if let Some((name, _)) = floor
        .features
        .difference(offered.features)
        .iter_names()
        .next()
    {
        return Some(GpuShortfall::Feature(name));
    }
    if let Some((name, _)) = floor
        .downlevel
        .difference(offered.downlevel)
        .iter_names()
        .next()
    {
        return Some(GpuShortfall::Feature(name));
    }
    let mut short = None;
    floor
        .limits
        .check_limits_with_fail_fn(&offered.limits, true, |name, needed, has| {
            short = Some(GpuShortfall::Limit { name, needed, has });
        });
    if short.is_some() {
        return short;
    }
    floor.formats.iter().find_map(|need| {
        let has = format_features(need.format);
        (!has.allowed_usages.contains(need.usages) || !has.flags.contains(need.flags)).then_some(
            GpuShortfall::Format {
                format: need.format,
                needs: need.needs,
            },
        )
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageBindings {
    pub sampled_textures: u32,
    pub samplers: u32,
    pub storage_buffers: u32,
    pub storage_textures: u32,
    pub uniform_buffers: u32,
}

pub fn stage_bindings(
    groups: &[&[wgpu::BindGroupLayoutEntry]],
    stage: wgpu::ShaderStages,
) -> StageBindings {
    let mut sum = StageBindings::default();
    for entry in groups.iter().flat_map(|group| group.iter()) {
        if !entry.visibility.contains(stage) {
            continue;
        }
        let count = entry.count.map_or(1, |count| count.get());
        let slot = match entry.ty {
            wgpu::BindingType::Texture { .. } => &mut sum.sampled_textures,
            wgpu::BindingType::Sampler(_) => &mut sum.samplers,
            wgpu::BindingType::StorageTexture { .. } => &mut sum.storage_textures,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                ..
            } => &mut sum.uniform_buffers,
            wgpu::BindingType::Buffer { .. } => &mut sum.storage_buffers,
            _ => continue,
        };
        *slot += count;
    }
    sum
}

pub fn layout_shortfall(
    groups: &[&[wgpu::BindGroupLayoutEntry]],
    limits: &wgpu::Limits,
) -> Option<GpuShortfall> {
    let groups_needed = groups.len() as u32;
    if groups_needed > limits.max_bind_groups {
        return Some(GpuShortfall::Limit {
            name: "max_bind_groups",
            needed: u64::from(groups_needed),
            has: u64::from(limits.max_bind_groups),
        });
    }
    [
        wgpu::ShaderStages::VERTEX,
        wgpu::ShaderStages::FRAGMENT,
        wgpu::ShaderStages::COMPUTE,
    ]
    .into_iter()
    .find_map(|stage| {
        let sum = stage_bindings(groups, stage);
        [
            (
                "max_sampled_textures_per_shader_stage",
                sum.sampled_textures,
                limits.max_sampled_textures_per_shader_stage,
            ),
            (
                "max_samplers_per_shader_stage",
                sum.samplers,
                limits.max_samplers_per_shader_stage,
            ),
            (
                "max_storage_buffers_per_shader_stage",
                sum.storage_buffers,
                limits.max_storage_buffers_per_shader_stage,
            ),
            (
                "max_storage_textures_per_shader_stage",
                sum.storage_textures,
                limits.max_storage_textures_per_shader_stage,
            ),
            (
                "max_uniform_buffers_per_shader_stage",
                sum.uniform_buffers,
                limits.max_uniform_buffers_per_shader_stage,
            ),
        ]
        .into_iter()
        .find(|(_, needed, has)| needed > has)
        .map(|(name, needed, has)| GpuShortfall::Limit {
            name,
            needed: u64::from(needed),
            has: u64::from(has),
        })
    })
}

impl GpuRefusal {
    pub fn new(shortfall: GpuShortfall, adapter: Option<wgpu::AdapterInfo>) -> Self {
        Self {
            shortfall,
            adapter: adapter.map(Box::new),
        }
    }
}

impl fmt::Display for GpuShortfall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no usable GPU"),
            Self::Device(error) => write!(f, "the GPU refused a device: {error}"),
            Self::Surface(error) => write!(f, "the GPU can't present to the window: {error}"),
            Self::Feature(name) => write!(f, "the GPU lacks the feature {name}"),
            Self::Limit { name, needed, has } => {
                write!(f, "limit {name} of {needed} is beyond the adapter's {has}")
            }
            Self::Format { format, needs } => {
                write!(f, "the GPU can't use {format:?} for {needs}")
            }
        }
    }
}

pub fn backend_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Noop => "noop",
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Dx12 => "DX12",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
    }
}

impl fmt::Display for GpuRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.adapter {
            Some(info) => write!(
                f,
                "{} ({} {} {}, {})",
                self.shortfall,
                info.name,
                info.driver,
                info.driver_info,
                backend_name(info.backend),
            ),
            None => write!(f, "{}", self.shortfall),
        }
    }
}

impl std::error::Error for GpuRefusal {}

impl From<GpuRefusal> for String {
    fn from(refusal: GpuRefusal) -> Self {
        refusal.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(features: wgpu::Features, limits: wgpu::Limits) -> Capabilities {
        Capabilities {
            features,
            downlevel: wgpu::DownlevelFlags::compliant(),
            limits,
        }
    }

    fn guaranteed(format: wgpu::TextureFormat) -> wgpu::TextureFormatFeatures {
        format.guaranteed_format_features(wgpu::Features::empty())
    }

    fn radv() -> wgpu::Limits {
        wgpu::Limits {
            max_texture_dimension_2d: 32_768,
            max_texture_array_layers: 8_192,
            max_sampled_textures_per_shader_stage: 8_388_606,
            max_storage_buffers_per_shader_stage: 8_388_606,
            max_storage_textures_per_shader_stage: 8_388_606,
            max_bind_groups: 8,
            max_buffer_size: 1 << 40,
            max_storage_buffer_binding_size: u32::MAX,
            ..wgpu::Limits::default()
        }
    }

    #[test]
    fn an_adapter_at_the_floor_passes() {
        let floor = live_floor();
        let at = offered(wgpu::Features::empty(), floor.limits.clone());
        assert_eq!(shortfall(&floor, &at, guaranteed), None);
        let above = offered(OPTIONAL_FEATURES, radv());
        assert_eq!(shortfall(&floor, &above, guaranteed), None);
    }

    #[test]
    fn a_limit_below_the_floor_is_named() {
        let floor = live_floor();
        let short = offered(
            wgpu::Features::all_webgpu_mask(),
            wgpu::Limits {
                max_sampled_textures_per_shader_stage: 16,
                ..radv()
            },
        );
        assert_eq!(
            shortfall(&floor, &short, guaranteed),
            Some(GpuShortfall::Limit {
                name: "max_sampled_textures_per_shader_stage",
                needed: u64::from(SAMPLED_TEXTURES_PER_STAGE),
                has: 16,
            })
        );
        let small = offered(wgpu::Features::empty(), wgpu::Limits::default());
        assert_eq!(
            shortfall(&floor, &small, guaranteed),
            Some(GpuShortfall::Limit {
                name: "max_texture_dimension_2d",
                needed: 16_384,
                has: 8_192,
            })
        );
    }

    #[test]
    fn without_timestamps_the_floor_passes_with_profiling_off() {
        let floor = live_floor();
        let plain = offered(wgpu::Features::FLOAT32_FILTERABLE, radv());
        assert_eq!(shortfall(&floor, &plain, guaranteed), None);
        assert!(!timestamps(plain.features));
        let timed = offered(OPTIONAL_FEATURES, radv());
        assert_eq!(shortfall(&floor, &timed, guaranteed), None);
        assert!(timestamps(timed.features));
    }

    #[test]
    fn a_missing_downlevel_capability_is_named() {
        let floor = live_floor();
        let mut old = offered(wgpu::Features::empty(), radv());
        old.downlevel
            .remove(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES);
        assert_eq!(
            shortfall(&floor, &old, guaranteed),
            Some(GpuShortfall::Feature("CUBE_ARRAY_TEXTURES"))
        );
    }

    #[test]
    fn a_format_without_its_usage_is_named() {
        let floor = live_floor();
        let at = offered(wgpu::Features::empty(), radv());
        let no_storage = |format: wgpu::TextureFormat| {
            let mut features = guaranteed(format);
            if format == wgpu::TextureFormat::R32Float {
                features
                    .allowed_usages
                    .remove(wgpu::TextureUsages::STORAGE_BINDING);
            }
            features
        };
        assert_eq!(
            shortfall(&floor, &at, no_storage),
            Some(GpuShortfall::Format {
                format: wgpu::TextureFormat::R32Float,
                needs: "linear depth and depth history",
            })
        );
    }

    #[test]
    fn the_first_shortfall_wins() {
        let floor = Floor {
            features: wgpu::Features::FLOAT32_FILTERABLE,
            ..live_floor()
        };
        let short = offered(wgpu::Features::empty(), wgpu::Limits::default());
        assert_eq!(
            shortfall(&floor, &short, guaranteed),
            Some(GpuShortfall::Feature("FLOAT32_FILTERABLE"))
        );
    }

    fn filterable(format: wgpu::TextureFormat) -> wgpu::TextureFormatFeatures {
        format.guaranteed_format_features(wgpu::Features::FLOAT32_FILTERABLE)
    }

    #[test]
    fn an_adapter_at_the_trace_floor_passes() {
        let floor = trace_floor();
        let at = offered(floor.features, floor.limits.clone());
        assert_eq!(shortfall(&floor, &at, filterable), None);
        let above = offered(OPTIONAL_FEATURES, radv());
        assert_eq!(shortfall(&floor, &above, filterable), None);
        let defaults = wgpu::Limits::default();
        assert_eq!(
            floor.limits.max_storage_buffer_binding_size,
            defaults.max_storage_buffer_binding_size
        );
        assert_eq!(floor.limits.max_buffer_size, defaults.max_buffer_size);
    }

    #[test]
    fn the_default_limits_lack_only_the_trace_floors_storage_buffers() {
        let floor = trace_floor();
        let plain = offered(wgpu::Features::FLOAT32_FILTERABLE, wgpu::Limits::default());
        assert_eq!(
            shortfall(&floor, &plain, filterable),
            Some(GpuShortfall::Limit {
                name: "max_storage_buffers_per_shader_stage",
                needed: u64::from(TRACE_STORAGE_BUFFERS_PER_STAGE),
                has: 8,
            })
        );
        let enough = offered(
            wgpu::Features::FLOAT32_FILTERABLE,
            wgpu::Limits {
                max_storage_buffers_per_shader_stage: TRACE_STORAGE_BUFFERS_PER_STAGE,
                ..wgpu::Limits::default()
            },
        );
        assert_eq!(shortfall(&floor, &enough, filterable), None);
    }

    #[test]
    fn the_trace_floor_needs_filterable_float32_and_storage_writes() {
        let floor = trace_floor();
        let unfiltered = offered(wgpu::Features::empty(), radv());
        assert_eq!(
            shortfall(&floor, &unfiltered, guaranteed),
            Some(GpuShortfall::Feature("FLOAT32_FILTERABLE"))
        );
        let at = offered(wgpu::Features::FLOAT32_FILTERABLE, radv());
        assert_eq!(
            shortfall(&floor, &at, guaranteed),
            Some(GpuShortfall::Format {
                format: wgpu::TextureFormat::Rgba32Float,
                needs: "the tracer's environment, sampled linearly",
            })
        );
        let no_storage = |format: wgpu::TextureFormat| {
            let mut features = filterable(format);
            features
                .flags
                .remove(wgpu::TextureFormatFeatureFlags::STORAGE_WRITE_ONLY);
            features
        };
        assert_eq!(
            shortfall(&floor, &at, no_storage),
            Some(GpuShortfall::Format {
                format: wgpu::TextureFormat::Rgba32Float,
                needs: "the traced colour, albedo and normal",
            })
        );
        let mut old = offered(wgpu::Features::FLOAT32_FILTERABLE, radv());
        old.downlevel.remove(wgpu::DownlevelFlags::COMPUTE_SHADERS);
        assert_eq!(
            shortfall(&floor, &old, filterable),
            Some(GpuShortfall::Feature("COMPUTE_SHADERS"))
        );
    }

    fn texture(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }
    }

    #[test]
    fn stage_sums_add_every_group_a_stage_sees() {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let one: Vec<_> = (0..10).map(|binding| texture(binding, fragment)).collect();
        let mut two: Vec<_> = (0..8).map(|binding| texture(binding, fragment)).collect();
        two.push(texture(8, wgpu::ShaderStages::VERTEX_FRAGMENT));
        two.push(wgpu::BindGroupLayoutEntry {
            count: std::num::NonZeroU32::new(3),
            ..texture(9, wgpu::ShaderStages::VERTEX)
        });
        let groups = [one.as_slice(), two.as_slice()];
        assert_eq!(stage_bindings(&groups, fragment).sampled_textures, 19);
        assert_eq!(
            stage_bindings(&groups, wgpu::ShaderStages::VERTEX).sampled_textures,
            4
        );
        assert_eq!(
            layout_shortfall(&groups, &wgpu::Limits::default()),
            Some(GpuShortfall::Limit {
                name: "max_sampled_textures_per_shader_stage",
                needed: 19,
                has: 16,
            })
        );
        assert_eq!(
            layout_shortfall(&groups[..1], &wgpu::Limits::default()),
            None
        );
    }

    fn adapter(backend: wgpu::Backend) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: "adapter".into(),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::DiscreteGpu,
            driver: "driver".into(),
            driver_info: "info".into(),
            backend,
        }
    }

    #[test]
    fn a_refusal_names_the_adapter_apart_from_the_shortfall() {
        let refusal = GpuRefusal::new(
            GpuShortfall::Limit {
                name: "max_texture_dimension_2d",
                needed: 16_384,
                has: 8_192,
            },
            Some(wgpu::AdapterInfo {
                name: "llvmpipe".into(),
                ..adapter(wgpu::Backend::Vulkan)
            }),
        );
        assert_eq!(
            refusal.adapter.as_ref().map(|info| info.name.as_str()),
            Some("llvmpipe")
        );
        let text = refusal.to_string();
        assert!(text.contains("max_texture_dimension_2d"), "{text}");
        assert!(text.contains("llvmpipe"), "{text}");
        assert!(text.contains("Vulkan"), "{text}");
        assert_eq!(GpuShortfall::NoAdapter.to_string(), "no usable GPU");
    }

    #[test]
    fn a_refusal_names_vulkan_dx12_and_metal() {
        for (backend, name) in [
            (wgpu::Backend::Vulkan, "Vulkan"),
            (wgpu::Backend::Dx12, "DX12"),
            (wgpu::Backend::Metal, "Metal"),
        ] {
            let text = GpuRefusal::new(
                GpuShortfall::Feature("FLOAT32_FILTERABLE"),
                Some(adapter(backend)),
            )
            .to_string();
            assert!(text.contains(name), "{text}");
            assert_eq!(backend_name(backend), name);
        }
    }

    #[test]
    fn dx12_tier_2_and_3_hold_the_floors_and_tier_1_at_feature_level_11_0_does_not() {
        let live = live_floor().limits;
        let trace = trace_floor().limits;
        let flat = wgpu::Limits::default();
        let tier2_sampled_textures = 1_000_000u32;
        let tier2_sampled_with_ray_tracing = tier2_sampled_textures / 2;
        let tier2_storage_buffers = 16u32;
        let tier2_storage_textures = 16u32;
        let texture_side = 16_384u32;
        assert!(tier2_sampled_with_ray_tracing >= live.max_sampled_textures_per_shader_stage);
        assert!(texture_side >= live.max_texture_dimension_2d);
        assert!(tier2_storage_textures >= live.max_storage_textures_per_shader_stage);
        assert!(tier2_storage_buffers >= trace.max_storage_buffers_per_shader_stage);
        assert!(tier2_sampled_textures >= flat.max_sampled_textures_per_shader_stage);
        assert!(tier2_storage_buffers >= flat.max_storage_buffers_per_shader_stage);
        assert!(tier2_storage_textures >= flat.max_storage_textures_per_shader_stage);
        assert!(texture_side >= flat.max_texture_dimension_2d);
        let tier3_descriptors = 1u32 << 20;
        let tier3_storage = tier3_descriptors / 4;
        assert!(tier3_descriptors / 2 >= live.max_sampled_textures_per_shader_stage);
        assert!(tier3_storage >= trace.max_storage_buffers_per_shader_stage);
        assert!(tier3_storage >= flat.max_storage_buffers_per_shader_stage);
        let tier1_feature_level_11_0_storage = 2u32;
        assert!(tier1_feature_level_11_0_storage < flat.max_storage_buffers_per_shader_stage);
        assert!(tier1_feature_level_11_0_storage < flat.max_storage_textures_per_shader_stage);
        assert!(tier1_feature_level_11_0_storage < trace.max_storage_buffers_per_shader_stage);
        assert!(tier1_feature_level_11_0_storage < live.max_storage_textures_per_shader_stage);
    }
}
