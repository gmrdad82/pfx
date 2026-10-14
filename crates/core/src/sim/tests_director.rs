use super::{
    Director, DirectorError, Entry, Lull, NO_SPOT, ONE, Phase, Ramp, Region, Rng, Schedule,
    SpawnLull, SpawnOrder, Spots, Table, Window, mix64,
};

const PER_TICK: i32 = 3600 * ONE;

fn plain(entries: Vec<Entry>, phases: Vec<Phase>) -> Schedule {
    Schedule {
        tables: vec![Table::new(entries)],
        phases,
        ..Schedule::default()
    }
}

fn three() -> Vec<Entry> {
    vec![Entry::new(10, 5), Entry::new(11, 3), Entry::new(12, 1)]
}

fn steady(start: u64, end: u64) -> Phase {
    let mut phase = Phase::new(start, end, 0);
    phase.rate = Ramp::flat(PER_TICK);
    phase
}

fn run(director: &mut Director, ticks: u64, spots: &Spots) -> Vec<Vec<SpawnOrder>> {
    let mut out = Vec::new();
    let mut all = Vec::new();
    for _ in 0..ticks {
        director.tick(spots, &mut out);
        all.push(out.clone());
    }
    all
}

fn spawn_ticks(all: &[Vec<SpawnOrder>]) -> Vec<usize> {
    all.iter()
        .enumerate()
        .filter(|(_, o)| !o.is_empty())
        .map(|(t, _)| t)
        .collect()
}

fn digest(all: &[Vec<SpawnOrder>]) -> u64 {
    let mut h = 0u64;
    for (t, orders) in all.iter().enumerate() {
        h = mix64(h ^ t as u64);
        for o in orders {
            for v in [
                o.entry as u64,
                o.count as u64,
                o.speed as u32 as u64,
                o.position[0] as u32 as u64,
                o.position[1] as u32 as u64,
                o.position[2] as u32 as u64,
                o.spot as u64,
            ] {
                h = mix64(h ^ v);
            }
        }
    }
    h
}

#[test]
fn bad_data_is_rejected_with_a_clear_error() {
    let ok = || plain(three(), vec![steady(0, 100)]);
    assert!(Director::new(ok(), 1).is_ok());

    let mut s = ok();
    s.tick_rate = 0;
    assert_eq!(s.check(), Err(DirectorError::TickRate));

    let s = Schedule {
        phases: vec![steady(0, 10)],
        ..Schedule::default()
    };
    assert_eq!(s.check(), Err(DirectorError::NoTables));

    let s = plain(
        vec![Entry::new(1, 0), Entry::new(2, 0)],
        vec![steady(0, 10)],
    );
    assert_eq!(s.check(), Err(DirectorError::ZeroWeight { table: 0 }));

    let s = plain(vec![], vec![]);
    assert_eq!(s.check(), Err(DirectorError::ZeroWeight { table: 0 }));

    let s = plain(
        vec![Entry::new(1, 1), Entry::new(1, 2)],
        vec![steady(0, 10)],
    );
    assert_eq!(s.check(), Err(DirectorError::DuplicateEntry { id: 1 }));

    let mut s = ok();
    s.phases[0].end = 0;
    assert_eq!(s.check(), Err(DirectorError::BadPhase { phase: 0 }));

    let mut s = ok();
    s.phases[0].table = 4;
    assert_eq!(
        s.check(),
        Err(DirectorError::UnknownTable { phase: 0, table: 4 })
    );

    let s = plain(three(), vec![steady(0, 50), steady(49, 90)]);
    assert_eq!(
        s.check(),
        Err(DirectorError::Overlap {
            first: 0,
            second: 1
        })
    );
    let mut s = plain(three(), vec![steady(0, 50), steady(50, 90)]);
    assert_eq!(s.check(), Ok(()));
    s.phases[1].start = 10;
    s.allow_overlap = true;
    assert_eq!(s.check(), Ok(()));

    let mut s = ok();
    s.tables[0].cap = 0;
    assert_eq!(s.check(), Err(DirectorError::ZeroCap { phase: 0 }));
    let mut s = ok();
    s.cap = 0;
    assert_eq!(s.check(), Err(DirectorError::ZeroCap { phase: 0 }));
    s.phases[0].rate = Ramp::flat(0);
    assert_eq!(s.check(), Ok(()));

    let mut s = plain(vec![Entry::new(1, 1).costing(5)], vec![steady(0, 10)]);
    s.cap = 4;
    assert_eq!(s.check(), Err(DirectorError::Unspawnable { phase: 0 }));

    let mut s = plain(vec![Entry::new(1, 1).tagged(1)], vec![steady(0, 10)]);
    s.phases[0].tag_filter = 2;
    assert_eq!(s.check(), Err(DirectorError::Unspawnable { phase: 0 }));

    let mut s = ok();
    s.phases[0].rate = Ramp::linear(-ONE, ONE);
    assert_eq!(s.check(), Err(DirectorError::BadRamp { phase: 0 }));
    let mut s = ok();
    s.phases[0].group = Ramp::stepped(ONE, 3 * ONE, 1);
    assert_eq!(s.check(), Err(DirectorError::BadRamp { phase: 0 }));

    let mut s = ok();
    s.phases[0].on_clear = Some(Lull::between(9, 3));
    assert_eq!(s.check(), Err(DirectorError::BadLull { phase: 0 }));

    let mut s = ok();
    s.lulls.push(Window { start: 7, end: 7 });
    assert_eq!(s.check(), Err(DirectorError::BadWindow { window: 0 }));

    assert!(
        DirectorError::ZeroWeight { table: 2 }
            .to_string()
            .contains("table 2")
    );
}

