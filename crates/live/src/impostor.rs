use bytemuck::{Pod, Zeroable};
use pfx_bake::impostor::{PropArtifact, PropManifest, read_prop_artifact};
use pfx_gpu::Gpu;
use std::path::Path;

pub const IMPOSTOR_WGSL: &str = r#"
struct Params {
    view_projection: mat4x4f,
    camera_position: vec4f,
    camera_right: vec4f,
    camera_up: vec4f,
    settings: vec4f,
    source: vec4f,
}
struct Instance {
    center_radius: vec4f,
    options: vec4u,
}
struct VertexOut {
    @builtin(position) clip: vec4f,
    @location(0) local: vec2f,
    @location(1) @interpolate(flat) instance_id: u32,
}
struct FragmentOut {
    @location(0) color: vec4f,
    @builtin(frag_depth) depth: f32,
}
struct Shade {
    color: vec4f,
    depth: f32,
}
struct Sample {
    color: vec4f,
    position: vec3f,
    weight: f32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var radiance: texture_2d_array<f32>;
@group(0) @binding(3) var geometry: texture_2d<f32>;
@group(0) @binding(4) var linear_sampler: sampler;
fn oct_decode(p: vec2f) -> vec3f {
    let y = 1.0 - abs(p.x) - abs(p.y);
    if y < 0.0 {
        return normalize(vec3f((1.0 - abs(p.y)) * select(-1.0, 1.0, p.x >= 0.0), y, (1.0 - abs(p.x)) * select(-1.0, 1.0, p.y >= 0.0)));
    }
    return normalize(vec3f(p.x, y, p.y));
}
fn oct_encode(d: vec3f) -> vec2f {
    let p = d.xz / max(abs(d.x) + abs(d.y) + abs(d.z), 1e-20);
    if d.y < 0.0 {
        return vec2f((1.0 - abs(p.y)) * select(-1.0, 1.0, p.x >= 0.0), (1.0 - abs(p.x)) * select(-1.0, 1.0, p.y >= 0.0));
    }
    return p;
}
fn frame_basis(direction: vec3f) -> mat3x3f {
    let forward = -direction;
    var nominal_up = vec3f(0.0, 1.0, 0.0);
    if abs(direction.y) > 0.95 { nominal_up = vec3f(0.0, 0.0, 1.0); }
    let right = normalize(cross(forward, nominal_up));
    let up = normalize(cross(right, forward));
    return mat3x3f(right, up, forward);
}
fn hash(pixel: vec2u, object_id: u32) -> f32 {
    var value = pixel.x * 1664525u + pixel.y * 1013904223u + object_id * 747796405u;
    value = (value ^ (value >> 16u)) * 2246822519u;
    value = (value ^ (value >> 13u)) * 3266489917u;
    return f32(value ^ (value >> 16u)) * (1.0 / 4294967296.0);
}
fn geometry_sample(uv: vec2f) -> vec4f {
    let size = i32(params.settings.x * params.settings.y);
    return textureLoad(geometry, clamp(vec2i(uv * f32(size)), vec2i(0), vec2i(size - 1)), 0);
}
fn frame_sample(cell: vec2i, local: vec2f, inst: Instance, low: i32, high: i32, anchor_mix: f32) -> Sample {
    let n = i32(params.settings.x);
    let bounded = clamp(cell, vec2i(0), vec2i(n - 1));
    let encoded = (vec2f(bounded) + vec2f(0.5)) * (2.0 / f32(n)) - vec2f(1.0);
    var direction = oct_decode(encoded);
    if params.settings.w > 0.5 { direction.y = abs(direction.y); }
    let basis = frame_basis(direction);
    let radius = inst.center_radius.w;
    let depth_scale = radius / params.source.x;
    let eye = inst.center_radius.xyz + direction * radius * 3.0;
    let scale = 1.0 / 3.0;
    var uv = vec2f(local.x * 0.5 + 0.5, 0.5 - local.y * 0.5);
    let cell_size = 1.0 / f32(n);
    var atlas_uv = (vec2f(bounded) + uv) * cell_size;
    var feature = geometry_sample(atlas_uv);
    for (var step = 0; step < 1; step++) {
        if feature.w <= 0.0 || feature.w * depth_scale > radius * 6.0 { break; }
        let ray = normalize(basis[2] + basis[0] * ((uv.x * 2.0 - 1.0) * scale) + basis[1] * ((1.0 - uv.y * 2.0) * scale));
        let point = eye + ray * feature.w * depth_scale;
        let card_point = inst.center_radius.xyz + params.camera_right.xyz * local.x * radius + params.camera_up.xyz * local.y * radius;
        let shift = vec2f(dot(point - card_point, params.camera_right.xyz), -dot(point - card_point, params.camera_up.xyz)) / max(radius, 1e-5);
        uv = clamp(uv - shift * 0.5, vec2f(0.5 / params.settings.y), vec2f(1.0 - 0.5 / params.settings.y));
        atlas_uv = (vec2f(bounded) + uv) * cell_size;
        feature = geometry_sample(atlas_uv);
    }
    let ray = normalize(basis[2] + basis[0] * ((uv.x * 2.0 - 1.0) * scale) + basis[1] * ((1.0 - uv.y * 2.0) * scale));
    let position = eye + ray * feature.w * depth_scale;
    let a = textureSampleLevel(radiance, linear_sampler, atlas_uv, low, 0.0);
    var color = a;
    if high != low { color = mix(a, textureSampleLevel(radiance, linear_sampler, atlas_uv, high, 0.0), anchor_mix); }
    return Sample(color, position, feature.w * depth_scale);
}
@vertex fn vertex(@builtin(vertex_index) vertex_id: u32, @builtin(instance_index) instance_id: u32) -> VertexOut {
    let corners = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0), vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0));
    let local = corners[vertex_id];
    let inst = instances[instance_id];
    let world = inst.center_radius.xyz + (params.camera_right.xyz * local.x + params.camera_up.xyz * local.y) * inst.center_radius.w;
    var output: VertexOut;
    output.clip = params.view_projection * vec4f(world, 1.0);
    output.local = local;
    output.instance_id = instance_id;
    return output;
}
fn shade(local: vec2f, instance_id: u32, pixel: vec4f) -> Shade {
    let inst = instances[instance_id];
    var toward = params.camera_position.xyz - inst.center_radius.xyz;
    if params.source.y > 0.5 { toward = params.camera_position.xyz; }
    toward = normalize(toward);
    var uv = oct_encode(toward);
    if params.settings.w > 0.5 { uv = oct_encode(vec3f(toward.x, abs(toward.y), toward.z)); }
    let grid = (uv * 0.5 + vec2f(0.5)) * params.settings.x - vec2f(0.5);
    let base = vec2i(floor(grid));
    let t = fract(grid);
    var cells: array<vec2i, 3>;
    var weights: vec3f;
    if t.x + t.y <= 1.0 {
        cells = array<vec2i, 3>(base, base + vec2i(1, 0), base + vec2i(0, 1));
        weights = vec3f(1.0 - t.x - t.y, t.x, t.y);
    } else {
        cells = array<vec2i, 3>(base + vec2i(1, 1), base + vec2i(0, 1), base + vec2i(1, 0));
        weights = vec3f(t.x + t.y - 1.0, 1.0 - t.x, 1.0 - t.y);
    }
    let low = i32(params.settings.z);
    let high = min(low + 1, i32(textureNumLayers(radiance)) - 1);
    let anchor_mix = fract(params.settings.z);
    var color = vec4f(0.0);
    var position = vec3f(0.0);
    var weight = 0.0;
    for (var i = 0; i < 3; i++) {
        let sample = frame_sample(cells[i], local, inst, low, high, anchor_mix);
        let amount = weights[i] * max(sample.color.a, 0.0);
        color += sample.color * weights[i];
        if sample.weight > 0.0 && sample.weight < inst.center_radius.w * 6.0 {
            position += sample.position * amount;
            weight += amount;
        }
    }
    if color.a <= 0.5 || hash(vec2u(pixel.xy), inst.options.x) < bitcast<f32>(inst.options.y) || weight <= 1e-5 { discard; }
    let clip = params.view_projection * vec4f(position / weight, 1.0);
    var output: Shade;
    output.color = vec4f(color.rgb / max(color.a, 1e-5), 1.0);
    output.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return output;
}
@fragment fn fragment(input: VertexOut) -> FragmentOut {
    let value = shade(input.local, input.instance_id, input.clip);
    return FragmentOut(value.color, value.depth);
}
@fragment fn shadow_fragment(input: VertexOut) -> @builtin(frag_depth) f32 {
    return shade(input.local, input.instance_id, input.clip).depth;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ImpostorInstance {
    pub center_radius: [f32; 4],
    pub meta: [u32; 4],
}

impl ImpostorInstance {
    pub fn new(center: [f32; 3], radius: f32, id: u32, mesh_fraction: f32) -> Result<Self, String> {
        if center.iter().any(|v| !v.is_finite())
            || !radius.is_finite()
            || radius <= 0.0
            || !mesh_fraction.is_finite()
            || !(0.0..=1.0).contains(&mesh_fraction)
        {
            return Err("invalid impostor instance".into());
        }
        Ok(Self {
            center_radius: [center[0], center[1], center[2], radius],
            meta: [id, mesh_fraction.to_bits(), 0, 0],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ImpostorParams {
    pub view_projection: [[f32; 4]; 4],
    pub camera_position: [f32; 4],
    pub camera_right: [f32; 4],
    pub camera_up: [f32; 4],
    pub settings: [f32; 4],
    pub source: [f32; 4],
}

pub type FrameWeights = ([(u32, u32); 3], [f32; 3]);

pub fn frame_weights(direction: [f32; 3], frames_per_side: u32) -> Result<FrameWeights, String> {
    if frames_per_side < 2
        || direction.iter().any(|v| !v.is_finite())
        || direction.iter().all(|v| *v == 0.0)
    {
        return Err("invalid frame lookup".into());
    }
    let sum = direction.iter().map(|v| v.abs()).sum::<f32>();
    let p = [direction[0] / sum, direction[2] / sum];
    let uv = if direction[1] < 0.0 {
        [
            (1.0 - p[1].abs()) * if p[0] < 0.0 { -1.0 } else { 1.0 },
            (1.0 - p[0].abs()) * if p[1] < 0.0 { -1.0 } else { 1.0 },
        ]
    } else {
        p
    };
    let n = frames_per_side as f32;
    let grid = uv.map(|v| (v * 0.5 + 0.5) * n - 0.5);
    let base = grid.map(f32::floor);
    let t = [grid[0] - base[0], grid[1] - base[1]];
    let (cells, weights) = if t[0] + t[1] <= 1.0 {
        ([(0, 0), (1, 0), (0, 1)], [1.0 - t[0] - t[1], t[0], t[1]])
    } else {
        (
            [(1, 1), (0, 1), (1, 0)],
            [t[0] + t[1] - 1.0, 1.0 - t[0], 1.0 - t[1]],
        )
    };
    let cell = cells.map(|(x, y)| {
        (
            (base[0] as i32 + x).clamp(0, frames_per_side as i32 - 1) as u32,
            (base[1] as i32 + y).clamp(0, frames_per_side as i32 - 1) as u32,
        )
    });
    Ok((cell, weights))
}

pub fn anchor_position(hours: &[f32], hour: f32) -> Result<f32, String> {
    if hours.is_empty()
        || !hour.is_finite()
        || hours.iter().any(|v| !v.is_finite())
        || hours.windows(2).any(|v| v[0] >= v[1])
    {
        return Err("invalid impostor anchors".into());
    }
    if hour <= hours[0] {
        return Ok(0.0);
    }
    for i in 0..hours.len() - 1 {
        if hour <= hours[i + 1] {
            return Ok(i as f32 + (hour - hours[i]) / (hours[i + 1] - hours[i]));
        }
    }
    Ok((hours.len() - 1) as f32)
}

pub fn mesh_fraction(
    distance: f32,
    switch_distance: f32,
    transition_width: f32,
) -> Result<f32, String> {
    if !distance.is_finite()
        || !switch_distance.is_finite()
        || !transition_width.is_finite()
        || switch_distance <= 0.0
        || transition_width <= 0.0
    {
        return Err("invalid impostor transition".into());
    }
    let near = switch_distance - transition_width * 0.5;
    let t = ((distance - near) / transition_width).clamp(0.0, 1.0);
    Ok(1.0 - t * t * (3.0 - 2.0 * t))
}

pub struct ImpostorPass {
    pub manifest: PropManifest,
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    shadow_groups: [wgpu::BindGroup; crate::shadow::CASCADE_COUNT],
    shadow_uniforms: [wgpu::Buffer; crate::shadow::CASCADE_COUNT],
    instances: wgpu::Buffer,
    capacity: usize,
}

impl ImpostorPass {
    pub fn load(
        gpu: &Gpu,
        folder: &Path,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Result<Self, String> {
        let artifact = read_prop_artifact(folder)?;
        Self::new(gpu, artifact, color_format, depth_format, capacity)
    }

    pub fn new(
        gpu: &Gpu,
        artifact: PropArtifact,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Result<Self, String> {
        if capacity == 0 || artifact.radiance.is_empty() {
            return Err("impostor pass needs capacity and anchors".into());
        }
        let size = artifact.manifest.atlas_size;
        let layers = artifact.radiance.len() as u32;
        let pixels = size as usize * size as usize;
        if size < 16
            || artifact.manifest.frames_per_side < 2
            || !artifact.manifest.bounds.radius.is_finite()
            || artifact.manifest.bounds.radius <= 0.0
            || !size.is_multiple_of(artifact.manifest.frames_per_side)
            || size / artifact.manifest.frames_per_side < 8
            || size > gpu.device.limits().max_texture_dimension_2d
            || layers > gpu.device.limits().max_texture_array_layers
            || artifact.radiance.len() != artifact.manifest.anchors.len()
            || artifact.geometry.len() != pixels * 16
            || artifact
                .radiance
                .iter()
                .any(|bytes| bytes.len() != pixels * 8)
            || capacity
                .checked_mul(std::mem::size_of::<ImpostorInstance>())
                .is_none()
        {
            return Err("impostor artifact exceeds the device or has invalid data".into());
        }
        let color = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impostor radiance"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (i, bytes) in artifact.radiance.iter().enumerate() {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &color,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 8),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let geometry = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impostor geometry"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            geometry.as_image_copy(),
            &artifact.geometry,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 16),
                rows_per_image: Some(size),
            },
            geometry.size(),
        );
        let color_view = color.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let geometry_view = geometry.create_view(&Default::default());
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor params"),
            size: std::mem::size_of::<ImpostorParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor instances"),
            size: (capacity * std::mem::size_of::<ImpostorInstance>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("impostor layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let make_group = |params: &wgpu::Buffer| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("impostor group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: instances.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&color_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&geometry_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let group = make_group(&uniform);
        let shadow_uniforms = std::array::from_fn(|_| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("impostor cascade params"),
                size: std::mem::size_of::<ImpostorParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let shadow_groups = std::array::from_fn(|index| make_group(&shadow_uniforms[index]));
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("impostor shader"),
                source: wgpu::ShaderSource::Wgsl(IMPOSTOR_WGSL.into()),
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("impostor pipeline layout"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("impostor pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });
        let shadow_pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("impostor shadow pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("shadow_fragment"),
                    targets: &[],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            });
        Ok(Self {
            manifest: artifact.manifest,
            pipeline,
            shadow_pipeline,
            group,
            uniform,
            shadow_groups,
            shadow_uniforms,
            instances,
            capacity,
        })
    }

    pub fn update(
        &self,
        gpu: &Gpu,
        mut params: ImpostorParams,
        hour: f32,
        items: &[ImpostorInstance],
    ) -> Result<(), String> {
        if items.len() > self.capacity {
            return Err("impostor instance capacity exceeded".into());
        }
        self.finish_params(&mut params, hour)?;
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&params));
        gpu.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(items));
        Ok(())
    }

    fn finish_params(&self, params: &mut ImpostorParams, hour: f32) -> Result<(), String> {
        params.settings = [
            self.manifest.frames_per_side as f32,
            (self.manifest.atlas_size / self.manifest.frames_per_side) as f32,
            anchor_position(&self.manifest.anchors, hour)?,
            if self.manifest.hemisphere { 1.0 } else { 0.0 },
        ];
        params.source = [self.manifest.bounds.radius, 0.0, 0.0, 0.0];
        Ok(())
    }

    pub fn update_shadows(
        &self,
        gpu: &Gpu,
        fit: &crate::shadow::Fit,
        hour: f32,
        items: &[ImpostorInstance],
    ) -> Result<(), String> {
        if items.len() > self.capacity {
            return Err("impostor instance capacity exceeded".into());
        }
        let forward = fit.toward_sun.map(|value| -value);
        let nominal_up = if fit.toward_sun[1].abs() > 0.95 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let right = cross(forward, nominal_up);
        let length = right.iter().map(|value| value * value).sum::<f32>().sqrt();
        let right = right.map(|value| value / length);
        let up = cross(right, forward);
        for (index, cascade) in fit.cascades.iter().enumerate() {
            let mut params = ImpostorParams {
                view_projection: cascade.view_proj.columns,
                camera_position: [fit.toward_sun[0], fit.toward_sun[1], fit.toward_sun[2], 0.0],
                camera_right: [right[0], right[1], right[2], 0.0],
                camera_up: [up[0], up[1], up[2], 0.0],
                settings: [0.0; 4],
                source: [0.0; 4],
            };
            self.finish_params(&mut params, hour)?;
            params.source[1] = 1.0;
            gpu.queue
                .write_buffer(&self.shadow_uniforms[index], 0, bytemuck::bytes_of(&params));
        }
        gpu.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(items));
        Ok(())
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, count: usize) -> Result<(), String> {
        if count > self.capacity {
            return Err("impostor instance capacity exceeded".into());
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..6, 0..count as u32);
        Ok(())
    }

    pub fn draw_shadow<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        count: usize,
        cascade: usize,
    ) -> Result<(), String> {
        if count > self.capacity {
            return Err("impostor instance capacity exceeded".into());
        }
        pass.set_pipeline(&self.shadow_pipeline);
        let group = self
            .shadow_groups
            .get(cascade)
            .ok_or("invalid shadow cascade")?;
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..6, 0..count as u32);
        Ok(())
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn prop_shadow_matches_mesh_footprint() {
        use pfx_bake::impostor::{PropSpec, bake_prop, bounds};
        use pfx_bake::{Anchor, BakeScene};
        use pfx_geom::shapes::Shape;
        use pfx_load::Sky;
        use pfx_materials::Material;
        use wgpu::util::DeviceExt;

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mesh = Shape::RoundBox {
            half: [0.5, 0.5, 0.5],
            radius: 0.02,
        }
        .mesh(0.02);
        let triangles = mesh_triangles(&mesh, [0.0; 3], 0);
        let bounds = bounds(&triangles, &[]).unwrap();
        let scene = BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: vec![Material::default()],
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 4,
                    height: 2,
                    texels: vec![[0.5; 4]; 8],
                },
                sun: pfx_trace::Sun {
                    direction: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                    intensity: 2.0,
                },
            }],
        };
        let artifact = bake_prop(
            &gpu,
            &scene,
            &PropSpec {
                name: "shadow-box".into(),
                node: Some("box".into()),
                file: None,
                frames_per_side: 8,
                atlas_size: 256,
                hemisphere: false,
                anchors: vec![12.0],
            },
            bounds,
            1,
            4,
        )
        .unwrap();
        let props = ImpostorPass::new(
            &gpu,
            artifact,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
            1,
        )
        .unwrap();
        let quality = crate::shadow::Quality {
            resolution: 128,
            ..Default::default()
        };
        let view = crate::shadow::View {
            eye: [0.0, 1.0, 4.0],
            forward: [0.0, -0.25, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y: 1.0,
            aspect: 1.0,
            near: 0.1,
            far: 10.0,
        };
        let mut mesh_shadows = crate::shadow::Shadows::new(&gpu.device, quality);
        let fit = mesh_shadows.fit(&view, [0.0, 1.0, 0.0]);
        let vertices = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow reference positions"),
                contents: bytemuck::cast_slice(&mesh.positions),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let indices = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow reference indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        mesh_shadows.render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &fit,
            &[crate::shadow::Caster {
                positions: &vertices,
                indices: &indices,
                first_index: 0,
                index_count: mesh.indices.len() as u32,
                instances: &[crate::shadow::Mat4::identity()],
                moving: false,
                generation: 0,
            }],
        );
        let mut prop_shadows = crate::shadow::Shadows::new(&gpu.device, quality);
        prop_shadows.render(&gpu.device, &gpu.queue, &mut encoder, &fit, &[]);
        let item = ImpostorInstance::new(bounds.center, bounds.radius, 1, 0.0).unwrap();
        props.update_shadows(&gpu, &fit, 12.0, &[item]).unwrap();
        prop_shadows
            .render_impostors(&mut encoder, &props, 1)
            .unwrap();
        let reference = shadow_pixels(&gpu, &mut encoder, mesh_shadows.atlas());
        let actual = shadow_pixels(&gpu, &mut encoder, prop_shadows.atlas());
        gpu.queue.submit(Some(encoder.finish()));
        let reference = read_shadow_pixels(&gpu, &reference);
        let actual = read_shadow_pixels(&gpu, &actual);
        let mut populated = 0;
        for cascade in 0..crate::shadow::CASCADE_COUNT {
            let start = cascade * 128 * 128;
            let end = start + 128 * 128;
            let mut overlap = 0;
            let mut union = 0;
            let mut mesh_count = 0;
            let mut prop_count = 0;
            for (mesh, prop) in reference[start..end].iter().zip(&actual[start..end]) {
                let mesh = *mesh < 0.999;
                let prop = *prop < 0.999;
                overlap += usize::from(mesh && prop);
                union += usize::from(mesh || prop);
                mesh_count += usize::from(mesh);
                prop_count += usize::from(prop);
            }
            if mesh_count > 0 {
                populated += 1;
                assert!(
                    overlap * 10 >= union * 7,
                    "cascade {cascade}: overlap {overlap}/{union}, mesh {mesh_count}, prop {prop_count}"
                );
            } else {
                assert_eq!(prop_count, 0, "unexpected prop shadow in cascade {cascade}");
            }
        }
        assert!(populated >= 2);
    }

    fn shadow_pixels(
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &wgpu::Texture,
    ) -> wgpu::Buffer {
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor shadow readback"),
            size: 512 * 128 * crate::shadow::CASCADE_COUNT as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: atlas,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(512),
                    rows_per_image: Some(128),
                },
            },
            wgpu::Extent3d {
                width: 128,
                height: 128,
                depth_or_array_layers: crate::shadow::CASCADE_COUNT as u32,
            },
        );
        buffer
    }

    fn read_shadow_pixels(gpu: &Gpu, buffer: &wgpu::Buffer) -> Vec<f32> {
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        let pixels = mapped
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        drop(mapped);
        buffer.unmap();
        pixels
    }

    fn identity() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn perspective(aspect: f32) -> [[f32; 4]; 4] {
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

    fn mesh_triangles(
        mesh: &pfx_geom::mesh::Mesh,
        offset: [f32; 3],
        material: u32,
    ) -> Vec<pfx_trace::bvh::Triangle> {
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

    fn mesh_handle(
        frame: &mut crate::frame::Frame,
        mesh: &pfx_geom::mesh::Mesh,
    ) -> crate::frame::MeshHandle {
        frame
            .upload_mesh(crate::frame::MeshData {
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

    fn png_preview(mesh: &[u16], impostor: &[u16], width: u32, height: u32) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("impostor-side-by-side.png");
        let preview_width = width / 2;
        let preview_height = height / 2;
        let mut pixels = vec![0u8; (preview_width * 2 * preview_height * 3) as usize];
        for y in 0..preview_height {
            for x in 0..preview_width * 2 {
                let source = if x < preview_width { mesh } else { impostor };
                let source_x = (x % preview_width) * 2;
                let source_y = y * 2;
                let source_index = ((source_y * width + source_x) * 4) as usize;
                let destination = ((y * preview_width * 2 + x) * 3) as usize;
                for channel in 0..3 {
                    let linear = half::f16::from_bits(source[source_index + channel])
                        .to_f32()
                        .max(0.0);
                    let mapped = linear / (1.0 + linear);
                    pixels[destination + channel] =
                        (mapped.powf(1.0 / 2.2) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, preview_width * 2, preview_height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        println!("side-by-side: {}", path.display());
    }

    fn display_value(bits: u16) -> f32 {
        let linear = half::f16::from_bits(bits).to_f32().max(0.0);
        linear / (1.0 + linear)
    }

    #[test]
    fn frame_weights_sum_to_one() {
        for x in -6..=6 {
            for y in -6..=6 {
                let (_, weights) = frame_weights([x as f32 + 0.1, y as f32 + 0.2, 1.0], 8).unwrap();
                assert!(weights.iter().all(|v| *v >= 0.0));
                assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn mesh_transition_is_continuous() {
        assert_eq!(mesh_fraction(4.0, 10.0, 4.0).unwrap(), 1.0);
        assert_eq!(mesh_fraction(10.0, 10.0, 4.0).unwrap(), 0.5);
        assert_eq!(mesh_fraction(16.0, 10.0, 4.0).unwrap(), 0.0);
    }

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(IMPOSTOR_WGSL).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn rounded_box_and_chrome_sphere_at_4k() {
        use pfx_bake::impostor::{PropSpec, bake_prop, bounds};
        use pfx_bake::{Anchor, BakeScene};
        use pfx_geom::shapes::Shape;
        use pfx_gpu::GpuProfiler;
        use pfx_load::Sky;
        use pfx_materials::Material;

        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let box_mesh = Shape::RoundBox {
            half: [0.8, 0.4, 0.6],
            radius: 0.12,
        }
        .mesh(0.006);
        let sphere_mesh = Shape::Ellipsoid {
            radius: 0.24,
            squash: 1.0,
        }
        .mesh(0.006);
        let sphere_offset = [0.0, 0.62, 0.0];
        let mut triangles = mesh_triangles(&box_mesh, [0.0; 3], 0);
        triangles.extend(mesh_triangles(&sphere_mesh, sphere_offset, 1));
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
        let scene = BakeScene {
            triangles,
            shapes: Vec::new(),
            materials: materials.clone(),
            anchors: vec![Anchor {
                hour: 12.0,
                sky: Sky {
                    width: 4,
                    height: 2,
                    texels: vec![[0.3, 0.4, 0.6, 1.0]; 8],
                },
                sun: pfx_trace::Sun {
                    direction: [0.4, 0.8, 0.4],
                    color: [1.0; 3],
                    intensity: 2.0,
                },
            }],
        };
        let bounds = bounds(&scene.triangles, &scene.shapes).unwrap();
        let spec = PropSpec {
            name: "rounded-box-chrome".into(),
            node: Some("test".into()),
            file: None,
            frames_per_side: 8,
            atlas_size: 512,
            hemisphere: false,
            anchors: vec![12.0],
        };
        let artifact = bake_prop(&gpu, &scene, &spec, bounds, 16, 7).unwrap();
        let artifact_bytes =
            artifact.geometry.len() + artifact.radiance.iter().map(Vec::len).sum::<usize>();
        let width = 3840;
        let height = 2160;
        let mut frame = crate::frame::Frame::new(gpu, width, height).unwrap();
        let box_handle = mesh_handle(&mut frame, &box_mesh);
        let sphere_handle = mesh_handle(&mut frame, &sphere_mesh);
        let mut view = identity();
        view[3] = [0.0, -bounds.center[1], -10.0, 1.0];
        let projection = perspective(width as f32 / height as f32);
        let view_projection = crate::frame::multiply(projection, view);
        let camera = crate::frame::Camera {
            view,
            projection,
            previous_view_projection: view_projection,
            position: [0.0, bounds.center[1], 10.0],
        };
        let mut sphere_model = identity();
        sphere_model[3] = [0.0, sphere_offset[1], 0.0, 1.0];
        let instances = [
            crate::frame::Instance::new(box_handle, identity(), 0, 1),
            crate::frame::Instance::new(sphere_handle, sphere_model, 1, 2),
        ];
        frame.set_sky(
            crate::sky::SkySource::Hdr(Sky {
                width: 4,
                height: 2,
                texels: vec![[0.3, 0.4, 0.6, 1.0]; 8],
            }),
            true,
        );
        let live_scene = crate::frame::Scene {
            camera,
            time: 0.0,
            seed: 7,
            sun: crate::frame::Sun {
                direction: [0.4, 0.8, 0.4],
                colour: [1.0; 3],
                intensity: 2.0,
            },
            instances: &instances,
            materials: &materials,
            deformers: &[],
            wind: Default::default(),
        };
        frame.render(&live_scene).unwrap();
        let mesh_pixels = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
        let impostor = ImpostorPass::new(
            &frame.gpu,
            artifact,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
            200,
        )
        .unwrap();
        let color = frame
            .gpu
            .offscreen(width, height, wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let depth = frame.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("impostor benchmark depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&Default::default());
        let params = ImpostorParams {
            view_projection,
            camera_position: [0.0, bounds.center[1], 10.0, 0.0],
            camera_right: [1.0, 0.0, 0.0, 0.0],
            camera_up: [0.0, 1.0, 0.0, 0.0],
            settings: [0.0; 4],
            source: [0.0; 4],
        };
        let single = [ImpostorInstance::new(bounds.center, bounds.radius, 3, 0.0).unwrap()];
        impostor.update(&frame.gpu, params, 12.0, &single).unwrap();
        let render = |count: usize, timer: Option<&mut GpuProfiler>, clear: bool| {
            let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
            let mut timer = timer;
            let index = timer.as_mut().and_then(|t| t.pass("200 impostors"));
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("impostor benchmark"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color.view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: if clear {
                                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: if clear {
                                wgpu::LoadOp::Clear(1.0)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: timer.as_ref().and_then(|t| t.render_writes(index)),
                });
                impostor.draw(&mut pass, count).unwrap();
            }
            let slot = timer.as_mut().and_then(|t| t.finish(&mut encoder));
            frame.gpu.queue.submit(Some(encoder.finish()));
            if let (Some(timer), Some(slot)) = (timer, slot) {
                timer.submitted(slot);
            }
        };
        render(1, None, true);
        let impostor_pixels = frame.gpu.readback_rgba16(&color).unwrap();
        png_preview(&mesh_pixels, &impostor_pixels, width, height);
        let error = mesh_pixels
            .chunks_exact(4)
            .zip(impostor_pixels.chunks_exact(4))
            .map(|(a, b)| {
                (0..3)
                    .map(|channel| (display_value(a[channel]) - display_value(b[channel])).abs())
                    .sum::<f32>()
            })
            .sum::<f32>()
            / (width * height * 3) as f32;
        let (foreground_error, foreground_pixels) = mesh_pixels
            .chunks_exact(4)
            .zip(impostor_pixels.chunks_exact(4))
            .filter_map(|(a, b)| {
                let reference = [0, 1, 2].map(|channel| display_value(a[channel]));
                let actual = [0, 1, 2].map(|channel| display_value(b[channel]));
                (reference
                    .iter()
                    .chain(actual.iter())
                    .any(|value| *value > 0.01))
                .then_some(
                    (0..3)
                        .map(|channel| (reference[channel] - actual[channel]).abs())
                        .sum::<f32>(),
                )
            })
            .fold((0.0, 0usize), |(error, count), value| {
                (error + value, count + 1)
            });
        let foreground_error = foreground_error / (foreground_pixels.max(1) * 3) as f32;
        let items = (0..200)
            .map(|i| {
                let x = (i % 20) as f32 - 9.5;
                let y = (i / 20) as f32 - 4.5;
                ImpostorInstance::new([x * 0.45, y * 0.42, 0.0], 0.22, i as u32 + 10, 0.0).unwrap()
            })
            .collect::<Vec<_>>();
        impostor.update(&frame.gpu, params, 12.0, &items).unwrap();
        let mut profiler = GpuProfiler::new(&frame.gpu.device, &frame.gpu.queue);
        for _ in 0..4 {
            render(200, None, true);
        }
        render(200, Some(&mut profiler), false);
        frame
            .gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let timings = profiler.collect(&frame.gpu.device);
        let milliseconds = timings
            .first()
            .and_then(|passes| passes.first())
            .map(|pass| pass.milliseconds)
            .unwrap_or(f64::NAN);
        println!(
            "4K impostor mesh MAE={error:.4}, foreground MAE={foreground_error:.4} over {foreground_pixels} pixels, 200 instances={milliseconds:.3} ms, atlas={artifact_bytes} bytes"
        );
        assert!(error <= 0.02, "4K impostor mesh MAE {error:.4}");
        assert!(
            milliseconds < 0.5,
            "200 impostors took {milliseconds:.3} ms"
        );
    }
}
