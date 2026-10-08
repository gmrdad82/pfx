use super::*;
use std::collections::BTreeMap;

const GOLDEN: &str = include_str!("golden/rng.txt");

fn sections() -> BTreeMap<String, Vec<String>> {
    GOLDEN
        .lines()
        .map(|line| {
            let (head, values) = line.split_once(" : ").unwrap();
            (
                head.to_string(),
                values.split(' ').map(str::to_string).collect(),
            )
        })
        .collect()
}

fn hex(h: &str) -> u64 {
    u64::from_str_radix(h, 16).unwrap()
}

fn section(name: &str) -> Vec<String> {
    sections()
        .remove(name)
        .unwrap_or_else(|| panic!("no section {name}"))
}

fn draws(rng: &mut Rng, n: usize, mut f: impl FnMut(&mut Rng) -> String) -> Vec<String> {
    (0..n).map(|_| f(rng)).collect()
}

#[test]
fn splitmix_mixer_matches_the_published_first_output() {
    assert_eq!(mix64(0), 0xe220_a839_7b1d_cdaf);
}

#[test]
fn xoshiro_matches_the_published_vector_for_state_1_2_3_4() {
    let mut rng = Rng::from_state(RngState {
        root: 0,
        words: [1, 2, 3, 4],
    });
    let out: Vec<u64> = (0..4).map(|_| rng.next_u64()).collect();
    assert_eq!(out, [11520, 0, 1509978240, 1215971899390074240]);
}

#[test]
fn the_first_thousand_draws_of_three_seeds_are_golden() {
    for seed in [0u64, 42, 0xdead_beef_cafe_f00d] {
        let want = section(&format!("u64 {seed}"));
        assert_eq!(want.len(), 1000);
        let mut rng = Rng::new(seed);
        for (i, w) in want.iter().enumerate() {
            assert_eq!(rng.next_u64(), hex(w), "seed {seed} draw {i}");
        }
    }
}

#[test]
fn u32_draws_take_the_high_half() {
    let mut rng = Rng::new(42);
    for w in section("u32 42") {
        assert_eq!(rng.next_u32() as u64, hex(&w));
    }
}

#[test]
fn floats_are_golden_and_in_the_unit_interval() {
    let mut rng = Rng::new(7);
    for w in section("f64 7") {
        let v = rng.next_f64();
        assert!((0.0..1.0).contains(&v));
        assert_eq!(v.to_bits(), hex(&w));
    }
    let mut rng = Rng::new(7);
    for w in section("f32 7") {
        let v = rng.next_f32();
        assert!((0.0..1.0).contains(&v));
        assert_eq!(v.to_bits() as u64, hex(&w));
    }
}

#[test]
fn float_extremes_stay_below_one() {
    let top = (u64::MAX >> 11) as f64 * (1.0 / 9007199254740992.0);
    assert!(top < 1.0);
    let top = (u64::MAX >> 40) as f32 * (1.0 / 16777216.0);
    assert!(top < 1.0);
}

#[test]
fn bounded_integers_are_golden() {
    for n in [6u64, 1000, 10_000_000_000_000_000_000, 3 << 62] {
        let want = section(&format!("below 7 {n}"));
        let mut rng = Rng::new(7);
        let got = draws(&mut rng, 100, |r| r.below(n).to_string());
        assert_eq!(got, want, "n {n}");
    }
    let mut rng = Rng::new(7);
    let got = draws(&mut rng, 100, |r| r.range_i64(-5, 5).to_string());
    assert_eq!(got, section("range_i64 7 -5 5"));
}

#[test]
fn bounded_integers_have_no_modulo_bias() {
    let mut rng = Rng::new(1234);
    let n = 6u64;
    let mut counts = [0u32; 6];
    for _ in 0..600_000 {
        counts[rng.below(n) as usize] += 1;
    }
    for c in counts {
        assert!((99_000..101_000).contains(&c), "{counts:?}");
    }
}