#[test]
fn a_rate_in_spawns_per_minute_fires_exactly_that_often() {
    let mut s = plain(three(), vec![Phase::new(0, 3600, 0)]);
    s.phases[0].rate = Ramp::flat(90 * ONE);
    let mut d = Director::new(s, 7).unwrap();
    let all = run(&mut d, 3600, &Spots::EMPTY);
    assert_eq!(all.iter().map(Vec::len).sum::<usize>(), 90);

    let mut s = plain(three(), vec![Phase::new(0, 3600, 0)]);
    s.phases[0].rate = Ramp::flat(ONE / 2);
    let mut d = Director::new(s, 7).unwrap();
    let all = run(&mut d, 7200, &Spots::EMPTY);
    assert_eq!(all.iter().map(Vec::len).sum::<usize>(), 0);
    let mut s = plain(three(), vec![Phase::new(0, 7200, 0)]);
    s.phases[0].rate = Ramp::flat(ONE / 2);
    let mut d = Director::new(s, 7).unwrap();
    let all = run(&mut d, 7200, &Spots::EMPTY);
    assert_eq!(all.iter().map(Vec::len).sum::<usize>(), 1);
}

#[test]
fn nothing_spawns_outside_a_phase() {
    let s = plain(three(), vec![steady(10, 20), steady(30, 40)]);
    let mut d = Director::new(s, 3).unwrap();
    let all = run(&mut d, 50, &Spots::EMPTY);
    let ticks = spawn_ticks(&all);
    assert!(
        ticks
            .iter()
            .all(|t| (10..20).contains(t) || (30..40).contains(t))
    );
    assert_eq!(ticks.len(), 20);
}

#[test]
fn ramps_are_linear_or_stepped_in_fixed_point() {
    let linear = Ramp::linear(0, 100 * ONE);
    assert_eq!(linear.at(0, 100), 0);
    assert_eq!(linear.at(50, 100), 50 * ONE);
    assert_eq!(linear.at(100, 100), 100 * ONE);
    let falling = Ramp::linear(10 * ONE, 2 * ONE);
    assert_eq!(falling.at(50, 100), 6 * ONE);
    let stepped = Ramp::stepped(0, 3 * ONE, 4);
    assert_eq!(stepped.at(24, 100), 0);
    assert_eq!(stepped.at(25, 100), ONE);
    assert_eq!(stepped.at(50, 100), 2 * ONE);
    assert_eq!(stepped.at(99, 100), 3 * ONE);
    assert_eq!(Ramp::flat(5).at(1_000_000, 3), 5);
    assert_eq!(
        Ramp::linear(0, i32::MAX).at(u64::MAX / 2, u64::MAX),
        i32::MAX / 2
    );
}

