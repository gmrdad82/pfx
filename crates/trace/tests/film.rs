use pfx_gpu::Gpu;
use pfx_gpu::pace::{Pacer, Turns};
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::detail::{Bounces, Detail};
use pfx_trace::film::{film_reflectance, srgb_byte};
use pfx_trace::lights::LocalLight;
use pfx_trace::{Camera, Scene, Sun, Trace};

const SIDE: u32 = 8;

fn white_sky() -> Sky {
    Sky {
        width: 4,
        height: 2,
        texels: vec![[1.0, 1.0, 1.0, 1.0]; 8],
    }
}

fn patch_scene(angle_deg: f32, thickness: f32, ior: f32, film_ior: f32) -> Scene<Sky> {
    let film = Material {
        base: [1.0; 3],
        roughness: 0.02,
        transmission: 1.0,
        ior,
        thin_film: thickness,
        thin_film_ior: film_ior,
        thin_film_amount: 1.0,
        ..Material::default()
    };
    let black = Material {
        base: [0.0; 3],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    let angle = angle_deg.to_radians();
    let (sin, cos) = angle.sin_cos();
    let edge = 20.0;
    let corner = |x: f32, y: f32| [x, y * cos, y * sin];
    let behind = |x: f32, y: f32| {
        let reach = 1.5;
        let p = corner(x * reach, y * reach);
        [p[0], p[1] + 0.5 * sin, p[2] - 0.5 * cos]
    };
    Scene {
        triangles: vec![
            Triangle {
                vertices: [
                    corner(-edge, -edge),
                    corner(edge, -edge),
                    corner(edge, edge),
                ],
                material: 0,
            },
            Triangle {
                vertices: [
                    corner(-edge, -edge),
                    corner(edge, edge),
                    corner(-edge, edge),
                ],
                material: 0,
            },
            Triangle {
                vertices: [
                    behind(-edge, -edge),
                    behind(edge, -edge),
                    behind(edge, edge),
                ],
                material: 1,
            },
            Triangle {
                vertices: [
                    behind(-edge, -edge),
                    behind(edge, edge),
                    behind(-edge, edge),
                ],
                material: 1,
            },
        ],
        shapes: Vec::new(),
        materials: vec![film, black],
        sky: white_sky(),
        camera: Camera {
            origin: [0.0, 0.0, 40.0],
            forward: [0.0, 0.0, -1.0],
            right: [0.05, 0.0, 0.0],
            up: [0.0, 0.05, 0.0],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
    }
}

fn patch_pixels(
    gpu: &Gpu,
    scene: &Scene<Sky>,
    lights: Vec<LocalLight>,
    passes: u32,
) -> Vec<[f32; 3]> {
    seeded_pixels(gpu, scene, lights, passes, 0)
}

fn seeded_pixels(
    gpu: &Gpu,
    scene: &Scene<Sky>,
    lights: Vec<LocalLight>,
    passes: u32,
    first_seed: u32,
) -> Vec<[f32; 3]> {
    let detail = Detail {
        lights,
        ..Detail::default()
    };
    let mut trace = Trace::new_detailed(gpu, scene, &detail, SIDE, SIDE).unwrap();
    for pass in 0..passes {
        trace.sample(gpu, 32, first_seed + pass).unwrap();
    }
    let output = trace.readback(gpu).unwrap();
    (0..(SIDE * SIDE) as usize)
        .map(|pixel| {
            std::array::from_fn(|channel| {
                let at = pixel * 16 + channel * 4;
                f32::from_le_bytes(output.color[at..at + 4].try_into().unwrap())
            })
        })
        .collect()
}

fn mean(pixels: &[[f32; 3]]) -> [f32; 3] {
    std::array::from_fn(|channel| {
        pixels.iter().map(|p| p[channel] as f64).sum::<f64>() as f32 / pixels.len() as f32
    })
}

fn patch_colour(
    gpu: &Gpu,
    angle_deg: f32,
    thickness: f32,
    ior: f32,
    film_ior: f32,
    passes: u32,
) -> [f32; 3] {
    let scene = patch_scene(angle_deg, thickness, ior, film_ior);
    mean(&patch_pixels(gpu, &scene, Vec::new(), passes))
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn film_curve_dump() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    println!("angle_deg,thickness_nm,r,g,b");
    for angle in [0.0_f32, 45.0, 70.0] {
        for step in 0..=40 {
            let thickness = step as f32 * 10.0;
            let [r, g, b] = patch_colour(&gpu, angle, thickness, 1.4, 1.9, 256);
            println!("{angle},{thickness},{r:.6},{g:.6},{b:.6}");
        }
    }
}

fn table() -> Vec<(f32, f32, [f32; 3])> {
    include_str!("../film/cycles.csv")
        .lines()
        .skip(1)
        .map(|line| {
            let v: Vec<f32> = line.split(',').map(|x| x.parse().unwrap()).collect();
            (v[0], v[1], [v[2], v[3], v[4]])
        })
        .collect()
}

fn worst_byte_error(ours: [f32; 3], theirs: [f32; 3]) -> f32 {
    (0..3)
        .map(|c| (srgb_byte(ours[c]) - srgb_byte(theirs[c])).abs())
        .fold(0.0, f32::max)
}

#[test]
fn film_reflectance_matches_cycles_within_two_bytes() {
    let mut worst = 0.0_f32;
    for (angle, thickness, cycles) in table() {
        if thickness == 0.0 {
            continue;
        }
        let ours = film_reflectance(angle.to_radians().cos(), thickness, 1.0, 1.9, 1.4);
        let error = worst_byte_error(ours, cycles);
        assert!(
            error <= 2.0,
            "{angle} deg, {thickness} nm: {ours:?} against {cycles:?}, {error}"
        );
        worst = worst.max(error);
    }
    println!("worst sRGB byte difference {worst}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn patch_at_180_nm_matches_cycles() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let rows: Vec<_> = table().into_iter().filter(|r| r.1 == 180.0).collect();
    assert_eq!(rows.len(), 3);
    for (angle, thickness, cycles) in rows {
        let ours = patch_colour(&gpu, angle, thickness, 1.4, 1.9, 256);
        let error = worst_byte_error(ours, cycles);
        println!("{angle} deg: tracer {ours:?}, cycles {cycles:?}, {error}");
        assert!(error <= 2.0, "{angle} deg: {error}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn plain_transmissive_surface_reflects_once_like_cycles() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let rows: Vec<_> = table().into_iter().filter(|r| r.1 == 0.0).collect();
    assert_eq!(rows.len(), 3);
    for (angle, thickness, cycles) in rows {
        let ours = patch_colour(&gpu, angle, thickness, 1.4, 1.9, 256);
        let error = worst_byte_error(ours, cycles);
        println!("{angle} deg: tracer {ours:?}, cycles {cycles:?}, {error}");
        assert!(error <= 2.0, "{angle} deg: {error}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_point_light_still_highlights_glass() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = patch_scene(0.0, 0.0, 1.4, 1.9);
    let light = LocalLight::point([0.0, 0.0, 10.0], [1.0; 3], 50.0, 100.0);
    let plain = patch_pixels(&gpu, &scene, Vec::new(), 64);
    let lit = patch_pixels(&gpu, &scene, vec![light], 64);
    let peak = |pixels: &[[f32; 3]]| pixels.iter().map(|p| p[1]).fold(0.0_f32, f32::max);
    println!("peak without {}, with {}", peak(&plain), peak(&lit));
    assert!(peak(&lit) > peak(&plain) + 0.05);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_rect_light_on_glass_is_reflected_once() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = patch_scene(0.0, 0.0, 1.4, 1.9);
    let rect = LocalLight::rect(
        [0.0, 0.0, 10.0],
        [0.0, 5.0, 0.0],
        [5.0, 0.0, 0.0],
        [1.0; 3],
        100.0,
    );
    let ours = mean(&patch_pixels(&gpu, &scene, vec![rect], 256));
    let sky_only = mean(&patch_pixels(&gpu, &scene, Vec::new(), 256));
    let reflectance = sky_only[1];
    let expected = [reflectance * 2.0; 3];
    println!("tracer {ours:?}, expected {expected:?}, sky alone {sky_only:?}");
    assert!(worst_byte_error(ours, expected) <= 2.0);
}

fn plane_seen_from(gpu: &Gpu, height: f32, level: f32) -> [f32; 3] {
    let table = Material {
        base: [0.86, 0.85, 0.9],
        roughness: 0.45,
        specular: 0.028,
        ..Material::default()
    };
    let edge = 40.0 * height;
    let spread = 0.4 * height / 40.0;
    let corner = |x: f32, z: f32| [x, level, z];
    let scene = Scene {
        triangles: vec![
            Triangle {
                vertices: [
                    corner(-edge, -edge),
                    corner(edge, -edge),
                    corner(edge, edge),
                ],
                material: 0,
            },
            Triangle {
                vertices: [
                    corner(-edge, -edge),
                    corner(edge, edge),
                    corner(-edge, edge),
                ],
                material: 0,
            },
        ],
        shapes: Vec::new(),
        materials: vec![table],
        sky: white_sky(),
        camera: Camera {
            origin: [0.0, height, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [spread * 0.01, 0.0, 0.0],
            up: [0.0, 0.0, spread * 0.01],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
    };
    mean(&patch_pixels(gpu, &scene, Vec::new(), 128))
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_far_camera_does_not_light_a_plane_with_itself() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let near = plane_seen_from(&gpu, 40.0, -0.05);
    let far = plane_seen_from(&gpu, 2600.0, -0.05);
    println!("near {near:?}, far {far:?}");
    for channel in 0..3 {
        assert!(
            (far[channel] / near[channel] - 1.0).abs() < 0.01,
            "{near:?} against {far:?}"
        );
    }
}

fn glass() -> Material {
    Material {
        base: [1.0; 3],
        roughness: 0.02,
        transmission: 1.0,
        ior: 1.4,
        ..Material::default()
    }
}

#[derive(Clone, Copy)]
struct Sheet {
    albedo: f32,
    ior: f32,
    height: f32,
    slab: f32,
    sphere: f32,
    tint: [f32; 3],
    camera: f32,
}

const SHEET: Sheet = Sheet {
    albedo: 0.8,
    ior: 1.4,
    height: 5.0,
    slab: 0.0,
    sphere: 0.0,
    tint: [1.0; 3],
    camera: 60.0,
};

const TINT: [f32; 3] = [0.9, 0.92, 1.0];

fn quad(corners: [[f32; 3]; 4], outward: [f32; 3], material: u32) -> [Triangle; 2] {
    let [a, b, c, d] = corners;
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let facing = normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
    if facing >= 0.0 {
        [
            Triangle {
                vertices: [a, b, c],
                material,
            },
            Triangle {
                vertices: [a, c, d],
                material,
            },
        ]
    } else {
        [
            Triangle {
                vertices: [a, c, b],
                material,
            },
            Triangle {
                vertices: [a, d, c],
                material,
            },
        ]
    }
}

fn level(y: f32, e: f32, outward: f32, material: u32) -> [Triangle; 2] {
    quad(
        [[-e, y, -e], [-e, y, e], [e, y, e], [e, y, -e]],
        [0.0, outward, 0.0],
        material,
    )
}

fn sheet_scene(sheet: Sheet) -> Scene<Sky> {
    let table = Material {
        base: [sheet.albedo; 3],
        roughness: 1.0,
        specular: 0.0,
        ..Material::default()
    };
    let glass = Material {
        base: sheet.tint,
        ior: sheet.ior,
        ..glass()
    };
    let e = 50.0;
    let (low, high) = (sheet.height, sheet.height + sheet.slab);
    let mut triangles = Vec::new();
    let mut shapes = Vec::new();
    triangles.extend(level(0.0, e, 1.0, 0));
    if sheet.sphere > 0.0 {
        shapes.push(pfx_trace::shapes::Shape::Ellipsoid {
            center: [0.0, sheet.height, 0.0],
            radii: [sheet.sphere; 3],
            material: 1,
        });
    } else {
        triangles.extend(level(high, e, 1.0, 1));
    }
    if sheet.slab > 0.0 {
        triangles.extend(level(low, e, -1.0, 1));
        for (axis, sign) in [(0, 1.0), (0, -1.0), (2, 1.0), (2, -1.0)] {
            let side = |y: f32, along: f32| {
                let mut p = [0.0, y, 0.0];
                p[axis] = sign * e;
                p[2 - axis] = along;
                p
            };
            let mut outward = [0.0; 3];
            outward[axis] = sign;
            triangles.extend(quad(
                [side(low, -e), side(low, e), side(high, e), side(high, -e)],
                outward,
                1,
            ));
        }
    }
    let span = 0.2 / (sheet.camera.max(1e-3));
    Scene {
        triangles,
        shapes,
        materials: vec![table, glass],
        sky: white_sky(),
        camera: Camera {
            origin: [0.0, sheet.camera, 0.0],
            forward: [0.0, -1.0, 0.0],
            right: [span, 0.0, 0.0],
            up: [0.0, 0.0, span],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
    }
}

fn sheet_figure(gpu: &Gpu, sheet: Sheet, bounces: Bounces, samples: u32) -> f32 {
    let [r, g, b] = sheet_colour(gpu, sheet, bounces, samples);
    (r + g + b) / 3.0
}

fn sheet_colour(gpu: &Gpu, sheet: Sheet, bounces: Bounces, samples: u32) -> [f32; 3] {
    let detail = Detail {
        bounces,
        ..Detail::default()
    };
    let mut trace = Trace::new_detailed(gpu, &sheet_scene(sheet), &detail, SIDE, SIDE).unwrap();
    let mut pacer = Pacer::default();
    let mut turns = Turns::default();
    let stats = trace
        .sample_paced(gpu, samples, 7, &mut pacer, |ms| turns.add(ms))
        .unwrap();
    turns.turn();
    assert!(stats.longest_ms() < 300.0, "{}", stats.longest_ms());
    let output = trace.readback(gpu).unwrap();
    let pixels: Vec<[f32; 3]> = (0..(SIDE * SIDE) as usize)
        .map(|pixel| {
            std::array::from_fn(|channel| {
                let at = pixel * 16 + channel * 4;
                f32::from_le_bytes(output.color[at..at + 4].try_into().unwrap())
            })
        })
        .collect();
    mean(&pixels)
}

fn cycles_sheets() -> Vec<(Sheet, f32)> {
    let at = |albedo: f32, ior: f32| Sheet {
        albedo,
        ior,
        ..SHEET
    };
    vec![
        (at(1.0, 1.4), 0.9506),
        (at(0.8, 1.4), 0.6662),
        (at(0.5, 1.4), 0.3547),
        (at(0.2, 1.4), 0.1384),
        (at(1.0, 1.33), 0.9686),
        (at(0.5, 1.33), 0.3724),
        (at(1.0, 1.5), 0.9200),
        (at(0.5, 1.5), 0.3334),
        (at(1.0, 1.8), 0.8122),
        (at(0.5, 1.8), 0.2972),
        (
            Sheet {
                height: 1.0,
                ..SHEET
            },
            0.6406,
        ),
        (
            Sheet {
                height: 20.0,
                ..SHEET
            },
            0.7699,
        ),
        (Sheet { slab: 1.0, ..SHEET }, 0.7875),
        (
            Sheet {
                slab: 1.0,
                ..at(0.5, 1.5)
            },
            0.5049,
        ),
        (
            Sheet {
                camera: 2.5,
                ..SHEET
            },
            0.6603,
        ),
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_clear_sheet_over_a_table_reads_like_cycles() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut worst: f32 = 0.0;
    for (sheet, cycles) in cycles_sheets() {
        let ours = sheet_figure(&gpu, sheet, Bounces::REFERENCE, 4096);
        println!(
            "albedo {} ior {} height {} slab {} camera {}: tracer {ours:.4}, cycles {cycles:.4}",
            sheet.albedo, sheet.ior, sheet.height, sheet.slab, sheet.camera
        );
        worst = worst.max((ours - cycles).abs());
    }
    println!("worst {worst:.4}");
    assert!(worst < 0.01, "{worst}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_deep_clear_sheet_is_a_lossless_cavity_like_cycles() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let cases = [
        (
            Sheet {
                albedo: 1.0,
                ..SHEET
            },
            0.9990,
        ),
        (
            Sheet {
                albedo: 0.5,
                ..SHEET
            },
            0.3581,
        ),
        (Sheet { slab: 1.0, ..SHEET }, 0.7935),
        (
            Sheet {
                camera: 2.5,
                ..SHEET
            },
            0.6724,
        ),
    ];
    for (sheet, cycles) in cases {
        let ours = sheet_figure(&gpu, sheet, Bounces::deep(64), 4096);
        println!(
            "deep albedo {} slab {} camera {}: tracer {ours:.4}, cycles {cycles:.4}",
            sheet.albedo, sheet.slab, sheet.camera
        );
        assert!((ours - cycles).abs() < 0.01, "{ours} against {cycles}");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_glass_sphere_and_a_tinted_slab_over_a_table_read_like_cycles() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let sphere = Sheet {
        ior: 1.5,
        height: 1.5,
        sphere: 1.0,
        ..SHEET
    };
    let liquid = Sheet {
        ior: 1.33,
        height: 1.0,
        slab: 2.0,
        tint: TINT,
        ..SHEET
    };
    let cases = [
        (sphere, Bounces::REFERENCE, [0.8081; 3]),
        (sphere, Bounces::deep(64), [0.8088; 3]),
        (
            Sheet {
                ior: 1.33,
                tint: TINT,
                ..sphere
            },
            Bounces::REFERENCE,
            [0.6962, 0.7170, 0.8029],
        ),
        (liquid, Bounces::REFERENCE, [0.6379, 0.6657, 0.7837]),
        (liquid, Bounces::deep(64), [0.6390, 0.6673, 0.7892]),
        (
            Sheet {
                albedo: 0.5,
                ..liquid
            },
            Bounces::REFERENCE,
            [0.4008, 0.4178, 0.4893],
        ),
    ];
    for (sheet, bounces, cycles) in cases {
        let ours = sheet_colour(&gpu, sheet, bounces, 4096);
        println!(
            "sphere {} slab {} ior {} albedo {} {bounces:?}: tracer {ours:.4?}, cycles {cycles:.4?}",
            sheet.sphere, sheet.slab, sheet.ior, sheet.albedo
        );
        for channel in 0..3 {
            assert!(
                (ours[channel] - cycles[channel]).abs() < 0.01,
                "{ours:?} against {cycles:?}"
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn each_bounce_limit_cuts_paths_where_cycles_does() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let sheet = Sheet {
        albedo: 1.0,
        ior: 1.8,
        ..SHEET
    };
    let reference = Bounces::REFERENCE;
    let cases = [
        (reference, 0.8122),
        (
            Bounces {
                diffuse: 64,
                ..reference
            },
            0.9662,
        ),
        (
            Bounces {
                glossy: 64,
                ..reference
            },
            0.8115,
        ),
        (
            Bounces {
                transmission: 64,
                ..reference
            },
            0.8133,
        ),
        (
            Bounces {
                total: 64,
                ..reference
            },
            0.8124,
        ),
        (
            Bounces {
                diffuse: 2,
                ..reference
            },
            0.5702,
        ),
        (
            Bounces {
                glossy: 2,
                ..reference
            },
            0.7031,
        ),
        (
            Bounces {
                total: 6,
                ..reference
            },
            0.7024,
        ),
    ];
    for (bounces, cycles) in cases {
        let ours = sheet_figure(&gpu, sheet, bounces, 4096);
        println!("{bounces:?}: tracer {ours:.4}, cycles {cycles:.4}");
        assert!(
            (ours - cycles).abs() < 0.01,
            "{bounces:?}: {ours} against {cycles}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_closed_glass_sphere_keeps_its_energy() {
    use pfx_trace::shapes::Shape;
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = Scene {
        triangles: Vec::new(),
        shapes: vec![Shape::Ellipsoid {
            center: [0.0, 0.0, 0.0],
            radii: [1.0; 3],
            material: 0,
        }],
        materials: vec![glass()],
        sky: white_sky(),
        camera: Camera {
            origin: [0.0, 0.0, 6.0],
            forward: [0.0, 0.0, -1.0],
            right: [0.1, 0.0, 0.0],
            up: [0.0, 0.1, 0.0],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
    };
    let pixels = patch_pixels(&gpu, &scene, Vec::new(), 256);
    let ours = mean(&pixels);
    println!("sphere {ours:?}");
    for value in ours {
        assert!((value - 1.0).abs() < 0.02, "{ours:?}");
    }
}

fn slab_scene() -> Scene<Sky> {
    let table = Material {
        base: [0.7, 0.7, 0.7],
        roughness: 0.8,
        specular: 0.028,
        ..Material::default()
    };
    let edge = 40.0;
    let corner = |x: f32, z: f32| [x, 0.0, z];
    Scene {
        triangles: vec![
            Triangle {
                vertices: [
                    corner(-edge, -edge),
                    corner(edge, edge),
                    corner(edge, -edge),
                ],
                material: 1,
            },
            Triangle {
                vertices: [
                    corner(-edge, -edge),
                    corner(-edge, edge),
                    corner(edge, edge),
                ],
                material: 1,
            },
        ],
        shapes: vec![pfx_trace::shapes::Shape::RoundedBox {
            center: [0.0, 0.6, 0.0],
            half: [2.0, 0.3, 2.0],
            radius: 0.05,
            material: 0,
        }],
        materials: vec![glass(), table],
        sky: Sky {
            width: 4,
            height: 2,
            texels: vec![
                [1.0, 0.9, 0.8, 1.0],
                [0.5, 0.6, 0.9, 1.0],
                [1.4, 1.3, 1.2, 1.0],
                [0.7, 0.8, 0.7, 1.0],
                [0.2, 0.2, 0.2, 1.0],
                [0.1, 0.1, 0.12, 1.0],
                [0.15, 0.12, 0.1, 1.0],
                [0.1, 0.1, 0.1, 1.0],
            ],
        },
        camera: Camera {
            origin: [0.3, 5.0, 0.2],
            forward: [0.0, -1.0, 0.0],
            right: [0.2, 0.0, 0.0],
            up: [0.0, 0.0, 0.2],
        },
        sun: Sun {
            direction: [0.0, 1.0, 0.0],
            color: [0.0; 3],
            intensity: 0.0,
        },
    }
}

struct Spread {
    mean: [f64; 3],
    error: [f64; 3],
    variance: f64,
    colour: f64,
}

fn spread(runs: &[Vec<[f32; 3]>]) -> Spread {
    let k = runs.len() as f64;
    let pixels = runs[0].len();
    let image_means: Vec<[f64; 3]> = runs.iter().map(|run| mean(run).map(f64::from)).collect();
    let mean = std::array::from_fn(|c| image_means.iter().map(|m| m[c]).sum::<f64>() / k);
    let error = std::array::from_fn(|c| {
        let v = image_means
            .iter()
            .map(|m| (m[c] - mean[c]).powi(2))
            .sum::<f64>()
            / (k - 1.0);
        (v / k).sqrt()
    });
    let variance_of = |value: &dyn Fn(&[f32; 3]) -> f64| {
        (0..pixels)
            .map(|p| {
                let values: Vec<f64> = runs.iter().map(|run| value(&run[p])).collect();
                let m = values.iter().sum::<f64>() / k;
                values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (k - 1.0)
            })
            .sum::<f64>()
            / pixels as f64
    };
    let variance = (0..3)
        .map(|c| variance_of(&|p: &[f32; 3]| p[c] as f64))
        .sum();
    let colour = variance_of(&|p: &[f32; 3]| (p[0] - p[1]) as f64)
        + variance_of(&|p: &[f32; 3]| (p[2] - p[1]) as f64);
    Spread {
        mean,
        error,
        variance,
        colour,
    }
}

const SLAB_RUNS: u32 = 32;
const BEFORE_SLAB_MEAN: [f64; 3] = [0.5739, 0.5862, 0.5850];
const BEFORE_SLAB_ERROR: [f64; 3] = [0.0033, 0.0106, 0.0037];
const BEFORE_SLAB_VARIANCE: f64 = 8.47;
const BEFORE_SLAB_COLOUR: f64 = 15.27;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_clear_glass_slab_refracts_all_channels_together() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = slab_scene();
    let runs: Vec<_> = (0..SLAB_RUNS)
        .map(|run| seeded_pixels(&gpu, &scene, Vec::new(), 2, 1000 + run * 2))
        .collect();
    let ours = spread(&runs);
    println!(
        "slab at 64 spp: mean {:?} ± {:?}, per-pixel variance {:.6}, colour variance {:.6}",
        ours.mean, ours.error, ours.variance, ours.colour
    );
    assert!(
        ours.variance < BEFORE_SLAB_VARIANCE / 50.0,
        "{}",
        ours.variance
    );
    assert!(ours.colour < BEFORE_SLAB_COLOUR / 1000.0, "{}", ours.colour);
    for c in 0..3 {
        let noise = (ours.error[c].powi(2) + BEFORE_SLAB_ERROR[c].powi(2)).sqrt();
        assert!(
            (ours.mean[c] - BEFORE_SLAB_MEAN[c]).abs() <= 4.0 * noise,
            "{:?} against {BEFORE_SLAB_MEAN:?}",
            ours.mean
        );
    }
}
