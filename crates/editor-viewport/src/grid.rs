use egui::Color32;
use glam::{Mat4, Vec3, Vec4};
use pfx_gpu::{Gpu, wgpu};

pub type Matrix = [[f32; 4]; 4];

pub const STEPS: f32 = 10.0;
pub const LINES: f32 = 100.0;
pub const LEVEL_BIAS: f32 = -0.2;
pub const LEVEL_MIN: f32 = -2.0;
pub const LEVEL_MAX: f32 = 3.0;
pub const XRAY: f32 = 0.35;
const UNIFORM_BYTES: u64 = 224;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lines {
    pub minor: [f32; 4],
    pub major: [f32; 4],
    pub x: [f32; 4],
    pub z: [f32; 4],
    pub through: f32,
}

impl Lines {
    pub fn new(minor: Color32, major: Color32, x: Color32, z: Color32) -> Lines {
        Lines {
            minor: linear(minor),
            major: linear(major),
            x: linear(x),
            z: linear(z),
            through: 0.0,
        }
    }

    pub fn through(self, through: f32) -> Lines {
        Lines {
            through: through.clamp(0.0, 1.0),
            ..self
        }
    }
}

pub fn linear(colour: Color32) -> [f32; 4] {
    let [r, g, b, a] = colour.to_srgba_unmultiplied();
    let channel = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [channel(r), channel(g), channel(b), f32::from(a) / 255.0]
}

pub struct Target<'a> {
    pub output: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub depth: &'a wgpu::TextureView,
    pub size: [u32; 2],
    pub depth_size: [u32; 2],
    pub view: Matrix,
    pub projection: Matrix,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub small: f32,
    pub large: f32,
    pub decimals: f32,
    pub fade: f32,
    pub centre: [f32; 2],
}

fn orthographic(projection: &Matrix) -> bool {
    projection[2][3] == 0.0
}

pub fn levels(view: &Matrix, projection: &Matrix) -> Option<Levels> {
    let world = Mat4::from_cols_array_2d(view).inverse();
    if !world.is_finite() {
        return None;
    }
    let camera = world.w_axis.truncate();
    let forward = -world.z_axis.truncate().normalize_or_zero();
    let (distance, centre) = if orthographic(projection) {
        let height = 2.0 / projection[1][1].abs().max(1e-12);
        let centre = if forward.y.abs() > 1e-6 {
            let hit = camera + forward * (-camera.y / forward.y);
            [hit.x, hit.z]
        } else {
            [camera.x, camera.z]
        };
        (height / 2.0, centre)
    } else {
        (camera.y.abs(), [camera.x, camera.z])
    };
    let level = distance.max(1e-6).ln() / STEPS.ln() + LEVEL_BIAS;
    let clamped = level.clamp(LEVEL_MIN, LEVEL_MAX);
    let floored = clamped.floor();
    let small = STEPS.powf(floored);
    let fade = STEPS
        .powf(level - 1.0)
        .clamp(STEPS.powf(LEVEL_MIN), STEPS.powf(LEVEL_MAX));
    Some(Levels {
        small,
        large: small * STEPS,
        decimals: clamped - floored,
        fade: (LINES - STEPS) * fade,
        centre,
    })
}

