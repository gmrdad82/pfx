use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use pfx_core::sim::particles::{Blend, Emitter, IDENTITY, Matrix, STEP, Sprite};
use pfx_gpu::wgpu;
use pfx_load::scene::{Attach, Particles, Scene};
use wgpu::util::DeviceExt;

use crate::effects::HDR_FORMAT;
use crate::lights::LocalLight;
use crate::probes::{PROBE_WGSL, ProbeLighting};

pub const MAX_LIGHTS: usize = 16;
pub const CATCH_UP: f32 = 120.0;
pub const MAX_SPAN: f32 = 0.1;
pub const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SprayInstance {
    pub position_size: [f32; 4],
    pub previous_angle: [f32; 4],
    pub velocity_stretch: [f32; 4],
    pub colour_alpha: [f32; 4],
    pub flags: [f32; 4],
}

impl SprayInstance {
    pub fn new(sprite: &Sprite, stretch: f32, blend: Blend, lit: bool, span: f32) -> Self {
        let [x, y, z] = sprite.position;
        let [vx, vy, vz] = sprite.velocity;
        let span = if span.is_finite() { span.max(0.0) } else { 0.0 };
        let [px, py, pz] = [x - vx * span, y - vy * span, z - vz * span];
        let [r, g, b] = sprite.colour;
        Self {
            position_size: [x, y, z, sprite.size],
            previous_angle: [px, py, pz, sprite.angle],
            velocity_stretch: [vx, vy, vz, stretch.max(0.0)],
            colour_alpha: [r, g, b, sprite.alpha],
            flags: [
                f32::from(u8::from(lit)),
                f32::from(u8::from(blend == Blend::Add)),
                span,
                0.0,
            ],
        }
    }

    pub fn additive(&self) -> bool {
        self.flags[1] > 0.5
    }
}

#[derive(Clone, Debug)]
struct Link {
    object: Option<usize>,
    model: Matrix,
    attach: Option<Attach>,
}

impl Link {
    fn world(&self, objects: Option<&[Matrix]>, palettes: &Palettes) -> Matrix {
        let Some(attach) = &self.attach else {
            return IDENTITY;
        };
        let object = self
            .object
            .and_then(|index| objects.and_then(|objects| objects.get(index)))
            .copied()
            .unwrap_or(self.model);
        let palette = self
            .object
            .and_then(|index| palettes.get(&index))
            .map(Vec::as_slice);
        attach.world(object, palette)
    }
}

pub type Palettes = BTreeMap<usize, Vec<Matrix>>;

#[derive(Clone, Debug, Default)]
pub struct Sprays {
    particles: Vec<Particles>,
    links: Vec<Link>,
    emitters: Vec<Emitter>,
    steps: u64,
    sprites: Vec<Sprite>,
    last: Option<f32>,
    span: f32,
}

fn target(time: f32) -> u64 {
    (time.max(0.0) / STEP + 1e-3).floor() as u64
}

fn lerp(a: &Matrix, b: &Matrix, k: f32) -> Matrix {
    std::array::from_fn(|c| std::array::from_fn(|r| a[c][r] + (b[c][r] - a[c][r]) * k))
}

