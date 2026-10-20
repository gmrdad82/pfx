use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use pfx_live::flat::{FlatScene, Light, ShadowCurve, cpu};

use crate::licence::{civil_day, date};
use crate::{
    Colour, Composition, FADE_IN, KeyError, LENGTH, Phase, Settings, Timeline, build_date, built,
    covers, hidden, verify,
};

fn pair() -> (SigningKey, String) {
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let public = STANDARD.encode(signing.verifying_key().to_bytes());
    (signing, public)
}

fn mint(signing: &SigningKey, payload: &str) -> String {
    let signed = format!("PITO1.{}", URL_SAFE_NO_PAD.encode(payload.as_bytes()));
    let signature = signing.sign(signed.as_bytes());
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

fn claims(tier: &str, timed: Option<(&str, u32)>) -> String {
    let timed = timed.map_or(String::new(), |(through, grace)| {
        format!(r#","type":"timed","paid_through":"{through}","grace_days":{grace}"#)
    });
    format!(
        r#"{{"v":2,"id":"k1","email":"a@b.c","app":"pfx","tier":"{tier}","major":3,"issued":"2026-01-01"{timed}}}"#
    )
}

fn settings(key: String) -> Settings {
    Settings {
        hide: true,
        key: Some(key),
        ..Settings::default()
    }
}

#[test]
fn a_pro_key_covering_the_build_date_hides_the_splash() {
    let (signing, public) = pair();
    let timed = settings(mint(&signing, &claims("pro", Some(("2026-12-31", 14)))));
    assert!(hidden(&timed, Some(&public), Some("2026-10-19")));
    assert!(hidden(&timed, Some(&public), Some("2027-01-14")));
}

#[test]
fn no_key_an_indie_key_or_an_expired_key_shows_the_splash() {
    let (signing, public) = pair();
    assert!(!hidden(&Settings::default(), Some(&public), None));
    let indie = settings(mint(&signing, &claims("indie", None)));
    assert!(!hidden(&indie, Some(&public), Some("2026-10-19")));
    let timed = settings(mint(&signing, &claims("pro", Some(("2026-12-31", 14)))));
    assert!(!hidden(&timed, Some(&public), Some("2027-01-15")));
    assert!(!hidden(&timed, Some(&public), None));
    let lifetime = settings(mint(&signing, &claims("pro", None)));
    assert!(!hidden(&lifetime, Some(&public), None));
    assert!(!hidden(&lifetime, Some(&public), Some("2026-10-19")));
    let pro = mint(&signing, &claims("pro", None));
    let unasked = Settings {
        hide: false,
        ..settings(pro.clone())
    };
    assert!(!hidden(&unasked, Some(&public), None));
    assert!(!hidden(&settings(pro), None, None));
}

#[test]
fn a_forged_or_foreign_key_shows_the_splash() {
    let (signing, public) = pair();
    let other = SigningKey::from_bytes(&[9u8; 32]);
    let forged = mint(&other, &claims("pro", None));
    assert_eq!(verify(&public, &forged), Err(KeyError::Signature));
    assert!(!hidden(&settings(forged), Some(&public), None));
    let token = mint(&signing, &claims("pro", None));
    let (head, tail) = token.rsplit_once('.').unwrap();
    let tampered = format!("{}x.{tail}", head);
    assert!(verify(&public, &tampered).is_err());
    let foreign = mint(
        &signing,
        &claims("pro", None).replace(r#""app":"pfx""#, r#""app":"other""#),
    );
    assert!(!hidden(&settings(foreign), Some(&public), None));
    assert_eq!(verify(&public, "PITO2.a.b"), Err(KeyError::Prefix));
    assert_eq!(verify(&public, "nonsense"), Err(KeyError::Shape));
}

#[test]
fn an_armoured_key_with_line_breaks_verifies() {
    let (signing, public) = pair();
    let token = mint(&signing, &claims("pro", Some(("2026-12-31", 14))));
    let mut armoured = String::from("----- BEGIN PITO MD LICENSE -----\n");
    for chunk in token.as_bytes().chunks(40) {
        armoured.push_str(std::str::from_utf8(chunk).unwrap());
        armoured.push('\n');
    }
    armoured.push_str("------ END PITO MD LICENSE ------\n");
    let claims = verify(&public, &armoured).unwrap();
    assert!(covers(&claims, Some("2026-10-19")));
}

#[test]
fn build_dates_come_from_epoch_seconds_without_a_clock() {
    assert_eq!(date(0).as_deref(), Some("1970-01-01"));
    assert_eq!(date(1_792_368_000).as_deref(), Some("2026-10-19"));
    assert_eq!(date(951_782_400).as_deref(), Some("2000-02-29"));
    for day in [0, 11_016, 20_745, -1] {
        let text = date(day * 86_400).unwrap();
        assert_eq!(civil_day(&text), Some(day));
    }
}

#[test]
fn the_games_own_build_date_comes_first_and_a_bad_one_is_ignored() {
    assert_eq!(
        build_date(Some("2026-10-19")).as_deref(),
        Some("2026-10-19")
    );
    assert_eq!(build_date(Some("2026-02-30")), built());
    assert_eq!(build_date(Some("")), built());
    assert_eq!(build_date(None), built());
}

#[test]
fn the_splash_fades_in_holds_and_fades_out_in_two_seconds() {
    let t = Timeline;
    assert_eq!(t.opacity(0.0), 0.0);
    assert!(t.opacity(FADE_IN * 0.5) > 0.2 && t.opacity(FADE_IN * 0.5) < 0.8);
    assert_eq!(t.opacity(1.0), 1.0);
    assert_eq!(t.opacity(LENGTH), 0.0);
    assert!(matches!(t.phase(1.0, true), Phase::Showing { .. }));
    assert_eq!(t.phase(LENGTH, true), Phase::Done);
}

#[test]
fn loading_never_shortens_the_splash_and_the_splash_never_adds_waiting() {
    let t = Timeline;
    assert!(matches!(t.phase(0.3, true), Phase::Showing { .. }));
    assert_eq!(t.phase(LENGTH + 0.5, false), Phase::Waiting);
    assert_eq!(t.phase(LENGTH + 0.5, true), Phase::Done);
}

#[test]
fn settings_read_from_toml_and_refuse_unknown_keys() {
    let s: Settings = toml::from_str("background = \"#102030\"\nhide = true\n").unwrap();
    assert_eq!(s.background, Colour([0x10, 0x20, 0x30]));
    assert!(s.hide && s.key.is_none());
    assert!(toml::from_str::<Settings>("skippable = true").is_err());
    assert!(toml::from_str::<Settings>("colour = \"#000000\"").is_err());
    assert!(toml::from_str::<Settings>("background = \"red\"").is_err());
    assert_eq!(Colour::parse("#0c0a12").unwrap(), Colour::GROUND);
}

fn rendered(c: &Composition, at: f64, opacity: f32) -> Vec<[f32; 4]> {
    let size = c.size();
    let frame = c.frame(at, opacity);
    let curve = ShadowCurve::none();
    let scene = FlatScene {
        layout: size.map(|v| v as f32),
        clear: Some(frame.clear),
        curve: &curve,
        light: Light::default(),
        draws: &frame.draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    cpu::render_sprited(&scene, Some(c.icons()), c.sprites(), size).unwrap()
}

#[test]
fn the_splash_draws_on_the_ground_and_vanishes_at_zero_opacity() {
    let ground = Colour::GROUND.srgba().0;
    for size in [[192, 108], [108, 192]] {
        let c = Composition::new(size, Colour::GROUND.srgba()).unwrap();
        let pixels = rendered(&c, 1.0, 1.0);
        let lit = pixels
            .iter()
            .filter(|p| (0..3).any(|k| (p[k] - ground[k]).abs() > 0.1))
            .count();
        assert!(lit > 50, "{size:?} drew {lit} pixels");
        let corner = pixels[0];
        assert!((0..3).all(|k| (corner[k] - ground[k]).abs() < 0.01));
        let hidden = rendered(&c, 0.0, 0.0);
        assert!(
            hidden
                .iter()
                .all(|p| (0..3).all(|k| (p[k] - ground[k]).abs() < 0.01)),
            "{size:?} shows through at zero opacity"
        );
    }
}

#[test]
fn the_rule_grows_from_the_centre_before_the_site_appears() {
    let c = Composition::new([192, 108], Colour::GROUND.srgba()).unwrap();
    let before = c.frame(0.3, 1.0).draws.len();
    let growing = c.frame(0.6, 1.0).draws.len();
    let after = c.frame(1.2, 1.0).draws.len();
    assert!(before < growing && growing < after);
    let rule = |at: f64| {
        let draw = c
            .frame(at, 1.0)
            .draws
            .into_iter()
            .find(|d| {
                matches!(d.fill, pfx_live::flat::Fill::Gradient(_))
                    && matches!(d.shape, pfx_live::flat::Shape::Rect { .. })
            })
            .unwrap();
        (draw.transform[0][0], draw.transform[3][0])
    };
    let (narrow, centre_a) = rule(0.5);
    let (wide, centre_b) = rule(0.84);
    assert!(narrow < wide);
    assert!((centre_a - centre_b).abs() < 0.5);
}
