use super::*;

const DT: f32 = 1.0 / 60.0;

struct Wall {
    distance: f32,
}

impl Probe for Wall {
    fn clear(&self, _from: Vec3, _direction: Vec3, _radius: f32, max: f32) -> f32 {
        max.min(self.distance)
    }
}

fn follow(tuning: FollowTuning) -> Follow {
    Follow::new(tuning)
}

fn flat() -> FollowTuning {
    FollowTuning {
        damping: [0.0; 3],
        look_ahead_damping: 0.0,
        ..FollowTuning::follow3()
    }
}

fn run(rig: &mut Follow, target: Vec3, ticks: usize) {
    for _ in 0..ticks {
        rig.tick(DT, &Input::at(target), Comfort::FULL);
    }
}

fn near(a: f32, b: f32, within: f32) {
    assert!((a - b).abs() <= within, "{a} is not within {within} of {b}");
}

#[test]
fn a_target_inside_the_dead_zone_leaves_the_camera_where_it_is() {
    let mut rig = follow(FollowTuning {
        dead_zone: [2.0, 1.0, 0.0],
        ..flat()
    });
    run(&mut rig, [0.0; 3], 2);
    run(&mut rig, [1.5, -0.8, 0.0], 30);
    assert_eq!(rig.focus(), [0.0; 3]);
}

#[test]
fn a_target_past_the_dead_zone_drags_the_camera_by_the_excess_only() {
    let mut rig = follow(FollowTuning {
        dead_zone: [2.0, 1.0, 0.0],
        ..flat()
    });
    run(&mut rig, [0.0; 3], 2);
    run(&mut rig, [5.0, 3.0, 0.0], 5);
    near(rig.focus()[0], 3.0, 1e-5);
    near(rig.focus()[1], 2.0, 1e-5);
}

#[test]
fn a_target_offset_moves_what_the_camera_centres_on() {
    let mut rig = follow(FollowTuning {
        offset: [0.0, 2.0, 0.0],
        ..flat()
    });
    run(&mut rig, [1.0, 0.0, 0.0], 3);
    assert_eq!(rig.focus(), [1.0, 2.0, 0.0]);
}

#[test]
fn world_bounds_hold_the_camera_whatever_the_target_does() {
    let mut rig = follow(FollowTuning {
        bounds: Some(Bounds::new([-5.0, -2.0, -1.0], [5.0, 2.0, 1.0])),
        ..flat()
    });
    for step in 0..400 {
        let swing = (step as f32 * 0.07).sin() * 40.0;
        rig.tick(DT, &Input::at([swing, swing * 0.5, swing]), Comfort::FULL);
        let focus = rig.focus();
        assert!((-5.0..=5.0).contains(&focus[0]));
        assert!((-2.0..=2.0).contains(&focus[1]));
        assert!((-1.0..=1.0).contains(&focus[2]));
    }
}

#[test]
fn bounds_keep_the_whole_view_inside_when_it_has_a_size() {
    let mut rig = follow(FollowTuning {
        bounds: Some(Bounds::new([0.0, 0.0, 0.0], [20.0, 10.0, 0.0])),
        ..FollowTuning {
            damping: [0.0; 3],
            ..FollowTuning::follow2()
        }
    });
    rig.set_view_half([8.0, 4.5]);
    run(&mut rig, [-30.0, -30.0, 0.0], 3);
    assert_eq!(rig.focus(), [8.0, 4.5, 0.0]);
    run(&mut rig, [90.0, 90.0, 0.0], 3);
    assert_eq!(rig.focus(), [12.0, 5.5, 0.0]);
}

#[test]
fn a_view_bigger_than_the_bounds_centres_on_them() {
    let mut rig = follow(FollowTuning {
        bounds: Some(Bounds::new([0.0, 0.0, 0.0], [10.0, 6.0, 0.0])),
        ..FollowTuning::follow2()
    });
    rig.set_view_half([8.0, 4.5]);
    run(&mut rig, [3.0, 3.0, 0.0], 3);
    assert_eq!(rig.focus(), [5.0, 3.0, 0.0]);
}

