use pfx_gpu::Gpu;
use pfx_recipe::parse;
use pfx_recipe_scene::pace::turn_reporting;
use pfx_still::{Film, Staging, View, eyepiece, trace};
use std::path::{Path, PathBuf};

const PLAN: &str = r#"recipe = 1
name = "plan"

[sun]
model = "authored"
toward = [0.0, 1.0, 0.0]
radius_deg = 0.5
color = [1.0, 1.0, 1.0]
irradiance = 3.0

[sky]
kind = "analytic"
intensity = 0.0

[[camera]]
name = "plan"
kind = "perspective"
position = [0.0, 5.0, 0.0]
target = [0.0, 0.0, 0.0]
up = [0.0, 0.0, -1.0]
fov_y_deg = 30.0
ortho_height = 4.0

[[object]]
name = "ground"
kind = "plane"
size = [4.0, 4.0]
material = "ground"
"#;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tmp")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn png(dir: &Path, name: &str, texel: impl Fn(u32) -> [u8; 4]) {
    let mut texels = Vec::new();
    for _ in 0..16 {
        for x in 0..16 {
            texels.extend(texel(x));
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 16, 16);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&texels).unwrap();
    writer.finish().unwrap();
    std::fs::write(dir.join(name), out).unwrap();
}

fn fixtures(dir: &Path) {
    png(dir, "stripes.png", |x| {
        if x < 8 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    });
    png(dir, "tilted.png", |_| [218, 128, 218, 255]);
    png(dir, "polished.png", |_| [0, 25, 255, 255]);
}

fn traced(gpu: &Gpu, dir: &Path, material: &str) -> Film {
    let recipe = parse(&format!("{PLAN}\n[materials.ground]\n{material}")).unwrap();
    let built = pfx_build::build(&recipe, dir).unwrap();
    let staging = Staging::of(&recipe, &built.parts).unwrap();
    let camera = pfx_recipe_scene::camera(&recipe, "plan").unwrap();
    let size = [64, 64];
    let staged = staging
        .stage(eyepiece(camera, View::Ortho, size).unwrap())
        .unwrap();
    let (film, _) = trace(gpu, &staged, size, 8, 1, &mut |_| (), &mut turn_reporting).unwrap();
    pfx_recipe_scene::pace::turn();
    film
}

fn luminance(pixel: [f32; 4]) -> f32 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}

fn column(film: &Film, metres: f32) -> f32 {
    let [width, height] = film.size;
    let x = ((metres / 4.0 * width as f32) as u32).min(width - 1);
    let rows = height / 4..height * 3 / 4;
    let count = rows.len() as f32;
    rows.map(|y| luminance(film.color[(y * width + x) as usize]))
        .sum::<f32>()
        / count
}

fn mean(film: &Film) -> f32 {
    film.color.iter().map(|p| luminance(*p)).sum::<f32>() / film.color.len() as f32
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_traced_recipe_draws_its_uv_transform_and_its_normal_and_roughness_maps() {
    let dir = scratch("trace-maps");
    fixtures(&dir);
    let gpu = pfx_recipe_scene::headless().unwrap();
    let ground = "base = [0.8, 0.8, 0.8]\nroughness = 1.0\nmetalness = 0.0\n";
    let every_metre = traced(
        &gpu,
        &dir,
        &format!("{ground}base_map = \"stripes.png\"\nuv_scale = [4.0, 4.0]\n"),
    );
    let every_two = traced(
        &gpu,
        &dir,
        &format!("{ground}base_map = \"stripes.png\"\nuv_scale = [2.0, 2.0]\n"),
    );
    let at = |film: &Film| [0.25, 0.75, 1.25].map(|m| column(film, m));
    let one = at(&every_metre);
    let two = at(&every_two);
    println!("stripes at 0.25, 0.75 and 1.25 m: every metre {one:?}, every 2 m {two:?}");
    assert!(one[0] > 0.2 && one[1] < 0.02 && one[2] > 0.2, "{one:?}");
    assert!(two[0] > 0.2 && two[1] > 0.2 && two[2] < 0.02, "{two:?}");
    let plain = mean(&traced(&gpu, &dir, ground));
    let tilted = mean(&traced(
        &gpu,
        &dir,
        &format!("{ground}normal_map = \"tilted.png\"\n"),
    ));
    let metal = "base = [0.8, 0.8, 0.8]\nroughness = 1.0\nmetalness = 1.0\n";
    let rough = mean(&traced(&gpu, &dir, metal));
    let polished = mean(&traced(
        &gpu,
        &dir,
        &format!("{metal}roughness_map = \"polished.png\"\n"),
    ));
    println!(
        "plain {plain:.4}, under a 45 degree normal map {tilted:.4}; rough metal {rough:.4}, polished by its map {polished:.4}"
    );
    assert!(tilted < plain * 0.9, "{tilted} {plain}");
    assert!(polished > rough * 2.0, "{polished} {rough}");
}
