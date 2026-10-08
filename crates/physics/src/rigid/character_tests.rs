use super::*;

fn world() -> World {
    World::new(WorldDesc::default())
}

fn block(world: &mut World, at: [f32; 3], half: [f32; 3]) -> BodyId {
    let body = world.add_body(BodyDesc::fixed(Pose::at(at)));
    world
        .add_collider(body, ColliderDesc::cuboid(half))
        .unwrap();
    body
}

fn floor(world: &mut World) -> BodyId {
    block(world, [0.0, -0.5, 0.0], [50.0, 0.5, 50.0])
}

fn ramp(world: &mut World, degrees: f32, foot: f32) {
    let angle = degrees.to_radians();
    let half = [4.0, 0.25, 2.0];
    let (sin, cos) = angle.sin_cos();
    let center = [
        foot + half[0] * cos + half[1] * sin,
        half[0] * sin - half[1] * cos,
        0.0,
    ];
    let body = world.add_body(BodyDesc::fixed(
        Pose::at(center).turned([0.0, 0.0, 1.0], angle),
    ));
    world
        .add_collider(body, ColliderDesc::cuboid(half))
        .unwrap();
}

fn settle(world: &mut World, character: &mut Character, ticks: u32) {
    for _ in 0..ticks {
        character.step(world);
        world.step();
    }
}

fn run(world: &mut World, character: &mut Character, ticks: u32, direction: [f32; 3], jump: bool) {
    for _ in 0..ticks {
        character.drive(direction, jump);
        character.step(world);
        world.step();
    }
}

fn standing(desc: CharacterDesc, feet: [f32; 3]) -> (World, Character) {
    let mut world = world();
    floor(&mut world);
    let mut character = Character::spawn(&mut world, desc, feet).unwrap();
    settle(&mut world, &mut character, 30);
    (world, character)
}

#[test]
fn a_character_falls_lands_and_stands_on_the_floor() {
    let mut world = world();
    floor(&mut world);
    let mut character =
        Character::spawn(&mut world, CharacterDesc::default(), [0.0, 2.0, 0.0]).unwrap();
    assert!(!character.grounded());
    settle(&mut world, &mut character, 90);
    assert!(character.grounded());
    let feet = character.feet();
    assert!(feet[1] >= 0.0 && feet[1] < 0.05, "feet {feet:?}");
    let body = world.pose(character.body()).unwrap().position;
    assert_eq!(body, character.center());
}

#[test]
fn walking_reaches_the_speed_and_stops() {
    let (mut world, mut character) = standing(CharacterDesc::default(), [0.0, 0.0, 0.0]);
    run(&mut world, &mut character, 60, [1.0, 0.0, 0.0], false);
    let velocity = character.velocity();
    assert!((velocity[0] - 5.0).abs() < 1.0e-3, "{velocity:?}");
    let x = character.feet()[0];
    assert!(x > 4.0 && x < 5.0, "x {x}");
    run(&mut world, &mut character, 30, [0.0; 3], false);
    assert_eq!(character.velocity()[0], 0.0);
    assert!(character.grounded());
}

#[test]
fn slopes_under_the_climb_angle_are_walked_and_steeper_ones_are_not() {
    let mut easy = world();
    floor(&mut easy);
    ramp(&mut easy, 30.0, 1.0);
    let mut walker =
        Character::spawn(&mut easy, CharacterDesc::default(), [0.0, 0.0, 0.0]).unwrap();
    settle(&mut easy, &mut walker, 20);
    run(&mut easy, &mut walker, 120, [1.0, 0.0, 0.0], false);
    let feet = walker.feet();
    assert!(feet[1] > 1.0, "climbed the 30° ramp: {feet:?}");
    assert!(walker.grounded());

    let mut steep = world();
    floor(&mut steep);
    ramp(&mut steep, 60.0, 1.0);
    let mut blocked =
        Character::spawn(&mut steep, CharacterDesc::default(), [0.0, 0.0, 0.0]).unwrap();
    settle(&mut steep, &mut blocked, 20);
    run(&mut steep, &mut blocked, 120, [1.0, 0.0, 0.0], false);
    let feet = blocked.feet();
    assert!(feet[1] < 0.35, "held below the 60° ramp: {feet:?}");
    assert!(feet[0] < 1.2, "{feet:?}");
}