#[test]
fn damping_converges_on_the_target_without_overshooting() {
    let mut rig = follow(FollowTuning {
        damping: [0.3, 0.3, 0.3],
        ..FollowTuning::follow3()
    });
    run(&mut rig, [0.0; 3], 2);
    let mut last = 0.0;
    for _ in 0..600 {
        rig.tick(DT, &Input::at([10.0, 0.0, 0.0]), Comfort::FULL);
        let x = rig.focus()[0];
        assert!(x >= last && x <= 10.0 + 1e-4);
        last = x;
    }
    near(last, 10.0, 1e-3);
}

#[test]
fn damping_runs_per_axis() {
    let mut rig = follow(FollowTuning {
        damping: [0.0, 1.0, 0.1],
        ..FollowTuning::follow3()
    });
    run(&mut rig, [0.0; 3], 2);
    run(&mut rig, [4.0, 4.0, 4.0], 6);
    let focus = rig.focus();
    near(focus[0], 4.0, 1e-5);
    assert!(focus[1] < focus[2] && focus[2] < focus[0]);
}

#[test]
fn damping_covers_the_same_ground_at_any_tick_rate() {
    let at = |rate: f32| {
        let mut rig = follow(FollowTuning {
            damping: [0.4; 3],
            ..FollowTuning::follow3()
        });
        rig.tick(1.0 / rate, &Input::at([0.0; 3]), Comfort::FULL);
        for _ in 0..(rate as usize) {
            rig.tick(1.0 / rate, &Input::at([6.0, 0.0, 0.0]), Comfort::FULL);
        }
        rig.focus()[0]
    };
    near(at(60.0), at(240.0), 1e-3);
}

#[test]
fn look_ahead_leads_in_the_direction_of_travel_and_scales_with_speed() {
    let lead = |speed: f32| {
        let mut rig = follow(FollowTuning {
            look_ahead: 2.0,
            look_ahead_speed: 6.0,
            ..flat()
        });
        let mut x = 0.0;
        for _ in 0..30 {
            x += speed * DT;
            rig.tick(DT, &Input::at([x, 0.0, 0.0]), Comfort::FULL);
        }
        rig.focus()[0] - x
    };
    near(lead(6.0), 2.0, 1e-3);
    near(lead(3.0), 1.0, 1e-3);
    near(lead(-6.0), -2.0, 1e-3);
    near(lead(0.0), 0.0, 1e-6);
}

#[test]
fn reduced_motion_turns_look_ahead_off() {
    let lead = |comfort: Comfort| {
        let mut rig = follow(FollowTuning {
            look_ahead: 2.0,
            look_ahead_speed: 6.0,
            ..flat()
        });
        let mut x = 0.0;
        for _ in 0..30 {
            x += 6.0 * DT;
            rig.tick(DT, &Input::at([x, 0.0, 0.0]), comfort);
        }
        rig.focus()[0] - x
    };
    near(lead(Comfort::FULL), 2.0, 1e-3);
    near(lead(Comfort::reduced(0.0)), 0.0, 1e-6);
}

#[test]
fn a_plane_rig_follows_in_x_and_y_and_keeps_its_depth() {
    let mut rig = follow(FollowTuning {
        damping: [0.0; 3],
        look_ahead_damping: 0.0,
        look_ahead: 3.0,
        ..FollowTuning::follow2()
    });
    run(&mut rig, [0.0; 3], 2);
    for step in 1..20 {
        rig.tick(DT, &Input::at([step as f32 * 0.1, 0.0, 0.5]), Comfort::FULL);
    }
    let view = rig.pose(1.0);
    assert_eq!(view.look_at[2], 0.5);
    assert_eq!(view.at[2], 10.5);
    assert_eq!(view.at[0], view.look_at[0]);
}

#[test]
fn pixel_snapping_puts_the_view_on_whole_pixels() {
    let mut rig = follow(FollowTuning {
        pixels_per_unit: Some(16.0),
        damping: [0.0; 3],
        ..FollowTuning::follow2()
    });
    run(&mut rig, [0.0; 3], 2);
    for step in 0..50 {
        rig.tick(
            DT,
            &Input::at([step as f32 * 0.0137, step as f32 * 0.0091, 0.0]),
            Comfort::FULL,
        );
        for alpha in [0.0, 0.3, 0.77, 1.0] {
            let view = rig.pose(alpha);
            for axis in 0..2 {
                let pixels = view.look_at[axis] * 16.0;
                near(pixels, pixels.round(), 1e-3);
            }
        }
    }
}

