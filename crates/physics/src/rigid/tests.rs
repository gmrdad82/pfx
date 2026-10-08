use pfx_core::motion::cubic_in_out;

use super::*;

const G: f32 = 9.81;

fn still(gravity: [f32; 3]) -> World {
    World::new(WorldDesc {
        gravity,
        ..WorldDesc::default()
    })
}

fn ground(world: &mut World, friction: Friction) -> BodyId {
    let body = world.add_body(BodyDesc::fixed(Pose::at([0.0, -0.5, 0.0])));
    world
        .add_collider(
            body,
            ColliderDesc::cuboid([50.0, 0.5, 50.0]).with_friction(friction),
        )
        .unwrap();
    body
}

fn sphere(world: &mut World, desc: BodyDesc, radius: f32) -> BodyId {
    let body = world.add_body(desc);
    world
        .add_collider(body, ColliderDesc::sphere(radius))
        .unwrap();
    body
}

fn state(world: &World, body: BodyId) -> BodyState {
    world.body(body).unwrap()
}

#[test]
fn free_fall_matches_the_closed_form() {
    let mut world = World::default();
    let body = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([0.0, 100.0, 0.0])),
        0.5,
    );
    let h = world.dt();
    let steps = 120u32;
    world.run(steps);
    let s = state(&world, body);
    let n = steps as f32;
    let t = n * h;
    let velocity = -G * t;
    assert!((s.linear_velocity[1] - velocity).abs() < 1e-3, "{s:?}");
    let k = WorldDesc::default().iterations as f32;
    let (hk, m) = (h / k, n * k);
    let discrete = 100.0 - G * hk * hk * m * (m + 1.0) * 0.5;
    assert!(
        (s.pose.position[1] - discrete).abs() < 2e-3,
        "{s:?} {discrete}"
    );
    let continuous = 100.0 - 0.5 * G * t * t;
    assert!((s.pose.position[1] - continuous).abs() <= 0.5 * G * hk * t + 2e-3);
    assert_eq!(s.pose.position[0], 0.0);
    assert_eq!(s.pose.position[2], 0.0);
}

#[test]
fn an_elastic_collision_conserves_momentum() {
    let mut world = still([0.0; 3]);
    let a = world
        .add_body(BodyDesc::dynamic(Pose::at([-2.0, 0.0, 0.0])).with_velocity([3.0, 0.0, 0.0]));
    world
        .add_collider(
            a,
            ColliderDesc::sphere(0.5)
                .with_density(2.0)
                .with_restitution(1.0, Combine::Max)
                .with_friction(Friction::uniform(0.0)),
        )
        .unwrap();
    let b = world
        .add_body(BodyDesc::dynamic(Pose::at([2.0, 0.0, 0.0])).with_velocity([-1.0, 0.0, 0.0]));
    world
        .add_collider(
            b,
            ColliderDesc::sphere(0.5)
                .with_density(1.0)
                .with_restitution(1.0, Combine::Max)
                .with_friction(Friction::uniform(0.0)),
        )
        .unwrap();
    let (ma, mb) = (world.mass(a).unwrap(), world.mass(b).unwrap());
    assert!((ma / mb - 2.0).abs() < 1e-5);
    let before = ma * 3.0 - mb;
    let energy_before = 0.5 * ma * 9.0 + 0.5 * mb;
    world.run(120);
    let (va, vb) = (
        state(&world, a).linear_velocity,
        state(&world, b).linear_velocity,
    );
    let after = ma * va[0] + mb * vb[0];
    assert!(
        (after - before).abs() < 1e-4 * before.abs().max(1.0),
        "{before} {after}"
    );
    assert!((ma * va[1] + mb * vb[1]).abs() < 1e-5);
    assert!((ma * va[2] + mb * vb[2]).abs() < 1e-5);
    let expect_a = ((ma - mb) * 3.0 - 2.0 * mb) / (ma + mb);
    let expect_b = (ma - mb + 2.0 * ma * 3.0) / (ma + mb);
    assert!((va[0] - expect_a).abs() < 0.05, "{va:?} {expect_a}");
    assert!((vb[0] - expect_b).abs() < 0.05, "{vb:?} {expect_b}");
    let energy_after = 0.5 * ma * va[0] * va[0] + 0.5 * mb * vb[0] * vb[0];
    assert!((energy_after - energy_before).abs() < 0.02 * energy_before);
}

