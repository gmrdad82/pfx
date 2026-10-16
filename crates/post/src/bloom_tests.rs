use crate::bloom::{self, Bloom, BloomKind};
use crate::chain::{Chain, Pass};
use crate::image::Image;
use crate::parse::parse_str;

pub(crate) const SIZE: u32 = 512;

pub(crate) fn point(value: f32) -> Image {
    let mut image = Image::new(SIZE, SIZE);
    let i = image.index(SIZE / 2, SIZE / 2);
    image.pixels[i] = [value, value, value, 1.0];
    image
}

pub(crate) fn blob(peak: f32, sigma: f32) -> Image {
    let c = SIZE as f32 / 2.0;
    Image::from_fn(SIZE, SIZE, |x, y| {
        let dx = x as f32 + 0.5 - c;
        let dy = y as f32 + 0.5 - c;
        let v = peak * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
        [v, v * 0.8, v * 0.6, 1.0]
    })
}

pub(crate) fn delta(out: &Image, source: &Image) -> Image {
    Image::from_fn(out.width, out.height, |x, y| {
        let a = out.at(x, y);
        let b = source.at(x, y);
        [a[0] - b[0], a[1] - b[1], a[2] - b[2], 1.0]
    })
}

fn profile(glow: &Image, radius: f32, diagonal: bool) -> f32 {
    let c = SIZE as f32 / 2.0;
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let dirs: [(f32, f32); 4] = if diagonal {
        [(d, d), (-d, d), (d, -d), (-d, -d)]
    } else {
        [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
    };
    dirs.iter()
        .map(|(dx, dy)| glow.sample(c + dx * radius, c + dy * radius)[0])
        .sum::<f32>()
        / 4.0
}

pub(crate) fn roundness(glow: &Image) -> f32 {
    [12.0, 24.0, 48.0, 96.0, 160.0]
        .into_iter()
        .map(|radius| {
            let axis = profile(glow, radius, false);
            let diagonal = profile(glow, radius, true);
            assert!(axis > 0.0, "no glow at {radius}");
            (axis - diagonal).abs() / axis
        })
        .fold(0.0, f32::max)
}

pub(crate) fn total(image: &Image) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for p in &image.pixels {
        for ch in 0..3 {
            sum[ch] += f64::from(p[ch]);
        }
    }
    sum
}

#[test]
fn a_bright_pixel_blooms_into_a_round_glow() {
    let source = point(4000.0);
    let out = bloom::apply(&source, &Bloom::round(0.15, 1.0));
    let off = roundness(&delta(&out, &source));
    assert!(off <= 0.1, "axis and diagonal {:.1}% apart", off * 100.0);
}

#[test]
fn the_round_bloom_conserves_energy() {
    for threshold in [0.0, 1.0] {
        let source = blob(6.0, 6.0);
        let out = bloom::apply(&source, &Bloom::round(0.5, threshold));
        let (before, after) = (total(&source), total(&out));
        for ch in 0..3 {
            let off = (after[ch] - before[ch]).abs() / before[ch];
            assert!(
                off < 0.02,
                "threshold {threshold} channel {ch}: {} before, {} after",
                before[ch],
                after[ch]
            );
        }
    }
}