fn uniform(target: &Target<'_>, lines: &Lines, levels: &Levels) -> Vec<u8> {
    let view = Mat4::from_cols_array_2d(&target.view);
    let projection = Mat4::from_cols_array_2d(&target.projection);
    let world = view.inverse();
    let inverse = (projection * view).inverse();
    let camera = world.w_axis.truncate();
    let forward = -world.z_axis.truncate().normalize_or_zero();
    let ortho = if orthographic(&target.projection) {
        1.0
    } else {
        0.0
    };
    let [w, h] = target.size.map(|side| side.max(1) as f32);
    let [dw, dh] = target.depth_size.map(|side| side.max(1) as f32);
    let vectors = [
        Vec4::new(target.projection[2][2], target.projection[3][2], 0.0, 0.0),
        camera.extend(ortho),
        forward.extend(lines.through),
        Vec4::new(
            levels.centre[0],
            levels.centre[1],
            levels.small,
            levels.decimals,
        ),
        Vec4::new(levels.fade, levels.large, w, h),
        Vec4::new(dw, dh, 0.0, 0.0),
        Vec4::from_array(lines.minor),
        Vec4::from_array(lines.major),
        Vec4::from_array(lines.x),
        Vec4::from_array(lines.z),
    ];
    let mut bytes = Vec::with_capacity(UNIFORM_BYTES as usize);
    for value in inverse
        .to_cols_array()
        .into_iter()
        .chain(vectors.iter().flat_map(|v| v.to_array()))
    {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.resize(UNIFORM_BYTES as usize, 0);
    bytes
}

struct Pipe {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
}

#[derive(Default)]
pub struct Grid {
    pipe: Option<Pipe>,
}

pub struct Drawn {
    group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl Drawn {
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl Grid {
    pub fn prepare(&mut self, gpu: &Gpu, target: &Target<'_>, lines: &Lines) -> Option<Drawn> {
        let levels = levels(&target.view, &target.projection)?;
        let device = &gpu.device;
        if self
            .pipe
            .as_ref()
            .is_none_or(|pipe| pipe.format != target.format)
        {
            self.pipe = Some(pipe(device, target.format));
        }
        let pipe = self.pipe.as_ref()?;
        gpu.queue
            .write_buffer(&pipe.uniform, 0, &uniform(target, lines, &levels));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pfx editor grid"),
            layout: &pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pipe.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(target.depth),
                },
            ],
        });
        Some(Drawn {
            group,
            pipeline: pipe.pipeline.clone(),
        })
    }

    pub fn encode(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target<'_>,
        lines: &Lines,
    ) {
        let Some(drawn) = self.prepare(gpu, target, lines) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pfx editor grid"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        drawn.record(&mut pass);
    }

    pub fn draw(&mut self, gpu: &Gpu, target: &Target<'_>, lines: &Lines) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pfx editor grid"),
            });
        self.encode(gpu, &mut encoder, target, lines);
        gpu.queue.submit([encoder.finish()]);
    }
}

