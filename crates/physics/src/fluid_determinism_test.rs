use super::inspect::mapped;
use super::*;
use crate::shape::Form;
use pfx_gpu::Gpu;
use pfx_gpu::pace::{self, Turns};
use std::collections::BTreeMap;

const DESK_FRAMES: u32 = 540;
const DESK_DT: f32 = 1.0 / 60.0;

fn open() -> Gpu {
    pollster::block_on(Gpu::headless()).expect("gpu")
}

fn read_buffer(gpu: &Gpu, buffer: &wgpu::Buffer, size: u64) -> Vec<u8> {
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("determinism readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    gpu.queue.submit(Some(encoder.finish()));
    mapped(&gpu.device, &staging)
}

fn read_words(gpu: &Gpu, buffer: &wgpu::Buffer, count: usize) -> Vec<u32> {
    bytemuck::cast_slice(&read_buffer(gpu, buffer, count as u64 * 4)).to_vec()
}

fn read_texture(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let padded = (size.width * 8).div_ceil(256) * 256;
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("determinism texels"),
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
    data.chunks_exact(padded as usize)
        .flat_map(|row| &row[..size.width as usize * 8])
        .copied()
        .collect()
}

fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b).position(|(x, y)| x != y)
}

fn look(tall: f32, active: f32, weight_b: f32, weight_a: f32) -> Look {
    Look {
        tall,
        active,
        weight_b,
        weight_a,
    }
}

fn desk_pours() -> Vec<Pour> {
    let pour = |key: &str, target: Target| Pour {
        key: key.into(),
        target,
        from: None,
        fill: None,
    };
    let mut out = vec![pour(
        "pool",
        Target::outline(
            [513.7, 519.4],
            483.836,
            [
                [0.1044, 0.822],
                [0.0713, 1.350],
                [0.0586, 2.759],
                [0.0518, 2.989],
                [0.0425, 1.623],
            ],
            &look(100.0, 0.0, 0.0, 2.8),
        )
        .squash(1.0)
        .pull(0.0),
    )];
    let bubbles = [
        (
            [1084.0, 212.0],
            [110.24, 108.16],
            [-0.10392, 0.07431, 27.42353],
        ),
        ([1292.0, 226.0], [96.0, 104.0], [0.15882, 0.07902, 19.00784]),
        ([1470.0, 236.0], [80.0, 99.0], [-0.01373, 0.08863, 35.83922]),
        ([1560.0, 470.0], [70.0, 64.0], [0.06, 0.08, 11.5]),
    ];
    for (slot, (centre, radii, form)) in bubbles.into_iter().enumerate() {
        out.push(pour(
            &format!("bubble{slot}"),
            Target::drop(
                centre,
                radii,
                &Form {
                    tilt: form[0],
                    irregular: form[1],
                    seed: form[2],
                    rim: 0.0,
                    weight: [0.9, 0.05],
                },
                &look(81.6, if slot == 0 { 1.0 } else { 0.0 }, 0.0, 0.5),
            ),
        ));
    }
    out.push(pour(
        "stem",
        Target::curve(
            [
                [1139.12, 305.669],
                [1138.962, 339.608],
                [1138.838, 366.004],
                [1138.68, 399.943],
            ],
            [28.0, 28.0, 8.0],
            &look(24.0, 1.0, 0.0, 0.5),
        )
        .flare(4.0)
        .drain(0.0)
        .pull(2.0)
        .mass(1.0),
    ));
    out.push(pour(
        "bead",
        Target::bead([1096.56, 559.2304], 8.0, &look(8.0, 0.8, 1.0, 0.0)),
    ));
    for slot in 0..5 {
        let near = [1188.5 + 6.0 * slot as f32, 662.5 + 62.0 * slot as f32];
        let far = [near[0] + 230.0, near[1] - 18.0];
        out.push(pour(
            &format!("pill{slot}"),
            Target::capsule(
                near,
                far,
                [12.0, 24.0],
                &look(48.0, if slot == 0 { 1.0 } else { 0.0 }, 0.0, 0.6),
            )
            .organic(4.2, 0.08, 19.5 + slot as f32)
            .drifting(0.0)
            .pull(1.4)
            .mass(1.0),
        ));
    }
    out
}

fn desk_fluid(device: &wgpu::Device) -> Fluid {
    let mut desc = FluidDesc::new(1792, 1024);
    desc.max_particles = 262_144;
    desc.max_targets = 512;
    Fluid::new(device, desc)
}

struct Poured {
    live: u32,
    particles: Vec<u8>,
    height: Vec<u8>,
    edge: Vec<u8>,
    passes: BTreeMap<String, Vec<f64>>,
}

