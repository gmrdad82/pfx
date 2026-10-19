use pfx_build::Registry;
use pfx_still::{Noise, Reel, Seed, Take, finished};
use std::path::{Path, PathBuf};
use std::time::Instant;

const RECIPE: &str = r#"recipe = 1
name = "pan"

[sun]
model = "authored"
toward = [0.5, 0.7, 0.4]
radius_deg = 0.5
color = [1.0, 0.95, 0.9]
irradiance = 3.0

[sky]
kind = "analytic"
intensity = 0.6

[[camera]]
name = "main"
position = [-0.4, 1.1, 3.2]
target = [-0.4, 0.3, 0.0]
fov_y_deg = 40.0

[camera.path]
kind = "keys"

[[camera.path.keys]]
t = 1.0
position = [0.4, 1.1, 3.2]
target = [0.4, 0.3, 0.0]

[materials.tiles]
base = [1.0, 1.0, 1.0]
roughness = 0.6
base_map = "tiles.png"
uv_scale = [3.0, 3.0]

[materials.wall]
base = [0.7, 0.7, 0.68]
roughness = 0.9

[materials.lacquer]
family = "lacquer"
base = [0.73, 0.06, 0.0]
roughness = 0.45
clearcoat = 1.0
clearcoat_roughness = 0.07

[materials.metal]
base = [0.9, 0.9, 0.92]
metalness = 1.0
roughness = 0.25

[[object]]
name = "floor"
kind = "plane"
size = [6.0, 4.0]
material = "tiles"

[[object]]
name = "wall"
kind = "box"
at = [0.0, 1.0, -1.6]
size = [6.0, 2.0, 0.1]
material = "wall"

[[object]]
name = "cube"
kind = "box"
at = [-0.5, 0.3, 0.0]
size = [0.6, 0.6, 0.6]
material = "lacquer"

[[object]]
name = "post"
kind = "box"
at = [0.7, 0.4, 0.3]
size = [0.25, 0.8, 0.25]
material = "metal"
"#;

struct Setup {
    name: &'static str,
    samples: u32,
    noise: Option<Noise>,
    animated: bool,
}

struct Clip {
    frames: Vec<Vec<[f32; 4]>>,
    mean_samples: f64,
    seconds: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |flag: &str, default: u32| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map_or(default, |v| v.parse().expect(flag))
    };
    let frames = number("--frames", 10);
    let spp = number("--spp", 64);
    let reference = number("--reference", 1024);
    let width = number("--width", 320);
    let size = [width, width * 5 / 8];
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tmp/flicker")).to_path_buf();
    let recipe = write_scene(&dir);
    let threshold = args
        .iter()
        .position(|a| a == "--threshold")
        .and_then(|i| args.get(i + 1))
        .map_or(0.01, |v| v.parse().expect("--threshold"));
    let noise = Noise {
        threshold,
        min_samples: 16,
    };
    let setups = [
        Setup {
            name: "reference",
            samples: reference,
            noise: None,
            animated: false,
        },
        Setup {
            name: "locked, fixed",
            samples: spp,
            noise: None,
            animated: false,
        },
        Setup {
            name: "animated, fixed",
            samples: spp,
            noise: None,
            animated: true,
        },
        Setup {
            name: "locked, adaptive",
            samples: spp * 4,
            noise: Some(noise),
            animated: false,
        },
        Setup {
            name: "animated, adaptive",
            samples: spp * 4,
            noise: Some(noise),
            animated: true,
        },
        Setup {
            name: "locked, adaptive, floored",
            samples: spp * 4,
            noise: Some(noise.floored()),
            animated: false,
        },
        Setup {
            name: "animated, adaptive, floored",
            samples: spp * 4,
            noise: Some(noise.floored()),
            animated: true,
        },
    ];
    let mut clips = Vec::new();
    for setup in &setups {
        let clip = render(&recipe, size, frames, setup);
        eprintln!(
            "{}: {:.1} spp mean, {:.1} s",
            setup.name, clip.mean_samples, clip.seconds
        );
        clips.push(clip);
    }
    let truth = &clips[0];
    println!(
        "{} frames of {}x{}, a pan, threshold {threshold}; flicker in 8-bit levels of the denoised, finished frames",
        frames, size[0], size[1]
    );
    println!("| setup | mean spp | s a frame | static MAD | static share | temporal error |");
    println!("|---|---|---|---|---|---|");
    for (setup, clip) in setups.iter().zip(&clips).skip(1) {
        let (mad, share) = static_mad(&truth.frames, &clip.frames);
        println!(
            "| {} | {:.1} | {:.2} | {:.3} | {:.0}% | {:.3} |",
            setup.name,
            clip.mean_samples,
            clip.seconds / f64::from(frames),
            mad,
            share * 100.0,
            temporal_error(&truth.frames, &clip.frames)
        );
    }
}

