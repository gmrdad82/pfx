use std::hash::{Hash, Hasher};

use bytemuck::{Pod, Zeroable};
use pfx_gpu::{GpuProfiler, wgpu};
use pfx_load::scene::Light;
use pfx_materials::Material;

use crate::frame::{Matrix, multiply};
use crate::shadow::{CASCADE_COUNT, FaceTiles};

pub const MAX_LIGHTS: usize = 4;
pub const FACES: usize = 6;
pub const CASTER_SLOTS: usize = CASCADE_COUNT + MAX_LIGHTS * FACES;
pub const CASTER_BYTES: u64 = 80;
pub const DEFAULT_FACE_RESOLUTION: u32 = 1024;
const MARGIN_TEXELS: f32 = 8.0;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const FACE_AXES: [([f32; 3], [f32; 3]); FACES] = [
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
];

pub const LIGHTS_WGSL: &str = r#"
const FIRST_TILE_LAYER: u32 = 3u;
struct LocalLight {
    position_radius: vec4f,
    colour_range: vec4f,
    shadow: vec4f,
    bias: vec4f,
}
struct LocalLights {
    info: vec4u,
    lights: array<LocalLight, 4>,
    faces: array<mat4x4f, 24>,
}
@group(1) @binding(4) var shadow_transmission: texture_2d_array<u32>;
@group(1) @binding(5) var<uniform> local_lights: LocalLights;
@group(1) @binding(6) var light_faces: texture_depth_2d_array;
@group(1) @binding(7) var light_sampler: sampler_comparison;
@group(1) @binding(8) var<storage, read> caster_tints: array<vec4f>;

fn caster_skips(material: u32) -> bool {
    return caster.mode.x > 0.5 && caster_tints[material].w > 0.5;
}

fn light_face(d: vec3f) -> u32 {
    let a = abs(d);
    if (a.x >= a.y && a.x >= a.z) {
        return select(1u, 0u, d.x > 0.0);
    }
    if (a.y >= a.z) {
        return select(3u, 2u, d.y > 0.0);
    }
    return select(5u, 4u, d.z > 0.0);
}

fn light_shadow(light: LocalLight, world: vec3f, normal: vec3f, toward: vec3f) -> f32 {
    if (light.shadow.x < 0.0) {
        return 1.0;
    }
    let offset = world - light.position_radius.xyz;
    let reach = max(abs(offset.x), max(abs(offset.y), abs(offset.z)));
    let texel = reach * light.shadow.y;
    let shifted = world + normal * texel * light.bias.x + toward * texel * light.bias.y;
    let layer = u32(light.shadow.x) + light_face(shifted - light.position_radius.xyz);
    let clip = local_lights.faces[layer] * vec4f(shifted, 1.0);
    if (clip.w <= 0.0) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2f(ndc.x, -ndc.y) * 0.5 + vec2f(0.5);
    let step = light.shadow.w / vec2f(textureDimensions(light_faces));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            lit += textureSampleCompareLevel(light_faces, light_sampler, uv + vec2f(f32(x), f32(y)) * step, layer, ndc.z);
        }
    }
    return lit / 9.0;
}

fn light_tint(light: LocalLight, world: vec3f, normal: vec3f, toward: vec3f) -> vec3f {
    let tile = local_lights.info.z;
    if (tile == 0u || light.shadow.x < 0.0) {
        return vec3f(1.0);
    }
    let offset = world - light.position_radius.xyz;
    let reach = max(abs(offset.x), max(abs(offset.y), abs(offset.z)));
    let texel = reach * light.shadow.y;
    let shifted = world + normal * texel * light.bias.x + toward * texel * light.bias.y;
    let face = u32(light.shadow.x) + light_face(shifted - light.position_radius.xyz);
    let clip = local_lights.faces[face] * vec4f(shifted, 1.0);
    if (clip.w <= 0.0) {
        return vec3f(1.0);
    }
    let ndc = clip.xyz / clip.w;
    let uv = clamp(vec2f(ndc.x, -ndc.y) * 0.5 + vec2f(0.5), vec2f(0.0), vec2f(1.0));
    let columns = max(local_lights.info.w, 1u);
    let per = columns * columns;
    let slot = face % per;
    let corner = vec2i(vec2u(slot % columns, slot / columns) * tile);
    let at = corner + min(vec2i(uv * f32(tile)), vec2i(i32(tile) - 1));
    let stored = textureLoad(shadow_transmission, at, FIRST_TILE_LAYER + face / per, 0);
    return select(vec3f(1.0), unpack4x8unorm(stored.x).xyz, ndc.z > bitcast<f32>(stored.y));
}

