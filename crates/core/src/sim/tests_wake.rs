use super::particles::{IDENTITY, Matrix, STEP};
use super::wake::{Recipe, STRIDE, Wake};

fn at(z: f32) -> Matrix {
    let mut m = IDENTITY;
    m[3][2] = z;
    m
}

fn driven(seconds: f32, parked: f32) -> Wake {
    let mut wake = Wake::new(Recipe::default());
    let stop = 20.0 * seconds;
    let place = |t: f32| at((20.0 * t).min(stop));
    let steps = ((seconds + parked) / STEP).round() as u32;
    for n in 0..steps {
        let t = n as f32 * STEP;
        wake.step(STEP, place(t), place(t + STEP));
    }
    wake
}

fn speed(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

#[test]
fn the_wake_follows_behind_and_fades_with_distance() {
    let wake = driven(1.0, 0.0);
    let behind = wake.velocity([0.0, 0.0, 18.0]);
    assert!(behind[2] > 3.0, "{behind:?}");
    assert!(behind[1] > 0.0, "{behind:?}");
    let aside = wake.velocity([1.5, 0.0, 18.0]);
    assert!(aside[0] > 0.0, "{aside:?}");
    assert!(speed(aside) < speed(behind));
    let far = wake.velocity([6.0, 0.0, 18.0]);
    assert!(speed(far) < 0.05 * speed(behind), "{far:?}");
    assert_eq!(wake.velocity([40.0, 0.0, 18.0]), [0.0; 3]);
    assert_eq!(wake.velocity([0.0, 0.0, 60.0]), [0.0; 3]);
}

#[test]
fn the_wake_decays_with_time_and_is_gone_after_its_length() {
    let probe = [0.0, 0.0, 10.0];
    let fresh = speed(driven(1.0, 0.0).velocity(probe));
    let later = speed(driven(1.0, 0.5).velocity(probe));
    let old = speed(driven(1.0, 1.5).velocity(probe));
    assert!(fresh > 0.0);
    assert!(later < fresh, "{later} {fresh}");
    assert!(old < later * (-1.0f32 / 0.8).exp(), "{old} {later}");
    let gone = driven(1.0, Recipe::default().length + 0.1);
    assert_eq!(gone.samples(), 0);
    assert_eq!(gone.velocity(probe), [0.0; 3]);
}

#[test]
fn the_same_poses_give_the_same_field_on_a_fixed_grid() {
    let a = driven(1.2, 0.3);
    let b = driven(1.2, 0.3);
    assert_eq!(a.samples(), b.samples());
    assert_eq!(a.samples() as u64, ((1.2 / STEP).round() as u64) / STRIDE);
    for x in [0.0f32, 0.5, 1.0, 2.0] {
        for z in [5.0f32, 12.0, 23.5] {
            assert_eq!(a.velocity([x, 0.2, z]), b.velocity([x, 0.2, z]));
        }
    }
}

#[test]
fn a_standing_object_and_a_bad_step_leave_no_wake() {
    let mut wake = Wake::new(Recipe::default());
    for _ in 0..240 {
        wake.step(STEP, at(5.0), at(5.0));
    }
    assert_eq!(wake.samples(), 0);
    assert_eq!(wake.velocity([0.0, 0.0, 5.0]), [0.0; 3]);
    let before = wake.time();
    wake.step(0.0, at(0.0), at(1.0));
    wake.step(f32::NAN, at(0.0), at(1.0));
    wake.step(-STEP, at(0.0), at(1.0));
    assert_eq!(wake.time(), before);
    assert_eq!(wake.samples(), 0);
}

#[test]
fn wakes_add_and_reset_clears_one() {
    use super::particles::Field;
    use super::wake::Wakes;
    let a = driven(1.0, 0.0);
    let b = driven(1.0, 0.0);
    let probe = [0.3, 0.0, 12.0];
    let one = a.velocity(probe);
    let both = Wakes(&[a.clone(), b]).velocity(probe);
    for i in 0..3 {
        assert!((both[i] - 2.0 * one[i]).abs() < 1e-5);
    }
    assert_eq!(Wakes(&[]).velocity(probe), [0.0; 3]);
    let mut cleared = a;
    cleared.reset();
    assert_eq!(cleared.samples(), 0);
    assert_eq!(cleared.time(), 0.0);
    assert_eq!(cleared.velocity(probe), [0.0; 3]);
}

#[test]
fn the_wake_point_rides_the_object_frame() {
    let mut wake = Wake::new(Recipe {
        at: [2.0, 0.0, 0.0],
        ..Recipe::default()
    });
    for n in 0..120 {
        let t = n as f32 * STEP;
        wake.step(STEP, at(20.0 * t), at(20.0 * (t + STEP)));
    }
    let offset = wake.velocity([2.0, 0.0, 15.0]);
    let centre = wake.velocity([0.0, 0.0, 15.0]);
    assert!(offset[2] > centre[2], "{offset:?} {centre:?}");
}
