use super::svg::{self, Board as Parsed};
use super::*;
use pfx_gpu::{Gpu, wgpu};

const NAMES: [&str; 11] = [
    "card", "faded", "sheet", "panel", "pill", "dashed", "arc", "glow", "icons", "pointer", "scrim",
];

fn fixture(name: &str) -> String {
    format!("{}/tests/flat/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn reference(name: &str, scale: u32) -> ([u32; 2], Vec<[u8; 4]>) {
    let file = std::fs::File::open(fixture(&format!("{name}@{scale}x.png"))).unwrap();
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    let channels = info.color_type.samples();
    let pixels = bytes[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| [p[0], p[1], p[2], if channels == 4 { p[3] } else { 255 }])
        .collect();
    ([info.width, info.height], pixels)
}

pub(super) const FILTERS: [(&str, svg::Filter); 3] = [
    ("lift1", svg::Filter::Elevation(1.0)),
    ("lift2", svg::Filter::Elevation(2.0)),
    ("halo", svg::Filter::Glow { sigma: 8.0 }),
];

pub(super) fn test_curve() -> ShadowCurve {
    ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![
            CurvePoint {
                elevation: 1.0,
                offset: 4.0,
                sigma: 5.0,
                alpha: 0.22,
            },
            CurvePoint {
                elevation: 2.0,
                offset: 12.0,
                sigma: 16.0,
                alpha: 0.26,
            },
        ],
    )
    .unwrap()
}

fn parsed(name: &str) -> (Parsed, Icons) {
    let source = std::fs::read_to_string(fixture(&format!("{name}.svg"))).unwrap();
    let parsed = svg::read(&source, &FILTERS).unwrap();
    let icons = if parsed.icons.is_empty() {
        Icons::bake(&[icons::pointer()]).unwrap()
    } else {
        Icons::bake(&parsed.icons).unwrap()
    };
    (parsed, icons)
}

fn scene<'a>(
    parsed: &'a Parsed,
    curve: &'a ShadowCurve,
    icons: Option<&'a FlatIcons>,
) -> FlatScene<'a> {
    FlatScene {
        layout: parsed.size,
        clear: Some(Srgba::hex(0)),
        curve,
        light: Light::default(),
        draws: &parsed.draws,
        groups: &parsed.groups,
        text: &[],
        icons,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    }
}