fn slide(friction: Friction) -> (World, BodyId) {
    let mut world = World::default();
    ground(
        &mut world,
        Friction::uniform(1.0).with_combine(Combine::Average),
    );
    let body = world.add_body(BodyDesc::dynamic(Pose::at([0.0, 0.5, 0.0])));
    world
        .add_collider(
            body,
            ColliderDesc::cuboid([0.5, 0.5, 0.5]).with_friction(friction),
        )
        .unwrap();
    world.run(30);
    (world, body)
}

#[test]
fn friction_stops_a_slide_at_the_closed_form_distance() {
    let mu = 0.5;
    let speed = 5.0;
    let (mut world, body) = slide(Friction::uniform(mu).with_combine(Combine::Min));
    let start = state(&world, body).pose.position[0];
    world.set_velocity(body, [speed, 0.0, 0.0], [0.0; 3]);
    let mut steps = 0;
    while state(&world, body).linear_velocity[0] > 1e-3 && steps < 600 {
        world.step();
        steps += 1;
    }
    let distance = state(&world, body).pose.position[0] - start;
    let expected = speed * speed / (2.0 * mu * G);
    assert!(
        (distance - expected).abs() < 0.05 * expected,
        "{distance} vs {expected}"
    );
    let time = steps as f32 * world.dt();
    let stop = speed / (mu * G);
    assert!(
        (time - stop).abs() < 0.05 * stop + 2.0 * world.dt(),
        "{time} vs {stop}"
    );
}

#[test]
fn static_friction_holds_where_dynamic_friction_slides() {
    let angle = 0.6f32.atan();
    let tilted = [G * angle.sin(), -G * angle.cos(), 0.0];
    let grip = Friction::new(0.8, 0.4).with_combine(Combine::Min);
    let (mut world, body) = slide(grip);
    world.set_gravity(tilted);
    let start = state(&world, body).pose.position[0];
    world.run(120);
    assert!((state(&world, body).pose.position[0] - start).abs() < 1e-2);

    world.set_velocity(body, [1.0, 0.0, 0.0], [0.0; 3]);
    world.run(60);
    let moving = state(&world, body).linear_velocity[0];
    let expected = 1.0 + G * (angle.sin() - 0.4 * angle.cos());
    assert!((moving - expected).abs() < 0.1, "{moving} vs {expected}");

    let (mut world, body) = slide(Friction::uniform(0.4).with_combine(Combine::Min));
    world.set_gravity(tilted);
    world.run(60);
    assert!(state(&world, body).linear_velocity[0] > 0.5);
}

fn shot(world_ccd: bool, bullet: bool, wall: Kind) -> f32 {
    let mut world = World::new(WorldDesc {
        gravity: [0.0; 3],
        ccd: world_ccd,
        ..WorldDesc::default()
    });
    let plate = world.add_body(BodyDesc {
        kind: wall,
        ..BodyDesc::dynamic(Pose::IDENTITY)
    });
    world
        .add_collider(plate, ColliderDesc::cuboid([0.01, 1.0, 1.0]))
        .unwrap();
    let mut desc = BodyDesc::dynamic(Pose::at([-2.0, 0.0, 0.0])).with_velocity([200.0, 0.0, 0.0]);
    desc.ccd = bullet;
    let ball = sphere(&mut world, desc, 0.05);
    world.run(10);
    state(&world, ball).pose.position[0]
}

#[test]
fn a_fast_sphere_tunnels_a_thin_box_only_without_ccd() {
    assert!(shot(true, true, Kind::Kinematic) < 0.0);
    assert!(shot(true, false, Kind::Kinematic) > 1.0);
    assert!(shot(true, false, Kind::Fixed) < 0.0);
    assert!(shot(true, true, Kind::Fixed) < 0.0);
    assert!(shot(false, true, Kind::Fixed) > 1.0);
}

