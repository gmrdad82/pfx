mod common;

use common::{golden, scratch};
use pfx_build::{Registry, write};
use pfx_gpu::{Gpu, wgpu};
use pfx_live::renderer::{Exposure, Renderer};
use pfx_load::scene::Scene;
use pfx_recipe_scene::pace::{TurnState, turn_reporting, turn_soon};
use pfx_recipe_scene::{SETTLE_FRAMES, difference, headless, psnr, shoot, write_png};
use pfx_still::{Request, View, looked_with, trace};
use std::path::{Path, PathBuf};

const SIZE: [u32; 2] = [640, 400];
const SAMPLES: u32 = 16;
const SEED: u32 = 7;
const BOX: u32 = 8;

struct Case {
    set: &'static str,
    camera: &'static str,
    overrides: &'static [&'static str],
    live: f64,
    traced: f64,
}

const TABLE: Case = Case {
    set: "table",
    camera: "main",
    overrides: &[],
    live: 40.0,
    traced: 44.0,
};

const TOYBOX: Case = Case {
    set: "toybox",
    camera: "high",
    overrides: &["camera.high.fstop=0.0"],
    live: 30.0,
    traced: 37.0,
};

fn emitted(case: &Case) -> (PathBuf, PathBuf) {
    let folder = golden(case.set);
    let recipe = folder.join(format!("{}.toml", case.set));
    let set: Vec<String> = case.overrides.iter().map(|s| s.to_string()).collect();
    let (mut r, built) = pfx_recipe_scene::load(&recipe, None, &set, &Registry::new()).unwrap();
    r.output.scene = Some(format!("{}.scene.toml", case.set));
    r.output.scene_camera = Some(case.camera.to_string());
    assert_eq!(r.output.scene_format, 1);
    let out = scratch(&format!("scene-gpu-{}", case.set));
    write(&built, &r, &folder, Some(&out)).unwrap();
    (recipe, out.join(format!("{}.scene.toml", case.set)))
}

fn overrides(case: &Case) -> Vec<String> {
    case.overrides.iter().map(|s| s.to_string()).collect()
}

fn engine_live(gpu: &Gpu, scene: &Scene) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = gpu.offscreen(SIZE[0], SIZE[1], format).unwrap();
    let mut renderer = Renderer::new_with_output_format(gpu.clone(), SIZE[0], SIZE[1], format)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    renderer.set_exposure(Exposure::Fixed(1.0)).unwrap();
    let mut staged = renderer.stage(scene).unwrap();
    let finish = staged.finish();
    let mut turns = TurnState::default();
    for frame in 0..SETTLE_FRAMES {
        let drawn = staged.frame(
            SIZE[0] as f32 / SIZE[1] as f32,
            frame as f32 / pfx_recipe_scene::FRAME_RATE,
            SEED,
        );
        let timings = renderer
            .render(
                &drawn.scene,
                &drawn.text,
                &drawn.effects,
                finish,
                &target.view,
            )
            .unwrap();
        let measured: f64 = timings.iter().map(|t| t.milliseconds).sum();
        let measured = if timings.is_empty() {
            pfx_recipe_scene::UNMEASURED_MS
        } else {
            measured
        };
        turn_soon(&mut turns, measured);
    }
    gpu.readback_rgba8(&target).unwrap()
}

fn pfx_live(gpu: &Gpu, case: &Case, recipe: &Path) -> Vec<u8> {
    let (r, built) =
        pfx_recipe_scene::load(recipe, None, &overrides(case), &Registry::new()).unwrap();
    shoot(gpu, &r, &built.parts, case.camera, SIZE).unwrap()
}

fn pfx_traced(case: &Case, recipe: &Path) -> Vec<u8> {
    let set = overrides(case);
    let rendered = pfx_still::render(
        &Request {
            recipe,
            camera: case.camera,
            size: SIZE,
            samples: SAMPLES,
            noise: None,
            seed: Some(SEED),
            view: View::Lens,
            variant: None,
            set: &set,
            pick: &pfx_recipe::Pick::default(),
            crop: None,
            denoise: false,
            time: None,
            counters: false,
        },
        &Registry::new(),
        &mut |_| {},
    )
    .unwrap();
    looked_with(&rendered.film, &rendered.finish, false).unwrap()
}

