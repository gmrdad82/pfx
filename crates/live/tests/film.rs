use pfx_gpu::pace::Turns;
use pfx_gpu::{Gpu, wgpu};
use pfx_live::frame::{
    Camera, Frame, Instance, Matrix, MeshData, OpaqueFeatures, Scene, SceneWind, Sun, multiply,
};
use pfx_materials::{FILM, Material, film_conductor, film_dielectric, film_rgb};
use pfx_trace::film::{conductor_reflectance, film_reflectance, srgb_byte};

const DIELECTRICS: [(f32, f32); 8] = [
    (1.9, 1.4),
    (1.33, 1.5),
    (1.5, 2.4),
    (2.0, 1.5),
    (2.6, 1.5),
    (1.45, 1.0),
    (2.3, 1.6),
    (1.6, 1.33),
];
const METALS: [[f32; 3]; 5] = [
    [0.95, 0.64, 0.54],
    [1.0, 0.78, 0.34],
    [0.56, 0.57, 0.58],
    [0.91, 0.92, 0.92],
    [0.2, 0.3, 0.8],
];
const METAL_FILMS: [f32; 3] = [1.5, 2.0, 2.6];
const DIELECTRIC_REACH: u32 = 1000;
const METAL_REACH: u32 = 400;
const STEP_NM: u32 = 5;
const STEEPEST_DEG: u32 = 70;
const MICRON: (u32, u32) = (900, 1100);

#[derive(Clone, Copy, Debug)]
enum Stack {
    Dielectric { film: f32, substrate: f32 },
    Conductor { film: f32, f0: [f32; 3] },
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    stack: Stack,
    cosine: f32,
    thickness: f32,
}

fn cosines() -> Vec<f32> {
    (0..=STEEPEST_DEG)
        .step_by(5)
        .map(|deg| (deg as f32).to_radians().cos())
        .collect()
}

fn stacks() -> Vec<(Stack, u32)> {
    let mut out: Vec<(Stack, u32)> = DIELECTRICS
        .iter()
        .map(|&(film, substrate)| (Stack::Dielectric { film, substrate }, DIELECTRIC_REACH))
        .collect();
    for f0 in METALS {
        for film in METAL_FILMS {
            out.push((Stack::Conductor { film, f0 }, METAL_REACH));
        }
    }
    out
}

fn sweep() -> Vec<Sample> {
    let mut out = Vec::new();
    for (stack, reach) in stacks() {
        for cosine in cosines() {
            for thickness in (0..=reach).step_by(STEP_NM as usize) {
                out.push(Sample {
                    stack,
                    cosine,
                    thickness: thickness as f32,
                });
            }
        }
    }
    out
}

fn micron() -> Vec<Sample> {
    let mut out = Vec::new();
    for &(film, substrate) in &DIELECTRICS {
        for cosine in cosines() {
            for thickness in (MICRON.0..=MICRON.1).step_by(STEP_NM as usize) {
                out.push(Sample {
                    stack: Stack::Dielectric { film, substrate },
                    cosine,
                    thickness: thickness as f32,
                });
            }
        }
    }
    out
}

fn tracer(sample: &Sample) -> [f32; 3] {
    match sample.stack {
        Stack::Dielectric { film, substrate } => {
            film_reflectance(sample.cosine, sample.thickness, 1.0, film, substrate)
        }
        Stack::Conductor { film, f0 } => {
            conductor_reflectance(sample.cosine, sample.thickness, film, f0)
        }
    }
}

fn live(sample: &Sample) -> [f32; 3] {
    match sample.stack {
        Stack::Dielectric { film, substrate } => {
            film_dielectric(sample.cosine, sample.thickness, film, substrate)
        }
        Stack::Conductor { film, f0 } => film_conductor(sample.cosine, sample.thickness, film, f0),
    }
}

