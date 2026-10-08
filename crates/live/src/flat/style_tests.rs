use super::grid::{CharGrid, Glyph, Grid, Interior, Rules, Weight};
use super::style::{BEVEL, Outline, restyle, restyle_group};
use super::svg;
use super::tests::{FILTERS, test_curve};
use super::viewport::{Frame, Mirror, Orientation, Viewport};
use super::*;
use pfx_core::ease::Ease;
use pfx_gpu::window::{LetterboxRounding, RenderScale, Size, viewport_with};

const NAMES: [&str; 12] = [
    "card", "faded", "sheet", "panel", "pill", "dashed", "arc", "glow", "icons", "pointer",
    "scrim", "board",
];

const MONO: &[u8] = pfx_text::fixture::IBM_PLEX_MONO;

pub(super) fn parsed(name: &str) -> (svg::Board, Icons) {
    let path = format!("{}/tests/flat/{name}.svg", env!("CARGO_MANIFEST_DIR"));
    let parsed = svg::read(&std::fs::read_to_string(path).unwrap(), &FILTERS).unwrap();
    let icons = if parsed.icons.is_empty() {
        Icons::bake(&[icons::pointer()]).unwrap()
    } else {
        Icons::bake(&parsed.icons).unwrap()
    };
    (parsed, icons)
}

pub(super) fn plain<'a>(
    layout: [f32; 2],
    clear: Option<Srgba>,
    curve: &'a ShadowCurve,
    draws: &'a [Draw],
) -> FlatScene<'a> {
    FlatScene {
        layout,
        clear,
        curve,
        light: Light::default(),
        draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    }
}

fn bytes(instance: &Instance) -> Vec<u8> {
    bytemuck::bytes_of(instance).to_vec()
}

#[test]
fn unstyled_draws_pack_the_bytes_they_packed_before() {
    let curve = test_curve();
    for name in NAMES {
        let (parsed, icons) = parsed(name);
        let fit = Fit::new(
            parsed.size,
            [parsed.size[0] as u32 * 2, parsed.size[1] as u32 * 2],
        )
        .unwrap();
        for draw in &parsed.draws {
            assert_eq!(draw.style, None);
            assert_eq!(
                pack_shape(draw, &fit, Some(&icons)).map(|i| bytes(&i)),
                pack_plain(draw, &fit, Some(&icons)).map(|i| bytes(&i)),
                "{name}"
            );
            let light = Light::default();
            let shadow = pack_shadow(draw, &fit, &curve, &light);
            if let Some(instance) = shadow {
                assert_eq!(instance.info[0] & BEVEL, 0);
            }
        }
    }
}

#[test]
fn the_unstyled_shader_is_the_styled_one_without_the_style_code() {
    assert!(shader::source().contains(shader::STYLE_CALL));
    assert!(!shader::unstyled_source().contains("flat_bevel"));
    assert_eq!(
        shader::source()
            .replace(shader::STYLE_CALL, "")
            .replace(&format!("{}\n", shader::STYLE_WGSL), ""),
        shader::unstyled_source()
    );
    for source in [
        shader::source(),
        shader::flat_source(),
        shader::unstyled_source(),
    ] {
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
    }
    assert!(!shader::flat_source().contains("flat_lit"));
}

fn overlap(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[1].min(b[1]) - a[0].max(b[0])).max(0.0)
}

fn alpha_row(image: &[[f32; 4]], width: u32, y: u32) -> Vec<f32> {
    (0..width)
        .map(|x| image[(y * width + x) as usize][3])
        .collect()
}

#[test]
fn outline_strokes_cover_like_analytic_edges() {
    let curve = ShadowCurve::none();
    for (scale, left, top) in [(1u32, 30.3f32, 35.0f32), (2, 30.25, 34.6), (1, 31.0, 35.5)] {
        let width = 3.0;
        let size = [40.0, 30.0];
        let draws = [
            Draw::rect([left + size[0] * 0.5, top + size[1] * 0.5], size, 0.0)
                .fill(Srgba::hex(0xc04030))
                .elevation(1.0)
                .style(Style::outline(width)),
        ];
        let target = [100 * scale, 100 * scale];
        let image =
            cpu::render(&plain([100.0, 100.0], None, &curve, &draws), None, target).unwrap();
        let s = scale as f32;
        let row = (top + size[1] * 0.5).floor() as u32 * scale;
        let alphas = alpha_row(&image, target[0], row);
        for (x, alpha) in alphas.iter().enumerate() {
            let pixel = [x as f32 / s, (x + 1) as f32 / s];
            let bands = [
                [left - width * 0.5, left + width * 0.5],
                [left + size[0] - width * 0.5, left + size[0] + width * 0.5],
            ];
            let expected = bands.iter().map(|band| overlap(pixel, *band)).sum::<f32>() * s;
            assert!(
                (alpha - expected).abs() < 2e-4,
                "scale {scale}, x {x}: {alpha} against {expected}"
            );
        }
        let column = (left + size[0] * 0.5).floor() as u32 * scale;
        for y in 0..target[1] {
            let alpha = image[(y * target[0] + column) as usize][3];
            let pixel = [y as f32 / s, (y + 1) as f32 / s];
            let bands = [
                [top - width * 0.5, top + width * 0.5],
                [top + size[1] - width * 0.5, top + size[1] + width * 0.5],
            ];
            let expected = bands.iter().map(|band| overlap(pixel, *band)).sum::<f32>() * s;
            assert!((alpha - expected).abs() < 2e-4, "scale {scale}, y {y}");
        }
        assert!(
            pack_shadow(
                &draws[0],
                &Fit::new([100.0; 2], target).unwrap(),
                &test_curve(),
                &Light::default()
            )
            .is_none(),
            "an outline casts no soft shadow"
        );
    }
}

#[test]
fn dashed_outlines_follow_the_perimeter_with_butt_ends() {
    let curve = ShadowCurve::none();
    let left = 20.0;
    let top = 40.0;
    let draws = [Draw::rect([left + 30.0, top + 15.0], [60.0, 30.0], 0.0)
        .fill(Srgba::hex(0x3060a0))
        .style(Style::Outline(Outline::dashed(4.0, 6.0, 4.0)))];
    let image = cpu::render(
        &plain([100.0, 100.0], None, &curve, &draws),
        None,
        [100, 100],
    )
    .unwrap();
    let row = top as u32;
    let alphas = alpha_row(&image, 100, row);
    for x in (left as u32 + 1)..(left as u32 + 58) {
        let s = [x as f32 - left, x as f32 + 1.0 - left];
        let on = (0..8)
            .map(|k| overlap(s, [k as f32 * 10.0, k as f32 * 10.0 + 6.0]))
            .sum::<f32>();
        assert!(
            (alphas[x as usize] - on).abs() < 2e-4,
            "x {x}: {} against {on}",
            alphas[x as usize]
        );
    }
}

