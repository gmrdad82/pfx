use pfx_gpu::Gpu;
use wgpu::util::DeviceExt;

pub const RT_WGSL: &str = include_str!("rt.wgsl");

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RtGeometry {
    pub opaque: Vec<[f32; 3]>,
    pub filtered: Vec<[f32; 3]>,
    pub map: Vec<u32>,
    pub boxes: Vec<[f32; 6]>,
}

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
        let mut map = vec![leading as u32, triangles.len() as u32];
        map.extend(opaque_slots);
        map.extend(filtered_slots);
        map.extend(boxes.iter().map(|(index, _)| *index));
        Self {
            opaque: first,
            filtered: rest,
            map,
            boxes: boxes.into_iter().map(|(_, bounds)| bounds).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.opaque.is_empty() && self.filtered.is_empty() && self.boxes.is_empty()
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
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("trace rt world"),
            max_instances: 2,
            flags,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        for (slot, blas) in [&tri_blas, &box_blas].into_iter().flatten().enumerate() {
            *tlas.get_mut_single(slot).unwrap() =
                Some(wgpu::TlasInstance::new(blas, IDENTITY, slot as u32, 0xff));
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
        Self {
            tlas,
            map,
            _blases: [tri_blas, box_blas].into_iter().flatten().collect(),
            _inputs: inputs,
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
        assert_eq!(geometry.map, vec![1, 3, 1, 0, 2, 4]);
        assert_eq!(geometry.opaque.len(), 3);
        assert_eq!(geometry.filtered.len(), 6);
        let none_solid = RtGeometry::new(&[tri(0.0), tri(1.0)], &[false, false], Vec::new());
        assert_eq!(none_solid.map, vec![2, 2, 0, 1]);
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
