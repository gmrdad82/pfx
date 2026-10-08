use bytemuck::{Pod, Zeroable};
use pfx_geom::mesh::Mesh;
use pfx_gpu::GpuProfiler;
use pfx_materials::Material;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wgpu::util::DeviceExt;

use crate::frame::{CountedBuffer, MESH_GENERATION_LAST, MESH_SLOT_BITS, MESH_SLOT_MASK};

pub use crate::maps::CausticProjection;

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const REACTIVE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

pub type Matrix = [[f32; 4]; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

impl MeshHandle {
    pub fn slot(self) -> usize {
        (self.0 & MESH_SLOT_MASK) as usize
    }

    pub fn generation(self) -> u32 {
        self.0 >> MESH_SLOT_BITS
    }

    fn pack(slot: usize, generation: u32) -> Self {
        Self(slot as u32 | generation << MESH_SLOT_BITS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlassStats {
    pub live: usize,
    pub slots: usize,
    pub buffers: usize,
}

#[derive(Clone, Copy)]
pub struct Surface {
    pub mesh: MeshHandle,
    pub model: Matrix,
    pub material: Material,
    pub liquid: bool,
    pub fluid_height: f32,
    pub ripple_height: f32,
    pub caustic_strength: f32,
    pub tinted: Option<Material>,
    pub id: u32,
    pub casts_shadow: bool,
    pub shadow_only: bool,
    pub clip: [[f32; 4]; 2],
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub view: Matrix,
    pub projection: Matrix,
}

pub struct Inputs<'a> {
    pub opaque_hdr_mips: &'a wgpu::TextureView,
    pub opaque_depth: &'a wgpu::TextureView,
    pub reflection_cube: &'a wgpu::TextureView,
    pub caustic: &'a wgpu::TextureView,
    pub caustic_projection: Option<CausticProjection>,
    pub fluid_height: &'a wgpu::TextureView,
    pub ripple_height: &'a wgpu::TextureView,
    pub mip_count: u32,
    pub sky_sh: [[f32; 3]; 9],
}

pub struct Render<'a> {
    pub camera: Camera,
    pub surfaces: &'a [Surface],
    pub inputs: Inputs<'a>,
    pub output: &'a wgpu::TextureView,
    pub profiler: Option<&'a mut GpuProfiler>,
}

#[derive(Clone, Copy)]
pub struct Refraction {
    pub uv: [f32; 2],
    pub front_depth: f32,
    pub back_depth: f32,
    pub front_normal: [f32; 3],
    pub back_normal: [f32; 3],
    pub projection_scale: [f32; 2],
    pub ior: f32,
    pub receiver_depth: f32,
}

struct MeshGpu {
    vertices: CountedBuffer,
    indices: CountedBuffer,
    vertex_capacity: usize,
    index_capacity: usize,
    count: u32,
    bounds: [[f32; 3]; 2],
}

struct MeshSlot {
    generation: u32,
    mesh: Option<MeshGpu>,
}

#[derive(Default)]
pub struct GlassMeshes {
    slots: Vec<MeshSlot>,
    free: Vec<usize>,
    buffers: Arc<AtomicUsize>,
}

fn check_glass(mesh: &Mesh) -> Result<(), String> {
    if mesh.positions.len() != mesh.normals.len()
        || mesh.positions.len() != mesh.uvs.len()
        || mesh.indices.is_empty()
    {
        return Err("glass mesh needs positions, normals, UVs, and indices".into());
    }
    Ok(())
}

fn glass_vertices(mesh: &Mesh) -> Vec<Vertex> {
    mesh.positions
        .iter()
        .zip(&mesh.normals)
        .zip(&mesh.uvs)
        .map(|((position, normal), uv)| Vertex {
            position: [position[0], position[1], position[2], 1.0],
            normal: [normal[0], normal[1], normal[2], 0.0],
            uv: [uv[0], uv[1], 0.0, 0.0],
            look: plain_look(),
        })
        .collect()
}

fn glass_buffer(
    device: &wgpu::Device,
    live: &Arc<AtomicUsize>,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> CountedBuffer {
    CountedBuffer::new(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage: usage | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        }),
        live,
    )
}

impl GlassMeshes {
    fn get(&self, handle: MeshHandle) -> Option<&MeshGpu> {
        let slot = self.slots.get(handle.slot())?;
        if slot.generation == handle.generation() {
            slot.mesh.as_ref()
        } else {
            None
        }
    }

    pub fn stats(&self) -> GlassStats {
        GlassStats {
            live: self.slots.iter().filter(|slot| slot.mesh.is_some()).count(),
            slots: self.slots.len(),
            buffers: self.buffers.load(Ordering::Relaxed),
        }
    }

    fn upload(&mut self, device: &wgpu::Device, mesh: &Mesh) -> Result<MeshHandle, String> {
        check_glass(mesh)?;
        let vertices = glass_vertices(mesh);
        let gpu = MeshGpu {
            vertices: glass_buffer(
                device,
                &self.buffers,
                "glass vertices",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: glass_buffer(
                device,
                &self.buffers,
                "glass indices",
                bytemuck::cast_slice(&mesh.indices),
                wgpu::BufferUsages::INDEX,
            ),
            vertex_capacity: vertices.len(),
            index_capacity: mesh.indices.len(),
            count: mesh.indices.len() as u32,
            bounds: [mesh.bounds.min, mesh.bounds.max],
        };
        if let Some(slot) = self.free.pop() {
            let entry = &mut self.slots[slot];
            entry.mesh = Some(gpu);
            return Ok(MeshHandle::pack(slot, entry.generation));
        }
        let slot = self.slots.len();
        if slot > MESH_SLOT_MASK as usize {
            return Err("too many glass meshes".into());
        }
        self.slots.push(MeshSlot {
            generation: 0,
            mesh: Some(gpu),
        });
        Ok(MeshHandle::pack(slot, 0))
    }

    fn release(&mut self, handle: MeshHandle) -> Result<(), String> {
        if self.get(handle).is_none() {
            return Err("glass mesh handle is unknown or already released".into());
        }
        let slot = handle.slot();
        let entry = &mut self.slots[slot];
        entry.mesh = None;
        entry.generation += 1;
        if entry.generation < MESH_GENERATION_LAST {
            self.free.push(slot);
        }
        Ok(())
    }

    fn replace(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        handle: MeshHandle,
        mesh: &Mesh,
    ) -> Result<(), String> {
        check_glass(mesh)?;
        if self.get(handle).is_none() {
            return Err("glass mesh handle is unknown or already released".into());
        }
        let live = &self.buffers;
        let Some(gpu) = self.slots[handle.slot()].mesh.as_mut() else {
            return Err("glass mesh handle is unknown or already released".into());
        };
        let vertices = glass_vertices(mesh);
        if vertices.len() <= gpu.vertex_capacity {
            queue.write_buffer(&gpu.vertices, 0, bytemuck::cast_slice(&vertices));
        } else {
            gpu.vertices = glass_buffer(
                device,
                live,
                "glass vertices",
                bytemuck::cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            );
            gpu.vertex_capacity = vertices.len();
        }
        if mesh.indices.len() <= gpu.index_capacity {
            queue.write_buffer(&gpu.indices, 0, bytemuck::cast_slice(&mesh.indices));
        } else {
            gpu.indices = glass_buffer(
                device,
                live,
                "glass indices",
                bytemuck::cast_slice(&mesh.indices),
                wgpu::BufferUsages::INDEX,
            );
            gpu.index_capacity = mesh.indices.len();
        }
        gpu.count = mesh.indices.len() as u32;
        gpu.bounds = [mesh.bounds.min, mesh.bounds.max];
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 4],
    normal: [f32; 4],
    uv: [f32; 4],
    look: [f32; 4],
}

pub const VERTEX_FLOATS: usize = 16;
pub const LOOK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub fn plain_look() -> [f32; 4] {
    [0.0, 0.0, 1.0, 0.0]
}

pub fn subsurface_colour(tint: [f32; 3], sky_sh: &[[f32; 3]; 9], normal: [f32; 3]) -> [f32; 3] {
    let irradiance = crate::frame::sky_irradiance(sky_sh, normal);
    std::array::from_fn(|k| tint[k] * irradiance[k] / std::f32::consts::PI)
}

pub fn uniform_sky_sh(irradiance: [f32; 3]) -> [[f32; 3]; 9] {
    let mut sh = [[0.0; 3]; 9];
    sh[0] = irradiance.map(|e| e / 0.282095);
    sh
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneGpu {
    projection: Matrix,
    world_from_view: [[f32; 4]; 3],
    extent: [f32; 4],
    caustic: [[f32; 4]; 3],
    sky: [[f32; 4]; 9],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectGpu {
    model_view: Matrix,
    mvp: Matrix,
    normal: [[f32; 4]; 3],
    base_roughness: [f32; 4],
    tint_absorption: [f32; 4],
    optics: [f32; 4],
    film: [f32; 4],
    liquid: [f32; 4],
    tinted_base: [f32; 4],
    tinted_subsurface: [f32; 4],
    pick: [u32; 4],
    clip: [[f32; 4]; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CastGpu {
    model: Matrix,
    view_proj: Matrix,
    tint: [f32; 4],
    tinted: [f32; 4],
    liquid: [f32; 4],
    clip: [[f32; 4]; 2],
}

struct Casts {
    group: wgpu::BindGroup,
    draws: Vec<MeshHandle>,
}

struct Picks {
    scene: wgpu::BindGroup,
    ids: wgpu::BindGroup,
    compose: wgpu::BindGroup,
    rects: Vec<[u32; 4]>,
}

pub struct Pass {
    width: u32,
    height: u32,
    viewport: (u32, u32),
    _front: wgpu::Texture,
    front_view: wgpu::TextureView,
    _front_id: wgpu::Texture,
    front_id_view: wgpu::TextureView,
    _front_look: wgpu::Texture,
    front_look_view: wgpu::TextureView,
    _front_depth: wgpu::Texture,
    front_depth_view: wgpu::TextureView,
    _back: wgpu::Texture,
    back_view: wgpu::TextureView,
    _back_depth: wgpu::Texture,
    back_depth_view: wgpu::TextureView,
    _reactive: wgpu::Texture,
    reactive_view: wgpu::TextureView,
    scene: wgpu::Buffer,
    objects: wgpu::Buffer,
    capacity: usize,
    scene_layout: wgpu::BindGroupLayout,
    id_layout: wgpu::BindGroupLayout,
    compose_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    front_pipe: wgpu::RenderPipeline,
    back_pipe: wgpu::RenderPipeline,
    compose_pipe: wgpu::RenderPipeline,
    pick_pipe: wgpu::RenderPipeline,
    picks: Option<Picks>,
    cast_layout: wgpu::BindGroupLayout,
    cast_pipe: wgpu::RenderPipeline,
    cast_objects: wgpu::Buffer,
    cast_capacity: usize,
    casts: Option<Casts>,
    meshes: GlassMeshes,
}

pub fn cast_tint(material: &Material) -> [f32; 3] {
    let passed = if material.thickness > 0.0 && material.absorption > 0.0 {
        pfx_materials::beer(material.thickness, material.base, material.absorption)
    } else {
        material.base
    };
    passed.map(|channel| (channel * material.transmission).clamp(0.0, 1.0))
}

pub fn casting(surfaces: &[Surface]) -> bool {
    surfaces.iter().any(|surface| surface.casts_shadow)
}

pub fn drawn(surfaces: &[Surface]) -> bool {
    surfaces.iter().any(|surface| !surface.shadow_only)
}

pub fn clips(clip: &[[f32; 4]; 2], point: [f32; 3]) -> bool {
    clip.iter().any(|plane| {
        plane[..3].iter().any(|&value| value != 0.0)
            && plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] > plane[3]
    })
}

pub fn thickness(front_depth: f32, back_depth: f32) -> f32 {
    (back_depth - front_depth).max(0.0)
}

pub fn beer_lambert(distance: f32, tint: [f32; 3], absorption: f32) -> [f32; 3] {
    pfx_materials::beer(distance.max(0.0), tint, absorption.max(0.0))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let inverse = dot(v, v).sqrt().max(1e-8).recip();
    [v[0] * inverse, v[1] * inverse, v[2] * inverse]
}

fn refract(incident: [f32; 3], normal: [f32; 3], eta: f32) -> Option<[f32; 3]> {
    let cos = -dot(incident, normal);
    let k = 1.0 - eta * eta * (1.0 - cos * cos);
    (k >= 0.0).then(|| {
        let v = eta * cos - k.sqrt();
        [
            eta * incident[0] + v * normal[0],
            eta * incident[1] + v * normal[1],
            eta * incident[2] + v * normal[2],
        ]
    })
}

pub fn refraction_offset(
    Refraction {
        uv,
        front_depth,
        back_depth,
        front_normal,
        back_normal,
        projection_scale,
        ior,
        receiver_depth,
    }: Refraction,
) -> [f32; 2] {
    if front_depth <= 0.0 || back_depth <= front_depth || ior <= 1.0 {
        return [0.0; 2];
    }
    let ndc = [uv[0] * 2.0 - 1.0, 1.0 - uv[1] * 2.0];
    let entry = [
        ndc[0] * front_depth / projection_scale[0],
        ndc[1] * front_depth / projection_scale[1],
        -front_depth,
    ];
    let incident = normalize(entry);
    let front = normalize(front_normal);
    let back = normalize(back_normal);
    let Some(inside) = refract(incident, front, 1.0 / ior) else {
        return [0.0; 2];
    };
    let travel = thickness(front_depth, back_depth) / (-inside[2]).max(0.1);
    let exit = [
        entry[0] + inside[0] * travel,
        entry[1] + inside[1] * travel,
        -back_depth,
    ];
    let outgoing = refract(inside, [-back[0], -back[1], -back[2]], ior).unwrap_or(inside);
    let reach = (receiver_depth - back_depth).max(0.0) / (-outgoing[2]).max(0.1);
    let target = [exit[0] + outgoing[0] * reach, exit[1] + outgoing[1] * reach];
    let target_uv = [
        0.5 + target[0] * projection_scale[0] / (2.0 * receiver_depth.max(back_depth)),
        0.5 - target[1] * projection_scale[1] / (2.0 * receiver_depth.max(back_depth)),
    ];
    [target_uv[0] - uv[0], target_uv[1] - uv[1]]
}

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = (0..4).map(|k| a[k][row] * b[col][k]).sum();
        }
    }
    out
}

fn transform(m: Matrix, p: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| (0..4).map(|col| m[col][row] * p[col]).sum())
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normal_columns(model_view: Matrix) -> [[f32; 4]; 3] {
    let a = [model_view[0][0], model_view[0][1], model_view[0][2]];
    let b = [model_view[1][0], model_view[1][1], model_view[1][2]];
    let c = [model_view[2][0], model_view[2][1], model_view[2][2]];
    let columns = [cross(b, c), cross(c, a), cross(a, b)];
    let determinant = dot(a, columns[0]);
    columns.map(|v| {
        [
            v[0] / determinant,
            v[1] / determinant,
            v[2] / determinant,
            0.0,
        ]
    })
}

fn world_from_view(view: Matrix) -> [[f32; 4]; 3] {
    let columns = normal_columns(view);
    let eye = std::array::from_fn::<f32, 3, _>(|i| {
        -(view[i][0] * view[3][0] + view[i][1] * view[3][1] + view[i][2] * view[3][2])
    });
    [
        [columns[0][0], columns[1][0], columns[2][0], eye[0]],
        [columns[0][1], columns[1][1], columns[2][1], eye[1]],
        [columns[0][2], columns[1][2], columns[2][2], eye[2]],
    ]
}

fn target(
    device: &wgpu::Device,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
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
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    dimension: wgpu::TextureViewDimension,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

struct PipelineSpec<'a> {
    layout: &'a wgpu::PipelineLayout,
    vertex: &'a str,
    fragment: &'a str,
    targets: &'a [Option<wgpu::ColorTargetState>],
    cull: Option<wgpu::Face>,
    depth: Option<wgpu::DepthStencilState>,
}

fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    spec: PipelineSpec<'_>,
) -> wgpu::RenderPipeline {
    let PipelineSpec {
        layout,
        vertex,
        fragment,
        targets,
        cull,
        depth,
    } = spec;
    let attributes =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
    let buffers = [wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    }];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex),
            compilation_options: Default::default(),
            buffers: if vertex == "surface_vertex" || vertex == "cast_vertex" {
                &buffers
            } else {
                &[]
            },
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets,
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: cull,
            ..Default::default()
        },
        depth_stencil: depth,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}

