use bytemuck::{Pod, Zeroable};
use pfx_geom::tree::{Card, CardLod, Tree, tree_sway};
use pfx_gpu::wgpu;
use pfx_physics::{Emitter, ParticleKind};
use wgpu::util::DeviceExt;

pub const DEFAULT_RESOLUTION: u32 = 1024;
pub const DEFAULT_PROCEDURAL_STRENGTH: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanopyMode {
    #[default]
    Cookie,
    Procedural,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gobo {
    pub gust: [f32; 5],
    pub sway: [[f32; 4]; 2],
    pub branch: [f32; 4],
    pub clusters: [f32; 5],
    pub leaves: [f32; 6],
    pub weight: f32,
}

impl Gobo {
    pub fn validate(&self) -> Result<(), String> {
        let all = self
            .gust
            .iter()
            .chain(self.sway.iter().flatten())
            .chain(&self.branch)
            .chain(&self.clusters)
            .chain(&self.leaves)
            .chain(std::iter::once(&self.weight));
        if all.clone().any(|value| !value.is_finite()) {
            return Err("canopy gobo values must be finite".into());
        }
        if self.branch[0] == self.branch[1]
            || self.clusters[3] == self.clusters[4]
            || self.leaves[1] == self.leaves[2]
        {
            return Err("canopy gobo smoothstep edges must differ".into());
        }
        if self.weight <= 0.0 {
            return Err("canopy gobo weight must be positive".into());
        }
        Ok(())
    }
}

impl Default for Gobo {
    fn default() -> Self {
        Self {
            gust: [0.2, 0.6, 0.35, 1.0, 0.4],
            sway: [[1.0, 8.0, 0.005, 0.01], [0.8, 6.0, 0.004, 0.004]],
            branch: [-0.5, 0.2, 0.8, 0.6],
            clusters: [3.0, 4.0, 2.0, 0.3, 0.6],
            leaves: [16.0, 0.35, 0.55, 1.2, 1.0, 0.08],
            weight: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardColors {
    pub tint_g: [f32; 3],
    pub tint_b: [f32; 3],
    pub glow: [f32; 3],
}

impl Default for CardColors {
    fn default() -> Self {
        Self {
            tint_g: [0.9, 0.9, 0.85],
            tint_b: [0.8, 0.85, 0.7],
            glow: [0.8, 0.8, 0.6],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CanopyFrame {
    pub origin: [f32; 3],
    pub time: f32,
    pub strength: f32,
    pub direction: [f32; 3],
    pub sun: [f32; 3],
    pub receiver_y: f32,
    pub angular_radius: f32,
}

pub const CANOPY_LIGHT_HEAD: &str = r#"
struct CanopyParams {
    center_extent: vec4f,
    sun_receiver: vec4f,
    wind: vec4f,
    direction: vec4f,
    projected_extent: vec4f,
    mode_sharpness: vec4f,
}
@group(3) @binding(9) var canopy_cookie: texture_2d<f32>;
@group(3) @binding(10) var canopy_sampler: sampler;
@group(3) @binding(11) var<uniform> canopy_params: CanopyParams;
fn canopy_hash(p: vec2f) -> f32 {
    let q = fract(p * vec2f(123.34, 456.21));
    return fract((q.x + 45.32) * (q.y + 45.32) * 34.23 + q.x * 13.7);
}
fn canopy_noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(canopy_hash(i), canopy_hash(i + vec2f(1.0, 0.0)), u.x), mix(canopy_hash(i + vec2f(0.0, 1.0)), canopy_hash(i + vec2f(1.0, 1.0)), u.x), u.y);
}
fn canopy_fbm(p: vec2f) -> f32 {
    return canopy_noise(p) * 0.55 + canopy_noise(p * 2.13 + 3.1) * 0.28 + canopy_noise(p * 4.37 + 7.7) * 0.17;
}
"#;

pub const CANOPY_LIGHT_TAIL: &str = r#"fn canopy_light(world_position: vec3f) -> f32 {
    let sun_input = canopy_params.sun_receiver.xyz;
    let sun = select(vec3f(0.0, 1.0, 0.0), sun_input / max(length(sun_input), 1e-6), length(sun_input) > 1e-6);
    if (canopy_params.mode_sharpness.x > 0.5) {
        let origin = vec3f(canopy_params.center_extent.x, canopy_params.wind.z, canopy_params.center_extent.y);
        let horizontal = vec3f(-sun.z, 0.0, sun.x);
        let gobo_u = horizontal / max(length(horizontal), 1e-6);
        let gobo_v = cross(gobo_u, sun);
        let q = vec2f(dot(world_position - origin, gobo_u), dot(world_position - origin, gobo_v));
        let light = procedural_canopy(q, canopy_params.wind.x);
        let scale = canopy_params.mode_sharpness.z;
        if (scale == 1.0) { return light; }
        return 1.0 - (1.0 - light) * scale;
    }
    let reference = select(vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0), abs(sun.y) > 0.99);
    let axis_u = normalize(cross(sun, reference));
    let axis_v = cross(axis_u, sun);
    let projected = vec2f(dot(world_position, axis_u), dot(world_position, axis_v));
    let uv = (projected - canopy_params.projected_extent.xy) / canopy_params.projected_extent.zw + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0))) { return 1.0; }
    let texel = 1.0 / vec2f(textureDimensions(canopy_cookie));
    let crown = vec3f(canopy_params.center_extent.x, canopy_params.wind.w, canopy_params.center_extent.y);
    let distance = dot(crown - world_position, sun);
    if (distance <= 0.0) { return 1.0; }
    let radius = max(canopy_params.direction.w * distance * canopy_params.mode_sharpness.y / canopy_params.projected_extent.z, texel.x);
    var shade = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            if (x * x + y * y <= 4) {
                let offset = vec2f(f32(x), f32(y)) * radius * 0.5;
                shade += textureSampleLevel(canopy_cookie, canopy_sampler, uv + offset, 0.0).r;
            }
        }
    }
    return 1.0 - shade / 13.0;
}
"#;

fn literal(value: f32) -> String {
    format!("{value:?}")
}

