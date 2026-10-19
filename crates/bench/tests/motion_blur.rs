use std::path::{Path, PathBuf};

use pfx_bench::shot::{Follow, Lens, Quality, Shot, play};
use pfx_load::fixture::glb;
use pfx_load::scene::Scene;
use serde_json::json;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 192;
const FPS: u32 = 30;
const FROM: u64 = 9;
const SEED: u64 = 7;
const SPEED: f32 = 12.0;

const MATERIALS: &str = "[materials.glow]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [1.0, 1.0, 1.0]

[materials.lime]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [0.0, 1.0, 0.0]

[materials.ember]
base = [0.0, 0.0, 0.0]
roughness = 1.0
specular = 0.0
emission = [1.0, 0.0, 0.0]
";

fn floats(bin: &mut Vec<u8>, values: &[f32]) -> (usize, usize) {
    let offset = bin.len();
    for value in values {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    (offset, values.len() * 4)
}

fn rig_glb() -> Vec<u8> {
    let corners: [[f32; 3]; 8] = [
        [-0.25, -0.25, -0.25],
        [0.25, -0.25, -0.25],
        [0.25, 0.25, -0.25],
        [-0.25, 0.25, -0.25],
        [-0.25, -0.25, 0.25],
        [0.25, -0.25, 0.25],
        [0.25, 0.25, 0.25],
        [-0.25, 0.25, 0.25],
    ];
    let faces: [([usize; 4], [f32; 3]); 6] = [
        ([4, 5, 6, 7], [0.0, 0.0, 1.0]),
        ([1, 0, 3, 2], [0.0, 0.0, -1.0]),
        ([5, 1, 2, 6], [1.0, 0.0, 0.0]),
        ([0, 4, 7, 3], [-1.0, 0.0, 0.0]),
        ([7, 6, 2, 3], [0.0, 1.0, 0.0]),
        ([0, 1, 5, 4], [0.0, -1.0, 0.0]),
    ];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (corner, normal) in faces {
        let base = (positions.len() / 3) as u32;
        for index in corner {
            positions.extend_from_slice(&corners[index]);
            normals.extend_from_slice(&normal);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut bin = Vec::new();
    let position = floats(&mut bin, &positions);
    let normal = floats(&mut bin, &normals);
    let index = (bin.len(), indices.len() * 4);
    for value in &indices {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    let times = floats(&mut bin, &[0.0, 2.0]);
    let travel = floats(&mut bin, &[0.0, 0.0, 0.0, 2.0 * SPEED, 0.0, 0.0]);
    let view = |(offset, length): (usize, usize)| json!({ "buffer": 0, "byteOffset": offset, "byteLength": length });
    let document = json!({
        "asset": { "version": "2.0", "generator": "pfx test" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [
            { "name": "rig", "children": [1, 2] },
            { "name": "car", "mesh": 0 },
            { "name": "mount", "translation": [0.0, 0.0, 3.0] },
        ],
        "meshes": [{ "name": "car", "primitives": [{
            "attributes": { "POSITION": 0, "NORMAL": 1 },
            "indices": 2,
        }] }],
        "animations": [{
            "name": "drive",
            "samplers": [{ "input": 3, "output": 4, "interpolation": "LINEAR" }],
            "channels": [{ "sampler": 0, "target": { "node": 0, "path": "translation" } }],
        }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": [view(position), view(normal), view(index), view(times), view(travel)],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3", "min": [-0.25, -0.25, -0.25], "max": [0.25, 0.25, 0.25] },
            { "bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5125, "count": 36, "type": "SCALAR" },
            { "bufferView": 3, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [2.0] },
            { "bufferView": 4, "componentType": 5126, "count": 2, "type": "VEC3" },
        ],
    });
    glb(&document.to_string(), &bin)
}

fn scene(name: &str, blur: &str) -> Scene {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("shot-motion-blur")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../load/tests/scenes");
    std::fs::copy(fixtures.join("block.gltf"), root.join("block.gltf")).unwrap();
    std::fs::write(root.join("blur.materials.toml"), MATERIALS).unwrap();
    std::fs::write(root.join("rig.glb"), rig_glb()).unwrap();
    let mut text = String::from(
        "format = 1\n\nmaterials = [\"blur.materials.toml\"]\nfallback = \"glow\"\n\n[mesh.block]\nfile = \"block.gltf\"\n\n[mesh.rig]\nfile = \"rig.glb\"\n\n[[object]]\nname = \"runner\"\nmesh = \"rig\"\nmaterial = \"lime\"\n\n[object.animation]\nclip = \"drive\"\n",
    );
    for post in 0..64 {
        text.push_str(&format!(
            "\n[[object]]\nname = \"post{post}\"\nmesh = \"block\"\nat = [{:.2}, 0.0, -6.0]\nscale = [0.2, 8.0, 0.2]\nmaterial = \"ember\"\n",
            -4.0 + post as f32 * 0.5
        ));
    }
    text.push_str("\n[camera]\nat = [0.0, 0.0, 3.0]\nlook_at = [0.0, 0.0, 0.0]\nfov = 40.0\n");
    text.push_str(blur);
    std::fs::write(root.join("follow.scene.toml"), &text).unwrap();
    Scene::open(root.join("follow.scene.toml")).unwrap()
}

fn shot(scene: &Scene) -> Vec<[f32; 3]> {
    shot_from(scene, "mount", FPS, FROM)
}

fn shot_from(scene: &Scene, follow: &str, fps: u32, from: u64) -> Vec<[f32; 3]> {
    let mut last = Vec::new();
    play(
        &Shot {
            scene,
            size: (WIDTH, HEIGHT),
            fps: pfx_core::rate::Rate::whole(fps),
            from,
            frames: 1,
            camera: Some(Follow {
                name: follow.into(),
                lens: Lens {
                    fov: None,
                    near: None,
                },
            }),
            quality: Quality::Final,
            seed: SEED,
            shutter: None,
            fixed_exposure: true,
        },
        |frame| {
            last = frame
                .pixels
                .chunks_exact(4)
                .map(|texel| [0, 1, 2].map(|channel| half(texel[channel])))
                .collect();
            Ok(())
        },
    )
    .unwrap();
    last
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

const BLUR: &str = "\n[motion_blur]\nshutter = 1.0\nmax_px = 64.0\nsamples = 24\n";

fn blur(samples: u32) -> String {
    format!("\n[motion_blur]\nshutter = 1.0\nmax_px = 64.0\nsamples = {samples}\n")
}

fn luminance(image: &[[f32; 3]]) -> Vec<f32> {
    image
        .iter()
        .map(|[r, g, b]| 0.2126 * r + 0.7152 * g + 0.0722 * b)
        .collect()
}

fn deviation(image: &[f32], x: u32, y: u32) -> f32 {
    let at = |x: u32, y: u32| image[(y * WIDTH + x) as usize];
    let around = (at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1)) / 4.0;
    (at(x, y) - around).abs()
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (total, count) = values.fold((0.0, 0), |(t, c), v| (t + v, c + 1));
    total / count.max(1) as f32
}

struct Car {
    inside: Vec<bool>,
}

fn car(sharp: &[[f32; 3]]) -> Car {
    let lit: Vec<bool> = sharp.iter().map(|texel| texel[1] > 0.5).collect();
    let near = |x: u32, y: u32, reach: i32| {
        (-reach..=reach).all(|dy| {
            (-reach..=reach).all(|dx| {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                nx < 0
                    || ny < 0
                    || nx >= WIDTH as i32
                    || ny >= HEIGHT as i32
                    || lit[(ny as u32 * WIDTH + nx as u32) as usize]
            })
        })
    };
    let mut inside = vec![false; lit.len()];
    for y in 1..HEIGHT - 1 {
        for x in 1..WIDTH - 1 {
            let i = (y * WIDTH + x) as usize;
            inside[i] = lit[i] && near(x, y, 2);
        }
    }
    let count = inside.iter().filter(|&&on| on).count();
    assert!(count > 200, "the followed car is on screen: {count} pixels");
    Car { inside }
}

struct Measured {
    car: f32,
    world: f32,
}

fn measured(sharp: &[[f32; 3]], blurred: &[[f32; 3]]) -> Measured {
    let shape = car(sharp);
    let car_change = sharp
        .iter()
        .zip(blurred)
        .zip(&shape.inside)
        .filter(|(_, inside)| **inside)
        .map(|((a, b), _)| (a[1] - b[1]).abs())
        .fold(0.0, f32::max);
    let world = mean(sharp.iter().zip(blurred).map(|(a, b)| (a[0] - b[0]).abs()));
    println!("car change {car_change:.4}, world change {world:.4}");
    Measured {
        car: car_change,
        world,
    }
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_followed_node_stays_sharp_while_the_still_world_behind_it_streaks() {
    let sharp = shot(&scene("closed", ""));
    let blurred = shot(&scene("open", BLUR));
    let m = measured(&sharp, &blurred);
    assert!(m.car < 0.02, "the followed node smeared: {}", m.car);
    assert!(m.world > 0.02, "the still wall streaks: {}", m.world);
}

const CHASE: &str = r#"recipe = 1
name = "chase"

[output]
glb = "street.glb"
scene = "street.scene.toml"

[timeline]
fps = 24
frames = 24
size = [320, 192]

[[route]]
name = "main"
points = [[0.0, 0.0, 0.0], [0.0, 0.0, 400.0]]

[[follow]]
object = "car"
route = "main"
speed = [[0.0, 24.0]]

[[object]]
name = "car"
kind = "box"
at = [0.0, 0.5, 0.0]
size = [1.8, 0.8, 4.2]
material = "lime"

[[camera]]
name = "chase"
fov_y_deg = 50.0

[camera.rig]
follow = "car"

[materials.lime]
base = [0.0, 0.0, 0.0]
roughness = 1.0
emission = [0.0, 1.0, 0.0]

[materials.ember]
base = [0.0, 0.0, 0.0]
roughness = 1.0
emission = [1.0, 0.0, 0.0]
"#;

const RIMS: &str = r#"recipe = 1
name = "rims"

[output]
glb = "street.glb"
scene = "street.scene.toml"

[timeline]
fps = 24
frames = 24
size = [320, 192]

[[route]]
name = "main"
points = [[0.0, 0.0, 0.0], [0.0, 0.0, 400.0]]

[[follow]]
object = "car"
route = "main"
speed = [[0.0, 33.0]]

[[object]]
name = "car"
kind = "generator"
generator = "pfx/car"
params = { style = "sports", seed = 1 }

[[camera]]
name = "side"
fov_y_deg = 30.0

[camera.rig]
follow = "car"
offset = [5.0, 0.5, 0.0]
look = [0.0, 0.4, 0.0]
"#;

fn chase_text(blur: &str) -> String {
    let mut text = CHASE.to_string();
    for post in 0..60 {
        for (side, x) in [("l", -3.0), ("r", 3.0)] {
            text.push_str(&format!(
                "\n[[object]]\nname = \"post{post}{side}\"\nkind = \"box\"\nat = [{x:.1}, 1.0, {:.1}]\nsize = [0.2, 2.0, 0.2]\nmaterial = \"ember\"\n",
                post as f32 * 3.0
            ));
        }
    }
    text + blur
}

fn folder(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("shot-motion-blur")
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
    Scene::open(root.join("scenes/street.scene.toml")).unwrap()
}

fn still(name: &str, text: &str, camera: &str) -> Vec<[f32; 3]> {
    let root = folder(name);
    std::fs::write(root.join("still.toml"), text).unwrap();
    let time = 0.5;
    let (recipe, built) = pfx_recipe_scene::load_at(
        &root.join("still.toml"),
        None,
        &[],
        &pfx_recipe::Pick::default(),
        &pfx_build::Registry::new(),
        Some(time),
    )
    .unwrap();
    let gpu = pfx_recipe_scene::headless().unwrap();
    let rgba =
        pfx_recipe_scene::shoot_posed(&gpu, (&recipe, &built), camera, [WIDTH, HEIGHT], Some(time))
            .unwrap();
    rgba.chunks_exact(4)
        .map(|texel| [0, 1, 2].map(|c| (f32::from(texel[c]) / 255.0).powf(2.2)))
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_timeline_scenes_followed_car_stays_sharp_while_the_road_streaks() {
    let sharp = shot_from(&exported("chase-closed", &chase_text("")), "chase", 24, 12);
    let blurred = shot_from(&exported("chase-open", &chase_text(BLUR)), "chase", 24, 12);
    let m = measured(&sharp, &blurred);
    assert!(m.car < 0.02, "the followed car smeared: {}", m.car);
    assert!(m.world > 0.02, "the posts streak: {}", m.world);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_recipe_still_keeps_its_followed_car_sharp_while_the_road_streaks() {
    let sharp = still("still-closed", &chase_text(""), "chase");
    let blurred = still("still-open", &chase_text(BLUR), "chase");
    let m = measured(&sharp, &blurred);
    assert!(m.car < 0.02, "the followed car smeared: {}", m.car);
    assert!(m.world > 0.02, "the posts streak: {}", m.world);
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn spinning_rims_converge_to_a_smooth_smear_as_samples_grow() {
    const RIM: [u32; 4] = [30, 85, 70, 120];
    let rough = |image: &[[f32; 3]]| {
        let light = luminance(image);
        mean(
            (RIM[1]..RIM[3])
                .flat_map(|y| (RIM[0]..RIM[2]).map(move |x| (x, y)))
                .map(|(x, y)| deviation(&light, x, y)),
        )
    };
    let sharp = rough(&shot_from(&exported("rims-closed", RIMS), "side", 24, 12));
    let runs: Vec<f32> = [8, 24, 64]
        .iter()
        .map(|&samples| {
            let scene = exported(
                &format!("rims-{samples}"),
                &format!("{RIMS}{}", blur(samples)),
            );
            rough(&shot_from(&scene, "side", 24, 12))
        })
        .collect();
    println!("rim deviation: sharp {sharp:.4}, samples 8, 24, 64: {runs:?}");
    assert!(runs[1] < runs[0] && runs[2] < runs[1], "{runs:?}");
    assert!(runs[2] < 0.5 * runs[0], "{runs:?}");
    assert!(
        runs[2] < 0.5 * sharp,
        "{runs:?} against the still rim's {sharp}"
    );
}

const COLUMN: &str = r#"recipe = 1
name = "skin"

[timeline]
fps = 24
frames = 24
size = [320, 192]

[[object]]
name = "col"
kind = "mesh"
path = "column.glb"
at = [0.0, 0.0, 0.0]
clip = "slide"
loop = false
material = "lime"

[[camera]]
name = "eye"
position = [0.0, 1.5, 7.0]
target = [0.0, 1.5, 0.0]
fov_y_deg = 40.0

[materials.lime]
base = [0.0, 0.0, 0.0]
roughness = 1.0
emission = [0.0, 1.0, 0.0]
"#;

fn slide() -> Vec<pfx_core::anim::Clip> {
    use pfx_core::anim::{Channel, Clip, Interpolation, Property};
    vec![
        Clip::new(
            "slide",
            vec![Channel {
                joint: 0,
                property: Property::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![-10.0, 0.0, 0.0, 10.0, 0.0, 0.0],
            }],
        )
        .unwrap(),
    ]
}

fn skinned_still(name: &str, blur: &str) -> Vec<[f32; 3]> {
    let text = format!("{COLUMN}{blur}");
    let root = folder(name);
    std::fs::write(
        root.join("column.glb"),
        pfx_load::fixture::skinned_column_with(3, slide()),
    )
    .unwrap();
    std::fs::write(root.join("still.toml"), &text).unwrap();
    let time = 0.5;
    let (recipe, built) = pfx_recipe_scene::load_at(
        &root.join("still.toml"),
        None,
        &[],
        &pfx_recipe::Pick::default(),
        &pfx_build::Registry::new(),
        Some(time),
    )
    .unwrap();
    assert!(built.skinned(), "the column is skinned");
    let gpu = pfx_recipe_scene::headless().unwrap();
    let rgba =
        pfx_recipe_scene::shoot_posed(&gpu, (&recipe, &built), "eye", [WIDTH, HEIGHT], Some(time))
            .unwrap();
    rgba.chunks_exact(4)
        .map(|texel| [0, 1, 2].map(|c| (f32::from(texel[c]) / 255.0).powf(2.2)))
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn a_skinned_piece_in_a_recipe_still_blurs_along_its_own_motion() {
    let sharp = skinned_still("skin-closed", "");
    let blurred = skinned_still("skin-open", BLUR);
    let lit = |image: &[[f32; 3]]| image.iter().filter(|texel| texel[1] > 0.05).count();
    let row = HEIGHT / 2;
    let across = |image: &[[f32; 3]]| {
        (0..WIDTH)
            .filter(|&x| image[(row * WIDTH + x) as usize][1] > 0.05)
            .count()
    };
    println!(
        "column: {} px lit sharp, {} blurred; {} px wide sharp, {} blurred",
        lit(&sharp),
        lit(&blurred),
        across(&sharp),
        across(&blurred)
    );
    assert!(across(&sharp) > 4, "the column is on screen");
    assert!(
        across(&blurred) >= across(&sharp) + 6,
        "the skinned column streaks along its slide"
    );
}
