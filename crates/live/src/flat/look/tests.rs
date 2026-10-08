use super::super::tests::test_curve;
use super::super::{
    Draw, FILL_GRADIENT, Fill, FlatScene, FlatText, Glyphs, Light, Pattern, PatternKind, Roles,
    Shadow, Shape, Srgba, Stroke, TokenKey, cpu, pack_shape, translation,
};
use super::*;
use crate::renderer::Renderer;
use pfx_gpu::Gpu;

const LAYOUT: [f32; 2] = [1920.0, 1080.0];

fn board_tokens() -> TokenSet {
    TokenSet::new()
        .with("board", Srgba::hex(0xe4e7eb))
        .with("card", Srgba::hex(0xfafbfc))
        .with("ink", Srgba::hex(0x30363f))
        .with("accent", Srgba::hex(0x3d7ea6))
        .with("warm", Srgba::hex(0xc97b4a))
}

fn dark_tokens() -> TokenSet {
    TokenSet::new()
        .with("board", Srgba::hex(0x15181d))
        .with("card", Srgba::hex(0x252a31))
        .with("ink", Srgba::hex(0xd8dde3))
        .with("accent", Srgba::hex(0x6fb3d9))
}

fn sky() -> Gradient {
    Gradient::linear(
        [0.0, 0.0],
        [0.0, 1080.0],
        &[
            (0.0, Srgba::hex(0x1d2240).0),
            (0.55, Srgba::hex(0x7a4f86).0),
            (1.0, Srgba::hex(0xe7a96b).0),
        ],
    )
    .unwrap()
    .space(Interpolation::Oklab)
}

fn board_draws() -> Vec<Draw> {
    let tokens = board_tokens();
    let token = |key: &str| tokens.get(key).unwrap();
    let mut draws = vec![
        Draw::rect([960.0, 540.0], [1800.0, 960.0], 24.0)
            .fill(token("board"))
            .id(1),
    ];
    for column in 0..5 {
        for row in 0..3 {
            let centre = [300.0 + column as f32 * 330.0, 260.0 + row as f32 * 250.0];
            draws.push(
                Draw::rect(centre, [260.0, 180.0], 18.0)
                    .fill(token("card"))
                    .stroke(Stroke::solid(token("ink").alpha(0.25), 2.0))
                    .elevation(1.0)
                    .id(10 + column * 3 + row),
            );
            draws.push(
                Draw::circle([centre[0] - 70.0, centre[1] - 30.0], 26.0)
                    .fill(if (column + row) % 2 == 0 {
                        token("accent")
                    } else {
                        token("warm")
                    })
                    .elevation(1.0),
            );
            draws.push(
                Draw::rect([centre[0] + 30.0, centre[1] + 45.0], [150.0, 22.0], 11.0)
                    .fill(token("ink").alpha(0.6))
                    .elevation(1.0),
            );
        }
    }
    let pill = Gradient::linear(
        [-200.0, 0.0],
        [200.0, 0.0],
        &[
            (0.0, token("accent").0),
            (0.5, Srgba::hex(0xf1f3f5).0),
            (1.0, token("warm").0),
        ],
    )
    .unwrap();
    draws.push(
        Draw::rect([960.0, 1000.0], [400.0, 48.0], 24.0)
            .elevation(2.0)
            .id(99),
    );
    draws.last_mut().unwrap().fill = Fill::Gradient(pill);
    draws.push(
        Draw::new(Shape::Ring {
            radius: 60.0,
            width: 10.0,
        })
        .at([1760.0, 140.0])
        .fill(token("accent"))
        .pattern(Pattern::new(
            PatternKind::Stripes,
            8.0,
            0.8,
            0.4,
            Srgba::WHITE.alpha(0.5),
        )),
    );
    draws
}

