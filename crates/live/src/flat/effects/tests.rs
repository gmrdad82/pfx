use super::super::tests::{FILTERS, test_curve};
use super::super::{
    ADDITIVE, BLUR, Fit, FlatScene, Group, Icon, Icons, Instance, UNPICKED, cpu, icons, pack_shape,
    scaling, svg, translation,
};
use super::*;
use pfx_core::clock::Steps;
use pfx_physics::rigid::{World, WorldDesc};

const FRAME: f32 = 1.0 / 64.0;

fn fit(size: [u32; 2]) -> Fit {
    Fit::new([size[0] as f32, size[1] as f32], size).unwrap()
}

fn tick(clock: &mut Clock) -> Tick {
    clock.advance(FRAME)
}

fn sparks() -> Burst {
    Burst {
        count: 50,
        life: [0.4, 0.9],
        gravity: [0.0, 600.0],
        drag: 1.5,
        spin: [1.0, 8.0],
        shutter: 1.0 / 60.0,
        ..Burst::new(
            Shape::Rect {
                half: [4.0, 2.0],
                radii: [1.0; 4],
            },
            Fill::Solid(Srgba::hex(0xf0b429)),
        )
    }
}

fn board() -> (svg::Board, Icons) {
    let path = format!("{}/tests/flat/board.svg", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(path).unwrap();
    let parsed = svg::read(&source, &FILTERS).unwrap();
    let icons = Icons::bake(&parsed.icons).unwrap();
    (parsed, icons)
}

fn scene<'a>(
    layout: [f32; 2],
    draws: &'a [Draw],
    groups: &'a [Group],
    curve: &'a super::super::ShadowCurve,
    light: Light,
) -> FlatScene<'a> {
    FlatScene {
        layout,
        clear: Some(Srgba::hex(0xeceff3)),
        curve,
        light,
        draws,
        groups,
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    }
}

fn overlap(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[1].min(b[1]) - a[0].max(b[0])).max(0.0)
}

#[test]
fn blur_is_exact_along_the_motion_of_a_straight_edge() {
    let size = [160, 80];
    let fit = fit(size);
    for (shift, x_range) in [
        (30.0f32, 0..160),
        (0.6, 50..110),
        (3.0, 40..120),
        (90.0, 0..160),
    ] {
        let draw = Draw::rect([80.0, 40.0], [40.0, 30.0], 0.0)
            .fill(Srgba::WHITE)
            .blur([shift, 0.0]);
        let instance = pack_shape(&draw, &fit, None).unwrap();
        assert_ne!(instance.info[0] & BLUR, 0);
        let mut worst = 0.0f32;
        for py in [32u32, 40, 47] {
            for px in x_range.clone() {
                let centre = [px as f32 + 0.5, py as f32 + 0.5];
                let ours = blur::cover(&instance, None, false, instance.to_local(centre));
                let samples = 20_000;
                let mut sum = 0.0f64;
                for k in 0..samples {
                    let t = (k as f32 + 0.5) / samples as f32 - 0.5;
                    let left = 60.0 + t * shift;
                    sum += f64::from(overlap(
                        [centre[0] - 0.5, centre[0] + 0.5],
                        [left, left + 40.0],
                    ));
                }
                let reference = (sum / f64::from(samples)) as f32;
                worst = worst.max((ours - reference).abs());
            }
        }
        assert!(worst < 2e-3, "shift {shift}: worst {worst}");
    }
}

fn brute(
    stat: &Instance,
    icons: Option<&Icons>,
    cell: Option<[f32; 4]>,
    pixel: [f32; 2],
    shift: [f32; 2],
    stroke: bool,
) -> f32 {
    let (n, times) = (12, 96);
    let mut inside = 0u32;
    for k in 0..times {
        let t = (k as f32 + 0.5) / times as f32 - 0.5;
        for sy in 0..n {
            for sx in 0..n {
                let p = [
                    pixel[0] + (sx as f32 + 0.5) / n as f32 - 0.5 - t * shift[0],
                    pixel[1] + (sy as f32 + 0.5) / n as f32 - 0.5 - t * shift[1],
                ];
                let local = stat.to_local(p);
                if let Some([cx, cy, hx, hy]) = cell
                    && ((local[0] - cx).abs() > hx || (local[1] - cy).abs() > hy)
                {
                    continue;
                }
                let d = cpu::field(stat, icons, local)[0];
                let mut d = if stroke {
                    d.abs() - stat.stroke[0] * 0.5
                } else {
                    d
                };
                if stroke && stat.info[0] & super::super::DASHED != 0 {
                    let along = super::super::sdf::rect_along(
                        local,
                        [stat.shape[0], stat.shape[1]],
                        stat.corners,
                    );
                    let dash = super::super::sdf::dash(
                        along,
                        stat.stroke[1],
                        stat.stroke[2],
                        stat.stroke[3],
                    );
                    d = d.max(dash[0]);
                }
                if d < 0.0 {
                    inside += 1;
                }
            }
        }
    }
    inside as f32 / (n * n * times) as f32
}

