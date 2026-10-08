use pfx_core::daylight::{Daylight, REFERENCE_HOUR};
use pfx_gpu::Gpu;
use pfx_load::Sky;
use pfx_materials::Material;
use pfx_trace::bvh::Triangle;
use pfx_trace::denoise;
use pfx_trace::sky::{AnalyticSky, Environment};
use pfx_trace::{
    Camera, Output, Projection, Scene, Sun, Trace, detail::Detail, equirect_direction,
};

const WIDTH: u32 = 512;
const HEIGHT: u32 = 256;

fn floats(bytes: &[u8]) -> Vec<[f32; 4]> {
    bytes
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn world_camera() -> Camera {
    Camera {
        origin: [0.0; 3],
        forward: [0.0, 0.0, -1.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
    }
}

fn dark() -> Sun {
    Sun {
        direction: [0.0, 1.0, 0.0],
        color: [0.0; 3],
        intensity: 0.0,
    }
}

fn run(gpu: &Gpu, trace: &mut Trace, passes: u32, per_pass: u32, seed: u32) -> Output {
    for pass in 0..passes {
        trace.sample(gpu, per_pass, seed + pass).unwrap();
    }
    trace.readback(gpu).unwrap()
}

fn quad(corners: [[f32; 3]; 4], material: u32) -> [Triangle; 2] {
    let [a, b, c, d] = corners;
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
}

fn cube(centre: [f32; 3], half: f32, material: u32) -> Vec<Triangle> {
    let p = |x: f32, y: f32, z: f32| {
        [
            centre[0] + x * half,
            centre[1] + y * half,
            centre[2] + z * half,
        ]
    };
    let mut out = Vec::new();
    for sign in [-1.0, 1.0] {
        out.extend(quad(
            [
                p(sign, -1.0, -1.0),
                p(sign, 1.0, -1.0),
                p(sign, 1.0, 1.0),
                p(sign, -1.0, 1.0),
            ],
            material,
        ));
        out.extend(quad(
            [
                p(-1.0, sign, -1.0),
                p(1.0, sign, -1.0),
                p(1.0, sign, 1.0),
                p(-1.0, sign, 1.0),
            ],
            material,
        ));
        out.extend(quad(
            [
                p(-1.0, -1.0, sign),
                p(1.0, -1.0, sign),
                p(1.0, 1.0, sign),
                p(-1.0, 1.0, sign),
            ],
            material,
        ));
    }
    out
}

fn fibonacci(count: usize) -> Vec<[f32; 3]> {
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    (0..count)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / count as f32;
            let radius = (1.0 - y * y).sqrt();
            let phi = golden * i as f32;
            [radius * phi.cos(), y, radius * phi.sin()]
        })
        .collect()
}

fn pointed(direction: [f32; 3]) -> Camera {
    let forward = unit(direction);
    let helper = if forward[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let right = unit(cross(forward, helper));
    let up = cross(right, forward);
    let tan = 1e-4;
    Camera {
        origin: [0.0; 3],
        forward,
        right: right.map(|v| v * tan),
        up: up.map(|v| v * tan),
    }
}

fn texel_spread(sky: &Sky, direction: [f32; 3]) -> f32 {
    let d = unit(direction);
    let u = (d[0].atan2(-d[2]) / std::f32::consts::TAU + 0.5).rem_euclid(1.0);
    let v = d[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    let x = (u * sky.width as f32) as i32;
    let y = (v * sky.height as f32) as i32;
    let mut lo = f32::MAX;
    let mut hi = 0.0_f32;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let col = (x + dx).rem_euclid(sky.width as i32) as usize;
            let row = (y + dy).clamp(0, sky.height as i32 - 1) as usize;
            let texel = sky.texels[row * sky.width as usize + col];
            let value = texel[0] + texel[1] + texel[2];
            lo = lo.min(value);
            hi = hi.max(value);
        }
    }
    (hi - lo) / hi.max(1e-6)
}

