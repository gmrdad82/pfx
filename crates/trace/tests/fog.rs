#[path = "support/spinning.rs"]
mod spinning;
#[path = "support/street.rs"]
mod street;

use std::path::Path;

use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::scene::Scene;
use pfx_trace::Trace;
use pfx_trace::fog::Fog;
use pfx_trace::gpu::Output;
use street::{Haze, Street, fnv};

const PIN_SIZE: [u32; 2] = [320, 180];
const PIN_SAMPLES: u32 = 8;
const PIN_SEED: u32 = 3;
const NO_FOG_BYTES: [u64; 3] = [0xea2d73c26aaabe19, 0x6a8c884d6196274a, 0xcdd0c6a021c95aa7];
const NO_FOG_BYTES_RAY_QUERIES: [u64; 3] =
    [0xea2d73c26aaabe19, 0x6a8c884d6196274a, 0xffe10e179962895e];

static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn staged(gpu: &Gpu, scene: &Scene, size: [u32; 2], hardware: bool) -> Trace {
    let _env = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = std::env::var_os("PFX_TRACE_RT");
    unsafe { std::env::set_var("PFX_TRACE_RT", if hardware { "1" } else { "0" }) };
    let staged = pfx_trace::stage::scene(scene, size[0] as f32 / size[1] as f32).unwrap();
    let trace = staged.trace(gpu, size[0], size[1]);
    match before {
        Some(value) => unsafe { std::env::set_var("PFX_TRACE_RT", value) },
        None => unsafe { std::env::remove_var("PFX_TRACE_RT") },
    }
    trace.unwrap()
}

fn traced(gpu: &Gpu, trace: &mut Trace, samples: u32, seed: u32) -> Output {
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    for pass in 0..samples {
        trace
            .sample_paced(gpu, 1, seed.wrapping_add(pass), &mut pacer, |ms| {
                turns.add(ms)
            })
            .unwrap();
    }
    turns.turn();
    trace.readback(gpu).unwrap()
}

fn hashes(output: &Output) -> [u64; 3] {
    [fnv(&output.color), fnv(&output.albedo), fnv(&output.normal)]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_street_without_haze_traces_the_bytes_it_traced_before_fog() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = street::scene(base(), "pin", Street::default());
    let mut all = Vec::new();
    for hardware in [false, true] {
        let mut trace = staged(&gpu, &scene, PIN_SIZE, hardware);
        if hardware && !trace.hardware_rays() {
            println!("no ray queries on this adapter; the hardware path is skipped");
            continue;
        }
        let found = hashes(&traced(&gpu, &mut trace, PIN_SAMPLES, PIN_SEED));
        println!("hardware {hardware}: {found:#x?}");
        let pinned = if hardware {
            NO_FOG_BYTES_RAY_QUERIES
        } else {
            NO_FOG_BYTES
        };
        all.push((hardware, found, pinned));
    }
    for (hardware, found, pinned) in all {
        assert_eq!(found, pinned, "hardware {hardware}");
    }
}

fn base() -> &'static Path {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
}

fn fogged(lamp: bool) -> Street {
    Street {
        haze: Some(Haze::STREET),
        lamp,
        sun: street::SIDE_SUN,
        dark: false,
    }
}