fn pour_desk(gpu: &Gpu, wait_every: u32, mut profiler: Option<&mut GpuProfiler>) -> Poured {
    let mut fluid = desk_fluid(&gpu.device);
    let pours = desk_pours();
    let wires: Vec<[f32; 4]> = (0..24)
        .map(|k| {
            let x = 1100.0 + 12.0 * k as f32;
            [x, 560.0 + 3.0 * k as f32, x + 12.0, 563.0 + 3.0 * k as f32]
        })
        .collect();
    let mut turns = Turns::default();
    let mut passes: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for frame in 0..DESK_FRAMES {
        turns.poll();
        let time = (frame + 1) as f32 * DESK_DT;
        fluid.set_wires(&wires, Wire::default());
        fluid.set_smoothing(Smoothing::default());
        fluid.set_outline(Outline::default());
        fluid.sync(&gpu.queue, &pours, DESK_DT);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        fluid.encode(
            &gpu.queue,
            &mut encoder,
            DESK_DT,
            time,
            1.0,
            true,
            profiler.as_deref_mut(),
        );
        let slot = profiler
            .as_deref_mut()
            .and_then(|profile| profile.finish(&mut encoder));
        gpu.queue.submit(Some(encoder.finish()));
        if let (Some(profile), Some(slot)) = (profiler.as_deref_mut(), slot) {
            profile.submitted(slot);
        }
        if (frame + 1) % wait_every == 0 || frame + 1 == DESK_FRAMES {
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            let mut gpu_ms = None;
            if let Some(profile) = profiler.as_deref_mut() {
                for timings in profile.collect(&gpu.device) {
                    if let Some(frame_ms) = pace::gpu_ms(&timings) {
                        *gpu_ms.get_or_insert(0.0) += frame_ms;
                    }
                    for timing in timings {
                        passes
                            .entry(timing.label.clone())
                            .or_default()
                            .push(timing.milliseconds);
                    }
                }
            }
            turns.add(gpu_ms.unwrap_or(f64::NAN));
        }
    }
    let live = fluid.particle_count();
    Poured {
        live,
        particles: read_buffer(
            gpu,
            fluid.particles_buffer(),
            fluid.high as u64 * std::mem::size_of::<Particle>() as u64,
        ),
        height: read_texture(gpu, fluid.height_texture()),
        edge: read_texture(gpu, fluid.edge_texture()),
        passes,
    }
}

