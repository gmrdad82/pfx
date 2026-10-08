use std::sync::Arc;

use super::fixture;
use super::math::{IDENTITY, invert, multiply, nlerp, quat_axis_angle, slerp, transform_point};
use super::*;

fn close(a: &[f32], b: &[f32], tolerance: f32) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
}

fn channel(
    property: Property,
    interpolation: Interpolation,
    times: &[f32],
    values: &[f32],
) -> Channel {
    Channel {
        joint: 0,
        property,
        interpolation,
        times: times.to_vec(),
        values: values.to_vec(),
    }
}

#[test]
fn linear_translation_samples_between_keys_and_clamps_outside() {
    let channel = channel(
        Property::Translation,
        Interpolation::Linear,
        &[1.0, 3.0],
        &[0.0, 0.0, 0.0, 2.0, 4.0, 6.0],
    );
    assert_eq!(channel.sample(2.0)[..3], [1.0, 2.0, 3.0]);
    assert_eq!(channel.sample(1.5)[..3], [0.5, 1.0, 1.5]);
    assert_eq!(channel.sample(0.0)[..3], [0.0, 0.0, 0.0]);
    assert_eq!(channel.sample(9.0)[..3], [2.0, 4.0, 6.0]);
}

#[test]
fn step_holds_each_key_until_the_next() {
    let channel = channel(
        Property::Scale,
        Interpolation::Step,
        &[0.0, 1.0],
        &[1.0, 1.0, 1.0, 2.0, 3.0, 4.0],
    );
    assert_eq!(channel.sample(0.999)[..3], [1.0, 1.0, 1.0]);
    assert_eq!(channel.sample(1.0)[..3], [2.0, 3.0, 4.0]);
}

#[test]
fn cubic_spline_matches_the_hermite_basis() {
    let channel = channel(
        Property::Translation,
        Interpolation::CubicSpline,
        &[0.0, 2.0],
        &[
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0,
            0.0,
        ],
    );
    let u = 0.25f32;
    let h00 = 2.0 * u * u * u - 3.0 * u * u + 1.0;
    let h10 = u * u * u - 2.0 * u * u + u;
    let h01 = -2.0 * u * u * u + 3.0 * u * u;
    let expected = h00 * 1.0 + h10 * 2.0 * 1.0 + h01 * 3.0;
    let got = channel.sample(0.5);
    assert!((got[0] - expected).abs() < 1e-6, "{got:?} vs {expected}");
    assert!((got[0] - 1.59375).abs() < 1e-6);
    assert_eq!(channel.sample(2.0)[0], 3.0);
}