#[test]
fn locked_axes_never_drift_over_ten_thousand_steps() {
    let mut world = World::default();
    ground(&mut world, Friction::default());
    let card = world.add_body(
        BodyDesc::dynamic(Pose::at([0.0, 0.6, 0.25]))
            .with_locks(Locks::PLANE_XY)
            .with_spin([3.0, 2.0, 5.0]),
    );
    world
        .add_collider(card, ColliderDesc::round_box([0.5, 0.1, 0.35], 0.02))
        .unwrap();
    let pin = world.add_body(
        BodyDesc::dynamic(Pose::at([3.0, 2.0, -1.0]))
            .with_locks(Locks {
                translation: [true; 3],
                rotation: [false; 3],
            })
            .with_velocity([1.0, 1.0, 1.0]),
    );
    world
        .add_collider(pin, ColliderDesc::cuboid([0.3, 0.3, 0.3]))
        .unwrap();
    let mut balls = Vec::new();
    for i in 0..10_000u32 {
        if i % 500 == 0 {
            let x = if i % 1000 == 0 { -3.0 } else { 3.0 };
            let ball = sphere(
                &mut world,
                BodyDesc::dynamic(Pose::at([x, 0.6, 1.5]))
                    .with_velocity([-x * 2.0, 0.5, -3.0])
                    .with_spin([1.0, 4.0, 2.0]),
                0.2,
            );
            balls.push(ball);
            world.apply_impulse_at(card, [0.3, 0.5, 0.7], [0.4, 0.6, 0.5]);
            world.apply_torque_impulse(pin, [0.2, -0.1, 0.3]);
            world.add_force(pin, [5.0, 5.0, 5.0]);
        }
        world.step();
        let s = state(&world, card);
        assert_eq!(s.pose.position[2].to_bits(), 0.25f32.to_bits(), "step {i}");
        assert_eq!(s.pose.rotation[0], 0.0, "step {i}");
        assert_eq!(s.pose.rotation[1], 0.0, "step {i}");
        assert_eq!(s.linear_velocity[2], 0.0);
        assert_eq!(s.angular_velocity[0], 0.0);
        assert_eq!(s.angular_velocity[1], 0.0);
        let p = state(&world, pin);
        assert_eq!(p.pose.position, [3.0, 2.0, -1.0], "step {i}");
        assert_eq!(p.linear_velocity, [0.0; 3]);
    }
    assert_eq!(balls.len(), 20);
}

#[test]
fn a_point_gravity_orbit_stays_bounded() {
    let mut world = World::new(WorldDesc {
        gravity: [0.0; 3],
        substeps: 2,
        ..WorldDesc::default()
    });
    let strength = 10.0;
    let radius = 2.0;
    world.add_source(Source::attractor([0.0; 3], strength));
    let speed = (strength / radius).sqrt();
    let mut desc = BodyDesc::dynamic(Pose::at([radius, 0.0, 0.0])).with_velocity([0.0, 0.0, speed]);
    desc.can_sleep = false;
    let body = sphere(&mut world, desc, 0.1);
    let period = std::f32::consts::TAU * radius / speed;
    let steps = (10.0 * period * world.rate()) as u32;
    let mut low = f32::MAX;
    let mut high = 0.0f32;
    for _ in 0..steps {
        world.step();
        let [x, y, z] = state(&world, body).pose.position;
        let r = (x * x + y * y + z * z).sqrt();
        low = low.min(r);
        high = high.max(r);
        assert!(y.abs() < 1e-4);
    }
    assert!(low > 0.95 * radius && high < 1.05 * radius, "{low} {high}");
}

#[test]
fn zones_replace_or_add_gravity_and_points_repel() {
    let mut world = World::default();
    world.add_source(Source::Zone {
        region: Region::Box {
            pose: Pose::at([10.0, 0.0, 0.0]).turned([0.0, 1.0, 0.0], 0.5),
            half: [2.0, 50.0, 2.0],
        },
        gravity: [0.0, 5.0, 0.0],
        replace: true,
    });
    world.add_source(Source::Zone {
        region: Region::Sphere {
            center: [-10.0, 0.0, 0.0],
            radius: 10.0,
        },
        gravity: [2.0, 0.0, 0.0],
        replace: false,
    });
    world.add_source(Source::Point {
        center: [20.0, 0.0, 0.0],
        strength: -4.0,
        falloff: Falloff::Constant,
        range: 3.0,
        softening: 0.0,
    });
    let up = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([10.0, 0.0, 0.0])),
        0.2,
    );
    let side = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([-10.0, 0.0, 0.0])),
        0.2,
    );
    let pushed = world.add_body(
        BodyDesc::dynamic(Pose::at([21.0, 0.0, 0.0])).with_locks(Locks {
            translation: [false, true, true],
            rotation: [false; 3],
        }),
    );
    world
        .add_collider(pushed, ColliderDesc::sphere(0.2))
        .unwrap();
    let plain = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([30.0, 0.0, 0.0])),
        0.2,
    );
    let light = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([10.0, 0.0, 1.0])).with_gravity_scale(0.5),
        0.2,
    );
    world.run(60);
    let v = |b| state(&world, b).linear_velocity;
    assert!(
        (v(up)[1] - 5.0).abs() < 1e-3 && v(up)[0] == 0.0,
        "{:?}",
        v(up)
    );
    assert!((v(light)[1] - 2.5).abs() < 1e-3, "{:?}", v(light));
    assert!((v(side)[0] - 2.0).abs() < 1e-3 && (v(side)[1] + G).abs() < 1e-3);
    let x = state(&world, pushed).pose.position[0];
    assert!(x > 22.8, "{x}");
    assert!(v(pushed)[0] > 3.8 && v(pushed)[0] < 4.01, "{:?}", v(pushed));
    assert_eq!(v(plain)[0], 0.0);
    assert!((v(plain)[1] + G).abs() < 1e-3);
}

