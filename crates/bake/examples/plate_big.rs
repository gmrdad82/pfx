use std::path::{Path, PathBuf};
use std::time::Instant;

use pfx_bake::plate::{PlateRun, bake_plates, parse_plates};
use pfx_gpu::Gpu;
use pfx_gpu::pace::Turns;

const SIDE: u32 = 224;

fn peak_mib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmHWM:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|kib| kib.parse::<u64>().ok())
        })
        .map_or(0, |kib| kib / 1024)
}

fn grid_gltf(root: &Path, name: &str, phase: f32) -> u64 {
    let verts = (SIDE + 1) * (SIDE + 1);
    let mut positions = Vec::with_capacity(verts as usize * 12);
    let mut normals = Vec::with_capacity(verts as usize * 12);
    let mut uvs = Vec::with_capacity(verts as usize * 8);
    for z in 0..=SIDE {
        for x in 0..=SIDE {
            let u = x as f32 / SIDE as f32;
            let v = z as f32 / SIDE as f32;
            let h = 0.04 * ((u * 19.0 + phase).sin() * (v * 23.0).cos());
            for value in [u - 0.5, h, v - 0.5] {
                positions.extend_from_slice(&value.to_le_bytes());
            }
            for value in [0.0f32, 1.0, 0.0] {
                normals.extend_from_slice(&value.to_le_bytes());
            }
            for value in [u, v] {
                uvs.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let mut indices = Vec::with_capacity(SIDE as usize * SIDE as usize * 24);
    for z in 0..SIDE {
        for x in 0..SIDE {
            let a = z * (SIDE + 1) + x;
            let b = a + 1;
            let c = a + SIDE + 1;
            let d = c + 1;
            for index in [a, c, b, b, c, d] {
                indices.extend_from_slice(&index.to_le_bytes());
            }
        }
    }
    let views = [positions.len(), normals.len(), uvs.len(), indices.len()];
    let mut bin = Vec::new();
    for part in [&positions, &normals, &uvs, &indices] {
        bin.extend_from_slice(part);
    }
    std::fs::write(root.join(format!("{name}.bin")), &bin).unwrap();
    let mut offset = 0;
    let buffer_views: Vec<String> = views
        .iter()
        .map(|len| {
            let view = format!("{{\"buffer\":0,\"byteOffset\":{offset},\"byteLength\":{len}}}");
            offset += len;
            view
        })
        .collect();
    let triangles = u64::from(SIDE) * u64::from(SIDE) * 2;
    let gltf = format!(
        "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
\"nodes\":[{{\"name\":\"Grid\",\"mesh\":0}}],\
\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1,\"TEXCOORD_0\":2}},\"indices\":3}}]}}],\
\"accessors\":[\
{{\"bufferView\":0,\"componentType\":5126,\"count\":{verts},\"type\":\"VEC3\",\"min\":[-0.5,-0.04,-0.5],\"max\":[0.5,0.04,0.5]}},\
{{\"bufferView\":1,\"componentType\":5126,\"count\":{verts},\"type\":\"VEC3\"}},\
{{\"bufferView\":2,\"componentType\":5126,\"count\":{verts},\"type\":\"VEC2\"}},\
{{\"bufferView\":3,\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}],\
\"bufferViews\":[{}],\
\"buffers\":[{{\"uri\":\"{name}.bin\",\"byteLength\":{}}}]}}",
        triangles * 3,
        buffer_views.join(","),
        bin.len()
    );
    std::fs::write(root.join(format!("{name}.gltf")), gltf).unwrap();
    triangles
}

fn scene(root: &Path, millions: f64, unique: bool) -> u64 {
    let per = u64::from(SIDE) * u64::from(SIDE) * 2;
    let count = ((millions * 1e6) / per as f64).ceil() as u64;
    let meshes = if unique { count } else { 1 };
    let across = (count as f64).sqrt().ceil() as u64;
    std::fs::write(
        root.join("big.materials.toml"),
        "format = 1\n\n[materials.ground]\nbase = [0.5, 0.48, 0.45]\nroughness = 0.8\n",
    )
    .unwrap();
    let mut text = String::from(
        "format = 1\n\nmaterials = [\"big.materials.toml\"]\nfallback = \"ground\"\n\n",
    );
    for mesh in 0..meshes {
        grid_gltf(root, &format!("grid{mesh}"), mesh as f32 * 0.37);
        text.push_str(&format!(
            "[mesh.grid{mesh}]\nfile = \"grid{mesh}.gltf\"\n\n"
        ));
    }
    for index in 0..count {
        let x = (index % across) as f32 - across as f32 * 0.5;
        let z = (index / across) as f32 - across as f32 * 0.5;
        text.push_str(&format!(
            "[[object]]\nname = \"tile {index}\"\nmesh = \"grid{}\"\nat = [{x}, 0.0, {z}]\nmaterial = \"ground\"\n\n",
            index % meshes
        ));
    }
    let height = across as f32 * 0.9;
    text.push_str(&format!(
        "[sun]\nmodel = \"authored\"\ntoward = [0.4, 0.8, 0.3]\ncolor = [1.0, 0.95, 0.9]\nirradiance = 3.0\nreference_hour = 12.0\n\n\
[sky]\nkind = \"analytic\"\n\n[[light]]\nname = \"lamp\"\nposition = [0.0, 2.0, 0.0]\ncolor = [1.0, 0.9, 0.8]\nintensity = 0.6\nradius = 0.04\nrange = 6.0\n\n[camera]\nat = [0.0, {height}, {height}]\nlook_at = [0.0, 0.0, 0.0]\nfov = 50.0\n"
    ));
    std::fs::write(root.join("big.scene.toml"), text).unwrap();
    count * per
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let millions: f64 = args.first().map_or(10.0, |v| v.parse().unwrap());
    let width: u32 = args.get(1).map_or(640, |v| v.parse().unwrap());
    let height: u32 = args.get(2).map_or(360, |v| v.parse().unwrap());
    let unique = args.get(3).is_some_and(|v| v == "unique");
    let full = args.get(4).is_some_and(|v| v == "full");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tmp/plate-big")
        .join(format!(
            "{millions}m-{}",
            if unique { "unique" } else { "shared" }
        ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let triangles = scene(&root, millions, unique);
    println!("scene: {triangles} triangles");
    let recipe = format!(
        "[plates]\nscene = \"big.scene.toml\"\nsize = [{width}, {height}]\noverscan = 0.0\n{}\nthreshold = 0.05\nmin_samples = 4\nmax_samples = 16\n",
        if full {
            "anchors = [9.0, 12.0, 17.0]"
        } else {
            "anchors = [12.0]\nlayers = [\"color\", \"depth\"]"
        }
    );
    std::fs::write(
        root.join("bake.toml"),
        format!("app = \"pfx\"\nscene = \"grid0.gltf\"\nanchors = [12.0]\nsamples = 16\n\n[[volumes]]\nname = \"tile\"\nmin = [-0.5, 0.0, -0.5]\nmax = [0.5, 0.5, 0.5]\nspacing = 0.5\n\n{recipe}"),
    )
    .unwrap();
    let recipe = parse_plates(recipe.as_bytes()).unwrap().unwrap();
    let gpu = pollster::block_on(Gpu::headless()).unwrap();
    let started = Instant::now();
    let mut turns = Turns::default();
    let mut between = |ms: f64| turns.add(ms);
    let mut log = |line: &str| {
        println!(
            "{:7.1} s  {:6} MiB  {line}",
            started.elapsed().as_secs_f64(),
            peak_mib()
        )
    };
    let outcome = bake_plates(PlateRun {
        gpu: &gpu,
        recipe: &recipe,
        scene: &root.join("big.scene.toml"),
        anchors: recipe.anchors.as_deref().unwrap(),
        seed: 7,
        max_samples: None,
        out: &root.join("plates"),
        force: true,
        between: &mut between,
        log: &mut log,
    });
    println!(
        "{:7.1} s  peak {} MiB  {}",
        started.elapsed().as_secs_f64(),
        peak_mib(),
        match outcome {
            Ok(outcomes) => format!("baked {}", outcomes.len()),
            Err(error) => error,
        }
    );
}
