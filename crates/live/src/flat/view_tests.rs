use super::cpu::{self, render_viewed};
use super::cpu_text::CpuText;
use super::tests::{gpu, gpu_render};
use super::*;
use pfx_text::{Baseline, Representation, Style as TextStyle, TextEngine};

const LAYOUT: [f32; 2] = [640.0, 460.0];
const SIZE: [u32; 2] = [960, 690];

fn view() -> View {
    View::about([320.0, 230.0])
        .zoom(1.35)
        .roll(0.4)
        .pan([18.0, -12.0])
}

fn curve() -> ShadowCurve {
    ShadowCurve::new(
        Srgba::hex(0x2b3442),
        vec![
            CurvePoint::new(1.0, 4.0, 5.0, 0.22).contact(1.0, 1.5, 0.3),
            CurvePoint::new(2.0, 12.0, 16.0, 0.26),
        ],
    )
    .unwrap()
}

fn light() -> Light {
    Light {
        shadow: [0.4, 1.0],
        ..Light::default()
    }
}

fn draws() -> Vec<Draw> {
    vec![
        Draw::rect([320.0, 230.0], [600.0, 420.0], 24.0)
            .fill(Srgba::hex(0xeceff3))
            .shadow(Shadow::None)
            .id(1),
        Draw::rect([180.0, 150.0], [180.0, 120.0], 16.0)
            .vertical(Srgba::hex(0x5b78e6), Srgba::hex(0x3a4fc0))
            .stroke(Stroke::solid(Srgba::WHITE, 3.0))
            .elevation(1.0)
            .id(2),
        Draw::rect([430.0, 150.0], [160.0, 110.0], 12.0)
            .fill(Srgba::WHITE)
            .pattern(Pattern::new(
                PatternKind::Stripes,
                12.0,
                0.6,
                0.35,
                Srgba::hex(0x3d8fd8).alpha(0.8),
            ))
            .elevation(2.0)
            .id(3),
        Draw::circle([160.0, 330.0], 50.0)
            .fill(Srgba::hex(0xf0b429))
            .stroke(Stroke::dashed(Srgba::hex(0x2b3442), 4.0, 10.0, 6.0))
            .elevation(1.0)
            .id(4),
        Draw::new(Shape::Arc {
            radius: 44.0,
            width: 12.0,
            start: 0.3,
            sweep: 4.0,
        })
        .at([320.0, 330.0])
        .fill(Srgba::hex(0x138f8a))
        .elevation(1.0)
        .id(5),
        Draw::circle([470.0, 330.0], 30.0)
            .fill(Srgba::hex(0xe2614c))
            .shadow(Shadow::Glow(Glow {
                colour: Srgba::hex(0xe2614c).alpha(0.7),
                sigma: 8.0,
            }))
            .elevation(1.0)
            .id(6),
        Draw::rect([320.0, 410.0], [120.0, 26.0], 13.0)
            .fill(Srgba::hex(0x2b3442))
            .blur([40.0, 6.0])
            .elevation(2.0),
        Draw::new(Shape::Icon(Icon(0)))
            .transform(place([540.0, 70.0], 0.3))
            .fill(Srgba::hex(0x3d8fd8))
            .stroke(Stroke::solid(Srgba::WHITE, 4.0))
            .elevation(2.0)
            .id(7),
        Draw::new(Shape::Text(0))
            .transform(translation(60.0, 90.0))
            .elevation(3.0)
            .id(8),
    ]
}

fn turned(view: &Matrix, v: [f32; 2]) -> [f32; 2] {
    [
        view[0][0] * v[0] + view[1][0] * v[1],
        view[0][1] * v[0] + view[1][1] * v[1],
    ]
}

fn folded(draws: &[Draw], curve: &ShadowCurve, view: View) -> (Vec<Draw>, ShadowCurve, Light) {
    let m = view.matrix();
    let draws = draws
        .iter()
        .map(|draw| {
            let mut draw = *draw;
            draw.transform = multiply(m, draw.transform);
            draw.fx.blur = turned(&m, draw.fx.blur);
            if let Shadow::Glow(glow) = &mut draw.shadow {
                glow.sigma *= view.zoom;
            }
            draw
        })
        .collect();
    let mut curve = curve.clone();
    for point in &mut curve.points {
        point.offset *= view.zoom;
        point.sigma *= view.zoom;
        if let Some(contact) = &mut point.contact {
            contact.offset *= view.zoom;
            contact.sigma *= view.zoom;
        }
    }
    let light = Light {
        shadow: turned(&m, light().shadow),
        ..light()
    };
    (draws, curve, light)
}