#[test]
fn a_sensor_reports_enter_and_exit() {
    let mut world = still([0.0; 3]);
    let gate = world.add_body(BodyDesc::fixed(Pose::IDENTITY));
    let sensor = world
        .add_collider(gate, ColliderDesc::cuboid([0.5, 0.5, 0.5]).as_sensor())
        .unwrap();
    let ball = world
        .add_body(BodyDesc::dynamic(Pose::at([-3.0, 0.0, 0.0])).with_velocity([3.0, 0.0, 0.0]));
    let probe = world
        .add_collider(ball, ColliderDesc::sphere(0.25))
        .unwrap();
    let mut log = Vec::new();
    for step in 0..120 {
        world.step();
        for event in world.events() {
            log.push((step, *event));
        }
    }
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(
        log[0].1,
        Event::SensorEntered {
            sensor,
            other: probe
        }
    );
    assert_eq!(
        log[1].1,
        Event::SensorExited {
            sensor,
            other: probe
        }
    );
    let enter = (2.25f32 / 3.0 * 60.0) as i32;
    let exit = (3.75f32 / 3.0 * 60.0) as i32;
    assert!((log[0].0 - enter).abs() <= 2, "{log:?}");
    assert!((log[1].0 - exit).abs() <= 2, "{log:?}");
    assert!(state(&world, ball).pose.position[0] > 2.0);
}

#[test]
fn a_contact_reports_its_impulse() {
    let mut world = World::default();
    let floor = ground(&mut world, Friction::default());
    let ball = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([0.0, 2.0, 0.0])),
        0.25,
    );
    let mass = world.mass(ball).unwrap();
    let mut hits = Vec::new();
    for _ in 0..120 {
        world.step();
        for event in world.events() {
            if let Event::ContactStarted { a, b, impulse } = *event {
                hits.push((a, b, impulse));
            }
        }
    }
    assert_eq!(hits.len(), 1, "{hits:?}");
    let (a, b, impulse) = hits[0];
    let bodies = [world.body_of(a).unwrap(), world.body_of(b).unwrap()];
    assert!(bodies.contains(&floor) && bodies.contains(&ball));
    let speed = (2.0 * G * 1.75f32).sqrt();
    assert!(
        impulse > 0.5 * mass * speed && impulse < 2.5 * mass * speed,
        "{impulse}"
    );
}

#[test]
fn a_resting_contact_past_its_threshold_reports_its_impulse() {
    let mut world = World::default();
    ground(&mut world, Friction::default());
    let mut desc = BodyDesc::dynamic(Pose::at([0.0, 0.5, 0.0]));
    desc.can_sleep = false;
    let block = world.add_body(desc);
    let collider = world
        .add_collider(
            block,
            ColliderDesc::cuboid([0.5, 0.5, 0.5]).with_impulse_threshold(0.01),
        )
        .unwrap();
    world.run(60);
    let weight = world.mass(block).unwrap() * G * world.dt();
    let reported: Vec<f32> = world
        .events()
        .iter()
        .filter_map(|event| match *event {
            Event::ContactImpulse { a, b, impulse } if a == collider || b == collider => {
                Some(impulse)
            }
            _ => None,
        })
        .collect();
    assert_eq!(reported.len(), 1, "{:?}", world.events());
    assert!(
        (reported[0] - weight).abs() < 0.05 * weight,
        "{reported:?} {weight}"
    );
}

#[test]
fn a_stack_of_ten_cards_settles_and_sleeps() {
    let mut world = World::default();
    ground(&mut world, Friction::default());
    let half = [0.5, 0.1, 0.35];
    let cards: Vec<BodyId> = (0..10)
        .map(|i| {
            let body = world.add_body(BodyDesc::dynamic(Pose::at([
                0.0,
                0.1 + 0.201 * i as f32,
                0.0,
            ])));
            world
                .add_collider(body, ColliderDesc::round_box(half, 0.02))
                .unwrap();
            body
        })
        .collect();
    let mut steps = 0;
    while !cards.iter().all(|&c| world.is_asleep(c)) && steps < 1200 {
        world.step();
        steps += 1;
    }
    assert!(steps < 1200, "the stack never slept");
    let top = state(&world, cards[9]).pose.position;
    assert!(top[0].abs() < 0.02 && top[2].abs() < 0.02, "{top:?}");
    assert!((top[1] - 1.9).abs() < 0.06, "{top:?}");
}

