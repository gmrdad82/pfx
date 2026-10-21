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
fn post_passes_at_rest_keep_every_bit_and_grain_follows_frame_and_seed() {
    let size = [64u32, 40u32];
    let place = Place {
        layout: [96.0, 60.0],
        scale: 64.0 / 96.0,
        offset: [0.0, 0.0],
    };
    let pixels = test_image(size);
    let grain = Grain::new(0.0, 1.5);
    assert!(grain.image(&pixels, size, &place, 3, 7) == pixels);
    assert!(Grade::new().image(&pixels) == pixels);
    for mode in [FadeMode::Mix, FadeMode::Add] {
        let fade = Fade {
            colour: [1.0, 1.0, 1.0, 1.0],
            amount: 0.0,
            mode,
        };
        assert!(fade.image(&pixels) == pixels);
    }
    for letterbox in [
        Letterbox::aspect(2.39, [0.0, 0.0, 0.0, 1.0]),
        Letterbox::height(12.0, [0.0, 0.0, 0.0, 1.0]),
    ] {
        assert!(letterbox.amount(0.0).image(&pixels, size, &place) == pixels);
    }
    let grain = Grain::new(0.1, 1.5);
    let a = grain.image(&pixels, size, &place, 3, 7);
    assert!(a == grain.image(&pixels, size, &place, 3, 7));
    assert!(a != pixels);
    assert!(a != grain.image(&pixels, size, &place, 4, 7));
    assert!(a != grain.image(&pixels, size, &place, 3, 8));
    let bars = Letterbox::height(10.0, [0.0, 0.0, 0.0, 1.0]).image(&pixels, size, &place);
    assert_eq!(bars[0], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        bars[(20 * 64 + 32) as usize],
        pixels[(20 * 64 + 32) as usize]
    );
    let grey = Grade::new().saturation(0.0).pixel([0.8, 0.2, 0.4, 1.0]);
    assert!((grey[0] - grey[1]).abs() < 1e-6 && (grey[1] - grey[2]).abs() < 1e-6);
    assert_eq!(Fade::pulse(0.0, 0.05, 0.3), 0.0);
    assert_eq!(Fade::pulse(0.05, 0.05, 0.3), 1.0);
    assert_eq!(Fade::pulse(0.4, 0.05, 0.3), 0.0);
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
    assert!(gpu::PALETTE_BYTES <= limits.max_uniform_buffer_binding_size);
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

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_grain_grade_letterbox_and_fade_match_their_twins() {
    let gpu = device();
    let mut look = LookGpu::new(&gpu.device, &gpu.queue);
    let size = [320u32, 200u32];
    let place = Place {
        layout: [480.0, 270.0],
        scale: 320.0 / 480.0,
        offset: [0.0, 10.0],
    };
    let pixels = f16(&test_image(size));
    let source = upload(&gpu, size, &pixels);
    let target = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    fn surface(t: &OffscreenTarget) -> Surface<'_> {
        Surface {
            view: &t.view,
            size: [320, 200],
        }
    }
    let grain = Grain::new(0.12, 1.5);
    let mut frames = Vec::new();
    for (frame, seed) in [(3, 7), (3, 7), (4, 7)] {
        let out = run(
            &gpu,
            &mut look,
            |look| {
                look.grain(
                    surface(&source),
                    surface(&target),
                    &grain,
                    &place,
                    frame,
                    seed,
                )
            },
            &[&target],
        )
        .remove(0);
        let twin = grain.image(&pixels, size, &place, frame, seed);
        let differing = out
            .iter()
            .zip(&twin)
            .filter(|(a, b)| worst(&[**a], &[**b]) > 2.0 / 255.0)
            .count();
        assert!(
            differing * 100 <= out.len(),
            "grain: {differing} of {} pixels differ",
            out.len()
        );
        frames.push(out);
    }
    assert!(
        frames[0] == frames[1],
        "grain is the same for a frame and seed"
    );
    assert!(frames[0] != frames[2], "grain moves with the frame");
    let grade = Grade::new()
        .saturation(0.3)
        .contrast(1.2)
        .brightness(-0.03)
        .tint([1.0, 0.85, 0.6, 0.7]);
    let out = run(
        &gpu,
        &mut look,
        |look| look.grade(surface(&source), surface(&target), &grade),
        &[&target],
    )
    .remove(0);
    let twin = grade.image(&pixels);
    assert!(
        worst(&out, &twin) < 1.5 / 255.0,
        "grade {}",
        worst(&out, &twin) * 255.0
    );
    for letterbox in [
        Letterbox::aspect(2.39, [0.02, 0.02, 0.03, 1.0]).amount(0.6),
        Letterbox::aspect(1.0, [0.0, 0.0, 0.0, 0.8]),
        Letterbox::height(31.3, [0.1, 0.0, 0.0, 1.0]).amount(0.5),
    ] {
        let out = run(
            &gpu,
            &mut look,
            |look| look.letterbox(surface(&source), surface(&target), &letterbox, &place),
            &[&target],
        )
        .remove(0);
        let twin = letterbox.image(&pixels, size, &place);
        assert!(
            worst(&out, &twin) < 1.5 / 255.0,
            "{letterbox:?} {}",
            worst(&out, &twin) * 255.0
        );
    }
    for fade in [
        Fade::to([0.0, 0.0, 0.0, 1.0], 0.35),
        Fade::flash([1.0, 0.95, 0.8, 1.0], 0.6),
    ] {
        let out = run(
            &gpu,
            &mut look,
            |look| look.fade(surface(&source), surface(&target), &fade),
            &[&target],
        )
        .remove(0);
        let twin = fade.image(&pixels);
        assert!(
            worst(&out, &twin) < 1.5 / 255.0,
            "{fade:?} {}",
            worst(&out, &twin) * 255.0
        );
    }
}