#[test]
fn group_size_and_speed_follow_their_ramps() {
    let mut phase = steady(0, 100);
    phase.group = Ramp::linear(ONE, 5 * ONE);
    phase.speed = Ramp::linear(ONE, 2 * ONE);
    let mut d = Director::new(plain(three(), vec![phase]), 5).unwrap();
    let all = run(&mut d, 100, &Spots::EMPTY);
    let orders: Vec<_> = all.iter().flatten().collect();
    assert_eq!(orders.len(), 100);
    assert_eq!(orders[0].count, 1);
    assert_eq!(orders[0].speed, ONE);
    assert!(orders.windows(2).all(|w| w[0].count <= w[1].count));
    assert!(orders.windows(2).all(|w| w[0].speed <= w[1].speed));
    assert!(orders[99].count >= 4);
    assert!(orders[99].speed > 2 * ONE - ONE / 50);
}

#[test]
fn tag_filters_and_weights_decide_the_pick() {
    let entries = vec![
        Entry::new(1, 1).tagged(1),
        Entry::new(2, 1).tagged(2),
        Entry::new(3, 1).tagged(3),
        Entry::new(4, 0).tagged(2),
    ];
    let mut phase = steady(0, 400);
    phase.tag_filter = 2;
    let mut d = Director::new(plain(entries.clone(), vec![phase]), 9).unwrap();
    let all = run(&mut d, 400, &Spots::EMPTY);
    let ids: Vec<u32> = all.iter().flatten().map(|o| o.entry).collect();
    assert_eq!(ids.len(), 400);
    assert!(ids.iter().all(|&i| i == 2 || i == 3));
    assert!(ids.iter().filter(|&&i| i == 2).count() > 140);
    assert!(ids.iter().filter(|&&i| i == 3).count() > 140);

    let heavy = vec![Entry::new(1, 90), Entry::new(2, 10)];
    let mut d = Director::new(plain(heavy, vec![steady(0, 1000)]), 9).unwrap();
    let all = run(&mut d, 1000, &Spots::EMPTY);
    let ones = all.iter().flatten().filter(|o| o.entry == 1).count();
    assert!((850..950).contains(&ones), "{ones}");
}

#[test]
fn the_overall_cap_holds_and_a_death_frees_room() {
    let mut s = plain(three(), vec![steady(0, 100)]);
    s.cap = 5;
    let mut d = Director::new(s, 2).unwrap();
    let all = run(&mut d, 20, &Spots::EMPTY);
    assert_eq!(all.iter().flatten().map(|o| o.count).sum::<u32>(), 5);
    assert_eq!(d.load(), 5);
    let id = all.iter().flatten().next().unwrap().entry;
    let alive = d.alive(id);
    assert_eq!(d.died(id, alive + 10), alive);
    assert_eq!(d.alive(id), 0);
    assert_eq!(d.load(), 5 - alive as u64);
    let more = run(&mut d, 20, &Spots::EMPTY);
    assert_eq!(more.iter().flatten().map(|o| o.count).sum::<u32>(), alive);
    assert_eq!(d.load(), 5);
    assert_eq!(d.died(999, 3), 0);
}

#[test]
fn costs_and_table_caps_bound_what_is_alive() {
    let entries = vec![
        Entry::new(1, 4).costing(3),
        Entry::new(2, 4).costing(1),
        Entry::new(3, 4).costing(0),
    ];
    let mut phase = steady(0, 2000);
    phase.group = Ramp::flat(4 * ONE);
    let mut s = plain(entries, vec![phase]);
    s.tables[0].cap = 7;
    let mut d = Director::new(s, 11).unwrap();
    let mut out = Vec::new();
    let mut seen_free = false;
    for t in 0..2000u32 {
        d.tick(&Spots::EMPTY, &mut out);
        assert!(d.load() <= 7 && d.table_load(0) <= 7);
        seen_free |= out.iter().any(|o| o.entry == 3);
        if t % 7 == 0 {
            for id in [1, 2, 3] {
                d.died(id, 100);
            }
        }
    }
    assert!(seen_free);
    let mut s = plain(
        vec![Entry::new(1, 1)],
        vec![{
            let mut p = steady(0, 10);
            p.group = Ramp::flat(4 * ONE);
            p
        }],
    );
    s.cap = 6;
    let mut d = Director::new(s, 1).unwrap();
    let all = run(&mut d, 10, &Spots::EMPTY);
    let counts: Vec<u32> = all.iter().flatten().map(|o| o.count).collect();
    assert_eq!(counts, vec![4, 2]);
}

