use std::time::Instant;

use pfx_gpu::{Gpu, GpuProfiler, OffscreenTarget, wgpu};
use pfx_physics::{Chain, ChainParams, Fluid, FluidDesc, FluidState, Look, Params, Pour, Target};

use super::{manners, percentile};

pub const TANK: [f32; 2] = [384.0, 256.0];
pub const FRAMES: u32 = 600;
const BEADS: usize = 5;
const ANCHOR: [f32; 2] = [192.0, 30.0];

pub fn desc() -> FluidDesc {
    FluidDesc {
        width: TANK[0] as u32,
        height: TANK[1] as u32,
        max_particles: 32_768,
        max_targets: 16,
        params: Params::default(),
        settle: 2.5,
    }
}

pub fn params() -> ChainParams {
    ChainParams {
        pull: 30.0,
        drag: 3.0,
        flow: 20.0,
        slack: 1.05,
        ..ChainParams::default()
    }
}

fn look(tall: f32, weight: f32) -> Look {
    Look {
        tall,
        active: 0.5,
        weight_b: weight,
        weight_a: 0.0,
    }
}

pub fn pours() -> Vec<Pour> {
    let pour = |key: &str, target: Target| Pour {
        key: key.into(),
        target,
        from: None,
        fill: None,
    };
    let mut out = vec![pour(
        "pool",
        Target::bead([TANK[0] * 0.5, TANK[1] * 0.6], 90.0, &look(60.0, 1.0)),
    )];
    for (index, at) in [[90.0, 70.0], [300.0, 80.0], [200.0, 200.0]]
        .into_iter()
        .enumerate()
    {
        out.push(pour(
            &format!("drop {index}"),
            Target::bead(at, 14.0, &look(14.0, 0.2)),
        ));
    }
    out
}

fn goals(time: f32) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    let beads: Vec<[f32; 2]> = (0..BEADS)
        .map(|index| {
            let swing =
                (time * 0.8 + index as f32 * 0.4).sin() * 12.0 * index as f32 / BEADS as f32;
            [ANCHOR[0] + swing, ANCHOR[1] + 24.0 * (index + 1) as f32]
        })
        .collect();
    let cords = beads.iter().map(|bead| [bead[0] + 10.0, bead[1]]).collect();
    (beads, cords)
}

pub struct Run {
    pub sim_ms: Vec<f64>,
    pub state: FluidState,
    pub height: Vec<u16>,
    pub wires: Vec<[f32; 4]>,
}

pub fn run(gpu: &Gpu, frames: u32) -> Run {
    let device = &gpu.device;
    let queue = &gpu.queue;
    let mut fluid = Fluid::new(device, desc());
    let mut chain = Chain::new(params());
    let pours = pours();
    let dt = 1.0 / 60.0;
    let mut profiler = GpuProfiler::new(device, queue);
    let mut sim_ms = Vec::new();
    let mut pace = manners::Pace::timed("bench", "physics", u64::from(frames));
    for number in 0..frames {
        let tick = Instant::now();
        let time = (number + 1) as f32 * dt;
        let (beads, cords) = goals(time);
        chain.step(dt, time, ANCHOR, &beads, &cords, 0.0);
        let wires = chain.wires();
        fluid.set_field(
            &wires,
            [2.1, 5.0, 3.2],
            [12.0, 3.0, 24.0],
            [10.0, 22.0, 0.0, 6.0, 8.0, 20.0, 0.35, 0.0],
        );
        fluid.sync(queue, &pours, dt);
        let mut encoder = device.create_command_encoder(&Default::default());
        fluid.encode(
            queue,
            &mut encoder,
            dt,
            time,
            1.0,
            true,
            Some(&mut profiler),
        );
        let slot = profiler.finish(&mut encoder);
        queue.submit(Some(encoder.finish()));
        if let Some(slot) = slot {
            profiler.submitted(slot);
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mut gpu_ms = None;
        for frame in profiler.collect(device) {
            let ms: f64 = frame.iter().map(|pass| pass.milliseconds).sum();
            if ms > 0.0 {
                sim_ms.push(ms);
                *gpu_ms.get_or_insert(0.0) += ms;
            }
        }
        pace.after_gpu(tick.elapsed().as_secs_f64() * 1000.0, gpu_ms);
    }
    let state = fluid.inspect(device, queue);
    let texture = fluid.height_texture().clone();
    let size = texture.size();
    let view = texture.create_view(&Default::default());
    let height = gpu
        .readback_rgba16(&OffscreenTarget {
            texture,
            view,
            format: wgpu::TextureFormat::Rgba16Float,
            width: size.width,
            height: size.height,
        })
        .unwrap();
    Run {
        sim_ms,
        state,
        height,
        wires: chain.wires(),
    }
}

pub fn measure() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let first = run(&gpu, FRAMES);
    let second = run(&gpu, FRAMES);
    let mut sorted = first.sim_ms.clone();
    sorted.sort_by(f64::total_cmp);
    println!(
        "physics: a {}x{} SPH tank with {} pours and a rope of {BEADS} beads, {FRAMES} frames at 60 Hz: fluid sim p50 {:.3} ms, p99 {:.3} ms over {} timed frames; {} live particles, {} asleep, height {:.3}..{:.3}",
        TANK[0],
        TANK[1],
        pours().len(),
        if sorted.is_empty() {
            0.0
        } else {
            percentile(&sorted, 0.5)
        },
        if sorted.is_empty() {
            0.0
        } else {
            percentile(&sorted, 0.99)
        },
        sorted.len(),
        first.state.live,
        first.state.asleep,
        first.state.height_min,
        first.state.height_max
    );
    let same =
        first.state == second.state && first.height == second.height && first.wires == second.wires;
    println!(
        "physics determinism: two runs give {} fluid state, height field and rope",
        if same { "the same" } else { "a different" }
    );
    assert!(same, "the physics runs differ");
}
