use super::*;
use crate::flat::{Draw, Light, ShadowCurve, Srgba};
use pfx_gpu::Gpu;
use pfx_gpu::screens::{Aspect, Device, ScreenPolicy};
use pfx_gpu::window::Size;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn pattern(width: u32, height: u32) -> Vec<[f32; 4]> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let checker = ((x / 3 + y / 3) % 2) as f32;
                let ramp = x as f32 / (width - 1) as f32;
                let edge = if x < width / 2 { 0.1 } else { 0.9 };
                [checker, ramp, edge, 1.0].map(|v| half::f16::from_f32(v).to_f32())
            })
        })
        .collect()
}

#[test]
fn the_shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(WGSL).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn scale_one_copies_the_source_exactly() {
    let source = pattern(20, 12);
    let bars = [0.2, 0.3, 0.4, 1.0];
    for filter in [Filter::Sharp, Filter::Nearest] {
        let out = cpu(
            &source,
            [20, 12],
            [26, 16],
            rect(3.0, 2.0, 20.0, 12.0),
            bars,
            filter,
        );
        for y in 0..16 {
            for x in 0..26 {
                let pixel = out[(y * 26 + x) as usize];
                if (3..23).contains(&x) && (2..14).contains(&y) {
                    let texel = source[((y - 2) * 20 + x - 3) as usize];
                    for k in 0..4 {
                        assert!((pixel[k] - texel[k]).abs() < 1e-5, "{filter:?} {x} {y}");
                    }
                } else {
                    assert_eq!(pixel, bars, "{x} {y}");
                }
            }
        }
    }
}

fn bilinear_row(row: &[f32], scale: f32, width: usize) -> Vec<f32> {
    (0..width)
        .map(|x| {
            let u = (x as f32 + 0.5) / scale - 0.5;
            let base = u.floor();
            let t = u - base;
            let at = |i: i32| row[i.clamp(0, row.len() as i32 - 1) as usize];
            at(base as i32) * (1.0 - t) + at(base as i32 + 1) * t
        })
        .collect()
}

