use pfx_gpu::Gpu;
use pfx_live::frame::{Camera, Frame, Instance, Matrix, MeshData, Scene, SceneWind, Sun, multiply};
use pfx_live::shadow::{Quality, ReceiverBox, Shadows, View};
use pfx_materials::Material;

const SIZE: u32 = 384;
const HEIGHT: f32 = 6.0;
const RADIUS_DEG: f32 = 2.0;

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn perspective() -> Matrix {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
    [
        [f, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn look_down() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -HEIGHT, 1.0],
    ]
}

fn camera() -> Camera {
    Camera {
        view: look_down(),
        projection: perspective(),
        previous_view_projection: multiply(perspective(), look_down()),
        position: [0.0, HEIGHT, 0.0],
    }
}

fn view() -> View {
    View {
        eye: [0.0, HEIGHT, 0.0],
        forward: [0.0, -1.0, 0.0],
        up: [0.0, 0.0, -1.0],
        fov_y: 50.0_f32.to_radians(),
        aspect: 1.0,
        near: 0.1,
        far: 12.0,
    }
}

fn lambert() -> Material {
    Material {
        base: [0.8, 0.8, 0.8],
        roughness: 1.0,
        specular: 0.0,
        metalness: 0.0,
        clearcoat: 0.0,
        ..Material::default()
    }
}

fn toward_sun(degrees: f32) -> [f32; 3] {
    let angle = degrees.to_radians();
    [angle.sin(), angle.cos(), 0.0]
}

fn quad(frame: &mut Frame, corners: [[f32; 3]; 4], normal: [f32; 3]) -> Instance {
    let handle = frame
        .upload_mesh(MeshData {
            positions: &corners,
            normals: &[normal; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 2, 1, 1, 2, 3],
        })
        .unwrap();
    let mut instance = Instance::new(handle, identity(), 0, 1);
    instance.two_sided = true;
    instance
}