#[test]
fn a_kinematic_body_eased_or_sprung_pushes_what_it_meets() {
    let mut world = World::default();
    ground(&mut world, Friction::default());
    let start = Pose::at([-2.0, 0.5, 0.0]);
    let pusher = world.add_body(BodyDesc::kinematic(start));
    world
        .add_collider(pusher, ColliderDesc::cuboid([0.5, 0.5, 0.5]))
        .unwrap();
    world.set_drive(
        pusher,
        Some(Drive::ease(
            start,
            Pose::at([2.0, 0.5, 0.0]),
            1.0,
            cubic_in_out,
        )),
    );
    let crate_ = world.add_body(BodyDesc::dynamic(Pose::at([0.0, 0.5, 0.0])));
    world
        .add_collider(crate_, ColliderDesc::cuboid([0.4, 0.4, 0.4]))
        .unwrap();
    world.run(30);
    let halfway = state(&world, pusher).pose.position[0];
    assert!(halfway.abs() < 1e-3, "{halfway}");
    world.run(60);
    assert!(world.drive(pusher).unwrap().settled());
    assert_eq!(state(&world, pusher).pose.position, [2.0, 0.5, 0.0]);
    assert!(state(&world, crate_).pose.position[0] > 2.5);

    let mut world = still([0.0; 3]);
    let follower = world.add_body(BodyDesc::kinematic(Pose::IDENTITY));
    world
        .add_collider(follower, ColliderDesc::sphere(0.2))
        .unwrap();
    world.set_drive(follower, Some(Drive::spring(200.0, [0.0; 3])));
    if let Some(Drive::Spring(spring)) = world.drive_mut(follower) {
        spring.target = [1.0, 2.0, 0.0];
    }
    world.run(120);
    let at = state(&world, follower).pose.position;
    assert!(
        (at[0] - 1.0).abs() < 1e-3 && (at[1] - 2.0).abs() < 1e-3,
        "{at:?}"
    );
}

#[test]
fn queries_respect_layers() {
    let mut world = still([0.0; 3]);
    let floor = ground(&mut world, Friction::default());
    let ghost = world.add_body(BodyDesc::fixed(Pose::at([0.0, 3.0, 0.0])));
    let ghost_collider = world
        .add_collider(
            ghost,
            ColliderDesc::cuboid([1.0, 0.25, 1.0]).with_layers(Layers::new(2, u32::MAX)),
        )
        .unwrap();
    let zone = world.add_body(BodyDesc::fixed(Pose::at([5.0, 1.0, 0.0])));
    let zone_collider = world
        .add_collider(zone, ColliderDesc::sphere(1.0).as_sensor())
        .unwrap();
    world.step();

    let hit = world
        .raycast([0.0, 10.0, 0.0], [0.0, -1.0, 0.0], 100.0, Filter::ALL)
        .unwrap();
    assert_eq!(hit.collider, ghost_collider);
    assert!((hit.distance - 6.75).abs() < 1e-4);
    assert_eq!(hit.normal, [0.0, 1.0, 0.0]);
    let hit = world
        .raycast([0.0, 10.0, 0.0], [0.0, -2.0, 0.0], 100.0, Filter::layers(1))
        .unwrap();
    assert_eq!(hit.body, Some(floor));
    assert!((hit.distance - 10.0).abs() < 1e-4);
    assert!((hit.point[1]).abs() < 1e-4);
    assert!(
        world
            .raycast([0.0, 10.0, 0.0], [0.0, -1.0, 0.0], 5.0, Filter::layers(1))
            .is_none()
    );

    let ball = Shape::Sphere { radius: 0.5 };
    let cast = world
        .shape_cast(
            &ball,
            Pose::at([0.0, 10.0, 0.0]),
            [0.0, -1.0, 0.0],
            100.0,
            Filter::layers(1),
        )
        .unwrap();
    assert_eq!(cast.body, Some(floor));
    assert!((cast.distance - 9.5).abs() < 1e-3, "{cast:?}");
    assert!(cast.point[1].abs() < 1e-3, "{cast:?}");
    assert!((cast.normal[1] - 1.0).abs() < 1e-4, "{cast:?}");

    assert!(
        world
            .point_overlaps([5.0, 1.0, 0.0], Filter::ALL)
            .is_empty()
    );
    assert_eq!(
        world.point_overlaps([5.0, 1.0, 0.0], Filter::ALL.with_sensors()),
        vec![zone_collider]
    );
    let found = world.shape_overlaps(
        &Shape::Box {
            half: [0.5, 2.0, 0.5],
        },
        Pose::at([0.0, 1.5, 0.0]),
        Filter::ALL,
    );
    assert_eq!(found.len(), 2);
    assert!(found.contains(&ghost_collider));
    let found = world.shape_overlaps(
        &Shape::Box {
            half: [0.5, 2.0, 0.5],
        },
        Pose::at([0.0, 1.5, 0.0]),
        Filter::layers(2),
    );
    assert_eq!(found, vec![ghost_collider]);
    let excluded = world.raycast(
        [0.0, 10.0, 0.0],
        [0.0, -1.0, 0.0],
        100.0,
        Filter::ALL.excluding(ghost),
    );
    assert_eq!(excluded.unwrap().body, Some(floor));
}