fn assert_same(name: &str, a: &Poured, b: &Poured) {
    assert_eq!(a.live, b.live, "{name}: live particles");
    for (field, left, right) in [
        ("particles", &a.particles, &b.particles),
        ("height", &a.height, &b.height),
        ("edge", &a.edge, &b.edge),
    ] {
        let differ = first_difference(left, right);
        assert!(
            differ.is_none(),
            "{name}: {field} first differs at byte {differ:?} of {}",
            left.len()
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_same_pours_repeat_bit_for_bit() {
    let gpu = open();
    let first = pour_desk(&gpu, 1, None);
    assert!(first.live > 100_000, "{}", first.live);
    let again = pour_desk(&gpu, 1, None);
    assert_same("again", &first, &again);
    let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
    let batched = pour_desk(&gpu, 5, Some(&mut profiler));
    assert_same("batched and profiled", &first, &batched);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn desk_pass_times() {
    let gpu = open();
    let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
    let poured = pour_desk(&gpu, 1, Some(&mut profiler));
    println!("{} live particles", poured.live);
    for (label, mut ms) in poured.passes {
        ms.sort_by(f64::total_cmp);
        let at = |fraction: f64| ms[((ms.len() - 1) as f64 * fraction).round() as usize];
        println!(
            "{label}: {} frames, median {:.3} ms, p95 {:.3} ms",
            ms.len(),
            at(0.5),
            at(0.95)
        );
        assert!(ms.iter().all(|value| value.is_finite()));
    }
}

fn scattered(count: usize, table: [f32; 2], seed: u32) -> Vec<Particle> {
    let mut state = seed;
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / (1u32 << 24) as f32
    };
    let hot = [[600.5, 600.5], [7.0, 7.0], [table[0] - 2.0, table[1] - 2.0]];
    (0..count)
        .map(|i| {
            let roll = random();
            let pos = if roll < 0.1 {
                let at = hot[(random() * hot.len() as f32) as usize % hot.len()];
                [at[0] + random() * 5.0, at[1] + random() * 5.0]
            } else if roll < 0.12 {
                [
                    random() * table[0] * 1.2 - 10.0,
                    random() * table[1] * 1.2 - 10.0,
                ]
            } else {
                [random() * table[0], random() * table[1]]
            };
            let pos = [
                ((pos[0] / KERNEL).floor() + 0.1 + 0.8 * random()) * KERNEL,
                ((pos[1] / KERNEL).floor() + 0.1 + 0.8 * random()) * KERNEL,
            ];
            let fate = random();
            Particle {
                pos,
                vel: [0.0; 2],
                owner: 0,
                rho: 1.0,
                life: if fate < 0.03 { 0.0 } else { 1.0 },
                born: if fate > 0.97 { -1.0 } else { i as f32 },
            }
        })
        .collect()
}

struct GpuBins {
    cells: [usize; 2],
    counts: Vec<u32>,
    starts: Vec<u32>,
    items: Vec<u32>,
}

fn bin_on_gpu(gpu: &Gpu, table: [u32; 2], particles: &[Particle]) -> GpuBins {
    let mut fluid = Fluid::new(
        &gpu.device,
        FluidDesc {
            width: table[0],
            height: table[1],
            max_particles: particles.len() as u32,
            max_targets: 1,
            params: Params::default(),
            settle: FluidDesc::default().settle,
        },
    );
    fluid.high = particles.len() as u32;
    gpu.queue
        .write_buffer(&fluid.particles, 0, bytemuck::cast_slice(particles));
    gpu.queue.write_buffer(
        &fluid.uniform,
        0,
        bytemuck::bytes_of(&fluid.sim_uniform(0, 0.0, 1.0)),
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_bind_group(0, &fluid.bind, &[]);
        fluid.bin(
            &mut pass,
            fluid.high.div_ceil(256),
            (fluid.cells[0] * fluid.cells[1]).div_ceil(BLOCK),
        );
    }
    gpu.queue.submit(Some(encoder.finish()));
    let cells = [fluid.cells[0] as usize, fluid.cells[1] as usize];
    let total = cells[0] * cells[1];
    GpuBins {
        cells,
        counts: read_words(gpu, fluid.cell_counts(), total),
        starts: read_words(
            gpu,
            fluid.cell_starts(),
            total + total.div_ceil(BLOCK as usize),
        ),
        items: read_words(gpu, fluid.cell_items(), particles.len()),
    }
}

#[test]
fn the_cpu_bins_count_scan_and_keep_index_order() {
    assert_eq!(exclusive_scan(&[3, 0, 2, 5, 0]), [0, 3, 3, 5, 10]);
    assert!(exclusive_scan(&[]).is_empty());
    let table = [120.0, 90.0];
    let particles = scattered(4000, table, 0x9e37_79b9);
    let cells = cells_for(table);
    let binned = bin(&particles, cells);
    assert_eq!(binned.starts, exclusive_scan(&binned.counts));
    let placed = particles
        .iter()
        .filter(|particle| binned_cell(particle, cells).is_some())
        .count();
    assert_eq!(binned.items.len(), placed);
    for (id, (&start, &count)) in binned.starts.iter().zip(&binned.counts).enumerate() {
        let slice = &binned.items[start as usize..(start + count) as usize];
        assert!(slice.windows(2).all(|pair| pair[0] < pair[1]), "cell {id}");
        assert!(
            slice
                .iter()
                .all(|&i| binned_cell(&particles[i as usize], cells) == Some(id))
        );
    }
    assert!(binned.counts.iter().any(|&count| count > SLOTS));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_prefix_sum_matches_the_cpu() {
    let gpu = open();
    let table = [1600, 1600];
    let particles = scattered(150_000, [1600.0, 1600.0], 0x51f1_5eed);
    let bins = bin_on_gpu(&gpu, table, &particles);
    let total = bins.cells[0] * bins.cells[1];
    assert!(total.div_ceil(BLOCK as usize) > BLOCK as usize, "{total}");
    let reference = bin(&particles, bins.cells);
    assert_eq!(bins.counts, reference.counts);
    let starts = exclusive_scan(&bins.counts);
    assert_eq!(&bins.starts[..total], starts.as_slice());
    let blocks: Vec<u32> = bins
        .counts
        .chunks(BLOCK as usize)
        .map(|chunk| chunk.iter().sum())
        .collect();
    assert_eq!(&bins.starts[total..], exclusive_scan(&blocks).as_slice());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_scatter_keeps_particle_order_within_a_cell() {
    let gpu = open();
    let table = [400, 300];
    let particles = scattered(60_000, [400.0, 300.0], 0x2545_f491);
    let bins = bin_on_gpu(&gpu, table, &particles);
    let reference = bin(&particles, bins.cells);
    assert!(reference.counts.iter().any(|&count| count > SLOTS));
    let placed = reference.items.len();
    assert_eq!(&bins.items[..placed], reference.items.as_slice());
    for (&start, &count) in bins.starts.iter().zip(&bins.counts) {
        let slice = &bins.items[start as usize..(start + count) as usize];
        assert!(slice.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
