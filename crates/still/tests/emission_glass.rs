use pfx_build::Registry;
use pfx_recipe::{Recipe, load};
use pfx_recipe_scene::pace::{turn, turn_reporting};
use pfx_recipe_scene::{headless, still, write_png};
use pfx_still::{Reel, Seed, Staging, Take, View, eyepiece, looked, looked_with, trace};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const SIZE: [u32; 2] = [320, 320];
const HALF: f64 = 1.0;
const SAMPLES: u32 = 128;
const SEED: u32 = 3;
const TOLERANCE: f64 = 28.0;

const HEAD: &str = r#"recipe = 1
name = "lamp"

[[camera]]
name = "main"
kind = "orthographic"
position = [0.0, 0.0, 5.0]
target = [0.0, 0.0, 0.0]
up = [0.0, 1.0, 0.0]
ortho_height = 2.0

[sun]
model = "authored"
toward = [0.3, 0.8, 0.5]
irradiance = 0.3
"#;

const LAMP: &str = r#"
[materials.lamp]
base = [0.1, 0.0, 0.0]
metalness = 0.0
roughness = 0.5
emission = [2.2, 0.04, 0.02]

[materials.lens]
base = [1.0, 1.0, 1.0]
metalness = 0.0
roughness = 0.02
transmission = 1.0
ior = 1.5

[materials.backdrop]
base = [0.2, 0.2, 0.2]
roughness = 1.0
specular = 0.0

[[object]]
name = "backdrop"
kind = "box"
size = [4.0, 4.0, 0.1]
at = [0.0, 0.0, -0.6]
material = "backdrop"

[[object]]
name = "lamp"
kind = "box"
size = [1.0, 0.3, 0.2]
material = "lamp"

[[object]]
name = "lens"
kind = "box"
size = [1.4, 0.6, 0.04]
at = [0.0, 0.0, 0.4]
material = "lens"
"#;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn scene(name: &str, file: &str, text: &str) -> PathBuf {
    let dir = root().join("tmp").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(file);
    std::fs::write(&path, text).unwrap();
    path
}

fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        for side in [0usize, 1] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let first = positions.len() as u32;
            for (a, b) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                let mut p = [0.0; 3];
                p[axis] = if side == 0 { lo[axis] } else { hi[axis] };
                p[u] = if a == 0 { lo[u] } else { hi[u] };
                p[v] = if b == 0 { lo[v] } else { hi[v] };
                positions.push(p);
                let mut n = [0.0; 3];
                n[axis] = if side == 0 { -1.0 } else { 1.0 };
                normals.push(n);
            }
            let quad = if side == 1 {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            indices.extend(quad.map(|k| first + k));
        }
    }
    (positions, normals, indices)
}

