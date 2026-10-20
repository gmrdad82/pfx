use pfx_build::Registry;
use pfx_recipe::load;
use pfx_recipe_scene::pace::{turn, turn_reporting};
use pfx_recipe_scene::{Output, headless, shoot_posed_as};
use pfx_still::{Staging, View, eyepiece, trace};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const SIZE: [u32; 2] = [384, 384];
const HALF: f64 = 3.0;
const SAMPLES: u32 = 512;
const SEED: u32 = 3;
const EXPOSURE: f32 = 0.125;

const BOARD: &str = r#"recipe = 1
name = "parity"

[[camera]]
name = "main"
kind = "orthographic"
position = [0.0, 5.0, 0.0]
target = [0.0, 0.0, 0.0]
up = [0.0, 0.0, -1.0]
ortho_height = 6.0

[look]
view = "standard"
exposure = 0.125

[materials.grey]
base = [0.5, 0.5, 0.5]
roughness = 1.0
specular = 0.0

[materials.white]
base = [0.8, 0.8, 0.8]
roughness = 1.0
specular = 0.0

[materials.mid]
base = [0.18, 0.18, 0.18]
roughness = 1.0
specular = 0.0

[materials.red]
base = [0.7, 0.25, 0.1]
roughness = 1.0
specular = 0.0

[materials.gloss]
base = [0.3, 0.3, 0.32]
roughness = 0.35
specular = 0.5

[[object]]
name = "ground"
kind = "plane"
size = [6.0, 6.0]
material = "grey"

[[object]]
name = "white"
kind = "box"
size = [0.9, 0.02, 0.9]
at = [1.4, 0.01, -1.4]
material = "white"

[[object]]
name = "mid"
kind = "box"
size = [0.9, 0.02, 0.9]
at = [1.4, 0.01, 1.4]
material = "mid"

[[object]]
name = "red"
kind = "box"
size = [0.9, 0.02, 0.9]
at = [-1.4, 0.01, 1.4]
material = "red"

[[object]]
name = "ball"
kind = "sphere"
radius = 0.6
at = [-1.6, 0.6, -1.6]
material = "grey"

[[object]]
name = "gloss"
kind = "sphere"
radius = 0.45
at = [0.0, 0.45, 2.1]
material = "gloss"
"#;

const BLACK_SKY: &str = r#"
[sky]
kind = "room"

[sky.room]
width = 64
floor = [0.0, 0.0, 0.0]
wall = [0.0, 0.0, 0.0]
ceiling = [0.0, 0.0, 0.0]
"#;

const SUN: &str = r#"
[sun]
model = "authored"
toward = [0.35, 1.0, 0.25]
irradiance = 2.0
"#;

const DARK_SUN: &str = r#"
[sun]
model = "authored"
toward = [0.35, 1.0, 0.25]
irradiance = 0.0
"#;

const ANALYTIC_SKY: &str = r#"
[sky]
kind = "analytic"
"#;

const HDR_SKY: &str = r#"
[sky]
kind = "hdr"
path = "window.hdr"
"#;

const ROOM_SKY: &str = r#"
[sky]
kind = "room"

[sky.room]
width = 256

[[sky.room.lights]]
az = 40.0
el = 35.0
power = 2.0
"#;

const LAMP: &str = r#"
[[light]]
name = "lamp"
position = [0.2, 1.6, 0.1]
color = [1.0, 0.95, 0.85]
intensity = 5.0
radius = 0.05
shadow = true
"#;

struct Case {
    name: &'static str,
    parts: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        name: "sun, black sky",
        parts: &[SUN, BLACK_SKY],
    },
    Case {
        name: "analytic sky",
        parts: &[DARK_SUN, ANALYTIC_SKY],
    },
    Case {
        name: "hdr sky",
        parts: &[DARK_SUN, HDR_SKY],
    },
    Case {
        name: "room sky",
        parts: &[DARK_SUN, ROOM_SKY],
    },
    Case {
        name: "point light",
        parts: &[DARK_SUN, BLACK_SKY, LAMP],
    },
    Case {
        name: "street defaults",
        parts: &[],
    },
];

const CLOSE: f64 = 0.04;
const BOUNCED: f64 = 0.07;

