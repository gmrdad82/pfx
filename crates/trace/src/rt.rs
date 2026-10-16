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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RtInstance {
    pub mesh: u32,
    pub opaque: bool,
    pub transform: [f32; 12],
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
        }
    }

    pub(crate) fn place(&mut self, levels: &crate::levels::Levels, solid: &[bool]) {
        self.meshes = levels
            .meshes
            .iter()
            .map(|mesh| mesh.triangles.iter().flat_map(|t| t.vertices).collect())
            .collect();
        for placed in levels.placed.iter().flatten() {
            let len = levels.meshes[placed.mesh].triangles.len();
            let base = placed.base as usize;
            self.map.push(placed.base);
            self.instances.push(RtInstance {
                mesh: placed.mesh as u32,
                opaque: solid[base..base + len].iter().all(|&solid| solid),
                transform: crate::detail::Pose::new(placed.transform)
                    .map_or([0.0; 12], |pose| pose.rows()),
            });
        }
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

pub struct Rt {
    pub tlas: wgpu::Tlas,
    pub map: wgpu::Buffer,
    first_instance: usize,
    _blases: Vec<wgpu::Blas>,
    _inputs: Vec<wgpu::Buffer>,
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
            max_instances: (leading + geometry.instances.len()).max(1) as u32,
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
                0xff,
            ));
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
        encoder.build_acceleration_structures(entries.iter(), [&tlas]);
        gpu.queue.submit([encoder.finish()]);
        let map = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("trace rt map"),
            contents: bytemuck::cast_slice(&geometry.map),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut inputs = vertex_buffers;
        inputs.extend(box_buffer);
        inputs.extend(mesh_buffers.into_iter().flatten());
        let mut blases: Vec<wgpu::Blas> = [tri_blas, box_blas].into_iter().flatten().collect();
        blases.extend(variants.into_iter().map(|(_, blas, _)| blas));
        Self {
            tlas,
            map,
            first_instance: leading,
            _blases: blases,
            _inputs: inputs,
        }
    }

    pub(crate) fn set_transforms(&mut self, gpu: &Gpu, transforms: &[[f32; 12]]) {
        for (k, transform) in transforms.iter().enumerate() {
            if let Some(Some(instance)) = self.tlas.get_mut_single(self.first_instance + k) {
                instance.transform = *transform;
            }
        }
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("trace rt top"),
            });
        encoder.build_acceleration_structures(std::iter::empty(), [&self.tlas]);
        gpu.queue.submit([encoder.finish()]);
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
