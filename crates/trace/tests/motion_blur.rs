#[path = "support/spinning.rs"]
mod spinning;

use pfx_gpu::Gpu;
use pfx_trace::Trace;
use spinning::{BODY, BODY_SIZE, FLAG, FLAG_HALF, Moves, WALL, WHEEL, luminance, pixel};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const SAMPLES: u32 = 32;
const SEED: u32 = 7;

static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn staged(gpu: &Gpu, moves: Moves, lit: bool, hardware: bool) -> Trace {
    let _env = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = std::env::var_os("PFX_TRACE_RT");
    unsafe { std::env::set_var("PFX_TRACE_RT", if hardware { "1" } else { "0" }) };
    let trace = spinning::stage(gpu, moves, lit, [WIDTH, HEIGHT], SAMPLES);
    match before {
        Some(value) => unsafe { std::env::set_var("PFX_TRACE_RT", value) },
        None => unsafe { std::env::remove_var("PFX_TRACE_RT") },
    }
    trace.unwrap()
}

fn traced(trace: &mut Trace, gpu: &Gpu) -> Vec<u8> {
    trace.sample(gpu, SAMPLES, SEED).unwrap();
    trace.readback_color(gpu).unwrap()
}

fn render(gpu: &Gpu, moves: Moves, lit: bool, hardware: bool) -> Option<Vec<f32>> {
    let mut trace = staged(gpu, moves, lit, hardware);
    if hardware && !trace.hardware_rays() {
        println!("no ray queries on this adapter; the hardware path is skipped");
        return None;
    }
    Some(luminance(&traced(&mut trace, gpu)))
}

fn at(image: &[f32], x: f32, y: f32) -> f32 {
    let x = (x as u32).min(WIDTH - 1);
    let y = (y as u32).min(HEIGHT - 1);
    image[(y * WIDTH + x) as usize]
}

fn row(image: &[f32], from: [f32; 3], to: [f32; 3], band: i32) -> Vec<f32> {
    let a = pixel(WIDTH, HEIGHT, from);
    let b = pixel(WIDTH, HEIGHT, to);
    let along = [b[0] - a[0], b[1] - a[1]];
    let steps = along[0].abs().max(along[1].abs()).round() as usize;
    let across = if along[0].abs() >= along[1].abs() {
        [0.0, 1.0]
    } else {
        [1.0, 0.0]
    };
    (0..=steps)
        .map(|k| {
            let t = k as f32 / steps as f32;
            let x = a[0] + along[0] * t;
            let y = a[1] + along[1] * t;
            let sum: f32 = (-band..=band)
                .map(|o| at(image, x + across[0] * o as f32, y + across[1] * o as f32))
                .sum();
            sum / (2 * band + 1) as f32
        })
        .collect()
}

fn width(profile: &[f32]) -> usize {
    let plateau = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    let a = plateau(&profile[..6]);
    let b = plateau(&profile[profile.len() - 6..]);
    let (low, span) = (a.min(b), (a - b).abs());
    profile
        .iter()
        .filter(|&&v| {
            let f = (v - low) / span;
            f > 0.1 && f < 0.9
        })
        .count()
}

fn circle(image: &[f32], center: [f32; 3], radius: f32) -> Vec<f32> {
    (0..720)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / 720.0;
            let p = pixel(
                WIDTH,
                HEIGHT,
                [
                    center[0] + radius * a.cos(),
                    center[1] + radius * a.sin(),
                    center[2],
                ],
            );
            at(image, p[0], p[1])
        })
        .collect()
}

fn spread(values: &[f32]) -> f32 {
    let lo = values.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    hi - lo
}

fn region_diff(a: &[f32], b: &[f32], rows: std::ops::Range<u32>) -> f32 {
    rows.flat_map(|y| (0..WIDTH).map(move |x| (y * WIDTH + x) as usize))
        .map(|i| (a[i] - b[i]).abs())
        .fold(0.0, f32::max)
}