impl Pass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = [width.max(1), height.max(1)];
        let (front, front_view) = target(device, size, HDR_FORMAT, "glass front normals and depth");
        let (front_id, front_id_view) = target(
            device,
            size,
            wgpu::TextureFormat::R32Uint,
            "glass front ids",
        );
        let (front_look, front_look_view) = target(device, size, LOOK_FORMAT, "glass front look");
        let (front_depth, front_depth_view) = target(
            device,
            size,
            wgpu::TextureFormat::Depth32Float,
            "glass front depth",
        );
        let (back, back_view) = target(device, size, HDR_FORMAT, "glass back normals and depth");
        let (back_depth, back_depth_view) = target(
            device,
            size,
            wgpu::TextureFormat::Depth32Float,
            "glass back depth",
        );
        let (reactive, reactive_view) =
            target(device, size, REACTIVE_FORMAT, "glass reactive mask");
        let scene = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glass scene"),
            size: std::mem::size_of::<SceneGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let objects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glass objects"),
            size: std::mem::size_of::<ObjectGpu>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass scene layout"),
            entries: &scene_entries(),
        });
        let id_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass id layout"),
            entries: &id_entries(),
        });
        let compose_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass compose layout"),
            entries: &compose_entries(),
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass WGSL"),
            source: wgpu::ShaderSource::Wgsl(glass_source().into()),
        });
        let front_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glass front pipeline layout"),
            bind_group_layouts: &[&scene_layout],
            push_constant_ranges: &[],
        });
        let back_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glass back pipeline layout"),
            bind_group_layouts: &[&scene_layout, &id_layout],
            push_constant_ranges: &[],
        });
        let compose_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("glass compose pipeline layout"),
                bind_group_layouts: &[&scene_layout, &id_layout, &compose_layout],
                push_constant_ranges: &[],
            });
        let geometry_targets = [
            Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::R32Uint,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: LOOK_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
        ];
        let front_pipe = pipeline(
            device,
            &shader,
            PipelineSpec {
                layout: &front_layout,
                vertex: "surface_vertex",
                fragment: "front_fragment",
                targets: &geometry_targets,
                cull: Some(wgpu::Face::Back),
                depth: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
            },
        );
        let back_pipe = pipeline(
            device,
            &shader,
            PipelineSpec {
                layout: &back_layout,
                vertex: "surface_vertex",
                fragment: "back_fragment",
                targets: &geometry_targets[..1],
                cull: Some(wgpu::Face::Front),
                depth: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Greater,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
            },
        );
        let compose_targets = [
            Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: REACTIVE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }),
        ];
        let compose_pipe = pipeline(
            device,
            &shader,
            PipelineSpec {
                layout: &compose_pipeline_layout,
                vertex: "fullscreen_vertex",
                fragment: "compose_fragment",
                targets: &compose_targets,
                cull: None,
                depth: None,
            },
        );
        let pick_pipe = pipeline(
            device,
            &shader,
            PipelineSpec {
                layout: &compose_pipeline_layout,
                vertex: "fullscreen_vertex",
                fragment: "pick_fragment",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R32Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                cull: None,
                depth: None,
            },
        );
        let cast_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass cast layout"),
            entries: &cast_entries(),
        });
        let cast_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass cast WGSL"),
            source: wgpu::ShaderSource::Wgsl(CAST_WGSL.into()),
        });
        let cast_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glass cast pipeline layout"),
            bind_group_layouts: &[&cast_layout],
            push_constant_ranges: &[],
        });
        let cast_pipe = pipeline(
            device,
            &cast_shader,
            PipelineSpec {
                layout: &cast_pipeline_layout,
                vertex: "cast_vertex",
                fragment: "cast_fragment",
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::shadow::TRANSMISSION_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                cull: None,
                depth: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
            },
        );
        let cast_capacity = crate::shadow::CASCADE_COUNT;
        let cast_objects = cast_buffer(device, cast_capacity);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glass sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            width: size[0],
            height: size[1],
            viewport: (size[0], size[1]),
            _front: front,
            front_view,
            _front_id: front_id,
            front_id_view,
            _front_look: front_look,
            front_look_view,
            _front_depth: front_depth,
            front_depth_view,
            _back: back,
            back_view,
            _back_depth: back_depth,
            back_depth_view,
            _reactive: reactive,
            reactive_view,
            scene,
            objects,
            capacity: 1,
            scene_layout,
            id_layout,
            compose_layout,
            sampler,
            front_pipe,
            back_pipe,
            compose_pipe,
            pick_pipe,
            picks: None,
            cast_layout,
            cast_pipe,
            cast_objects,
            cast_capacity,
            casts: None,
            meshes: GlassMeshes::default(),
        }
    }

    pub fn prepare_casts(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: &[Matrix],
        surfaces: &[Surface],
        fluid_height: &wgpu::TextureView,
        ripple_height: &wgpu::TextureView,
    ) -> Result<bool, String> {
        self.casts = None;
        let casting: Vec<&Surface> = surfaces
            .iter()
            .filter(|surface| surface.casts_shadow)
            .collect();
        if casting.is_empty() || layers.is_empty() {
            return Ok(false);
        }
        for surface in &casting {
            if self.meshes.get(surface.mesh).is_none() {
                return Err("glass surface refers to a missing or released mesh".into());
            }
            if !surface.clip.iter().flatten().all(|value| value.is_finite()) {
                return Err("glass clip planes must be finite".into());
            }
        }
        let rows: Vec<CastGpu> = layers
            .iter()
            .flat_map(|view_proj| {
                casting.iter().map(move |surface| {
                    let tint = cast_tint(&surface.material);
                    let tinted = cast_tint(&surface.tinted.unwrap_or(surface.material));
                    CastGpu {
                        model: surface.model,
                        view_proj: *view_proj,
                        tint: [tint[0], tint[1], tint[2], 1.0],
                        tinted: [tinted[0], tinted[1], tinted[2], 1.0],
                        liquid: [
                            if surface.liquid { 1.0 } else { 0.0 },
                            surface.fluid_height,
                            surface.ripple_height,
                            0.0,
                        ],
                        clip: surface.clip,
                    }
                })
            })
            .collect();
        if rows.len() > self.cast_capacity {
            self.cast_capacity = rows.len().next_power_of_two();
            self.cast_objects = cast_buffer(device, self.cast_capacity);
        }
        queue.write_buffer(&self.cast_objects, 0, bytemuck::cast_slice(&rows));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass cast group"),
            layout: &self.cast_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cast_objects.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(fluid_height),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(ripple_height),
                },
            ],
        });
        self.casts = Some(Casts {
            group,
            draws: casting.iter().map(|surface| surface.mesh).collect(),
        });
        Ok(true)
    }

    pub fn draw_casts(&self, pass: &mut wgpu::RenderPass<'_>, layer: usize) {
        let Some(casts) = &self.casts else {
            return;
        };
        pass.set_pipeline(&self.cast_pipe);
        pass.set_bind_group(0, &casts.group, &[]);
        let first = (layer * casts.draws.len()) as u32;
        for (index, handle) in casts.draws.iter().enumerate() {
            let Some(mesh) = self.meshes.get(*handle) else {
                continue;
            };
            let row = first + index as u32;
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, row..row + 1);
        }
    }

    pub fn reactive(&self) -> &wgpu::TextureView {
        &self.reactive_view
    }
    pub fn front_depth(&self) -> &wgpu::TextureView {
        &self.front_depth_view
    }
    pub fn front_id(&self) -> &wgpu::TextureView {
        &self.front_id_view
    }
    pub fn front_look(&self) -> &wgpu::TextureView {
        &self.front_look_view
    }
    pub fn mesh_buffers(&self, handle: MeshHandle) -> Option<(&wgpu::Buffer, &wgpu::Buffer, u32)> {
        self.meshes
            .get(handle)
            .map(|mesh| (&*mesh.vertices, &*mesh.indices, mesh.count))
    }

    pub fn mesh_stats(&self) -> GlassStats {
        self.meshes.stats()
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    pub fn textures(&self) -> Vec<wgpu::Texture> {
        vec![
            self._front.clone(),
            self._front_id.clone(),
            self._front_look.clone(),
            self._front_depth.clone(),
            self._back.clone(),
            self._back_depth.clone(),
            self._reactive.clone(),
        ]
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), String> {
        crate::viewport::check([width, height], [self.width, self.height])?;
        self.viewport = (width, height);
        Ok(())
    }

    pub fn take_meshes(&mut self) -> GlassMeshes {
        std::mem::take(&mut self.meshes)
    }

    pub fn set_meshes(&mut self, meshes: GlassMeshes) {
        self.meshes = meshes;
    }

    pub fn release_mesh(&mut self, handle: MeshHandle) -> Result<(), String> {
        self.meshes.release(handle)
    }

    pub fn replace_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        handle: MeshHandle,
        mesh: &Mesh,
    ) -> Result<(), String> {
        self.meshes.replace(device, queue, handle, mesh)
    }
    pub fn front_normals(&self) -> &wgpu::TextureView {
        &self.front_view
    }
    pub fn back_normals(&self) -> &wgpu::TextureView {
        &self.back_view
    }
    pub fn back_depth(&self) -> &wgpu::TextureView {
        &self.back_depth_view
    }

    pub fn upload_mesh(
        &mut self,
        device: &wgpu::Device,
        mesh: &Mesh,
    ) -> Result<MeshHandle, String> {
        self.meshes.upload(device, mesh)
    }

    pub fn liquid_grid(columns: u32, rows: u32, half_extent: [f32; 2], depth: f32) -> Mesh {
        let columns = columns.max(1);
        let rows = rows.max(1);
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        let count = (columns + 1) * (rows + 1);
        for layer in 0..2 {
            for y in 0..=rows {
                for x in 0..=columns {
                    let uv = [x as f32 / columns as f32, y as f32 / rows as f32];
                    positions.push([
                        (uv[0] * 2.0 - 1.0) * half_extent[0],
                        (uv[1] * 2.0 - 1.0) * half_extent[1],
                        if layer == 0 { 0.0 } else { -depth.max(0.001) },
                    ]);
                    normals.push(if layer == 0 {
                        [0.0, 0.0, 1.0]
                    } else {
                        [0.0, 0.0, -1.0]
                    });
                    uvs.push(uv);
                }
            }
        }
        for y in 0..rows {
            for x in 0..columns {
                let a = y * (columns + 1) + x;
                let b = a + 1;
                let c = a + columns + 1;
                let d = c + 1;
                indices.extend([a, b, d, a, d, c]);
                indices.extend([
                    a + count,
                    d + count,
                    b + count,
                    a + count,
                    c + count,
                    d + count,
                ]);
            }
        }
        let perimeter = |x: u32, y: u32| y * (columns + 1) + x;
        for x in 0..columns {
            for (a, b) in [
                (perimeter(x, 0), perimeter(x + 1, 0)),
                (perimeter(x + 1, rows), perimeter(x, rows)),
            ] {
                indices.extend([a, a + count, b + count, a, b + count, b]);
            }
        }
        for y in 0..rows {
            for (a, b) in [
                (perimeter(columns, y), perimeter(columns, y + 1)),
                (perimeter(0, y + 1), perimeter(0, y)),
            ] {
                indices.extend([a, a + count, b + count, a, b + count, b]);
            }
        }
        Mesh::new(positions, normals, Vec::new(), uvs, indices)
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.capacity {
            return;
        }
        self.capacity = count.next_power_of_two();
        self.objects = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glass objects"),
            size: (self.capacity * std::mem::size_of::<ObjectGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        Render {
            camera,
            surfaces,
            inputs,
            output,
            profiler,
        }: Render<'_>,
    ) -> Result<(), String> {
        let drawn: Vec<Surface> = surfaces
            .iter()
            .filter(|surface| !surface.shadow_only)
            .copied()
            .collect();
        let surfaces = drawn.as_slice();
        if surfaces.len() > 4095 {
            return Err("glass pass supports at most 4095 surfaces".into());
        }
        let view_det = dot(
            [camera.view[0][0], camera.view[0][1], camera.view[0][2]],
            cross(
                [camera.view[1][0], camera.view[1][1], camera.view[1][2]],
                [camera.view[2][0], camera.view[2][1], camera.view[2][2]],
            ),
        );
        if !view_det.is_finite() || view_det.abs() < 1e-8 {
            return Err("glass camera has a singular view transform".into());
        }
        for surface in surfaces {
            if self.meshes.get(surface.mesh).is_none() {
                return Err("glass surface refers to a missing or released mesh".into());
            }
            let mv = multiply(camera.view, surface.model);
            let determinant = dot(
                [mv[0][0], mv[0][1], mv[0][2]],
                cross(
                    [mv[1][0], mv[1][1], mv[1][2]],
                    [mv[2][0], mv[2][1], mv[2][2]],
                ),
            );
            if !determinant.is_finite() || determinant.abs() < 1e-8 {
                return Err("glass model has a singular transform".into());
            }
            if !surface.clip.iter().flatten().all(|value| value.is_finite()) {
                return Err("glass clip planes must be finite".into());
            }
        }
        self.ensure_capacity(device, surfaces.len());
        let meshes: Vec<&MeshGpu> = surfaces
            .iter()
            .filter_map(|surface| self.meshes.get(surface.mesh))
            .collect();
        let scene = SceneGpu {
            projection: camera.projection,
            world_from_view: world_from_view(camera.view),
            extent: [
                self.viewport.0 as f32,
                self.viewport.1 as f32,
                inputs.mip_count.max(1) as f32,
                0.0,
            ],
            caustic: CausticProjection::rows(inputs.caustic_projection, 0, 0.0),
            sky: inputs.sky_sh.map(|c| [c[0], c[1], c[2], 0.0]),
        };
        queue.write_buffer(&self.scene, 0, bytemuck::bytes_of(&scene));
        let objects: Vec<_> = surfaces
            .iter()
            .map(|surface| {
                let mv = multiply(camera.view, surface.model);
                let m = surface.material;
                let t = surface.tinted.unwrap_or(m);
                ObjectGpu {
                    model_view: mv,
                    mvp: multiply(camera.projection, mv),
                    normal: normal_columns(mv),
                    base_roughness: [m.base[0], m.base[1], m.base[2], m.roughness],
                    tint_absorption: [
                        m.subsurface_tint[0],
                        m.subsurface_tint[1],
                        m.subsurface_tint[2],
                        m.absorption,
                    ],
                    optics: [m.ior, m.dispersion, m.transmission, m.subsurface],
                    film: [
                        m.thin_film,
                        m.thin_film_ior,
                        m.thin_film_amount,
                        surface.caustic_strength,
                    ],
                    liquid: [
                        if surface.liquid { 1.0 } else { 0.0 },
                        surface.fluid_height,
                        surface.ripple_height,
                        0.0,
                    ],
                    tinted_base: [t.base[0], t.base[1], t.base[2], t.thin_film_amount],
                    tinted_subsurface: [
                        t.subsurface_tint[0],
                        t.subsurface_tint[1],
                        t.subsurface_tint[2],
                        t.subsurface,
                    ],
                    pick: [surface.id, 0, 0, 0],
                    clip: surface.clip,
                }
            })
            .collect();
        if !objects.is_empty() {
            queue.write_buffer(&self.objects, 0, bytemuck::cast_slice(&objects));
        }
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass scene group"),
            layout: &self.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.scene.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.objects.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(inputs.fluid_height),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs.ripple_height),
                },
            ],
        });
        let id_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass front id group"),
            layout: &self.id_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&self.front_id_view),
            }],
        });
        let compose_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass compose group"),
            layout: &self.compose_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.front_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.back_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.front_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs.opaque_hdr_mips),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(inputs.reflection_cube),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(inputs.caustic),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(inputs.opaque_depth),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&self.front_look_view),
                },
            ],
        });
        let shown: Vec<Option<[u32; 4]>> = surfaces
            .iter()
            .zip(&objects)
            .zip(&meshes)
            .map(|((surface, object), mesh)| {
                self.bounds(
                    &object.mvp,
                    &mesh.bounds,
                    surface.fluid_height.abs() + surface.ripple_height.abs(),
                )
            })
            .collect();
        let rects: Vec<[u32; 4]> = shown.iter().flatten().copied().collect();
        let mut profiler = profiler;
        let front_time = profiler.as_deref_mut().and_then(|p| p.pass("glass front"));
        let back_time = profiler.as_deref_mut().and_then(|p| p.pass("glass back"));
        let compose_time = profiler
            .as_deref_mut()
            .and_then(|p| p.pass("glass compose"));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass front"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.front_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.front_id_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.front_look_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.front_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|p| p.render_writes(front_time)),
            });
            crate::viewport::apply(&mut pass, [self.viewport.0, self.viewport.1]);
            pass.set_pipeline(&self.front_pipe);
            pass.set_bind_group(0, &scene_group, &[]);
            for (index, mesh) in meshes.iter().enumerate() {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, index as u32..index as u32 + 1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass back"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.back_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.back_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: profiler.as_deref().and_then(|p| p.render_writes(back_time)),
            });
            crate::viewport::apply(&mut pass, [self.viewport.0, self.viewport.1]);
            pass.set_pipeline(&self.back_pipe);
            pass.set_bind_group(0, &scene_group, &[]);
            pass.set_bind_group(1, &id_group, &[]);
            for (index, mesh) in meshes.iter().enumerate() {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, index as u32..index as u32 + 1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass composite"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: output,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &self.reactive_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: profiler
                    .as_deref()
                    .and_then(|p| p.render_writes(compose_time)),
            });
            crate::viewport::apply(&mut pass, [self.viewport.0, self.viewport.1]);
            pass.set_pipeline(&self.compose_pipe);
            pass.set_bind_group(0, &scene_group, &[]);
            pass.set_bind_group(1, &id_group, &[]);
            pass.set_bind_group(2, &compose_group, &[]);
            for rect in &rects {
                pass.set_scissor_rect(rect[0], rect[1], rect[2], rect[3]);
                pass.draw(0..3, 0..1);
            }
        }
        let picked: Vec<[u32; 4]> = surfaces
            .iter()
            .zip(&shown)
            .filter_map(|(surface, rect)| (surface.id != 0).then_some(*rect).flatten())
            .collect();
        self.picks = (!picked.is_empty()).then_some(Picks {
            scene: scene_group,
            ids: id_group,
            compose: compose_group,
            rects: picked,
        });
        Ok(())
    }

    pub fn has_ids(&self) -> bool {
        self.picks.is_some()
    }

    pub fn encode_ids(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        ids: &wgpu::TextureView,
        profiler: Option<&mut GpuProfiler>,
    ) {
        let Some(picks) = &self.picks else {
            return;
        };
        let mut profiler = profiler;
        let timing = profiler.as_deref_mut().and_then(|p| p.pass("glass ids"));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("glass ids"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ids,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: profiler.as_deref().and_then(|p| p.render_writes(timing)),
        });
        crate::viewport::apply(&mut pass, [self.viewport.0, self.viewport.1]);
        pass.set_pipeline(&self.pick_pipe);
        pass.set_bind_group(0, &picks.scene, &[]);
        pass.set_bind_group(1, &picks.ids, &[]);
        pass.set_bind_group(2, &picks.compose, &[]);
        for rect in &picks.rects {
            pass.set_scissor_rect(rect[0], rect[1], rect[2], rect[3]);
            pass.draw(0..3, 0..1);
        }
    }

    fn bounds(&self, mvp: &Matrix, bounds: &[[f32; 3]; 2], padding: f32) -> Option<[u32; 4]> {
        let (width, height) = self.viewport;
        let mut low = [f32::INFINITY; 2];
        let mut high = [f32::NEG_INFINITY; 2];
        let mut behind = false;
        let mut visible = false;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let p = [
                        bounds[x][0],
                        bounds[y][1],
                        bounds[z][2] + if z == 1 { padding } else { -padding },
                        1.0,
                    ];
                    let clip = transform(*mvp, p);
                    if clip[3] <= 0.0 {
                        behind = true;
                        continue;
                    }
                    visible = true;
                    let uv = [
                        (clip[0] / clip[3] * 0.5 + 0.5) * width as f32,
                        (0.5 - clip[1] / clip[3] * 0.5) * height as f32,
                    ];
                    for i in 0..2 {
                        low[i] = low[i].min(uv[i]);
                        high[i] = high[i].max(uv[i]);
                    }
                }
            }
        }
        if !visible {
            return None;
        }
        if behind {
            return Some([0, 0, width, height]);
        }
        let x0 = (low[0].floor() - 2.0).clamp(0.0, width as f32) as u32;
        let y0 = (low[1].floor() - 2.0).clamp(0.0, height as f32) as u32;
        let x1 = (high[0].ceil() + 2.0).clamp(0.0, width as f32) as u32;
        let y1 = (high[1].ceil() + 2.0).clamp(0.0, height as f32) as u32;
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }
}