const PATCHES: &[(&str, [f64; 2], f64, f64)] = &[
    ("ground", [2.5, -2.5], 0.3, CLOSE),
    ("white", [1.4, -1.4], 0.3, CLOSE),
    ("mid", [1.4, 1.4], 0.3, CLOSE),
    ("red", [-1.4, 1.4], 0.3, CLOSE),
    ("ball", [-1.6, -1.6], 0.25, BOUNCED),
    ("gloss", [0.0, 2.1], 0.2, CLOSE),
];

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn board(name: &str, parts: &[&str]) -> PathBuf {
    let dir = root()
        .join("tmp/parity")
        .join(name.replace([' ', ','], "-"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        root().join("tests/fixtures/sky/window.hdr"),
        dir.join("window.hdr"),
    )
    .unwrap();
    let file = dir.join("scene.toml");
    std::fs::write(&file, format!("{BOARD}{}", parts.concat())).unwrap();
    file
}

fn traced(path: &Path) -> Vec<[f32; 3]> {
    let recipe = load(path).unwrap().recipe;
    let built = pfx_build::build_with(&recipe, path.parent().unwrap(), &Registry::new()).unwrap();
    let recipe = built.recipe(&recipe);
    let staging = Staging::of(&recipe, &built.parts).unwrap();
    let lens = eyepiece(
        pfx_recipe_scene::camera(&recipe, "main").unwrap(),
        View::Ortho,
        SIZE,
    )
    .unwrap();
    let stage = staging.stage(lens).unwrap();
    let (film, _) = trace(
        &headless().unwrap(),
        &stage,
        SIZE,
        SAMPLES,
        SEED,
        &mut |_| (),
        &mut turn_reporting,
    )
    .unwrap();
    film.color.iter().map(|p| [p[0], p[1], p[2]]).collect()
}

fn live(path: &Path) -> Vec<[f32; 3]> {
    let (recipe, built) = pfx_recipe_scene::load(path, None, &[], &Registry::new()).unwrap();
    let gpu = headless().unwrap();
    let output = Output {
        wide: true,
        dither: None,
    };
    let bytes = shoot_posed_as(&gpu, (&recipe, &built), "main", SIZE, None, output).unwrap();
    bytes
        .chunks_exact(8)
        .map(|p| {
            std::array::from_fn(|c| {
                let level = f32::from(u16::from_be_bytes([p[c * 2], p[c * 2 + 1]])) / 65535.0;
                pfx_materials::linear_channel(level) / EXPOSURE
            })
        })
        .collect()
}

fn save(path: &Path, image: &[[f32; 3]]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIZE[0], SIZE[1]);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = image
        .iter()
        .flat_map(|p| {
            p.map(|v| {
                (pfx_post::color::encode_channel((v * EXPOSURE).clamp(0.0, 1.0)) * 255.0 + 0.5)
                    as u8
            })
        })
        .collect();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&bytes)
        .unwrap();
}

fn mean(image: &[[f32; 3]], at: [f64; 2], reach: f64) -> [f64; 3] {
    let pixel = |v: f64| (v + HALF) / (2.0 * HALF) * f64::from(SIZE[0]);
    let (cx, cy, r) = (
        pixel(at[0]),
        pixel(at[1]),
        reach / (2.0 * HALF) * f64::from(SIZE[0]),
    );
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for y in (cy - r) as usize..=(cy + r) as usize {
        for x in (cx - r) as usize..=(cx + r) as usize {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let p = image[y * SIZE[0] as usize + x];
            for c in 0..3 {
                sum[c] += f64::from(p[c]);
            }
            count += 1.0;
        }
    }
    sum.map(|v| v / count)
}

fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

struct Row {
    patch: &'static str,
    tolerance: f64,
    live: [f64; 3],
    traced: [f64; 3],
}

impl Row {
    fn ratio(&self) -> f64 {
        luma(self.live) / luma(self.traced).max(1e-6)
    }

    fn worst_channel(&self) -> f64 {
        (0..3)
            .filter(|&c| self.traced[c] > 1e-3)
            .map(|c| (self.live[c] / self.traced[c] - 1.0).abs())
            .fold(0.0, f64::max)
    }
}

fn compare(case: &Case) -> Vec<Row> {
    let path = board(case.name, case.parts);
    let traced = traced(&path);
    turn();
    let live = live(&path);
    turn();
    save(&path.with_file_name("traced.png"), &traced);
    save(&path.with_file_name("live.png"), &live);
    PATCHES
        .iter()
        .map(|&(patch, at, reach, tolerance)| Row {
            patch,
            tolerance,
            live: mean(&live, at, reach),
            traced: mean(&traced, at, reach),
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn live_and_traced_lighting_agree_per_patch() {
    let _gpu = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut off = Vec::new();
    for case in CASES {
        let rows = compare(case);
        for row in &rows {
            println!(
                "{:16} {:7} live {:.4?} traced {:.4?} live/traced {:.3} worst channel {:.3}",
                case.name,
                row.patch,
                row.live,
                row.traced,
                row.ratio(),
                row.worst_channel()
            );
        }
        off.extend(
            rows.iter()
                .filter(|row| row.worst_channel() > row.tolerance)
                .map(|row| format!("{} {}: {:.3}", case.name, row.patch, row.worst_channel())),
        );
    }
    assert!(off.is_empty(), "{off:?}");
}