fn render(
    frame: &mut Frame,
    shadows: &mut Shadows,
    instances: &[Instance],
    sun: Sun,
    shadowed: bool,
) -> Vec<f32> {
    let materials = [lambert()];
    let scene = Scene {
        camera: camera(),
        time: 0.0,
        seed: 5,
        sun,
        instances,
        materials: &materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    let fit = shadows.fit(&view(), sun.direction);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame
        .encode(
            &scene,
            &mut encoder,
            shadowed.then_some((shadows, &fit)),
            None,
        )
        .unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    frame
        .gpu
        .readback_rgba16(&frame.targets.hdr)
        .unwrap()
        .chunks_exact(4)
        .map(|p| {
            let rgb: [f32; 3] = std::array::from_fn(|c| half::f16::from_bits(p[c]).to_f32());
            (rgb[0] + rgb[1] + rgb[2]) / 3.0
        })
        .collect()
}

fn setup_with(radius: f32, receiver: ReceiverBox, quality: Quality) -> (Frame, Shadows) {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let shadows = Shadows::new(
        &frame.gpu.device,
        Quality {
            resolution: 2048,
            receiver: Some(receiver),
            caster_margin: 3.0,
            sun_radius_deg: radius,
            ..quality
        },
    );
    (frame, shadows)
}

fn sun(direction: [f32; 3]) -> Sun {
    Sun {
        direction,
        colour: [1.0; 3],
        intensity: 3.0,
    }
}

fn tilted(frame: &mut Frame, degrees: f32) -> Instance {
    let tilt = degrees.to_radians();
    let (s, c) = tilt.sin_cos();
    let corner = |x: f32, z: f32| [x * c, x * s, z];
    quad(
        frame,
        [
            corner(-1.2, -1.2),
            corner(1.2, -1.2),
            corner(-1.2, 1.2),
            corner(1.2, 1.2),
        ],
        [-s, c, 0.0],
    )
}

fn lit_mean(unshadowed: &[f32], shadowed: &[f32]) -> (f64, f64) {
    let mut reference = 0.0;
    let mut ours = 0.0;
    let mut count = 0.0;
    for (a, b) in unshadowed.iter().zip(shadowed) {
        if *a > 0.05 {
            reference += f64::from(*a);
            ours += f64::from(*b);
            count += 1.0;
        }
    }
    assert!(count > 1000.0, "the plane covers {count} pixels");
    (reference / count, ours / count)
}

fn tilted_plane_ratio(radius: f32, tilt: f32, sun_degrees: f32) -> f64 {
    tilted_plane_ratio_with(radius, tilt, sun_degrees, Quality::default())
}

fn tilted_plane_ratio_with(radius: f32, tilt: f32, sun_degrees: f32, quality: Quality) -> f64 {
    let (mut frame, mut shadows) = setup_with(
        radius,
        ReceiverBox {
            min: [-1.5, -0.2, -1.5],
            max: [1.5, 1.5, 1.5],
        },
        quality,
    );
    let plane = tilted(&mut frame, tilt);
    let light = sun(toward_sun(sun_degrees));
    let reference = render(&mut frame, &mut shadows, &[plane], light, false);
    let shadowed = render(&mut frame, &mut shadows, &[plane], light, true);
    let (reference, ours) = lit_mean(&reference, &shadowed);
    println!("tilt {tilt}, sun {sun_degrees}: unshadowed {reference:.4}, shadowed {ours:.4}");
    ours / reference
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_tilted_plane_under_a_soft_sun_reads_fully_lit() {
    for (tilt, sun_degrees) in [
        (30.0, 40.0),
        (60.0, 20.0),
        (45.0, 40.0),
        (70.0, 15.0),
        (-60.0, 20.0),
    ] {
        let ratio = tilted_plane_ratio(RADIUS_DEG, tilt, sun_degrees);
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "tilt {tilt}, sun {sun_degrees}: shadowed over unshadowed {ratio:.4}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_soft_sun_reads_as_lit_as_the_hard_filter_on_a_tilted_plane() {
    let soft = tilted_plane_ratio(RADIUS_DEG, 30.0, 40.0);
    let hard = tilted_plane_ratio(0.0, 30.0, 40.0);
    assert!(
        (soft - hard).abs() < 0.01,
        "soft {soft:.4} against hard {hard:.4}"
    );
}

fn penumbra(height: f32) -> f64 {
    let image = penumbra_image(height, Quality::default());
    let row = &image[(SIZE as usize / 2) * SIZE as usize..][..SIZE as usize];
    let lit = row.iter().copied().fold(0.0_f32, f32::max);
    assert!(lit > 0.1, "the floor is lit: {lit}");
    let shadow = row.iter().copied().fold(f32::MAX, f32::min);
    assert!(shadow < lit * 0.3, "the blocker shades the floor: {shadow}");
    let width = row
        .iter()
        .map(|v| (v - shadow) / (lit - shadow))
        .filter(|v| *v > 0.1 && *v < 0.9)
        .count();
    println!("height {height}: penumbra {width} pixels");
    width as f64
}

fn penumbra_image(height: f32, quality: Quality) -> Vec<f32> {
    let (mut frame, mut shadows) = setup_with(
        RADIUS_DEG,
        ReceiverBox {
            min: [-3.0, -0.1, -3.0],
            max: [3.0, 0.1, 3.0],
        },
        quality,
    );
    let floor = quad(
        &mut frame,
        [
            [-3.0, 0.0, -3.0],
            [3.0, 0.0, -3.0],
            [-3.0, 0.0, 3.0],
            [3.0, 0.0, 3.0],
        ],
        [0.0, 1.0, 0.0],
    );
    let mut blocker = quad(
        &mut frame,
        [
            [-8.0, height, -4.0],
            [0.0, height, -4.0],
            [-8.0, height, 4.0],
            [0.0, height, 4.0],
        ],
        [0.0, 1.0, 0.0],
    );
    blocker.shadow_only = true;
    blocker.id = 2;
    let light = sun(toward_sun(45.0));
    render(&mut frame, &mut shadows, &[floor, blocker], light, true)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_occluders_penumbra_widens_with_its_distance() {
    let near = penumbra(0.6);
    let far = penumbra(1.8);
    assert!(near >= 1.0, "the near penumbra is {near} pixels");
    assert!(
        far > near * 2.0,
        "penumbra {near} pixels at 0.6 m against {far} at 1.8 m"
    );
}

fn display(value: f32) -> i32 {
    (value.clamp(0.0, 1.0) * 255.0).round() as i32
}

fn compare(full: Quality, cut: Quality) -> Vec<(f32, i32, f64)> {
    [0.6, 1.8]
        .into_iter()
        .map(|height| {
            let exact = penumbra_image(height, full);
            let early = penumbra_image(height, cut);
            let worst = exact
                .iter()
                .zip(&early)
                .map(|(a, b)| (display(*a) - display(*b)).abs())
                .max()
                .unwrap();
            let mean = exact
                .iter()
                .zip(&early)
                .map(|(a, b)| f64::from((display(*a) - display(*b)).abs()))
                .sum::<f64>()
                / exact.len() as f64;
            println!("height {height}: worst {worst}/255, mean {mean:.4}/255");
            (height, worst, mean)
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_soft_suns_early_outs_match_the_full_filter_within_one_level() {
    let full = Quality {
        lit_skip: false,
        umbra_skip: false,
        ..Quality::default()
    };
    let early = Quality {
        umbra_skip: false,
        ..Quality::default()
    };
    for (height, worst, _) in compare(full, early) {
        assert!(
            worst <= 1,
            "height {height}: the early outs differ by {worst}/255"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_soft_suns_approximations_move_only_penumbra_edges() {
    let today = Quality {
        nearest_blocker: false,
        umbra_skip: false,
        blend_dither: false,
        ..Quality::default()
    };
    for (name, cut) in [
        (
            "umbra skip",
            Quality {
                umbra_skip: true,
                ..today
            },
        ),
        (
            "nearest blocker",
            Quality {
                nearest_blocker: true,
                ..today
            },
        ),
        ("shipped", Quality::default()),
    ] {
        println!("{name}");
        for (height, worst, mean) in compare(today, cut) {
            assert!(
                mean < 0.05,
                "{name} at height {height}: {mean:.4}/255 on average, worst {worst}/255"
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_shipped_soft_sun_keeps_a_grazing_plane_lit() {
    let today = Quality {
        nearest_blocker: false,
        umbra_skip: false,
        blend_dither: false,
        ..Quality::default()
    };
    for (tilt, sun_degrees) in [(45.0, 40.0), (70.0, 15.0)] {
        let before = tilted_plane_ratio_with(RADIUS_DEG, tilt, sun_degrees, today);
        let after = tilted_plane_ratio(RADIUS_DEG, tilt, sun_degrees);
        assert!(
            after >= before - 0.005,
            "tilt {tilt}, sun {sun_degrees}: {after:.4} against today's {before:.4}"
        );
    }
}

fn contact_strip(nearest: bool) -> f64 {
    let (mut frame, mut shadows) = setup_with(
        RADIUS_DEG * 2.0,
        ReceiverBox {
            min: [-3.0, -0.1, -3.0],
            max: [3.0, 0.1, 3.0],
        },
        Quality {
            nearest_blocker: nearest,
            ..Quality::default()
        },
    );
    let floor = quad(
        &mut frame,
        [
            [-3.0, 0.0, -3.0],
            [3.0, 0.0, -3.0],
            [-3.0, 0.0, 3.0],
            [3.0, 0.0, 3.0],
        ],
        [0.0, 1.0, 0.0],
    );
    let mut instances = vec![floor];
    let mut lip = quad(
        &mut frame,
        [
            [-4.0, 0.03, -4.0],
            [0.0, 0.03, -4.0],
            [-4.0, 0.03, 4.0],
            [0.0, 0.03, 4.0],
        ],
        [0.0, 1.0, 0.0],
    );
    lip.shadow_only = true;
    lip.id = 2;
    instances.push(lip);
    for slat in 0..12 {
        let z = -3.0 + slat as f32 * 0.5;
        let mut far = quad(
            &mut frame,
            [
                [-8.0, 1.5, z],
                [8.0, 1.5, z],
                [-8.0, 1.5, z + 0.2],
                [8.0, 1.5, z + 0.2],
            ],
            [0.0, 1.0, 0.0],
        );
        far.shadow_only = true;
        far.id = 3 + slat;
        instances.push(far);
    }
    let image = render(
        &mut frame,
        &mut shadows,
        &instances,
        sun(toward_sun(30.0)),
        true,
    );
    let size = SIZE as usize;
    let mut total = 0.0;
    let mut count = 0.0;
    for y in 0..size {
        for x in 0..size {
            let world_x = (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            let reach = HEIGHT * (50.0_f32.to_radians() * 0.5).tan();
            let at = world_x * reach;
            if (-0.08..-0.01).contains(&at) {
                total += f64::from(image[y * size + x]);
                count += 1.0;
            }
        }
    }
    assert!(count > 100.0, "the strip covers {count} pixels");
    total / count
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn near_contact_stays_dark_beside_far_blockers() {
    let averaged = contact_strip(false);
    let nearest = contact_strip(true);
    println!("contact strip: averaged {averaged:.4}, nearest {nearest:.4}");
    assert!(
        nearest < averaged * 0.8,
        "the nearest blocker keeps the contact umbra dark: {nearest:.4} against {averaged:.4}"
    );
}

fn floor(frame: &mut Frame) -> Instance {
    quad(
        frame,
        [
            [-3.0, 0.0, -3.0],
            [3.0, 0.0, -3.0],
            [-3.0, 0.0, 3.0],
            [3.0, 0.0, 3.0],
        ],
        [0.0, 1.0, 0.0],
    )
}

fn caster(frame: &mut Frame, x: [f32; 2], z: [f32; 2], height: f32, id: u32) -> Instance {
    let mut instance = quad(
        frame,
        [
            [x[0], height, z[0]],
            [x[1], height, z[0]],
            [x[0], height, z[1]],
            [x[1], height, z[1]],
        ],
        [0.0, 1.0, 0.0],
    );
    instance.shadow_only = true;
    instance.id = id;
    instance
}

fn ground(pixel: usize) -> f32 {
    ((pixel as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0) * HEIGHT * (50.0_f32.to_radians() * 0.5).tan()
}

struct Shot {
    ambient: Vec<f32>,
    lit: Vec<f32>,
    shadowed: Vec<f32>,
}

impl Shot {
    fn take(frame: &mut Frame, shadows: &mut Shadows, instances: &[Instance], light: Sun) -> Self {
        let dark = Sun {
            intensity: 0.0,
            ..light
        };
        Self {
            ambient: render(frame, shadows, &instances[..1], dark, false),
            lit: render(frame, shadows, &instances[..1], light, false),
            shadowed: render(frame, shadows, instances, light, true),
        }
    }

    fn sun_share(&self, keep: impl Fn(f32, f32) -> bool) -> f64 {
        let size = SIZE as usize;
        let mut sun = 0.0;
        let mut ours = 0.0;
        let mut count = 0.0;
        for row in 0..size {
            for column in 0..size {
                if !keep(ground(column), ground(row)) {
                    continue;
                }
                let at = row * size + column;
                sun += f64::from(self.lit[at] - self.ambient[at]);
                ours += f64::from(self.shadowed[at] - self.ambient[at]);
                count += 1.0;
            }
        }
        assert!(count > 100.0, "the region covers {count} pixels");
        assert!(sun > count * 0.05, "the region is lit unshadowed");
        ours / sun
    }
}

const LIP_HEIGHT: f32 = 0.15;
const LIP_SUN: f32 = 30.0;
const SLAT_HEIGHT: f32 = 1.5;

struct Slats {
    resolution: u32,
    half: f32,
    top: f32,
    height: f32,
    span: [f32; 2],
    lip: f32,
    period: f32,
    width: f32,
    radius: f32,
    caster_margin: f32,
}

fn lip_under_slats(quality: Quality, slats: &Slats) -> Shot {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let mut shadows = Shadows::new(
        &frame.gpu.device,
        Quality {
            resolution: slats.resolution,
            receiver: Some(ReceiverBox {
                min: [-slats.half, -0.1, -3.0],
                max: [slats.half, slats.top, 3.0],
            }),
            caster_margin: slats.caster_margin,
            sun_radius_deg: slats.radius,
            ..quality
        },
    );
    let mut instances = vec![floor(&mut frame)];
    instances.push(caster(
        &mut frame,
        [-slats.lip, 0.0],
        [-4.0, 4.0],
        LIP_HEIGHT,
        2,
    ));
    let count = (6.0 / slats.period) as u32;
    for slat in 0..count {
        let z = -3.0 + slat as f32 * slats.period;
        instances.push(caster(
            &mut frame,
            slats.span,
            [z, z + slats.width],
            slats.height,
            3 + slat,
        ));
    }
    Shot::take(
        &mut frame,
        &mut shadows,
        &instances,
        sun(toward_sun(LIP_SUN)),
    )
}

fn in_lip_umbra(x: f32, slats: &Slats) -> bool {
    let shift = LIP_HEIGHT * LIP_SUN.to_radians().tan();
    let margin = LIP_HEIGHT * slats.radius.to_radians().tan() * 3.0;
    (-slats.lip - shift + margin..-shift - margin).contains(&x)
}

fn near_lip_edge(x: f32, slats: &Slats) -> bool {
    let shift = LIP_HEIGHT * LIP_SUN.to_radians().tan();
    let margin = LIP_HEIGHT * slats.radius.to_radians().tan() * 3.0;
    (-shift - 0.06..-shift - margin).contains(&x)
}

fn phase(z: f32, slats: &Slats) -> f32 {
    (z + 3.0).rem_euclid(slats.period)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_thin_lips_umbra_stays_dark_beside_far_slats() {
    let slats = Slats {
        resolution: 2048,
        half: 3.0,
        top: 0.2,
        height: SLAT_HEIGHT,
        span: [-8.0, 8.0],
        lip: 0.3,
        period: 0.25,
        width: 0.15,
        radius: RADIUS_DEG * 2.0,
        caster_margin: 3.0,
    };
    let share = |layered: bool| {
        lip_under_slats(
            Quality {
                layered,
                ..Quality::default()
            },
            &slats,
        )
        .sun_share(|x, z| {
            in_lip_umbra(x, &slats) && (0.17..0.23).contains(&phase(z, &slats)) && z.abs() < 2.4
        })
    };
    let one = share(false);
    let layered = share(true);
    println!("sun through the lip beside slats: one averaged layer {one:.4}, layered {layered:.4}");
    assert!(
        layered < 0.01,
        "the lip's umbra lets {layered:.4} of the sun through"
    );
    assert!(
        one > layered + 0.02,
        "one averaged layer let {one:.4} through against {layered:.4}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_thin_lip_hidden_under_pancaked_slats_stays_dark() {
    let slats = Slats {
        resolution: 512,
        half: 0.6,
        top: 0.8,
        height: 3.0,
        span: [-1.0, 2.0],
        lip: 0.08,
        period: 0.25,
        width: 0.2,
        radius: RADIUS_DEG * 2.0,
        caster_margin: 0.3,
    };
    let share = |near_map: bool| {
        lip_under_slats(
            Quality {
                near_map,
                ..Quality::default()
            },
            &slats,
        )
        .sun_share(|x, z| {
            near_lip_edge(x, &slats) && (0.16..0.2).contains(&phase(z, &slats)) && z.abs() < 2.4
        })
    };
    let one = share(false);
    let split = share(true);
    println!("sun through the lip under slats: one map {one:.4}, near map {split:.4}");
    assert!(
        split < 0.01,
        "the lip's umbra under a slat lets {split:.4} of the sun through"
    );
    assert!(
        one > split + 0.02,
        "one map let {one:.4} through against {split:.4}"
    );
}

fn outside_the_box(outer_scale: f32) -> (f64, Vec<f32>) {
    let (mut frame, mut shadows) = setup_with(
        RADIUS_DEG,
        ReceiverBox {
            min: [-1.0, -0.1, -1.0],
            max: [1.0, 0.1, 1.0],
        },
        Quality {
            outer_scale,
            ..Quality::default()
        },
    );
    let instances = [
        floor(&mut frame),
        caster(&mut frame, [1.5, 2.5], [-0.6, 0.6], 0.6, 2),
        caster(&mut frame, [-0.5, 0.0], [-0.5, 0.5], 0.6, 3),
    ];
    let shot = Shot::take(
        &mut frame,
        &mut shadows,
        &instances,
        sun(toward_sun(LIP_SUN)),
    );
    let shift = 0.6 * LIP_SUN.to_radians().tan();
    let share = shot
        .sun_share(|x, z| (1.5 - shift + 0.15..2.5 - shift - 0.15).contains(&x) && z.abs() < 0.4);
    (share, shot.shadowed)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_receiver_outside_the_box_reads_the_outer_cascade() {
    let (inner, inner_image) = outside_the_box(0.0);
    let (outer, outer_image) = outside_the_box(3.0);
    println!("sun outside the box: box only {inner:.4}, outer cascade {outer:.4}");
    assert!(
        inner > 0.99,
        "without the outer cascade {inner:.4} of the sun arrives"
    );
    assert!(
        outer < 0.01,
        "the outer cascade lets {outer:.4} of the sun through"
    );
    let size = SIZE as usize;
    let mut inside = 0;
    for row in 0..size {
        for column in 0..size {
            let (gx, gz) = (ground(column), ground(row));
            if gx.abs() < 0.9 && gz.abs() < 0.9 {
                inside += 1;
                assert_eq!(
                    inner_image[row * size + column].to_bits(),
                    outer_image[row * size + column].to_bits(),
                    "inside the box at ({gx:.3}, {gz:.3})"
                );
            }
        }
    }
    assert!(inside > 1000, "{inside} pixels inside the box");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_new_switches_leave_a_single_layer_inside_the_box_unchanged() {
    let off = Quality {
        layered: false,
        outer_scale: 0.0,
        near_map: false,
        soft_offset: 1.0,
        ..Quality::default()
    };
    let on = Quality {
        layered: true,
        soft_offset: 1.0,
        ..Quality::default()
    };
    for height in [0.6, 1.8] {
        let before = penumbra_image(height, off);
        let after = penumbra_image(height, on);
        let differ = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        assert_eq!(differ, 0, "height {height}: {differ} pixels changed");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_soft_offset_keeps_grazing_planes_lit_and_moves_steep_edges_in() {
    for (tilt, sun_degrees) in [(60.0, 20.0), (70.0, 15.0), (45.0, 40.0), (30.0, 40.0)] {
        let full = tilted_plane_ratio_with(
            RADIUS_DEG,
            tilt,
            sun_degrees,
            Quality {
                soft_offset: 1.0,
                ..Quality::default()
            },
        );
        let shipped = tilted_plane_ratio(RADIUS_DEG, tilt, sun_degrees);
        assert!(
            shipped >= full - 0.005,
            "tilt {tilt}, sun {sun_degrees}: {shipped:.4} against the full offset's {full:.4}"
        );
    }
}
