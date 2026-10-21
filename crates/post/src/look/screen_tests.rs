use super::gpu::Surface;
use super::screen::*;
use super::{Grain, Place, Vignette};
use pfx_gpu::{Gpu, OffscreenTarget};

const SIZE: [u32; 2] = [320, 200];

fn place() -> Place {
    Place {
        layout: [320.0, 200.0],
        scale: 1.0,
        offset: [0.0, 0.5],
    }
}

fn plan_has(look: &ScreenLook, label: &str) -> bool {
    look.passes().contains(&label)
}

#[test]
fn the_crt_preset_is_the_default_and_switches_every_part_on() {
    let look = ScreenLook::crt();
    assert_eq!(ScreenLook::default(), look);
    assert_eq!(look.validate(), Ok(()));
    for part in ScreenPart::ALL {
        assert!(look.is_on(part), "{}", part.name());
    }
    assert!(!look.is_off());
    assert_eq!(look.curve.strength, 0.02);
    assert_eq!(look.curve.edge, CURVE_EDGE);
    assert_eq!(look.scanlines.strength, 0.065);
    assert_eq!(look.scanlines.period, 2.25);
    assert_eq!(look.grain.response, 0.68);
    assert_eq!(look.bloom.threshold, 0.7);
    assert_eq!(
        look.passes(),
        vec![
            "look grain",
            "look glow",
            "look glow",
            "look glow",
            "look crt",
            "look vignette",
            PRESENT
        ]
    );
}

#[test]
fn a_look_with_every_part_off_records_no_pass() {
    let off = ScreenLook::off();
    assert!(off.is_off());
    assert_eq!(off.validate(), Ok(()));
    assert!(off.passes().is_empty());
    assert_eq!(off.grain_pass(), None);
    assert_eq!(off.crt_pass(), None);
    assert_eq!(off.vignette_pass(&place()), None);
    let mut zero = ScreenLook::crt();
    for part in ScreenPart::ALL {
        zero.set_strength(part, 0.0);
    }
    assert!(zero.is_off(), "a strength of zero is off too");
    assert!(zero.passes().is_empty());
}

#[test]
fn each_part_off_removes_its_pass_and_only_its_pass() {
    let all = ScreenLook::crt();
    let off = all.with(ScreenPart::Grain, false);
    assert!(!plan_has(&off, "look grain"));
    assert_eq!(off.passes().len(), all.passes().len() - 1);
    let off = all.with(ScreenPart::Vignette, false);
    assert!(!plan_has(&off, "look vignette"));
    assert_eq!(off.passes().len(), all.passes().len() - 1);
    let off = all.with(ScreenPart::Bloom, false);
    assert!(!plan_has(&off, "look glow"));
    assert_eq!(off.passes().len(), all.passes().len() - 3);
    assert_eq!(off.crt_pass().unwrap().glow, None);
    let off = all.with(ScreenPart::Curve, false);
    assert_eq!(off.crt_pass().unwrap().curvature, None);
    assert!(off.crt_pass().unwrap().scanlines.is_some());
    let off = all.with(ScreenPart::Scanlines, false);
    assert_eq!(off.crt_pass().unwrap().scanlines, None);
    assert!(off.crt_pass().unwrap().curvature.is_some());
    let shared = all
        .with(ScreenPart::Curve, false)
        .with(ScreenPart::Scanlines, false)
        .with(ScreenPart::Bloom, false);
    assert!(!plan_has(&shared, "look crt"));
    assert_eq!(
        shared.passes(),
        vec!["look grain", "look vignette", PRESENT]
    );
}

#[test]
fn one_part_alone_records_its_passes_and_the_present() {
    let expect: [(ScreenPart, &[&str]); 5] = [
        (ScreenPart::Curve, &["look crt", PRESENT]),
        (ScreenPart::Scanlines, &["look crt", PRESENT]),
        (ScreenPart::Grain, &["look grain", PRESENT]),
        (ScreenPart::Vignette, &["look vignette", PRESENT]),
        (
            ScreenPart::Bloom,
            &["look glow", "look glow", "look glow", "look crt", PRESENT],
        ),
    ];
    for (part, passes) in expect {
        let look = ScreenLook::only(part);
        assert_eq!(look.passes(), passes.to_vec(), "{}", part.name());
        for other in ScreenPart::ALL {
            assert_eq!(look.is_on(other), other == part);
        }
    }
}

