use crate::bloom::Bloom;
use crate::chain::{Buffers, Chain, Pass, Uniforms};
use crate::color::{self, REC709};
use crate::fx::{Bw, Cel, Noir, Outline, TiltShift};
use crate::grade::{Dither, Grain, Saturation, Vignette, Warmth};
use crate::hash::{self, BLUE_NOISE};
use crate::image::Image;
use crate::parse::parse_str;
use crate::preset::{Style, names};
use crate::tone::{self, Tone};
use crate::wgsl;

fn gray(value: f32) -> [f32; 3] {
    [value, value, value]
}

fn luma(c: [f32; 3]) -> f32 {
    c[0] * REC709[0] + c[1] * REC709[1] + c[2] * REC709[2]
}

fn monotone(mut f: impl FnMut(f32) -> f32) {
    let mut prev = f(0.0);
    assert!(prev.is_finite());
    for i in 1..=256 {
        let x = i as f32 / 16.0;
        let y = f(x);
        assert!(y.is_finite(), "{x}");
        assert!(y + 1e-4 >= prev, "{x} {prev} {y}");
        prev = y;
    }
}

#[test]
fn tone_curves_are_monotone_and_meet_their_ends() {
    monotone(|x| tone::aces(gray(x), 1.0)[0]);
    monotone(|x| tone::aces(gray(x), 1.5)[0]);
    monotone(|x| tone::neutral(gray(x), 1.0, 0.04, 0.15, true)[0]);
    monotone(|x| tone::neutral(gray(x), 0.5, 0.04, 0.15, false)[0]);
    monotone(|x| luma(tone::agx(gray(x))));

    assert!(tone::aces(gray(0.0), 1.0)[0].abs() < 1e-6);
    assert!((tone::aces(gray(16.0), 1.0)[0] - 1.0).abs() < 1e-5);
    assert!(tone::aces(gray(0.0), 1.5)[0].abs() < 1e-6);
    assert!((tone::aces(gray(16.0), 1.5)[0] - 1.0).abs() < 1e-5);

    let clamped = tone::neutral(gray(0.0), 1.0, 0.04, 0.15, true);
    assert!(clamped[0].abs() < 1e-6);
    let clamped_hi = tone::neutral(gray(16.0), 1.0, 0.04, 0.15, true);
    assert!(clamped_hi[0] > 0.95 && clamped_hi[0] <= 1.0);

    let open = tone::neutral(gray(0.0), 0.5, 0.04, 0.15, false);
    assert!(open[0].abs() < 1e-6);
    let open_hi = tone::neutral(gray(16.0), 0.5, 0.04, 0.15, false);
    assert!(open_hi[0] > 0.95 && open_hi[0] < 1.0);

    assert!(luma(tone::agx(gray(0.0))).abs() < 1e-4);
    assert!(luma(tone::agx(gray(64.0))) > 0.95);
}

#[test]
fn a_neutral_grade_matches_aces() {
    let image = Image::solid(4, 4, [1.0, 1.0, 1.0, 1.0]);
    let mut chain = Chain::new();
    chain.passes = vec![
        Pass::Exposure(1.0),
        Pass::Bloom(Bloom::neutral(0.0)),
        Pass::Vignette(Vignette::power(0.0, 2.0, 1.0, 1.0)),
        Pass::Saturation(Saturation::rec709(1.0)),
        Pass::Warmth(Warmth::linear(0.0)),
        Pass::Tone(Tone::aces()),
    ];
    let out = chain.apply(&image, None, None);
    let expect = tone::aces([1.0, 1.0, 1.0], 1.0);
    for pixel in &out.pixels {
        for channel in 0..3 {
            assert!((pixel[channel] - expect[channel]).abs() < 1e-5);
        }
        assert!((pixel[3] - 1.0).abs() < 1e-6);
    }
}

#[test]
fn bw_noir_and_sepia_have_the_right_channels() {
    let color = Image::from_fn(8, 8, |x, y| [x as f32 / 7.0, y as f32 / 7.0, 0.25, 1.0]);
    let bw = Style::Bw.chain().apply(&color, None, None);
    for pixel in &bw.pixels {
        assert!((pixel[0] - pixel[1]).abs() < 1e-5);
        assert!((pixel[1] - pixel[2]).abs() < 1e-5);
    }
    let weighted = Pass::Bw(Bw {
        weights: [0.25, 0.5, 0.25],
    })
    .run(&Image::solid(1, 1, [1.0, 0.0, 0.0, 1.0]), None, None, 0, 0);
    let pixel = weighted.at(0, 0);
    assert!((pixel[0] - 0.25).abs() < 1e-6);
    assert!((pixel[0] - pixel[1]).abs() < 1e-6 && (pixel[1] - pixel[2]).abs() < 1e-6);

    let noir = Style::Noir.chain().apply(&color, None, None);
    for pixel in &noir.pixels {
        assert!((pixel[0] - pixel[1]).abs() < 1e-5);
        assert!((pixel[1] - pixel[2]).abs() < 1e-5);
    }
    let keep = Pass::Noir(Noir {
        contrast: 1.0,
        crush: 0.0,
        keep: Some([1.0, 0.0, 0.0]),
        keep_range: 0.45,
    });
    let red = keep
        .run(&Image::solid(1, 1, [1.0, 0.0, 0.0, 1.0]), None, None, 0, 0)
        .at(0, 0);
    let blue = keep
        .run(&Image::solid(1, 1, [0.0, 0.0, 1.0, 1.0]), None, None, 0, 0)
        .at(0, 0);
    assert!(red[0] > red[1]);
    assert!((blue[0] - blue[1]).abs() < 1e-5 && (blue[1] - blue[2]).abs() < 1e-5);

    let sepia = Style::Sepia
        .chain()
        .apply(&Image::solid(2, 2, [0.5, 0.5, 0.5, 1.0]), None, None);
    let pixel = sepia.at(0, 0);
    assert!(pixel[0] >= pixel[1] && pixel[1] >= pixel[2] && pixel[0] > pixel[2]);
}

