use pfx_recipe::{Problem, Recipe, apply_set, parse, validate};

const HEAD: &str = r#"
recipe = 1
name = "presets"

[timeline]
fps = 30
seconds = 6.0

[[object]]
name = "actor"
kind = "box"
size = [1.0, 1.0, 2.0]

"#;

fn recipe(tail: &str) -> Recipe {
    let text = format!("{HEAD}{tail}");
    parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn same(preset: &str, expanded: &str) {
    let a = recipe(preset);
    let b = recipe(expanded);
    assert_eq!(a.cameras, b.cameras);
    let problems: Vec<String> = validate(&a).iter().map(Problem::to_string).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

fn refused(tail: &str) -> String {
    let text = format!("{HEAD}{tail}");
    parse(&text).expect_err(&text).to_string()
}

#[test]
fn chase_expands_into_the_rig_it_names() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"chase\"\nfollow = \"actor\"\ndistance = 8.0\nheight = 2.0\nlateral = 1.0\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.rig]\nfollow = \"actor\"\noffset = [-1.0, 2.0, -8.0]\nlook = [0.0, 0.6, 0.0]\nlag = 0.25\nlook_ahead = 0.3\nyaw_lag = 0.4\nbank = 4.0\n",
    );
}

#[test]
fn chase_focuses_on_the_actor_when_the_camera_has_depth_of_field() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\nfstop = 2.8\n\n[camera.preset]\nkind = \"chase\"\nfollow = \"actor\"\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\nfstop = 2.8\n\n[camera.rig]\nfollow = \"actor\"\noffset = [0.0, 1.1, -5.5]\nlook = [0.0, 0.6, 0.0]\nlag = 0.25\nlook_ahead = 0.3\nyaw_lag = 0.4\nbank = 4.0\nfocus = \"follow\"\n",
    );
}

#[test]
fn a_rig_table_beside_a_preset_wins_key_by_key() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"chase\"\nfollow = \"actor\"\n\n[camera.rig]\nlag = 0.0\nshake = { translation = 0.02 }\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.rig]\nfollow = \"actor\"\noffset = [0.0, 1.1, -5.5]\nlook = [0.0, 0.6, 0.0]\nlag = 0.0\nlook_ahead = 0.3\nyaw_lag = 0.4\nbank = 4.0\nshake = { translation = 0.02 }\n",
    );
}

#[test]
fn orbit_expands_into_a_path_and_the_start_pose() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 35.0\n\n[camera.preset]\nkind = \"orbit\"\nradius = 3.0\nheight = 1.5\nturns = 0.5\nrise_deg = 10.0\nease = \"smooth\"\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 35.0\nposition = [0.0, 1.5, 3.0]\ntarget = [0.0, 1.0, 0.0]\n\n[camera.path]\nkind = \"orbit\"\ntarget = [0.0, 1.0, 0.0]\nturns = 0.5\nrise_deg = 10.0\nradius_scale = 1.0\nease = \"smooth\"\n",
    );
}

#[test]
fn orbit_starts_at_its_azimuth_around_the_target() {
    let r = recipe(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"orbit\"\ntarget = [1.0, 0.0, 2.0]\nradius = 4.0\nazimuth_deg = 90.0\nheight = 2.0\n",
    );
    let at = r.cameras[0].position;
    assert!(
        (at[0] - 5.0).abs() < 1e-12 && (at[1] - 2.0).abs() < 1e-12 && (at[2] - 2.0).abs() < 1e-12
    );
}

#[test]
fn dolly_expands_into_two_keys_with_a_lens_change() {
    same(
        "[[camera]]\nname = \"c\"\n\n[camera.preset]\nkind = \"dolly\"\nfrom = [0.0, 1.0, 6.0]\nto = [0.0, 1.0, 2.0]\ntarget = [0.0, 1.0, 0.0]\nfocal_mm = [35.0, 70.0]\n",
        "[[camera]]\nname = \"c\"\nposition = [0.0, 1.0, 6.0]\ntarget = [0.0, 1.0, 0.0]\nfocal_mm = 35.0\nsensor_mm = 36.0\n\n[camera.path]\nkind = \"keys\"\nease = \"smooth\"\n\n[[camera.path.keys]]\nt = 0.0\nposition = [0.0, 1.0, 6.0]\ntarget = [0.0, 1.0, 0.0]\nfocal_mm = 35.0\n\n[[camera.path.keys]]\nt = 1.0\nposition = [0.0, 1.0, 2.0]\ntarget = [0.0, 1.0, 0.0]\nfocal_mm = 70.0\n",
    );
}

