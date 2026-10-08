use super::*;
use crate::fx::Comfort;
use crate::tween::Tween;

const FRAME: f32 = 1.0 / 64.0;

fn run(clock: &mut Clock, frames: u32) -> Vec<Tick> {
    (0..frames).map(|_| clock.advance(FRAME)).collect()
}

#[test]
fn a_steady_scale_slows_easings_steps_and_lifetimes_exactly() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.set_speed(0.75);
    let mut tween = Tween::new(0.0f32, 1.0, 1.0);
    let mut physics = Steps::new(120.0, &clock);
    let mut steps = 0;
    let mut life = 0.0f64;
    for tick in run(&mut clock, 64) {
        assert_eq!(tick.dt, 0.75 * FRAME);
        assert_eq!(tick.paced, 0.75 * FRAME);
        tween.step(tick.dt);
        steps += physics.due(&clock);
        life += f64::from(tick.dt);
    }
    assert_eq!(clock.time(), 0.75);
    assert_eq!(tween.time(), 0.75);
    assert_eq!(tween.value(), 0.75);
    assert_eq!(steps, 90);
    assert_eq!(life, 0.75);
    assert_eq!(clock.ticks(), 45);
    let mut full = Clock::new(60.0).unwrap();
    let mut steps = Steps::new(120.0, &full);
    let mut total = 0;
    for _ in 0..64 {
        full.advance(FRAME);
        total += steps.due(&full);
    }
    assert_eq!((full.time(), full.ticks(), total), (1.0, 60, 120));
}

#[test]
fn a_hit_stop_holds_its_floor_then_eases_back() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.hit_stop(HitStop::new(0.0, 0.25, 0.5).with_ease(Ease::QuadOut));
    let held: f64 = run(&mut clock, 16).iter().map(|t| f64::from(t.dt)).sum();
    assert_eq!(held, 0.0);
    assert_eq!(clock.ticks(), 0);
    assert_eq!(clock.scale(), 0.0);
    let back: f64 = run(&mut clock, 32).iter().map(|t| f64::from(t.dt)).sum();
    assert!((back - 0.5 * 2.0 / 3.0).abs() < 1e-6, "{back}");
    assert!(clock.dip().is_none());
    assert_eq!(clock.scale(), 1.0);
    let after = clock.advance(FRAME);
    assert_eq!(after.dt, FRAME);
}

#[test]
fn reduced_motion_scales_the_dip_and_removes_it_at_zero() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.hit_stop(Comfort::reduced(0.5).hit_stop(HitStop::new(0.2, 0.25, 0.25)));
    assert!((clock.dip().unwrap().floor - 0.6).abs() < 1e-6);
    let mut none = Clock::new(60.0).unwrap();
    none.hit_stop(Comfort::reduced(0.0).hit_stop(HitStop::new(0.0, 0.25, 0.25)));
    assert!(none.dip().is_none());
    assert_eq!(none.advance(FRAME).dt, FRAME);
}

#[test]
fn a_pause_runs_no_ticks_and_freezes_the_dip() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.hit_stop(HitStop::new(0.0, 0.1, 0.1));
    clock.pause();
    for tick in run(&mut clock, 100) {
        assert_eq!((tick.ticks, tick.dt, tick.paced), (0, 0.0, 0.0));
    }
    assert_eq!(clock.dip().unwrap().elapsed, 0.0);
    clock.resume();
    clock.advance(FRAME);
    assert!(clock.dip().unwrap().elapsed > 0.0);
}

fn input(tick: u64) -> u64 {
    crate::sim::mix64(tick ^ 0x9e37_79b9)
}

fn audit(speed: f32, stops: &[(u32, HitStop)], ticks: u64) -> (Vec<(u64, u64)>, u64, u32) {
    let mut clock = Clock::new(60.0).unwrap();
    clock.set_speed(speed);
    let mut seen = Vec::new();
    let mut state = 0u64;
    let mut frames = 0;
    while clock.ticks() < ticks {
        if let Some((_, stop)) = stops.iter().find(|(at, _)| *at == frames) {
            clock.hit_stop(*stop);
        }
        let before = clock.ticks();
        let tick = clock.advance(1.0 / 60.0);
        for index in before..before + u64::from(tick.ticks) {
            if index >= ticks {
                break;
            }
            let given = input(index);
            seen.push((index, given));
            state = crate::sim::mix64(state ^ given);
        }
        frames += 1;
    }
    (seen, state, frames)
}

