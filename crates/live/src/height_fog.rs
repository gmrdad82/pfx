use bytemuck::{Pod, Zeroable};
use pfx_gpu::wgpu;
use wgpu::util::DeviceExt;

use crate::effects::{
    EffectCommands, EffectFrame, EffectTargets, HDR_FORMAT, SCENE_DEPTH_FORMAT, VolumeTimestamps,
    buffer_entry, color_attachment, pipeline_with, premultiplied_target, shadow_entry,
    texture_entry, uniform, volume_targets,
};
use crate::lights::LocalLight;

pub const FOG_SCALE: u32 = 2;
pub const FOG_LIGHTS: usize = 8;
pub const FOG_ABSORPTION: f32 = 0.3;
pub const FOG_SCATTERING: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightFog {
    pub height: f32,
    pub falloff: f32,
    pub density: f32,
    pub reach: f32,
    pub shafts: f32,
    pub steps: u32,
    pub color: [f32; 3],
    pub phase: f32,
    pub ambient: Option<[f32; 3]>,
}

impl Default for HeightFog {
    fn default() -> Self {
        Self {
            height: 0.0,
            falloff: 1.0,
            density: 0.1,
            reach: 300.0,
            shafts: 1.0,
            steps: 64,
            color: [1.0; 3],
            phase: 0.5,
            ambient: None,
        }
    }
}