#[test]
fn parts_round_trip_their_names() {
    for part in ScreenPart::ALL {
        assert_eq!(ScreenPart::named(part.name()), Some(part));
    }
    assert_eq!(ScreenPart::named("bezel"), None);
}

#[test]
fn bad_values_are_refused_naming_the_part_and_field() {
    type Bend = fn(&mut ScreenLook, f32);
    let cases: [(Bend, &[f32], &str); 13] = [
        (
            |l, v| l.curve.strength = v,
            &[-0.01, 0.26],
            "curve.strength",
        ),
        (|l, v| l.curve.edge = v, &[-0.1, 0.6], "curve.edge"),
        (
            |l, v| l.scanlines.strength = v,
            &[-0.1, 1.1],
            "scanlines.strength",
        ),
        (
            |l, v| l.scanlines.period = v,
            &[0.5, 65.0],
            "scanlines.period",
        ),
        (|l, v| l.grain.strength = v, &[-0.1, 1.5], "grain.strength"),
        (|l, v| l.grain.size = v, &[0.0, -1.0, 65.0], "grain.size"),
        (|l, v| l.grain.response = v, &[-0.1, 1.1], "grain.response"),
        (
            |l, v| l.vignette.strength = v,
            &[-0.1, 1.1],
            "vignette.strength",
        ),
        (|l, v| l.vignette.radius = v, &[0.0, 5.0], "vignette.radius"),
        (
            |l, v| l.vignette.falloff = v,
            &[0.0, 4.5],
            "vignette.falloff",
        ),
        (|l, v| l.bloom.strength = v, &[-1.0, 4.5], "bloom.strength"),
        (
            |l, v| l.bloom.threshold = v,
            &[-0.1, 1.1],
            "bloom.threshold",
        ),
        (|l, v| l.bloom.radius = v, &[0.0, 65.0], "bloom.radius"),
    ];
    for (bend, values, field) in cases {
        for &value in values.iter().chain(&[f32::NAN, f32::INFINITY]) {
            let mut look = ScreenLook::crt();
            bend(&mut look, value);
            let error = look.validate().unwrap_err();
            assert!(error.contains(field), "{field} = {value}: {error}");
            assert!(!error.contains('\n'), "one line: {error}");
            let mut off = look;
            for part in ScreenPart::ALL {
                off.set(part, false);
            }
            assert!(
                off.validate().is_err(),
                "a part that is off still holds valid values: {field}"
            );
        }
    }
}

#[test]
fn range_ends_are_accepted() {
    let mut look = ScreenLook::crt();
    look.curve.strength = CURVE_MAX;
    look.curve.edge = 0.0;
    look.scanlines.strength = 1.0;
    look.scanlines.period = PERIOD_MIN;
    look.grain.strength = 1.0;
    look.grain.size = LENGTH_MAX;
    look.grain.response = 0.0;
    look.vignette.strength = 1.0;
    look.vignette.radius = VIGNETTE_MAX;
    look.vignette.falloff = VIGNETTE_MAX;
    look.bloom.strength = BLOOM_MAX;
    look.bloom.threshold = 1.0;
    look.bloom.radius = LENGTH_MAX;
    assert_eq!(look.validate(), Ok(()));
    look.scanlines.period = LENGTH_MAX;
    look.grain.response = 1.0;
    look.bloom.threshold = 0.0;
    assert_eq!(look.validate(), Ok(()));
}

#[test]
fn the_parts_map_onto_the_flat_look_passes() {
    let look = ScreenLook::crt();
    let crt = look.crt_pass().unwrap();
    let lines = crt.scanlines.unwrap();
    assert_eq!((lines.period, lines.strength), (2.25, 0.065));
    let curve = crt.curvature.unwrap();
    assert_eq!((curve.amount, curve.corner), (0.02, CURVE_EDGE));
    assert_eq!(curve.bezel, [0.0, 0.0, 0.0, 1.0]);
    let glow = crt.glow.unwrap();
    assert_eq!(
        (glow.threshold, glow.radius, glow.strength),
        (0.7, 4.0, 0.3)
    );
    assert_eq!(crt.aberration, None);
    assert_eq!(
        look.grain_pass(),
        Some(Grain {
            amount: 0.04,
            size: 1.0,
            response: 0.68
        })
    );
}

