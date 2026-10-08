use super::inspect::mapped;
use super::*;
use half::f16;
use pfx_gpu::Gpu;

const SIZE: usize = 96;
const STRIDES: [f32; 10] = [256.0, 128.0, 64.0, 32.0, 16.0, 8.0, 4.0, 2.0, 1.0, 1.0];

type Field = Vec<[f32; 4]>;

fn pool_look() -> Look {
    Look {
        tall: 30.0,
        active: 0.3,
        weight_b: 0.0,
        weight_a: 0.2,
    }
}

fn pool_pour() -> Pour {
    Pour {
        key: "pool".into(),
        target: Target::bead([48.0, 48.0], 24.0, &pool_look()),
        from: None,
        fill: None,
    }
}

fn small(width: u32, settle: f32) -> FluidDesc {
    FluidDesc {
        width,
        height: width,
        max_particles: 1024,
        max_targets: 4,
        params: Params::default(),
        settle,
    }
}

fn open() -> Gpu {
    pollster::block_on(Gpu::headless()).expect("gpu")
}

fn read_texels(gpu: &Gpu, texture: &wgpu::Texture) -> Field {
    let size = texture.size();
    let padded = (size.width * 8).div_ceil(256) * 256;
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texel readback"),
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
    let data = mapped(&gpu.device, &staging);
    let mut out = Vec::new();
    for y in 0..size.height as usize {
        for x in 0..size.width as usize {
            let at = y * padded as usize + x * 8;
            out.push(std::array::from_fn(|c| {
                f16::from_bits(u16::from_le_bytes([data[at + c * 2], data[at + c * 2 + 1]]))
                    .to_f32()
            }));
        }
    }
    out
}

fn read_particles(gpu: &Gpu, fluid: &Fluid) -> Vec<Particle> {
    let size = fluid.particle_count() as u64 * std::mem::size_of::<Particle>() as u64;
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(fluid.particles_buffer(), 0, &staging, 0, size);
    gpu.queue.submit(Some(encoder.finish()));
    bytemuck::cast_slice(&mapped(&gpu.device, &staging)).to_vec()
}

fn rounded(value: [f32; 4]) -> [f32; 4] {
    value.map(|v| f16::from_f32(v).to_f32())
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn gather(particles: &[Particle], look: [f32; 4], params: &Params) -> Field {
    let reach = KERNEL * params.gather.clamp(1.0, 2.0);
    let keep = KERNEL * KERNEL / (reach * reach);
    let rest = rest_density();
    let mut field = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let at = [x as f32 + 0.5, y as f32 + 0.5];
            let mut sums = [0.0f32; 4];
            let mut weight = 0.0f32;
            for particle in particles {
                if particle.born < 0.0 || particle.life <= 1e-3 {
                    continue;
                }
                let d = sub2(at, particle.pos);
                let r2 = dot2(d, d);
                let wide = reach * reach;
                if r2 >= wide {
                    continue;
                }
                let e = wide - r2;
                let w =
                    particle.life * 4.0 / (std::f32::consts::PI * reach.powi(8)) * e * e * e * keep;
                if w <= 0.0 {
                    continue;
                }
                for k in 0..4 {
                    sums[k] += w * look[k];
                }
                weight += w;
            }
            let h = sums[0] / rest;
            let inv = h / weight.max(1e-6);
            field.push(rounded([h, sums[1] * inv, sums[2] * inv, sums[3] * inv]));
        }
    }
    field
}

fn blur(source: &Field, vertical: bool, sigma: f32, threshold: f32) -> Field {
    let sigma = sigma.max(0.5);
    let reach = (sigma * 2.5).ceil() as i32;
    let mut out = Vec::new();
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let mut sum = [0.0f32; 4];
            let mut total = 0.0;
            for k in -reach..=reach {
                let c = if vertical {
                    [x, (y + k).clamp(0, SIZE as i32 - 1)]
                } else {
                    [(x + k).clamp(0, SIZE as i32 - 1), y]
                };
                let w = (-((k * k) as f32) / (2.0 * sigma * sigma)).exp();
                let mut v = source[c[1] as usize * SIZE + c[0] as usize];
                if threshold > 0.0 {
                    let ratio = v[3] / v[0].max(1e-3);
                    let t = threshold + (1.5 - threshold) * smooth(1.5, 2.2, ratio);
                    v = [smooth(t * 0.5, t * 1.5, v[0]), v[0], v[3], 0.0];
                }
                for i in 0..4 {
                    sum[i] += v[i] * w;
                }
                total += w;
            }
            out.push(rounded(sum.map(|v| v / total)));
        }
    }
    out
}