fn scene<'a>(
    draws: &'a [Draw],
    curve: &'a ShadowCurve,
    light: Light,
    text: &'a [FlatText<'a>],
    icons: Option<&'a FlatIcons>,
) -> FlatScene<'a> {
    FlatScene {
        layout: LAYOUT,
        clear: Some(Srgba::hex(0xffffff)),
        curve,
        light,
        draws,
        groups: &[],
        text,
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

fn compare(a: &[[f32; 4]], b: &[[f32; 4]]) -> (f64, u8) {
    let mut total = 0u64;
    let mut worst = 0u8;
    for (a, b) in a.iter().zip(b) {
        let (a, b) = (quantize(*a), quantize(*b));
        for c in 0..3 {
            let d = a[c].abs_diff(b[c]);
            total += u64::from(d);
            worst = worst.max(d);
        }
    }
    (total as f64 / (a.len() * 3) as f64, worst)
}

fn paragraph(engine: &mut TextEngine) -> pfx_text::Paragraph {
    engine
        .layout(
            "Push in 1,284",
            TextStyle {
                family: "EB Garamond",
                size: 36.0,
                line_height: 44.0,
                wrap_width: None,
                pixels_per_unit: 1.0,
                representation: Representation::Msdf,
            },
            &Baseline::Straight {
                origin: [0.0, 0.0],
                direction: [1.0, 0.0],
            },
        )
        .unwrap()
}

#[test]
fn a_view_draws_the_board_as_if_its_matrix_were_folded_into_every_draw() {
    let icons = Icons::bake(&[icons::pointer()]).unwrap();
    let mut engine = TextEngine::new(pfx_text::fixture::EB_GARAMOND).unwrap();
    let paragraph = paragraph(&mut engine);
    let text = [CpuText::paragraph(&paragraph, Srgba::hex(0x2b3442))];
    let curve = curve();
    let draws = draws();
    let base = scene(&draws, &curve, light(), &[], None);
    let plain = cpu::render_with_text(&base, Some(&icons), &text, SIZE).unwrap();
    for identity in [IDENTITY, View::about([100.0, 40.0]).matrix()] {
        let same = render_viewed(&base, Some(&icons), Some(&text), SIZE, Some(identity)).unwrap();
        assert!(same == plain, "an identity view changed the board");
    }
    let viewed = render_viewed(
        &base,
        Some(&icons),
        Some(&text),
        SIZE,
        Some(view().matrix()),
    )
    .unwrap();
    let (folded_draws, folded_curve, folded_light) = folded(&draws, &curve, view());
    let folded = scene(&folded_draws, &folded_curve, folded_light, &[], None);
    let expected = cpu::render_with_text(&folded, Some(&icons), &text, SIZE).unwrap();
    let (mean, worst) = compare(&viewed, &expected);
    assert!(mean < 0.02 && worst <= 2, "mean {mean:.4} worst {worst}");
    assert!(compare(&viewed, &plain).0 > 1.0, "the view moved nothing");
    let fit = Fit::new(LAYOUT, SIZE)
        .unwrap()
        .viewed(Some(view().matrix()))
        .unwrap();
    for (point, id) in [
        ([180.0, 150.0], 2),
        ([430.0, 150.0], 3),
        ([160.0, 330.0], 4),
    ] {
        let [x, y] = fit.to_target(point).map(|v| v as u32);
        let picked = cpu::pick_viewed(
            &base,
            Some(&icons),
            None,
            SIZE,
            [x, y],
            Some(view().matrix()),
        );
        assert_eq!(picked.unwrap(), id);
        let back = fit.to_layout(fit.to_target(point));
        assert!((back[0] - point[0]).abs() < 1e-3 && (back[1] - point[1]).abs() < 1e-3);
    }
    assert!(
        Fit::new(LAYOUT, SIZE)
            .unwrap()
            .viewed(Some(View::about([0.0; 2]).zoom(0.0).matrix()))
            .is_err()
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_draws_and_picks_a_viewed_board_like_its_twin() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let icons = Icons::bake(&[icons::pointer()]).unwrap();
    let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons.clone());
    let mut engine = TextEngine::new(pfx_text::fixture::EB_GARAMOND).unwrap();
    let paragraph = paragraph(&mut engine);
    let atlas = crate::text::GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
    let colour = Srgba::hex(0x2b3442);
    let gpu_text = [FlatText {
        atlas: &atlas,
        glyphs: Glyphs::Paragraph(&paragraph),
        colour,
    }];
    let cpu_text = [CpuText::paragraph(&paragraph, colour)];
    let curve = curve();
    let draws = draws();
    let base = scene(&draws, &curve, light(), &gpu_text, Some(&flat_icons));
    let (plain, plain_ids) = gpu_render(&gpu, &mut pass, &base, SIZE);
    pass.set_view(Some(IDENTITY));
    let (same, same_ids) = gpu_render(&gpu, &mut pass, &base, SIZE);
    assert!(same == plain && same_ids == plain_ids);
    let matrix = view().matrix();
    pass.set_view(Some(matrix));
    let (pixels, ids) = gpu_render(&gpu, &mut pass, &base, SIZE);
    let twin = render_viewed(&base, Some(&icons), Some(&cpu_text), SIZE, Some(matrix)).unwrap();
    let (mean, worst) = compare(&pixels, &twin);
    eprintln!("viewed board against its twin: mean {mean:.4}/255, worst {worst}/255");
    assert!(mean < 0.15 && worst <= 4, "mean {mean:.4} worst {worst}");
    let mut checked = 0;
    let mut differing = 0;
    for y in (0..SIZE[1]).step_by(3) {
        for x in (0..SIZE[0]).step_by(3) {
            let expected = cpu::pick_viewed(
                &base,
                Some(&icons),
                Some(&cpu_text),
                SIZE,
                [x, y],
                Some(matrix),
            )
            .unwrap();
            checked += 1;
            if ids[(y * SIZE[0] + x) as usize] != expected {
                differing += 1;
            }
        }
    }
    assert!(
        differing * 1000 <= checked,
        "{differing} of {checked} picks differ"
    );
}