pub fn glass_source() -> String {
    format!("{}\n{}", pfx_materials::FILM, GLASS_WGSL)
}

pub const GLASS_WGSL: &str = r#"
struct Scene {
    projection: mat4x4f,
    world0: vec4f,
    world1: vec4f,
    world2: vec4f,
    extent: vec4f,
    caustic_rect: vec4f,
    caustic_plane: vec4f,
    caustic_info: vec4f,
    sky: array<vec4f, 9>,
}
struct Object {
    model_view: mat4x4f,
    mvp: mat4x4f,
    normal0: vec4f,
    normal1: vec4f,
    normal2: vec4f,
    base_roughness: vec4f,
    tint_absorption: vec4f,
    optics: vec4f,
    film: vec4f,
    liquid: vec4f,
    tinted_base: vec4f,
    tinted_subsurface: vec4f,
    pick: vec4u,
    clip0: vec4f,
    clip1: vec4f,
}
@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(2) var fluid_tex: texture_2d<f32>;
@group(0) @binding(3) var ripple_tex: texture_2d<f32>;
@group(1) @binding(0) var front_id_tex: texture_2d<u32>;
@group(2) @binding(0) var front_tex: texture_2d<f32>;
@group(2) @binding(1) var back_tex: texture_2d<f32>;
@group(2) @binding(2) var front_depth_tex: texture_depth_2d;
@group(2) @binding(3) var opaque_tex: texture_2d<f32>;
@group(2) @binding(4) var reflection_tex: texture_cube<f32>;
@group(2) @binding(5) var caustic_tex: texture_2d<f32>;
@group(2) @binding(6) var opaque_depth_tex: texture_depth_2d;
@group(2) @binding(7) var linear_sampler: sampler;
@group(2) @binding(8) var front_look_tex: texture_2d<f32>;

