#[allow(dead_code)]
#[path = "support/desk.rs"]
mod desk;
#[path = "support/output.rs"]
mod output;
#[allow(dead_code)]
#[path = "support/viewport.rs"]
mod room;

use pfx_gpu::Gpu;
use pfx_gpu::screens::Budget;
use room::Room;

const CAPACITY: [u32; 2] = [512, 256];

fn gpu() -> Gpu {
    pollster::block_on(Gpu::headless()).unwrap()
}

fn crop(pixels: &[u16], width: u32, rect: (u32, u32)) -> Vec<f32> {
    let mut values = Vec::with_capacity((rect.0 * rect.1 * 4) as usize);
    for y in 0..rect.1 {
        let row = (y * width * 4) as usize;
        for value in &pixels[row..row + (rect.0 * 4) as usize] {
            values.push(half::f16::from_bits(*value).to_f32());
        }
    }
    values
}

fn image(room: &mut Room) -> Vec<f32> {
    let width = room.output.width;
    let size = room.renderer.size();
    crop(&room.pixels(), width, size)
}

#[derive(Debug)]
struct Difference {
    mean: f64,
    worst: f64,
    visible: f64,
}

fn difference(a: &[f32], b: &[f32]) -> Difference {
    assert_eq!(a.len(), b.len());
    let mut total = 0.0;
    let mut worst = 0.0f64;
    let mut visible = 0usize;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut largest = 0.0f64;
        for channel in 0..3 {
            let d = f64::from((x[channel] - y[channel]).abs()) * 255.0;
            total += d;
            largest = largest.max(d);
        }
        worst = worst.max(largest);
        if largest > 12.0 {
            visible += 1;
        }
    }
    let pixels = a.len() / 4;
    Difference {
        mean: total / (pixels * 3) as f64,
        worst,
        visible: visible as f64 / pixels as f64,
    }
}