fn round_trip<E: Clone + Into<Environment>>(
    gpu: &Gpu,
    scene: &Scene<E>,
    passes: u32,
    avoid: impl Fn([f32; 3]) -> bool,
) -> (Output, Sky) {
    let mut panorama = Trace::new_detailed(
        gpu,
        scene,
        &Detail {
            projection: Projection::Equirectangular,
            ..Detail::default()
        },
        WIDTH,
        HEIGHT,
    )
    .unwrap();
    let output = run(gpu, &mut panorama, passes, 4, 41);
    let sky = Sky {
        width: WIDTH,
        height: HEIGHT,
        texels: floats(&output.color),
    };
    let loaded = Environment::Hdr(sky.clone());
    let mut probe = Trace::new(gpu, scene, 2, 2).unwrap();
    let mut compared = 0;
    let directions = fibonacci(96);
    for &direction in &directions {
        if avoid(direction) || texel_spread(&sky, direction) > 0.02 {
            continue;
        }
        probe.set_camera(pointed(direction)).unwrap();
        let direct = floats(&run(gpu, &mut probe, 1, 8, 7).color);
        let direct: [f32; 3] =
            std::array::from_fn(|c| direct.iter().map(|p| p[c]).sum::<f32>() / 4.0);
        let looked = loaded.radiance(direction);
        for c in 0..3 {
            let gap = (looked[c] - direct[c]).abs();
            assert!(
                gap <= 0.01 * direct[c].abs() + 1e-4,
                "{direction:?} channel {c}: sky lookup {} against direct trace {}",
                looked[c],
                direct[c]
            );
        }
        compared += 1;
    }
    println!("compared {compared} of {} directions", directions.len());
    assert!(
        compared >= 48,
        "only {compared} directions were flat enough"
    );
    (output, sky)
}