fn pipe(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipe {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pfx editor grid"),
        source: wgpu::ShaderSource::Wgsl(shader().into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("pfx editor grid"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pfx editor grid"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pfx editor grid"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("cover"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("ink"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    Pipe {
        format,
        layout,
        pipeline,
        uniform: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pfx editor grid"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    }
}

pub fn plane_point(view: &Matrix, projection: &Matrix, ndc: [f32; 2]) -> Option<Vec3> {
    let inverse = (Mat4::from_cols_array_2d(projection) * Mat4::from_cols_array_2d(view)).inverse();
    let near = inverse.project_point3(Vec3::new(ndc[0], ndc[1], 0.0));
    let far = inverse.project_point3(Vec3::new(ndc[0], ndc[1], 0.5));
    let ray = far - near;
    let along = -near.y / ray.y;
    (ray.y.abs() > 1e-12 && along > 0.0 && along.is_finite()).then(|| near + ray * along)
}

fn shader() -> String {
    [pfx_live::depth::DEPTH_WGSL, SHADER].concat()
}

const SHADER: &str = r#"
struct Grid {
    inverse: mat4x4f,
    depth: vec4f,
    camera: vec4f,
    forward: vec4f,
    plane: vec4f,
    fade: vec4f,
    depth_size: vec4f,
    minor: vec4f,
    major: vec4f,
    x: vec4f,
    z: vec4f,
}

@group(0) @binding(0) var<uniform> grid: Grid;
@group(0) @binding(1) var depth: texture_depth_2d;

@vertex
fn cover(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let x = f32(index & 1u) * 4.0 - 1.0;
    let y = f32(index >> 1u) * 4.0 - 1.0;
    return vec4f(x, y, 0.0, 1.0);
}

fn unproject(ndc: vec2f, z: f32) -> vec3f {
    let p = grid.inverse * vec4f(ndc, z, 1.0);
    return p.xyz / p.w;
}

fn ruled(at: vec2f, step: f32) -> f32 {
    let u = at / step;
    let width = max(fwidth(u), vec2f(1e-6));
    let away = abs(fract(u - 0.5) - 0.5) / width;
    let crowd = clamp(2.0 - 4.0 * width, vec2f(0.0), vec2f(1.0));
    let line = clamp(1.0 - away, vec2f(0.0), vec2f(1.0)) * crowd;
    return max(line.x, line.y);
}

fn axis(at: f32) -> f32 {
    let width = max(fwidth(at), 1e-9);
    return clamp(1.25 - abs(at) / width, 0.0, 1.0);
}

fn over(under: vec4f, colour: vec3f, alpha: f32) -> vec4f {
    let a = alpha + under.a * (1.0 - alpha);
    let rgb = (colour * alpha + under.rgb * under.a * (1.0 - alpha)) / max(a, 1e-6);
    return vec4f(rgb, a);
}

fn distance_at(raw: f32) -> f32 {
    if pfx_is_sky(raw) {
        return 1e30;
    }
    return pfx_projection_distance(pfx_standard_depth(raw), grid.depth.x, grid.depth.y, grid.camera.w > 0.5);
}

@fragment
fn ink(@builtin(position) at: vec4f) -> @location(0) vec4f {
    let size = grid.fade.zw;
    let ndc = vec2f(at.x / size.x * 2.0 - 1.0, 1.0 - at.y / size.y * 2.0);
    let near = unproject(ndc, 0.0);
    let ray = unproject(ndc, 0.5) - near;
    let along = -near.y / ray.y;
    let ahead = abs(ray.y) > 1e-12 && along > 0.0;
    let hit = near + ray * select(0.0, along, ahead);
    let flat = hit.xz;
    let small = grid.plane.z;
    let decimals = grid.plane.w;
    let minor = ruled(flat, small);
    let major = ruled(flat, grid.fade.y);
    let along_x = axis(hit.z);
    let along_z = axis(hit.x);

    let ortho = grid.camera.w > 0.5;
    let toward = select(normalize(grid.camera.xyz - hit), -grid.forward.xyz, ortho);
    let angle_fade = smoothstep(0.05, 0.2, abs(toward.y));
    let centre = vec3f(grid.plane.x, 0.0, grid.plane.y);
    let dist_fade = smoothstep(0.02, 0.3, 1.0 - distance(hit, centre) / grid.fade.x);

    let major_colour = mix(grid.major, grid.minor, decimals);
    var ink = vec4f(0.0);
    ink = over(ink, grid.minor.rgb, minor * grid.minor.a * (1.0 - decimals));
    ink = over(ink, major_colour.rgb, major * major_colour.a);
    ink = over(ink, grid.x.rgb, along_x * grid.x.a);
    ink = over(ink, grid.z.rgb, along_z * grid.z.a);

    let limit = vec2i(grid.depth_size.xy) - vec2i(1, 1);
    let pixel = vec2i(at.xy * grid.depth_size.xy / size);
    let raw = textureLoad(depth, clamp(pixel, vec2i(0, 0), limit), 0);
    let scene = distance_at(raw);
    let mine = dot(hit - grid.camera.xyz, grid.forward.xyz);
    let hidden = mine > scene * 1.002 + 1e-3;
    let shown = select(1.0, grid.forward.w, hidden);

    let alpha = ink.a * dist_fade * angle_fade * shown * select(0.0, 1.0, ahead);
    if alpha <= 0.0 {
        discard;
    }
    return vec4f(ink.rgb, alpha);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn look(eye: Vec3, target: Vec3) -> (Matrix, Matrix) {
        let camera = pfx_load::scene::Camera {
            projection: pfx_load::scene::Projection::Perspective { fov: 40.0 },
            at: eye.to_array(),
            look_at: target.to_array(),
            near: 0.05,
            far: 50.0,
            ..pfx_load::scene::Camera::default()
        };
        let camera = pfx_live::scene::camera(&camera, 16.0 / 9.0);
        (camera.view, camera.projection)
    }

    #[test]
    fn the_steps_follow_the_height_above_the_ground_and_blend_between_levels() {
        let (view, projection) = look(Vec3::new(0.0, 1.5, 4.0), Vec3::ZERO);
        let low = levels(&view, &projection).unwrap();
        assert!((low.small - 0.1).abs() < 1e-6);
        assert!((low.large - 1.0).abs() < 1e-5);
        assert!(low.decimals > 0.0 && low.decimals < 1.0);
        assert_eq!(low.centre, [0.0, 4.0]);
        let (view, projection) = look(Vec3::new(0.0, 40.0, 4.0), Vec3::ZERO);
        let high = levels(&view, &projection).unwrap();
        assert!((high.small - 10.0).abs() < 1e-4);
        assert!(high.fade > low.fade * 10.0);
        let (view, projection) = look(Vec3::new(0.0, -1.5, 4.0), Vec3::ZERO);
        assert!((levels(&view, &projection).unwrap().small - 0.1).abs() < 1e-6);
    }

    #[test]
    fn the_grid_lies_on_the_ground_plane_under_the_camera() {
        let (view, projection) = look(Vec3::new(0.0, 1.5, 4.0), Vec3::ZERO);
        let centre = plane_point(&view, &projection, [0.0, 0.0]).unwrap();
        assert!(centre.y.abs() < 1e-4);
        assert!(centre.length() < 1e-3, "{centre}");
        let (view, projection) = look(Vec3::new(0.0, 1.5, 4.0), Vec3::new(0.0, 1.5, 0.0));
        assert!(plane_point(&view, &projection, [0.0, 0.5]).is_none());
        let below = plane_point(&view, &projection, [0.3, -0.5]).unwrap();
        assert!(below.y.abs() < 1e-4 && below.z < 4.0, "{below}");
    }
}