fn quantize(pixel: [f32; 4]) -> [u8; 3] {
    std::array::from_fn(|i| (pixel[i].clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[derive(Clone, Copy, Debug)]
struct Difference {
    mean: f64,
    worst: u8,
    over: f64,
}

fn difference(ours: &[[u8; 3]], theirs: &[[u8; 4]]) -> Difference {
    assert_eq!(ours.len(), theirs.len());
    let mut total = 0u64;
    let mut worst = 0u8;
    let mut over = 0usize;
    for (a, b) in ours.iter().zip(theirs) {
        let mut pixel_worst = 0u8;
        for c in 0..3 {
            let d = a[c].abs_diff(b[c]);
            total += u64::from(d);
            pixel_worst = pixel_worst.max(d);
        }
        worst = worst.max(pixel_worst);
        if pixel_worst > 8 {
            over += 1;
        }
    }
    Difference {
        mean: total as f64 / (ours.len() * 3) as f64,
        worst,
        over: over as f64 / ours.len() as f64,
    }
}

fn cpu_image(name: &str, scale: u32) -> (Vec<[u8; 3]>, Vec<[u8; 4]>) {
    let (parsed, icons) = parsed(name);
    let curve = test_curve();
    let (size, reference) = reference(name, scale);
    assert_eq!(
        size,
        [parsed.size[0] as u32 * scale, parsed.size[1] as u32 * scale]
    );
    let image = cpu::render(&scene(&parsed, &curve, None), Some(&icons), size).unwrap();
    (image.into_iter().map(quantize).collect(), reference)
}

#[test]
fn the_cpu_twin_matches_the_reference_renders() {
    let mut report = String::new();
    let mut failures = Vec::new();
    for name in NAMES {
        for scale in [1, 2] {
            let (ours, theirs) = cpu_image(name, scale);
            let d = difference(&ours, &theirs);
            report += &format!(
                "{name}@{scale}x: mean {:.3}/255, worst {}/255, {:.3}% over 8\n",
                d.mean,
                d.worst,
                d.over * 100.0
            );
            if d.mean >= 1.0 {
                failures.push(format!("{name}@{scale}x mean {:.3}", d.mean));
            }
        }
    }
    eprintln!("{report}");
    assert!(failures.is_empty(), "{failures:?}\n{report}");
}

#[test]
fn the_cpu_twin_recreates_the_board() {
    let (ours, theirs) = cpu_image("board", 1);
    let d = difference(&ours, &theirs);
    eprintln!(
        "board@1x: mean {:.3}/255, worst {}/255, {:.3}% over 8",
        d.mean,
        d.worst,
        d.over * 100.0
    );
    assert!(d.mean < 1.0, "board mean {:.3}", d.mean);
}

#[test]
fn shaders_validate_without_optional_features() {
    for (label, source) in [
        ("flat", shader::source()),
        ("dial-free", shader::flat_source()),
        ("finish", shader::FINISH_WGSL.to_string()),
    ] {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{label}: {}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{label}: {error:?}"));
    }
}

#[test]
fn every_flat_layout_fits_webgpu_defaults() {
    let limits = wgpu::Limits::default();
    let view = pass::view_entries();
    let layer = pass::layer_entries();
    let text = pass::text_entries();
    let finish = pass::finish_entries();
    for groups in [
        vec![view.as_slice()],
        vec![view.as_slice(), layer.as_slice()],
        vec![text.as_slice()],
        vec![finish.as_slice()],
    ] {
        assert_eq!(pfx_gpu::layout_shortfall(&groups, &limits), None);
    }
    assert!(INSTANCE_ATTRIBUTES <= limits.max_vertex_attributes);
    assert_eq!(
        std::mem::size_of::<Instance>() as u32,
        INSTANCE_ATTRIBUTES * 16
    );
    assert!((std::mem::size_of::<Instance>() as u32) <= limits.max_vertex_buffer_array_stride);
    assert!(INSTANCE_ATTRIBUTES * 4 <= limits.max_inter_stage_shader_components);
    let dynamic = text
        .iter()
        .filter(|entry| {
            matches!(
                entry.ty,
                wgpu::BindingType::Buffer {
                    has_dynamic_offset: true,
                    ..
                }
            )
        })
        .count() as u32;
    assert!(dynamic <= limits.max_dynamic_uniform_buffers_per_pipeline_layout);
}

#[test]
fn the_layout_letterboxes_into_the_target() {
    let fit = Fit::new([1920.0, 1080.0], [3840, 2160]).unwrap();
    assert_eq!((fit.scale, fit.offset), (2.0, [0.0, 0.0]));
    assert_eq!(fit.to_target([330.0, 92.0]), [660.0, 184.0]);
    let deck = Fit::new([1920.0, 1080.0], [1280, 800]).unwrap();
    assert!((deck.scale - 2.0 / 3.0).abs() < 1e-6);
    assert_eq!(deck.offset, [0.0, 40.0]);
    assert_eq!(deck.viewport(), [0.0, 40.0, 1280.0, 760.0]);
    let back = deck.to_layout(deck.to_target([123.0, 456.0]));
    assert!((back[0] - 123.0).abs() < 1e-3 && (back[1] - 456.0).abs() < 1e-3);
    let tall = Fit::new([1920.0, 1080.0], [1080, 1920]).unwrap();
    assert_eq!(tall.offset[0], 0.0);
    assert!(tall.offset[1] > 0.0);
    let clip = deck.clip_from_layout();
    let corner = |p: [f32; 2]| {
        [
            clip[0][0] * p[0] + clip[3][0],
            clip[1][1] * p[1] + clip[3][1],
        ]
    };
    let top_left = corner([0.0, 0.0]);
    let bottom_right = corner([1920.0, 1080.0]);
    assert!((top_left[0] + 1.0).abs() < 1e-5 && (top_left[1] - 0.9).abs() < 1e-5);
    assert!((bottom_right[0] - 1.0).abs() < 1e-5 && (bottom_right[1] + 0.9).abs() < 1e-5);
    for target in [
        [3840, 2160],
        [1280, 800],
        [1366, 768],
        [2560, 1080],
        [1080, 1920],
        [201, 100],
    ] {
        let fit = Fit::new([1920.0, 1080.0], target).unwrap();
        let shared = pfx_gpu::window::letterbox_with(
            pfx_gpu::window::LAYOUT_UNITS,
            pfx_gpu::window::Size {
                width: target[0],
                height: target[1],
            },
            pfx_gpu::window::LetterboxRounding::Nearest,
        )
        .unwrap();
        assert_eq!(fit.letterbox, shared);
        for point in [[0.0, 0.0], [330.0, 92.0], [1919.0, 1079.0]] {
            assert_eq!(fit.to_target(point), shared.map(point[0], point[1]));
        }
    }
    assert!(Fit::new([0.0, 1.0], [1, 1]).is_err());
    assert!(Fit::new([1.0, 1.0], [0, 1]).is_err());
}

#[test]
fn painters_order_is_by_elevation_with_groups_kept_whole() {
    let draw = |elevation: f32, group: Option<u16>| {
        let mut draw = Draw::rect([0.0, 0.0], [10.0, 10.0], 0.0)
            .fill(Srgba::WHITE)
            .elevation(elevation);
        draw.group = group;
        draw
    };
    let draws = [
        draw(2.0, None),
        draw(1.0, Some(0)),
        draw(0.0, None),
        draw(1.5, Some(0)),
        draw(1.0, None),
        draw(1.0, Some(1)),
    ];
    let mut order = Order::default();
    order.build(&draws, 2).unwrap();
    assert_eq!(
        order.steps,
        vec![
            Step::Draw(2),
            Step::Layer(0),
            Step::Draw(1),
            Step::Draw(3),
            Step::EndLayer(0),
            Step::Draw(4),
            Step::Layer(1),
            Step::Draw(5),
            Step::EndLayer(1),
            Step::Draw(0),
        ]
    );
    assert!(order.build(&draws, 1).is_err());
    let mut bad = draws;
    bad[0].elevation = f32::NAN;
    assert!(order.build(&bad, 2).is_err());
}

#[test]
fn shadows_follow_the_curve_the_light_and_the_frame() {
    let curve = test_curve();
    let fit = Fit::new([1920.0, 1080.0], [3840, 2160]).unwrap();
    let card = Draw::rect([100.0, 100.0], [180.0, 62.0], 12.0)
        .fill(Srgba::WHITE)
        .elevation(1.0);
    let shadow = pack_shadow(&card, &fit, &curve, &Light::default()).unwrap();
    assert_eq!(shadow.kind(), KIND_SHADOW);
    assert_eq!(shadow.icon, [5.0, 5.0, 0.0, 4.0]);
    assert!((shadow.fill_top[3] - 0.22).abs() < 1e-6);
    assert_eq!(shadow.axes, [2.0, 0.0, 0.0, 2.0]);
    let sheet = pack_shadow(&card.elevation(2.0), &fit, &curve, &Light::default()).unwrap();
    assert_eq!(sheet.icon, [16.0, 16.0, 0.0, 12.0]);
    let sideways = Light {
        shadow: [1.0, 0.0],
        ..Light::default()
    };
    let side = pack_shadow(&card, &fit, &curve, &sideways).unwrap();
    assert!((side.icon[2] - 4.0).abs() < 1e-5 && side.icon[3].abs() < 1e-5);
    let turned = card.transform(place([100.0, 100.0], 0.5));
    let cast = pack_shadow(&turned, &fit, &curve, &Light::default()).unwrap();
    let layout_offset = [
        turned.transform[0][0] * cast.icon[2] + turned.transform[1][0] * cast.icon[3],
        turned.transform[0][1] * cast.icon[2] + turned.transform[1][1] * cast.icon[3],
    ];
    assert!(layout_offset[0].abs() < 1e-4 && (layout_offset[1] - 4.0).abs() < 1e-4);
    let local = pack_shadow(
        &turned.shadow(Shadow::Turned),
        &fit,
        &curve,
        &Light::default(),
    )
    .unwrap();
    assert_eq!([local.icon[2], local.icon[3]], [0.0, 4.0]);
    let tipped = card.transform(multiply(translation(100.0, 100.0), tip_x(1.0)));
    let footprint = pack_shadow(&tipped, &fit, &curve, &Light::default()).unwrap();
    assert!((footprint.icon[1] - 5.0 / 1.0f32.cos()).abs() < 1e-3);
    assert_eq!(
        pack_shadow(&card.elevation(0.0), &fit, &curve, &Light::default()),
        None
    );
    assert_eq!(
        pack_shadow(&card.shadow(Shadow::None), &fit, &curve, &Light::default()),
        None
    );
    let glow = card.shadow(Shadow::Glow(Glow {
        colour: Srgba::hex(0x3d8fd8).alpha(0.25),
        sigma: 10.0,
    }));
    let lit = pack_shadow(&glow, &fit, &curve, &Light::default()).unwrap();
    assert_eq!([lit.icon[2], lit.icon[3]], [0.0, 0.0]);
    assert!((lit.fill_top[3] - 0.25).abs() < 1e-6);
}

#[test]
fn dials_at_zero_pack_the_dial_free_instance() {
    let fit = Fit::new([400.0, 300.0], [800, 600]).unwrap();
    let draw = Draw::rect([200.0, 150.0], [120.0, 60.0], 12.0)
        .fill(Srgba::hex(0xffffff))
        .stroke(Stroke::solid(Srgba::hex(0x3d8fd8), 3.0));
    let flat = pack_shape(&draw, &fit, None).unwrap();
    let zero = pack_shape(&draw.material(Material::default()), &fit, None).unwrap();
    assert_eq!(bytemuck::bytes_of(&flat), bytemuck::bytes_of(&zero));
    assert_eq!(flat.info[0] & (LIT | SLAB), 0);
    let thick = pack_shape(
        &draw.material(Material {
            thickness: 8.0,
            bevel: 3.0,
            ..Material::default()
        }),
        &fit,
        None,
    )
    .unwrap();
    assert_eq!(thick.info[0] & (LIT | SLAB), LIT | SLAB);
    let glossy = pack_shape(
        &draw.material(Material {
            gloss: 0.5,
            roughness: 0.4,
            ..Material::default()
        }),
        &fit,
        None,
    )
    .unwrap();
    assert_eq!(glossy.info[0] & (LIT | SLAB), LIT);
}

#[test]
fn the_cpu_twin_picks_the_topmost_footprint() {
    let curve = test_curve();
    let draws = [
        Draw::rect([50.0, 50.0], [80.0, 80.0], 10.0)
            .fill(Srgba::WHITE)
            .id(1),
        Draw::circle([70.0, 70.0], 20.0)
            .fill(Srgba::hex(0x3d8fd8))
            .elevation(1.0)
            .id(2),
        Draw::rect([20.0, 20.0], [16.0, 16.0], 0.0)
            .stroke(Stroke::solid(Srgba::hex(0xe2614c), 2.0))
            .elevation(2.0)
            .id(3),
        Draw::rect([50.0, 50.0], [100.0, 100.0], 0.0)
            .fill(Srgba::WHITE.alpha(0.0))
            .elevation(3.0)
            .id(4),
    ];
    let scene = FlatScene {
        layout: [100.0, 100.0],
        clear: None,
        curve: &curve,
        light: Light::default(),
        draws: &draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let pick = |x, y| cpu::pick(&scene, None, [200, 200], [x, y]).unwrap();
    assert_eq!(pick(60, 60), 1);
    assert_eq!(pick(140, 140), 2);
    assert_eq!(pick(24, 40), 3);
    assert_eq!(pick(40, 40), 1);
    assert_eq!(pick(195, 5), 0);
}

pub(super) fn gpu() -> Gpu {
    pollster::block_on(Gpu::headless()).unwrap()
}

pub(super) fn gpu_render(
    gpu: &Gpu,
    pass: &mut FlatPass,
    scene: &FlatScene<'_>,
    size: [u32; 2],
) -> (Vec<[f32; 4]>, Vec<u32>) {
    let colour = gpu
        .offscreen(size[0], size[1], pass::COLOUR_FORMAT)
        .unwrap();
    let ids = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("flat ids"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: pass::ID_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let ids_view = ids.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pass.render(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        scene,
        FlatTarget {
            colour: &colour.view,
            ids: Some(&ids_view),
            size,
            clear: Some(scene.clear.map_or([0.0; 4], |c| c.premultiplied())),
            clear_ids: true,
        },
        None,
    )
    .unwrap();
    gpu.queue.submit(Some(encoder.finish()));
    let pixels = gpu
        .readback_rgba16(&colour)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|i| half::f16::from_bits(p[i]).to_f32()))
        .collect();
    let ids = read_ids(gpu, &ids, size);
    (pixels, ids)
}

pub(super) fn read_ids(gpu: &Gpu, texture: &wgpu::Texture, size: [u32; 2]) -> Vec<u32> {
    let row = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flat ids readback"),
        size: u64::from(row * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = buffer.slice(..).get_mapped_range();
    let mut ids = Vec::with_capacity((size[0] * size[1]) as usize);
    for y in 0..size[1] {
        let start = (y * row) as usize;
        ids.extend(
            bytes[start..start + (size[0] * 4) as usize]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
        );
    }
    ids
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_matches_the_references_and_its_twin() {
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut pass = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let curve = test_curve();
    let mut report = String::new();
    let mut failures = Vec::new();
    for name in NAMES.into_iter().chain(["board"]) {
        let (parsed, icons) = parsed(name);
        let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons.clone());
        for scale in [1, 2] {
            let (size, theirs) = reference(name, scale);
            let scene = scene(&parsed, &curve, Some(&flat_icons));
            let (pixels, _) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, size));
            let ours = pixels.into_iter().map(quantize).collect::<Vec<_>>();
            let twin = turns
                .cpu(|| cpu::render(&scene, Some(&icons), size))
                .unwrap()
                .into_iter()
                .map(quantize)
                .map(|[r, g, b]| [r, g, b, 255])
                .collect::<Vec<_>>();
            let reference = difference(&ours, &theirs);
            let against_twin = difference(&ours, &twin);
            report += &format!(
                "{name}@{scale}x: reference mean {:.3}/255 worst {}/255 ({:.3}% over 8); twin mean {:.4} worst {}\n",
                reference.mean,
                reference.worst,
                reference.over * 100.0,
                against_twin.mean,
                against_twin.worst
            );
            if reference.mean >= 1.0 {
                failures.push(format!(
                    "{name}@{scale}x reference mean {:.3}",
                    reference.mean
                ));
            }
            if against_twin.mean >= 0.15 || against_twin.worst > 2 {
                failures.push(format!(
                    "{name}@{scale}x twin mean {:.4} worst {}",
                    against_twin.mean, against_twin.worst
                ));
            }
        }
    }
    eprintln!("{report}");
    assert!(failures.is_empty(), "{failures:?}\n{report}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_ids_match_the_twins_picks() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = test_curve();
    let (mut parsed, icons) = parsed("card");
    for (index, draw) in parsed.draws.iter_mut().enumerate() {
        draw.id = index as u32 + 1;
    }
    let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons.clone());
    let scene = scene(&parsed, &curve, Some(&flat_icons));
    let size = [640, 460];
    let (_, ids) = gpu_render(&gpu, &mut pass, &scene, size);
    let mut checked = 0;
    let mut differing = 0;
    for y in (0..size[1]).step_by(3) {
        for x in (0..size[0]).step_by(3) {
            let expected = cpu::pick(&scene, Some(&icons), size, [x, y]).unwrap();
            checked += 1;
            if ids[(y * size[0] + x) as usize] != expected {
                differing += 1;
            }
        }
    }
    assert!(
        differing * 1000 <= checked,
        "{differing} of {checked} picks differ"
    );
    assert!(ids.iter().any(|&id| id != 0));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_flat_pipelines_run_on_a_device_at_webgpu_defaults() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("flat floor"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(device.limits(), wgpu::Limits::default());
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pass = FlatPass::new(&device, &queue);
    let curve = test_curve();
    let (parsed, icons) = parsed("icons");
    let flat_icons = FlatIcons::new(&device, &queue, icons);
    let mut environment = Vec::new();
    for y in 0..32 {
        for x in 0..64 {
            environment.push([x as f32 / 64.0, y as f32 / 32.0, 0.5, 1.0]);
        }
    }
    let environment = Environment::new(&device, &queue, 64, 32, &environment, 1.0).unwrap();
    let mut draws = parsed.draws.clone();
    draws.push(
        Draw::rect([100.0, 100.0], [80.0, 50.0], 12.0)
            .fill(Srgba::WHITE)
            .elevation(2.0)
            .group(0)
            .material(Material {
                thickness: 8.0,
                bevel: 3.0,
                gloss: 0.5,
                roughness: 0.3,
                reflection: 0.5,
            }),
    );
    let groups = [Group { opacity: 0.5 }];
    let mut scene = scene(&parsed, &curve, Some(&flat_icons));
    scene.draws = &draws;
    scene.groups = &groups;
    scene.environment = Some(&environment);
    let colour = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1040,
            height: 520,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: pass::COLOUR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let ids = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1040,
            height: 520,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: pass::ID_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let srgb = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1040,
            height: 520,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let colour_view = colour.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.render(
        &device,
        &queue,
        &mut encoder,
        &scene,
        FlatTarget {
            colour: &colour_view,
            ids: Some(&ids.create_view(&Default::default())),
            size: [1040, 520],
            clear: Some([0.0; 4]),
            clear_ids: true,
        },
        None,
    )
    .unwrap();
    for (linear, blend) in [(true, false), (false, true), (true, true)] {
        pass.encode_finish(
            &device,
            &mut encoder,
            &colour_view,
            &srgb.create_view(&Default::default()),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            linear,
            blend,
            None,
        );
    }
    queue.submit(Some(encoder.finish()));
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let error = pollster::block_on(device.pop_error_scope());
    assert!(error.is_none(), "{error:?}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn dials_at_zero_render_the_bytes_of_the_dial_free_shader() {
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    assert!(!shader::flat_source().contains("flat_lit"));
    let mut with_dials = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let mut without =
        turns.cpu(|| FlatPass::with_source(&gpu.device, &gpu.queue, shader::flat_source()));
    let curve = test_curve();
    for name in NAMES {
        let (mut parsed, icons) = parsed(name);
        for draw in &mut parsed.draws {
            draw.material = Material {
                roughness: 0.5,
                ..Material::default()
            };
        }
        let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons);
        let scene = scene(&parsed, &curve, Some(&flat_icons));
        let size = [parsed.size[0] as u32 * 2, parsed.size[1] as u32 * 2];
        let (a, ids_a) = turns.cpu(|| gpu_render(&gpu, &mut with_dials, &scene, size));
        let (b, ids_b) = turns.cpu(|| gpu_render(&gpu, &mut without, &scene, size));
        let bits = |pixels: &[[f32; 4]]| {
            pixels
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        };
        let (again, _) = turns.cpu(|| gpu_render(&gpu, &mut with_dials, &scene, size));
        let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        let worst = a
            .iter()
            .zip(&b)
            .flat_map(|(x, y)| (0..4).map(move |c| (x[c] - y[c]).abs()))
            .fold(0.0f32, f32::max);
        eprintln!(
            "{name}: {differing} pixels differ, worst {worst}, repeat equal {}",
            bits(&a) == bits(&again)
        );
        assert!(
            bits(&a) == bits(&b),
            "{name}: dials at zero changed the colour"
        );
        assert!(ids_a == ids_b, "{name}: dials at zero changed the ids");
    }
}

#[test]
#[ignore = "writes images for inspection"]
fn dump_cpu_twin() {
    let dir = format!("{}/../../tmp/flat-dump", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    for name in NAMES {
        let (ours, theirs) = cpu_image(name, 2);
        let (size, _) = reference(name, 2);
        let bytes = ours
            .iter()
            .flat_map(|a| [a[0], a[1], a[2], 255])
            .collect::<Vec<_>>();
        let diff = ours
            .iter()
            .zip(&theirs)
            .flat_map(|(a, b)| {
                let d = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
                let v = (u32::from(d) * 8).min(255) as u8;
                [v, v, v, 255]
            })
            .collect::<Vec<_>>();
        for (suffix, data) in [("ours", bytes), ("diff", diff)] {
            let file = std::fs::File::create(format!("{dir}/{name}-{suffix}.png")).unwrap();
            let mut encoder = png::Encoder::new(file, size[0], size[1]);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&data)
                .unwrap();
        }
    }
}

const SPIKE_FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;

pub(super) struct Spike {
    pub(super) icons: Icons,
    cursor: Icon,
    check: Icon,
}

pub(super) fn spike_icons() -> Spike {
    let icons = Icons::bake(&[icons::pointer(), icons::tick()]).unwrap();
    Spike {
        icons,
        cursor: Icon(0),
        check: Icon(1),
    }
}

fn wave(seed: u32) -> f32 {
    let x = seed.wrapping_mul(2_654_435_761).rotate_left(13) ^ seed.wrapping_mul(40_503);
    (x % 10_000) as f32 / 10_000.0
}

pub(super) fn spike_draws(
    spike: &Spike,
    t: f32,
    material: Material,
    label_count: usize,
    draws: &mut Vec<Draw>,
) {
    draws.clear();
    let ink = Srgba::hex(0x2b3442);
    let line = Srgba::hex(0xd3d9e1);
    let blue = Srgba::hex(0x3d8fd8);
    let kinds = [0xe2614c, 0x8e57b0, 0xf0b429, 0x138f8a].map(Srgba::hex);
    draws.push(Draw::rect([140.0, 540.0], [280.0, 1080.0], 0.0).fill(ink));
    draws.push(Draw::rect([900.0, 46.0], [1140.0, 92.0], 0.0).fill(Srgba::WHITE));
    draws.push(Draw::rect([1695.0, 540.0], [450.0, 1080.0], 0.0).fill(Srgba::WHITE));
    let lane_w = 216.0;
    let lanes: Vec<f32> = (0..5)
        .map(|i| 300.0 + i as f32 * (lane_w + 16.0) + lane_w * 0.5)
        .collect();
    for &x in &lanes {
        draws.push(
            Draw::rect([x, 589.0], [lane_w, 906.0], 28.0)
                .vertical(Srgba::WHITE.alpha(0.9), Srgba::WHITE.alpha(0.35))
                .stroke(Stroke::solid(line, 2.0)),
        );
    }
    let mut index = 0u32;
    while draws.len() < 400 {
        index += 1;
        let column = index % 12;
        let row = index / 12;
        let (x, y) = if column < 4 {
            (40.0 + column as f32 * 72.0, 120.0 + row as f32 * 26.0)
        } else {
            (
                1490.0 + (column - 4) as f32 * 54.0,
                120.0 + row as f32 * 26.0,
            )
        };
        let colour = kinds[index as usize % 4];
        let draw = match index % 4 {
            0 => Draw::rect([x, y], [48.0, 18.0], 6.0)
                .fill(Srgba::hex(0x36404f))
                .stroke(Stroke::solid(Srgba::hex(0x556178), 2.0)),
            1 => Draw::circle([x, y], 9.0).fill(colour).elevation(1.0),
            2 => Draw::new(Shape::Ring {
                radius: 9.0,
                width: 4.0,
            })
            .at([x, y])
            .fill(Srgba::hex(0xe1e5eb)),
            _ => Draw::new(Shape::Arc {
                radius: 9.0,
                width: 4.0,
                start: -1.57,
                sweep: 4.0,
            })
            .at([x, y])
            .fill(colour),
        };
        draws.push(draw.material(material));
    }
    for stack in 0..40 {
        let lane = lanes[1 + stack % 3];
        let y = 200.0 + (stack / 3) as f32 * 64.0 + (t * 0.7 + stack as f32).sin() * 3.0;
        let x = lane + (wave(stack as u32) - 0.5) * 10.0;
        let tilt = (wave(stack as u32 + 99) - 0.5) * 0.1;
        let kind = kinds[stack % 4];
        for depth in [2.0, 1.0] {
            draws.push(
                Draw::rect([0.0, 0.0], [lane_w - 36.0, 60.0], 13.0)
                    .transform(place([x + depth * 5.0, y - depth * 6.0], tilt))
                    .fill(Srgba::WHITE)
                    .stroke(Stroke::solid(line, 2.0))
                    .elevation(1.0)
                    .shadow(Shadow::None)
                    .material(material),
            );
        }
        let front = place([x, y], tilt);
        draws.push(
            Draw::rect([0.0, 0.0], [lane_w - 36.0, 60.0], 13.0)
                .transform(front)
                .fill(Srgba::WHITE)
                .elevation(1.0)
                .shadow(Shadow::Turned)
                .material(material)
                .id(stack as u32 + 1),
        );
        draws.push(
            Draw::rect([0.0, 0.0], [8.0, 62.0], 4.0)
                .transform(multiply(
                    front,
                    translation(-(lane_w - 36.0) * 0.5 + 4.0, 0.0),
                ))
                .fill(kind)
                .elevation(1.0)
                .shadow(Shadow::None),
        );
        draws.push(
            Draw::circle([0.0, 0.0], 17.0)
                .transform(multiply(
                    front,
                    translation(-(lane_w - 36.0) * 0.5 + 37.0, 0.0),
                ))
                .fill(kind.alpha(0.9))
                .elevation(1.0)
                .shadow(Shadow::None),
        );
    }
    for cursor in 0..100 {
        let x =
            lanes[1] - 80.0 + (cursor % 10) as f32 * 18.0 + (t * 2.0 + cursor as f32).sin() * 4.0;
        let y = 800.0 + (cursor / 10) as f32 * 22.0 + (t * 3.0 + cursor as f32 * 0.3).cos() * 3.0;
        let at = multiply(translation(x, y), scaling(0.62, 0.62));
        draws.push(
            Draw::new(Shape::Icon(spike.cursor))
                .transform(multiply(translation(4.0, 7.0), at))
                .fill(ink)
                .opacity(0.22)
                .elevation(1.6)
                .shadow(Shadow::None),
        );
        draws.push(
            Draw::new(Shape::Icon(spike.cursor))
                .transform(at)
                .fill(blue)
                .stroke(Stroke::solid(Srgba::WHITE, 6.0))
                .elevation(1.6)
                .shadow(Shadow::None)
                .id(1000 + cursor),
        );
    }
    for check in 0..500u32 {
        let lane = lanes[1 + (check % 3) as usize];
        let phase = (wave(check) + t * 0.4).fract();
        let x = lane + (wave(check + 7) - 0.5) * 150.0;
        let y = 790.0 - phase * 600.0;
        draws.push(
            Draw::rect([x, y + 24.0], [5.0, 48.0], 2.5)
                .vertical(blue.alpha(0.7), blue.alpha(0.0))
                .elevation(1.5)
                .shadow(Shadow::None),
        );
        draws.push(
            Draw::new(Shape::Icon(spike.check))
                .transform(multiply(translation(x, y), scaling(0.8, 0.8)))
                .stroke(Stroke::solid(blue, 7.0))
                .elevation(1.5)
                .shadow(Shadow::None),
        );
    }
    for label in 0..label_count {
        let stack = label % 40;
        let lane = lanes[1 + stack % 3];
        let y = 200.0 + (stack / 3) as f32 * 64.0 + if label < 40 { 0.0 } else { 26.0 };
        draws.push(
            Draw::new(Shape::Text(label))
                .transform(multiply(
                    translation(lane - 10.0, y + 8.0),
                    scaling(0.5, 0.5),
                ))
                .elevation(1.0)
                .id(2000 + label as u32),
        );
    }
    draws.push(
        Draw::new(Shape::Text(label_count))
            .transform(multiply(translation(1490.0, 560.0), scaling(0.5, 0.5)))
            .elevation(0.0),
    );
}

pub(super) fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_spike_scene_fits_its_budget_at_4k_and_on_a_deck_sized_target() {
    use crate::renderer::Renderer;
    use pfx_text::{Anchor, Face, Representation, Span, Style, TextEngine};
    let spike = spike_icons();
    let curve = test_curve();
    let mut engine = TextEngine::new(SPIKE_FONT).unwrap();
    let face = Face {
        family: "EB Garamond".into(),
        size: 44.0,
        line: 52.0,
        weight: 800,
        italic: false,
        spacing: 0.0,
    };
    let body = "The quick brown fox jumps over the lazy dog, then packs five dozen liquor jugs into a box. ".repeat(12);
    let paragraph = engine
        .layout(
            &body,
            Style {
                family: "EB Garamond",
                size: 32.0,
                line_height: 40.0,
                wrap_width: Some(820.0),
                pixels_per_unit: 1.0,
                representation: Representation::Msdf,
            },
            &pfx_text::Baseline::Straight {
                origin: [0.0, 0.0],
                direction: [1.0, 0.0],
            },
        )
        .unwrap();
    let mut environment_texels = Vec::new();
    for y in 0..64 {
        for x in 0..128 {
            let sky = 1.0 - y as f32 / 64.0;
            environment_texels.push([
                0.6 + 0.4 * sky,
                0.7 + 0.3 * sky,
                0.9,
                1.0 + (x % 7) as f32 * 0.0,
            ]);
        }
    }
    let mut report = String::new();
    let mut budget_misses = Vec::new();
    let variants: [(&str, Material, bool); 5] = [
        ("dials at zero", Material::default(), false),
        (
            "thickness 6, bevel 2",
            Material {
                thickness: 6.0,
                bevel: 2.0,
                ..Material::default()
            },
            false,
        ),
        (
            "gloss 0.4, roughness 0.3",
            Material {
                gloss: 0.4,
                roughness: 0.3,
                ..Material::default()
            },
            false,
        ),
        (
            "reflection 0.5, roughness 0.3",
            Material {
                reflection: 0.5,
                roughness: 0.3,
                ..Material::default()
            },
            true,
        ),
        ("40 patterned cards", Material::default(), false),
    ];
    for size in [[3840u32, 2160u32], [1280, 800]] {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = Renderer::new(gpu, size[0], size[1]).unwrap();
        let target = renderer
            .gpu()
            .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let flat_icons = FlatIcons::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            spike.icons.clone(),
        );
        let environment = Environment::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            128,
            64,
            &environment_texels,
            1.0,
        )
        .unwrap();
        let body_atlas = crate::text::GpuAtlas::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            &paragraph.atlas,
        )
        .unwrap();
        let mut draws = Vec::new();
        let mut turns = pfx_gpu::pace::Turns::default();
        for (round, (variant, material, reflective)) in
            std::iter::once(variants[0]).chain(variants).enumerate()
        {
            let mut samples: Vec<(f64, f64, f64)> = Vec::new();
            let mut glyphs = 0;
            let mut instances = 0;
            for frame in 0..90u32 {
                let t = frame as f32 / 60.0;
                let spans = (0..80)
                    .map(|label| Span {
                        text: format!(
                            "{:.1}e{}\n",
                            1.0 + wave(label * 31 + frame) * 8.9,
                            5 + label % 3
                        ),
                        face: face.clone(),
                        color: [0.106, 0.133, 0.2, 1.0],
                    })
                    .collect::<Vec<_>>();
                let rich = engine
                    .render_spans(
                        &spans,
                        None,
                        1.0,
                        Anchor::Start,
                        Representation::Msdf,
                        [0.0, 0.0],
                        1.0,
                        [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
                        [0.0; 3],
                    )
                    .unwrap();
                let atlas = crate::text::GpuAtlas::new(
                    &renderer.gpu().device,
                    &renderer.gpu().queue,
                    &rich.atlas,
                )
                .unwrap();
                let mut lines: Vec<Vec<crate::text::RichQuad>> = vec![Vec::new(); 80];
                for quad in &rich.quads {
                    if let Some(line) = lines.get_mut(quad.line_index) {
                        let mut placed = crate::text::RichQuad::from(quad);
                        placed.rect[1] -= quad.line_baseline;
                        line.push(placed);
                    }
                }
                let mut text: Vec<FlatText<'_>> = lines
                    .iter()
                    .map(|quads| FlatText {
                        atlas: &atlas,
                        glyphs: Glyphs::Quads(quads),
                        colour: Srgba::hex(0x2b3442),
                    })
                    .collect();
                text.push(FlatText {
                    atlas: &body_atlas,
                    glyphs: Glyphs::Paragraph(&paragraph),
                    colour: Srgba::hex(0x8d96a3),
                });
                glyphs = lines.iter().map(Vec::len).sum::<usize>() + paragraph.glyphs.len();
                spike_draws(&spike, t, material, 80, &mut draws);
                if variant == "40 patterned cards" {
                    let kinds = [
                        PatternKind::Stripes,
                        PatternKind::Dots,
                        PatternKind::CrossHatch,
                        PatternKind::Chevrons,
                    ];
                    for (index, draw) in draws
                        .iter_mut()
                        .filter(|draw| draw.id >= 1 && draw.id <= 40)
                        .enumerate()
                    {
                        draw.pattern = Some(Pattern::new(
                            kinds[index % 4],
                            9.0,
                            0.6,
                            0.3,
                            Srgba::hex(0x8d96a3).alpha(0.35),
                        ));
                    }
                }
                let scene = FlatScene {
                    layout: [1920.0, 1080.0],
                    clear: Some(Srgba::hex(0xeceff3)),
                    curve: &curve,
                    light: Light::default(),
                    draws: &draws,
                    groups: &[],
                    text: &text,
                    icons: Some(&flat_icons),
                    sprites: None,
                    environment: reflective.then_some(&environment),
                    post: false,
                    frame,
                    seed: 0,
                };
                let timings = renderer.render_flat(&scene, &target.view).unwrap();
                instances = renderer.flat_pass().unwrap().instance_count();
                let of = |label: &str| {
                    timings
                        .iter()
                        .filter(|timing| timing.label == label)
                        .map(|timing| timing.milliseconds)
                        .sum::<f64>()
                };
                let colour = of("flat");
                let ids = of("flat ids");
                turns.add(colour + ids);
                if frame >= 10 && !timings.is_empty() {
                    samples.push((colour, ids, colour + ids));
                }
            }
            assert!(samples.len() >= 60, "GPU timestamps are unavailable");
            if round == 0 {
                continue;
            }
            let mut colour = samples.iter().map(|s| s.0).collect::<Vec<_>>();
            let mut ids = samples.iter().map(|s| s.1).collect::<Vec<_>>();
            let mut total = samples.iter().map(|s| s.2).collect::<Vec<_>>();
            let (colour, ids, total) = (median(&mut colour), median(&mut ids), median(&mut total));
            report += &format!(
                "{}x{} {variant}: flat {colour:.3} ms, flat ids {ids:.3} ms, total {total:.3} ms ({} draws, {instances} instances, {glyphs} glyphs)\n",
                size[0],
                size[1],
                draws.len(),
            );
            if size[0] == 3840 && variant == "dials at zero" && total >= 4.0 {
                budget_misses.push(format!("4K flat frame {total:.3} ms"));
            }
        }
    }
    eprintln!("{report}");
    assert!(budget_misses.is_empty(), "{budget_misses:?}\n{report}");
}

fn linear_light(premultiplied: [f32; 4]) -> [f32; 4] {
    let a = premultiplied[3];
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0, 0.0, 0.0, a];
    for c in 0..3 {
        out[c] = pfx_materials::linear_channel(premultiplied[c] / a) * a;
    }
    out
}