#[test]
fn equirect_direction_inverts_the_sky_lookup() {
    for (row, col) in [(3, 7), (64, 300), (128, 256), (200, 1), (255, 511)] {
        let u = (col as f32 + 0.5) / WIDTH as f32;
        let v = (row as f32 + 0.5) / HEIGHT as f32;
        let d = equirect_direction(u, v);
        assert!((dot(d, d) - 1.0).abs() < 1e-5);
        let back_u = (d[0].atan2(-d[2]) / std::f32::consts::TAU + 0.5).rem_euclid(1.0);
        let back_v = d[1].acos() / std::f32::consts::PI;
        assert!((back_u - u).abs() < 1e-5, "{u} {back_u}");
        assert!((back_v - v).abs() < 1e-5, "{v} {back_v}");
    }
    assert!(dot(equirect_direction(0.5, 0.5), [0.0, 0.0, -1.0]) > 0.9999);
    assert!(dot(equirect_direction(0.75, 0.5), [1.0, 0.0, 0.0]) > 0.9999);
    assert!(dot(equirect_direction(0.3, 0.0), [0.0, 1.0, 0.0]) > 0.9999);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_emitter_room_traced_to_an_equirect_loads_back_as_the_same_sky() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let emitter = |emission: [f32; 3]| Material {
        base: [0.0; 3],
        specular: 0.0,
        roughness: 1.0,
        emission,
        ..Material::default()
    };
    let materials = vec![
        emitter([0.9, 0.2, 0.1]),
        emitter([0.1, 0.8, 0.2]),
        emitter([0.2, 0.3, 1.2]),
        emitter([0.6, 0.6, 0.1]),
        emitter([0.1, 0.5, 0.7]),
        emitter([0.4, 0.1, 0.6]),
        emitter([8.0, 7.5, 6.5]),
    ];
    let h = 3.0;
    let mut triangles = Vec::new();
    let wall = |axis: usize, sign: f32, material: u32| {
        let corner = |a: f32, b: f32| {
            let mut p = [0.0; 3];
            p[axis] = sign * h;
            p[(axis + 1) % 3] = a * h;
            p[(axis + 2) % 3] = b * h;
            p
        };
        quad(
            [
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ],
            material,
        )
    };
    for axis in 0..3 {
        triangles.extend(wall(axis, -1.0, axis as u32 * 2));
        triangles.extend(wall(axis, 1.0, axis as u32 * 2 + 1));
    }
    triangles.extend(quad(
        [
            [2.5, 0.4, -1.5],
            [2.5, 1.6, -1.5],
            [2.5, 1.6, -0.3],
            [2.5, 0.4, -0.3],
        ],
        6,
    ));
    let scene = Scene {
        triangles,
        shapes: Vec::new(),
        materials,
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0; 4]],
        },
        camera: world_camera(),
        sun: dark(),
    };
    let (output, sky) = round_trip(&gpu, &scene, 4, |_| false);
    let softbox = Environment::Hdr(sky.clone()).radiance(unit([2.5, 1.0, -0.9]));
    assert!(softbox[0] > 7.9, "the softbox reads {softbox:?}");
    let albedo = floats(&output.albedo);
    let normal = floats(&output.normal);
    for (index, (a, n)) in albedo.iter().zip(&normal).enumerate() {
        assert!((a[3] - 1.0).abs() < 1e-6, "pixel {index} misses the room");
        let x = index as u32 % WIDTH;
        let y = index as u32 / WIDTH;
        let d = equirect_direction(
            (x as f32 + 0.5) / WIDTH as f32,
            (y as f32 + 0.5) / HEIGHT as f32,
        );
        assert!(
            dot([n[0], n[1], n[2]], d) < -0.3,
            "pixel {index}: normal {n:?} against ray {d:?}"
        );
    }
    let filtered = denoise::filter(WIDTH, HEIGHT, &sky.texels, &albedo, &normal, 16).unwrap();
    let centre = |image: &[[f32; 4]], d: [f32; 3]| {
        Environment::Hdr(Sky {
            width: WIDTH,
            height: HEIGHT,
            texels: image.to_vec(),
        })
        .radiance(d)
    };
    for d in [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]] {
        let raw = centre(&sky.texels, d);
        let smooth = centre(&filtered, d);
        for c in 0..3 {
            assert!((raw[c] - smooth[c]).abs() <= 0.01 * raw[c] + 1e-4);
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_analytic_sky_traced_to_an_equirect_loads_back_as_the_same_sky() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let daylight = Daylight {
        hour: 15.0,
        day: 172.0,
        latitude: 45.0,
        heading: 180.0,
    };
    let model = AnalyticSky::new(daylight, REFERENCE_HOUR, 2.5, [0.2, 0.25, 0.2]);
    let scene = Scene {
        triangles: Vec::new(),
        shapes: Vec::new(),
        materials: vec![Material::default()],
        sky: Environment::Analytic(model),
        camera: world_camera(),
        sun: Sun::from_daylight(daylight),
    };
    let sun = model.sun;
    round_trip(&gpu, &scene, 4, |d| {
        dot(d, sun) > 5.0_f32.to_radians().cos()
    });
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_equirect_follows_the_camera_basis() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let emitter = |emission: [f32; 3]| Material {
        base: [0.0; 3],
        specular: 0.0,
        emission,
        ..Material::default()
    };
    let mut triangles = Vec::new();
    triangles.extend(quad(
        [
            [-0.5, -0.5, -4.0],
            [0.5, -0.5, -4.0],
            [0.5, 0.5, -4.0],
            [-0.5, 0.5, -4.0],
        ],
        0,
    ));
    triangles.extend(quad(
        [
            [4.0, -0.5, -0.5],
            [4.0, 0.5, -0.5],
            [4.0, 0.5, 0.5],
            [4.0, -0.5, 0.5],
        ],
        1,
    ));
    let scene = Scene {
        triangles,
        shapes: Vec::new(),
        materials: vec![emitter([1.0, 0.0, 0.0]), emitter([0.0, 1.0, 0.0])],
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.0; 4]],
        },
        camera: world_camera(),
        sun: dark(),
    };
    let detail = Detail {
        projection: Projection::Equirectangular,
        ..Detail::default()
    };
    let mut trace = Trace::new_detailed(&gpu, &scene, &detail, 64, 32).unwrap();
    let at = |image: &[[f32; 4]], u: f32| image[16 * 64 + (u * 64.0) as usize];
    let world = floats(&run(&gpu, &mut trace, 1, 4, 3).color);
    assert!(at(&world, 0.5)[0] > 0.99 && at(&world, 0.75)[1] > 0.99);
    trace
        .set_camera(Camera {
            origin: [0.0; 3],
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
        })
        .unwrap();
    let turned = floats(&run(&gpu, &mut trace, 1, 4, 3).color);
    assert!(at(&turned, 0.5)[1] > 0.99 && at(&turned, 0.25)[0] > 0.99);
}