#[test]
fn crane_keys_pitch_and_distance_over_the_timeline() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"crane\"\nfollow = \"actor\"\nfrom_pitch = 0.0\nto_pitch = 30.0\nfrom_distance = 6.0\nto_distance = 8.0\nsmooth = 0.4\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.rig]\nfollow = \"actor\"\noffset = [0.0, 0.0, -6.0]\nlook = [0.0, 0.6, 0.0]\nsmooth = { pitch = 0.4, distance = 0.4 }\n\n[camera.rig.tracks]\ndistance = [[0.0, 6.0], [6.0, 8.0]]\npitch = [[0.0, 0.0], [6.0, 30.0]]\n",
    );
}

#[test]
fn drone_circles_in_the_world_frame_with_a_seeded_drift() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"drone\"\nfollow = \"actor\"\ndistance = 10.0\nfrom_pitch = 0.0\nto_pitch = 20.0\nturns = 1.0\nseed = 7\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.rig]\nfollow = \"actor\"\noffset = [0.0, 0.0, -10.0]\nlook = [0.0, 0.6, 0.0]\nframe = \"world\"\nshake = { translation = 0.4, roll = 0.3, frequency = 0.35, seed = 7 }\n\n[camera.rig.tracks]\ndistance = [[0.0, 10.0]]\npitch = [[0.0, 0.0], [6.0, 20.0]]\norbit = [[0.0, 0.0], [1.5, 90.0], [3.0, 180.0], [4.5, 270.0], [6.0, 360.0]]\n",
    );
}

#[test]
fn drive_by_is_a_rig_released_at_the_start() {
    same(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\nfstop = 4.0\n\n[camera.preset]\nkind = \"drive-by\"\nfollow = \"actor\"\n",
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\nfstop = 4.0\n\n[camera.rig]\nfollow = \"actor\"\noffset = [-4.0, 1.4, 25.0]\nlook = [0.0, 0.5, 0.0]\nrelease = 0.0\nrelease_ease = 0.001\nfocus = \"follow\"\n",
    );
}

#[test]
fn set_reaches_a_preset_s_parameters() {
    let r = recipe(
        "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"chase\"\nfollow = \"actor\"\ndistance = 8.0\n",
    );
    let r = apply_set(&r, &["camera.c.preset.distance=9.0".to_string()]).expect("set");
    assert_eq!(r.cameras[0].rig.as_ref().expect("rig").offset[2], -9.0);
}

#[test]
fn a_preset_is_refused_by_name_when_it_cannot_apply() {
    assert!(
        refused("[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"chase\"\nfollow = \"actor\"\nspeed = 3.0\n")
            .contains("camera[c].preset.speed")
    );
    assert!(
        refused(
            "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"chase\"\n"
        )
        .contains("camera[c].preset.follow")
    );
    assert!(
        refused(
            "[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"swoop\"\n"
        )
        .contains("one of: chase, orbit, dolly, crane, drone, drive-by")
    );
    assert!(
        refused("[[camera]]\nname = \"c\"\nkind = \"orthographic\"\nposition = [0.0, 5.0, 0.0]\ntarget = [0.0, 0.0, 0.0]\northo_height = 2.0\n\n[camera.preset]\nkind = \"orbit\"\n")
            .contains("perspective")
    );
    let text = "recipe = 1\nname = \"x\"\n\n[[camera]]\nname = \"c\"\nfov_y_deg = 38.0\n\n[camera.preset]\nkind = \"crane\"\nfollow = \"a\"\n";
    assert!(
        parse(text)
            .expect_err("no length")
            .to_string()
            .contains("camera[c].preset.seconds")
    );
}

const SCENE: &str = "[[camera]]\nname = \"c\"\nposition = [0.0, 1.5, 4.0]\ntarget = [0.0, 1.0, 0.0]\nfov_y_deg = 38.0\n\n";

fn lit(text: &str) -> Recipe {
    recipe(&format!("{SCENE}{text}"))
}

