use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_materials::{Material, Shaded, eval};
use pfx_trace::adaptive::Adaptive;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::Detail;
use pfx_trace::lights::LocalLight;
use pfx_trace::matte::Matte;
use pfx_trace::probe::{Probe, texels};
use pfx_trace::rig::Rig;
use pfx_trace::shapes::Shape;
use pfx_trace::{Camera, Projection, Scene, Sun, Trace};

fn quad(corners: [[f32; 3]; 4], material: u32) -> [Triangle; 2] {
    [
        Triangle {
            vertices: [corners[0], corners[1], corners[2]],
            material,
        },
        Triangle {
            vertices: [corners[0], corners[2], corners[3]],
            material,
        },
    ]
}

fn floor(half: f32, material: u32) -> [Triangle; 2] {
    quad(
        [
            [-half, 0.0, -half],
            [half, 0.0, -half],
            [half, 0.0, half],
            [-half, 0.0, half],
        ],
        material,
    )
}

fn flat_sky(value: f32) -> Sky {
    Sky {
        width: 1,
        height: 1,
        texels: vec![[value, value, value, 1.0]],
    }
}

fn no_sun() -> Sun {
    Sun {
        direction: [0.0, 1.0, 0.0],
        color: [0.0; 3],
        intensity: 0.0,
    }
}

