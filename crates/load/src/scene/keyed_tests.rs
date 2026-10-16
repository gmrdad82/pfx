use std::path::{Path, PathBuf};

use super::*;

fn folder(name: &str) -> PathBuf {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/scene-keyed")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn open(name: &str, text: &str) -> Scene {
    let root = folder(name);
    let path = root.join("keyed.scene.toml");
    std::fs::write(&path, text).unwrap();
    let scene = Scene::open(&path).unwrap_or_else(|error| panic!("{error}"));
    let _ = std::fs::remove_dir_all(&root);
    scene
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

const KEYED: &str = "format = 1

[[light]]
name = \"lamp\"
kind = \"spot\"
position = [0.0, 2.0, 0.0]
intensity = 2.0
keys = { intensity = [[1.0, 2.0], [3.0, 8.0]], color = [[0.0, [1.0, 0.2, 0.1]], [2.0, [0.2, 0.4, 1.0]]], cone = [[0.0, [10.0, 20.0]], [2.0, [30.0, 40.0]]] }

[camera]
at = [0.0, 0.0, 5.0]
fov = 40.0
fstop = 4.0
keys = { fov = [[0.0, 40.0], [2.0, 20.0]], focus = [[0.0, 2.0], [2.0, 6.0]] }

[sun]
model = \"daylight\"
hour = 9.0
keys = { hour = [[0.0, 9.0], [2.0, 15.0]] }

[haze]
lo = [-1.0, 0.0, -1.0]
hi = [1.0, 2.0, 1.0]
amount = 0.1
keys = { amount = [[0.0, 0.0], [2.0, 0.6]] }

[finish]
exposure = 2.0
keys = { exposure = [[0.0, 2.0], [2.0, 4.0]] }
";

#[test]
fn keyed_values_sample_linearly_hold_past_the_ends_and_override_the_static_ones() {
    let scene = open("values", KEYED);
    let keys = &scene.keys;
    let mut light = scene.lights["lamp"];
    keys.light("lamp", 2.0).unwrap().tune(
        &mut light.intensity,
        &mut light.color,
        &mut light.range,
        &mut light.spot,
    );
    assert!(close(light.intensity, 5.0));
    assert_eq!(light.color, [0.2, 0.4, 1.0]);
    let spot = light.spot.unwrap();
    assert_eq!((spot.inner, spot.outer), (30.0, 40.0));
    let held = keys.light("lamp", 0.0).unwrap();
    assert_eq!(held.intensity, Some(2.0));
    assert_eq!(keys.light("lamp", 9.0).unwrap().intensity, Some(8.0));
    assert_eq!(held.range, None);

    let rest = scene.camera_or_default();
    let camera = keys.camera(&rest, 1.0);
    assert_eq!(camera.projection, Projection::Perspective { fov: 30.0 });
    let depth = camera.depth_of_field.unwrap();
    assert!(close(depth.distance, 4.0));
    assert!(close(
        depth.focal,
        0.024 / (2.0 * 15.0_f32.to_radians().tan())
    ));
    assert_eq!(rest.projection, Projection::Perspective { fov: 40.0 });

    let noon = keys.sun(1.0).unwrap();
    let path = folder("noon").join("keyed.scene.toml");
    std::fs::write(&path, KEYED).unwrap();
    let opened = Scene::open_at(&path, 12.0).unwrap().sun.unwrap();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    assert!(close(noon.hour, 12.0));
    assert_eq!(noon.direction, opened.direction);
    assert_eq!(noon.intensity, opened.intensity);
    assert!(close(scene.sun.unwrap().hour, 9.0));

    let haze = keys.haze(1.0).unwrap();
    assert!(close(haze.fog, 0.3 / 6.0));
    assert!(close(scene.haze.unwrap().fog, 0.1 / 6.0));

    let finish = keys.finish(1.0).unwrap();
    assert!(close(finish.gain, 1.5));
    assert_eq!(finish.lut_amount, None);
}

#[test]
fn sky_intensity_scales_an_analytic_sky_and_its_keys_rebuild_it() {
    let sky = |extra: &str| {
        open(
            "sky",
            &format!(
                "format = 1\n\n[sun]\nmodel = \"daylight\"\nhour = 10.0\n\n[sky]\nkind = \"analytic\"\n{extra}"
            ),
        )
    };
    let ambient = |scene: &Scene| match scene.environment() {
        Environment::Analytic(sky) => sky.ambient,
        Environment::Hdr(_) => panic!("an analytic sky"),
    };
    let plain = sky("");
    let doubled = sky("intensity = 2.0\n");
    assert!(close(ambient(&doubled), 2.0 * ambient(&plain)));
    assert!(plain.keys.sky.is_none());
    assert_eq!(plain.sky.as_ref().unwrap().look, SkyLook::default());

    let keyed = sky(
        "intensity = 2.0\nbackdrop = 1.5\nhorizon_color = [1.0, 0.5, 0.2]\nkeys = { intensity = [[0.0, 1.0], [2.0, 3.0]], horizon_amount = [[0.0, 0.0], [2.0, 1.0]] }\n",
    );
    match keyed.keys.environment(1.0).unwrap() {
        Environment::Analytic(sky) => assert!(close(sky.ambient, ambient(&doubled))),
        Environment::Hdr(_) => panic!("an analytic sky"),
    }
    assert_ne!(keyed.keys.sky_stamp(0.0), keyed.keys.sky_stamp(1.0));
    assert_eq!(keyed.keys.sky_stamp(1.0), keyed.keys.sky_stamp(1.0));
    let look = keyed.keys.look(1.0).unwrap();
    assert_eq!(look.backdrop, 1.5);
    assert_eq!(look.horizon_color, [1.0, 0.5, 0.2]);
    assert!(close(look.horizon_amount, 0.5));
    assert_eq!(look.horizon_width, 10.0);

    let untinted = sky("keys = { horizon_amount = [[0.0, 1.0]] }\n");
    assert_eq!(untinted.keys.look(0.0).unwrap().horizon_amount, 0.0);
}
