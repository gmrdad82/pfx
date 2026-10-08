use super::sdf::{self, rect, rect_along};
use super::style::Bevel;
use super::tests::test_curve;
use super::*;
use crate::renderer::Renderer;
use pfx_gpu::{Gpu, wgpu};
use std::f32::consts::TAU;

const MIXED: [f32; 4] = [0.0, 8.0, 16.0, 4.0];

fn old_rect(p: [f32; 2], half: [f32; 2], radius: f32) -> [f32; 3] {
    let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
    let s = [sign(p[0]), sign(p[1])];
    let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
    if q[0] > 0.0 && q[1] > 0.0 {
        let l = (q[0] * q[0] + q[1] * q[1]).sqrt();
        return [l - radius, s[0] * q[0] / l, s[1] * q[1] / l];
    }
    if q[0] > q[1] {
        [q[0] - radius, s[0], 0.0]
    } else {
        [q[1] - radius, 0.0, s[1]]
    }
}

#[test]
fn equal_corners_are_the_single_radius_field() {
    for (half, radius) in [
        ([40.0, 20.0], 8.0),
        ([30.0, 30.0], 30.0),
        ([50.0, 9.0], 0.0),
    ] {
        for i in -30..=30 {
            for j in -20..=20 {
                let p = [i as f32 * 2.7, j as f32 * 2.3];
                assert_eq!(
                    rect(p, half, [radius; 4]),
                    old_rect(p, half, radius),
                    "{half:?} {radius} {p:?}"
                );
            }
        }
    }
}

#[test]
fn each_corner_has_its_own_distance() {
    let half = [40.0, 20.0];
    let near = |a: f32, b: f32| assert!((a - b).abs() < 1e-5, "{a} against {b}");
    near(rect([-43.0, -24.0], half, MIXED)[0], 5.0);
    near(rect([32.0 + 3.0, -12.0 - 4.0], half, MIXED)[0], 5.0 - 8.0);
    near(rect([24.0 + 3.0, 4.0 + 4.0], half, MIXED)[0], 5.0 - 16.0);
    near(rect([-36.0 - 3.0, 16.0 + 4.0], half, MIXED)[0], 5.0 - 4.0);
    near(rect([-40.0, -20.0], half, MIXED)[0], 0.0);
    assert!(rect([39.0, -19.0], half, MIXED)[0] > 0.0);
    assert!(rect([-39.0, -19.0], half, MIXED)[0] < 0.0);
    assert!(rect([39.0, 19.0], half, MIXED)[0] > 0.0);
    assert!(rect([-39.0, 19.0], half, MIXED)[0] > 0.0);
    assert!(rect([-39.5, 19.5], half, [0.0, 0.0, 0.0, 0.0])[0] < 0.0);
}

#[test]
fn the_field_gradient_is_a_unit_normal() {
    let half = [40.0, 20.0];
    for i in -30..=30 {
        for j in -20..=20 {
            let p = [i as f32 * 2.9 + 0.13, j as f32 * 2.1 + 0.07];
            let [_, gx, gy] = rect(p, half, MIXED);
            assert!(((gx * gx + gy * gy).sqrt() - 1.0).abs() < 1e-4, "{p:?}");
        }
    }
}

#[test]
fn corner_radii_clamp_to_the_smaller_half_extent() {
    let fit = Fit::new([200.0, 100.0], [200, 100]).unwrap();
    let draw = Draw::rect_corners([100.0, 50.0], [20.0, 60.0], [100.0, -4.0, 5.0, f32::MAX])
        .fill(Srgba::WHITE);
    let instance = pack_shape(&draw, &fit, None).unwrap();
    assert_eq!(instance.corners, [10.0, 0.0, 5.0, 10.0]);
    assert_eq!(
        Draw::rect([0.0, 0.0], [10.0, 10.0], 3.0).shape,
        Shape::rounded([5.0, 5.0], 3.0)
    );
    assert_eq!(
        Shape::corners([5.0, 5.0], [3.0; 4]),
        Shape::rounded([5.0, 5.0], 3.0)
    );
    let shadow = pack_shadow(&draw.elevation(1.0), &fit, &test_curve(), &Light::default()).unwrap();
    assert_eq!(shadow.corners, [10.0, 0.0, 5.0, 10.0]);
}

enum Piece {
    Line([f32; 2], [f32; 2]),
    Arc([f32; 2], f32, f32),
}