#[test]
fn ghost_rings_and_arcs_dash_and_fade() {
    let curve = ShadowCurve::none();
    let ghost = Style::ghost(2.0, 5.0, 5.0, 0.4);
    let ring = Draw::new(Shape::Ring {
        radius: 30.0,
        width: 8.0,
    })
    .at([50.0, 50.0])
    .fill(Srgba::hex(0x205080))
    .style(ghost);
    let fit = Fit::new([100.0, 100.0], [100, 100]).unwrap();
    let instance = pack_shape(&ring, &fit, None).unwrap();
    assert_ne!(instance.info[0] & DASHED, 0);
    assert_eq!(instance.info[0] & (FILL_SOLID | FILL_VERTICAL), 0);
    assert!((instance.stroke_colour[3] - 0.4).abs() < 1e-6);
    let plain_ring = Draw {
        style: None,
        stroke: Some(Stroke::dashed(Srgba::WHITE, 2.0, 5.0, 5.0)),
        ..ring
    };
    assert_eq!(
        pack_shape(&plain_ring, &fit, None).unwrap().info[0] & DASHED,
        0,
        "an unstyled ring keeps its solid stroke"
    );
    let image = cpu::render(
        &plain([100.0, 100.0], None, &curve, &[ring]),
        None,
        [100, 100],
    )
    .unwrap();
    let outer = 34.0;
    let samples = (0..400)
        .map(|i| {
            let angle = i as f32 / 400.0 * std::f32::consts::TAU;
            let x = (50.0 + outer * angle.cos()) as usize;
            let y = (50.0 + outer * angle.sin()) as usize;
            image[y * 100 + x][3]
        })
        .collect::<Vec<_>>();
    let peak = samples.iter().copied().fold(0.0f32, f32::max);
    let gaps = samples.iter().filter(|&&a| a < 0.02).count();
    assert!(peak > 0.3 && peak <= 0.4 + 1e-4, "peak {peak}");
    assert!(
        gaps > 100,
        "the ring's outline is dashed: {gaps} gaps of 400"
    );
}

fn pixel(image: &[[f32; 4]], width: u32, x: u32, y: u32) -> [f32; 4] {
    image[(y * width + x) as usize]
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() < 2e-4)
}

#[test]
fn bevel_edges_are_whole_pixels_against_analytic_edges() {
    let curve = ShadowCurve::none();
    let fill = Srgba::hex(0x808890);
    let light = Srgba::hex(0xf0f2f4);
    let dark = Srgba::hex(0x303438);
    let mix = |a: Srgba, b: Srgba, t: f32| -> [f32; 4] {
        std::array::from_fn(|i| {
            if i == 3 {
                1.0
            } else {
                a.0[i] * t + b.0[i] * (1.0 - t)
            }
        })
    };
    for (scale, pixels) in [(1u32, 2.0f32), (2, 4.0)] {
        let draws = [Draw::rect([50.0, 50.0], [40.0, 30.0], 0.0)
            .fill(fill)
            .elevation(1.0)
            .style(Style::bevel(2.0, light, dark))];
        let target = [100 * scale, 100 * scale];
        let image =
            cpu::render(&plain([100.0, 100.0], None, &curve, &draws), None, target).unwrap();
        let s = scale;
        let y = 50 * s;
        let x0 = 30 * s;
        let x1 = 70 * s;
        let band = pixels as u32;
        for x in x0..x0 + band {
            assert!(
                near(pixel(&image, target[0], x, y), light.premultiplied()),
                "left x {x}"
            );
        }
        assert!(near(
            pixel(&image, target[0], x0 + band, y),
            fill.premultiplied()
        ));
        for x in x1 - band..x1 {
            assert!(
                near(pixel(&image, target[0], x, y), dark.premultiplied()),
                "right x {x}"
            );
        }
        assert!(near(
            pixel(&image, target[0], x1 - band - 1, y),
            fill.premultiplied()
        ));
        let x = 50 * s;
        for y in 35 * s..35 * s + band {
            assert!(
                near(pixel(&image, target[0], x, y), light.premultiplied()),
                "top y {y}"
            );
        }
        for y in 65 * s - band..65 * s {
            assert!(
                near(pixel(&image, target[0], x, y), dark.premultiplied()),
                "bottom y {y}"
            );
        }
        assert_eq!(pixel(&image, target[0], x, 35 * s - 1)[3], 0.0);
        let fit = Fit::new([100.0, 100.0], target).unwrap();
        assert!(pack_shadow(&draws[0], &fit, &test_curve(), &Light::default()).is_none());
        let bevelled = pack_shape(&draws[0], &fit, None).unwrap();
        assert_ne!(bevelled.info[0] & BEVEL, 0);
        assert_eq!(bevelled.info[0] & FX, 0);
        let moving = pack_shape(&draws[0].blur([12.0, 0.0]), &fit, None).unwrap();
        assert_eq!(
            moving.info[0] & BEVEL,
            0,
            "a blurred draw keeps its shift, not a band"
        );
        assert_ne!(moving.info[0] & BLUR, 0);
    }
    let draws = [Draw::rect([50.0, 50.0], [40.4, 30.0], 0.0)
        .fill(fill)
        .style(Style::bevel(2.0, light, dark))];
    let image = cpu::render(
        &plain([100.0, 100.0], None, &curve, &draws),
        None,
        [100, 100],
    )
    .unwrap();
    let left = 50.0 - 20.2;
    for x in 28..34u32 {
        let pixel_span = [x as f32, x as f32 + 1.0];
        let cover = overlap(pixel_span, [left, 100.0]);
        let band = overlap(pixel_span, [left, left + 2.0]);
        let expected: [f32; 4] = std::array::from_fn(|i| {
            light.premultiplied()[i] * band + fill.premultiplied()[i] * (cover - band)
        });
        assert!(
            near(pixel(&image, 100, x, 50), expected),
            "x {x}: {:?} against {expected:?}",
            pixel(&image, 100, x, 50)
        );
    }
    let _ = mix;
    let deck = Fit::new([1920.0, 1080.0], [1280, 800]).unwrap();
    let draws = [Draw::rect([450.0, 450.0], [300.0, 300.0], 0.0)
        .fill(fill)
        .style(Style::bevel(2.0, light, dark))];
    let image = cpu::render(
        &plain([1920.0, 1080.0], None, &curve, &draws),
        None,
        [1280, 800],
    )
    .unwrap();
    let [px, py] = deck.to_target([300.0, 450.0]);
    assert_eq!([px, py], [200.0, 340.0]);
    assert!(near(pixel(&image, 1280, 200, 340), light.premultiplied()));
    assert!(
        near(pixel(&image, 1280, 201, 340), fill.premultiplied()),
        "2 layout units are one whole pixel at 1280x800"
    );
}

#[test]
fn bevels_light_the_sides_facing_the_light_and_split_at_the_far_corners() {
    let curve = ShadowCurve::none();
    let fill = Srgba::hex(0x808890);
    let light = Srgba::WHITE;
    let dark = Srgba::hex(0x000000);
    let draws = [
        Draw::rect([30.0, 30.0], [40.0, 40.0], 0.0).fill(fill),
        Draw::circle([75.0, 30.0], 20.0).fill(fill),
        Draw::new(Shape::Ring {
            radius: 16.0,
            width: 10.0,
        })
        .at([50.0, 75.0])
        .fill(fill),
    ]
    .map(|draw| draw.style(Style::bevel(3.0, light, dark)));
    let image = cpu::render(
        &plain([100.0, 100.0], None, &curve, &draws),
        None,
        [100, 100],
    )
    .unwrap();
    let lum = |x: u32, y: u32| pixel(&image, 100, x, y)[0];
    assert!(
        lum(11, 30) > 0.99 && lum(30, 11) > 0.99,
        "left and top are lit"
    );
    assert!(
        lum(48, 30) < 0.01 && lum(30, 48) < 0.01,
        "right and bottom are dark"
    );
    assert!(
        lum(48, 12) < 0.5 && lum(47, 11) > 0.5,
        "the top right corner splits"
    );
    assert!(
        lum(10, 47) > 0.5 && lum(12, 49) < 0.5,
        "the bottom left corner splits"
    );
    assert!(
        lum(57, 22) > 0.99 && lum(93, 38) < 0.01,
        "the circle is lit from the top left"
    );
    assert!(
        lum(36, 61) > 0.99,
        "the ring's outer edge is lit at the top left"
    );
    assert!(
        lum(41, 66) < 0.01,
        "the ring's inner edge is dark at the top left"
    );
}