#[test]
fn a_handover_slides_to_the_new_target_in_its_time_and_never_jumps() {
    let mut rig = follow(FollowTuning {
        handover: 1.0,
        dead_zone: [5.0, 5.0, 5.0],
        ..flat()
    });
    run(&mut rig, [0.0; 3], 3);
    rig.hand_over();
    let mut last = rig.focus()[0];
    let mut biggest = 0.0f32;
    for _ in 0..60 {
        rig.tick(DT, &Input::at([20.0, 0.0, 0.0]), Comfort::FULL);
        let x = rig.focus()[0];
        assert!(x >= last);
        biggest = biggest.max(x - last);
        last = x;
    }
    near(last, 20.0, 1e-3);
    assert!(biggest < 1.0, "a step of {biggest}");
}

#[test]
fn a_handover_of_no_time_cuts_to_the_new_target() {
    let mut rig = follow(FollowTuning {
        handover: 0.0,
        ..flat()
    });
    run(&mut rig, [0.0; 3], 3);
    rig.hand_over();
    rig.tick(DT, &Input::at([20.0, 0.0, 0.0]), Comfort::FULL);
    assert_eq!(rig.focus()[0], 20.0);
    assert_eq!(rig.pose(0.0).look_at[0], 20.0);
}

#[test]
fn the_pose_interpolates_between_ticks_by_alpha() {
    let mut rig = follow(flat());
    run(&mut rig, [0.0; 3], 2);
    rig.tick(DT, &Input::at([4.0, 0.0, 0.0]), Comfort::FULL);
    near(rig.pose(0.0).look_at[0], 0.0, 1e-6);
    near(rig.pose(0.5).look_at[0], 2.0, 1e-6);
    near(rig.pose(1.0).look_at[0], 4.0, 1e-6);
}

fn person(tuning: PersonTuning) -> Person {
    Person::new(tuning)
}

fn walk(rig: &mut Person, inputs: &[Input], comfort: Comfort, probe: &dyn Probe) {
    for input in inputs {
        rig.tick(DT, input, comfort, probe);
    }
}

#[test]
fn the_pitch_never_leaves_its_clamp() {
    let mut rig = person(PersonTuning {
        pitch_range: [-60.0, 70.0],
        ..PersonTuning::default()
    });
    let up = Input::at([0.0; 3]).mouse([0.0, -4000.0]);
    walk(&mut rig, &[up; 5], Comfort::FULL, &Open);
    near(rig.pitch(), 70.0f32.to_radians(), 1e-5);
    let down = Input::at([0.0; 3]).mouse([0.0, 9000.0]);
    walk(&mut rig, &[down; 5], Comfort::FULL, &Open);
    near(rig.pitch(), (-60.0f32).to_radians(), 1e-5);
    let stick = Input::at([0.0; 3]).looking([0.0, 1.0]);
    walk(&mut rig, &[stick; 600], Comfort::FULL, &Open);
    near(rig.pitch(), 70.0f32.to_radians(), 1e-5);
}

#[test]
fn a_clamp_past_straight_up_still_stops_short_of_it() {
    let mut rig = person(PersonTuning {
        pitch_range: [-120.0, 120.0],
        ..PersonTuning::default()
    });
    let up = Input::at([0.0; 3]).mouse([0.0, -90000.0]);
    walk(&mut rig, &[up], Comfort::FULL, &Open);
    assert!(rig.pitch() < 90.0f32.to_radians());
    let view = rig.pose(1.0);
    assert!(view.forward()[1] < 1.0);
}

#[test]
fn the_mouse_turns_the_view_right_and_down_and_invert_flips_each_axis() {
    let turned = |tuning: PersonTuning| {
        let mut rig = person(tuning);
        let input = Input::at([0.0; 3]).mouse([100.0, 100.0]);
        walk(&mut rig, &[input], Comfort::FULL, &Open);
        (rig.yaw(), rig.pitch())
    };
    let (yaw, pitch) = turned(PersonTuning::default());
    near(yaw, TAU - 10.0f32.to_radians(), 1e-5);
    near(pitch, -10.0f32.to_radians(), 1e-5);
    let (yaw, pitch) = turned(PersonTuning {
        invert_x: true,
        invert_y: true,
        ..PersonTuning::default()
    });
    near(yaw, 10.0f32.to_radians(), 1e-5);
    near(pitch, 10.0f32.to_radians(), 1e-5);
}