#[test]
fn a_character_on_a_slope_past_the_slide_angle_slides_down() {
    let mut world = world();
    floor(&mut world);
    ramp(&mut world, 60.0, 0.0);
    let desc = CharacterDesc {
        max_climb: 50.0,
        ..CharacterDesc::default()
    };
    let mut character = Character::spawn(&mut world, desc, [2.0, 4.5, 0.0]).unwrap();
    settle(&mut world, &mut character, 240);
    let feet = character.feet();
    assert!(feet[1] < 0.5, "slid to the foot: {feet:?}");
    assert!(feet[0] < 1.0, "{feet:?}");
}

#[test]
fn steps_up_to_the_step_height_are_climbed_and_taller_ones_block() {
    for (height, climbs) in [(0.2, true), (0.28, true), (0.45, false)] {
        let mut world = world();
        floor(&mut world);
        block(
            &mut world,
            [6.5, height * 0.5, 0.0],
            [5.0, height * 0.5, 2.0],
        );
        let mut character =
            Character::spawn(&mut world, CharacterDesc::default(), [0.0, 0.0, 0.0]).unwrap();
        settle(&mut world, &mut character, 20);
        run(&mut world, &mut character, 90, [1.0, 0.0, 0.0], false);
        let feet = character.feet();
        if climbs {
            assert!(feet[0] > 2.0, "a {height} step is climbed: {feet:?}");
            assert!((feet[1] - height).abs() < 0.05, "{feet:?}");
        } else {
            assert!(feet[0] < 1.5, "a {height} step blocks: {feet:?}");
            assert!(feet[1] < 0.05, "{feet:?}");
        }
    }
}

#[test]
fn ground_snap_keeps_a_character_on_a_small_drop_and_off_without_it() {
    let airborne = |snap: f32| {
        let mut world = world();
        block(&mut world, [-10.0, -0.5, 0.0], [10.0, 0.5, 4.0]);
        block(&mut world, [10.0, -0.65, 0.0], [10.0, 0.5, 4.0]);
        let desc = CharacterDesc {
            snap,
            ..CharacterDesc::default()
        };
        let mut character = Character::spawn(&mut world, desc, [-2.0, 0.0, 0.0]).unwrap();
        settle(&mut world, &mut character, 20);
        let mut airborne = 0;
        for _ in 0..60 {
            character.drive([1.0, 0.0, 0.0], false);
            character.step(&mut world);
            world.step();
            if !character.grounded() {
                airborne += 1;
            }
        }
        assert!(character.feet()[0] > 2.0);
        assert!((character.feet()[1] + 0.15).abs() < 0.05);
        airborne
    };
    assert_eq!(airborne(0.2), 0, "snapped down the 0.15 drop");
    assert!(airborne(0.0) > 0, "fell off the 0.15 drop without snap");
}

#[test]
fn a_character_rides_a_moving_platform() {
    let mut world = world();
    floor(&mut world);
    let platform = world.add_body(BodyDesc::kinematic(Pose::at([0.0, 0.5, 0.0])));
    world
        .add_collider(platform, ColliderDesc::cuboid([1.5, 0.25, 1.5]))
        .unwrap();
    let mut character =
        Character::spawn(&mut world, CharacterDesc::default(), [0.0, 0.75, 0.0]).unwrap();
    settle(&mut world, &mut character, 20);
    assert_eq!(character.ground(), Some(platform));
    let start = character.feet();
    for tick in 1..=120 {
        let t = tick as f32 / 60.0;
        world.move_to(platform, Pose::at([t, 0.5 + 0.5 * t, 0.0]));
        character.step(&mut world);
        world.step();
    }
    let feet = character.feet();
    assert!(
        (feet[0] - start[0] - 2.0).abs() < 0.05,
        "{start:?} {feet:?}"
    );
    assert!(
        (feet[1] - start[1] - 1.0).abs() < 0.03,
        "{start:?} {feet:?}"
    );
    assert!(character.grounded());
    let top = world.pose(platform).unwrap().position[1] + 0.25;
    assert!(
        feet[1] >= top - 1.0e-3 && feet[1] < top + 0.05,
        "{top} {feet:?}"
    );

    for tick in 1..=60 {
        let t = tick as f32 / 60.0;
        world.move_to(platform, Pose::at([2.0, 1.5 - 0.6 * t, 0.0]));
        character.step(&mut world);
        world.step();
    }
    let top = world.pose(platform).unwrap().position[1] + 0.25;
    let feet = character.feet();
    assert!(
        feet[1] >= top - 1.0e-3 && feet[1] < top + 0.05,
        "{top} {feet:?}"
    );
    assert!(character.grounded());
}

