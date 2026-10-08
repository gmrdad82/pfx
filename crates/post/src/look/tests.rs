use super::gpu::{LookGpu, Surface};
use super::*;
use pfx_gpu::{Gpu, OffscreenTarget};

fn neutral_palette() -> Palette {
    Palette::hex(&[
        0x101418, 0x2a3038, 0x4a525c, 0x6e7781, 0x959ea8, 0xc2c9d0, 0xeef1f4, 0x7a2e2e, 0xb8574a,
        0xd9a05b, 0xe8d77a, 0x4f7a3a, 0x7fb069, 0x2f5d7c, 0x5fa3c7, 0x6b4e8c,
    ])
    .unwrap()
}

fn grey(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

#[test]
fn bayer_matrices_are_the_standard_ones() {
    assert_eq!(bayer_matrix(2).unwrap(), vec![0, 2, 3, 1]);
    assert_eq!(
        bayer_matrix(4).unwrap(),
        vec![0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5]
    );
    let eight = bayer_matrix(8).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            let expected = (crate::hash::bayer(x, y) * 64.0 - 0.5).round() as u32;
            assert_eq!(eight[(y * 8 + x) as usize], expected, "({x}, {y})");
        }
    }
    for order in [2, 4, 8] {
        let mut sorted = bayer_matrix(order).unwrap();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..order * order).collect::<Vec<_>>());
    }
    assert!(bayer_matrix(3).is_none());
}

#[test]
fn dither_patterns_match_their_matrices() {
    let palette = Palette::hex(&[0x000000, 0xffffff]).unwrap();
    for order in [2u32, 4, 8] {
        let quantise =
            Quantise::new(palette.clone(), Dither::Bayer { order, spread: 1.0 }).unwrap();
        let matrix = bayer_matrix(order).unwrap();
        for level in [0.2f32, 0.35, 0.5, 0.62, 0.8] {
            let lightness = oklab([level; 3])[0];
            let mut whites = 0;
            for y in 0..order * 2 {
                for x in 0..order * 2 {
                    let out = quantise.pixel(grey(level), [x, y]);
                    let t = (matrix[((y % order) * order + x % order) as usize] as f32 + 0.5)
                        / (order * order) as f32;
                    let expected = lightness + t - 0.5 > 0.5;
                    assert_eq!(
                        out[0] > 0.5,
                        expected,
                        "order {order} level {level} ({x}, {y})"
                    );
                    whites += u32::from(out[0] > 0.5);
                }
            }
            let expected_share = matrix
                .iter()
                .filter(|&&m| (m as f32 + 0.5) / (order * order) as f32 > 1.0 - lightness)
                .count() as u32
                * 4;
            assert_eq!(whites, expected_share);
        }
    }
}

#[test]
fn quantising_is_exact_for_in_palette_colours() {
    for palette in [
        neutral_palette(),
        Palette::cube(6).unwrap(),
        Palette::hex(&[0x000000, 0xffffff]).unwrap(),
    ] {
        for dither in [Dither::None] {
            let quantise = Quantise::new(palette.clone(), dither).unwrap();
            for (index, colour) in palette.colours().iter().enumerate() {
                for alpha in [1.0f32, 0.5] {
                    let input = [
                        colour[0] * alpha,
                        colour[1] * alpha,
                        colour[2] * alpha,
                        alpha,
                    ];
                    let out = quantise.pixel(input, [index as u32, 3]);
                    assert_eq!(out, input, "{colour:?} at alpha {alpha}");
                }
            }
        }
    }
    assert_eq!(Palette::cube(6).unwrap().len(), 216);
    assert!(Palette::hex(&[0x123456]).is_err());
    assert!(Palette::new(vec![[0.5; 3]; 257]).is_err());
    assert!(
        Quantise::new(
            neutral_palette(),
            Dither::Bayer {
                order: 3,
                spread: 0.1
            }
        )
        .is_err()
    );
}

#[test]
fn quantising_measures_distance_in_oklab() {
    let palette = Palette::hex(&[0x000000, 0x777777, 0xffffff]).unwrap();
    let mid = Quantise::new(palette, Dither::None).unwrap();
    let out = mid.pixel(grey(0.2), [0, 0]);
    assert!((out[0] - 0x77 as f32 / 255.0).abs() < 1e-6, "{out:?}");
    assert_eq!(mid.pixel([0.0; 4], [0, 0]), [0.0; 4]);
}

