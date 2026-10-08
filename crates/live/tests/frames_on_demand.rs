#[allow(dead_code)]
#[path = "support/desk.rs"]
mod desk;

use desk::{Desk, Pose, Show, error};
use pfx_gpu::Gpu;
use pfx_live::demand::{self, Limits, Need, Region, SETTLE};
use pfx_live::lights::LocalLight;
use pfx_live::renderer::{Exposure, Renderer};
use pfx_post::{Style, Tape};

const W: u32 = 480;
const H: u32 = 300;

fn inside(regions: &[Region], index: usize, width: u32) -> bool {
    let x = (index as u32) % width;
    let y = (index as u32) / width;
    regions.iter().any(|region| region.contains(x, y))
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn frames_on_demand_need_an_srgb_output() {
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut renderer = Renderer::new(gpu, 64, 64).unwrap();
    assert!(
        renderer
            .set_frames_on_demand(Some(Limits::default()))
            .is_err()
    );
    assert_eq!(renderer.frames_on_demand(), None);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn dirty_tracking_sees_each_kind_of_change() {
    let mut desk = Desk::new(W, H);
    let still = Pose::default();
    let show = Show {
        overlay: true,
        ..Show::default()
    };
    assert_eq!(desk.settle(still, 1.0, show), 1 + SETTLE);
    assert_eq!(desk.needed(still, 1.0, show), Need::None);
    assert_eq!(
        desk.needed(still, 7.5, show),
        Need::None,
        "time alone changes nothing"
    );
    desk.renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    desk.renderer.set_haze(None);
    desk.renderer.set_lens(None);
    desk.renderer.set_tape(None);
    assert_eq!(
        desk.needed(still, 1.0, show),
        Need::None,
        "setting a value it already has changes nothing"
    );
    let turned = Pose { yaw: 0.1, ..still };
    assert_eq!(desk.needed(turned, 1.0, show), Need::Reproject);
    let swung = Pose { yaw: 3.0, ..still };
    assert_eq!(desk.needed(swung, 1.0, show), Need::Full);
    let zoomed = Pose {
        zoom: 0.01,
        ..still
    };
    assert_eq!(desk.needed(zoomed, 1.0, show), Need::Reproject);
    let far_zoom = Pose { zoom: 0.1, ..still };
    assert_eq!(desk.needed(far_zoom, 1.0, show), Need::Full);
    for (label, changed) in [
        (
            "an instance moved",
            Show {
                card_shift: 0.01,
                ..show
            },
        ),
        (
            "a material changed",
            Show {
                card_tint: 0.1,
                ..show
            },
        ),
        (
            "surface text moved",
            Show {
                text_shift: 0.01,
                ..show
            },
        ),
        ("content changed", Show { content: 1, ..show }),
    ] {
        assert_eq!(desk.needed(still, 1.0, changed), Need::Full, "{label}");
    }
    let whole = Need::Partial(vec![Region::whole(W, H)]);
    let relabelled = Show {
        overlay_revision: 1,
        ..show
    };
    assert_eq!(desk.needed(still, 1.0, relabelled), whole);
    let bare = Show {
        overlay: false,
        ..show
    };
    assert_eq!(desk.needed(still, 1.0, bare), whole);
    let steaming = Show {
        steam: true,
        ..show
    };
    let steam = desk.region_of_steam(still);
    assert_eq!(
        desk.needed(still, 1.0, steaming),
        Need::Full,
        "the first steam keeps a still layer"
    );
    desk.draw(still, 1.0, steaming, None);
    match desk.needed(still, 1.1, steaming) {
        Need::Partial(regions) => {
            assert_eq!(regions.len(), 1);
            assert_eq!(regions[0], steam);
            assert!(steam.area() * 4 < Region::whole(W, H).area(), "{steam:?}");
        }
        other => panic!("steam needs its rect, not {other:?}"),
    }

    desk.settle(still, 1.0, show);
    type Change = Box<dyn Fn(&mut Renderer)>;
    let changes: Vec<(&str, Change)> = vec![
        (
            "lights",
            Box::new(|renderer: &mut Renderer| {
                renderer
                    .set_lights(&[LocalLight {
                        position: [0.0, 1.4, 0.3],
                        colour: [1.0, 0.9, 0.8],
                        intensity: 2.0,
                        radius: 0.05,
                        range: 3.0,
                        shadow: false,
                    }])
                    .unwrap()
            }),
        ),
        (
            "finish",
            Box::new(|renderer: &mut Renderer| renderer.set_finish(Style::Film.chain())),
        ),
        (
            "sharpen",
            Box::new(|renderer: &mut Renderer| renderer.set_sharpen(0.5).unwrap()),
        ),
        (
            "plain finish",
            Box::new(|renderer: &mut Renderer| renderer.clear_finish()),
        ),
        (
            "exposure",
            Box::new(|renderer: &mut Renderer| {
                renderer.set_exposure(Exposure::Fixed(1.2)).unwrap()
            }),
        ),
        (
            "sky visibility",
            Box::new(|renderer: &mut Renderer| renderer.set_sky_visibility(false)),
        ),
        (
            "reflection occlusion",
            Box::new(|renderer: &mut Renderer| renderer.set_reflection_occlusion(0.4).unwrap()),
        ),
    ];
    for (label, change) in &changes {
        change(&mut desk.renderer);
        assert_eq!(desk.needed(still, 1.0, show), Need::Full, "{label}");
        assert_eq!(desk.settle(still, 1.0, show), 1 + SETTLE, "{label}");
        assert_eq!(desk.needed(still, 1.0, show), Need::None, "{label}");
    }
    desk.renderer.set_tape(Some(Tape::forward(1.0)));
    for _ in 0..3 {
        assert_eq!(
            desk.draw(still, 1.0, show, None).0,
            Need::Full,
            "a running tape"
        );
    }
    desk.renderer.set_tape(None);
    assert_eq!(desk.settle(still, 1.0, show), 1 + SETTLE);
    desk.renderer
        .set_exposure(Exposure::Auto { bias: 0.0 })
        .unwrap();
    let mut adapting = 0;
    loop {
        let time = 1.0 + adapting as f32 / 60.0;
        if desk.draw(still, time, show, None).0 == Need::None {
            break;
        }
        adapting += 1;
        assert!(adapting < 600, "auto exposure never settles");
    }
    println!("auto exposure drew {adapting} frames before going idle");
    assert!(
        adapting > 1 + SETTLE,
        "auto exposure keeps drawing while it adapts"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn idle_frames_draw_nothing_and_present_the_last_image() {
    let mut desk = Desk::new(W, H);
    let still = Pose::default();
    let show = Show {
        overlay: true,
        ..Show::default()
    };
    desk.settle(still, 1.0, show);
    let shown = desk.pixels();
    let submitted = desk.renderer.frames_submitted();
    for tick in 0..120 {
        let (need, timings) = desk.draw(still, 1.0 + tick as f32 / 60.0, show, None);
        assert_eq!(need, Need::None);
        assert!(timings.is_empty());
    }
    assert_eq!(desk.renderer.frames_submitted(), submitted);
    let output = desk.output.view.clone();
    desk.renderer.present_last(&output).unwrap();
    assert_eq!(desk.pixels(), shown);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn reprojecting_onto_the_cached_camera_is_exact() {
    for overlay in [true, false] {
        let mut desk = Desk::new(W, H);
        let still = Pose::default();
        let show = Show {
            overlay,
            ..Show::default()
        };
        desk.settle(still, 1.0, show);
        let shown = desk.pixels();
        let (_, timings) = desk.draw(still, 1.1, show, Some(Need::Reproject));
        assert!(timings.iter().any(|pass| pass.label == "reproject"));
        assert!(!desk.renderer.last_pass_order().contains(&"post"));
        let warped = desk.pixels();
        let differ = shown.iter().zip(&warped).filter(|(a, b)| a != b).count();
        let spots: Vec<(usize, usize, [u8; 4], [u8; 4])> = shown
            .chunks_exact(4)
            .zip(warped.chunks_exact(4))
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(12)
            .map(|(index, (a, b))| {
                (
                    index % W as usize,
                    index / W as usize,
                    a.try_into().unwrap(),
                    b.try_into().unwrap(),
                )
            })
            .collect();
        println!("overlay {overlay}: {differ} bytes, {spots:?}");
        assert_eq!(differ, 0, "overlay {overlay}: {differ} bytes moved");
        let output = desk.output.view.clone();
        desk.renderer.present_last(&output).unwrap();
        assert_eq!(desk.pixels(), shown);
    }
}

fn fresh(desk: &mut Desk, pose: Pose, show: Show, time: f32) -> Vec<u8> {
    for frame in 0..2 * SETTLE {
        desk.draw(pose, time + frame as f32 / 60.0, show, Some(Need::Full));
    }
    desk.pixels()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn reprojection_error_stays_small_inside_the_limits() {
    let (width, height) = (1280, 800);
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut reference = Desk::with_gpu(gpu.clone(), width, height);
    let mut warped = Desk::with_gpu(gpu, width, height);
    let base = Pose::default();
    let show = Show::default();
    let start = 1.0;
    let cached = fresh(&mut warped, base, show, start);
    let again = fresh(&mut reference, base, show, start);
    assert_eq!(cached, again, "two desks draw the same frame");
    let floor = error(&cached, &fresh(&mut reference, base, show, start + 0.5));
    println!(
        "TAA floor: mean {:.3}, visible {:.4}%",
        floor.mean,
        floor.visible * 100.0
    );
    let limits = Limits::default();
    let depth = loop {
        if let Some(depth) = warped.renderer.demand_stats().unwrap().depth {
            break depth;
        }
        warped.draw(base, start, show, Some(Need::Reproject));
    };
    println!("probe depth {depth:.3} m");
    let mut rows = Vec::new();
    let sweep: Vec<(&str, f32, Pose)> = [0.05, 0.1, 0.2, 0.3, 0.45, 0.6, 0.8, 1.2]
        .into_iter()
        .map(|degrees| {
            (
                "orbit yaw",
                degrees,
                Pose {
                    yaw: degrees,
                    ..base
                },
            )
        })
        .chain([0.1, 0.25, 0.5].into_iter().map(|degrees| {
            (
                "orbit pitch",
                degrees,
                Pose {
                    pitch: degrees,
                    ..base
                },
            )
        }))
        .chain(
            [0.002, 0.005, 0.01, 0.02]
                .into_iter()
                .map(|push| ("push", push, Pose { push, ..base })),
        )
        .chain(
            [0.005, 0.01, 0.02, 0.04]
                .into_iter()
                .map(|zoom| ("zoom", zoom, Pose { zoom, ..base })),
        )
        .collect();
    for (kind, amount, pose) in sweep {
        let truth = fresh(&mut reference, pose, show, start);
        warped.draw(pose, start, show, Some(Need::Reproject));
        let moved = warped.pixels();
        warped.draw(pose, start, show, Some(Need::Reproject));
        let holes = warped.renderer.demand_stats().unwrap().holes;
        let from = warped.camera(base);
        let to = warped.camera(pose);
        let motion = demand::motion(
            (&from.view, &from.projection, from.position),
            (&to.view, &to.projection, to.position),
            Some(depth),
        )
        .unwrap();
        let measured = error(&truth, &moved);
        let accepted = motion.angle <= limits.angle && motion.zoom <= limits.zoom;
        println!(
            "{kind} {amount}: angle {:.3}°, zoom {:.4}, holes {:.4}%, mean {:.3}, visible {:.3}%, worst {}, {}",
            motion.angle.to_degrees(),
            motion.zoom,
            holes * 100.0,
            measured.mean,
            measured.visible * 100.0,
            measured.worst,
            if accepted { "reprojects" } else { "redraws" }
        );
        rows.push((kind, amount, accepted, measured, holes));
    }
    for (kind, amount, accepted, measured, holes) in &rows {
        if *accepted && *holes <= limits.holes {
            assert!(
                measured.mean < 1.5 && measured.visible < 0.01,
                "{kind} {amount}: mean {:.3}, visible {:.4}",
                measured.mean,
                measured.visible
            );
        }
    }
    let yaw: Vec<f64> = rows
        .iter()
        .filter(|row| row.0 == "orbit yaw")
        .map(|row| row.3.mean)
        .collect();
    assert!(
        yaw.first() < yaw.last(),
        "error grows with the drift: {yaw:?}"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn partial_redraw_matches_a_full_frame_inside_its_rects_and_leaves_the_rest() {
    let still = Pose::default();
    let show = Show {
        steam: true,
        overlay: true,
        ..Show::default()
    };
    let cycle = SETTLE as f32 / 60.0;
    let t0 = 2.0;
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut partial = Desk::with_gpu(gpu.clone(), W, H);
    partial
        .renderer
        .set_frames_on_demand(Some(Limits {
            settle: 0,
            ..Limits::default()
        }))
        .unwrap();
    partial.draw(still, t0, show, Some(Need::Full));
    let before = partial.pixels();
    let mut previous = before.clone();
    for step in 1..=3 {
        let time = t0 + cycle * step as f32;
        let need = partial.needed(still, time, show);
        let Need::Partial(regions) = need.clone() else {
            panic!("steam alone needs a partial frame, not {need:?}");
        };
        assert!(
            regions.iter().map(Region::area).sum::<u64>() * 4 < Region::whole(W, H).area(),
            "{regions:?}"
        );
        partial.draw(still, time, show, Some(need));
        assert!(
            !partial
                .renderer
                .last_pass_order()
                .iter()
                .any(|pass| *pass == "opaque" || *pass == "TAA"),
            "{:?}",
            partial.renderer.last_pass_order()
        );
        let redrawn = partial.pixels();
        let mut full = Desk::with_gpu(gpu.clone(), W, H);
        full.draw(still, time, show, Some(Need::Full));
        let expected = full.pixels();
        let mut changed = 0;
        for (index, (got, want)) in redrawn
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let old = &previous[index * 4..index * 4 + 4];
            if inside(&regions, index, W) {
                assert_eq!(
                    got, want,
                    "step {step}: pixel {index} inside differs from a full frame"
                );
                changed += usize::from(got != old);
            } else {
                assert_eq!(got, old, "step {step}: pixel {index} outside was touched");
                assert_eq!(
                    got, want,
                    "step {step}: pixel {index} outside differs from a full frame"
                );
            }
        }
        assert!(
            changed > 50,
            "step {step}: the steam moved {changed} pixels"
        );
        previous = redrawn;
    }
    let gone = Show {
        steam: false,
        ..show
    };
    let need = partial.needed(still, t0, gone);
    assert!(matches!(need, Need::Partial(_)), "{need:?}");
    partial.draw(still, t0 + 4.0 * cycle, gone, Some(need));
    let mut clear = Desk::with_gpu(gpu, W, H);
    clear.draw(still, t0, gone, Some(Need::Full));
    assert_eq!(partial.pixels(), clear.pixels(), "steam leaves no trace");
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn partial_redraw_under_a_reaching_finish_grows_its_rects_and_matches_a_full_frame() {
    let still = Pose::default();
    let show = Show {
        steam: true,
        overlay: true,
        ..Show::default()
    };
    let cycle = SETTLE as f32 / 60.0;
    let t0 = 2.0;
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut partial = Desk::with_gpu(gpu.clone(), W, H);
    partial
        .renderer
        .set_frames_on_demand(Some(Limits {
            settle: 0,
            ..Limits::default()
        }))
        .unwrap();
    partial.renderer.set_finish(desk::kuwahara_finish());
    partial.draw(still, t0, show, Some(Need::Full));
    let grown = partial.region_of_steam(still).grow(3, W, H);
    let mut previous = partial.pixels();
    for step in 1..=3 {
        let time = t0 + cycle * step as f32;
        let need = partial.needed(still, time, show);
        assert_eq!(need, Need::Partial(vec![grown]), "step {step}");
        partial.draw(still, time, show, Some(need));
        let order = partial.renderer.last_pass_order();
        assert!(
            !order.iter().any(|pass| *pass == "opaque" || *pass == "TAA"),
            "{order:?}"
        );
        let redrawn = partial.pixels();
        let mut full = Desk::with_gpu(gpu.clone(), W, H);
        full.renderer.set_finish(desk::kuwahara_finish());
        full.draw(still, time, show, Some(Need::Full));
        let expected = full.pixels();
        let mut changed = 0;
        let mut worst_inside = 0u8;
        let mut worst_outside = 0u8;
        for (index, (got, want)) in redrawn
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let old = &previous[index * 4..index * 4 + 4];
            let apart = got
                .iter()
                .zip(want)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            if grown.contains(index as u32 % W, index as u32 / W) {
                worst_inside = worst_inside.max(apart);
                changed += usize::from(got != old);
            } else {
                assert_eq!(got, old, "step {step}: pixel {index} outside was touched");
                worst_outside = worst_outside.max(apart);
            }
        }
        println!(
            "step {step}: {grown:?}, {changed} pixels moved, worst inside {worst_inside}/255, outside {worst_outside}/255"
        );
        assert!(worst_inside <= 1, "step {step}: {worst_inside}/255 inside");
        assert!(
            worst_outside <= 1,
            "step {step}: {worst_outside}/255 outside"
        );
        assert!(
            changed > 50,
            "step {step}: the steam moved {changed} pixels"
        );
        previous = redrawn;
    }
    partial.renderer.set_lens(Some(pfx_post::dof::Lens {
        focal_m: 0.05,
        sensor_m: 0.036,
        fstop: 2.8,
        aperture: 0.05 / 2.8,
        focus_m: 1.2,
        near_m: 0.0,
        near_blur: 0.0,
        far_m: 0.0,
        far_blur: 0.0,
        ground_m: None,
    }));
    partial.draw(still, t0, show, Some(Need::Full));
    assert_eq!(
        partial.needed(still, t0 + cycle, show),
        Need::Partial(vec![Region::whole(W, H)]),
        "a lens reads beyond the reach"
    );
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn overlay_changes_recomposite_without_drawing_the_scene() {
    let still = Pose::default();
    let with = Show {
        overlay: true,
        ..Show::default()
    };
    let without = Show::default();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let mut desk = Desk::with_gpu(gpu.clone(), W, H);
    desk.settle(still, 1.0, with);
    let labelled = desk.pixels();
    let mut plain = Desk::with_gpu(gpu, W, H);
    plain.settle(still, 1.0, without);
    let bare = plain.pixels();
    assert_ne!(labelled, bare);
    let (need, _) = desk.draw(still, 1.0, without, None);
    assert_eq!(need, Need::Partial(vec![Region::whole(W, H)]));
    assert!(!desk.renderer.last_pass_order().contains(&"opaque"));
    assert_eq!(desk.pixels(), bare);
    let (need, _) = desk.draw(
        still,
        1.0,
        Show {
            overlay_revision: 3,
            ..with
        },
        None,
    );
    assert_eq!(need, Need::Partial(vec![Region::whole(W, H)]));
    assert_eq!(desk.pixels(), labelled);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn drift_reprojects_and_refreshes_within_its_limits() {
    let mut desk = Desk::new(W, H);
    let show = Show {
        steam: true,
        overlay: true,
        ..Show::default()
    };
    let limits = Limits::default();
    let mut kinds = [0u32; 3];
    let mut since = 0u32;
    for tick in 0..600 {
        let time = 1.0 + tick as f32 / 60.0;
        let (need, _) = desk.draw(Pose::drift(time), time, show, None);
        match need {
            Need::Full => {
                kinds[0] += 1;
                since = 0;
            }
            Need::Reproject => {
                kinds[1] += 1;
                since += 1;
                assert!(since <= limits.frames);
            }
            Need::Partial(_) => kinds[2] += 1,
            Need::None => panic!("a drifting camera with steam always draws"),
        }
    }
    println!(
        "full {}, reproject {}, partial {}",
        kinds[0], kinds[1], kinds[2]
    );
    assert!(kinds[1] > kinds[0] * 4, "{kinds:?}");
}