#[test]
fn rotation_slerps_at_constant_speed() {
    let quarter = quat_axis_angle([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2);
    let channel = channel(
        Property::Rotation,
        Interpolation::Linear,
        &[0.0, 1.0],
        &[
            0.0, 0.0, 0.0, 1.0, quarter[0], quarter[1], quarter[2], quarter[3],
        ],
    );
    let half = channel.sample(0.5);
    assert!(
        close(&half, &[0.0, 0.0, 0.382_683_43, 0.923_879_5], 1e-6),
        "{half:?}"
    );
    let third = channel.sample(1.0 / 3.0);
    let angle = 2.0 * third[2].atan2(third[3]);
    assert!((angle - std::f32::consts::FRAC_PI_6).abs() < 1e-5);
    let flipped = slerp([0.0, 0.0, 0.0, 1.0], quarter.map(|value| -value), 0.5);
    assert!(close(&flipped, &half, 1e-6));
}

#[test]
fn cubic_rotation_comes_back_normalized() {
    let rig = fixture::rig(3);
    let clip = &rig.clips()[rig.clip("wave").unwrap()];
    let mut pose = Pose::rest(rig.skeleton());
    for step in 0..=24 {
        clip.sample(step as f32 * 0.05, &mut pose);
        for local in pose.locals() {
            let q = local.rotation;
            let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            assert!((length - 1.0).abs() < 1e-6);
        }
    }
    clip.sample(0.6, &mut pose);
    let expected = quat_axis_angle([0.0, 0.0, 1.0], -0.5);
    assert!(close(&pose.locals()[1].rotation, &expected, 1e-6));
}

#[test]
fn bad_channels_and_rigs_are_refused_by_name() {
    let short = Clip::new(
        "short",
        vec![channel(
            Property::Translation,
            Interpolation::Linear,
            &[0.0, 1.0],
            &[0.0; 5],
        )],
    );
    assert!(matches!(short, Err(AnimError::Channel { ref clip, .. }) if clip == "short"));
    let falling = Clip::new(
        "falling",
        vec![channel(
            Property::Scale,
            Interpolation::Step,
            &[1.0, 0.5],
            &[1.0; 6],
        )],
    );
    assert!(falling.unwrap_err().to_string().contains("do not rise"));
    let far = Clip::new(
        "far",
        vec![Channel {
            joint: 7,
            ..channel(Property::Scale, Interpolation::Step, &[0.0], &[1.0; 3])
        }],
    )
    .unwrap();
    assert!(Rig::new(fixture::skeleton(2), vec![far]).is_err());
    let late = fixture::clips(2)
        .remove(0)
        .with_events(vec![ClipEvent::new(4.0, "late")]);
    assert!(matches!(late, Err(AnimError::Event { ref event, .. }) if event == "late"));
    let orphan = Skeleton::new(
        vec![Joint {
            name: "a".into(),
            parent: Some(0),
            rest: Trs::IDENTITY,
        }],
        vec![IDENTITY],
    );
    assert_eq!(orphan, Err(AnimError::ParentOrder("a".into())));
}

#[test]
fn the_rest_pose_gives_an_identity_palette() {
    let rig = fixture::rig(4);
    let palette = Pose::rest(rig.skeleton()).palette(rig.skeleton());
    assert_eq!(palette.len(), 4);
    for matrix in &palette {
        assert!(close(matrix.as_flattened(), IDENTITY.as_flattened(), 1e-6));
    }
}

#[test]
fn globals_chain_through_parents() {
    let rig = fixture::rig(3);
    let mut pose = Pose::rest(rig.skeleton());
    pose.locals_mut()[1].rotation = quat_axis_angle([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2);
    let globals = pose.globals(rig.skeleton());
    let tip = transform_point(&globals[2], [0.0, 0.0, 0.0]);
    assert!(close(&tip, &[-1.0, 1.0, 0.0], 1e-6), "{tip:?}");
}

#[test]
fn blending_mixes_each_layer_over_the_ones_before() {
    let rig = Arc::new(fixture::rig(3));
    let mut animator = Animator::new(rig.clone(), 60);
    animator.play("lift", 0).unwrap();
    animator.blend("bend", 0.25, 0).unwrap();
    let tick = 18;
    let pose = animator.pose(tick);
    let mut lift = Pose::rest(rig.skeleton());
    rig.clips()[rig.clip("lift").unwrap()].sample(0.3, &mut lift);
    let mut bend = Pose::rest(rig.skeleton());
    rig.clips()[rig.clip("bend").unwrap()].sample(0.3, &mut bend);
    let mut expected = lift.clone();
    rig.clips()[rig.clip("bend").unwrap()].apply(0.3, &mut expected, 0.25);
    assert_eq!(pose, expected);
    let root = pose.locals()[0].translation;
    assert!(close(&root, &[0.0, 0.375, 0.0], 1e-6), "{root:?}");
    let bent = nlerp(Trs::IDENTITY.rotation, bend.locals()[1].rotation, 0.25);
    assert_eq!(pose.locals()[1].rotation, bent);
    animator.set_weight("bend", 0.0).unwrap();
    assert_eq!(animator.pose(tick), lift);
    assert_eq!(
        animator.set_weight("wave", 0.5),
        Err(AnimError::NotPlaying("wave".into()))
    );
    assert!(matches!(
        animator.blend("bend", 1.5, 0),
        Err(AnimError::Weight { .. })
    ));
    assert_eq!(animator.playing(), vec![("lift", 1.0), ("bend", 0.0)]);
}

#[test]
fn a_crossfade_runs_from_the_old_mix_to_the_new_over_its_ticks() {
    let rig = Arc::new(fixture::rig(3));
    let mut animator = Animator::new(rig.clone(), 60);
    animator.play("bend", 0).unwrap();
    animator.crossfade("lift", 0.5, 30).unwrap();
    let before = {
        let mut only = Animator::new(rig.clone(), 60);
        only.play("bend", 0).unwrap();
        only
    };
    let after = {
        let mut only = Animator::new(rig.clone(), 60);
        only.play("lift", 30).unwrap();
        only
    };
    assert_eq!(animator.pose(30), before.pose(30));
    let mut expected = before.pose(45);
    expected.mix(&after.pose(45), 0.5);
    assert_eq!(animator.pose(45), expected);
    assert_eq!(animator.pose(60), after.pose(60));
    assert!(animator.advance(61).is_empty());
    assert_eq!(animator.playing(), vec![("lift", 1.0)]);
    assert_eq!(animator.pose(70), after.pose(70));
    assert!(animator.crossfade("nope", 0.1, 70).is_err());
    assert_eq!(
        animator.crossfade("bend", -1.0, 70),
        Err(AnimError::Fade(-1.0))
    );
}

#[test]
fn events_fire_once_per_crossing_in_tick_order_and_loop() {
    let rig = Arc::new(fixture::rig(2));
    let mut animator = Animator::new(rig, 10);
    animator.play("bend", 3).unwrap();
    let mut heard = Vec::new();
    for tick in 3..=28 {
        for fired in animator.advance(tick) {
            heard.push((fired.tick, fired.event));
        }
    }
    assert_eq!(
        heard,
        vec![
            (3, "start".to_string()),
            (8, "half".to_string()),
            (13, "start".to_string()),
            (18, "half".to_string()),
            (23, "start".to_string()),
            (28, "half".to_string()),
        ]
    );
}

#[test]
fn a_once_clip_holds_its_last_key_and_fires_its_events_once() {
    let rig = Arc::new(fixture::rig(2));
    let mut animator = Animator::new(rig.clone(), 10);
    animator.play("bend", 0).unwrap();
    animator.set_playback("bend", Playback::Once).unwrap();
    let mut count = 0;
    for tick in 0..40 {
        count += animator.advance(tick).len();
    }
    assert_eq!(count, 2);
    assert!(animator.finished("bend", 10));
    assert!(!animator.finished("bend", 9));
    assert_eq!(animator.time("bend", 39), Some(1.0));
}

#[test]
fn a_skipped_stretch_of_ticks_delivers_every_crossing() {
    let rig = Arc::new(fixture::rig(2));
    let mut animator = Animator::new(rig, 10);
    animator.play("bend", 0).unwrap();
    assert_eq!(animator.advance(0).len(), 1);
    let fired = animator.advance(30);
    let names: Vec<&str> = fired.iter().map(|fired| fired.event.as_str()).collect();
    assert_eq!(names, ["half", "start", "half", "start", "half", "start"]);
}

#[test]
fn speed_changes_keep_the_clip_time_continuous() {
    let rig = Arc::new(fixture::rig(2));
    let mut animator = Animator::new(rig, 60);
    animator.play("bend", 0).unwrap();
    let before = animator.time("bend", 30).unwrap();
    animator.set_speed("bend", 2.0, 30).unwrap();
    assert_eq!(animator.time("bend", 30).unwrap(), before);
    let later = animator.time("bend", 36).unwrap();
    assert!((later - (0.5 + 0.2)).abs() < 1e-6, "{later}");
    assert_eq!(
        animator.set_speed("bend", -1.0, 36),
        Err(AnimError::Speed(-1.0))
    );
}

#[test]
fn cpu_skinning_moves_vertices_with_their_joints() {
    let rig = fixture::rig(2);
    let column = fixture::column(2);
    let mut pose = Pose::rest(rig.skeleton());
    pose.locals_mut()[0].translation = [2.0, 0.0, 0.0];
    let palette = pose.palette(rig.skeleton());
    let skinned = skin(
        &column.positions,
        &column.normals,
        &column.tangents,
        &column.joints,
        &column.weights,
        &palette,
    )
    .unwrap();
    for (moved, rest) in skinned.positions.iter().zip(&column.positions) {
        assert!(close(moved, &[rest[0] + 2.0, rest[1], rest[2]], 1e-6));
    }
    assert_eq!(skinned.normals, column.normals);
    let rest = Pose::rest(rig.skeleton()).palette(rig.skeleton());
    let still = skin(
        &column.positions,
        &column.normals,
        &column.tangents,
        &column.joints,
        &column.weights,
        &rest,
    )
    .unwrap();
    assert_eq!(still.positions, column.positions);
    let bad = skin(
        &column.positions,
        &column.normals,
        &[],
        &column.joints,
        &column.weights,
        &palette[..1],
    );
    assert!(matches!(bad, Err(AnimError::Skin(_))));
}

#[test]
fn blend_matrix_is_the_weighted_sum_of_the_palette() {
    let a = multiply(&IDENTITY, &invert(&IDENTITY).unwrap());
    let mut b = IDENTITY;
    b[3] = [4.0, 0.0, 0.0, 1.0];
    let matrix = blend_matrix(&[a, b], [0, 1, 0, 0], [0.75, 0.25, 0.0, 0.0]);
    assert_eq!(matrix[3], [1.0, 0.0, 0.0, 1.0]);
    let mut weights = vec![[2.0, 2.0, 0.0, 0.0], [0.0; 4]];
    normalize_weights(&mut weights);
    assert_eq!(weights, vec![[0.5, 0.5, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]]);
}

fn script_hash() -> u64 {
    let rig = Arc::new(fixture::rig(4));
    let mut animator = Animator::new(rig, 60);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |bits: u32| {
        hash ^= u64::from(bits);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    animator.play("wave", 0).unwrap();
    for tick in 0..400u64 {
        match tick {
            40 => animator.blend("bend", 0.3, tick).unwrap(),
            90 => animator.crossfade("twist", 0.4, tick).unwrap(),
            100 => animator.blend("lift", 0.6, tick).unwrap(),
            150 => animator.set_speed("twist", 1.7, tick).unwrap(),
            200 => animator.crossfade("bend", 0.25, tick).unwrap(),
            210 => animator.crossfade("wave", 0.5, tick).unwrap(),
            300 => animator.set_playback("wave", Playback::Once).unwrap(),
            _ => {}
        }
        for fired in animator.advance(tick) {
            mix(fired.tick as u32);
            for byte in fired.event.bytes() {
                mix(u32::from(byte));
            }
        }
        for value in animator.palette(tick).as_flattened().as_flattened() {
            mix(value.to_bits());
        }
    }
    hash
}

#[test]
fn a_scripted_run_gives_the_same_bits_on_every_platform() {
    assert_eq!(script_hash(), script_hash());
    assert_eq!(script_hash(), GOLDEN);
}

const GOLDEN: u64 = 0xaa9a_f1b0_3c58_ea34;

#[test]
fn anim_sources_call_no_std_transcendentals() {
    let sources = [
        ("animator.rs", include_str!("animator.rs")),
        ("clip.rs", include_str!("clip.rs")),
        ("math.rs", include_str!("math.rs")),
        ("pose.rs", include_str!("pose.rs")),
        ("skeleton.rs", include_str!("skeleton.rs")),
        ("skin.rs", include_str!("skin.rs")),
        ("sprite.rs", include_str!("sprite.rs")),
    ];
    let forbidden = [
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "exp",
        "ln",
        "log",
        "powf",
        "powi",
        "cbrt",
        "hypot",
        "sin_cos",
        "mul_add",
        "to_degrees",
        "to_radians",
    ];
    for (file, source) in sources {
        for (number, line) in source.lines().enumerate() {
            for name in forbidden {
                for call in [format!(".{name}("), format!("::{name}(")] {
                    assert!(!line.contains(&call), "{file}:{} calls {name}", number + 1);
                }
            }
            for text in ["SystemTime", "Instant", "thread_rng", "HashMap"] {
                assert!(!line.contains(text), "{file}:{} uses {text}", number + 1);
            }
        }
    }
}

#[test]
fn ping_pong_runs_to_and_fro_and_fires_on_every_pass() {
    let rig = Arc::new(fixture::rig(2));
    let mut animator = Animator::new(rig, 10);
    animator.play("bend", 0).unwrap();
    animator.set_playback("bend", Playback::PingPong).unwrap();
    assert_eq!(animator.time("bend", 3), Some(0.3));
    assert_eq!(animator.time("bend", 13), Some(0.7));
    assert_eq!(animator.time("bend", 20), Some(0.0));
    let mut heard = Vec::new();
    for tick in 0..=40 {
        for fired in animator.advance(tick) {
            heard.push((fired.tick, fired.event));
        }
    }
    let expected: Vec<(u64, String)> = [
        (0, "start"),
        (5, "half"),
        (15, "half"),
        (20, "start"),
        (25, "half"),
        (35, "half"),
        (40, "start"),
    ]
    .into_iter()
    .map(|(tick, name)| (tick, name.to_string()))
    .collect();
    assert_eq!(heard, expected);
}

#[test]
fn a_start_offset_skips_the_marks_before_it() {
    let rig = Arc::new(fixture::rig(2));
    let mut animator = Animator::new(rig, 10);
    animator.play("bend", 0).unwrap();
    animator.set_start("bend", 0.6, 0).unwrap();
    assert_eq!(animator.time("bend", 0), Some(0.6));
    let fired: Vec<(u64, String)> = (0..=10)
        .flat_map(|tick| animator.advance(tick))
        .map(|fired| (fired.tick, fired.event))
        .collect();
    assert_eq!(
        fired,
        vec![(4, "start".to_string()), (9, "half".to_string())]
    );
    assert_eq!(
        animator.set_start("bend", -1.0, 0),
        Err(AnimError::Start(-1.0))
    );
}

#[test]
fn a_blend_set_shares_its_weights_by_their_sum() {
    let rig = Arc::new(fixture::rig(3));
    let mut animator = Animator::new(rig.clone(), 60);
    animator
        .play_blend(&[("lift", 0.6), ("bend", 0.2), ("wave", 0.2)], 0)
        .unwrap();
    assert_eq!(
        animator.playing(),
        vec![("lift", 1.0), ("bend", 0.25), ("wave", 0.2)]
    );
    let root = animator.pose(12).locals()[0].translation;
    assert!(close(&root, &[0.0, 0.25, 0.0], 1e-6), "{root:?}");
    assert!(animator.play_blend(&[("lift", -1.0)], 0).is_err());
    assert!(animator.play_blend(&[], 0).is_err());
}
