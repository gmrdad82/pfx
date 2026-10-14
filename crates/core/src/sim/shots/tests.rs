use super::super::{Aabb, Overlaps, Rng, mix64};
use super::*;

fn shot<const N: usize>(pos: [i32; N], vel: [i32; N], life: Option<u32>) -> Shot<N> {
    Shot {
        pos,
        vel,
        half: [2; N],
        life,
        tag: 0,
    }
}

fn fire(pattern: Pattern, charge: u32, cost: u32) -> Fire {
    Fire {
        pattern,
        rate: Rate { charge, cost },
        burst: None,
        windup: 0,
        speed: 1000,
        spread: 0,
    }
}

const WIDE: Span<2> = Span::new([-1_000_000; 2], [1_000_000; 2]);

#[test]
fn handles_go_stale_and_slots_reuse_in_a_fixed_order() {
    let mut shots = Shots::<2>::new(3);
    let a = shots.spawn(shot([0, 0], [1, 0], None)).unwrap();
    let b = shots.spawn(shot([5, 0], [1, 0], None)).unwrap();
    let c = shots.spawn(shot([9, 0], [1, 0], None)).unwrap();
    assert_eq!([a.index, b.index, c.index], [0, 1, 2]);
    assert!(shots.spawn(shot([0, 0], [0, 0], None)).is_none());
    assert_eq!(shots.despawn(b).unwrap().pos, [5, 0]);
    assert!(shots.get(b).is_none());
    assert!(shots.despawn(b).is_none());
    let d = shots.spawn(shot([7, 7], [0, 0], None)).unwrap();
    assert_eq!(d.index, b.index);
    assert_ne!(d, b);
    assert!(shots.get(b).is_none());
    let order: Vec<u32> = shots.iter().map(|(id, _)| id.index).collect();
    assert_eq!(order, [0, 1, 2]);
    assert_eq!(shots.get(d).unwrap().pos, [7, 7]);
    assert!(shots.contains(a) && shots.contains(c));
}

#[test]
fn life_counts_moves_and_leaving_the_bounds_culls() {
    let mut shots = Shots::<2>::new(4);
    let short = shots.spawn(shot([0, 0], [1, 0], Some(2))).unwrap();
    let fast = shots.spawn(shot([70, 0], [20, 0], None)).unwrap();
    let bounds = Span::new([-100, -100], [100, 100]);
    assert!(shots.step(bounds).is_empty());
    assert!(shots.step(bounds).is_empty());
    assert_eq!(shots.get(short).unwrap().pos, [2, 0]);
    assert_eq!(shots.get(fast).unwrap().pos, [110, 0]);
    assert_eq!(
        shots.step(bounds),
        &[(short, End::Expired), (fast, End::Left)]
    );
    assert!(shots.is_empty());
}

#[test]
fn a_fast_shot_hits_a_thin_target_inside_its_step() {
    let mut shots = Shots::<2>::new(2);
    let id = shots.spawn(shot([0, 0], [100, 0], None)).unwrap();
    shots.step(WIDE);
    assert_eq!(shots.swept(id), Some(Span::new([-2, -2], [102, 2])));
    let mut ids = Vec::new();
    let mut boxes: Vec<Aabb> = Vec::new();
    shots.boxes(&mut ids, &mut boxes);
    let wall = [Aabb::new([50, -10], [51, 10])];
    let hits = Overlaps::new().find(&boxes, &wall).to_vec();
    assert_eq!(hits.len(), 1);
    assert_eq!(ids[hits[0].a as usize], id);
}