pub fn canopy_light_wgsl(gobo: &Gobo) -> String {
    let [g0, g1, g2, g3, g4] = gobo.gust.map(literal);
    let [x0, x1, x2, x3] = gobo.sway[0].map(literal);
    let [z0, z1, z2, z3] = gobo.sway[1].map(literal);
    let [b0, b1, b2, b3] = gobo.branch.map(literal);
    let [c0, c1, c2, c3, c4] = gobo.clusters.map(literal);
    let [l0, l1, l2, l3, l4, l5] = gobo.leaves.map(literal);
    let weight = literal(gobo.weight);
    let procedural = format!(
        "fn procedural_canopy(q: vec2f, t: f32) -> f32 {{
    let gust = sin(t * {g0}) * {g1} + sin(t * {g2} + {g3}) * {g4};
    let sway = vec2f(sin(t * {x0} + q.y * {x1}) * {x2} + gust * {x3}, cos(t * {z0} + q.x * {z1}) * {z2} + gust * {z3});
    let p = q + sway;
    let branch = smoothstep({b0}, {b1}, p.x * {b2} + p.y * {b3});
    let clusters = smoothstep({c3}, {c4}, canopy_fbm(p * {c0} + vec2f({c1}, {c2})));
    let leaves = smoothstep({l1}, {l2}, canopy_fbm(p * {l0} + vec2f(sin(t * {l3}) * {l5}, cos(t * {l4}) * {l5})));
    return 1.0 - leaves * clusters * branch * {weight};
}}
"
    );
    [CANOPY_LIGHT_HEAD, &procedural, CANOPY_LIGHT_TAIL].concat()
}

pub const CANOPY_COOKIE_WGSL: &str = r#"
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
@group(0) @binding(0) var<uniform> params: CanopyParams;
@group(0) @binding(1) var<storage, read> cards: array<Card>;
@group(0) @binding(2) var alpha_atlas: texture_2d<f32>;
@group(0) @binding(3) var alpha_sampler: sampler;
fn tree_sway(rest: vec3f, pivot: vec3f, level: f32, stiffness: f32, wind_time: f32, strength: f32, direction: vec3f) -> vec3f {
    let d = normalize(direction.xz + vec2f(0.00001, 0.0));
    let arm = length(rest - pivot);
    let phase = wind_time * (0.7 + level * 0.41) + pivot.x * 0.83 + pivot.z * 0.67;
    let bend = sin(phase) * strength * (1.0 - stiffness) * arm * (0.012 + 0.009 * level);
    return rest + vec3f(d.x * bend, -abs(bend) * 0.12, d.y * bend);
}
struct Varying { @builtin(position) clip: vec4f, @location(0) uv: vec2f, @location(1) @interpolate(flat) atlas: vec4f, @location(2) @interpolate(flat) lod: vec4f, }
@vertex fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Varying {
    let card = cards[instance];
    let corners = array<vec2f, 6>(vec2f(-1.0,-1.0), vec2f(1.0,-1.0), vec2f(-1.0,1.0), vec2f(-1.0,1.0), vec2f(1.0,-1.0), vec2f(1.0,1.0));
    let corner = corners[vertex];
    let rest = card.position_size.xyz + card.right_size.xyz * card.position_size.w * corner.x + card.up.xyz * card.right_size.w * corner.y;
    let moved = tree_sway(rest, card.pivot_level.xyz, card.pivot_level.w, card.motion_color.x, params.wind.x, params.wind.y, params.direction.xyz) + vec3f(params.center_extent.x, params.wind.z, params.center_extent.y);
    let sun_input = params.sun_receiver.xyz;
    let sun = select(vec3f(0.0, 1.0, 0.0), sun_input / max(length(sun_input), 1e-6), length(sun_input) > 1e-6);
    let reference = select(vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0), abs(sun.y) > 0.99);
    let axis_u = normalize(cross(sun, reference));
    let axis_v = cross(axis_u, sun);
    let projected = vec2f(dot(moved, axis_u), dot(moved, axis_v));
    let uv = (projected - params.projected_extent.xy) / params.projected_extent.zw + vec2f(0.5);
    var out: Varying;
    out.clip = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = (corner + vec2f(1.0)) * 0.5;
    out.atlas = card.atlas;
    out.lod = card.lod;
    return out;
}
@fragment fn fragment(input: Varying) -> @location(0) f32 {
    let uv = input.atlas.xy + input.uv * input.atlas.zw;
    let weight = select(1.0 - input.lod.x, input.lod.x, input.lod.z > 0.5);
    return textureSample(alpha_atlas, alpha_sampler, uv).r * weight;
}
"#;

const CANOPY_VISIBLE_TEMPLATE: &str = r#"
struct CanopyParams { center_extent: vec4f, sun_receiver: vec4f, wind: vec4f, direction: vec4f, projected_extent: vec4f, mode_sharpness: vec4f, }
struct Card { position_size: vec4f, right_size: vec4f, up: vec4f, pivot_level: vec4f, motion_color: vec4f, atlas: vec4f, lod: vec4f, }
@group(0) @binding(0) var<uniform> params: CanopyParams;
@group(0) @binding(1) var<storage, read> cards: array<Card>;
@group(0) @binding(2) var alpha_atlas: texture_2d<f32>;
@group(0) @binding(3) var alpha_sampler: sampler;
@group(1) @binding(0) var<uniform> view_proj: mat4x4f;
fn tree_sway(rest: vec3f, pivot: vec3f, level: f32, stiffness: f32, wind_time: f32, strength: f32, direction: vec3f) -> vec3f {
    let d = normalize(direction.xz + vec2f(0.00001, 0.0));
    let arm = length(rest - pivot);
    let phase = wind_time * (0.7 + level * 0.41) + pivot.x * 0.83 + pivot.z * 0.67;
    let bend = sin(phase) * strength * (1.0 - stiffness) * arm * (0.012 + 0.009 * level);
    return rest + vec3f(d.x * bend, -abs(bend) * 0.12, d.y * bend);
}
struct Varying { @builtin(position) clip: vec4f, @location(0) uv: vec2f, @location(1) @interpolate(flat) atlas: vec4f, @location(2) @interpolate(flat) color: vec3f, @location(3) @interpolate(flat) normal: vec3f, }
@vertex fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Varying {
    let card = cards[instance];
    let corners = array<vec2f, 6>(vec2f(-1.0,-1.0), vec2f(1.0,-1.0), vec2f(-1.0,1.0), vec2f(-1.0,1.0), vec2f(1.0,-1.0), vec2f(1.0,1.0));
    let corner = corners[vertex];
    let rest = card.position_size.xyz + card.right_size.xyz * card.position_size.w * corner.x + card.up.xyz * card.right_size.w * corner.y;
    let moved = tree_sway(rest, card.pivot_level.xyz, card.pivot_level.w, card.motion_color.x, params.wind.x, params.wind.y, params.direction.xyz) + vec3f(params.center_extent.x, params.wind.z, params.center_extent.y);
    var out: Varying;
    out.clip = view_proj * vec4f(moved, 1.0);
    out.uv = (corner + vec2f(1.0)) * 0.5;
    out.atlas = card.atlas;
    out.color = card.motion_color.yzw;
    out.normal = normalize(cross(card.right_size.xyz, card.up.xyz));
    return out;
}
@fragment fn fragment(input: Varying, @builtin(front_facing) front: bool) -> @location(0) vec4f {
    let uv = input.atlas.xy + input.uv * input.atlas.zw;
    let texel = textureSample(alpha_atlas, alpha_sampler, uv);
    let normal = select(-input.normal, input.normal, front);
    let sun = normalize(params.sun_receiver.xyz);
    let facing = dot(normal, sun);
    let diffuse = max(facing, 0.0);
    let transmission = max(-facing, 0.0) * 0.48;
    let pigment = mix(input.color, @TINT_G@, texel.g * 0.6);
    let petal = mix(pigment, @TINT_B@, texel.b * 0.65);
    let lit = petal * (0.22 + diffuse * 0.78) + @GLOW@ * transmission * 0.52;
    return vec4f(lit, texel.r);
}
"#;