#[test]
fn upscales_map_pixels_exactly() {
    let quarter = UpscaleMap::new([480, 270], [3840, 2160], Upscale::Integer).unwrap();
    assert_eq!((quarter.offset, quarter.extent), ([0, 0], [3840, 2160]));
    let deck = UpscaleMap::new([480, 270], [1280, 800], Upscale::Integer).unwrap();
    assert_eq!((deck.offset, deck.extent), ([160, 130], [960, 540]));
    let fit = UpscaleMap::new([480, 270], [1280, 800], Upscale::Fit).unwrap();
    assert_eq!((fit.offset, fit.extent), ([0, 40], [1280, 720]));
    let small = UpscaleMap::new([480, 270], [400, 300], Upscale::Integer).unwrap();
    assert_eq!(small.extent, [400, 225]);
    for y in 0..800 {
        for x in 0..1280 {
            let got = deck.source_pixel([x, y]);
            let inside = (160..1120).contains(&x) && (130..670).contains(&y);
            assert_eq!(got.is_some(), inside);
            if let Some(p) = got {
                assert_eq!(p, [(x - 160) / 2, (y - 130) / 2]);
            }
        }
    }
    for x in 0..1280 {
        let p = fit.source_pixel([x, 400]).unwrap();
        assert_eq!(p[0], x * 480 / 1280);
    }
    let pixels: Vec<[f32; 4]> = (0..4).map(|i| grey(i as f32 / 4.0)).collect();
    let up = UpscaleMap::new([2, 2], [4, 4], Upscale::Integer)
        .unwrap()
        .image(&pixels, [0.0; 4]);
    assert_eq!(up[0], pixels[0]);
    assert_eq!(up[3], pixels[1]);
    assert_eq!(up[15], pixels[3]);
}

#[test]
fn gradients_follow_their_stops() {
    let black = [0.0, 0.0, 0.0, 1.0];
    let white = [1.0, 1.0, 1.0, 1.0];
    let red = [1.0, 0.0, 0.0, 1.0];
    let line = Gradient::linear([0.0, 0.0], [100.0, 0.0], &[(0.0, black), (1.0, white)]).unwrap();
    assert_eq!(line.paint([0.0, 5.0]), black);
    assert_eq!(line.paint([150.0, 5.0]), white);
    assert!((line.paint([50.0, 0.0])[0] - 0.5).abs() < 1e-6);
    let hard = Gradient::linear(
        [0.0, 0.0],
        [0.0, 10.0],
        &[(0.0, black), (0.5, black), (0.5, red), (1.0, red)],
    )
    .unwrap();
    assert_eq!(hard.colour(0.49), black);
    assert_eq!(hard.colour(0.5), red);
    let perceptual = line.space(Interpolation::Oklab);
    let mid = perceptual.colour(0.5);
    assert!(mid[0] > 0.38 && mid[0] < 0.40, "{mid:?}");
    let disc = Gradient::radial([50.0, 50.0], [10.0, 20.0], &[(0.0, white), (1.0, black)]).unwrap();
    assert_eq!(disc.paint([50.0, 50.0]), white);
    assert_eq!(disc.paint([60.0, 50.0]), black);
    assert!((disc.position([50.0, 60.0]) - 0.5).abs() < 1e-6);
    let unordered = Gradient::linear([0.0; 2], [1.0, 0.0], &[(0.6, black), (0.2, white)]).unwrap();
    assert_eq!(
        unordered.stops().map(|s| s.0).collect::<Vec<_>>(),
        [0.6, 0.6]
    );
    assert!(Gradient::linear([0.0; 2], [1.0, 0.0], &[]).is_err());
    assert!(Gradient::linear([0.0; 2], [1.0, 0.0], &[(0.0, black); 9]).is_err());
    assert!(Gradient::radial([0.0; 2], [0.0, 1.0], &[(0.0, black)]).is_err());
}