#[test]
fn a_query_sees_a_collider_by_its_layer_even_when_that_layer_collides_with_nothing() {
    let mut world = still([0.0; 3]);
    let ghost = world.add_body(BodyDesc::fixed(Pose::at([0.0, 3.0, 0.0])));
    let ghost_collider = world
        .add_collider(
            ghost,
            ColliderDesc::cuboid([1.0, 0.25, 1.0]).with_layers(Layers::new(4, 0)),
        )
        .unwrap();
    world.step();
    let down = [0.0, -1.0, 0.0];
    let hit = world
        .raycast([0.0, 10.0, 0.0], down, 100.0, Filter::layers(4))
        .unwrap();
    assert_eq!(hit.collider, ghost_collider);
    assert!(
        world
            .raycast([0.0, 10.0, 0.0], down, 100.0, Filter::layers(1 | 2))
            .is_none()
    );
    let found = world.shape_overlaps(
        &Shape::Sphere { radius: 0.5 },
        Pose::at([0.0, 3.0, 0.0]),
        Filter::layers(4),
    );
    assert_eq!(found, vec![ghost_collider]);
    assert_eq!(
        world.point_overlaps([0.0, 3.0, 0.0], Filter::layers(u32::MAX)),
        vec![ghost_collider]
    );
}

#[test]
fn refreshing_the_queries_shows_new_colliders_and_moved_poses_without_a_step() {
    let mut world = still([0.0; 3]);
    let wall = world.add_body(BodyDesc::fixed(Pose::at([4.0, 0.0, 0.0])));
    let collider = world
        .add_collider(wall, ColliderDesc::cuboid([0.5, 2.0, 2.0]))
        .unwrap();
    let ahead = [1.0, 0.0, 0.0];
    assert!(world.raycast([0.0; 3], ahead, 20.0, Filter::ALL).is_none());
    world.refresh_queries();
    let hit = world.raycast([0.0; 3], ahead, 20.0, Filter::ALL).unwrap();
    assert_eq!(hit.collider, collider);
    assert!((hit.distance - 3.5).abs() < 1e-4, "{hit:?}");
    assert_eq!(world.steps(), 0);
    assert!(world.set_pose(wall, Pose::at([8.0, 0.0, 0.0])));
    let stale = world.raycast([0.0; 3], ahead, 20.0, Filter::ALL).unwrap();
    assert!((stale.distance - 3.5).abs() < 1e-4, "{stale:?}");
    world.refresh_queries();
    let moved = world.raycast([0.0; 3], ahead, 20.0, Filter::ALL).unwrap();
    assert!((moved.distance - 7.5).abs() < 1e-4, "{moved:?}");
    assert_eq!(world.steps(), 0);
    assert_eq!(world.pose(wall).unwrap().position, [8.0, 0.0, 0.0]);
}

#[test]
fn colliders_layers_keep_bodies_apart_or_together() {
    let mut world = World::default();
    let floor = world.add_body(BodyDesc::fixed(Pose::at([0.0, -0.5, 0.0])));
    world
        .add_collider(
            floor,
            ColliderDesc::cuboid([10.0, 0.5, 10.0]).with_layers(Layers::new(1, 1)),
        )
        .unwrap();
    let falls = world.add_body(BodyDesc::dynamic(Pose::at([0.0, 1.0, 0.0])));
    world
        .add_collider(
            falls,
            ColliderDesc::sphere(0.25).with_layers(Layers::new(2, u32::MAX)),
        )
        .unwrap();
    let rests = sphere(
        &mut world,
        BodyDesc::dynamic(Pose::at([2.0, 1.0, 0.0])),
        0.25,
    );
    world.run(120);
    assert!(state(&world, falls).pose.position[1] < -1.0);
    assert!((state(&world, rests).pose.position[1] - 0.25).abs() < 0.02);
}

