use super::*;

fn tuning(seed: u64) -> ShakeTuning {
    ShakeTuning {
        translation: [24.0, 16.0],
        roll: 0.05,
        frequency: 18.0,
        decay: 1.5,
        seed,
    }
}

fn track(seed: u64) -> Vec<ShakeOffset> {
    let mut shake = Shake::new(tuning(seed));
    let mut out = Vec::new();
    for frame in 0..240 {
        if frame % 50 == 0 {
            shake.add(0.6);
        }
        shake.step(1.0 / 60.0, 1.0);
        out.push(shake.offset(1.0));
    }
    out
}

#[test]
fn shake_repeats_from_its_seed_and_differs_across_seeds() {
    let a = track(7);
    assert_eq!(a, track(7));
    assert_ne!(a, track(8));
    let bits = |offsets: &[ShakeOffset]| {
        offsets
            .iter()
            .flat_map(|o| {
                [
                    o.translation[0].to_bits(),
                    o.translation[1].to_bits(),
                    o.roll.to_bits(),
                ]
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&a), bits(&track(7)));
    assert!(a.iter().any(|o| o.translation[0].abs() > 1.0));
    assert!(a.iter().all(|o| o.translation[0].abs() <= 24.0
        && o.translation[1].abs() <= 16.0
        && o.roll.abs() <= 0.05));
}

#[test]
fn shake_is_trauma_squared_and_decays_to_rest() {
    let mut shake = Shake::new(tuning(3));
    shake.add(0.5);
    shake.add(0.4);
    assert!((shake.trauma() - 0.9).abs() < 1e-6);
    shake.add(0.5);
    assert_eq!(shake.trauma(), 1.0);
    shake.step(0.25, 1.0);
    let full = shake.offset(1.0);
    let mut half = shake;
    half.trauma = shake.trauma * 0.5;
    let quarter = half.offset(1.0);
    assert!((quarter.translation[0] - full.translation[0] * 0.25).abs() < 1e-5);
    assert!((quarter.roll - full.roll * 0.25).abs() < 1e-7);
    for _ in 0..60 {
        shake.step(1.0 / 60.0, 1.0);
    }
    assert_eq!(shake.trauma(), 0.0);
    assert!(shake.offset(1.0).is_zero());
}

#[test]
fn the_noise_is_smooth_and_bounded() {
    let mut previous = smooth_noise(11, 0.0);
    assert_eq!(previous, 0.0);
    for step in 1..4000 {
        let value = smooth_noise(11, step as f64 * 0.001);
        assert!(value.abs() <= 1.0);
        assert!((value - previous).abs() < 0.01, "step {step}");
        previous = value;
    }
}

#[test]
fn intensity_quickens_the_shake_without_jumping_it() {
    let mut slow = Shake::new(tuning(5));
    let mut fast = slow;
    slow.add(1.0);
    fast.add(1.0);
    slow.step(0.1, 1.0);
    fast.step(0.1, 3.0);
    assert!((fast.phase() - 3.0 * slow.phase()).abs() < 1e-9);
    let response = Response {
        gain: Curve::new(0.5, 2.0, Ease::QuadIn),
        rate: Curve::new(1.0, 4.0, Ease::Linear),
    };
    assert_eq!(response.gain(0.0), 0.5);
    assert_eq!(response.gain(1.0), 2.0);
    assert_eq!(response.rate(0.5), 2.5);
    assert_eq!(response.gain(f32::NAN), 0.5);
    assert_eq!(Response::STEADY.gain(0.7), 1.0);
}

#[test]
fn reduced_motion_at_zero_removes_shake_blur_flashes_and_hit_stop() {
    let none = Comfort::reduced(0.0);
    assert_eq!(none.shake(5.0), 0.0);
    assert_eq!(none.blur([3.0, -4.0]), [0.0, 0.0]);
    assert_eq!(none.flash(0.8), 0.0);
    assert_eq!(none.hit_stop(HitStop::new(0.0, 0.2, 0.2)).floor, 1.0);
    assert_eq!(none.glitch(0.8), 0.4);
    let half = Comfort::reduced(0.5);
    assert_eq!(half.shake(4.0), 2.0);
    assert_eq!(half.glitch(1.0), 0.75);
    assert_eq!(Comfort::FULL.glitch(0.6), 0.6);
    assert_eq!(Comfort::reduced(7.0).amount(), 1.0);
    assert_eq!(Comfort::reduced(f32::NAN).amount(), 1.0);
}

#[test]
fn no_more_than_three_flashes_start_in_any_second() {
    let mut gate = FlashGate::new();
    let mut admitted = Vec::new();
    for step in 0..600 {
        let now = step as f64 / 60.0;
        if gate.admit(now) {
            admitted.push(now);
        }
    }
    assert_eq!(admitted.len(), 30);
    for (index, start) in admitted.iter().enumerate() {
        let within = admitted[index..]
            .iter()
            .take_while(|at| **at < start + 1.0)
            .count();
        assert!(within <= FLASHES_PER_SECOND, "{within} at {start}");
    }
    let mut gate = FlashGate::new();
    assert!(gate.admit(0.0) && gate.admit(0.1) && gate.admit(0.2));
    assert!(!gate.admit(0.9));
    assert!(gate.admit(1.0));
    assert!(!gate.admit(f64::NAN));
}