const MOVING: Moves = Moves {
    steps: 3,
    travel: 0.8,
    spin: 1.6,
    pan: 0.0,
    rise: 0.0,
    dense: 0,
};

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_spinning_wheel_on_a_moving_body_smears_and_the_still_background_stays_sharp() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut blurred_paths = Vec::new();
    for hardware in [false, true] {
        let Some(sharp) = render(&gpu, Moves::default(), false, hardware) else {
            continue;
        };
        let blurred = render(&gpu, MOVING, false, hardware).unwrap();
        let right = BODY[0] + BODY_SIZE[0] * 0.5;
        let body_row = |image: &[f32]| {
            row(
                image,
                [right - 0.6, BODY[1] - 0.3, 0.0],
                [right + 0.8, BODY[1] - 0.3, 0.0],
                8,
            )
        };
        let top = BODY[1] + BODY_SIZE[1] * 0.5;
        let body_column = |image: &[f32]| {
            row(
                image,
                [right - 0.25, top - 0.2, 0.0],
                [right - 0.25, top + 0.2, 0.0],
                8,
            )
        };
        let along = (width(&body_row(&sharp)), width(&body_row(&blurred)));
        let across = (width(&body_column(&sharp)), width(&body_column(&blurred)));
        let stripes_top = pixel(WIDTH, HEIGHT, [0.0, 2.4, WALL])[1] as u32;
        let stripes_bottom = pixel(WIDTH, HEIGHT, [0.0, 1.6, WALL])[1] as u32;
        let still = region_diff(&sharp, &blurred, stripes_top - 4..stripes_bottom + 4);
        let wheel = [BODY[0], BODY[1], 0.05];
        let spokes = (
            spread(&circle(&sharp, wheel, 0.3)),
            spread(&circle(&blurred, wheel, 0.3)),
        );
        println!(
            "hardware {hardware}: body edge along its path {} px sharp, {} px blurred; top edge {} px sharp, {} px blurred; stripes differ by at most {still}; spokes swing {:.3} sharp, {:.3} blurred",
            along.0, along.1, across.0, across.1, spokes.0, spokes.1
        );
        let travel_px = MOVING.travel * pixel(WIDTH, HEIGHT, [1.0, 0.0, 0.0])[0]
            - MOVING.travel * pixel(WIDTH, HEIGHT, [0.0, 0.0, 0.0])[0];
        assert!(
            along.0 <= 4,
            "hardware {hardware}: the still body is blurred"
        );
        assert!(
            along.1 as f32 >= 0.6 * travel_px,
            "hardware {hardware}: the body does not smear along its path"
        );
        assert!(
            across.1 <= 4,
            "hardware {hardware}: the body smears across its path"
        );
        assert!(
            still <= 1e-5,
            "hardware {hardware}: the still stripes changed by {still}"
        );
        assert!(
            spokes.1 < 0.5 * spokes.0,
            "hardware {hardware}: the spokes stay sharp"
        );
        blurred_paths.push(blurred);
    }
    if let [software, hardware] = &blurred_paths[..] {
        let mean = software
            .iter()
            .zip(hardware)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / software.len() as f32;
        println!("software and hardware blur differ by {mean} on average");
        assert!(mean < 2e-3, "the two paths blur differently: {mean}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_spinning_wheel_smears_around_its_hub_and_keeps_its_rim_round() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let spin = Moves {
        steps: 3,
        spin: 1.6,
        ..Moves::default()
    };
    for hardware in [false, true] {
        let Some(sharp) = render(&gpu, Moves::default(), false, hardware) else {
            continue;
        };
        let blurred = render(&gpu, spin, false, hardware).unwrap();
        let wheel = [BODY[0], BODY[1], 0.05];
        let spokes = (
            spread(&circle(&sharp, wheel, 0.3)),
            spread(&circle(&blurred, wheel, 0.3)),
        );
        let rim = |image: &[f32]| {
            row(
                image,
                [wheel[0], wheel[1] + WHEEL - 0.04, wheel[2]],
                [wheel[0], wheel[1] + WHEEL + 0.12, wheel[2]],
                4,
            )
        };
        let edge = (width(&rim(&sharp)), width(&rim(&blurred)));
        println!(
            "hardware {hardware}: spokes swing {:.3} sharp, {:.3} spun; the rim's edge {} px sharp, {} px spun",
            spokes.0, spokes.1, edge.0, edge.1
        );
        assert!(
            spokes.1 < 0.25 * spokes.0,
            "hardware {hardware}: the spokes stay sharp"
        );
        assert!(
            edge.1 <= 4,
            "hardware {hardware}: the spin blurred the rim's outline"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_panning_camera_smears_the_still_stripes() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let pan = Moves {
        steps: 3,
        pan: 0.6,
        ..Moves::default()
    };
    for hardware in [false, true] {
        let Some(sharp) = render(&gpu, Moves::default(), false, hardware) else {
            continue;
        };
        let blurred = render(&gpu, pan, false, hardware).unwrap();
        let stripes = |image: &[f32]| row(image, [-3.0, 2.0, WALL], [3.0, 2.0, WALL], 10);
        let contrast = (spread(&stripes(&sharp)), spread(&stripes(&blurred)));
        println!(
            "hardware {hardware}: the stripes swing {:.3} still, {:.3} panned",
            contrast.0, contrast.1
        );
        assert!(
            contrast.1 < 0.5 * contrast.0,
            "hardware {hardware}: the pan left the stripes sharp"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_deforming_flag_smears_its_moving_edge_only() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let rise = Moves {
        steps: 3,
        rise: 0.4,
        ..Moves::default()
    };
    for hardware in [false, true] {
        let Some(sharp) = render(&gpu, Moves::default(), false, hardware) else {
            continue;
        };
        let blurred = render(&gpu, rise, false, hardware).unwrap();
        let top = FLAG[1] + FLAG_HALF;
        let bottom = FLAG[1] - FLAG_HALF;
        let edge = |image: &[f32], y: f32| {
            width(&row(
                image,
                [FLAG[0], y - 0.3, 0.0],
                [FLAG[0], y + 0.3, 0.0],
                6,
            ))
        };
        let moving = (edge(&sharp, top), edge(&blurred, top));
        let resting = (edge(&sharp, bottom), edge(&blurred, bottom));
        println!(
            "hardware {hardware}: the rising edge {} px still, {} px deformed; the resting edge {} px still, {} px deformed",
            moving.0, moving.1, resting.0, resting.1
        );
        let rise_px = pixel(WIDTH, HEIGHT, [0.0, 0.0, 0.0])[1]
            - pixel(WIDTH, HEIGHT, [0.0, rise.rise, 0.0])[1];
        assert!(
            moving.1 as f32 >= 0.6 * rise_px,
            "hardware {hardware}: the rising edge is sharp"
        );
        assert!(
            resting.1 <= 4,
            "hardware {hardware}: the resting edge smeared"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_still_scene_keeps_its_bytes_with_motion_blur_on() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let held = Moves {
        steps: 3,
        ..Moves::default()
    };
    for hardware in [false, true] {
        let mut off = staged(&gpu, Moves::default(), true, hardware);
        if hardware && !off.hardware_rays() {
            continue;
        }
        let mut on = staged(&gpu, held, true, hardware);
        assert!(
            !on.moving(),
            "hardware {hardware}: a still scene compiled the motion path"
        );
        assert_eq!(
            traced(&mut off, &gpu),
            traced(&mut on, &gpu),
            "hardware {hardware}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn motion_blur_repeats_its_bytes_and_a_kept_trace_matches_a_fresh_stage() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let first = Moves {
        steps: 3,
        travel: 0.4,
        spin: 0.5,
        pan: 0.2,
        rise: 0.0,
        dense: 2000,
    };
    let second = Moves {
        travel: 0.8,
        spin: 1.2,
        pan: -0.3,
        ..first
    };
    for hardware in [false, true] {
        let mut kept = staged(&gpu, first, true, hardware);
        if hardware && !kept.hardware_rays() {
            continue;
        }
        let once = traced(&mut kept, &gpu);
        let mut again = staged(&gpu, first, true, hardware);
        assert_eq!(
            once,
            traced(&mut again, &gpu),
            "hardware {hardware}: not repeatable"
        );
        let posed = spinning::posed(second, WIDTH, HEIGHT);
        kept.set_instance_transforms(&gpu, &posed.models).unwrap();
        kept.set_motion(&gpu, &posed.motion, &posed.cameras)
            .unwrap();
        let moved = traced(&mut kept, &gpu);
        let mut fresh = staged(&gpu, second, true, hardware);
        assert_ne!(
            once, moved,
            "hardware {hardware}: the new motion changed nothing"
        );
        assert_eq!(
            moved,
            traced(&mut fresh, &gpu),
            "hardware {hardware}: kept and fresh differ"
        );
    }
}