#[test]
fn a_character_rides_a_driven_platform_without_riding_when_off() {
    let ride = |ride: bool| {
        let mut world = world();
        floor(&mut world);
        let platform = world.add_body(BodyDesc::kinematic(Pose::at([0.0, 0.25, 0.0])));
        world
            .add_collider(platform, ColliderDesc::cuboid([1.5, 0.25, 1.5]))
            .unwrap();
        let desc = CharacterDesc {
            ride,
            ..CharacterDesc::default()
        };
        let mut character = Character::spawn(&mut world, desc, [0.0, 0.5, 0.0]).unwrap();
        settle(&mut world, &mut character, 10);
        world.set_drive(
            platform,
            Some(Drive::ease(
                Pose::at([0.0, 0.25, 0.0]),
                Pose::at([0.0, 0.25, 1.0]),
                1.0,
                |t| t,
            )),
        );
        settle(&mut world, &mut character, 60);
        character.feet()[2]
    };
    assert!((ride(true) - 1.0).abs() < 0.03);
    assert!(ride(false).abs() < 0.03);
}

#[test]
fn coyote_time_allows_a_jump_just_after_a_ledge_and_not_later() {
    let jumped = |late: u32| {
        let mut world = world();
        block(&mut world, [-5.0, -0.5, 0.0], [5.0, 0.5, 4.0]);
        let desc = CharacterDesc {
            coyote: 6,
            snap: 0.0,
            ..CharacterDesc::default()
        };
        let mut character = Character::spawn(&mut world, desc, [-1.0, 0.0, 0.0]).unwrap();
        settle(&mut world, &mut character, 20);
        let mut off = 0;
        while off == 0 || off < late {
            character.drive([1.0, 0.0, 0.0], false);
            character.step(&mut world);
            world.step();
            if !character.grounded() {
                off += 1;
            }
            assert!(character.ticks() < 200);
        }
        let before = character.velocity()[1];
        run(&mut world, &mut character, 1, [1.0, 0.0, 0.0], true);
        character.velocity()[1] > before + 1.0
    };
    assert!(jumped(1), "a jump on the first tick off the ledge");
    assert!(jumped(5), "a jump within coyote time");
    assert!(!jumped(8), "no jump after coyote time");
}

#[test]
fn a_buffered_jump_fires_on_landing_within_the_buffer_only() {
    let jumps = |early: u32| {
        let mut world = world();
        floor(&mut world);
        let desc = CharacterDesc {
            jump_buffer: 6,
            ..CharacterDesc::default()
        };
        let mut character = Character::spawn(&mut world, desc, [0.0, 3.0, 0.0]).unwrap();
        let mut landed_at = None;
        for tick in 0..200u32 {
            character.step(&mut world);
            world.step();
            if character.grounded() {
                landed_at = Some(tick);
                break;
            }
        }
        let landed_at = landed_at.unwrap();
        let mut world = World::new(WorldDesc::default());
        floor(&mut world);
        let mut character = Character::spawn(&mut world, desc, [0.0, 3.0, 0.0]).unwrap();
        let press = landed_at - early;
        let mut rose = false;
        for tick in 0..(landed_at + 20) {
            character.drive([0.0; 3], tick >= press && tick < press + 2);
            character.step(&mut world);
            world.step();
            if tick > landed_at && character.velocity()[1] > 1.0 {
                rose = true;
            }
        }
        rose
    };
    assert!(jumps(3), "pressed 3 ticks early");
    assert!(jumps(5), "pressed 5 ticks early");
    assert!(!jumps(12), "pressed 12 ticks early");
}