#[test]
fn the_charge_never_drifts() {
    let mut emitter = Emitter::new(fire(Pattern::Single, 7, 60));
    let volleys: u32 = (0..60_000).map(|_| emitter.tick(true)).sum();
    assert_eq!(volleys, 7_000);
    assert_eq!(emitter.charge(), 0);
    let mut rapid = Emitter::new(fire(Pattern::Single, 5, 2));
    let per_tick: Vec<u32> = (0..4).map(|_| rapid.tick(true)).collect();
    assert_eq!(per_tick, [2, 3, 2, 3]);
    let mut idle = Emitter::new(fire(Pattern::Single, 1, 10));
    for _ in 0..100 {
        assert_eq!(idle.tick(false), 0);
    }
    assert_eq!(idle.charge(), 10);
    assert_eq!(idle.tick(true), 1);
}

#[test]
fn bursts_keep_their_gaps_and_cooldown() {
    let mut emitter = Emitter::new(Fire {
        burst: Some(Burst {
            count: 3,
            gap: 2,
            cooldown: 4,
        }),
        ..fire(Pattern::Single, 1, 1)
    });
    let fired: Vec<u32> = (0..14).map(|_| emitter.tick(true)).collect();
    assert_eq!(fired, [1, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0, 1]);
    let mut at_once = Emitter::new(Fire {
        burst: Some(Burst {
            count: 4,
            gap: 0,
            cooldown: 0,
        }),
        ..fire(Pattern::Single, 1, 1)
    });
    assert_eq!(at_once.tick(true), 4);
}

#[test]
fn a_telegraph_winds_up_and_can_be_cancelled() {
    let mut emitter = Emitter::new(Fire {
        windup: 3,
        ..fire(Pattern::Single, 1, 1)
    });
    assert_eq!(emitter.tick(true), 0);
    assert_eq!(emitter.phase(), Phase::Winding { left: 3, total: 3 });
    assert_eq!(emitter.tick(true), 0);
    assert_eq!(emitter.tick(true), 0);
    assert_eq!(emitter.phase(), Phase::Winding { left: 1, total: 3 });
    assert_eq!(emitter.tick(true), 1);
    assert_eq!(emitter.phase(), Phase::Ready);
    assert_eq!(emitter.tick(true), 0);
    assert!(emitter.cancel());
    assert_eq!(emitter.phase(), Phase::Ready);
    assert!(!emitter.cancel());
    let mut telegraph = Telegraph::new();
    telegraph.start(2);
    assert!(!telegraph.tick());
    assert!(telegraph.tick());
    assert!(!telegraph.is_winding());
}

#[test]
fn flat_patterns_round_once() {
    let mut rng = Rng::new(1);
    let mut out = Vec::new();
    let mut fan = Emitter::new(fire(
        Pattern::Fan {
            count: 3,
            arc: 90_000,
        },
        1,
        1,
    ));
    fan.tick2([0, 0], Aim2::Angle(0), true, &mut rng, &mut out);
    assert_eq!(out, [[707, -707], [1000, 0], [707, 707]]);
    out.clear();
    let mut ring = Emitter::new(fire(Pattern::Ring { count: 4 }, 1, 1));
    ring.tick2([0, 0], Aim2::Angle(90_000), true, &mut rng, &mut out);
    assert_eq!(out, [[0, 1000], [-1000, 0], [0, -1000], [1000, 0]]);
    out.clear();
    let mut spiral = Emitter::new(fire(
        Pattern::Spiral {
            arms: 1,
            step: 300_000,
        },
        1,
        1,
    ));
    for _ in 0..3 {
        spiral.tick2([0, 0], Aim2::Angle(0), true, &mut rng, &mut out);
    }
    assert_eq!(out, [[1000, 0], [500, -866], [-500, -866]]);
    assert_eq!(spiral.spin(), 180_000);
}