fn local_light_radiance(surface: Shaded, wo: vec3f, t: vec3f, b: vec3f, n: vec3f, world: vec3f, geometric: vec3f, film: vec3f) -> vec3f {
    var total = vec3f(0.0);
    let count = min(local_lights.info.x, 4u);
    for (var index = 0u; index < count; index++) {
        let light = local_lights.lights[index];
        let to_light = light.position_radius.xyz - world;
        let distance = length(to_light);
        let range = light.colour_range.w;
        if (distance >= range || distance <= 1e-6) {
            continue;
        }
        let l = to_light / distance;
        let ndl = dot(n, l);
        if (ndl <= 0.0 || dot(geometric, l) <= 0.0) {
            continue;
        }
        let ratio = distance / range;
        let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
        let reach = max(distance, light.position_radius.w);
        var lit = surface;
        lit.roughness = max(surface.roughness, sqrt(clamp(light.position_radius.w / (2.0 * distance), 0.0, 1.0)));
        let wi = vec3f(dot(l, t), dot(l, b), dot(l, n));
        let radiance = light.colour_range.xyz * window * window / (reach * reach);
        total += eval_filmed(lit, wo, wi, film) * radiance * ndl * light_shadow(light, world, geometric, l) * light_tint(light, world, geometric, l);
    }
    return total;
}

fn shadow_tint_one(world: vec3f, normal: vec3f, instance: u32, cascade: i32) -> vec3f {
    var view_proj = shadow_setting.view_proj_0;
    var texel = shadow_setting.texels.x;
    if (cascade == 1) {
        view_proj = shadow_setting.view_proj_1;
        texel = shadow_setting.texels.y;
    } else if (cascade == 2) {
        view_proj = shadow_setting.view_proj_2;
        texel = shadow_setting.texels.z;
    }
    let shifted = shadow_offset(world, normal, shadow_setting.sun.xyz, shadow_setting.filter_params, texel, -1.0);
    let projected = shadow_project(view_proj, shifted);
    if (any(projected < vec3f(0.0)) || any(projected > vec3f(1.0))) {
        return vec3f(1.0);
    }
    let size = vec2i(textureDimensions(shadow_transmission));
    let at = clamp(vec2i(projected.xy * vec2f(size)), vec2i(0), size - vec2i(1));
    let stored = textureLoad(shadow_transmission, at, cascade, 0);
    let behind = projected.z - shadow_setting.sun.w > bitcast<f32>(stored.y) && stored.z != instance + 1u;
    return select(vec3f(1.0), unpack4x8unorm(stored.x).xyz, behind);
}

fn shadow_tint(world: vec3f, normal: vec3f, instance: u32) -> vec3f {
    if (local_lights.info.y == 0u || shadow_setting.texels.w > 0.5) {
        return vec3f(1.0);
    }
    let distance = length(world - frame.camera_position.xyz);
    let choice = shadow_select(distance, shadow_setting.splits, shadow_setting.filter_params.w);
    let tint = shadow_tint_one(world, normal, instance, i32(choice.x));
    if (choice.z <= 0.0) {
        return tint;
    }
    return mix(tint, shadow_tint_one(world, normal, instance, i32(choice.y)), choice.z);
}

struct TransmitOutput {
    @builtin(position) position: vec4f,
    @location(0) world_position: vec3f,
    @location(1) @interpolate(flat) instance: u32,
    @location(2) @interpolate(flat) tint: vec4f,
}

@vertex fn transmit_vertex(input: VertexInput) -> TransmitOutput {
    let instance = instances[input.instance_index];
    var output: TransmitOutput;
    output.tint = caster_tints[instance.fields.x];
    output.instance = input.instance_index;
    if (output.tint.w < 0.5) {
        output.position = vec4f(2.0, 2.0, 2.0, 1.0);
        return output;
    }
    let position = caster_position(input, instance);
    let world = instance.model * vec4f(position, 1.0);
    output.position = caster.view_proj * world;
    output.world_position = world.xyz;
    return output;
}