fn glb(parts: &[([f32; 3], [f32; 3], usize)], materials: serde_json::Value) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    for &(lo, hi, material) in parts {
        let (positions, normals, indices) = cuboid(lo, hi);
        let mut view = |bytes: &[u8]| {
            views.push(serde_json::json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len()}));
            bin.extend_from_slice(bytes);
            views.len() - 1
        };
        let p = view(&floats(&positions));
        let n = view(&floats(&normals));
        let i = view(
            &indices
                .iter()
                .flat_map(|k| k.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        accessors.push(serde_json::json!({"bufferView": p, "componentType": 5126, "count": positions.len(), "type": "VEC3", "min": lo, "max": hi}));
        accessors.push(serde_json::json!({"bufferView": n, "componentType": 5126, "count": normals.len(), "type": "VEC3"}));
        accessors.push(serde_json::json!({"bufferView": i, "componentType": 5125, "count": indices.len(), "type": "SCALAR"}));
        let at = accessors.len();
        primitives.push(serde_json::json!({"attributes": {"POSITION": at - 3, "NORMAL": at - 2}, "indices": at - 1, "material": material}));
    }
    let document = serde_json::json!({
        "asset": {"version": "2.0"},
        "extensionsUsed": ["KHR_materials_transmission", "KHR_materials_emissive_strength", "KHR_materials_ior"],
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "lamp", "mesh": 0}],
        "meshes": [{"primitives": primitives}],
        "materials": materials,
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
    });
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn floats(values: &[[f32; 3]]) -> Vec<u8> {
    values
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

fn imported(name: &str) -> PathBuf {
    let materials = serde_json::json!([
        {"name": "lamp", "pbrMetallicRoughness": {"baseColorFactor": [0.1, 0.0, 0.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.5},
         "emissiveFactor": [1.0, 0.04 / 2.2, 0.02 / 2.2], "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": 2.2}}},
        {"name": "lens", "pbrMetallicRoughness": {"baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.02},
         "extensions": {"KHR_materials_transmission": {"transmissionFactor": 1.0}, "KHR_materials_ior": {"ior": 1.5}}},
        {"name": "backdrop", "pbrMetallicRoughness": {"baseColorFactor": [0.2, 0.2, 0.2, 1.0], "metallicFactor": 0.0, "roughnessFactor": 1.0}}
    ]);
    let bytes = glb(
        &[
            ([-2.0, -2.0, -0.65], [2.0, 2.0, -0.55], 2),
            ([-0.5, -0.15, -0.1], [0.5, 0.15, 0.1], 0),
            ([-0.7, -0.3, 0.38], [0.7, 0.3, 0.42], 1),
        ],
        materials,
    );
    let dir = root().join("tmp").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lamp.glb"), bytes).unwrap();
    scene(
        name,
        "scene.toml",
        &format!("{HEAD}\n[[object]]\nname = \"model\"\nkind = \"mesh\"\npath = \"lamp.glb\"\n"),
    )
}

fn staged(path: &Path) -> (Recipe, Staging) {
    let recipe = load(path).unwrap().recipe;
    let built = pfx_build::build_with(&recipe, path.parent().unwrap(), &Registry::new()).unwrap();
    let recipe = built.recipe(&recipe);
    let staging = Staging::of(&recipe, &built.parts).unwrap();
    (recipe, staging)
}

fn traced(path: &Path) -> Vec<u8> {
    let (recipe, staging) = staged(path);
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
    looked(&film, &staging.finish).unwrap()
}

fn mean(image: &[u8], at: [f64; 2]) -> [f64; 3] {
    let x = ((at[0] + HALF) / (2.0 * HALF) * f64::from(SIZE[0])) as i64;
    let y = ((HALF - at[1]) / (2.0 * HALF) * f64::from(SIZE[1])) as i64;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    for dy in -4..=4 {
        for dx in -12..=12 {
            let k = ((y + dy) as usize * SIZE[0] as usize + (x + dx) as usize) * 4;
            for c in 0..3 {
                sum[c] += f64::from(image[k + c]);
            }
            count += 1.0;
        }
    }
    sum.map(|v| v / count)
}

fn clipped(path: &Path) -> Vec<u8> {
    let mut reel = Reel::open(path, None, &[], Some(SEED), &Registry::new()).unwrap();
    let camera = pfx_recipe_scene::camera(&reel.recipe, "main")
        .unwrap()
        .clone();
    let take = Take {
        size: SIZE,
        samples: SAMPLES,
        noise: None,
        seed: Seed::Locked,
    };
    let rendered = reel.frame(&camera, take, &mut |_| ()).unwrap();
    looked_with(&rendered.film, (&rendered.finish, &rendered.overlay), false).unwrap()
}

fn compare(name: &str, file: &Path) {
    compare_with(name, file, traced(file));
}

fn compare_with(name: &str, file: &Path, traced: Vec<u8>) {
    compare_seen(name, file, traced, true);
}

fn compare_seen(name: &str, file: &Path, traced: Vec<u8>, glows: bool) {
    write_png(
        &root().join(format!("tmp/{name}-traced.png")),
        SIZE,
        &traced,
    )
    .unwrap();
    turn();
    let live = still(file, "main", SIZE).unwrap();
    write_png(&root().join(format!("tmp/{name}-live.png")), SIZE, &live).unwrap();
    turn();
    let [t, l] = [mean(&traced, [0.0, 0.0]), mean(&live, [0.0, 0.0])];
    let off = (0..3).map(|c| (t[c] - l[c]).abs()).fold(0.0, f64::max);
    println!("{name}: the lamp behind its lens, traced {t:.1?}, live {l:.1?}, off by {off:.1}");
    for (side, [r, g, b]) in [("traced", t), ("live", l)] {
        if glows {
            assert!(r > 180.0, "{name}: the {side} lamp is bright: {r} {g} {b}");
            assert!(
                r > 3.0 * g.max(b),
                "{name}: the {side} lamp is red: {r} {g} {b}"
            );
        } else {
            assert!(
                r < 1.5 * g.max(b),
                "{name}: the {side} metal pane hides the red lamp: {r} {g} {b}"
            );
        }
    }
    assert!(
        off <= TOLERANCE,
        "{name}: traced and live agree within {TOLERANCE}: {off}"
    );
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn an_emitter_behind_a_glass_lens_glows_red_traced_as_live() {
    let _gpu = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    compare(
        "emission-glass",
        &scene("emission-glass", "scene.toml", &format!("{HEAD}{LAMP}")),
    );
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn an_imported_gltf_emitter_behind_its_lens_glows_red_traced_as_live() {
    let _gpu = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    compare("emission-glass-gltf", &imported("emission-glass-gltf"));
}

#[test]
fn an_imported_gltf_lamp_keeps_its_emission_and_its_lens_its_glass() {
    let (_, staging) = staged(&imported("emission-glass-cpu"));
    let named = |name: &str| {
        let at = staging
            .names
            .iter()
            .position(|known| known == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", staging.names));
        staging.materials[at]
    };
    let lamp = named("lamp");
    for (got, want) in lamp.emission.iter().zip([2.2, 0.04, 0.02]) {
        assert!((got - want).abs() < 1e-3, "{:?}", lamp.emission);
    }
    assert_eq!(lamp.metalness, 0.0);
    let lens = named("lens");
    assert_eq!(lens.transmission, 1.0);
    assert_eq!(lens.emission, [0.0; 3]);
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn an_imported_emitter_behind_its_lens_glows_red_in_a_traced_clip_and_with_glass_shadows() {
    let _gpu = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let file = imported("emission-glass-clip");
    compare_with("emission-glass-clip", &file, clipped(&file));
    let text = std::fs::read_to_string(&file).unwrap();
    let shadows = scene(
        "emission-glass-clip",
        "shadows.toml",
        &format!("{text}\n[trace]\ntransmissive_shadows = true\n"),
    );
    compare("emission-glass-shadows", &shadows);
}

fn pane(metalness: f32) -> String {
    let lens = "[materials.lens]\nbase = [1.0, 1.0, 1.0]\nmetalness = 0.0\nroughness = 0.02\ntransmission = 1.0\nior = 1.5\n";
    assert!(LAMP.contains(lens));
    LAMP.replace(
        lens,
        &format!(
            "[materials.lens]\nfamily = \"glass\"\nbase = [1.0, 1.0, 1.0]\nmetalness = {metalness:.1}\nroughness = 0.0\ntransmission = 0.96\n"
        ),
    )
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn a_metallic_transmissive_pane_draws_as_metal_traced_as_live() {
    let _gpu = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let file = scene("metal-pane", "scene.toml", &format!("{HEAD}{}", pane(1.0)));
    compare_seen("metal-pane", &file, traced(&file), false);
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn a_transmissive_pane_with_no_metal_shows_its_emitter_traced_as_live() {
    let _gpu = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let file = scene("clear-pane", "scene.toml", &format!("{HEAD}{}", pane(0.0)));
    compare_seen("clear-pane", &file, traced(&file), true);
}
