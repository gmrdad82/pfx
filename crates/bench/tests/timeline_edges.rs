use std::path::{Path, PathBuf};

use pfx_bench::shot::{Follow, Lens, Quality, Shot, play};
use pfx_load::scene::Scene;
use pfx_still::{Reel, Seed, Step, Take, looked_with};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 160;
const FPS: u32 = 60;
const FRAMES: u32 = 15;
const RESUME: u64 = 8;
const BAND: std::ops::Range<u32> = 74..86;

const RECIPE: &str = r#"recipe = 1
name = "edges"

[output]
glb = "edges.glb"
scene = "edges.scene.toml"

[timeline]
fps = 60
frames = 15
size = [320, 160]

BLUR

[sun]
model = "authored"
toward = [0.3, 0.6, 1.0]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 1.0

[[camera]]
name = "main"
position = [-0.5, 0.5, 4.0]
target = [-0.5, 0.5, 0.0]
fov_y_deg = 45.0
keys = { position = [[0.0, [-0.5, 0.5, 4.0]], [0.25, [0.5, 0.5, 4.0]]], target = [[0.0, [-0.5, 0.5, 0.0]], [0.25, [0.5, 0.5, 0.0]]] }

[materials.glow]
base = [0.0, 0.0, 0.0]
roughness = 1.0
emission = [1.0, 1.0, 1.0]

[[object]]
name = "rig"
kind = "empty"
at = [-2.5, 0.5, 0.0]
keys = { at = [[0.0, [-2.5, 0.5, 0.0]], [0.25, [2.5, 0.5, 0.0]]] }

[[object]]
name = "block"
kind = "box"
size = [0.4, 0.4, 0.4]
parent = "rig"
material = "glow"
"#;

fn recipe(blur: &str) -> String {
    RECIPE.replace("BLUR", blur)
}