#[test]
fn grain_stays_off_bright_ink_by_its_response() {
    let look = ScreenLook::crt();
    let grain = look.grain_pass().unwrap();
    let shift = Grain::shift(3, 11);
    let spread = |v: f32| {
        (0..400)
            .map(|i| {
                let p = [(i % 20) as f32 * 1.7, (i / 20) as f32 * 1.3];
                (grain.pixel([v, v, v, 1.0], p, shift)[0] - v).abs()
            })
            .fold(0.0f32, f32::max)
    };
    let ink = spread(1.0);
    let dark = spread(0.1);
    assert!(ink <= grain.amount * (1.0 - grain.response) + 1e-6);
    assert!(dark > ink * 2.0, "dark {dark} against bright ink {ink}");
}

#[test]
fn the_vignette_falls_off_from_its_radius_to_its_strength() {
    let mut look = ScreenLook::only(ScreenPart::Vignette);
    look.vignette = VignettePart {
        on: true,
        strength: 0.4,
        radius: 0.5,
        falloff: 0.5,
    };
    let vignette: Vignette = look.vignette_pass(&place()).unwrap();
    assert_eq!(vignette.centre, [160.0, 100.0]);
    assert_eq!(vignette.radius, [80.0, 50.0]);
    assert_eq!(vignette.light([160.0, 100.0]), 1.0);
    assert_eq!(vignette.light([240.0, 100.0]), 1.0);
    assert!((vignette.light([320.0, 100.0]) - 0.6).abs() < 1e-6);
    let mid = vignette.light([280.0, 100.0]);
    assert!(mid < 1.0 && mid > 0.6);
    assert!(vignette.light([300.0, 100.0]) < mid);
}

#[test]
fn the_screen_shader_validates_and_presents_in_the_output_encoding() {
    let module = naga::front::wgsl::parse_str(SCREEN_WGSL)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(SCREEN_WGSL)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    assert_eq!(
        pfx_gpu::layout_shortfall(&[present_entries().as_slice()], &wgpu::Limits::default()),
        None
    );
    use wgpu::TextureFormat::*;
    assert_eq!(present_entry(Bgra8UnormSrgb), "screen_present_decode");
    assert_eq!(present_entry(Rgba8UnormSrgb), "screen_present_decode");
    assert_eq!(present_entry(Bgra8Unorm), "screen_present");
    assert_eq!(present_entry(Rgba16Float), "screen_present");
    assert_eq!(canvas_format(Bgra8UnormSrgb), Bgra8Unorm);
    assert_eq!(canvas_format(Rgba16Float), Rgba16Float);
}

fn device() -> Gpu {
    pollster::block_on(Gpu::headless()).unwrap()
}

fn halves(pixels: &[[f32; 4]]) -> Vec<u16> {
    pixels
        .iter()
        .flatten()
        .map(|v| half::f16::from_f32(*v).to_bits())
        .collect()
}

fn upload(gpu: &Gpu, pixels: &[[f32; 4]]) -> OffscreenTarget {
    let target = gpu
        .offscreen(SIZE[0], SIZE[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    gpu.queue.write_texture(
        target.texture.as_image_copy(),
        bytemuck::cast_slice(&halves(pixels)),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE[0] * 8),
            rows_per_image: Some(SIZE[1]),
        },
        wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
    );
    target
}

fn read(gpu: &Gpu, target: &OffscreenTarget) -> Vec<[f32; 4]> {
    gpu.readback_rgba16(target)
        .unwrap()
        .chunks_exact(4)
        .map(|p| std::array::from_fn(|i| half::f16::from_bits(p[i]).to_f32()))
        .collect()
}

fn f16(pixels: &[[f32; 4]]) -> Vec<[f32; 4]> {
    pixels
        .iter()
        .map(|p| p.map(|v| half::f16::from_f32(v).to_f32()))
        .collect()
}