#[test]
fn speed_and_hit_stops_change_pacing_never_the_ticks_or_their_inputs() {
    let stop = HitStop::new(0.0, 0.2, 0.3);
    let stops = [(30, stop), (95, HitStop::new(0.1, 0.05, 0.6)), (200, stop)];
    let (full, full_state, full_frames) = audit(1.0, &[], 1200);
    let (slow, slow_state, slow_frames) = audit(0.75, &stops, 1200);
    assert_eq!(full.len(), 1200);
    assert_eq!(full, slow);
    assert_eq!(full_state, slow_state);
    assert!(
        slow_frames > full_frames * 4 / 3,
        "{slow_frames} {full_frames}"
    );
}

#[test]
fn a_saved_clock_resumes_at_the_same_tick_mid_dip() {
    let mut clock = Clock::new(30.0).unwrap();
    clock.set_speed(0.75);
    run(&mut clock, 40);
    clock.hit_stop(HitStop::new(0.25, 0.1, 0.4).with_ease(Ease::SineOut));
    run(&mut clock, 13);
    let bytes = clock.save();
    let mut restored = Clock::restore(&bytes).unwrap();
    assert_eq!(restored, clock);
    assert_eq!(run(&mut restored, 80), run(&mut clock, 80));
    assert_eq!(restored.save(), clock.save());
}

#[test]
fn the_saved_bytes_are_pinned() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.set_speed(0.75);
    run(&mut clock, 64);
    clock.hit_stop(HitStop::new(0.0, 0.25, 0.5));
    run(&mut clock, 4);
    let bytes = clock.save();
    let mut expected = Vec::new();
    expected.extend(1u32.to_le_bytes());
    expected.extend(60.0f64.to_le_bytes());
    expected.extend(0.75f32.to_le_bytes());
    expected.extend([0, 1, Ease::QuadOut as u8, 0]);
    expected.extend(0.75f64.to_le_bytes());
    expected.extend(45u64.to_le_bytes());
    expected.extend(0.0f32.to_le_bytes());
    expected.extend(0.25f32.to_le_bytes());
    expected.extend(0.5f32.to_le_bytes());
    expected.extend(0.0625f64.to_le_bytes());
    assert_eq!(bytes.to_vec(), expected);
    assert_eq!(Ease::QuadOut as u8, 5);
}

#[test]
fn a_bad_save_is_refused() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.hit_stop(HitStop::new(0.0, 0.25, 0.5));
    clock.advance(FRAME);
    let good = clock.save();
    assert_eq!(
        Clock::restore(&good[..55]),
        Err(ClockError::Length { found: 55 })
    );
    let edit = |at: usize, bytes: &[u8]| {
        let mut copy = good;
        copy[at..at + bytes.len()].copy_from_slice(bytes);
        Clock::restore(&copy)
    };
    assert_eq!(
        edit(0, &2u32.to_le_bytes()),
        Err(ClockError::Version { found: 2 })
    );
    for (at, bytes) in [
        (4, f64::NAN.to_le_bytes().to_vec()),
        (4, 0.0f64.to_le_bytes().to_vec()),
        (12, 9.0f32.to_le_bytes().to_vec()),
        (16, vec![2]),
        (17, vec![7]),
        (18, vec![31]),
        (19, vec![1]),
        (20, (-1.0f64).to_le_bytes().to_vec()),
        (20, f64::INFINITY.to_le_bytes().to_vec()),
        (36, 1.0f32.to_le_bytes().to_vec()),
        (40, (-0.5f32).to_le_bytes().to_vec()),
        (48, 9.0f64.to_le_bytes().to_vec()),
    ] {
        assert!(
            matches!(edit(at, &bytes), Err(ClockError::Invalid(_))),
            "byte {at}"
        );
    }
    assert!(Clock::new(0.0).is_err());
    assert!(Clock::new(f64::NAN).is_err());
}

#[test]
fn the_sound_rate_follows_the_scale_as_far_as_the_game_ties_it() {
    let mut clock = Clock::new(60.0).unwrap();
    clock.set_speed(0.75);
    assert_eq!(clock.sound_rate(0.0), 1.0);
    assert_eq!(clock.sound_rate(1.0), 0.75);
    assert_eq!(clock.sound_rate(0.5), 0.875);
    clock.pause();
    assert_eq!(clock.sound_rate(1.0), 1.0 / 16.0);
}

#[test]
fn steps_rebuilt_on_resume_start_from_the_clock() {
    let mut clock = Clock::new(60.0).unwrap();
    run(&mut clock, 640);
    let mut steps = Steps::new(60.0, &clock);
    assert_eq!(steps.done(), 600);
    assert_eq!(steps.due(&clock), 0);
    run(&mut clock, 64);
    assert_eq!(steps.due(&clock), 60);
}