fn compare(draw: Draw, shift: [f32; 2], icons: Option<&Icons>, stroke: bool) -> (f32, f32) {
    let size = [120, 120];
    let fit = fit(size);
    let stat = pack_shape(&draw, &fit, icons).unwrap();
    let moving = pack_shape(&draw.blur(shift), &fit, icons).unwrap();
    let cell = match draw.shape {
        Shape::Icon(handle) => {
            let [x0, y0, x1, y1] = icons.unwrap().entry(handle).unwrap().bounds;
            Some([
                (x0 + x1) * 0.5,
                (y0 + y1) * 0.5,
                (x1 - x0) * 0.5,
                (y1 - y0) * 0.5,
            ])
        }
        _ => None,
    };
    let [x0, y0, x1, y1] = moving.pixel_bounds();
    let (mut sum, mut worst, mut count) = (0.0f32, 0.0f32, 0);
    for py in (y0.max(0.0) as u32..(y1.ceil() as u32).min(size[1])).step_by(3) {
        for px in (x0.max(0.0) as u32..(x1.ceil() as u32).min(size[0])).step_by(3) {
            let centre = [px as f32 + 0.5, py as f32 + 0.5];
            let ours = blur::cover(&moving, icons, stroke, moving.to_local(centre));
            let theirs = brute(&stat, icons, cell, centre, shift, stroke);
            let error = (ours - theirs).abs();
            sum += error;
            worst = worst.max(error);
            count += 1;
        }
    }
    (sum / count as f32, worst)
}

#[test]
fn blur_matches_the_integral_of_moving_shapes() {
    let tick = Icons::bake(&[icons::tick(), icons::pointer()]).unwrap();
    let cases = [
        (
            "circle",
            Draw::circle([60.0, 60.0], 15.0).fill(Srgba::WHITE),
            [18.0, 12.0],
            false,
        ),
        (
            "turned rect",
            Draw::rect([0.0, 0.0], [30.0, 16.0], 4.0)
                .transform(place([60.0, 60.0], 0.5))
                .fill(Srgba::WHITE),
            [-20.0, 9.0],
            false,
        ),
        (
            "ring",
            Draw::new(Shape::Ring {
                radius: 14.0,
                width: 5.0,
            })
            .at([60.0, 60.0])
            .fill(Srgba::WHITE),
            [0.0, 24.0],
            false,
        ),
        (
            "rect outline",
            Draw::rect([60.0, 60.0], [36.0, 24.0], 6.0).stroke(Stroke::solid(Srgba::WHITE, 3.0)),
            [16.0, 16.0],
            true,
        ),
        (
            "dashed outline",
            Draw::rect([60.0, 60.0], [60.0, 30.0], 8.0).stroke(Stroke::dashed(
                Srgba::WHITE,
                3.0,
                9.0,
                6.0,
            )),
            [14.0, 0.0],
            true,
        ),
        (
            "corner rect",
            Draw::rect_corners([60.0, 60.0], [40.0, 28.0], [0.0, 12.0, 4.0, 10.0])
                .fill(Srgba::WHITE),
            [17.0, -9.0],
            false,
        ),
        (
            "corner outline",
            Draw::rect_corners([60.0, 60.0], [44.0, 30.0], [10.0, 0.0, 14.0, 0.0])
                .stroke(Stroke::solid(Srgba::WHITE, 3.0)),
            [-12.0, 14.0],
            true,
        ),
        (
            "corner dashes",
            Draw::rect_corners([60.0, 60.0], [60.0, 30.0], [4.0, 12.0, 8.0, 12.0])
                .stroke(Stroke::dashed(Srgba::WHITE, 3.0, 9.0, 6.0)),
            [14.0, 5.0],
            true,
        ),
        (
            "check",
            Draw::new(Shape::Icon(Icon(0)))
                .transform(multiply(translation(60.0, 60.0), scaling(1.6, 1.6)))
                .stroke(Stroke::solid(Srgba::WHITE, 5.0)),
            [26.0, -14.0],
            true,
        ),
        (
            "pointer",
            Draw::new(Shape::Icon(Icon(1)))
                .transform(multiply(translation(40.0, 30.0), scaling(0.8, 0.8)))
                .fill(Srgba::WHITE),
            [30.0, 20.0],
            false,
        ),
    ];
    let mut report = String::new();
    let mut misses = Vec::new();
    for (name, draw, shift, stroke) in cases {
        let icons = matches!(draw.shape, Shape::Icon(_)).then_some(&tick);
        let (mean, worst) = compare(draw, shift, icons, stroke);
        report += &format!("{name}: mean {mean:.4} worst {worst:.4}\n");
        if mean > 0.002 || worst > 0.03 {
            misses.push(name);
        }
    }
    eprintln!("{report}");
    assert!(misses.is_empty(), "{misses:?}\n{report}");
}

#[test]
fn the_closed_form_agrees_with_the_march() {
    let fit = fit([160, 120]);
    let cases = [
        (
            Draw::rect([0.0, 0.0], [50.0, 30.0], 9.0).fill(Srgba::WHITE),
            false,
        ),
        (
            Draw::rect([0.0, 0.0], [50.0, 30.0], 2.0).stroke(Stroke::solid(Srgba::WHITE, 6.0)),
            true,
        ),
        (Draw::circle([0.0, 0.0], 17.0).fill(Srgba::WHITE), false),
        (
            Draw::circle([0.0, 0.0], 17.0).stroke(Stroke::solid(Srgba::WHITE, 4.0)),
            true,
        ),
        (
            Draw::new(Shape::Ring {
                radius: 15.0,
                width: 6.0,
            })
            .fill(Srgba::WHITE),
            false,
        ),
    ];
    for (index, (draw, stroke)) in cases.into_iter().enumerate() {
        for (shift, turn) in [
            ([24.0f32, 0.0f32], 0.0f32),
            ([13.0, -21.0], 0.7),
            ([0.5, 0.3], 2.0),
        ] {
            let moving = draw.transform(place([80.0, 60.0], turn)).blur(shift);
            let instance = pack_shape(&moving, &fit, None).unwrap();
            assert!(blur::closed(&instance, stroke));
            let [x0, y0, x1, y1] = instance.pixel_bounds();
            let mut worst = 0.0f32;
            for py in y0.max(0.0) as u32..(y1.ceil() as u32).min(120) {
                for px in x0.max(0.0) as u32..(x1.ceil() as u32).min(160) {
                    let local = instance.to_local([px as f32 + 0.5, py as f32 + 0.5]);
                    let closed = blur::cover_with(&instance, None, stroke, local, true);
                    let marched = blur::cover_with(&instance, None, stroke, local, false);
                    worst = worst.max((closed - marched).abs());
                }
            }
            assert!(worst < 0.01, "case {index} shift {shift:?}: {worst}");
        }
    }
}

