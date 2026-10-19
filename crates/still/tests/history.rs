use pfx_build::Registry;
use pfx_still::{Film, Noise, Reel, Seed, Take};
use std::path::PathBuf;

const FPS: f64 = 24.0;
const SIZE: [u32; 2] = [108, 192];

const BASE: &str = r#"recipe = 1
name = "history"

[timeline]
fps = 24
frames = 9

[materials.white]
base = [0.8, 0.8, 0.8]
roughness = 0.6

[materials.glow]
base = [0.2, 0.2, 0.2]
emission = [6.0, 4.0, 2.0]

[[object]]
name = "floor"
kind = "box"
size = [4.0, 0.1, 4.0]
material = "white"
at = [0.0, -0.05, 0.0]

[[object]]
name = "post"
kind = "box"
size = [0.3, 0.8, 0.3]
material = "white"
at = [-0.6, 0.4, -0.4]
"#;

const MOVER: &str = r#"
[[object]]
name = "mover"
kind = "box"
size = [0.4, 0.4, 0.4]
material = "white"
at = [0.0, 0.2, 0.0]
keys = { at = [[0.0, [-0.8, 0.2, 0.0]], [0.4, [0.8, 0.2, 0.3]]], rotate = [[0.0, [0.0, 0.0, 0.0]], [0.4, [0.0, 70.0, 0.0]]] }
"#;

const GLOW: &str = r#"
[[object]]
name = "glow"
kind = "box"
size = [0.2, 0.2, 0.2]
material = "glow"
at = [0.0, 0.6, 0.0]
keys = { at = [[0.0, [0.5, 0.6, -0.5]], [0.4, [-0.5, 0.7, 0.5]]] }
"#;

const LAMP: &str = r#"
[[light]]
name = "lamp"
position = [0.5, 1.2, 0.8]
intensity = 4.0
keys = { intensity = [[0.0, 2.0], [0.4, 9.0]], color = [[0.0, [1.0, 0.3, 0.2]], [0.4, [0.8, 0.9, 1.0]]] }
"#;

const SUN: &str = r#"
[sun]
model = "authored"
toward = [0.3, 0.6, 1.0]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 3.0
"#;

const DAYLIGHT: &str = r#"
[sun]
model = "daylight"
hour = 9.0
latitude = 45.0
heading = 180.0
keys = { hour = [[0.0, 9.0], [0.4, 17.0]] }
"#;

const SKY: &str = r#"
[sky]
kind = "hdr"
path = "sky.hdr"
source = "made by the test, no licence"
intensity = 1.0
"#;

const WIDE: &str = r#"
[[camera]]
name = "main"
position = [0.0, 1.0, 3.0]
target = [0.0, 0.3, 0.0]
fov_y_deg = 100.0
keys = { fov_y_deg = [[0.0, 100.0], [0.4, 30.0]] }
"#;

const HAZE: &str = r#"
[haze]
amount = 0.3
height = 0.0
falloff = 0.15
keys = { amount = [[0.0, 0.1], [0.4, 0.6]] }
"#;

const STILL_CAMERA: &str = r#"
[[camera]]
name = "main"
position = [0.0, 1.0, 3.0]
target = [0.0, 0.3, 0.0]
fov_y_deg = 40.0
"#;

const MOVING_CAMERA: &str = r#"
[[camera]]
name = "main"
position = [0.0, 1.0, 3.0]
target = [0.0, 0.3, 0.0]
fov_y_deg = 40.0
keys = { position = [[0.0, [-0.5, 1.0, 3.0]], [0.4, [0.6, 1.3, 2.6]]], fov_y_deg = [[0.0, 40.0], [0.4, 28.0]] }
"#;

const BLUR: &str = r#"
[motion_blur]
shutter = 0.5
"#;

fn recipe(name: &str, parts: &[&str]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("tmp/still-tests/history").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("history.toml");
    std::fs::write(&path, format!("{BASE}{}", parts.concat())).unwrap();
    let size = [1024u32, 512];
    let sky: Vec<[f32; 4]> = (0..size[0] * size[1])
        .map(|at| {
            let (x, y) = (at % size[0], at / size[0]);
            let spot = if (x / 16 + y / 16) % 7 == 0 { 4.0 } else { 0.0 };
            [
                0.3 + spot,
                0.4 + 0.3 * (y as f32 / 512.0),
                0.6 + spot * 0.5,
                1.0,
            ]
        })
        .collect();
    std::fs::write(dir.join("sky.hdr"), pfx_still::hdr(size, &sky).unwrap()).unwrap();
    path
}

fn take() -> Take {
    Take {
        size: SIZE,
        samples: 32,
        noise: Some(
            Noise {
                threshold: 0.01,
                min_samples: 16,
            }
            .floored(),
        ),
        seed: Seed::Locked,
    }
}

fn film(reel: &mut Reel, frame: u32) -> Film {
    let camera = reel.pose_moving(f64::from(frame) / FPS, "main").unwrap();
    reel.frame(&camera, take(), &mut |_| {}).unwrap().film
}

fn bits(film: &Film) -> (Vec<u32>, Vec<u32>) {
    let of = |pixels: &[[f32; 4]]| pixels.iter().flatten().map(|v| v.to_bits()).collect();
    (of(&film.color), of(film.denoised.as_deref().unwrap_or(&[])))
}

fn carried(name: &str, parts: &[&str]) -> Option<String> {
    let recipe = recipe(name, parts);
    let open = || Reel::open(&recipe, None, &[], None, &Registry::new()).unwrap();
    let mut running = open();
    let after: Vec<Film> = (0..6).map(|frame| film(&mut running, frame)).collect();
    let mut fresh = open();
    let mut found = Vec::new();
    for frame in 3..6 {
        let (raw, denoised) = bits(&film(&mut fresh, frame));
        let (was_raw, was_denoised) = bits(&after[frame as usize]);
        if raw != was_raw || denoised != was_denoised {
            found.push(format!(
                "frame {frame} ({})",
                if raw != was_raw {
                    "raw trace"
                } else {
                    "denoised only"
                }
            ));
        }
    }
    (!found.is_empty()).then(|| format!("{name}: {}", found.join(", ")))
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_frame_is_the_same_after_its_predecessors_or_first_on_a_fresh_reel() {
    let cases: [(&str, &[&str]); 9] = [
        ("camera", &[SUN, MOVING_CAMERA]),
        ("mover", &[SUN, STILL_CAMERA, MOVER]),
        ("glow", &[SUN, STILL_CAMERA, GLOW]),
        ("lamp", &[SUN, STILL_CAMERA, LAMP]),
        ("blur", &[SUN, STILL_CAMERA, MOVER, BLUR]),
        ("sky", &[SUN, SKY, WIDE]),
        ("haze", &[SUN, STILL_CAMERA, HAZE]),
        ("daylight", &[DAYLIGHT, STILL_CAMERA]),
        (
            "all",
            &[SUN, SKY, MOVING_CAMERA, MOVER, GLOW, LAMP, BLUR, HAZE],
        ),
    ];
    let carried: Vec<String> = cases
        .iter()
        .filter_map(|(name, parts)| carried(name, parts))
        .collect();
    assert!(
        carried.is_empty(),
        "state carried between frames:\n{}",
        carried.join("\n")
    );
}