fn pieces(half: [f32; 2], r: [f32; 4]) -> Vec<(Piece, f32)> {
    let [tl, tr, br, bl] = r;
    let quarter = TAU * 0.25;
    let list = [
        Piece::Line([-half[0] + tl, -half[1]], [half[0] - tr, -half[1]]),
        Piece::Arc([half[0] - tr, -half[1] + tr], tr, -quarter),
        Piece::Line([half[0], -half[1] + tr], [half[0], half[1] - br]),
        Piece::Arc([half[0] - br, half[1] - br], br, 0.0),
        Piece::Line([half[0] - br, half[1]], [-half[0] + bl, half[1]]),
        Piece::Arc([-half[0] + bl, half[1] - bl], bl, quarter),
        Piece::Line([-half[0], half[1] - bl], [-half[0], -half[1] + tl]),
        Piece::Arc([-half[0] + tl, -half[1] + tl], tl, 2.0 * quarter),
    ];
    list.into_iter()
        .map(|piece| {
            let length = match &piece {
                Piece::Line(a, b) => ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(),
                Piece::Arc(_, radius, _) => radius * quarter,
            };
            (piece, length)
        })
        .collect()
}

#[test]
fn the_perimeter_runs_clockwise_through_every_corner() {
    let half = [50.0, 30.0];
    for radii in [
        MIXED,
        [10.0; 4],
        [0.0; 4],
        [24.0, 0.0, 12.0, 0.0],
        [0.0, 0.0, 0.0, 28.0],
    ] {
        let mut start = 0.0f32;
        let mut checked = 0;
        let total: f32 = pieces(half, radii).iter().map(|(_, length)| length).sum();
        for (piece, length) in pieces(half, radii) {
            if length > 1e-3 {
                for step in 1..40 {
                    let t = step as f32 / 40.0;
                    let point = match &piece {
                        Piece::Line(a, b) => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                        Piece::Arc(centre, radius, from) => {
                            let angle = from + t * TAU * 0.25;
                            [
                                centre[0] + radius * angle.cos(),
                                centre[1] + radius * angle.sin(),
                            ]
                        }
                    };
                    let along = rect_along(point, half, radii);
                    let want = start + length * t;
                    assert!(
                        (along.s - want).abs() < 2e-3 * total,
                        "{radii:?} at {point:?}: {} against {want}",
                        along.s
                    );
                    checked += 1;
                }
            }
            start += length;
        }
        assert!(checked > 100);
    }
}

#[test]
fn the_perimeter_tangent_follows_the_stroke() {
    let half = [50.0, 30.0];
    let at = |p: [f32; 2]| rect_along(p, half, MIXED).tangent;
    assert_eq!(at([0.0, -30.0]), [1.0, 0.0]);
    assert_eq!(at([50.0, 0.0]), [0.0, 1.0]);
    assert_eq!(at([0.0, 30.0]), [-1.0, 0.0]);
    assert_eq!(at([-50.0, 0.0]), [0.0, -1.0]);
}