#[test]
fn a_lull_follows_a_number_of_spawns() {
    let mut phase = steady(0, 40);
    phase.after_spawns = Some(SpawnLull {
        spawns: 3,
        lull: Lull::fixed(10),
    });
    let mut d = Director::new(plain(three(), vec![phase]), 4).unwrap();
    let all = run(&mut d, 40, &Spots::EMPTY);
    let ticks = spawn_ticks(&all);
    assert_eq!(&ticks[..7], &[0, 1, 2, 13, 14, 15, 26]);
}

#[test]
fn a_cleared_wave_starts_a_lull() {
    let mut phase = steady(0, 100);
    phase.on_clear = Some(Lull::fixed(20));
    let mut s = plain(three(), vec![phase]);
    s.cap = 3;
    let mut d = Director::new(s, 4).unwrap();
    let mut out = Vec::new();
    let mut ticks = Vec::new();
    for t in 0..60 {
        if t == 10 {
            for id in [10, 11, 12] {
                d.died(id, 9);
            }
        }
        d.tick(&Spots::EMPTY, &mut out);
        if !out.is_empty() {
            ticks.push(t);
        }
    }
    assert!(ticks.iter().take_while(|&&t| t < 10).count() >= 1);
    let after: Vec<i32> = ticks.into_iter().filter(|&t| t >= 10).collect();
    assert_eq!(after[0], 30);
}

#[test]
fn a_quiet_window_spawns_nothing_and_stores_no_burst() {
    let mut s = plain(three(), vec![steady(0, 30)]);
    s.lulls.push(Window { start: 5, end: 15 });
    let mut d = Director::new(s, 4).unwrap();
    let mut out = Vec::new();
    let mut per_tick = Vec::new();
    for t in 0..30 {
        assert_eq!(d.is_quiet(), (5..15).contains(&t));
        d.tick(&Spots::EMPTY, &mut out);
        per_tick.push(out.len());
    }
    assert!(per_tick[5..15].iter().all(|&n| n == 0));
    assert_eq!(per_tick[15], 1);
    assert_eq!(per_tick.iter().sum::<usize>(), 20);
}

#[test]
fn overlapping_phases_each_spawn_when_allowed() {
    let mut s = plain(three(), vec![steady(0, 20), steady(10, 30)]);
    s.allow_overlap = true;
    let mut d = Director::new(s, 4).unwrap();
    let all = run(&mut d, 30, &Spots::EMPTY);
    assert_eq!(all[5].len(), 1);
    assert_eq!(all[15].len(), 2);
    assert_eq!(all[25].len(), 1);
}

#[test]
fn positions_come_from_points_and_regions_in_2d_and_3d() {
    let points = [[1, 2, 3], [40, 50, 60]];
    let mut d = Director::new(plain(three(), vec![steady(0, 100)]), 6).unwrap();
    let all = run(&mut d, 100, &Spots::points(&points));
    let mut used = [false; 2];
    for o in all.iter().flatten() {
        assert_eq!(o.position, points[o.spot as usize]);
        used[o.spot as usize] = true;
    }
    assert_eq!(used, [true, true]);

    let regions = [
        Region::flat([-10, 20], [10, 30]),
        Region::new([100, 100, 100], [105, 106, 107]),
    ];
    let mut d = Director::new(plain(three(), vec![steady(0, 200)]), 6).unwrap();
    let all = run(&mut d, 200, &Spots::regions(&regions));
    let mut hit = [false; 2];
    for o in all.iter().flatten() {
        let r = regions[o.spot as usize];
        hit[o.spot as usize] = true;
        for axis in 0..3 {
            let top = r.max[axis].max(r.min[axis] + 1);
            assert!(
                (r.min[axis]..top).contains(&o.position[axis]) || o.position[axis] == r.min[axis]
            );
        }
        if o.spot == 0 {
            assert_eq!(o.position[2], 0);
        }
    }
    assert_eq!(hit, [true, true]);

    let mut d = Director::new(plain(three(), vec![steady(0, 5)]), 6).unwrap();
    let all = run(&mut d, 5, &Spots::EMPTY);
    assert!(
        all.iter()
            .flatten()
            .all(|o| o.spot == NO_SPOT && o.position == [0; 3])
    );
}

