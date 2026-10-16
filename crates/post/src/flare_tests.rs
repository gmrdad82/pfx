use crate::chain::{Chain, Pass};
use crate::cube;
use crate::flare::Flare;
use crate::grade::{Lut, LutShape};
use crate::image::Image;
use crate::tone::Tone;

fn ramp(width: u32, height: u32) -> Image {
    Image::from_fn(width, height, |x, y| {
        [
            x as f32 / (width - 1) as f32,
            y as f32 / (height - 1) as f32,
            ((x + y) % 7) as f32 / 6.0,
            1.0,
        ]
    })
}

pub(crate) fn lilac(c: [f32; 3]) -> [f32; 3] {
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    [
        (c[0] * 0.9 + l * 0.1 + 0.04).min(1.0),
        (c[1] * 0.86 + l * 0.08).min(1.0),
        (c[2] * 0.9 + l * 0.1 + 0.08).min(1.0),
    ]
}

fn hot(width: u32, height: u32) -> Image {
    Image::from_fn(width, height, |x, y| {
        let dx = x as f32 - width as f32 * 0.3;
        let dy = y as f32 - height as f32 * 0.25;
        let sun = if dx * dx + dy * dy < 36.0 { 60.0 } else { 0.0 };
        let base = 0.2 + 0.1 * ((x / 9 + y / 13) % 3) as f32;
        [base + sun, base + sun * 0.8, base + sun * 0.5, 1.0]
    })
}