#[test]
fn bounded_integers_respect_their_ranges() {
    let mut rng = Rng::new(3);
    for _ in 0..10_000 {
        assert!(rng.below(1) == 0);
        assert!(rng.below_u32(10) < 10);
        assert!(rng.index(7) < 7);
        let v = rng.range_i64(i64::MIN, i64::MAX);
        assert!(v < i64::MAX);
        let v = rng.range_i32(-3, 4);
        assert!((-3..4).contains(&v));
        let v = rng.range_u64(10, 20);
        assert!((10..20).contains(&v));
        let v = rng.range_f64(-2.0, 3.0);
        assert!((-2.0..3.0).contains(&v));
        let v = rng.range_f32(-2.0, 3.0);
        assert!((-2.0..3.0).contains(&v));
    }
}

#[test]
#[should_panic]
fn below_zero_panics() {
    Rng::new(1).below(0);
}

#[test]
fn shuffles_are_golden_permutations() {
    for (seed, n) in [(7u64, 20usize), (8, 52)] {
        let mut items: Vec<usize> = (0..n).collect();
        Rng::new(seed).shuffle(&mut items);
        let got: Vec<String> = items.iter().map(|i| i.to_string()).collect();
        assert_eq!(got, section(&format!("shuffle {seed} {n}")));
        let mut sorted = items.clone();
        sorted.sort();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>());
    }
    let mut empty: [u8; 0] = [];
    Rng::new(1).shuffle(&mut empty);
    let mut one = [9];
    Rng::new(1).shuffle(&mut one);
    assert_eq!(one, [9]);
}

#[test]
fn shuffle_reaches_every_arrangement_of_three() {
    let mut rng = Rng::new(11);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..600 {
        let mut v = [0, 1, 2];
        rng.shuffle(&mut v);
        seen.insert(v);
    }
    assert_eq!(seen.len(), 6);
}

#[test]
fn weighted_choice_is_golden() {
    let mut rng = Rng::new(7);
    let got = draws(&mut rng, 100, |r| {
        r.weighted_index_u32(&[1, 2, 3, 0, 10]).unwrap().to_string()
    });
    assert_eq!(got, section("weighted_u32 7 1,2,3,0,10"));
    let mut rng = Rng::new(7);
    let got = draws(&mut rng, 100, |r| {
        r.weighted_index(&[0.5, 0.0, 2.5, 1.0]).unwrap().to_string()
    });
    assert_eq!(got, section("weighted_f64 7 0.5,0,2.5,1"));
}

#[test]
fn weighted_choice_follows_the_weights_and_skips_zeros() {
    let mut rng = Rng::new(21);
    let mut counts = [0u32; 4];
    for _ in 0..100_000 {
        counts[rng.weighted_index_u32(&[1, 0, 3, 6]).unwrap()] += 1;
    }
    assert_eq!(counts[1], 0);
    assert!((9_500..10_500).contains(&counts[0]), "{counts:?}");
    assert!((29_000..31_000).contains(&counts[2]), "{counts:?}");
    assert!((59_000..61_000).contains(&counts[3]), "{counts:?}");
    let mut counts = [0u32; 4];
    for _ in 0..100_000 {
        counts[rng.weighted_index(&[1.0, 0.0, 3.0, 6.0]).unwrap()] += 1;
    }
    assert_eq!(counts[1], 0);
    assert!((9_500..10_500).contains(&counts[0]), "{counts:?}");
    assert!((59_000..61_000).contains(&counts[3]), "{counts:?}");
}

#[test]
fn weighted_choice_refuses_bad_weights() {
    let mut rng = Rng::new(1);
    assert_eq!(rng.weighted_index(&[]), None);
    assert_eq!(rng.weighted_index(&[0.0, 0.0]), None);
    assert_eq!(rng.weighted_index(&[1.0, -1.0]), None);
    assert_eq!(rng.weighted_index(&[1.0, f64::NAN]), None);
    assert_eq!(rng.weighted_index(&[1.0, f64::INFINITY]), None);
    assert_eq!(rng.weighted_index(&[f64::MAX, f64::MAX]), None);
    assert_eq!(rng.weighted_index_u32(&[]), None);
    assert_eq!(rng.weighted_index_u32(&[0, 0]), None);
    assert_eq!(rng.weighted_index(&[0.0, 2.0, 0.0]), Some(1));
    assert_eq!(rng.weighted_index_u32(&[0, 2, 0]), Some(1));
    assert_eq!(
        rng.weighted_index_u32(&[u32::MAX, u32::MAX]).map(|i| i < 2),
        Some(true)
    );
}