#[test]
fn a_led_shot_meets_a_moving_target() {
    let mut rng = Rng::new(2);
    let mut emitter = Emitter::new(fire(Pattern::Single, 1, 1));
    let target = [3000, -1000];
    let vel = [-7, 13];
    let mut out = Vec::new();
    emitter.tick2([0, 0], Aim2::At { target, vel }, true, &mut rng, &mut out);
    let t = intercept([0, 0], target, vel, 1000).unwrap();
    let ticks = t.round() as i32;
    let shot = [out[0][0] * ticks, out[0][1] * ticks];
    let there = [target[0] + vel[0] * ticks, target[1] + vel[1] * ticks];
    let miss = (shot[0] - there[0]).abs().max((shot[1] - there[1]).abs());
    assert!(miss < 1000, "{shot:?} {there:?}");
    out.clear();
    let mut slow = Emitter::new(Fire {
        speed: 5,
        ..fire(Pattern::Single, 1, 1)
    });
    slow.tick2(
        [0, 0],
        Aim2::At {
            target: [100, 0],
            vel: [0, 50],
        },
        true,
        &mut rng,
        &mut out,
    );
    assert_eq!(out, [[5, 0]]);
    assert!(intercept([0, 0], [0, 0], [1, 0], 5).is_none());
}

#[test]
fn deep_patterns_lie_in_the_basis() {
    let mut rng = Rng::new(3);
    let mut out = Vec::new();
    let aim = Aim3::Basis {
        forward: [1, 0, 0],
        up: [0, 0, 1],
    };
    let mut fan = Emitter::new(fire(
        Pattern::Fan {
            count: 3,
            arc: 90_000,
        },
        1,
        1,
    ));
    fan.tick3([0; 3], aim, true, &mut rng, &mut out);
    assert_eq!(out, [[707, 707, 0], [1000, 0, 0], [707, -707, 0]]);
    out.clear();
    let mut crown = Emitter::new(fire(
        Pattern::Crown {
            count: 4,
            tilt: 60_000,
        },
        1,
        1,
    ));
    crown.tick3([0; 3], aim, true, &mut rng, &mut out);
    assert_eq!(
        out,
        [[500, -866, 0], [500, 0, 866], [500, 866, 0], [500, 0, -866]]
    );
    out.clear();
    let mut cone = Emitter::new(Fire {
        spread: 10_000,
        ..fire(Pattern::Ring { count: 32 }, 1, 1)
    });
    cone.tick3(
        [0; 3],
        Aim3::Basis {
            forward: [0, 0, 5],
            up: [0, 0, 1],
        },
        true,
        &mut rng,
        &mut out,
    );
    for v in &out {
        let len2: i64 = v.iter().map(|&c| i64::from(c) * i64::from(c)).sum();
        assert!((990_000..=1_010_000).contains(&len2), "{v:?}");
    }
}

#[test]
fn followers_catch_up_at_their_limit() {
    let mut members = [
        Member {
            pos: [0, 0],
            offset: [-10, 0],
            limit: None,
        },
        Member {
            pos: [0, 0],
            offset: [0, -10],
            limit: Some(3),
        },
    ];
    follow(&mut members, [100, 0]);
    assert_eq!(members[0].pos, [90, 0]);
    assert_eq!(members[1].pos, [3, 0]);
    assert_eq!(toward([0, 0, 0], [3, 4, 0], 5), [3, 4, 0]);
    assert_eq!(toward([0, 0], [30, 40], 5), [3, 4]);
    assert_eq!(toward([0, 0], [30, 40], 0), [0, 0]);
}

#[test]
fn stuns_and_slows_stack_by_their_rules() {
    let mut mover = Mover::new([0, 0], 10);
    mover.slow(1, 2, 3);
    mover.slow(2, 4, 5);
    mover.slow(3, 4, 10);
    assert_eq!(mover.slows().len(), 2);
    assert_eq!(mover.speed(), 5);
    mover.stun(2);
    mover.stun(1);
    let path: Vec<i32> = (0..8).map(|_| mover.step([1000, 0])[0]).collect();
    assert_eq!(path, [0, 0, 5, 10, 15, 22, 29, 36]);
    for _ in 0..3 {
        mover.step([1000, 0]);
    }
    assert_eq!(mover.speed(), 10);
    assert!(mover.slows().is_empty());
}