#[test]
fn a_finish_without_a_kind_blooms_round_and_named_kinds_keep_theirs() {
    let kind = |text: &str| {
        parse_str(text)
            .unwrap()
            .passes
            .iter()
            .find_map(|pass| match pass {
                Pass::Bloom(bloom) => Some(bloom.kind),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(kind("bloom = 0.3\n"), BloomKind::Round);
    assert_eq!(
        kind("bloom = { strength = 0.3, threshold = 0.9 }\n"),
        BloomKind::Round
    );
    assert_eq!(
        kind("bloom = { kind = \"round\", strength = 0.3 }\n"),
        BloomKind::Round
    );
    assert_eq!(
        kind("bloom = { kind = \"box\", strength = 0.3 }\n"),
        BloomKind::Box
    );
    assert_eq!(
        kind("bloom = { kind = \"tent\", strength = 0.3 }\n"),
        BloomKind::Tent
    );
    assert_eq!(
        kind("bloom = { kind = \"rings\", strength = 0.3 }\n"),
        BloomKind::Rings
    );
    assert_eq!(kind("style = \"crt\"\nbloom = 0.3\n"), BloomKind::Box);
}

#[test]
fn the_round_bloom_at_zero_strength_is_identity() {
    let chain = Chain {
        passes: vec![Pass::Bloom(Bloom::round(0.0, 1.0))],
        seed: 0,
        frame: 0,
    };
    let source = blob(6.0, 6.0);
    assert_eq!(chain.apply(&source, None, None), source);
}

mod gpu {
    use super::*;
    use crate::gpu::GpuChain;
    use crate::tone::Tone;
    use pfx_gpu::{Gpu, OffscreenTarget};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("round bloom test output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        OffscreenTarget {
            texture,
            view,
            format: FORMAT,
            width,
            height,
        }
    }

    fn run(gpu: &Gpu, chain: &Chain, source: &Image) -> Image {
        let input = gpu.offscreen(source.width, source.height, FORMAT).unwrap();
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half::f16::from_f32(*v).to_bits()))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        let out = output(gpu, source.width, source.height);
        let post = GpuChain::new(chain.clone(), &gpu.device, source.width, source.height);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("round bloom test"),
            });
        post.run(&mut encoder, &input.view, None, None, &out.view, 0, 0);
        gpu.queue.submit(Some(encoder.finish()));
        let read = gpu.readback_rgba16(&out).unwrap();
        let pixels = read
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|ch| half::f16::from_bits(p[ch]).to_f32()))
            .collect();
        Image::from_pixels(source.width, source.height, pixels).unwrap()
    }

    fn bloom_only(bloom: Bloom) -> Chain {
        Chain {
            passes: vec![Pass::Bloom(bloom)],
            seed: 0,
            frame: 0,
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_round_bloom_is_round_conserves_energy_and_matches_the_cpu() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = point(4000.0);
        let chain = bloom_only(Bloom::round(0.15, 1.0));
        let got = delta(&run(&gpu, &chain, &source), &source);
        let want = delta(&chain.apply(&source, None, None), &source);
        let off = roundness(&got);
        assert!(off <= 0.1, "axis and diagonal {:.1}% apart", off * 100.0);
        let centre = SIZE / 2;
        for (x, y) in [
            (centre + 12, centre),
            (centre + 40, centre + 40),
            (centre, centre + 150),
        ] {
            let (g, w) = (got.at(x, y)[0], want.at(x, y)[0]);
            assert!(
                (g - w).abs() <= 0.05 * w.abs() + 1e-4,
                "({x}, {y}): {g} vs {w}"
            );
        }
        for threshold in [0.0, 1.0] {
            let source = blob(6.0, 6.0);
            let out = run(&gpu, &bloom_only(Bloom::round(0.5, threshold)), &source);
            let (before, after) = (total(&source), total(&out));
            for ch in 0..3 {
                let off = (after[ch] - before[ch]).abs() / before[ch];
                assert!(off < 0.02, "threshold {threshold} channel {ch}: {off}");
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_fused_round_finish_matches_the_cpu() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = Image::from_fn(240, 432, |x, y| {
            let dx = x as f32 - 80.0;
            let dy = y as f32 - 120.0;
            let sun = if dx * dx + dy * dy < 49.0 { 300.0 } else { 0.0 };
            let base = 0.15 + 0.1 * ((x / 9 + y / 13) % 3) as f32;
            [base + sun, base + sun * 0.8, base + sun * 0.5, 1.0]
        });
        let chain = Chain {
            passes: vec![
                Pass::Exposure(1.3),
                Pass::Bloom(Bloom::round(0.3, 1.0)),
                Pass::Tone(Tone::aces()),
                Pass::Encode,
            ],
            seed: 0,
            frame: 0,
        };
        let got = run(&gpu, &chain, &source);
        let want = chain.apply(&source, None, None);
        let encode = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        let mut worst = 0;
        for (g, w) in got.pixels.iter().zip(&want.pixels) {
            for ch in 0..3 {
                worst = worst.max((encode(g[ch]) - encode(w[ch])).abs());
            }
        }
        assert!(worst <= 2, "{worst}/255");
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_round_bloom_2160x3840() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (2160, 3840);
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        let input = gpu.offscreen(width, height, FORMAT).unwrap();
        let out = output(&gpu, width, height);
        for (name, bloom) in [
            (
                "box",
                Bloom::pyramid(0.15, 0.9, 0.7, 0.6, 0.15, 12, 50.0, 2),
            ),
            ("round", Bloom::round(0.15, 0.9)),
        ] {
            let chain = Chain {
                passes: vec![
                    Pass::Exposure(1.2),
                    Pass::Bloom(bloom),
                    Pass::Tone(Tone::aces()),
                ],
                seed: 0,
                frame: 0,
            };
            let post = turns.cpu(|| GpuChain::new(chain.clone(), &gpu.device, width, height));
            let mut samples = Vec::new();
            for attempt in 0..15 {
                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("round bloom benchmark"),
                        });
                post.run_timed(
                    &mut encoder,
                    &input.view,
                    None,
                    None,
                    &out.view,
                    0,
                    0,
                    Some(&mut timer),
                );
                let slot = timer.finish(&mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                if let Some(slot) = slot {
                    timer.submitted(slot);
                }
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                let timings = timer.collect(&gpu.device);
                let passes: Vec<_> = timings.iter().flatten().collect();
                let total: f64 = passes.iter().map(|pass| pass.milliseconds).sum();
                turns.add(total);
                if attempt == 14 {
                    for pass in &passes {
                        println!("{name} {} {:.3} ms", pass.label, pass.milliseconds);
                    }
                }
                if attempt > 0 {
                    samples.push(total);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{name} bloom + grade {width}x{height} median {:.3} ms",
                samples[7]
            );
        }
    }
}