#[test]
fn choose_picks_from_the_slice() {
    let mut rng = Rng::new(1);
    let items = ["a", "b", "c"];
    for _ in 0..100 {
        assert!(items.contains(rng.choose(&items).unwrap()));
    }
    assert_eq!(rng.choose::<u8>(&[]), None);
}

#[test]
fn chance_and_bool_track_their_probability() {
    let mut rng = Rng::new(8);
    let hits = (0..100_000).filter(|_| rng.chance(0.25)).count();
    assert!((24_000..26_000).contains(&hits), "{hits}");
    let heads = (0..100_000).filter(|_| rng.next_bool()).count();
    assert!((49_000..51_000).contains(&heads), "{heads}");
    assert!(!rng.chance(0.0));
    assert!(rng.chance(1.0));
}

#[test]
fn forks_are_golden() {
    for label in [1u64, 2] {
        let mut child = Rng::new(7).fork(label);
        for w in section(&format!("fork 7 {label}")) {
            assert_eq!(child.next_u64(), hex(&w));
        }
    }
    let mut grandchild = Rng::new(7).fork(1).fork(2);
    for w in section("fork2 7 1 2") {
        assert_eq!(grandchild.next_u64(), hex(&w));
    }
}

#[test]
fn a_fork_ignores_how_much_its_parent_has_drawn() {
    let fresh = Rng::new(99);
    let mut used = Rng::new(99);
    for _ in 0..12345 {
        used.next_u64();
    }
    let mut a = fresh.fork(5);
    let mut b = used.fork(5);
    for _ in 0..100 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
}

#[test]
fn a_fork_ignores_what_its_siblings_draw() {
    let parent = Rng::new(2024);
    let mut lane_a = parent.fork(1);
    let mut lane_b = parent.fork(2);
    let first_a: Vec<u64> = (0..50).map(|_| lane_a.next_u64()).collect();
    let mut lane_b_busy = parent.fork(2);
    for _ in 0..777 {
        lane_b.next_u64();
    }
    let mut lane_a_again = parent.fork(1);
    let again: Vec<u64> = (0..50).map(|_| lane_a_again.next_u64()).collect();
    assert_eq!(first_a, again);
    let expected_b = parent.fork(2).next_u64();
    assert_eq!(lane_b_busy.next_u64(), expected_b);
}

#[test]
fn forks_differ_from_each_other_and_from_the_parent() {
    let parent = Rng::new(1);
    let mut seen = std::collections::BTreeSet::new();
    seen.insert(parent.clone().next_u64());
    for label in 0..1000 {
        assert!(seen.insert(parent.fork(label).next_u64()), "label {label}");
    }
    assert_ne!(parent.fork(0).state(), parent.state());
    assert_ne!(Rng::new(1).fork(2).state(), Rng::new(2).fork(1).state());
}

#[test]
fn state_round_trips_and_resumes_the_stream() {
    let mut rng = Rng::new(31);
    for _ in 0..10 {
        rng.next_u64();
    }
    let saved = rng.state();
    let mut resumed = Rng::from_state(saved);
    assert_eq!(resumed, rng);
    for _ in 0..100 {
        assert_eq!(resumed.next_u64(), rng.next_u64());
    }
    assert_eq!(resumed.fork(3), rng.fork(3));
}

#[test]
fn an_all_zero_state_falls_back_to_seeding() {
    let rng = Rng::from_state(RngState {
        root: 5,
        words: [0; 4],
    });
    assert_eq!(rng, Rng::new(5));
}

