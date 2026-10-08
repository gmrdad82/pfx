use super::cpu::{self, pick_with_text, render_with_text};
use super::cpu_text::CpuText;
use super::tests::{gpu, gpu_render};
use super::*;
use pfx_text::{Anchor, Baseline, Face, Paragraph, Representation, Span, Style, TextEngine};

const FONT: &[u8] = pfx_text::fixture::EB_GARAMOND;

fn paragraph(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    representation: Representation,
) -> Paragraph {
    engine
        .layout(
            text,
            Style {
                family: "EB Garamond",
                size,
                line_height: size * 1.2,
                wrap_width: None,
                pixels_per_unit: 1.0,
                representation,
            },
            &Baseline::Straight {
                origin: [0.0, 0.0],
                direction: [1.0, 0.0],
            },
        )
        .unwrap()
}

fn scene<'a>(
    layout: [f32; 2],
    curve: &'a ShadowCurve,
    draws: &'a [Draw],
    text: &'a [FlatText<'a>],
) -> FlatScene<'a> {
    FlatScene {
        layout,
        clear: Some(Srgba::WHITE),
        curve,
        light: Light::default(),
        draws,
        groups: &[],
        text,
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    }
}

fn ink(
    image: &[[f32; 4]],
    width: usize,
    rows: std::ops::Range<usize>,
    columns: std::ops::Range<usize>,
) -> f32 {
    rows.flat_map(|y| columns.clone().map(move |x| (x, y)))
        .map(|(x, y)| 1.0 - image[y * width + x][0])
        .sum()
}

fn dark(image: &[[f32; 4]], width: usize, x0: usize, x1: usize) -> usize {
    (0..image.len() / width)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| image[y * width + x][0] < 0.5)
        .count()
}

