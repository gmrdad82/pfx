use super::tests::test_curve;
use super::*;

fn contact_curve() -> ShadowCurve {
    ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![CurvePoint::new(1.0, 6.0, 12.0, 0.2).contact(1.0, 1.5, 0.45)],
    )
    .unwrap()
}

fn single(offset: f32, sigma: f32, alpha: f32) -> ShadowCurve {
    ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![CurvePoint::new(1.0, offset, sigma, alpha)],
    )
    .unwrap()
}

fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    outer[0] - outer[2] <= inner[0] - inner[2] + 1e-4
        && outer[1] - outer[3] <= inner[1] - inner[3] + 1e-4
        && outer[0] + outer[2] >= inner[0] + inner[2] - 1e-4
        && outer[1] + outer[3] >= inner[1] + inner[3] - 1e-4
}

fn composited(both: &Instance, wide: &Instance, near: &Instance, icons: Option<&Icons>) {
    let [x0, y0, x1, y1] = both.pixel_bounds();
    for i in 0..=16 {
        for j in 0..=16 {
            let pixel = [
                x0 + (x1 - x0) * i as f32 / 16.0,
                y0 + (y1 - y0) * j as f32 / 16.0,
            ];
            let p = both.to_local(pixel);
            let ours = cpu::paint(both, icons, p).colour;
            let a = cpu::paint(wide, icons, p).colour;
            let b = cpu::paint(near, icons, p).colour;
            let two: [f32; 4] = std::array::from_fn(|k| b[k] + a[k] * (1.0 - b[3]));
            assert!(
                ours.iter().zip(two).all(|(o, t)| (o - t).abs() < 1e-6),
                "{pixel:?}: {ours:?} vs {two:?}"
            );
        }
    }
    assert!(contains(both.bounds, wide.bounds) && contains(both.bounds, near.bounds));
}

#[test]
fn a_contact_layer_composites_like_a_second_shadow() {
    let fit = Fit::new([400.0, 300.0], [400, 300]).unwrap();
    let light = Light::default();
    let card = Draw::rect([200.0, 150.0], [160.0, 100.0], 10.0)
        .fill(Srgba::WHITE)
        .elevation(1.0);
    let plain = pack_shadow(&card, &fit, &test_curve(), &light).unwrap();
    assert_eq!((plain.stroke, plain.fill_bottom), ([0.0; 4], [0.0; 4]));
    composited(
        &pack_shadow(&card, &fit, &contact_curve(), &light).unwrap(),
        &pack_shadow(&card, &fit, &single(6.0, 12.0, 0.2), &light).unwrap(),
        &pack_shadow(&card, &fit, &single(1.0, 1.5, 0.45), &light).unwrap(),
        None,
    );
    let atlas = Icons::bake(&[icons::pointer()]).unwrap();
    let pointer = Draw::new(Shape::Icon(Icon(0)))
        .at([150.0, 100.0])
        .fill(Srgba::WHITE)
        .elevation(1.0);
    let cast = |curve: &ShadowCurve| {
        pack_shadow_with(&pointer, &fit, curve, &light, Some(&atlas)).unwrap()
    };
    composited(
        &cast(&contact_curve()),
        &cast(&single(6.0, 12.0, 0.2)),
        &cast(&single(1.0, 1.5, 0.45)),
        Some(&atlas),
    );
    let rising = ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![CurvePoint::new(2.0, 6.0, 12.0, 0.2).contact(2.0, 3.0, 0.4)],
    )
    .unwrap();
    let mid = rising.at(1.0).unwrap().contact.unwrap();
    assert!((mid.offset - 1.0).abs() < 1e-6 && (mid.sigma - 1.5).abs() < 1e-6);
    assert!((mid.alpha - 0.2).abs() < 1e-6);
    assert!(
        ShadowCurve::new(
            Srgba::WHITE,
            vec![CurvePoint::new(1.0, 1.0, 1.0, 0.5).contact(0.0, -1.0, 0.5)]
        )
        .is_err()
    );
}

fn blurred(shape: &IconShape, grow: f32, sigma: f32, p: [f32; 2]) -> f32 {
    let step = 0.25f32;
    let reach = 4.0 * sigma;
    let n = (2.0 * reach / step) as i32;
    let (mut total, mut weight) = (0.0f64, 0.0f64);
    for i in 0..=n {
        for j in 0..=n {
            let q = [
                p[0] - reach + i as f32 * step,
                p[1] - reach + j as f32 * step,
            ];
            let r2 = f64::from((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2));
            let w = (-r2 / (2.0 * f64::from(sigma * sigma))).exp();
            weight += w;
            if shape.distance(q) - grow < 0.0 {
                total += w;
            }
        }
    }
    (total / weight) as f32
}

