use pfx_build::Registry;
use pfx_build::gltf_io;
use pfx_recipe_scene::pace::turn;
use pfx_recipe_scene::{still, write_png};
use std::path::{Path, PathBuf};

const SIZE: [u32; 2] = [320, 320];

const HEAD: &str = r#"recipe = 1
name = "parts"

[[camera]]
name = "main"
position = [1.6, 1.2, 3.4]
target = [0.0, 0.1, 0.4]
fov_y_deg = 35.0

[sun]
model = "authored"
toward = [0.3, 0.8, 0.5]
irradiance = 1.0
"#;

const PARTS: &str = r#"
[materials.lamp]
family = "emissive"
base = [0.3, 0.02, 0.02]
roughness = 0.4
emission = [6.0, 0.2, 0.1]

[materials.lens]
family = "glass"
base = [0.95, 0.85, 0.85]
roughness = 0.03
transmission = 0.92
ior = 1.49
thickness = 0.006
absorption = 3.5

[materials.body]
family = "lacquer"
base = [0.05, 0.2, 0.6]
roughness = 0.35
clearcoat = 1.0
clearcoat_roughness = 0.04

[materials.panel]
base = [0.8, 0.7, 0.6]
roughness = 0.6
base_map = "panel.png"
uv_scale = [2.0, 2.0]

[[object]]
name = "body"
kind = "box"
size = [2.0, 0.8, 1.0]
material = "body"

[[object]]
name = "lamp"
kind = "box"
size = [0.4, 0.15, 0.05]
at = [0.6, 0.1, 0.55]
material = "lamp"

[[object]]
name = "lens"
kind = "box"
size = [0.5, 0.2, 0.02]
at = [0.6, 0.1, 0.6]
material = "lens"

[[object]]
name = "panel"
kind = "box"
size = [0.6, 0.4, 0.02]
at = [-0.5, 0.0, 0.52]
material = "panel"
"#;

fn dir() -> PathBuf {
    let dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tmp/export-still"
    ))
    .to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn panel() -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 8, 8);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let data: Vec<u8> = (0..64u32)
        .flat_map(|i| {
            let (x, y) = (i % 8, i / 8);
            if (x + y) % 2 == 0 {
                [220, 200, 60, 255]
            } else {
                [40, 90, 160, 255]
            }
        })
        .collect();
    writer.write_image_data(&data).unwrap();
    drop(writer);
    out
}

fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    let total: f64 = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(p, q)| {
            (0..3)
                .map(|c| (f64::from(p[c]) - f64::from(q[c])).abs())
                .sum::<f64>()
        })
        .sum();
    total / (a.len() / 4 * 3) as f64
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn an_exported_recipe_draws_the_same_live_still_after_its_round_trip() {
    let dir = dir();
    std::fs::write(dir.join("panel.png"), panel()).unwrap();
    let source = dir.join("parts.toml");
    std::fs::write(&source, format!("{HEAD}{PARTS}")).unwrap();
    let glb = gltf_io::export_glb(&source, &Registry::new(), None).unwrap();
    std::fs::write(dir.join("parts.glb"), &glb).unwrap();
    let again = dir.join("again.toml");
    std::fs::write(
        &again,
        format!("{HEAD}\n[[object]]\nname = \"parts\"\nkind = \"mesh\"\npath = \"parts.glb\"\n"),
    )
    .unwrap();
    let before = still(&source, "main", SIZE).unwrap();
    write_png(&dir.join("before.png"), SIZE, &before).unwrap();
    turn();
    let after = still(&again, "main", SIZE).unwrap();
    write_png(&dir.join("after.png"), SIZE, &after).unwrap();
    turn();
    let off = mean_difference(&before, &after);
    println!("export round trip: the live stills differ by {off:.3} levels on average");
    assert!(off < 1.0, "the live stills differ by {off:.3} levels");
}