fn level(mask: &Field, x: i32, y: i32) -> f32 {
    let c = [x.clamp(0, SIZE as i32 - 1), y.clamp(0, SIZE as i32 - 1)];
    mask[c[1] as usize * SIZE + c[0] as usize][0]
}

fn flood_init(mask: &Field) -> Field {
    let mut out = Vec::new();
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let c = level(mask, x, y);
            let inside = c >= 0.5;
            let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| (level(mask, x + dx, y + dy) >= 0.5) != inside);
            let grad = [
                (level(mask, x + 1, y) - level(mask, x - 1, y)) * 0.5,
                (level(mask, x, y + 1) - level(mask, x, y - 1)) * 0.5,
            ];
            let slope = len2(grad);
            let mut shift = [0.0, 0.0];
            if slope > 1e-4 {
                shift = scale2(grad, 1.0 / slope * ((0.5 - c) / slope).clamp(-1.5, 1.5));
            }
            let offset = if edge { shift } else { [60000.0, 60000.0] };
            out.push(rounded([
                offset[0],
                offset[1],
                if inside { 1.0 } else { 0.0 },
                0.0,
            ]));
        }
    }
    out
}

fn flood_step(source: &Field, stride: i32) -> Field {
    let mut out = Vec::new();
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let own = source[y as usize * SIZE + x as usize];
            let mut best = [own[0], own[1]];
            let mut near = if own[0] < 30000.0 { len2(best) } else { 1e9 };
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let step = [dx * stride, dy * stride];
                    let q = [x + step[0], y + step[1]];
                    if q[0] < 0 || q[1] < 0 || q[0] >= SIZE as i32 || q[1] >= SIZE as i32 {
                        continue;
                    }
                    let s = source[q[1] as usize * SIZE + q[0] as usize];
                    if s[0] > 30000.0 {
                        continue;
                    }
                    let offset = [s[0] + step[0] as f32, s[1] + step[1] as f32];
                    let d = len2(offset);
                    if d < near {
                        near = d;
                        best = offset;
                    }
                }
            }
            out.push(rounded([best[0], best[1], own[2], 0.0]));
        }
    }
    out
}