#[test]
fn the_tracer_reads_the_haze_the_live_path_marches() {
    assert_eq!(
        (pfx_trace::fog::ABSORPTION, pfx_trace::fog::SCATTERING),
        (
            pfx_live::height_fog::FOG_ABSORPTION,
            pfx_live::height_fog::FOG_SCATTERING
        )
    );
    let scene = street::scene(base(), "keys", fogged(false));
    let fog = pfx_trace::stage::scene_fog(&scene, 0.0).unwrap();
    let haze = Haze::STREET;
    assert_eq!(
        (fog.density, fog.falloff, fog.reach, fog.phase, fog.color),
        (haze.fog, haze.falloff, haze.reach, haze.phase, haze.color)
    );
    let clear = street::scene(base(), "clear keys", Street::default());
    assert_eq!(pfx_trace::stage::scene_fog(&clear, 0.0), None);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_fogged_street_traces_the_same_bytes_twice_and_hands_the_denoiser_its_fog() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = street::scene(base(), "twice", fogged(true));
    let clear = street::scene(base(), "twice clear", fogged(true).cleared());
    for hardware in [false, true] {
        let mut first = staged(&gpu, &scene, PIN_SIZE, hardware);
        if hardware && !first.hardware_rays() {
            println!("no ray queries on this adapter; the hardware path is skipped");
            continue;
        }
        let a = traced(&gpu, &mut first, 4, 9);
        let mut second = staged(&gpu, &scene, PIN_SIZE, hardware);
        let b = traced(&gpu, &mut second, 4, 9);
        assert_eq!(hashes(&a), hashes(&b), "hardware {hardware}");
        let mut plain = staged(&gpu, &clear, PIN_SIZE, hardware);
        let c = traced(&gpu, &mut plain, 4, 9);
        let sky = |output: &Output| street::pixels(&output.albedo)[0];
        println!(
            "hardware {hardware}: the sky's albedo {:?} in fog, {:?} clear",
            sky(&a),
            sky(&c)
        );
        assert!(sky(&a)[..3].iter().all(|&v| v > 0.05 && v < 0.5));
        assert_eq!(sky(&c)[..3], [0.0; 3]);
    }
}

fn smear(image: &[f32], size: [u32; 2]) -> usize {
    let right = spinning::BODY[0] + spinning::BODY_SIZE[0] * 0.5;
    let y = spinning::BODY[1] - 0.3;
    let a = spinning::pixel(size[0], size[1], [right - 0.6, y, 0.0]);
    let b = spinning::pixel(size[0], size[1], [right + 0.8, y, 0.0]);
    let row = a[1].round() as u32;
    let profile: Vec<f32> = (a[0].round() as u32..=b[0].round() as u32)
        .map(|x| {
            (row - 8..=row + 8)
                .map(|y| image[(y * size[0] + x) as usize])
                .sum::<f32>()
                / 17.0
        })
        .collect();
    let plateau = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    let (inside, outside) = (
        plateau(&profile[..6]),
        plateau(&profile[profile.len() - 6..]),
    );
    let (low, span) = (inside.min(outside), (inside - outside).abs());
    profile
        .iter()
        .filter(|&&v| {
            let f = (v - low) / span;
            f > 0.1 && f < 0.9
        })
        .count()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_moving_body_still_smears_inside_fog() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let size = [960, 540];
    let fog = Fog {
        height: 0.0,
        falloff: 0.0,
        density: 0.05,
        reach: 40.0,
        shafts: 1.0,
        color: [1.0; 3],
        phase: 0.3,
        bounces: 0,
    };
    let moving = spinning::Moves {
        steps: 3,
        travel: 0.8,
        spin: 1.6,
        ..spinning::Moves::default()
    };
    let mut widths = Vec::new();
    for (moves, fog) in [
        (spinning::Moves::default(), Some(fog)),
        (moving, None),
        (moving, Some(fog)),
    ] {
        let mut trace = spinning::stage_fogged(&gpu, moves, false, size, 32, fog).unwrap();
        trace.sample(&gpu, 32, 7).unwrap();
        let image = spinning::luminance(&trace.readback_color(&gpu).unwrap());
        widths.push(smear(&image, size));
    }
    println!(
        "body edge: {} px sharp in fog, {} px blurred clear, {} px blurred in fog",
        widths[0], widths[1], widths[2]
    );
    assert!(widths[0] <= 3, "the still body blurs in fog");
    assert!(
        widths[2].abs_diff(widths[1]) <= widths[1] / 5 + 2,
        "fog changes the smear"
    );
}

const EMPTY_SIZE: [u32; 2] = [160, 96];
const EMPTY_TAN: f32 = 0.5;
const EMPTY_REACH: f32 = 20.0;
const EMPTY_SUN: [f32; 3] = [0.3, 0.4, -1.0];
const EMPTY_LAMP: [f32; 3] = [1.0, 0.5, -4.0];