#[test]
fn each_concern_draws_from_its_own_stream() {
    let entries_of = |all: &[Vec<SpawnOrder>]| -> Vec<(u32, u32)> {
        all.iter().flatten().map(|o| (o.entry, o.count)).collect()
    };
    let base = plain(three(), vec![steady(0, 60)]);
    let few = [[0, 0, 0]];
    let many = [[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]];
    let regions = [Region::flat([0, 0], [9, 9])];
    let a = run(
        &mut Director::new(base.clone(), 8).unwrap(),
        60,
        &Spots::points(&few),
    );
    let b = run(
        &mut Director::new(base.clone(), 8).unwrap(),
        60,
        &Spots::points(&many),
    );
    let c = run(
        &mut Director::new(base.clone(), 8).unwrap(),
        60,
        &Spots::regions(&regions),
    );
    assert_eq!(entries_of(&a), entries_of(&b));
    assert_eq!(entries_of(&a), entries_of(&c));
    assert_ne!(
        a.iter().flatten().map(|o| o.position).collect::<Vec<_>>(),
        b.iter().flatten().map(|o| o.position).collect::<Vec<_>>()
    );

    let mut calm = base.clone();
    calm.phases[0].after_spawns = Some(SpawnLull {
        spawns: 2,
        lull: Lull::fixed(0),
    });
    let mut jittery = base.clone();
    jittery.phases[0].after_spawns = Some(SpawnLull {
        spawns: 2,
        lull: Lull::between(1, 9),
    });
    let x = run(&mut Director::new(calm, 8).unwrap(), 60, &Spots::EMPTY);
    let y = run(&mut Director::new(jittery, 8).unwrap(), 60, &Spots::EMPTY);
    let xs = entries_of(&x);
    let ys = entries_of(&y);
    assert!(ys.len() < xs.len());
    assert_eq!(xs[..ys.len()], ys[..]);
    assert_ne!(spawn_ticks(&x), spawn_ticks(&y));
}

fn rich() -> Schedule {
    let mut table = Table::new(vec![
        Entry::new(1, 6).tagged(1),
        Entry::new(2, 3).tagged(1).costing(2),
        Entry::new(3, 2).tagged(2).costing(3),
        Entry::new(4, 1).tagged(2).costing(5),
    ]);
    table.cap = 14;
    let mut a = Phase::new(0, 300, 0);
    a.rate = Ramp::linear(60 * ONE, 600 * ONE);
    a.group = Ramp::stepped(ONE, 4 * ONE, 3);
    a.speed = Ramp::linear(ONE, 3 * ONE / 2);
    a.after_spawns = Some(SpawnLull {
        spawns: 7,
        lull: Lull::between(5, 25),
    });
    let mut b = Phase::new(300, 600, 0);
    b.rate = Ramp::flat(1200 * ONE);
    b.tag_filter = 2;
    b.on_clear = Some(Lull::between(10, 30));
    Schedule {
        cap: 12,
        tables: vec![table],
        phases: vec![a, b],
        lulls: vec![Window {
            start: 450,
            end: 480,
        }],
        ..Schedule::default()
    }
}

fn play(d: &mut Director, from: u64, to: u64, spots: &Spots) -> Vec<Vec<SpawnOrder>> {
    let mut out = Vec::new();
    let mut all = Vec::new();
    let killer = Rng::new(99);
    for t in from..to {
        d.tick(spots, &mut out);
        all.push(out.clone());
        let mut kr = killer.fork(t);
        if kr.below(4) == 0 {
            d.died(1 + kr.below(4) as u32, 1 + kr.below(3) as u32);
        }
        if t % 97 == 0 {
            for id in 1..=4 {
                d.died(id, 50);
            }
        }
    }
    all
}