pub fn canopy_visible_wgsl(colors: &CardColors) -> String {
    let colour = |c: [f32; 3]| {
        format!(
            "vec3f({}, {}, {})",
            literal(c[0]),
            literal(c[1]),
            literal(c[2])
        )
    };
    CANOPY_VISIBLE_TEMPLATE
        .replace("@TINT_G@", &colour(colors.tint_g))
        .replace("@TINT_B@", &colour(colors.tint_b))
        .replace("@GLOW@", &colour(colors.glow))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    center_extent: [f32; 4],
    sun_receiver: [f32; 4],
    wind: [f32; 4],
    direction: [f32; 4],
    projected_extent: [f32; 4],
    mode_sharpness: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCard {
    position_size: [f32; 4],
    right_size: [f32; 4],
    up: [f32; 4],
    pivot_level: [f32; 4],
    motion_color: [f32; 4],
    atlas: [f32; 4],
    lod: [f32; 4],
}

fn gpu_card(card: &Card, coverage: f32, id: u32, near: bool) -> GpuCard {
    GpuCard {
        position_size: [
            card.position[0],
            card.position[1],
            card.position[2],
            card.size[0],
        ],
        right_size: [card.right[0], card.right[1], card.right[2], card.size[1]],
        up: [card.up[0], card.up[1], card.up[2], 0.0],
        pivot_level: [
            card.sway.pivot[0],
            card.sway.pivot[1],
            card.sway.pivot[2],
            card.sway.level,
        ],
        motion_color: [
            card.sway.stiffness,
            card.color[0],
            card.color[1],
            card.color[2],
        ],
        atlas: card.atlas,
        lod: [coverage, id as f32, if near { 1.0 } else { 0.0 }, 0.0],
    }
}

fn near_coverage(diameter: f32, projection_y: f32, height: u32, depth: f32) -> f32 {
    let pixels = diameter * projection_y.abs() * height as f32 / (2.0 * depth.abs().max(0.05));
    ((pixels - 9.0) / 3.0).clamp(0.0, 1.0)
}

struct CardGroup {
    start: usize,
    end: usize,
    cluster: bool,
}

pub struct Canopy {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub resolution: u32,
    pub center: [f32; 2],
    pub extent: [f32; 2],
    pub card_count: u32,
    source: Vec<GpuCard>,
    groups: Vec<CardGroup>,
    visible: Vec<GpuCard>,
    card_buffer: wgpu::Buffer,
    params: wgpu::Buffer,
    bind: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    cookie_layout: wgpu::BindGroupLayout,
    sample_layout: wgpu::BindGroupLayout,
    current: Params,
    canopy_height: f32,
    weight: f32,
}

impl Canopy {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, tree: &Tree, resolution: u32) -> Self {
        let resolution = resolution.max(16);
        let cards: Vec<GpuCard> = tree
            .cards
            .iter()
            .enumerate()
            .map(|(index, card)| gpu_card(card, 1.0, index as u32, true))
            .collect();
        let mut groups = Vec::new();
        let mut index = 0;
        while index < tree.cards.len() {
            let start = index;
            let cluster = tree.cards[index].lod == CardLod::Cluster;
            index += 1;
            if cluster {
                while index < tree.cards.len() && tree.cards[index].lod == CardLod::Individual {
                    index += 1;
                }
            }
            groups.push(CardGroup {
                start,
                end: index,
                cluster,
            });
        }
        let card_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canopy cards"),
            contents: bytemuck::cast_slice(&cards),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let extent = [tree.spec.crown_width * 2.8, tree.spec.crown_width * 2.8];
        let current = Params {
            center_extent: [0.0, 0.0, extent[0], extent[1]],
            sun_receiver: [0.0, 1.0, 0.0, 0.0],
            wind: [0.0, 0.0, 0.0, tree.spec.height * 0.8],
            direction: [1.0, 0.0, 0.0, 0.00465],
            projected_extent: [0.0, 0.0, extent[0], extent[1]],
            mode_sharpness: [0.0, 1.0, 0.0, DEFAULT_PROCEDURAL_STRENGTH],
        };
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("canopy params"),
            contents: bytemuck::bytes_of(&current),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canopy cookie"),
            size: wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tree alpha atlas"),
            size: wgpu::Extent3d {
                width: tree.atlas.width,
                height: tree.atlas.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &tree.atlas.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(tree.atlas.width * 4),
                rows_per_image: Some(tree.atlas.height),
            },
            wgpu::Extent3d {
                width: tree.atlas.width,
                height: tree.atlas.height,
                depth_or_array_layers: 1,
            },
        );
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canopy cookie layout"),
            entries: &cookie_entries(),
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canopy cookie bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: card_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("canopy cookie shader"),
            source: wgpu::ShaderSource::Wgsl(CANOPY_COOKIE_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("canopy cookie pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canopy cookie pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrc,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("canopy sample layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
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
        Self {
            texture,
            view,
            sampler,
            resolution,
            center: [0.0, 0.0],
            extent,
            card_count: cards.len() as u32,
            source: cards,
            groups,
            visible: Vec::with_capacity(tree.cards.len()),
            card_buffer,
            params,
            bind,
            pipeline,
            cookie_layout: layout,
            sample_layout,
            current,
            canopy_height: tree.spec.height * 0.8,
            weight: 1.0,
        }
    }

    pub fn sample_layout(&self) -> &wgpu::BindGroupLayout {
        &self.sample_layout
    }

    pub fn select_lod(
        &mut self,
        queue: &wgpu::Queue,
        camera: crate::frame::Camera,
        height: u32,
        origin: [f32; 3],
    ) {
        self.visible.clear();
        for group in &self.groups {
            let index = group.start;
            let card = self.source[index];
            if group.cluster {
                let center = [
                    card.position_size[0] + origin[0],
                    card.position_size[1] + origin[1],
                    card.position_size[2] + origin[2],
                ];
                let view =
                    crate::frame::transform(camera.view, [center[0], center[1], center[2], 1.0]);
                let near = near_coverage(0.032, camera.projection[1][1], height, view[2]);
                let id = index as u32;
                if near < 1.0 {
                    let mut far = card;
                    far.lod = [near, id as f32, 0.0, 0.0];
                    self.visible.push(far);
                }
                if near > 0.0 {
                    for flower in &self.source[index + 1..group.end] {
                        let mut flower = *flower;
                        flower.lod = [near, id as f32, 1.0, 0.0];
                        self.visible.push(flower);
                    }
                }
            } else {
                self.visible.push(card);
            }
        }
        self.card_count = self.visible.len() as u32;
        if !self.visible.is_empty() {
            queue.write_buffer(&self.card_buffer, 0, bytemuck::cast_slice(&self.visible));
        }
    }
    pub fn cookie_layout(&self) -> &wgpu::BindGroupLayout {
        &self.cookie_layout
    }
    pub fn cookie_bind(&self) -> &wgpu::BindGroup {
        &self.bind
    }
    pub fn params(&self) -> &wgpu::Buffer {
        &self.params
    }
    pub fn set_mode(&mut self, mode: CanopyMode) {
        self.current.mode_sharpness[0] = match mode {
            CanopyMode::Cookie => 0.0,
            CanopyMode::Procedural => 1.0,
        };
    }
    pub fn set_sharpness(&mut self, sharpness: f32) -> Result<(), String> {
        if !sharpness.is_finite() || sharpness <= 0.0 {
            return Err("canopy sharpness must be finite and positive".into());
        }
        self.current.mode_sharpness[1] = sharpness;
        Ok(())
    }
    pub fn set_strength(&mut self, strength: f32) -> Result<(), String> {
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err("canopy strength must be finite and between 0 and 1".into());
        }
        self.current.mode_sharpness[2] = strength / self.weight;
        self.current.mode_sharpness[3] = strength;
        Ok(())
    }
    pub fn set_weight(&mut self, weight: f32) {
        self.weight = weight;
        self.current.mode_sharpness[2] = self.current.mode_sharpness[3] / weight;
    }
    pub fn strength(&self) -> f32 {
        self.current.mode_sharpness[3]
    }
    pub fn sample_bind(&self, device: &wgpu::Device) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("canopy sample bind"),
            layout: &self.sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&self.view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: self.params.as_entire_binding(),
                },
            ],
        })
    }
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: CanopyFrame,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let (center, extent) = projected_bounds(
            frame.origin,
            self.canopy_height,
            self.current.center_extent[2],
            frame.sun,
            frame.receiver_y,
        );
        self.center = center;
        self.extent = extent;
        self.current.center_extent[0..2].copy_from_slice(&[frame.origin[0], frame.origin[2]]);
        self.current.projected_extent = [center[0], center[1], extent[0], extent[1]];
        self.current.sun_receiver = [frame.sun[0], frame.sun[1], frame.sun[2], frame.receiver_y];
        self.current.wind = [
            frame.time,
            frame.strength,
            frame.origin[1],
            self.canopy_height + frame.origin[1],
        ];
        self.current.direction = [
            frame.direction[0],
            frame.direction[1],
            frame.direction[2],
            frame.angular_radius,
        ];
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&self.current));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canopy cookie"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.draw(0..6, 0..self.card_count);
    }
}