fn write_scene(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let side = 64u32;
    let mut texels = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let tile = ((x / 16) + (y / 16)) % 2 == 0;
            let grain = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) % 41;
            let level = if tile { 200 } else { 70 } + grain as u8;
            texels.extend([level, level - 10, level - 25, 255]);
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, side, side);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&texels).unwrap();
    writer.finish().unwrap();
    std::fs::write(dir.join("tiles.png"), out).unwrap();
    let recipe = dir.join("pan.toml");
    std::fs::write(&recipe, RECIPE).unwrap();
    recipe
}

fn render(recipe: &Path, size: [u32; 2], frames: u32, setup: &Setup) -> Clip {
    let seed = (setup.name == "reference").then_some(0x5eed_f11c);
    let mut reel = Reel::open(recipe, None, &[], seed, &Registry::new()).unwrap();
    let camera = pfx_recipe_scene::camera(&reel.recipe, "main")
        .unwrap()
        .clone();
    let path = camera.path.clone().unwrap();
    let start = Instant::now();
    let mut out = Vec::new();
    let mut samples = 0.0;
    for frame in 0..frames {
        let take = Take {
            size,
            samples: setup.samples,
            noise: setup.noise,
            seed: if setup.animated {
                Seed::Animated(frame)
            } else {
                Seed::Locked
            },
        };
        let posed = camera.at(path.time(frame, frames));
        let rendered = reel.frame(&posed, take, &mut |_| {}).unwrap();
        samples += rendered.mean_samples;
        out.push(finished(&rendered.film, (&rendered.finish, &rendered.overlay), true).unwrap());
    }
    Clip {
        frames: out,
        mean_samples: samples / f64::from(frames),
        seconds: start.elapsed().as_secs_f64(),
    }
}

fn level(pixel: [f32; 4], channel: usize) -> f64 {
    f64::from(pixel[channel].clamp(0.0, 1.0)) * 255.0
}

fn static_mad(truth: &[Vec<[f32; 4]>], clip: &[Vec<[f32; 4]>]) -> (f64, f64) {
    let (mut sum, mut count, mut total) = (0.0, 0u64, 0u64);
    for t in 1..truth.len() {
        for i in 0..truth[t].len() {
            total += 1;
            let still =
                (0..3).all(|c| (level(truth[t][i], c) - level(truth[t - 1][i], c)).abs() < 1.0);
            if !still {
                continue;
            }
            count += 1;
            sum += (0..3)
                .map(|c| (level(clip[t][i], c) - level(clip[t - 1][i], c)).abs())
                .sum::<f64>()
                / 3.0;
        }
    }
    (
        sum / count.max(1) as f64,
        count as f64 / total.max(1) as f64,
    )
}

fn temporal_error(truth: &[Vec<[f32; 4]>], clip: &[Vec<[f32; 4]>]) -> f64 {
    let (mut sum, mut count) = (0.0, 0u64);
    for t in 1..truth.len() {
        for i in 0..truth[t].len() {
            for c in 0..3 {
                let want = level(truth[t][i], c) - level(truth[t - 1][i], c);
                let got = level(clip[t][i], c) - level(clip[t - 1][i], c);
                sum += (got - want).abs();
                count += 1;
            }
        }
    }
    sum / count.max(1) as f64
}