#[test]
fn equal_seeds_replay_equal_runs() {
    let run = |seed| {
        let mut rng = Rng::new(seed);
        let mut v: Vec<u32> = (0..30).collect();
        rng.shuffle(&mut v);
        (v, rng.next_f64().to_bits(), rng.below(1000))
    };
    assert_eq!(run(77), run(77));
    assert_ne!(run(77), run(78));
}

fn unhex(h: &str) -> Vec<u8> {
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect()
}

fn saved(seed: u64, draws: usize) -> Rng {
    let mut rng = Rng::new(seed);
    for _ in 0..draws {
        rng.next_u64();
    }
    rng
}

#[test]
fn the_stream_version_is_one() {
    assert_eq!(STREAM_VERSION, 1);
    assert_eq!(STATE_BYTES, 44);
}

#[test]
fn saved_states_are_golden_bytes() {
    for seed in [0u64, 42, 0xdead_beef_cafe_f00d] {
        for draws in [0usize, 1, 1000] {
            let want = section(&format!("save {seed} {draws}"));
            assert_eq!(want.len(), 1);
            let bytes = saved(seed, draws).save();
            assert_eq!(bytes.to_vec(), unhex(&want[0]), "seed {seed} draws {draws}");
        }
    }
}

#[test]
fn saved_forks_are_golden_bytes() {
    let mut child = Rng::new(7).fork(1);
    assert_eq!(child.save().to_vec(), unhex(&section("save_fork 7 1 0")[0]));
    for _ in 0..100 {
        child.next_u64();
    }
    assert_eq!(
        child.save().to_vec(),
        unhex(&section("save_fork 7 1 100")[0])
    );
}