impl Sprays {
    pub fn of(scene: &Scene) -> Self {
        let particles: Vec<Particles> = scene.particles.values().cloned().collect();
        let links = particles
            .iter()
            .map(|particles| {
                let object = particles
                    .attach
                    .as_ref()
                    .and_then(|attach| attach.index(scene));
                Link {
                    object,
                    model: object.map_or(IDENTITY, |index| scene.objects[index].model),
                    attach: particles.attach.clone(),
                }
            })
            .collect();
        let emitters = particles
            .iter()
            .map(|particles| Emitter::new(particles.recipe.clone(), particles.at))
            .collect();
        Self {
            particles,
            links,
            emitters,
            steps: 0,
            sprites: Vec::new(),
            last: None,
            span: 0.0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.emitters.is_empty()
    }

    pub fn same(&self, scene: &Scene) -> bool {
        self.particles.len() == scene.particles.len()
            && self.particles.iter().eq(scene.particles.values())
            && self.links.iter().all(|link| {
                link.object.is_none_or(|index| {
                    scene
                        .objects
                        .get(index)
                        .is_some_and(|object| object.model == link.model)
                })
            })
    }

    pub fn live(&self) -> usize {
        self.emitters.iter().map(Emitter::live).sum()
    }

    pub fn dropped(&self) -> u64 {
        self.emitters.iter().map(Emitter::dropped).sum()
    }

    pub fn reset(&mut self) {
        for emitter in &mut self.emitters {
            emitter.reset();
        }
        self.steps = 0;
        self.last = None;
        self.span = 0.0;
    }

    pub fn parents(&self, objects: Option<&[Matrix]>, palettes: &Palettes, out: &mut Vec<Matrix>) {
        out.clear();
        out.extend(self.links.iter().map(|link| link.world(objects, palettes)));
    }

    pub fn seek(&mut self, time: f32, mut pose: impl FnMut(f32, &mut Vec<Matrix>)) {
        if self.emitters.is_empty() || !time.is_finite() {
            return;
        }
        let span = self
            .last
            .map(|last| time - last)
            .filter(|span| (0.0..=MAX_SPAN).contains(span))
            .unwrap_or(0.0);
        let goal = target(time);
        let window = (CATCH_UP / STEP) as u64;
        if goal < self.steps || goal - self.steps > window {
            self.reset();
            self.steps = goal.saturating_sub(window);
        }
        self.last = Some(time);
        self.span = span;
        if self.steps == goal {
            return;
        }
        let count = self.emitters.len();
        let mut from = Vec::with_capacity(count);
        let mut to = Vec::with_capacity(count);
        pose(self.steps as f32 * STEP, &mut from);
        from.resize(count, IDENTITY);
        while self.steps < goal {
            to.clear();
            pose((self.steps + 1) as f32 * STEP, &mut to);
            to.resize(count, IDENTITY);
            for (emitter, (from, to)) in self.emitters.iter_mut().zip(from.iter().zip(&to)) {
                emitter.step(STEP, *from, *to);
            }
            std::mem::swap(&mut from, &mut to);
            self.steps += 1;
        }
    }

    pub fn seek_posed(&mut self, time: f32, mut objects: impl FnMut(f32) -> Option<Vec<Matrix>>) {
        self.seek_rigged(time, |t| objects(t).map(|posed| (posed, Palettes::new())));
    }

    pub fn seek_rigged(
        &mut self,
        time: f32,
        mut posing: impl FnMut(f32) -> Option<(Vec<Matrix>, Palettes)>,
    ) {
        let links = std::mem::take(&mut self.links);
        let empty = Palettes::new();
        self.seek(time, |t, out| {
            let posed = posing(t);
            let (objects, palettes) = match &posed {
                Some((objects, palettes)) => (Some(objects.as_slice()), palettes),
                None => (None, &empty),
            };
            out.extend(links.iter().map(|link| link.world(objects, palettes)));
        });
        self.links = links;
    }

    pub fn seek_between(&mut self, span: [f32; 2], before: &[Matrix], after: &[Matrix]) {
        let [t0, t1] = span;
        let width = t1 - t0;
        self.seek(t1, |t, out| {
            let k = if width > 0.0 {
                ((t - t0) / width).clamp(0.0, 1.0)
            } else {
                1.0
            };
            out.extend(before.iter().zip(after).map(|(a, b)| lerp(a, b, k)));
        });
    }

    pub fn pack(&mut self, out: &mut Vec<SprayInstance>) {
        out.clear();
        for emitter in &self.emitters {
            self.sprites.clear();
            emitter.sprites(&mut self.sprites);
            let recipe = emitter.recipe();
            out.extend(self.sprites.iter().map(|sprite| {
                SprayInstance::new(sprite, recipe.stretch, recipe.blend, recipe.lit, self.span)
            }));
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLight {
    position_radius: [f32; 4],
    colour_range: [f32; 4],
    spot: [f32; 4],
    cone: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SprayFrame {
    view_proj: [[f32; 4]; 4],
    motion_view_proj: [[f32; 4]; 4],
    previous_view_proj: [[f32; 4]; 4],
    inverse_motion: [[f32; 4]; 4],
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    sun: [f32; 4],
    sun_color: [f32; 4],
    info: [f32; 4],
    scatter: [f32; 4],
    sky: [[f32; 4]; 9],
    lights: [GpuLight; MAX_LIGHTS],
}

pub struct SprayView {
    pub view_proj: Matrix,
    pub motion_view_proj: Matrix,
    pub previous_view_proj: Matrix,
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub sun: [f32; 3],
    pub sun_color: [f32; 3],
    pub sun_intensity: f32,
    pub sky_sh: [[f32; 3]; 9],
    pub size: [u32; 2],
    pub phase: f32,
    pub shafts: f32,
}

pub struct SprayTargets<'a> {
    pub hdr: &'a wgpu::TextureView,
    pub velocity: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub shadow: (&'a wgpu::TextureView, &'a wgpu::Buffer),
    pub lights: &'a [LocalLight],
    pub probes: &'a ProbeLighting,
}

pub struct SprayPass {
    layout: wgpu::BindGroupLayout,
    empty: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    ordered: Vec<SprayInstance>,
}

pub fn spray_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let buffer = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    vec![
        buffer(
            0,
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BufferBindingType::Uniform,
        ),
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        buffer(
            2,
            wgpu::ShaderStages::VERTEX,
            wgpu::BufferBindingType::Storage { read_only: true },
        ),
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        buffer(
            4,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::BufferBindingType::Uniform,
        ),
    ]
}

impl SprayPass {
    pub fn new(device: &wgpu::Device, probes: &ProbeLighting) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live spray bindings"),
            entries: &spray_entries(),
        });
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("live spray empty"),
            entries: &[],
        });
        let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live spray empty"),
            layout: &empty_layout,
            entries: &[],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("live spray layout"),
            bind_group_layouts: &[
                Some(&layout),
                Some(&empty_layout),
                Some(&empty_layout),
                Some(probes.layout()),
            ],
            immediate_size: 0,
        });
        let source = format!("{PROBE_WGSL}\n{SPRAY_WGSL}");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("live sprays"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let blend = |src_factor, dst_factor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        let premultiplied = blend(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha);
        let covered = blend(
            wgpu::BlendFactor::SrcAlpha,
            wgpu::BlendFactor::OneMinusSrcAlpha,
        );
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("live sprays"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(wgpu::BlendState {
                            color: premultiplied,
                            alpha: premultiplied,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: VELOCITY_FORMAT,
                        blend: Some(wgpu::BlendState {
                            color: covered,
                            alpha: covered,
                        }),
                        write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,
                    }),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            layout,
            empty,
            pipeline,
            ordered: Vec::new(),
        }
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        view: &SprayView,
        targets: SprayTargets<'_>,
        sprays: &[SprayInstance],
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        if sprays.is_empty() {
            return;
        }
        let forward = view.forward;
        let depth = |instance: &SprayInstance| {
            (0..3)
                .map(|k| (instance.position_size[k] - view.eye[k]) * forward[k])
                .sum::<f32>()
        };
        self.ordered.clear();
        self.ordered.extend_from_slice(sprays);
        self.ordered.sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        let mut packed = [GpuLight::zeroed(); MAX_LIGHTS];
        let mut count = 0;
        for light in targets
            .lights
            .iter()
            .filter(|light| light.lit())
            .take(MAX_LIGHTS)
        {
            let (spot, cone) = match light.spot {
                Some(spot) => {
                    let (scale, offset) = spot.scale_offset();
                    let d = spot.direction;
                    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
                    (
                        [d[0] / length, d[1] / length, d[2] / length, scale],
                        [offset, 1.0, 0.0, 0.0],
                    )
                }
                None => ([0.0; 4], [0.0; 4]),
            };
            packed[count] = GpuLight {
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
                spot,
                cone,
            };
            count += 1;
        }
        let four = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let inverse = crate::renderer::invert(view.motion_view_proj);
        let frame = SprayFrame {
            view_proj: view.view_proj,
            motion_view_proj: view.motion_view_proj,
            previous_view_proj: view.previous_view_proj,
            inverse_motion: inverse.unwrap_or(IDENTITY),
            eye: four(view.eye),
            right: four(view.right),
            up: four(view.up),
            forward: four(view.forward),
            sun: four(view.sun),
            sun_color: [
                view.sun_color[0],
                view.sun_color[1],
                view.sun_color[2],
                view.sun_intensity,
            ],
            info: [
                count as f32,
                view.size[0] as f32,
                view.size[1] as f32,
                f32::from(u8::from(inverse.is_some())),
            ],
            scatter: [
                view.phase.clamp(-0.9, 0.9),
                view.shafts.clamp(0.0, 1.0),
                0.0,
                0.0,
            ],
            sky: view.sky_sh.map(four),
            lights: packed,
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live spray frame"),
            contents: bytemuck::bytes_of(&frame),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("live spray instances"),
            contents: bytemuck::cast_slice(&self.ordered),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live spray bindings"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(targets.depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(targets.shadow.0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: targets.shadow.1.as_entire_binding(),
                },
            ],
        });
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("sprays"),
            color_attachments: &[attachment(targets.hdr), attachment(targets.velocity)],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
        });
        crate::viewport::apply(&mut pass, view.size);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_bind_group(1, &self.empty, &[]);
        pass.set_bind_group(2, &self.empty, &[]);
        pass.set_bind_group(3, targets.probes.group(), &[]);
        pass.draw(0..6, 0..self.ordered.len() as u32);
    }
}

