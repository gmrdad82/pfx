use pfx_gpu::Gpu;

use crate::gpu::{Trace, texture};

pub(crate) struct Parts {
    pub buffers: [(u32, wgpu::Buffer); 9],
    pub lit: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub depth: Vec<f32>,
    pub distance: Vec<f32>,
    pub id: Vec<u32>,
    pub normal: Vec<[f32; 3]>,
}

impl Probe {
    pub fn without(&self, ids: &[u32]) -> Vec<u32> {
        self.id
            .iter()
            .map(|&id| if ids.contains(&id) { 0 } else { id })
            .collect()
    }

    pub fn material_id(material: u32) -> u32 {
        material + 1
    }
}

pub fn shader(lit: bool) -> String {
    format!(
        "{}\n{}",
        crate::gpu::shader(lit),
        include_str!("probe.wgsl")
    )
}

impl Trace {
    pub fn probe(&self, gpu: &Gpu) -> Result<Probe, String> {
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("trace probe shader"),
                source: wgpu::ShaderSource::Wgsl(shader(self.probe_parts.lit).into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("trace probe pipeline"),
                layout: None,
                module: &module,
                entry_point: Some("probe"),
                compilation_options: Default::default(),
                cache: None,
            });
        let depth = texture(gpu, self.width, self.height, "trace probe depth");
        let normal = texture(gpu, self.width, self.height, "trace probe normal");
        let mut frame = self.base;
        frame.size[2] = 0;
        frame.size[3] = 1;
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace probe frame"),
            size: std::mem::size_of_val(&frame) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue
            .write_buffer(&uniform, 0, bytemuck::bytes_of(&frame));
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 17,
                resource: wgpu::BindingResource::TextureView(&depth.view),
            },
            wgpu::BindGroupEntry {
                binding: 18,
                resource: wgpu::BindingResource::TextureView(&normal.view),
            },
        ];
        entries.extend(self.probe_parts.buffers.iter().map(|(binding, buffer)| {
            wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            }
        }));
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace probe bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("trace probe"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("trace probe"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        }
        gpu.queue.submit(Some(encoder.finish()));
        let depth = texels(&gpu.readback_bytes(&depth)?);
        let normal = texels(&gpu.readback_bytes(&normal)?);
        Ok(Probe {
            width: self.width,
            height: self.height,
            depth: depth.iter().map(|texel| texel[0]).collect(),
            distance: depth.iter().map(|texel| texel[2]).collect(),
            id: depth.iter().map(|texel| texel[1].round() as u32).collect(),
            normal: normal
                .iter()
                .map(|texel| [texel[0], texel[1], texel[2]])
                .collect(),
        })
    }
}

pub fn texels(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytes
        .chunks_exact(16)
        .map(|chunk| {
            std::array::from_fn(|k| f32::from_le_bytes(chunk[k * 4..k * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_shader_validates() {
        for lit in [false, true] {
            let source = shader(lit);
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    #[test]
    fn ids_drop_what_is_not_outlined() {
        let probe = Probe {
            width: 3,
            height: 1,
            depth: vec![1.0; 3],
            distance: vec![1.0; 3],
            id: vec![0, 1, 3],
            normal: vec![[0.0, 0.0, 1.0]; 3],
        };
        assert_eq!(probe.without(&[Probe::material_id(2)]), vec![0, 1, 0]);
        assert_eq!(texels(&[0u8; 32]).len(), 2);
    }
}