struct SurfaceVertex {
    @location(0) position: vec4f,
    @location(1) normal: vec4f,
    @location(2) uv: vec4f,
    @location(3) look: vec4f,
}
struct SurfaceOut {
    @builtin(position) position: vec4f,
    @location(0) view_normal: vec3f,
    @location(1) view_depth: f32,
    @location(2) @interpolate(flat) object_id: u32,
    @location(3) look: vec4f,
    @location(4) world: vec3f,
}
struct FrontOut {
    @location(0) normal_depth: vec4f,
    @location(1) id: u32,
    @location(2) look: vec4f,
}
struct ComposeOut {
    @location(0) color: vec4f,
    @location(1) reactive: f32,
}
fn height_at(uv: vec2f, object: Object) -> f32 {
    let fluid_size = vec2f(textureDimensions(fluid_tex));
    let ripple_size = vec2f(textureDimensions(ripple_tex));
    let fluid_xy = vec2i(clamp(uv * fluid_size, vec2f(0.0), fluid_size - vec2f(1.0)));
    let ripple_xy = vec2i(clamp(uv * ripple_size, vec2f(0.0), ripple_size - vec2f(1.0)));
    return textureLoad(fluid_tex, fluid_xy, 0).r * object.liquid.y + textureLoad(ripple_tex, ripple_xy, 0).r * object.liquid.z;
}
fn to_world(view_point: vec3f) -> vec3f {
    return mat3x3f(scene.world0.xyz, scene.world1.xyz, scene.world2.xyz) * view_point + vec3f(scene.world0.w, scene.world1.w, scene.world2.w);
}
fn clip_plane(plane: vec4f, world: vec3f) -> bool {
    return any(plane.xyz != vec3f(0.0)) && dot(plane.xyz, world) > plane.w;
}
fn clipped(object: Object, world: vec3f) -> bool {
    return clip_plane(object.clip0, world) || clip_plane(object.clip1, world);
}
fn cap(plane: vec4f, world: vec3f, t: f32) -> f32 {
    if (!clip_plane(plane, world)) { return t; }
    let eye = vec3f(scene.world0.w, scene.world1.w, scene.world2.w);
    let along = dot(plane.xyz, world - eye);
    if (abs(along) < 1e-8) { return 0.0; }
    return min(t, (plane.w - dot(plane.xyz, eye)) / along);
}
@vertex fn surface_vertex(v: SurfaceVertex, @builtin(instance_index) index: u32) -> SurfaceOut {
    let object = objects[index];
    var p = v.position;
    var n = v.normal.xyz;
    if (object.liquid.x > 0.5 && n.z > 0.5) {
        let step = 1.0 / vec2f(textureDimensions(fluid_tex));
        let dx = height_at(v.uv.xy + vec2f(step.x, 0.0), object) - height_at(v.uv.xy - vec2f(step.x, 0.0), object);
        let dy = height_at(v.uv.xy + vec2f(0.0, step.y), object) - height_at(v.uv.xy - vec2f(0.0, step.y), object);
        p.z += height_at(v.uv.xy, object);
        n = normalize(vec3f(-dx / (2.0 * step.x), -dy / (2.0 * step.y), 1.0));
    }
    let view = object.model_view * p;
    var out: SurfaceOut;
    out.position = object.mvp * p;
    out.view_normal = normalize(mat3x3f(object.normal0.xyz, object.normal1.xyz, object.normal2.xyz) * n);
    out.view_depth = -view.z;
    out.object_id = index + 1u;
    out.look = v.look;
    out.world = to_world(view.xyz);
    return out;
}
@fragment fn front_fragment(input: SurfaceOut) -> FrontOut {
    if (input.look.z < 0.0) { discard; }
    if (clipped(objects[input.object_id - 1u], input.world)) { discard; }
    var out: FrontOut;
    out.normal_depth = vec4f(input.view_normal, input.view_depth);
    out.id = input.object_id;
    out.look = vec4f(input.look.xy, select(0.0, 1.0, input.look.w > 0.5), 0.0);
    return out;
}
@fragment fn back_fragment(input: SurfaceOut) -> @location(0) vec4f {
    let pixel = vec2i(input.position.xy);
    if (textureLoad(front_id_tex, pixel, 0).r != input.object_id) { discard; }
    let object = objects[input.object_id - 1u];
    if (!clipped(object, input.world)) {
        return vec4f(input.view_normal, input.view_depth);
    }
    let first = cap(object.clip0, input.world, 1.0);
    let t = cap(object.clip1, input.world, first);
    if (t <= 0.0) { discard; }
    var plane = object.clip1.xyz;
    if (t == first) {
        plane = object.clip0.xyz;
    }
    let normal = normalize(transpose(mat3x3f(scene.world0.xyz, scene.world1.xyz, scene.world2.xyz)) * plane);
    return vec4f(normal, input.view_depth * t);
}
@vertex fn fullscreen_vertex(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4f {
    let xy = vec2f(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4f(xy * 2.0 - vec2f(1.0), 0.0, 1.0);
}
fn refract_ray(incident: vec3f, normal: vec3f, eta: f32) -> vec3f {
    let cosine = -dot(incident, normal);
    let k = 1.0 - eta * eta * (1.0 - cosine * cosine);
    if (k < 0.0) { return reflect(incident, normal); }
    return eta * incident + (eta * cosine - sqrt(k)) * normal;
}
fn landing(uv: vec2f, front_depth: f32, back_depth: f32, front_normal: vec3f, back_normal: vec3f, ior: f32, receiver_depth: f32) -> vec4f {
    let ndc = vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let entry = vec3f(ndc * front_depth / vec2f(scene.projection[0].x, scene.projection[1].y), -front_depth);
    let incident = normalize(entry);
    let inside = refract_ray(incident, front_normal, 1.0 / ior);
    let travel = max(back_depth - front_depth, 0.0) / max(-inside.z, 0.1);
    let exit = entry + inside * travel;
    let outgoing = refract_ray(inside, -back_normal, ior);
    let reach = max(receiver_depth - back_depth, 0.0) / max(-outgoing.z, 0.1);
    let hit = exit + outgoing * reach;
    return vec4f(hit, max(receiver_depth, back_depth));
}
fn sample_uv(uv: vec2f, front_depth: f32, back_depth: f32, front_normal: vec3f, back_normal: vec3f, ior: f32, receiver_depth: f32) -> vec2f {
    let land = landing(uv, front_depth, back_depth, front_normal, back_normal, ior, receiver_depth);
    return vec2f(0.5 + land.x * scene.projection[0].x / (2.0 * land.w), 0.5 - land.y * scene.projection[1].y / (2.0 * land.w));
}
fn caustic_light(view_point: vec3f, strength: f32) -> vec3f {
    if (scene.caustic_info.z < 0.5 || strength == 0.0) {
        return vec3f(0.0);
    }
    let world = to_world(view_point);
    let toward = scene.caustic_plane.xyz;
    let along = (scene.caustic_plane.w - world.y) / max(toward.y, 1e-4);
    let at = (world.xz + toward.xz * along - scene.caustic_rect.xy) / scene.caustic_rect.zw;
    if (any(at < vec2f(0.0)) || any(at > vec2f(1.0))) {
        return vec3f(0.0);
    }
    return textureSampleLevel(caustic_tex, linear_sampler, at, 0.0).rgb * strength;
}
fn sky_irradiance(n: vec3f) -> vec3f {
    let d = normalize(n);
    let basis0 = vec3f(0.282095, 0.488603 * d.y, 0.488603 * d.z);
    let basis1 = vec3f(0.488603 * d.x, 1.092548 * d.x * d.y, 1.092548 * d.y * d.z);
    let basis2 = vec3f(0.315392 * (3.0 * d.z * d.z - 1.0), 1.092548 * d.x * d.z, 0.546274 * (d.x * d.x - d.y * d.y));
    let sh = scene.sky;
    let value = sh[0].xyz * basis0.x + sh[1].xyz * basis0.y + sh[2].xyz * basis0.z
        + sh[3].xyz * basis1.x + sh[4].xyz * basis1.y + sh[5].xyz * basis1.z
        + sh[6].xyz * basis2.x + sh[7].xyz * basis2.y + sh[8].xyz * basis2.z;
    return max(value, vec3f(0.0));
}
@fragment fn compose_fragment(@builtin(position) position: vec4f) -> ComposeOut {
    let pixel = vec2i(position.xy);
    let id = textureLoad(front_id_tex, pixel, 0).r;
    if (id == 0u) { discard; }
    let front_z = textureLoad(front_depth_tex, pixel, 0);
    let opaque_z = textureLoad(opaque_depth_tex, pixel, 0);
    if (opaque_z < front_z) { discard; }
    let front = textureLoad(front_tex, pixel, 0);
    let back = textureLoad(back_tex, pixel, 0);
    if (back.w <= front.w) { discard; }
    let object = objects[id - 1u];
    let look = textureLoad(front_look_tex, pixel, 0);
    var base = object.base_roughness.xyz;
    var scatter_tint = object.tint_absorption.xyz;
    var subsurface = object.optics.w;
    var film_amount = object.film.z;
    if (look.z > 0.5) {
        let t = clamp(look.x, 0.0, 1.0);
        base = mix(base, object.tinted_base.xyz, t);
        scatter_tint = mix(scatter_tint, object.tinted_subsurface.xyz, t);
        film_amount = mix(film_amount, object.tinted_base.w, t);
        subsurface = mix(subsurface, object.tinted_subsurface.w, t) * max(look.y, 0.0);
    }
    let uv = position.xy / scene.extent.xy;
    let ray = normalize(vec3f(vec2f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0) / vec2f(scene.projection[0].x, scene.projection[1].y), -1.0));
    let normal = normalize(front.xyz);
    let inner = refract_ray(ray, normal, 1.0 / max(object.optics.x, 1.01));
    let distance = (back.w - front.w) / max(-inner.z, 0.1);
    var background_depth = back.w + max(back.w - front.w, 0.01) * 2.0;
    if (opaque_z < 0.999) {
        let linear = scene.projection[3].z / (opaque_z + scene.projection[2].z);
        background_depth = max(linear, back.w);
    }
    let max_lod = max(scene.extent.z - 1.0, 0.0);
    let lod = clamp(object.base_roughness.w * object.base_roughness.w * max_lod, 0.0, max_lod);
    let ior = vec3f(object.optics.x - 0.016 * object.optics.y, object.optics.x, object.optics.x + 0.022 * object.optics.y);
    var refracted = vec3f(0.0);
    for (var channel = 0u; channel < 3u; channel++) {
        let sample_at = sample_uv(uv, front.w, back.w, normal, normalize(back.xyz), ior[channel], background_depth);
        let fit = scene.extent.xy / vec2f(textureDimensions(opaque_tex));
        let sample_color = textureSampleLevel(opaque_tex, linear_sampler, clamp(sample_at, vec2f(0.0), vec2f(1.0)) * fit, lod).rgb;
        refracted[channel] = sample_color[channel];
    }
    let absorption = exp(-max(object.tint_absorption.w, 0.0) * distance * (vec3f(1.0) - clamp(scatter_tint, vec3f(0.0), vec3f(1.0))));
    refracted *= absorption * base;
    var scattered = clamp(subsurface * (1.0 - exp(-distance)), 0.0, 1.0);
    if (look.z > 0.5) {
        scattered = 1.0 - exp(-subsurface * distance);
    }
    let world = mat3x3f(scene.world0.xyz, scene.world1.xyz, scene.world2.xyz);
    let lit_scatter = scatter_tint * sky_irradiance(world * normal) / 3.14159265;
    refracted = mix(refracted, lit_scatter, scattered);
    let cosine = clamp(-dot(ray, normal), 0.0, 1.0);
    let f0 = pow((object.optics.x - 1.0) / (object.optics.x + 1.0), 2.0);
    var fresnel = vec3f(f0 + (1.0 - f0) * pow(1.0 - cosine, 5.0));
    if (film_amount > 0.0 && object.film.x > 0.0) {
        fresnel = mix(fresnel, film_dielectric(cosine, object.film.x, object.film.y, object.optics.x), clamp(film_amount, 0.0, 1.0));
    }
    let reflected_world = normalize(world * reflect(ray, normal));
    let reflected = textureSampleLevel(reflection_tex, linear_sampler, reflected_world, lod).rgb;
    let caustic = caustic_light(landing(uv, front.w, back.w, normal, normalize(back.xyz), ior.y, background_depth).xyz, object.film.w);
    let color = mix(reflected, refracted + caustic, (vec3f(1.0) - fresnel) * clamp(object.optics.z, 0.0, 1.0));
    var out: ComposeOut;
    out.color = vec4f(color, 1.0);
    out.reactive = 1.0;
    return out;
}
@fragment fn pick_fragment(@builtin(position) position: vec4f) -> @location(0) u32 {
    let pixel = vec2i(position.xy);
    let id = textureLoad(front_id_tex, pixel, 0).r;
    if (id == 0u) { discard; }
    let front_z = textureLoad(front_depth_tex, pixel, 0);
    let opaque_z = textureLoad(opaque_depth_tex, pixel, 0);
    if (opaque_z < front_z) { discard; }
    let front = textureLoad(front_tex, pixel, 0);
    let back = textureLoad(back_tex, pixel, 0);
    if (back.w <= front.w) { discard; }
    let pick = objects[id - 1u].pick.x;
    if (pick == 0u) { discard; }
    return pick;
}
"#;

pub const CAST_WGSL: &str = r#"
struct Cast {
    model: mat4x4f,
    view_proj: mat4x4f,
    tint: vec4f,
    tinted: vec4f,
    liquid: vec4f,
    clip0: vec4f,
    clip1: vec4f,
}
@group(0) @binding(0) var<storage, read> casts: array<Cast>;
@group(0) @binding(1) var cast_fluid: texture_2d<f32>;
@group(0) @binding(2) var cast_ripple: texture_2d<f32>;
struct CastVertex {
    @location(0) position: vec4f,
    @location(1) normal: vec4f,
    @location(2) uv: vec4f,
    @location(3) look: vec4f,
}
struct CastOut {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) row: u32,
    @location(1) look: vec4f,
    @location(2) world: vec3f,
}
fn cast_height(uv: vec2f, caster: Cast) -> f32 {
    let fluid_size = vec2f(textureDimensions(cast_fluid));
    let ripple_size = vec2f(textureDimensions(cast_ripple));
    let fluid_xy = vec2i(clamp(uv * fluid_size, vec2f(0.0), fluid_size - vec2f(1.0)));
    let ripple_xy = vec2i(clamp(uv * ripple_size, vec2f(0.0), ripple_size - vec2f(1.0)));
    return textureLoad(cast_fluid, fluid_xy, 0).r * caster.liquid.y + textureLoad(cast_ripple, ripple_xy, 0).r * caster.liquid.z;
}
@vertex fn cast_vertex(v: CastVertex, @builtin(instance_index) row: u32) -> CastOut {
    let caster = casts[row];
    var p = v.position.xyz;
    if (caster.liquid.x > 0.5 && v.normal.z > 0.5) {
        p.z += cast_height(v.uv.xy, caster);
    }
    let world = caster.model * vec4f(p, 1.0);
    var out: CastOut;
    out.position = caster.view_proj * world;
    out.row = row;
    out.look = v.look;
    out.world = world.xyz;
    return out;
}
fn cast_clip(plane: vec4f, world: vec3f) -> bool {
    return any(plane.xyz != vec3f(0.0)) && dot(plane.xyz, world) > plane.w;
}
@fragment fn cast_fragment(input: CastOut) -> @location(0) vec4u {
    if (input.look.z < 0.0) { discard; }
    let caster = casts[input.row];
    if (cast_clip(caster.clip0, input.world) || cast_clip(caster.clip1, input.world)) { discard; }
    var tint = caster.tint.xyz;
    if (input.look.w > 0.5) {
        tint = mix(tint, caster.tinted.xyz, clamp(input.look.x, 0.0, 1.0));
    }
    return vec4u(pack4x8unorm(vec4f(tint, 1.0)), bitcast<u32>(input.position.z), 0u, 0u);
}
"#;