fn unit(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn henyey_greenstein(cosine: f32, g: f32) -> f32 {
    let fac = 1.0 + g * g - 2.0 * g * cosine;
    (1.0 - g * g) / (4.0 * std::f32::consts::PI * fac * fac.sqrt())
}

fn empty_ray(x: f32, y: f32) -> [f32; 3] {
    let aspect = EMPTY_SIZE[0] as f32 / EMPTY_SIZE[1] as f32;
    let ndc = [
        x / EMPTY_SIZE[0] as f32 * 2.0 - 1.0,
        y / EMPTY_SIZE[1] as f32 * 2.0 - 1.0,
    ];
    unit([ndc[0] * EMPTY_TAN * aspect, -ndc[1] * EMPTY_TAN, -1.0])
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn lamp_light(
    lamp: &pfx_trace::lights::LocalLight,
    p: [f32; 3],
    d: [f32; 3],
    phase: f32,
    extinction: f32,
) -> f32 {
    use pfx_trace::lights::LightShape;
    let seen = |to: [f32; 3]| {
        let r = dot(to, to).sqrt();
        (
            r,
            henyey_greenstein(dot(d, to.map(|v| v / r)), phase) * (-extinction * r).exp(),
        )
    };
    match lamp.shape {
        LightShape::Rect { half_u, half_v, .. } => {
            let cells = 4;
            let normal = unit([
                half_u[1] * half_v[2] - half_u[2] * half_v[1],
                half_u[2] * half_v[0] - half_u[0] * half_v[2],
                half_u[0] * half_v[1] - half_u[1] * half_v[0],
            ]);
            let radiance = lamp.intensity / lamp.area();
            let cell = lamp.area() / (cells * cells) as f32;
            let mut total = 0.0;
            for i in 0..cells {
                for j in 0..cells {
                    let a = 2.0 * (i as f32 + 0.5) / cells as f32 - 1.0;
                    let b = 2.0 * (j as f32 + 0.5) / cells as f32 - 1.0;
                    let x: [f32; 3] = std::array::from_fn(|axis| {
                        lamp.position[axis] + half_u[axis] * a + half_v[axis] * b
                    });
                    let to = sub3(x, p);
                    let (r, lit) = seen(to);
                    let facing = -dot(normal, to) / r;
                    if facing > 0.0 {
                        total += radiance * facing * cell / (r * r) * lit;
                    }
                }
            }
            total
        }
        _ => {
            let to = sub3(lamp.position, p);
            let (r, lit) = seen(to);
            lamp.falloff(r) * lamp.cone(to.map(|v| -v)) * lit
        }
    }
}

fn integral(
    fog: &Fog,
    lamps: &[pfx_trace::lights::LocalLight],
    sun: f32,
    steps: usize,
    d: [f32; 3],
) -> f32 {
    let extinction = fog.extinction();
    let scattering = fog.albedo()[0] * extinction;
    let s = unit(EMPTY_SUN);
    let dt = EMPTY_REACH / steps as f32;
    let mut total = 0.0;
    for k in 0..steps {
        let t = (k as f32 + 0.5) * dt;
        let p = d.map(|v| v * t);
        let b = dot(p, s);
        let exit = -b + (b * b - (t * t - EMPTY_REACH * EMPTY_REACH)).sqrt();
        let lit_sun = sun * henyey_greenstein(dot(d, s), fog.phase) * (-extinction * exit).exp();
        let lit_lamps: f32 = lamps
            .iter()
            .map(|lamp| lamp_light(lamp, p, d, fog.phase, extinction))
            .sum();
        total += (-extinction * t).exp() * scattering * (lit_sun + lit_lamps) * dt;
    }
    total
}

fn integrals(
    fog: &Fog,
    lamps: &[pfx_trace::lights::LocalLight],
    sun: f32,
    steps: usize,
) -> Vec<f32> {
    let width = EMPTY_SIZE[0] as usize;
    let mut wanted = vec![0.0; width * EMPTY_SIZE[1] as usize];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = (EMPTY_SIZE[1] as usize).div_ceil(threads);
    std::thread::scope(|scope| {
        for (band, chunk) in wanted.chunks_mut(rows * width).enumerate() {
            scope.spawn(move || {
                for (i, value) in chunk.iter_mut().enumerate() {
                    let (x, y) = (i % width, band * rows + i / width);
                    *value = integral(
                        fog,
                        lamps,
                        sun,
                        steps,
                        empty_ray(x as f32 + 0.5, y as f32 + 0.5),
                    );
                }
            });
        }
    });
    wanted
}

fn empty_fog(
    gpu: &Gpu,
    fog: Fog,
    lamps: Vec<pfx_trace::lights::LocalLight>,
    sun: f32,
    sky: f32,
    samples: u32,
) -> Vec<[f32; 4]> {
    let aspect = EMPTY_SIZE[0] as f32 / EMPTY_SIZE[1] as f32;
    let scene = pfx_trace::Scene {
        triangles: vec![pfx_trace::bvh::Triangle {
            vertices: [
                [0.0, -500.0, 50.0],
                [1.0, -500.0, 50.0],
                [0.0, -500.0, 51.0],
            ],
            material: 0,
        }],
        shapes: Vec::new(),
        materials: vec![pfx_materials::Material {
            base: [0.0; 3],
            specular: 0.0,
            ..pfx_materials::Material::default()
        }],
        sky: pfx_load::Sky {
            width: 1,
            height: 1,
            texels: vec![[sky, sky, sky, 1.0]],
        },
        camera: pfx_trace::Camera {
            origin: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            right: [EMPTY_TAN * aspect, 0.0, 0.0],
            up: [0.0, EMPTY_TAN, 0.0],
        },
        sun: pfx_trace::Sun {
            direction: unit(EMPTY_SUN),
            color: [1.0; 3],
            intensity: sun,
        },
    };
    let detail = pfx_trace::detail::Detail {
        lights: lamps,
        fog: Some(fog),
        ..pfx_trace::detail::Detail::default()
    };
    let mut trace =
        Trace::new_detailed(gpu, &scene, &detail, EMPTY_SIZE[0], EMPTY_SIZE[1]).unwrap();
    street::pixels(&traced(gpu, &mut trace, samples, 3).color)
}

fn block_errors(pixels: &[[f32; 4]], wanted: &[f32]) -> Vec<([u32; 2], f32)> {
    let mut errors = Vec::new();
    for by in 0..EMPTY_SIZE[1] / 16 {
        for bx in 0..EMPTY_SIZE[0] / 16 {
            let (mut found, mut sum) = (0.0, 0.0);
            for y in by * 16..by * 16 + 16 {
                for x in bx * 16..bx * 16 + 16 {
                    let i = (y * EMPTY_SIZE[0] + x) as usize;
                    found += pixels[i][0];
                    sum += wanted[i];
                }
            }
            errors.push(([bx, by], (found / sum - 1.0).abs()));
        }
    }
    errors
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_fog_scatters_the_sun_and_a_lamp_as_their_integral_says() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let fog = empty_haze(0);
    let lamp = lamp(EMPTY_LAMP, 20.0, 1.0e4);
    let sun = 3.0;
    let aspect = EMPTY_SIZE[0] as f32 / EMPTY_SIZE[1] as f32;
    let pixels = empty_fog(&gpu, fog, vec![lamp], sun, 0.0, 512);
    let wanted = integrals(&fog, &[lamp], sun, 2000);
    let lamp_pixel = [
        (EMPTY_LAMP[0] / -EMPTY_LAMP[2] / (EMPTY_TAN * aspect) + 1.0) * 0.5 * EMPTY_SIZE[0] as f32,
        (1.0 - EMPTY_LAMP[1] / -EMPTY_LAMP[2] / EMPTY_TAN) * 0.5 * EMPTY_SIZE[1] as f32,
    ];
    let lamp_block = [lamp_pixel[0] as u32 / 16, lamp_pixel[1] as u32 / 16];
    let (mut worst, mut beside) = (0.0_f32, 0.0_f32);
    for (block, off) in block_errors(&pixels, &wanted) {
        if block == lamp_block {
            beside = off;
        } else {
            worst = worst.max(off);
        }
    }
    println!(
        "the traced fog is within {:.2}% of the integral in every 16 px block, {:.2}% in the lamp's",
        worst * 100.0,
        beside * 100.0
    );
    assert!(
        worst < 0.005,
        "the fog strays {:.2}% from its integral",
        worst * 100.0
    );
    assert!(
        beside < 0.05,
        "the lamp's glow strays {:.2}%",
        beside * 100.0
    );
}
fn empty_haze(bounces: u32) -> Fog {
    Fog {
        height: 0.0,
        falloff: 0.0,
        density: 0.05,
        reach: EMPTY_REACH,
        shafts: 1.0,
        color: [0.2; 3],
        phase: 0.5,
        bounces,
    }
}

fn lamp(position: [f32; 3], intensity: f32, range: f32) -> pfx_trace::lights::LocalLight {
    pfx_trace::lights::LocalLight::point(position, [1.0; 3], intensity, range)
}

fn spot(position: [f32; 3], direction: [f32; 3], intensity: f32) -> pfx_trace::lights::LocalLight {
    pfx_trace::lights::LocalLight {
        shape: pfx_trace::lights::LightShape::Spot {
            direction,
            inner_deg: 20.0,
            outer_deg: 35.0,
        },
        ..lamp(position, intensity, 1.0e4)
    }
}

fn panel(intensity: f32) -> pfx_trace::lights::LocalLight {
    pfx_trace::lights::LocalLight::rect(
        [0.0, 0.0, 3.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0; 3],
        intensity,
    )
}

fn tree_lamps() -> Vec<pfx_trace::lights::LocalLight> {
    vec![
        lamp([3.0, 1.0, 2.0], 15.0, 1.0e4),
        lamp([-3.0, 0.5, 1.0], 10.0, 1.0e4),
        lamp([0.0, 4.0, -6.0], 30.0, 1.0e4),
        lamp([6.0, -1.0, -3.0], 20.0, 8.0),
        lamp([-8.5, 2.0, -8.0], 25.0, 1.0e4),
        lamp([2.0, -4.0, -5.0], 12.0, 6.0),
        lamp([-1.0, 3.0, 4.0], 18.0, 1.0e4),
        lamp([11.0, 1.0, -12.0], 40.0, 1.0e4),
        lamp([-4.0, -3.0, 3.0], 8.0, 5.0),
        lamp([0.5, 6.0, -10.0], 35.0, 3.0),
        spot([0.0, 5.0, -4.0], [0.0, -1.0, -0.5], 40.0),
    ]
}

fn against_integral(pixels: &[[f32; 4]], wanted: &[f32]) -> (f32, f32) {
    let found: f32 = pixels.iter().map(|p| p[0]).sum();
    let mean = (found / wanted.iter().sum::<f32>() - 1.0).abs();
    let worst = block_errors(pixels, wanted)
        .into_iter()
        .map(|(_, off)| off)
        .fold(0.0_f32, f32::max);
    (mean, worst)
}

fn frame_mean(pixels: &[[f32; 4]]) -> f64 {
    pixels.iter().map(|p| p[0] as f64).sum::<f64>() / pixels.len() as f64
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn one_light_a_fog_segment_from_a_tree_of_lamps_keeps_the_integral_at_1024_spp() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let fog = empty_haze(0);
    let mut lamps = vec![panel(30.0)];
    lamps.extend(tree_lamps());
    let sun = 1.0;
    let pixels = empty_fog(&gpu, fog, lamps.clone(), sun, 0.0, 1024);
    let wanted = integrals(&fog, &lamps, sun, 1000);
    let (mean, worst) = against_integral(&pixels, &wanted);
    println!(
        "a sun, a rect and a tree of eleven lamps: the frame's mean is {:.3}% from the integral, every 16 px block within {:.2}%",
        mean * 100.0,
        worst * 100.0
    );
    assert!(mean < 0.005, "the fog's mean strays {:.3}%", mean * 100.0);
    assert!(worst < 0.03, "a block strays {:.2}%", worst * 100.0);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_light_tree_centred_on_the_camera_still_lights_the_fog_it_starts_in() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let fog = empty_haze(0);
    let lamps = vec![
        lamp([4.0, 0.0, 1.0], 20.0, 1.0e4),
        lamp([-4.0, 0.0, -1.0], 15.0, 1.0e4),
        spot([0.0, 3.0, 0.5], [0.0, -1.0, -1.0], 40.0),
        lamp([0.0, -3.0, -0.5], 10.0, 1.0e4),
        lamp([5.0, -1.0, 2.0], 25.0, 1.0e4),
        lamp([-5.0, 1.0, -2.0], 12.0, 1.0e4),
    ];
    let sun = 1.0;
    let pixels = empty_fog(&gpu, fog, lamps.clone(), sun, 0.0, 1024);
    let wanted = integrals(&fog, &lamps, sun, 1000);
    let (mean, worst) = against_integral(&pixels, &wanted);
    println!(
        "a sun and a tree of six lamps whose bounds are centred on the camera: the frame's mean is {:.3}% from the integral, every 16 px block within {:.2}%",
        mean * 100.0,
        worst * 100.0
    );
    assert!(mean < 0.005, "the fog's mean strays {:.3}%", mean * 100.0);
    assert!(worst < 0.03, "a block strays {:.2}%", worst * 100.0);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn fog_bounces_light_a_shared_pick_as_the_sum_of_its_lights_alone() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let fog = empty_haze(2);
    let (sun, sky) = (1.0, 0.05);
    let mut everything = vec![panel(120.0)];
    everything.extend(tree_lamps());
    let whole = frame_mean(&empty_fog(&gpu, fog, everything, sun, sky, 1024));
    let parts = [
        ("the sun and the sky", Vec::new(), sun, sky),
        ("the rect", vec![panel(120.0)], 0.0, 0.0),
        ("the tree", tree_lamps(), 0.0, 0.0),
    ]
    .map(|(name, lamps, sun, sky)| {
        (
            name,
            frame_mean(&empty_fog(&gpu, fog, lamps, sun, sky, 1024)),
        )
    });
    let sum: f64 = parts.iter().map(|(_, mean)| mean).sum();
    let off = (whole / sum - 1.0).abs();
    println!(
        "two fog bounces: every light together {whole:.6}, alone {parts:?}, summed {sum:.6}, {:.3}% apart",
        off * 100.0
    );
    assert!(parts.iter().all(|(_, mean)| *mean > 0.0));
    assert!(
        off < 0.005,
        "the lights together stray {:.3}% from their sum",
        off * 100.0
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_traced_fog_looks_like_the_live_fog() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let size = [640, 360];
    let street = Street {
        dark: true,
        ..fogged(false)
    };
    let scene = street::scene(base(), "live", street);
    let live: Vec<[f32; 4]> = street::live(&gpu, &scene, size)
        .into_iter()
        .map(|p| [p[0], p[1], p[2], 1.0])
        .collect();
    let mut trace = staged(&gpu, &scene, size, false);
    let traced = street::pixels(&traced(&gpu, &mut trace, 128, 5).color);
    let (live, traced) = (street::levels(&live), street::levels(&traced));
    let (pixels, blocks) = (
        street::apart(&live, &traced),
        street::apart(
            &street::blocks(&live, size, 8),
            &street::blocks(&traced, size, 8),
        ),
    );
    println!(
        "the fog alone: live and traced {pixels:.1} levels apart a pixel, {blocks:.1} an 8 px block"
    );
    assert!(blocks < 8.0, "the fog differs by {blocks:.1} levels");
}