#[test]
fn sensitivity_scales_the_turn_and_the_stick_turns_at_a_rate() {
    let mut fast = person(PersonTuning {
        mouse: 0.2,
        ..PersonTuning::default()
    });
    let mut slow = person(PersonTuning {
        mouse: 0.1,
        ..PersonTuning::default()
    });
    let input = Input::at([0.0; 3]).mouse([0.0, -50.0]);
    walk(&mut fast, &[input], Comfort::FULL, &Open);
    walk(&mut slow, &[input], Comfort::FULL, &Open);
    near(fast.pitch(), slow.pitch() * 2.0, 1e-6);
    let mut stick = person(PersonTuning {
        stick: 90.0,
        ..PersonTuning::default()
    });
    let held = Input::at([0.0; 3]).looking([0.0, 1.0]);
    walk(&mut stick, &[held; 30], Comfort::FULL, &Open);
    near(stick.pitch(), 45.0f32.to_radians(), 1e-4);
}

#[test]
fn smoothing_spreads_a_turn_over_time_and_loses_none_of_it() {
    let mut rig = person(PersonTuning {
        smoothing: 0.15,
        ..PersonTuning::default()
    });
    let flick = Input::at([0.0; 3]).mouse([0.0, -200.0]);
    walk(&mut rig, &[flick], Comfort::FULL, &Open);
    let first = rig.pitch();
    assert!(first > 0.0 && first < 20.0f32.to_radians());
    let idle = Input::at([0.0; 3]);
    walk(&mut rig, &[idle; 120], Comfort::FULL, &Open);
    near(rig.pitch(), 20.0f32.to_radians(), 1e-4);
}

#[test]
fn head_bob_follows_the_speed_and_stops_when_standing_or_airborne() {
    let mut rig = person(PersonTuning {
        bob: Some(Bob::default()),
        ..PersonTuning::default()
    });
    let mut x = 0.0;
    let mut tops: f32 = 0.0;
    let mut lows: f32 = 0.0;
    for _ in 0..180 {
        x += 5.0 * DT;
        rig.tick(DT, &Input::at([x, 0.0, 0.0]), Comfort::FULL, &Open);
        let y = rig.pose(1.0).at[1] - 1.6;
        tops = tops.max(y);
        lows = lows.min(y);
    }
    assert!(tops > 0.02 && lows < -0.02, "bobbed {lows} to {tops}");
    assert!(rig.bob_amplitude() > 0.9);
    for _ in 0..180 {
        rig.tick(DT, &Input::at([x, 0.0, 0.0]), Comfort::FULL, &Open);
    }
    assert!(rig.bob_amplitude() < 1e-3);
    near(rig.pose(1.0).at[1], 1.6, 1e-3);
    let mut jump = person(PersonTuning {
        bob: Some(Bob::default()),
        ..PersonTuning::default()
    });
    let mut x = 0.0;
    for _ in 0..120 {
        x += 5.0 * DT;
        jump.tick(
            DT,
            &Input::at([x, 0.0, 0.0]).airborne(),
            Comfort::FULL,
            &Open,
        );
    }
    assert_eq!(jump.bob_amplitude(), 0.0);
}

#[test]
fn reduced_motion_turns_head_bob_off() {
    let bob = |comfort: Comfort| {
        let mut rig = person(PersonTuning {
            bob: Some(Bob::default()),
            ..PersonTuning::default()
        });
        let mut x = 0.0;
        let mut spread: f32 = 0.0;
        for _ in 0..180 {
            x += 5.0 * DT;
            rig.tick(DT, &Input::at([x, 0.0, 0.0]), comfort, &Open);
            spread = spread.max((rig.pose(1.0).at[1] - 1.6).abs());
        }
        spread
    };
    assert!(bob(Comfort::FULL) > 0.02);
    assert_eq!(bob(Comfort::reduced(0.0)), 0.0);
}

fn third() -> PersonTuning {
    PersonTuning {
        third: Some(Third::default()),
        ..PersonTuning::default()
    }
}