#[test]
fn three_point_lighting_writes_a_three_point_rig_on_an_analytic_sky() {
    let r = lit("[lighting]\npreset = \"three-point\"\ndistance = 2.0\nintensity = 2.0\n");
    let names: Vec<&str> = r.lights.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["rig_key", "rig_fill", "rig_back"]);
    assert_eq!(r.lights[0].intensity, 1.8 * 4.0 * 2.0);
    assert!(r.lights[0].shadow && !r.lights[1].shadow);
    let sky = r.sky.as_ref().expect("sky");
    assert_eq!(sky.kind, pfx_recipe::SkyKind::Analytic);
    assert_eq!(sky.intensity, 0.5);
    assert_eq!(r.sun.as_ref().expect("sun").irradiance, 0.0);
    assert_eq!(r.look.as_ref().expect("look").exposure, 1.0);
    let problems: Vec<String> = validate(&r).iter().map(Problem::to_string).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn every_lighting_preset_reads_and_validates() {
    for kind in ["three-point", "golden-hour", "overcast", "night"] {
        let r = lit(&format!("[lighting]\npreset = \"{kind}\"\n"));
        assert!(
            r.sky.is_some() && r.sun.is_some() && r.look.is_some(),
            "{kind}"
        );
        let problems: Vec<String> = validate(&r).iter().map(Problem::to_string).collect();
        assert!(problems.is_empty(), "{kind}: {problems:?}");
    }
}

#[test]
fn golden_hour_puts_a_warm_low_sun_at_its_azimuth_and_elevation() {
    let r = lit("[lighting]\npreset = \"golden-hour\"\nazimuth_deg = 90.0\nelevation_deg = 10.0\n");
    let sun = r.sun.as_ref().expect("sun");
    let want = [10f64.to_radians().cos(), 10f64.to_radians().sin(), 0.0];
    assert!((sun.toward[0] - want[0]).abs() < 1e-12);
    assert!((sun.toward[1] - want[1]).abs() < 1e-12);
    assert!(sun.toward[2].abs() < 1e-12);
    assert!(sun.color[0] > sun.color[2]);
    let bounce = &r.lights[0];
    assert!(bounce.position[0] < 0.0, "{:?}", bounce.position);
}

#[test]
fn the_recipe_s_own_sky_and_lights_win_over_the_preset() {
    let r = lit(
        "[lighting]\npreset = \"three-point\"\n\n[sky]\nkind = \"analytic\"\nintensity = 0.9\n\n[[light]]\nname = \"rig_key\"\nposition = [1.0, 2.0, 3.0]\nintensity = 1.0\n",
    );
    assert_eq!(r.sky.as_ref().expect("sky").intensity, 0.9);
    assert_eq!(r.lights.len(), 3);
    let key = r.lights.iter().find(|l| l.name == "rig_key").expect("key");
    assert_eq!(key.position, [1.0, 2.0, 3.0]);
}

#[test]
fn a_fetched_hdri_replaces_the_analytic_sky_and_falls_back_without_it() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/preset-hdri");
    std::fs::create_dir_all(dir.join("assets/sky")).expect("dir");
    let text = format!("{HEAD}{SCENE}[lighting]\npreset = \"overcast\"\n");
    let load = |text: &str| {
        std::fs::write(dir.join("r.toml"), text).expect("write");
        pfx_recipe::load(&dir.join("r.toml")).expect("load").recipe
    };
    let without = load(&text);
    assert_eq!(
        without.sky.as_ref().expect("sky").kind,
        pfx_recipe::SkyKind::Analytic
    );
    std::fs::write(dir.join("assets/sky/kloofendal_overcast_puresky.hdr"), b"").expect("hdr");
    let with = load(&text);
    let sky = with.sky.as_ref().expect("sky");
    assert_eq!(sky.kind, pfx_recipe::SkyKind::Hdr);
    assert_eq!(
        sky.path.as_deref(),
        Some("assets/sky/kloofendal_overcast_puresky.hdr")
    );
    assert_eq!(
        sky.source.as_deref(),
        Some("polyhaven:kloofendal_overcast_puresky, CC0")
    );
    assert_eq!(with.sun.as_ref().expect("sun").irradiance, 0.0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_lighting_preset_is_refused_by_name_when_it_cannot_apply() {
    assert!(
        refused("[lighting]\npreset = \"disco\"\n")
            .contains("one of: three-point, golden-hour, overcast, night")
    );
    assert!(refused("[lighting]\n").contains("lighting.preset"));
    assert!(
        refused("[lighting]\npreset = \"night\"\nbrightness = 2.0\n")
            .contains("lighting.brightness")
    );
}