#[test]
fn flat_drops_elevation_shadow_and_dials_but_keeps_glow() {
    let fit = Fit::new([100.0, 100.0], [100, 100]).unwrap();
    let curve = test_curve();
    let light = Light::default();
    let lifted = Draw::rect([50.0, 50.0], [20.0, 20.0], 4.0)
        .fill(Srgba::WHITE)
        .elevation(2.0)
        .material(Material {
            thickness: 4.0,
            bevel: 2.0,
            gloss: 0.5,
            ..Material::default()
        });
    assert!(pack_shadow(&lifted, &fit, &curve, &light).is_some());
    let flat = lifted.style(Style::Flat);
    assert!(pack_shadow(&flat, &fit, &curve, &light).is_none());
    let instance = pack_shape(&flat, &fit, None).unwrap();
    assert_eq!(instance.info[0] & (LIT | SLAB), 0);
    let unlit = Draw {
        material: Material::default(),
        ..lifted
    };
    assert_eq!(
        bytes(&instance),
        bytes(&pack_plain(&unlit, &fit, None).unwrap())
    );
    let glowing = flat.shadow(Shadow::Glow(Glow {
        colour: Srgba::hex(0x40a0ff),
        sigma: 6.0,
    }));
    assert!(pack_shadow(&glowing, &fit, &curve, &light).is_some());
    let mut draws = vec![lifted, lifted.group(0), lifted.group(1)];
    restyle_group(&mut draws, 0, Some(Style::Flat));
    assert_eq!(
        draws.iter().map(|d| d.style).collect::<Vec<_>>(),
        vec![None, Some(Style::Flat), None]
    );
    restyle(&mut draws, None);
    assert!(draws.iter().all(|d| d.style.is_none()));
}

fn test_frame() -> Frame {
    Frame {
        size: [600.0, 1000.0],
        bezel: 24.0,
        radius: 48.0,
        colour: Srgba::hex(0x22262c),
    }
}

const MARKS: [[f32; 2]; 7] = [
    [0.1, 0.1],
    [0.9, 0.1],
    [0.5, 0.5],
    [0.15, 0.8],
    [0.85, 0.92],
    [0.3, 0.35],
    [0.7, 0.6],
];

const MARK: f32 = 30.0;

fn marks(layout: [f32; 2]) -> Vec<Draw> {
    MARKS
        .iter()
        .enumerate()
        .map(|(index, at)| {
            Draw::rect([at[0] * layout[0], at[1] * layout[1]], [MARK, MARK], 0.0)
                .fill(Srgba::hex(0x3060a0))
                .elevation(1.0)
                .shadow(Shadow::None)
                .id(index as u32 + 1)
        })
        .collect()
}

fn placed_scene(viewport: &Viewport, now: f32) -> (viewport::Placement, Vec<Draw>) {
    let placement = viewport.at(now);
    let mut draws = marks(placement.layout);
    draws.push(
        Draw::rect(
            [placement.layout[0] * 0.5, placement.layout[1] * 0.5],
            [placement.layout[0] * 1.6, 40.0],
            0.0,
        )
        .fill(Srgba::hex(0xa04030))
        .elevation(0.5)
        .shadow(Shadow::None)
        .id(99),
    );
    placement.place_all(&mut draws);
    draws.extend(placement.frame_draws(10.0, Some(Srgba::hex(0xe8eaee)), 4000.0));
    (placement, draws)
}

#[test]
fn the_pointer_map_matches_the_drawing_through_a_turn() {
    let curve = ShadowCurve::none();
    let layout = [1920.0, 1080.0];
    for (target, step) in [([1280u32, 800u32], 5u32), ([1920, 1080], 7)] {
        let mut viewport = Viewport::new([960.0, 540.0], test_frame());
        viewport.turn_to(
            Orientation::Landscape,
            1.0,
            0.6,
            Ease::SineInOut,
            false,
            false,
        );
        let window = viewport_with(
            Size {
                width: target[0],
                height: target[1],
            },
            RenderScale::Native,
            Size {
                width: 1920,
                height: 1080,
            },
            LetterboxRounding::Nearest,
        )
        .unwrap();
        let fit = Fit::new(layout, target).unwrap();
        for now in [0.0, 1.0, 1.15, 1.3, 1.45, 1.599, 1.6, 2.5] {
            let (placement, draws) = placed_scene(&viewport, now);
            let expected_layout = if now >= 1.6 {
                [952.0, 552.0]
            } else {
                [552.0, 952.0]
            };
            assert_eq!(placement.layout, expected_layout, "at {now}");
            let scene = plain(layout, Some(Srgba::hex(0xe8eaee)), &curve, &draws);
            let half = MARK * 0.5;
            for (index, at) in MARKS.iter().enumerate() {
                let inner = [at[0] * placement.layout[0], at[1] * placement.layout[1]];
                let window_point = placement.to_window(inner);
                let pixel = fit.to_target(window_point);
                let picked =
                    cpu::pick(&scene, None, target, [pixel[0] as u32, pixel[1] as u32]).unwrap();
                assert_eq!(picked, index as u32 + 1, "mark {index} at {now}");
                let pointer =
                    window.pointer_to_layout(pixel[0].floor() + 0.5, pixel[1].floor() + 0.5);
                let back = placement.pointer(pointer).unwrap();
                assert!(
                    (back[0] - inner[0]).abs() <= 1.0 / fit.scale
                        && (back[1] - inner[1]).abs() <= 1.0 / fit.scale,
                    "mark {index} at {now}: {back:?} against {inner:?}"
                );
            }
            let margin = 1.5 / fit.scale;
            let mut checked = 0;
            for y in (0..target[1]).step_by(step as usize) {
                for x in (0..target[0]).step_by(step as usize) {
                    let pointer = window.pointer_to_layout(x as f32 + 0.5, y as f32 + 0.5);
                    let picked = cpu::pick(&scene, None, target, [x, y]).unwrap();
                    let Some(inner) = placement.pointer(pointer) else {
                        let edge = placement.to_layout(pointer);
                        let outside = edge[0] < -margin
                            || edge[1] < -margin
                            || edge[0] > placement.layout[0] + margin
                            || edge[1] > placement.layout[1] + margin;
                        if outside {
                            assert_eq!(picked, 0, "outside the screen at {x},{y}, {now}");
                        }
                        continue;
                    };
                    let screen_edge = inner[0]
                        .min(inner[1])
                        .min(placement.layout[0] - inner[0])
                        .min(placement.layout[1] - inner[1]);
                    if screen_edge < margin + placement.frame.screen_radius() {
                        continue;
                    }
                    let mut expected = None;
                    let mut ambiguous = false;
                    for (index, at) in MARKS.iter().enumerate() {
                        let c = [at[0] * placement.layout[0], at[1] * placement.layout[1]];
                        let d = (inner[0] - c[0]).abs().max((inner[1] - c[1]).abs()) - half;
                        if d < -margin {
                            expected = Some(index as u32 + 1);
                        } else if d < margin {
                            ambiguous = true;
                        }
                    }
                    let bar = (inner[1] - placement.layout[1] * 0.5).abs() - 20.0;
                    if expected.is_none() {
                        if bar < -margin {
                            expected = Some(99);
                        } else if bar < margin {
                            ambiguous = true;
                        }
                    }
                    if ambiguous {
                        continue;
                    }
                    checked += 1;
                    assert_eq!(picked, expected.unwrap_or(0), "pixel {x},{y} at {now}");
                }
            }
            assert!(checked > 1000, "{checked} pixels checked");
        }
    }
}

