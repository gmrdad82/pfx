use super::follow::{self, Drive};
use crate::{Built, Registry, build_with};
use glam::{DMat4, DQuat, DVec3, EulerRot};
use pfx_motion::vehicle::Chassis;
use pfx_motion::{Rate, Route, Trs};
use pfx_recipe::{Recipe, parse};
use std::f64::consts::{PI, TAU};
use std::path::Path;

const ROUTE: &str = "[[0.0, 0.0, 0.0], [0.0, 0.0, 80.0], [0.0, 0.0, 120.0], [-20.0, 0.0, 150.0], [-50.0, 0.0, 160.0], [-90.0, 0.0, 160.0], [-200.0, 0.0, 160.0]]";

const SPEED: &str =
    "[[0.0, 12.0], [2.0, 28.0], [3.5, 28.0], [5.0, 14.0], [6.0, 14.0], [8.0, 24.0]]";

fn recipe(fps: u32, speed: &str, follow: &str) -> Recipe {
    let text = format!(
        "recipe = 1\nname = \"drive\"\nseed = 3\n\n[timeline]\nfps = {fps}\nseconds = 8.0\n\n[[route]]\nname = \"main\"\npoints = {ROUTE}\n\n[[object]]\nname = \"hero\"\nkind = \"generator\"\ngenerator = \"pfx/car\"\nparams = {{ style = \"sports\" }}\n\n[[follow]]\nobject = \"hero\"\nroute = \"main\"\nspeed = {speed}\n{follow}"
    );
    parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

struct Run {
    built: Built,
    drive: Drive,
    route: Route,
    chassis: Chassis,
    dt: f64,
}

fn run(fps: u32, speed: &str, follow: &str) -> Run {
    let r = recipe(fps, speed, follow);
    let built = build_with(&r, Path::new("."), &Registry::new()).unwrap();
    let routes = crate::routes(&r).unwrap();
    let route = routes["main"].clone();
    let facts = &built.object_facts()["hero"];
    let size = (
        facts["wheelbase"].as_f64().unwrap(),
        facts["track"].as_f64().unwrap(),
    );
    let timeline = r.timeline.unwrap();
    let drive = follow::drive(&r.follows[0], &route, &timeline, size);
    let chassis = follow::chassis(&r.follows[0], size).unwrap_or_default();
    Run {
        built,
        drive,
        route,
        chassis,
        dt: timeline.step(),
    }
}

fn dynamic() -> Run {
    run(60, SPEED, "dynamics = {}\n")
}

fn turn(t: &Trs) -> DQuat {
    DQuat::from_array(t.rotation).normalize()
}

fn keys<'a>(r: &'a Run, node: &str) -> &'a [Trs] {
    &r.built
        .tracks
        .iter()
        .find(|t| t.node == node)
        .unwrap_or_else(|| panic!("no track for {node}"))
        .keys
}

fn local(r: &Run, node: &str, n: usize) -> DQuat {
    let pose = DQuat::from_mat4(&r.drive.pose[n]).normalize();
    pose.inverse() * turn(&keys(r, node)[n])
}

fn frame(r: &Run, t: f64) -> usize {
    (t / r.dt).round() as usize
}

fn force(r: &Run, n: usize) -> DVec3 {
    pfx_motion::vehicle::forces(&r.drive.pose, r.dt)[n]
}

#[test]
fn braking_pitches_the_body_nose_down_by_the_weight_transfer() {
    let r = dynamic();
    let n = frame(&r, 4.6);
    let lean = r.drive.lean.as_ref().unwrap()[n];
    let decel = 14.0 / 1.5;
    let c = r.chassis;
    let want = c.mass * decel * c.cg_height / (c.stiffness * c.wheelbase * c.wheelbase);
    assert!(lean.pitch > 0.0, "{}", lean.pitch);
    assert!(
        (lean.pitch - want).abs() < 0.2 * want,
        "pitch {} deg, expected about {} deg",
        lean.pitch.to_degrees(),
        want.to_degrees()
    );
    assert!(lean.pitch.to_degrees() > 1.0 && lean.pitch.to_degrees() < 3.0);
    let (_, pitch, _) = local(&r, "hero.body", n).to_euler(EulerRot::YXZ);
    assert!((pitch - lean.pitch).abs() < 1e-3, "{pitch} {}", lean.pitch);
    let throttle = r.drive.lean.as_ref().unwrap()[frame(&r, 1.5)];
    assert!(throttle.pitch < 0.0, "{}", throttle.pitch);
    let cruise = r.drive.lean.as_ref().unwrap()[frame(&r, 3.4)];
    assert!(cruise.pitch.abs() < 0.2 * want);
}

#[test]
fn a_right_hand_bend_rolls_the_body_toward_the_outside_by_its_lateral_acceleration() {
    let r = dynamic();
    let n = (0..r.drive.distance.len())
        .filter(|&n| (5.2..6.0).contains(&(n as f64 * r.dt)))
        .max_by(|a, b| force(&r, *b).x.total_cmp(&force(&r, *a).x))
        .unwrap();
    let lateral = force(&r, n).x;
    assert!(lateral < -3.0, "{lateral}");
    assert!(r.route.curvature(r.drive.distance[n]) > 0.0);
    let lean = r.drive.lean.as_ref().unwrap()[n];
    let c = r.chassis;
    let want = c.mass * lateral * c.cg_height / (c.stiffness * c.track * c.track);
    assert!(lean.roll < 0.0);
    assert!(
        (lean.roll - want).abs() < 0.35 * want.abs(),
        "roll {} deg, expected about {} deg",
        lean.roll.to_degrees(),
        want.to_degrees()
    );
    let body = DMat4::from_quat(local(&r, "hero.body", n));
    let outside = body.transform_point3(DVec3::new(0.8, 1.0, 0.0));
    let inside = body.transform_point3(DVec3::new(-0.8, 1.0, 0.0));
    assert!(outside.y < inside.y, "{outside} {inside}");
}

