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