#[test]
fn cel_bands_count_on_a_gray_ramp() {
    let image = Image::from_fn(64, 1, |x, _| {
        let v = x as f32 / 63.0;
        [v, v, v, 1.0]
    });
    let pass = Pass::Cel(Cel {
        bands: 4,
        shadow: 1.0,
        threshold: 0.0,
        softness: 0.0,
        spec: 0.0,
        spec_roughness: 0.25,
        spec_threshold: 0.72,
        light: [0.0, 0.0, 1.0],
    });
    let out = pass.run(&image, None, None, 0, 0);
    let mut levels = std::collections::BTreeSet::new();
    for pixel in &out.pixels {
        levels.insert(pixel[0].to_bits());
    }
    assert_eq!(levels.len(), 4);
}

#[test]
fn cel_lit_material_has_no_black_regions() {
    let image = Image::solid(96, 48, [0.72, 0.43, 0.18, 1.0]);
    let normal: Vec<[f32; 3]> = (0..48)
        .flat_map(|_| (0..96).map(|x| [x as f32 / 48.0 - 1.0, 0.0, 0.75]))
        .collect();
    let result = Style::Cel.chain().apply(&image, None, Some(&normal));
    assert!(
        result
            .pixels
            .iter()
            .all(|p| p[0].max(p[1]).max(p[2]) > 0.025)
    );
}

#[test]
fn rubber_hose_has_wide_boil_and_repeatable_frames() {
    let image = Image::solid(96, 64, [0.72, 0.72, 0.72, 1.0]);
    let depth: Vec<f32> = (0..64)
        .flat_map(|_| (0..96).map(|x| if x < 48 { 0.2 } else { 0.8 }))
        .collect();
    let cel = Style::Cel.chain().apply(&image, Some(&depth), None);
    let mut hose = Style::RubberHose.chain();
    hose.seed = 31;
    hose.frame = 3;
    let a = hose.apply(&image, Some(&depth), None);
    let same = hose.apply(&image, Some(&depth), None);
    hose.frame = 4;
    let next = hose.apply(&image, Some(&depth), None);
    assert_eq!(a, same);
    assert_ne!(a, next);
    let cel_width = (39..57).filter(|&x| cel.at(x, 24)[0] < 0.12).count();
    let hose_width = (39..57).filter(|&x| a.at(x, 24)[0] < 0.12).count();
    assert!(
        hose_width >= cel_width * 3,
        "{hose_width} < 3 * {cel_width}"
    );
}

#[test]
fn rubber_hose_fills_a_lit_sphere_with_ink() {
    let size = 128;
    let center = (size as f32 - 1.0) * 0.5;
    let radius = 49.0;
    let mut pixels = Vec::with_capacity(size * size);
    let mut normals = Vec::with_capacity(size * size);
    let mut depth = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let nx = (x as f32 - center) / radius;
            let ny = (center - y as f32) / radius;
            let rr = nx * nx + ny * ny;
            if rr < 1.0 {
                pixels.push([0.76, 0.76, 0.76, 1.0]);
                normals.push([nx, ny, (1.0 - rr).sqrt()]);
                depth.push(0.4);
            } else {
                pixels.push([0.93, 0.93, 0.93, 1.0]);
                normals.push([0.0, 0.0, 0.0]);
                depth.push(1.0);
            }
        }
    }
    let source = Image::from_pixels(size as u32, size as u32, pixels).unwrap();
    let result = Style::RubberHose
        .chain()
        .apply(&source, Some(&depth), Some(&normals));
    let mut covered = 0;
    let mut ink = 0;
    for (i, normal) in normals.iter().enumerate() {
        if normal[2] > 0.0 {
            covered += 1;
            ink += usize::from(result.pixels[i][0] < 0.1);
        }
    }
    let fraction = ink as f32 / covered as f32;
    assert!((0.15..=0.40).contains(&fraction), "{fraction}");
}

