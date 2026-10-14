use super::{Aabb, Aabb3, Hit, Overlaps, Overlaps3, Rng};

fn brute(a: &[Aabb], b: &[Aabb]) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            if x.overlaps(y) {
                hits.push(Hit {
                    a: i as u32,
                    b: j as u32,
                });
            }
        }
    }
    hits
}

fn scene(rng: &mut Rng, shots: usize, targets: usize, span: [i32; 2]) -> (Vec<Aabb>, Vec<Aabb>) {
    let snap = |rng: &mut Rng, hi: i32| rng.range_i32(-8, hi) / 4 * 4;
    let shots = (0..shots)
        .map(|_| {
            let at = [snap(rng, span[0]), snap(rng, span[1])];
            match rng.below(8) {
                0 => Aabb::new(at, at),
                1 => Aabb::point(at),
                _ => Aabb::around(at, [rng.range_i32(0, 6), rng.range_i32(0, 6)]),
            }
        })
        .collect();
    let targets = (0..targets)
        .map(|_| {
            let at = [snap(rng, span[0]), snap(rng, span[1])];
            Aabb::new(at, [at[0] + snap(rng, 160), at[1] + snap(rng, 160)])
        })
        .collect();
    (shots, targets)
}

#[test]
fn hits_match_every_pair_tested_in_order() {
    let mut rng = Rng::new(1407);
    let mut overlaps = Overlaps::new();
    for (shots, targets, span) in [
        (0, 10, [100, 100]),
        (10, 0, [100, 100]),
        (300, 60, [400, 300]),
        (300, 60, [40, 3000]),
        (2000, 400, [3840, 2160]),
        (500, 100, [8, 8]),
        (2000, 400, [1_900_000, 1_000_000]),
    ] {
        for _ in 0..4 {
            let (a, b) = scene(&mut rng, shots, targets, span);
            assert_eq!(overlaps.find(&a, &b), brute(&a, &b).as_slice());
            assert_eq!(overlaps.find(&b, &a), brute(&b, &a).as_slice());
        }
    }
}

#[test]
fn touching_and_empty_boxes_never_hit() {
    let target = [Aabb::new([0, 0], [10, 10])];
    let shots = [
        Aabb::point([10, 5]),
        Aabb::point([5, 10]),
        Aabb::new([5, 5], [5, 9]),
        Aabb::point([9, 9]),
        Aabb::point([0, 0]),
    ];
    let mut overlaps = Overlaps::new();
    assert_eq!(
        overlaps.find(&shots, &target),
        &[Hit { a: 3, b: 0 }, Hit { a: 4, b: 0 }]
    );
}

#[test]
fn a_few_huge_boxes_among_many_small_ones_match_every_pair() {
    let mut rng = Rng::new(1429);
    let mut overlaps = Overlaps::new();
    for _ in 0..4 {
        let (mut a, mut b) = scene(&mut rng, 2000, 40, [3000, 3000]);
        for _ in 0..20 {
            let at = [rng.range_i32(-100, 100), rng.range_i32(-100, 100)];
            b.push(Aabb::new(
                at,
                [at[0] + 3000, at[1] + rng.range_i32(1, 3000)],
            ));
        }
        a.push(Aabb::new([i32::MIN, i32::MIN], [i32::MAX, i32::MAX]));
        assert_eq!(overlaps.find(&a, &b), brute(&a, &b).as_slice());
        assert_eq!(overlaps.find(&b, &a), brute(&b, &a).as_slice());
    }
}

#[test]
fn boxes_in_3d_match_every_pair_tested_in_order() {
    let mut rng = Rng::new(3);
    let mut overlaps = Overlaps3::new();
    let boxes = |rng: &mut Rng, n: usize, size: i32| -> Vec<Aabb3> {
        (0..n)
            .map(|_| {
                let at = [
                    rng.range_i32(-500, 500),
                    rng.range_i32(-500, 500),
                    rng.range_i32(-50, 50),
                ];
                Aabb3::around(
                    at,
                    [
                        rng.range_i32(0, size),
                        rng.range_i32(0, size),
                        rng.range_i32(0, size),
                    ],
                )
            })
            .collect()
    };
    for (n, m, size) in [(800, 200, 30), (300, 5, 600), (0, 3, 9)] {
        let a = boxes(&mut rng, n, 8);
        let b = boxes(&mut rng, m, size);
        let mut expected = Vec::new();
        for (i, x) in a.iter().enumerate() {
            for (j, y) in b.iter().enumerate() {
                if x.overlaps(y) {
                    expected.push(Hit {
                        a: i as u32,
                        b: j as u32,
                    });
                }
            }
        }
        assert_eq!(overlaps.find(&a, &b), expected.as_slice());
    }
}

#[test]
#[ignore = "bench; run with --release"]
fn bench_shots_against_targets_on_a_wide_field() {
    let mut rng = Rng::new(9);
    let field = [1_900_000, 1_000_000];
    let targets: Vec<Aabb> = (0..400)
        .map(|_| {
            let half = rng.range_i32(22_000, 120_001);
            Aabb::around(
                [rng.range_i32(0, field[0]), rng.range_i32(0, field[1])],
                [half, half * 3 / 4],
            )
        })
        .collect();
    let shots: Vec<Aabb> = (0..2000)
        .map(|_| {
            let at = [rng.range_i32(0, field[0]), rng.range_i32(0, field[1])];
            Aabb::new(
                [at[0] - 3_500, at[1] - 7_000 - 20_000],
                [at[0] + 3_500, at[1] + 7_000],
            )
        })
        .collect();
    let mut overlaps = Overlaps::new();
    let hits = overlaps.find(&shots, &targets).len();
    let mut fastest = |a: &[Aabb], b: &[Aabb]| {
        (0..400)
            .map(|_| {
                let started = std::time::Instant::now();
                for _ in 0..5 {
                    std::hint::black_box(overlaps.find(a, b));
                }
                started.elapsed().as_secs_f64() * 1e3 / 5.0
            })
            .fold(f64::MAX, f64::min)
    };
    let wide = fastest(&shots, &targets);
    let (small, small_targets) = scene(&mut rng, 2000, 400, [3840, 2160]);
    let screen = fastest(&small, &small_targets);
    let small_hits = Overlaps::new().find(&small, &small_targets).len();
    println!("wide field: {hits} hits, {wide:.3} ms; 4K field: {small_hits} hits, {screen:.3} ms");
}