const COOL: [f32; 4] = [0.7, 0.85, 1.0, 1.0];
const WARM: [f32; 4] = [1.0, 0.85, 0.7, 1.0];

fn close(a: [f32; 4], b: [f32; 4], tolerance: f32) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() <= tolerance)
}

#[test]
fn a_split_tone_is_the_warmth_luma_curve_with_a_tint_per_end() {
    let image = crate::image::Image::from_fn(16, 16, |x, y| {
        let v = (x + 16 * y) as f32 / 255.0;
        [v, v * 0.8 + 0.1, 1.0 - v, 1.0]
    });
    for (amount, low, high, shadow, highlight) in [
        (1.0, [0.8, 1.2], [1.2, 0.8], 0.1, 0.9),
        (0.4, [0.9, 1.1], [1.15, 0.7], 0.25, 0.6),
        (0.75, [1.3, 0.6], [0.7, 1.25], 0.0, 1.0),
    ] {
        let warmth = crate::grade::Warmth::luma_curve(amount, low, high, shadow, highlight);
        let want = crate::grade::warmth(&image, &warmth);
        let split = SplitTone::new(
            [low[0], 1.0, low[1], amount],
            [high[0], 1.0, high[1], amount],
        )
        .range(shadow, highlight);
        let got = Grade::new().split(split).image(&image.pixels);
        assert!(
            worst(&got, &want.pixels) < 1e-5,
            "{amount} {low:?} {high:?}: {}",
            worst(&got, &want.pixels)
        );
    }
}