fn hash_shots<const N: usize>(shots: &Shots<N>) -> u64 {
    shots.iter().fold(0, |h, (id, s)| {
        let mut h = mix64(h ^ u64::from(id.index) ^ (u64::from(id.generation) << 32));
        for c in s.pos.iter().chain(&s.vel) {
            h = mix64(h ^ (*c as u32 as u64));
        }
        h
    })
}

fn flat_scene(ticks: u32) -> (Shots<2>, Vec<Emitter>, Vec<u64>) {
    let mut rng = Rng::new(92).fork(1);
    let mut shots = Shots::<2>::new(4096);
    let patterns = [
        Pattern::Fan {
            count: 5,
            arc: 40_000,
        },
        Pattern::Ring { count: 12 },
        Pattern::Spiral {
            arms: 3,
            step: 7_500,
        },
        Pattern::Single,
    ];
    let mut emitters: Vec<Emitter> = (0..50)
        .map(|i| {
            Emitter::new(Fire {
                pattern: patterns[i % 4],
                rate: Rate {
                    charge: 3 + i as u32 % 5,
                    cost: 200,
                },
                burst: (i % 7 == 0).then_some(Burst {
                    count: 3,
                    gap: 2,
                    cooldown: 20,
                }),
                windup: if i % 5 == 0 { 6 } else { 0 },
                speed: 300 + 20 * i as i32,
                spread: if i % 3 == 0 { 2_000 } else { 0 },
            })
        })
        .collect();
    let mut out = Vec::new();
    let mut hashes = Vec::new();
    let bounds = Span::new([-200_000, -120_000], [200_000, 120_000]);
    for tick in 0..ticks {
        for (i, emitter) in emitters.iter_mut().enumerate() {
            let origin = [i as i32 * 7_000 - 175_000, (i as i32 % 9) * 20_000 - 80_000];
            let aim = if i % 2 == 0 {
                Aim2::At {
                    target: [(tick as i32) * 40 - 20_000, 0],
                    vel: [40, 0],
                }
            } else {
                Aim2::Angle(i as i32 * 9_000)
            };
            out.clear();
            emitter.tick2(origin, aim, true, &mut rng, &mut out);
            for &vel in &out {
                let _ = shots.spawn(Shot {
                    pos: origin,
                    vel,
                    half: [4, 8],
                    life: Some(400),
                    tag: i as u32,
                });
            }
        }
        shots.step(bounds);
        hashes.push(hash_shots(&shots));
    }
    (shots, emitters, hashes)
}

fn deep_scene(ticks: u32) -> (Shots<3>, Vec<u64>) {
    let mut rng = Rng::new(92).fork(2);
    let mut shots = Shots::<3>::new(4096);
    let mut emitters: Vec<Emitter> = (0..50)
        .map(|i: i32| {
            Emitter::new(Fire {
                pattern: if i % 2 == 0 {
                    Pattern::Crown {
                        count: 8,
                        tilt: 20_000,
                    }
                } else {
                    Pattern::Fan {
                        count: 4,
                        arc: 30_000,
                    }
                },
                rate: Rate {
                    charge: 2 + i as u32 % 3,
                    cost: 400,
                },
                burst: None,
                windup: 0,
                speed: 500 + 10 * i,
                spread: if i % 4 == 0 { 3_000 } else { 0 },
            })
        })
        .collect();
    let mut out = Vec::new();
    let mut hashes = Vec::new();
    let bounds = Span::new([-300_000; 3], [300_000; 3]);
    for tick in 0..ticks {
        for (i, emitter) in emitters.iter_mut().enumerate() {
            let origin = [i as i32 * 5_000 - 125_000, 0, (i as i32 % 5) * 3_000];
            let aim = if i % 3 == 0 {
                Aim3::At {
                    target: [0, tick as i32 * 30, 10_000],
                    vel: [0, 30, 0],
                    up: [0, 0, 1],
                }
            } else {
                Aim3::Basis {
                    forward: [1, i as i32 - 25, 2],
                    up: [0, 0, 1],
                }
            };
            out.clear();
            emitter.tick3(origin, aim, true, &mut rng, &mut out);
            for &vel in &out {
                let _ = shots.spawn(Shot {
                    pos: origin,
                    vel,
                    half: [6; 3],
                    life: Some(500),
                    tag: i as u32,
                });
            }
        }
        shots.step(bounds);
        hashes.push(hash_shots(&shots));
    }
    (shots, hashes)
}