fn ortho_coverage(gpu: &Gpu, depth: f32) -> Vec<f32> {
    let scene = Scene {
        triangles: cube([0.0, 0.0, -depth], 0.5, 0),
        shapes: Vec::new(),
        materials: vec![Material::default()],
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.2, 0.2, 0.2, 1.0]],
        },
        camera: world_camera(),
        sun: dark(),
    };
    let detail = Detail {
        projection: Projection::Orthographic {
            width: 2.0,
            height: 2.0,
        },
        ..Detail::default()
    };
    let mut trace = Trace::new_detailed(gpu, &scene, &detail, 64, 64).unwrap();
    let output = run(gpu, &mut trace, 2, 4, 5);
    floats(&output.albedo).iter().map(|p| p[3]).collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_orthographic_unit_cube_face_is_exactly_its_pixel_width_at_any_depth() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let near = ortho_coverage(&gpu, 2.0);
    let far = ortho_coverage(&gpu, 40.0);
    assert_eq!(near, far);
    for y in 0..64 {
        for x in 0..64 {
            let inside = (16..48).contains(&x) && (16..48).contains(&y);
            let value = near[y * 64 + x];
            assert_eq!(value, if inside { 1.0 } else { 0.0 }, "pixel {x}, {y}");
        }
    }
    let row: usize = (0..64).filter(|&x| near[32 * 64 + x] > 0.5).count();
    assert_eq!(row, 32);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_perspective_camera_is_unchanged_by_the_projection_field() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let scene = Scene {
        triangles: cube([0.2, -0.1, -3.0], 0.6, 0),
        shapes: Vec::new(),
        materials: vec![Material::default()],
        sky: Sky {
            width: 1,
            height: 1,
            texels: vec![[0.6, 0.7, 0.8, 1.0]],
        },
        camera: Camera {
            origin: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            right: [0.5, 0.0, 0.0],
            up: [0.0, 0.5, 0.0],
        },
        sun: Sun {
            direction: unit([0.3, 0.8, 0.4]),
            color: [1.0; 3],
            intensity: 2.0,
        },
    };
    let mut plain = Trace::new(&gpu, &scene, 32, 32).unwrap();
    let mut switched = Trace::new_detailed(
        &gpu,
        &scene,
        &Detail {
            projection: Projection::Orthographic {
                width: 1.0,
                height: 1.0,
            },
            ..Detail::default()
        },
        32,
        32,
    )
    .unwrap();
    switched.set_projection(Projection::Perspective).unwrap();
    assert_eq!(
        run(&gpu, &mut plain, 2, 4, 9),
        run(&gpu, &mut switched, 2, 4, 9)
    );
    let mut flat = scene.camera;
    flat.right = [0.0; 3];
    switched.set_camera(flat).unwrap();
    assert!(
        switched
            .set_projection(Projection::Equirectangular)
            .is_err()
    );
    assert!(
        switched
            .set_projection(Projection::Orthographic {
                width: f32::NAN,
                height: 1.0
            })
            .is_err()
    );
}