#[test]
fn a_split_tone_tints_shadows_and_highlights_and_keeps_midtones_at_the_tints_mean() {
    let grade = Grade::new().split(SplitTone::new(COOL, WARM).range(0.2, 0.8));
    assert!(!grade.is_identity());
    let shadow = grade.pixel(grey(0.1));
    assert!(close(shadow, [0.07, 0.085, 0.1, 1.0], 1e-6), "{shadow:?}");
    assert!(shadow[2] > shadow[0], "shadows lean to the shadow tint");
    let highlight = grade.pixel(grey(0.9));
    assert!(
        close(highlight, [0.9, 0.765, 0.63, 1.0], 1e-6),
        "{highlight:?}"
    );
    assert!(
        highlight[0] > highlight[2],
        "highlights lean to the highlight tint"
    );
    let mid = grade.pixel(grey(0.5));
    assert!(close(mid, [0.425, 0.425, 0.425, 1.0], 1e-5), "{mid:?}");
    let bound = 0.5 * 0.15;
    assert!(mid[..3].iter().all(|v| (v - 0.5).abs() <= bound + 1e-6));
    let premultiplied = grade.pixel([0.05, 0.05, 0.05, 0.5]);
    assert!(
        close(premultiplied, [0.035, 0.0425, 0.05, 0.5], 1e-6),
        "{premultiplied:?}"
    );
    assert_eq!(grade.pixel([0.0; 4]), [0.0; 4]);
    let half = Grade::new().split(SplitTone::new(
        [COOL[0], COOL[1], COOL[2], 0.5],
        [WARM[0], WARM[1], WARM[2], 0.5],
    ));
    let low = half.pixel(grey(0.0));
    assert_eq!(low, [0.0, 0.0, 0.0, 1.0]);
    let dark = half.split.scale(0.0);
    assert!((dark[0] - 0.85).abs() < 1e-6 && (dark[2] - 1.0).abs() < 1e-6);
}

#[test]
fn a_split_tone_at_rest_keeps_every_bit() {
    let size = [64u32, 40u32];
    let pixels = test_image(size);
    let rest = Grade::new().split(SplitTone::default());
    assert!(rest.is_identity());
    assert!(rest.image(&pixels) == pixels);
    let silent = Grade::new().split(SplitTone::new(
        [COOL[0], COOL[1], COOL[2], 0.0],
        [WARM[0], WARM[1], WARM[2], 0.0],
    ));
    assert!(silent.is_identity());
    assert!(silent.image(&pixels) == pixels);
    let graded = Grade::new().saturation(0.4).tint([1.0, 0.9, 0.75, 0.5]);
    let with_rest = graded.split(SplitTone::default());
    assert!(graded.image(&pixels) == with_rest.image(&pixels));
}

#[test]
fn scanlines_darken_every_period_th_row_by_their_strength() {
    let size = [8u32, 16u32];
    let place = Place {
        layout: [8.0, 16.0],
        scale: 1.0,
        offset: [0.0, 0.5],
    };
    let pixels = vec![grey(0.8); 8 * 16];
    let crt = Crt {
        scanlines: Some(Scanlines {
            period: 4.0,
            strength: 0.25,
        }),
        ..Crt::default()
    };
    let out = crt.scanlines_image(&pixels, size, &place);
    for row in 0..16usize {
        let v = out[row * 8][0];
        match row % 4 {
            0 => assert!((v - 0.6).abs() < 1e-6, "row {row}: {v}"),
            2 => assert!((v - 0.8).abs() < 1e-6, "row {row}: {v}"),
            _ => assert!(v > 0.6 && v < 0.8, "row {row}: {v}"),
        }
        assert!(out[row * 8..row * 8 + 8].iter().all(|p| *p == out[row * 8]));
    }
}

#[test]
fn a_bloom_threshold_keeps_only_light_above_it() {
    let open = Bloom::new(12.0, 1.0);
    assert_eq!(open.threshold, 0.0);
    for v in [0.0, 0.3, 1.0] {
        assert_eq!(open.bright(grey(v)), grey(v));
    }
    let bloom = open.threshold(0.6);
    assert_eq!(bloom.bright(grey(0.5)), [0.0; 4]);
    assert_eq!(bloom.bright(grey(0.6)), [0.0; 4]);
    let bright = bloom.bright(grey(0.9));
    let k = (luma(grey(0.9)) - 0.6) / luma(grey(0.9));
    assert!(close(bright, grey(0.9).map(|v| v * k), 1e-6), "{bright:?}");
    assert!((bright[0] - 0.3).abs() < 1e-5);
}

