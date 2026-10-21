use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use pfx_live::flat::{Draw, Fill, FlatScene, Icons, Light, ShadowCurve, SpriteImage, Srgba, cpu};

use crate::licence::{civil_day, date};
use crate::{
    Badge, Colour, Composition, FADE_IN, Group, KeyError, LENGTH, Lockup, Phase, Settings, Show,
    Tier, Timeline, build_date, built, covers, hidden, unlock, verify,
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
fn a_pro_key_hides_the_splash_under_auto_but_never_under_always() {
    let (signing, public) = pair();
    let auto = settings(mint(&signing, &claims("pro", Some(("2026-12-31", 14)))));
    assert_eq!(auto.show, Show::Auto);
    assert!(hidden(&auto, Some(&public), Some("2026-10-19")));
    let always = Settings {
        show: Show::Always,
        ..auto
    };
    assert!(!hidden(&always, Some(&public), Some("2026-10-19")));
    let keyless = Settings {
        show: Show::Always,
        ..Settings::default()
    };
    assert!(!hidden(&keyless, Some(&public), None));
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

fn grouped(group: &str, through: &str, grace: u32) -> String {
    claims("pro", Some((through, grace)))
        .replace(r#""v":2,"#, &format!(r#""v":2,"group":"{group}","#))
}

const BUILT: Option<&str> = Some("2026-10-20");

fn hides(key: Option<&str>, public: &str) -> [bool; 3] {
    let key = key.map(str::to_string);
    let default = Settings {
        key: key.clone(),
        ..Settings::default()
    };
    let asked = Settings {
        hide: true,
        show: Show::Auto,
        key: key.clone(),
        ..Settings::default()
    };
    let always = Settings {
        hide: true,
        show: Show::Always,
        key,
        ..Settings::default()
    };
    [default, asked, always].map(|settings| hidden(&settings, Some(public), BUILT))
}

#[test]
fn no_key_never_hides_the_splash() {
    let (_, public) = pair();
    assert_eq!(hides(None, &public), [false, false, false]);
}

#[test]
fn an_indie_key_never_hides_the_splash() {
    let (signing, public) = pair();
    let indie = mint(&signing, &claims("indie", Some(("2026-12-31", 14))));
    assert_eq!(hides(Some(&indie), &public), [false, false, false]);
}

#[test]
fn a_customer_pro_key_hides_the_splash_only_when_asked_under_auto() {
    let (signing, public) = pair();
    let unnamed = mint(&signing, &claims("pro", Some(("2026-12-31", 14))));
    let customer = mint(&signing, &grouped("customer", "2026-12-31", 14));
    let null = mint(
        &signing,
        &claims("pro", Some(("2026-12-31", 14))).replace(r#""v":2,"#, r#""v":2,"group":null,"#),
    );
    for key in [&unnamed, &customer, &null] {
        assert_eq!(verify(&public, key).unwrap().group, Group::Customer);
        assert_eq!(hides(Some(key), &public), [false, true, false]);
        assert!(!hidden(
            &settings(key.clone()),
            Some(&public),
            Some("2027-01-15")
        ));
    }
}

#[test]
fn a_friends_family_key_never_hides_the_splash() {
    let (signing, public) = pair();
    for grace in [0, 14] {
        let key = mint(&signing, &grouped("friends-family", "9999-12-31", grace));
        let claims = verify(&public, &key).unwrap();
        assert_eq!(
            (claims.tier.as_str(), claims.group),
            ("pro", Group::FriendsFamily)
        );
        assert!(claims.covers_build("2026-10-20"));
        assert_eq!(hides(Some(&key), &public), [false, false, false]);
    }
}

#[test]
fn an_owner_key_hides_the_splash_only_when_asked_under_auto() {
    let (signing, public) = pair();
    for grace in [0, 14] {
        let key = mint(&signing, &grouped("owner", "9999-12-31", grace));
        assert_eq!(verify(&public, &key).unwrap().group, Group::Owner);
        assert_eq!(hides(Some(&key), &public), [false, true, false]);
        assert!(hidden(
            &settings(key.clone()),
            Some(&public),
            Some("9999-12-31")
        ));
    }
    let indie = mint(
        &signing,
        &grouped("owner", "9999-12-31", 0).replace(r#""tier":"pro""#, r#""tier":"indie""#),
    );
    assert_eq!(verify(&public, &indie).unwrap().group, Group::Owner);
    assert_eq!(hides(Some(&indie), &public), [false, false, false]);
}

#[test]
fn an_education_nonprofit_or_unknown_group_never_hides_the_splash() {
    let (signing, public) = pair();
    for group in ["education", "nonprofit", "staff"] {
        let key = mint(&signing, &grouped(group, "2026-12-31", 14));
        assert_eq!(hides(Some(&key), &public), [false, false, false], "{group}");
    }
    let numbered = mint(
        &signing,
        &claims("pro", Some(("2026-12-31", 14))).replace(r#""v":2,"#, r#""v":2,"group":1,"#),
    );
    assert_eq!(verify(&public, &numbered), Err(KeyError::Payload));
}

#[test]
fn the_badge_reads_the_keys_group_then_its_tier() {
    let (signing, public) = pair();
    let built = Some("2026-10-20");
    let badge = |payload: &str| {
        let licence = unlock(&public, &mint(&signing, payload), built).ok();
        Badge::of(licence.as_ref())
    };
    assert_eq!(Badge::of(None), Badge::Free);
    assert_eq!(
        badge(&claims("indie", Some(("2026-12-31", 14)))),
        Badge::Indie
    );
    assert_eq!(badge(&claims("pro", Some(("2026-12-31", 14)))), Badge::Pro);
    assert_eq!(badge(&claims("pro", Some(("2026-01-31", 14)))), Badge::Free);
    assert_eq!(badge(&grouped("owner", "9999-12-31", 14)), Badge::Owner);
    assert_eq!(
        badge(&grouped("friends-family", "9999-12-31", 0)),
        Badge::FriendsFamily
    );
    assert_eq!(badge(&grouped("education", "2026-12-31", 14)), Badge::Pro);
    let owner = unlock(
        &public,
        &mint(&signing, &grouped("owner", "9999-12-31", 14)),
        built,
    )
    .unwrap();
    assert_eq!(owner.covered_through, "9999-12-31");
    assert_eq!(owner.tier, Tier::Pro);
    assert!(owner.may_hide_splash());
    let family = unlock(
        &public,
        &mint(&signing, &grouped("friends-family", "9999-12-31", 14)),
        built,
    )
    .unwrap();
    assert!(!family.may_hide_splash());
    assert_eq!(
        Badge::ALL.map(Badge::label),
        ["Free", "Indie", "Pro", "Owner", "F&F"]
    );
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
    let always: Settings = toml::from_str("show = \"always\"").unwrap();
    assert_eq!(always.show, Show::Always);
    assert_eq!(
        toml::from_str::<Settings>(&toml::to_string(&always).unwrap()).unwrap(),
        always
    );
    assert!(
        !toml::to_string(&Settings::default())
            .unwrap()
            .contains("show")
    );
    assert_eq!(
        toml::from_str::<Settings>("show = \"auto\"").unwrap(),
        Settings::default()
    );
    assert!(toml::from_str::<Settings>("show = \"never\"").is_err());
    assert!(toml::from_str::<Settings>("show = true").is_err());
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

fn reference(
    gpu: &pfx_gpu::Gpu,
    size: [u32; 2],
    at: f64,
    opacity: f32,
    output: &pfx_gpu::OffscreenTarget,
) -> Vec<u8> {
    use pfx_live::flat::{FlatIcons, FlatPass, FlatSprites};
    let ground = Colour::GROUND.srgba();
    let c = Composition::new(size, ground).unwrap();
    let icons = FlatIcons::new(&gpu.device, &gpu.queue, c.icons().clone());
    let sprites = c
        .sprites()
        .map(|s| FlatSprites::new(&gpu.device, &gpu.queue, s.clone()));
    let frame = c.frame(at, opacity);
    let curve = ShadowCurve::none();
    let scene = FlatScene {
        layout: size.map(|v| v as f32),
        clear: Some(ground),
        curve: &curve,
        light: Light::default(),
        draws: &frame.draws,
        groups: &[],
        text: &[],
        icons: Some(&icons),
        sprites: sprites.as_ref(),
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    let mut pass = FlatPass::new(&gpu.device, &gpu.queue);
    pass.prepare(&gpu.device, &gpu.queue, &scene, size).unwrap();
    let target = pass.colour_target(&gpu.device, size);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pass.encode_colour(&mut encoder, &target, Some(ground.premultiplied()), None);
    pass.encode_finish(
        &gpu.device,
        &mut encoder,
        &target,
        &output.view,
        output.format,
        output.format.is_srgb(),
        false,
        None,
    );
    gpu.queue.submit([encoder.finish()]);
    gpu.readback_rgba8(output).unwrap()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_player_draws_the_same_bytes_as_its_composition_through_a_flat_pass() {
    use pfx_gpu::wgpu::TextureFormat;
    let gpu = pollster::block_on(pfx_gpu::Gpu::headless()).unwrap();
    let size = [320, 180];
    let mut player = crate::Player::start(&Settings::default(), None, size, false).unwrap();
    for _ in 0..2500 {
        if player.ready() {
            break;
        }
        player.settle(&gpu, size).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(player.ready(), "the splash composes within five seconds");
    assert_eq!(
        player.phase(5_000_000_000, true),
        Phase::Showing {
            at: 0.0,
            opacity: 0.0
        }
    );
    let output = gpu
        .offscreen(size[0], size[1], TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    let phase = player.phase(6_000_000_000, false);
    let Phase::Showing { at, opacity } = phase else {
        panic!("one second in, the splash shows: {phase:?}");
    };
    player
        .draw(&gpu, &output.view, output.format, size, phase)
        .unwrap();
    let drawn = gpu.readback_rgba8(&output).unwrap();
    let ground = drawn[..4].to_vec();
    let lit = drawn.chunks_exact(4).filter(|p| *p != ground).count();
    assert!(lit > 500, "the player drew {lit} pixels off the ground");
    assert!(drawn == reference(&gpu, size, at, opacity, &output));
    let waiting = player.phase(7_500_000_000, false);
    assert_eq!(waiting, Phase::Waiting);
    player
        .draw(&gpu, &output.view, output.format, size, waiting)
        .unwrap();
    let held = gpu.readback_rgba8(&output).unwrap();
    assert!(held.chunks_exact(4).all(|p| p == ground));
    assert_eq!(player.phase(7_500_000_000, true), Phase::Done);
}

fn board(
    size: [u32; 2],
    ground: Srgba,
    draws: &[Draw],
    icons: &Icons,
    sprites: Option<&SpriteImage>,
) -> Vec<[f32; 4]> {
    let curve = ShadowCurve::none();
    let scene = FlatScene {
        layout: size.map(|v| v as f32),
        clear: Some(ground),
        curve: &curve,
        light: Light::default(),
        draws,
        groups: &[],
        text: &[],
        icons: None,
        sprites: None,
        environment: None,
        post: false,
        frame: 0,
        seed: 0,
    };
    cpu::render_sprited(&scene, Some(icons), sprites, size).unwrap()
}

#[test]
fn a_lockup_on_a_board_draws_exactly_the_splashs_lockup() {
    for ground in [Colour::GROUND.srgba(), Srgba::hex(0xf4f1ea)] {
        for size in [[320, 180], [180, 320]] {
            let c = Composition::new(size, ground).unwrap();
            let pieces = crate::compose::pieces(size, crate::compose::is_light(ground)).unwrap();
            let lockup = Lockup::new(ground, pieces[0].height, 1.0).unwrap();
            let at = [pieces[0].at[0] - lockup.size()[0] * 0.5, pieces[0].at[1]];
            let own = lockup.draws(at);
            let dense = Lockup::new(ground, pieces[0].height, 2.0).unwrap();
            assert_eq!(dense.size(), lockup.size());
            assert_eq!(dense.draws(at).len(), own.len());
            assert!(
                dense
                    .draws(at)
                    .iter()
                    .zip(&own)
                    .all(|(a, b)| a.transform == b.transform)
            );
            let (thin, thick) = (lockup.sprites().unwrap(), dense.sprites().unwrap());
            assert!(thick.width * 2 >= thin.width * 3 && thick.height * 2 >= thin.height * 3);
            assert!(own.iter().all(|d| d.group.is_none()));
            assert!(own.iter().any(|d| matches!(d.fill, Fill::Image(_))));
            let frame = c.frame(LENGTH, 1.0);
            assert!(frame.draws.len() > own.len());
            let splash = board(
                size,
                ground,
                &frame.draws[..own.len()],
                c.icons(),
                c.sprites(),
            );
            let g = ground.0;
            let lit = splash
                .iter()
                .filter(|p| (0..3).any(|k| (p[k] - g[k]).abs() > 0.1))
                .count();
            assert!(lit > 200, "{size:?} drew {lit} lockup pixels");
            let alone = board(
                size,
                ground,
                &own,
                &lockup.icons().unwrap(),
                lockup.sprites(),
            );
            let mut shapes = vec![pfx_live::flat::icons::tick()];
            shapes.extend_from_slice(lockup.shapes());
            let icons = Icons::bake(&shapes).unwrap();
            let sheet = lockup.sprites().unwrap();
            let row = (sheet.width * 4) as usize;
            let mut rgba = Vec::with_capacity(row * 2 * sheet.height as usize);
            for line in sheet.rgba.chunks_exact(row) {
                rgba.extend([255, 0, 0, 255].repeat(sheet.width as usize));
                rgba.extend_from_slice(line);
            }
            let atlas =
                SpriteImage::new(sheet.width * 2, sheet.height, rgba, sheet.filter).unwrap();
            let shared = board(
                size,
                ground,
                &lockup.draws_in(at, 1, [0.5, 0.0, 1.0, 1.0]),
                &icons,
                Some(&atlas),
            );
            for (name, drawn) in [("alone", &alone), ("shared", &shared)] {
                let worst = splash
                    .iter()
                    .zip(drawn.iter())
                    .flat_map(|(a, b)| (0..4).map(move |k| (a[k] - b[k]).abs()))
                    .fold(0.0f32, f32::max);
                assert!(
                    worst <= 2.0 / 255.0,
                    "{name} at {size:?} on {g:?} differs from the splash by {}/255",
                    worst * 255.0
                );
            }
        }
    }
}
