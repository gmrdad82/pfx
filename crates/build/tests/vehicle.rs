use glam::{DQuat, DVec3};
use pfx_build::{Built, Registry, build_with};
use pfx_recipe::{Kind, parse};
use std::fmt::Write;
use std::path::Path;

fn built(text: &str) -> Built {
    let recipe = parse(text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    build_with(&recipe, Path::new("."), &Registry::new()).unwrap()
}

fn keys<'a>(b: &'a Built, node: &str) -> &'a [pfx_motion::Trs] {
    &b.tracks
        .iter()
        .find(|t| t.node == node)
        .unwrap_or_else(|| panic!("no track for {node}"))
        .keys
}

fn turn(b: &Built, node: &str, n: usize) -> DQuat {
    DQuat::from_array(keys(b, node)[n].rotation).normalize()
}

fn spot(b: &Built, node: &str, n: usize) -> DVec3 {
    DVec3::from(keys(b, node)[n].translation)
}

const FOLLOWED: &str = "recipe = 1\nname = \"t\"\nseed = 3\n\n[timeline]\nfps = 30\nframes = 30\n\n[[route]]\nname = \"main\"\npoints = [[0.0, 0.0, 0.0], [0.0, 0.0, 20.0], [15.0, 0.0, 40.0], [30.0, 0.0, 45.0]]\n\n[[object]]\nname = \"car\"\nkind = \"box\"\nsize = [1.8, 0.6, 4.0]\n\n[[follow]]\nobject = \"car\"\nroute = \"main\"\nspeed = [[0.0, 10.0]]\n";

#[test]
fn a_riders_own_keys_add_to_the_follows_pose() {
    let b = built(&format!(
        "{FOLLOWED}\n[[object]]\nname = \"car.hood\"\nkind = \"box\"\nsize = [1.0, 0.1, 1.0]\nparent = \"car\"\nat = [0.0, 0.6, 1.2]\nkeys = {{ rotate = [[0.0, [0.0, 0.0, 0.0]], [1.0, [-40.0, 0.0, 0.0]]], at = [[0.0, [0.0, 0.6, 1.2]], [1.0, [0.0, 0.8, 1.2]]] }}\n"
    ));
    for n in [0usize, 15, 30] {
        let t = n as f64 / 30.0;
        let opened = turn(&b, "car", n).inverse() * turn(&b, "car.hood", n);
        let want = DQuat::from_rotation_x((-40.0 * t).to_radians());
        assert!(opened.angle_between(want) < 1e-9, "frame {n}");
        let place = spot(&b, "car", n) + turn(&b, "car", n) * DVec3::new(0.0, 0.6 + 0.2 * t, 1.2);
        assert!(
            (spot(&b, "car.hood", n) - place).length() < 1e-9,
            "frame {n}"
        );
    }
}

#[test]
fn a_keyed_car_steers_and_spins_its_wheels_from_its_own_motion() {
    let (radius, speed, fps) = (20.0f64, 10.0f64, 30u32);
    let mut at = String::new();
    let mut rotate = String::new();
    for k in 0..=60 {
        let t = f64::from(k) / f64::from(fps);
        let angle = speed * t / radius;
        let sep = if k == 0 { "" } else { ", " };
        let _ = write!(
            at,
            "{sep}[{t}, [{}, 0.0, {}]]",
            radius - radius * angle.cos(),
            radius * angle.sin()
        );
        let _ = write!(rotate, "{sep}[{t}, [0.0, {}, 0.0]]", angle.to_degrees());
    }
    let b = built(&format!(
        "recipe = 1\nname = \"t\"\nseed = 3\n\n[timeline]\nfps = {fps}\nframes = 60\n\n[[object]]\nname = \"hero\"\nkind = \"generator\"\ngenerator = \"pfx/car\"\nkeys = {{ at = [{at}], rotate = [{rotate}] }}\n"
    ));
    let facts = &b.object_facts()["hero"];
    let base = facts["wheelbase"].as_f64().unwrap();
    let wheel = facts["wheels"]["rl"]["radius"].as_f64().unwrap();
    let n = 30;
    let local = |node: &str| turn(&b, "hero.body", n).inverse() * turn(&b, node, n);
    let axle = local("hero.wheel_fl") * DVec3::X;
    let steer = (-axle.z).atan2(axle.x);
    let want = (base / radius).atan();
    assert!(steer > 0.0);
    assert!((steer - want).abs() < 0.02 * want, "{steer} {want}");
    let rear = local("hero.wheel_rl") * DVec3::X;
    assert!(rear.z.abs() < 1e-6);
    let spin = |n: usize| {
        let (axis, angle) =
            (turn(&b, "hero.body", n).inverse() * turn(&b, "hero.wheel_rl", n)).to_axis_angle();
        if axis.x < 0.0 { -angle } else { angle }
    };
    let rate = (spin(31) - spin(30) + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    let want = speed / f64::from(fps) / wheel;
    assert!((rate - want).abs() < 0.01 * want, "{rate} {want}");
}

#[test]
fn an_empty_is_a_pivot_that_draws_nothing() {
    let text = format!(
        "{FOLLOWED}\n[[object]]\nname = \"pivot\"\nkind = \"empty\"\nat = [0.0, 2.0, 0.0]\nkeys = {{ rotate = [[0.0, [0.0, 0.0, 0.0]], [1.0, [0.0, 90.0, 0.0]]] }}\n\n[[object]]\nname = \"arm\"\nkind = \"box\"\nsize = [0.2, 0.2, 0.2]\nparent = \"pivot\"\nat = [1.0, 0.0, 0.0]\n\n[[object]]\nname = \"rig\"\nkind = \"empty\"\n\n[[object]]\nname = \"rider\"\nkind = \"box\"\nsize = [0.2, 0.2, 0.2]\nparent = \"rig\"\nat = [0.0, 1.0, 0.0]\n\n[[follow]]\nobject = \"rig\"\nroute = \"main\"\nspeed = [[0.0, 5.0]]\n"
    );
    let recipe = parse(&text).unwrap();
    assert_eq!(recipe.objects[1].kind, Kind::Empty);
    assert_eq!(recipe.objects[1].kind.name(), "empty");
    let b = built(&text);
    let pivot = b.parts.iter().find(|p| p.node.name == "pivot").unwrap();
    assert!(pivot.node.mesh.indices.is_empty());
    let arm = spot(&b, "arm", 30);
    assert!((arm - DVec3::new(0.0, 2.0, -1.0)).length() < 1e-9, "{arm}");
    assert!((spot(&b, "pivot", 30) - DVec3::new(0.0, 2.0, 0.0)).length() < 1e-9);
    for n in [0usize, 30] {
        let want = spot(&b, "rig", n) + turn(&b, "rig", n) * DVec3::Y;
        assert!((spot(&b, "rider", n) - want).length() < 1e-9);
    }
    assert!(spot(&b, "rig", 30).length() > 4.0);
}