pub const SPRAY_WGSL: &str = r#"
const SPRAY_PI: f32 = 3.14159265;
const SPRAY_FRAC_4_PI: f32 = 0.07957747154594767;
struct SprayLight { position_radius: vec4f, colour_range: vec4f, spot: vec4f, cone: vec4f, }
struct SprayFrame {
    view_proj: mat4x4f, motion_view_proj: mat4x4f, previous_view_proj: mat4x4f, inverse_motion: mat4x4f,
    eye: vec4f, right: vec4f, up: vec4f, forward: vec4f, sun: vec4f, sun_color: vec4f, info: vec4f, scatter: vec4f,
    sky: array<vec4f, 9>,
    lights: array<SprayLight, 16>,
}
struct SprayShadow {
    splits: vec4f, filter_params: vec4f, sun: vec4f, texels: vec4f,
    view_proj_0: mat4x4f, view_proj_1: mat4x4f, view_proj_2: mat4x4f,
}
struct Spray {
    position_size: vec4f, previous_angle: vec4f, velocity_stretch: vec4f, colour_alpha: vec4f, flags: vec4f,
}
@group(0) @binding(0) var<uniform> spray: SprayFrame;
@group(0) @binding(1) var spray_depth: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> sprays: array<Spray>;
@group(0) @binding(3) var spray_shadow_atlas: texture_depth_2d_array;
@group(0) @binding(4) var<uniform> spray_shadow: SprayShadow;
struct SprayOut {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) colour: vec4f,
    @location(2) world: vec3f,
    @location(3) motion: vec2f,
    @location(4) axis_x: vec3f,
    @location(5) axis_y: vec3f,
    @location(6) @interpolate(flat) flags: vec4f,
}
fn spray_relative(p: Spray) -> vec3f {
    let span = p.flags.z;
    if (span <= 0.0 || spray.info.w < 0.5) { return p.velocity_stretch.xyz; }
    let seen = spray.inverse_motion * (spray.previous_view_proj * vec4f(p.previous_angle.xyz, 1.0));
    if (abs(seen.w) <= 1e-6) { return p.velocity_stretch.xyz; }
    return (p.position_size.xyz - seen.xyz / seen.w) / span;
}
@vertex fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> SprayOut {
    let p = sprays[index];
    let corners = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0), vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0));
    let corner = corners[vertex];
    let half = 0.5 * p.position_size.w;
    let spin = p.previous_angle.w;
    var axis_x = spray.right.xyz * cos(spin) + spray.up.xyz * sin(spin);
    var axis_y = spray.up.xyz * cos(spin) - spray.right.xyz * sin(spin);
    var along = half;
    var stretched = 0.0;
    if (p.velocity_stretch.w > 0.0) {
        let to_eye = p.position_size.xyz - spray.eye.xyz;
        let ray = select(spray.forward.xyz, normalize(to_eye), dot(to_eye, to_eye) > 1e-10);
        let v = spray_relative(p);
        let across = v - ray * dot(v, ray);
        let speed = length(across);
        if (speed > 1e-4) {
            axis_y = across / speed;
            axis_x = normalize(cross(axis_y, ray));
            along = half + 0.5 * p.velocity_stretch.w * speed;
            stretched = 1.0;
        }
    }
    let offset = axis_x * corner.x * half + axis_y * corner.y * along;
    let world = p.position_size.xyz + offset;
    let before = p.previous_angle.xyz + offset;
    var out: SprayOut;
    out.clip = spray.view_proj * vec4f(world, 1.0);
    let current = spray.motion_view_proj * vec4f(world, 1.0);
    let previous = spray.previous_view_proj * vec4f(before, 1.0);
    out.motion = (previous.xy / previous.w - current.xy / current.w) * vec2f(0.5, -0.5);
    out.uv = corner;
    out.colour = p.colour_alpha;
    out.world = world;
    out.axis_x = axis_x;
    out.axis_y = axis_y;
    out.flags = vec4f(p.flags.x, p.flags.y, half, stretched);
    return out;
}
fn spray_sun(world: vec3f, depth: f32) -> f32 {
    if (spray_shadow.splits.w <= 0.0) { return 1.0; }
    var layer = 0;
    var matrix = spray_shadow.view_proj_0;
    if (depth > spray_shadow.splits.y) { layer = 1; matrix = spray_shadow.view_proj_1; }
    if (depth > spray_shadow.splits.z) { layer = 2; matrix = spray_shadow.view_proj_2; }
    let clip = matrix * vec4f(world, 1.0);
    let point = clip.xyz / clip.w;
    let uv = point.xy * vec2f(0.5, -0.5) + vec2f(0.5);
    if (any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) || point.z < 0.0 || point.z > 1.0) { return 1.0; }
    let size = vec2i(textureDimensions(spray_shadow_atlas));
    let base = clamp(vec2i(uv * vec2f(size) - vec2f(0.5)), vec2i(0), size - vec2i(2));
    var lit = 0.0;
    for (var k = 0; k < 4; k++) {
        let depth = textureLoad(spray_shadow_atlas, base + vec2i(k & 1, k >> 1), layer, 0);
        lit += select(0.0, 0.25, point.z - spray_shadow.sun.w <= depth);
    }
    return lit;
}
fn spray_sky(n: vec3f) -> vec3f {
    let d = normalize(n);
    let s = spray.sky;
    let value = s[0].xyz * 0.282095 + s[1].xyz * (0.488603 * d.y) + s[2].xyz * (0.488603 * d.z)
        + s[3].xyz * (0.488603 * d.x) + s[4].xyz * (1.092548 * d.x * d.y) + s[5].xyz * (1.092548 * d.y * d.z)
        + s[6].xyz * (0.315392 * (3.0 * d.z * d.z - 1.0)) + s[7].xyz * (1.092548 * d.x * d.z)
        + s[8].xyz * (0.546274 * (d.x * d.x - d.y * d.y));
    return max(value, vec3f(0.0));
}
fn spray_wrap(n: vec3f, l: vec3f) -> f32 {
    return max(dot(n, l) + 0.5, 0.0) / 1.5;
}
fn spray_phase(cosine: f32) -> f32 {
    let g = spray.scatter.x;
    let denom = 1.0 + g * g - 2.0 * g * cosine;
    return SPRAY_FRAC_4_PI * (1.0 - g * g) / (denom * sqrt(denom));
}
fn spray_scatter(ray: vec3f, n: vec3f, l: vec3f, thick: f32) -> f32 {
    return mix(spray_phase(dot(ray, l)), spray_wrap(n, l) / SPRAY_PI, thick);
}
fn spray_ambient(world: vec3f, n: vec3f) -> vec3f {
    let grid = sample_live_irradiance(world, n);
    return select(spray_sky(n), grid, grid.x >= 0.0);
}
struct SprayTargets { @location(0) colour: vec4f, @location(1) velocity: vec4f, }
@fragment fn fs_main(in: SprayOut) -> SprayTargets {
    let r2 = dot(in.uv, in.uv);
    if (r2 >= 1.0) { discard; }
    let shape = (1.0 - r2) * (1.0 - r2);
    let scene = textureLoad(spray_depth, vec2i(in.clip.xy), 0).x;
    let depth = dot(in.world - spray.eye.xyz, spray.forward.xyz);
    let fade = clamp((scene - depth) / max(in.flags.z, 0.02), 0.0, 1.0);
    let alpha = clamp(in.colour.a * shape * fade, 0.0, 1.0);
    if (alpha <= 0.0) { discard; }
    var radiance = in.colour.rgb;
    if (in.flags.x > 0.5) {
        let ray = normalize(in.world - spray.eye.xyz);
        let n = normalize(in.axis_x * in.uv.x + in.axis_y * in.uv.y - ray * sqrt(1.0 - r2));
        let thick = alpha;
        let sun = normalize(spray.sun.xyz);
        var light = spray.sun_color.rgb * spray.sun_color.w * mix(1.0, spray_sun(in.world, depth), spray.scatter.y) * spray_scatter(ray, n, sun, thick);
        let facing = spray_ambient(in.world, n);
        light += mix(0.5 * (facing + spray_ambient(in.world, -n)), facing, thick) / SPRAY_PI;
        let count = u32(spray.info.x);
        for (var k = 0u; k < count; k++) {
            let lamp = spray.lights[k];
            let to_light = lamp.position_radius.xyz - in.world;
            let distance = length(to_light);
            let range = lamp.colour_range.w;
            if (distance >= range || distance <= 1e-6) { continue; }
            let l = to_light / distance;
            var cone = 1.0;
            if (lamp.cone.y > 0.5) {
                let along = clamp(dot(lamp.spot.xyz, -l) * lamp.spot.w + lamp.cone.x, 0.0, 1.0);
                cone = along * along;
                if (cone <= 0.0) { continue; }
            }
            let ratio = distance / range;
            let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
            let reach = max(distance, lamp.position_radius.w);
            light += lamp.colour_range.rgb * window * window * cone / (reach * reach) * spray_scatter(ray, n, l, thick);
        }
        radiance = in.colour.rgb * light;
    }
    var out: SprayTargets;
    out.colour = vec4f(radiance * alpha, select(alpha, 0.0, in.flags.y > 0.5));
    out.velocity = vec4f(in.motion, 0.0, select(alpha, 0.0, in.flags.w > 0.5));
    return out;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_validates_and_the_instance_is_80_bytes() {
        let source = format!("{PROBE_WGSL}\n{SPRAY_WGSL}");
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(std::mem::size_of::<SprayInstance>(), 80);
        assert_eq!(std::mem::size_of::<SprayFrame>() % 16, 0);
    }
}