#[test]
fn comic_midtone_has_screen_frequency() {
    let image = Image::solid(128, 128, [0.48, 0.48, 0.48, 1.0]);
    let out = Style::Comic.chain().apply(&image, None, None);
    let mean = out.pixels.iter().filter(|p| p[0] < 0.7).count() as f32 / out.pixels.len() as f32;
    let correlation = |dx: u32, dy: u32| {
        let mut score = 0.0;
        for y in 8..120 {
            for x in 8..120 {
                let a = if out.at(x, y)[0] < 0.7 { 1.0 } else { 0.0 };
                let b = if out.at(x + dx, y + dy)[0] < 0.7 {
                    1.0
                } else {
                    0.0
                };
                score += (a - mean) * (b - mean);
            }
        }
        score
    };
    assert!(correlation(7, 2) > correlation(3, 0) * 1.5);
}

#[test]
fn outline_darkens_a_depth_step() {
    let image = Image::solid(32, 8, [0.7, 0.7, 0.7, 1.0]);
    let mut depth = vec![0.2; 32 * 8];
    for y in 0..8 {
        for x in 16..32 {
            depth[y * 32 + x] = 0.8;
        }
    }
    let pass = Pass::Outline(Outline {
        enabled: true,
        thickness: 1.0,
        color: [0.0, 0.0, 0.0],
        local_color: false,
        alpha: 1.0,
        crease_angle: 35.0,
        depth_gap: 0.05,
    });
    let out = pass.run(&image, Some(&depth), None, 0, 0);
    let edge = out.at(15, 2);
    let flat = out.at(2, 2);
    assert!(flat[0] - edge[0] > 0.2);
    assert!(edge[0] < 0.05);
    assert!(flat[0] > 0.6);
}

#[test]
fn dither_stays_inside_half_a_step() {
    let image = Image::solid(16, 16, [0.4, 0.4, 0.4, 1.0]);
    let out = Pass::Dither(Dither { amplitude: 1.0 }).run(&image, None, None, 2, 3);
    let again = Pass::Dither(Dither { amplitude: 1.0 }).run(&image, None, None, 2, 3);
    assert_eq!(out.pixels, again.pixels);
    let mut differed = false;
    for pixel in &out.pixels {
        for channel in pixel.iter().take(3) {
            let delta = (channel - 0.4).abs();
            assert!(delta <= 0.5 / 255.0 + 1e-6);
            if delta > 1e-8 {
                differed = true;
            }
        }
    }
    assert!(differed);
}

#[test]
fn grain_follows_the_seed_and_the_frame() {
    let image = Image::solid(16, 16, [0.4, 0.2, 0.5, 1.0]);
    let pass = Pass::Grain(Grain {
        strength: 0.1,
        response: 0.6,
        clamp: false,
    });
    let same_a = pass.run(&image, None, None, 4, 9);
    let same_b = pass.run(&image, None, None, 4, 9);
    let other_frame = pass.run(&image, None, None, 5, 9);
    let other_seed = pass.run(&image, None, None, 4, 10);
    assert_eq!(same_a.pixels, same_b.pixels);
    assert_ne!(same_a.pixels, other_frame.pixels);
    assert_ne!(same_a.pixels, other_seed.pixels);
}

#[test]
fn every_style_is_finite_on_a_pattern() {
    let image = Image::from_fn(64, 64, |x, y| {
        let u = x as f32 / 63.0;
        let v = y as f32 / 63.0;
        if x == 3 && y == 3 {
            [4.0, 3.0, 1.0, 1.0]
        } else if (x / 8 + y / 8) % 2 == 0 {
            [u, v * 0.5, 0.2, 1.0]
        } else {
            [0.15, u, v, 1.0]
        }
    });
    let depth: Vec<f32> = (0..64 * 64).map(|i| i as f32 / (64.0 * 64.0)).collect();
    let normal: Vec<[f32; 3]> = (0..64 * 64)
        .map(|i| {
            let x = (i % 64) as f32 / 63.0 * 2.0 - 1.0;
            let y = (i / 64) as f32 / 63.0 * 2.0 - 1.0;
            let len = (x * x + y * y + 1.0).sqrt();
            [x / len, y / len, 1.0 / len]
        })
        .collect();
    for style in Style::ALL {
        let mut chain = style.chain();
        chain.seed = 11;
        chain.frame = 6;
        let out = chain.apply(&image, Some(&depth), Some(&normal));
        assert_eq!(out.width, 64);
        assert_eq!(out.height, 64);
        for pixel in &out.pixels {
            for channel in pixel {
                assert!(channel.is_finite(), "{}", style.name());
            }
        }
        if style == Style::OneBit {
            for pixel in &out.pixels {
                for channel in pixel.iter().take(3) {
                    assert!(*channel == 0.0 || *channel == 1.0);
                }
            }
        }
    }
    let mid = Image::solid(64, 64, [0.5, 0.5, 0.5, 1.0]);
    let bits = Style::OneBit.chain().apply(&mid, None, None);
    let mut saw_zero = false;
    let mut saw_one = false;
    for pixel in &bits.pixels {
        if pixel[0] == 0.0 {
            saw_zero = true;
        }
        if pixel[0] == 1.0 {
            saw_one = true;
        }
    }
    assert!(saw_zero && saw_one);
}

