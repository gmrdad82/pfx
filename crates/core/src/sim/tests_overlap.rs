use super::{Aabb, Hit, Overlaps, Rng};

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