fn projected_bounds(
    origin: [f32; 3],
    canopy_height: f32,
    base_extent: f32,
    sun: [f32; 3],
    _receiver_y: f32,
) -> ([f32; 2], [f32; 2]) {
    let [u, v] = projection_axes(sun);
    let crown = [origin[0], origin[1] + canopy_height, origin[2]];
    let center = [dot3(crown, u), dot3(crown, v)];
    let extent = [base_extent + canopy_height, base_extent + canopy_height];
    (center, extent)
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}

fn projection_axes(sun: [f32; 3]) -> [[f32; 3]; 2] {
    let length = dot3(sun, sun).sqrt();
    let s = if length > 1.0e-6 {
        sun.map(|component| component / length)
    } else {
        [0.0, 1.0, 0.0]
    };
    let reference = if s[1].abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = [
        s[1] * reference[2] - s[2] * reference[1],
        s[2] * reference[0] - s[0] * reference[2],
        s[0] * reference[1] - s[1] * reference[0],
    ];
    let length = dot3(u, u).sqrt().max(1.0e-6);
    let u = u.map(|component| component / length);
    let v = [
        u[1] * s[2] - u[2] * s[1],
        u[2] * s[0] - u[0] * s[2],
        u[0] * s[1] - u[1] * s[0],
    ];
    [u, v]
}