fn above(height: f32) -> Camera {
    Camera {
        origin: [0.0, height, 0.0],
        forward: [0.0, -1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, -1.0],
    }
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn sample(trace: &mut Trace, gpu: &Gpu, samples: u32, seed: u32) {
    let mut done = 0;
    while done < samples {
        let count = (samples - done).min(64);
        trace.sample(gpu, count, seed).unwrap();
        done += count;
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn disc_lights_give_their_irradiance() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let ground = Material {
        base: [0.6, 0.6, 0.6],
        roughness: 0.7,
        ..Material::default()
    };
    let scene = Scene {
        triangles: floor(4.0, 0).to_vec(),
        shapes: Vec::new(),
        materials: vec![ground],
        sky: flat_sky(0.0),
        camera: Camera {
            origin: [0.0, 4.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [0.5, 0.0, 0.0],
            up: [0.0, 0.0, -0.5],
        },
        sun: no_sun(),
    };
    let disc = LocalLight::disc(
        [0.3, 1.2, -0.2],
        [0.0, -1.0, 0.0],
        0.4,
        [1.0, 0.9, 0.8],
        2.5,
    );
    let detail = Detail {
        lights: vec![disc],
        ..Detail::default()
    };
    let size = 24u32;
    let mut trace = Trace::new_detailed(&gpu, &scene, &detail, size, size).unwrap();
    sample(&mut trace, &gpu, 8192, 5);
    let traced = texels(&trace.readback(&gpu).unwrap().color);
    let shaded = Shaded::from_material(&ground);
    let radiance = disc.colour.map(|c| c * disc.intensity / disc.area());
    let mut worst = 0.0f32;
    for y in 0..size {
        for x in 0..size {
            let mut expected = [0.0f64; 3];
            for sy in 0..4 {
                for sx in 0..4 {
                    let ndc_x = (x as f32 + (sx as f32 + 0.5) / 4.0) / size as f32 * 2.0 - 1.0;
                    let ndc_y = (y as f32 + (sy as f32 + 0.5) / 4.0) / size as f32 * 2.0 - 1.0;
                    let ray = [0.5 * ndc_x, -1.0, 0.5 * ndc_y];
                    let l = length(ray);
                    let ray = ray.map(|v| v / l);
                    let at = [4.0 * ray[0] / -ray[1], 0.0, 4.0 * ray[2] / -ray[1]];
                    let wo = [-ray[2], -ray[0], -ray[1]];
                    let rings = 32;
                    let spokes = 64;
                    for ring in 0..rings {
                        let r0 = disc_radius(ring, rings, 0.4);
                        let r1 = disc_radius(ring + 1, rings, 0.4);
                        let r = 0.5 * (r0 + r1);
                        let area = std::f32::consts::PI * (r1 * r1 - r0 * r0) / spokes as f32;
                        for spoke in 0..spokes {
                            let phi = (spoke as f32 + 0.5) / spokes as f32 * std::f32::consts::TAU;
                            let point = [0.3 + r * phi.cos(), 1.2, -0.2 + r * phi.sin()];
                            let to = [point[0] - at[0], point[1] - at[1], point[2] - at[2]];
                            let d2 = to[0] * to[0] + to[1] * to[1] + to[2] * to[2];
                            let d = d2.sqrt();
                            let dir = to.map(|v| v / d);
                            let brdf = eval(&shaded, wo, [dir[2], dir[0], dir[1]]);
                            let weight = dir[1] * dir[1] * area / d2;
                            for c in 0..3 {
                                expected[c] += f64::from(brdf[c] * radiance[c] * weight) / 16.0;
                            }
                        }
                    }
                }
            }
            let got = traced[(y * size + x) as usize];
            for c in 0..3 {
                let error = (f64::from(got[c]) / expected[c] - 1.0).abs() as f32;
                worst = worst.max(error);
                assert!(
                    error < 0.03,
                    "{x},{y} channel {c}: {} against {}",
                    got[c],
                    expected[c]
                );
            }
        }
    }
    eprintln!("disc light: every pixel within {worst:.4} of the integrated irradiance");
}

fn disc_radius(ring: u32, rings: u32, radius: f32) -> f32 {
    radius * (ring as f32 / rings as f32).sqrt()
}

fn catcher_scene() -> (Scene, Detail) {
    let scene = Scene {
        triangles: floor(1.2, 0).to_vec(),
        shapes: vec![Shape::Ellipsoid {
            center: [0.0, 0.6, 0.0],
            radii: [0.5; 3],
            material: 1,
        }],
        materials: vec![
            Material {
                base: [0.8; 3],
                roughness: 0.9,
                ..Material::default()
            },
            Material {
                base: [0.7, 0.4, 0.2],
                roughness: 0.4,
                ..Material::default()
            },
        ],
        sky: flat_sky(0.05),
        camera: above(5.0),
        sun: no_sun(),
    };
    let to_origin = [-2.0f32, -3.0, 0.0];
    let l = length(to_origin);
    let detail = Detail {
        lights: vec![LocalLight::disc(
            [2.0, 3.0, 0.0],
            to_origin.map(|v| v / l),
            0.3,
            [1.0; 3],
            40.0,
        )],
        projection: Projection::Orthographic {
            width: 4.0,
            height: 4.0,
        },
        ..Detail::default()
    };
    (scene, detail)
}

const SIZE: u32 = 40;

fn pixel_at(x: f32, z: f32) -> usize {
    let px = ((x + 2.0) / 4.0 * SIZE as f32) as usize;
    let py = ((z + 2.0) / 4.0 * SIZE as f32) as usize;
    py * SIZE as usize + px
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_shadow_catcher_keeps_only_its_shadow_in_alpha() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (scene, detail) = catcher_scene();
    let mut plain = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
    sample(&mut plain, &gpu, 64, 9);
    let opaque = texels(&plain.readback(&gpu).unwrap().color);
    assert!(opaque.iter().all(|t| t[3] == 1.0));
    let mut trace = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
    assert!(trace.set_matte(Matte::catcher(2)).is_err());
    assert!(
        trace
            .set_matte(Matte {
                transparent: false,
                catcher: Some(0)
            })
            .is_err()
    );
    trace.set_matte(Matte::catcher(0)).unwrap();
    assert_eq!(trace.matte(), Matte::catcher(0));
    sample(&mut trace, &gpu, 512, 9);
    let image = texels(&trace.readback(&gpu).unwrap().color);
    let ball = image[pixel_at(0.0, 0.0)];
    let shadow = image[pixel_at(-0.8, 0.0)];
    let lit = image[pixel_at(1.0, 0.6)];
    let sky = image[pixel_at(-1.8, -1.8)];
    assert!((ball[3] - 1.0).abs() < 1e-6 && ball[0] > 0.05, "{ball:?}");
    assert!(shadow[3] > 0.7, "{shadow:?}");
    assert_eq!(shadow[..3], [0.0; 3]);
    assert!(lit[3] < 0.03, "{lit:?}");
    assert_eq!(lit[..3], [0.0; 3]);
    assert_eq!(sky, [0.0; 4]);
    trace.set_matte(Matte::transparent()).unwrap();
    sample(&mut trace, &gpu, 16, 9);
    let film = texels(&trace.readback(&gpu).unwrap().color);
    assert_eq!(film[pixel_at(1.0, 0.6)][3], 1.0);
    assert!(film[pixel_at(1.0, 0.6)][0] > 0.0);
    assert_eq!(film[pixel_at(-1.8, -1.8)], [0.0; 4]);
    trace.set_matte(Matte::default()).unwrap();
    sample(&mut trace, &gpu, 64, 9);
    let back = texels(&trace.readback(&gpu).unwrap().color);
    assert_eq!(back, opaque);
    eprintln!(
        "catcher: ball {:.3}, shadow alpha {:.3}, lit floor alpha {:.4}",
        ball[3], shadow[3], lit[3]
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_probe_reads_depth_id_and_normal_at_pixel_centres() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (scene, detail) = catcher_scene();
    let trace = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
    let probe = trace.probe(&gpu).unwrap();
    assert_eq!((probe.width, probe.height), (SIZE, SIZE));
    let centre = (SIZE / 2 * SIZE + SIZE / 2) as usize;
    assert_eq!(probe.id[centre], Probe::material_id(1));
    let top = 5.0 - 1.1;
    assert!(
        (probe.depth[centre] - top).abs() < 0.01,
        "{}",
        probe.depth[centre]
    );
    let ground = pixel_at(1.0, 0.6);
    assert_eq!(probe.id[ground], Probe::material_id(0));
    assert!((probe.depth[ground] - 5.0).abs() < 1e-4);
    assert!((probe.normal[ground][1] - 1.0).abs() < 1e-4);
    assert_eq!(probe.id[pixel_at(-1.8, -1.8)], 0);
    assert_eq!(probe.depth[pixel_at(-1.8, -1.8)], 0.0);
    assert_eq!(probe.without(&[Probe::material_id(0)])[ground], 0);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn adaptive_sampling_stops_where_the_image_is_clean() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (scene, detail) = catcher_scene();
    let mut fixed = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
    fixed.set_matte(Matte::catcher(0)).unwrap();
    sample(&mut fixed, &gpu, 1024, 21);
    let reference = texels(&fixed.readback(&gpu).unwrap().color);
    let mut trace = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
    trace.set_matte(Matte::catcher(0)).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let adaptive = Adaptive {
        threshold: 0.02,
        min_samples: 16,
        max_samples: 1024,
        growth: 1.5,
    };
    let converged = trace
        .sample_adaptive(&gpu, adaptive, 21, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let image = texels(&trace.readback(&gpu).unwrap().color);
    let mean = converged.mean_samples();
    eprintln!(
        "adaptive: {} rounds, mean {mean:.1} samples, last {} with {} pixels running",
        converged.rounds.len(),
        converged.max_samples(),
        converged.rounds.last().unwrap().active
    );
    assert!(mean < f64::from(converged.max_samples()), "{mean}");
    assert!(
        converged.rounds[1].active < converged.pixels / 2,
        "{:?}",
        converged.rounds
    );
    let sky = pixel_at(-1.8, -1.8);
    assert_eq!(image[sky], [0.0; 4]);
    let block = |img: &[[f32; 4]], cx: f32, cz: f32, c: usize| {
        let mut sum = 0.0;
        for dz in -2..=2 {
            for dx in -2..=2 {
                sum += img[pixel_at(cx + dx as f32 * 0.1, cz + dz as f32 * 0.1)][c];
            }
        }
        sum / 25.0
    };
    for (cx, cz) in [(0.0, 0.0), (-0.8, 0.0), (0.2, 0.25)] {
        for c in 0..4 {
            let want = block(&reference, cx, cz, c);
            let got = block(&image, cx, cz, c);
            assert!(
                (got - want).abs() <= 0.03 * want.abs() + 0.01,
                "({cx}, {cz}) channel {c}: {got} against {want}"
            );
        }
    }
    let mut again = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
    again.set_matte(Matte::catcher(0)).unwrap();
    let mut pacer = Pacer::default();
    let repeat = again
        .sample_adaptive(&gpu, adaptive, 21, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    assert_eq!(repeat.rounds, converged.rounds);
    assert_eq!(texels(&again.readback(&gpu).unwrap().color), image);
}

fn icon() -> (Scene, Detail) {
    let rig = Rig::default();
    let metal = Material {
        base: [0.262, 0.279, 0.305],
        metalness: 1.0,
        roughness: 0.22,
        ..Material::default()
    };
    let paint = Material {
        base: [0.6, 0.12, 0.08],
        roughness: 0.35,
        clearcoat: 0.6,
        ..Material::default()
    };
    let ground = Material {
        base: [0.8; 3],
        roughness: 0.9,
        ..Material::default()
    };
    let scene = Scene {
        triangles: floor(6.0, 2).to_vec(),
        shapes: vec![
            Shape::RoundedBox {
                center: [0.0, 0.09, 0.0],
                half: [0.62, 0.09, 0.62],
                radius: 0.06,
                material: 0,
            },
            Shape::Capsule {
                a: [-0.32, 0.28, 0.12],
                b: [0.32, 0.28, -0.12],
                radius: 0.12,
                material: 1,
            },
        ],
        materials: vec![metal, paint, ground],
        sky: rig.sky(),
        camera: Camera {
            origin: [0.0, 2.4, 2.4],
            forward: [
                0.0,
                -std::f32::consts::FRAC_1_SQRT_2,
                -std::f32::consts::FRAC_1_SQRT_2,
            ],
            right: [1.0, 0.0, 0.0],
            up: [
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
                -std::f32::consts::FRAC_1_SQRT_2,
            ],
        },
        sun: no_sun(),
    };
    let detail = Detail {
        lights: rig.lights(),
        projection: Projection::Orthographic {
            width: 1.8,
            height: 1.8,
        },
        ..Detail::default()
    };
    (scene, detail)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_clean_icon_at_512() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (scene, detail) = icon();
    let size = 512;
    let mut trace = Trace::new_detailed(&gpu, &scene, &detail, size, size).unwrap();
    trace.set_matte(Matte::catcher(2)).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let adaptive = Adaptive {
        threshold: 0.005,
        min_samples: 32,
        max_samples: 1024,
        growth: 1.5,
    };
    let converged = trace
        .sample_adaptive(&gpu, adaptive, 7, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    let image = texels(&trace.readback(&gpu).unwrap().color);
    let busy: f64 = converged.stats.milliseconds.iter().sum();
    eprintln!(
        "icon 512: mean {:.1} samples, cap {} reached by {} pixels, {} submissions, longest {:.2} ms, GPU {:.0} ms, wall {:.0} ms",
        converged.mean_samples(),
        converged.max_samples(),
        converged.rounds.last().unwrap().active,
        converged.stats.count(),
        converged.stats.longest_ms(),
        busy,
        converged.stats.wall_ms
    );
    for round in &converged.rounds {
        eprintln!(
            "  {} samples: {} pixels running",
            round.samples, round.active
        );
    }
    let counts = trace.sample_counts(&gpu).unwrap();
    let probe = trace.probe(&gpu).unwrap();
    let ids = probe.without(&[Probe::material_id(2)]);
    let split = |object: bool| {
        let picked: Vec<u32> = counts
            .iter()
            .zip(&ids)
            .filter(|(_, id)| (**id != 0) == object)
            .map(|(count, _)| *count)
            .collect();
        picked.iter().map(|&c| f64::from(c)).sum::<f64>() / picked.len().max(1) as f64
    };
    eprintln!(
        "  objects {:.1} samples, catcher and background {:.1}",
        split(true),
        split(false)
    );
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let finished = pfx_post::view::finish(&image, pfx_post::View::Standard, 0.0);
    let rgba: Vec<u8> = pfx_post::view::delivery8(&finished, size)
        .into_iter()
        .flatten()
        .collect();
    write_png(&out.join("icon.png"), size, size, &rgba);
    let heat: Vec<u8> = counts
        .iter()
        .flat_map(|&c| {
            let v = (255.0 * f64::from(c) / f64::from(adaptive.max_samples)).round() as u8;
            [v, v, v, 255]
        })
        .collect();
    write_png(&out.join("icon-samples.png"), size, size, &heat);
    eprintln!("  wrote {}", out.join("icon.png").display());
    let covered = image.iter().filter(|t| t[3] > 0.99).count();
    assert!(covered > (size * size / 8) as usize, "{covered}");
    assert!(image.iter().all(|t| t.iter().all(|v| v.is_finite())));
    assert!(converged.mean_samples() < f64::from(adaptive.max_samples));
}

fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(rgba)
        .unwrap();
}

fn over_grey(image: &[[f32; 4]]) -> Vec<f32> {
    let grey = pfx_post::view::srgb_decode(0.5);
    image
        .iter()
        .flat_map(|t| {
            let composite = pfx_trace::matte::over(*t, [grey; 3]);
            [0, 1, 2].map(|c| 255.0 * pfx_post::view::srgb_encode(composite[c].clamp(0.0, 1.0)))
        })
        .collect()
}

fn gap(a: &[f32], b: &[f32]) -> (f32, f32) {
    let mut errors: Vec<f32> = a.iter().zip(b).map(|(x, y)| (x - y).abs()).collect();
    let rms = (errors.iter().map(|e| e * e).sum::<f32>() / errors.len() as f32).sqrt();
    errors.sort_by(f32::total_cmp);
    (rms, errors[errors.len() * 999 / 1000])
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn clean_icon_budget() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (scene, detail) = icon();
    let size = 512;
    let mut reference = Trace::new_detailed(&gpu, &scene, &detail, size, size).unwrap();
    reference.set_matte(Matte::catcher(2)).unwrap();
    sample(&mut reference, &gpu, 16384, 99);
    let truth = over_grey(&texels(&reference.readback(&gpu).unwrap().color));
    let ids = reference
        .probe(&gpu)
        .unwrap()
        .without(&[Probe::material_id(2)]);
    let region = |image: &[f32], object: bool| {
        let pick = |values: &[f32]| -> Vec<f32> {
            values
                .chunks(3)
                .zip(&ids)
                .filter(|(_, id)| (**id != 0) == object)
                .flat_map(|(v, _)| v.to_vec())
                .collect()
        };
        gap(&pick(image), &pick(&truth))
    };
    let mut turns = Turns::default();
    eprintln!(
        "threshold, cap | mean samples | GPU ms | objects rms / p99.9 | catcher rms / p99.9 (8-bit over grey, against 16384 samples)"
    );
    for (threshold, cap) in [(0.01, 1024), (0.01, 2048), (0.01, 4096), (0.005, 4096)] {
        let mut trace = Trace::new_detailed(&gpu, &scene, &detail, size, size).unwrap();
        trace.set_matte(Matte::catcher(2)).unwrap();
        let mut pacer = Pacer::default();
        let adaptive = Adaptive {
            threshold,
            min_samples: 32,
            max_samples: cap,
            growth: 1.5,
        };
        let converged = trace
            .sample_adaptive(&gpu, adaptive, 7, &mut pacer, |ms| turns.add(ms))
            .unwrap();
        let image = over_grey(&texels(&trace.readback(&gpu).unwrap().color));
        let busy: f64 = converged.stats.milliseconds.iter().sum();
        let (objects, catcher) = (region(&image, true), region(&image, false));
        eprintln!(
            "{threshold}, {cap} | {:.1} | {busy:.0} | {:.3} / {:.2} | {:.3} / {:.2}",
            converged.mean_samples(),
            objects.0,
            objects.1,
            catcher.0,
            catcher.1
        );
        assert!(objects.0 < 4.0 && catcher.0 < 2.0);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_catcher_follows_transmissive_shadows() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let (mut scene, detail) = catcher_scene();
    scene.materials[1] = Material {
        base: [0.9; 3],
        roughness: 0.05,
        transmission: 1.0,
        ior: 1.5,
        ..Material::default()
    };
    let shadow = |transmissive: bool| {
        let detail = Detail {
            transmissive_shadows: transmissive,
            ..detail.clone()
        };
        let mut trace = Trace::new_detailed(&gpu, &scene, &detail, SIZE, SIZE).unwrap();
        trace.set_matte(Matte::catcher(0)).unwrap();
        sample(&mut trace, &gpu, 256, 13);
        let image = texels(&trace.readback(&gpu).unwrap().color);
        (image[pixel_at(-0.8, 0.0)], image[pixel_at(1.0, 0.6)])
    };
    let (opaque, opaque_lit) = shadow(false);
    let (glass, glass_lit) = shadow(true);
    eprintln!(
        "glass ball over the catcher: shadow alpha {:.3} opaque, {:.3} transmissive",
        opaque[3], glass[3]
    );
    assert!(opaque[3] > 0.7, "{opaque:?}");
    assert!(glass[3] < 0.3, "{glass:?}");
    assert!(opaque_lit[3] < 0.03 && glass_lit[3] < 0.03);
    assert_eq!(glass[..3], [0.0; 3]);
}