fn flat<'a>(layout: [f32; 2], draws: &'a [Draw], curve: &'a ShadowCurve) -> FlatScene<'a> {
    FlatScene {
        layout,
        clear: None,
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

fn pixel(image: &[[f32; 4]], width: u32, x: u32, y: u32) -> [f32; 4] {
    image[(y * width + x) as usize]
}

const CORNERS: [f32; 4] = [0.0, 40.0, 0.0, 40.0];

fn corner_draw() -> Draw {
    Draw::rect_corners([100.0, 60.0], [160.0, 80.0], CORNERS)
        .fill(Srgba::hex(0xf0b429))
        .id(7)
        .shadow(Shadow::None)
}

#[test]
fn the_twin_draws_square_and_round_corners_together() {
    let curve = ShadowCurve::none();
    let draws = [corner_draw()];
    let image = cpu::render(&flat([200.0, 120.0], &draws, &curve), None, [200, 120]).unwrap();
    let inside = |x, y| pixel(&image, 200, x, y);
    assert!(inside(21, 21)[3] > 0.99);
    assert!(inside(177, 97)[3] > 0.99);
    assert!(inside(177, 22)[3] < 0.01, "the top right is cut round");
    assert!(inside(22, 97)[3] < 0.01, "the bottom left is cut round");
    assert!(inside(100, 60)[3] > 0.99);
}

#[test]
fn picking_follows_every_corner() {
    let curve = ShadowCurve::none();
    let draws = [corner_draw()];
    let scene = flat([200.0, 120.0], &draws, &curve);
    let pick = |x, y| cpu::pick(&scene, None, [200, 120], [x, y]).unwrap();
    assert_eq!(pick(21, 21), 7);
    assert_eq!(pick(177, 97), 7);
    assert_eq!(pick(177, 22), 0);
    assert_eq!(pick(22, 97), 0);
}

#[test]
fn outlines_and_patterns_honour_corners() {
    let curve = ShadowCurve::none();
    let draws = [Draw::rect_corners([100.0, 60.0], [160.0, 80.0], CORNERS)
        .fill(Srgba::hex(0x203040))
        .stroke(Stroke::solid(Srgba::hex(0xff0000), 4.0))
        .pattern(Pattern::new(
            PatternKind::Stripes,
            6.0,
            0.5,
            0.5,
            Srgba::WHITE,
        ))
        .shadow(Shadow::None)];
    let image = cpu::render(&flat([200.0, 120.0], &draws, &curve), None, [200, 120]).unwrap();
    let at = |x, y| pixel(&image, 200, x, y);
    let red = |p: [f32; 4]| p[0] > 0.9 && p[1] < 0.1 && p[3] > 0.99;
    assert!(red(at(21, 21)), "{:?}", at(21, 21));
    assert!(red(at(178, 98)), "{:?}", at(178, 98));
    assert!(!red(at(178, 22)), "the stroke follows the round corner in");
    assert!(at(179, 21)[3] < 0.01);
    assert!(at(21, 98)[3] < 0.01);
    assert!(red(at(60, 21)));
}

#[test]
fn bevels_light_square_and_round_corners_by_their_own_shape() {
    let curve = ShadowCurve::none();
    let draws = [Draw::rect_corners([100.0, 60.0], [160.0, 80.0], CORNERS)
        .fill(Srgba::hex(0x808080))
        .style(Style::Bevel(Bevel::new(
            6.0,
            Srgba::WHITE,
            Srgba::hex(0x000000),
        )))
        .shadow(Shadow::None)];
    let image = cpu::render(&flat([200.0, 120.0], &draws, &curve), None, [200, 120]).unwrap();
    let at = |x, y| pixel(&image, 200, x, y);
    assert!(at(21, 21)[0] > 0.95, "{:?}", at(21, 21));
    assert!(at(178, 98)[0] < 0.05, "{:?}", at(178, 98));
    assert!(at(177, 22)[3] < 0.01);
    assert!((at(100, 60)[0] - 128.0 / 255.0).abs() < 1e-3);
    let arc = at(
        (160.0 - 40.0 + 40.0 * std::f32::consts::FRAC_1_SQRT_2 + 20.0 - 2.0) as u32,
        (20.0 + 40.0 - 40.0 * std::f32::consts::FRAC_1_SQRT_2 + 2.0) as u32,
    );
    assert!(arc[3] > 0.99 && arc[0] > 0.02 && arc[0] < 0.98, "{arc:?}");
}

fn shadow_alpha(radii: [f32; 4]) -> Vec<[f32; 4]> {
    let curve = test_curve();
    let draws = [Draw::new(Shape::corners([40.0, 30.0], radii))
        .at([100.0, 60.0])
        .fill(Srgba::WHITE)
        .elevation(2.0)];
    let mut scene = flat([200.0, 120.0], &draws, &curve);
    scene.clear = Some(Srgba::WHITE);
    cpu::render(&scene, None, [200, 120]).unwrap()
}

#[test]
fn shadows_hug_square_corners_and_round_ones() {
    let square = shadow_alpha([0.0; 4]);
    let round = shadow_alpha([30.0; 4]);
    let mixed = shadow_alpha([0.0, 30.0, 0.0, 30.0]);
    let dark = |image: &[[f32; 4]], x, y| 1.0 - pixel(image, 200, x, y)[0];
    let (x, y) = (146, 98);
    assert!(
        dark(&square, x, y) > dark(&round, x, y) * 1.3,
        "{} {}",
        dark(&square, x, y),
        dark(&round, x, y)
    );
    assert!((dark(&mixed, x, y) - dark(&square, x, y)).abs() < 1e-3);
    let (x, y) = (146, 40);
    assert!(
        dark(&mixed, x, y) < dark(&square, x, y) - 0.005,
        "{} {}",
        dark(&mixed, x, y),
        dark(&square, x, y)
    );
    assert!((dark(&mixed, x, y) - dark(&round, x, y)).abs() < 1e-3);
}

struct Rig {
    renderer: Renderer,
    target: pfx_gpu::OffscreenTarget,
}

impl Rig {
    fn new(size: [u32; 2]) -> Self {
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let renderer = Renderer::new(gpu, size[0], size[1]).unwrap();
        let target = renderer
            .gpu()
            .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        Self { renderer, target }
    }

    fn render(&mut self, scene: &FlatScene<'_>) -> Vec<[f32; 4]> {
        self.renderer.render_flat(scene, &self.target.view).unwrap();
        self.renderer
            .gpu()
            .readback_rgba16(&self.target)
            .unwrap()
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|i| half::f16::from_bits(p[i]).to_f32()))
            .collect()
    }
}

