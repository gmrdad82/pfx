use pfx_recipe::{Drift, Dynamics, Follow, apply_set, apply_variant, parse, validate};

const TEXT: &str = r#"
recipe = 1
name = "chase"

[[object]]
name = "hero"
kind = "box"
size = [2.0, 1.2, 4.5]

[[follow]]
object = "hero"
route = "main"
lane = 1.5
speed = [[0.0, 10.0], [4.0, 30.0]]
drift = { amount = 0.4, period = 2.0, slip_deg = 6.0 }

[variant.wide]
set = { "follow.hero.lane" = -2.0, "follow.hero.drift.amount" = 0.9 }
"#;

#[test]
fn a_follow_reads_and_set_and_variants_reach_it() {
    let r = parse(TEXT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        r.follows,
        [Follow {
            object: "hero".to_string(),
            route: "main".to_string(),
            lane: 1.5,
            start: 0.0,
            speed: vec![[0.0, 10.0], [4.0, 30.0]],
            drift: Some(Drift {
                amount: 0.4,
                period: 2.0,
                slip_deg: 6.0,
            }),
            dynamics: None,
            ride: 0.0,
            root_motion: pfx_recipe::RootMotion::Ignore,
            wake: None,
        }]
    );
    let set = apply_set(
        &r,
        &[
            "follow.hero.lane=0.5".into(),
            "follow.hero.speed=[[0.0, 20.0]]".into(),
        ],
    )
    .unwrap();
    assert_eq!(set.follows[0].lane, 0.5);
    assert_eq!(set.follows[0].speed, [[0.0, 20.0]]);
    let wide = apply_variant(&r, "wide").unwrap();
    assert_eq!(wide.follows[0].lane, -2.0);
    assert_eq!(wide.follows[0].drift.unwrap().amount, 0.9);
}

#[test]
fn a_follows_dynamics_read_with_defaults_and_refuse_what_is_not_physical() {
    let text = TEXT.replace(
        "drift = { amount = 0.4, period = 2.0, slip_deg = 6.0 }",
        "dynamics = { mass = 1200.0, cg_height = 0.45 }",
    );
    let r = parse(&text).unwrap_or_else(|e| panic!("{e}"));
    let d = r.follows[0].dynamics.unwrap();
    assert_eq!(
        d,
        Dynamics {
            mass: 1200.0,
            cg_height: 0.45,
            ..Dynamics::default()
        }
    );
    assert_eq!(d.wheelbase, None);
    let set = apply_set(&r, &["follow.hero.dynamics.mass=900.0".into()]).unwrap();
    assert_eq!(set.follows[0].dynamics.unwrap().mass, 900.0);
    let bad = parse(&text.replace("mass = 1200.0", "mass = 0.0, damping = -1.0")).unwrap();
    let paths: Vec<String> = validate(&bad).into_iter().map(|p| p.path).collect();
    assert!(
        paths.contains(&"follow[hero].dynamics.mass".to_string()),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"follow[hero].dynamics.damping".to_string()),
        "{paths:?}"
    );
    let unknown = text.replace("cg_height = 0.45", "height = 0.45");
    assert!(parse(&unknown).is_err());
}

const WAKE: &str = r#"
recipe = 1
name = "wake"

[[object]]
name = "hero"
kind = "box"
size = [2.0, 1.2, 4.5]

[[object]]
name = "leaf"
kind = "box"
size = [0.1, 0.01, 0.08]

[[follow]]
object = "hero"
route = "main"
speed = [[0.0, 10.0]]
wake = { at = [0.0, 0.3, -2.0], strength = 0.7, radius = 1.5, decay = 1.2 }

[[particles]]
name = "leaves"
mesh = "leaf"
mass = 0.002
area = 0.004
flat = 0.8
tumble = [1.0, 4.0]
ground = 0.0
bounce = 0.2
friction = 0.6
keep = false
lift = 3.0
"#;

#[test]
fn a_follow_wake_and_mesh_particles_read_their_keys() {
    use pfx_recipe::Setting;
    let r = parse(WAKE).unwrap_or_else(|e| panic!("{e}"));
    let wake = r.follows[0].wake.as_ref().unwrap();
    assert!(wake.contains(&("strength", Setting::Number(0.7))));
    assert!(wake.contains(&("radius", Setting::Number(1.5))));
    assert!(wake.contains(&("decay", Setting::Number(1.2))));
    assert!(wake.contains(&("at", Setting::Numbers(vec![0.0, 0.3, -2.0]))));
    let leaves = &r.effects.particles[0];
    assert_eq!(leaves.mesh.as_deref(), Some("leaf"));
    for (key, value) in [
        ("mass", Setting::Number(0.002)),
        ("area", Setting::Number(0.004)),
        ("flat", Setting::Number(0.8)),
        ("tumble", Setting::Numbers(vec![1.0, 4.0])),
        ("ground", Setting::Number(0.0)),
        ("bounce", Setting::Number(0.2)),
        ("friction", Setting::Number(0.6)),
        ("keep", Setting::Flag(false)),
        ("lift", Setting::Number(3.0)),
    ] {
        assert!(leaves.settings.contains(&(key, value.clone())), "{key}");
    }
    let plain = parse(TEXT).unwrap();
    assert_eq!(plain.follows[0].wake, None);
}

#[test]
fn a_wake_or_a_body_key_out_of_range_is_refused_by_its_key() {
    for (from, to, key) in [
        ("radius = 1.5", "radius = 0.0", "wake.radius"),
        ("decay = 1.2", "decay = -1.0", "wake.decay"),
        ("strength = 0.7", "strength = -0.1", "wake.strength"),
        ("strength = 0.7", "strength = 0.7, gust = 2.0", "gust"),
        ("mass = 0.002", "mass = 0.0", "mass"),
        ("flat = 0.8", "flat = 1.5", "flat"),
        ("tumble = [1.0, 4.0]", "tumble = [4.0, 1.0]", "tumble"),
        ("bounce = 0.2", "bounce = 2.0", "bounce"),
        ("friction = 0.6", "friction = -0.5", "friction"),
        ("lift = 3.0", "lift = -1.0", "lift"),
        ("keep = false", "keep = 1", "keep"),
        ("mesh = \"leaf\"", "mesh = 3", "mesh"),
    ] {
        assert!(WAKE.contains(from), "{from}");
        let error = parse(&WAKE.replace(from, to)).expect_err(to);
        assert!(error.to_string().contains(key), "{to}: {error}");
    }
}