#[test]
fn the_third_person_camera_sits_behind_the_character_at_its_distance() {
    let mut rig = person(third());
    rig.set_third(true);
    walk(
        &mut rig,
        &[Input::at([2.0, 0.0, 3.0]); 300],
        Comfort::FULL,
        &Open,
    );
    let view = rig.pose(1.0);
    near(view.at[0], 2.0, 1e-3);
    near(view.at[1], 1.6, 1e-3);
    near(view.at[2], 3.0 + 4.0, 1e-2);
    near(rig.arm(), 4.0, 1e-2);
}

#[test]
fn the_third_person_camera_never_goes_through_a_wall() {
    let mut rig = person(third());
    rig.set_third(true);
    let wall = Wall { distance: 1.5 };
    for _ in 0..300 {
        rig.tick(DT, &Input::at([0.0; 3]), Comfort::FULL, &wall);
        assert!(rig.arm() <= 1.5 - 0.05 + 1e-5, "arm {}", rig.arm());
    }
    near(rig.arm(), 1.45, 1e-4);
    let squeezed = Wall { distance: 0.1 };
    for _ in 0..30 {
        rig.tick(DT, &Input::at([0.0; 3]), Comfort::FULL, &squeezed);
    }
    near(rig.arm(), 0.4, 1e-4);
}

#[test]
fn the_third_person_camera_pulls_in_at_once_and_eases_back_out() {
    let mut rig = person(third());
    rig.set_third(true);
    walk(&mut rig, &[Input::at([0.0; 3]); 300], Comfort::FULL, &Open);
    let wall = Wall { distance: 1.0 };
    rig.tick(DT, &Input::at([0.0; 3]), Comfort::FULL, &wall);
    near(rig.arm(), 0.95, 1e-5);
    rig.tick(DT, &Input::at([0.0; 3]), Comfort::FULL, &Open);
    let once = rig.arm();
    assert!(once > 0.95 && once < 1.2);
    walk(&mut rig, &[Input::at([0.0; 3]); 300], Comfort::FULL, &Open);
    near(rig.arm(), 4.0, 1e-2);
}

#[test]
fn the_same_rig_switches_between_first_and_third_person() {
    let mut rig = person(third());
    walk(&mut rig, &[Input::at([0.0; 3]); 5], Comfort::FULL, &Open);
    assert_eq!(rig.arm(), 0.0);
    rig.set_third(true);
    walk(&mut rig, &[Input::at([0.0; 3]); 300], Comfort::FULL, &Open);
    assert!(rig.arm() > 3.9);
    rig.set_third(false);
    walk(&mut rig, &[Input::at([0.0; 3]); 600], Comfort::FULL, &Open);
    assert_eq!(rig.arm(), 0.0);
    let mut plain = person(PersonTuning::default());
    plain.set_third(true);
    assert!(!plain.is_third());
}

#[test]
fn a_person_handover_slides_the_camera_to_the_new_target() {
    let mut rig = person(PersonTuning {
        handover: 1.0,
        ..PersonTuning::default()
    });
    walk(&mut rig, &[Input::at([0.0; 3]); 3], Comfort::FULL, &Open);
    rig.hand_over();
    let mut last = rig.pose(1.0).at[0];
    let mut biggest = 0.0f32;
    for _ in 0..90 {
        rig.tick(DT, &Input::at([30.0, 0.0, 0.0]), Comfort::FULL, &Open);
        let x = rig.pose(1.0).at[0];
        assert!(x >= last);
        biggest = biggest.max(x - last);
        last = x;
    }
    near(last, 30.0, 1e-3);
    assert!(biggest < 1.5, "a step of {biggest}");
}

#[test]
fn the_view_looks_along_the_yaw_and_pitch() {
    let mut rig = person(PersonTuning::default());
    rig.set_look(0.0, 0.0);
    walk(&mut rig, &[Input::at([0.0; 3]); 2], Comfort::FULL, &Open);
    let forward = rig.pose(1.0).forward();
    near(forward[2], -1.0, 1e-5);
    rig.set_look(90.0f32.to_radians(), 0.0);
    walk(&mut rig, &[Input::at([0.0; 3]); 2], Comfort::FULL, &Open);
    near(rig.pose(1.0).forward()[0], -1.0, 1e-5);
}