#[test]
fn a_blur_under_a_quarter_pixel_packs_the_still_instance() {
    let fit = fit([200, 200]);
    let draw = Draw::rect([100.0, 100.0], [40.0, 20.0], 5.0).fill(Srgba::WHITE);
    let still = pack_shape(&draw, &fit, None).unwrap();
    assert_eq!(pack_shape(&draw.blur([0.2, 0.1]), &fit, None), Some(still));
    assert_eq!(
        pack_shape(&draw.blur([f32::NAN, 9.0]), &fit, None),
        Some(still)
    );
    let moving = pack_shape(&draw.blur([30.0, 0.0]), &fit, None).unwrap();
    assert_eq!(moving.info[0] & BLUR, BLUR);
    assert_eq!(moving.info[0] & super::super::LIT, 0);
    assert_eq!(moving.bounds[2], still.bounds[2] + 15.0);
    let lit = draw.material(super::super::Material {
        thickness: 4.0,
        bevel: 1.0,
        ..Default::default()
    });
    let moving_lit = pack_shape(&lit.blur([30.0, 0.0]), &fit, None).unwrap();
    assert_eq!(moving_lit.info[0], moving.info[0]);
}

fn identical(a: &[Draw], b: &[Draw]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| format!("{a:?}") == format!("{b:?}"))
}

#[test]
fn frames_without_effects_are_byte_identical() {
    let (parsed, icons) = board();
    let curve = test_curve();
    let mut effects = Effects::new(EffectsDesc {
        shake: ShakeTuning {
            translation: [20.0, 20.0],
            roll: 0.02,
            frequency: 15.0,
            decay: 2.0,
            seed: 5,
        },
        ..EffectsDesc::new(parsed.size, 9)
    });
    effects.recipe(sparks()).unwrap();
    let mut clock = Clock::new(60.0).unwrap();
    for _ in 0..30 {
        effects.step(&tick(&mut clock));
    }
    let light = Light::default();
    let (composed, composed_light) = effects.compose(&parsed.draws, light);
    assert!(identical(composed, &parsed.draws));
    assert_eq!(composed_light, light);
    let size = [480, 270];
    let raw = cpu::render(
        &scene(parsed.size, &parsed.draws, &parsed.groups, &curve, light),
        Some(&icons),
        size,
    )
    .unwrap();
    let through = cpu::render(
        &scene(
            parsed.size,
            composed,
            &parsed.groups,
            &curve,
            composed_light,
        ),
        Some(&icons),
        size,
    )
    .unwrap();
    assert!(
        raw.iter()
            .zip(&through)
            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits))
    );
    let fit = Fit::new(parsed.size, size).unwrap();
    for draw in &parsed.draws {
        if let Some(instance) = pack_shape(draw, &fit, Some(&icons)) {
            assert_eq!(instance.info[0] & super::super::FX, 0);
        }
    }
    assert!(!super::super::shader::flat_source().contains("flat_blur"));
    assert!(super::super::shader::source().contains("flat_blur"));
}

#[test]
fn shake_repeats_from_its_seed_and_moves_the_whole_board() {
    let layout = [1920.0, 1080.0];
    let tuning = |seed| ShakeTuning {
        translation: [30.0, 20.0],
        roll: 0.03,
        frequency: 20.0,
        decay: 1.2,
        seed,
    };
    let run = |seed| {
        let mut effects = Effects::new(EffectsDesc {
            shake: tuning(seed),
            ..EffectsDesc::new(layout, 1)
        });
        let mut clock = Clock::new(60.0).unwrap();
        let mut cameras = Vec::new();
        for frame in 0..120 {
            if frame % 40 == 0 {
                effects.shake(0.7);
            }
            effects.step(&tick(&mut clock));
            cameras.push(effects.camera());
        }
        cameras
    };
    let a = run(3);
    assert_eq!(a, run(3));
    assert_ne!(a, run(4));
    assert!(a.iter().any(|m| m[3][0].abs() > 1.0));
    let mut effects = Effects::new(EffectsDesc {
        shake: tuning(3),
        ..EffectsDesc::new(layout, 1)
    });
    assert_eq!(effects.camera(), IDENTITY);
    effects.shake(1.0);
    let mut clock = Clock::new(60.0).unwrap();
    effects.step(&tick(&mut clock));
    let card = Draw::rect([300.0, 200.0], [100.0, 60.0], 8.0).fill(Srgba::WHITE);
    let light = Light::default();
    let offset = effects.offset();
    let (draws, shaken) = effects.compose(&[card], light);
    assert!(!offset.is_zero());
    let moved = draws[0].transform;
    let centre = [960.0, 540.0];
    let (s, c) = offset.roll.sin_cos();
    let expect = [
        centre[0] + offset.translation[0] + c * (300.0 - centre[0]) - s * (200.0 - centre[1]),
        centre[1] + offset.translation[1] + s * (300.0 - centre[0]) + c * (200.0 - centre[1]),
    ];
    assert!((moved[3][0] - expect[0]).abs() < 1e-3 && (moved[3][1] - expect[1]).abs() < 1e-3);
    let turned = shaken.shadow;
    assert!((turned[0] + s).abs() < 1e-6 && (turned[1] - c).abs() < 1e-6);
}