fn quantize(pixel: [f32; 4]) -> [u8; 3] {
    std::array::from_fn(|i| (pixel[i].clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[test]
fn a_label_draws_on_the_cpu_twin_in_painters_order_and_picks_by_its_id() {
    let mut engine = TextEngine::new(FONT).unwrap();
    let paragraph = paragraph(&mut engine, "MMMMMMMM", 40.0, Representation::Msdf);
    let text = [CpuText::paragraph(&paragraph, Srgba::hex(0x2b3442))];
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
    let scene = scene([400.0, 120.0], &curve, &draws, &[]);
    let image = render_with_text(&scene, None, &text, [400, 120]).unwrap();
    assert!(dark(&image, 400, 20, 190) > 200, "the label is missing");
    assert_eq!(dark(&image, 400, 205, 400), 0);
    let bare = cpu::render(&scene, None, [400, 120]).unwrap();
    assert_eq!(dark(&bare, 400, 0, 400), 0);

    let mut found = None;
    'search: for y in 40..90 {
        for x in 20..190 {
            if image[y * 400 + x][0] < 0.3 {
                found = Some([x as u32, y as u32]);
                break 'search;
            }
        }
    }
    let at = found.unwrap();
    assert_eq!(
        pick_with_text(&scene, None, &text, [400, 120], at).unwrap(),
        5
    );
    assert_eq!(
        pick_with_text(&scene, None, &text, [400, 120], [300, 60]).unwrap(),
        6
    );
    assert_eq!(
        pick_with_text(&scene, None, &text, [400, 120], [2, 2]).unwrap(),
        0
    );
    assert_eq!(
        pick_with_text(&scene, None, &[], [400, 120], at).unwrap_err(),
        "a draw names text 0, which is missing"
    );
}

#[test]
fn a_draw_that_names_a_missing_text_is_an_error_and_plain_render_skips_it() {
    let curve = ShadowCurve::none();
    let draws = [Draw::new(Shape::Text(3)).transform(translation(10.0, 10.0))];
    let scene = scene([100.0, 100.0], &curve, &draws, &[]);
    assert!(render_with_text(&scene, None, &[], [100, 100]).is_err());
    let plain = cpu::render(&scene, None, [100, 100]).unwrap();
    assert!(plain.iter().all(|pixel| *pixel == [1.0; 4]));
}

#[test]
fn msdf_and_coverage_labels_agree_on_the_cpu() {
    let mut engine = TextEngine::new(FONT).unwrap();
    let curve = ShadowCurve::none();
    let draws = [Draw::new(Shape::Text(0)).transform(translation(12.0, 60.0))];
    let mut renders = Vec::new();
    for representation in [Representation::Msdf, Representation::Coverage] {
        let paragraph = paragraph(&mut engine, "Ledger 1,284 gjy", 36.0, representation);
        let text = [CpuText::paragraph(&paragraph, Srgba::hex(0x000000))];
        let scene = scene([400.0, 90.0], &curve, &draws, &[]);
        renders.push(render_with_text(&scene, None, &text, [400, 90]).unwrap());
    }
    let (msdf, coverage) = (&renders[0], &renders[1]);
    let total = ink(msdf, 400, 0..90, 0..400);
    assert!(total > 300.0, "{total}");
    let ratio = ink(coverage, 400, 0..90, 0..400) / total;
    assert!((ratio - 1.0).abs() < 0.08, "ink ratio {ratio}");
    let mean = msdf
        .iter()
        .zip(coverage)
        .map(|(a, b)| f64::from((a[0] - b[0]).abs()))
        .sum::<f64>()
        / msdf.len() as f64
        * 255.0;
    assert!(mean < 3.0, "mean {mean}/255");
}

#[test]
fn a_labels_colour_opacity_and_scale_apply() {
    let mut engine = TextEngine::new(FONT).unwrap();
    let paragraph = paragraph(&mut engine, "WWWW", 40.0, Representation::Msdf);
    let curve = ShadowCurve::none();
    let render = |colour: Srgba, opacity: f32, scale: f32| {
        let text = [CpuText::paragraph(&paragraph, colour)];
        let draws = [Draw::new(Shape::Text(0))
            .transform(multiply(translation(10.0, 70.0), scaling(scale, scale)))
            .opacity(opacity)];
        let scene = scene([300.0, 100.0], &curve, &draws, &[]);
        render_with_text(&scene, None, &text, [300, 100]).unwrap()
    };
    let full = render(Srgba::hex(0x000000), 1.0, 1.0);
    let half = render(Srgba::hex(0x000000), 0.5, 1.0);
    let a = ink(&full, 300, 0..100, 0..300);
    let b = ink(&half, 300, 0..100, 0..300);
    assert!((b / a - 0.5).abs() < 0.01, "{}", b / a);
    let red = render(Srgba::hex(0xff0000), 1.0, 1.0);
    let darkest = red
        .iter()
        .min_by(|p, q| p[1].total_cmp(&q[1]))
        .copied()
        .unwrap();
    assert!(
        darkest[0] > 0.99 && darkest[1] < 0.05 && darkest[2] < 0.05,
        "{darkest:?}"
    );
    let big = render(Srgba::hex(0x000000), 1.0, 1.6);
    let c = ink(&big, 300, 0..100, 0..300);
    assert!((c / a - 2.56).abs() < 0.15, "{}", c / a);
}

#[test]
fn quads_draw_and_their_clip_rect_cuts_the_label() {
    let mut engine = TextEngine::new(FONT).unwrap();
    let face = Face {
        family: "EB Garamond".into(),
        size: 32.0,
        line: 40.0,
        weight: 500,
        italic: false,
        spacing: 0.0,
    };
    let spans = [Span {
        text: "Centre".into(),
        face,
        color: [0.0, 0.0, 0.0, 1.0],
    }];
    let curve = ShadowCurve::none();
    let draws = [Draw::new(Shape::Text(0))];
    let rich = engine
        .render_spans(
            &spans,
            None,
            1.0,
            Anchor::Start,
            Representation::Msdf,
            [20.0, 50.0],
            1.0,
            [0.0, 0.0, 70.0, 100.0],
            [0.0; 3],
        )
        .unwrap();
    let quads = crate::text::rich_quads(&rich);
    let text = [CpuText {
        atlas: &rich.atlas,
        glyphs: Glyphs::Quads(&quads),
        colour: Srgba::WHITE,
    }];
    let scene = scene([400.0, 100.0], &curve, &draws, &[]);
    let image = render_with_text(&scene, None, &text, [400, 100]).unwrap();
    assert!(dark(&image, 400, 20, 70) > 50);
    assert_eq!(dark(&image, 400, 72, 400), 0);
}

const MEAN_LIMIT: f64 = 0.02;
const WORST_LIMIT: u8 = 4;
const INK_LIMIT: f64 = 0.005;
const ID_LIMIT: f64 = 0.005;

struct Case {
    name: &'static str,
    representation: Representation,
    transform: Matrix,
    size: [u32; 2],
}

fn cases() -> Vec<Case> {
    let squash = multiply(
        translation(30.0, 90.0),
        multiply(rotation(-0.2), scaling(1.0, 0.35)),
    );
    let skew = {
        let mut m = scaling(1.4, 0.8);
        m[1][0] = 0.5;
        multiply(translation(20.0, 80.0), m)
    };
    vec![
        Case {
            name: "msdf 1x",
            representation: Representation::Msdf,
            transform: translation(20.0, 70.0),
            size: [480, 120],
        },
        Case {
            name: "msdf 2x",
            representation: Representation::Msdf,
            transform: translation(20.0, 70.0),
            size: [960, 240],
        },
        Case {
            name: "msdf squashed and turned",
            representation: Representation::Msdf,
            transform: squash,
            size: [480, 120],
        },
        Case {
            name: "msdf skewed",
            representation: Representation::Msdf,
            transform: skew,
            size: [480, 120],
        },
        Case {
            name: "msdf small",
            representation: Representation::Msdf,
            transform: multiply(translation(20.0, 70.0), scaling(0.4, 0.4)),
            size: [480, 120],
        },
        Case {
            name: "coverage 1x",
            representation: Representation::Coverage,
            transform: translation(20.0, 70.0),
            size: [480, 120],
        },
        Case {
            name: "coverage 2x",
            representation: Representation::Coverage,
            transform: translation(20.0, 70.0),
            size: [960, 240],
        },
        Case {
            name: "coverage minified",
            representation: Representation::Coverage,
            transform: multiply(translation(20.0, 70.0), scaling(0.5, 0.5)),
            size: [480, 120],
        },
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_twin_draws_labels_like_the_gpu_within_its_stated_tolerance() {
    let gpu = gpu();
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let mut engine = TextEngine::new(FONT).unwrap();
    let curve = ShadowCurve::none();
    let mut report = String::new();
    let mut failures = Vec::new();
    for case in cases() {
        let paragraph = paragraph(
            &mut engine,
            "Ledger 1,284 gjy MM",
            36.0,
            case.representation,
        );
        let atlas = crate::text::GpuAtlas::new(&gpu.device, &gpu.queue, &paragraph.atlas).unwrap();
        let colour = Srgba::hex(0x2b3442);
        let gpu_text = [FlatText {
            atlas: &atlas,
            glyphs: Glyphs::Paragraph(&paragraph),
            colour,
        }];
        let cpu_text = [CpuText::paragraph(&paragraph, colour)];
        let draws = [Draw::new(Shape::Text(0)).transform(case.transform).id(7)];
        let scene = scene([480.0, 120.0], &curve, &draws, &gpu_text);
        let (pixels, ids) = gpu_render(&gpu, &mut pass, &scene, case.size);
        let twin = render_with_text(&scene, None, &cpu_text, case.size).unwrap();
        let mut total = 0u64;
        let mut worst = 0u8;
        let mut gpu_ink = 0.0f64;
        let mut twin_ink = 0.0f64;
        for (a, b) in pixels.iter().zip(&twin) {
            let (a, b) = (quantize(*a), quantize(*b));
            for c in 0..3 {
                let d = a[c].abs_diff(b[c]);
                total += u64::from(d);
                worst = worst.max(d);
            }
            gpu_ink += f64::from(255 - a[0]);
            twin_ink += f64::from(255 - b[0]);
        }
        let mean = total as f64 / (pixels.len() * 3) as f64;
        let ratio = twin_ink / gpu_ink;
        let mut id_misses = 0usize;
        let mut id_total = 0usize;
        for y in 0..case.size[1] {
            for x in 0..case.size[0] {
                let expected = pick_with_text(&scene, None, &cpu_text, case.size, [x, y]).unwrap();
                let got = ids[(y * case.size[0] + x) as usize];
                if expected != 0 || got != 0 {
                    id_total += 1;
                    if expected != got {
                        id_misses += 1;
                    }
                }
            }
        }
        report += &format!(
            "{}: mean {mean:.4}/255 worst {worst}/255 ink {ratio:.4} ids {id_misses}/{id_total}\n",
            case.name
        );
        if mean > MEAN_LIMIT
            || worst > WORST_LIMIT
            || (ratio - 1.0).abs() > INK_LIMIT
            || id_misses as f64 > id_total as f64 * ID_LIMIT
        {
            failures.push(case.name);
        }
    }
    eprintln!("{report}");
    assert!(failures.is_empty(), "{failures:?}\n{report}");
}