fn grey(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

fn flat(v: f32) -> Vec<[f32; 4]> {
    vec![grey(v); (SIZE[0] * SIZE[1]) as usize]
}

fn ramp() -> Vec<[f32; 4]> {
    f16(&(0..SIZE[0] * SIZE[1])
        .map(|i| {
            let u = (i % SIZE[0]) as f32 / SIZE[0] as f32;
            let v = (i / SIZE[0]) as f32 / SIZE[1] as f32;
            [u, v, 1.0 - u * v, 1.0]
        })
        .collect::<Vec<_>>())
}

struct Bench {
    gpu: Gpu,
    screen: ScreenGpu,
    output: OffscreenTarget,
}

impl Bench {
    fn new() -> Self {
        let gpu = device();
        let screen = ScreenGpu::new(&gpu.device, &gpu.queue);
        let output = gpu
            .offscreen(SIZE[0], SIZE[1], wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        Self {
            gpu,
            screen,
            output,
        }
    }

    fn run(
        &mut self,
        pixels: &[[f32; 4]],
        look: &ScreenLook,
        frame: u32,
    ) -> (usize, Vec<[f32; 4]>) {
        let source = upload(&self.gpu, pixels);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let recorded = self
            .screen
            .encode(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                Surface {
                    view: &source.view,
                    size: SIZE,
                },
                &self.output.view,
                self.output.format,
                look,
                &place(),
                frame,
                9,
                None,
            )
            .unwrap();
        self.gpu.queue.submit(Some(encoder.finish()));
        (recorded, read(&self.gpu, &self.output))
    }
}

fn at(pixels: &[[f32; 4]], x: u32, y: u32) -> [f32; 4] {
    pixels[(y * SIZE[0] + x) as usize]
}

fn worst(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter()
        .zip(b)
        .flat_map(|(x, y)| (0..4).map(move |i| (x[i] - y[i]).abs()))
        .fold(0.0, f32::max)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_scanlines_darken_every_period_th_row_by_their_strength() {
    let mut bench = Bench::new();
    let mut look = ScreenLook::only(ScreenPart::Scanlines);
    look.scanlines.strength = 0.25;
    look.scanlines.period = 4.0;
    let (recorded, lined) = bench.run(&flat(0.8), &look, 0);
    assert_eq!(recorded, look.passes().len());
    for row in 0..SIZE[1] {
        for x in [0, 77, SIZE[0] - 1] {
            let v = at(&lined, x, row)[0];
            match row % 4 {
                0 => assert!((v - 0.6).abs() < 2e-3, "row {row}: {v}"),
                2 => assert!((v - 0.8).abs() < 2e-3, "row {row}: {v}"),
                _ => assert!(v > 0.6 - 2e-3 && v < 0.8 + 2e-3, "row {row}: {v}"),
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_curve_leaves_the_corners_black_past_its_edge_and_keeps_the_centre() {
    let mut bench = Bench::new();
    let mut look = ScreenLook::only(ScreenPart::Curve);
    look.curve.strength = 0.1;
    look.curve.edge = 0.05;
    let pixels = ramp();
    let (_, curved) = bench.run(&pixels, &look, 0);
    for (x, y) in [
        (0, 0),
        (SIZE[0] - 1, 0),
        (0, SIZE[1] - 1),
        (SIZE[0] - 1, SIZE[1] - 1),
        (2, 3),
    ] {
        assert_eq!(at(&curved, x, y), [0.0, 0.0, 0.0, 1.0], "corner {x},{y}");
    }
    let centre = at(&curved, SIZE[0] / 2, SIZE[1] / 2);
    let want = at(&pixels, SIZE[0] / 2, SIZE[1] / 2);
    assert!(
        (0..4).all(|i| (centre[i] - want[i]).abs() < 4e-3),
        "{centre:?} against {want:?}"
    );
    let mid_edge = at(&curved, SIZE[0] / 2, 0);
    let inside = at(&curved, SIZE[0] / 2, 12);
    assert!(mid_edge[1] < inside[1], "the edge falls off toward black");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_grain_stays_off_bright_ink_and_follows_the_frame_index() {
    let mut bench = Bench::new();
    let mut look = ScreenLook::only(ScreenPart::Grain);
    look.grain.strength = 0.2;
    look.grain.response = 1.0;
    let (_, white) = bench.run(&flat(1.0), &look, 4);
    assert!(worst(&white, &flat(1.0)) < 1e-3, "white ink takes no grain");
    look.grain.response = 0.68;
    let (_, ink) = bench.run(&flat(1.0), &look, 4);
    let (_, dark) = bench.run(&flat(0.2), &look, 4);
    let spread = |pixels: &[[f32; 4]], v: f32| {
        pixels
            .iter()
            .map(|p| (p[0] - v).abs())
            .fold(0.0f32, f32::max)
    };
    assert!(spread(&ink, 1.0) <= 0.2 * 0.32 + 2e-3);
    assert!(spread(&dark, 0.2) > spread(&ink, 1.0) * 1.5);
    let twin = Grain {
        amount: 0.2,
        size: 1.0,
        response: 0.68,
    }
    .image(&f16(&flat(0.2)), SIZE, &place(), 4, 9);
    assert!(worst(&dark, &twin) < 2e-3, "{}", worst(&dark, &twin));
    let (_, again) = bench.run(&flat(0.2), &look, 4);
    assert!(again == dark, "one frame index gives one grain");
    let (_, next) = bench.run(&flat(0.2), &look, 5);
    assert!(next != dark, "the next frame index moves the grain");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_vignette_falls_off_as_its_twin_says() {
    let mut bench = Bench::new();
    let mut look = ScreenLook::only(ScreenPart::Vignette);
    look.vignette.strength = 0.5;
    look.vignette.radius = 0.6;
    look.vignette.falloff = 0.5;
    let pixels = f16(&flat(0.8));
    let (_, shaded) = bench.run(&pixels, &look, 0);
    let twin = look
        .vignette_pass(&place())
        .unwrap()
        .image(&pixels, SIZE, &place());
    assert!(worst(&shaded, &twin) < 2e-3, "{}", worst(&shaded, &twin));
    assert!((at(&shaded, 160, 100)[0] - 0.8).abs() < 1e-3);
    assert!((at(&shaded, 0, 0)[0] - 0.4).abs() < 2e-3);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_bloom_leaves_pixels_under_its_threshold_untouched() {
    let mut bench = Bench::new();
    let mut look = ScreenLook::only(ScreenPart::Bloom);
    look.bloom.threshold = 0.6;
    look.bloom.strength = 1.0;
    look.bloom.radius = 3.0;
    let dim = f16(&flat(0.5));
    let (recorded, out) = bench.run(&dim, &look, 0);
    assert_eq!(recorded, 5);
    assert!(out == dim, "light under the threshold adds nothing");
    let mut bright = dim.clone();
    for y in 90..110 {
        for x in 150..170 {
            bright[(y * SIZE[0] + x) as usize] = grey(1.0);
        }
    }
    let (_, out) = bench.run(&bright, &look, 0);
    assert!(at(&out, 160, 100)[0] > 1.05);
    assert!(
        at(&out, 145, 100)[0] > 0.5 + 1e-3,
        "light spills past the edge"
    );
    assert_eq!(
        at(&out, 10, 10),
        at(&bright, 10, 10),
        "far pixels keep their bits"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_stack_with_every_part_on_is_deterministic_and_off_records_nothing() {
    let mut bench = Bench::new();
    let look = ScreenLook::crt();
    let pixels = ramp();
    let (recorded, first) = bench.run(&pixels, &look, 7);
    assert_eq!(recorded, look.passes().len());
    let (_, second) = bench.run(&pixels, &look, 7);
    assert!(first == second, "two renders of one frame are equal");
    assert_eq!(at(&first, 0, 0), [0.0, 0.0, 0.0, 1.0]);
    let (recorded, untouched) = bench.run(&pixels, &ScreenLook::off(), 7);
    assert_eq!(recorded, 0);
    assert!(untouched == first, "an off look writes nothing");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_present_into_an_srgb_output_hands_back_the_bytes_it_read() {
    let gpu = device();
    let mut screen = ScreenGpu::new(&gpu.device, &gpu.queue);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let canvas = Canvas::new(&gpu.device, SIZE, format);
    let bytes: Vec<u8> = (0..SIZE[0] * SIZE[1])
        .flat_map(|i| {
            let x = (i % SIZE[0]) as u8;
            let y = (i / SIZE[0]) as u8;
            [x, y, x ^ y, 255]
        })
        .collect();
    gpu.queue.write_texture(
        canvas.texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE[0] * 4),
            rows_per_image: Some(SIZE[1]),
        },
        wgpu::Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
    );
    let output = gpu.offscreen(SIZE[0], SIZE[1], format).unwrap();
    let mut look = ScreenLook::only(ScreenPart::Vignette);
    look.vignette.radius = VIGNETTE_MAX;
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let recorded = screen
        .encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            Surface {
                view: &canvas.read,
                size: SIZE,
            },
            &output.view,
            format,
            &look,
            &place(),
            0,
            0,
            None,
        )
        .unwrap();
    gpu.queue.submit(Some(encoder.finish()));
    assert_eq!(recorded, 2);
    let back = gpu.readback_rgba8(&output).unwrap();
    let off = back
        .iter()
        .zip(&bytes)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(
        off <= 1,
        "a look that changes nothing keeps the bytes: {off}"
    );
}