#[test]
fn the_save_layout_is_version_root_then_four_words_little_endian() {
    let state = RngState {
        root: 0x0102_0304_0506_0708,
        words: [1, 2, 3, 0xff00_0000_0000_0000],
    };
    let bytes = state.to_bytes();
    assert_eq!(&bytes[..4], [1, 0, 0, 0]);
    assert_eq!(&bytes[4..12], [8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(&bytes[12..20], [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[20..28], [2, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[28..36], [3, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[36..44], [0, 0, 0, 0, 0, 0, 0, 0xff]);
    assert_eq!(RngState::from_bytes(&bytes), Ok(state));
}

#[test]
fn restoring_then_continuing_matches_an_uninterrupted_stream() {
    for seed in [0u64, 42, 0xdead_beef_cafe_f00d] {
        for cut in [0usize, 1, 7, 1000] {
            let mut straight = Rng::new(seed);
            let mut first = Rng::new(seed);
            for _ in 0..cut {
                first.next_u64();
            }
            let mut resumed = Rng::restore(&first.save()).unwrap();
            assert_eq!(resumed, first);
            for _ in 0..cut {
                straight.next_u64();
            }
            for i in 0..500 {
                assert_eq!(
                    resumed.next_u64(),
                    straight.next_u64(),
                    "seed {seed} cut {cut} draw {i}"
                );
            }
            assert_eq!(resumed.save(), straight.save());
        }
    }
}

#[test]
fn restoring_continues_every_derived_draw() {
    let mut straight = Rng::new(9);
    let mut first = Rng::new(9);
    for _ in 0..33 {
        straight.next_u64();
        first.next_u64();
    }
    let mut resumed = Rng::restore(&first.save()).unwrap();
    let mut a: Vec<u32> = (0..40).collect();
    let mut b = a.clone();
    straight.shuffle(&mut a);
    resumed.shuffle(&mut b);
    assert_eq!(a, b);
    assert_eq!(straight.below(1000), resumed.below(1000));
    assert_eq!(straight.range_i64(-9, 9), resumed.range_i64(-9, 9));
    assert_eq!(straight.next_f64().to_bits(), resumed.next_f64().to_bits());
    assert_eq!(straight.next_f32().to_bits(), resumed.next_f32().to_bits());
    assert_eq!(
        straight.weighted_index_u32(&[1, 2, 3]),
        resumed.weighted_index_u32(&[1, 2, 3])
    );
    assert_eq!(
        straight.weighted_index(&[0.5, 2.5]),
        resumed.weighted_index(&[0.5, 2.5])
    );
}

#[test]
fn restored_forks_resume_and_fork_like_the_original() {
    let parent = saved(2024, 50);
    let mut child = parent.fork(4);
    for _ in 0..25 {
        child.next_u64();
    }
    let mut restored = Rng::restore(&child.save()).unwrap();
    assert_eq!(restored, child);
    for _ in 0..200 {
        assert_eq!(restored.next_u64(), child.next_u64());
    }
    assert_eq!(restored.fork(8), child.fork(8));
    assert_eq!(restored.fork(8).fork(1), child.fork(8).fork(1));
    let restored_parent = Rng::restore(&parent.save()).unwrap();
    assert_eq!(restored_parent.fork(4), parent.fork(4));
    assert_eq!(
        restored_parent.fork(4).save(),
        Rng::new(2024).fork(4).save()
    );
}

#[test]
fn a_fresh_from_root_state_is_encoded_explicitly() {
    let fresh = RngState {
        root: 5,
        words: [0; 4],
    };
    let bytes = fresh.to_bytes();
    assert_eq!(bytes, Rng::new(5).save());
    let back = RngState::from_bytes(&bytes).unwrap();
    assert_ne!(back.words, [0; 4]);
    assert_eq!(back, Rng::new(5).state());
    assert_eq!(Rng::restore(&bytes).unwrap(), Rng::new(5));
}

#[test]
fn saving_is_stable_through_a_second_round_trip() {
    let rng = saved(77, 13);
    let once = rng.save();
    let twice = Rng::restore(&once).unwrap().save();
    assert_eq!(once, twice);
}

#[test]
fn a_save_of_the_wrong_length_is_refused() {
    let good = Rng::new(1).save();
    assert_eq!(
        RngState::from_bytes(&[]),
        Err(SimError::Length { found: 0 })
    );
    assert_eq!(
        RngState::from_bytes(&good[..43]),
        Err(SimError::Length { found: 43 })
    );
    let mut long = good.to_vec();
    long.push(0);
    assert_eq!(
        RngState::from_bytes(&long),
        Err(SimError::Length { found: 45 })
    );
    assert_eq!(Rng::restore(&good[..8]), Err(SimError::Length { found: 8 }));
}

#[test]
fn a_save_of_another_version_is_refused() {
    let good = Rng::new(1).save();
    for version in [0u32, 2, 0x0100_0000, u32::MAX] {
        let mut bytes = good;
        bytes[..4].copy_from_slice(&version.to_le_bytes());
        assert_eq!(
            RngState::from_bytes(&bytes),
            Err(SimError::Version { found: version })
        );
        assert_eq!(
            Rng::restore(&bytes),
            Err(SimError::Version { found: version })
        );
    }
}

#[test]
fn an_all_zero_saved_state_is_refused() {
    let mut bytes = Rng::new(1).save();
    bytes[12..].fill(0);
    assert_eq!(RngState::from_bytes(&bytes), Err(SimError::ZeroState));
    assert_eq!(Rng::restore(&bytes), Err(SimError::ZeroState));
    bytes[4..12].fill(0);
    assert_eq!(RngState::from_bytes(&bytes), Err(SimError::ZeroState));
}

#[test]
fn one_set_bit_in_the_words_is_accepted() {
    let mut bytes = Rng::new(1).save();
    bytes[12..].fill(0);
    bytes[43] = 0x80;
    let state = RngState::from_bytes(&bytes).unwrap();
    assert_eq!(state.words, [0, 0, 0, 1 << 63]);
}

#[test]
fn refusals_read_in_plain_words() {
    assert_eq!(
        SimError::Length { found: 3 }.to_string(),
        "saved rng state is 3 bytes, expected 44"
    );
    assert_eq!(
        SimError::Version { found: 2 }.to_string(),
        "saved rng state is stream version 2, this build reads 1"
    );
    assert_eq!(
        SimError::ZeroState.to_string(),
        "saved rng state has no bits set in its four words"
    );
}