fn reference_edge(particles: &[Particle], look: [f32; 4], params: &Params) -> Field {
    let smoothing = Smoothing::default();
    let outline = Outline::default();
    let raw = gather(particles, look, params);
    let raw = blur(&raw, false, smoothing.fine, 0.0);
    let raw = blur(&raw, true, smoothing.fine, 0.0);
    let across = blur(&raw, false, outline.shape[0], outline.shape[3]);
    let mask = blur(&across, true, outline.shape[0], 0.0);
    let mut seeds = flood_init(&mask);
    for stride in STRIDES {
        seeds = flood_step(&seeds, stride as i32);
    }
    mask.iter()
        .zip(&seeds)
        .map(|(m, s)| {
            let d = if s[0] < 30000.0 {
                len2([s[0], s[1]]).min(300.0)
            } else {
                300.0
            };
            rounded([m[0], m[1], m[2], if s[2] > 0.5 { d } else { -d }])
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn edge_view_has_the_table_size_and_format() {
    let gpu = open();
    let fluid = Fluid::new(&gpu.device, small(48, SETTLE));
    let texture = fluid.edge_texture();
    assert_eq!(texture.width(), 48);
    assert_eq!(texture.height(), 48);
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
    assert_eq!(texture.dimension(), wgpu::TextureDimension::D2);
    assert!(
        texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    );
    let desk = Fluid::new(&gpu.device, FluidDesc::new(1536, 864));
    assert_eq!(desk.edge_texture().width(), 1536);
    assert_eq!(desk.edge_texture().height(), 864);
    let _view: &wgpu::TextureView = desk.edge_view();
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn edge_view_matches_a_cpu_jump_flood_of_the_particle_field() {
    let gpu = open();
    let params = Params::default();
    let mut fluid = Fluid::new(&gpu.device, small(SIZE as u32, SETTLE));
    let pour = pool_pour();
    let look = pour.target.look;
    let dt = SUBSTEP * 4.0;
    fluid.sync(&gpu.queue, &[pour], dt);
    fluid.set_wires(&[], Wire::default());
    fluid.set_smoothing(Smoothing::default());
    fluid.set_outline(Outline::default());
    for frame in 0..40 {
        fluid.step(&gpu.device, &gpu.queue, dt, frame as f32 * dt, 1.0, true);
    }
    let particles = read_particles(&gpu, &fluid);
    assert!(particles.len() > 100, "{}", particles.len());
    let expected = reference_edge(&particles, look, &params);
    let actual = read_texels(&gpu, fluid.edge_texture());
    assert_eq!(actual.len(), expected.len());
    let mut close = 0;
    let mut worst = 0.0f32;
    let mut mask_error = 0.0f32;
    let mut inside = 0;
    let mut outside = 0;
    for (a, e) in actual.iter().zip(&expected) {
        assert!(a.iter().all(|v| v.is_finite()));
        let gap = (a[3] - e[3]).abs();
        worst = worst.max(gap);
        if gap <= 0.15 {
            close += 1;
        }
        mask_error = mask_error.max((a[0] - e[0]).abs());
        if a[3] > 0.0 {
            inside += 1;
        } else {
            outside += 1;
        }
    }
    assert!(inside > 200 && outside > 200, "{inside} {outside}");
    let fraction = close as f32 / actual.len() as f32;
    assert!(
        fraction > 0.99,
        "close {fraction} worst {worst} mask {mask_error}"
    );
    assert!(mask_error < 0.01, "mask {mask_error}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_profiler_receives_the_five_fluid_labels() {
    let gpu = open();
    let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
    let mut fluid = Fluid::new(&gpu.device, small(SIZE as u32, SETTLE));
    let dt = SUBSTEP * 4.0;
    fluid.sync(&gpu.queue, &[pool_pour()], dt);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    fluid.encode(
        &gpu.queue,
        &mut encoder,
        dt,
        0.0,
        1.0,
        true,
        Some(&mut profiler),
    );
    let slot = profiler.finish(&mut encoder);
    gpu.queue.submit(Some(encoder.finish()));
    let Some(slot) = slot else {
        return;
    };
    profiler.submitted(slot);
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let frames = profiler.collect(&gpu.device);
    assert_eq!(frames.len(), 1);
    let labels: Vec<&str> = frames[0].iter().map(|t| t.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "fluid sim",
            "fluid gather",
            "fluid blur",
            "fluid edges",
            "fluid raise"
        ]
    );
    assert!(frames[0].iter().all(|t| t.milliseconds.is_finite()));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_profiler_stays_empty_while_the_fluid_rests() {
    let gpu = open();
    let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
    let mut fluid = Fluid::new(&gpu.device, small(SIZE as u32, 0.0));
    let dt = SUBSTEP * 4.0;
    fluid.sync(&gpu.queue, &[pool_pour()], dt);
    fluid.sync(&gpu.queue, &[pool_pour()], 1.0);
    fluid.step(&gpu.device, &gpu.queue, dt, 0.0, 1.0, true);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    fluid.encode(
        &gpu.queue,
        &mut encoder,
        dt,
        0.0,
        1.0,
        true,
        Some(&mut profiler),
    );
    assert!(profiler.finish(&mut encoder).is_none());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn settle_moves_the_moment_inspect_reports_resting() {
    let gpu = open();
    let rested = |settle: f32| {
        let mut fluid = Fluid::new(&gpu.device, small(SIZE as u32, settle));
        fluid.sync(&gpu.queue, &[pool_pour()], 0.1);
        fluid.sync(&gpu.queue, &[pool_pour()], 3.0);
        assert_eq!(fluid.settle(), settle);
        fluid.inspect(&gpu.device, &gpu.queue).resting
    };
    assert!(rested(1.0));
    assert!(rested(2.5));
    assert!(!rested(4.0));
    assert_eq!(FluidDesc::default().settle, SETTLE);
}