pub fn projected_coverage(
    tree: &Tree,
    time: f32,
    strength: f32,
    direction: [f32; 3],
    sun: [f32; 3],
    _receiver_y: f32,
) -> f32 {
    let [axis_u, axis_v] = projection_axes(sun);
    let tile_width = tree.atlas.width as usize / 6;
    let mut opacity = [0.0_f32; 6];
    for row in tree
        .atlas
        .pixels
        .chunks_exact(tree.atlas.width as usize * 4)
    {
        for (tile, sum) in opacity.iter_mut().enumerate() {
            *sum += (tile * tile_width..(tile + 1) * tile_width)
                .map(|x| row[x * 4] as f32 / 255.0)
                .sum::<f32>();
        }
    }
    let divisor = (tile_width * tree.atlas.height as usize).max(1) as f32;
    for value in &mut opacity {
        *value /= divisor;
    }
    let mut area = 0.0;
    for c in &tree.cards {
        let corners = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0]];
        let mut p = [[0.0; 2]; 3];
        for (i, corner) in corners.iter().enumerate() {
            let rest = [
                c.position[0]
                    + c.right[0] * c.size[0] * corner[0]
                    + c.up[0] * c.size[1] * corner[1],
                c.position[1]
                    + c.right[1] * c.size[0] * corner[0]
                    + c.up[1] * c.size[1] * corner[1],
                c.position[2]
                    + c.right[2] * c.size[0] * corner[0]
                    + c.up[2] * c.size[1] * corner[1],
            ];
            let m = tree_sway(
                rest,
                c.sway.pivot,
                c.sway.level,
                c.sway.stiffness,
                time,
                strength,
                direction,
            );
            p[i] = [dot3(m, axis_u), dot3(m, axis_v)];
        }
        let ax = p[1][0] - p[0][0];
        let ay = p[1][1] - p[0][1];
        let bx = p[2][0] - p[0][0];
        let by = p[2][1] - p[0][1];
        area += (ax * by - ay * bx).abs() * opacity[(c.atlas[0] * 6.0).round() as usize];
    }
    area
}

pub fn petal_emitters(
    tree: &Tree,
    origin: [f32; 3],
    time: f32,
    strength: f32,
    direction: [f32; 3],
    max_total_rate: f32,
) -> Vec<Emitter> {
    if !max_total_rate.is_finite() || max_total_rate <= 0.0 || tree.petal_sources.is_empty() {
        return Vec::new();
    }
    let native_rate = tree.petal_sources[0].rate.max(0.001);
    let count = ((max_total_rate / native_rate).ceil() as usize).clamp(1, tree.petal_sources.len());
    let per_source = max_total_rate / count as f32;
    (0..count)
        .map(|i| {
            let index = ((i as f32 + 0.5) * tree.petal_sources.len() as f32 / count as f32).floor()
                as usize;
            let source = &tree.petal_sources[index.min(tree.petal_sources.len() - 1)];
            let position = tree_sway(
                source.position,
                source.sway.pivot,
                source.sway.level,
                source.sway.stiffness,
                time,
                strength,
                direction,
            );
            let position = [
                position[0] + origin[0],
                position[1] + origin[1],
                position[2] + origin[2],
            ];
            let jitter = 0.7
                + 0.3
                    * ((source.seed.wrapping_mul(0x9e3779b97f4a7c15) >> 40) as f32 / 16_777_216.0);
            Emitter::new(
                ParticleKind::Petal,
                position,
                source.rate.min(per_source) * jitter,
                source.seed,
            )
        })
        .collect()
}