pub fn scene_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
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
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        texture_entry(
            2,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::VERTEX,
        ),
        texture_entry(
            3,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::VERTEX,
        ),
    ]
}

fn cast_buffer(device: &wgpu::Device, rows: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glass casts"),
        size: (rows * std::mem::size_of::<CastGpu>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub fn cast_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        texture_entry(
            1,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::VERTEX,
        ),
        texture_entry(
            2,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::VERTEX,
        ),
    ]
}

pub fn id_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![texture_entry(
        0,
        wgpu::TextureSampleType::Uint,
        wgpu::TextureViewDimension::D2,
        wgpu::ShaderStages::FRAGMENT,
    )]
}

pub fn compose_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    vec![
        texture_entry(
            0,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
        texture_entry(
            1,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
        texture_entry(
            2,
            wgpu::TextureSampleType::Depth,
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
        texture_entry(
            3,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
        texture_entry(
            4,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::Cube,
            wgpu::ShaderStages::FRAGMENT,
        ),
        texture_entry(
            5,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
        texture_entry(
            6,
            wgpu::TextureSampleType::Depth,
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
        wgpu::BindGroupLayoutEntry {
            binding: 7,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
        texture_entry(
            8,
            wgpu::TextureSampleType::Float { filterable: true },
            wgpu::TextureViewDimension::D2,
            wgpu::ShaderStages::FRAGMENT,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pfx_geom::shapes::Shape;
    use pfx_gpu::Gpu;

    #[test]
    fn clip_planes_cut_where_the_point_lies_past_them() {
        let none = [[0.0; 4]; 2];
        assert!(!clips(&none, [5.0, 5.0, 5.0]));
        let strip = [[0.0, 1.0, 0.0, 0.1], [0.0, -1.0, 0.0, 0.0]];
        assert!(clips(&strip, [0.0, 0.2, 0.0]));
        assert!(clips(&strip, [0.0, -0.1, 0.0]));
        assert!(!clips(&strip, [0.0, 0.05, 0.0]));
    }

    #[test]
    fn a_shadow_only_surface_is_not_drawn() {
        let surface = Surface {
            mesh: MeshHandle(0),
            model: [[0.0; 4]; 4],
            material: Material::default(),
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: true,
            shadow_only: true,
            clip: [[0.0; 4]; 2],
        };
        assert!(casting(&[surface]) && !drawn(&[surface]));
        let shown = Surface {
            shadow_only: false,
            ..surface
        };
        assert!(drawn(&[surface, shown]));
        assert!(!drawn(&[]));
    }

    #[test]
    fn shader_validates() {
        for source in [glass_source().as_str(), CAST_WGSL] {
            let module = naga::front::wgsl::parse_str(source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn subsurface_takes_the_sky_at_the_normal() {
        let tint = [0.6, 0.5, 0.9];
        let uniform = subsurface_colour(
            tint,
            &uniform_sky_sh([std::f32::consts::PI; 3]),
            [0.0, 1.0, 0.0],
        );
        for k in 0..3 {
            assert!((uniform[k] - tint[k]).abs() < 1e-5, "{uniform:?}");
        }
        let mut sh = uniform_sky_sh([2.0; 3]);
        sh[1] = [1.5; 3];
        let up = subsurface_colour(tint, &sh, [0.0, 1.0, 0.0]);
        let flank = subsurface_colour(tint, &sh, [0.8, 0.6, 0.0]);
        let side = crate::frame::sky_irradiance(&sh, [0.8, 0.6, 0.0]);
        for k in 0..3 {
            assert!(flank[k] < up[k]);
            assert!((flank[k] - tint[k] * side[k] / std::f32::consts::PI).abs() < 1e-6);
            assert!((up[k] - tint[k] * (2.0 + 1.5 * 0.488603) / std::f32::consts::PI).abs() < 1e-5);
        }
    }

    #[test]
    fn front_and_back_give_thickness() {
        assert_eq!(thickness(2.0, 2.4), 0.4000001);
        assert_eq!(thickness(2.4, 2.0), 0.0);
    }

    #[test]
    fn beer_matches_materials() {
        let tint = [0.2, 0.5, 0.9];
        let actual = beer_lambert(2.0, tint, 0.7);
        let expected = pfx_materials::beer(2.0, tint, 0.7);
        for i in 0..3 {
            assert!((actual[i] - expected[i]).abs() < 1e-6);
        }
        assert!(actual[0] < actual[1] && actual[1] < actual[2]);
    }

    #[test]
    fn refraction_matches_slab_reference() {
        let uv = [0.7, 0.5];
        let offset = refraction_offset(Refraction {
            uv,
            front_depth: 2.0,
            back_depth: 2.3,
            front_normal: [0.0, 0.0, 1.0],
            back_normal: [0.0, 0.0, -1.0],
            projection_scale: [1.0, 1.0],
            ior: 1.56,
            receiver_depth: 4.0,
        });
        let incident_x = 0.4 / (1.0_f32 + 0.4 * 0.4).sqrt();
        let inside_x = incident_x / 1.56;
        let inside_z = -(1.0 - inside_x * inside_x).sqrt();
        let exit_x = 0.8 + inside_x * 0.3 / -inside_z;
        let outgoing_x = incident_x;
        let outgoing_z = -(1.0 - outgoing_x * outgoing_x).sqrt();
        let hit_x = exit_x + outgoing_x * 1.7 / -outgoing_z;
        let expected = 0.5 + hit_x / 8.0 - uv[0];
        assert!(
            (offset[0] - expected).abs() < 1e-5,
            "{offset:?} vs {expected}"
        );
        assert!(offset[0] < 0.0);
        assert!(offset[1].abs() < 1e-6);
    }

    fn identity() -> Matrix {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn solid_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: u32,
        cube: bool,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glass test solid texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for layer in 0..layers {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &[0u8; 8],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(if cube {
                wgpu::TextureViewDimension::Cube
            } else {
                wgpu::TextureViewDimension::D2
            }),
            ..Default::default()
        });
        (texture, view)
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn glass_slab_offsets_checker() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let width = 256;
        let height = 256;
        let mut pass = Pass::new(&gpu.device, width, height);
        let mesh = Shape::RoundBox {
            half: [0.65, 0.65, 0.14],
            radius: 0.02,
        }
        .mesh(0.025);
        let handle = pass.upload_mesh(&gpu.device, &mesh).unwrap();
        let opaque = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let output = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let mut checker = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let value = if ((x / 8) + (y / 8)) % 2 == 0 {
                    0.9
                } else {
                    0.1
                };
                checker.extend([
                    half::f16::from_f32(value).to_bits(),
                    half::f16::from_f32(value).to_bits(),
                    half::f16::from_f32(value).to_bits(),
                    half::f16::from_f32(1.0).to_bits(),
                ]);
            }
        }
        gpu.upload_rgba16(&opaque, &checker).unwrap();
        let (_sky, sky_view) = solid_texture(&gpu.device, &gpu.queue, 6, true);
        let (_caustic, caustic_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_fluid, fluid_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_ripple, ripple_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_depth, depth_view) = target(
            &gpu.device,
            [width, height],
            wgpu::TextureFormat::Depth32Float,
            "glass test opaque depth",
        );
        let mut model = identity();
        let angle = 0.32_f32;
        model[0] = [angle.cos(), 0.0, angle.sin(), 0.0];
        model[2] = [-angle.sin(), 0.0, angle.cos(), 0.0];
        model[3][2] = -2.2;
        let surface = Surface {
            mesh: handle,
            model,
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
        };
        let near = 0.1;
        let far = 100.0;
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        let projection = [
            [f, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ];
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("glass slab test"),
            });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &opaque.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        {
            let _depth_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass test depth clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        pass.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            Render {
                camera: Camera {
                    view: identity(),
                    projection,
                },
                surfaces: &[surface],
                inputs: Inputs {
                    opaque_hdr_mips: &opaque.view,
                    opaque_depth: &depth_view,
                    reflection_cube: &sky_view,
                    caustic: &caustic_view,
                    caustic_projection: None,
                    fluid_height: &fluid_view,
                    ripple_height: &ripple_view,
                    mip_count: 1,
                    sky_sh: uniform_sky_sh([std::f32::consts::PI; 3]),
                },
                output: &output.view,
                profiler: Some(&mut profiler),
            },
        )
        .unwrap();
        let slot = profiler.finish(&mut encoder);
        gpu.queue.submit([encoder.finish()]);
        if let Some(slot) = slot {
            profiler.submitted(slot);
        }
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let pixels = gpu.readback_rgba16(&output).unwrap();
        let changed = pixels
            .chunks_exact(4)
            .zip(checker.chunks_exact(4))
            .filter(|(out, old)| {
                (half::f16::from_bits(out[0]).to_f32() - half::f16::from_bits(old[0]).to_f32())
                    .abs()
                    > 0.2
            })
            .count();
        assert!(changed > 100, "only {changed} checker pixels moved");
        let errors: Vec<_> = (-7..=7)
            .map(|shift| {
                let mut error = 0.0;
                for y in 96..160 {
                    for x in 96..160 {
                        let index = ((y * width + x) * 4) as usize;
                        let source = ((y * width + (x as i32 + shift) as u32) * 4) as usize;
                        let actual = half::f16::from_bits(pixels[index]).to_f32();
                        let expected = half::f16::from_bits(checker[source]).to_f32();
                        error += (actual - expected).abs();
                    }
                }
                error
            })
            .collect();
        let best = errors
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert_ne!(best.0, 7, "checker edges stayed in place");
        assert!(
            best.1 < &(errors[7] * 0.9),
            "shifted checker did not fit better: {errors:?}"
        );
        let timings = profiler.collect(&gpu.device);
        assert!(!timings.is_empty(), "GPU timestamps unavailable");
        let total: f64 = timings[0].iter().map(|pass| pass.milliseconds).sum();
        println!("glass slab 256x256: {total:.3} ms; {changed} shifted pixels");
        for timing in &timings[0] {
            println!("{}: {:.3} ms", timing.label, timing.milliseconds);
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn subsurface_is_lit_by_the_sky_at_the_front_normal() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let width = 64;
        let height = 64;
        let mut pass = Pass::new(&gpu.device, width, height);
        let mesh = Shape::RoundBox {
            half: [0.65, 0.65, 0.14],
            radius: 0.02,
        }
        .mesh(0.025);
        let handle = pass.upload_mesh(&gpu.device, &mesh).unwrap();
        let opaque = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let output = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let (_sky, sky_view) = solid_texture(&gpu.device, &gpu.queue, 6, true);
        let (_caustic, caustic_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_fluid, fluid_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_ripple, ripple_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_depth, depth_view) = target(
            &gpu.device,
            [width, height],
            wgpu::TextureFormat::Depth32Float,
            "glass subsurface opaque depth",
        );
        let mut model = identity();
        model[3][2] = -2.2;
        let mut material = pfx_materials::fixtures::glass();
        material.subsurface = 1.0e4;
        material.subsurface_tint = [0.6, 0.5, 0.9];
        material.absorption = 0.0;
        material.thin_film_amount = 0.0;
        material.transmission = 1.0;
        let surface = Surface {
            mesh: handle,
            model,
            material,
            liquid: false,
            fluid_height: 0.0,
            ripple_height: 0.0,
            caustic_strength: 0.0,
            tinted: None,
            id: 0,
            casts_shadow: false,
            shadow_only: false,
            clip: [[0.0; 4]; 2],
        };
        let mut sky_sh = uniform_sky_sh([2.0, 2.2, 2.5]);
        sky_sh[2] = [-1.2, -0.9, -0.5];
        sky_sh[3] = [0.4, 0.3, 0.2];
        let near = 0.1;
        let far = 100.0;
        let f = 1.0 / (30.0_f32.to_radians() * 0.5).tan();
        let projection = [
            [f, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ];
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass subsurface clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        pass.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            Render {
                camera: Camera {
                    view: identity(),
                    projection,
                },
                surfaces: &[surface],
                inputs: Inputs {
                    opaque_hdr_mips: &opaque.view,
                    opaque_depth: &depth_view,
                    reflection_cube: &sky_view,
                    caustic: &caustic_view,
                    caustic_projection: None,
                    fluid_height: &fluid_view,
                    ripple_height: &ripple_view,
                    mip_count: 1,
                    sky_sh,
                },
                output: &output.view,
                profiler: None,
            },
        )
        .unwrap();
        gpu.queue.submit([encoder.finish()]);
        let pixels = gpu.readback_rgba16(&output).unwrap();
        let centre = ((height / 2 * width + width / 2) * 4) as usize;
        let actual: [f32; 3] =
            std::array::from_fn(|k| half::f16::from_bits(pixels[centre + k]).to_f32());
        let scattered = subsurface_colour(material.subsurface_tint, &sky_sh, [0.0, 0.0, 1.0]);
        let ray_cos = 1.0 / (1.0 + 2.0 * (0.5 / width as f32 / f).powi(2)).sqrt();
        let f0 = ((material.ior - 1.0) / (material.ior + 1.0)).powi(2);
        let fresnel = f0 + (1.0 - f0) * (1.0 - ray_cos).powi(5);
        for k in 0..3 {
            let expected = scattered[k] * (1.0 - fresnel);
            assert!(
                (actual[k] - expected).abs() < expected * 0.01 + 2e-3,
                "{actual:?} vs {scattered:?} after Fresnel {fresnel}"
            );
        }
        let up = subsurface_colour(material.subsurface_tint, &sky_sh, [0.0, 1.0, 0.0]);
        assert!(
            (up[0] - scattered[0]).abs() > 0.05,
            "the test sky must differ by normal"
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn twenty_glass_slabs_at_4k() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let width = 3840;
        let height = 2160;
        let mut pass = Pass::new(&gpu.device, width, height);
        let mesh = Shape::RoundBox {
            half: [0.22, 0.22, 0.07],
            radius: 0.01,
        }
        .mesh(0.025);
        let handle = pass.upload_mesh(&gpu.device, &mesh).unwrap();
        let opaque = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let output = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let (_sky, sky_view) = solid_texture(&gpu.device, &gpu.queue, 6, true);
        let (_caustic, caustic_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_fluid, fluid_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_ripple, ripple_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_depth, depth_view) = target(
            &gpu.device,
            [width, height],
            wgpu::TextureFormat::Depth32Float,
            "glass budget opaque depth",
        );
        let surfaces: Vec<_> = (0..20)
            .map(|index| {
                let mut model = identity();
                let angle = 0.25_f32;
                model[0] = [angle.cos(), 0.0, angle.sin(), 0.0];
                model[2] = [-angle.sin(), 0.0, angle.cos(), 0.0];
                model[3] = [
                    ((index % 5) as f32 - 2.0) * 0.51,
                    ((index / 5) as f32 - 1.5) * 0.51,
                    -4.0,
                    1.0,
                ];
                Surface {
                    mesh: handle,
                    model,
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
                }
            })
            .collect();
        let near = 0.1;
        let far = 100.0;
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        let projection = [
            [f * height as f32 / width as f32, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ];
        let camera = Camera {
            view: identity(),
            projection,
        };
        let mut clear = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("glass budget clear"),
            });
        {
            let _pass = clear.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass budget targets"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &opaque.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &output.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }
        gpu.queue.submit([clear.finish()]);
        let inputs = || Inputs {
            opaque_hdr_mips: &opaque.view,
            opaque_depth: &depth_view,
            reflection_cube: &sky_view,
            caustic: &caustic_view,
            caustic_projection: None,
            fluid_height: &fluid_view,
            ripple_height: &ripple_view,
            mip_count: 1,
            sky_sh: uniform_sky_sh([std::f32::consts::PI; 3]),
        };
        let mut warmup = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("glass budget warmup"),
            });
        pass.encode(
            &gpu.device,
            &gpu.queue,
            &mut warmup,
            Render {
                camera,
                surfaces: &surfaces,
                inputs: inputs(),
                output: &output.view,
                profiler: None,
            },
        )
        .unwrap();
        gpu.queue.submit([warmup.finish()]);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("glass budget timing"),
            });
        let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
        for _ in 0..8 {
            pass.encode(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                Render {
                    camera,
                    surfaces: &surfaces,
                    inputs: inputs(),
                    output: &output.view,
                    profiler: Some(&mut profiler),
                },
            )
            .unwrap();
        }
        let slot = profiler.finish(&mut encoder).unwrap();
        gpu.queue.submit([encoder.finish()]);
        profiler.submitted(slot);
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let timings = profiler.collect(&gpu.device);
        assert_eq!(timings.len(), 1);
        let mut samples: Vec<_> = timings[0]
            .chunks_exact(3)
            .map(|passes| passes.iter().map(|pass| pass.milliseconds).sum::<f64>())
            .collect();
        assert_eq!(samples.len(), 8);
        samples.sort_by(f64::total_cmp);
        println!(
            "glass 4K 20 objects: median {:.3} ms, p95 {:.3} ms, max {:.3} ms",
            samples[4], samples[7], samples[7]
        );
        assert!(
            samples[4] <= 1.0,
            "glass median exceeds the 1.0 ms budget: {:.3} ms",
            samples[4]
        );
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn glass_adds_the_caustic_where_its_light_lands() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (128u32, 128u32);
        let mut pass = Pass::new(&gpu.device, width, height);
        let mesh = Shape::RoundBox {
            half: [0.65, 0.65, 0.14],
            radius: 0.02,
        }
        .mesh(0.025);
        let handle = pass.upload_mesh(&gpu.device, &mesh).unwrap();
        let opaque = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
        let grey = half::f16::from_f32(0.3).to_bits();
        let one = half::f16::from_f32(1.0).to_bits();
        gpu.upload_rgba16(
            &opaque,
            &[grey, grey, grey, one].repeat((width * height) as usize),
        )
        .unwrap();
        let caustic = gpu.offscreen(1, 1, HDR_FORMAT).unwrap();
        let light = [0.5f32, 0.25, 0.0, 1.0].map(|v| half::f16::from_f32(v).to_bits());
        gpu.upload_rgba16(&caustic, &light).unwrap();
        let (_sky, sky_view) = solid_texture(&gpu.device, &gpu.queue, 6, true);
        let (_fluid, fluid_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_ripple, ripple_view) = solid_texture(&gpu.device, &gpu.queue, 1, false);
        let (_depth, depth_view) = target(
            &gpu.device,
            [width, height],
            wgpu::TextureFormat::Depth32Float,
            "glass caustic depth",
        );
        let mut model = identity();
        model[3][2] = -2.2;
        let mut view = identity();
        view[3] = [-0.4, 0.3, -0.2, 1.0];
        let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
        let (near, far) = (0.1, 100.0);
        let projection = [
            [f, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, far * near / (near - far), 0.0],
        ];
        let mut model_world = model;
        model_world[3][0] = 0.4;
        model_world[3][1] = -0.3;
        model_world[3][2] = -2.0;
        let mut run = |strength: f32, projection_in: Option<CausticProjection>| {
            let output = gpu.offscreen(width, height, HDR_FORMAT).unwrap();
            gpu.upload_rgba16(&output, &[0, 0, 0, 0].repeat((width * height) as usize))
                .unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("glass caustic depth clear"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
            }
            let surface = Surface {
                mesh: handle,
                model: model_world,
                material: pfx_materials::fixtures::glass(),
                liquid: false,
                fluid_height: 0.0,
                ripple_height: 0.0,
                caustic_strength: strength,
                tinted: None,
                id: 0,
                casts_shadow: false,
                shadow_only: false,
                clip: [[0.0; 4]; 2],
            };
            pass.encode(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                Render {
                    camera: Camera { view, projection },
                    surfaces: &[surface],
                    inputs: Inputs {
                        opaque_hdr_mips: &opaque.view,
                        opaque_depth: &depth_view,
                        reflection_cube: &sky_view,
                        caustic: &caustic.view,
                        caustic_projection: projection_in,
                        fluid_height: &fluid_view,
                        ripple_height: &ripple_view,
                        mip_count: 1,
                        sky_sh: uniform_sky_sh([std::f32::consts::PI; 3]),
                    },
                    output: &output.view,
                    profiler: None,
                },
            )
            .unwrap();
            gpu.queue.submit([encoder.finish()]);
            let pixels = gpu.readback_rgba16(&output).unwrap();
            let index = ((height / 2 * width + width / 2) * 4) as usize;
            std::array::from_fn::<f32, 3, _>(|c| half::f16::from_bits(pixels[index + c]).to_f32())
        };
        let below = CausticProjection {
            rect: [0.2, -4.0, 0.8, 4.0],
            height: 0.0,
            toward_sun: [0.0, 1.0, 0.0],
        };
        let aside = CausticProjection {
            rect: [-0.6, -4.0, 0.5, 4.0],
            ..below
        };
        let dark = run(0.0, Some(below));
        let lit = run(1.0, Some(below));
        let missed = run(1.0, Some(aside));
        let unset = run(1.0, None);
        assert!(dark[0] > 0.05, "the glass covers the centre: {dark:?}");
        let added: [f32; 3] = std::array::from_fn(|c| lit[c] - dark[c]);
        assert!(added[0] > 0.2 && added[0] <= 0.5, "red caustic {added:?}");
        assert!(
            (added[1] / added[0] - 0.5).abs() < 0.02,
            "green caustic {added:?}"
        );
        assert!(added[2].abs() < 1e-3, "blue caustic {added:?}");
        assert_eq!(missed, dark, "a caustic outside its rectangle adds nothing");
        assert_eq!(unset, dark, "no projection, no caustic");
    }
}