#[test]
fn the_pool_does_not_allocate_after_warm_up_and_the_cap_holds() {
    let layout = [1920.0, 1080.0];
    let mut effects = Effects::new(EffectsDesc {
        particles: 5000,
        rings: 32,
        ..EffectsDesc::new(layout, 11)
    });
    let id = effects.recipe(sparks()).unwrap();
    let game: Vec<Draw> = (0..200)
        .map(|i| Draw::rect([10.0 + i as f32 * 9.0, 500.0], [8.0, 8.0], 2.0).fill(Srgba::WHITE))
        .collect();
    let mut clock = Clock::new(60.0).unwrap();
    let frame = |effects: &mut Effects, clock: &mut Clock, bursts: u32| {
        let mut spawned = 0;
        for b in 0..bursts {
            spawned += effects.burst(id, [100.0 + b as f32 * 50.0, 300.0]);
        }
        effects.ring(Ring::new([960.0, 540.0], [10.0, 300.0], Srgba::WHITE, 0.4));
        effects.step(&tick(clock));
        (spawned, effects.compose(&game, Light::default()).0.len())
    };
    for _ in 0..30 {
        frame(&mut effects, &mut clock, 8);
    }
    let pools = effects.pool_capacity();
    let out = effects.out_capacity();
    let pointer = effects.composed().as_ptr();
    let mut spawned_total = 0u64;
    for _ in 0..640 {
        let (spawned, drawn) = frame(&mut effects, &mut clock, 8);
        spawned_total += u64::from(spawned);
        assert!(effects.live() <= 5000);
        assert!(drawn <= game.len() + 5000 + 32);
    }
    assert!(spawned_total > 40_000, "{spawned_total}");
    assert!(effects.dropped() > 0);
    assert_eq!(effects.pool_capacity(), pools);
    assert_eq!(effects.out_capacity(), out);
    assert_eq!(effects.composed().as_ptr(), pointer);
    let mut flood = Effects::new(EffectsDesc {
        particles: 500,
        ..EffectsDesc::new(layout, 2)
    });
    let id = flood.recipe(sparks()).unwrap();
    for _ in 0..20 {
        flood.burst(id, [0.0, 0.0]);
    }
    assert_eq!(flood.live(), 500);
    assert_eq!(flood.dropped(), 500);
    assert_eq!(flood.compose(&[], Light::default()).0.len(), 500);
}

#[test]
fn the_clock_scale_slows_effect_lifetimes_exactly() {
    let layout = [1920.0, 1080.0];
    let mut recipe = sparks();
    recipe.life = [1.0, 1.0];
    recipe.count = 10;
    let mut clock = Clock::new(60.0).unwrap();
    clock.set_speed(0.75);
    let mut effects = Effects::new(EffectsDesc::new(layout, 4));
    let id = effects.recipe(recipe).unwrap();
    effects.burst(id, [500.0, 500.0]);
    effects.ring(Ring::new([0.0, 0.0], [0.0, 100.0], Srgba::WHITE, 0.5));
    let mut frames = 0;
    while effects.live() > 0 {
        effects.step(&tick(&mut clock));
        frames += 1;
        if frames == 43 {
            assert_eq!(effects.compose(&[], Light::default()).0.len(), 10);
        }
    }
    assert_eq!(frames, 86);
    let mut full = Clock::new(60.0).unwrap();
    let mut effects = Effects::new(EffectsDesc::new(layout, 4));
    let id = effects.recipe(recipe).unwrap();
    effects.burst(id, [500.0, 500.0]);
    let mut frames = 0;
    while effects.live() > 0 {
        effects.step(&tick(&mut full));
        frames += 1;
    }
    assert_eq!(frames, 64);
    let mut stopped = Clock::new(60.0).unwrap();
    let mut effects = Effects::new(EffectsDesc::new(layout, 4));
    let id = effects.recipe(recipe).unwrap();
    effects.burst(id, [500.0, 500.0]);
    effects.hit_stop(&mut stopped, HitStop::new(0.0, 0.5, 0.0));
    for _ in 0..32 {
        effects.step(&tick(&mut stopped));
    }
    assert_eq!(effects.live(), 10);
    let mut frames = 0;
    while effects.live() > 0 {
        effects.step(&tick(&mut stopped));
        frames += 1;
    }
    assert_eq!(frames, 64);
}

#[test]
fn reduced_motion_at_zero_removes_shake_blur_flashes_and_hit_stop() {
    let layout = [1920.0, 1080.0];
    let mut effects = Effects::new(EffectsDesc {
        shake: ShakeTuning {
            translation: [30.0, 30.0],
            roll: 0.05,
            frequency: 20.0,
            decay: 0.5,
            seed: 1,
        },
        comfort: Comfort::reduced(0.0),
        ..EffectsDesc::new(layout, 1)
    });
    let id = effects.recipe(sparks()).unwrap();
    effects.burst(id, [500.0, 500.0]);
    effects.shake(1.0);
    let mut clock = Clock::new(60.0).unwrap();
    effects.step(&tick(&mut clock));
    assert!(effects.offset().is_zero());
    assert_eq!(effects.camera(), IDENTITY);
    assert!(!effects.flash(Flash::new(Srgba::WHITE, 0.8, 0.05, 0.2)));
    let fast = Draw::rect([300.0, 300.0], [40.0, 40.0], 4.0)
        .fill(Srgba::WHITE)
        .blur([40.0, 0.0]);
    let (draws, _) = effects.compose(&[fast], Light::default());
    assert!(draws.iter().all(|draw| draw.fx.blur == [0.0, 0.0]));
    assert!(draws.len() > 1);
    effects.hit_stop(&mut clock, HitStop::new(0.0, 0.3, 0.3));
    assert!(clock.dip().is_none());
    let tape = Tape {
        strength: 0.8,
        ..Tape::default()
    };
    assert_eq!(effects.tape(tape).strength, 0.4);
    effects.comfort = Comfort::reduced(0.5);
    effects.shake(1.0);
    let half = effects.offset();
    effects.comfort = Comfort::FULL;
    let whole = effects.offset();
    assert!((half.translation[0] - whole.translation[0] * 0.5).abs() < 1e-5);
    let (draws, _) = effects.compose(&[fast], Light::default());
    assert_eq!(draws[0].fx.blur, [40.0, 0.0]);
}

