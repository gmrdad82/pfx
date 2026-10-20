use std::time::Instant;

use egui::{Align2, Pos2, RawInput, Rect, ViewportId, ViewportInfo, vec2};
use egui_wgpu::{RendererOptions, ScreenDescriptor};
use pfx_gpu::pace::Pace;
use pfx_gpu::stats::percentiles;
use pfx_gpu::{Gpu, GpuProfiler, wgpu};
use pfx_live::hud::{self, Readout, Shown};

const SIZE: [u32; 2] = [3840, 2160];
const SCALE: f32 = 2.0;
const WARM: usize = 60;
const FRAMES: usize = 600;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
const CHANGE: usize = 30;

fn readout(frame: usize) -> Readout {
    let wobble = (frame % 7) as f64 * 0.01;
    Readout {
        pace: Some(hud::Pace {
            fps: 120.0 - wobble,
            p50_ms: 8.33 + wobble,
            p99_ms: 8.9 + wobble,
            max_ms: 11.2 + wobble,
        }),
        cpu_ms: Some(0.71 + wobble),
        gpu: Some((0, 3.52 + wobble)),
        passes: [
            "opaque",
            "shadow",
            "sky",
            "taa",
            "post",
            "particles",
            "glass",
        ]
        .iter()
        .enumerate()
        .map(|(index, name)| (name.to_string(), 0.4 + index as f64 * 0.1 + wobble))
        .collect(),
        scale: Some(1.0),
        present: Some(Shown {
            mode: "Mailbox".into(),
            vrr: true,
            vrr_in_effect: Some(true),
            frame_rate: "uncapped".into(),
        }),
        drawn: Some([412, 1180]),
        hud_ms: Some(0.05 + wobble),
        ..Readout::default()
    }
}

fn input() -> RawInput {
    let mut input = RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::ZERO,
            vec2(SIZE[0] as f32 / SCALE, SIZE[1] as f32 / SCALE),
        )),
        max_texture_side: Some(8192),
        ..RawInput::default()
    };
    input.viewports.insert(
        ViewportId::ROOT,
        ViewportInfo {
            native_pixels_per_point: Some(SCALE),
            ..ViewportInfo::default()
        },
    );
    input
}

fn line(name: &str, values: &[f64]) {
    let spread = percentiles(values.iter().copied());
    println!(
        "{{\"what\":\"{name}\",\"frames\":{},\"p50_ms\":{:.4},\"p99_ms\":{:.4},\"max_ms\":{:.4}}}",
        values.len(),
        spread.p50,
        spread.p99,
        spread.max
    );
}

fn main() {
    let gpu = pollster::block_on(Gpu::headless()).expect("a GPU");
    let ctx = hud::context();
    let corner = Pos2::new(SIZE[0] as f32 / SCALE - 6.0, 6.0);
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hud cost target"),
        size: wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut egui = egui_wgpu::Renderer::new(&gpu.device, FORMAT, RendererOptions::PREDICTABLE);
    let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: SCALE,
    };
    let mut pace = Pace::new();
    let mut cpu = Vec::new();
    let mut empty = Vec::new();
    let mut every = Vec::new();
    let bare = hud::context();
    let busy = hud::context();
    let mut gpu_ms = Vec::new();
    let mut triangles = 0;
    for frame in 0..WARM + FRAMES {
        let started = Instant::now();
        let mut full = ctx.run_ui(input(), |root| {
            hud::ui_at(
                root.ctx(),
                &readout(frame / CHANGE),
                corner,
                Align2::RIGHT_TOP,
            );
        });
        let primitives = ctx.tessellate(std::mem::take(&mut full.shapes), full.pixels_per_point);
        let spent = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let mut nothing = bare.run_ui(input(), |_| {});
        let _ = bare.tessellate(
            std::mem::take(&mut nothing.shapes),
            nothing.pixels_per_point,
        );
        let spent_empty = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let mut changing = busy.run_ui(input(), |root| {
            hud::ui_at(root.ctx(), &readout(frame), corner, Align2::RIGHT_TOP);
        });
        let _ = busy.tessellate(
            std::mem::take(&mut changing.shapes),
            changing.pixels_per_point,
        );
        let spent_every = started.elapsed().as_secs_f64() * 1000.0;
        triangles = primitives
            .iter()
            .map(|clipped| match &clipped.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.indices.len() / 3,
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum::<usize>();
        pace.frame(|| {
            for (id, deltas) in &full.textures_delta.set {
                for delta in deltas {
                    egui.update_texture(&gpu.device, &gpu.queue, *id, delta);
                }
            }
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("hud cost"),
                });
            let mut commands =
                egui.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &primitives, &screen);
            let index = profiler.pass("hud");
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("hud cost"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: profiler.render_writes(index),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let mut pass = pass.forget_lifetime();
                egui.render(&mut pass, &primitives, &screen);
            }
            let slot = profiler.finish(&mut encoder);
            commands.push(encoder.finish());
            gpu.queue.submit(commands);
            if let Some(slot) = slot {
                profiler.submitted(slot);
            }
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        });
        for id in &full.textures_delta.free {
            egui.free_texture(id);
        }
        let timed: Vec<f64> = profiler
            .collect(&gpu.device)
            .into_iter()
            .flatten()
            .map(|pass| pass.milliseconds)
            .collect();
        if frame >= WARM {
            cpu.push(spent);
            empty.push(spent_empty);
            every.push(spent_every);
            gpu_ms.extend(timed);
        }
    }
    println!(
        "{{\"size\":[{},{}],\"scale\":{SCALE},\"triangles\":{triangles}}}",
        SIZE[0], SIZE[1]
    );
    line(
        "cpu: an egui frame with the panel, values changing 4 times a second",
        &cpu,
    );
    line("cpu: an empty egui frame", &empty);
    line(
        "cpu: the panel with every value changing every frame",
        &every,
    );
    let added: Vec<f64> = cpu
        .iter()
        .zip(&empty)
        .map(|(with, without)| with - without)
        .collect();
    line(
        "cpu: the panel's share, with minus empty, frame by frame",
        &added,
    );
    if gpu_ms.is_empty() {
        println!("{{\"what\":\"gpu\",\"note\":\"this device has no timestamp queries\"}}");
    } else {
        line("gpu: the panel's paint at 4K", &gpu_ms);
    }
}