#[test]
fn an_identity_cube_leaves_the_image_unchanged() {
    let text = cube::format(&cube::generate(17, |c| c), "identity");
    let lut = cube::parse(&text).unwrap();
    assert_eq!((lut.size, lut.shape), (17, LutShape::Cube));
    let curves = cube::parse("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap();
    let source = ramp(37, 23);
    for lut in [lut, curves] {
        let out = Chain {
            passes: vec![Pass::Lut(lut)],
            seed: 0,
            frame: 0,
        }
        .apply(&source, None, None);
        for (got, want) in out.pixels.iter().zip(&source.pixels) {
            for ch in 0..4 {
                assert!((got[ch] - want[ch]).abs() < 1e-5, "{got:?} {want:?}");
            }
        }
    }
}

#[test]
fn malformed_and_oversized_cubes_are_refused() {
    let entries = |n: usize| "0.5 0.5 0.5\n".repeat(n);
    let cases = [
        String::new(),
        "0 0 0\n".to_string(),
        format!("LUT_3D_SIZE 2\n{}", entries(7)),
        format!("LUT_3D_SIZE 2\n{}", entries(9)),
        format!("LUT_3D_SIZE 2\n{}0.5 0.5\n", entries(7)),
        format!("LUT_3D_SIZE 2\n{}0.5 nan 0.5\n", entries(7)),
        format!("LUT_3D_SIZE 2\n{}0.5 x 0.5\n", entries(7)),
        format!("LUT_3D_SIZE 65\n{}", entries(4)),
        format!("LUT_3D_SIZE 1\n{}", entries(1)),
        format!("LUT_3D_SIZE 18446744073709551617\n{}", entries(8)),
        format!("LUT_3D_SIZE 2\nLUT_1D_SIZE 2\n{}", entries(8)),
        format!("LUT_3D_SIZE 2\nDOMAIN_MIN 1 0 0\n{}", entries(8)),
        format!("LUT_3D_SIZE 2\n{}TITLE \"late\"\n", entries(8)),
        format!("LUT_1D_SIZE 70000\n{}", entries(2)),
        format!("lut_3d_size 2\n{}", entries(8)),
    ];
    for case in &cases {
        assert!(cube::parse(case).is_err(), "{case:?}");
    }
    assert!(cube::parse_bytes(&[0xff, 0xfe, 0x00]).is_err());
    assert!(cube::parse_bytes(&vec![b'#'; cube::MAX_BYTES + 1]).is_err());
    let fine = format!(
        "# made here\nTITLE \"t\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n\n{}",
        entries(8)
    );
    assert_eq!(cube::parse(&fine).unwrap().domain, [[0.0; 3], [2.0; 3]]);
}

#[test]
fn a_finish_reads_its_cube_through_the_caller_and_mixes_by_amount() {
    let text = cube::format(&cube::generate(9, lilac), "lilac");
    let table: toml::Table = "lut = \"looks/lilac.cube\"\nlut_amount = 0.5\ntone = \"aces\"\n"
        .parse()
        .unwrap();
    let mut asked = Vec::new();
    let chain = crate::parse::parse_with(&table, &mut |path: &str| {
        asked.push(path.to_string());
        Ok(text.clone().into_bytes())
    })
    .unwrap();
    assert_eq!(asked, ["looks/lilac.cube"]);
    let Some(Pass::Lut(lut)) = chain.passes.first() else {
        panic!("the lut comes before the tone: {:?}", chain.passes);
    };
    assert!((lut.amount - 0.5).abs() < 1e-6);
    assert!(matches!(chain.passes[1], Pass::Tone(Tone::Aces { .. })));
    let mid = lut.sample([0.5; 3]);
    assert!(mid[0] > mid[1] && mid[2] > mid[1], "{mid:?}");
    assert!(crate::parse::parse(&table).is_err());
    let lone: toml::Table = "lut_amount = 0.5\n".parse().unwrap();
    assert!(crate::parse::parse(&lone).is_err());
}

#[test]
fn a_flare_off_changes_nothing_and_on_mirrors_the_sun() {
    let source = hot(96, 128);
    let base = Chain {
        passes: vec![Pass::Tone(Tone::aces())],
        seed: 0,
        frame: 0,
    };
    let mut off = base.clone();
    off.set_flare(Flare {
        amount: 0.0,
        ..Flare::default()
    });
    assert!(matches!(off.passes[0], Pass::Flare(_)));
    assert_eq!(
        base.apply(&source, None, None),
        off.apply(&source, None, None)
    );
    let mut on = base.clone();
    on.set_flare(Flare {
        amount: 1.0,
        threshold: 2.0,
        ..Flare::default()
    });
    let lit = on.apply(&source, None, None);
    let plain = base.apply(&source, None, None);
    let (x, y) = (96 - 1 - 29, 128 - 1 - 32);
    assert!(
        lit.at(x, y)[0] > plain.at(x, y)[0],
        "the first ghost mirrors the sun through the centre"
    );
}

#[test]
fn a_source_just_over_the_threshold_ghosts_faintly_and_a_sun_fully() {
    let spot = |level: f32| {
        Image::from_fn(96, 128, |x, y| {
            let (dx, dy) = (x as f32 - 28.8, y as f32 - 32.0);
            let c = if dx * dx + dy * dy < 36.0 { level } else { 0.1 };
            [c, c * 0.02, c * 0.01, 1.0]
        })
    };
    let mut chain = Chain::new();
    chain.set_flare(Flare {
        amount: 0.4,
        threshold: 1.5,
        ..Flare::default()
    });
    let ghost = |level: f32| {
        let source = spot(level);
        chain.apply(&source, None, None).at(66, 95)[0] - source.at(66, 95)[0]
    };
    let (tail, sun) = (ghost(3.2), ghost(60.0));
    assert!(sun > 0.05, "{sun}");
    assert!(
        tail < sun * 0.05,
        "a tail light at 3.2 ghosts {tail}, the sun {sun}"
    );
}

mod gpu {
    use super::*;
    use crate::gpu::GpuChain;
    use pfx_gpu::{Gpu, OffscreenTarget};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    fn upload(gpu: &Gpu, source: &Image) -> OffscreenTarget {
        let input = gpu.offscreen(source.width, source.height, FORMAT).unwrap();
        let pixels: Vec<u16> = source
            .pixels
            .iter()
            .flat_map(|p| p.iter().map(|v| half::f16::from_f32(*v).to_bits()))
            .collect();
        gpu.upload_rgba16(&input, &pixels).unwrap();
        input
    }

    fn output(gpu: &Gpu, width: u32, height: u32) -> OffscreenTarget {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flare test output"),
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

    fn run(gpu: &Gpu, chain: &Chain, source: &Image) -> Vec<u16> {
        let input = upload(gpu, source);
        let out = output(gpu, source.width, source.height);
        let post = GpuChain::new(chain.clone(), &gpu.device, source.width, source.height);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flare test"),
            });
        post.run(&mut encoder, &input.view, None, None, &out.view, 0, 0);
        gpu.queue.submit(Some(encoder.finish()));
        gpu.readback_rgba16(&out).unwrap()
    }

    fn near(got: &[u16], want: &Image, tolerance: f32) {
        for (pixel, (got, want)) in got.chunks_exact(4).zip(&want.pixels).enumerate() {
            for ch in 0..3 {
                let got = half::f16::from_bits(got[ch]).to_f32();
                let limit = tolerance * (1.0 + want[ch].abs());
                assert!(
                    (got - want[ch]).abs() <= limit,
                    "pixel {pixel} channel {ch}: {got} vs {}",
                    want[ch]
                );
            }
        }
    }

    fn finish() -> Chain {
        Chain {
            passes: vec![
                Pass::Exposure(1.0),
                Pass::Bloom(crate::bloom::Bloom::neutral(0.2)),
                Pass::Tone(Tone::aces()),
                Pass::Encode,
            ],
            seed: 0,
            frame: 0,
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_flare_off_is_byte_identical() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = hot(200, 360);
        let mut off = finish();
        off.set_flare(Flare {
            amount: 0.0,
            ..Flare::default()
        });
        assert!(matches!(off.passes[2], Pass::Flare(_)));
        assert_eq!(run(&gpu, &finish(), &source), run(&gpu, &off, &source));
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn gpu_flare_and_cube_match_the_cpu() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let source = hot(216, 384);
        let mut chain = Chain::new();
        chain.set_flare(Flare {
            amount: 0.8,
            threshold: 2.0,
            ..Flare::default()
        });
        near(
            &run(&gpu, &chain, &source),
            &chain.apply(&source, None, None),
            0.02,
        );
        let ramped = ramp(64, 48);
        let mut tinted = cube::parse(&cube::format(&cube::generate(33, lilac), "lilac")).unwrap();
        tinted.amount = 0.7;
        let curves = Lut {
            size: 5,
            values: (0..5)
                .map(|i| {
                    let v = i as f32 / 4.0;
                    [v * v, v, v.sqrt()]
                })
                .collect(),
            shape: LutShape::Curves,
            amount: 1.0,
            domain: [[0.0; 3], [2.0; 3]],
        };
        for lut in [tinted, curves] {
            let chain = Chain {
                passes: vec![Pass::Lut(lut)],
                seed: 0,
                frame: 0,
            };
            near(
                &run(&gpu, &chain, &ramped),
                &chain.apply(&ramped, None, None),
                0.004,
            );
        }
    }

    #[test]
    #[ignore = "needs a GPU; run under pgpu"]
    fn benchmark_flare_2160x3840() {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (width, height) = (2160, 3840);
        let mut timer = pfx_gpu::GpuProfiler::new(&gpu.device, &gpu.queue);
        let mut turns = pfx_gpu::pace::Turns::default();
        let input = gpu.offscreen(width, height, FORMAT).unwrap();
        let out = output(&gpu, width, height);
        let mut chain = Chain::new();
        chain.set_flare(Flare::default());
        let post = turns.cpu(|| GpuChain::new(chain.clone(), &gpu.device, width, height));
        let mut samples = Vec::new();
        for attempt in 0..15 {
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("flare benchmark"),
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
                    println!("{} {:.3} ms", pass.label, pass.milliseconds);
                }
            }
            if attempt > 0 {
                samples.push(total);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!("flare {width}x{height} median {:.3} ms", samples[7]);
    }
}