#[test]
fn flashes_are_capped_at_three_a_second_whatever_the_game_asks() {
    let mut effects = Effects::new(EffectsDesc::new([1920.0, 1080.0], 1));
    let mut clock = Clock::new(60.0).unwrap();
    let mut starts = Vec::new();
    for _ in 0..(64 * 5) {
        if effects.flash(Flash::new(Srgba::WHITE, 0.6, 0.02, 0.05)) {
            starts.push(effects.real_time());
        }
        effects.step(&tick(&mut clock));
    }
    assert_eq!(starts.len(), 15);
    for (index, start) in starts.iter().enumerate() {
        assert!(
            starts[index..]
                .iter()
                .take_while(|at| **at < start + 1.0)
                .count()
                <= 3
        );
    }
    clock.set_speed(0.25);
    let mut slow = Effects::new(EffectsDesc::new([1920.0, 1080.0], 1));
    let mut count = 0;
    for _ in 0..64 {
        count += u32::from(slow.flash(Flash::new(Srgba::WHITE, 0.6, 0.0, 0.01)));
        slow.step(&tick(&mut clock));
    }
    assert_eq!(count, 3);
    let mut flashing = Effects::new(EffectsDesc::new([1920.0, 1080.0], 1));
    assert!(flashing.flash(Flash::new(Srgba::WHITE, 0.6, 0.1, 0.2)));
    let mut clock = Clock::new(60.0).unwrap();
    flashing.step(&tick(&mut clock));
    let (draws, _) = flashing.compose(&[], Light::default());
    assert_eq!(draws.len(), 1);
    let flash = draws[0];
    assert_eq!(flash.elevation, FLASH_ELEVATION);
    assert!(flash.fx.unpicked);
    assert!(flash.opacity > 0.0 && flash.opacity < 0.6);
}

#[test]
fn intensity_drives_every_effect_through_the_games_curves() {
    let layout = [1920.0, 1080.0];
    let quick = Response {
        gain: Curve::new(1.0, 3.0, Ease::Linear),
        rate: Curve::new(1.0, 2.0, Ease::Linear),
    };
    let mut effects = Effects::new(EffectsDesc {
        responses: Responses {
            shake: quick,
            burst: quick,
            ring: quick,
            flash: quick,
            blur: quick,
        },
        ..EffectsDesc::new(layout, 1)
    });
    let id = effects.recipe(sparks()).unwrap();
    assert_eq!(effects.burst(id, [0.0, 0.0]), 50);
    effects.intensity = 1.0;
    assert_eq!(effects.burst(id, [0.0, 0.0]), 150);
    assert_eq!(effects.motion([10.0, 0.0], 0.5), [15.0, 0.0]);
    effects.intensity = 0.5;
    assert_eq!(effects.motion([10.0, 0.0], 0.5), [10.0, 0.0]);
}

#[test]
fn additive_and_unpicked_draws_composite_and_pick_as_asked() {
    let size = [100, 100];
    let fit = fit(size);
    let glow = Draw::circle([50.0, 50.0], 20.0)
        .fill(Srgba::hex(0x3d8fd8))
        .additive()
        .unpicked()
        .id(9);
    let instance = pack_shape(&glow, &fit, None).unwrap();
    assert_eq!(
        instance.info[0] & (ADDITIVE | UNPICKED),
        ADDITIVE | UNPICKED
    );
    let paint = cpu::paint(&instance, None, instance.to_local([50.5, 50.5]));
    assert_eq!(paint.colour[3], 0.0);
    assert!(paint.colour[2] > 0.5);
    let card = Draw::rect([50.0, 50.0], [80.0, 80.0], 4.0)
        .fill(Srgba::hex(0x202020))
        .id(3);
    let curve = test_curve();
    let draws = [card, glow];
    let scene = scene([100.0, 100.0], &draws, &[], &curve, Light::default());
    assert_eq!(cpu::pick(&scene, None, size, [50, 50]).unwrap(), 3);
    let image = cpu::render(&scene, None, size).unwrap();
    let centre = image[50 * 100 + 50];
    let dark = 0x20 as f32 / 255.0;
    assert!((centre[2] - (dark + 0xd8 as f32 / 255.0)).abs() < 1e-3);
    assert_eq!(centre[3], 1.0);
    let fast = Draw::rect([50.0, 50.0], [20.0, 20.0], 2.0)
        .fill(Srgba::WHITE)
        .blur([60.0, 0.0])
        .id(5);
    let lone = [fast];
    let moving = FlatScene {
        draws: &lone,
        ..scene
    };
    let image = cpu::render(&moving, None, size).unwrap();
    assert!(image[50 * 100 + 75][3] > 0.1);
    assert_eq!(cpu::pick(&moving, None, size, [75, 50]).unwrap(), 0);
    assert_eq!(cpu::pick(&moving, None, size, [50, 50]).unwrap(), 5);
}

