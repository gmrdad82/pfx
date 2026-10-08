use crate::{
    BEND_WGSL, FIXED_DT, Fluid, FluidDesc, Look, Pour, Ripple, RippleGpu, SUBSTEP, Target,
};

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn fluid_heightfield_on_gpu() {
    let gpu = open();
    let desc = FluidDesc {
        width: 64,
        height: 64,
        max_particles: 512,
        max_targets: 4,
        params: crate::Params::default(),
        settle: FluidDesc::default().settle,
    };
    let mut fluid = Fluid::new(&gpu.device, desc);
    let look = Look {
        tall: 1.0,
        active: 0.3,
        weight_b: 0.0,
        weight_a: 0.2,
    };
    let pour = Pour {
        key: "bead".into(),
        target: Target::bead([32.0, 32.0], 8.0, &look),
        from: None,
        fill: None,
    };
    let dt = SUBSTEP * 4.0;
    fluid.sync(&gpu.queue, &[pour], dt);
    let spawned = fluid.particle_count();
    assert!(spawned > 4, "{spawned}");
    for frame in 0..30 {
        fluid.step(&gpu.device, &gpu.queue, dt, frame as f32 * dt, 1.0, true);
    }
    let bytes = read_buffer(&gpu, fluid.particles_buffer());
    let particles: &[crate::Particle] = bytemuck::cast_slice(&bytes);
    let alive = particles
        .iter()
        .take(spawned as usize)
        .filter(|particle| {
            particle.born > 0.0 && particle.pos[0].is_finite() && particle.pos[1].is_finite()
        })
        .count();
    assert_eq!(alive, spawned as usize);
    let height = read_height(&gpu, fluid.height_texture());
    let sum: f32 = height.iter().copied().filter(|v| v.is_finite()).sum();
    assert!(sum > 0.0, "{sum}");
    assert!(height.iter().all(|v| v.is_finite()));

    let mut cpu = Ripple::new(16, 16);
    let splash = crate::Drop {
        uv: [0.5, 0.5],
        radius: 0.12,
        amount: 0.8,
    };
    cpu.step(FIXED_DT, &[splash]);
    let mut gpu_ripple = RippleGpu::new(&gpu.device, &gpu.queue, 16, 16);
    gpu_ripple.step(&gpu.device, &gpu.queue, FIXED_DT, &[splash]);
    let image = read_height(&gpu, gpu_ripple.texture());
    let centre = image[8 * 16 + 8];
    let cpu_centre = cpu.height_at(8, 8);
    assert!(
        (centre - cpu_centre).abs() < 0.03,
        "gpu {centre} cpu {cpu_centre}"
    );

    let mut cpu_two = Ripple::new(16, 16);
    cpu_two.step(2.0 * FIXED_DT, &[splash]);
    let mut gpu_two = RippleGpu::new(&gpu.device, &gpu.queue, 16, 16);
    gpu_two.step(&gpu.device, &gpu.queue, 2.0 * FIXED_DT, &[splash]);
    let two = read_height(&gpu, gpu_two.texture());
    let cpu_two_centre = cpu_two.height_at(8, 8);
    assert!(cpu_two_centre.abs() > 0.01, "cpu {cpu_two_centre}");
    assert!(
        (two[8 * 16 + 8] - cpu_two_centre).abs() < 0.03,
        "two steps in one frame: gpu {} cpu {cpu_two_centre}",
        two[8 * 16 + 8]
    );

    let source = format!(
        "{BEND_WGSL}\n@group(0) @binding(0) var<storage, read_write> out: array<f32>;\n@compute @workgroup_size(1)\nfn main() {{\n  let bent = bend(vec3f(0.4, 0.2, -0.1), 3u, 1.2, 1.0, vec3f(1.0, 0.0, 0.2), 3.0);\n  out[0] = bent.x + bent.y + bent.z;\n}}\n"
    );
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Bend"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bend"),
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
    let _pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Bend"),
            layout: Some(
                &gpu.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("Bend"),
                        bind_group_layouts: &[&layout],
                        push_constant_ranges: &[],
                    }),
            ),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
}

struct Gpu {
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn open() -> Gpu {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("gpu adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("physics"),
        required_features: wgpu::Features::empty(),
        ..Default::default()
    }))
    .expect("gpu device");
    Gpu {
        _instance: instance,
        _adapter: adapter,
        device,
        queue,
    }
}

fn read_buffer(gpu: &Gpu, buffer: &wgpu::Buffer) -> Vec<u8> {
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    gpu.queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (send, recv) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    recv.recv().expect("map").expect("map buffer");
    slice.get_mapped_range().to_vec()
}

fn read_height(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<f32> {
    let size = texture.size();
    let padded = (size.width * 8).div_ceil(256) * 256;
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("height readback"),
        size: (padded * size.height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (send, recv) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    recv.recv().expect("map").expect("map height");
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((size.width * size.height) as usize);
    for y in 0..size.height as usize {
        for x in 0..size.width as usize {
            let at = y * padded as usize + x * 8;
            let bits = u16::from_le_bytes([data[at], data[at + 1]]);
            out.push(f16_to_f32(bits));
        }
    }
    out
}

fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign << 31
        } else {
            let mut frac = man;
            let mut exponent = 127 - 15 + 1;
            while frac & 0x400 == 0 {
                frac <<= 1;
                exponent -= 1;
            }
            frac &= 0x3ff;
            (sign << 31) | (exponent << 23) | (frac << 13)
        }
    } else if exp == 31 {
        (sign << 31) | (255 << 23) | (man << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}