fn engine_traced(gpu: &Gpu, scene: &Scene) -> Vec<u8> {
    let staged = pfx_trace::stage::scene(scene, SIZE[0] as f32 / SIZE[1] as f32).unwrap();
    let mut turn = turn_reporting;
    let (film, _) = trace(gpu, &staged, SIZE, SAMPLES, SEED, &mut |_| {}, &mut turn).unwrap();
    let finish = scene.finish.as_ref().map(|f| f.chain.clone()).unwrap();
    looked_with(&film, &finish, false).unwrap()
}

fn boxed(rgba: &[u8]) -> Vec<u8> {
    let [width, height] = SIZE.map(|v| v / BOX);
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0u32; 3];
            for dy in 0..BOX {
                for dx in 0..BOX {
                    let at = (((y * BOX + dy) * SIZE[0] + x * BOX + dx) * 4) as usize;
                    for k in 0..3 {
                        sum[k] += u32::from(rgba[at + k]);
                    }
                }
            }
            let area = BOX * BOX;
            out.extend(sum.map(|v| ((v + area / 2) / area) as u8));
            out.push(255);
        }
    }
    out
}

fn compare(case: &Case, way: &str, pfx: &[u8], engine: &[u8], floor: f64) {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/scene-gpu")
        .join(case.set);
    write_png(&out.join(format!("{way}-pfx.png")), SIZE, pfx).unwrap();
    write_png(&out.join(format!("{way}-scene.png")), SIZE, engine).unwrap();
    write_png(
        &out.join(format!("{way}-difference.png")),
        SIZE,
        &difference(pfx, engine),
    )
    .unwrap();
    let measured = psnr(pfx, engine).unwrap();
    let smooth = psnr(&boxed(pfx), &boxed(engine)).unwrap();
    println!(
        "{} {way}: PSNR {measured:.2} dB, {smooth:.2} dB over {BOX}x{BOX} boxes",
        case.set
    );
    let measured = if way == "traced" { smooth } else { measured };
    assert!(
        measured >= floor,
        "{} {way}: the scene draws {measured:.2} dB from pfx's stage, under {floor} dB",
        case.set
    );
}