#[test]
fn shards_collide_follow_the_clock_and_leave_with_their_bodies() {
    let mut world = World::new(WorldDesc {
        rate: 60.0,
        gravity: [0.0, 900.0, 0.0],
        ..WorldDesc::default()
    });
    let floor = world.add_body(pfx_physics::rigid::BodyDesc::fixed(
        pfx_physics::rigid::Pose::at([960.0, 1000.0, 0.0]),
    ));
    world.add_collider(
        floor,
        pfx_physics::rigid::ColliderDesc::cuboid([2000.0, 20.0, 50.0]),
    );
    let mut shards = Shards::new(64, 3);
    let shatter = Shatter {
        count: 12,
        life: [3.0, 3.0],
        ..Shatter::new(Fill::Solid(Srgba::WHITE), [[6.0, 4.0], [12.0, 8.0]])
    };
    assert_eq!(shards.spawn(&mut world, &shatter, [960.0, 700.0]), 12);
    let mut clock = Clock::new(60.0).unwrap();
    clock.set_speed(0.75);
    let mut steps = Steps::new(f64::from(world.rate()), &clock);
    let mut taken = 0;
    for _ in 0..128 {
        let tick = tick(&mut clock);
        for _ in 0..steps.due(&clock) {
            world.step();
            taken += 1;
        }
        shards.step(&mut world, tick.dt);
    }
    assert_eq!(taken, 90);
    let mut out = Vec::new();
    shards.draws(&world, &mut out);
    assert_eq!(out.len(), 12);
    for draw in &out {
        assert!(draw.transform[3][1] < 985.0, "{}", draw.transform[3][1]);
        assert!(draw.fx.unpicked);
    }
    let bodies: Vec<_> = shards.bodies().collect();
    for _ in 0..256 {
        let tick = tick(&mut clock);
        for _ in 0..steps.due(&clock) {
            world.step();
        }
        shards.step(&mut world, tick.dt);
    }
    assert_eq!(shards.live(), 0);
    assert!(bodies.iter().all(|body| world.pose(*body).is_none()));
}