#[test]
fn a_jump_reaches_its_height_and_a_variable_jump_is_lower() {
    let apex = |variable_jump: f32, hold: u32| {
        let desc = CharacterDesc {
            variable_jump,
            jump_speed: 5.0,
            ..CharacterDesc::default()
        };
        let (mut world, mut character) = standing(desc, [0.0, 0.0, 0.0]);
        let start = character.feet()[1];
        let mut top = 0.0f32;
        for tick in 0..120 {
            character.drive([0.0; 3], tick < hold);
            character.step(&mut world);
            world.step();
            top = top.max(character.feet()[1]);
        }
        top - start
    };
    let dt = 1.0 / 60.0;
    let mut rise = 5.0f32;
    let mut height = 0.0;
    loop {
        rise -= 9.81 * dt;
        if rise <= 0.0 {
            break;
        }
        height += rise * dt;
    }
    let full = apex(0.0, 60);
    assert!(
        (full - height).abs() < 1.0e-3,
        "full jump {full}, {height} expected"
    );
    let held = apex(0.6, 60);
    assert!((held - full).abs() < 1.0e-4, "held to the apex {held}");
    let short = apex(0.6, 5);
    assert!(short < full * 0.6, "short hop {short}");
    let never = CharacterDesc {
        jump_speed: 0.0,
        ..CharacterDesc::default()
    };
    let (mut world, mut character) = standing(never, [0.0, 0.0, 0.0]);
    let start = character.feet()[1];
    run(&mut world, &mut character, 30, [0.0; 3], true);
    assert!(
        (character.feet()[1] - start).abs() < 1.0e-3,
        "jump_speed 0 never jumps"
    );
}

#[test]
fn a_two_d_character_stays_in_its_plane() {
    let mut world = world();
    floor(&mut world);
    block(&mut world, [4.0, 1.0, 0.0], [0.5, 1.0, 2.0]);
    let desc = CharacterDesc::flat(Plane::Xy);
    let mut character = Character::spawn(&mut world, desc, [0.0, 0.0, 0.25]).unwrap();
    assert_eq!(world.locks(character.body()), Some(Locks::PLANE_XY));
    for tick in 0..600u32 {
        let direction = [1.0, 0.0, if tick % 2 == 0 { 1.0 } else { -0.7 }];
        character.drive(direction, tick % 50 < 10);
        character.step(&mut world);
        world.step();
        assert_eq!(
            character.feet()[2].to_bits(),
            0.25f32.to_bits(),
            "tick {tick}"
        );
        let body = world.pose(character.body()).unwrap();
        assert_eq!(body.position[2].to_bits(), 0.25f32.to_bits(), "tick {tick}");
        assert_eq!(character.velocity()[2], 0.0);
    }
    assert!(character.feet()[0] > 3.0, "{:?}", character.feet());
}

#[test]
fn wall_slide_slows_the_fall_and_wall_jump_kicks_away() {
    let mut world = world();
    floor(&mut world);
    block(&mut world, [1.0, 5.0, 0.0], [0.5, 5.0, 2.0]);
    let desc = CharacterDesc {
        wall_slide: Some(1.5),
        wall_jump: Some([4.0, 6.0]),
        ..CharacterDesc::flat(Plane::Xy)
    };
    let mut character = Character::spawn(&mut world, desc, [0.0, 6.0, 0.0]).unwrap();
    run(&mut world, &mut character, 40, [1.0, 0.0, 0.0], false);
    assert!(character.on_wall().is_some());
    let fall = character.velocity()[1];
    assert!((-1.5 - 1.0e-4..-1.0).contains(&fall), "slid at {fall}");
    let normal = character.on_wall().unwrap();
    assert!(normal[0] < -0.9, "{normal:?}");
    run(&mut world, &mut character, 1, [1.0, 0.0, 0.0], true);
    let kick = character.velocity();
    assert!(kick[1] > 5.0, "{kick:?}");
    assert!(kick[0] < -3.0, "{kick:?}");

    let mut plain = World::new(WorldDesc::default());
    floor(&mut plain);
    block(&mut plain, [1.0, 5.0, 0.0], [0.5, 5.0, 2.0]);
    let mut falling =
        Character::spawn(&mut plain, CharacterDesc::flat(Plane::Xy), [0.0, 6.0, 0.0]).unwrap();
    run(&mut plain, &mut falling, 40, [1.0, 0.0, 0.0], false);
    assert!(falling.velocity()[1] < -4.0, "{:?}", falling.velocity());
    run(&mut plain, &mut falling, 1, [1.0, 0.0, 0.0], true);
    assert!(
        falling.velocity()[1] < 0.0,
        "no wall jump unless configured"
    );
}

