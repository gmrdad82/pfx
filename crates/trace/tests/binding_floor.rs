use naga::valid::{Capabilities, ValidationFlags, Validator};
use pfx_gpu::floor::TRACE_STORAGE_BUFFERS_PER_STAGE;
use pfx_gpu::{GpuShortfall, layout_shortfall, live_floor, stage_bindings, trace_floor, wgpu};
use pfx_trace::gpu::shader;

type Entries = Vec<wgpu::BindGroupLayoutEntry>;

struct Reflected {
    groups: Vec<Entries>,
    storage_formats: Vec<(naga::StorageFormat, naga::StorageAccess)>,
    sampled_floats: usize,
    samplers: usize,
}

fn reflect(lit: bool) -> Reflected {
    let module = naga::front::wgsl::parse_str(&shader(lit)).unwrap();
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .unwrap();
    let index = module
        .entry_points
        .iter()
        .position(|entry| entry.name == "main" && entry.stage == naga::ShaderStage::Compute)
        .expect("the tracer's compute entry main");
    let used = info.get_entry_point(index);
    let mut reflected = Reflected {
        groups: Vec::new(),
        storage_formats: Vec::new(),
        sampled_floats: 0,
        samplers: 0,
    };
    for (handle, global) in module.global_variables.iter() {
        if used[handle].is_empty() {
            continue;
        }
        let Some(binding) = &global.binding else {
            continue;
        };
        let buffer = |ty| wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let ty = match global.space {
            naga::AddressSpace::Uniform => buffer(wgpu::BufferBindingType::Uniform),
            naga::AddressSpace::Storage { access } => buffer(wgpu::BufferBindingType::Storage {
                read_only: !access.contains(naga::StorageAccess::STORE),
            }),
            naga::AddressSpace::Handle => match module.types[global.ty].inner {
                naga::TypeInner::Image {
                    class: naga::ImageClass::Storage { format, access },
                    ..
                } => {
                    reflected.storage_formats.push((format, access));
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    }
                }
                naga::TypeInner::Image {
                    class:
                        naga::ImageClass::Sampled {
                            kind: naga::ScalarKind::Float,
                            ..
                        },
                    ..
                } => {
                    reflected.sampled_floats += 1;
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    }
                }
                naga::TypeInner::Sampler { .. } => {
                    reflected.samplers += 1;
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                }
                ref other => panic!("an unexpected handle in the tracer: {other:?}"),
            },
            space => panic!("an unexpected address space in the tracer: {space:?}"),
        };
        let group = binding.group as usize;
        if reflected.groups.len() <= group {
            reflected.groups.resize_with(group + 1, Vec::new);
        }
        reflected.groups[group].push(wgpu::BindGroupLayoutEntry {
            binding: binding.binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        });
    }
    reflected
}

fn groups(entries: &[Entries]) -> Vec<&[wgpu::BindGroupLayoutEntry]> {
    entries.iter().map(Vec::as_slice).collect()
}

fn storage_buffers(lit: bool) -> u32 {
    stage_bindings(&groups(&reflect(lit).groups), wgpu::ShaderStages::COMPUTE).storage_buffers
}

#[test]
fn both_tracer_layouts_fit_the_trace_floor() {
    let floor = trace_floor();
    for lit in [false, true] {
        let reflected = reflect(lit);
        assert_eq!(
            layout_shortfall(&groups(&reflected.groups), &floor.limits),
            None,
            "the {} tracer needs more than the trace floor",
            if lit { "lit" } else { "unlit" }
        );
    }
}

#[test]
fn the_trace_floor_is_the_most_storage_buffers_the_tracer_binds() {
    assert_eq!(storage_buffers(false), 12);
    assert_eq!(storage_buffers(true), 15);
    assert_eq!(TRACE_STORAGE_BUFFERS_PER_STAGE, storage_buffers(true));
    assert_eq!(
        trace_floor().limits.max_storage_buffers_per_shader_stage,
        TRACE_STORAGE_BUFFERS_PER_STAGE
    );
}

#[test]
fn a_floor_one_short_fails_each_layout() {
    for lit in [false, true] {
        let needed = storage_buffers(lit);
        let short = wgpu::Limits {
            max_storage_buffers_per_shader_stage: needed - 1,
            ..trace_floor().limits
        };
        assert_eq!(
            layout_shortfall(&groups(&reflect(lit).groups), &short),
            Some(GpuShortfall::Limit {
                name: "max_storage_buffers_per_shader_stage",
                needed: u64::from(needed),
                has: u64::from(needed - 1),
            })
        );
    }
    let no_storage_textures = wgpu::Limits {
        max_storage_textures_per_shader_stage: 2,
        ..trace_floor().limits
    };
    assert_eq!(
        layout_shortfall(&groups(&reflect(false).groups), &no_storage_textures),
        Some(GpuShortfall::Limit {
            name: "max_storage_textures_per_shader_stage",
            needed: 3,
            has: 2,
        })
    );
}

#[test]
fn the_live_floor_does_not_cover_the_tracer() {
    assert_eq!(
        layout_shortfall(&groups(&reflect(true).groups), &live_floor().limits),
        Some(GpuShortfall::Limit {
            name: "max_storage_buffers_per_shader_stage",
            needed: u64::from(TRACE_STORAGE_BUFFERS_PER_STAGE),
            has: 8,
        })
    );
}

#[test]
fn the_trace_floor_holds_the_formats_and_filtering_the_tracer_uses() {
    let floor = trace_floor();
    for lit in [false, true] {
        let reflected = reflect(lit);
        assert_eq!(reflected.storage_formats.len(), 3);
        for (format, access) in &reflected.storage_formats {
            assert_eq!(*format, naga::StorageFormat::Rgba32Float);
            assert_eq!(*access, naga::StorageAccess::STORE);
        }
        assert!(reflected.sampled_floats > 0 && reflected.samplers > 0);
    }
    assert!(floor.formats.iter().any(|need| {
        need.format == wgpu::TextureFormat::Rgba32Float
            && need.usages.contains(wgpu::TextureUsages::STORAGE_BINDING)
            && need
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::STORAGE_WRITE_ONLY)
    }));
    assert!(floor.formats.iter().any(|need| {
        need.format == wgpu::TextureFormat::Rgba32Float
            && need.usages.contains(wgpu::TextureUsages::TEXTURE_BINDING)
            && need
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
    }));
    assert!(floor.features.contains(wgpu::Features::FLOAT32_FILTERABLE));
    let defaults = wgpu::Limits::default();
    assert_eq!(
        floor.limits.max_storage_buffer_binding_size, defaults.max_storage_buffer_binding_size,
        "image-sized buffers are the build's check, not the floor's"
    );
    assert_eq!(floor.limits.max_buffer_size, defaults.max_buffer_size);
    assert!(
        floor
            .downlevel
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
    );
}