fn live_and_traced(case: &Case) {
    let (recipe, file) = emitted(case);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .starts_with("format = 1\n")
    );
    let scene = Scene::open(&file).unwrap();
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
    let gpu = headless().unwrap();
    let pfx = pfx_live(&gpu, case, &recipe);
    turn_reporting(Default::default());
    let engine = engine_live(&gpu, &scene);
    turn_reporting(Default::default());
    compare(case, "live", &pfx, &engine, case.live);
    let pfx = pfx_traced(case, &recipe);
    turn_reporting(Default::default());
    let engine = engine_traced(&gpu, &scene);
    turn_reporting(Default::default());
    compare(case, "traced", &pfx, &engine, case.traced);
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn the_tables_scene_staged_by_the_engine_draws_as_pfx_draws_the_recipe() {
    live_and_traced(&TABLE);
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn the_toybox_scene_staged_by_the_engine_draws_as_pfx_draws_the_recipe() {
    live_and_traced(&TOYBOX);
}

fn looked(name: &str, look: &str) -> PathBuf {
    let source = golden(TABLE.set).join(format!("{}.toml", TABLE.set));
    let text = std::fs::read_to_string(source).unwrap();
    let anchor = "[look]\nexposure = 1.0\n";
    assert!(text.contains(anchor));
    let out = scratch(&format!("scene-gpu-look-{name}"));
    let recipe = out.join(format!("{}.toml", TABLE.set));
    std::fs::write(
        &recipe,
        text.replacen(anchor, &format!("{anchor}{look}\n"), 1),
    )
    .unwrap();
    recipe
}

fn mean(rgba: &[u8]) -> f64 {
    let sum: f64 = rgba
        .chunks_exact(4)
        .flat_map(|p| p[..3].iter().map(|&v| f64::from(v)))
        .sum();
    sum / (rgba.len() / 4 * 3) as f64
}

fn shift(with: &[u8], without: &[u8]) -> f64 {
    let total: f64 = with
        .chunks_exact(4)
        .zip(without.chunks_exact(4))
        .flat_map(|(a, b)| (0..3).map(move |k| (f64::from(a[k]) - f64::from(b[k])).abs()))
        .sum();
    total / (with.len() / 4 * 3) as f64
}

fn linear(level: u8) -> f64 {
    let c = f64::from(level) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn grain_spread(with: &[u8], without: &[u8], response: f64) -> (f64, usize) {
    let mut z = Vec::new();
    for (a, b) in with.chunks_exact(4).zip(without.chunks_exact(4)) {
        if a[..3]
            .iter()
            .chain(&b[..3])
            .any(|&v| !(3..=252).contains(&v))
        {
            continue;
        }
        let plain = [0, 1, 2].map(|k| linear(b[k]));
        let luma = 0.2126 * plain[0] + 0.7152 * plain[1] + 0.0722 * plain[2];
        let scale = 1.0 - response * luma;
        z.extend((0..3).map(|k| (linear(a[k]) - plain[k]) / scale));
    }
    let n = z.len() as f64;
    let mean = z.iter().sum::<f64>() / n;
    let spread = (z.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
    (spread, z.len() / 3)
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn the_looks_grain_and_chromatic_read_alike_live_and_traced() {
    let gpu = headless().unwrap();
    let mut shots = Vec::new();
    for (name, look) in [
        ("plain", ""),
        ("grained", "grain = { amount = 0.05, seed = 2 }"),
        ("aberrated", "chromatic = { amount = 0.04, falloff = 1.0 }"),
    ] {
        let recipe = looked(name, look);
        let live = pfx_live(&gpu, &TABLE, &recipe);
        turn_reporting(Default::default());
        let traced = pfx_traced(&TABLE, &recipe);
        turn_reporting(Default::default());
        shots.push((live, traced));
    }
    let [
        (live, traced),
        (live_grain, traced_grain),
        (live_chroma, traced_chroma),
    ] = shots.try_into().unwrap();
    for (way, plain, grained, aberrated) in [
        ("live", &live, &live_grain, &live_chroma),
        ("traced", &traced, &traced_grain, &traced_chroma),
    ] {
        println!(
            "{way}: mean {:.2}, grained {:.2}, aberrated {:.2}",
            mean(plain),
            mean(grained),
            mean(aberrated)
        );
        assert!(
            (mean(grained) - mean(plain)).abs() < 0.6,
            "{way} grain moves the mean"
        );
        assert!(
            (mean(aberrated) - mean(plain)).abs() < 0.6,
            "{way} chromatic moves the mean"
        );
    }
    let expected = 0.05 * (1.0f64 / 6.0).sqrt();
    let (live_spread, live_count) = grain_spread(&live_grain, &live, 0.5);
    let (traced_spread, traced_count) = grain_spread(&traced_grain, &traced, 0.5);
    println!(
        "grain in linear light over the response: live {live_spread:.4} over {live_count} pixels, traced {traced_spread:.4} over {traced_count}, expected {expected:.4}"
    );
    assert!(live_count > 10_000 && traced_count > 10_000);
    for (way, spread) in [("live", live_spread), ("traced", traced_spread)] {
        assert!(
            (spread / expected - 1.0).abs() < 0.15,
            "{way} grain spreads {spread:.4}, expected {expected:.4}"
        );
    }
    assert!(
        (live_spread / traced_spread - 1.0).abs() < 0.1,
        "grain spreads {live_spread:.4} live and {traced_spread:.4} traced"
    );
    let live_shift = shift(&boxed(&live_chroma), &boxed(&live));
    let traced_shift = shift(&boxed(&traced_chroma), &boxed(&traced));
    println!("chromatic: live {live_shift:.3}, traced {traced_shift:.3} levels a channel");
    assert!(live_shift > 0.05 && traced_shift > 0.05);
    assert!(
        (live_shift / traced_shift).ln().abs() < 0.5_f64.ln().abs(),
        "chromatic shifts {live_shift:.3} live and {traced_shift:.3} traced"
    );
}

fn leaf_png() -> Vec<u8> {
    const SIDE: u32 = 64;
    let leaves = [
        (0.30, 0.30, 0.6, 0.20, 0.08),
        (0.70, 0.28, 2.2, 0.20, 0.08),
        (0.50, 0.55, 1.4, 0.24, 0.09),
        (0.25, 0.72, 2.8, 0.18, 0.07),
        (0.75, 0.74, 0.3, 0.18, 0.07),
    ];
    let mut pixels = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (u, v) = ((x as f64 + 0.5) / 64.0, (y as f64 + 0.5) / 64.0);
            let inside = leaves.iter().any(|&(cx, cy, turn, long, wide)| {
                let (du, dv) = (u - cx, v - cy);
                let along = du * f64::cos(turn) + dv * f64::sin(turn);
                let across = dv * f64::cos(turn) - du * f64::sin(turn);
                (along / long).powi(2) + (across / wide).powi(2) <= 1.0
            });
            pixels.extend([60, 150, 45, if inside { 255 } else { 0 }]);
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, SIDE, SIDE);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&pixels).unwrap();
    writer.finish().unwrap();
    out
}

fn leaf_tree(turned: bool) -> Vec<u8> {
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut normals = Vec::new();
    let mut state = 7u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f64::from(state >> 8) / f64::from(1u32 << 24)
    };
    for card in 0..14 {
        let centre = [next() - 0.5, 0.6 + 0.8 * next(), next() * 0.4 - 0.2];
        let tilt = (next() - 0.5) * 0.8;
        let half = 0.18;
        let first = positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push([
                (centre[0] + a * half * f64::cos(tilt)) as f32,
                (centre[1] + b * half) as f32,
                (centre[2] + a * half * f64::sin(tilt)) as f32,
            ]);
            uvs.push([(a as f32 + 1.0) / 2.0, (1.0 - b as f32) / 2.0]);
        }
        let away = turned && card % 2 == 1;
        let order = if away {
            [0, 2, 1, 0, 3, 2]
        } else {
            [0, 1, 2, 0, 2, 3]
        };
        indices.extend(order.map(|k| first + k));
        let facing = if away { -1.0f32 } else { 1.0 };
        normals.extend([[0.0, 0.0, facing]; 4]);
    }
    let mut bin: Vec<u8> = positions
        .iter()
        .flatten()
        .flat_map(|v: &f32| v.to_le_bytes())
        .collect();
    let normal_at = bin.len();
    bin.extend(normals.iter().flatten().flat_map(|v: &f32| v.to_le_bytes()));
    let uv_at = bin.len();
    bin.extend(uvs.iter().flatten().flat_map(|v: &f32| v.to_le_bytes()));
    let index_at = bin.len();
    bin.extend(indices.iter().flat_map(|k| k.to_le_bytes()));
    let image_at = bin.len();
    bin.extend(leaf_png());
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let lo = [0, 1, 2].map(|k| positions.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min));
    let hi = [0, 1, 2].map(|k| {
        positions
            .iter()
            .map(|p| p[k])
            .fold(f32::NEG_INFINITY, f32::max)
    });
    let document = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "crown", "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
        "materials": [{
            "name": "leaves",
            "alphaMode": "MASK",
            "alphaCutoff": 0.5,
            "doubleSided": true,
            "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicFactor": 0.0, "roughnessFactor": 0.8}
        }],
        "textures": [{"source": 0, "sampler": 0}],
        "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071}],
        "images": [{"mimeType": "image/png", "bufferView": 4}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": normal_at},
            {"buffer": 0, "byteOffset": normal_at, "byteLength": uv_at - normal_at},
            {"buffer": 0, "byteOffset": uv_at, "byteLength": index_at - uv_at},
            {"buffer": 0, "byteOffset": index_at, "byteLength": image_at - index_at},
            {"buffer": 0, "byteOffset": image_at, "byteLength": bin.len() - image_at}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": positions.len(), "type": "VEC3", "min": lo, "max": hi},
            {"bufferView": 1, "componentType": 5126, "count": normals.len(), "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": uvs.len(), "type": "VEC2"},
            {"bufferView": 3, "componentType": 5125, "count": indices.len(), "type": "SCALAR"}
        ]
    });
    pfx_build::library::join_glb(&document, &bin)
}