#[test]
fn grids_draw_lines_on_their_spacing() {
    let grid = Grid::new(20.0, 1.0, [1.0, 1.0, 1.0, 1.0]).major(5, 3.0, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(grid.paint([20.0, 7.0], 2.0), [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(grid.paint([10.0, 10.0], 2.0), [0.0; 4]);
    assert_eq!(grid.paint([100.0, 33.0], 2.0), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(grid.paint([101.0, 33.0], 2.0), [0.0, 0.0, 1.0, 1.0]);
    assert!((grid.paint([20.5, 33.0], 2.0)[3] - 0.5).abs() < 1e-5);
}

#[test]
fn a_vignette_pools_light() {
    let pool = Vignette {
        centre: [960.0, 540.0],
        radius: [400.0, 300.0],
        falloff: 1.0,
        floor: 0.2,
        colour: [0.0, 0.0, 0.0, 1.0],
    };
    assert_eq!(pool.light([960.0, 540.0]), 1.0);
    assert_eq!(pool.light([1350.0, 540.0]), 1.0);
    assert!((pool.light([0.0, 0.0]) - 0.2).abs() < 1e-6);
    let lit = pool.pixel([0.5, 0.5, 0.5, 1.0], [0.0, 0.0]);
    assert!((lit[0] - 0.1).abs() < 1e-6);
}

#[test]
fn wipes_sweep_from_one_side_to_the_other() {
    let layout = [0.0, 0.0, 1920.0, 1080.0];
    let wipe = Blend::Wipe {
        angle: 0.0,
        softness: 40.0,
    };
    for p in [[0.0, 0.0], [960.0, 500.0], [1920.0, 1080.0]] {
        assert_eq!(wipe.weight(0.0, p, layout), 0.0);
        assert_eq!(wipe.weight(1.0, p, layout), 1.0);
    }
    let half = wipe.weight(0.5, [960.0, 0.0], layout);
    assert!((half - 0.5).abs() < 1e-3, "{half}");
    assert!(wipe.weight(0.5, [100.0, 0.0], layout) == 1.0);
    assert!(wipe.weight(0.5, [1800.0, 0.0], layout) == 0.0);
    assert_eq!(Blend::Crossfade.weight(0.25, [5.0, 5.0], layout), 0.25);
}

#[test]
fn curvature_maps_the_centre_to_itself() {
    let curve = Curvature {
        amount: 0.1,
        corner: 0.02,
        bezel: [0.0; 4],
    };
    let centre = curve.source([640.0, 400.0], [1280, 800]).unwrap();
    assert!((centre[0] - 640.0).abs() < 1e-3 && (centre[1] - 400.0).abs() < 1e-3);
    assert!(curve.source([1.0, 1.0], [1280, 800]).is_none());
    let edge = curve.source([1279.0, 400.0], [1280, 800]).unwrap();
    assert!(edge[0] > 1270.0);
}

#[test]
fn oklab_round_trips() {
    for c in [[0.0f32; 3], [1.0; 3], [0.2, 0.5, 0.8], [0.9, 0.1, 0.3]] {
        let back = srgb(oklab(c));
        for i in 0..3 {
            assert!((back[i] - c[i]).abs() < 2e-4, "{c:?} {back:?}");
        }
    }
    assert!((oklab([1.0; 3])[0] - 1.0).abs() < 1e-4);
}

#[test]
fn the_look_shader_validates_and_fits_webgpu_defaults() {
    let source = shader::source();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    let limits = wgpu::Limits::default();
    for palette in [false, true] {
        let entries = gpu::entries(palette);
        assert_eq!(
            pfx_gpu::layout_shortfall(&[entries.as_slice()], &limits),
            None
        );
    }
    assert!(gpu::PALETTE_BYTES <= u64::from(limits.max_uniform_buffer_binding_size));
    assert_eq!(gpu::glow_factor(2.0), 1);
    assert_eq!(gpu::glow_factor(4.0), 2);
    assert_eq!(gpu::glow_factor(12.0), 4);
    assert_eq!(gpu::glow_factor(1000.0), 4);
}

fn device() -> Gpu {
    pollster::block_on(Gpu::headless()).unwrap()
}

fn upload(gpu: &Gpu, size: [u32; 2], pixels: &[[f32; 4]]) -> OffscreenTarget {
    let target = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let halves: Vec<u16> = pixels
        .iter()
        .flatten()
        .map(|v| half::f16::from_f32(*v).to_bits())
        .collect();
    gpu.queue.write_texture(
        target.texture.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 8),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
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

fn test_image(size: [u32; 2]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let u = x as f32 / size[0] as f32;
            let v = y as f32 / size[1] as f32;
            let a = if (x / 7 + y / 5) % 9 == 0 { 0.5 } else { 1.0 };
            out.push([u * a, v * a, (1.0 - u * v) * a, a]);
        }
    }
    out
}

fn run(
    gpu: &Gpu,
    look: &mut LookGpu,
    record: impl FnOnce(&mut LookGpu),
    targets: &[&OffscreenTarget],
) -> Vec<Vec<[f32; 4]>> {
    look.begin();
    record(look);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    look.flush(&gpu.device, &gpu.queue, &mut encoder, None);
    gpu.queue.submit(Some(encoder.finish()));
    targets.iter().map(|t| read(gpu, t)).collect()
}

fn worst(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter()
        .zip(b)
        .flat_map(|(x, y)| (0..4).map(move |i| (x[i] - y[i]).abs()))
        .fold(0.0, f32::max)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_quantises_like_its_twin() {
    let gpu = device();
    let mut look = LookGpu::new(&gpu.device, &gpu.queue);
    let size = [97, 61];
    let mut pixels = f16(&test_image(size));
    let palette = neutral_palette();
    for (i, colour) in palette.colours().iter().enumerate() {
        pixels[i] = [colour[0], colour[1], colour[2], 1.0];
    }
    let pixels = f16(&pixels);
    let source = upload(&gpu, size, &pixels);
    for dither in [
        Dither::None,
        Dither::Bayer {
            order: 2,
            spread: 0.08,
        },
        Dither::Bayer {
            order: 4,
            spread: 0.12,
        },
        Dither::Bayer {
            order: 8,
            spread: 0.2,
        },
    ] {
        let quantise = Quantise::new(palette.clone(), dither).unwrap();
        let target = gpu
            .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let out = run(
            &gpu,
            &mut look,
            |look| {
                look.quantise(
                    Surface {
                        view: &source.view,
                        size,
                    },
                    Surface {
                        view: &target.view,
                        size,
                    },
                    &quantise,
                )
            },
            &[&target],
        )
        .remove(0);
        let twin = f16(&quantise.image(&pixels, size));
        let differing = out
            .iter()
            .zip(&twin)
            .filter(|(a, b)| worst(&[**a], &[**b]) > 1e-3)
            .count();
        assert!(
            differing * 1000 <= out.len(),
            "{dither:?}: {differing} of {} pixels differ",
            out.len()
        );
        if dither == Dither::None {
            let byte = |p: [f32; 4]| p.map(|v| (v * 255.0).round() as u8);
            for (i, colour) in palette.colours().iter().enumerate() {
                let want = byte([colour[0], colour[1], colour[2], 1.0]);
                assert_eq!(byte(out[i]), want, "palette entry {i}");
                assert_eq!(byte(pixels[i]), want, "palette entry {i}");
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_upscale_is_pixel_exact() {
    let gpu = device();
    let mut look = LookGpu::new(&gpu.device, &gpu.queue);
    let low = [120u32, 68u32];
    let pixels = f16(&test_image(low));
    let source = upload(&gpu, low, &pixels);
    for (target_size, mode) in [
        ([960u32, 544u32], Upscale::Integer),
        ([1280, 800], Upscale::Integer),
        ([1280, 800], Upscale::Fit),
        ([1000, 700], Upscale::Fit),
    ] {
        let map = UpscaleMap::new(low, target_size, mode).unwrap();
        let target = gpu
            .offscreen(
                target_size[0],
                target_size[1],
                wgpu::TextureFormat::Rgba16Float,
            )
            .unwrap();
        let clear = [0.125, 0.25, 0.375, 1.0];
        let out = run(
            &gpu,
            &mut look,
            |look| {
                look.upscale(
                    Surface {
                        view: &source.view,
                        size: low,
                    },
                    Surface {
                        view: &target.view,
                        size: target_size,
                    },
                    &map,
                    clear,
                )
            },
            &[&target],
        )
        .remove(0);
        let twin = map.image(&pixels, f16(&[clear])[0]);
        assert!(out == twin, "{target_size:?} {mode:?} is not pixel-exact");
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_backdrop_vignette_crt_and_mix_match_their_twins() {
    let gpu = device();
    let mut look = LookGpu::new(&gpu.device, &gpu.queue);
    let size = [320u32, 200u32];
    let place = Place {
        layout: [480.0, 270.0],
        scale: 320.0 / 480.0,
        offset: [0.0, 10.0],
    };
    let mut pixels = test_image(size);
    for p in pixels.iter_mut().skip(3000).take(9000) {
        *p = [0.0; 4];
    }
    let pixels = f16(&pixels);
    let source = upload(&gpu, size, &pixels);
    let other = upload(&gpu, size, &f16(&vec![[0.9, 0.1, 0.2, 1.0]; 320 * 200]));
    let target = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    fn surface(t: &OffscreenTarget) -> Surface<'_> {
        Surface {
            view: &t.view,
            size: [320, 200],
        }
    }
    let sky = Gradient::linear(
        [0.0, 0.0],
        [0.0, 270.0],
        &[
            (0.0, [0.1, 0.1, 0.3, 1.0]),
            (0.5, [0.8, 0.4, 0.6, 1.0]),
            (1.0, [1.0, 0.9, 0.5, 1.0]),
        ],
    )
    .unwrap()
    .space(Interpolation::Oklab);
    let backdrop = Backdrop {
        colour: Some([0.05, 0.05, 0.05, 1.0]),
        gradient: Some(sky),
        grid: Some(Grid::new(30.0, 1.5, [1.0, 1.0, 1.0, 0.4]).major(4, 3.0, [0.2, 0.9, 1.0, 0.8])),
    };
    let out = run(
        &gpu,
        &mut look,
        |look| look.backdrop(surface(&source), surface(&target), &backdrop, &place),
        &[&target],
    )
    .remove(0);
    let twin = backdrop.under(&pixels, size, &place);
    assert!(
        worst(&out, &twin) < 2.5 / 255.0,
        "backdrop {}",
        worst(&out, &twin) * 255.0
    );
    let pool = Vignette {
        centre: [240.0, 135.0],
        radius: [120.0, 80.0],
        falloff: 0.8,
        floor: 0.15,
        colour: [0.05, 0.0, 0.1, 1.0],
    };
    let out = run(
        &gpu,
        &mut look,
        |look| look.vignette(surface(&source), surface(&target), &pool, &place),
        &[&target],
    )
    .remove(0);
    let twin = pool.image(&pixels, size, &place);
    assert!(
        worst(&out, &twin) < 1.5 / 255.0,
        "vignette {}",
        worst(&out, &twin) * 255.0
    );
    let crt = Crt {
        scanlines: Some(Scanlines {
            period: 3.0,
            strength: 0.4,
        }),
        ..Crt::default()
    };
    let out = run(
        &gpu,
        &mut look,
        |look| {
            look.crt(
                &gpu.device,
                surface(&source),
                surface(&target),
                &crt,
                &place,
            )
        },
        &[&target],
    )
    .remove(0);
    let twin = crt.scanlines_image(&pixels, size, &place);
    assert!(
        worst(&out, &twin) < 1.5 / 255.0,
        "scanlines {}",
        worst(&out, &twin) * 255.0
    );
    for blend in [
        Blend::Crossfade,
        Blend::Wipe {
            angle: 0.6,
            softness: 30.0,
        },
    ] {
        let out = run(
            &gpu,
            &mut look,
            |look| {
                look.mix(
                    surface(&source),
                    surface(&other),
                    surface(&target),
                    &blend,
                    0.4,
                    &place,
                )
            },
            &[&target],
        )
        .remove(0);
        let others = f16(&vec![[0.9, 0.1, 0.2, 1.0]; 320 * 200]);
        let twin: Vec<[f32; 4]> = pixels
            .iter()
            .zip(&others)
            .enumerate()
            .map(|(i, (a, b))| {
                let pixel = [(i % 320) as f32 + 0.5, (i / 320) as f32 + 0.5];
                let w = blend.weight(0.4, place.to_layout(pixel), place.area(size));
                std::array::from_fn(|c| a[c] + (b[c] - a[c]) * w)
            })
            .collect();
        assert!(
            worst(&out, &twin) < 1.5 / 255.0,
            "{blend:?} {}",
            worst(&out, &twin) * 255.0
        );
    }
    let full = Crt {
        scanlines: Some(Scanlines {
            period: 3.0,
            strength: 0.3,
        }),
        glow: Some(Glow {
            threshold: 0.5,
            radius: 6.0,
            strength: 0.8,
            tint: [0.4, 1.0, 0.5, 1.0],
        }),
        curvature: Some(Curvature {
            amount: 0.08,
            corner: 0.03,
            bezel: [0.0, 0.0, 0.0, 1.0],
        }),
        aberration: Some(Aberration { shift: 1.0 }),
    };
    let out = run(
        &gpu,
        &mut look,
        |look| {
            look.crt(
                &gpu.device,
                surface(&source),
                surface(&target),
                &full,
                &place,
            )
        },
        &[&target],
    )
    .remove(0);
    assert!(out.iter().flatten().all(|v| v.is_finite()));
    assert_eq!(out[0], [0.0, 0.0, 0.0, 1.0]);
    let centre = out[(100 * 320 + 160) as usize];
    assert!(centre[3] > 0.0);
}
