mod common;

use common::*;
use pfx_input::{
    Button, Input, Key, MotorCommand, Motors, PadId, PadInfo, Rumble, Rumbler, Script,
};

const STEP: u32 = 10_000;

#[test]
fn strength_scales_with_the_intensity_setting() {
    for (percent, expected) in [(100u8, 65535u16), (50, 32768), (25, 16384), (0, 0)] {
        let rumbler = Rumbler::new(STEP, percent);
        assert_eq!(rumbler.scale(1.0), expected, "{percent}%");
    }
    let rumbler = Rumbler::new(STEP, 80);
    assert_eq!(rumbler.scale(0.5), 26214);
    assert_eq!(rumbler.scale(2.0), rumbler.scale(1.0));
    assert_eq!(rumbler.scale(f32::NAN), 0);
    assert_eq!(Rumbler::new(STEP, 200).intensity(), 100);
}

#[test]
fn an_event_runs_for_its_duration_then_stops() {
    let mut rumbler = Rumbler::new(STEP, 50);
    assert!(rumbler.play(PadId(0), Motors::Two, Rumble::motors(1.0, 0.5, 30)));
    let first = rumbler.step();
    assert_eq!(
        first,
        vec![MotorCommand {
            pad: PadId(0),
            low: 32768,
            high: 16384,
            hold_ms: 40,
        }]
    );
    assert!(rumbler.step().is_empty());
    assert!(rumbler.step().is_empty());
    let stop = rumbler.step();
    assert_eq!(stop.len(), 1);
    assert_eq!((stop[0].low, stop[0].high), (0, 0));
    assert!(rumbler.step().is_empty());
}

#[test]
fn overlapping_events_take_the_stronger_motor() {
    let mut rumbler = Rumbler::new(STEP, 100);
    rumbler.play(PadId(1), Motors::Two, Rumble::motors(0.25, 0.0, 50));
    rumbler.play(PadId(1), Motors::Two, Rumble::motors(0.0, 1.0, 20));
    let command = rumbler.step()[0];
    assert_eq!((command.low, command.high), (16384, 65535));
    rumbler.step();
    let command = rumbler.step()[0];
    assert_eq!((command.low, command.high), (16384, 0));
}

#[test]
fn a_single_motor_gets_the_stronger_of_both() {
    let mut rumbler = Rumbler::new(STEP, 100);
    rumbler.play(PadId(2), Motors::One, Rumble::motors(0.2, 0.6, 10));
    let command = rumbler.step()[0];
    assert_eq!(command.low, command.high);
    assert_eq!(command.low, rumbler.scale(0.6));
}

#[test]
fn zero_intensity_or_no_motors_plays_nothing() {
    let mut rumbler = Rumbler::new(STEP, 0);
    assert!(!rumbler.play(PadId(0), Motors::Two, Rumble::new(1.0, 100)));
    assert!(rumbler.step().is_empty());
    let mut rumbler = Rumbler::new(STEP, 100);
    assert!(!rumbler.play(PadId(0), Motors::None, Rumble::new(1.0, 100)));
    assert!(rumbler.play(PadId(0), Motors::Two, Rumble::new(1.0, 100)));
    assert_eq!(rumbler.step()[0].low, 65535);
    rumbler.set_intensity(0);
    let stop = rumbler.step();
    assert_eq!((stop[0].low, stop[0].high), (0, 0));
}

#[test]
fn attack_and_release_shape_the_level() {
    let rumble = Rumble::new(1.0, 100).attack(20).release(40);
    assert_eq!(rumble.level_at(0), [0.0, 0.0]);
    assert_eq!(rumble.level_at(10_000), [0.5, 0.5]);
    assert_eq!(rumble.level_at(40_000), [1.0, 1.0]);
    assert_eq!(rumble.level_at(80_000), [0.5, 0.5]);
    assert_eq!(rumble.level_at(100_000), [0.0, 0.0]);
}

#[test]
fn input_rumbles_only_the_pad_in_use() {
    let mut input = Input::new(actions(), STEP);
    let mut backend = Script::new();
    input.set_rumble_intensity(40);
    backend.push(vec![
        connect(0, dualsense()),
        connect(1, PadInfo::from_ids("pad", None, None, Motors::None)),
    ]);
    input.update(&mut backend);
    assert!(!input.rumble(Rumble::new(1.0, 50)));
    backend.push(vec![button(0, Button::South, 1.0)]);
    input.update(&mut backend);
    assert!(input.rumble(Rumble::new(1.0, 20)));
    assert!(!input.rumble_pad(PadId(1), Rumble::new(1.0, 20)));
    input.update(&mut backend);
    input.update(&mut backend);
    input.update(&mut backend);
    assert_eq!(backend.motors.len(), 2);
    assert_eq!(backend.motors[0].pad, PadId(0));
    assert_eq!(backend.motors[0].low, 26214);
    assert_eq!((backend.motors[1].low, backend.motors[1].high), (0, 0));
    backend.push(vec![key(Key::Space, true)]);
    input.update(&mut backend);
    assert!(!input.rumble(Rumble::new(1.0, 20)));
}