#[test]
fn walking_into_a_dynamic_box_pushes_it_and_push_off_leaves_it() {
    let pushed = |push: bool| {
        let mut world = world();
        floor(&mut world);
        let crate_body = world.add_body(BodyDesc::dynamic(Pose::at([1.5, 0.3, 0.0])));
        world
            .add_collider(crate_body, ColliderDesc::cuboid([0.3, 0.3, 0.3]))
            .unwrap();
        let desc = CharacterDesc {
            push,
            ..CharacterDesc::default()
        };
        let mut character = Character::spawn(&mut world, desc, [0.0, 0.0, 0.0]).unwrap();
        settle(&mut world, &mut character, 10);
        run(&mut world, &mut character, 60, [1.0, 0.0, 0.0], false);
        world.pose(crate_body).unwrap().position[0]
    };
    assert!(pushed(true) > 2.0);
    assert!(pushed(false) < 1.6);
}

#[test]
fn gravity_from_a_zone_turns_the_character_and_its_jump() {
    let mut world = World::new(WorldDesc {
        gravity: [0.0, -9.81, 0.0],
        ..WorldDesc::default()
    });
    block(&mut world, [-0.5, 0.0, 0.0], [0.5, 50.0, 50.0]);
    world.add_source(Source::Zone {
        region: Region::Box {
            pose: Pose::IDENTITY,
            half: [60.0, 60.0, 60.0],
        },
        gravity: [-9.81, 0.0, 0.0],
        replace: true,
    });
    assert_eq!(world.gravity_at([3.0, 0.0, 0.0]), [-9.81, 0.0, 0.0]);
    let mut character =
        Character::spawn(&mut world, CharacterDesc::default(), [2.0, 0.0, 0.0]).unwrap();
    assert_eq!(character.up(), [1.0, 0.0, 0.0]);
    settle(&mut world, &mut character, 90);
    assert!(character.grounded());
    let feet = character.feet();
    assert!(feet[0] >= 0.0 && feet[0] < 0.05, "{feet:?}");
    run(&mut world, &mut character, 10, [0.0, 0.0, 0.0], true);
    assert!(character.feet()[0] > 0.5, "{:?}", character.feet());
}

fn course(world: &mut World) -> BodyId {
    floor(world);
    ramp(world, 25.0, 3.0);
    block(world, [-3.0, 0.1, 0.0], [1.0, 0.1, 1.0]);
    block(world, [0.0, 1.0, -4.0], [6.0, 1.0, 0.5]);
    let platform = world.add_body(BodyDesc::kinematic(Pose::at([-6.0, 0.5, 0.0])));
    world
        .add_collider(platform, ColliderDesc::cuboid([1.0, 0.25, 1.0]))
        .unwrap();
    for i in 0..4 {
        let body = world.add_body(BodyDesc::dynamic(Pose::at([
            1.0 + i as f32 * 0.7,
            0.3,
            1.5,
        ])));
        world
            .add_collider(body, ColliderDesc::cuboid([0.25, 0.25, 0.25]))
            .unwrap();
    }
    platform
}