#[test]
fn every_shape_builds_and_mass_comes_from_density_or_is_given() {
    let mut world = World::default();
    let shapes = [
        Shape::Sphere { radius: 0.5 },
        Shape::Cylinder {
            half_height: 0.5,
            radius: 0.3,
        },
        Shape::Disc {
            radius: 0.5,
            thickness: 0.05,
        },
        Shape::Capsule {
            half_height: 0.5,
            radius: 0.2,
        },
        Shape::Box {
            half: [0.5, 0.5, 0.5],
        },
        Shape::RoundBox {
            half: [0.5, 0.05, 0.35],
            radius: 0.02,
        },
        Shape::Hull {
            points: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
        },
    ];
    for shape in shapes {
        let body = world.add_body(BodyDesc::default());
        assert!(world.add_collider(body, ColliderDesc::new(shape)).is_some());
        assert!(world.mass(body).unwrap() > 0.0);
    }
    let body = world.add_body(BodyDesc::default());
    world
        .add_collider(
            body,
            ColliderDesc::cuboid([0.5, 0.5, 0.5]).with_density(3.0),
        )
        .unwrap();
    assert!((world.mass(body).unwrap() - 3.0).abs() < 1e-5);
    let body = world.add_body(BodyDesc::default().with_mass(Mass {
        mass: 7.0,
        center: [0.0; 3],
        inertia: [1.0; 3],
    }));
    world
        .add_collider(
            body,
            ColliderDesc::cuboid([0.5, 0.5, 0.5]).with_density(3.0),
        )
        .unwrap();
    assert!((world.mass(body).unwrap() - 7.0).abs() < 1e-5);
    let flat = world.add_body(BodyDesc::default());
    let degenerate = Shape::Hull {
        points: vec![[0.0; 3], [1.0, 0.0, 0.0]],
    };
    assert!(
        world
            .add_collider(flat, ColliderDesc::new(degenerate))
            .is_none()
    );
}

#[test]
fn forces_last_one_step_and_impulses_act_at_once() {
    let mut world = still([0.0; 3]);
    let body = sphere(&mut world, BodyDesc::dynamic(Pose::IDENTITY), 0.5);
    let mass = world.mass(body).unwrap();
    world.add_force(body, [mass * 6.0, 0.0, 0.0]);
    world.step();
    let v = state(&world, body).linear_velocity[0];
    assert!((v - 6.0 * world.dt()).abs() < 1e-5);
    world.step();
    assert_eq!(state(&world, body).linear_velocity[0], v);
    world.apply_impulse(body, [0.0, mass * 2.0, 0.0]);
    assert!((state(&world, body).linear_velocity[1] - 2.0).abs() < 1e-5);
    world.set_gravity_scale(body, 0.0);
    world.set_gravity([0.0, -G, 0.0]);
    world.step();
    assert!((state(&world, body).linear_velocity[1] - 2.0).abs() < 1e-5);
    world.set_damping(body, 1.0, 1.0);
    world.run(60);
    assert!(state(&world, body).linear_velocity[1] < 1.0);
    assert!(world.remove_body(body));
    assert!(world.body(body).is_none());
    assert!(!world.apply_impulse(body, [1.0, 0.0, 0.0]));
}

fn scene() -> (World, Vec<BodyId>) {
    let mut world = World::new(WorldDesc {
        substeps: 2,
        ..WorldDesc::default()
    });
    ground(&mut world, Friction::new(0.7, 0.5));
    world.add_source(Source::attractor([0.0, 4.0, 0.0], 3.0));
    let mut bodies = Vec::new();
    for i in 0..40 {
        let x = (i % 5) as f32 * 0.6 - 1.2;
        let z = (i / 5 % 4) as f32 * 0.6 - 0.9;
        let y = 1.0 + (i / 20) as f32 * 1.5 + (i % 3) as f32 * 0.1;
        let desc = BodyDesc::dynamic(Pose::at([x, y, z]).turned([1.0, 2.0, 0.5], i as f32 * 0.3))
            .with_spin([0.1 * i as f32, 0.0, -0.05 * i as f32]);
        let body = world.add_body(desc);
        let shape = match i % 4 {
            0 => ColliderDesc::sphere(0.2),
            1 => ColliderDesc::round_box([0.25, 0.05, 0.18], 0.02),
            2 => ColliderDesc::new(Shape::Capsule {
                half_height: 0.15,
                radius: 0.1,
            }),
            _ => ColliderDesc::new(Shape::Disc {
                radius: 0.2,
                thickness: 0.04,
            }),
        };
        world
            .add_collider(body, shape.with_restitution(0.3, Combine::Average))
            .unwrap();
        bodies.push(body);
    }
    let locked =
        world.add_body(BodyDesc::dynamic(Pose::at([3.0, 1.0, 0.0])).with_locks(Locks::PLANE_XY));
    world
        .add_collider(locked, ColliderDesc::round_box([0.5, 0.1, 0.35], 0.02))
        .unwrap();
    bodies.push(locked);
    let start = Pose::at([-3.0, 0.3, 0.0]);
    let sweeper = world.add_body(BodyDesc::kinematic(start));
    world
        .add_collider(sweeper, ColliderDesc::cuboid([0.3, 0.3, 1.5]))
        .unwrap();
    world.set_drive(
        sweeper,
        Some(Drive::ease(
            start,
            Pose::at([3.0, 0.3, 0.0]),
            4.0,
            cubic_in_out,
        )),
    );
    bodies.push(sweeper);
    (world, bodies)
}