const DISTANCES: [(&str, f64, f64); 4] = [
    ("near", 2.0, -0.85),
    ("mid", 10.0, -0.29),
    ("far", 30.0, 6.1),
    ("distant", 80.0, 34.9),
];

fn leaf_coverage(rgba: &[u8]) -> [u32; 4] {
    let edges = [0, 256, 368, 496, SIZE[0]];
    let mut out = [0; 4];
    for (column, count) in out.iter_mut().enumerate() {
        for y in 0..SIZE[1] {
            for x in edges[column]..edges[column + 1] {
                let at = ((y * SIZE[0] + x) * 4) as usize;
                let [r, g, b] = [rgba[at], rgba[at + 1], rgba[at + 2]].map(i32::from);
                if g > r + 10 && g > b + 10 {
                    *count += 1;
                }
            }
        }
    }
    out
}

fn leaf_mass(rgba: &[u8]) -> [f64; 4] {
    let edges = [0, 256, 368, 496, SIZE[0]];
    let mut out = [0.0; 4];
    for (column, mass) in out.iter_mut().enumerate() {
        for y in 0..SIZE[1] {
            for x in edges[column]..edges[column + 1] {
                let at = ((y * SIZE[0] + x) * 4) as usize;
                let [r, g, b] = [rgba[at], rgba[at + 1], rgba[at + 2]].map(f64::from);
                *mass += (g - r.max(b)).max(0.0);
            }
        }
    }
    out
}