fn blur(position: &str) -> String {
    format!("[motion_blur]\nshutter = 0.5\nmax_px = 64.0\nsamples = 24\nposition = \"{position}\"")
}

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("timeline-edges")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn exported(name: &str, text: &str) -> Scene {
    let root = folder(name);
    let recipe = pfx_recipe::parse(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(pfx_recipe::validate(&recipe), []);
    let built = pfx_build::build(&recipe, &root).unwrap();
    pfx_build::write(&built, &recipe, &root, Some(&root.join("scenes"))).unwrap();
    Scene::open(root.join("scenes/edges.scene.toml")).unwrap()
}

fn half(bits: u16) -> f32 {
    let sign = if bits >> 15 == 1 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

fn live(scene: &Scene, from: u64, frames: u64) -> Vec<Vec<f32>> {
    let mut shot = Vec::new();
    play(
        &Shot {
            scene,
            size: (WIDTH, HEIGHT),
            fps: pfx_core::rate::Rate::whole(FPS),
            from,
            frames,
            camera: Some(Follow {
                name: "camera.main".into(),
                lens: Lens {
                    fov: None,
                    near: None,
                },
            }),
            quality: Quality::Draft,
            seed: 7,
            shutter: None,
            fixed_exposure: true,
        },
        |frame| {
            shot.push(
                frame
                    .pixels
                    .chunks_exact(4)
                    .map(|texel| {
                        let [r, g, b] = [0, 1, 2].map(|channel| half(texel[channel]));
                        0.2126 * r + 0.7152 * g + 0.0722 * b
                    })
                    .collect(),
            );
            Ok(())
        },
    )
    .unwrap();
    shot
}

fn traced(name: &str, text: &str) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let path = folder(name).join("edges.toml");
    std::fs::write(&path, text).unwrap();
    let mut reel = Reel::open(&path, None, &[], None, &pfx_build::Registry::new()).unwrap();
    assert!(reel.blurs());
    let mut shoot = |moving: bool, time: f64| {
        let camera = match moving {
            true => reel.pose_moving(time, "main").unwrap(),
            false => {
                reel.pose(time).unwrap();
                reel.recipe.cameras[0].clone()
            }
        };
        let take = Take {
            size: [WIDTH, HEIGHT],
            samples: 16,
            noise: None,
            seed: Seed::Locked,
        };
        let rendered = reel.frame(&camera, take, &mut |_: Step| {}).unwrap();
        looked_with(&rendered.film, (&rendered.finish, &rendered.overlay), false)
            .unwrap()
            .chunks_exact(4)
            .map(|texel| {
                let [r, g, b] = [0, 1, 2].map(|c| (f32::from(texel[c]) / 255.0).powf(2.2));
                0.2126 * r + 0.7152 * g + 0.0722 * b
            })
            .collect::<Vec<f32>>()
    };
    (0..=FRAMES)
        .map(|frame| {
            let time = f64::from(frame) / f64::from(FPS);
            (shoot(false, time), shoot(true, time))
        })
        .unzip()
}

fn at(image: &[f32], x: u32, y: u32) -> f32 {
    image[(y * WIDTH + x) as usize]
}

fn centre(image: &[f32]) -> f32 {
    let columns: Vec<u32> = (0..WIDTH)
        .filter(|&x| {
            BAND.clone()
                .any(|y| at(image, x, y) > at(image, 0, y) + 0.2)
        })
        .collect();
    assert!(!columns.is_empty(), "the block is on screen");
    (columns[0] + columns[columns.len() - 1]) as f32 * 0.5
}

fn spread(sharp: &[f32], blurred: &[f32]) -> f32 {
    (0..WIDTH)
        .flat_map(|x| BAND.map(move |y| (x, y)))
        .map(|(x, y)| (at(blurred, x, y) - at(sharp, x, y)).abs())
        .sum::<f32>()
        / BAND.len() as f32
}

fn spreads(sharp: &[Vec<f32>], blurred: &[Vec<f32>]) -> Vec<f32> {
    sharp
        .iter()
        .zip(blurred)
        .map(|(sharp, blurred)| spread(sharp, blurred))
        .collect()
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn near(value: f32, to: f32, within: f32) -> bool {
    (value - to).abs() <= to * within
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn the_first_and_last_timeline_frames_streak_like_their_neighbours_live_and_traced() {
    let last = FRAMES as usize;
    let sharp = live(&exported("sharp", &recipe("")), 0, u64::from(FRAMES) + 1);
    let centres: Vec<f32> = sharp.iter().map(|image| centre(image)).collect();
    println!("live sharp block centres, frames 0 to {FRAMES}: {centres:?}");
    let steps: Vec<f32> = centres.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let step = mean(&steps);
    assert!(step > 8.0, "the block crosses the frame: {centres:?}");
    assert!(
        steps.iter().all(|&s| near(s, step, 0.2)),
        "the block moves at constant speed to the last frame, never back to the start: {centres:?}"
    );
    for position in ["end", "centre", "start"] {
        let scene = exported(position, &recipe(&blur(position)));
        let blurred = live(&scene, 0, u64::from(FRAMES) + 1);
        let measured = spreads(&sharp, &blurred);
        println!("live {position}: edge spread, frames 0 to {FRAMES}: {measured:?}");
        let inner = mean(&measured[1..last]);
        assert!(
            inner > 0.5,
            "live {position}: the block streaks: {measured:?}"
        );
        for (frame, &value) in measured.iter().enumerate().take(last).skip(1) {
            assert!(
                near(value, inner, 0.25),
                "live {position}: frame {frame} is out of step: {measured:?}"
            );
        }
        assert!(
            near(measured[last], measured[last - 1], 0.1),
            "live {position}: the last frame streaks with the one before it: {measured:?}"
        );
        assert!(
            measured[0] <= measured[1] * 1.1,
            "live {position}: the first frame holds its pose and never smears: {measured:?}"
        );
        let resumed = live(&scene, RESUME, 2);
        let resumed = spread(&sharp[RESUME as usize], &resumed[0]);
        println!("live {position}: a run resumed at frame {RESUME}: {resumed}");
        assert!(
            near(resumed, measured[RESUME as usize], 0.1),
            "live {position}: a resumed run's first frame streaks as in the whole run: {resumed} against {measured:?}"
        );
    }
    for position in ["end", "centre", "start"] {
        let (sharp, blurred) = traced(&format!("traced-{position}"), &recipe(&blur(position)));
        let measured = spreads(&sharp, &blurred);
        println!("traced {position}: edge spread, frames 0 to {FRAMES}: {measured:?}");
        let inner = mean(&measured[1..last]);
        assert!(
            inner > 0.5,
            "traced {position}: the block streaks: {measured:?}"
        );
        for (frame, &value) in measured.iter().enumerate().take(last).skip(1) {
            assert!(
                near(value, inner, 0.25),
                "traced {position}: frame {frame} is out of step: {measured:?}"
            );
        }
        let (first, end) = (measured[0] / inner, measured[last] / inner);
        let (want_first, want_end) = match position {
            "end" => (0.0, 1.0),
            "centre" => (0.5, 0.5),
            _ => (1.0, 0.0),
        };
        assert!(
            (first - want_first).abs() <= 0.15 && (end - want_end).abs() <= 0.15,
            "traced {position}: the shutter holds the timeline's ends, first {first:.2} of the inner frames' streak, last {end:.2}: {measured:?}"
        );
    }
}