#[test]
fn tilt_shift_uses_depth_and_leaves_the_focus_plane() {
    let image = Image::from_fn(32, 8, |x, y| {
        if (x + y) % 2 == 0 {
            [1.0, 1.0, 1.0, 1.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    });
    let mut chain = Chain::new();
    chain.passes.push(Pass::TiltShift(TiltShift {
        focus: 0.5,
        range: 0.22,
        radius: 5.0,
    }));
    assert_eq!(chain.apply(&image, None, None).pixels, image.pixels);
    assert_eq!(chain.apply(&image, Some(&[1.0]), None).pixels, image.pixels);
    let mut depth = vec![0.5; 32 * 8];
    for y in 0..8 {
        for x in 16..32 {
            depth[y * 32 + x] = 1.0;
        }
    }
    let out = chain.apply(&image, Some(&depth), None);
    assert_eq!(out.at(2, 2), image.at(2, 2));
    let before = image.at(20, 2)[0];
    let after = out.at(20, 2)[0];
    assert!((after - 0.5).abs() < (before - 0.5).abs());
}

#[test]
fn toml_parses_styles_and_refuses_unknown_keys() {
    let noir = parse_str("style = \"noir\"\n").expect("noir");
    assert_eq!(noir.passes[0].name(), "noir");

    let exposure = parse_str("style = \"cel\"\nexposure = 1.25\n").expect("exposure");
    assert!(matches!(&exposure.passes[0], Pass::Exposure(v) if (*v - 1.25).abs() < 1e-6));
    let whole = parse_str("style = \"cel\"\nexposure = 2\n").expect("integer");
    assert!(matches!(&whole.passes[0], Pass::Exposure(v) if *v == 2.0));

    let unknown = parse_str("nope = 1\n").unwrap_err().to_string();
    assert!(unknown.contains("nope"), "{unknown}");
    assert!(unknown.contains("allowed:"), "{unknown}");
    assert!(unknown.contains("exposure"), "{unknown}");

    let bad_style = parse_str("style = \"nope\"\n").unwrap_err().to_string();
    assert!(bad_style.contains("nope"), "{bad_style}");
    assert!(bad_style.contains("noir"), "{bad_style}");
    assert!(bad_style.contains("cel"), "{bad_style}");

    for style in Style::ALL {
        let name = style.name();
        let chain =
            parse_str(&format!("style = \"{name}\"\n")).unwrap_or_else(|_| panic!("{name}"));
        assert!(!chain.passes.is_empty(), "{name}");
    }

    let nested = parse_str("bloom = { foo = 1 }\n").unwrap_err().to_string();
    assert!(nested.contains("bloom.foo"), "{nested}");
    assert!(nested.contains("allowed:"), "{nested}");
    assert!(nested.contains("strength"), "{nested}");

    let bad_number = parse_str("cel_shadow = \"soft\"\n")
        .unwrap_err()
        .to_string();
    assert!(bad_number.contains("cel_shadow"), "{bad_number}");
    assert!(bad_number.contains("number"), "{bad_number}");

    assert!(parse_str("").expect("empty").passes.is_empty());
    let grain = parse_str("grain = 0.02\n").expect("grain");
    assert!(matches!(
        grain.passes.last(),
        Some(Pass::Grain(Grain {
            strength,
            response: 0.0,
            clamp: false,
        })) if (*strength - 0.02).abs() < 1e-6
    ));
    let kinds = parse_str(
        "bloom = { kind = \"tent\", strength = 0.2 }\nvignette = { kind = \"power\", strength = 0.3 }\nwarmth = 0.25\ntone = { kind = \"neutral\", start = 1.0, clamp = false }\npaper_grain = 0.1\n",
    )
    .expect("kinds");
    assert!(matches!(
        kinds.passes.iter().find(|pass| matches!(pass, Pass::Bloom(_))),
        Some(Pass::Bloom(bloom)) if bloom.kind == crate::bloom::BloomKind::Tent
            && (bloom.strength - 0.2).abs() < 1e-6
            && bloom.radius == 4
            && bloom.spread == 16.0
    ));
    assert!(matches!(
        kinds.passes.iter().find(|pass| matches!(pass, Pass::Warmth(_))),
        Some(Pass::Warmth(warmth)) if warmth.kind == crate::grade::WarmthKind::Linear
            && (warmth.amount - 0.25).abs() < 1e-6
    ));
    assert!(matches!(
        kinds.passes.iter().find(|pass| matches!(pass, Pass::Tone(_))),
        Some(Pass::Tone(Tone::Neutral { start, clamp: false, .. })) if (*start - 1.0).abs() < 1e-6
    ));
    let bare = parse_str("tone = 1\n").unwrap_err().to_string();
    assert!(bare.contains("table"), "{bare}");
    assert!(parse_str("paper = 0.1\n").is_err());
    let encoded = parse_str("style = \"noir\"\nencode = true\n").expect("encode");
    assert!(
        encoded
            .passes
            .iter()
            .any(|pass| matches!(pass, Pass::Encode))
    );
}

#[test]
fn styles_dispatch_compute_shaders() {
    for style in Style::ALL {
        for pass in style.chain().passes {
            let dispatches = pass.dispatches(Buffers::GEOMETRY);
            assert!(!dispatches.is_empty(), "{}", style.name());
            for dispatch in dispatches {
                assert!(dispatch.shader.contains("@compute"));
                assert!(dispatch.shader.contains("fn main"));
                assert!(dispatch.shader.contains("struct Post"));
            }
        }
    }
    assert!(wgsl::GRAIN.contains("0x8da6b343"));
    assert!(wgsl::TONE.contains("2.51"));
    assert!(wgsl::BLOOM_RINGS.contains("0.37"));
    assert!(wgsl::CEL.contains("normal_tex"));
    assert!(wgsl::OUTLINE.contains("depth_buf"));
    assert!(wgsl::LUT.contains("lut_buf"));
    assert!(wgsl::ONE_BIT_BLUE.contains("blue_buf"));
    assert!(wgsl::BLOOM_ADD.contains("bloom_tex"));
    let distortion = Pass::Distortion(0.1).dispatches(Buffers::NONE);
    let curvature = Pass::Curvature(0.1).dispatches(Buffers::NONE);
    assert_eq!(distortion[0].shader, curvature[0].shader);
}

#[test]
fn stylised_shaders_validate_without_a_gpu() {
    for source in [
        wgsl::CEL,
        wgsl::OUTLINE,
        wgsl::RUBBER_HOSE,
        wgsl::COMIC,
        wgsl::MODERN_COMIC,
    ] {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn uniforms_are_a_hundred_and_twelve_bytes() {
    let bytes = Uniforms::new(3, 4, 5, 6, [0.0; 16]).bytes();
    assert_eq!(bytes.len(), 112);
    let word = |at: usize| u32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!([word(96), word(100), word(104), word(108)], [3, 4, 3, 4]);
    let bytes = Uniforms::new(3, 4, 5, 6, [0.0; 16])
        .extent((7, 8), (9, 10))
        .bytes();
    let word = |at: usize| u32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!([word(96), word(100), word(104), word(108)], [7, 8, 9, 10]);
    assert_eq!(u32::from_ne_bytes(bytes[0..4].try_into().unwrap()), 3);
    assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 4);
    assert_eq!(u32::from_ne_bytes(bytes[8..12].try_into().unwrap()), 5);
    assert_eq!(u32::from_ne_bytes(bytes[12..16].try_into().unwrap()), 6);
}

#[test]
fn split_chain_matches_whole_chain_without_overlay() {
    let source = Image::from_fn(23, 17, |x, y| {
        [x as f32 / 11.0, y as f32 / 8.0, (x + y) as f32 / 20.0, 1.0]
    });
    let mut box_chain = Chain::new();
    box_chain.passes = vec![
        Pass::Exposure(1.0),
        Pass::Bloom(Bloom::neutral(0.2)),
        Pass::Tone(Tone::aces()),
    ];
    let mut tent = Chain::new();
    tent.passes = vec![
        Pass::Bloom(Bloom::tent(0.2, 1.0, 4, 16.0, 2)),
        Pass::Black(0.02),
        Pass::Tone(Tone::neutral(1.0, false)),
    ];
    let mut rings = Chain::new();
    rings.passes = vec![
        Pass::Bloom(Bloom::rings(
            0.2,
            1.0,
            4,
            crate::bloom::NEUTRAL_RADII,
            crate::bloom::NEUTRAL_GAINS,
        )),
        Pass::Exposure(1.0),
        Pass::Tone(Tone::aces()),
        Pass::Saturation(Saturation::rec709(1.1)),
    ];
    for (name, mut chain) in [
        ("box", box_chain),
        ("tent", tent),
        ("rings", rings),
        ("neon", Style::Neon.chain()),
    ] {
        if let Some(Pass::Bloom(bloom)) = chain
            .passes
            .iter_mut()
            .find(|p| matches!(p, Pass::Bloom(_)))
        {
            bloom.strength = 0.4;
        }
        let whole = chain.apply(&source, None, None);
        let (first, final_stage) = chain.split_after_bloom();
        let staged = final_stage.apply(&first.apply(&source, None, None), None, None);
        assert_eq!(whole, staged, "{name}");
    }
}

#[test]
fn style_names_follow_the_enum() {
    assert_eq!(Style::ALL.len(), names().len());
    for (style, name) in Style::ALL.iter().zip(names()) {
        assert_eq!(style.name(), *name);
        assert_eq!(Style::parse(name), Some(*style));
    }
}

#[test]
fn modern_comic_steps_black_and_antialiasing() {
    let gradient = Image::from_fn(512, 5, |x, _| {
        let value = x as f32 / 511.0;
        [value, value, value, 1.0]
    });
    let pass = crate::comic::ModernComic::default();
    let out = crate::comic::apply(&gradient, &pass, None, None);
    let mut levels = std::collections::BTreeSet::new();
    for x in 10..502 {
        let value = gradient.at(x, 2)[0];
        let boundary = (value * 4.0).fract();
        if boundary < 0.9 && (value - pass.black_threshold).abs() > 0.01 {
            levels.insert((out.at(x, 2)[0] * 255.0).round() as u32);
        }
    }
    assert!(levels.len() <= pass.tone_steps as usize + 1, "{levels:?}");
    assert_eq!(out.at(10, 2)[0], 0.0);
    let edge = Image::from_fn(5, 1, |x, _| {
        let value = [0.20, 0.23, 0.249, 0.26, 0.28][x as usize];
        [value, value, value, 1.0]
    });
    let edge_out = crate::comic::apply(&edge, &pass, None, None);
    assert!(edge_out.at(2, 0)[0] > 0.125 && edge_out.at(2, 0)[0] < 0.375);
}

#[test]
fn modern_comic_silhouette_is_heavier_than_crease() {
    let image = Image::solid(32, 12, [0.55, 0.55, 0.55, 1.0]);
    let depth: Vec<f32> = (0..12)
        .flat_map(|_| (0..32).map(|x| if x < 10 { 0.2 } else { 0.8 }))
        .collect();
    let normal: Vec<[f32; 3]> = (0..12)
        .flat_map(|_| {
            (0..32).map(|x| {
                if x < 22 {
                    [0.0, 0.0, 1.0]
                } else {
                    [0.0, 0.8, 0.6]
                }
            })
        })
        .collect();
    let out = crate::comic::apply(
        &image,
        &crate::comic::ModernComic::default(),
        Some(&depth),
        Some(&normal),
    );
    let dark = |start, end| (start..end).filter(|&x| out.at(x, 6)[0] < 0.2).count();
    assert!(dark(6, 14) >= 2 * dark(18, 26));
}

#[test]
fn modern_comic_image_only_keeps_feature_ink() {
    let image = Image::from_fn(21, 9, |x, _| {
        let value = if x == 10 { 0.11 } else { 0.62 };
        [value, value, value, 1.0]
    });
    let pass = crate::comic::ModernComic::default();
    let mut without_lines = pass;
    without_lines.interior_line_strength = 0.0;
    let plain = crate::comic::apply(&image, &without_lines, None, None);
    let inked = crate::comic::apply(&image, &pass, None, None);
    assert!(inked.at(10, 4)[0] < plain.at(10, 4)[0] * 0.5);
}

#[test]
fn modern_comic_name_and_parameters_round_trip() {
    assert_eq!(Style::parse("modern_comic"), Some(Style::ModernComic));
    let parsed = parse_str(
        "style = \"modern_comic\"\ntone_steps = 5\nblack_threshold = 0.22\nline_weight = 0.9\ninterior_line_strength = 0.6\nsaturation_lift = 1.2\nhighlight_threshold = 0.8\n",
    )
    .unwrap();
    assert!(
        matches!(parsed.passes.as_slice(), [Pass::ModernComic(p)] if p.tone_steps == 5
        && (p.black_threshold - 0.22).abs() < 1e-6
        && (p.line_weight - 0.9).abs() < 1e-6
        && (p.interior_line_strength - 0.6).abs() < 1e-6
        && (p.saturation_lift - 1.2).abs() < 1e-6
        && (p.highlight_threshold - 0.8).abs() < 1e-6)
    );
}

#[test]
#[ignore = "visual check; run manually"]
fn modern_comic_look_comparison() {
    let width = 1920;
    let height = 1080;
    let mut pixels = Vec::with_capacity((width * height) as usize);
    let mut depth = Vec::with_capacity((width * height) as usize);
    let mut normal = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let xf = x as f32;
            let yf = y as f32;
            let torso_x = (xf - 960.0).abs();
            let torso_y = (yf - 750.0).abs();
            let torso = torso_x < 255.0 && torso_y < 255.0;
            let neck = (xf - 960.0).abs() < 73.0 && (yf - 511.0).abs() < 75.0;
            let hx = (xf - 960.0) / 153.0;
            let hy = (yf - 344.0) / 188.0;
            let head = hx * hx + hy * hy < 1.0;
            let hair = head && (yf < 258.0 + (xf - 960.0).abs() * 0.17 || xf < 850.0);
            let beard = head && yf > 433.0 && (xf - 960.0).abs() < 110.0;
            let eye = head
                && ((xf - 906.0).abs() < 18.0 || (xf - 1014.0).abs() < 18.0)
                && (yf - 343.0).abs() < 5.0;
            let highlight =
                head && ((xf - 888.0) / 27.0).powi(2) + ((yf - 294.0) / 11.0).powi(2) < 1.0;
            let jacket_lapel =
                torso && yf < 805.0 && ((xf - 960.0).abs() - (yf - 550.0) * 0.27).abs() < 37.0;
            let arm = yf > 565.0
                && yf < 955.0
                && ((xf - 660.0).abs() < 58.0 || (xf - 1260.0).abs() < 58.0);
            let light = (0.82 + (960.0 - xf) / 520.0).clamp(0.24, 1.4);
            let (base, z, n) = if highlight {
                ([1.2, 0.99, 0.82], 0.18, [hx * 0.6, hy * 0.3, 1.0])
            } else if eye || hair || beard {
                ([0.025, 0.02, 0.025], 0.19, [hx * 0.4, hy * 0.3, 1.0])
            } else if head || neck {
                ([1.0, 0.59, 0.40], 0.2, [hx * 0.6, hy * 0.3, 1.0])
            } else if jacket_lapel {
                ([0.21, 0.25, 0.37], 0.32, [0.8, 0.0, 0.6])
            } else if torso || arm {
                ([0.13, 0.16, 0.28], 0.36, [0.0, 0.0, 1.0])
            } else {
                ([0.55, 0.36, 0.26], 0.96, [0.0, 0.0, 1.0])
            };
            let brightness = if highlight {
                1.0
            } else if eye || hair || beard {
                0.6 + light * 0.3
            } else if xf > 1040.0 && (head || neck || torso || arm || jacket_lapel) {
                light * 0.3
            } else {
                light
            };
            pixels.push([
                base[0] * brightness,
                base[1] * brightness,
                base[2] * brightness,
                1.0,
            ]);
            depth.push(z);
            normal.push(n);
        }
    }
    let scene = Image::from_pixels(width, height, pixels).unwrap();
    std::fs::create_dir_all("tmp/modern-comic").unwrap();
    for style in [
        Style::ModernComic,
        Style::Cel,
        Style::Comic,
        Style::RubberHose,
    ] {
        let image = style.chain().apply(&scene, Some(&depth), Some(&normal));
        let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
        for pixel in image.pixels {
            for value in pixel[..3].iter() {
                bytes.push((color::encode_channel((*value).clamp(0.0, 1.0)) * 255.0).round() as u8);
            }
        }
        std::fs::write(format!("tmp/modern-comic/{}.ppm", style.name()), bytes).unwrap();
    }
}

#[test]
fn bayer_bits_match_the_table_and_blue_noise_is_present() {
    for y in 0..8 {
        for x in 0..8 {
            assert_eq!(bayer_bits(x, y), hash::bayer(x, y));
        }
    }
    assert_eq!(BLUE_NOISE.len(), 4096);
    assert_eq!(*BLUE_NOISE.iter().min().unwrap(), 0);
    assert_eq!(*BLUE_NOISE.iter().max().unwrap(), 255);
}

fn bayer_bits(x: u32, y: u32) -> f32 {
    let mut v = 0u32;
    let mut xx = x & 7;
    let mut yy = y & 7;
    let mut half = 4u32;
    let mut mul = 1u32;
    for _ in 0..3 {
        let cx = xx >= half;
        let cy = yy >= half;
        if xx >= half {
            xx -= half;
        }
        if yy >= half {
            yy -= half;
        }
        let add = if cx && !cy {
            2
        } else if !cx && cy {
            3
        } else if cx && cy {
            1
        } else {
            0
        };
        v += add * mul;
        mul *= 4;
        half /= 2;
    }
    (v as f32 + 0.5) / 64.0
}

#[test]
fn empty_chain_and_empty_image_stay_put() {
    let image = Image::solid(2, 2, [0.2, 0.3, 0.4, 1.0]);
    assert_eq!(Chain::new().apply(&image, None, None), image);
    let empty = Image::new(0, 0);
    for style in Style::ALL {
        let out = style.chain().apply(&empty, None, None);
        assert!(out.pixels.is_empty(), "{}", style.name());
    }
}

#[test]
fn linear_taps_pair_the_point_gaussian() {
    for (radius, spread) in [(0, 4.0), (1, 2.0), (4, 16.0), (7, 9.0), (12, 30.0)] {
        let (center, taps) = crate::bloom::linear_taps(radius, spread);
        let point: Vec<f64> = (0..=radius)
            .map(|k| (-f64::from(k * k) / f64::from(spread)).exp())
            .collect();
        let total = point[0] + 2.0 * point[1..].iter().sum::<f64>();
        let sum = center + 2.0 * taps.iter().map(|tap| tap.0).sum::<f64>();
        assert!((sum - 1.0).abs() < 1e-12, "{radius}: {sum}");
        assert!((center - point[0] / total).abs() < 1e-12);
        assert_eq!(taps.len(), (radius as usize).div_ceil(2));
        for (index, (weight, offset)) in taps.iter().enumerate() {
            let k = 1 + 2 * index;
            let right = point.get(k + 1).copied().unwrap_or(0.0);
            assert!((weight - (point[k] + right) / total).abs() < 1e-12);
            assert!(*offset >= k as f64 && *offset <= (k + 1) as f64);
        }
    }
}

#[test]
fn a_box_bloom_blurs_with_linear_taps_unless_told_not_to() {
    let blurs = |bloom: Bloom| {
        let mut chain = Chain::new();
        chain.passes.push(Pass::Bloom(bloom));
        chain.passes[0].dispatches(Buffers::GEOMETRY)[1].clone()
    };
    for radius in [0, 3, 4, 9] {
        let bloom = Bloom::pyramid(0.3, 1.0, 0.0, 0.0, 0.0, radius, 12.0, 2);
        let linear = blurs(bloom);
        assert_eq!(linear.aux, crate::chain::Aux::LinearSampler);
        assert_eq!(
            linear.shader.as_ref(),
            crate::wgsl::bloom_blur_linear(radius, 12.0)
        );
        naga::front::wgsl::parse_str(&linear.shader).unwrap();
        let point = blurs(Bloom {
            linear_taps: false,
            ..bloom
        });
        assert_eq!(point.aux, crate::chain::Aux::None);
        assert_eq!(point.shader.as_ref(), crate::wgsl::BLOOM_BLUR);
    }
    let tent = blurs(Bloom::tent(0.3, 1.0, 4, 16.0, 2));
    assert_eq!(tent.shader.as_ref(), crate::wgsl::BLOOM_BLUR);
    let parsed = parse_str("bloom = { kind = \"box\", strength = 0.3, linear_taps = false }\n")
        .expect("linear_taps");
    assert!(matches!(
        parsed.passes.as_slice(),
        [Pass::Bloom(bloom)] if !bloom.linear_taps && (bloom.strength - 0.3).abs() < 1e-6
    ));
}

#[test]
fn a_luma_curve_warmth_moves_red_and_blue_only() {
    let chain = parse_str(
        "warmth = { kind = \"luma_curve\", amount = 0.5, low = [1.0, 1.0], high = [1.04, 0.96], shadow = 0.2, highlight = 0.8 }\n",
    )
    .expect("luma curve");
    let [Pass::Warmth(warmth)] = chain.passes.as_slice() else {
        panic!("{:?}", chain.passes);
    };
    assert_eq!(warmth.kind, crate::grade::WarmthKind::LumaCurve);
    assert_eq!((warmth.low, warmth.high), ([1.0, 1.0], [1.04, 0.96]));
    assert_eq!((warmth.shadow, warmth.highlight), (0.2, 0.8));
    let image = Image::from_fn(4, 1, |x, _| {
        let v = x as f32 / 3.0;
        [v, v, v, 1.0]
    });
    let out = chain.apply(&image, None, None);
    for (before, after) in image.pixels.iter().zip(&out.pixels) {
        assert_eq!(before[1], after[1]);
        assert!(after[0] >= before[0] && after[2] <= before[2]);
    }
    assert_eq!(out.pixels[0], image.pixels[0]);
    assert!(out.pixels[3][0] > out.pixels[3][1]);
    assert!(parse_str("warmth = { kind = \"luma_curve\", low = [1.0, 1.0, 1.0] }\n").is_err());
}

#[test]
fn passes_know_whether_and_how_far_a_rect_can_be_posted_alone() {
    use crate::fx::{Pixelate, Watercolor};
    let reach = |pass: Pass| pass.region_reach();
    assert_eq!(reach(Pass::Exposure(1.2)), Some(0));
    assert_eq!(reach(Pass::Encode), Some(0));
    assert_eq!(reach(Pass::Kuwahara(3)), Some(3));
    assert_eq!(reach(Pass::Kuwahara(40)), Some(8));
    assert_eq!(
        reach(Pass::Watercolor(Watercolor {
            darkening: 0.5,
            grain: 0.2,
            scale: 6.0
        })),
        Some(1)
    );
    assert_eq!(reach(Pass::Weave(3.0)), Some(7));
    assert_eq!(reach(Pass::Aberration(2.5)), Some(4));
    assert_eq!(reach(Pass::Bloom(crate::bloom::Bloom::neutral(0.3))), None);
    assert_eq!(reach(Pass::Distortion(0.1)), None);
    assert_eq!(reach(Pass::Tape(crate::tape::Tape::OFF)), None);
    assert_eq!(
        reach(Pass::Pixel(Pixelate {
            size: 4,
            levels: 0,
            palette: false,
        })),
        None
    );
    assert_eq!(
        reach(Pass::Grade(vec![Pass::Exposure(1.1), Pass::Encode])),
        Some(0)
    );
}