@fragment fn transmit_fragment(input: TransmitOutput) -> @location(0) vec4u {
    if (clipped(input.instance, input.world_position)) {
        discard;
    }
    return vec4u(pack4x8unorm(vec4f(input.tint.xyz, 1.0)), bitcast<u32>(input.position.z), input.instance + 1u, 0u);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalLight {
    pub position: [f32; 3],
    pub colour: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
    pub range: f32,
    pub shadow: bool,
}

pub fn local_light(light: &Light) -> LocalLight {
    LocalLight {
        position: light.position,
        colour: light.color,
        intensity: light.intensity,
        radius: light.radius,
        range: light.range,
        shadow: light.shadow,
    }
}

impl LocalLight {
    pub fn faded(self, fade: f32) -> Self {
        Self {
            intensity: self.intensity * fade.max(0.0),
            ..self
        }
    }

    pub fn lit(&self) -> bool {
        self.intensity > 0.0 && self.range > 0.0 && self.colour.iter().any(|&value| value > 0.0)
    }

    fn valid(&self) -> bool {
        self.position
            .iter()
            .chain(&self.colour)
            .chain([&self.intensity, &self.radius, &self.range])
            .all(|value| value.is_finite())
            && self.colour.iter().all(|&value| value >= 0.0)
            && self.intensity >= 0.0
            && self.radius >= 0.0
            && self.range >= 0.0
    }

    pub fn near(&self) -> f32 {
        (self.radius * 0.5).clamp(0.002, (self.range * 0.5).max(0.002))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightQuality {
    pub resolution: u32,
    pub pcf_radius: f32,
    pub normal_bias: f32,
    pub depth_bias: f32,
}

impl Default for LightQuality {
    fn default() -> Self {
        Self {
            resolution: DEFAULT_FACE_RESOLUTION,
            pcf_radius: 1.5,
            normal_bias: 1.5,
            depth_bias: 1.5,
        }
    }
}

pub fn sheet_tint(material: &Material) -> Option<[f32; 3]> {
    (material.subsurface > 0.0).then(|| {
        let share = (2.0 * material.subsurface).min(1.0);
        material
            .base
            .map(|channel| (channel * share).clamp(0.0, 1.0))
    })
}

pub fn tan_half(resolution: u32) -> f32 {
    let margin = (2.0 * MARGIN_TEXELS / resolution.max(32) as f32).min(0.5);
    1.0 / (1.0 - margin)
}

pub fn face_view_proj(light: &LocalLight, face: usize, resolution: u32) -> Matrix {
    let (forward, up) = FACE_AXES[face % FACES];
    let right = cross(forward, up);
    let eye = light.position;
    let view = [
        [right[0], up[0], -forward[0], 0.0],
        [right[1], up[1], -forward[1], 0.0],
        [right[2], up[2], -forward[2], 0.0],
        [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
    ];
    let near = light.near();
    let far = light.range.max(near * 2.0);
    let f = 1.0 / tan_half(resolution);
    let projection = [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ];
    multiply(projection, view)
}

pub fn face_of(direction: [f32; 3]) -> usize {
    let a = direction.map(f32::abs);
    if a[0] >= a[1] && a[0] >= a[2] {
        return if direction[0] > 0.0 { 0 } else { 1 };
    }
    if a[1] >= a[2] {
        return if direction[1] > 0.0 { 2 } else { 3 };
    }
    if direction[2] > 0.0 { 4 } else { 5 }
}

pub fn falloff(light: &LocalLight, distance: f32) -> f32 {
    if distance >= light.range || distance <= 1e-6 {
        return 0.0;
    }
    let ratio = distance / light.range;
    let window = (1.0 - ratio.powi(4)).clamp(0.0, 1.0);
    let reach = distance.max(light.radius);
    light.intensity * window * window / (reach * reach)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLight {
    position_radius: [f32; 4],
    colour_range: [f32; 4],
    shadow: [f32; 4],
    bias: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLights {
    info: [u32; 4],
    lights: [GpuLight; MAX_LIGHTS],
    faces: [Matrix; MAX_LIGHTS * FACES],
}

struct FaceMaps {
    resolution: u32,
    layers: u32,
    texture: wgpu::Texture,
    backup: wgpu::Texture,
    array: wgpu::TextureView,
    views: Vec<wgpu::TextureView>,
}

pub type FaceDraw<'d> = dyn FnMut(&mut wgpu::RenderPass<'_>, usize, bool) + 'd;

pub struct Lights {
    lights: Vec<LocalLight>,
    quality: LightQuality,
    uniform: wgpu::Buffer,
    maps: Option<FaceMaps>,
    empty_faces: wgpu::TextureView,
    empty_transmission: wgpu::TextureView,
    sampler: wgpu::Sampler,
    tints: Vec<Option<[f32; 3]>>,
    table: wgpu::Buffer,
    table_capacity: usize,
    written: Vec<[f32; 4]>,
    cache: Option<u64>,
    clean: bool,
    reused: bool,
    extended: bool,
}

impl Lights {
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live local lights"),
            size: std::mem::size_of::<GpuLights>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let empty_faces = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("live empty light faces"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let empty_transmission = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("live empty transmission"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: crate::shadow::TRANSMISSION_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("live light compare"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let table_capacity = 1;
        Self {
            lights: Vec::new(),
            quality: LightQuality::default(),
            uniform,
            maps: None,
            empty_faces,
            empty_transmission,
            sampler,
            tints: Vec::new(),
            table: table_buffer(device, table_capacity),
            table_capacity,
            written: Vec::new(),
            cache: None,
            clean: false,
            reused: false,
            extended: false,
        }
    }

    pub fn set(&mut self, lights: &[LocalLight]) -> Result<(), String> {
        if lights.len() > MAX_LIGHTS {
            return Err(format!("at most {MAX_LIGHTS} local lights"));
        }
        if !lights.iter().all(LocalLight::valid) {
            return Err("local lights need finite, nonnegative values".into());
        }
        self.lights = lights.to_vec();
        Ok(())
    }

    pub fn lights(&self) -> &[LocalLight] {
        &self.lights
    }

    pub fn set_quality(&mut self, quality: LightQuality) -> Result<(), String> {
        if quality.resolution < 32
            || ![quality.pcf_radius, quality.normal_bias, quality.depth_bias]
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
        {
            return Err("light shadow quality needs at least 32 texels and finite biases".into());
        }
        self.quality = quality;
        Ok(())
    }

    pub fn quality(&self) -> LightQuality {
        self.quality
    }

    pub fn set_transmission(&mut self, tints: &[Option<[f32; 3]>]) -> Result<(), String> {
        if tints
            .iter()
            .flatten()
            .flatten()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("transmission tints must be fractions".into());
        }
        self.tints = tints.to_vec();
        Ok(())
    }

    pub fn transmissive(&self, material: u32) -> bool {
        self.tints
            .get(material as usize)
            .is_some_and(Option::is_some)
    }

    pub fn any_transmissive(&self) -> bool {
        self.tints.iter().any(Option::is_some)
    }

    pub fn table(&self) -> &wgpu::Buffer {
        &self.table
    }

    pub fn table_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for tint in &self.tints {
            tint.map(|value| value.map(f32::to_bits)).hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        materials: usize,
    ) -> bool {
        let rows: Vec<[f32; 4]> = (0..materials.max(1))
            .map(|index| match self.tints.get(index).copied().flatten() {
                Some(tint) => [tint[0], tint[1], tint[2], 1.0],
                None => [1.0, 1.0, 1.0, 0.0],
            })
            .collect();
        let mut grown = false;
        if rows.len() > self.table_capacity {
            self.table_capacity = rows.len().next_power_of_two();
            self.table = table_buffer(device, self.table_capacity);
            self.written.clear();
            grown = true;
        }
        if rows != self.written {
            queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&rows));
            self.written = rows;
        }
        grown
    }

    pub fn shadowed(&self) -> Vec<(usize, u32)> {
        self.lights
            .iter()
            .enumerate()
            .filter(|(_, light)| light.lit() && light.shadow)
            .enumerate()
            .map(|(slot, (index, _))| (index, (slot * FACES) as u32))
            .collect()
    }

    pub fn faces(&self) -> Vec<(u32, Matrix)> {
        let resolution = self.quality.resolution;
        self.shadowed()
            .into_iter()
            .flat_map(|(index, first)| {
                let light = self.lights[index];
                (0..FACES).map(move |face| {
                    (
                        first + face as u32,
                        face_view_proj(&light, face, resolution),
                    )
                })
            })
            .collect()
    }

    pub fn uniform(&self) -> &wgpu::Buffer {
        &self.uniform
    }

    pub fn faces_view(&self) -> &wgpu::TextureView {
        self.maps
            .as_ref()
            .map_or(&self.empty_faces, |maps| &maps.array)
    }

    pub fn empty_transmission(&self) -> &wgpu::TextureView {
        &self.empty_transmission
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn reused(&self) -> bool {
        self.reused
    }

    pub fn extended(&self) -> bool {
        self.extended
    }

    pub fn write_uniform(&mut self, queue: &wgpu::Queue, transmission: bool, tiles: FaceTiles) {
        let mut data = GpuLights::zeroed();
        let shadowed = self.shadowed();
        let resolution = self.quality.resolution;
        let mut count = 0;
        for (index, light) in self.lights.iter().enumerate() {
            if !light.lit() {
                continue;
            }
            let layer = shadowed
                .iter()
                .find(|(shadowed, _)| *shadowed == index)
                .map_or(-1.0, |(_, first)| *first as f32);
            data.lights[count] = GpuLight {
                position_radius: [
                    light.position[0],
                    light.position[1],
                    light.position[2],
                    light.radius,
                ],
                colour_range: [
                    light.colour[0] * light.intensity,
                    light.colour[1] * light.intensity,
                    light.colour[2] * light.intensity,
                    light.range,
                ],
                shadow: [
                    layer,
                    2.0 * tan_half(resolution) / resolution as f32,
                    0.0,
                    self.quality.pcf_radius,
                ],
                bias: [self.quality.normal_bias, self.quality.depth_bias, 0.0, 0.0],
            };
            count += 1;
        }
        for (layer, matrix) in self.faces() {
            data.faces[layer as usize] = matrix;
        }
        let tiles = if transmission {
            tiles
        } else {
            FaceTiles::default()
        };
        data.info = [
            count as u32,
            u32::from(transmission),
            tiles.tile,
            tiles.columns,
        ];
        self.extended = count > 0 || transmission;
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        content: u64,
        moving: bool,
        draw: &mut FaceDraw<'_>,
        profiler: Option<&mut GpuProfiler>,
    ) {
        let shadowed = self.shadowed();
        if shadowed.is_empty() {
            self.reused = false;
            return;
        }
        let layers = (shadowed.len() * FACES) as u32;
        let resolution = self
            .quality
            .resolution
            .min(device.limits().max_texture_dimension_2d);
        if self
            .maps
            .as_ref()
            .is_none_or(|maps| maps.layers != layers || maps.resolution != resolution)
        {
            self.maps = Some(face_maps(device, resolution, layers));
            self.cache = None;
            self.clean = false;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        resolution.hash(&mut hasher);
        for (index, _) in &shadowed {
            let light = self.lights[*index];
            for value in light.position.iter().chain([&light.range, &light.radius]) {
                value.to_bits().hash(&mut hasher);
            }
        }
        let key = hasher.finish();
        let hit = self.cache == Some(key);
        if hit && self.clean && !moving {
            self.reused = true;
            return;
        }
        self.reused = hit;
        let Some(maps) = &self.maps else {
            return;
        };
        let total = (usize::from(!hit) + usize::from(moving)) * maps.views.len();
        let mut profiler = profiler;
        let timing = if total > 0 {
            profiler
                .as_deref_mut()
                .and_then(|timer| timer.pass("light shadows"))
        } else {
            None
        };
        let mut faces = Faces {
            done: 0,
            total,
            timing,
            profiler: profiler.as_deref(),
        };
        if hit {
            if !self.clean {
                copy_layers(encoder, &maps.backup, &maps.texture, resolution, layers);
            }
        } else {
            faces.encode(encoder, maps, true, draw);
            copy_layers(encoder, &maps.texture, &maps.backup, resolution, layers);
        }
        if moving {
            faces.encode(encoder, maps, false, draw);
        }
        self.cache = Some(key);
        self.clean = !moving;
    }
}

struct Faces<'p> {
    done: usize,
    total: usize,
    timing: Option<u32>,
    profiler: Option<&'p GpuProfiler>,
}

impl Faces<'_> {
    fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        maps: &FaceMaps,
        statics: bool,
        draw: &mut FaceDraw<'_>,
    ) {
        for (layer, view) in maps.views.iter().enumerate() {
            let writes = self
                .profiler
                .and_then(|timer| timer.render_writes(self.timing))
                .map(|writes| wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: writes
                        .beginning_of_pass_write_index
                        .filter(|_| self.done == 0),
                    end_of_pass_write_index: writes
                        .end_of_pass_write_index
                        .filter(|_| self.done + 1 == self.total),
                })
                .filter(|writes| {
                    writes.beginning_of_pass_write_index.is_some()
                        || writes.end_of_pass_write_index.is_some()
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("light shadow face"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: if statics {
                            wgpu::LoadOp::Clear(1.0)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: writes,
                occlusion_query_set: None,
            });
            draw(&mut pass, layer, statics);
            self.done += 1;
        }
    }
}

fn table_buffer(device: &wgpu::Device, rows: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("live caster tints"),
        size: (rows.max(1) * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn face_maps(device: &wgpu::Device, resolution: u32, layers: u32) -> FaceMaps {
    let size = wgpu::Extent3d {
        width: resolution,
        height: resolution,
        depth_or_array_layers: layers,
    };
    let make = |label| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let texture = make("live light faces");
    let backup = make("live light faces static");
    let array = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let views = (0..layers)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    FaceMaps {
        resolution,
        layers,
        texture,
        backup,
        array,
        views,
    }
}

fn copy_layers(
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::Texture,
    to: &wgpu::Texture,
    resolution: u32,
    layers: u32,
) {
    encoder.copy_texture_to_texture(
        from.as_image_copy(),
        to.as_image_copy(),
        wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: layers,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Camera, Frame, Instance, MeshData, Scene, SceneWind, Sun, transform};
    use crate::probes::{PROBE_WGSL, ProbeLighting};
    use crate::shadow::{Quality, Shadows, View};
    use half::f16;
    use pfx_bake::{BakeScene, Emitters, Grid, GridSpec, Probe, write_artifact_with_emitters};
    use pfx_gpu::Gpu;
    use std::path::Path;

    const SIZE: u32 = 256;

    #[test]
    fn light_tiles_start_after_the_cascades() {
        assert!(LIGHTS_WGSL.contains(&format!("const FIRST_TILE_LAYER: u32 = {CASCADE_COUNT}u;")));
    }

    fn light() -> LocalLight {
        LocalLight {
            position: [0.5, 1.0, 0.0],
            colour: [1.0; 3],
            intensity: 1.0,
            radius: 0.0,
            range: 10.0,
            shadow: true,
        }
    }

    #[test]
    fn each_face_sees_its_own_axis_inside_the_margin() {
        let light = light();
        let resolution = 512;
        for direction in [
            [1.0, 0.2, -0.3],
            [-0.7, 0.69, 0.1],
            [0.1, 0.9, 0.3],
            [0.2, -1.0, 0.999],
            [0.4, -0.2, 0.6],
            [-0.3, 0.3, -0.31],
            [0.577, 0.577, 0.577],
        ] {
            let face = face_of(direction);
            let matrix = face_view_proj(&light, face, resolution);
            let mut previous = -1.0;
            for distance in [0.05, 0.5, 2.0, 9.0] {
                let length = dot(direction, direction).sqrt();
                let point = [0, 1, 2]
                    .map(|axis| light.position[axis] + direction[axis] / length * distance);
                let clip = transform(matrix, [point[0], point[1], point[2], 1.0]);
                let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
                let inside = 1.0 / tan_half(resolution) + 1e-5;
                assert!(
                    ndc[0].abs() <= inside && ndc[1].abs() <= inside,
                    "{direction:?} {ndc:?}"
                );
                assert!(ndc[2] > previous && ndc[2] < 1.0, "{direction:?} {ndc:?}");
                previous = ndc[2];
            }
        }
        let margin = (1.0 - 1.0 / tan_half(resolution)) * 0.5 * resolution as f32;
        assert!((margin - MARGIN_TEXELS).abs() < 1e-3);
    }

    #[test]
    fn falloff_is_inverse_square_and_ends_at_the_range() {
        let light = LocalLight {
            intensity: 0.88,
            radius: 0.06,
            range: 3.0,
            ..light()
        };
        let near = falloff(&light, 0.5);
        assert!((near - 0.88 / 0.25).abs() / near < 2e-3);
        assert_eq!(falloff(&light, 3.0), 0.0);
        assert!(falloff(&light, 0.01) <= 0.88 / (0.06 * 0.06));
        assert!(!light.faded(0.0).lit());
        assert_eq!(light.faded(4.0).intensity, 0.88 * 4.0);
    }

    #[test]
    fn sheet_tint_is_base_times_twice_the_translucency() {
        let paper = Material {
            base: [0.9, 0.85, 0.7],
            subsurface: 0.22,
            ..Material::default()
        };
        let tint = sheet_tint(&paper).unwrap();
        for (channel, base) in tint.iter().zip(paper.base) {
            assert!((channel - base * 0.44).abs() < 1e-6);
        }
        let thick = Material {
            subsurface: 0.7,
            ..paper
        };
        assert_eq!(sheet_tint(&thick).unwrap(), paper.base);
        assert_eq!(sheet_tint(&Material::default()), None);
    }

    #[test]
    fn light_settings_are_checked() {
        let broken = LocalLight {
            intensity: f32::NAN,
            ..light()
        };
        assert!(!broken.valid());
        assert!(light().valid());
        assert!(
            !LocalLight {
                range: -1.0,
                ..light()
            }
            .valid()
        );
    }

    fn quad(centre: [f32; 3], half: [f32; 2]) -> (Vec<[f32; 3]>, Vec<u32>) {
        let [x, y, z] = centre;
        (
            vec![
                [x - half[0], y, z + half[1]],
                [x + half[0], y, z + half[1]],
                [x + half[0], y, z - half[1]],
                [x - half[0], y, z - half[1]],
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn upload(frame: &mut Frame, centre: [f32; 3], half: [f32; 2]) -> crate::frame::MeshHandle {
        let (positions, indices) = quad(centre, half);
        frame
            .upload_mesh(MeshData {
                positions: &positions,
                normals: &[[0.0, 1.0, 0.0]; 4],
                tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
                uvs: &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                uvs1: None,
                alpha: None,
                indices: &indices,
            })
            .unwrap()
    }

    fn identity() -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn camera() -> Camera {
        let near = 0.1;
        let far = 100.0;
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        let projection = [
            [f, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ];
        let view = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, -2.0, 1.0],
        ];
        Camera {
            view,
            projection,
            previous_view_projection: multiply(projection, view),
            position: [0.0, 2.0, 0.0],
        }
    }

    fn pixel(x: f32, z: f32) -> usize {
        let extent = 2.0 * (25.0_f32).to_radians().tan();
        let column = ((x / extent * 0.5 + 0.5) * SIZE as f32) as usize;
        let row = ((z / extent * 0.5 + 0.5) * SIZE as f32) as usize;
        row * SIZE as usize + column
    }

    fn matte(base: [f32; 3]) -> Material {
        Material {
            base,
            roughness: 1.0,
            specular: 0.0,
            ..Material::default()
        }
    }

    struct Bench {
        frame: Frame,
        shadows: Shadows,
        instances: Vec<Instance>,
        materials: Vec<Material>,
    }

    impl Bench {
        fn render(&mut self, sun: f32) -> Vec<[f32; 3]> {
            let view = View {
                eye: [0.0, 2.0, 0.0],
                forward: [0.0, -1.0, 0.0],
                up: [0.0, 0.0, -1.0],
                fov_y: 50.0_f32.to_radians(),
                aspect: 1.0,
                near: 0.1,
                far: 100.0,
            };
            let toward = [0.70710677, 0.70710677, 0.0];
            let fit = self.shadows.fit(&view, toward);
            let scene = Scene {
                camera: camera(),
                time: 0.0,
                seed: 7,
                sun: Sun {
                    direction: toward,
                    colour: [1.0; 3],
                    intensity: sun,
                },
                instances: &self.instances,
                materials: &self.materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            let mut encoder = self
                .frame
                .gpu
                .device
                .create_command_encoder(&Default::default());
            self.frame
                .encode(&scene, &mut encoder, Some((&mut self.shadows, &fit)), None)
                .unwrap();
            self.frame.gpu.queue.submit(Some(encoder.finish()));
            self.frame
                .gpu
                .readback_rgba16(&self.frame.targets.hdr)
                .unwrap()
                .chunks_exact(4)
                .map(|texel| [0, 1, 2].map(|k| f16::from_bits(texel[k]).to_f32()))
                .collect()
        }
    }

    fn bench() -> Bench {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
        let shadows = Shadows::new(
            &frame.gpu.device,
            Quality {
                resolution: 1024,
                receiver: Some(crate::shadow::ReceiverBox {
                    min: [-1.0, -0.1, -1.0],
                    max: [1.0, 1.0, 1.0],
                }),
                ..Default::default()
            },
        );
        let floor = upload(&mut frame, [0.0, 0.0, 0.0], [2.0, 2.0]);
        Bench {
            frame,
            shadows,
            instances: vec![Instance::new(floor, identity(), 0, 1)],
            materials: vec![matte([0.5; 3])],
        }
    }

    fn lambert(albedo: f32, light: &LocalLight, point: [f32; 3]) -> f32 {
        let to = [0, 1, 2].map(|k| light.position[k] - point[k]);
        let distance = dot(to, to).sqrt();
        albedo / std::f32::consts::PI * falloff(light, distance) * to[1] / distance
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn a_point_light_shadows_a_plane() {
        let mut bench = bench();
        let blocker = upload(&mut bench.frame, [0.25, 0.5, 0.0], [0.1, 0.1]);
        bench
            .instances
            .push(Instance::new(blocker, identity(), 0, 2));
        let dark = bench.render(0.0);
        bench.frame.set_lights(&[light()]).unwrap();
        let lit = bench.render(0.0);
        let again = bench.render(0.0);
        assert!(bench.frame.lights().reused());
        assert_eq!(lit, again);
        bench
            .frame
            .set_lights(&[LocalLight {
                shadow: false,
                ..light()
            }])
            .unwrap();
        let open = bench.render(0.0);
        let lamp =
            |image: &[[f32; 3]], x: f32, z: f32| image[pixel(x, z)][1] - dark[pixel(x, z)][1];
        let side = lambert(0.5, &light(), [-0.5, 0.0, 0.0]);
        let centre = lambert(0.5, &light(), [0.0, 0.0, 0.0]);
        println!(
            "lamp at the side {:.4} (expected {side:.4}), centre shadowed {:.4}, centre open {:.4} (expected {centre:.4})",
            lamp(&lit, -0.5, 0.0),
            lamp(&lit, 0.0, 0.0),
            lamp(&open, 0.0, 0.0)
        );
        assert!((lamp(&lit, -0.5, 0.0) - side).abs() < side * 0.03);
        assert!((lamp(&open, 0.0, 0.0) - centre).abs() < centre * 0.03);
        assert!(lamp(&lit, 0.0, 0.0).abs() < centre * 0.03);
        assert!(lamp(&lit, 0.0, 0.3).abs() > centre * 0.5);
        bench.frame.set_lights(&[]).unwrap();
        assert_eq!(bench.render(0.0), dark);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn thin_casters_tint_the_sun_and_opaque_casters_still_block_it() {
        let mut bench = bench();
        let paper = upload(&mut bench.frame, [0.3, 0.5, -0.35], [0.2, 0.25]);
        let card = upload(&mut bench.frame, [0.3, 0.5, 0.35], [0.2, 0.25]);
        let stock = Material {
            subsurface: 0.3,
            ..matte([0.9, 0.6, 0.3])
        };
        bench.materials.extend([stock, matte([0.4; 3])]);
        bench.instances.extend([
            Instance::new(paper, identity(), 1, 2),
            Instance::new(card, identity(), 2, 3),
        ]);
        let ambient = bench.render(0.0);
        let opaque = bench.render(1.0);
        let tints: Vec<Option<[f32; 3]>> = bench.materials.iter().map(sheet_tint).collect();
        bench.frame.set_transmission(&tints).unwrap();
        let through = bench.render(1.0);
        let sun = |image: &[[f32; 3]], x: f32, z: f32| {
            let at = pixel(x, z);
            [0, 1, 2].map(|k| image[at][k] - ambient[at][k])
        };
        let open = sun(&through, -0.7, -0.35);
        let behind_paper = sun(&through, -0.2, -0.35);
        let behind_card = sun(&through, -0.2, 0.35);
        let expected = sheet_tint(&stock).unwrap();
        println!(
            "open {open:?}, behind paper {behind_paper:?} (tint {expected:?}), behind card {behind_card:?}"
        );
        for k in 0..3 {
            assert!((behind_paper[k] / open[k] - expected[k]).abs() < 0.02);
            assert!(behind_card[k].abs() < open[k] * 0.02);
            assert!(sun(&opaque, -0.2, -0.35)[k].abs() < open[k] * 0.02);
        }
        for (x, z) in [(-0.7, -0.35), (-0.2, 0.35), (-0.7, 0.6), (0.8, -0.8)] {
            assert_eq!(sun(&opaque, x, z), sun(&through, x, z), "{x} {z}");
        }
        bench.frame.set_transmission(&[]).unwrap();
        assert_eq!(bench.render(1.0), opaque);
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn emission_scale_fades_the_baked_lamp() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let spec = GridSpec {
            min: [0.0; 3],
            max: [0.0; 3],
            spacing: 1.0,
        };
        let grid = |value: f32| {
            Grid::new(
                spec,
                vec![Probe {
                    lobes: [[value; 3]; 6],
                    visibility: [100.0; 6],
                }],
            )
            .unwrap()
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp");
        std::fs::create_dir_all(&root).unwrap();
        let sample = |baked: f32, live: f32| {
            let name = format!("live-emission-test-{baked}-{live}");
            let path = root.join(&name);
            if path.exists() {
                std::fs::remove_dir_all(&path).unwrap();
            }
            let scene = BakeScene {
                triangles: Vec::new(),
                shapes: Vec::new(),
                materials: vec![Material::default()],
                anchors: vec![pfx_bake::Anchor {
                    hour: 22.0,
                    sky: pfx_load::Sky {
                        width: 1,
                        height: 1,
                        texels: vec![[0.0; 4]],
                    },
                    sun: pfx_trace::Sun {
                        direction: [0.0, 1.0, 0.0],
                        color: [0.0; 3],
                        intensity: 0.0,
                    },
                }],
            };
            write_artifact_with_emitters(
                &root,
                Path::new(&name),
                &scene,
                &[grid(0.1 + 0.5 * baked)],
                &Emitters {
                    grid: grid(0.5),
                    scales: vec![baked],
                    direct: false,
                },
                1,
                1,
            )
            .unwrap();
            let mut lighting =
                ProbeLighting::load(&gpu.device, &gpu.queue, &path, 22.0, [0.0; 3]).unwrap();
            lighting.set_emission(&gpu.queue, live).unwrap();
            let value = probe_value(&gpu, &lighting);
            std::fs::remove_dir_all(&path).unwrap();
            value
        };
        let off = sample(0.0, 0.0);
        let on = sample(0.0, 2.0);
        let kept = sample(1.0, 1.0);
        let removed = sample(1.0, 0.0);
        println!("off {off:.4}, on {on:.4}, kept {kept:.4}, removed {removed:.4}");
        assert!((off - 0.1).abs() < 1e-3);
        assert!((on - 1.1).abs() < 1e-2);
        assert!((kept - 0.6).abs() < 1e-2);
        assert!((removed - 0.1).abs() < 1e-3);
    }

    fn probe_value(gpu: &Gpu, lighting: &ProbeLighting) -> f32 {
        let empty = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[],
            });
        let empty_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &empty,
            entries: &[],
        });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&empty, &empty, &empty, lighting.layout()],
                push_constant_ranges: &[],
            });
        let source = format!(
            "{PROBE_WGSL}\n@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {{ var p = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0)); return vec4f(p[i], 0.0, 1.0); }}\n@fragment fn fs() -> @location(0) vec4f {{ return vec4f(sample_live_irradiance(vec3f(0.0), vec3f(0.0, 1.0, 0.0)), 1.0); }}"
        );
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });
        let target = gpu
            .offscreen(1, 1, wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
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
            pass.set_bind_group(0, &empty_group, &[]);
            pass.set_bind_group(1, &empty_group, &[]);
            pass.set_bind_group(2, &empty_group, &[]);
            pass.set_bind_group(3, lighting.group(), &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        f16::from_bits(gpu.readback_rgba16(&target).unwrap()[0]).to_f32()
    }
}