#[test]
fn two_worlds_stepping_the_same_inputs_match_bit_for_bit() {
    let (mut a, bodies) = scene();
    let (mut b, _) = scene();
    let mut events = 0;
    for step in 0..1000 {
        if step % 100 == 0 {
            let body = bodies[step / 100];
            a.apply_impulse(body, [0.5, 1.0, -0.2]);
            b.apply_impulse(body, [0.5, 1.0, -0.2]);
        }
        a.step();
        b.step();
        assert_eq!(a.events(), b.events(), "step {step}");
        events += a.events().len();
    }
    assert!(events > 0);
    let bits = |w: &World| -> Vec<u32> {
        w.bodies()
            .into_iter()
            .flat_map(|id| {
                let s = w.body(id).unwrap();
                s.pose
                    .position
                    .into_iter()
                    .chain(s.pose.rotation)
                    .chain(s.linear_velocity)
                    .chain(s.angular_velocity)
                    .map(f32::to_bits)
                    .chain([u32::from(s.asleep)])
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(bits(&a), bits(&b));
}

fn rain() -> (World, Vec<BodyId>) {
    let mut world = World::default();
    ground(&mut world, Friction::default());
    for (x, z) in [(-1.3, 0.0), (1.3, 0.0), (0.0, -1.3), (0.0, 1.3)] {
        let wall = world.add_body(BodyDesc::fixed(Pose::at([x, 2.0, z])));
        let half = if x == 0.0 {
            [1.3, 2.0, 0.5]
        } else {
            [0.5, 2.0, 1.3]
        };
        world
            .add_collider(wall, ColliderDesc::cuboid(half))
            .unwrap();
    }
    let spheres = (0..500)
        .map(|i| {
            let x = (i % 10) as f32 * 0.15 - 0.675 + 0.005 * (i % 7) as f32;
            let z = (i / 10 % 10) as f32 * 0.15 - 0.675;
            let y = 1.0 + (i / 100) as f32 * 0.15;
            let desc = BodyDesc::dynamic(Pose::at([x, y, z]))
                .with_velocity([0.0, -10.0, 0.0])
                .with_ccd();
            sphere(&mut world, desc, 0.05)
        })
        .collect();
    (world, spheres)
}

fn timed(world: &mut World, steps: u32) -> f64 {
    let start = std::time::Instant::now();
    world.run(steps);
    start.elapsed().as_secs_f64() * 1000.0 / f64::from(steps)
}

#[test]
fn five_hundred_spheres_with_ccd_stay_in_their_box() {
    let (mut world, spheres) = rain();
    let per_step = timed(&mut world, 60);
    for body in spheres {
        let [x, y, z] = state(&world, body).pose.position;
        assert!(y > 0.0 && x.abs() < 0.8 && z.abs() < 0.8, "{x} {y} {z}");
    }
    eprintln!("rigid: 500 spheres with CCD at 60 Hz: {per_step:.3} ms per step on the CPU");
}

#[test]
#[ignore = "a CPU timing; run with --release -- --ignored --nocapture"]
fn five_hundred_spheres_timed() {
    let (mut world, spheres) = rain();
    let falling = timed(&mut world, 60);
    let settling = timed(&mut world, 540);
    let awake = spheres.iter().filter(|&&b| !world.is_asleep(b)).count();
    eprintln!(
        "rigid: 500 spheres with CCD at 60 Hz: {falling:.3} ms per step falling and piling (first second), {settling:.3} ms per step over the next nine, {awake} awake at the end"
    );
}