#[test]
fn the_front_wheels_steer_by_the_paths_curvature_and_countersteer_a_slide() {
    let r = dynamic();
    let base = r.chassis.wheelbase;
    let steer = |n: usize| {
        let axle = local(&r, "hero.wheel_fl", n) * DVec3::X;
        (-axle.z).atan2(axle.x)
    };
    let n = frame(&r, 5.6);
    let curvature = r.route.curvature(r.drive.distance[n]);
    let want = -(base * curvature).atan() - r.drive.counter[n];
    assert!(want < -0.02, "{want}");
    assert!((steer(n) - want).abs() < 1e-6, "{} {want}", steer(n));
    let rear = local(&r, "hero.wheel_rl", n) * DVec3::X;
    assert!(rear.z.abs() < 1e-9);
    assert!(steer(frame(&r, 1.0)).abs() < 1e-9);
    let exit: Vec<f64> = (frame(&r, 6.2)..frame(&r, 8.0))
        .map(|n| r.drive.counter[n])
        .collect();
    let most = exit.iter().copied().fold(0.0, f64::min);
    assert!(
        most.to_degrees() < -1.0 && most.to_degrees() > -20.0,
        "{}",
        most.to_degrees()
    );
    let braking: f64 = (frame(&r, 3.6)..frame(&r, 5.0))
        .map(|n| r.drive.counter[n].abs())
        .fold(0.0, f64::max);
    assert!(braking < 1e-3, "{braking}");
}

#[test]
fn without_dynamics_the_follow_keys_as_before() {
    let plain = run(60, SPEED, "");
    assert!(plain.drive.lean.is_none());
    assert!(plain.drive.counter.iter().all(|c| *c == 0.0));
    let n = frame(&plain, 5.6);
    let body = local(&plain, "hero.body", n);
    assert!(body.angle_between(DQuat::IDENTITY) < 1e-9);
}

fn spin_of(r: &Run, node: &str, n: usize) -> f64 {
    let (axis, angle) = local(r, node, n).to_axis_angle();
    if axis.x < 0.0 { -angle } else { angle }
}

#[test]
fn a_wheel_spins_by_its_travel_over_its_radius() {
    let r = dynamic();
    let radius = r.built.object_facts()["hero"]["wheels"]["rl"]["radius"]
        .as_f64()
        .unwrap();
    for n in [frame(&r, 1.0), frame(&r, 3.0), frame(&r, 7.0)] {
        let travelled = r.drive.distance[n + 1] - r.drive.distance[n];
        let want = travelled / radius;
        let got = (spin_of(&r, "hero.wheel_rl", n + 1) - spin_of(&r, "hero.wheel_rl", n) + PI)
            .rem_euclid(TAU)
            - PI;
        assert!((got - want).abs() < 1e-6, "frame {n}: {got} {want}");
        let speed = travelled / r.dt;
        assert!((want / r.dt - speed / radius).abs() < 1e-9);
    }
}

#[test]
fn a_wheel_past_half_a_turn_a_frame_is_keyed_between_frames_and_spins_forward() {
    let r = run(30, "[[0.0, 70.0]]", "");
    let radius = r.built.object_facts()["hero"]["wheels"]["rl"]["radius"]
        .as_f64()
        .unwrap();
    let per_frame = 70.0 * r.dt / radius;
    assert!(per_frame > PI, "{per_frame}");
    let track = r
        .built
        .tracks
        .iter()
        .find(|t| t.node == "hero.wheel_rl")
        .unwrap();
    let keyed = track.keyed(Rate::whole(30));
    let inside = track
        .between
        .iter()
        .filter(|(t, _)| *t > 10.0 * r.dt && *t < 11.0 * r.dt)
        .count();
    assert_eq!(inside, (per_frame / (TAU / 3.0)).ceil() as usize - 1);
    for pair in keyed.windows(2) {
        let step = turn(&pair[0].1).angle_between(turn(&pair[1].1));
        assert!(step < TAU / 3.0 + 1e-6, "{step}");
    }
    let n = 10;
    let mid = (n as f64 + 0.5) * r.dt;
    let pose = DQuat::from_mat4(&r.drive.pose[n]).normalize();
    let between = pose.inverse() * turn(&track.sample(Rate::whole(30), mid));
    let want =
        0.5 * (spin_of(&r, "hero.wheel_rl", n) + spin_of(&r, "hero.wheel_rl", n) + per_frame);
    let wanted = DQuat::from_axis_angle(DVec3::X, want);
    assert!(between.angle_between(wanted) < 1e-3);
    let naive = pfx_motion::sample(&track.keys, Rate::whole(30), mid);
    let backwards = pose.inverse() * turn(&naive);
    assert!(backwards.angle_between(wanted) > 1.0);
    let glb = r.built.try_glb().unwrap();
    let doc = gltf::Gltf::from_slice(&glb).unwrap();
    let animation = doc.animations().next().unwrap();
    let samples = animation
        .channels()
        .filter(|c| c.target().node().name() == Some("hero.wheel_rl"))
        .map(|c| c.sampler().input().count())
        .max()
        .unwrap();
    assert_eq!(samples, keyed.len());
}