#[test]
fn scenes_replay_bit_for_bit() {
    let (flat, _, flat_hashes) = flat_scene(300);
    let (deep, deep_hashes) = deep_scene(300);
    assert_eq!(flat_scene(300).2, flat_hashes);
    assert_eq!(deep_scene(300).1, deep_hashes);
    assert_eq!(
        (flat.len(), flat_hashes[299], deep.len(), deep_hashes[299]),
        (
            1620,
            6_559_739_396_303_870_446,
            592,
            8_229_261_719_678_727_378
        )
    );
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_two_thousand_shots_and_fifty_emitters() {
    use std::time::Instant;
    let (mut flat, mut emitters, _) = flat_scene(400);
    let mut rng = Rng::new(7);
    let mut out = Vec::new();
    let mut ids = Vec::new();
    let mut boxes: Vec<Aabb> = Vec::new();
    while flat.len() < 2_000 {
        let _ = flat.spawn(shot(
            [0, 0],
            [rng.range_i32(-9, 10), rng.range_i32(-9, 10)],
            None,
        ));
    }
    let bounds = Span::new([-200_000, -120_000], [200_000, 120_000]);
    let ticks = 2_000;
    let start = Instant::now();
    let mut live = 0;
    for tick in 0..ticks {
        for (i, emitter) in emitters.iter_mut().enumerate() {
            let origin = [i as i32 * 7_000 - 175_000, 0];
            out.clear();
            emitter.tick2(
                origin,
                Aim2::At {
                    target: [tick, 0],
                    vel: [1, 0],
                },
                true,
                &mut rng,
                &mut out,
            );
            for &vel in &out {
                let _ = flat.spawn(shot(origin, vel, Some(400)));
            }
        }
        flat.step(bounds);
        flat.boxes(&mut ids, &mut boxes);
        live += flat.len();
    }
    let flat_cost = start.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);
    let mut deep = Shots::<3>::new(4096);
    let mut emitters3: Vec<Emitter> = (0..50)
        .map(|i| {
            Emitter::new(Fire {
                spread: 2_000,
                ..fire(
                    Pattern::Crown {
                        count: 8,
                        tilt: 20_000,
                    },
                    1 + i % 3,
                    40,
                )
            })
        })
        .collect();
    let mut out3 = Vec::new();
    let mut boxes3: Vec<Span<3>> = Vec::new();
    let bounds3 = Span::new([-300_000; 3], [300_000; 3]);
    let mut live3 = 0;
    let mut start = Instant::now();
    for tick in -100..ticks {
        if tick == 0 {
            start = Instant::now();
            live3 = 0;
        }
        for (i, emitter) in emitters3.iter_mut().enumerate() {
            let origin = [i as i32 * 5_000 - 125_000, 0, 0];
            out3.clear();
            let aim = Aim3::At {
                target: [0, tick, 0],
                vel: [0, 1, 0],
                up: [0, 0, 1],
            };
            emitter.tick3(origin, aim, true, &mut rng, &mut out3);
            for &vel in &out3 {
                let _ = deep.spawn(shot(origin, vel, Some(100)));
            }
        }
        deep.step(bounds3);
        deep.boxes(&mut ids, &mut boxes3);
        live3 += deep.len();
    }
    let deep_cost = start.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);
    println!(
        "2D: {flat_cost:.1} us a tick, {} live on average; 3D: {deep_cost:.1} us a tick, {} live on average",
        live / ticks as usize,
        live3 / ticks as usize
    );
}