fn scene<'a>(draws: &'a [Draw], curve: &'a ShadowCurve, text: &'a [FlatText<'a>]) -> FlatScene<'a> {
    FlatScene {
        layout: LAYOUT,
        clear: Some(Srgba::hex(0xc9ced4)),
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

fn retro_palette() -> Palette {
    Palette::hex(&[
        0x0e1013, 0x262b33, 0x47505c, 0x707b88, 0x9aa5b1, 0xc9ced4, 0xe4e7eb, 0xfafbfc, 0x30363f,
        0x3d7ea6, 0x6fb3d9, 0xc97b4a, 0xe3a77f, 0x5b8c4a, 0x8a5a9e, 0xd9c35b,
    ])
    .unwrap()
}

fn retro(text: LowText, upscale: Upscale) -> Look {
    Look::new()
        .with(LookPass::LowRes(
            LowRes::new([480, 270]).upscale(upscale).text(text),
        ))
        .with(LookPass::Quantise(
            Quantise::new(
                retro_palette(),
                Dither::Bayer {
                    order: 4,
                    spread: 0.09,
                },
            )
            .unwrap(),
        ))
}

fn terminal() -> Look {
    Look::new().with(LookPass::Crt(Crt {
        scanlines: Some(Scanlines {
            period: 4.0,
            strength: 0.35,
        }),
        glow: Some(Glow {
            threshold: 0.55,
            radius: 6.0,
            strength: 0.7,
            tint: Srgba::hex(0x7cf2a0).0,
        }),
        curvature: Some(Curvature {
            amount: 0.06,
            corner: 0.03,
            bezel: [0.0, 0.0, 0.0, 1.0],
        }),
        aberration: Some(Aberration { shift: 1.0 }),
    }))
}

fn pool() -> Vignette {
    Vignette {
        centre: [960.0, 470.0],
        radius: [620.0, 380.0],
        falloff: 0.9,
        floor: 0.2,
        colour: [0.02, 0.02, 0.05, 1.0],
    }
}

fn sky_look() -> Look {
    Look::new()
        .with(LookPass::Backdrop(Backdrop {
            colour: None,
            gradient: Some(sky()),
            grid: Some(
                Grid::new(40.0, 1.0, Srgba::hex(0xffffff).alpha(0.2).0).major(
                    5,
                    2.0,
                    Srgba::hex(0xffffff).alpha(0.4).0,
                ),
            ),
        }))
        .with(LookPass::Tokens(dark_tokens()))
}

#[test]
fn token_sets_mix_in_oklab_and_keep_their_ends() {
    let light = board_tokens();
    let dark = dark_tokens();
    assert_eq!(light.mix(&dark, 0.0), light);
    let end = light.mix(&dark, 1.0);
    for (key, colour) in dark.iter() {
        assert_eq!(end.get(key), Some(colour));
    }
    assert_eq!(end.get("warm"), light.get("warm"));
    let mid = light.mix(&dark, 0.5).get("board").unwrap();
    let a = pfx_post::look::oklab([0.894, 0.906, 0.922]);
    let b = pfx_post::look::oklab([0.082, 0.094, 0.114]);
    let m = pfx_post::look::oklab([mid.0[0], mid.0[1], mid.0[2]]);
    assert!((m[0] - (a[0] + b[0]) * 0.5).abs() < 2e-3, "{m:?}");
    let collected: TokenSet = [("x", Srgba::WHITE)].into_iter().collect();
    assert_eq!(collected.len(), 1);
}

#[test]
fn recolouring_swaps_tokens_by_colour_and_keeps_alpha() {
    let recolour = Recolour::new(&board_tokens(), &dark_tokens());
    assert!(!recolour.is_identity());
    assert!(Recolour::new(&board_tokens(), &board_tokens()).is_identity());
    assert_eq!(recolour.colour(Srgba::hex(0xe4e7eb)), Srgba::hex(0x15181d));
    let faded = recolour.colour(Srgba::hex(0x30363f).alpha(0.6));
    assert_eq!(faded, Srgba::hex(0xd8dde3).alpha(0.6));
    assert_eq!(recolour.colour(Srgba::hex(0xc97b4a)), Srgba::hex(0xc97b4a));
    assert_eq!(recolour.colour(Srgba::hex(0x123456)), Srgba::hex(0x123456));
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let copy = SceneCopy::recoloured(&base, &recolour);
    assert_eq!(copy.draws[0].fill, Fill::Solid(Srgba::hex(0x15181d)));
    assert_eq!(
        copy.draws[1].stroke.unwrap().colour,
        Srgba::hex(0xd8dde3).alpha(0.25)
    );
    let Fill::Gradient(pill) = copy.draws[draws.len() - 2].fill else {
        panic!("the pill keeps its gradient");
    };
    assert_eq!(pill.stops().next().unwrap().1, Srgba::hex(0x6fb3d9).0);
    assert_eq!(copy.clear, Some(Srgba::hex(0xc9ced4)));
}

#[test]
fn easings_meet_their_ends() {
    for easing in [
        Easing::Linear,
        Easing::In,
        Easing::Out,
        Easing::InOut,
        Easing::Steps(4),
    ] {
        assert_eq!(easing.at(0.0), 0.0, "{easing:?}");
        assert_eq!(easing.at(1.0), 1.0, "{easing:?}");
        let mut last = 0.0;
        for i in 0..=100 {
            let v = easing.at(i as f32 / 100.0);
            assert!(v >= last, "{easing:?} is not monotone");
            last = v;
        }
    }
    assert_eq!(Easing::Steps(4).at(0.3), 0.25);
    assert!((Easing::InOut.at(0.5) - 0.5).abs() < 1e-6);
}

#[test]
fn transitions_are_deterministic_from_game_time() {
    let run = || {
        let mut stack = LookStack::new(Look::new()).unwrap().base(board_tokens());
        stack
            .switch(
                retro(LowText::Pixelated, Upscale::Integer),
                10.0,
                Transition::wipe(0.8, Easing::InOut, 0.0, 60.0),
            )
            .unwrap();
        [9.0, 10.0, 10.2, 10.4, 10.79, 10.8, 30.0].map(|t| {
            let frame = stack.frame(t);
            (frame.progress.to_bits(), frame.settled(), frame.blend)
        })
    };
    let first = run();
    assert_eq!(first, run());
    assert_eq!(first[0].0, 0.0f32.to_bits());
    assert!(!first[2].1);
    assert_eq!(f32::from_bits(first[3].0), Easing::InOut.at(0.5));
    assert!(first[5].1 && first[6].1);
    let stack = LookStack::new(terminal()).unwrap();
    assert!(stack.frame(0.0).settled());
    assert_eq!(stack.frame(5.0).to, &terminal());
}

#[test]
fn reduced_motion_shortens_and_crossfades() {
    let mut stack = LookStack::new(Look::new())
        .unwrap()
        .motion(Motion::Reduced { longest: 0.15 });
    stack
        .switch(
            terminal(),
            2.0,
            Transition::wipe(1.2, Easing::In, 1.0, 30.0),
        )
        .unwrap();
    let frame = stack.frame(2.075);
    assert_eq!(frame.blend, Blend::Crossfade);
    assert!((frame.progress - 0.5).abs() < 1e-4, "{}", frame.progress);
    assert!(stack.frame(2.16).settled());
    let mut instant = LookStack::new(Look::new())
        .unwrap()
        .motion(Motion::Reduced { longest: 0.0 });
    instant
        .switch(terminal(), 2.0, Transition::crossfade(1.0, Easing::Linear))
        .unwrap();
    assert!(instant.frame(2.0).settled());
    assert_eq!(instant.frame(2.0).to, &terminal());
}

#[test]
fn switching_mid_transition_starts_from_the_look_shown() {
    let a = Look::new();
    let b = terminal();
    let c = sky_look();
    let mut stack = LookStack::new(a.clone()).unwrap();
    let fade = Transition::crossfade(1.0, Easing::Linear);
    stack.switch(b.clone(), 0.0, fade).unwrap();
    stack.switch(c.clone(), 0.3, fade).unwrap();
    let frame = stack.frame(0.3);
    assert_eq!((frame.from, frame.to), (&a, &c));
    stack.switch(b.clone(), 0.9, fade).unwrap();
    let frame = stack.frame(0.9);
    assert_eq!((frame.from, frame.to), (&c, &b));
    assert!(stack.switch(c, f64::NAN, fade).is_err());
}

#[test]
fn looks_validate_their_passes() {
    assert!(Look::new().validate().is_ok());
    assert!(retro(LowText::Crisp, Upscale::Fit).validate().is_ok());
    let twice = Look::new()
        .with(LookPass::LowRes(LowRes::new([10, 10])))
        .with(LookPass::LowRes(LowRes::new([20, 20])));
    assert!(twice.validate().is_err());
    assert!(
        Look::new()
            .with(LookPass::LowRes(LowRes::new([0, 10])))
            .validate()
            .is_err()
    );
    assert!(LookStack::new(twice).is_err());
    assert!(LookFrame::still(&Look::new(), &TokenSet::new()).is_identity());
    assert!(!LookFrame::still(&terminal(), &TokenSet::new()).is_identity());
}

#[test]
fn gradient_fills_pack_and_render_on_the_cpu_twin() {
    let gradient = Gradient::radial(
        [0.0, 0.0],
        [100.0, 50.0],
        &[(0.0, Srgba::WHITE.0), (1.0, Srgba::hex(0x204060).0)],
    )
    .unwrap();
    let mut draw =
        Draw::rect([150.0, 100.0], [200.0, 100.0], 0.0).shadow(super::super::Shadow::None);
    draw.fill = Fill::Gradient(gradient);
    let fit = Fit::new([300.0, 200.0], [300, 200]).unwrap();
    let instance = pack_shape(&draw, &fit, None).unwrap();
    assert_eq!(instance.info[0] & FILL_GRADIENT, FILL_GRADIENT);
    assert_eq!(instance.fill_top, [0.0, 0.0, 100.0, 50.0]);
    assert_eq!(instance.fill_bottom, [0.0, 2.0, 1.0, 1.0]);
    let draws = [draw];
    let curve = ShadowCurve::none();
    let scene = FlatScene {
        layout: [300.0, 200.0],
        clear: None,
        curve: &curve,
        light: Light::default(),
        draws: &draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let image = cpu::render(&scene, None, [300, 200]).unwrap();
    for (x, y) in [(150u32, 100u32), (180, 110), (100, 70), (240, 130)] {
        let local = [x as f32 + 0.5 - 150.0, y as f32 + 0.5 - 100.0];
        let want = gradient.paint(local);
        let got = image[(y * 300 + x) as usize];
        for c in 0..4 {
            assert!(
                (got[c] - want[c]).abs() < 1e-5,
                "({x}, {y}) {got:?} {want:?}"
            );
        }
    }
    assert_eq!(image[0], [0.0; 4]);
}

#[test]
fn crisp_text_lands_where_the_upscaled_board_puts_it() {
    for (target, mode) in [
        ([3840u32, 2160u32], Upscale::Integer),
        ([1280, 800], Upscale::Integer),
        ([1280, 800], Upscale::Fit),
    ] {
        let fit = Fit::new(LAYOUT, target).unwrap();
        let low_fit = Fit::new(LAYOUT, [480, 270]).unwrap();
        let map = UpscaleMap::new([480, 270], target, mode).unwrap();
        let k = map.scale();
        let axes = [
            low_fit.scale * k[0] / fit.scale,
            low_fit.scale * k[1] / fit.scale,
        ];
        let shift = [
            (map.offset[0] as f32 + low_fit.offset[0] * k[0] - fit.offset[0]) / fit.scale,
            (map.offset[1] as f32 + low_fit.offset[1] * k[1] - fit.offset[1]) / fit.scale,
        ];
        for point in [
            [0.0f32, 0.0],
            [960.0, 540.0],
            [1920.0, 1080.0],
            [333.0, 777.0],
        ] {
            let low = low_fit.to_target(point);
            let board = [
                map.offset[0] as f32 + low[0] * k[0],
                map.offset[1] as f32 + low[1] * k[1],
            ];
            let text =
                fit.to_target([point[0] * axes[0] + shift[0], point[1] * axes[1] + shift[1]]);
            assert!(
                (board[0] - text[0]).abs() < 1e-2 && (board[1] - text[1]).abs() < 1e-2,
                "{target:?} {mode:?} {point:?}: {board:?} against {text:?}"
            );
        }
    }
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

    fn pixels(&self) -> Vec<[u16; 4]> {
        self.renderer
            .gpu()
            .readback_rgba16(&self.target)
            .unwrap()
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect()
    }

    fn plain(&mut self, scene: &FlatScene<'_>) -> Vec<[u16; 4]> {
        self.renderer.render_flat(scene, &self.target.view).unwrap();
        self.pixels()
    }

    fn look(&mut self, scene: &FlatScene<'_>, frame: &LookFrame<'_>) -> Vec<[u16; 4]> {
        self.renderer
            .render_flat_look(scene, frame, &self.target.view)
            .unwrap();
        self.pixels()
    }
}

fn f32s(pixels: &[[u16; 4]]) -> Vec<[f32; 4]> {
    pixels
        .iter()
        .map(|p| p.map(|v| half::f16::from_bits(v).to_f32()))
        .collect()
}

fn byte(p: [f32; 4]) -> [u8; 3] {
    std::array::from_fn(|i| (p[i].clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn no_look_frames_are_byte_identical() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let identity = Look::new();
    let tokens = board_tokens();
    for size in [[1920u32, 1080u32], [1280, 800]] {
        let mut rig = turns.cpu(|| Rig::new(size));
        let plain = turns.cpu(|| rig.plain(&base));
        let still = turns.cpu(|| rig.look(&base, &LookFrame::still(&identity, &tokens)));
        assert!(
            plain == still,
            "{size:?}: an identity look changed the frame"
        );
        assert_eq!(rig.renderer.last_pass_order(), ["flat", "flat ids"]);
        let again = turns.cpu(|| rig.plain(&base));
        assert!(plain == again);
        let mut fresh = turns.cpu(|| Rig::new(size));
        assert!(plain == turns.cpu(|| fresh.plain(&base)));
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn gradient_fills_match_the_cpu_twin_on_the_gpu() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    for size in [[1920u32, 1080u32], [1280, 800]] {
        let mut rig = turns.cpu(|| Rig::new(size));
        let gpu = f32s(&turns.cpu(|| rig.plain(&base)));
        let twin = turns.cpu(|| cpu::render(&base, None, size)).unwrap();
        let mut total = 0u64;
        let mut worst = 0u8;
        for (a, b) in gpu.iter().zip(&twin) {
            let (a, b) = (byte(*a), byte(*b));
            for c in 0..3 {
                let d = a[c].abs_diff(b[c]);
                total += u64::from(d);
                worst = worst.max(d);
            }
        }
        let mean = total as f64 / (gpu.len() * 3) as f64;
        assert!(
            mean < 0.15 && worst <= 2,
            "{size:?}: mean {mean:.4} worst {worst}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn low_resolution_upscales_pixel_exactly() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let mut low = turns.cpu(|| Rig::new([480, 270]));
    let small = turns.cpu(|| low.plain(&base));
    let tokens = TokenSet::new();
    for (size, mode) in [
        ([1920u32, 1080u32], Upscale::Integer),
        ([1280, 800], Upscale::Integer),
        ([1280, 800], Upscale::Fit),
        ([3840, 2160], Upscale::Integer),
    ] {
        let look = Look::new().with(LookPass::LowRes(LowRes::new([480, 270]).upscale(mode)));
        let mut rig = turns.cpu(|| Rig::new(size));
        let out = turns.cpu(|| rig.look(&base, &LookFrame::still(&look, &tokens)));
        let map = UpscaleMap::new([480, 270], size, mode).unwrap();
        let clear = base.clear.unwrap().premultiplied();
        let clear_bits = clear.map(|v| half::f16::from_f32(v).to_bits());
        for y in 0..size[1] {
            turns.poll();
            for x in 0..size[0] {
                let got = out[(y * size[0] + x) as usize];
                match map.source_pixel([x, y]) {
                    Some([sx, sy]) => assert_eq!(
                        got,
                        small[(sy * 480 + sx) as usize],
                        "{size:?} {mode:?} ({x}, {y})"
                    ),
                    None => {
                        for c in 0..4 {
                            let d = (half::f16::from_bits(got[c]).to_f32()
                                - half::f16::from_bits(clear_bits[c]).to_f32())
                            .abs();
                            assert!(d < 1e-3, "{size:?} letterbox ({x}, {y})");
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn quantised_looks_hold_their_palette() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let palette: Vec<[u8; 3]> = retro_palette()
        .colours()
        .iter()
        .map(|c| byte([c[0], c[1], c[2], 1.0]))
        .collect();
    let tokens = TokenSet::new();
    for (size, mode) in [
        ([1920u32, 1080u32], Upscale::Integer),
        ([1280, 800], Upscale::Fit),
    ] {
        let look = retro(LowText::Pixelated, mode);
        let mut rig = turns.cpu(|| Rig::new(size));
        let out = f32s(&turns.cpu(|| rig.look(&base, &LookFrame::still(&look, &tokens))));
        let map = UpscaleMap::new([480, 270], size, mode).unwrap();
        let mut colours = std::collections::BTreeSet::new();
        for (i, p) in out.iter().enumerate() {
            let at = [i as u32 % size[0], i as u32 / size[0]];
            if map.source_pixel(at).is_none() {
                continue;
            }
            let b = byte(*p);
            assert!(
                palette.contains(&b),
                "{size:?} {at:?} {b:?} is not in the palette"
            );
            colours.insert(b);
        }
        assert!(colours.len() >= 6, "{} colours", colours.len());
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn transitions_render_deterministically() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let size = [1280u32, 800u32];
    let mut stack = LookStack::new(sky_look()).unwrap().base(board_tokens());
    let from = sky_look();
    let to = retro(LowText::Pixelated, Upscale::Fit).with(LookPass::Crt(Crt {
        scanlines: Some(Scanlines {
            period: 4.0,
            strength: 0.3,
        }),
        ..Crt::default()
    }));
    stack
        .switch(
            to.clone(),
            4.0,
            Transition::wipe(1.0, Easing::InOut, 0.4, 80.0),
        )
        .unwrap();
    let tokens = board_tokens();
    let mut rig = turns.cpu(|| Rig::new(size));
    let start = turns.cpu(|| rig.look(&base, &stack.frame(4.0)));
    let from_still = turns.cpu(|| rig.look(&base, &LookFrame::still(&from, &tokens)));
    assert!(start == from_still, "progress 0 is the look switched from");
    let end = turns.cpu(|| rig.look(&base, &stack.frame(5.0)));
    let to_still = turns.cpu(|| rig.look(&base, &LookFrame::still(&to, &tokens)));
    assert!(end == to_still, "progress 1 is the look switched to");
    let middle = turns.cpu(|| rig.look(&base, &stack.frame(4.5)));
    assert!(middle != start && middle != end);
    let mut other = turns.cpu(|| Rig::new(size));
    assert!(middle == turns.cpu(|| other.look(&base, &stack.frame(4.5))));
    assert!(middle == turns.cpu(|| rig.look(&base, &stack.frame(4.5))));
    let mut fade = LookStack::new(Look::new()).unwrap().base(board_tokens());
    fade.switch(
        Look::new().with(LookPass::Tokens(dark_tokens())),
        0.0,
        Transition::crossfade(1.0, Easing::Linear),
    )
    .unwrap();
    let half_way = f32s(&turns.cpu(|| rig.look(&base, &fade.frame(0.5))));
    let fit = Fit::new(LAYOUT, size).unwrap();
    let at = fit.to_target([795.0, 385.0]);
    let pixel = half_way[(at[1] as u32 * size[0] + at[0] as u32) as usize];
    let want = board_tokens()
        .mix(&dark_tokens(), 0.5)
        .get("board")
        .unwrap();
    for c in 0..3 {
        assert!(
            (pixel[c] - want.0[c]).abs() < 1.5 / 255.0,
            "{pixel:?} {want:?}"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn crisp_text_stays_sharp_over_a_low_resolution_board() {
    let mut turns = pfx_gpu::pace::Turns::default();
    const SANS: &[u8] = pfx_text::fixture::DM_SANS;
    let mut engine = pfx_text::TextEngine::new(SANS).unwrap();
    let rich = engine
        .render_spans(
            &[pfx_text::Span {
                text: "Sixty one".into(),
                face: pfx_text::Face {
                    family: "DM Sans".into(),
                    size: 44.0,
                    line: 55.0,
                    weight: 600,
                    italic: false,
                    spacing: 0.0,
                },
                color: [1.0; 4],
            }],
            None,
            1.0,
            pfx_text::Anchor::Start,
            pfx_text::Representation::Msdf,
            [0.0, 0.0],
            1.0,
            [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
            [0.0; 3],
        )
        .unwrap();
    let quads = crate::text::rich_quads(&rich);
    let size = [1920u32, 1080u32];
    let mut rig = turns.cpu(|| Rig::new(size));
    let atlas = crate::text::GpuAtlas::new(
        &rig.renderer.gpu().device,
        &rig.renderer.gpu().queue,
        &rich.atlas,
    )
    .unwrap();
    let text = [FlatText {
        atlas: &atlas,
        glyphs: Glyphs::Quads(&quads),
        colour: Srgba::hex(0x30363f),
    }];
    let mut draws = board_draws();
    draws.push(
        Draw::new(Shape::Text(0))
            .transform(translation(820.0, 520.0))
            .elevation(3.0),
    );
    let curve = test_curve();
    let base = scene(&draws, &curve, &text);
    let tokens = TokenSet::new();
    let low = |text| Look::new().with(LookPass::LowRes(LowRes::new([480, 270]).text(text)));
    let pixelated =
        turns.cpu(|| rig.look(&base, &LookFrame::still(&low(LowText::Pixelated), &tokens)));
    let crisp = turns.cpu(|| rig.look(&base, &LookFrame::still(&low(LowText::Crisp), &tokens)));
    let blocky = |pixels: &[[u16; 4]], x0: u32, y0: u32, x1: u32, y1: u32| {
        let mut broken = 0;
        for y in (y0..y1).step_by(4) {
            for x in (x0..x1).step_by(4) {
                let first = pixels[(y * size[0] + x) as usize];
                if (0..4).any(|dy| {
                    (0..4).any(|dx| pixels[((y + dy) * size[0] + x + dx) as usize] != first)
                }) {
                    broken += 1;
                }
            }
        }
        broken
    };
    assert_eq!(blocky(&pixelated, 800, 480, 1120, 600), 0);
    assert!(blocky(&crisp, 800, 480, 1120, 600) > 20);
    assert_eq!(blocky(&crisp, 0, 0, 640, 400), 0);
    assert!(pixelated[..(400 * size[0]) as usize] == crisp[..(400 * size[0]) as usize]);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn grouped_scenes_render_through_the_renderer_with_timings() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let mut draws = board_draws();
    for draw in draws.iter_mut().skip(1).take(6) {
        draw.group = Some(0);
    }
    let groups = [super::super::Group { opacity: 0.5 }];
    let curve = test_curve();
    let mut base = scene(&draws, &curve, &[]);
    base.groups = &groups;
    let mut rig = turns.cpu(|| Rig::new([640, 360]));
    let tokens = TokenSet::new();
    for look in [
        Look::new(),
        terminal(),
        retro(LowText::Pixelated, Upscale::Fit),
    ] {
        let timings = rig
            .renderer
            .render_flat_look(&base, &LookFrame::still(&look, &tokens), &rig.target.view)
            .unwrap();
        assert!(timings.iter().any(|timing| timing.label == "flat"));
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn look_passes_cost_at_4k_and_deck_size() {
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let tokens = board_tokens();
    let quantise = |dither| LookPass::Quantise(Quantise::new(retro_palette(), dither).unwrap());
    let bayer = Dither::Bayer {
        order: 4,
        spread: 0.09,
    };
    let crt = |crt: Crt| LookPass::Crt(crt);
    let lines = Crt {
        scanlines: Some(Scanlines {
            period: 4.0,
            strength: 0.35,
        }),
        ..Crt::default()
    };
    let glow = Crt {
        glow: Some(Glow {
            threshold: 0.55,
            radius: 6.0,
            strength: 0.7,
            tint: Srgba::hex(0x7cf2a0).0,
        }),
        ..Crt::default()
    };
    let curved = Crt {
        curvature: Some(Curvature {
            amount: 0.06,
            corner: 0.03,
            bezel: [0.0, 0.0, 0.0, 1.0],
        }),
        aberration: Some(Aberration { shift: 1.0 }),
        ..Crt::default()
    };
    let web = LookPass::Quantise(Quantise::new(Palette::cube(6).unwrap(), bayer).unwrap());
    let variants: Vec<(&str, Look)> = vec![
        ("no look", Look::new()),
        ("tokens", Look::new().with(LookPass::Tokens(dark_tokens()))),
        ("backdrop: gradient and grid", sky_look()),
        ("vignette", Look::new().with(LookPass::Vignette(pool()))),
        (
            "quantise 16, full size",
            Look::new().with(quantise(Dither::None)),
        ),
        (
            "quantise 16, Bayer 4, full size",
            Look::new().with(quantise(bayer)),
        ),
        ("quantise 216, Bayer 4, full size", Look::new().with(web)),
        ("scanlines", Look::new().with(crt(lines))),
        ("phosphor glow", Look::new().with(crt(glow))),
        ("curvature and aberration", Look::new().with(crt(curved))),
        (
            "low resolution 480x270",
            Look::new().with(LookPass::LowRes(
                LowRes::new([480, 270]).upscale(Upscale::Fit),
            )),
        ),
        (
            "low resolution, quantise 16 with Bayer 4",
            retro(LowText::Pixelated, Upscale::Fit),
        ),
        (
            "full stack: low resolution, quantise and Bayer 4, then CRT",
            retro(LowText::Pixelated, Upscale::Fit).with(crt(Crt {
                scanlines: lines.scanlines,
                glow: glow.glow,
                curvature: curved.curvature,
                aberration: curved.aberration,
            })),
        ),
    ];
    let mut report = String::new();
    for size in [[3840u32, 2160u32], [1280, 800]] {
        let mut turns = pfx_gpu::pace::Turns::default();
        let mut rig = turns.cpu(|| Rig::new(size));
        for (round, (label, look)) in std::iter::once(variants[0].clone())
            .chain(variants.iter().cloned())
            .enumerate()
        {
            let mut totals = Vec::new();
            let mut passes: BTreeMap<String, Vec<f64>> = BTreeMap::new();
            for frame in 0..70 {
                turns.poll();
                let timings = rig
                    .renderer
                    .render_flat_look(&base, &LookFrame::still(&look, &tokens), &rig.target.view)
                    .unwrap();
                let flat: f64 = timings
                    .iter()
                    .filter(|t| t.label != "flat ids")
                    .map(|t| t.milliseconds)
                    .sum();
                turns.add(timings.iter().map(|t| t.milliseconds).sum());
                if frame >= 10 && !timings.is_empty() {
                    totals.push(flat);
                    let mut by_label: BTreeMap<String, f64> = BTreeMap::new();
                    for timing in &timings {
                        *by_label.entry(timing.label.clone()).or_default() += timing.milliseconds;
                    }
                    for (label, ms) in by_label {
                        passes.entry(label).or_default().push(ms);
                    }
                }
            }
            assert!(totals.len() >= 50, "GPU timestamps are unavailable");
            if round == 0 {
                continue;
            }
            let parts = passes
                .iter_mut()
                .filter(|(label, _)| label.as_str() != "flat ids")
                .map(|(label, values)| format!("{label} {:.3}", median(values)))
                .collect::<Vec<_>>()
                .join(", ");
            report += &format!(
                "{}x{} {label}: {:.3} ms ({parts})\n",
                size[0],
                size[1],
                median(&mut totals)
            );
        }
    }
    eprintln!("{report}");
}

fn shared_grey_tokens() -> TokenSet {
    TokenSet::new()
        .with("card.fill", Srgba::hex(0x808080))
        .with("panel.fill", Srgba::hex(0x808080))
        .with("edge", Srgba::hex(0x808080).alpha(0.5))
        .with("hatch", Srgba::hex(0x202020))
}

fn swap_tokens() -> TokenSet {
    TokenSet::new()
        .with("card.fill", Srgba::hex(0xcc2222))
        .with("panel.fill", Srgba::hex(0x2222cc))
        .with("edge", Srgba::hex(0x22cc22))
        .with("hatch", Srgba::hex(0xeeeeee))
}

#[test]
fn token_keys_are_stable_names() {
    assert_eq!(TokenKey::new("card"), TokenKey::from("card"));
    assert_eq!(TokenKey::new("card"), TokenKey::from(&"card".to_string()));
    assert_ne!(TokenKey::new("card"), TokenKey::new("panel"));
    assert_ne!(TokenKey::new(""), TokenKey::new("a"));
    const KEY: TokenKey = TokenKey::new("card");
    assert_eq!(KEY, TokenKey::new("card"));
    assert!(Roles::default().is_none());
    assert!(
        !Draw::circle([0.0, 0.0], 1.0)
            .fill_token("a")
            .roles
            .is_none()
    );
}

#[test]
fn keyed_draws_take_different_colours_where_bytes_collide() {
    let base = shared_grey_tokens();
    let recolour = Recolour::new(&base, &swap_tokens());
    let grey = Srgba::hex(0x808080);
    let draws = [
        Draw::rect([20.0, 20.0], [20.0, 20.0], 0.0)
            .fill(grey)
            .fill_token("card.fill"),
        Draw::rect([60.0, 20.0], [20.0, 20.0], 0.0)
            .fill(grey)
            .fill_token("panel.fill"),
        Draw::rect([100.0, 20.0], [20.0, 20.0], 0.0).fill(grey),
    ];
    let curve = test_curve();
    let copy = SceneCopy::recoloured(&scene(&draws, &curve, &[]), &recolour);
    assert_eq!(copy.draws[0].fill, Fill::Solid(Srgba::hex(0xcc2222)));
    assert_eq!(copy.draws[1].fill, Fill::Solid(Srgba::hex(0x2222cc)));
    let untagged = copy.draws[2].fill;
    assert!(
        untagged == Fill::Solid(Srgba::hex(0xcc2222))
            || untagged == Fill::Solid(Srgba::hex(0x2222cc)),
        "an untagged draw still matches its colour by bytes"
    );
}

#[test]
fn keys_cover_every_colour_slot_of_a_draw() {
    let base = shared_grey_tokens();
    let recolour = Recolour::new(&base, &swap_tokens());
    let grey = Srgba::hex(0x808080);
    let curve = test_curve();
    let mut glowing = Draw::circle([20.0, 20.0], 8.0)
        .fill(grey)
        .stroke(Stroke::solid(grey.alpha(0.25), 2.0))
        .pattern(Pattern::new(
            PatternKind::Dots,
            4.0,
            0.0,
            0.5,
            Srgba::hex(0x202020),
        ))
        .fill_token("card.fill")
        .stroke_token("edge")
        .pattern_token("hatch")
        .glow_token("panel.fill");
    glowing.shadow = Shadow::Glow(super::super::Glow {
        colour: grey,
        sigma: 4.0,
    });
    let split = Draw::rect([60.0, 20.0], [20.0, 20.0], 0.0)
        .vertical(grey, grey)
        .fill_token("card.fill")
        .fill_end_token("panel.fill");
    let draws = [glowing, split];
    let copy = SceneCopy::recoloured(&scene(&draws, &curve, &[]), &recolour);
    let out = copy.draws[0];
    assert_eq!(out.fill, Fill::Solid(Srgba::hex(0xcc2222)));
    assert_eq!(out.pattern.unwrap().colour, Srgba::hex(0xeeeeee));
    assert_eq!(
        out.stroke.unwrap().colour,
        Srgba::hex(0x22cc22).alpha(0.25 / 0.5),
        "alpha stays relative to the base token's"
    );
    let Shadow::Glow(glow) = out.shadow else {
        panic!("the glow stays a glow");
    };
    assert_eq!(glow.colour, Srgba::hex(0x2222cc));
    assert_eq!(
        copy.draws[1].fill,
        Fill::Vertical(Srgba::hex(0xcc2222), Srgba::hex(0x2222cc))
    );
}

#[test]
fn a_key_the_look_leaves_alone_keeps_the_colour_the_draw_has() {
    let base = shared_grey_tokens();
    let curve = test_curve();
    let off_by_one = Srgba::hex(0x818181);
    let draws = [Draw::rect([20.0, 20.0], [20.0, 20.0], 0.0)
        .fill(off_by_one)
        .fill_token("card.fill")];
    let only_panel = TokenSet::new().with("panel.fill", Srgba::hex(0x2222cc));
    let mut effective = base.clone();
    for (key, colour) in only_panel.iter() {
        effective.insert(key, colour);
    }
    let recolour = Recolour::new(&base, &effective);
    let copy = SceneCopy::recoloured(&scene(&draws, &curve, &[]), &recolour);
    assert_eq!(copy.draws[0].fill, Fill::Solid(off_by_one));
    assert!(Recolour::new(&base, &base).is_identity());
    let unknown = Recolour::new(&base, &base.clone().with("fresh", Srgba::WHITE));
    assert!(!unknown.is_identity());
    assert_eq!(
        unknown.pick(
            Srgba::hex(0x123456).alpha(0.5),
            Some(TokenKey::new("fresh"))
        ),
        Srgba::WHITE.alpha(0.5)
    );
    assert_eq!(
        unknown.pick(Srgba::hex(0x123456), Some(TokenKey::new("missing"))),
        Srgba::hex(0x123456)
    );
}

#[test]
fn text_takes_the_key_of_the_first_draw_that_places_it() {
    let draws = [
        Draw::new(Shape::Text(1)).fill_token("a"),
        Draw::new(Shape::Text(1)).fill_token("b"),
        Draw::new(Shape::Text(0)),
        Draw::new(Shape::Text(9)).fill_token("c"),
        Draw::circle([0.0, 0.0], 1.0).fill_token("d"),
    ];
    assert_eq!(
        text_keys(&draws, 3),
        vec![None, Some(TokenKey::new("a")), None]
    );
}

#[test]
fn keyed_tokens_crossfade_with_the_sets() {
    let base = shared_grey_tokens();
    let swap = swap_tokens();
    let middle = base.mix(&swap, 0.5);
    let recolour = Recolour::new(&base, &middle);
    let got = recolour.pick(Srgba::hex(0x808080), Some(TokenKey::new("card.fill")));
    let want = middle.get("card.fill").unwrap();
    assert_eq!(got, want);
    assert_ne!(got, Srgba::hex(0x808080));
    assert_ne!(got, Srgba::hex(0xcc2222));
    let start = Recolour::new(&base, &base.mix(&swap, 0.0));
    assert!(start.is_identity());
}

fn keyed_scene_draws() -> Vec<Draw> {
    let grey = Srgba::hex(0x808080);
    vec![
        Draw::rect([50.0, 50.0], [60.0, 60.0], 0.0)
            .fill(grey)
            .fill_token("card.fill"),
        Draw::rect([150.0, 50.0], [60.0, 60.0], 0.0)
            .fill(grey)
            .fill_token("panel.fill"),
    ]
}

fn small_scene<'a>(draws: &'a [Draw], curve: &'a ShadowCurve) -> FlatScene<'a> {
    FlatScene {
        layout: [200.0, 100.0],
        clear: Some(Srgba::hex(0x000000)),
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

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn keyed_roles_render_apart_and_keep_the_bytes_of_an_unkeyed_frame() {
    let draws = keyed_scene_draws();
    let curve = ShadowCurve::none();
    let scene = small_scene(&draws, &curve);
    let base = shared_grey_tokens();
    let look = Look::new().with(LookPass::Tokens(swap_tokens()));
    let mut rig = Rig::new([200, 100]);
    let pixels = f32s(&rig.look(&scene, &LookFrame::still(&look, &base)));
    let at = |x: u32, y: u32| byte(pixels[(y * 200 + x) as usize]);
    assert_eq!(at(50, 50), [0xcc, 0x22, 0x22]);
    assert_eq!(at(150, 50), [0x22, 0x22, 0xcc]);
    let identity = Look::new();
    let plain = rig.plain(&scene);
    let still = rig.look(&scene, &LookFrame::still(&identity, &base));
    assert!(plain == still, "keys alone change no bytes");
    let same = Look::new().with(LookPass::Tokens(base.clone()));
    assert!(plain == rig.look(&scene, &LookFrame::still(&same, &base)));
    let untagged: Vec<Draw> = draws
        .iter()
        .map(|draw| Draw {
            roles: Roles::default(),
            ..*draw
        })
        .collect();
    let untagged_scene = small_scene(&untagged, &curve);
    assert!(plain == rig.plain(&untagged_scene));
}

#[test]
fn shadow_tints_apply_to_the_curve_colour() {
    let curve = Srgba::hex(0x2b3442);
    assert_eq!(ShadowTint::new().apply(curve), curve);
    assert_eq!(ShadowTint::new().alpha(0.5).apply(curve), curve.alpha(0.5));
    assert_eq!(
        ShadowTint::new()
            .colour(Srgba::hex(0x000000).alpha(0.8))
            .alpha(2.0)
            .apply(curve),
        Srgba::hex(0x000000).alpha(1.0)
    );
    assert_eq!(
        ShadowTint::new().colour(Srgba::hex(0x101010)).apply(curve),
        Srgba::hex(0x101010)
    );
    let bad = Look::new().with(LookPass::Shadow(ShadowTint::new().alpha(f32::NAN)));
    assert!(bad.validate().is_err());
    let negative = Look::new().with(LookPass::Shadow(ShadowTint::new().alpha(-0.1)));
    assert!(negative.validate().is_err());
    let twice = Look::new()
        .with(LookPass::Shadow(ShadowTint::new()))
        .with(LookPass::Shadow(ShadowTint::new()));
    assert!(twice.validate().is_err());
    let ok = Look::new().with(LookPass::Shadow(ShadowTint::new().alpha(0.0)));
    assert!(ok.validate().is_ok());
    assert!(ok.shadow().is_some() && Look::new().shadow().is_none());
}

#[test]
fn shadow_tints_crossfade_with_the_transition_and_wipe_apart() {
    let base = TokenSet::new();
    let curve = Srgba::hex(0x2b3442);
    let plain = Look::new();
    let dark = Look::new().with(LookPass::Shadow(
        ShadowTint::new().colour(Srgba::hex(0x000000)).alpha(0.5),
    ));
    let mut stack = LookStack::new(plain.clone()).unwrap().base(base.clone());
    stack
        .switch(
            dark.clone(),
            0.0,
            Transition::crossfade(1.0, Easing::Linear),
        )
        .unwrap();
    let frame = stack.frame(0.0);
    assert_eq!(side_shadow(&frame, frame.to, true, curve), curve);
    let frame = stack.frame(0.5);
    let middle = side_shadow(&frame, frame.to, true, curve);
    let want = Srgba(mix_oklab(curve.0, Srgba::hex(0x000000).alpha(0.5).0, 0.5));
    assert_eq!(middle, want);
    let frame = stack.frame(1.0);
    assert_eq!(
        side_shadow(&frame, frame.to, false, curve),
        Srgba::hex(0x000000).alpha(0.5)
    );
    stack
        .switch(
            plain.clone(),
            2.0,
            Transition::wipe(1.0, Easing::Linear, 0.0, 10.0),
        )
        .unwrap();
    let frame = stack.frame(2.5);
    assert_eq!(
        side_shadow(&frame, frame.from, false, curve),
        Srgba::hex(0x000000).alpha(0.5)
    );
    assert_eq!(side_shadow(&frame, frame.to, false, curve), curve);
    assert!(plain.same_pixels(&dark));
    let tokens_and_tint = dark
        .clone()
        .with(LookPass::Tokens(TokenSet::new().with("a", Srgba::WHITE)));
    assert!(plain.same_pixels(&tokens_and_tint));
    assert!(!plain.same_pixels(&terminal()));
}

fn tinted_curve(tint: &ShadowTint) -> ShadowCurve {
    let mut curve = test_curve();
    curve.colour = tint.apply(curve.colour);
    curve
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_look_shadow_tint_draws_the_frame_of_the_curve_it_stands_for() {
    let draws = board_draws();
    let curve = test_curve();
    let base = scene(&draws, &curve, &[]);
    let tint = ShadowTint::new().colour(Srgba::hex(0x000000)).alpha(0.7);
    let tinted = tinted_curve(&tint);
    let reference = scene(&draws, &tinted, &[]);
    let look = Look::new().with(LookPass::Shadow(tint));
    let tokens = TokenSet::new();
    let mut rig = Rig::new([1280, 800]);
    let want = rig.plain(&reference);
    let got = rig.look(&base, &LookFrame::still(&look, &tokens));
    assert!(got == want, "the tint is the curve's colour and scale");
    let plain = rig.plain(&base);
    assert!(plain != want, "the tint changes the shadows");
    let none = Look::new();
    assert!(plain == rig.look(&base, &LookFrame::still(&none, &tokens)));
    let mut stack = LookStack::new(Look::new()).unwrap().base(TokenSet::new());
    stack
        .switch(
            look.clone(),
            0.0,
            Transition::crossfade(1.0, Easing::Linear),
        )
        .unwrap();
    let middle = Srgba(mix_oklab(curve.colour.0, tint.apply(curve.colour).0, 0.5));
    let mut half = test_curve();
    half.colour = middle;
    let want = rig.plain(&scene(&draws, &half, &[]));
    let got = rig.look(&base, &stack.frame(0.5));
    assert!(got == want, "a crossfade draws once with the mixed tint");
    assert!(rig.look(&base, &stack.frame(0.0)) == plain);
    assert!(rig.look(&base, &stack.frame(1.0)) == rig.plain(&reference));
}

#[test]
fn warming_plans_a_pass_for_each_slot_a_look_draws_in() {
    let size = [1280u32, 800u32];
    let low = [480u32, 270u32];
    assert_eq!(warm_slots([&Look::new()], size).unwrap(), []);
    assert_eq!(warm_slots([&sky_look()], size).unwrap(), [(0, size)]);
    assert_eq!(
        warm_slots([&retro(LowText::Pixelated, Upscale::Fit)], size).unwrap(),
        [(0, low), (2, low)]
    );
    assert_eq!(
        warm_slots([&retro(LowText::Crisp, Upscale::Fit)], size).unwrap(),
        [(0, low), (1, size), (2, low), (3, size)]
    );
    let all = [
        sky_look(),
        retro(LowText::Crisp, Upscale::Fit),
        terminal(),
        retro(LowText::Crisp, Upscale::Fit),
    ];
    assert_eq!(
        warm_slots(&all, size).unwrap(),
        [(0, size), (0, low), (1, size), (2, low), (3, size)]
    );
    let bad = Look::new().with(LookPass::LowRes(LowRes::new([0, 0])));
    assert!(warm_slots([&bad], size).is_err());
}

fn lit_draws() -> Vec<Draw> {
    let mut draws = board_draws();
    draws.push(
        Draw::rect([700.0, 540.0], [220.0, 140.0], 20.0)
            .fill(Srgba::hex(0xfafbfc))
            .elevation(2.0)
            .material(super::super::Material {
                thickness: 6.0,
                bevel: 2.0,
                gloss: 0.4,
                roughness: 0.3,
                reflection: 0.0,
            })
            .id(200),
    );
    draws
}

fn storage_view(gpu: &Gpu, size: [u32; 2]) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("warm output"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    texture.create_view(&Default::default())
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_warmed_renderer_makes_no_flat_pipeline_on_its_first_frames() {
    let mut turns = pfx_gpu::pace::Turns::default();
    let draws = lit_draws();
    let curve = test_curve();
    let plain = scene(&draws, &curve, &[]);
    let posted = FlatScene {
        post: true,
        ..scene(&draws, &curve, &[])
    };
    let size = [1280u32, 800u32];
    let looks = [
        sky_look(),
        retro(LowText::Crisp, Upscale::Fit),
        retro(LowText::Pixelated, Upscale::Fit),
        terminal(),
        Look::new().with(LookPass::Vignette(pool())),
    ];
    let tokens = board_tokens();
    for format in [
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let gpu = turns.cpu(|| pollster::block_on(Gpu::headless()).unwrap());
        let mut renderer =
            turns.cpu(|| Renderer::new_with_output_format(gpu, size[0], size[1], format).unwrap());
        let target = renderer.gpu().offscreen(size[0], size[1], format).unwrap();
        assert_eq!(renderer.flat_pipelines_made_on_render_thread(), 0);
        turns
            .cpu(|| renderer.warm_flat(format, size, &looks))
            .unwrap();
        let warmed = renderer.flat_pipelines_made_on_render_thread();
        assert!(warmed > 0, "{format:?}: warming made nothing");
        turns
            .cpu(|| renderer.warm_flat(format, size, &looks))
            .unwrap();
        assert_eq!(
            renderer.flat_pipelines_made_on_render_thread(),
            warmed,
            "{format:?}: warming twice made more"
        );
        let hdr = format == wgpu::TextureFormat::Rgba16Float;
        let posted_view = hdr.then(|| storage_view(renderer.gpu(), size));
        let through = posted_view.as_ref().unwrap_or(&target.view);
        turns.cpu(|| renderer.render_flat(&plain, &target.view).unwrap());
        turns.cpu(|| renderer.render_flat(&posted, through).unwrap());
        for (index, look) in looks.iter().enumerate() {
            turns.cpu(|| {
                renderer
                    .render_flat_look(&plain, &LookFrame::still(look, &tokens), &target.view)
                    .unwrap()
            });
            assert_eq!(
                renderer.flat_pipelines_made_on_render_thread(),
                warmed,
                "{format:?}: look {index} made a pipeline on the render thread"
            );
        }
        let mut stack = LookStack::new(looks[0].clone())
            .unwrap()
            .base(tokens.clone());
        stack
            .switch(
                looks[1].clone(),
                0.0,
                Transition::wipe(1.0, Easing::InOut, 0.4, 80.0),
            )
            .unwrap();
        turns.cpu(|| {
            renderer
                .render_flat_look(&plain, &stack.frame(0.5), &target.view)
                .unwrap()
        });
        let mut posted_look = LookStack::new(looks[3].clone())
            .unwrap()
            .base(tokens.clone());
        posted_look
            .switch(
                looks[2].clone(),
                0.0,
                Transition::crossfade(1.0, Easing::Linear),
            )
            .unwrap();
        turns.cpu(|| {
            renderer
                .render_flat_look(&posted, &posted_look.frame(0.5), through)
                .unwrap()
        });
        assert_eq!(
            renderer.flat_pipelines_made_on_render_thread(),
            warmed,
            "{format:?}: a frame made a pipeline on the render thread"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn an_unwarmed_renderer_counts_the_flat_pipelines_its_first_frame_makes() {
    let draws = lit_draws();
    let curve = test_curve();
    let plain = scene(&draws, &curve, &[]);
    let mut rig = Rig::new([640, 400]);
    assert_eq!(rig.renderer.flat_pipelines_made_on_render_thread(), 0);
    rig.plain(&plain);
    let first = rig.renderer.flat_pipelines_made_on_render_thread();
    assert!(first > 0);
    rig.plain(&plain);
    assert_eq!(rig.renderer.flat_pipelines_made_on_render_thread(), first);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_look_set_counts_its_pipelines_exactly() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let size = [1280u32, 800u32];
    let own = LookGpu::new(&gpu.device, &gpu.queue).pipelines();
    assert_eq!(own, 9);
    let slot = FlatPass::new(&gpu.device, &gpu.queue).pipelines_made() + 2;

    let mut identity = FlatLooks::new(&gpu.device, &gpu.queue);
    assert_eq!(identity.pipelines_made(), own);
    identity
        .warm(&gpu.device, &gpu.queue, [&Look::new()], size)
        .unwrap();
    assert_eq!(identity.pipelines_made(), own);

    let mut full = FlatLooks::new(&gpu.device, &gpu.queue);
    let crisp = retro(LowText::Crisp, Upscale::Fit);
    full.warm(&gpu.device, &gpu.queue, [&crisp], size).unwrap();
    assert_eq!(full.pipelines_made(), own + 4 * slot);
    full.warm(&gpu.device, &gpu.queue, [&crisp], size).unwrap();
    assert_eq!(full.pipelines_made(), own + 4 * slot);

    let mut runtime = FlatLooks::new(&gpu.device, &gpu.queue);
    runtime
        .warm(&gpu.device, &gpu.queue, [&sky_look()], size)
        .unwrap();
    assert_eq!(runtime.pipelines_made(), own + slot);
    runtime
        .warm(&gpu.device, &gpu.queue, [&sky_look(), &terminal()], size)
        .unwrap();
    assert_eq!(runtime.pipelines_made(), own + slot);
    runtime
        .warm(
            &gpu.device,
            &gpu.queue,
            [&retro(LowText::Pixelated, Upscale::Fit)],
            size,
        )
        .unwrap();
    assert_eq!(runtime.pipelines_made(), own + 2 * slot);
}