pub fn sky_ambient(sh: &[[f32; 3]; 9]) -> [f32; 3] {
    sh[0].map(|value| (value * 0.282095 / std::f32::consts::PI).max(0.0))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FogLight {
    pub position_range: [f32; 4],
    pub color_radius: [f32; 4],
    pub spot: [f32; 4],
    pub cone: [f32; 4],
}

impl FogLight {
    pub fn point(light: &LocalLight) -> Self {
        let power = light.intensity.max(0.0);
        Self {
            position_range: [
                light.position[0],
                light.position[1],
                light.position[2],
                light.range,
            ],
            color_radius: [
                light.colour[0] * power,
                light.colour[1] * power,
                light.colour[2] * power,
                light.radius,
            ],
            spot: [0.0, -1.0, 0.0, -2.0],
            cone: [0.0; 4],
        }
    }
}

pub fn fog_lights(lights: &[LocalLight], eye: [f32; 3], reach: f32) -> Vec<FogLight> {
    let distance = |light: &LocalLight| {
        let d: [f32; 3] = std::array::from_fn(|axis| light.position[axis] - eye[axis]);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let mut near: Vec<(f32, &LocalLight)> = lights
        .iter()
        .filter(|light| light.lit())
        .map(|light| (distance(light), light))
        .filter(|(distance, light)| *distance < reach + light.range)
        .collect();
    near.sort_by(|left, right| left.0.total_cmp(&right.0));
    near.iter()
        .take(FOG_LIGHTS)
        .map(|(_, light)| FogLight::point(light))
        .collect()
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FogUniform {
    shape: [f32; 4],
    march: [f32; 4],
    color: [f32; 4],
    ambient: [f32; 4],
    lights: [FogLight; FOG_LIGHTS],
}

fn fog_uniform(fog: &HeightFog, ambient: [f32; 3], lights: &[FogLight]) -> FogUniform {
    let mut packed = [FogLight::default(); FOG_LIGHTS];
    for (slot, light) in packed.iter_mut().zip(lights) {
        *slot = *light;
    }
    let ambient = fog.ambient.unwrap_or(ambient);
    FogUniform {
        shape: [
            fog.height,
            fog.falloff.max(0.0),
            fog.density.max(0.0),
            fog.reach.max(0.0),
        ],
        march: [
            fog.shafts.clamp(0.0, 1.0),
            fog.steps.clamp(1, 256) as f32,
            fog.phase.clamp(-0.9, 0.9),
            lights.len().min(FOG_LIGHTS) as f32,
        ],
        color: [
            fog.color[0].max(0.0),
            fog.color[1].max(0.0),
            fog.color[2].max(0.0),
            0.0,
        ],
        ambient: [
            ambient[0].max(0.0),
            ambient[1].max(0.0),
            ambient[2].max(0.0),
            0.0,
        ],
        lights: packed,
    }
}

pub const FOG_COMMON_WGSL: &str = r#"
struct Frame {
    view_proj: mat4x4f, eye: vec4f, right: vec4f, up: vec4f, forward: vec4f,
    lens: vec4f, sun: vec4f, sun_color: vec4f, extent: vec4f,
    volume_lo: vec4f, volume_hi: vec4f, volume_color: vec4f, volume_control: vec4f,
}
struct Shadow {
    splits: vec4f, filter_params: vec4f, sun: vec4f, texels: vec4f,
    view_proj_0: mat4x4f, view_proj_1: mat4x4f, view_proj_2: mat4x4f,
}
struct Light { position_range: vec4f, color_radius: vec4f, spot: vec4f, cone: vec4f, }
struct Fog { shape: vec4f, march: vec4f, color: vec4f, ambient: vec4f, lights: array<Light, 8>, }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var scene_depth: texture_2d<f32>;
@group(0) @binding(2) var shadow_atlas: texture_depth_2d_array;
@group(0) @binding(3) var<uniform> shadow: Shadow;
@group(0) @binding(4) var<uniform> fog: Fog;
const ABSORPTION: f32 = 0.3;
const SCATTERING: f32 = 0.3;
const FRAC_4_PI: f32 = 0.07957747154594767;
const LIGHT_STEPS: u32 = 8u;
struct Out { @builtin(position) clip: vec4f, @location(0) uv: vec2f, }
@vertex fn vs_main(@builtin(vertex_index) vertex: u32) -> Out {
    let xy = array<vec2f, 3>(vec2f(-1.0,-1.0),vec2f(3.0,-1.0),vec2f(-1.0,3.0));
    var out: Out;
    out.clip = vec4f(xy[vertex], 0.0, 1.0);
    out.uv = xy[vertex] * vec2f(0.5,-0.5) + vec2f(0.5);
    return out;
}
fn pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let mixed = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (mixed >> 22u) ^ mixed;
}
fn interleaved_gradient_noise(pixel: vec2f, index: u32) -> f32 {
    let xy = pixel + 5.588238 * f32(index % 64u);
    return fract(52.9829189 * fract(0.06711056 * xy.x + 0.00583715 * xy.y));
}
fn henyey_greenstein(cosine: f32, g: f32) -> f32 {
    let denom = 1.0 + g * g - 2.0 * g * cosine;
    return FRAC_4_PI * (1.0 - g * g) / (denom * sqrt(denom));
}
fn fog_density(y: f32) -> f32 {
    return fog.shape.z * exp(min(-fog.shape.y * (y - fog.shape.x), 30.0));
}
fn optical_path(y: f32, rise: f32, length: f32) -> f32 {
    let fh = fog.shape.y * rise;
    if (abs(fh * length) > 0.00125) {
        return (fog_density(y) - fog_density(y + rise * length)) / fh;
    }
    return fog_density(y) * length;
}
fn sunlight(clip: vec4f, layer: i32, size: vec2u) -> f32 {
    let point = clip.xyz / clip.w;
    let uv = point.xy * vec2f(0.5, -0.5) + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || point.z < 0.0 || point.z > 1.0) { return 1.0; }
    let at = clamp(vec2i(uv * vec2f(size)), vec2i(0), vec2i(size) - vec2i(1));
    return select(0.0, 1.0, point.z - shadow.sun.w <= textureLoad(shadow_atlas, at, layer, 0));
}
fn light_span(index: u32, ray: vec3f) -> vec2f {
    let light = fog.lights[index];
    let to_light = light.position_range.xyz - frame.eye.xyz;
    let centre = dot(to_light, ray);
    let range = light.position_range.w;
    let gap = range * range - (dot(to_light, to_light) - centre * centre);
    if (gap <= 0.0) { return vec2f(1.0, 0.0); }
    let half_chord = sqrt(gap);
    return vec2f(centre - half_chord, centre + half_chord);
}
fn local_light(index: u32, p: vec3f, ray: vec3f) -> vec3f {
    let light = fog.lights[index];
    let to_light = light.position_range.xyz - p;
    let d2 = dot(to_light, to_light);
    let range = light.position_range.w;
    let ratio2 = d2 / max(range * range, 0.00000001);
    let window = clamp(1.0 - ratio2 * ratio2, 0.0, 1.0);
    let near = max(d2, light.color_radius.w * light.color_radius.w);
    var attenuation = window * window / max(near, 0.00000001);
    let l = to_light * inverseSqrt(max(d2, 0.000000000001));
    if (light.spot.w > -1.5) {
        let cd = dot(-light.spot.xyz, l);
        let spot = clamp(cd * light.cone.x + light.cone.y, 0.0, 1.0);
        attenuation *= spot * spot;
    }
    return light.color_radius.rgb * attenuation * henyey_greenstein(dot(ray, l), fog.march.z);
}
fn march(pixel: vec2f, uv: vec2f, scene_limit: f32) -> vec4f {
    let centre_clip = frame.view_proj * vec4f(frame.eye.xyz + frame.forward.xyz, 1.0);
    let centre = centre_clip.xy / centre_clip.w;
    let ray = normalize(frame.forward.xyz + frame.right.xyz * ((uv.x * 2.0 - 1.0 - centre.x) * frame.lens.x) + frame.up.xyz * ((1.0 - uv.y * 2.0 - centre.y) * frame.lens.y));
    let along = max(dot(ray, normalize(frame.forward.xyz)), 0.001);
    let span = min(scene_limit / along, fog.shape.w);
    if (span <= 0.0 || fog.shape.z <= 0.0) { return vec4f(0.0); }
    let steps = max(u32(fog.march.y), 1u);
    let step_length = span / f32(steps);
    let index = pcg(bitcast<u32>(frame.extent.z) ^ bitcast<u32>(frame.extent.w));
    let jitter = interleaved_gradient_noise(pixel, index);
    let sun = normalize(frame.sun.xyz);
    let sun_light = frame.sun_color.rgb * frame.sun_color.w * henyey_greenstein(dot(ray, sun), fog.march.z);
    let shafts = fog.march.x;
    let count = min(u32(fog.march.w), 8u);
    let extinction = ABSORPTION + SCATTERING;
    let shadowed = shafts > 0.0 && shadow.splits.w > 0.0;
    let atlas = textureDimensions(shadow_atlas);
    let origin_0 = shadow.view_proj_0 * vec4f(frame.eye.xyz, 1.0);
    let origin_1 = shadow.view_proj_1 * vec4f(frame.eye.xyz, 1.0);
    let origin_2 = shadow.view_proj_2 * vec4f(frame.eye.xyz, 1.0);
    let delta_0 = shadow.view_proj_0 * vec4f(ray, 0.0);
    let delta_1 = shadow.view_proj_1 * vec4f(ray, 0.0);
    let delta_2 = shadow.view_proj_2 * vec4f(ray, 0.0);
    var background = 1.0;
    var light = vec3f(0.0);
    for (var i = 0u; i < steps; i++) {
        if (background < 0.001) { break; }
        let distance = (f32(i) + jitter) * step_length;
        let p = frame.eye.xyz + ray * distance;
        let density = fog_density(p.y);
        background *= exp(-step_length * density * extinction);
        var sunlit = 1.0;
        if (shadowed) {
            let depth = distance * along;
            var clip = origin_0 + delta_0 * distance;
            var layer = 0;
            if (depth > shadow.splits.y) { clip = origin_1 + delta_1 * distance; layer = 1; }
            if (depth > shadow.splits.z) { clip = origin_2 + delta_2 * distance; layer = 2; }
            sunlit = mix(1.0, sunlight(clip, layer, atlas), shafts);
        }
        var sample_color = sun_light * sunlit + fog.ambient.rgb;
        light += fog.color.rgb * SCATTERING * density * step_length * sample_color * background;
    }
    for (var l = 0u; l < count; l++) {
        let chord = light_span(l, ray);
        let enter = max(chord.x, 0.0);
        let leave = min(chord.y, span);
        if (leave <= enter) { continue; }
        let light_step = (leave - enter) / f32(LIGHT_STEPS);
        for (var k = 0u; k < LIGHT_STEPS; k++) {
            let distance = enter + (f32(k) + jitter) * light_step;
            let p = frame.eye.xyz + ray * distance;
            let seen = exp(-optical_path(frame.eye.y, ray.y, distance) * extinction);
            light += fog.color.rgb * SCATTERING * fog_density(p.y) * light_step * local_light(l, p, ray) * seen;
        }
    }
    return vec4f(light, 1.0 - background);
}
"#;

const MARCH_MAIN_WGSL: &str = r#"
struct VolumeOut { @location(0) radiance: vec4f, @location(1) distance: f32, }
@fragment fn fs_main(in: Out) -> VolumeOut {
    let screen = vec2i(clamp(in.uv * frame.extent.xy, vec2f(0.0), frame.extent.xy - vec2f(1.0)));
    let scene_limit = textureLoad(scene_depth, screen, 0).x;
    var out: VolumeOut;
    out.radiance = march(in.clip.xy, in.uv, scene_limit);
    out.distance = scene_limit;
    return out;
}
"#;

pub const FOG_EDGE: f32 = 0.05;

const UPSAMPLE_MAIN_WGSL: &str = r#"
@group(0) @binding(5) var low_color: texture_2d<f32>;
@group(0) @binding(6) var low_depth: texture_2d<f32>;
override edge: f32 = 0.05;
override low_scale: f32 = 2.0;
@fragment fn fs_main(in: Out) -> @location(0) vec4f {
    let size = min(vec2f(textureDimensions(low_color)), ceil(frame.extent.xy / low_scale));
    let pixel = vec2i(in.clip.xy);
    let full = textureLoad(scene_depth, pixel, 0).x;
    let point = in.clip.xy / frame.extent.xy * size - vec2f(0.5);
    let base = vec2i(floor(point));
    let fraction = fract(point);
    let max_pixel = vec2i(size) - vec2i(1);
    var total = 0.0;
    var result = vec4f(0.0);
    var closest = 1e30;
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let at = clamp(base + vec2i(x, y), vec2i(0), max_pixel);
            let depth = textureLoad(low_depth, at, 0).x;
            let relative = abs(full - depth) / max(min(full, depth), 0.0001);
            closest = min(closest, relative);
            let spatial = select(1.0 - fraction.x, fraction.x, x == 1) * select(1.0 - fraction.y, fraction.y, y == 1);
            let d = relative / edge;
            let weight = (spatial + 0.0001) * max(1.0 - d * d, 0.0);
            result += textureLoad(low_color, at, 0) * weight;
            total += weight;
        }
    }
    if (closest > edge || total <= 0.000001) {
        return march(in.clip.xy, in.clip.xy / frame.extent.xy, full);
    }
    return result / total;
}
"#;

