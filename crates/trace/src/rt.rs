use pfx_gpu::Gpu;
use wgpu::util::DeviceExt;

pub const RT_WGSL: &str = include_str!("rt.wgsl");

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RtGeometry {
    pub opaque: Vec<[f32; 3]>,
    pub filtered: Vec<[f32; 3]>,
    pub map: Vec<u32>,
    pub boxes: Vec<[f32; 6]>,
    pub meshes: Vec<Vec<[f32; 3]>>,
    pub instances: Vec<RtInstance>,
    pub tail: Vec<u32>,
    pub movers: Vec<[f32; 6]>,
    pub room: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RtInstance {
    pub mesh: u32,
    pub opaque: bool,
    pub transform: [f32; 12],
    pub moving: bool,
    pub sprite: bool,
    pub walked: bool,
}

pub const MAX_INSTANCES: usize = (1 << 24) - 2;

impl RtGeometry {
    pub fn new(triangles: &[[[f32; 3]; 3]], opaque: &[bool], boxes: Vec<(u32, [f32; 6])>) -> Self {
        let mut first = Vec::new();
        let mut rest = Vec::new();
        let mut opaque_slots = Vec::new();
        let mut filtered_slots = Vec::new();
        for (slot, (vertices, &solid)) in triangles.iter().zip(opaque).enumerate() {
            if solid {
                first.extend_from_slice(vertices);
                opaque_slots.push(slot as u32);
            } else {
                rest.extend_from_slice(vertices);
                filtered_slots.push(slot as u32);
            }
        }
        let leading = if opaque_slots.is_empty() {
            filtered_slots.len()
        } else {
            opaque_slots.len()
        };
        let mut map = vec![leading as u32, triangles.len() as u32, 0];
        map.extend(opaque_slots);
        map.extend(filtered_slots);
        map.extend(boxes.iter().map(|(index, _)| *index));
        map[2] = map.len() as u32;
        Self {
            opaque: first,
            filtered: rest,
            map,
            boxes: boxes.into_iter().map(|(_, bounds)| bounds).collect(),
            meshes: Vec::new(),
            instances: Vec::new(),
            tail: Vec::new(),
            movers: Vec::new(),
            room: 0,
        }
    }

    pub(crate) fn place(&mut self, levels: &crate::levels::Levels, opaque: &[bool], room: usize) {
        self.meshes = levels
            .meshes
            .iter()
            .map(|mesh| mesh.triangles.iter().flat_map(|t| t.vertices).collect())
            .collect();
        for (placed, &opaque) in levels.placed.iter().flatten().zip(opaque) {
            self.map.push(placed.base);
            self.instances.push(RtInstance {
                mesh: placed.mesh as u32,
                opaque,
                transform: crate::detail::Pose::new(placed.transform)
                    .map_or([0.0; 12], |pose| pose.rows()),
                moving: placed.moving(),
                sprite: placed.word & crate::gpu::SPRITE != 0,
                walked: placed.word & crate::gpu::SPRITE != 0 || placed.deform > 1,
            });
        }
        self.map.extend(std::iter::repeat_n(0, room));
        self.room = room;
    }

    pub fn is_empty(&self) -> bool {
        self.opaque.is_empty()
            && self.filtered.is_empty()
            && self.boxes.is_empty()
            && self.instances.is_empty()
    }
}

pub fn boxes() -> bool {
    std::env::var("PFX_TRACE_RT_BOXES").is_ok_and(|value| value == "1")
}

pub fn offered(gpu: &Gpu) -> bool {
    gpu.device
        .features()
        .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
}

pub fn wanted(gpu: &Gpu) -> bool {
    offered(gpu) && std::env::var("PFX_TRACE_RT").map_or(true, |value| value != "0")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayQueries {
    On,
    NotOffered,
    TurnedOff,
    NotUsed,
}

impl RayQueries {
    pub fn word(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::NotOffered => "not offered by the adapter",
            Self::TurnedOff => "off by PFX_TRACE_RT=0",
            Self::NotUsed => "not used for this scene",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    pub adapter: String,
    pub backend: String,
    pub driver: String,
    pub ray_queries: RayQueries,
    pub levels: bool,
    pub instances: usize,
}

impl Path {
    pub fn of(gpu: &Gpu, hardware: bool, levels: bool, instances: usize) -> Self {
        let ray_queries = if hardware {
            RayQueries::On
        } else if !offered(gpu) {
            RayQueries::NotOffered
        } else if !wanted(gpu) {
            RayQueries::TurnedOff
        } else {
            RayQueries::NotUsed
        };
        let driver = [gpu.info.driver.as_str(), gpu.info.driver_info.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            adapter: gpu.info.name.clone(),
            backend: gpu.info.backend.to_str().to_owned(),
            driver,
            ray_queries,
            levels,
            instances,
        }
    }

    pub fn hardware(&self) -> bool {
        self.ray_queries == RayQueries::On
    }

    pub fn bvh(&self) -> &'static str {
        match (self.hardware(), self.levels) {
            (true, true) => "hardware TLAS over per-mesh BLASes",
            (true, false) => "hardware TLAS over one BLAS",
            (false, true) => "software two-level",
            (false, false) => "software flat",
        }
    }

    pub fn line(&self) -> String {
        let driver = if self.driver.is_empty() {
            String::new()
        } else {
            format!(", {}", self.driver)
        };
        let instances = if self.levels {
            format!(", {} instances", self.instances)
        } else {
            String::new()
        };
        format!(
            "{} ({}{driver}), ray queries {}, BVH {}{instances}",
            self.adapter,
            self.backend,
            self.ray_queries.word(),
            self.bvh(),
        )
    }
}

pub struct Rt {
    pub tlas: wgpu::Tlas,
    pub movers_tlas: wgpu::Tlas,
    pub map: wgpu::Buffer,
    pub bytes: u64,
    first_instance: usize,
    movers_slot: usize,
    bases_at: usize,
    sprite_mesh: Option<usize>,
    sprites: Vec<bool>,
    walked: Vec<bool>,
    opaque: Vec<bool>,
    movers: [Option<(wgpu::Blas, wgpu::Buffer)>; 2],
    _blases: Vec<wgpu::Blas>,
    meshes: Vec<wgpu::Blas>,
    instance_mesh: Vec<usize>,
    _inputs: Vec<wgpu::Buffer>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listed {
    pub mask: Option<u8>,
    pub rows: Option<[f32; 12]>,
    pub custom: Option<u32>,
    pub opaque: bool,
    pub mover: bool,
}

pub const SPRITE_MASK: u8 = 1;
const BOX_MASK: u8 = 1;
const MESH_MASK: u8 = 2;
const INACTIVE: [f32; 6] = [f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0];

fn split_movers(boxes: &[[f32; 6]], moving: &[bool], walked: &[bool]) -> [Vec<[f32; 6]>; 2] {
    let kinds: Vec<bool> = moving
        .iter()
        .zip(walked)
        .filter(|(moves, _)| **moves)
        .map(|(_, walked)| *walked)
        .collect();
    [true, false].map(|wanted| {
        if !kinds.contains(&wanted) {
            return Vec::new();
        }
        boxes
            .iter()
            .zip(kinds.iter().chain(std::iter::repeat(&false)))
            .map(|(bounds, kind)| if *kind == wanted { *bounds } else { INACTIVE })
            .collect()
    })
}

fn mask(moving: bool, sprite: bool) -> u8 {
    match (moving, sprite) {
        (true, _) => 0,
        (false, true) => SPRITE_MASK,
        (false, false) => 0xff,
    }
}

fn movers_blas(
    gpu: &Gpu,
    boxes: &[[f32; 6]],
) -> Option<(
    wgpu::Blas,
    wgpu::Buffer,
    wgpu::BlasAABBGeometrySizeDescriptor,
)> {
    if boxes.is_empty() {
        return None;
    }
    let size = wgpu::BlasAABBGeometrySizeDescriptor {
        primitive_count: boxes.len() as u32,
        flags: wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION,
    };
    let buffer = input(gpu, "trace rt movers", bytemuck::cast_slice(boxes));
    let blas = gpu.device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("trace rt movers"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::AABBs {
            descriptors: vec![size.clone()],
        },
    );
    Some((blas, buffer, size))
}

const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

fn input(gpu: &Gpu, label: &'static str, bytes: &[u8]) -> wgpu::Buffer {
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::BLAS_INPUT,
        })
}

impl Rt {
    pub fn build(gpu: &Gpu, geometry: &RtGeometry) -> Self {
        let device = &gpu.device;
        let lists: Vec<(&Vec<[f32; 3]>, wgpu::AccelerationStructureGeometryFlags)> = [
            (
                &geometry.opaque,
                wgpu::AccelerationStructureGeometryFlags::OPAQUE,
            ),
            (
                &geometry.filtered,
                wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION,
            ),
        ]
        .into_iter()
        .filter(|(vertices, _)| !vertices.is_empty())
        .collect();
        let sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor> = lists
            .iter()
            .map(
                |(vertices, flags)| wgpu::BlasTriangleGeometrySizeDescriptor {
                    vertex_format: wgpu::VertexFormat::Float32x3,
                    vertex_count: vertices.len() as u32,
                    index_format: None,
                    index_count: None,
                    flags: *flags,
                },
            )
            .collect();
        let vertex_buffers: Vec<wgpu::Buffer> = lists
            .iter()
            .map(|(vertices, _)| input(gpu, "trace rt vertices", bytemuck::cast_slice(vertices)))
            .collect();
        let flags = wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE;
        let tri_blas = (!sizes.is_empty()).then(|| {
            device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("trace rt triangles"),
                    flags,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: sizes.clone(),
                },
            )
        });
        let box_size = wgpu::BlasAABBGeometrySizeDescriptor {
            primitive_count: geometry.boxes.len() as u32,
            flags: wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION,
        };
        let box_buffer = (!geometry.boxes.is_empty())
            .then(|| input(gpu, "trace rt boxes", bytemuck::cast_slice(&geometry.boxes)));
        let box_blas = box_buffer.as_ref().map(|_| {
            device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("trace rt shapes"),
                    flags,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::AABBs {
                    descriptors: vec![box_size.clone()],
                },
            )
        });
        let mut variants: Vec<(
            (u32, bool),
            wgpu::Blas,
            wgpu::BlasTriangleGeometrySizeDescriptor,
        )> = Vec::new();
        let mut mesh_buffers: Vec<Option<wgpu::Buffer>> = vec![None; geometry.meshes.len()];
        for instance in &geometry.instances {
            let key = (instance.mesh, instance.opaque);
            if variants.iter().any(|(k, _, _)| *k == key) {
                continue;
            }
            let vertices = &geometry.meshes[instance.mesh as usize];
            if mesh_buffers[instance.mesh as usize].is_none() {
                mesh_buffers[instance.mesh as usize] = Some(input(
                    gpu,
                    "trace rt mesh vertices",
                    bytemuck::cast_slice(vertices),
                ));
            }
            let size = wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: vertices.len() as u32,
                index_format: None,
                index_count: None,
                flags: if instance.opaque {
                    wgpu::AccelerationStructureGeometryFlags::OPAQUE
                } else {
                    wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION
                },
            };
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("trace rt mesh"),
                    flags,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size.clone()],
                },
            );
            variants.push((key, blas, size));
        }
        let leading = [&tri_blas, &box_blas].into_iter().flatten().count();
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("trace rt world"),
            max_instances: (leading + geometry.instances.len() + geometry.room + 1) as u32,
            flags,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        for (slot, blas) in [&tri_blas, &box_blas].into_iter().flatten().enumerate() {
            *tlas.get_mut_single(slot).unwrap() =
                Some(wgpu::TlasInstance::new(blas, IDENTITY, slot as u32, 0xff));
        }
        for (k, instance) in geometry.instances.iter().enumerate() {
            let (_, blas, _) = variants
                .iter()
                .find(|(key, _, _)| *key == (instance.mesh, instance.opaque))
                .expect("every instance has its variant");
            *tlas.get_mut_single(leading + k).unwrap() = Some(wgpu::TlasInstance::new(
                blas,
                instance.transform,
                k as u32 + 1,
                mask(instance.moving, instance.sprite),
            ));
        }
        let moving: Vec<bool> = geometry.instances.iter().map(|i| i.moving).collect();
        let walked: Vec<bool> = geometry.instances.iter().map(|i| i.walked).collect();
        let [walked_boxes, hardware_boxes] = split_movers(&geometry.movers, &moving, &walked);
        let world_movers = movers_blas(gpu, &walked_boxes);
        let movers_slot = leading + geometry.instances.len() + geometry.room;
        if let Some((blas, _, _)) = &world_movers {
            *tlas.get_mut_single(movers_slot).unwrap() = Some(wgpu::TlasInstance::new(
                blas,
                IDENTITY,
                crate::moving::MOVERS,
                0xff,
            ));
        }
        let movers = movers_blas(gpu, &hardware_boxes);
        let mut movers_tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("trace rt movers"),
            max_instances: (geometry.instances.len() + geometry.room + 1) as u32,
            flags,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        *movers_tlas.get_mut_single(0).unwrap() = movers.as_ref().map(|(blas, _, _)| {
            wgpu::TlasInstance::new(blas, IDENTITY, crate::moving::MOVERS, BOX_MASK)
        });
        let instance_mesh: Vec<usize> = geometry
            .instances
            .iter()
            .map(|instance| {
                variants
                    .iter()
                    .position(|(key, _, _)| *key == (instance.mesh, instance.opaque))
                    .expect("every instance has its variant")
            })
            .collect();
        for (k, instance) in geometry.instances.iter().enumerate() {
            if instance.moving && !instance.walked {
                *movers_tlas.get_mut_single(k + 1).unwrap() = Some(wgpu::TlasInstance::new(
                    &variants[instance_mesh[k]].1,
                    instance.transform,
                    k as u32 + 1,
                    MESH_MASK,
                ));
            }
        }
        let mut entries = Vec::new();
        if let Some(blas) = &tri_blas {
            entries.push(wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(
                    sizes
                        .iter()
                        .zip(&vertex_buffers)
                        .map(|(size, buffer)| wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer: buffer,
                            first_vertex: 0,
                            vertex_stride: 12,
                            index_buffer: None,
                            first_index: None,
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        })
                        .collect(),
                ),
            });
        }
        if let (Some(blas), Some(buffer)) = (&box_blas, &box_buffer) {
            entries.push(wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry {
                    size: &box_size,
                    stride: 24,
                    aabb_buffer: buffer,
                    primitive_offset: 0,
                }]),
            });
        }
        for (blas, buffer, size) in world_movers.iter().chain(&movers) {
            entries.push(wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry {
                    size,
                    stride: 24,
                    aabb_buffer: buffer,
                    primitive_offset: 0,
                }]),
            });
        }
        for ((mesh, _), blas, size) in &variants {
            let buffer = mesh_buffers[*mesh as usize]
                .as_ref()
                .expect("every variant has its vertices");
            entries.push(wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size,
                        vertex_buffer: buffer,
                        first_vertex: 0,
                        vertex_stride: 12,
                        index_buffer: None,
                        first_index: None,
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            });
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("trace rt build"),
        });
        encoder.build_acceleration_structures(entries.iter(), [&tlas, &movers_tlas]);
        gpu.queue.submit([encoder.finish()]);
        let mut words = geometry.map.clone();
        words.extend_from_slice(&geometry.tail);
        let map = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("trace rt map"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let mut inputs = vertex_buffers;
        inputs.extend(box_buffer);
        inputs.extend(mesh_buffers.into_iter().flatten());
        let blases: Vec<wgpu::Blas> = [tri_blas, box_blas].into_iter().flatten().collect();
        let meshes: Vec<wgpu::Blas> = variants.into_iter().map(|(_, blas, _)| blas).collect();
        let bytes = inputs.iter().map(wgpu::Buffer::size).sum::<u64>()
            + map.size()
            + 3 * inputs.iter().map(wgpu::Buffer::size).sum::<u64>() / 2
            + 64 * (leading + 2 * (geometry.instances.len() + geometry.room) + 1) as u64;
        Self {
            tlas,
            movers_tlas,
            map,
            bytes,
            first_instance: leading,
            movers_slot,
            bases_at: geometry.map[2] as usize,
            sprite_mesh: geometry
                .instances
                .iter()
                .position(|instance| instance.sprite)
                .map(|k| instance_mesh[k]),
            sprites: geometry.instances.iter().map(|i| i.sprite).collect(),
            walked,
            opaque: geometry.instances.iter().map(|i| i.opaque).collect(),
            movers: [world_movers, movers]
                .map(|built| built.map(|(blas, buffer, _)| (blas, buffer))),
            _blases: blases,
            meshes,
            instance_mesh,
            _inputs: inputs,
        }
    }

    pub fn listed(&self, k: usize) -> Listed {
        let tlas = self.tlas.get();
        let instance = tlas.get(self.first_instance + k).and_then(Option::as_ref);
        Listed {
            mask: instance.map(|instance| instance.mask),
            rows: instance.map(|instance| instance.transform),
            custom: instance.map(|instance| instance.custom_data),
            opaque: self.opaque.get(k).copied().unwrap_or(false),
            mover: self
                .movers_tlas
                .get()
                .get(k + 1)
                .is_some_and(Option::is_some),
        }
    }

    fn place_movers(&mut self, moving: &[bool]) {
        for (k, &moves) in moving.iter().enumerate() {
            let walked = self.walked.get(k).copied().unwrap_or(true);
            let transform = self
                .tlas
                .get_mut_single(self.first_instance + k)
                .and_then(|instance| instance.as_ref().map(|instance| instance.transform));
            let placed = match (moves && !walked, transform, self.instance_mesh.get(k)) {
                (true, Some(transform), Some(&mesh)) => Some(wgpu::TlasInstance::new(
                    &self.meshes[mesh],
                    transform,
                    k as u32 + 1,
                    MESH_MASK,
                )),
                _ => None,
            };
            if let Some(slot) = self.movers_tlas.get_mut_single(k + 1) {
                *slot = placed;
            }
        }
    }

    pub(crate) fn set_movers(&mut self, gpu: &Gpu, moving: &[bool], boxes: &[[f32; 6]]) {
        for (k, &moves) in moving.iter().enumerate() {
            if let Some(Some(instance)) = self.tlas.get_mut_single(self.first_instance + k) {
                instance.mask = mask(moves, self.sprites.get(k).copied().unwrap_or(false));
            }
        }
        let [walked_boxes, hardware_boxes] = split_movers(boxes, moving, &self.walked);
        let built = [
            movers_blas(gpu, &walked_boxes),
            movers_blas(gpu, &hardware_boxes),
        ];
        *self.tlas.get_mut_single(self.movers_slot).unwrap() =
            built[0].as_ref().map(|(blas, _, _)| {
                wgpu::TlasInstance::new(blas, IDENTITY, crate::moving::MOVERS, 0xff)
            });
        *self.movers_tlas.get_mut_single(0).unwrap() = built[1].as_ref().map(|(blas, _, _)| {
            wgpu::TlasInstance::new(blas, IDENTITY, crate::moving::MOVERS, BOX_MASK)
        });
        self.place_movers(moving);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("trace rt movers"),
            });
        let entries: Vec<wgpu::BlasBuildEntry<'_>> = built
            .iter()
            .flatten()
            .map(|(blas, buffer, size)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::AabbGeometries(vec![wgpu::BlasAabbGeometry {
                    size,
                    stride: 24,
                    aabb_buffer: buffer,
                    primitive_offset: 0,
                }]),
            })
            .collect();
        encoder.build_acceleration_structures(entries.iter(), [&self.tlas, &self.movers_tlas]);
        gpu.queue.submit([encoder.finish()]);
        self.movers = built.map(|built| built.map(|(blas, buffer, _)| (blas, buffer)));
    }

    pub(crate) fn set_sprites(
        &mut self,
        gpu: &Gpu,
        first: usize,
        sprites: &[([f32; 12], bool)],
        base: u32,
    ) -> Result<(), String> {
        let Some(variant) = self.sprite_mesh else {
            return Err("this trace was staged without sprites".into());
        };
        let room = self
            .movers_slot
            .checked_sub(self.first_instance + first)
            .ok_or("the sprites start past the trace's instances")?;
        if sprites.len() > room {
            return Err(format!(
                "this trace has room for {room} sprites and {} were given",
                sprites.len()
            ));
        }
        for j in 0..room {
            let k = first + j;
            *self.tlas.get_mut_single(self.first_instance + k).unwrap() =
                sprites.get(j).map(|&(transform, moving)| {
                    wgpu::TlasInstance::new(
                        &self.meshes[variant],
                        transform,
                        k as u32 + 1,
                        mask(moving, true),
                    )
                });
            if let Some(slot) = self.movers_tlas.get_mut_single(k + 1) {
                *slot = None;
            }
        }
        let count = first + sprites.len();
        self.sprites.truncate(first);
        self.sprites.resize(count, true);
        self.walked.truncate(first);
        self.walked.resize(count, true);
        self.opaque.truncate(first);
        self.opaque.resize(count, false);
        self.instance_mesh.truncate(first);
        self.instance_mesh.resize(count, variant);
        if !sprites.is_empty() {
            gpu.queue.write_buffer(
                &self.map,
                4 * (self.bases_at + first) as u64,
                bytemuck::cast_slice(&vec![base; sprites.len()]),
            );
        }
        Ok(())
    }

    pub(crate) fn build_top(&self, gpu: &Gpu) {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("trace rt top"),
            });
        encoder.build_acceleration_structures(std::iter::empty(), [&self.tlas, &self.movers_tlas]);
        gpu.queue.submit([encoder.finish()]);
    }

    pub(crate) fn set_transforms(&mut self, transforms: &[[f32; 12]]) {
        for (k, transform) in transforms.iter().enumerate() {
            if let Some(Some(instance)) = self.tlas.get_mut_single(self.first_instance + k) {
                instance.transform = *transform;
            }
            if let Some(Some(instance)) = self.movers_tlas.get_mut_single(k + 1) {
                instance.transform = *transform;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_leads_with_the_first_geometry_then_slots_then_shapes() {
        let tri = |x: f32| [[x, 0.0, 0.0], [x, 1.0, 0.0], [x, 0.0, 1.0]];
        let geometry = RtGeometry::new(
            &[tri(0.0), tri(1.0), tri(2.0)],
            &[false, true, false],
            vec![(4, [0.0; 6])],
        );
        assert_eq!(geometry.map, vec![1, 3, 7, 1, 0, 2, 4]);
        assert_eq!(geometry.opaque.len(), 3);
        assert_eq!(geometry.filtered.len(), 6);
        let none_solid = RtGeometry::new(&[tri(0.0), tri(1.0)], &[false, false], Vec::new());
        assert_eq!(none_solid.map, vec![2, 2, 5, 0, 1]);
    }

    #[test]
    fn the_ray_query_shader_validates() {
        for lit in [false, true] {
            let source = crate::gpu::shader_rt(lit);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