fn quantize(pixel: [f32; 4]) -> [u8; 4] {
    std::array::from_fn(|i| (pixel[i].clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn blur_draws() -> Vec<Draw> {
    let blue = Srgba::hex(0x3d8fd8);
    let ink = Srgba::hex(0x2b3442);
    vec![
        Draw::rect([200.0, 150.0], [380.0, 280.0], 18.0)
            .fill(Srgba::hex(0xf4f6f8))
            .id(1),
        Draw::rect([90.0, 70.0], [70.0, 40.0], 10.0)
            .fill(Srgba::hex(0xe2614c))
            .stroke(Stroke::solid(ink, 3.0))
            .elevation(1.0)
            .blur([36.0, 0.0])
            .id(2),
        Draw::circle([220.0, 70.0], 18.0)
            .fill(blue)
            .blur([20.0, 14.0])
            .id(3),
        Draw::new(Shape::Ring {
            radius: 16.0,
            width: 5.0,
        })
        .at([320.0, 80.0])
        .fill(ink)
        .blur([0.0, 30.0]),
        Draw::new(Shape::Arc {
            radius: 18.0,
            width: 6.0,
            start: -1.2,
            sweep: 3.6,
        })
        .at([80.0, 190.0])
        .fill(Srgba::hex(0x8e57b0))
        .blur([-24.0, 10.0]),
        Draw::new(Shape::Icon(Icon(0)))
            .transform(multiply(translation(190.0, 190.0), scaling(1.8, 1.8)))
            .stroke(Stroke::solid(blue, 6.0))
            .blur([10.0, -40.0])
            .id(4),
        Draw::new(Shape::Icon(Icon(1)))
            .transform(multiply(translation(260.0, 160.0), scaling(1.1, 1.1)))
            .fill(blue)
            .stroke(Stroke::solid(Srgba::WHITE, 5.0))
            .blur([28.0, 18.0]),
        Draw::rect([0.0, 0.0], [60.0, 30.0], 8.0)
            .transform(place([330.0, 220.0], 0.4))
            .fill(Srgba::hex(0x138f8a))
            .material(super::super::Material {
                thickness: 4.0,
                bevel: 1.5,
                gloss: 0.3,
                ..Default::default()
            })
            .blur([-30.0, 0.0]),
        Draw::rect([110.0, 250.0], [80.0, 26.0], 8.0)
            .stroke(Stroke::dashed(ink, 2.0, 8.0, 5.0))
            .blur([18.0, 0.0]),
        Draw::circle([230.0, 255.0], 22.0)
            .fill(Srgba::hex(0xf0b429))
            .additive(),
        Draw::rect([330.0, 140.0], [40.0, 40.0], 6.0)
            .fill(Srgba::hex(0x556178).alpha(0.7))
            .unpicked()
            .id(7),
    ]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_gpu_blurs_and_blends_like_its_twin() {
    use super::super::tests::{gpu, gpu_render};
    use super::super::{FlatIcons, FlatPass};
    let gpu = gpu();
    let icons = Icons::bake(&[icons::tick(), icons::pointer()]).unwrap();
    let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons.clone());
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = test_curve();
    let draws = blur_draws();
    let mut report = String::new();
    let mut misses = Vec::new();
    for scale in [1u32, 2] {
        let size = [400 * scale, 300 * scale];
        let mut scene = scene([400.0, 300.0], &draws, &[], &curve, Light::default());
        scene.icons = Some(&flat_icons);
        let (pixels, ids) = gpu_render(&gpu, &mut pass, &scene, size);
        let twin = cpu::render(&scene, Some(&icons), size).unwrap();
        let (mut sum, mut worst) = (0.0f64, 0u8);
        for (index, (ours, theirs)) in pixels.iter().zip(&twin).enumerate() {
            let (a, b) = (quantize(*ours), quantize(*theirs));
            if (0..4).any(|i| a[i].abs_diff(b[i]) > 2) {
                eprintln!(
                    "{scale}x at {} {}: gpu {a:?} twin {b:?}",
                    index as u32 % size[0] / scale,
                    index as u32 / size[0] / scale
                );
            }
            for i in 0..4 {
                let d = a[i].abs_diff(b[i]);
                sum += f64::from(d);
                worst = worst.max(d);
            }
        }
        let mean = sum / (pixels.len() * 4) as f64;
        report += &format!("{scale}x: twin mean {mean:.4}/255 worst {worst}/255\n");
        if mean >= 0.15 || worst > 2 {
            misses.push(format!("{scale}x mean {mean:.4} worst {worst}"));
        }
        let at = |x: u32, y: u32| ids[((y * scale) * size[0] + x * scale) as usize];
        assert_eq!(at(330, 140), 1, "an unpicked draw leaves the card's id");
        assert_eq!(at(90, 70), 2);
        assert_eq!(at(220, 70), 3);
        let pick = cpu::pick(&scene, Some(&icons), size, [330 * scale, 140 * scale]).unwrap();
        assert_eq!(pick, 1);
    }
    eprintln!("{report}");
    assert!(misses.is_empty(), "{misses:?}\n{report}");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_frame_without_effects_keeps_its_bytes_after_effects_ran() {
    use super::super::tests::{gpu, gpu_render};
    use super::super::{FlatIcons, FlatPass};
    let gpu = gpu();
    let (parsed, icons) = board();
    let flat_icons = FlatIcons::new(&gpu.device, &gpu.queue, icons.clone());
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    let curve = test_curve();
    let size = [1920, 1080];
    let plain = |pass: &mut FlatPass, draws: &[Draw], light: Light| {
        let mut scene = scene(parsed.size, draws, &parsed.groups, &curve, light);
        scene.icons = Some(&flat_icons);
        gpu_render(&gpu, pass, &scene, size)
    };
    let before = plain(&mut pass, &parsed.draws, Light::default());
    let mut effects = Effects::new(EffectsDesc {
        shake: ShakeTuning {
            translation: [16.0, 10.0],
            roll: 0.01,
            frequency: 20.0,
            decay: 1.0,
            seed: 2,
        },
        ..EffectsDesc::new(parsed.size, 5)
    });
    let id = effects
        .recipe(Burst {
            additive: true,
            ..sparks()
        })
        .unwrap();
    let mut clock = Clock::new(60.0).unwrap();
    effects.burst(id, [900.0, 500.0]);
    effects.shake(0.8);
    effects.flash(Flash::new(Srgba::WHITE, 0.4, 0.05, 0.2));
    effects.step(&tick(&mut clock));
    let (draws, light) = effects.compose(&parsed.draws, Light::default());
    let draws = draws.to_vec();
    let during = plain(&mut pass, &draws, light);
    assert_ne!(
        during
            .0
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>(),
        before
            .0
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>()
    );
    let mut calm = Effects::new(EffectsDesc::new(parsed.size, 5));
    let (draws, light) = calm.compose(&parsed.draws, Light::default());
    let draws = draws.to_vec();
    let after = plain(&mut pass, &draws, light);
    let bits = |image: &[[f32; 4]]| {
        image
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    assert!(bits(&before.0) == bits(&after.0), "colour bytes changed");
    assert!(before.1 == after.1, "ids changed");
    let mut fresh = FlatPass::new(&gpu.device, &gpu.queue);
    let again = plain(&mut fresh, &parsed.draws, Light::default());
    assert!(bits(&before.0) == bits(&again.0));
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_burst_heavy_frame_fits_at_4k_and_on_a_deck_sized_target() {
    use crate::renderer::Renderer;
    use pfx_gpu::{Gpu, wgpu};
    let (mut parsed, _) = board();
    for draw in &mut parsed.draws {
        draw.group = None;
    }
    let mut shapes = parsed.icons.clone();
    shapes.push(icons::tick());
    let icons = Icons::bake(&shapes).unwrap();
    let check = Icon(parsed.icons.len() as u32);
    let curve = test_curve();
    let colours = [0xe2614c, 0x8e57b0, 0xf0b429, 0x138f8a, 0x3d8fd8].map(Srgba::hex);
    let recipes = [
        Burst {
            count: 40,
            speed: [150.0, 520.0],
            life: [0.8, 1.2],
            gravity: [0.0, 500.0],
            drag: 0.8,
            spin: [1.0, 9.0],
            size: [0.6, 1.4],
            scale: Curve::new(1.0, 0.4, Ease::QuadIn),
            ..Burst::new(Shape::Circle { radius: 5.0 }, Fill::Solid(colours[2]))
        },
        Burst {
            count: 30,
            speed: [100.0, 420.0],
            life: [0.8, 1.2],
            gravity: [0.0, 700.0],
            drag: 0.5,
            spin: [2.0, 12.0],
            size: [0.8, 1.6],
            shutter: 1.0 / 60.0,
            ..Burst::new(
                Shape::Rect {
                    half: [7.0, 4.0],
                    radii: [1.5; 4],
                },
                Fill::Solid(colours[0]),
            )
        },
        Burst {
            count: 15,
            speed: [60.0, 200.0],
            life: [0.8, 1.2],
            size: [0.8, 1.2],
            additive: true,
            colour: Some(Fade {
                from: colours[4],
                to: colours[1],
                ease: Ease::Linear,
            }),
            ..Burst::new(
                Shape::Ring {
                    radius: 6.0,
                    width: 2.0,
                },
                Fill::Solid(colours[4]),
            )
        },
    ];
    let mut report = String::new();
    let mut misses = Vec::new();
    for size in [[3840u32, 2160u32], [1280, 800]] {
        let mut turns = pfx_gpu::pace::Turns::default();
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let mut renderer = turns.cpu(|| Renderer::new(gpu, size[0], size[1]).unwrap());
        let target = renderer
            .gpu()
            .offscreen(size[0], size[1], wgpu::TextureFormat::Rgba16Float)
            .unwrap();
        let flat_icons = super::super::FlatIcons::new(
            &renderer.gpu().device,
            &renderer.gpu().queue,
            icons.clone(),
        );
        for (variant, [bursting, streaking, blurring, shaking]) in [
            ("warm-up", [true; 4]),
            ("board and 500 still checks", [false; 4]),
            ("blur on 500 checks", [false, false, true, false]),
            ("5,000 particles", [true, false, false, false]),
            (
                "5,000 particles, a third blurred",
                [true, true, false, false],
            ),
            ("5,000 particles, shake, blur on 500 checks", [true; 4]),
        ] {
            let active = bursting && streaking && blurring && shaking;
            turns.poll();
            let mut effects = Effects::new(EffectsDesc {
                particles: 6000,
                shake: ShakeTuning {
                    translation: [18.0, 12.0],
                    roll: 0.012,
                    frequency: 22.0,
                    decay: 0.6,
                    seed: 8,
                },
                ..EffectsDesc::new(parsed.size, 21)
            });
            let ids: Vec<_> = recipes
                .iter()
                .map(|r| {
                    effects
                        .recipe(Burst {
                            shutter: if streaking { r.shutter } else { 0.0 },
                            ..*r
                        })
                        .unwrap()
                })
                .collect();
            let mut clock = Clock::new(60.0).unwrap();
            let mut game = Vec::new();
            let mut samples = Vec::new();
            let mut live = 0;
            let mut drawn = 0;
            for frame in 0..150u32 {
                turns.poll();
                let tick = clock.advance(1.0 / 60.0);
                if bursting {
                    for (k, id) in ids.iter().enumerate() {
                        let x = 300.0 + ((frame * 7 + k as u32 * 131) % 1300) as f32;
                        effects.burst(*id, [x, 300.0 + (k as f32) * 150.0]);
                    }
                }
                if shaking && frame % 30 == 0 {
                    effects.shake(0.6);
                }
                effects.step(&tick);
                game.clear();
                game.extend_from_slice(&parsed.draws);
                let t = clock.time() as f32;
                for c in 0..500u32 {
                    let lane = 420.0 + (c % 5) as f32 * 260.0;
                    let phase = ((c as f32 * 0.618_034) + t * 0.6).fract();
                    let x = lane + ((c * 37) % 150) as f32 - 75.0;
                    let y = 960.0 - phase * 820.0;
                    let mut draw = Draw::new(Shape::Icon(check))
                        .transform(multiply(translation(x, y), scaling(0.8, 0.8)))
                        .stroke(Stroke::solid(colours[4], 7.0))
                        .elevation(1.5)
                        .shadow(Shadow::None);
                    if blurring {
                        draw = draw.blur(effects.motion([0.0, -0.6 * 820.0], 1.0 / 60.0));
                    }
                    game.push(draw);
                }
                live = effects.live();
                let (draws, light) = effects.compose(&game, Light::default());
                drawn = draws.len();
                let scene = FlatScene {
                    layout: parsed.size,
                    clear: Some(Srgba::hex(0xeceff3)),
                    curve: &curve,
                    light,
                    draws,
                    groups: &[],
                    text: &[],
                    icons: Some(&flat_icons),
                    sprites: None,
                    environment: None,
                    post: false,
                    frame,
                    seed: 0,
                };
                turns.poll();
                let timings = renderer.render_flat(&scene, &target.view).unwrap();
                let of = |label: &str| {
                    timings
                        .iter()
                        .filter(|timing| timing.label == label)
                        .map(|timing| timing.milliseconds)
                        .sum::<f64>()
                };
                let (colour, ids) = (of("flat"), of("flat ids"));
                turns.add(colour + ids);
                if frame >= 70 && !timings.is_empty() {
                    samples.push((colour, ids, colour + ids));
                }
            }
            if variant == "warm-up" {
                continue;
            }
            assert!(samples.len() >= 60, "GPU timestamps are unavailable");
            let mut colour = samples.iter().map(|s| s.0).collect::<Vec<_>>();
            let mut ids = samples.iter().map(|s| s.1).collect::<Vec<_>>();
            let mut total = samples.iter().map(|s| s.2).collect::<Vec<_>>();
            let (colour, ids, total) = (median(&mut colour), median(&mut ids), median(&mut total));
            report += &format!(
                "{}x{} {variant}: flat {colour:.3} ms, flat ids {ids:.3} ms, total {total:.3} ms ({drawn} draws, {live} particles)\n",
                size[0], size[1]
            );
            if active && size[0] == 1280 && total * 16.0 > 16.7 {
                misses.push(format!("Deck estimate {:.2} ms at 16x", total * 16.0));
            }
            if bursting {
                assert!(live >= 4500, "{live} particles");
            }
        }
    }
    eprintln!("{report}");
    assert!(misses.is_empty(), "{misses:?}\n{report}");
}