fn replay(rig: &mut Rig, ticks: usize) -> Vec<View> {
    let mut views = Vec::new();
    let mut x = 0.0;
    for step in 0..ticks {
        x += (step as f32 * 0.05).sin() * 0.3;
        let input = Input::at([x, 0.0, step as f32 * 0.02])
            .mouse([(step % 7) as f32 - 3.0, (step % 5) as f32 - 2.0])
            .looking([0.0, ((step % 3) as f32 - 1.0) * 0.4]);
        rig.tick(DT, &input, Comfort::FULL, &Wall { distance: 2.5 });
        views.push(rig.pose(0.5));
    }
    views
}

#[test]
fn every_rig_replays_bit_for_bit_from_the_same_inputs_by_tick() {
    let make: [fn() -> Rig; 3] = [
        || Rig::Follow(Follow::new(FollowTuning::follow3())),
        || {
            Rig::Follow(Follow::new(FollowTuning {
                look_ahead: 2.0,
                dead_zone: [1.0, 1.0, 1.0],
                ..FollowTuning::follow2()
            }))
        },
        || {
            let mut person = Person::new(PersonTuning {
                bob: Some(Bob::default()),
                smoothing: 0.1,
                ..third()
            });
            person.set_third(true);
            Rig::Person(person)
        },
    ];
    for make in make {
        assert_eq!(replay(&mut make(), 300), replay(&mut make(), 300));
    }
}

#[test]
fn a_blend_eases_from_the_old_view_to_the_new_one() {
    let from = View {
        at: [0.0; 3],
        look_at: [0.0, 0.0, -1.0],
        up: UP,
    };
    let to = View {
        at: [10.0, 0.0, 0.0],
        look_at: [10.0, 0.0, -1.0],
        up: UP,
    };
    let mut blend = Blend::new(from, 1.0);
    assert_eq!(blend.apply(to), from);
    blend.step(0.5);
    near(blend.apply(to).at[0], 5.0, 1e-5);
    blend.step(0.6);
    assert!(blend.done());
    assert_eq!(blend.apply(to), to);
    assert!(Blend::new(from, 0.0).done());
}

fn stream() -> Vec<Input> {
    (0..240)
        .map(|step| {
            let step = step as f32;
            Input::at([step * 0.031, (step % 17.0) * 0.01, step * 0.012 - 1.0])
                .mouse([(step % 7.0) - 3.0, (step % 5.0) - 2.0])
                .looking([((step % 11.0) - 5.0) * 0.1, ((step % 3.0) - 1.0) * 0.4])
        })
        .collect()
}

fn bits(views: &[View]) -> Vec<u32> {
    views
        .iter()
        .flat_map(|view| [view.at, view.look_at, view.up])
        .flatten()
        .map(f32::to_bits)
        .collect()
}

fn poses(mut rig: Rig) -> Vec<View> {
    let mut out = Vec::new();
    for (step, input) in stream().iter().enumerate() {
        rig.tick(DT, input, Comfort::FULL, &Wall { distance: 2.5 });
        if step % 40 == 39 {
            out.push(rig.pose(0.5));
            out.push(rig.pose(1.0));
        }
    }
    out
}

#[test]
fn rig_poses_after_a_fixed_input_stream_are_pinned_to_exact_bits() {
    let follow = poses(Rig::Follow(Follow::new(FollowTuning {
        look_ahead: 2.0,
        dead_zone: [0.5, 0.25, 0.5],
        ..FollowTuning::follow3()
    })));
    let first = poses(Rig::Person(Person::new(PersonTuning {
        bob: Some(Bob::default()),
        smoothing: 0.08,
        ..PersonTuning::default()
    })));
    let mut third_person = Person::new(PersonTuning {
        bob: Some(Bob::default()),
        third: Some(Third::default()),
        ..PersonTuning::default()
    });
    third_person.set_third(true);
    let third = poses(Rig::Person(third_person));
    let digest = |views: &[View]| {
        bits(views)
            .iter()
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, bits| {
                (hash ^ u64::from(*bits)).wrapping_mul(0x0100_0000_01b3)
            })
    };
    assert_eq!([digest(&follow), digest(&first), digest(&third)], PINNED);
}

const PINNED: [u64; 3] = [
    7_338_984_540_158_502_336,
    15_637_321_010_532_887_637,
    633_970_099_457_045_819,
];