#[test]
fn the_sharp_filter_is_steeper_than_bilinear_and_never_rings() {
    let row: Vec<f32> = (0..16).map(|x| if x < 8 { 0.0 } else { 1.0 }).collect();
    let source: Vec<[f32; 4]> = row.iter().map(|v| [*v, *v, *v, 1.0]).collect();
    let out = cpu(
        &source,
        [16, 1],
        [64, 1],
        rect(0.0, 0.0, 64.0, 1.0),
        [0.0; 4],
        Filter::Sharp,
    );
    let sharp: Vec<f32> = out.iter().map(|pixel| pixel[0]).collect();
    let smooth = bilinear_row(&row, 4.0, 64);
    let rise = |values: &[f32]| values.iter().filter(|v| **v > 0.1 && **v < 0.9).count();
    assert!(rise(&sharp) < rise(&smooth), "{sharp:?}");
    assert!(sharp.iter().all(|v| (0.0..=1.0).contains(v)));
    assert!(sharp.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn bars_decode_for_srgb_outputs_and_premultiply() {
    let bars = [0.2, 0.4, 0.6, 1.0];
    assert_eq!(
        bar_value(bars, wgpu::TextureFormat::Rgba16Float),
        [0.2, 0.4, 0.6, 1.0]
    );
    let linear = bar_value(bars, wgpu::TextureFormat::Rgba8UnormSrgb);
    assert!((linear[0] - 0.0331).abs() < 1e-3);
    assert!((linear[2] - 0.3185).abs() < 1e-3);
    assert_eq!(
        bar_value([1.0, 1.0, 1.0, 0.5], wgpu::TextureFormat::Rgba16Float),
        [0.5, 0.5, 0.5, 0.5]
    );
}

#[test]
fn the_hud_clips_to_the_pixels_whose_centres_are_in_the_content() {
    assert_eq!(
        clip(rect(320.0, 0.0, 640.0, 360.0), [1280, 360]),
        Some([320, 0, 640, 360])
    );
    assert_eq!(
        clip(rect(0.0, 40.0, 1280.0, 720.0), [1280, 800]),
        Some([0, 40, 1280, 720])
    );
    assert_eq!(
        clip(rect(10.4, 0.6, 20.0, 10.0), [64, 64]),
        Some([10, 1, 20, 10])
    );
    assert_eq!(
        clip(rect(-8.0, -8.0, 100.0, 100.0), [64, 32]),
        Some([0, 0, 64, 32])
    );
    assert_eq!(clip(rect(70.0, 0.0, 10.0, 10.0), [64, 64]), None);
}

#[test]
fn the_transfer_encodes_once_between_source_and_output() {
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8UnormSrgb, Rgba16Float};
    assert_eq!(
        Transfer::between(Rgba16Float, Bgra8UnormSrgb),
        Transfer::Decode
    );
    assert_eq!(
        Transfer::between(Rgba16Float, Rgba8UnormSrgb),
        Transfer::Decode
    );
    assert_eq!(Transfer::between(Rgba16Float, Rgba16Float), Transfer::Copy);
    assert_eq!(Transfer::between(Rgba16Float, Bgra8Unorm), Transfer::Copy);
    assert_eq!(
        Transfer::between(Rgba8UnormSrgb, Bgra8UnormSrgb),
        Transfer::Copy
    );
    assert_eq!(
        Transfer::between(Rgba8UnormSrgb, Bgra8Unorm),
        Transfer::Encode
    );
}

fn gpu() -> Gpu {
    pollster::block_on(Gpu::headless()).unwrap()
}

fn to_f16(pixels: &[[f32; 4]]) -> Vec<u16> {
    pixels
        .iter()
        .flat_map(|pixel| pixel.map(|v| half::f16::from_f32(v).to_bits()))
        .collect()
}

fn from_f16(bits: &[u16]) -> Vec<[f32; 4]> {
    bits.chunks_exact(4)
        .map(|p| std::array::from_fn(|i| half::f16::from_bits(p[i]).to_f32()))
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_upscale_matches_its_cpu_reference_with_bars_and_a_native_hud() {
    let gpu = gpu();
    let format = wgpu::TextureFormat::Rgba16Float;
    let source_size = [48, 27];
    let pixels = pattern(48, 27);
    let source = gpu.offscreen(48, 27, format).unwrap();
    gpu.upload_rgba16(&source, &to_f16(&pixels)).unwrap();
    let output = gpu.offscreen(128, 80, format).unwrap();
    let policy = ScreenPolicy {
        desktop: vec![Aspect::WIDE],
        bars: [0.25, 0.5, 0.75, 1.0],
        ..ScreenPolicy::default()
    };
    let report = policy
        .fit(
            Device::Desktop,
            Size {
                width: 128,
                height: 80,
            },
        )
        .unwrap();
    assert_eq!(report.content, rect(0.0, 4.0, 128.0, 72.0));
    let curve = ShadowCurve::none();
    let draws = [Draw::rect([240.0, 135.0], [480.0, 270.0], 0.0).fill(Srgba::WHITE)];
    let hud = FlatScene {
        layout: report.layout_units(),
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
    let mut upscale = Upscale::new(&gpu.device);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    upscale
        .encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &Compose {
                source: &source.view,
                source_format: format,
                source_size,
                output: &output.view,
                output_format: format,
                output_size: [128, 80],
                content: report.content,
                bars: report.bar_colour,
                filter: Filter::Sharp,
                backdrop: None,
            },
            None,
        )
        .unwrap();
    upscale
        .encode_overlay(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &Overlay {
                scene: &hud,
                look: None,
                output: &output.view,
                output_format: format,
                output_size: [128, 80],
                content: report.content,
            },
            None,
        )
        .unwrap();
    gpu.queue.submit(Some(encoder.finish()));
    let actual = from_f16(&gpu.readback_rgba16(&output).unwrap());
    let expected = cpu(
        &pixels,
        source_size,
        [128, 80],
        report.content,
        bar_value(report.bar_colour, format),
        Filter::Sharp,
    );
    let mut worst = 0.0f32;
    for y in 0..80usize {
        for x in 0..128usize {
            let got = actual[y * 128 + x];
            if x < 32 && (4..22).contains(&y) {
                assert!(
                    got.iter().all(|v| (v - 1.0).abs() < 1e-3),
                    "hud at {x} {y}: {got:?}"
                );
                continue;
            }
            if x == 32 || y == 22 {
                continue;
            }
            let want = expected[y * 128 + x];
            for k in 0..4 {
                worst = worst.max((got[k] - want[k]).abs());
            }
            if !(4..76).contains(&y) {
                assert_eq!(got, [0.25, 0.5, 0.75, 1.0], "bar at {x} {y}");
            }
        }
    }
    assert!(worst < 2e-3, "worst {worst}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_bars_take_the_games_colour_on_an_srgb_output() {
    let gpu = gpu();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut state = 11u32;
    let bytes: Vec<u8> = (0..40 * 25 * 4)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if i % 4 == 3 { 255 } else { (state >> 24) as u8 }
        })
        .collect();
    let source = gpu.offscreen(40, 25, format).unwrap();
    gpu.upload_rgba8(&source, &bytes).unwrap();
    let output = gpu.offscreen(64, 40, format).unwrap();
    let mut upscale = Upscale::new(&gpu.device);
    for filter in [Filter::Sharp, Filter::Nearest] {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        upscale
            .encode(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &Compose {
                    source: &source.view,
                    source_format: format,
                    source_size: [40, 25],
                    output: &output.view,
                    output_format: format,
                    output_size: [64, 40],
                    content: rect(12.0, 8.0, 40.0, 25.0),
                    bars: [
                        0x33 as f32 / 255.0,
                        0x66 as f32 / 255.0,
                        0x99 as f32 / 255.0,
                        1.0,
                    ],
                    filter,
                    backdrop: None,
                },
                None,
            )
            .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        let read = gpu.readback_rgba8(&output).unwrap();
        for y in 0..40usize {
            for x in 0..64usize {
                let got = &read[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
                let want: [u8; 4] = if (12..52).contains(&x) && (8..33).contains(&y) {
                    let at = ((y - 8) * 40 + x - 12) * 4;
                    [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
                } else {
                    [0x33, 0x66, 0x99, 0xff]
                };
                for k in 0..4 {
                    assert!(
                        got[k].abs_diff(want[k]) <= 1,
                        "{filter:?} {x} {y}: {got:?} against {want:?}"
                    );
                }
            }
        }
    }
    let view = upscale
        .source_target(&gpu.device, [32, 20], format)
        .unwrap();
    drop(view);
    assert!(upscale.source_target(&gpu.device, [0, 20], format).is_err());
    assert_eq!(upscale.source_texture().unwrap().width(), 32);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_upscale_costs_little_at_4k_and_deck_size() {
    let gpu = gpu();
    let format = wgpu::TextureFormat::Rgba16Float;
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut upscale = Upscale::new(&gpu.device);
    let mut report = String::new();
    for (source_size, output_size) in [
        ([1920, 1080], [3840, 2160]),
        ([2880, 1620], [3840, 2160]),
        ([640, 400], [1280, 800]),
    ] {
        let source = gpu
            .offscreen(source_size[0], source_size[1], format)
            .unwrap();
        let output = gpu
            .offscreen(output_size[0], output_size[1], format)
            .unwrap();
        for filter in [Filter::Sharp, Filter::Nearest] {
            let mut profiler = GpuProfiler::new(&gpu.device, &gpu.queue);
            let mut times = Vec::new();
            for frame in 0..64u64 {
                turns.cpu(|| {
                    let mut encoder = gpu
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                    upscale
                        .encode(
                            &gpu.device,
                            &gpu.queue,
                            &mut encoder,
                            &Compose {
                                source: &source.view,
                                source_format: format,
                                source_size,
                                output: &output.view,
                                output_format: format,
                                output_size,
                                content: rect(
                                    0.0,
                                    0.0,
                                    output_size[0] as f32,
                                    output_size[1] as f32,
                                ),
                                bars: [0.0, 0.0, 0.0, 1.0],
                                filter,
                                backdrop: None,
                            },
                            Some(&mut profiler),
                        )
                        .unwrap();
                    let slot = profiler.finish_frame(&mut encoder, frame);
                    gpu.queue.submit(Some(encoder.finish()));
                    if let Some(slot) = slot {
                        profiler.submitted(slot);
                    }
                    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
                    for timings in profiler.collect_frames(&gpu.device) {
                        if timings.frame >= 8 {
                            times.extend(timings.passes.iter().map(|pass| pass.milliseconds));
                        }
                    }
                });
            }
            if times.is_empty() {
                continue;
            }
            times.sort_by(f64::total_cmp);
            report += &format!(
                "{source_size:?} to {output_size:?} {filter:?}: median {:.3} ms over {} frames\n",
                times[times.len() / 2],
                times.len()
            );
        }
    }
    eprintln!("{report}");
    assert!(!report.is_empty() || !gpu.profiling());
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_sub_rect_of_a_larger_source_upscales_like_an_exact_source() {
    let gpu = gpu();
    let format = wgpu::TextureFormat::Rgba16Float;
    let pixels = pattern(48, 27);
    let exact = gpu.offscreen(48, 27, format).unwrap();
    gpu.upload_rgba16(&exact, &to_f16(&pixels)).unwrap();
    let mut padded = vec![[100.0, -50.0, 100.0, 1.0]; 64 * 40];
    for y in 0..27 {
        for x in 0..48 {
            padded[y * 64 + x] = pixels[y * 48 + x];
        }
    }
    let larger = gpu.offscreen(64, 40, format).unwrap();
    gpu.upload_rgba16(&larger, &to_f16(&padded)).unwrap();
    let mut upscale = Upscale::new(&gpu.device);
    for filter in [Filter::Sharp, Filter::Nearest] {
        let mut images = Vec::new();
        for source in [&exact, &larger] {
            let output = gpu.offscreen(128, 80, format).unwrap();
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            upscale
                .encode(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    &Compose {
                        source: &source.view,
                        source_format: format,
                        source_size: [48, 27],
                        output: &output.view,
                        output_format: format,
                        output_size: [128, 80],
                        content: rect(0.0, 4.0, 128.0, 72.0),
                        bars: [0.0, 0.0, 0.0, 1.0],
                        filter,
                        backdrop: None,
                    },
                    None,
                )
                .unwrap();
            gpu.queue.submit(Some(encoder.finish()));
            images.push(gpu.readback_rgba16(&output).unwrap());
        }
        assert!(images[0] == images[1], "{filter:?} read past its rect");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_encoded_float_source_reaches_an_srgb_output_unchanged() {
    let gpu = gpu();
    let source_format = wgpu::TextureFormat::Rgba16Float;
    let pixels: Vec<[f32; 4]> = (0..40 * 25)
        .map(|i| {
            let v = (i % 256) as f32 / 255.0;
            [v, 1.0 - v, ((i * 7) % 256) as f32 / 255.0, 1.0]
        })
        .collect();
    let source = gpu.offscreen(40, 25, source_format).unwrap();
    gpu.upload_rgba16(&source, &to_f16(&pixels)).unwrap();
    let mut upscale = Upscale::new(&gpu.device);
    let output_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let output = gpu.offscreen(64, 40, output_format).unwrap();
    for filter in [Filter::Sharp, Filter::Nearest] {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        upscale
            .encode(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &Compose {
                    source: &source.view,
                    source_format,
                    source_size: [40, 25],
                    output: &output.view,
                    output_format,
                    output_size: [64, 40],
                    content: rect(12.0, 8.0, 40.0, 25.0),
                    bars: [0.2, 0.4, 0.6, 1.0],
                    filter,
                    backdrop: None,
                },
                None,
            )
            .unwrap();
        gpu.queue.submit(Some(encoder.finish()));
        let read = gpu.readback_rgba8(&output).unwrap();
        for y in 0..40usize {
            for x in 0..64usize {
                let got = &read[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
                let want = if (12..52).contains(&x) && (8..33).contains(&y) {
                    pixels[(y - 8) * 40 + x - 12]
                } else {
                    [0.2, 0.4, 0.6, 1.0]
                };
                for k in 0..4 {
                    let want = half::f16::from_f32(want[k]).to_f32();
                    let want = (want * 255.0).round() as u8;
                    assert!(
                        got[k].abs_diff(want) <= 1,
                        "{output_format:?} {filter:?} {x} {y}: {got:?} against {want}"
                    );
                }
            }
        }
    }
}
