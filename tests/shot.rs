use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const FPS: u32 = 30;
const KEYS: usize = 12;
const STEP: f32 = 0.1;

const SCENE: &str = r#"format = 1

include = ["room.scene.toml"]

[mesh.rig]
file = "rig.glb"

[[object]]
name = "rig"
mesh = "rig"
at = [0.0, 0.4, 0.0]
material = "clay"

[object.animation]
clip = "drive"
"#;

fn glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut text = json.to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out
}

fn rig_glb() -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut floats = |values: &[f32], kind: &str, width: usize, bounds: bool| {
        let offset = bin.len();
        for value in values {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        views.push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": values.len() * 4 }));
        let mut accessor = json!({
            "bufferView": views.len() - 1,
            "componentType": 5126,
            "count": values.len() / width,
            "type": kind,
        });
        if bounds {
            let column = |k: usize| values.iter().skip(k).step_by(width).copied();
            accessor["min"] = json!(
                (0..width)
                    .map(|k| column(k).fold(f32::INFINITY, f32::min))
                    .collect::<Vec<_>>()
            );
            accessor["max"] = json!(
                (0..width)
                    .map(|k| column(k).fold(f32::NEG_INFINITY, f32::max))
                    .collect::<Vec<_>>()
            );
        }
        accessors.push(accessor);
        accessors.len() - 1
    };
    let corners: [[f32; 3]; 8] = [
        [-0.3, -0.3, -0.3],
        [0.3, -0.3, -0.3],
        [0.3, 0.3, -0.3],
        [-0.3, 0.3, -0.3],
        [-0.3, -0.3, 0.3],
        [0.3, -0.3, 0.3],
        [0.3, 0.3, 0.3],
        [-0.3, 0.3, 0.3],
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
    let position = floats(&positions, "VEC3", 3, true);
    let normal = floats(&normals, "VEC3", 3, false);
    let times: Vec<f32> = (0..KEYS).map(|key| key as f32 / FPS as f32).collect();
    let input = floats(&times, "SCALAR", 1, true);
    let body: Vec<f32> = (0..KEYS)
        .flat_map(|key| [key as f32 * STEP, 0.0, 0.0])
        .collect();
    let body = floats(&body, "VEC3", 3, false);
    let spin: Vec<f32> = (0..KEYS)
        .flat_map(|key| {
            let half = key as f32 * 0.15;
            [0.0, half.sin(), 0.0, half.cos()]
        })
        .collect();
    let spin = floats(&spin, "VEC4", 4, false);
    let chase: Vec<f32> = (0..KEYS)
        .flat_map(|key| [key as f32 * STEP, 1.0, 4.0])
        .collect();
    let chase = floats(&chase, "VEC3", 3, false);
    let offset = bin.len();
    for index in &indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    views.push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": indices.len() * 4 }));
    accessors.push(json!({
        "bufferView": views.len() - 1,
        "componentType": 5125,
        "count": indices.len(),
        "type": "SCALAR",
    }));
    let index_accessor = accessors.len() - 1;
    let document = json!({
        "asset": { "version": "2.0", "generator": "pfx shot test" },
        "scene": 0,
        "scenes": [{ "nodes": [0, 1] }],
        "nodes": [
            { "name": "box.body", "mesh": 0 },
            { "name": "camera.chase", "camera": 0, "translation": [0.0, 1.0, 4.0] },
        ],
        "cameras": [{ "type": "perspective", "perspective": { "yfov": 0.7, "aspectRatio": 1.0, "znear": 0.1 } }],
        "meshes": [{ "name": "box", "primitives": [{
            "attributes": { "POSITION": position, "NORMAL": normal },
            "indices": index_accessor,
        }] }],
        "animations": [{
            "name": "drive",
            "samplers": [
                { "input": input, "output": body, "interpolation": "LINEAR" },
                { "input": input, "output": spin, "interpolation": "LINEAR" },
                { "input": input, "output": chase, "interpolation": "LINEAR" },
            ],
            "channels": [
                { "sampler": 0, "target": { "node": 0, "path": "translation" } },
                { "sampler": 1, "target": { "node": 0, "path": "rotation" } },
                { "sampler": 2, "target": { "node": 1, "path": "translation" } },
            ],
        }],
        "buffers": [{ "byteLength": bin.len() }],
        "bufferViews": views,
        "accessors": accessors,
    });
    glb(&document, &bin)
}

fn folder() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let folder = root
        .join("tmp")
        .join(format!("shot-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    for entry in std::fs::read_dir(root.join("crates/load/tests/scenes")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
    }
    std::fs::write(folder.join("rig.glb"), rig_glb()).unwrap();
    std::fs::write(folder.join("shot.scene.toml"), SCENE).unwrap();
    folder
}

fn shot(folder: &Path, out: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pfx"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("shot")
        .arg(folder.join("shot.scene.toml"))
        .args(["--frames", "4", "--from", "2", "--size", "64x64", "--fps"])
        .arg(FPS.to_string())
        .arg("--out")
        .arg(folder.join(out))
        .args(extra)
        .output()
        .unwrap()
}

fn lines(output: &Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn frames(dir: &Path) -> Vec<Vec<u8>> {
    (0..4)
        .map(|index| std::fs::read(dir.join(format!("{index:06}.png"))).unwrap())
        .collect()
}

#[test]
#[ignore = "needs a GPU; run under pgpu"]
fn two_shots_of_an_animated_glb_give_the_same_bytes_and_follow_its_camera_node() {
    let folder = folder();
    let args = ["--camera", "chase", "--final", "--json"];
    let first = lines(&shot(&folder, "a", &args));
    let second = lines(&shot(&folder, "b", &args));
    let a = frames(&folder.join("a"));
    assert_eq!(a, frames(&folder.join("b")));
    assert_ne!(a[0], a[3], "the node animation moves the frames");
    for (index, line) in first.iter().take(4).enumerate() {
        let number = 2 + index as u64;
        assert_eq!(line["number"], json!(number));
        let at: Vec<f64> = serde_json::from_value(line["camera"]["at"].clone()).unwrap();
        assert!(
            (at[0] - number as f64 * f64::from(STEP)).abs() < 1e-4,
            "{at:?}"
        );
        assert!(
            (at[1] - 1.4).abs() < 1e-4 && (at[2] - 4.0).abs() < 1e-4,
            "{at:?}"
        );
    }
    let summary = first.last().unwrap();
    assert_eq!(summary["camera"], json!("rig/camera.chase"));
    assert_eq!(summary["warm_frames"], json!(2));
    assert_eq!(summary["frames"], json!(4));
    assert_eq!(
        second.last().unwrap()["scene_sha256"],
        summary["scene_sha256"]
    );
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(folder.join("a").join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["fps"], json!(FPS));
    assert_eq!(manifest["size"], json!([64, 64]));
    std::fs::remove_dir_all(&folder).unwrap();
}