fn render_in_linear_light(
    scene: &FlatScene<'_>,
    icons: Option<&Icons>,
    size: [u32; 2],
) -> Vec<[u8; 3]> {
    let fit = Fit::new(scene.layout, size).unwrap();
    let mut order = Order::default();
    order.build(scene.draws, scene.groups.len()).unwrap();
    let pixels = (size[0] * size[1]) as usize;
    let clear = linear_light(scene.clear.unwrap().premultiplied());
    let mut image = vec![clear; pixels];
    let mut layer: Option<(Vec<[f32; 4]>, f32)> = None;
    let over = |dst: &mut [f32; 4], src: [f32; 4]| {
        for c in 0..4 {
            dst[c] = src[c] + dst[c] * (1.0 - src[3]);
        }
    };
    for step in &order.steps {
        match *step {
            Step::Layer(group) => {
                layer = Some((
                    vec![[0.0; 4]; pixels],
                    scene.groups[usize::from(group)].opacity,
                ))
            }
            Step::EndLayer(_) => {
                if let Some((layer, opacity)) = layer.take() {
                    for (dst, src) in image.iter_mut().zip(layer) {
                        over(dst, src.map(|v| v * opacity));
                    }
                }
            }
            Step::Draw(index) => {
                let draw = &scene.draws[index];
                let target = match layer.as_mut() {
                    Some((pixels, _)) => pixels,
                    None => &mut image,
                };
                let instances = [
                    pack_shadow(draw, &fit, scene.curve, &scene.light),
                    pack_shape(draw, &fit, icons),
                ];
                for instance in instances.iter().flatten() {
                    let [x0, y0, x1, y1] = instance.pixel_bounds();
                    for y in (y0.floor().max(0.0) as u32)..(y1.ceil().max(0.0) as u32).min(size[1])
                    {
                        for x in
                            (x0.floor().max(0.0) as u32)..(x1.ceil().max(0.0) as u32).min(size[0])
                        {
                            let local = instance.to_local([x as f32 + 0.5, y as f32 + 0.5]);
                            let colour = cpu::paint(instance, icons, local).colour;
                            over(
                                &mut target[(y * size[0] + x) as usize],
                                linear_light(colour),
                            );
                        }
                    }
                }
            }
        }
    }
    image
        .into_iter()
        .map(|p| {
            std::array::from_fn(|c| {
                let straight = if p[3] > 0.0 { p[c] / p[3] } else { 0.0 };
                (pfx_materials::encode_channel(straight.clamp(0.0, 1.0)) * 255.0).round() as u8
            })
        })
        .collect()
}