#[test]
fn grain_response_scales_grain_by_one_less_its_share_of_luminance() {
    let size = [48u32, 32u32];
    let place = Place {
        layout: [48.0, 32.0],
        scale: 1.0,
        offset: [0.0, 0.0],
    };
    let image = |v: f32| vec![grey(v); (size[0] * size[1]) as usize];
    let (white, mid, ink) = (image(1.0), image(0.5), image(0.0));
    for response in [0.0f32, 0.5, 0.6, 0.75, 1.0] {
        let grain = Grain::new(0.2, 1.5).response(response);
        let on_white = grain.image(&white, size, &place, 2, 9);
        let on_mid = grain.image(&mid, size, &place, 2, 9);
        let on_ink = grain.image(&ink, size, &place, 2, 9);
        let mut moved = 0;
        for i in 0..on_mid.len() {
            let d_mid = on_mid[i][0] - 0.5;
            let d_white = on_white[i][0] - 1.0;
            assert!(d_mid.abs() <= 0.2 * (1.0 - 0.5 * response) + 1e-6);
            assert!(d_white.abs() <= 0.2 * (1.0 - response) + 1e-6);
            let want = d_mid * (1.0 - response) / (1.0 - 0.5 * response);
            assert!((d_white - want).abs() < 1e-5, "{response} at {i}");
            assert!(on_ink[i][0] >= 0.0 && on_ink[i][0] <= 0.2 + 1e-6);
            assert_eq!(on_mid[i][3], 1.0);
            moved += usize::from(d_mid.abs() > 1e-3);
        }
        assert!(
            moved * 2 > on_mid.len(),
            "{response}: grain moves most pixels"
        );
    }
    let off = Grain::new(0.2, 1.5)
        .response(1.0)
        .image(&white, size, &place, 2, 9);
    assert!(
        worst(&off, &white) < 1e-6,
        "a response of 1 keeps white clean"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_split_tone_scanlines_grain_response_and_bloom_threshold_keep_their_rules() {
    let gpu = device();
    let mut look = LookGpu::new(&gpu.device, &gpu.queue);
    let size = [320u32, 200u32];
    let count = (size[0] * size[1]) as usize;
    let place = Place {
        layout: [320.0, 200.0],
        scale: 1.0,
        offset: [0.0, 0.5],
    };
    fn surface(t: &OffscreenTarget) -> Surface<'_> {
        Surface {
            view: &t.view,
            size: [320, 200],
        }
    }
    let target = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let pixels = f16(&test_image(size));
    let source = upload(&gpu, size, &pixels);
    let grade = Grade::new()
        .saturation(0.6)
        .split(SplitTone::new(COOL, WARM).range(0.15, 0.85));
    let mut renders = Vec::new();
    for _ in 0..2 {
        renders.push(
            run(
                &gpu,
                &mut look,
                |look| look.grade(surface(&source), surface(&target), &grade),
                &[&target],
            )
            .remove(0),
        );
    }
    assert!(renders[0] == renders[1], "two split-tone renders are equal");
    let twin = grade.image(&pixels);
    assert!(
        worst(&renders[0], &twin) < 1.5 / 255.0,
        "split tone {}",
        worst(&renders[0], &twin) * 255.0
    );
    let ramp: Vec<[f32; 4]> = f16(&(0..count)
        .map(|i| grey((i as u32 % size[0]) as f32 / (size[0] - 1) as f32))
        .collect::<Vec<_>>());
    let ramp_source = upload(&gpu, size, &ramp);
    let toned = run(
        &gpu,
        &mut look,
        |look| look.grade(surface(&ramp_source), surface(&target), &grade),
        &[&target],
    )
    .remove(0);
    for x in [16usize, 32] {
        let p = toned[x];
        let v = ramp[x][0];
        assert!(
            close(p, [v * 0.7, v * 0.85, v, 1.0], 2e-3),
            "shadow {x}: {p:?}"
        );
    }
    for x in [288usize, 300] {
        let p = toned[x];
        let v = ramp[x][0];
        assert!(
            close(p, [v, v * 0.85, v * 0.7, 1.0], 2e-3),
            "highlight {x}: {p:?}"
        );
    }
    let mid = toned[160];
    let v = ramp[160][0];
    assert!(
        mid[..3].iter().all(|c| (c - v).abs() <= v * 0.15 + 2e-3),
        "mid {mid:?}"
    );
    assert!(
        (mid[0] - mid[2]).abs() < 4e-3,
        "midtones stay neutral: {mid:?}"
    );
    let flat = upload(&gpu, size, &f16(&vec![grey(0.8); count]));
    let crt = Crt {
        scanlines: Some(Scanlines {
            period: 4.0,
            strength: 0.25,
        }),
        ..Crt::default()
    };
    let lined = run(
        &gpu,
        &mut look,
        |look| look.crt(&gpu.device, surface(&flat), surface(&target), &crt, &place),
        &[&target],
    )
    .remove(0);
    for row in 0..size[1] as usize {
        let v = lined[row * size[0] as usize][0];
        match row % 4 {
            0 => assert!((v - 0.6).abs() < 2e-3, "row {row}: {v}"),
            2 => assert!((v - 0.8).abs() < 2e-3, "row {row}: {v}"),
            _ => {}
        }
    }
    let white = upload(&gpu, size, &vec![grey(1.0); count]);
    let mid_grey = upload(&gpu, size, &vec![grey(0.5); count]);
    let clean = Grain::new(0.2, 1.5).response(1.0);
    let out = run(
        &gpu,
        &mut look,
        |look| look.grain(surface(&white), surface(&target), &clean, &place, 2, 9),
        &[&target],
    )
    .remove(0);
    assert!(
        worst(&out, &vec![grey(1.0); count]) < 1e-3,
        "white takes no grain"
    );
    let out = run(
        &gpu,
        &mut look,
        |look| look.grain(surface(&mid_grey), surface(&target), &clean, &place, 2, 9),
        &[&target],
    )
    .remove(0);
    assert!(out.iter().all(|p| (p[0] - 0.5).abs() <= 0.1 + 2e-3));
    assert!(out.iter().filter(|p| (p[0] - 0.5).abs() > 5e-3).count() * 2 > count);
    let mut bright = vec![grey(0.5); count];
    for y in 90..110 {
        for x in 150..170 {
            bright[y * size[0] as usize + x] = grey(1.0);
        }
    }
    let spare = gpu
        .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
        .unwrap();
    let mut bloomed = |layer_pixels: &[[f32; 4]], bloom: &Bloom| {
        let layer = upload(&gpu, size, layer_pixels);
        run(
            &gpu,
            &mut look,
            |look| {
                look.bloom(
                    surface(&layer),
                    surface(&spare),
                    surface(&source),
                    surface(&target),
                    bloom,
                    &place,
                    &place,
                )
            },
            &[&target],
        )
        .remove(0)
    };
    let dim = vec![grey(0.5); count];
    let above = Bloom::new(3.0, 1.0).threshold(0.6);
    assert!(
        bloomed(&dim, &above) == pixels,
        "light under the threshold leaves every pixel untouched"
    );
    assert!(bloomed(&dim, &Bloom::new(3.0, 1.0)) != pixels);
    let accent = bloomed(&bright, &above);
    let centre = 100 * size[0] as usize + 160;
    assert!(accent[centre][0] > pixels[centre][0] + 0.05);
    assert_eq!(
        accent[10 * size[0] as usize + 10],
        pixels[10 * size[0] as usize + 10]
    );
}