#[test]
fn the_same_seed_gives_the_same_bytes_and_the_output_is_frozen() {
    let points = [[0, 0, 0], [5, 6, 7], [-3, 9, 2]];
    let regions = [Region::new([0, 0, 0], [50, 60, 70])];
    let spots = Spots {
        points: &points,
        regions: &regions,
    };
    let a = play(&mut Director::new(rich(), 42).unwrap(), 0, 600, &spots);
    let b = play(&mut Director::new(rich(), 42).unwrap(), 0, 600, &spots);
    assert_eq!(a, b);
    let c = play(&mut Director::new(rich(), 43).unwrap(), 0, 600, &spots);
    assert_ne!(digest(&a), digest(&c));
    assert!(a.iter().map(Vec::len).sum::<usize>() > 30);
    assert_eq!(digest(&a), GOLDEN);
}

const GOLDEN: u64 = 15396052419400446833;

#[test]
fn a_saved_director_resumes_exactly() {
    let points = [[0, 0, 0], [5, 6, 7]];
    let regions = [Region::new([0, 0, 0], [50, 60, 70])];
    let spots = Spots {
        points: &points,
        regions: &regions,
    };
    let mut whole = Director::new(rich(), 42).unwrap();
    let first = play(&mut whole, 0, 250, &spots);
    let bytes = whole.save();
    let rest = play(&mut whole, 250, 600, &spots);

    let mut resumed = Director::new(rich(), 1).unwrap();
    resumed.restore(&bytes).unwrap();
    assert_eq!(resumed.now(), 250);
    assert_eq!(play(&mut resumed, 250, 600, &spots), rest);
    assert_eq!(resumed.save(), whole.save());
    assert!(first.iter().map(Vec::len).sum::<usize>() > 0);

    let mut other = rich();
    other.cap = 13;
    let mut wrong = Director::new(other, 42).unwrap();
    assert_eq!(wrong.restore(&bytes), Err(DirectorError::Mismatch));
    let mut d = Director::new(rich(), 42).unwrap();
    assert!(matches!(
        d.restore(&bytes[..bytes.len() - 1]),
        Err(DirectorError::Length { .. })
    ));
    let mut bad = bytes.clone();
    bad[0] = 9;
    assert_eq!(d.restore(&bad), Err(DirectorError::Version { found: 9 }));
    let mut zeroed = bytes.clone();
    let len = zeroed.len();
    zeroed[len - 32..].fill(0);
    assert_eq!(d.restore(&zeroed), Err(DirectorError::Corrupt));
    assert_eq!(d.now(), 0);
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_one_tick_with_twenty_phases_two_hundred_entries_and_five_hundred_alive() {
    let entries: Vec<Entry> = (0..200)
        .map(|i| Entry::new(i, 1 + i % 9).tagged(1 << (i % 5)).costing(1))
        .collect();
    let build = |rate: i32| {
        let table = Table::new(entries.clone());
        let phases = (0..20)
            .map(|i| {
                let mut p = Phase::new(0, 10_000_000, 0);
                p.rate = Ramp::linear(rate, rate + i * ONE);
                p.group = Ramp::stepped(ONE, 3 * ONE, 4);
                p.speed = Ramp::linear(ONE, 2 * ONE);
                p
            })
            .collect();
        Schedule {
            cap: 500,
            allow_overlap: true,
            tables: vec![table],
            phases,
            ..Schedule::default()
        }
    };
    for (label, rate, kills, every) in [
        ("every phase fires each tick", PER_TICK, 20u32, 1u32),
        ("at the cap, phases waiting for room", 20 * ONE, 1, 10),
    ] {
        let mut d = Director::new(build(rate), 5).unwrap();
        let points: Vec<[i32; 3]> = (0..64).map(|i| [i, i * 2, 0]).collect();
        let spots = Spots::points(&points);
        let mut out = Vec::new();
        for _ in 0..20_000 {
            d.tick(&spots, &mut out);
        }
        assert_eq!(d.load(), 500);
        let mut orders = 0usize;
        let ticks = 20_000u32;
        let start = std::time::Instant::now();
        for t in 0..ticks {
            for k in 0..if t % every == 0 { kills } else { 0 } {
                d.died((t * 13 + k * 17) % 200, 1);
            }
            d.tick(&spots, &mut out);
            orders += out.len();
        }
        let each = start.elapsed().as_secs_f64() * 1e6 / ticks as f64;
        println!(
            "{label}: {each:.2} us per tick, {:.1} orders per tick, {} alive",
            orders as f64 / ticks as f64,
            d.load()
        );
    }
}