pub fn cookie_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
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
            visibility: wgpu::ShaderStages::VERTEX,
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
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cluster_lod_changes_with_distance_without_coverage_jump() {
        let mut previous = 1.0;
        for depth in [4.0, 6.0, 8.0, 10.0, 14.0, 20.0, 30.0] {
            let near = near_coverage(0.032, 2.7, 2160, depth);
            let far = 1.0 - near;
            assert!(near <= previous);
            assert!((near + far - 1.0).abs() < 1e-6);
            previous = near;
        }
        assert_eq!(near_coverage(0.032, 2.7, 2160, 4.0), 1.0);
        assert_eq!(near_coverage(0.032, 2.7, 2160, 30.0), 0.0);
        for pixels in [9.0_f32, 12.0] {
            let depth = 0.032 * 2.7 * 2160.0 / (2.0 * pixels);
            let before = near_coverage(0.032, 2.7, 2160, depth - 0.001);
            let after = near_coverage(0.032, 2.7, 2160, depth + 0.001);
            assert!((before - after).abs() < 0.001);
        }
    }
    #[test]
    fn shaders_and_area() {
        for source in [
            CANOPY_COOKIE_WGSL.to_string(),
            canopy_visible_wgsl(&CardColors::default()),
        ] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
        naga::front::wgsl::parse_str(&canopy_light_wgsl(&Gobo::default())).unwrap();
        let tree = Tree::new(9, pfx_geom::tree::TreeSpec::plum(), 0.02);
        let overhead = projected_coverage(&tree, 0.0, 0.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.0);
        let tile_width = tree.atlas.width as usize / 6;
        let alpha = |tile: usize| -> f32 {
            tree.atlas
                .pixels
                .chunks_exact(4)
                .enumerate()
                .filter(|(index, _)| index % tree.atlas.width as usize / tile_width == tile)
                .map(|(_, rgba)| rgba[0] as f32 / 255.0)
                .sum::<f32>()
                / (tile_width * tree.atlas.height as usize) as f32
        };
        let expected = tree
            .cards
            .iter()
            .map(|c| {
                4.0 * c.size[0]
                    * c.size[1]
                    * (c.right[0] * c.up[2] - c.right[2] * c.up[0]).abs()
                    * alpha((c.atlas[0] * 6.0).round() as usize)
            })
            .sum::<f32>();
        assert!((overhead - expected).abs() < expected * 1e-5);
        let angled = projected_coverage(&tree, 0.0, 0.0, [1.0, 0.0, 0.0], [0.4, 1.0, 0.2], 0.0);
        assert!((overhead - angled).abs() > overhead * 0.001);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn cookie_angular_radius_softens_an_edge() {
        let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
        let tree = Tree::new(9, pfx_geom::tree::TreeSpec::plum(), 0.02);
        let mut canopy = Canopy::new(&gpu.device, &gpu.queue, &tree, 256);
        canopy.current.wind[3] = 1.0;
        canopy.current.projected_extent = [0.0, 0.0, 1.0, 1.0];
        let pixels: Vec<u8> = (0..256 * 256)
            .map(|index| if index % 256 >= 128 { 255 } else { 0 })
            .collect();
        let cookie = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filter edge"),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &cookie,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(256),
            },
            wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
        );
        let cookie_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter edge bind"),
            layout: canopy.sample_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(
                        &cookie.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(&canopy.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: canopy.params.as_entire_binding(),
                },
            ],
        });
        let empty = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("filter empty layout"),
                entries: &[],
            });
        let empty_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter empty bind"),
            layout: &empty,
            entries: &[],
        });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("filter layout"),
                bind_group_layouts: &[&empty, &empty, &empty, canopy.sample_layout()],
                push_constant_ranges: &[],
            });
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("filter shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {{ let corners = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(corners[index], 0.0, 1.0); }}\n@fragment fn fragment() -> @location(0) vec4f {{ let light = canopy_light(vec3f(0.03, 0.0, 0.0)); return vec4f(light, light, light, 1.0); }}",
                    canopy_light_wgsl(&Gobo::default())
                )
                .into(),
            ),
        });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("filter pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
        let target = gpu
            .offscreen(1, 1, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let mut sample = |angular_radius, sharpness| {
            canopy.current.direction[3] = angular_radius;
            canopy.set_sharpness(sharpness).unwrap();
            gpu.queue
                .write_buffer(&canopy.params, 0, bytemuck::bytes_of(&canopy.current));
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("filter sample"),
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
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&pipeline);
                for group in 0..3 {
                    pass.set_bind_group(group, &empty_bind, &[]);
                }
                pass.set_bind_group(3, &cookie_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            gpu.queue.submit([encoder.finish()]);
            gpu.readback_rgba8(&target).unwrap()[0]
        };
        let sharp = sample(0.0001, 1.0);
        let soft = sample(0.08, 1.0);
        let sharper = sample(0.08, 0.3);
        assert!(sharp < 5, "sharp={sharp}");
        assert!(soft > 100 && soft < 200, "soft={soft}");
        assert!(sharper < soft);
    }
    #[test]
    fn morning_and_evening_shade_tracks_projected_crown() {
        let tree = Tree::new(9, pfx_geom::tree::TreeSpec::plum(), 0.02);
        let origin = [2.0, 0.0, -3.0];
        let height = tree.spec.height * 0.8;
        let base = tree.spec.crown_width * 2.8;
        for hour in [7.0, 18.0] {
            let sun = pfx_core::daylight::Daylight {
                hour,
                day: pfx_core::daylight::Daylight::DAY,
                latitude: pfx_core::daylight::Daylight::LATITUDE,
                heading: pfx_core::daylight::Daylight::HEADING,
            }
            .sun()
            .y_up
            .map(|value| value as f32);
            let (center, extent) = projected_bounds(origin, height, base, sun, 0.0);
            let [u, v] = projection_axes(sun);
            let crown = [origin[0], origin[1] + height, origin[2]];
            let expected = [dot3(crown, u), dot3(crown, v)];
            assert!((center[0] - expected[0]).abs() < 1e-5);
            assert!((center[1] - expected[1]).abs() < 1e-5);
            assert!(extent[0] > base);
            let mut sum = [0.0; 2];
            let mut weight = 0.0;
            for card in &tree.cards {
                let world = [
                    origin[0] + card.position[0],
                    origin[1] + card.position[1],
                    origin[2] + card.position[2],
                ];
                let projected = [dot3(world, u), dot3(world, v)];
                let area = card.size[0] * card.size[1];
                sum[0] += projected[0] * area;
                sum[1] += projected[1] * area;
                weight += area;
            }
            let centroid = [sum[0] / weight, sum[1] / weight];
            assert!((centroid[0] - center[0]).abs() < extent[0] * 0.2);
            assert!((centroid[1] - center[1]).abs() < extent[1] * 0.2);
        }
    }
    #[test]
    fn vertical_receiver_uses_sun_perpendicular_coordinates() {
        let sun = [0.4, 0.7, 0.3];
        let [u, v] = projection_axes(sun);
        let receiver = [0.1, -0.2, 0.0];
        let along_sun = [
            receiver[0] + sun[0] * 3.0,
            receiver[1] + sun[1] * 3.0,
            receiver[2] + sun[2] * 3.0,
        ];
        assert!((dot3(receiver, u) - dot3(along_sun, u)).abs() < 1.0e-6);
        assert!((dot3(receiver, v) - dot3(along_sun, v)).abs() < 1.0e-6);
        assert!(dot3(u, v).abs() < 1.0e-6);
    }
    #[test]
    fn petals_are_seeded_and_rate_limited() {
        let tree = Tree::new(41, pfx_geom::tree::TreeSpec::cherry(), 0.02);
        let mut first = petal_emitters(&tree, [0.0; 3], 1.0, 1.0, [1.0, 0.0, 0.0], 5.0);
        let mut second = petal_emitters(&tree, [0.0; 3], 1.0, 1.0, [1.0, 0.0, 0.0], 5.0);
        assert_eq!(first.len(), second.len());
        assert!(first.len() < tree.petal_sources.len());
        let wind = pfx_physics::Wind::new(4, [1.0, 0.0, 0.0], 0.5);
        for frame in 0..1200 {
            let time = frame as f32 / 60.0;
            for emitter in &mut first {
                emitter.step(1.0 / 60.0, time, &wind);
            }
            for emitter in &mut second {
                emitter.step(1.0 / 60.0, time, &wind);
            }
        }
        let total = first.iter().map(|e| e.specks().len()).sum::<usize>();
        assert!(total > 0 && total < 40, "{total} live petals");
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.specks().len(), b.specks().len());
            for (left, right) in a.specks().iter().zip(b.specks()) {
                assert_eq!(left.pos, right.pos);
            }
        }
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use pfx_geom::tree::TreeSpec;
    use pfx_gpu::{Gpu, GpuProfiler};

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn low_sun_cookie_centroid() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let tree = Tree::new(9, TreeSpec::plum(), 0.02);
        let mut canopy = Canopy::new(&gpu.device, &gpu.queue, &tree, 256);
        let origin = [2.0, 0.0, -3.0];
        for hour in [7.0, 18.0] {
            let sun = pfx_core::daylight::Daylight {
                hour,
                day: pfx_core::daylight::Daylight::DAY,
                latitude: pfx_core::daylight::Daylight::LATITUDE,
                heading: pfx_core::daylight::Daylight::HEADING,
            }
            .sun()
            .y_up
            .map(|value| value as f32);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            canopy.update(
                &gpu.queue,
                &mut encoder,
                CanopyFrame {
                    origin,
                    time: 0.0,
                    strength: 0.0,
                    direction: [1.0, 0.0, 0.0],
                    sun,
                    receiver_y: 0.0,
                    angular_radius: 0.00465,
                },
                None,
            );
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("low sun cookie readback"),
                size: 256 * 256,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                canopy.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(256),
                    },
                },
                wgpu::Extent3d {
                    width: 256,
                    height: 256,
                    depth_or_array_layers: 1,
                },
            );
            gpu.queue.submit(Some(encoder.finish()));
            let (send, receive) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    send.send(result).unwrap();
                });
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range();
            let mut sum = [0.0_f32; 2];
            let mut weight = 0.0_f32;
            for (index, alpha) in mapped.iter().enumerate() {
                let x = index % 256;
                let y = index / 256;
                let uv = [(x as f32 + 0.5) / 256.0, (y as f32 + 0.5) / 256.0];
                let world = [
                    canopy.center[0] + (uv[0] - 0.5) * canopy.extent[0],
                    canopy.center[1] + (uv[1] - 0.5) * canopy.extent[1],
                ];
                let alpha = f32::from(*alpha) / 255.0;
                sum[0] += world[0] * alpha;
                sum[1] += world[1] * alpha;
                weight += alpha;
            }
            assert!(weight > 1.0);
            let centroid = [sum[0] / weight, sum[1] / weight];
            let [u, v] = projection_axes(sun);
            let crown = [origin[0], origin[1] + tree.spec.height * 0.8, origin[2]];
            let expected = [dot3(crown, u), dot3(crown, v)];
            assert!((centroid[0] - expected[0]).abs() < canopy.extent[0] * 0.2);
            assert!((centroid[1] - expected[1]).abs() < canopy.extent[1] * 0.2);
            drop(mapped);
            readback.unmap();
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct WoodVertex {
        position: [f32; 3],
        pivot: [f32; 3],
        level: f32,
        stiffness: f32,
    }

    const WOOD_WGSL: &str = r#"