fn script(tick: u32) -> ([f32; 3], bool) {
    let phase = (tick / 40) % 6;
    let direction = match phase {
        0 => [1.0, 0.0, 0.2],
        1 => [0.3, 0.0, 1.0],
        2 => [-1.0, 0.0, -0.4],
        3 => [-0.6, 0.0, 0.8],
        4 => [0.0, 0.0, -1.0],
        _ => [0.8, 0.0, 0.0],
    };
    (direction, tick % 37 < 8)
}

fn played(desc: CharacterDesc, ticks: u32) -> Vec<u32> {
    let mut world = world();
    let platform = course(&mut world);
    let mut characters = [
        Character::spawn(&mut world, desc, [0.0, 0.5, 0.0]).unwrap(),
        Character::spawn(&mut world, desc, [-6.0, 1.0, 0.0]).unwrap(),
    ];
    for tick in 0..ticks {
        let t = (tick % 240) as f32 / 120.0;
        let swing = if t < 1.0 { t } else { 2.0 - t };
        world.move_to(platform, Pose::at([-6.0 + 2.0 * swing, 0.5 + swing, 0.0]));
        for (i, character) in characters.iter_mut().enumerate() {
            let (direction, jump) = script(tick + i as u32 * 17);
            character.drive(direction, jump);
            character.step(&mut world);
        }
        world.step();
    }
    let mut bits = Vec::new();
    for character in &characters {
        bits.extend(character.feet().map(f32::to_bits));
        bits.extend(character.velocity().map(f32::to_bits));
    }
    for body in world.bodies() {
        bits.extend(world.pose(body).unwrap().position.map(f32::to_bits));
    }
    bits
}

fn fingerprint(bits: &[u32]) -> u64 {
    bits.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, word| {
        word.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
    })
}

#[test]
fn the_same_inputs_give_the_same_positions_bit_for_bit() {
    let solid = played(CharacterDesc::default(), 720);
    assert_eq!(solid, played(CharacterDesc::default(), 720));
    let desc = CharacterDesc {
        wall_slide: Some(2.0),
        wall_jump: Some([4.0, 6.0]),
        variable_jump: 0.5,
        ..CharacterDesc::flat(Plane::Xy)
    };
    let flat = played(desc, 720);
    assert_eq!(flat, played(desc, 720));
    if cfg!(feature = "enhanced-determinism") {
        assert_eq!(
            [fingerprint(&solid), fingerprint(&flat)],
            [0x37ab_f569_820c_94e9, 0x9028_01b4_9cb1_e102],
            "the same on Linux and on Windows under Wine; a change here changes every recorded run"
        );
    }
}

#[test]
fn a_bad_description_is_refused_by_name() {
    let mut world = world();
    let refused = |desc: CharacterDesc, world: &mut World| {
        Character::spawn(world, desc, [0.0; 3]).unwrap_err()
    };
    let error = refused(
        CharacterDesc {
            radius: 0.0,
            ..CharacterDesc::default()
        },
        &mut world,
    );
    assert!(error.contains("radius"), "{error}");
    let error = refused(
        CharacterDesc {
            height: 0.4,
            ..CharacterDesc::default()
        },
        &mut world,
    );
    assert!(error.contains("height"), "{error}");
    let error = refused(
        CharacterDesc {
            max_climb: 95.0,
            ..CharacterDesc::default()
        },
        &mut world,
    );
    assert!(error.contains("max_climb"), "{error}");
    assert!(world.bodies().is_empty());
}

#[test]
fn a_new_size_rebuilds_the_capsule_and_keeps_the_feet() {
    let (mut world, mut character) = standing(CharacterDesc::default(), [0.0, 0.0, 0.0]);
    let feet = character.feet();
    let old = character.collider();
    character
        .set_desc(
            &mut world,
            CharacterDesc {
                height: 1.0,
                radius: 0.25,
                ..CharacterDesc::default()
            },
        )
        .unwrap();
    assert_ne!(character.collider(), old);
    assert_eq!(character.feet(), feet);
    settle(&mut world, &mut character, 10);
    assert!(character.grounded());
    assert!((character.center()[1] - 0.5).abs() < 0.05);
}