#[test]
fn reduced_motion_turns_at_once_and_a_turn_settles() {
    let mut viewport = Viewport::new([960.0, 540.0], test_frame());
    viewport.turn_to(Orientation::Landscape, 2.0, 0.6, Ease::Linear, true, true);
    let placement = viewport.at(2.0);
    assert!(placement.settled());
    assert_eq!(placement.orientation, Orientation::Landscape);
    assert_eq!(placement.angle, 0.0);
    let mut viewport = Viewport::new([960.0, 540.0], test_frame());
    viewport.turn_to(Orientation::Landscape, 2.0, 0.6, Ease::Linear, true, false);
    let middle = viewport.at(2.3);
    assert!(!middle.settled());
    assert_eq!(middle.orientation, Orientation::Portrait);
    assert!((middle.angle - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
    let corner = viewport.at(2.0).to_window([0.0, 0.0]);
    let end = viewport.at(2.59999).to_window([0.0, 0.0]);
    let settled = viewport.at(2.6);
    let frame_end = viewport.at(2.59999).frame_draws(0.0, None, 0.0);
    let frame_settled = settled.frame_draws(0.0, None, 0.0);
    let reach = |draw: &Draw| {
        let Shape::Rect { half, .. } = draw.shape else {
            unreachable!()
        };
        let m = draw.transform;
        [
            (m[0][0] * half[0]).abs() + (m[1][0] * half[1]).abs(),
            (m[0][1] * half[0]).abs() + (m[1][1] * half[1]).abs(),
        ]
    };
    let a = reach(&frame_end[0]);
    let b = reach(&frame_settled[0]);
    assert!(
        (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05,
        "the turned frame lands on the landscape frame: {a:?} {b:?}"
    );
    assert_ne!(corner, end);
    viewport.settle(2.7);
    assert_eq!(viewport.orientation, Orientation::Landscape);
    assert_eq!(viewport.turn, None);
    assert_eq!(viewport.at(3.0).layout, [952.0, 552.0]);
    viewport.turn_to(Orientation::Landscape, 3.0, 0.6, Ease::Linear, true, false);
    assert_eq!(
        viewport.turn, None,
        "turning to the same orientation does nothing"
    );
}

#[test]
fn mirrors_keep_shapes_upright_and_reverse_gravity() {
    let curve = ShadowCurve::none();
    let base = Viewport::new([960.0, 540.0], test_frame());
    let layout = base.at(0.0).layout;
    let tilted = Draw::rect([100.0, 200.0], [60.0, 20.0], 0.0)
        .transform(place([100.0, 200.0], 0.3))
        .fill(Srgba::WHITE);
    let label =
        Draw::new(Shape::Text(0)).transform(multiply(translation(40.0, 60.0), scaling(0.5, 0.5)));
    for (mirror, gravity) in [
        (Mirror::None, [0.0, 1.0]),
        (Mirror::Vertical, [0.0, -1.0]),
        (Mirror::Horizontal, [0.0, 1.0]),
    ] {
        let placement = base.mirror(mirror).at(0.0);
        assert_eq!(placement.gravity(), gravity, "{mirror:?}");
        let p = [200.0, 300.0];
        let a = placement.to_window(p);
        let b = placement.to_window([p[0] + gravity[0], p[1] + gravity[1]]);
        assert!((b[0] - a[0]).abs() < 1e-4 && (b[1] - a[1] - 1.0).abs() < 1e-4);
        let point = [123.0, 456.0];
        let window = placement.to_window(point);
        let expected = match mirror {
            Mirror::None => [
                960.0 - layout[0] * 0.5 + 123.0,
                540.0 - layout[1] * 0.5 + 456.0,
            ],
            Mirror::Vertical => [
                960.0 - layout[0] * 0.5 + 123.0,
                540.0 + layout[1] * 0.5 - 456.0,
            ],
            Mirror::Horizontal => [
                960.0 + layout[0] * 0.5 - 123.0,
                540.0 - layout[1] * 0.5 + 456.0,
            ],
        };
        assert!(
            (window[0] - expected[0]).abs() < 1e-3 && (window[1] - expected[1]).abs() < 1e-3,
            "{mirror:?}: {window:?} against {expected:?}"
        );
        let back = placement.to_layout(window);
        assert!((back[0] - point[0]).abs() < 1e-3 && (back[1] - point[1]).abs() < 1e-3);
        let text = placement.place(&label).transform;
        assert!(
            text[0][0] > 0.0 && text[1][1] > 0.0,
            "{mirror:?}: text stays upright"
        );
        let turned = placement.place(&tilted).transform;
        let det = turned[0][0] * turned[1][1] - turned[1][0] * turned[0][1];
        assert!(det > 0.0, "{mirror:?}: shapes are not reflected");
        let angle = turned[0][1].atan2(turned[0][0]);
        let expected_angle = if mirror == Mirror::None { 0.3 } else { -0.3 };
        assert!((angle - expected_angle).abs() < 1e-5, "{mirror:?}: {angle}");
        let reflected = placement.reflected(tilted.transform);
        let det = reflected[0][0] * reflected[1][1] - reflected[1][0] * reflected[0][1];
        assert_eq!(
            det > 0.0,
            mirror == Mirror::None,
            "reflected mirrors the shape too"
        );
    }
    let mut turning = base.mirror(Mirror::Vertical);
    turning.turn_to(Orientation::Landscape, 0.0, 1.0, Ease::Linear, true, false);
    for now in [0.25, 0.5, 0.75] {
        let placement = turning.at(now);
        let g = placement.gravity();
        let a = placement.to_window([200.0, 300.0]);
        let b = placement.to_window([200.0 + g[0] * 10.0, 300.0 + g[1] * 10.0]);
        assert!(
            (b[0] - a[0]).abs() < 1e-3 && (b[1] - a[1] - 10.0).abs() < 1e-3,
            "{now}"
        );
    }
    let placement = base.mirror(Mirror::Vertical).at(0.0);
    let mut draws = marks(placement.layout);
    placement.place_all(&mut draws);
    let scene = plain([1920.0, 1080.0], None, &curve, &draws);
    let fit = Fit::new([1920.0, 1080.0], [1920, 1080]).unwrap();
    let top_mark = placement.to_window([0.1 * layout[0], 0.1 * layout[1]]);
    assert!(
        top_mark[1] > 540.0,
        "the first mark is at the bottom when mirrored"
    );
    let pixel = fit.to_target(top_mark);
    assert_eq!(
        cpu::pick(
            &scene,
            None,
            [1920, 1080],
            [pixel[0] as u32, pixel[1] as u32]
        )
        .unwrap(),
        1
    );
}

#[test]
fn the_frame_covers_what_spills_out_of_the_screen() {
    let curve = ShadowCurve::none();
    let mut viewport = Viewport::new([960.0, 540.0], test_frame());
    viewport.turn_to(Orientation::Landscape, 1.0, 1.0, Ease::Linear, false, false);
    for now in [0.0, 1.3, 1.7] {
        let (placement, draws) = placed_scene(&viewport, now);
        let scene = plain([1920.0, 1080.0], None, &curve, &draws);
        let pick = |p: [f32; 2]| {
            cpu::pick(&scene, None, [1920, 1080], [p[0] as u32, p[1] as u32]).unwrap()
        };
        let mid = placement.layout[1] * 0.5;
        assert_eq!(
            pick(placement.to_window([placement.layout[0] * 0.25, mid])),
            99
        );
        assert_eq!(
            pick(placement.to_window([-10.0, mid])),
            0,
            "the bezel covers the bar at {now}"
        );
        assert_eq!(
            pick(placement.to_window([-100.0, mid])),
            0,
            "the mat covers the bar at {now}"
        );
    }
    let (_, draws) = placed_scene(&viewport, 0.0);
    let image = cpu::render(
        &plain([1920.0, 1080.0], None, &curve, &draws),
        None,
        [1920, 1080],
    )
    .unwrap();
    let at = |x: f32| image[540 * 1920 + x as usize];
    assert!(near(
        at(960.0 - 300.0 + 10.0),
        test_frame().colour.premultiplied()
    ));
    assert!(near(
        at(960.0 - 400.0),
        Srgba::hex(0xe8eaee).premultiplied()
    ));
}

fn mono_face(size: f32, line: f32) -> pfx_text::Face {
    pfx_text::Face {
        family: "IBM Plex Mono".into(),
        size,
        line,
        weight: 400,
        italic: false,
        spacing: 0.0,
    }
}

fn known_layout() -> (Vec<Draw>, Vec<(&'static str, Srgba)>) {
    let ink = Srgba::hex(0x303030);
    let ground = Srgba::hex(0xf0f0f0);
    let draws = vec![
        Draw::rect([60.0, 50.0], [100.0, 80.0], 0.0)
            .stroke(Stroke::solid(ink, 2.0))
            .id(1),
        Draw::rect([60.0, 40.0], [100.0, 2.0], 0.0).fill(ink).id(2),
        Draw::rect([140.0, 50.0], [40.0, 60.0], 12.0)
            .fill(ground)
            .stroke(Stroke::solid(ink, 2.0))
            .elevation(1.0)
            .id(3),
        Draw::rect([30.0, 105.0], [40.0, 2.0], 0.0).fill(ink).id(4),
        Draw::circle([175.0, 105.0], 3.0).fill(ink).id(5),
        Draw::new(Shape::Text(0))
            .transform(translation(20.0, 20.0))
            .elevation(0.5),
        Draw::new(Shape::Icon(Icon(0))).at([100.0, 100.0]).fill(ink),
    ];
    (draws, vec![("ab", ink)])
}

#[test]
fn shapes_map_to_box_drawing_on_a_known_layout() {
    let grid = Grid::new([0.0, 0.0], [10.0, 10.0], 20, 12).unwrap();
    let (draws, labels) = known_layout();
    let mut chars = CharGrid::new(grid, Rules::default());
    chars.map(&draws, 0, &labels).unwrap();
    let expected = [
        "",
        " ┌────────┐",
        " │ab      │ ┌──┐",
        " │        │ │  │",
        " ├────────┤ │  │",
        " │        │ │  │",
        " │        │ │  │",
        " │        │ └──┘",
        " └────────┘",
        "",
        " ╶──╴            •",
        "",
    ];
    assert_eq!(chars.lines(), expected);
    let mut rounded = CharGrid::new(
        grid,
        Rules {
            rounded: 8.0,
            raised: Some((1.0, Weight::Heavy)),
            ..Rules::default()
        },
    );
    rounded.map(&draws, 0, &labels).unwrap();
    assert_eq!(rounded.lines()[2], " │ab      │ ┏━━┓");
    let mut light_rounded = CharGrid::new(
        grid,
        Rules {
            rounded: 8.0,
            ..Rules::default()
        },
    );
    light_rounded.map(&draws, 0, &labels).unwrap();
    assert_eq!(light_rounded.lines()[2], " │ab      │ ╭──╮");
    assert_eq!(light_rounded.lines()[7], " │        │ ╰──╯");
    let mut double = CharGrid::new(
        grid,
        Rules {
            line: Weight::Double,
            ..Rules::default()
        },
    );
    double.map(&draws, 0, &labels).unwrap();
    assert_eq!(double.lines()[1], " ╔════════╗");
    assert_eq!(double.lines()[4], " ╠════════╣ ║  ║");
    assert_eq!(double.lines()[7], " ║        ║ ╚══╝");
    let mut overlapping = CharGrid::new(grid, Rules::default());
    overlapping.shape(
        &Draw::rect([50.0, 50.0], [60.0, 60.0], 0.0).stroke(Stroke::solid(Srgba::WHITE, 1.0)),
    );
    overlapping.shape(
        &Draw::rect([80.0, 70.0], [60.0, 60.0], 0.0).stroke(Stroke::solid(Srgba::WHITE, 1.0)),
    );
    assert_eq!(overlapping.lines()[4], "  │  ┌─┼──┐", "outlines cross");
    let mut covered = CharGrid::new(grid, Rules::default());
    covered.shape(
        &Draw::rect([50.0, 50.0], [60.0, 60.0], 0.0).stroke(Stroke::solid(Srgba::WHITE, 1.0)),
    );
    covered.shape(&Draw::rect([80.0, 70.0], [60.0, 60.0], 0.0).fill(Srgba::WHITE));
    assert_eq!(
        covered.lines()[4],
        "  │  ┌────┐",
        "a filled box covers what is beneath"
    );
    assert_eq!(covered.lines()[7], "  └──│    │");
    assert!(matches!(covered.cell(6, 7).unwrap().glyph, Glyph::Empty));
    assert_eq!(covered.cell(6, 7).unwrap().ground, Some(Srgba::WHITE));
}

#[test]
fn grid_draws_are_flat_crisp_and_merged() {
    let grid = Grid::new([0.0, 0.0], [10.0, 10.0], 20, 12).unwrap();
    let (draws, labels) = known_layout();
    let mut chars = CharGrid::new(
        grid,
        Rules {
            ink: Some(Srgba::hex(0x40ff70)),
            ground: Some(Srgba::hex(0x081008)),
            interior: Interior::Background,
            thickness: 0.2,
            ..Rules::default()
        },
    );
    chars.map(&draws, 0, &labels).unwrap();
    let mut out = Vec::new();
    chars.draws(Some(0), &mut out);
    assert!(out.iter().all(|d| d.elevation == 0.0
        && d.shadow == Shadow::None
        && d.material == Material::default()
        && !matches!(d.shape, Shape::Icon(_))));
    assert_eq!(
        out.iter()
            .filter(|d| matches!(d.shape, Shape::Text(0)))
            .count(),
        1
    );
    let ground = out
        .iter()
        .filter(|d| d.fill == Fill::Solid(Srgba::hex(0x081008)))
        .count();
    assert_eq!(ground, 1, "the filled box's ground is one rect");
    let lines = out
        .iter()
        .filter(|d| {
            matches!(d.shape, Shape::Rect { .. }) && d.fill == Fill::Solid(Srgba::hex(0x40ff70))
        })
        .count();
    assert!(lines <= 14, "line runs merge across cells: {lines}");
    let curve = ShadowCurve::none();
    let scene = plain(
        [200.0, 120.0],
        Some(Srgba::hex(0)),
        &curve,
        &out[..out.len() - 1],
    );
    let image = cpu::render(&scene, None, [200, 120]).unwrap();
    let green = |x: u32, y: u32| image[(y * 200 + x) as usize][1];
    for x in 16..104 {
        assert!(green(x, 15) > 0.9, "the top line is unbroken at x {x}");
    }
    for y in 16..84 {
        assert!(green(15, y) > 0.9, "the left line is unbroken at y {y}");
    }
    let mut double = CharGrid::new(
        grid,
        Rules {
            line: Weight::Double,
            ink: Some(Srgba::WHITE),
            thickness: 0.2,
            ..Rules::default()
        },
    );
    double.shape(
        &Draw::rect([60.0, 50.0], [100.0, 80.0], 0.0).stroke(Stroke::solid(Srgba::WHITE, 1.0)),
    );
    let mut out = Vec::new();
    double.draws(None, &mut out);
    let scene = plain([200.0, 120.0], Some(Srgba::hex(0)), &curve, &out);
    let image = cpu::render(&scene, None, [200, 120]).unwrap();
    let white = |x: u32, y: u32| image[(y * 200 + x) as usize][0];
    assert!(white(30, 13) > 0.9 && white(30, 17) > 0.9 && white(30, 15) < 0.1);
    for (x, y) in [(13, 13), (13, 17), (17, 13), (17, 17), (13, 30), (17, 30)] {
        assert!(white(x, y) > 0.9, "the double corner is drawn at {x},{y}");
    }
    for (x, y) in [(15, 15), (17, 15), (15, 17), (15, 30), (30, 15), (11, 11)] {
        assert!(white(x, y) < 0.1, "the double corner is open at {x},{y}");
    }
}

#[test]
fn text_snaps_to_cells_in_the_callers_mono_font() {
    let mut engine = pfx_text::TextEngine::new(MONO).unwrap();
    let face = mono_face(20.0, 28.0);
    let cell = Grid::measure(&mut engine, &face).unwrap();
    assert!(
        (cell[0] - 12.0).abs() < 0.05,
        "Plex Mono advances 0.6 em: {cell:?}"
    );
    let grid = Grid::new([7.0, 11.0], [cell[0], 32.0], 30, 6).unwrap();
    let mut chars = CharGrid::new(grid, Rules::default());
    chars.text(
        [7.0 + cell[0] * 2.2, 11.0 + 32.0 * 0.9],
        "Hello, grid",
        Srgba::hex(0xff0000),
        0,
    );
    chars.text(
        [7.0 + cell[0] * 4.0, 11.0 + 64.0],
        "x  y",
        Srgba::hex(0x00ff00),
        0,
    );
    chars.shape(&Draw::rect([200.0, 160.0], [60.0, 2.0], 0.0).fill(Srgba::WHITE));
    assert_eq!(chars.lines()[1].trim_start(), "Hello, grid");
    assert_eq!(chars.lines()[1].find('H'), Some(2));
    assert_eq!(chars.lines()[2], "    x  y");
    let (rich, quads) = chars
        .render(&mut engine, &face, pfx_text::Representation::Msdf)
        .unwrap();
    assert_eq!(rich.quads.len(), quads.len());
    assert_eq!(quads.len(), "Hello,grid".len() + 2);
    for (quad, snapped) in rich.quads.iter().zip(&quads) {
        let dx = snapped.rect[0] - quad.rect[0];
        let pen = quad.pen[0] + dx - grid.origin[0];
        let column = pen / cell[0];
        assert!(
            (column - column.round()).abs() < 1e-3,
            "pen {pen} is on a cell edge"
        );
        let dy = snapped.rect[1] - quad.rect[1];
        let baseline = quad.line_baseline + dy - grid.origin[1];
        let row = quad.line_index as f32;
        let within = baseline - row * 32.0;
        assert!(
            (0.0..32.0).contains(&within),
            "the baseline {baseline} sits in row {row}"
        );
    }
    let first = quads
        .iter()
        .zip(&rich.quads)
        .find(|(_, q)| q.line_index == 1)
        .unwrap();
    let second = quads
        .iter()
        .zip(&rich.quads)
        .find(|(_, q)| q.line_index == 2)
        .unwrap();
    let base = |(s, q): (&crate::text::RichQuad, &pfx_text::RichGlyphQuad)| {
        q.line_baseline + (s.rect[1] - q.rect[1])
    };
    assert!(
        (base(second) - base(first) - 32.0).abs() < 1e-3,
        "rows are one cell apart"
    );
    assert_eq!(rich.quads[0].color[0], 1.0, "each run keeps its ink");
}

fn bits(pixels: &[[f32; 4]]) -> Vec<u32> {
    pixels.iter().flatten().map(|v| v.to_bits()).collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn unstyled_frames_are_byte_identical_on_the_gpu() {
    use super::tests::{gpu, gpu_render};
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut styled = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let mut unstyled =
        turns.cpu(|| FlatPass::with_source(&gpu.device, &gpu.queue, shader::unstyled_source()));
    let curve = test_curve();
    for name in NAMES {
        for lit in [false, true] {
            let (mut parsed, icons) = parsed(name);
            if lit {
                for draw in &mut parsed.draws {
                    draw.material = Material {
                        thickness: 2.0,
                        bevel: 1.0,
                        gloss: 0.15,
                        roughness: 0.45,
                        ..Material::default()
                    };
                }
            }
            let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons);
            let scene = FlatScene {
                groups: &parsed.groups,
                icons: Some(&flat_icons),
                sprites: None,
                ..plain(parsed.size, Some(Srgba::hex(0)), &curve, &parsed.draws)
            };
            let size = [parsed.size[0] as u32 * 2, parsed.size[1] as u32 * 2];
            let (a, ids_a) = turns.cpu(|| gpu_render(&gpu, &mut styled, &scene, size));
            let (b, ids_b) = turns.cpu(|| gpu_render(&gpu, &mut unstyled, &scene, size));
            let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
            eprintln!("{name}, lit {lit}: {differing} pixels differ");
            assert!(
                bits(&a) == bits(&b),
                "{name}: the style code changed the colour"
            );
            assert!(ids_a == ids_b, "{name}: the style code changed the ids");
        }
    }
}

pub(super) fn styled_sheet() -> Vec<Draw> {
    let fill = Srgba::hex(0x8a94a0);
    let light = Srgba::hex(0xf4f6f8);
    let dark = Srgba::hex(0x2a2e34);
    let ink = Srgba::hex(0x1f6fb0);
    let mut draws = vec![
        Draw::rect([90.0, 70.0], [120.0, 70.0], 0.0)
            .fill(fill)
            .style(Style::bevel(2.0, light, dark))
            .id(1),
        Draw::rect([0.0, 0.0], [120.0, 70.0], 10.0)
            .transform(place([240.0, 70.0], 0.2))
            .fill(fill)
            .style(Style::bevel(3.0, light, dark))
            .id(2),
        Draw::circle([380.0, 70.0], 40.0)
            .fill(fill)
            .style(Style::bevel(3.0, light, dark))
            .id(3),
        Draw::new(Shape::Ring {
            radius: 34.0,
            width: 16.0,
        })
        .at([500.0, 70.0])
        .fill(fill)
        .style(Style::bevel(3.0, light, dark))
        .id(4),
        Draw::rect([90.0, 190.0], [120.0, 70.0], 12.0)
            .fill(ink)
            .elevation(2.0)
            .style(Style::outline(2.0))
            .id(5),
        Draw::rect([0.0, 0.0], [120.0, 70.0], 12.0)
            .transform(place([240.0, 190.0], -0.15))
            .fill(ink)
            .elevation(1.0)
            .style(Style::ghost(3.0, 9.0, 6.0, 0.45))
            .id(6),
        Draw::circle([380.0, 190.0], 40.0)
            .fill(ink)
            .style(Style::Outline(Outline::dashed(3.0, 12.0, 7.0)))
            .id(7),
        Draw::new(Shape::Arc {
            radius: 34.0,
            width: 10.0,
            start: -1.2,
            sweep: 4.2,
        })
        .at([500.0, 190.0])
        .fill(ink)
        .style(Style::ghost(2.0, 6.0, 5.0, 0.6))
        .id(8),
        Draw::rect([150.0, 300.0], [220.0, 60.0], 14.0)
            .fill(Srgba::WHITE)
            .elevation(2.0)
            .material(Material {
                thickness: 4.0,
                bevel: 2.0,
                gloss: 0.4,
                ..Material::default()
            })
            .style(Style::Flat)
            .id(9),
        Draw::rect([420.0, 300.0], [220.0, 60.0], 14.0)
            .fill(Srgba::WHITE)
            .elevation(2.0)
            .id(10),
    ];
    draws.push(
        Draw::rect([300.0, 360.0], [560.0, 20.0], 0.0)
            .vertical(fill, Srgba::hex(0x50585f))
            .style(Style::bevel(2.0, light, dark))
            .id(11),
    );
    draws
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_draws_styles_like_its_twin() {
    use super::tests::{gpu, gpu_render};
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut pass = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let curve = test_curve();
    let draws = styled_sheet();
    for scale in [1u32, 2] {
        let scene = plain([600.0, 400.0], Some(Srgba::hex(0xdfe3e8)), &curve, &draws);
        let size = [600 * scale, 400 * scale];
        let (ours, ids) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, size));
        let twin = turns.cpu(|| cpu::render(&scene, None, size)).unwrap();
        let mut total = 0.0f64;
        let mut worst = 0.0f32;
        for (a, b) in ours.iter().zip(&twin) {
            for c in 0..3 {
                let d = (a[c] - b[c]).abs();
                total += f64::from(d);
                worst = worst.max(d);
            }
        }
        let mean = total / (ours.len() * 3) as f64 * 255.0;
        eprintln!(
            "scale {scale}: mean {mean:.3}/255, worst {:.1}/255",
            worst * 255.0
        );
        assert!(mean < 0.2, "mean {mean}");
        assert!(worst * 255.0 < 4.0, "worst {}", worst * 255.0);
        let mut agree = 0usize;
        let mut checked = 0usize;
        for y in (0..size[1]).step_by(3) {
            for x in (0..size[0]).step_by(3) {
                checked += 1;
                if cpu::pick(&scene, None, size, [x, y]).unwrap() == ids[(y * size[0] + x) as usize]
                {
                    agree += 1;
                }
            }
        }
        let share = agree as f64 / checked as f64;
        eprintln!("scale {scale}: ids agree on {:.4}", share);
        assert!(share > 0.998);
    }
}

fn terminal_face() -> pfx_text::Face {
    mono_face(20.0, 30.0)
}

pub(super) struct Terminal {
    pub(super) draws: Vec<Draw>,
    pub(super) rich: pfx_text::RichParagraph,
    pub(super) quads: Vec<crate::text::RichQuad>,
    pub(super) cells: [u32; 2],
}

pub(super) fn terminal(source: &[Draw], lines: usize) -> Terminal {
    let mut engine = pfx_text::TextEngine::new(MONO).unwrap();
    let face = terminal_face();
    let cell = Grid::measure(&mut engine, &face).unwrap();
    let grid = Grid::cover([1920.0, 1080.0], [cell[0], 30.0]).unwrap();
    let phosphor = Srgba::hex(0x7dffa0);
    let mut chars = CharGrid::new(
        grid,
        Rules {
            ink: Some(phosphor),
            ground: Some(Srgba::hex(0x06140c)),
            rounded: 8.0,
            raised: Some((1.5, Weight::Double)),
            ..Rules::default()
        },
    );
    let shapes = source
        .iter()
        .filter(|draw| !matches!(draw.shape, Shape::Text(_)))
        .copied()
        .collect::<Vec<_>>();
    chars.map(&shapes, 0, &[]).unwrap();
    for line in 0..lines {
        chars.text(
            [
                grid.origin[0] + cell[0] * 2.0,
                grid.origin[1] + 30.0 * line as f32,
            ],
            &format!(
                "{line:03} the quick brown fox jumps over the lazy dog [ok] {}",
                line * 7919 % 1000
            ),
            phosphor,
            0,
        );
    }
    let (rich, quads) = chars
        .render(&mut engine, &face, pfx_text::Representation::Msdf)
        .unwrap();
    let mut draws = Vec::new();
    chars.draws(Some(0), &mut draws);
    Terminal {
        draws,
        rich,
        quads,
        cells: [grid.columns, grid.rows],
    }
}

fn write_png(path: &str, size: [u32; 2], pixels: &[[f32; 4]]) {
    let data = pixels
        .iter()
        .flat_map(|p| {
            let a = p[3].max(1e-6);
            [p[0] / a, p[1] / a, p[2] / a, p[3]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&data)
        .unwrap();
}

#[test]
#[ignore = "writes images for inspection; needs a GPU"]
fn dump_styles() {
    use super::tests::{gpu, gpu_render, spike_draws, spike_icons};
    let dir = format!("{}/../../tmp/flat-styles", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut pass = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let curve = test_curve();
    let sheet = styled_sheet();
    let scene = plain([600.0, 400.0], Some(Srgba::hex(0xdfe3e8)), &curve, &sheet);
    let (pixels, _) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, [1200, 800]));
    write_png(&format!("{dir}/sheet@2x.png"), [1200, 800], &pixels);
    let spike = spike_icons();
    let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, spike.icons.clone());
    let mut source = Vec::new();
    spike_draws(&spike, 0.0, Material::default(), 0, &mut source);
    source.retain(|draw| !matches!(draw.shape, Shape::Text(_)));
    let mut viewport = Viewport::new([960.0, 540.0], test_frame());
    viewport.turn_to(
        Orientation::Landscape,
        0.0,
        1.0,
        Ease::SineInOut,
        false,
        false,
    );
    for (label, now, mirror) in [
        ("portrait", 0.0, Mirror::None),
        ("turning", 0.4, Mirror::None),
        ("landscape", 1.0, Mirror::None),
        ("mirrored", 0.0, Mirror::Vertical),
    ] {
        let placement = viewport.mirror(mirror).at(now);
        let scale = (placement.layout[0] / 1920.0).min(placement.layout[1] / 1080.0);
        let mut draws = vec![
            Draw::rect(
                [placement.layout[0] * 0.5, placement.layout[1] * 0.5],
                placement.layout,
                0.0,
            )
            .fill(Srgba::hex(0xeceff3))
            .shadow(Shadow::None),
        ];
        draws.extend(source.iter().map(|draw| {
            let mut draw = *draw;
            draw.transform = multiply(scaling(scale, scale), draw.transform);
            draw
        }));
        placement.place_all(&mut draws);
        draws.extend(placement.frame_draws(10.0, Some(Srgba::hex(0x9aa3ad)), 4000.0));
        let scene = FlatScene {
            icons: Some(&flat_icons),
            sprites: None,
            ..plain([1920.0, 1080.0], Some(Srgba::hex(0x9aa3ad)), &curve, &draws)
        };
        let (pixels, _) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, [1280, 800]));
        write_png(&format!("{dir}/viewport-{label}.png"), [1280, 800], &pixels);
    }
    let term = terminal(&source, 12);
    let atlas = crate::text::GpuAtlas::new(&gpu.device, &gpu.queue, &term.rich.atlas).unwrap();
    let text = [FlatText {
        atlas: &atlas,
        glyphs: Glyphs::Quads(&term.quads),
        colour: Srgba::WHITE,
    }];
    let scene = FlatScene {
        text: &text,
        ..plain(
            [1920.0, 1080.0],
            Some(Srgba::hex(0x020804)),
            &curve,
            &term.draws,
        )
    };
    for size in [[1280u32, 800u32], [3840, 2160]] {
        let (pixels, _) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, size));
        write_png(
            &format!("{dir}/terminal-{}x{}.png", size[0], size[1]),
            size,
            &pixels,
        );
    }
    let mut outlined = source.clone();
    restyle(&mut outlined, Some(Style::outline(2.0)));
    let mut bevelled = source.clone();
    restyle(
        &mut bevelled,
        Some(Style::bevel(
            2.0,
            Srgba::hex(0xffffff),
            Srgba::hex(0x404850),
        )),
    );
    for (label, draws) in [("outlined", outlined), ("bevelled", bevelled)] {
        let scene = FlatScene {
            icons: Some(&flat_icons),
            sprites: None,
            ..plain([1920.0, 1080.0], Some(Srgba::hex(0xeceff3)), &curve, &draws)
        };
        let (pixels, _) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, [1280, 800]));
        write_png(&format!("{dir}/spike-{label}.png"), [1280, 800], &pixels);
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_grid_font_holds_the_deck_floor() {
    use super::tests::{gpu, gpu_render};
    let gpu = gpu();
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut pass = turns.cpu(|| FlatPass::new(&gpu.device, &gpu.queue));
    let mut engine = pfx_text::TextEngine::new(MONO).unwrap();
    let curve = ShadowCurve::none();
    let mut report = String::new();
    let mut floor = None;
    for size in [14.0f32, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0] {
        let face = mono_face(size, size * 1.5);
        let cell = Grid::measure(&mut engine, &face).unwrap();
        let grid = Grid::cover([1920.0, 1080.0], cell).unwrap();
        let mut heights = [0u32; 2];
        for (slot, glyph) in ["x", "H"].into_iter().enumerate() {
            let mut chars = CharGrid::new(grid, Rules::default());
            chars.text(grid.corner([4, 4]), glyph, Srgba::hex(0x000000), 0);
            let (rich, quads) = chars
                .render(&mut engine, &face, pfx_text::Representation::Msdf)
                .unwrap();
            let atlas = crate::text::GpuAtlas::new(&gpu.device, &gpu.queue, &rich.atlas).unwrap();
            let text = [FlatText {
                atlas: &atlas,
                glyphs: Glyphs::Quads(&quads),
                colour: Srgba::WHITE,
            }];
            let mut draws = Vec::new();
            chars.draws(Some(0), &mut draws);
            let scene = FlatScene {
                text: &text,
                ..plain([1920.0, 1080.0], Some(Srgba::WHITE), &curve, &draws)
            };
            let (pixels, _) = turns.cpu(|| gpu_render(&gpu, &mut pass, &scene, [1280, 800]));
            heights[slot] = (0..800usize)
                .filter(|&y| (0..1280usize).any(|x| pixels[y * 1280 + x][1] < 0.7))
                .count() as u32;
        }
        if heights[0] >= 9 && floor.is_none() {
            floor = Some(size);
        }
        report += &format!(
            "size {size}: cell {:.1}x{:.1}, grid {}x{}, x-height {} px, cap {} px at 1280x800\n",
            cell[0], cell[1], grid.columns, grid.rows, heights[0], heights[1]
        );
    }
    eprintln!("{report}");
    assert!(floor.is_some(), "{report}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn styles_viewport_and_grid_cost() {
    use super::tests::{median, spike_draws, spike_icons};
    use crate::renderer::Renderer;
    use pfx_gpu::Gpu;
    let spike = spike_icons();
    let curve = test_curve();
    let mut source = Vec::new();
    spike_draws(&spike, 0.0, Material::default(), 0, &mut source);
    source.retain(|draw| !matches!(draw.shape, Shape::Text(_)));
    let light = Srgba::hex(0xffffff);
    let dark = Srgba::hex(0x404850);
    let styled = |style: Style| {
        let mut draws = source.clone();
        restyle(&mut draws, Some(style));
        draws
    };
    let frame = Frame {
        size: [1128.0, 1968.0],
        bezel: 24.0,
        radius: 64.0,
        colour: Srgba::hex(0x22262c),
    };
    let mut viewport = Viewport::new([960.0, 540.0], frame).orientation(Orientation::Landscape);
    viewport.turn_to(Orientation::Portrait, 0.0, 1.0, Ease::Linear, true, false);
    let placement = viewport.at(0.5);
    let mut framed = source.clone();
    placement.place_all(&mut framed);
    framed.extend(placement.frame_draws(10.0, Some(Srgba::hex(0x9aa3ad)), 4000.0));
    let mut mirrored = source.clone();
    Viewport::new([960.0, 540.0], frame)
        .orientation(Orientation::Landscape)
        .mirror(Mirror::Vertical)
        .at(0.0)
        .place_all(&mut mirrored);
    let term = terminal(&source, 30);
    let variants: Vec<(&str, Vec<Draw>, bool)> = vec![
        ("no style", source.clone(), false),
        ("every shape outlined", styled(Style::outline(2.0)), false),
        (
            "every shape a ghost",
            styled(Style::ghost(2.0, 8.0, 6.0, 0.4)),
            false,
        ),
        (
            "every shape bevelled",
            styled(Style::bevel(2.0, light, dark)),
            false,
        ),
        ("every shape flat", styled(Style::Flat), false),
        ("in a frame, mid-turn", framed, false),
        ("mirrored", mirrored, false),
        ("character grid", term.draws.clone(), true),
    ];
    let mut report = String::new();
    for size in [[3840u32, 2160u32], [1280, 800]] {
        let mut turns = pfx_gpu::pace::Turns::default();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = turns.cpu(|| Renderer::new(gpu, size[0], size[1]).unwrap());
        let target = renderer
            .gpu()
            .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let flat_icons = FlatIcons::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            spike.icons.clone(),
        );
        let atlas = crate::text::GpuAtlas::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            &term.rich.atlas,
        )
        .unwrap();
        let text = [FlatText {
            atlas: &atlas,
            glyphs: Glyphs::Quads(&term.quads),
            colour: Srgba::WHITE,
        }];
        for (round, (variant, draws, with_text)) in
            std::iter::once(&variants[0]).chain(&variants).enumerate()
        {
            let mut samples: Vec<(f64, f64, f64)> = Vec::new();
            let mut instances = 0;
            for frame in 0..90u32 {
                let scene = FlatScene {
                    layout: [1920.0, 1080.0],
                    clear: Some(Srgba::hex(0xeceff3)),
                    curve: &curve,
                    light: Light::default(),
                    draws,
                    groups: &[],
                    text: if *with_text { &text } else { &[] },
                    icons: Some(&flat_icons),
                    sprites: None,
                    environment: None,
                    post: false,
                    frame,
                    seed: 0,
                };
                turns.poll();
                let timings = renderer.render_flat(&scene, &target.view).unwrap();
                instances = renderer.flat_pass().unwrap().instance_count();
                let of = |label: &str| {
                    timings
                        .iter()
                        .filter(|timing| timing.label == label)
                        .map(|timing| timing.milliseconds)
                        .sum::<f64>()
                };
                let colour = of("flat");
                let ids = of("flat ids");
                turns.add(colour + ids);
                if frame >= 10 && !timings.is_empty() {
                    samples.push((colour, ids, colour + ids));
                }
            }
            assert!(samples.len() >= 60, "GPU timestamps are unavailable");
            if round == 0 {
                continue;
            }
            let mut colour = samples.iter().map(|s| s.0).collect::<Vec<_>>();
            let mut ids = samples.iter().map(|s| s.1).collect::<Vec<_>>();
            let mut total = samples.iter().map(|s| s.2).collect::<Vec<_>>();
            let (colour, ids, total) = (median(&mut colour), median(&mut ids), median(&mut total));
            report += &format!(
                "{}x{} {variant}: flat {colour:.3} ms, flat ids {ids:.3} ms, total {total:.3} ms ({} draws, {instances} instances{})\n",
                size[0],
                size[1],
                draws.len(),
                if *with_text {
                    format!(
                        ", {} glyphs, {}x{} cells",
                        term.quads.len(),
                        term.cells[0],
                        term.cells[1]
                    )
                } else {
                    String::new()
                },
            );
        }
    }
    eprintln!("{report}");
}