#[test]
fn icon_shadows_match_a_brute_force_blur_of_the_icon() {
    let shapes = [icons::pointer(), icons::tick()];
    let atlas = Icons::bake(&shapes).unwrap();
    let fit = Fit::new([200.0, 200.0], [200, 200]).unwrap();
    let light = Light::default();
    let curve = single(0.0, 5.0, 1.0);
    let mut report = String::new();
    let mut worst = 0.0f32;
    for (index, shape) in shapes.iter().enumerate() {
        let draw = Draw::new(Shape::Icon(Icon(index as u32)))
            .at([100.0, 100.0])
            .stroke(Stroke::solid(Srgba::WHITE, 6.0))
            .fill(Srgba::WHITE)
            .elevation(1.0);
        assert!(pack_shadow(&draw, &fit, &curve, &light).is_none());
        let instance = pack_shadow_with(&draw, &fit, &curve, &light, Some(&atlas)).unwrap();
        assert_eq!(instance.kind(), KIND_ICON_SHADOW);
        let [x0, y0, x1, y1] = atlas.entry(Icon(index as u32)).unwrap().bounds;
        let mut at = (0.0f32, [0.0f32; 2]);
        for i in 0..=12 {
            for j in 0..=12 {
                let p = [
                    x0 - 10.0 + (x1 - x0 + 20.0) * i as f32 / 12.0,
                    y0 - 10.0 + (y1 - y0 + 20.0) * j as f32 / 12.0,
                ];
                let error = (cpu::icon_cast(&instance, Some(&atlas), p, 5.0)
                    - blurred(shape, 3.0, 5.0, p))
                .abs();
                if error > at.0 {
                    at = (error, p);
                }
            }
        }
        worst = worst.max(at.0);
        report += &format!("icon {index}: worst {:.4} at {:?}\n", at.0, at.1);
    }
    eprintln!("{report}");
    assert!(worst < 0.12, "{report}");
    let bare = Draw::new(Shape::Icon(Icon(1)))
        .fill(Srgba::WHITE)
        .elevation(1.0);
    assert!(pack_shadow_with(&bare, &fit, &curve, &light, Some(&atlas)).is_none());
    assert!(pack_shape(&bare, &fit, Some(&atlas)).is_none());
}

fn grain_and_cast_draws() -> Vec<Draw> {
    let card = |centre: [f32; 2], pattern: Pattern| {
        Draw::rect(centre, [120.0, 90.0], 10.0)
            .fill(Srgba::hex(0x3d8fd8))
            .shadow(Shadow::None)
            .pattern(pattern)
    };
    vec![
        card(
            [80.0, 70.0],
            Pattern::grain(3.0, 0.2, Srgba::hex(0x2b3442).alpha(0.4)),
        ),
        card(
            [220.0, 70.0],
            Pattern::grain(1.5, 0.8, Srgba::WHITE.alpha(0.5)),
        ),
        card(
            [360.0, 70.0],
            Pattern::fibre(2.0, 0.6, 0.5, Srgba::WHITE.alpha(0.45)),
        ),
        card(
            [500.0, 70.0],
            Pattern::grain(4.0, 0.5, Srgba::hex(0x2b3442).alpha(0.5)).offset([17.3, -40.6]),
        )
        .transform(place([500.0, 70.0], 0.4)),
        card(
            [80.0, 180.0],
            Pattern::new(PatternKind::Dots, 14.0, 0.0, 0.45, Srgba::hex(0xe2614c))
                .offset([5.0, 3.0]),
        ),
        Draw::rect([240.0, 230.0], [140.0, 80.0], 12.0)
            .fill(Srgba::WHITE)
            .elevation(1.0),
        Draw::new(Shape::Icon(Icon(0)))
            .transform(multiply(translation(360.0, 170.0), scaling(1.4, 1.4)))
            .fill(Srgba::hex(0x3d8fd8))
            .stroke(Stroke::solid(Srgba::WHITE, 4.0))
            .elevation(1.0),
        Draw::new(Shape::Icon(Icon(1)))
            .transform(multiply(place([500.0, 240.0], 0.3), scaling(2.0, 2.0)))
            .stroke(Stroke::solid(Srgba::hex(0x138f8a), 6.0))
            .elevation(1.0)
            .shadow(Shadow::Turned),
        Draw::new(Shape::Icon(Icon(0)))
            .at([580.0, 160.0])
            .fill(Srgba::hex(0xf0b429))
            .shadow(Shadow::Glow(Glow {
                colour: Srgba::hex(0xf0b429).alpha(0.6),
                sigma: 8.0,
            })),
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_draws_grain_offsets_and_cast_icons_like_the_twin() {
    let gpu = super::tests::gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let atlas = Icons::bake(&[icons::pointer(), icons::tick()]).unwrap();
    let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, atlas.clone());
    let draws = grain_and_cast_draws();
    let curve = contact_curve();
    let scene = FlatScene {
        layout: [640.0, 320.0],
        clear: Some(Srgba::hex(0xeceff3)),
        curve: &curve,
        light: Light::default(),
        draws: &draws,
        groups: &[],
        text: &[],
        icons: Some(&flat_icons),
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut report = String::new();
    for scale in [1, 2] {
        let size = [640 * scale, 320 * scale];
        let (pixels, _) = super::tests::gpu_render(&gpu, &mut pass, &scene, size);
        let twin = cpu::render(&scene, Some(&atlas), size).unwrap();
        let (mut total, mut worst, mut over) = (0u64, 0u8, 0usize);
        for (a, b) in pixels.iter().zip(&twin) {
            let mut pixel = 0u8;
            for c in 0..3 {
                let d = byte(a[c]).abs_diff(byte(b[c]));
                total += u64::from(d);
                pixel = pixel.max(d);
            }
            worst = worst.max(pixel);
            over += usize::from(pixel > 2);
        }
        let mean = total as f64 / (pixels.len() * 3) as f64;
        let share = over as f64 / pixels.len() as f64;
        report += &format!(
            "@{scale}x: twin mean {mean:.4}/255, worst {worst}/255, {:.4}% over 2\n",
            share * 100.0
        );
        assert!(mean < 0.15 && share < 1e-3, "{report}");
    }
    eprintln!("{report}");
}
