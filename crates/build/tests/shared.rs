use pfx_build::{build, library::split_glb};
use pfx_glb::summary;
use pfx_recipe::parse;
use std::path::Path;

const STREET: &str = r#"recipe = 1
name = "street"

[materials.paint]
base = [0.7, 0.1, 0.1]

[materials.chrome]
base = [0.9, 0.9, 0.9]
metalness = 1.0

[[object]]
name = "first"
kind = "box"
size = [1.0, 0.5, 2.0]
material = "paint"

[[object]]
name = "turned"
kind = "box"
size = [1.0, 0.5, 2.0]
at = [3.0, 0.0, 0.0]
rotate = [0.0, 90.0, 0.0]
material = "paint"

[[object]]
name = "further"
kind = "box"
size = [1.0, 0.5, 2.0]
at = [0.0, 1.0, -4.0]
material = "paint"

[[object]]
name = "ball"
kind = "sphere"
radius = 0.5
at = [-3.0, 0.0, 0.0]
material = "chrome"
"#;

#[test]
fn repeated_placements_write_one_shared_mesh_and_keep_their_world_bounds() {
    let recipe = parse(STREET).unwrap();
    let built = build(&recipe, Path::new(".")).unwrap();
    let glb = built.try_glb().unwrap();
    let (json, _) = split_glb(&glb).unwrap();
    let nodes = json["nodes"].as_array().unwrap();
    let meshes = json["meshes"].as_array().unwrap();
    assert_eq!(nodes.len(), 4);
    assert_eq!(
        meshes.len(),
        2,
        "three boxes share one mesh beside the ball"
    );
    let mesh_of = |name: &str| {
        nodes
            .iter()
            .find(|node| node["name"] == name)
            .and_then(|node| node["mesh"].as_u64())
            .unwrap()
    };
    assert_eq!(mesh_of("first"), mesh_of("turned"));
    assert_eq!(mesh_of("first"), mesh_of("further"));
    assert_ne!(mesh_of("first"), mesh_of("ball"));
    let shared = summary(&glb).unwrap();
    for part in &built.parts {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for point in &part.node.mesh.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(f64::from(point[k]));
                hi[k] = hi[k].max(f64::from(point[k]));
            }
        }
        let written = shared
            .nodes
            .iter()
            .find(|node| node.name == part.node.name)
            .unwrap();
        for k in 0..3 {
            assert!(
                (written.bounds[0][k] - lo[k]).abs() < 1e-5
                    && (written.bounds[1][k] - hi[k]).abs() < 1e-5,
                "{}: {:?} against {lo:?} {hi:?}",
                part.node.name,
                written.bounds
            );
        }
    }
}

fn model(colour: [f32; 3], length: f32) -> Vec<u8> {
    let half = [0.5f32, 0.25, length * 0.5];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for axis in 0..3 {
        for side in [-1.0f32, 1.0] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let first = positions.len() as u32;
            for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let mut p = [0.0f32; 3];
                p[axis] = side * half[axis];
                p[u] = a * half[u];
                p[v] = b * half[v];
                positions.push(p);
                let mut n = [0.0f32; 3];
                n[axis] = side;
                normals.push(n);
            }
            let quad = if side > 0.0 {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            indices.extend(quad.map(|k| first + k));
        }
    }
    let floats = |values: &[[f32; 3]]| -> Vec<u8> {
        values
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    };
    let mut bin = floats(&positions);
    let normal_at = bin.len();
    bin.extend(floats(&normals));
    let index_at = bin.len();
    bin.extend(indices.iter().flat_map(|k| k.to_le_bytes()));
    let document = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "body", "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0}]}],
        "materials": [{"name": "paint", "pbrMetallicRoughness": {"baseColorFactor": [colour[0], colour[1], colour[2], 1.0], "metallicFactor": 0.0}}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": normal_at},
            {"buffer": 0, "byteOffset": normal_at, "byteLength": index_at - normal_at},
            {"buffer": 0, "byteOffset": index_at, "byteLength": bin.len() - index_at}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": positions.len(), "type": "VEC3",
             "min": [-half[0], -half[1], -half[2]], "max": half},
            {"bufferView": 1, "componentType": 5126, "count": normals.len(), "type": "VEC3"},
            {"bufferView": 2, "componentType": 5125, "count": indices.len(), "type": "SCALAR"}
        ]
    });
    pfx_build::library::join_glb(&document, &bin)
}

#[test]
fn copies_of_a_model_whose_materials_clash_with_another_share_one_renamed_material_and_mesh() {
    let dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tmp/shared-clash"
    ));
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("red.glb"), model([0.8, 0.1, 0.1], 2.0)).unwrap();
    std::fs::write(dir.join("blue.glb"), model([0.1, 0.1, 0.8], 3.0)).unwrap();
    let mut text = String::from("recipe = 1\nname = \"clash\"\n");
    for (name, file, x) in [
        ("r0", "red", 0.0),
        ("b0", "blue", 2.0),
        ("b1", "blue", 4.0),
        ("r1", "red", 6.0),
        ("b2", "blue", 8.0),
    ] {
        text.push_str(&format!(
            "\n[[object]]\nname = \"{name}\"\nkind = \"mesh\"\npath = \"{file}.glb\"\nat = [{x}, 0.0, 0.0]\n"
        ));
    }
    let built = build(&parse(&text).unwrap(), dir).unwrap();
    let glb = built.try_glb().unwrap();
    let (json, _) = split_glb(&glb).unwrap();
    let materials: Vec<&str> = json["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(materials, ["paint", "b0.paint"]);
    assert_eq!(json["meshes"].as_array().unwrap().len(), 2);
}