fn live_covers_as_traced(set: &str, (way, live): (&str, &[u8]), traced: &[u8]) {
    let near = [leaf_coverage(live)[0], leaf_coverage(traced)[0]].map(f64::from);
    assert!(
        (near[0] / near[1] - 1.0).abs() <= 0.1,
        "{set}: {way} covers {} pixels of the near copy, traced {}",
        near[0],
        near[1]
    );
    let [lit, mass] = [leaf_mass(live), leaf_mass(traced)];
    let scale = lit[0] / mass[0];
    let ratios: Vec<f64> = (0..4).map(|k| lit[k] / (scale * mass[k])).collect();
    println!("{set}: {way} leaf mass over traced at 2, 10, 30, 80 m: {ratios:.3?}");
    for (k, (name, distance, _)) in DISTANCES.iter().enumerate() {
        assert!(
            (0.8..=1.25).contains(&ratios[k]),
            "{set}: {way} draws {:.3} of the traced leaves on the {name} copy at {distance} m",
            ratios[k]
        );
    }
}

fn covers_alike(set: &str, (way, want): (&str, &[u8]), (other, got): (&str, &[u8])) {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/scene-gpu")
        .join(set);
    write_png(&out.join(format!("{way}.png")), SIZE, want).unwrap();
    write_png(&out.join(format!("{other}.png")), SIZE, got).unwrap();
    let want = leaf_coverage(want);
    let got = leaf_coverage(got);
    println!("{set}: leaf pixels at 2, 10, 30, 80 m: {way} {want:?}, {other} {got:?}");
    for (k, (name, distance, _)) in DISTANCES.iter().enumerate() {
        assert!(
            want[k] >= 4,
            "{set}: {way} draws {} leaf pixels for the {name} copy at {distance} m",
            want[k]
        );
        let slack = 0.15 * f64::from(want[k]) + 4.0;
        assert!(
            (f64::from(got[k]) - f64::from(want[k])).abs() <= slack,
            "{set}: {other} draws {} leaf pixels for the {name} copy at {distance} m, {way} {}",
            got[k],
            want[k]
        );
    }
}