#[test]
fn gradient_fills_take_every_style() {
    let fit = Fit::new([100.0, 100.0], [100, 100]).unwrap();
    let first = [0.2, 0.4, 0.6, 0.5];
    let strongest = [0.9, 0.1, 0.1, 1.0];
    let gradient =
        Gradient::linear([0.0, 0.0], [1.0, 0.0], &[(0.0, first), (1.0, strongest)]).unwrap();
    let draw = Draw::rect([50.0, 50.0], [40.0, 30.0], 0.0).elevation(1.0);
    let draw = Draw {
        fill: Fill::Gradient(gradient),
        ..draw
    };
    let outlined = style::resolve(&draw.style(Style::outline(2.0)));
    assert_eq!(outlined.fill, Fill::None);
    assert_eq!(outlined.stroke.unwrap().colour, Srgba(first));
    let bevelled = pack_shape(
        &draw.style(Style::bevel(2.0, Srgba::WHITE, Srgba::hex(0))),
        &fit,
        None,
    )
    .unwrap();
    assert_ne!(bevelled.info[0] & BEVEL, 0);
    assert_eq!(
        bevelled.info[0] & (FILL_SOLID | FILL_VERTICAL),
        FILL_GRADIENT
    );
    let grid = Grid::new([0.0, 0.0], [10.0, 10.0], 10, 10).unwrap();
    let mut chars = CharGrid::new(grid, Rules::default());
    chars.shape(&draw);
    assert_eq!(chars.cell(4, 4).unwrap().ground, Some(Srgba(strongest)));
    assert_eq!(chars.lines()[3], "   ┌──┐");
}