fn corner_sheet() -> Vec<Draw> {
    let ink = Srgba::hex(0xf0f4f8);
    vec![
        Draw::rect_corners([90.0, 70.0], [140.0, 80.0], [0.0, 30.0, 12.0, 4.0])
            .fill(Srgba::hex(0x3d7ea6))
            .stroke(Stroke::solid(ink, 4.0))
            .elevation(2.0)
            .id(1),
        Draw::rect_corners([250.0, 70.0], [140.0, 80.0], [28.0, 0.0, 28.0, 0.0])
            .stroke(Stroke::dashed(ink, 3.0, 11.0, 7.0))
            .id(2),
        Draw::rect_corners([410.0, 70.0], [140.0, 80.0], [0.0, 36.0, 0.0, 36.0])
            .fill(Srgba::hex(0x30363f))
            .pattern(Pattern::new(
                PatternKind::Chevrons,
                8.0,
                0.4,
                0.4,
                Srgba::hex(0xc97b4a),
            ))
            .elevation(1.0)
            .id(3),
        Draw::rect_corners([90.0, 190.0], [140.0, 80.0], [20.0, 0.0, 0.0, 20.0])
            .fill(Srgba::hex(0x808080))
            .style(Style::Bevel(Bevel::new(
                8.0,
                Srgba::WHITE,
                Srgba::hex(0x101010),
            )))
            .id(4),
        Draw::rect_corners([250.0, 190.0], [140.0, 80.0], [0.0, 40.0, 10.0, 40.0])
            .fill(Srgba::hex(0xc97b4a))
            .blur([18.0, 6.0])
            .id(5),
        Draw::rect_corners([410.0, 190.0], [140.0, 80.0], [0.0, 30.0, 0.0, 30.0])
            .stroke(Stroke::solid(Srgba::hex(0x6fb3d9), 6.0))
            .style(Style::ghost(4.0, 10.0, 6.0, 1.0))
            .id(6),
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn per_corner_rects_match_their_twin_on_the_gpu() {
    let curve = test_curve();
    let draws = corner_sheet();
    let scene = flat([500.0, 260.0], &draws, &curve);
    for size in [[500u32, 260u32], [1000, 520]] {
        let mut rig = Rig::new(size);
        let gpu = rig.render(&scene);
        let twin = cpu::render(&scene, None, size).unwrap();
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut total = 0u64;
        let mut worst = (0u8, 0usize);
        for (index, (a, b)) in gpu.iter().zip(&twin).enumerate() {
            for c in 0..3 {
                let d = byte(a[c]).abs_diff(byte(b[c]));
                total += u64::from(d);
                if d > worst.0 {
                    worst = (d, index);
                }
            }
        }
        let mean = total as f64 / (gpu.len() * 3) as f64;
        assert!(
            mean < 0.2 && worst.0 <= 6,
            "{size:?}: mean {mean:.4} worst {} at ({}, {})",
            worst.0,
            worst.1 as u32 % size[0],
            worst.1 as u32 / size[0]
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_lit_slab_keeps_square_and_round_corners_on_the_gpu() {
    let curve = ShadowCurve::none();
    let draws = [Draw::rect_corners([100.0, 60.0], [160.0, 80.0], CORNERS)
        .fill(Srgba::hex(0xf0b429))
        .material(Material {
            thickness: 6.0,
            bevel: 2.0,
            gloss: 0.4,
            roughness: 0.4,
            reflection: 0.0,
        })
        .shadow(Shadow::None)];
    let scene = flat([200.0, 120.0], &draws, &curve);
    let mut rig = Rig::new([200, 120]);
    let image = rig.render(&scene);
    let at = |x, y| pixel(&image, 200, x, y);
    assert!(at(21, 21)[3] > 0.95, "{:?}", at(21, 21));
    assert!(at(177, 97)[3] > 0.95, "{:?}", at(177, 97));
    assert!(at(177, 22)[3] < 0.05, "{:?}", at(177, 22));
    assert!(at(22, 97)[3] < 0.05, "{:?}", at(22, 97));
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn equal_corners_render_the_bytes_of_one_radius() {
    let curve = test_curve();
    let uniform = vec![
        Draw::rect([100.0, 60.0], [160.0, 80.0], 18.0)
            .fill(Srgba::hex(0xf0b429))
            .stroke(Stroke::dashed(Srgba::WHITE, 3.0, 9.0, 5.0))
            .elevation(1.0),
    ];
    let spelled = vec![
        Draw::rect_corners([100.0, 60.0], [160.0, 80.0], [18.0; 4])
            .fill(Srgba::hex(0xf0b429))
            .stroke(Stroke::dashed(Srgba::WHITE, 3.0, 9.0, 5.0))
            .elevation(1.0),
    ];
    let mut rig = Rig::new([200, 120]);
    let a = rig.render(&flat([200.0, 120.0], &uniform, &curve));
    let b = rig.render(&flat([200.0, 120.0], &spelled, &curve));
    assert!(a == b);
    assert!(sdf::corner([1.0, 1.0], [1.0, 2.0, 3.0, 4.0]) == 3.0);
}