pub fn march_source() -> String {
    format!("{FOG_COMMON_WGSL}\n{MARCH_MAIN_WGSL}")
}

pub fn upsample_source() -> String {
    format!("{FOG_COMMON_WGSL}\n{UPSAMPLE_MAIN_WGSL}")
}

struct Targets {
    size: [u32; 2],
    _color: wgpu::Texture,
    color_view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

pub struct HeightFogPass {
    march_layout: wgpu::BindGroupLayout,
    upsample_layout: wgpu::BindGroupLayout,
    march_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    _empty_shadow: wgpu::Texture,
    empty_shadow_view: wgpu::TextureView,
    empty_shadow_data: wgpu::Buffer,
    targets: Option<Targets>,
}

pub struct FogInputs<'a> {
    pub fog: &'a HeightFog,
    pub ambient: [f32; 3],
    pub lights: &'a [FogLight],
}

impl HeightFogPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let march_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("height fog bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                texture_entry(1),
                shadow_entry(2),
                buffer_entry(
                    3,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                buffer_entry(
                    4,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });
        let upsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("height fog upsample bindings"),
            entries: &[
                buffer_entry(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                texture_entry(1),
                shadow_entry(2),
                buffer_entry(
                    3,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                buffer_entry(
                    4,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
                texture_entry(5),
                texture_entry(6),
            ],
        });
        let march_pipeline = pipeline_with(
            device,
            "height fog march",
            &march_source(),
            &march_layout,
            &volume_targets(),
            &[],
        );
        let upsample_pipeline = pipeline_with(
            device,
            "height fog upsample",
            &upsample_source(),
            &upsample_layout,
            &[premultiplied_target()],
            &[
                ("low_scale", f64::from(FOG_SCALE)),
                ("edge", f64::from(FOG_EDGE)),
            ],
        );
        let empty_shadow = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("height fog empty shadow"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 3,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let empty_shadow_view = empty_shadow.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let empty_shadow_data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("height fog empty shadow data"),
            contents: &[0; 256],
            usage: wgpu::BufferUsages::UNIFORM,
        });
        Self {
            march_layout,
            upsample_layout,
            march_pipeline,
            upsample_pipeline,
            _empty_shadow: empty_shadow,
            empty_shadow_view,
            empty_shadow_data,
            targets: None,
        }
    }

    pub fn textures(&self) -> Vec<wgpu::Texture> {
        self.targets
            .as_ref()
            .map(|targets| vec![targets._color.clone(), targets._depth.clone()])
            .unwrap_or_default()
    }

    fn ensure(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.size == size)
        {
            return;
        }
        let texture = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let color = texture("height fog half color", HDR_FORMAT);
        let depth = texture("height fog half depth", SCENE_DEPTH_FORMAT);
        self.targets = Some(Targets {
            size,
            color_view: color.create_view(&Default::default()),
            _color: color,
            depth_view: depth.create_view(&Default::default()),
            _depth: depth,
        });
    }

    pub fn render(
        &mut self,
        commands: &mut EffectCommands<'_>,
        frame: &EffectFrame,
        inputs: FogInputs<'_>,
        targets: EffectTargets<'_>,
        timestamps: VolumeTimestamps<'_>,
    ) {
        let full = [frame.extent[0] as u32, frame.extent[1] as u32];
        if full[0] == 0 || full[1] == 0 {
            return;
        }
        let half = [full[0].div_ceil(FOG_SCALE), full[1].div_ceil(FOG_SCALE)];
        self.ensure(commands.device, half);
        let Some(low) = self.targets.as_ref() else {
            return;
        };
        let frame_buffer = uniform(commands.device, frame);
        let fog_buffer = commands
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("height fog"),
                contents: bytemuck::bytes_of(&fog_uniform(
                    inputs.fog,
                    inputs.ambient,
                    inputs.lights,
                )),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let (shadow_view, shadow_data) = targets
            .shadow
            .unwrap_or((&self.empty_shadow_view, &self.empty_shadow_data));
        let march_bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("height fog bindings"),
                layout: &self.march_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: frame_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(targets.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(shadow_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: shadow_data.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: fog_buffer.as_entire_binding(),
                    },
                ],
            });
        {
            let mut pass = commands
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    multiview_mask: None,
                    label: Some("height fog half resolution"),
                    color_attachments: &[
                        Some(color_attachment(&low.color_view, true)),
                        Some(wgpu::RenderPassColorAttachment {
                            view: &low.depth_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        }),
                    ],
                    depth_stencil_attachment: None,
                    timestamp_writes: timestamps.march,
                    occlusion_query_set: None,
                });
            crate::viewport::apply(&mut pass, half);
            pass.set_pipeline(&self.march_pipeline);
            pass.set_bind_group(0, &march_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let upsample_bind = commands
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("height fog upsample bindings"),
                layout: &self.upsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: frame_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(targets.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(shadow_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: shadow_data.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: fog_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&low.color_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&low.depth_view),
                    },
                ],
            });
        let mut pass = commands
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("height fog upsample"),
                color_attachments: &[Some(color_attachment(targets.hdr, false))],
                depth_stencil_attachment: None,
                timestamp_writes: timestamps.upsample,
                occlusion_query_set: None,
            });
        crate::viewport::apply(&mut pass, full);
        pass.set_pipeline(&self.upsample_pipeline);
        pass.set_bind_group(0, &upsample_bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_march_shader_validates() {
        for source in [march_source(), upsample_source()] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