fn mask_card_frames(set: &str, materials: &str, turned: bool) -> [(&'static str, Vec<u8>); 4] {
    let out = scratch(&format!("scene-gpu-{set}"));
    std::fs::write(out.join("tree.glb"), leaf_tree(turned)).unwrap();
    let mut text = String::from(
        "recipe = 1\nname = \"trees\"\n\n[output]\nglb = \"trees.glb\"\nscene = \"trees.scene.toml\"\nscene_camera = \"main\"\nscene_format = 1\n",
    );
    for (name, distance, x) in DISTANCES {
        text.push_str(&format!(
            "\n[[object]]\nname = \"{name}\"\nkind = \"mesh\"\npath = \"tree.glb\"\nshadow = \"none\"\nat = [{x:?}, 0.0, {:?}]\n",
            -distance
        ));
    }
    text.push_str("\n[[camera]]\nname = \"main\"\nposition = [0.0, 1.0, 0.0]\ntarget = [0.0, 1.0, -1.0]\nfov_y_deg = 40.0\n\n[sun]\nmodel = \"authored\"\ntoward = [0.3, 0.8, 0.5]\nradius_deg = 0.6\nirradiance = 3.0\n\n[sky]\nkind = \"analytic\"\nintensity = 0.8\n\n[look]\nexposure = 1.0\n");
    text.push_str(materials);
    let recipe = out.join("trees.toml");
    std::fs::write(&recipe, text).unwrap();
    let (r, built) = pfx_recipe_scene::load(&recipe, None, &[], &Registry::new()).unwrap();
    write(&built, &r, &out, None).unwrap();
    let scene = Scene::open(out.join("trees.scene.toml")).unwrap();
    let case = Case {
        set: "mask-cards",
        camera: "main",
        overrides: &[],
        live: 0.0,
        traced: 0.0,
    };
    let gpu = headless().unwrap();
    let live = pfx_live(&gpu, &case, &recipe);
    turn_reporting(Default::default());
    let live_scene = engine_live(&gpu, &scene);
    turn_reporting(Default::default());
    let traced = pfx_traced(&case, &recipe);
    turn_reporting(Default::default());
    let traced_scene = engine_traced(&gpu, &scene);
    turn_reporting(Default::default());
    [
        ("live", live),
        ("live-scene", live_scene),
        ("traced", traced),
        ("traced-scene", traced_scene),
    ]
}

fn pair<'a>(frames: &'a [(&'static str, Vec<u8>); 4], k: usize) -> (&'a str, &'a [u8]) {
    (frames[k].0, &frames[k].1)
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn repeated_mask_leaf_cards_cover_in_the_written_scene_as_in_the_recipe_at_every_distance() {
    let frames = mask_card_frames("mask-cards", "", true);
    let facing = mask_card_frames("mask-cards-facing", "", false);
    for k in [0, 1] {
        let [turned, all] = [&frames[k].1, &facing[k].1].map(|f| f64::from(leaf_coverage(f)[0]));
        assert!(
            turned >= 0.9 * all,
            "{}: cards turned from the camera cover {turned} pixels of the near copy, facing it {all}",
            frames[k].0
        );
    }
    covers_alike("mask-cards", pair(&frames, 0), pair(&frames, 1));
    covers_alike("mask-cards", pair(&frames, 2), pair(&frames, 3));
    live_covers_as_traced("mask-cards", pair(&frames, 0), &frames[2].1);
    live_covers_as_traced("mask-cards", pair(&frames, 1), &frames[3].1);
}

#[test]
#[ignore = "GPU: pgpu run --class clip --as pfx -- cargo test -- --ignored"]
fn a_recipe_material_over_mask_leaf_cards_keeps_their_cutout_live_traced_and_in_the_scene() {
    let own = mask_card_frames("mask-cards-own", "", true);
    let over = mask_card_frames(
        "mask-cards-over",
        "\n[materials.leaves]\nbase = [1.0, 1.0, 1.0]\nroughness = 1.0\nmetalness = 0.0\n",
        true,
    );
    for k in 0..4 {
        covers_alike(
            "mask-cards-over",
            (&format!("own-{}", own[k].0), &own[k].1),
            (&format!("over-{}", over[k].0), &over[k].1),
        );
    }
    live_covers_as_traced("mask-cards-over", pair(&over, 0), &over[2].1);
    live_covers_as_traced("mask-cards-over", pair(&over, 1), &over[3].1);
}