fn three_waves(sample: &Sample) -> [f32; 3] {
    match sample.stack {
        Stack::Dielectric { film, substrate } => {
            film_rgb(sample.cosine, sample.thickness, film, substrate)
        }
        Stack::Conductor { .. } => unreachable!(),
    }
}

fn levels(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3)
        .map(|c| (srgb_byte(a[c]) - srgb_byte(b[c])).abs())
        .fold(0.0, f32::max)
}

struct Worst {
    level: f32,
    at: Option<Sample>,
}

fn worst(samples: &[Sample], ours: &[[f32; 3]], theirs: &[[f32; 3]]) -> Worst {
    let mut out = Worst {
        level: 0.0,
        at: None,
    };
    for ((sample, a), b) in samples.iter().zip(ours).zip(theirs) {
        let level = levels(*a, *b);
        if level > out.level {
            out = Worst {
                level,
                at: Some(*sample),
            };
        }
    }
    out
}

fn steepest_step(samples: &[Sample], values: &[[f32; 3]]) -> f32 {
    let mut out = 0.0_f32;
    for (pair, value) in samples.windows(2).zip(values.windows(2)) {
        let same = pair[0].cosine == pair[1].cosine
            && format!("{:?}", pair[0].stack) == format!("{:?}", pair[1].stack)
            && pair[1].thickness > pair[0].thickness;
        if same {
            out = out.max(levels(value[0], value[1]));
        }
    }
    out
}

fn check_sweep(name: &str, samples: &[Sample], ours: &[[f32; 3]]) {
    let theirs: Vec<[f32; 3]> = samples.iter().map(tracer).collect();
    let (dielectric, conductor): (Vec<usize>, Vec<usize>) =
        (0..samples.len()).partition(|&i| matches!(samples[i].stack, Stack::Dielectric { .. }));
    for (kind, indices) in [("dielectric", dielectric), ("conductor", conductor)] {
        let pick = |values: &[[f32; 3]]| indices.iter().map(|&i| values[i]).collect::<Vec<_>>();
        let chosen: Vec<Sample> = indices.iter().map(|&i| samples[i]).collect();
        let found = worst(&chosen, &pick(ours), &pick(&theirs));
        println!(
            "{name}, {kind}: {} samples, worst {:.3}/255 at {:?}",
            chosen.len(),
            found.level,
            found.at
        );
        assert!(
            found.level <= 1.0,
            "{name}, {kind}: {:.3}/255 from the tracer at {:?}",
            found.level,
            found.at
        );
    }
}

fn check_micron(name: &str, samples: &[Sample], ours: &[[f32; 3]]) {
    let theirs: Vec<[f32; 3]> = samples.iter().map(tracer).collect();
    let three: Vec<[f32; 3]> = samples.iter().map(three_waves).collect();
    let (live_step, tracer_step, three_step) = (
        steepest_step(samples, ours),
        steepest_step(samples, &theirs),
        steepest_step(samples, &three),
    );
    let found = worst(samples, ours, &theirs);
    println!(
        "{name}, 0.9 to 1.1 um every {STEP_NM} nm: largest step {live_step:.2}/255 live, {tracer_step:.2} tracer, {three_step:.2} three wavelengths; worst {:.3}/255 from the tracer",
        found.level
    );
    assert!(
        live_step <= tracer_step + 1.0,
        "{name}: the live film bands at 1 um: {live_step} against the tracer's {tracer_step}"
    );
    assert!(three_step > 4.0 * live_step.max(1.0));
    assert!(found.level <= 1.0, "{:?}", found.at);
}

#[test]
fn the_live_film_matches_the_tracer_within_a_level() {
    let samples = sweep();
    let ours: Vec<[f32; 3]> = samples.iter().map(live).collect();
    check_sweep("cpu", &samples, &ours);
}

#[test]
fn a_micron_film_does_not_band() {
    let samples = micron();
    let ours: Vec<[f32; 3]> = samples.iter().map(live).collect();
    check_micron("cpu", &samples, &ours);
}