struct CanopyParams {
    center_extent: vec4f,
    sun_receiver: vec4f,
    wind: vec4f,
    direction: vec4f,
    projected_extent: vec4f,
}
@group(0) @binding(0) var<uniform> params: CanopyParams;
fn tree_sway(rest: vec3f, pivot: vec3f, level: f32, stiffness: f32, wind_time: f32, strength: f32, direction: vec3f) -> vec3f {
    let d = normalize(direction.xz + vec2f(0.00001, 0.0));
    let arm = length(rest - pivot);
    let phase = wind_time * (0.7 + level * 0.41) + pivot.x * 0.83 + pivot.z * 0.67;
    let bend = sin(phase) * strength * (1.0 - stiffness) * arm * (0.012 + 0.009 * level);
    return rest + vec3f(d.x * bend, -abs(bend) * 0.12, d.y * bend);
}
struct Wood { @location(0) position: vec3f, @location(1) pivot: vec3f, @location(2) level: f32, @location(3) stiffness: f32, }
@vertex fn vertex(input: Wood) -> @builtin(position) vec4f {
    let p = tree_sway(input.position, input.pivot, input.level, input.stiffness, params.wind.x, params.wind.y, params.direction.xyz);
    return vec4f(p.x / 8.0, (p.y - p.z) / 14.0, 0.0, 1.0);
}
@fragment fn fragment() -> @location(0) vec4f { return vec4f(0.29, 0.16, 0.11, 1.0); }
"#;

    const GROUND_WGSL: &str = r#"