fn finished(room: &mut Room) {
    room.renderer.set_lens(Some(room::lens()));
    room.renderer.set_sharpen(0.3).unwrap();
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_full_viewport_draws_the_same_bytes_as_a_plain_renderer() {
    let gpu = gpu();
    for finish in [false, true] {
        let mut plain = Room::new(gpu.clone(), CAPACITY[0], CAPACITY[1]);
        let mut driven = Room::new(gpu.clone(), CAPACITY[0], CAPACITY[1]);
        if finish {
            finished(&mut plain);
            finished(&mut driven);
        }
        driven
            .renderer
            .set_dynamic_resolution(Some(Budget {
                gpu_ms: 1e4,
                ..Budget::new(1.0)
            }))
            .unwrap();
        for frame in 0..24 {
            plain.draw(frame);
            assert_eq!(
                driven.renderer.apply_dynamic_resolution(CAPACITY).unwrap(),
                (CAPACITY[0], CAPACITY[1])
            );
            driven.draw(frame);
            if frame % 8 == 7 {
                assert!(
                    plain.pixels() == driven.pixels(),
                    "frame {frame}, finish {finish}"
                );
            }
        }
        assert_eq!(driven.renderer.capacity(), (CAPACITY[0], CAPACITY[1]));
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_fixed_viewport_matches_a_renderer_resized_to_it() {
    let gpu = gpu();
    for (rect, finish) in [((384, 192), false), ((384, 192), true), ((256, 128), false)] {
        let mut viewport = Room::new(gpu.clone(), CAPACITY[0], CAPACITY[1]);
        let mut resized = Room::new(gpu.clone(), rect.0, rect.1);
        if finish {
            finished(&mut viewport);
            finished(&mut resized);
        }
        viewport.renderer.set_viewport(rect.0, rect.1).unwrap();
        for frame in 0..32 {
            viewport.draw(frame);
            resized.draw(frame);
        }
        let error = difference(&image(&mut viewport), &image(&mut resized));
        println!("viewport {rect:?} finish {finish} against a resized renderer: {error:?}");
        assert!(error.worst <= 1.0, "{rect:?} {finish}: {error:?}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn changing_scale_every_four_frames_never_restarts_history_or_allocates() {
    let gpu = gpu();
    let mut room = Room::new(gpu, CAPACITY[0], CAPACITY[1]);
    finished(&mut room);
    room.draw(0);
    let targets = room.renderer.target_textures();
    let restarts = room.renderer.history_restarts();
    let scales = [1.0f32, 0.75, 0.5, 0.65, 0.9, 0.55, 0.8, 0.6];
    let mut sizes = Vec::new();
    for frame in 1..=120u32 {
        if frame % 4 == 0 {
            let scale = scales[(frame / 4) as usize % scales.len()];
            let width = (CAPACITY[0] as f32 * scale).round() as u32;
            let height = (CAPACITY[1] as f32 * scale).round() as u32;
            room.renderer.set_viewport(width, height).unwrap();
        }
        sizes.push(room.renderer.size());
        room.draw(frame);
        assert!(room.renderer.history_valid(), "frame {frame}");
        assert_eq!(room.renderer.history_restarts(), restarts, "frame {frame}");
    }
    assert!(sizes.contains(&(256, 128)));
    assert!(
        room.renderer.target_textures() == targets,
        "a scale change reallocated a target"
    );
    assert_eq!(room.renderer.capacity(), (CAPACITY[0], CAPACITY[1]));
}

fn settled(gpu: &Gpu, rect: (u32, u32), frames: &[u32]) -> Vec<Vec<f32>> {
    let mut room = Room::new(gpu.clone(), CAPACITY[0], CAPACITY[1]);
    room.renderer.set_viewport(rect.0, rect.1).unwrap();
    let mut images = Vec::new();
    for frame in 0..=*frames.last().unwrap() {
        room.draw(frame);
        if frames.contains(&frame) {
            images.push(image(&mut room));
        }
    }
    images
}

fn scaled(scale: f32) -> (u32, u32) {
    (
        (CAPACITY[0] as f32 * scale).round() as u32,
        (CAPACITY[1] as f32 * scale).round() as u32,
    )
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_scale_step_keeps_the_history_and_barely_shows() {
    let gpu = gpu();
    let after_frames = [32, 33, 35, 39];
    for (from, to) in [(0.75, 0.7), (0.75, 0.5), (0.5, 1.0), (1.0, 0.8)] {
        let before = scaled(from);
        let after = scaled(to);
        let reference = settled(&gpu, after, &after_frames);
        let mut stepped = Room::new(gpu.clone(), CAPACITY[0], CAPACITY[1]);
        stepped.renderer.set_viewport(before.0, before.1).unwrap();
        let mut resized = Room::new(gpu.clone(), before.0, before.1);
        resized.output = output::output(&gpu, CAPACITY[0], CAPACITY[1]);
        for frame in 0..32 {
            stepped.draw(frame);
            resized.draw(frame);
        }
        stepped.renderer.set_viewport(after.0, after.1).unwrap();
        resized.renderer.resize(after.0, after.1).unwrap();
        let mut steps = Vec::new();
        let mut restarts = Vec::new();
        for frame in 32..=*after_frames.last().unwrap() {
            stepped.draw(frame);
            resized.draw(frame);
            if let Some(at) = after_frames.iter().position(|f| *f == frame) {
                steps.push(difference(&image(&mut stepped), &reference[at]));
                restarts.push(difference(&image(&mut resized), &reference[at]));
            }
        }
        for (index, frame) in after_frames.iter().enumerate() {
            let (step, restart) = (&steps[index], &restarts[index]);
            println!(
                "step {from} to {to}, frame +{}: viewport mean {:.3} worst {:.1} visible {:.3}%, resize mean {:.3} worst {:.1} visible {:.3}%",
                frame - 32,
                step.mean,
                step.worst,
                step.visible * 100.0,
                restart.mean,
                restart.worst,
                restart.visible * 100.0
            );
        }
        assert_eq!(stepped.renderer.history_restarts(), 0);
        assert!(
            steps[0].mean < restarts[0].mean,
            "{from} to {to}: {steps:?} {restarts:?}"
        );
    }
}