fn shader_values(gpu: &Gpu, samples: &[Sample]) -> Vec<[f32; 3]> {
    let source = format!(
        "{FILM}
struct Case {{
    a: vec4f,
    b: vec4f,
}}
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> values: array<vec4f>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3u) {{
    let i = id.x;
    if (i >= arrayLength(&cases)) {{
        return;
    }}
    let c = cases[i];
    var v: vec3f;
    if (c.a.x < 0.5) {{
        v = film_dielectric(c.a.y, c.a.z, c.a.w, c.b.x);
    }} else {{
        v = film_conductor(c.a.y, c.a.z, c.a.w, c.b.xyz);
    }}
    values[i] = vec4f(v, 1.0);
}}
"
    );
    let cases: Vec<f32> = samples
        .iter()
        .flat_map(|sample| match sample.stack {
            Stack::Dielectric { film, substrate } => [
                0.0,
                sample.cosine,
                sample.thickness,
                film,
                substrate,
                0.0,
                0.0,
                0.0,
            ],
            Stack::Conductor { film, f0 } => [
                1.0,
                sample.cosine,
                sample.thickness,
                film,
                f0[0],
                f0[1],
                f0[2],
                0.0,
            ],
        })
        .collect();
    let device = &gpu.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("film sweep"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("film sweep"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("film cases"),
        size: (cases.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue
        .write_buffer(&input, 0, bytemuck::cast_slice(&cases));
    let bytes = (samples.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("film values"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("film readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("film sweep"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((samples.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let values: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
    readback.unmap();
    Turns::default().turn();
    values.chunks_exact(4).map(|v| [v[0], v[1], v[2]]).collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_shader_film_matches_the_tracer_within_a_level() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    for (name, samples) in [("gpu", sweep()), ("gpu at 1 um", micron())] {
        let ours = shader_values(&gpu, &samples);
        let cpu: Vec<[f32; 3]> = samples.iter().map(live).collect();
        let mirror = worst(&samples, &ours, &cpu);
        println!(
            "{name}: shader against its CPU mirror, worst {:.4}/255",
            mirror.level
        );
        assert!(mirror.level <= 0.25, "{:?}", mirror.at);
        if name == "gpu" {
            check_sweep(name, &samples, &ours);
        } else {
            check_micron(name, &samples, &ours);
        }
    }
}

const SIZE: u32 = 256;
const HEIGHT: f32 = 6.0;

fn identity() -> Matrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn projection(aspect: f32) -> Matrix {
    let near = 0.1;
    let far = 100.0;
    let f = 1.0 / (50.0_f32.to_radians() * 0.5).tan();
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, far * near / (near - far), 0.0],
    ]
}

fn camera(aspect: f32) -> Camera {
    let view = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -HEIGHT, 1.0],
    ];
    let projection = projection(aspect);
    Camera {
        view,
        projection,
        previous_view_projection: multiply(projection, view),
        position: [0.0, HEIGHT, 0.0],
    }
}

fn quad(frame: &mut Frame, lo: [f32; 2], hi: [f32; 2], y: f32, material: u32, id: u32) -> Instance {
    let handle = frame
        .upload_mesh(MeshData {
            positions: &[
                [lo[0], y, lo[1]],
                [hi[0], y, lo[1]],
                [lo[0], y, hi[1]],
                [hi[0], y, hi[1]],
            ],
            normals: &[[0.0, 1.0, 0.0]; 4],
            tangents: &[[1.0, 0.0, 0.0, 1.0]; 4],
            uvs: &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            uvs1: None,
            alpha: None,
            indices: &[0, 2, 1, 1, 2, 3],
        })
        .unwrap();
    Instance::new(handle, identity(), material, id)
}

fn plain() -> Material {
    Material {
        base: [0.7, 0.6, 0.5],
        roughness: 0.4,
        ..Material::default()
    }
}

fn metal() -> Material {
    Material {
        base: [0.9, 0.7, 0.4],
        roughness: 0.25,
        metalness: 1.0,
        ..Material::default()
    }
}

fn oil_slick(amount: f32) -> Material {
    Material {
        base: [0.05, 0.05, 0.06],
        roughness: 0.15,
        ior: 1.5,
        thin_film: 420.0,
        thin_film_ior: 1.45,
        thin_film_amount: amount,
        ..Material::default()
    }
}

fn anodised(amount: f32) -> Material {
    Material {
        base: [0.56, 0.57, 0.58],
        roughness: 0.2,
        metalness: 1.0,
        thin_film: 160.0,
        thin_film_ior: 2.4,
        thin_film_amount: amount,
        ..Material::default()
    }
}

const SUN: Sun = Sun {
    direction: [0.5, 0.8, 0.3],
    colour: [1.0, 0.95, 0.9],
    intensity: 3.0,
};

fn render(
    frame: &mut Frame,
    instances: &[Instance],
    materials: &[Material],
    on: OpaqueFeatures,
) -> Vec<u16> {
    let scene = Scene {
        camera: camera(1.0),
        time: 0.25,
        seed: 5,
        sun: SUN,
        instances,
        materials,
        deformers: &[],
        wind: SceneWind::default(),
    };
    frame.set_opaque_on(on);
    let mut encoder = frame.gpu.device.create_command_encoder(&Default::default());
    frame.encode(&scene, &mut encoder, None, None).unwrap();
    frame.gpu.queue.submit(Some(encoder.finish()));
    let pixels = frame.gpu.readback_rgba16(&frame.targets.hdr).unwrap();
    Turns::default().turn();
    pixels
}

fn pixel_of(point: [f32; 3]) -> [f32; 2] {
    let camera = camera(1.0);
    let clip = pfx_live::frame::transform(
        multiply(camera.projection, camera.view),
        [point[0], point[1], point[2], 1.0],
    );
    [
        (clip[0] / clip[3] * 0.5 + 0.5) * SIZE as f32,
        (0.5 - clip[1] / clip[3] * 0.5) * SIZE as f32,
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn parts_without_film_draw_the_same_bytes() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut frame = Frame::new(gpu, SIZE, SIZE).unwrap();
    let floor = quad(&mut frame, [-3.0, -3.0], [3.0, 3.0], 0.0, 0, 1);
    let brass = quad(&mut frame, [-2.5, -1.0], [-0.5, 1.0], 0.8, 1, 2);
    let filmed = [(0.5, -2.0), (2.5, 0.0)];
    let slick = quad(&mut frame, [0.5, -2.0], [2.5, 0.0], 0.6, 2, 3);
    let titanium = quad(&mut frame, [0.5, 0.5], [2.5, 2.5], 0.4, 3, 4);
    let instances = [floor, brass, slick, titanium];
    let with = [plain(), metal(), oil_slick(1.0), anodised(1.0)];
    let without = [plain(), metal(), oil_slick(0.0), anodised(0.0)];
    let general = OpaqueFeatures::ALL
        .without(OpaqueFeatures::named("fine_noise").unwrap())
        .without(OpaqueFeatures::named("procedural_lacquer").unwrap());

    let bare = render(&mut frame, &instances, &without, OpaqueFeatures::NONE);
    assert!(frame.opaque_drawn().iter().all(|used| !used.film));
    let compiled_in = render(&mut frame, &instances, &without, general);
    assert!(frame.opaque_drawn().iter().all(|used| used.film));
    assert!(
        bare == compiled_in,
        "compiling the film in changed a frame with no film"
    );

    let shown = render(&mut frame, &instances, &with, OpaqueFeatures::NONE);
    let drawn = frame.opaque_drawn().to_vec();
    assert_eq!(
        drawn.iter().filter(|used| used.film).count(),
        2,
        "the film compiles into the two filmed parts only: {drawn:?}"
    );
    let rects: Vec<[f32; 4]> = [
        (filmed[0], filmed[1], 0.6_f32),
        ((0.5, 0.5), (2.5, 2.5), 0.4),
    ]
    .iter()
    .map(|&((x0, z0), (x1, z1), y)| {
        let a = pixel_of([x0, y, z0]);
        let b = pixel_of([x1, y, z1]);
        [
            a[0].min(b[0]) - 1.0,
            a[1].min(b[1]) - 1.0,
            a[0].max(b[0]) + 1.0,
            a[1].max(b[1]) + 1.0,
        ]
    })
    .collect();
    let mut inside = 0;
    for (index, (a, b)) in bare.chunks_exact(4).zip(shown.chunks_exact(4)).enumerate() {
        if a == b {
            continue;
        }
        let (x, y) = (
            (index as u32 % SIZE) as f32 + 0.5,
            (index as u32 / SIZE) as f32 + 0.5,
        );
        assert!(
            rects
                .iter()
                .any(|r| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3]),
            "pixel {x}, {y} outside the filmed parts changed"
        );
        inside += 1;
    }
    assert!(inside > 2000, "the film shows on {inside} pixels");
}

fn opaque_ms(frame: &mut Frame) -> Option<f64> {
    frame
        .gpu
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let timings = frame.collect_timings();
    timings.last().and_then(|passes| {
        passes
            .iter()
            .find(|pass| pass.label == "opaque PBR")
            .map(|pass| pass.milliseconds)
    })
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_film_costs_this_much_at_4k() {
    let (width, height) = (3840, 2160);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    assert!(gpu.profiling(), "GPU timestamps unavailable");
    let mut frame = Frame::new(gpu, width, height).unwrap();
    let mut instances = Vec::new();
    let columns = 8;
    let rows = 5;
    for row in 0..rows {
        for column in 0..columns {
            let x = -6.0 + 12.0 * column as f32 / columns as f32;
            let z = -3.5 + 7.0 * row as f32 / rows as f32;
            let id = (row * columns + column) as u32;
            instances.push(quad(
                &mut frame,
                [x, z],
                [x + 12.0 / columns as f32, z + 7.0 / rows as f32],
                0.0,
                id % 2,
                id + 1,
            ));
        }
    }
    let aspect = width as f32 / height as f32;
    let scenes = [
        ("bare", [oil_slick(0.0), anodised(0.0)]),
        (
            "half an oil slick over a dark lacquer, half anodised titanium",
            [oil_slick(1.0), anodised(1.0)],
        ),
        ("all oil slick", [oil_slick(1.0), oil_slick(1.0)]),
        ("all anodised titanium", [anodised(1.0), anodised(1.0)]),
    ];
    let mut turns = Turns::default();
    let mut times = vec![Vec::new(); scenes.len()];
    for round in 0..40 {
        for (which, (_, materials)) in scenes.iter().enumerate() {
            let scene = Scene {
                camera: camera(aspect),
                time: 0.25,
                seed: 5,
                sun: SUN,
                instances: &instances,
                materials,
                deformers: &[],
                wind: SceneWind::default(),
            };
            frame.render(&scene).unwrap();
            let ms = opaque_ms(&mut frame).expect("the opaque pass reports a timing");
            turns.add(ms);
            if round >= 8 {
                times[which].push(ms);
            }
        }
    }
    turns.turn();
    let bare = median(times[0].clone());
    println!(
        "{width}x{height}, {} parts covering the frame, opaque pass without film {bare:.3} ms",
        instances.len()
    );
    for ((name, _), measured) in scenes.iter().zip(&times).skip(1) {
        let filmed = median(measured.clone());
        println!(
            "  {name}: {filmed:.3} ms, the film costs {:.3} ms",
            filmed - bare
        );
    }
}