@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(p[i], 0.0, 1.0);
}
@fragment fn fragment(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let uv = pos.xy / vec2f(3840.0, 2160.0);
    let world = vec3f((uv.x * 2.0 - 1.0) * 8.0, 0.0, (uv.y * 2.0 - 1.0) * 14.0);
    let light = canopy_light(world);
    return vec4f(vec3f(0.65, 0.74, 0.52) * (0.3 + 0.7 * light), 1.0);
}
"#;

    #[test]
    fn preview_shaders_validate() {
        for source in [
            WOOD_WGSL.to_string(),
            format!("{}\n{GROUND_WGSL}", canopy_light_wgsl(&Gobo::default())),
        ] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    fn pipeline(
        device: &wgpu::Device,
        shader: &str,
        layout: &wgpu::PipelineLayout,
        buffers: &[wgpu::VertexBufferLayout<'_>],
        alpha: bool,
    ) -> wgpu::RenderPipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tree test shader"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tree test pipeline"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: alpha.then_some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        })
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn cherry_over_ground_at_two_wind_times() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let tree = Tree::new(49, TreeSpec::cherry(), 0.004);
        let mut canopy = Canopy::new(&gpu.device, &gpu.queue, &tree, DEFAULT_RESOLUTION);
        let vertices: Vec<WoodVertex> = tree
            .wood
            .positions
            .iter()
            .zip(&tree.sway)
            .map(|(position, sway)| WoodVertex {
                position: *position,
                pivot: sway.pivot,
                level: sway.level,
                stiffness: sway.stiffness,
            })
            .collect();
        let vertex_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cherry wood vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cherry wood indices"),
                contents: bytemuck::cast_slice(&tree.wood.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let empty = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("unused ground group"),
                entries: &[],
            });
        let empty_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("unused ground bind"),
            layout: &empty,
            entries: &[],
        });
        let ground_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ground layout"),
                bind_group_layouts: &[&empty, &empty, &empty, canopy.sample_layout()],
                push_constant_ranges: &[],
            });
        let wood_bind_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("wood wind layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
        let wood_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wood wind bind"),
            layout: &wood_bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: canopy.params.as_entire_binding(),
            }],
        });
        let wood_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("wood layout"),
                bind_group_layouts: &[&wood_bind_layout],
                push_constant_ranges: &[],
            });
        let camera_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cherry camera layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera: [f32; 16] = [
            1.0 / 8.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0 / 14.0,
            0.0,
            0.0,
            0.0,
            -1.0 / 14.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        let camera_buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cherry camera"),
                contents: bytemuck::cast_slice(&camera),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let camera_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cherry camera bind"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let visible_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cherry cards layout"),
                bind_group_layouts: &[canopy.cookie_layout(), &camera_layout],
                push_constant_ranges: &[],
            });
        let ground = pipeline(
            &gpu.device,
            &format!("{}\n{GROUND_WGSL}", canopy_light_wgsl(&Gobo::default())),
            &ground_layout,
            &[],
            false,
        );
        let wood = pipeline(
            &gpu.device,
            WOOD_WGSL,
            &wood_layout,
            &[wgpu::VertexBufferLayout {
                array_stride: 32,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 12,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 24,
                        shader_location: 2,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 28,
                        shader_location: 3,
                    },
                ],
            }],
            false,
        );
        let visible = pipeline(
            &gpu.device,
            &canopy_visible_wgsl(&CardColors::default()),
            &visible_layout,
            &[],
            true,
        );
        let ground_bind = canopy.sample_bind(&gpu.device);
        let target = gpu
            .offscreen(3840, 2160, wgpu::TextureFormat::Rgba8UnormSrgb)
            .unwrap();
        let mut images = Vec::new();
        for (index, time) in [0.0_f32, 2.3].into_iter().enumerate() {
            let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let cookie_time = profiler.pass("canopy cookie");
            let wood_time = profiler.pass("cherry draw 4K");
            canopy.update(
                &gpu.queue,
                &mut encoder,
                CanopyFrame {
                    origin: [0.0; 3],
                    time,
                    strength: 2.0,
                    direction: [1.0, 0.0, 0.35],
                    sun: [0.25, 0.95, 0.15],
                    receiver_y: 0.0,
                    angular_radius: 0.00465,
                },
                profiler.render_writes(cookie_time),
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ground and cherry"),
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
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&ground);
                pass.set_bind_group(0, &empty_bind, &[]);
                pass.set_bind_group(1, &empty_bind, &[]);
                pass.set_bind_group(2, &empty_bind, &[]);
                pass.set_bind_group(3, &ground_bind, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("cherry draw"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: profiler.render_writes(wood_time),
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&wood);
                pass.set_bind_group(0, &wood_bind, &[]);
                pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..tree.wood.indices.len() as u32, 0, 0..1);
                pass.set_pipeline(&visible);
                pass.set_bind_group(0, canopy.cookie_bind(), &[]);
                pass.set_bind_group(1, &camera_bind, &[]);
                pass.draw(0..6, 0..canopy.card_count);
            }
            let slot = profiler.finish(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            if let Some(slot) = slot {
                profiler.submitted(slot);
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                for t in profiler.collect(&gpu.device).into_iter().flatten() {
                    eprintln!("{} at {}: {:.3} ms", t.label, time, t.milliseconds);
                }
            }
            let pixels = gpu.readback_rgba8(&target).unwrap();
            let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
            std::fs::create_dir_all(&output).unwrap();
            let file =
                std::fs::File::create(output.join(format!("trees-wind-{index}.png"))).unwrap();
            let mut png = png::Encoder::new(file, 3840, 2160);
            png.set_color(png::ColorType::Rgba);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
            images.push(pixels);
        }
        let mut single = tree.clone();
        let card = tree
            .cards
            .iter()
            .max_by(|a, b| {
                let projected = |c: &pfx_geom::tree::Card| {
                    (c.right[0] * c.up[2] - c.right[2] * c.up[0]).abs() * c.size[0] * c.size[1]
                };
                projected(a).total_cmp(&projected(b))
            })
            .copied()
            .unwrap();
        single.cards = vec![card];
        let mut one = Canopy::new(&gpu.device, &gpu.queue, &single, DEFAULT_RESOLUTION);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        one.update(
            &gpu.queue,
            &mut encoder,
            CanopyFrame {
                origin: [0.0; 3],
                time: 0.0,
                strength: 0.0,
                direction: [1.0, 0.0, 0.0],
                sun: [0.0, 1.0, 0.0],
                receiver_y: 0.0,
                angular_radius: 0.00465,
            },
            None,
        );
        let row = DEFAULT_RESOLUTION;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("one card cookie readback"),
            size: u64::from(row) * u64::from(row),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            one.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(row),
                },
            },
            wgpu::Extent3d {
                width: row,
                height: row,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let (send, recv) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        recv.recv().unwrap().unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        let covered =
            mapped.iter().map(|v| *v as f32 / 255.0).sum::<f32>() * one.extent[0] * one.extent[1]
                / (row * row) as f32;
        let expected = projected_coverage(&single, 0.0, 0.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.0);
        assert!(
            (covered - expected).abs() < expected * 0.35,
            "cookie area {covered} versus projected {expected}"
        );
        drop(mapped);
        buffer.unmap();
        let sway_shader = format!(
            "{}\n@group(0) @binding(0) var<storage, read_write> result: array<vec4f, 1>; @compute @workgroup_size(1) fn main() {{ result[0] = vec4f(tree_sway(vec3f(1.0, 6.0, 1.0), vec3f(0.0, 5.0, 0.0), 2.0, 0.18, 1.0, 2.0, vec3f(1.0, 0.0, 0.3)), 1.0); }}",
            pfx_geom::tree::TREE_SWAY_WGSL
        );
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tree sway reference"),
                source: wgpu::ShaderSource::Wgsl(sway_shader.into()),
            });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tree sway reference layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tree sway reference pipeline layout"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("tree sway reference pipeline"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tree sway result"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tree sway readback"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tree sway reference bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 16);
        gpu.queue.submit(Some(encoder.finish()));
        let (send, recv) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap();
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        recv.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let expected = tree_sway(
            [1.0, 6.0, 1.0],
            [0.0, 5.0, 0.0],
            2.0,
            0.18,
            1.0,
            2.0,
            [1.0, 0.0, 0.3],
        );
        for (i, value) in expected.into_iter().enumerate() {
            let actual = f32::from_le_bytes(mapped[i * 4..i * 4 + 4].try_into().unwrap());
            assert!(
                (actual - value).abs() < 1e-4,
                "sway component {i}: {actual} versus {value}"
            );
        }
        drop(mapped);
        readback.unmap();
        let blossoms = images[0]
            .chunks_exact(4)
            .filter(|pixel| pixel[0] > pixel[1].saturating_add(10) && pixel[2] > pixel[1])
            .count();
        assert!(blossoms > 1000, "only {blossoms} blossom pixels");
        let changed = images[0]
            .chunks_exact(4)
            .zip(images[1].chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(changed > 1000, "only {changed} pixels changed");
    }
}
