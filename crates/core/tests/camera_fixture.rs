mod fixture;

use pfx_core::camera::{Aim, DEG, Director, Level, Pose};

const DT: f32 = 1.0 / 60.0;

fn run(level: Level, frames: usize, input: impl Fn(&mut Director, usize)) -> Vec<Pose> {
    let mut director = Director::new(level, fixture::preset(), 2.0, 0.0);
    (0..frames)
        .map(|frame| {
            input(&mut director, frame);
            director.step(DT)
        })
        .collect()
}

#[test]
fn a_director_on_the_neutral_preset_is_deterministic() {
    let input = |director: &mut Director, frame: usize| {
        if frame == 10 {
            director.cursor(0.4, -0.2, false);
        }
        if frame == 90 {
            director.nudge();
        }
    };
    let first = run(Level::Full, 600, input);
    let second = run(Level::Full, 600, input);
    assert_eq!(first, second);
    assert!(first.iter().any(|pose| pose.moved));
    assert!(first.iter().all(|pose| {
        [pose.yaw, pose.pitch, pose.roll, pose.offset, pose.zoom]
            .iter()
            .all(|v| v.is_finite())
    }));
}

#[test]
fn the_neutral_preset_keeps_its_zoom_inside_its_limit() {
    let preset = fixture::preset();
    let poses = run(Level::Full, 600, |director, frame| {
        if frame < 30 {
            director.zoom_by(4.0);
        }
    });
    let widest = poses.iter().map(|pose| pose.zoom).fold(0.0, f32::max);
    assert!(widest > 1.0, "{widest}");
    assert!(widest <= preset.zoom.max + 1e-4, "{widest}");
}

#[test]
fn the_neutral_preset_turns_toward_a_target_and_settles_when_off() {
    let target = Aim {
        yaw: 2.0 * DEG,
        pitch: -DEG,
        zoom: 1.0,
        focus: 2.0,
        blur: 0.0,
    };
    let poses = run(Level::Full, 300, |director, frame| {
        if frame == 0 {
            director.attend(target);
        }
    });
    let idle = run(Level::Full, 300, |_, _| {});
    let (turned, still) = (poses.last().unwrap(), idle.last().unwrap());
    assert!(turned.look[0] > still.look[0], "{turned:?} {still:?}");
    assert!(turned.look[1] < still.look[1], "{turned:?} {still:?}");
    let mut director = Director::new(Level::Off, fixture::preset(), 2.0, 0.0);
    for _ in 0..600 {
        director.step(DT);
    }
    assert!(director.settled());
}