#[test]
fn references_composite_in_gamma_encoded_srgb() {
    let curve = test_curve();
    let mut report = String::new();
    let mut gamma_total = 0.0;
    let mut linear_total = 0.0;
    for name in ["card", "faded", "dashed", "glow", "scrim", "pill"] {
        let (parsed, icons) = parsed(name);
        let (size, theirs) = reference(name, 1);
        let scene = scene(&parsed, &curve, None);
        let gamma = cpu::render(&scene, Some(&icons), size)
            .unwrap()
            .into_iter()
            .map(quantize)
            .collect::<Vec<_>>();
        let linear = render_in_linear_light(&scene, Some(&icons), size);
        let gamma = difference(&gamma, &theirs).mean;
        let linear = difference(&linear, &theirs).mean;
        gamma_total += gamma;
        linear_total += linear;
        report += &format!(
            "{name}@1x: gamma-space mean {gamma:.3}/255, linear-light mean {linear:.3}/255\n"
        );
    }
    eprintln!("{report}");
    assert!(gamma_total * 2.0 < linear_total, "{report}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn labels_keep_painters_order_and_their_ids() {
    use pfx_text::{Representation, Style, TextEngine};
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let mut engine = TextEngine::new(SPIKE_FONT).unwrap();
    let paragraph = engine
        .layout(
            "MMMMMMMM",
            Style {
                family: "EB Garamond",
                size: 40.0,
                line_height: 48.0,
                wrap_width: None,
                pixels_per_unit: 1.0,
                representation: Representation::Msdf,
            },
            &pfx_text::Baseline::Straight {
                origin: [0.0, 0.0],
                direction: [1.0, 0.0],
            },
        )
        .unwrap();
    let atlas = crate::text::GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
    let text = [FlatText {
        atlas: &atlas,
        glyphs: Glyphs::Paragraph(&paragraph),
        colour: Srgba::hex(0x2b3442),
    }];
    let curve = ShadowCurve::none();
    let draws = [
        Draw::rect([200.0, 60.0], [400.0, 120.0], 0.0)
            .fill(Srgba::WHITE)
            .elevation(1.0),
        Draw::new(Shape::Text(0))
            .transform(translation(20.0, 80.0))
            .elevation(1.0)
            .id(5),
        Draw::rect([300.0, 60.0], [200.0, 120.0], 0.0)
            .fill(Srgba::WHITE)
            .elevation(2.0)
            .id(6),
    ];
    let scene = FlatScene {
        layout: [400.0, 120.0],
        clear: Some(Srgba::WHITE),
        curve: &curve,
        light: Light::default(),
        draws: &draws,
        groups: &[],
        text: &text,
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let (pixels, ids) = gpu_render(&gpu, &mut pass, &scene, [400, 120]);
    let dark = |x0: usize, x1: usize| {
        (40..90)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| pixels[y * 400 + x][0] < 0.5)
            .count()
    };
    assert!(dark(20, 190) > 200, "the label is missing");
    assert_eq!(dark(205, 400), 0, "the later card does not cover the label");
    assert!(ids.contains(&5));
    assert!(ids[(60 * 400) + 300] == 6);
}

fn pattern_draws() -> Vec<Draw> {
    let card = |centre: [f32; 2], pattern: Pattern| {
        Draw::rect(centre, [120.0, 120.0], 20.0)
            .fill(Srgba::WHITE)
            .pattern(pattern)
    };
    vec![
        card(
            [70.0, 80.0],
            Pattern::new(
                PatternKind::Stripes,
                12.0,
                45f32.to_radians(),
                0.35,
                Srgba::hex(0x3d8fd8).alpha(0.8),
            ),
        ),
        card(
            [200.0, 80.0],
            Pattern::new(PatternKind::Dots, 14.0, 0.0, 0.45, Srgba::hex(0xe2614c)),
        ),
        card(
            [330.0, 80.0],
            Pattern::new(
                PatternKind::CrossHatch,
                16.0,
                30f32.to_radians(),
                0.2,
                Srgba::hex(0x2b3442).alpha(0.5),
            ),
        ),
        card(
            [460.0, 80.0],
            Pattern::new(PatternKind::Chevrons, 18.0, 0.0, 0.3, Srgba::hex(0x138f8a)),
        ),
        Draw::circle([580.0, 80.0], 50.0)
            .fill(Srgba::hex(0xf0b429))
            .pattern(Pattern::new(
                PatternKind::Stripes,
                10.0,
                (-30f32).to_radians(),
                0.5,
                Srgba::WHITE.alpha(0.6),
            )),
    ]
}

fn pattern_scene<'a>(draws: &'a [Draw], curve: &'a ShadowCurve) -> FlatScene<'a> {
    FlatScene {
        layout: [640.0, 160.0],
        clear: Some(Srgba::hex(0xeceff3)),
        curve,
        light: Light::default(),
        draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    }
}

#[test]
fn patterns_match_their_reference_renders() {
    let draws = pattern_draws();
    let curve = ShadowCurve::none();
    let scene = pattern_scene(&draws, &curve);
    let mut report = String::new();
    for scale in [1, 2] {
        let (size, theirs) = reference("patterns", scale);
        let ours = cpu::render(&scene, None, size)
            .unwrap()
            .into_iter()
            .map(quantize)
            .collect::<Vec<_>>();
        let d = difference(&ours, &theirs);
        report += &format!(
            "patterns@{scale}x: mean {:.3}/255, worst {}/255, {:.3}% over 8\n",
            d.mean,
            d.worst,
            d.over * 100.0
        );
        assert!(d.mean < 1.0, "{report}");
    }
    eprintln!("{report}");
    let plain = Draw::rect([0.0, 0.0], [10.0, 10.0], 2.0).fill(Srgba::WHITE);
    let fit = Fit::new([10.0, 10.0], [10, 10]).unwrap();
    let packed = pack_shape(&plain, &fit, None).unwrap();
    assert_eq!(packed.info[0] & PATTERN_MASK, 0);
    assert_eq!((packed.info[2], packed.icon), (0, [0.0; 4]));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_draws_patterns_like_the_reference_and_the_twin() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let draws = pattern_draws();
    let curve = ShadowCurve::none();
    let scene = pattern_scene(&draws, &curve);
    for scale in [1, 2] {
        let (size, theirs) = reference("patterns", scale);
        let (pixels, _) = gpu_render(&gpu, &mut pass, &scene, size);
        let ours = pixels.into_iter().map(quantize).collect::<Vec<_>>();
        let twin = cpu::render(&scene, None, size)
            .unwrap()
            .into_iter()
            .map(quantize)
            .map(|[r, g, b]| [r, g, b, 255])
            .collect::<Vec<_>>();
        let reference = difference(&ours, &theirs);
        let against_twin = difference(&ours, &twin);
        eprintln!(
            "patterns@{scale}x: reference mean {:.3}/255 worst {}/255 ({:.3}% over 8); twin mean {:.4} worst {}",
            reference.mean,
            reference.worst,
            reference.over * 100.0,
            against_twin.mean,
            against_twin.worst
        );
        assert!(reference.mean < 1.0);
        assert!(against_twin.mean < 0.15 && against_twin.worst <= 2);
    }
}

#[test]
fn icon_slabs_scale_with_the_icon() {
    let icons = Icons::bake(&[icons::pointer(), icons::tick()]).unwrap();
    let bold = Material {
        thickness: 10.0,
        bevel: 4.0,
        gloss: 0.6,
        roughness: 0.25,
        reflection: 0.5,
    };
    let card = Draw::rect([0.0, 0.0], [180.0, 62.0], 12.0).material(bold);
    assert_eq!(slab_size(&card, None), Some((10.0, 4.0)));
    let small = Draw::rect([0.0, 0.0], [6.0, 6.0], 1.0).material(bold);
    assert_eq!(slab_size(&small, None), Some((10.0, 3.0)));
    let cursor = Draw::new(Shape::Icon(Icon(0)))
        .transform(scaling(0.62, 0.62))
        .material(bold);
    let entry = *icons.entry(Icon(0)).unwrap();
    let side = (entry.bounds[2] - entry.bounds[0]).min(entry.bounds[3] - entry.bounds[1]);
    let (thickness, bevel) = slab_size(&cursor, Some(&icons)).unwrap();
    assert!((thickness * 0.62 - side * 0.62 / 8.0).abs() < 1e-4);
    assert!((bevel - thickness * 0.5).abs() < 1e-5);
    let check = Draw::new(Shape::Icon(Icon(1)))
        .transform(scaling(0.8, 0.8))
        .stroke(Stroke::solid(Srgba::WHITE, 7.0))
        .material(bold);
    let (_, bevel) = slab_size(&check, Some(&icons)).unwrap();
    assert!(bevel * 0.8 <= 7.0 * 0.8 * 0.5 + 1e-5);
    assert_eq!(slab_size(&card.material(Material::default()), None), None);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn up_facing_faces_keep_their_token_bytes_at_bold() {
    let palette = [
        0x2b3442, 0x36404f, 0x8d96a3, 0xd3d9e1, 0xeceff3, 0xf2f4f7, 0xffffff, 0x138f8a, 0xe2614c,
        0x4c5fd0, 0xf0b429, 0x5f9e62, 0x8e57b0, 0x3d8fd8, 0x1f2632, 0xc5ccd6,
    ];
    let bold = Material {
        thickness: 10.0,
        bevel: 4.0,
        gloss: 0.6,
        roughness: 0.25,
        reflection: 0.5,
    };
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let icons = FlatIcons::new(
        &gpu.device,
        &gpu.queue,
        Icons::bake(&[icons::pointer()]).unwrap(),
    );
    let mut texels = Vec::new();
    for y in 0..32 {
        for x in 0..64 {
            texels.push([0.4 + x as f32 / 64.0, 0.5 + y as f32 / 64.0, 1.2, 1.0]);
        }
    }
    let environment = Environment::new(&gpu.device, &gpu.queue, 64, 32, &texels, 1.0).unwrap();
    let curve = test_curve();
    let mut draws = Vec::new();
    let mut interiors = Vec::new();
    for (index, &rgb) in palette.iter().enumerate() {
        let column = (index % 8) as f32;
        let row = (index / 8) as f32;
        let centre = [60.0 + column * 110.0, 60.0 + row * 220.0];
        draws.push(
            Draw::rect(centre, [90.0, 90.0], 14.0)
                .fill(Srgba::hex(rgb))
                .elevation(1.0)
                .shadow(Shadow::None),
        );
        interiors.push([
            centre[0] - 25.0,
            centre[1] - 25.0,
            centre[0] + 25.0,
            centre[1] + 25.0,
        ]);
        let below = [centre[0], centre[1] + 110.0];
        draws.push(
            Draw::circle(below, 40.0)
                .transform(place(below, 0.3))
                .fill(Srgba::hex(rgb))
                .elevation(1.0)
                .shadow(Shadow::None),
        );
        interiors.push([
            below[0] - 18.0,
            below[1] - 18.0,
            below[0] + 18.0,
            below[1] + 18.0,
        ]);
    }
    draws.push(
        Draw::new(Shape::Icon(Icon(0)))
            .transform(multiply(translation(900.0, 40.0), scaling(3.0, 3.0)))
            .fill(Srgba::hex(0x3d8fd8))
            .elevation(1.0)
            .shadow(Shadow::None),
    );
    interiors.push([910.0, 105.0, 925.0, 135.0]);
    let size = [1040, 480];
    let mut render = |material: Material| {
        let lit = draws
            .iter()
            .map(|draw| draw.material(material))
            .collect::<Vec<_>>();
        let scene = FlatScene {
            layout: [1040.0, 480.0],
            clear: Some(Srgba::hex(0xeceff3)),
            curve: &curve,
            light: Light::default(),
            draws: &lit,
            groups: &[],
            text: &[],
            icons: Some(&icons),
            sprites: None,
            environment: Some(&environment),
            post: false,
            frame: 0,
            seed: 0,
        };
        gpu_render(&gpu, &mut pass, &scene, size).0
    };
    let flat = render(Material::default());
    let lit = render(bold);
    let mut checked = 0;
    for [x0, y0, x1, y1] in interiors {
        for y in y0 as usize..y1 as usize {
            for x in x0 as usize..x1 as usize {
                let at = y * size[0] as usize + x;
                let bits = |p: [f32; 4]| p.map(f32::to_bits);
                assert_eq!(
                    bits(flat[at]),
                    bits(lit[at]),
                    "pixel {x},{y}: {:?} vs {:?}",
                    flat[at],
                    lit[at]
                );
                checked += 1;
            }
        }
    }
    let rims_changed = flat.iter().zip(&lit).filter(|(a, b)| a != b).count();
    eprintln!(
        "{checked} up-facing pixels kept their bits; {rims_changed} rim and side pixels changed"
    );
    assert!(rims_changed > 1000, "the dials did not light the rims");
}

const DECK_PATTERNS: [(&str, u32, PatternKind, f32, f32); 4] = [
    (
        "diagonal stripes",
        0xe2614c,
        PatternKind::Stripes,
        45.0,
        0.45,
    ),
    ("dots", 0x8e57b0, PatternKind::Dots, 0.0, 0.5),
    (
        "horizontal bars",
        0xf0b429,
        PatternKind::Stripes,
        90.0,
        0.45,
    ),
    ("cross-hatch", 0x138f8a, PatternKind::CrossHatch, 45.0, 0.25),
];

const PATTERN_ALPHA: f32 = 0.3;

const DECK_SCALES: [f32; 6] = [12.0, 10.0, 8.0, 6.0, 5.0, 4.0];

fn deck_pattern(kind: PatternKind, angle: f32, width: f32, scale: f32) -> Pattern {
    Pattern::new(
        kind,
        scale,
        angle.to_radians(),
        width,
        Srgba::WHITE.alpha(PATTERN_ALPHA),
    )
}

fn luminance(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn modulation(kind: PatternKind, angle: f32, width: f32, rgb: u32, scale: f32) -> f32 {
    let curve = ShadowCurve::none();
    let draws = [Draw::rect([90.0, 60.0], [150.0, 90.0], 12.0)
        .fill(Srgba::hex(rgb))
        .pattern(deck_pattern(kind, angle, width, scale))];
    let scene = FlatScene {
        layout: [180.0, 120.0],
        clear: Some(Srgba::WHITE),
        curve: &curve,
        light: Light::default(),
        draws: &draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let size = [960, 640];
    let fine = cpu::render(&scene, None, size).unwrap();
    let mut values = Vec::new();
    for y in 16..64 {
        for x in 20..100 {
            let mut sum = 0.0;
            for dy in 0..8 {
                for dx in 0..8 {
                    sum += luminance(fine[(y * 8 + dy) * 960 + x * 8 + dx]);
                }
            }
            values.push(sum / 64.0);
        }
    }
    values.sort_by(f32::total_cmp);
    let low = values[values.len() / 20];
    let high = values[values.len() * 19 / 20];
    let base = Srgba::hex(rgb).0;
    let marked: [f32; 4] = std::array::from_fn(|c| base[c] * (1.0 - PATTERN_ALPHA) + PATTERN_ALPHA);
    (high - low) / (luminance(marked) - luminance(base))
}

const DECK_MINIMUM: [f32; 4] = [8.0, 10.0, 6.0, 10.0];

#[test]
fn patterns_hold_their_shape_at_deck_size() {
    let deck = 2.0 / 3.0;
    let mut report = String::new();
    for ((name, rgb, kind, angle, width), minimum) in DECK_PATTERNS.into_iter().zip(DECK_MINIMUM) {
        let period = minimum * deck;
        let (feature, swing) = match kind {
            PatternKind::Stripes | PatternKind::CrossHatch => {
                (width.min(1.0 - width) * period, f32::MAX)
            }
            PatternKind::Dots => (width.min(1.0 - width) * period, f32::MAX),
            PatternKind::Chevrons => (
                width.min(1.0 - width) * period * std::f32::consts::FRAC_1_SQRT_2,
                period * 0.5,
            ),
        };
        let kept = modulation(kind, angle, width, rgb, minimum);
        report += &format!(
            "{name} ({kind:?}, width {width}, angle {angle}): distinct down to a period of {minimum} layout units, {period:.1} px at 1280x800, smallest feature {feature:.2} px, contrast kept {kept:.2}\n"
        );
        assert!(
            period >= 4.0 && feature >= 1.1 && swing >= 3.0 && kept >= 0.5,
            "{report}"
        );
    }
    eprintln!("{report}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_pattern_sheet_renders_at_deck_size() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = test_curve();
    let mut draws = Vec::new();
    for (row, (_, rgb, kind, angle, width)) in DECK_PATTERNS.into_iter().enumerate() {
        let y = 150.0 + row as f32 * 240.0;
        draws.push(
            Draw::circle([60.0, y], 22.0)
                .fill(Srgba::hex(rgb))
                .elevation(1.0)
                .shadow(Shadow::None),
        );
        for (column, scale) in DECK_SCALES.into_iter().enumerate() {
            let x = 250.0 + column as f32 * 300.0;
            let card = place([x, y - 40.0], 0.0);
            draws.push(
                Draw::rect([0.0, 0.0], [180.8, 62.0], 12.4)
                    .transform(card)
                    .fill(Srgba::WHITE)
                    .elevation(1.0),
            );
            draws.push(
                Draw::rect([0.0, 0.0], [8.0, 62.0], 4.0)
                    .transform(multiply(card, translation(-86.4, 0.0)))
                    .fill(Srgba::hex(rgb))
                    .pattern(deck_pattern(kind, angle, width, scale))
                    .elevation(1.0)
                    .shadow(Shadow::None),
            );
            draws.push(
                Draw::circle([0.0, 0.0], 16.7)
                    .transform(multiply(card, translation(-53.2, 0.0)))
                    .fill(Srgba::hex(rgb))
                    .elevation(1.0)
                    .shadow(Shadow::None),
            );
            draws.push(
                Draw::rect([x, y + 55.0], [180.8, 50.0], 10.0)
                    .fill(Srgba::hex(rgb))
                    .pattern(deck_pattern(kind, angle, width, scale))
                    .elevation(1.0),
            );
        }
    }
    let scene = FlatScene {
        layout: [1920.0, 1080.0],
        clear: Some(Srgba::hex(0xeceff3)),
        curve: &curve,
        light: Light::default(),
        draws: &draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let size = [1280, 800];
    let (pixels, _) = gpu_render(&gpu, &mut pass, &scene, size);
    let dir = format!("{}/../../tmp", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(format!("{dir}/flat-patterns-deck.png")).unwrap();
    let mut encoder = png::Encoder::new(file, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes = pixels.into_iter().flat_map(quantize).collect::<Vec<_>>();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

const SANS: &[u8] = pfx_text::fixture::DM_SANS;

fn sans_face(size: f32, weight: u16) -> pfx_text::Face {
    pfx_text::Face {
        family: "DM Sans".into(),
        size,
        line: size * 1.25,
        weight,
        italic: false,
        spacing: 0.0,
    }
}

fn spans_quads(
    engine: &mut pfx_text::TextEngine,
    text: &str,
    face: pfx_text::Face,
    origin: [f32; 2],
) -> (pfx_text::RichParagraph, Vec<crate::text::RichQuad>) {
    let rich = engine
        .render_spans(
            &[pfx_text::Span {
                text: text.into(),
                face,
                color: [1.0; 4],
            }],
            None,
            1.0,
            pfx_text::Anchor::Start,
            pfx_text::Representation::Msdf,
            origin,
            1.0,
            [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
            [0.0; 3],
        )
        .unwrap();
    let quads = crate::text::rich_quads(&rich);
    (rich, quads)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn text_roles_measure_against_the_deck_9_pixel_rule() {
    let font = SANS;
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut pass = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let mut engine = pfx_text::TextEngine::new(font).unwrap();
    let curve = ShadowCurve::none();
    let mut measure =
        |pass: &mut FlatPass, engine: &mut pfx_text::TextEngine, glyph: &str, size: f32| {
            turns.poll();
            let (rich, quads) = spans_quads(engine, glyph, sans_face(size, 400), [0.0, 0.0]);
            let atlas = crate::text::GpuAtlas::new(&gpu.device, &gpu.queue, &rich.atlas).unwrap();
            let text = [FlatText {
                atlas: &atlas,
                glyphs: Glyphs::Quads(&quads),
                colour: Srgba::hex(0x000000),
            }];
            let draws = [Draw::new(Shape::Text(0)).transform(translation(300.0, 300.0))];
            let scene = FlatScene {
                layout: [1920.0, 1080.0],
                clear: Some(Srgba::WHITE),
                curve: &curve,
                light: Light::default(),
                draws: &draws,
                groups: &[],
                text: &text,
                icons: None,
                sprites: None,
                environment: None,
                post: false,
                frame: 0,
                seed: 0,
            };
            let (pixels, _) = gpu_render(&gpu, pass, &scene, [1280, 800]);
            let rows = (0..800usize)
                .filter(|&y| (0..1280usize).any(|x| pixels[y * 1280 + x][1] < 0.7))
                .count();
            rows as u32
        };
    let mut report = String::new();
    for text_scale in [1.0f32, 1.4] {
        for role in [13.0f32, 14.0, 16.0, 19.0] {
            let cap = measure(&mut pass, &mut engine, "H", role * text_scale);
            let x_height = measure(&mut pass, &mut engine, "x", role * text_scale);
            report += &format!(
                "text scale {text_scale}: role {role} px at 1080 ({:.2} px at 1280x800): cap {cap} px, x-height {x_height} px{}\n",
                role * text_scale * 2.0 / 3.0,
                if x_height >= 9 { ", passes 9 px" } else { "" }
            );
        }
    }
    let mut scale = 1.0f32;
    while measure(&mut pass, &mut engine, "x", 13.0 * scale) < 9 {
        scale += 0.05;
    }
    let mut cap_scale = 1.0f32;
    while measure(&mut pass, &mut engine, "H", 13.0 * cap_scale) < 9 {
        cap_scale += 0.05;
    }
    report += &format!(
        "the 13 px role reaches a 9 px x-height at text scale {scale:.2} and a 9 px cap at {cap_scale:.2}\n"
    );
    eprintln!("{report}");
}

const TEXT_SCALE: f32 = 1.4;

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_deck_spike_with_real_text_sizes() {
    use crate::renderer::Renderer;
    let font = SANS;
    let spike = spike_icons();
    let curve = test_curve();
    let mut engine = pfx_text::TextEngine::new(font).unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let size = [1280u32, 800u32];
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut renderer = turns.cpu(|| Renderer::new(gpu, size[0], size[1]).unwrap());
    let target = renderer
        .gpu()
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let flat_icons = FlatIcons::new(
        &renderer.gpu().device,
        &renderer.gpu().queue,
        spike.icons.clone(),
    );
    let roles: [(f32, u16); 6] = [
        (28.0, 800),
        (22.0, 600),
        (24.0, 400),
        (19.0, 400),
        (38.0, 800),
        (13.0, 600),
    ];
    let subtle = Material {
        thickness: 2.0,
        bevel: 1.0,
        gloss: 0.15,
        reflection: 0.1,
        roughness: 0.45,
    };
    let mut report = String::new();
    let mut draws = Vec::new();
    for (variant, material) in [
        ("dials at zero", Material::default()),
        ("warm-up", Material::default()),
        ("subtle", subtle),
    ] {
        let mut samples = Vec::new();
        let mut glyphs = 0;
        for frame in 0..90u32 {
            let mut paragraphs = Vec::new();
            for label in 0..80u32 {
                turns.poll();
                let (size, weight) = roles[label as usize % roles.len()];
                let size = size * TEXT_SCALE;
                let text = if label < 40 {
                    format!(
                        "{:.1}e{}",
                        1.0 + wave(label * 31 + frame) * 8.9,
                        5 + label % 3
                    )
                } else {
                    format!("Wave {} of 3 · {}", 1 + (frame + label) % 3, label)
                };
                paragraphs.push(spans_quads(
                    &mut engine,
                    &text,
                    sans_face(size, weight),
                    [0.0, 0.0],
                ));
            }
            let body = "The quick brown fox jumps over the lazy dog and keeps on going. ";
            for line in 0..14u32 {
                turns.poll();
                let (size, weight) = roles[(line % 3) as usize + 1];
                let size = size * TEXT_SCALE;
                paragraphs.push(spans_quads(
                    &mut engine,
                    body,
                    sans_face(size, weight),
                    [0.0, 0.0],
                ));
            }
            let atlases = paragraphs
                .iter()
                .map(|(rich, _)| {
                    turns.poll();
                    crate::text::GpuAtlas::new(
                        &renderer.gpu().device,
                        &renderer.gpu().queue,
                        &rich.atlas,
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>();
            let text = paragraphs
                .iter()
                .zip(&atlases)
                .map(|((_, quads), atlas)| FlatText {
                    atlas,
                    glyphs: Glyphs::Quads(quads),
                    colour: Srgba::hex(0x2b3442),
                })
                .collect::<Vec<_>>();
            glyphs = paragraphs
                .iter()
                .map(|(_, quads)| quads.len())
                .sum::<usize>();
            spike_draws(&spike, frame as f32 / 60.0, material, 0, &mut draws);
            draws.retain(|draw| !matches!(draw.shape, Shape::Text(_)));
            for draw in &mut draws {
                if matches!(draw.shape, Shape::Icon(_)) && draw.elevation >= 1.0 {
                    draw.material = material;
                }
            }
            for index in 0..paragraphs.len() {
                let x = if index < 80 {
                    600.0 + (index % 3) as f32 * 200.0
                } else {
                    1490.0
                };
                let y = if index < 80 {
                    210.0 + (index / 3) as f32 * 30.0
                } else {
                    560.0 + (index - 80) as f32 * 32.0
                };
                draws.push(
                    Draw::new(Shape::Text(index))
                        .transform(translation(x, y))
                        .elevation(1.0),
                );
            }
            let scene = FlatScene {
                layout: [1920.0, 1080.0],
                clear: Some(Srgba::hex(0xeceff3)),
                curve: &curve,
                light: Light::default(),
                draws: &draws,
                groups: &[],
                text: &text,
                icons: Some(&flat_icons),
                sprites: None,
                environment: None,
                post: false,
                frame,
                seed: 0,
            };
            turns.poll();
            let timings = renderer.render_flat(&scene, &target.view).unwrap();
            let total = timings
                .iter()
                .filter(|timing| timing.label.starts_with("flat"))
                .map(|timing| timing.milliseconds)
                .sum::<f64>();
            turns.add(total);
            if frame >= 10 && !timings.is_empty() {
                samples.push(total);
            }
        }
        if variant == "warm-up" {
            continue;
        }
        let total = median(&mut samples);
        report += &format!(
            "1280x800 {variant}, DM Sans at roles of 13 to 38 px at text scale {TEXT_SCALE}: flat + ids {total:.3} ms here ({} draws, {glyphs} glyphs); Deck at 8x to 16x: {:.2} to {:.2} ms against 16.7 ms (60 fps) and 11.1 ms (90 fps)\n",
            draws.len(),
            total * 8.0,
            total * 16.0
        );
    }
    eprintln!("{report}");
}
